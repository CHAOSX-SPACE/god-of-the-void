//! Biblioteca de modelos en la app: buscar en Hugging Face medido contra esta Mac
//! (o una simulada), bajar con progreso — también como tarea del panel — y cancelar
//! (lo bajado queda en `.part` y la próxima vez REANUDA).

use crate::{emit, store, Hub};
use gotv_core::biblioteca::{self as bib, MacProfile};
use serde_json::json;
use std::collections::HashSet;
use std::time::Instant;

/// Lo instalado, sin tocar carpetas protegidas a las que el usuario no dio acceso.
pub fn installed(root: &std::path::Path) -> HashSet<String> {
    let cfg = gotv_core::Config::load(root);
    let dirs = crate::acceso::model_dirs_allowed(&cfg.model_dirs, &crate::acceso::granted_roots(&store::Store::load().trusted));
    bib::installed_set(&dirs)
}

/// La Mac real y la elegida (`"this"` o `"M4 Pro|48"`).
pub fn profiles(mac: &str) -> Result<(MacProfile, MacProfile), String> {
    let this = MacProfile::this_mac(&gotv_core::Hardware::detect());
    let chosen = if mac.is_empty() || mac == "this" {
        this.clone()
    } else {
        MacProfile::simulated(mac).ok_or_else(|| gotv_core::tr!("perfil de Mac inválido: {mac}", "invalid Mac profile: {mac}"))?
    };
    Ok((this, chosen))
}

/// La búsqueda tipada: (esta Mac, la Mac elegida, modelos medidos contra ella).
pub async fn entries(hub: &Hub, query: &str, org: &str, sort: &str, mac: &str) -> Result<(MacProfile, MacProfile, Vec<bib::Entry>), String> {
    let root = hub.root.lock().await.clone();
    let m = mac.to_string();
    let (this, chosen, inst) = tokio::task::spawn_blocking(move || profiles(&m).map(|(t, c)| (t, c, installed(&root))))
        .await
        .map_err(|e| e.to_string())??;
    let items = bib::search(query, org, sort, 100, &chosen, &inst).await.map_err(|e| e.to_string())?;
    Ok((this, chosen, items))
}

pub async fn search(hub: &Hub, query: &str, org: &str, sort: &str, mac: &str) -> Result<serde_json::Value, String> {
    let (this, chosen, items) = entries(hub, query, org, sort, mac).await?;
    let presets: Vec<MacProfile> = bib::PRESETS.iter().filter_map(|p| MacProfile::simulated(p)).collect();
    let pulls: Vec<serde_json::Value> = hub.pulls.lock().await.iter().map(|(r, p)| json!({"repo": r, "done": p.done, "total": p.total})).collect();
    Ok(json!({"this": this, "mac": chosen, "presets": presets, "items": items, "pulls": pulls}))
}

/// Una descarga en curso.
pub struct Pull {
    pub abort: tokio::task::AbortHandle,
    pub task: u64,
    pub done: u64,
    pub total: u64,
}

/// Qué se baja: un modelo de lenguaje (comprobando que quepa) o un generador (M0: con su
/// licencia aceptada y, si es restringido, el token del Llavero).
#[derive(Clone, Copy, PartialEq)]
pub enum Origen {
    Modelo,
    Generador,
}

/// Arranca la descarga en segundo plano. Antes comprueba, con la ficha REAL del
/// repo (su config + sus safetensors), que quepa en esta Mac: no se bajan 40 GB
/// para descubrir después que no arrancan.
pub async fn start_pull(hub: &Hub, repo: &str) -> Result<String, String> {
    start_pull_de(hub, repo, Origen::Modelo).await
}

