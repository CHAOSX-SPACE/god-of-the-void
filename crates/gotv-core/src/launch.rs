//! Arranque (E7): resuelve modelo y contexto según ESTA máquina, y se niega con números si el
//! modelo no cabe. Lo usan el CLI, la TUI y la app por igual. El motor es SIEMPRE el núcleo Rust
//! (`gotv-mlx`): desde 2026-11-12 GOTV no busca ni lanza Python.

use crate::config::{preset_to_budget, Config};
use crate::hardware::{available_now, Hardware, GIB};
use crate::models::{auto_context, choose, discover, fit, safe_ctx, ModelCard};
use anyhow::{anyhow, bail, Result};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Contexto mínimo con el que un agente puede trabajar.
pub const MIN_AGENT_CTX: u64 = 4096;

pub struct Launch {
    pub hw: Hardware,
    pub card: Option<ModelCard>,
    /// Lo que el usuario debe ver antes de empezar (elección, ajustes, avisos).
    pub notes: Vec<String>,
}

fn home() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".into()))
}

fn gb(b: u64) -> f64 {
    b as f64 / GIB as f64
}

/// El contexto REAL que tendrá un modelo: lo pedido (o automático) acotado por el tope del
/// modelo y por la memoria. Medido (GUI): se pedía 128k para el 32b y quedaba ~32k SIN aviso
/// — el usuario veía «lo mismo» arriba y creía que no se guardó.
#[derive(Debug, Clone)]
pub struct CtxPlan {
    pub ctx: u64,
    pub requested: Option<u64>,
    /// tope del propio modelo (`max_position_embeddings`)
    pub model_max: u64,
    /// lo que cabe en la memoria disponible
    pub mem_max: u64,
    pub notes: Vec<String>,
}

impl CtxPlan {
    /// Por qué el contexto no es lo pedido (o `None` si lo es).
    pub fn explain(&self, model: &str) -> Option<String> {
        let r = self.requested?;
        if r == self.ctx {
            return None;
        }
        Some(
            if r > self.model_max && self.ctx >= self.model_max.saturating_sub(64) {
                crate::tr!(
                "Pediste {r} tokens, pero {model} admite como máximo {}: el contexto queda en {}.",
                "You asked for {r} tokens, but {model} supports at most {}: the context stays at {}.",
                self.model_max,
                self.ctx
            )
            } else {
                crate::tr!(
                "Pediste {r} tokens; {model} admite hasta {}, pero con la memoria de esta Mac caben {}: el contexto queda en {}.",
                "You asked for {r} tokens; {model} supports up to {}, but this Mac's memory fits {}: the context stays at {}.",
                self.model_max,
                self.mem_max,
                self.ctx
            )
            },
        )
    }
}

pub fn resolve_context(
    c: &ModelCard,
    hw: &Hardware,
    avail: Option<u64>,
    preset: Option<usize>,
) -> Result<CtxPlan> {
    let requested = preset.map(|n| n as u64);
    let wanted = requested
        .unwrap_or_else(|| auto_context(c, hw, avail))
        .min(c.max_ctx);
    let f = fit(c, hw, avail, wanted);
    let mem_max = fit(c, hw, avail, c.max_ctx).max_ctx;
    let mut notes = vec![];
    let ctx = if f.ok {
        wanted
    } else if safe_ctx(c, &f) >= MIN_AGENT_CTX.min(c.max_ctx) {
        let safe = safe_ctx(c, &f);
        notes.push(crate::tr!(
            "⚠ contexto bajado de {wanted} a {safe} tokens para que quepa con margen",
            "⚠ context lowered from {wanted} to {safe} tokens so it fits with margin"
        ));
        safe
    } else {
        bail!(
            "{}",
            crate::tr!(
                "{} no cabe en esta Mac: {}",
                "{} does not fit this Mac: {}",
                c.name,
                f.why
            )
        );
    };
    Ok(CtxPlan {
        ctx,
        requested,
        model_max: c.max_ctx,
        mem_max,
        notes,
    })
}

/// Memoria que ocupa un modelo YA cargado con su contexto: se SUMA a la libre al
/// recalcular (medido: si no, GOTV decía «no cabe» por culpa del propio modelo).
pub fn loaded_footprint(c: &ModelCard, ctx: u64) -> u64 {
    c.need(ctx)
}

/// La ficha del modelo cargado (su carpeta es `cfg.model`).
pub fn loaded_card(cfg: &Config) -> Option<ModelCard> {
    let p = Path::new(&cfg.model);
    ModelCard::read(p, "gotv", &crate::models::display_name(p))
}

