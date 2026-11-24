//! F17 · las LECTURAS de git dentro del proceso, con `gix` (git en Rust puro, nivel 1 de L1): estado,
//! historial, autoría por línea, ramas y conflictos con sus tres lados. Medido el 2026-11-15 contra el
//! binario `git` (2.54): en el repo de GOTV (2 317 archivos, 182 commits) estado 9–20 ms (git 35), 200
//! commits 3,5 ms (27), ramas 0,1 ms (26), autoría de un archivo de 10 667 líneas 99–147 ms (127); en uno de
//! 11 631 archivos y 1 047 commits, estado 39 ms (68), historial 2,7 ms (32), autoría 55 ms (81).
//! Lo que `gix` no cubre (`log -L`, el texto de una diferencia) va al binario (nivel 2), en `super`.
use anyhow::{anyhow, Result};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// El repositorio que contiene `root` (sube hasta encontrarlo).
pub fn abrir(root: &Path) -> Result<gix::Repository> {
    gix::discover(root).map_err(|_| anyhow!("{}", crate::tr!("{} no está dentro de un repositorio git", "{} is not inside a git repository", root.display())))
}

/// La carpeta de trabajo del repositorio.
pub fn carpeta(repo: &gix::Repository) -> Result<PathBuf> {
    let w = repo.workdir().ok_or_else(|| anyhow!("{}", crate::i18n::t("el repositorio no tiene carpeta de trabajo (bare)", "the repository has no working folder (bare)")))?;
    Ok(w.canonicalize().unwrap_or_else(|_| w.to_path_buf()))
}

/// La ruta de `p` relativa a la carpeta del repositorio, con `/`.
pub fn relativa(repo: &gix::Repository, root: &Path, p: &str) -> Result<String> {
    relativa_a(&carpeta(repo)?, root, p)
}

/// Lo mismo, con la carpeta del repositorio ya sabida (canónica).
pub fn relativa_a(base: &Path, root: &Path, p: &str) -> Result<String> {
    let abs = crate::paths::resolve(root, p);
    // un archivo borrado no se puede canonizar: su carpeta sí
    let abs = abs.canonicalize().unwrap_or_else(|_| match (abs.parent().and_then(|d| d.canonicalize().ok()), abs.file_name()) {
        (Some(d), Some(n)) => d.join(n),
        _ => abs.clone(),
    });
    let r = abs.strip_prefix(base).map_err(|_| anyhow!("{}", crate::tr!("{p} está fuera del repositorio", "{p} is outside the repository")))?;
    Ok(r.to_string_lossy().replace('\\', "/"))
}

fn corto(id: &gix::ObjectId) -> String {
    id.to_hex_with_len(7).to_string()
}

/// Fecha corta de un commit (`2026-11-15`).
fn fecha(c: &gix::Commit<'_>) -> String {
    c.time().ok().and_then(|t| t.format(gix::date::time::format::SHORT).ok()).unwrap_or_default()
}

/// `a1b2c3d 2026-11-15 Ana — resumen`
fn linea_commit(c: &gix::Commit<'_>) -> String {
    let autor = c.author().map(|a| a.name.to_string()).unwrap_or_default();
    let resumen = c.message().map(|m| m.summary().to_string()).unwrap_or_default();
    format!("{} {} {autor} — {}", corto(&c.id), fecha(c), resumen.chars().take(120).collect::<String>())
}

/// Lo que GOTV no sabe manejar y lo dice en vez de adivinar.
fn advertencias(repo: &gix::Repository, base: &Path) -> Vec<String> {
    let mut v = Vec::new();
    if let Some(s) = repo.state() {
        v.push(crate::tr!("⚠ operación a medias: {s:?} (termínala o abórtala antes de seguir)", "⚠ operation in progress: {s:?} (finish or abort it before going on)"));
    }
    if base.join(".gitmodules").is_file() {
        v.push(crate::i18n::t("⚠ tiene submódulos: GOTV los ve pero no los maneja (usa bash)", "⚠ it has submodules: GOTV sees them but does not handle them (use bash)").into());
    }
    if std::fs::read_to_string(base.join(".gitattributes")).is_ok_and(|s| s.contains("filter=lfs")) {
        v.push(crate::i18n::t("⚠ usa Git LFS: los archivos grandes son punteros; GOTV no los baja", "⚠ it uses Git LFS: large files are pointers; GOTV does not fetch them").into());
    }
    if repo.config_snapshot().boolean("core.sparseCheckout").unwrap_or(false) {
        v.push(crate::i18n::t("⚠ sparse-checkout activo: no todos los archivos están en la carpeta", "⚠ sparse-checkout is on: not every file is in the folder").into());
    }
    v
}