pub async fn start_pull_de(hub: &Hub, repo: &str, origen: Origen) -> Result<String, String> {
    if !gotv_core::hub::valid_repo(repo) {
        return Err(gotv_core::tr!("repo inválido: «{repo}» (forma org/nombre)", "invalid repo: «{repo}» (org/name form)"));
    }
    if hub.pulls.lock().await.contains_key(repo) {
        return Err(gotv_core::tr!("{repo} ya se está bajando", "{repo} is already downloading"));
    }
    let estimado = match origen {
        Origen::Modelo => {
            let (card, _) = gotv_core::hub::inspect(repo).await.map_err(|e| e.to_string())?;
            let hw = gotv_core::Hardware::detect();
            let fit = gotv_core::fit(&card, &hw, None, 4096u64.min(card.max_ctx));
            if !fit.ok {
                return Err(gotv_core::tr!("no lo bajo: {}", "not downloading it: {}", fit.why));
            }
            card.weights
        }
        Origen::Generador => {
            use gotv_core::generadores as gen;
            let g = gen::ficha(repo).ok_or_else(|| gotv_core::tr!("{repo} no está en el catálogo de generadores", "{repo} is not in the generator catalog"))?;
            // las mismas puertas que `generadores::bajar`, ANTES de abrir una tarea
            if gen::aceptacion(g.repo).is_none() {
                return Err(gotv_core::tr!("falta aceptar la licencia de {} ({})", "the license of {} ({}) has not been accepted", g.nombre, g.licencia));
            }
            if g.restringido && !gotv_core::llaves::has(gen::LLAVE_HF) {
                return Err(gen::explicar_restringido(g));
            }
            g.bytes_repo
        }
    };
    let tid = hub.task_begin("download", &gotv_core::tr!("bajar {repo}", "download {repo}")).await;
    let (h, r) = (hub.clone(), repo.to_string());
    let job = tokio::spawn(async move {
        let t0 = Instant::now();
        let last = std::sync::Mutex::new(Instant::now() - std::time::Duration::from_secs(5));
        let progress = |done: u64, total: u64| {
            let mut l = last.lock().unwrap();
            if l.elapsed().as_millis() < 400 && done < total {
                return;
            }
            *l = Instant::now();
            let rate = done as f64 / 1e6 / t0.elapsed().as_secs_f64().max(0.1);
            emit(&h.app, json!({"kind": "pull", "repo": r, "state": "run", "done": done, "total": total, "rate": rate}));
            let (h2, r2) = (h.clone(), r.clone());
            tokio::spawn(async move {
                if let Some(p) = h2.pulls.lock().await.get_mut(&r2) {
                    p.done = done;
                    p.total = total;
                }
                let pct = (done * 100).checked_div(total).unwrap_or(0);
                h2.task_update(tid, &format!("{pct} % · {:.1}/{:.1} GB · {rate:.0} MB/s", done as f64 / 1e9, total as f64 / 1e9)).await;
            });
        };
        let res = match origen {
            Origen::Modelo => gotv_core::hub::pull(&r, &progress).await,
            Origen::Generador => gotv_core::generadores::bajar(&r, &progress).await,
        };
        h.pulls.lock().await.remove(&r);
        match res {
            Ok(dest) => {
                h.task_end(tid, "done", &gotv_core::tr!("listo en {}", "ready at {}", dest.display())).await;
                emit(&h.app, json!({"kind": "pull", "repo": r, "state": "done", "path": dest.display().to_string()}));
            }
            Err(e) => {
                h.task_end(tid, "error", &e.to_string()).await;
                emit(&h.app, json!({"kind": "pull", "repo": r, "state": "error", "error": e.to_string()}));
            }
        }
    });
    hub.pulls.lock().await.insert(repo.to_string(), Pull { abort: job.abort_handle(), task: tid, done: 0, total: estimado });
    emit(&hub.app, json!({"kind": "pull", "repo": repo, "state": "run", "done": 0, "total": estimado, "rate": 0.0}));
    Ok(gotv_core::tr!(
        "⬇ bajando {repo} ({:.1} GB) en segundo plano — el avance va en Tareas",
        "⬇ downloading {repo} ({:.1} GB) in the background — progress is in Tasks",
        estimado as f64 / 1e9
    ))
}

/// Corta la descarga; lo bajado queda en `.part` para reanudar.
pub async fn cancel_pull(hub: &Hub, repo: &str) -> bool {
    let Some(p) = hub.pulls.lock().await.remove(repo) else { return false };
    p.abort.abort();
    hub.task_end(p.task, "stopped", gotv_core::i18n::t("cancelada · lo bajado se reanuda la próxima vez", "cancelled · what was downloaded resumes next time")).await;
    emit(&hub.app, json!({"kind": "pull", "repo": repo, "state": "cancel"}));
    true
}

fn speed_label(s: &str) -> &'static str {
    match s {
        "muy-rapido" => gotv_core::i18n::t("muy rápido", "very fast"),
        "rapido" => gotv_core::i18n::t("rápido", "fast"),
        "medio" => gotv_core::i18n::t("medio", "medium"),
        "lento" => gotv_core::i18n::t("lento", "slow"),
        _ => gotv_core::i18n::t("muy lento", "very slow"),
    }
}

/// `/modelos <búsqueda>` en el chat: los que caben en esta Mac, del más rápido al más lento.
pub async fn chat_list(hub: &Hub, query: &str) -> String {
    let (_, chosen, items) = match entries(hub, query, "", "downloads", "this").await {
        Ok(v) => v,
        Err(e) => return format!("✗ {e}"),
    };
    let mac = chosen.label;
    let mut items: Vec<bib::Entry> = items.into_iter().filter(|e| e.fits).collect();
    if items.is_empty() {
        return gotv_core::tr!("Nada que quepa en {mac} con «{query}».", "Nothing that fits {mac} for «{query}».");
    }
    // los 12 más bajados que caben, ordenados del más rápido al más lento
    let total = items.len();
    items.truncate(12);
    items.sort_by(|a, b| b.tok_s.total_cmp(&a.tok_s));
    let mut out = gotv_core::tr!(
        "**Modelos para {mac}** — los {} más bajados de {total} que caben (tok/s estimados)\n\n| | modelo | peso | velocidad | |\n|---|---|---|---|---|\n",
        "**Models for {mac}** — the {} most downloaded of {total} that fit (estimated tok/s)\n\n| | model | size | speed | |\n|---|---|---|---|---|\n",
        items.len()
    );
    for e in &items {
        out.push_str(&format!(
            "| {} | `{}` | {:.1} GB | {} · ~{:.0} tok/s | {}{} |\n",
            if e.installed { "●" } else { "" },
            e.repo,
            e.bytes as f64 / 1e9,
            speed_label(e.speed),
            e.tok_s,
            if e.vision { "👁" } else { "" },
            if e.tight { " ⚠" } else { "" }
        ));
    }
    out.push_str(gotv_core::i18n::t(
        "\n● instalado · ⚠ cabe justo · bajar: `/descargar org/nombre` · la biblioteca completa con filtros: Configuración → Biblioteca",
        "\n● installed · ⚠ tight · download: `/download org/name` · the full library with filters: Settings → Library",
    ));
    out
}
