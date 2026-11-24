//! F9 · lo que hizo cada subagente en su rama, de vuelta al proyecto.
//!
//! Dos formas, con la MISMA regla: lo que no choca entra; lo que choca se reporta con sus archivos y su rama
//! se queda tal cual (no se pierde nada, no queda una fusión a medias).
//! - [`al_arbol`]: los cambios de la rama pasan al árbol del usuario SIN commit (como cuando un subagente
//!   escribía directo en el árbol). Lo usa el reparto en paralelo: así el agente principal y el usuario los
//!   ven como cambios suyos, y el punto de restauración del turno los puede deshacer. En proceso (gix).
//! - [`fusionar`]: `git merge --no-ff` con su commit de fusión (la acción `fusionar` de la herramienta).
//!   El choque se detecta ANTES con `git merge-tree --write-tree`, que no toca ni el árbol ni el índice.

use anyhow::{bail, Result};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

/// Lo que pasó con una rama.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fusion {
    /// Entró: estos archivos cambiaron.
    Hecha { rama: String, archivos: Vec<String> },
    /// No entró: choca con lo que ya hay en estos archivos. La rama sigue ahí.
    Choca { rama: String, archivos: Vec<String>, detalle: String },
    /// La rama no trae nada nuevo.
    SinCambios { rama: String },
    /// No se pudo ni intentar (p. ej. la rama no existe): NO es un choque (medido 2026-11-18: «CHOCA en ␣» con
    /// una rama inexistente).
    NoSeTrajo { rama: String, motivo: String },
}

impl Fusion {
    pub fn rama(&self) -> &str {
        match self {
            Fusion::Hecha { rama, .. } | Fusion::Choca { rama, .. } | Fusion::SinCambios { rama } | Fusion::NoSeTrajo { rama, .. } => rama,
        }
    }
}

/// `git` con su salida y su código (el de `git.rs` convierte todo código ≠ 0 en error, y aquí el 1 de
/// `merge-tree` es la respuesta «choca»).
fn git(dir: &Path, args: &[&str]) -> Result<(bool, String, String)> {
    let o = crate::entorno::comando("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_EDITOR", "true")
        .env("GIT_PAGER", "cat")
        .stdin(std::process::Stdio::null())
        .output()?;
    Ok((o.status.success(), String::from_utf8_lossy(&o.stdout).trim().to_string(), String::from_utf8_lossy(&o.stderr).trim().to_string()))
}

fn git_ok(dir: &Path, args: &[&str]) -> Result<String> {
    let (ok, out, err) = git(dir, args)?;
    if !ok {
        bail!("git {}: {err}", args.join(" "));
    }
    Ok(out)
}

fn lineas(s: &str) -> Vec<String> {
    s.lines().map(str::trim).filter(|l| !l.is_empty()).map(String::from).collect()
}

/// La raíz del repositorio de `dir` (gix, en proceso).
pub fn raiz_git(dir: &Path) -> Option<std::path::PathBuf> {
    gix::discover(dir).ok().and_then(|r| r.workdir().map(|w| w.canonicalize().unwrap_or_else(|_| w.to_path_buf())))
}

/// ¿`dir` está en un repo git con al menos un commit? (sin eso no hay ramas que aislar)
pub fn hay_repo(dir: &Path) -> bool {
    gix::discover(dir).ok().is_some_and(|r| r.head_id().is_ok())
}

/// ¿Existe la rama? (`Worktree::finish` borra la de un subagente que no cambió nada)
pub fn existe(dir: &Path, rama: &str) -> bool {
    gix::discover(dir).ok().is_some_and(|r| r.find_reference(format!("refs/heads/{rama}").as_str()).is_ok())
}

