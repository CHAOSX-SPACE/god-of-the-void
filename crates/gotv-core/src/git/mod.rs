//! F17 · git como herramienta propia. Las LECTURAS van dentro del proceso con `gix` (`leer.rs`); las
//! ESCRITURAS con el binario `git` del sistema (nivel 2), para respetar los hooks, la firma de commits y la
//! configuración del usuario: nunca se reimplementa un commit ni un rebase.
//!
//! Frenos: `confirmar` jamás usa `-a` ni `add -A`; `preparar` sólo acepta archivos concretos y lo que el
//! agente no tocó (regla L7) necesita `incluir_ajenos`, que el usuario aprueba siempre. `empujar` pregunta
//! siempre (también en bypass) y con `forzar` sólo usa `--force-with-lease`. Los destructivos por `bash`
//! (`reset --hard`, `clean -f`, `push --force`, `branch -D`) ya los niega `permissions.rs`, también en bypass.
pub mod leer;

use anyhow::{anyhow, bail, Result};
use serde_json::Value;
use std::collections::BTreeSet;
use std::path::Path;
use std::time::Duration;

/// Lo que la herramienta necesita del agente.
pub struct Contexto<'a> {
    pub root: &'a Path,
    /// las rutas (relativas a la raíz del proyecto) que cambió el agente
    pub del_agente: BTreeSet<String>,
}

/// Corre `git` (nivel 2) en la carpeta del repositorio, sin entrada (un hook que pregunta no cuelga), sin
/// pedir credenciales por terminal, con tope. Devuelve (éxito, salida).
pub async fn git(carpeta: &Path, args: &[&str], tope: Duration) -> Result<(bool, String)> {
    let mut k = crate::entorno::comando_tokio("git");
    k.args(args)
        .current_dir(carpeta)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_EDITOR", "true")
        .env("GIT_PAGER", "cat")
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true);
    let o = tokio::time::timeout(tope, k.output())
        .await
        .map_err(|_| anyhow!("{}", crate::tr!("git {} pasó de {} s: se cortó (¿un hook que espera algo?)", "git {} went over {} s: stopped (a hook waiting for something?)", args.first().unwrap_or(&""), tope.as_secs())))?
        .map_err(|e| anyhow!("git: {e}"))?;
    let mut s = String::from_utf8_lossy(&o.stdout).into_owned();
    let e = String::from_utf8_lossy(&o.stderr);
    if !e.trim().is_empty() {
        if !s.is_empty() && !s.ends_with('\n') {
            s.push('\n');
        }
        s.push_str(&e);
    }
    Ok((o.status.success(), s))
}

/// Lo mismo, síncrono y estricto (para `worktree.rs`): la salida limpia si salió bien; si no, el error con
/// la orden y lo que dijo `git`.
pub fn git_sinc(carpeta: &Path, args: &[&str]) -> Result<String> {
    let o = crate::entorno::comando("git")
        .arg("-C")
        .arg(carpeta)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_EDITOR", "true")
        .env("GIT_PAGER", "cat")
        .stdin(std::process::Stdio::null())
        .output()?;
    if !o.status.success() {
        bail!("git {}: {}", args.join(" "), String::from_utf8_lossy(&o.stderr).trim());
    }
    Ok(String::from_utf8_lossy(&o.stdout).trim().to_string())
}

/// Recorta una salida larga (primeras `n` líneas).
fn recorta(s: &str, n: usize) -> String {
    let l: Vec<&str> = s.lines().collect();
    let mut out = l.iter().take(n).copied().collect::<Vec<_>>().join("\n");
    if l.len() > n {
        out.push_str(&crate::tr!("\n… ({} líneas más)", "\n… ({} more lines)", l.len() - n));
    }
    out
}

fn texto(args: &Value, k: &str) -> Option<String> {
    args[k].as_str().map(str::trim).filter(|s| !s.is_empty()).map(String::from)
}

