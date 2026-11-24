//! C8 — Subagentes AISLADOS en su propia rama y carpeta (como `isolation: worktree` de Claude Code): el
//! subagente trabaja sin tocar el árbol del usuario; al terminar sus cambios quedan en un commit de esa rama y
//! el agente principal decide si entran. Sin cambios, la rama se borra.
//!
//! **En proceso (F9, 2026-11-18):** todo con gix, sin lanzar `git`. Antes cada rama costaba ~15 llamadas al
//! binario (`worktree add`, `add -A`, `commit`, `diff --stat`, `worktree remove`, `branch -D`: ~0,4 s). Ahora:
//! la rama es una referencia que apunta al HEAD, la carpeta se llena con el árbol de ese commit, y al cerrar se
//! arma el árbol nuevo desde la carpeta (respetando `.gitignore`, sin `.gotv/`) y se confirma en la rama.
//!
//! La carpeta va FUERA del repo (`~/.cache/gotv/worktrees/…`): dentro, un `git add -A` del agente principal la
//! metería como repositorio anidado.

use anyhow::{anyhow, Result};
use gix::bstr::{BStr, BString, ByteSlice};
use gix::object::tree::EntryKind;
use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

/// Bytes de git ↔ ruta de macOS (las rutas son bytes: no hay conversión que pueda fallar).
fn ruta(b: &[u8]) -> &Path {
    Path::new(OsStr::from_bytes(b))
}
fn bytes(p: &Path) -> BString {
    BString::from(p.as_os_str().as_bytes())
}
use std::sync::atomic::{AtomicU32, Ordering};

pub struct Worktree {
    /// donde trabaja el subagente (la MISMA subcarpeta del repo que el agente principal)
    pub path: PathBuf,
    pub branch: String,
    /// la raíz del repositorio del usuario
    repo: PathBuf,
    /// la raíz de la carpeta de la rama (lo que se borra al cerrar)
    carpeta: PathBuf,
    /// el commit del que salió
    base: gix::ObjectId,
}

fn slug(s: &str) -> String {
    let t: String = s.to_lowercase().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
    t.split('-').filter(|p| !p.is_empty()).take(5).collect::<Vec<_>>().join("-")
}

/// El repo, con GOTV como autor de lo que escriba (en memoria: la configuración del repo no se toca; es lo
/// mismo que `git -c user.name=… -c user.email=…`, y el reflog lo exige).
pub(crate) fn abrir(dir: &Path) -> Result<gix::Repository> {
    let mut repo = gix::discover(dir).map_err(|_| {
        anyhow!("{}", crate::i18n::t(
            "aislar un subagente necesita un repo git con al menos un commit (o lánzalo sin aislar)",
            "isolating a subagent needs a git repo with at least one commit (or launch it without isolation)",
        ))
    })?;
    {
        use gix::config::tree::{Committer, User};
        let mut c = repo.config_snapshot_mut();
        c.set_value(&User::NAME, "GOD OF THE VOID")?;
        c.set_value(&User::EMAIL, "gotv@localhost")?;
        c.set_value(&Committer::NAME, "GOD OF THE VOID")?;
        c.set_value(&Committer::EMAIL, "gotv@localhost")?;
    }
    Ok(repo)
}