/// Los cambios de `rama` (desde donde se separó de HEAD) al árbol de `raiz`, SIN commit, en proceso (gix; antes
/// `merge-base` + `diff --binary` + `git apply`). Archivo por archivo: si el del usuario sigue como en la base,
/// entra el de la rama; si ya es igual al de la rama, nada; si los dos cambiaron, fusión de texto a tres vías
/// (lo que hacía `git apply` por trozos). Todo o nada: si algo choca, no se escribe ningún archivo y la rama se
/// queda; si entra, la rama (de GOTV: `gotv/sub-…`) se borra.
pub fn al_arbol(raiz: &Path, rama: &str) -> Result<Fusion> {
    use gix::object::tree::EntryKind;
    let repo = crate::worktree::abrir(raiz)?;
    let top = raiz_git(raiz).unwrap_or_else(|| raiz.to_path_buf());
    let head = repo.head_id()?.detach();
    let punta = repo.find_reference(format!("refs/heads/{rama}").as_str())?.peel_to_id()?.detach();
    let base = repo.merge_base(head, punta)?.ok_or_else(|| anyhow::anyhow!("{}", crate::tr!("{rama} no comparte historia con HEAD", "{rama} shares no history with HEAD")))?.detach();
    let arbol = |c: gix::ObjectId| -> Result<gix::ObjectId> { Ok(repo.find_commit(c)?.tree_id()?.detach()) };
    let (a, b) = (crate::worktree::listado(&repo, arbol(base)?)?, crate::worktree::listado(&repo, arbol(punta)?)?);
    let mut rutas: Vec<&gix::bstr::BString> = b.iter().filter(|(p, x)| a.get(*p) != Some(x)).map(|(p, _)| p).collect();
    rutas.extend(a.keys().filter(|p| !b.contains_key(*p)));
    rutas.sort();
    let archivos: Vec<String> = rutas.iter().map(|p| p.to_string()).collect();
    if rutas.is_empty() {
        let _ = crate::worktree::borrar_rama(&repo, rama);
        return Ok(Fusion::SinCambios { rama: rama.into() });
    }
    let contenido = |e: Option<&(gix::ObjectId, EntryKind)>| -> Result<Option<Vec<u8>>> { e.map(|(id, _)| Ok(repo.find_blob(*id)?.data.clone())).transpose() };
    // (ruta en disco, contenido nuevo o None = borrar, ¿ejecutable?)
    let mut escribir: Vec<(std::path::PathBuf, Option<Vec<u8>>, bool)> = Vec::new();
    let mut chocan = Vec::new();
    for r in &rutas {
        let en_disco = top.join(std::ffi::OsStr::from_bytes(r.as_slice()));
        let (de_base, de_rama) = (contenido(a.get(*r))?, contenido(b.get(*r))?);
        let actual = std::fs::read(&en_disco).ok();
        let exe = b.get(*r).is_some_and(|(_, k)| *k == EntryKind::BlobExecutable);
        if actual == de_base {
            escribir.push((en_disco, de_rama, exe));
        } else if actual == de_rama {
            continue;
        } else if let (Some(act), Some(bas), Some(ram)) = (&actual, &de_base, &de_rama) {
            let mut fuera = Vec::new();
            let mut entrada = gix::diff::blob::InternedInput::new(&[][..], &[][..]);
            let r3 = gix::merge::blob::builtin_driver::text(&mut fuera, &mut entrada, Default::default(), act, bas, ram, Default::default());
            if r3 == gix::merge::blob::Resolution::Conflict {
                chocan.push(r.to_string());
            } else {
                escribir.push((en_disco, Some(fuera), exe));
            }
        } else {
            // borrado de un lado y cambiado del otro
            chocan.push(r.to_string());
        }
    }
    if !chocan.is_empty() {
        let detalle = crate::tr!("los dos cambiaron las mismas líneas de: {}", "both changed the same lines of: {}", chocan.join(", "));
        return Ok(Fusion::Choca { rama: rama.into(), archivos: chocan, detalle });
    }
    for (p, datos, exe) in escribir {
        match datos {
            Some(d) => {
                if let Some(dir) = p.parent() {
                    std::fs::create_dir_all(dir)?;
                }
                std::fs::write(&p, d)?;
                if exe {
                    use std::os::unix::fs::PermissionsExt;
                    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755))?;
                }
            }
            None => {
                let _ = std::fs::remove_file(&p);
            }
        }
    }
    let _ = crate::worktree::borrar_rama(&repo, rama);
    Ok(Fusion::Hecha { rama: rama.into(), archivos })
}