fn requiere(args: &Value, k: &str) -> Result<String> {
    texto(args, k).ok_or_else(|| anyhow!("{}", crate::tr!("falta '{k}'", "missing '{k}'")))
}

fn lista(args: &Value, k: &str) -> Vec<String> {
    match &args[k] {
        Value::Array(a) => a.iter().filter_map(|x| x.as_str().map(|s| s.trim().to_string())).filter(|s| !s.is_empty()).collect(),
        Value::String(s) if !s.trim().is_empty() => vec![s.trim().to_string()],
        _ => vec![],
    }
}

/// Un nombre de rama o remoto sin trucos (nada que `git` lea como opción).
fn nombre_limpio(n: &str) -> Result<&str> {
    if n.starts_with('-') || n.contains("..") || n.chars().any(|c| c.is_whitespace() || matches!(c, '~' | '^' | ':' | '?' | '*' | '[' | '\\')) {
        bail!("{}", crate::tr!("nombre no válido para git: «{n}»", "not a valid git name: «{n}»"));
    }
    Ok(n)
}

/// `diferencia` (nivel 2: el texto de una diferencia lo da `git`): sin archivo, el RESUMEN (preparado y sin
/// preparar, con +/−); con archivo, el detalle (300 líneas). `rango` = `a..b` o un commit.
async fn diferencia(carpeta: &Path, args: &Value) -> Result<String> {
    let rango = texto(args, "rango");
    if let Some(r) = &rango {
        if r.starts_with('-') {
            bail!("{}", crate::i18n::t("rango no válido", "invalid range"));
        }
    }
    let archivo = texto(args, "archivo");
    let preparado = args["preparado"].as_bool().unwrap_or(false);
    let base = |extra: &[&str]| {
        let mut a: Vec<String> = vec!["diff".into(), "--no-color".into(), "--no-ext-diff".into()];
        a.extend(extra.iter().map(|s| s.to_string()));
        if preparado {
            a.push("--cached".into());
        }
        if let Some(r) = &rango {
            a.push(r.clone());
        }
        a
    };
    match archivo {
        Some(f) => {
            let mut a = base(&[]);
            a.push("--".into());
            a.push(f);
            let refs: Vec<&str> = a.iter().map(String::as_str).collect();
            let (ok, s) = git(carpeta, &refs, Duration::from_secs(60)).await?;
            if !ok {
                bail!("{}", recorta(&s, 20));
            }
            Ok(if s.trim().is_empty() { crate::i18n::t("(sin diferencias)", "(no differences)").into() } else { recorta(&s, 300) })
        }
        None => {
            let mut partes = Vec::new();
            let quien: &[(bool, &str)] = if rango.is_some() || preparado { &[(true, "")] } else { &[(false, "sin preparar"), (true, "preparado")] };
            for (cached, titulo) in quien {
                let mut a: Vec<String> = vec!["diff".into(), "--no-color".into(), "--numstat".into()];
                if *cached && rango.is_none() {
                    a.push("--cached".into());
                }
                if let Some(r) = &rango {
                    a.push(r.clone());
                }
                let refs: Vec<&str> = a.iter().map(String::as_str).collect();
                let (ok, s) = git(carpeta, &refs, Duration::from_secs(60)).await?;
                if !ok {
                    bail!("{}", recorta(&s, 20));
                }
                let filas: Vec<String> = s
                    .lines()
                    .filter_map(|l| {
                        let mut p = l.splitn(3, '\t');
                        let (mas, menos, ruta) = (p.next()?, p.next()?, p.next()?);
                        Some(if mas == "-" { crate::tr!("{ruta} (binario)", "{ruta} (binary)") } else { format!("+{mas} −{menos} {ruta}") })
                    })
                    .collect();
                if !filas.is_empty() {
                    let t = match *titulo {
                        "sin preparar" => crate::i18n::t("sin preparar", "not staged"),
                        "preparado" => crate::i18n::t("preparado", "staged"),
                        _ => "",
                    };
                    let cab = if t.is_empty() { String::new() } else { format!("{t}:\n") };
                    partes.push(format!("{cab}{}", recorta(&filas.join("\n"), 80)));
                }
            }
            if partes.is_empty() {
                return Ok(crate::i18n::t("(sin diferencias)", "(no differences)").into());
            }
            partes.push(crate::i18n::t("(el detalle de uno: action=diferencia con archivo)", "(one file's detail: action=diferencia with archivo)").into());
            Ok(partes.join("\n"))
        }
    }
}