/// Memoria libre + lo que ocupa el modelo YA cargado (`cfg.model` es su carpeta).
/// Una sola regla para GUI y TUI: si no, el propio modelo hacía que GOTV dijera
/// «no cabe» o recortara el contexto de más.
pub fn avail_with_loaded(cfg: &Config) -> Option<u64> {
    let free = crate::hardware::available_now()?;
    let own = loaded_card(cfg)
        .map(|c| loaded_footprint(&c, cfg.context_budget as u64))
        .unwrap_or(0);
    Some(free + own)
}

/// Filtros cuando el usuario fija temperatura y el modelo no declara los suyos.
pub const DEFAULT_SAMPLING: gotv_engine::Sampling = gotv_engine::Sampling {
    temperature: 0.7,
    top_p: Some(0.8),
    top_k: Some(20),
    min_p: None,
};

pub fn prepare(cfg: &mut Config) -> Result<Launch> {
    let mut notes: Vec<String> = cfg.warnings.iter().map(|w| format!("⚠ {w}")).collect();
    let hw = Hardware::detect();
    let avail = available_now();
    let cards = discover(&cfg.model_dirs);

    // ── Modelo ──────────────────────────────────────────────────────────────
    let card: Option<ModelCard> = if cfg.model.is_empty() {
        let fits_min =
            |c: &&ModelCard| c.runnable && fit(c, &hw, avail, MIN_AGENT_CTX.min(c.max_ctx)).ok;
        // Orden: preferencia DECLARADA → último usado → el más capaz que quepa.
        // (Medido: con "último usado" primero, una prueba puntual con el 0.6b
        // dejaba al 0.6b como modelo por defecto por encima de `prefer`.)
        let preferred = cfg.prefer.iter().find_map(|p| {
            let p = p.to_lowercase();
            cards
                .iter()
                .filter(|c| fits_min(c))
                .find(|c| c.name.to_lowercase().contains(&p))
        });
        let last = read_last(&state_path());
        let pick = preferred
            .map(|c| (c, crate::i18n::t("tu preferido", "your preferred one")))
            .or_else(|| {
                last.as_deref()
                    .and_then(|l| cards.iter().find(|c| c.path.to_string_lossy() == l))
                    .filter(fits_min)
                    .map(|c| {
                        (
                            c,
                            crate::i18n::t("el último que usaste", "the last one you used"),
                        )
                    })
            })
            .or_else(|| {
                choose(&cards, &hw, avail, &[], 16384).map(|c| {
                    (
                        c,
                        crate::i18n::t("el más capaz que cabe", "the most capable that fits"),
                    )
                })
            });
        let (c, why) = pick.ok_or_else(|| {
            anyhow!(
                "{}",
                crate::tr!(
                    "ningún modelo cabe ahora en esta Mac ({:.1} GB libres de {:.1} GB). Mira `gotv models`.",
                    "no model fits this Mac right now ({:.1} GB free of {:.1} GB). See `gotv models`.",
                    gb(avail.unwrap_or(0)),
                    gb(hw.gpu_budget)
                )
            )
        })?;
        cfg.model = c.path.to_string_lossy().to_string();
        notes.push(crate::tr!(
            "modelo: {} — {why}",
            "model: {} — {why}",
            c.name
        ));
        Some(c.clone())
    } else {
        let p = Path::new(&cfg.model);
        let found = if p.exists() {
            ModelCard::read(p, "config", &crate::models::display_name(p))
        } else {
            // Por NOMBRE (`GOTV_MODEL=propios/x`, `org/repo` bajado con pull): el motor
            // necesita la RUTA de la carpeta.
            let c = cards.iter().find(|c| c.name == cfg.model).cloned();
            if let Some(c) = &c {
                cfg.model = c.path.to_string_lossy().to_string();
            }
            c
        };
        if found.is_none() {
            notes.push(crate::tr!(
                "⚠ no pude leer la ficha de {}: arranco sin chequeo de memoria",
                "⚠ could not read the model card of {}: starting without a memory check",
                cfg.model
            ));
        }
        found
    };

    // ── Contexto y memoria ──────────────────────────────────────────────────
    let preset = preset_to_budget(&cfg.context_preset);
    if let Some(c) = &card {
        if !c.runnable {
            bail!(
                "{}",
                crate::tr!(
                    "{} no es un modelo que corra el núcleo: {}",
                    "{} is not a model the core runs: {}",
                    c.name,
                    if c.arch == "propia" {
                        crate::i18n::t(
                            "es un modelo nuestro; se correrá en la ola S3",
                            "it is one of our own models; it will run in wave S3",
                        )
                    } else {
                        crate::i18n::t("no es un modelo de lenguaje", "it is not a language model")
                    }
                )
            );
        }
        // Banco del Air (pedido del Portador, 2026-11-14: «si no cabe usará disco y será más lento»):
        // con GOTV_FORZAR_MEMORIA se mide contra la Mac LIBRE y no contra lo libre ahora; lo que
        // ocupan otros programas pasa a disco. El presupuesto de GPU sigue mandando: es memoria fija.
        let avail = if std::env::var_os("GOTV_FORZAR_MEMORIA").is_some() { None } else { avail };
        let plan = resolve_context(c, &hw, avail, preset)?;
        notes.extend(plan.notes.iter().cloned());
        let ctx = plan.ctx;
        cfg.context_budget = ctx as usize;
        let f = fit(c, &hw, avail, ctx);
        let need = f.need;
        notes.push(crate::tr!(
            "{} · {} ({}) · contexto {} tok · memoria {:.1} de {:.1} GB",
            "{} · {} ({}) · context {} tok · memory {:.1} of {:.1} GB",
            hw.device,
            c.name,
            c.kind(),
            ctx,
            gb(need),
            gb(f.limit)
        ));
    } else {
        cfg.context_budget = preset.unwrap_or(16384);
    }

    // Muestreo: VORAZ por defecto para el trabajo de agente. Medido 2026-11-08
    // con Qwen3-32B y los rescates de recover.rs: voraz 3/3 ediciones correctas
    // (~60 s); el muestreo que recomienda Qwen (0.7 · 0.8 · 20) 0/2, con bucles
    // de 167 s y 951 s. El del fabricante queda en la ficha para P11, que lo
    // MEDIRÁ por modelo en vez de suponerlo. `temperature` en la config manda.
    let mut sampling = gotv_engine::Sampling::GREEDY;
    let from = match cfg.temperature {
        Some(t) => {
            sampling = card
                .as_ref()
                .and_then(|c| c.sampling)
                .unwrap_or(DEFAULT_SAMPLING);
            sampling.temperature = t;
            crate::i18n::t(
                "tuyo (temperature en la config)",
                "yours (temperature in the config)",
            )
        }
        None => crate::i18n::t(
            "voraz (el que midió mejor para usar herramientas)",
            "greedy (measured best for tool use)",
        ),
    };
    cfg.sampling = sampling;
    // S3: un modelo de chat (los propios: contexto 512, sin herramientas) va sin el
    // prompt de agente, que ni cabría
    cfg.chat_only = card.as_ref().is_some_and(|c| c.chat_only);
    if cfg.chat_only {
        notes.push(crate::i18n::t(
            "modelo de CHAT: sin herramientas ni prompt de agente (contexto corto); conversa, no edita",
            "CHAT model: no tools or agent prompt (short context); it talks, it does not edit",
        ).into());
    }
    notes.push(crate::tr!(
        "muestreo {from}: {}",
        "sampling {from}: {}",
        sampling.summary()
    ));

    Ok(Launch { hw, card, notes })
}