/// `git merge --no-ff` de `rama` en el HEAD de `raiz`, con su commit. Choque → nada cambia y se reporta.
pub fn fusionar(raiz: &Path, rama: &str, mensaje: &str) -> Result<Fusion> {
    let top = raiz_git(raiz).unwrap_or_else(|| raiz.to_path_buf());
    if !git(&top, &["rev-parse", "-q", "--verify", rama])?.0 {
        return Ok(Fusion::NoSeTrajo {
            rama: rama.into(),
            motivo: crate::tr!(
                "no existe esa rama (sin ramas aisladas —p. ej. subagentes por turnos con un modelo local— sus cambios ya están en tu árbol)",
                "that branch does not exist (without isolated branches —e.g. subagents taking turns with a local model— their changes are already in your tree)"
            ),
        });
    }
    if git(&top, &["merge-base", "--is-ancestor", rama, "HEAD"])?.0 {
        return Ok(Fusion::SinCambios { rama: rama.into() });
    }
    let base = git_ok(&top, &["merge-base", "HEAD", rama])?;
    let archivos = lineas(&git_ok(&top, &["diff", "--name-only", &base, rama])?);
    // 0 = se fusiona limpio; 1 = choca (la salida: el árbol y, con --name-only, los archivos en conflicto)
    let (limpio, salida, err) = git(&top, &["merge-tree", "--write-tree", "--name-only", "HEAD", rama])?;
    if !limpio {
        let chocan: Vec<String> = salida.lines().skip(1).take_while(|l| !l.trim().is_empty()).map(|l| l.trim().to_string()).collect();
        let detalle = if err.is_empty() { salida.lines().skip(1).skip_while(|l| !l.trim().is_empty()).collect::<Vec<_>>().join("\n").trim().to_string() } else { err };
        return Ok(Fusion::Choca { rama: rama.into(), archivos: if chocan.is_empty() { archivos } else { chocan }, detalle });
    }
    let (ok, _, err) = git(&top, &["-c", "user.name=GOD OF THE VOID", "-c", "user.email=gotv@localhost", "merge", "--no-ff", "--no-edit", "-m", mensaje, rama])?;
    if !ok {
        // p. ej. el usuario tiene cambios sin guardar en esos archivos: se deja todo como estaba
        if top.join(".git/MERGE_HEAD").exists() || git(&top, &["rev-parse", "-q", "--verify", "MERGE_HEAD"])?.0 {
            let _ = git(&top, &["merge", "--abort"]);
        }
        return Ok(Fusion::Choca { rama: rama.into(), archivos, detalle: err });
    }
    Ok(Fusion::Hecha { rama: rama.into(), archivos })
}