/// `historial` de un tramo de líneas de un archivo (`git log -L`, nivel 2: `gix` no lo tiene).
async fn historial_de_lineas(carpeta: &Path, archivo: &str, desde: u32, hasta: u32, n: usize) -> Result<String> {
    let rango = format!("{desde},{hasta}:{archivo}"); // juez-idioma: datos (sintaxis de git log -L)
    let n = n.to_string();
    let (ok, s) = git(carpeta, &["log", "--no-color", "-s", "--date=short", "--format=%h %ad %an — %s", "-n", &n, "-L", &rango], Duration::from_secs(120)).await?;
    if !ok {
        bail!("{}", recorta(&s, 20));
    }
    Ok(if s.trim().is_empty() { crate::i18n::t("(ningún commit toca esas líneas)", "(no commit touches those lines)").into() } else { recorta(&s, 200) })
}

/// ¿Quedan marcas de conflicto en un texto?
fn con_marcas(t: &str) -> bool {
    t.lines().any(|l| l.starts_with("<<<<<<< ") || l.starts_with(">>>>>>> ") || l == "=======")
}

/// La ejecución de la herramienta `git`.
pub async fn ejecutar(args: &Value, cx: Contexto<'_>) -> Result<String> {
    let root = cx.root;
    let accion = args["action"].as_str().unwrap_or("estado");
    let n = args["n"].as_u64().unwrap_or(20).clamp(1, 200) as usize;
    // las lecturas de gix son síncronas: fuera del hilo de tokio
    let aparte = |f: Box<dyn FnOnce() -> Result<String> + Send>| async move { tokio::task::spawn_blocking(f).await.map_err(|e| anyhow!("{e}"))? };
    let r = root.to_path_buf();
    match accion {
        "estado" | "status" => aparte(Box::new(move || leer::estado(&r))).await,
        "historial" | "log" => match (texto(args, "archivo"), args["desde"].as_u64(), args["hasta"].as_u64()) { // juez-idioma: datos (nombres de parámetros)
            (Some(a), Some(d), h) => {
                let carpeta = leer::carpeta(&leer::abrir(root)?)?;
                historial_de_lineas(&carpeta, &a, d as u32, h.unwrap_or(d) as u32, n).await
            }
            (a, _, _) => aparte(Box::new(move || leer::historial(&r, a.as_deref(), n))).await,
        },
        "autoria" | "blame" => {
            let a = requiere(args, "archivo")?; // juez-idioma: datos (nombre de parámetro)
            let d = args["desde"].as_u64().unwrap_or(1) as u32;
            let h = args["hasta"].as_u64().map(|h| h as u32).unwrap_or(d + 39);
            aparte(Box::new(move || leer::autoria(&r, &a, d, h))).await
        }
        "ramas" | "branches" => aparte(Box::new(move || leer::ramas(&r))).await,
        "conflictos" | "conflicts" => aparte(Box::new(move || leer::conflictos(&r))).await,
        "diferencia" | "diff" => {
            let carpeta = leer::carpeta(&leer::abrir(root)?)?;
            diferencia(&carpeta, args).await
        }
        _ => escribir(accion, args, &cx).await,
    }
}