/// Cuántos commits tiene `a` que no tiene `b`.
fn delante(repo: &gix::Repository, a: gix::ObjectId, b: gix::ObjectId) -> usize {
    repo.rev_walk([a]).with_hidden([b]).all().map(|w| w.take(10_000).filter(Result::is_ok).count()).unwrap_or(0)
}

/// `estado`: rama, adelantos y atrasos, archivos por estado, conflictos y lo que esté a medias.
pub fn estado(root: &Path) -> Result<String> {
    let repo = abrir(root)?;
    let base = carpeta(&repo)?;
    let mut out = Vec::new();
    let head = repo.head_id().ok().map(|h| h.detach());
    match repo.head_name()? {
        Some(nombre) => {
            let corto_rama = nombre.shorten().to_string();
            let mut l = crate::tr!("rama {corto_rama}", "branch {corto_rama}");
            if let (Some(h), Some(Ok(up))) = (head, repo.branch_remote_tracking_ref_name(nombre.as_ref(), gix::remote::Direction::Fetch)) {
                if let Ok(mut r) = repo.find_reference(up.as_ref()) {
                    if let Ok(u) = r.peel_to_id() {
                        let (a, d) = (delante(&repo, h, u.detach()), delante(&repo, u.detach(), h));
                        l.push_str(&crate::tr!(" · {a} por delante y {d} por detrás de {}", " · {a} ahead and {d} behind {}", up.shorten()));
                    }
                }
            }
            if head.is_none() {
                l.push_str(crate::i18n::t(" (sin commits todavía)", " (no commits yet)"));
            }
            out.push(l);
        }
        None => out.push(crate::tr!("HEAD suelto en {}", "detached HEAD at {}", head.map(|h| corto(&h)).unwrap_or_default())),
    }
    let mut preparados: Vec<String> = Vec::new();
    let mut sin_preparar: Vec<String> = Vec::new();
    let mut nuevos: Vec<String> = Vec::new();
    let mut conflictos: Vec<String> = Vec::new();
    let iter = repo.status(gix::progress::Discard)?.into_iter(Vec::<gix::bstr::BString>::new())?;
    for item in iter {
        let Ok(item) = item else { continue };
        match item {
            gix::status::Item::IndexWorktree(i) => {
                use gix::status::index_worktree::iter::Summary as S;
                let ruta = i.rela_path().to_string();
                match i.summary() {
                    Some(S::Conflict) => conflictos.push(ruta),
                    Some(S::Added) => nuevos.push(ruta),
                    Some(S::Removed) => sin_preparar.push(crate::tr!("{ruta} (borrado)", "{ruta} (deleted)")),
                    Some(S::Renamed | S::Copied) => sin_preparar.push(crate::tr!("{ruta} (movido)", "{ruta} (moved)")),
                    Some(S::TypeChange) => sin_preparar.push(crate::tr!("{ruta} (cambió de tipo)", "{ruta} (type changed)")),
                    Some(S::Modified | S::IntentToAdd) => sin_preparar.push(ruta),
                    None => {}
                }
            }
            gix::status::Item::TreeIndex(c) => {
                use gix::diff::index::ChangeRef as C;
                let ruta = c.location().to_string();
                preparados.push(match c {
                    C::Addition { .. } => crate::tr!("{ruta} (nuevo)", "{ruta} (new)"),
                    C::Deletion { .. } => crate::tr!("{ruta} (borrado)", "{ruta} (deleted)"),
                    C::Rewrite { .. } => crate::tr!("{ruta} (movido)", "{ruta} (moved)"),
                    C::Modification { .. } => ruta,
                });
            }
        }
    }
    // los conflictos también salen como «sin preparar» de la etapa: se quitan de ahí
    sin_preparar.retain(|r| !conflictos.contains(r));
    let mut bloque = |titulo: &str, v: &mut Vec<String>| {
        if !v.is_empty() {
            v.sort();
            v.dedup();
            let mas = v.len().saturating_sub(50);
            let mut l = format!("{titulo} ({}): {}", v.len(), v.iter().take(50).cloned().collect::<Vec<_>>().join(", "));
            if mas > 0 {
                l.push_str(&crate::tr!(" … y {mas} más", " … and {mas} more"));
            }
            out.push(l);
        }
    };
    bloque(crate::i18n::t("CONFLICTOS", "CONFLICTS"), &mut conflictos);
    bloque(crate::i18n::t("preparados para el commit", "staged for commit"), &mut preparados);
    bloque(crate::i18n::t("cambiados sin preparar", "changed, not staged"), &mut sin_preparar);
    bloque(crate::i18n::t("nuevos sin seguir", "untracked"), &mut nuevos);
    if out.len() == 1 {
        out.push(crate::i18n::t("sin cambios: la carpeta coincide con el último commit", "no changes: the folder matches the last commit").into());
    }
    out.extend(advertencias(&repo, &base));
    Ok(out.join("\n"))
}