/// Crea la rama `gotv/sub-N-<tema>` y su carpeta, desde el HEAD del proyecto.
pub fn create(root: &Path, label: &str) -> Result<Worktree> {
    let repo = abrir(root)?;
    let head = repo.head_commit().map_err(|_| {
        anyhow!("{}", crate::i18n::t(
            "aislar un subagente necesita un repo git con al menos un commit (o lánzalo sin aislar)",
            "isolating a subagent needs a git repo with at least one commit (or launch it without isolation)",
        ))
    })?;
    let top = repo.workdir().ok_or_else(|| anyhow!("{}", crate::i18n::t("el repositorio no tiene carpeta de trabajo", "the repository has no working folder")))?.to_path_buf();
    let top = top.canonicalize().unwrap_or(top);
    static N: AtomicU32 = AtomicU32::new(1);
    let n = N.fetch_add(1, Ordering::Relaxed);
    let tema = slug(label);
    let branch = format!("gotv/sub-{}-{n}{}", std::process::id(), if tema.is_empty() { String::new() } else { format!("-{tema}") });
    let name = top.file_name().and_then(|s| s.to_str()).unwrap_or("proyecto").to_string();
    let base_dir = crate::paths::home().join(".cache/gotv/worktrees").join(&name);
    let carpeta = base_dir.join(branch.replace('/', "-"));
    std::fs::create_dir_all(&carpeta)?;
    repo.reference(format!("refs/heads/{branch}"), head.id, gix::refs::transaction::PreviousValue::MustNotExist, format!("gotv: rama de un subagente ({label})"))?;
    if let Err(e) = volcar(&repo, head.tree_id()?.detach(), &carpeta) {
        let _ = std::fs::remove_dir_all(&carpeta);
        let _ = borrar_rama(&repo, &branch);
        return Err(e);
    }
    let root_c = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let rel = root_c.strip_prefix(&top).unwrap_or(Path::new(""));
    Ok(Worktree { path: carpeta.join(rel), branch, repo: top, carpeta, base: head.id })
}

/// Escribe el árbol `tree` en `dir` (archivos, ejecutables, enlaces; un submódulo queda como carpeta vacía).
fn volcar(repo: &gix::Repository, tree: gix::ObjectId, dir: &Path) -> Result<()> {
    let t = repo.find_tree(tree)?;
    for e in t.iter() {
        let e = e?;
        let destino = dir.join(ruta(e.filename()));
        match e.mode().kind() {
            EntryKind::Tree => {
                std::fs::create_dir_all(&destino)?;
                volcar(repo, e.oid().to_owned(), &destino)?;
            }
            EntryKind::Blob | EntryKind::BlobExecutable => {
                let b = repo.find_blob(e.oid())?;
                std::fs::write(&destino, &b.data)?;
                if e.mode().kind() == EntryKind::BlobExecutable {
                    use std::os::unix::fs::PermissionsExt;
                    std::fs::set_permissions(&destino, std::fs::Permissions::from_mode(0o755))?;
                }
            }
            EntryKind::Link => {
                let b = repo.find_blob(e.oid())?;
                std::os::unix::fs::symlink(ruta(&b.data), &destino)?;
            }
            EntryKind::Commit => std::fs::create_dir_all(&destino)?,
        }
    }
    Ok(())
}

/// Todas las entradas de un árbol (ruta → id y tipo), con las subcarpetas recorridas.
pub(crate) fn listado(repo: &gix::Repository, tree: gix::ObjectId) -> Result<BTreeMap<BString, (gix::ObjectId, EntryKind)>> {
    let mut rec = gix::traverse::tree::Recorder::default();
    repo.find_tree(tree)?.traverse().breadthfirst(&mut rec)?;
    Ok(rec.records.into_iter().filter(|e| e.mode.kind() != EntryKind::Tree).map(|e| (e.filepath, (e.oid, e.mode.kind()))).collect())
}

/// El árbol de lo que hay en `carpeta`, partiendo de `base`: lo que no ignora `.gitignore` entra (y lo que se
/// borró sale); `.gotv/` nunca (salidas largas, cachés: si viajaran, llegarían al usuario como «cambios»).
fn arbol_de(repo: &gix::Repository, carpeta: &Path, base: gix::ObjectId) -> Result<gix::ObjectId> {
    let antes = listado(repo, base)?;
    let mut ed = repo.edit_tree(base)?;
    let mut vistos = std::collections::BTreeSet::new();
    let walk = ignore::WalkBuilder::new(carpeta)
        .hidden(false)
        .require_git(false)
        .git_global(false)
        .filter_entry(|e| !matches!(e.file_name().to_str(), Some(".gotv" | ".git")))
        .build();
    for ent in walk.flatten() {
        let p = ent.path();
        let Ok(rel) = p.strip_prefix(carpeta) else { continue };
        if rel.as_os_str().is_empty() {
            continue;
        }
        let meta = std::fs::symlink_metadata(p)?;
        let ft = meta.file_type();
        if ft.is_dir() {
            continue;
        }
        let rel_b = bytes(rel);
        let (kind, datos) = if ft.is_symlink() {
            (EntryKind::Link, bytes(&std::fs::read_link(p)?).to_vec())
        } else {
            use std::os::unix::fs::PermissionsExt;
            (if meta.permissions().mode() & 0o111 != 0 { EntryKind::BlobExecutable } else { EntryKind::Blob }, std::fs::read(p)?)
        };
        let id = repo.write_blob(&datos)?.detach();
        if antes.get(&rel_b) != Some(&(id, kind)) {
            ed.upsert(rel_b.as_bstr(), kind, id)?;
        }
        vistos.insert(rel_b);
    }
    // lo que estaba y ya no está en la carpeta (lo ignorado que se versiona sigue en disco: se queda)
    for ruta in antes.keys() {
        if !vistos.contains(ruta) && !carpeta.join(self::ruta(ruta)).exists() {
            ed.remove(ruta.as_bstr())?;
        }
    }
    Ok(ed.write()?.detach())
}