/// Las acciones que escriben (binario `git`).
async fn escribir(accion: &str, args: &Value, cx: &Contexto<'_>) -> Result<String> {
    // el repositorio sólo para saber su carpeta: no se guarda a través de los `await`
    let carpeta = leer::carpeta(&leer::abrir(cx.root)?)?;
    let raiz = cx.root.canonicalize().unwrap_or_else(|_| cx.root.to_path_buf());
    // una ruta del proyecto → relativa al repositorio; y si es del agente (relativa a la raíz del proyecto)
    let rel_repo = |p: &str| leer::relativa_a(&carpeta, cx.root, p);
    let del_agente = |p: &str| -> bool {
        let abs = crate::paths::resolve(cx.root, p);
        let abs = abs.canonicalize().unwrap_or(abs);
        abs.strip_prefix(&raiz).map(|r| cx.del_agente.contains(&r.to_string_lossy().replace('\\', "/"))).unwrap_or(false)
    };
    let corto = Duration::from_secs(60);
    match accion {
        "preparar" | "add" => {
            let rutas = lista(args, "rutas");
            if rutas.is_empty() {
                bail!("{}", crate::i18n::t("preparar pide `rutas`: archivos concretos (nunca «.» ni -A)", "preparar needs `rutas`: concrete files (never «.» or -A)"));
            }
            let mut rel = Vec::new();
            let mut ajenos = Vec::new();
            for p in &rutas {
                if p == "." || p.starts_with('-') || p.contains('*') || crate::paths::resolve(cx.root, p).is_dir() {
                    bail!("{}", crate::tr!("«{p}» no es un archivo concreto: GOTV prepara archivo por archivo", "«{p}» is not a concrete file: GOTV stages file by file"));
                }
                if !del_agente(p) {
                    ajenos.push(p.clone());
                }
                rel.push(rel_repo(p)?);
            }
            if !ajenos.is_empty() && args["incluir_ajenos"].as_bool() != Some(true) {
                bail!("{}", crate::tr!(
                    "no los cambió GOTV en esta sesión: {}. Son del usuario: sólo entran con incluir_ajenos=true (el usuario lo aprueba).",
                    "GOTV did not change them in this session: {}. They are the user's: they only go in with incluir_ajenos=true (the user approves).",
                    ajenos.join(", ")
                ));
            }
            let mut a = vec!["add", "--"];
            a.extend(rel.iter().map(String::as_str));
            let (ok, s) = git(&carpeta, &a, corto).await?;
            if !ok {
                bail!("{}", recorta(&s, 20));
            }
            Ok(crate::tr!("preparados: {}", "staged: {}", rel.join(", ")))
        }
        "confirmar" | "commit" => {
            let mensaje = requiere(args, "mensaje")?;
            if !lista(args, "rutas").is_empty() {
                Box::pin(escribir("preparar", args, cx)).await?;
            }
            // sin -a: sólo lo preparado; un hook puede tardar (pruebas) y su salida se muestra
            let (ok, s) = git(&carpeta, &["commit", "-m", &mensaje], Duration::from_secs(300)).await?;
            if !ok {
                bail!("{}", crate::tr!("el commit no se hizo:\n{}", "the commit was not made:\n{}", recorta(&s, 40)));
            }
            Ok(recorta(&s, 40))
        }
        "guardar" | "stash" => {
            let mut a = vec!["stash", "push"];
            let m = texto(args, "mensaje");
            if let Some(m) = &m {
                a.push("-m");
                a.push(m);
            }
            let (ok, s) = git(&carpeta, &a, corto).await?;
            if !ok {
                bail!("{}", recorta(&s, 20));
            }
            Ok(recorta(&s, 20))
        }
        "recuperar" | "unstash" => {
            // `apply` (no `pop`): si choca, el guardado sigue ahí
            let (ok, s) = git(&carpeta, &["stash", "apply"], corto).await?;
            if !ok {
                bail!("{}", recorta(&s, 30));
            }
            let _ = git(&carpeta, &["stash", "drop"], corto).await;
            Ok(recorta(&s, 30))
        }
        "rama" | "branch" => {
            let nombre = requiere(args, "nombre")?;
            let (ok, s) = git(&carpeta, &["switch", "-c", nombre_limpio(&nombre)?], corto).await?;
            if !ok {
                bail!("{}", recorta(&s, 20));
            }
            Ok(crate::tr!("rama nueva «{nombre}» (estás en ella)", "new branch «{nombre}» (you are on it)"))
        }
        "cambiar" | "switch" => {
            let nombre = requiere(args, "nombre")?;
            let (ok, s) = git(&carpeta, &["switch", nombre_limpio(&nombre)?], corto).await?;
            if !ok {
                bail!("{}", crate::tr!("no se cambió de rama:\n{}", "did not switch branch:\n{}", recorta(&s, 20)));
            }
            Ok(crate::tr!("ahora en «{nombre}»", "now on «{nombre}»"))
        }
        "resolver" | "resolve" => {
            let archivo = requiere(args, "archivo")?;
            let rel = rel_repo(&archivo)?;
            let abs = carpeta.join(&rel);
            match (texto(args, "lado"), args["contenido"].as_str()) {
                (Some(lado), _) => {
                    let bandera = match lado.as_str() {
                        "nuestro" | "ours" => "--ours",
                        "suyo" | "theirs" => "--theirs",
                        otro => bail!("{}", crate::tr!("lado «{otro}»: nuestro o suyo", "side «{otro}»: nuestro or suyo")),
                    };
                    let (ok, s) = git(&carpeta, &["checkout", bandera, "--", &rel], corto).await?;
                    if !ok {
                        bail!("{}", recorta(&s, 20));
                    }
                }
                (None, Some(c)) => {
                    if con_marcas(c) && args["con_marcas"].as_bool() != Some(true) {
                        bail!("{}", crate::i18n::t("el contenido aún tiene marcas de conflicto (<<<<<<< ======= >>>>>>>): quítalas", "the content still has conflict markers (<<<<<<< ======= >>>>>>>): remove them"));
                    }
                    std::fs::write(&abs, c)?;
                }
                (None, None) => bail!("{}", crate::i18n::t("resolver pide `contenido` (el archivo final) o `lado` (nuestro | suyo)", "resolver needs `contenido` (the final file) or `lado` (nuestro | suyo)")),
            }
            let (ok, s) = git(&carpeta, &["add", "--", &rel], corto).await?;
            if !ok {
                bail!("{}", recorta(&s, 20));
            }
            let quedan = leer::conflictos_de(cx.root)?.len();
            Ok(crate::tr!("{rel} resuelto ({quedan} conflicto(s) más)", "{rel} resolved ({quedan} more conflict(s))"))
        }
        "traer" | "fetch" | "pull" => {
            let (ok, s) = if args["fusionar"].as_bool() == Some(true) || accion == "pull" {
                git(&carpeta, &["pull", "--ff-only"], Duration::from_secs(180)).await?
            } else {
                git(&carpeta, &["fetch", "--prune"], Duration::from_secs(180)).await?
            };
            if !ok {
                bail!("{}", recorta(&s, 30));
            }
            Ok(if s.trim().is_empty() { crate::i18n::t("al día", "up to date").into() } else { recorta(&s, 30) })
        }
        "empujar" | "push" => {
            let mut a: Vec<String> = vec!["push".into()];
            if args["forzar"].as_bool() == Some(true) {
                a.push("--force-with-lease".into());
            }
            if let Some(r) = texto(args, "remoto") {
                a.push(nombre_limpio(&r)?.to_string());
                if let Some(b) = texto(args, "nombre") {
                    a.push(nombre_limpio(&b)?.to_string());
                }
            }
            let refs: Vec<&str> = a.iter().map(String::as_str).collect();
            let (ok, s) = git(&carpeta, &refs, Duration::from_secs(180)).await?;
            if !ok {
                bail!("{}", recorta(&s, 30));
            }
            Ok(recorta(&s, 30))
        }
        otra => bail!("{}", crate::tr!(
            "acción desconocida: {otra} (estado · diferencia · historial · autoria · ramas · conflictos · resolver · preparar · confirmar · guardar · recuperar · rama · cambiar · traer · empujar)",
            "unknown action: {otra} (estado · diferencia · historial · autoria · ramas · conflictos · resolver · preparar · confirmar · guardar · recuperar · rama · cambiar · traer · empujar)"
        )),
    }
}