/// ¿Cambió `ruta` en este commit respecto a su primer padre? (o lo creó, si no tiene padre)
fn toca(repo: &gix::Repository, c: &gix::Commit<'_>, ruta: &str) -> bool {
    let id_en = |commit: &gix::Commit<'_>| commit.tree().ok().and_then(|t| t.lookup_entry_by_path(ruta).ok().flatten()).map(|e| e.object_id());
    let ahora = id_en(c);
    let antes = c.parent_ids().next().and_then(|p| repo.find_commit(p.detach()).ok()).map(|p| id_en(&p));
    match antes {
        Some(antes) => antes != ahora,
        None => ahora.is_some(),
    }
}

/// `historial`: los últimos `n` commits (de un archivo si se da), con fecha, autor y resumen.
pub fn historial(root: &Path, archivo: Option<&str>, n: usize) -> Result<String> {
    let repo = abrir(root)?;
    let Ok(head) = repo.head_id() else { return Ok(crate::i18n::t("(sin commits todavía)", "(no commits yet)").into()) };
    let ruta = archivo.map(|a| relativa(&repo, root, a)).transpose()?;
    let mut out = Vec::new();
    let mut vistos = 0usize;
    for info in repo.rev_walk([head.detach()]).all()? {
        let Ok(info) = info else { continue };
        vistos += 1;
        if vistos > 20_000 {
            break;
        }
        let Ok(c) = info.object() else { continue };
        if ruta.as_deref().is_some_and(|r| !toca(&repo, &c, r)) {
            continue;
        }
        out.push(linea_commit(&c));
        if out.len() >= n {
            break;
        }
    }
    if out.is_empty() {
        return Ok(crate::i18n::t("(ningún commit lo toca)", "(no commit touches it)").into());
    }
    Ok(out.join("\n"))
}

/// `autoria`: quién y en qué commit cambió cada línea de `desde..=hasta` (desde 1), con el código.
pub fn autoria(root: &Path, archivo: &str, desde: u32, hasta: u32) -> Result<String> {
    let repo = abrir(root)?;
    let ruta = relativa(&repo, root, archivo)?;
    let head = repo.head_id().map_err(|_| anyhow!("{}", crate::i18n::t("sin commits todavía", "no commits yet")))?.detach();
    let (desde, hasta) = (desde.max(1), hasta.max(desde.max(1)));
    let opciones = gix::repository::blame_file::Options { ranges: gix::blame::BlameRanges::from_one_based_inclusive_range(desde..=hasta).map_err(|e| anyhow!("{e}"))?, ..Default::default() };
    let r = repo.blame_file(ruta.as_str().into(), head, opciones).map_err(|e| anyhow!("{}", crate::tr!("no pude sacar la autoría de {ruta}: {e}", "could not blame {ruta}: {e}")))?;
    let texto = String::from_utf8_lossy(&r.blob).into_owned();
    let lineas: Vec<&str> = texto.lines().collect();
    let mut cache: BTreeMap<gix::ObjectId, String> = BTreeMap::new();
    let mut out = Vec::new();
    let mut mostradas = 0usize;
    for e in &r.entries {
        let ini = e.start_in_blamed_file as usize;
        let fin = ini + e.len.get() as usize;
        let cab = cache.entry(e.commit_id).or_insert_with(|| repo.find_commit(e.commit_id).map(|c| linea_commit(&c)).unwrap_or_else(|_| corto(&e.commit_id))).clone();
        out.push(format!("{}–{} · {cab}", ini + 1, fin));
        for (i, l) in lineas.iter().enumerate().take(fin).skip(ini) {
            if mostradas < 60 {
                out.push(format!("    {:>5}| {}", i + 1, l.chars().take(160).collect::<String>()));
            }
            mostradas += 1;
        }
    }
    if mostradas > 60 {
        out.push(crate::tr!("    … ({} líneas más)", "    … ({} more lines)", mostradas - 60));
    }
    Ok(out.join("\n"))
}