// ── Último modelo usado ─────────────────────────────────────────────────────

/// Formato propio (códec, mapa LAST); hasta 2026-11-18 era `state.json` (se migra solo la primera vez).
fn state_path() -> PathBuf {
    home().join(".config/gotv/state.gotv")
}

fn read_last(path: &Path) -> Option<String> {
    crate::programar::codec::mapa_migrado(path, &path.with_extension("json"), b"LAST").remove("last_model").filter(|m| !m.is_empty())
}

fn write_last(path: &Path, model: &str) {
    let mut m = crate::programar::codec::leer_mapa(path, b"LAST");
    m.insert("last_model".into(), model.into());
    let _ = crate::programar::codec::guardar_mapa(path, b"LAST", &m);
}

/// Recordar el modelo tras un arranque exitoso.
/// Tras apagar un modelo, macOS devuelve su memoria en segundos, no al instante
/// (medido: justo después de cerrar el 32b, el siguiente arranque veía 23 GB
/// libres en vez de 39 y achicaba el contexto de 41k a 13k). Espera a que la
/// memoria libre deje de subir (máx. `limit`).
pub async fn wait_memory_settles(limit: Duration) {
    let _ = tokio::task::spawn_blocking(move || wait_memory_settles_blocking(limit)).await;
}

pub fn wait_memory_settles_blocking(limit: Duration) {
    let t0 = std::time::Instant::now();
    let mut last = crate::hardware::available_now().unwrap_or(0);
    let mut calm = 0;
    while t0.elapsed() < limit {
        std::thread::sleep(Duration::from_millis(400));
        let now = crate::hardware::available_now().unwrap_or(0);
        // «quieta» = sube menos de 128 MiB entre lecturas, tres veces seguidas
        if now <= last + 128 * 1024 * 1024 {
            calm += 1;
            if calm >= 3 {
                return;
            }
        } else {
            calm = 0;
        }
        last = now;
    }
}