/// ¿Escribe en la carpeta de trabajo? (pide punto de restauración antes)
pub fn toca_archivos(accion: &str) -> bool {
    matches!(accion, "resolver" | "resolve" | "recuperar" | "unstash" | "cambiar" | "switch" | "traer" | "fetch" | "pull" | "guardar" | "stash")
}

/// La clase de permiso de cada acción: `None` = lectura.
pub fn clase(accion: &str) -> Option<&'static str> {
    match accion {
        "estado" | "status" | "historial" | "log" | "autoria" | "blame" | "ramas" | "branches" | "conflictos" | "conflicts" | "diferencia" | "diff" => None,
        "traer" | "fetch" | "pull" => Some("red"),
        "empujar" | "push" => Some("empujar"),
        _ => Some("edicion"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Un repo de juguete hecho con el propio `git`, con identidad fija y sin la configuración del usuario.
    pub fn sh(d: &Path, orden: &str) -> String {
        let o = std::process::Command::new("bash")
            .arg("-c")
            .arg(orden)
            .current_dir(d)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_AUTHOR_NAME", "Ana")
            .env("GIT_AUTHOR_EMAIL", "ana@x")
            .env("GIT_COMMITTER_NAME", "Ana")
            .env("GIT_COMMITTER_EMAIL", "ana@x")
            .output()
            .unwrap();
        format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr))
    }

    fn repo(tag: &str) -> crate::CarpetaPrueba {
        let d = crate::CarpetaPrueba::nueva(tag);
        sh(&d, "git init -q -b main && git config user.name Ana && git config user.email ana@x && git config commit.gpgsign false");
        d
    }

    fn cx(root: &Path, del_agente: &[&str]) -> Contexto<'static> {
        Contexto { root: Box::leak(root.to_path_buf().into_boxed_path()), del_agente: del_agente.iter().map(|s| s.to_string()).collect() }
    }

    #[tokio::test]
    async fn estado_historial_autoria_y_ramas() {
        let d = repo("gotv-git-leer");
        std::fs::write(d.join("a.txt"), "uno\ndos\ntres\n").unwrap();
        sh(&d, "git add a.txt && git commit -q -m 'primero'");
        std::fs::write(d.join("a.txt"), "uno\nDOS\ntres\n").unwrap();
        std::fs::write(d.join("b.txt"), "b\n").unwrap();
        sh(&d, "git add a.txt b.txt && git commit -q -m 'segundo: cambia dos' && GIT_AUTHOR_NAME=Beto git commit -q --allow-empty -m 'vacío de Beto'");
        // un archivo en cada situación
        std::fs::write(d.join("a.txt"), "uno\nDOS\ntres\ncuatro\n").unwrap();
        std::fs::write(d.join("c.txt"), "c\n").unwrap();
        sh(&d, "git add c.txt");
        std::fs::write(d.join("nuevo.txt"), "n\n").unwrap();
        std::fs::remove_file(d.join("b.txt")).unwrap();
        let e = ejecutar(&json!({"action": "estado"}), cx(&d, &[])).await.unwrap();
        assert!(e.starts_with("rama main"), "{e}");
        assert!(e.contains("preparados para el commit (1): c.txt (nuevo)"), "{e}");
        assert!(e.contains("cambiados sin preparar (2): a.txt, b.txt (borrado)"), "{e}");
        assert!(e.contains("nuevos sin seguir (1): nuevo.txt"), "{e}");
        // historial de un archivo: sólo los commits que lo tocan
        let h = ejecutar(&json!({"action": "historial", "archivo": "a.txt"}), cx(&d, &[])).await.unwrap();
        assert_eq!(h.lines().count(), 2, "{h}");
        assert!(h.lines().next().unwrap().contains("Ana — segundo: cambia dos"), "{h}");
        let todo = ejecutar(&json!({"action": "historial"}), cx(&d, &[])).await.unwrap();
        assert!(todo.lines().next().unwrap().contains("Beto — vacío de Beto"), "{todo}");
        // historial de una línea (git log -L)
        let l = ejecutar(&json!({"action": "historial", "archivo": "a.txt", "desde": 2, "hasta": 2}), cx(&d, &[])).await.unwrap();
        assert!(l.contains("segundo: cambia dos") && l.contains("primero"), "{l}");
        // autoría: la línea 2 la cambió «segundo», la 1 viene de «primero»
        let a = ejecutar(&json!({"action": "autoria", "archivo": "a.txt", "desde": 1, "hasta": 3}), cx(&d, &[])).await.unwrap();
        let (l1, l2) = (a.lines().position(|l| l.contains("primero")).unwrap(), a.lines().position(|l| l.contains("segundo")).unwrap());
        assert!(a.lines().nth(l2 + 1).unwrap().contains("2| DOS") && l1 != l2, "{a}");
        let r = ejecutar(&json!({"action": "ramas"}), cx(&d, &[])).await.unwrap();
        assert!(r.contains("* main — ") && r.contains("vacío de Beto"), "{r}");
        // diferencia: resumen y detalle
        let df = ejecutar(&json!({"action": "diferencia"}), cx(&d, &[])).await.unwrap();
        assert!(df.contains("+1 −0 a.txt") && df.contains("preparado:\n+1 −0 c.txt"), "{df}");
        let df = ejecutar(&json!({"action": "diferencia", "archivo": "a.txt"}), cx(&d, &[])).await.unwrap();
        assert!(df.contains("+cuatro"), "{df}");
    }

    /// Un conflicto de fusión con sus tres lados EXACTOS; resolverlo con el contenido final (sin marcas) y
    /// confirmar; `estado` lo dice mientras dura.
    #[tokio::test]
    async fn conflicto_con_sus_tres_lados() {
        let d = repo("gotv-git-conflicto");
        std::fs::write(d.join("p.txt"), "precio = 10\n").unwrap();
        sh(&d, "git add p.txt && git commit -q -m base && git switch -q -c otra");
        std::fs::write(d.join("p.txt"), "precio = 12\n").unwrap();
        sh(&d, "git commit -q -am 'sube a 12' && git switch -q main");
        std::fs::write(d.join("p.txt"), "precio = 11\n").unwrap();
        sh(&d, "git commit -q -am 'sube a 11' && git merge otra -q || true");
        let c = leer::conflictos_de(&d).unwrap();
        assert_eq!(c, vec![leer::Conflicto { ruta: "p.txt".into(), base: Some("precio = 10\n".into()), nuestro: Some("precio = 11\n".into()), suyo: Some("precio = 12\n".into()) }]);
        let e = ejecutar(&json!({"action": "estado"}), cx(&d, &["p.txt"])).await.unwrap();
        assert!(e.contains("CONFLICTOS (1): p.txt") && e.contains("operación a medias: Merge"), "{e}");
        let t = ejecutar(&json!({"action": "conflictos"}), cx(&d, &["p.txt"])).await.unwrap();
        assert!(t.contains("BASE (el ancestro común):\n    precio = 10") && t.contains("SUYO (lo que entra):\n    precio = 12"), "{t}");
        // con marcas: no
        assert!(ejecutar(&json!({"action": "resolver", "archivo": "p.txt", "contenido": "<<<<<<< HEAD\nx\n=======\ny\n>>>>>>> otra\n"}), cx(&d, &["p.txt"])).await.is_err());
        let r = ejecutar(&json!({"action": "resolver", "archivo": "p.txt", "contenido": "precio = 12 # las dos subidas: la mayor\n"}), cx(&d, &["p.txt"])).await.unwrap();
        assert!(r.contains("p.txt resuelto (0 conflicto(s) más)"), "{r}");
        ejecutar(&json!({"action": "confirmar", "mensaje": "fusiona otra"}), cx(&d, &["p.txt"])).await.unwrap();
        assert!(sh(&d, "git log -1 --format=%s").contains("fusiona otra"));
        assert!(leer::conflictos_de(&d).unwrap().is_empty());
    }

    /// L7: `confirmar` no incluye un archivo ajeno modificado (ni con `-a`, que no usa); preparar lo ajeno
    /// pide `incluir_ajenos`; «.» y carpetas, nunca.
    #[tokio::test]
    async fn confirmar_no_lleva_lo_ajeno() {
        let d = repo("gotv-git-ajeno");
        std::fs::write(d.join("mio.txt"), "1\n").unwrap();
        std::fs::write(d.join("suyo.txt"), "1\n").unwrap();
        sh(&d, "git add . && git commit -q -m inicio");
        std::fs::write(d.join("mio.txt"), "2\n").unwrap();
        std::fs::write(d.join("suyo.txt"), "2\n").unwrap();
        let e = ejecutar(&json!({"action": "confirmar", "mensaje": "x", "rutas": ["mio.txt", "suyo.txt"]}), cx(&d, &["mio.txt"])).await.unwrap_err().to_string();
        assert!(e.contains("suyo.txt") && e.contains("incluir_ajenos"), "{e}");
        assert!(ejecutar(&json!({"action": "preparar", "rutas": ["."]}), cx(&d, &["mio.txt"])).await.is_err());
        ejecutar(&json!({"action": "confirmar", "mensaje": "sólo lo mío", "rutas": ["mio.txt"]}), cx(&d, &["mio.txt"])).await.unwrap();
        let hecho = sh(&d, "git show --name-only --format= HEAD");
        assert_eq!(hecho.trim(), "mio.txt", "el ajeno quedó fuera");
        assert!(sh(&d, "git status --porcelain").contains(" M suyo.txt"));
        // con permiso explícito, sí
        ejecutar(&json!({"action": "preparar", "rutas": ["suyo.txt"], "incluir_ajenos": true}), cx(&d, &["mio.txt"])).await.unwrap();
        // rama, cambiar, guardar y recuperar
        sh(&d, "git commit -q -m 'lo suyo'");
        ejecutar(&json!({"action": "rama", "nombre": "prueba"}), cx(&d, &[])).await.unwrap();
        assert!(ejecutar(&json!({"action": "rama", "nombre": "--force"}), cx(&d, &[])).await.is_err());
        std::fs::write(d.join("mio.txt"), "3\n").unwrap();
        ejecutar(&json!({"action": "guardar", "mensaje": "a medias"}), cx(&d, &[])).await.unwrap();
        assert_eq!(std::fs::read_to_string(d.join("mio.txt")).unwrap(), "2\n");
        ejecutar(&json!({"action": "cambiar", "nombre": "main"}), cx(&d, &[])).await.unwrap();
        ejecutar(&json!({"action": "recuperar"}), cx(&d, &[])).await.unwrap();
        assert_eq!(std::fs::read_to_string(d.join("mio.txt")).unwrap(), "3\n");
    }

    #[test]
    fn clases_de_permiso() {
        assert_eq!(clase("estado"), None);
        assert_eq!(clase("autoria"), None);
        assert_eq!(clase("confirmar"), Some("edicion"));
        assert_eq!(clase("traer"), Some("red"));
        assert_eq!(clase("empujar"), Some("empujar"));
        assert!(toca_archivos("cambiar") && !toca_archivos("confirmar"));
    }
}