/// El informe para el agente principal (y el usuario).
pub fn informe(fusiones: &[Fusion]) -> String {
    let mut s = String::new();
    for f in fusiones {
        let linea = match f {
            Fusion::Hecha { rama, archivos } => crate::tr!("✓ {rama}: entró ({})", "✓ {rama}: merged in ({})", archivos.join(", ")),
            Fusion::SinCambios { rama } => crate::tr!("· {rama}: sin cambios", "· {rama}: no changes"),
            Fusion::NoSeTrajo { rama, motivo } => crate::tr!("✗ {rama}: no se trajo — {motivo}", "✗ {rama}: not brought in — {motivo}"),
            Fusion::Choca { rama, archivos, detalle } => {
                let d = detalle.lines().next().unwrap_or("").trim();
                crate::tr!(
                    "✗ {rama}: CHOCA en {} — no se fusionó; la rama queda para revisarla (`git diff HEAD...{rama}`){}",
                    "✗ {rama}: CONFLICTS in {} — not merged; the branch stays for review (`git diff HEAD...{rama}`){}",
                    archivos.join(", "),
                    if d.is_empty() { String::new() } else { format!(" · {d}") }
                )
            }
        };
        s.push_str(&linea);
        s.push('\n');
    }
    s.trim_end().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn sh(d: &Path, args: &[&str]) -> String {
        let o = Command::new("git").arg("-C").arg(d).args(args).output().unwrap();
        assert!(o.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&o.stderr));
        String::from_utf8_lossy(&o.stdout).trim().to_string()
    }

    fn repo(n: &str) -> crate::CarpetaPrueba {
        let d = crate::CarpetaPrueba::nueva(&format!("gotv-ramas-{n}"));
        sh(&d, &["init", "-q", "-b", "main"]);
        std::fs::write(d.join("a.txt"), "uno\ndos\ntres\n").unwrap();
        std::fs::write(d.join("b.txt"), "b\n").unwrap();
        sh(&d, &["add", "-A"]);
        sh(&d, &["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "-m", "base"]);
        d
    }

    /// Una rama de GOTV (por `worktree.rs`, como la crea un subagente) que escribe `archivo`.
    fn rama_que_escribe(d: &Path, tema: &str, archivo: &str, texto: &str) -> String {
        let wt = crate::worktree::create(d, tema).unwrap();
        std::fs::write(wt.path.join(archivo), texto).unwrap();
        wt.finish(&format!("gotv: {tema}")).unwrap();
        wt.branch.clone()
    }

    fn existe(d: &Path, rama: &str) -> bool {
        !sh(d, &["branch", "--list", rama]).is_empty()
    }

    /// Una rama que no existe no es un «choque»: se dice que no existe (y por qué puede pasar).
    #[test]
    fn una_rama_que_no_existe_no_es_un_choque() {
        let d = repo("no-existe");
        let f = fusionar(&d, "validacion-usuarios", "x").unwrap();
        assert!(matches!(&f, Fusion::NoSeTrajo { .. }), "{f:?}");
        let i = informe(&[f]);
        assert!((i.contains("no existe") || i.contains("does not exist")) && !i.contains("CHOCA") && !i.contains("CONFLICTS"), "{i}");
    }

    #[test]
    fn dos_ramas_en_archivos_distintos_entran_al_arbol_y_dos_que_chocan_no() {
        let d = repo("arbol");
        let r1 = rama_que_escribe(&d, "uno", "a.txt", "UNO\ndos\ntres\n");
        let r2 = rama_que_escribe(&d, "dos", "c.txt", "nuevo\n");
        let r3 = rama_que_escribe(&d, "tres", "a.txt", "otro\ndos\ntres\n");
        let f1 = al_arbol(&d, &r1).unwrap();
        let f2 = al_arbol(&d, &r2).unwrap();
        assert!(matches!(&f1, Fusion::Hecha { archivos, .. } if archivos == &vec!["a.txt".to_string()]), "{f1:?}");
        assert!(matches!(&f2, Fusion::Hecha { .. }), "{f2:?}");
        // la tercera toca la misma línea que la primera: choca, nada cambia y su rama queda
        let f3 = al_arbol(&d, &r3).unwrap();
        assert!(matches!(&f3, Fusion::Choca { archivos, .. } if archivos == &vec!["a.txt".to_string()]), "{f3:?}");
        assert_eq!(std::fs::read_to_string(d.join("a.txt")).unwrap(), "UNO\ndos\ntres\n");
        assert_eq!(std::fs::read_to_string(d.join("c.txt")).unwrap(), "nuevo\n");
        // sin commit: son cambios del árbol; las ramas que entraron se borran, la que chocó no
        assert_eq!(sh(&d, &["rev-list", "--count", "HEAD"]), "1");
        assert!(!existe(&d, &r1) && !existe(&d, &r2) && existe(&d, &r3));
        let inf = informe(&[f1, f2, f3]);
        assert!(inf.contains("✓") && inf.contains("✗") && inf.contains(&r3), "{inf}");
    }

    /// En proceso (gix): dos subagentes que tocan líneas DISTINTAS del mismo archivo entran los dos (lo que hacía
    /// `git apply` por trozos); uno que borra un archivo que otro ya cambió choca; y un cambio del USUARIO en el
    /// árbol, en otra parte del archivo, se conserva.
    #[test]
    fn mismo_archivo_lineas_distintas_entran_las_dos() {
        let d = repo("trozos");
        std::fs::write(d.join("a.txt"), "uno\ndos\ntres\ncuatro\ncinco\nseis\nsiete\n").unwrap();
        sh(&d, &["add", "-A"]);
        sh(&d, &["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "-m", "siete"]);
        let r1 = rama_que_escribe(&d, "arriba", "a.txt", "UNO\ndos\ntres\ncuatro\ncinco\nseis\nsiete\n");
        let r2 = rama_que_escribe(&d, "abajo", "a.txt", "uno\ndos\ntres\ncuatro\ncinco\nseis\nSIETE\n");
        // el usuario también tocó la línea del medio, sin confirmar
        std::fs::write(d.join("a.txt"), "uno\ndos\ntres\nCUATRO\ncinco\nseis\nsiete\n").unwrap();
        assert!(matches!(al_arbol(&d, &r1).unwrap(), Fusion::Hecha { .. }));
        assert!(matches!(al_arbol(&d, &r2).unwrap(), Fusion::Hecha { .. }));
        assert_eq!(std::fs::read_to_string(d.join("a.txt")).unwrap(), "UNO\ndos\ntres\nCUATRO\ncinco\nseis\nSIETE\n");
        // borrar lo que otro cambió: choca y no toca nada
        let wt = crate::worktree::create(&d, "borra").unwrap();
        std::fs::remove_file(wt.path.join("a.txt")).unwrap();
        wt.finish("gotv: borra").unwrap();
        assert!(matches!(al_arbol(&d, &wt.branch).unwrap(), Fusion::Choca { .. }));
        assert!(d.join("a.txt").is_file());
    }

    /// Medida (F9, ramas en proceso): el ciclo de una rama (crear + escribir + cerrar + traer al árbol) en un repo
    /// de 300 archivos. Antes ~0,35–0,5 s por ~15 llamadas a `git` (entrega de F9).
    /// `cargo test -p gotv-core --release --lib -- --ignored --nocapture medir_el_ciclo_de_una_rama`
    #[test]
    #[ignore]
    fn medir_el_ciclo_de_una_rama() {
        let d = repo("medir");
        for i in 0..300 {
            std::fs::create_dir_all(d.join(format!("src/m{}", i / 30))).unwrap();
            std::fs::write(d.join(format!("src/m{}/a{i}.rs", i / 30)), format!("pub fn f{i}() -> u32 {{ {i} }}\n").repeat(20)).unwrap();
        }
        sh(&d, &["add", "-A"]);
        sh(&d, &["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "-m", "300"]);
        let mut ms = Vec::new();
        for i in 0..10 {
            let t = std::time::Instant::now();
            let r = rama_que_escribe(&d, &format!("r{i}"), &format!("nuevo{i}.txt"), "x\n");
            assert!(matches!(al_arbol(&d, &r).unwrap(), Fusion::Hecha { .. }));
            ms.push(t.elapsed().as_secs_f64() * 1000.0);
        }
        ms.sort_by(|a, b| a.partial_cmp(b).unwrap());
        eprintln!("ciclo de una rama (300 archivos): mediana {:.0} ms · mín {:.0} · máx {:.0}", ms[5], ms[0], ms[9]);
    }

    #[test]
    fn fusionar_con_commit_y_el_choque_se_reporta_sin_fusion_a_medias() {
        let d = repo("merge");
        let r1 = rama_que_escribe(&d, "uno", "a.txt", "UNO\ndos\ntres\n");
        let r2 = rama_que_escribe(&d, "dos", "b.txt", "B\n");
        let r3 = rama_que_escribe(&d, "tres", "a.txt", "otro\ndos\ntres\n");
        assert!(matches!(fusionar(&d, &r1, "gotv: uno").unwrap(), Fusion::Hecha { .. }));
        assert!(matches!(fusionar(&d, &r2, "gotv: dos").unwrap(), Fusion::Hecha { .. }));
        let antes = sh(&d, &["rev-parse", "HEAD"]);
        let f3 = fusionar(&d, &r3, "gotv: tres").unwrap();
        assert!(matches!(&f3, Fusion::Choca { archivos, .. } if archivos.contains(&"a.txt".to_string())), "{f3:?}");
        // nada a medias: mismo HEAD, sin MERGE_HEAD, árbol limpio
        assert_eq!(sh(&d, &["rev-parse", "HEAD"]), antes);
        assert!(!d.join(".git/MERGE_HEAD").exists());
        assert!(sh(&d, &["status", "--porcelain"]).is_empty());
        assert_eq!(std::fs::read_to_string(d.join("b.txt")).unwrap(), "B\n");
        // dos commits de fusión
        assert_eq!(sh(&d, &["rev-list", "--merges", "--count", "HEAD"]), "2");
        // fusionar otra vez lo ya fusionado no hace nada
        assert!(matches!(fusionar(&d, &r1, "x").unwrap(), Fusion::SinCambios { .. }));
    }
}