/// `ramas`: locales (la actual marcada) y remotas, con su último commit.
pub fn ramas(root: &Path) -> Result<String> {
    let repo = abrir(root)?;
    let actual = repo.head_name()?.map(|n| n.as_bstr().to_string());
    let mut out = Vec::new();
    let refs = repo.references()?;
    for (titulo, iter) in [(crate::i18n::t("locales", "local"), refs.local_branches()?), (crate::i18n::t("remotas", "remote"), refs.remote_branches()?)] {
        let mut v = Vec::new();
        for r in iter.flatten().take(60) {
            let nombre = r.name().as_bstr().to_string();
            let marca = if actual.as_deref() == Some(nombre.as_str()) { "* " } else { "  " };
            let mut r = r;
            let ultimo = r.peel_to_commit().map(|c| linea_commit(&c)).unwrap_or_default();
            v.push(format!("{marca}{} — {ultimo}", r.name().shorten()));
        }
        if !v.is_empty() {
            out.push(format!("{titulo}:\n{}", v.join("\n")));
        }
    }
    if out.is_empty() {
        return Ok(crate::i18n::t("(sin ramas todavía)", "(no branches yet)").into());
    }
    Ok(out.join("\n"))
}

/// Un conflicto: la ruta y sus tres lados (base, lo nuestro, lo de ellos); `None` si un lado no existe
/// (añadido en un solo lado, o borrado).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Conflicto {
    pub ruta: String,
    pub base: Option<String>,
    pub nuestro: Option<String>,
    pub suyo: Option<String>,
}

/// Los conflictos del índice con sus tres etapas.
pub fn conflictos_de(root: &Path) -> Result<Vec<Conflicto>> {
    let repo = abrir(root)?;
    let indice = repo.index_or_empty()?;
    let mut por_ruta: BTreeMap<String, Conflicto> = BTreeMap::new();
    for e in indice.entries() {
        let etapa = e.stage();
        if matches!(etapa, gix::index::entry::Stage::Unconflicted) {
            continue;
        }
        let ruta = e.path(&indice).to_string();
        let texto = repo.find_object(e.id).ok().map(|o| String::from_utf8_lossy(&o.data).into_owned());
        let c = por_ruta.entry(ruta.clone()).or_insert_with(|| Conflicto { ruta, ..Default::default() });
        match etapa {
            gix::index::entry::Stage::Base => c.base = texto,
            gix::index::entry::Stage::Ours => c.nuestro = texto,
            gix::index::entry::Stage::Theirs => c.suyo = texto,
            gix::index::entry::Stage::Unconflicted => {}
        }
    }
    Ok(por_ruta.into_values().collect())
}

/// `conflictos`: cada archivo con sus tres lados separados (60 líneas por lado como mucho).
pub fn conflictos(root: &Path) -> Result<String> {
    let v = conflictos_de(root)?;
    if v.is_empty() {
        return Ok(crate::i18n::t("sin conflictos", "no conflicts").into());
    }
    let lado = |t: &Option<String>| -> String {
        match t {
            None => crate::i18n::t("(no existe en este lado)", "(does not exist on this side)").into(),
            Some(t) => {
                let l: Vec<&str> = t.lines().collect();
                let mut s = l.iter().take(60).map(|x| format!("    {x}")).collect::<Vec<_>>().join("\n");
                if l.len() > 60 {
                    s.push_str(&crate::tr!("\n    … ({} líneas más)", "\n    … ({} more lines)", l.len() - 60));
                }
                s
            }
        }
    };
    let mut out = vec![crate::tr!(
        "{} archivo(s) en conflicto. Resuelve cada uno con action=resolver (archivo + contenido final, o lado=nuestro|suyo).",
        "{} file(s) in conflict. Resolve each with action=resolver (archivo + final contenido, or lado=nuestro|suyo).",
        v.len()
    )];
    for c in &v {
        out.push(format!(
            "── {} ──\n{}:\n{}\n{}:\n{}\n{}:\n{}",
            c.ruta,
            crate::i18n::t("BASE (el ancestro común)", "BASE (the common ancestor)"),
            lado(&c.base),
            crate::i18n::t("NUESTRO (la rama actual)", "OURS (the current branch)"),
            lado(&c.nuestro),
            crate::i18n::t("SUYO (lo que entra)", "THEIRS (what comes in)"),
            lado(&c.suyo)
        ));
    }
    Ok(out.join("\n"))
}