/// Busca un modelo ejecutable por nombre (sin distinguir mayúsculas). Si el
/// texto coincide con varios y ninguno exacto, devuelve los candidatos.
pub fn find_model(cards: &[ModelCard], query: &str) -> Result<ModelCard, Vec<String>> {
    let q = query.trim().to_lowercase();
    let runnable: Vec<&ModelCard> = cards.iter().filter(|c| c.runnable).collect();
    if let Some(c) = runnable.iter().find(|c| {
        c.name.to_lowercase() == q || c.name.to_lowercase().rsplit('/').next() == Some(q.as_str())
    }) {
        return Ok((*c).clone());
    }
    let hits: Vec<&&ModelCard> = runnable
        .iter()
        .filter(|c| c.name.to_lowercase().contains(&q))
        .collect();
    match hits.as_slice() {
        [one] => Ok((**one).clone()),
        [] => Err(vec![]),
        many => Err(many.iter().map(|c| c.name.clone()).collect()),
    }
}

pub fn remember_model(model: &str) {
    write_last(&state_path(), model);
}

/// El último modelo usado (ruta local o `proveedor:modelo` de nube).
pub fn last_model() -> Option<String> {
    read_last(&state_path())
}

#[cfg(test)]
mod tests {
    #[test]
    fn busca_modelo_por_nombre() {
        let mk = |n: &str| crate::models::ModelCard {
            name: n.into(),
            path: n.into(),
            source: "t".into(),
            arch: "t".into(),
            layers: 1,
            attn_layers: 1,
            kv_heads: 1,
            head_dim: 1,
            mla_dim: None,
            max_ctx: 1,
            experts: None,
            bits: None,
            vision: false,
            weights: 1,
            runnable: true,
            sampling: None,
            vocab: None,
            audio: false,
            chat_only: false,
        };
        let cards = vec![
            mk("mlx-community/Qwen3-0.6B-4bit"),
            mk("mlx-community/Qwen3-32B-4bit-DWQ"),
            mk("mlx-community/Qwen3-8B-4bit"),
        ];
        assert_eq!(
            super::find_model(&cards, "32b").unwrap().name,
            "mlx-community/Qwen3-32B-4bit-DWQ"
        );
        assert_eq!(
            super::find_model(&cards, "0.6B").unwrap().name,
            "mlx-community/Qwen3-0.6B-4bit"
        );
        assert_eq!(super::find_model(&cards, "qwen3").unwrap_err().len(), 3);
        assert!(super::find_model(&cards, "llama").unwrap_err().is_empty());
    }

    use super::*;

    #[test]
    fn recuerda_el_ultimo_modelo() {
        let p = crate::CarpetaPrueba::archivo("gotv-state", "gotv");
        assert_eq!(read_last(&p), None);
        write_last(&p, "/m/Qwen3-32B");
        assert_eq!(read_last(&p).as_deref(), Some("/m/Qwen3-32B"));
        std::fs::write(&p, "basura").unwrap();
        assert_eq!(read_last(&p), None, "un estado roto no rompe el arranque");
        // el `state.json` de antes se migra al propio la primera vez
        let q = crate::CarpetaPrueba::archivo("gotv-state-viejo", "gotv");
        std::fs::write(q.with_extension("json"), r#"{"last_model":"/m/Huihui"}"#).unwrap();
        assert_eq!(read_last(&q).as_deref(), Some("/m/Huihui"));
        assert!(!q.with_extension("json").exists(), "el viejo se retira (queda como .json.migrado)");
        let _ = std::fs::remove_file(q.with_extension("json.migrado"));
    }

    /// El caso del Portador: pidió 128k a un modelo que admite 40 960. Antes se
    /// recortaba en silencio y la cabecera «no cambiaba»; ahora se explica.
    #[test]
    fn contexto_pedido_de_mas_se_explica() {
        let c = crate::models::ModelCard {
            name: "q32".into(),
            path: "q32".into(),
            source: "t".into(),
            arch: "t".into(),
            layers: 2,
            attn_layers: 2,
            kv_heads: 1,
            head_dim: 8,
            mla_dim: None,
            max_ctx: 40960,
            experts: None,
            bits: Some(4),
            vision: false,
            weights: 1 << 20,
            runnable: true,
            sampling: None,
            vocab: None,
            audio: false,
            chat_only: false,
        };
        let hw = Hardware::detect();
        let p = resolve_context(&c, &hw, Some(1 << 40), Some(131072)).unwrap();
        assert_eq!(
            (p.ctx, p.requested, p.model_max),
            (40960, Some(131072), 40960)
        );
        let e = p.explain("q32").expect("lo recortado se explica");
        assert!(e.contains("131072") && e.contains("40960"), "{e}");
        let q = resolve_context(&c, &hw, Some(1 << 40), Some(16384)).unwrap();
        assert_eq!(q.ctx, 16384);
        assert!(q.explain("q32").is_none(), "lo que cabe no necesita excusa");
    }

}