/// Lo que cambió entre dos árboles: `+ nuevo`, `~ cambiado`, `- borrado`.
pub(crate) fn resumen(repo: &gix::Repository, viejo: gix::ObjectId, nuevo: gix::ObjectId) -> Result<Vec<String>> {
    let (a, b) = (listado(repo, viejo)?, listado(repo, nuevo)?);
    let mut v = Vec::new();
    for (p, x) in &b {
        match a.get(p) {
            None => v.push(format!("+ {p}")),
            Some(y) if y != x => v.push(format!("~ {p}")),
            _ => {}
        }
    }
    v.extend(a.keys().filter(|p| !b.contains_key(*p)).map(|p| format!("- {p}")));
    v.sort_by(|x, y| x[2..].cmp(&y[2..]));
    Ok(v)
}

pub(crate) fn borrar_rama(repo: &gix::Repository, rama: &str) -> Result<()> {
    repo.find_reference(format!("refs/heads/{rama}").as_str())?.delete()?;
    Ok(())
}

/// La firma de los commits de GOTV (hora en formato crudo de git: segundos y zona).
pub(crate) fn con_firma<T>(f: impl FnOnce(gix::actor::SignatureRef<'_>) -> T) -> T {
    let segundos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let hora = format!("{segundos} +0000");
    f(gix::actor::SignatureRef { name: BStr::new("GOD OF THE VOID"), email: BStr::new("gotv@localhost"), time: &hora })
}

impl Worktree {
    /// Cierra la rama: commit de lo hecho y la carpeta se quita. Devuelve el resumen para el agente principal.
    pub fn finish(&self, message: &str) -> Result<String> {
        let repo = abrir(&self.repo)?;
        let base_tree = repo.find_commit(self.base)?.tree_id()?.detach();
        let nuevo = arbol_de(&repo, &self.carpeta, base_tree)?;
        let changed = nuevo != base_tree;
        let summary = if changed {
            con_firma(|f| repo.commit_as(f, f, format!("refs/heads/{}", self.branch), message, nuevo, [self.base]))?;
            let stat = resumen(&repo, base_tree, nuevo)?.join("\n");
            crate::tr!(
                "Sus cambios quedaron en la rama `{}` (tu árbol NO cambió):\n{stat}\nPara traerlos: bash `git merge {}`; para descartarlos: `git branch -D {}`.",
                "Its changes are on branch `{}` (your tree did NOT change):\n{stat}\nTo bring them in: bash `git merge {}`; to discard them: `git branch -D {}`.",
                self.branch,
                self.branch,
                self.branch
            )
        } else {
            crate::i18n::t("No cambió ningún archivo (la rama se borró).", "No file changed (the branch was deleted).").to_string()
        };
        std::fs::remove_dir_all(&self.carpeta)?;
        if !changed {
            let _ = borrar_rama(&repo, &self.branch);
        }
        Ok(summary)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn git(d: &Path, a: &[&str]) -> std::process::Output {
        std::process::Command::new("git").arg("-C").arg(d).args(["-c", "user.name=t", "-c", "user.email=t@t"]).args(a).output().unwrap()
    }

    fn repo(n: &str) -> crate::CarpetaPrueba {
        let d = crate::CarpetaPrueba::sin_crear(&format!("gotv-wt-{n}"));
        std::fs::create_dir_all(&d).unwrap();
        git(&d, &["init", "-q"]);
        git(&d, &["commit", "-q", "--allow-empty", "-m", "base"]);
        std::fs::write(d.join("a.txt"), "uno").unwrap();
        git(&d, &["add", "-A"]);
        git(&d, &["commit", "-q", "-m", "a"]);
        d
    }

    #[test]
    fn los_cambios_quedan_en_su_rama_y_el_arbol_no_cambia() {
        let d = repo("cambia");
        let wt = create(&d, "probar otro enfoque").unwrap();
        assert!(wt.path.join("a.txt").exists() && !wt.path.starts_with(&d), "fuera del repo");
        std::fs::write(wt.path.join("a.txt"), "dos").unwrap();
        std::fs::write(wt.path.join("b.txt"), "nuevo").unwrap();
        let r = finish_ok(&wt);
        assert!(r.contains(&wt.branch) && r.contains("b.txt"), "{r}");
        assert_eq!(std::fs::read_to_string(d.join("a.txt")).unwrap(), "uno", "el árbol del usuario no cambió");
        assert!(!d.join("b.txt").exists());
        assert!(!wt.path.exists(), "la carpeta se quitó");
        // y se puede fusionar (el commit de gix es un commit de git de verdad)
        assert!(git(&d, &["merge", "-q", &wt.branch]).status.success());
        assert_eq!(std::fs::read_to_string(d.join("a.txt")).unwrap(), "dos");
    }

    #[test]
    fn sin_cambios_la_rama_se_borra_y_sin_git_se_explica() {
        let d = repo("igual");
        let wt = create(&d, "mirar").unwrap();
        let r = finish_ok(&wt);
        assert!(r.contains("No cambió") || r.contains("No file changed"), "{r}");
        assert!(git(&d, &["branch", "--list", &wt.branch]).stdout.is_empty(), "rama borrada");
        let sin = crate::CarpetaPrueba::sin_crear("gotv-wt-sin-git");
        std::fs::create_dir_all(&sin).unwrap();
        assert!(create(&sin, "x").is_err());
    }

    /// En proceso: un borrado sale de la rama; lo de `.gitignore` y `.gotv/` no entra; el bit de ejecución
    /// viaja; y una subcarpeta del repo se respeta (el subagente trabaja donde el principal).
    #[test]
    fn borrados_ignorados_gotv_y_ejecutables() {
        let d = repo("detalles");
        std::fs::write(d.join(".gitignore"), "target/\n").unwrap();
        std::fs::create_dir_all(d.join("web")).unwrap();
        std::fs::write(d.join("web/x.sh"), "echo hola\n").unwrap();
        git(&d, &["add", "-A"]);
        git(&d, &["commit", "-q", "-m", "mas"]);
        let wt = create(&d.join("web"), "detalles").unwrap();
        assert!(wt.path.ends_with("web") && wt.path.join("x.sh").is_file(), "{}", wt.path.display());
        let raiz = wt.path.parent().unwrap().to_path_buf();
        std::fs::remove_file(raiz.join("a.txt")).unwrap();
        std::fs::create_dir_all(raiz.join("target")).unwrap();
        std::fs::write(raiz.join("target/basura.o"), "x").unwrap();
        std::fs::create_dir_all(wt.path.join(".gotv")).unwrap();
        std::fs::write(wt.path.join(".gotv/cache"), "x").unwrap();
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(wt.path.join("x.sh"), std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let r = finish_ok(&wt);
        assert!(r.contains("- a.txt") && r.contains("~ web/x.sh") && !r.contains("target") && !r.contains(".gotv"), "{r}");
        assert!(git(&d, &["merge", "-q", &wt.branch]).status.success());
        assert!(!d.join("a.txt").exists());
        let modo = String::from_utf8_lossy(&git(&d, &["ls-files", "-s", "web/x.sh"]).stdout).to_string();
        assert!(modo.starts_with("100755"), "{modo}");
    }

    fn finish_ok(wt: &Worktree) -> String {
        wt.finish("gotv: prueba").unwrap()
    }
}
