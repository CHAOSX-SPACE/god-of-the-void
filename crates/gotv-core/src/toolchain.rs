//! C5 — Las herramientas de programación de ESTA Mac: qué hay (para que el modelo elija
//! comandos que existen, como el bloque de entorno de Claude Code) y cómo instalar lo que
//! falta con su instalador oficial. Instalar toca fuera del proyecto: SIEMPRE con permiso
//! del usuario (también en bypass) y fuera de la caja de arena.

use std::path::Path;
use std::time::Duration;

/// (nombre, comando que da su versión)
const TOOLS: &[(&str, &[&str])] = &[
    ("git", &["git", "--version"]),
    // `python3` NO se ejecuta (decisión del Portador, 2026-11-12: GOTV no lanza Python para sí mismo, ni
    // para preguntar su versión; además el `/usr/bin/python3` de macOS sin las herramientas de línea
    // de comandos abre un diálogo de instalación): se busca en el PATH y la versión sale de su ruta
    ("python3", &[]),
    ("uv", &["uv", "--version"]),
    ("node", &["node", "--version"]),
    ("npm", &["npm", "--version"]),
    ("pnpm", &["pnpm", "--version"]),
    ("bun", &["bun", "--version"]),
    ("deno", &["deno", "--version"]),
    ("cargo", &["cargo", "--version"]),
    ("go", &["go", "version"]),
    ("java", &["java", "--version"]),
    ("php", &["php", "--version"]),
    ("ruby", &["ruby", "--version"]),
    ("swift", &["swift", "--version"]),
    ("docker", &["docker", "--version"]),
    ("gh", &["gh", "--version"]),
    ("brew", &["brew", "--version"]),
];

/// Cómo se instala cada una (preferencia: Homebrew; si no hay, el instalador oficial).
pub fn install_command(name: &str, has_brew: bool) -> Option<String> {
    let brew = |f: &str| format!("brew install {f}");
    Some(match (name, has_brew) {
        ("uv", true) => brew("uv"),
        ("uv", false) => "curl -LsSf https://astral.sh/uv/install.sh | sh".into(),
        ("node" | "npm", true) => brew("node"),
        ("pnpm", true) => brew("pnpm"),
        ("pnpm", false) => "npm install -g pnpm".into(),
        ("bun", true) => brew("oven-sh/bun/bun"),
        ("bun", false) => "curl -fsSL https://bun.sh/install | bash".into(),
        ("deno", true) => brew("deno"),
        ("cargo" | "rust" | "rustup", true) => {
            format!("{} && rustup-init -y --no-modify-path", brew("rustup"))
        }
        ("cargo" | "rust" | "rustup", false) => {
            "curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y".into()
        }
        ("go", true) => brew("go"),
        ("python3" | "python", true) => brew("python"),
        ("gh", true) => brew("gh"),
        ("php", true) => brew("php"),
        ("ruby", true) => brew("ruby"),
        ("java", true) => brew("openjdk"),
        ("brew" | "homebrew", _) => return None, // lo instala la persona: pide su contraseña
        _ => return None,
    })
}

/// Un `TMPDIR` propio para los comandos de versión. Algunos ensucian el temporal sólo por
/// dar su versión (medido 2026-11-11): `swift --version` deja un `TemporaryDirectory.XXXXXX`
/// y npm/pnpm su `node-compile-cache`; cada arranque de GOTV (y cada corrida de las pruebas)
/// los iba acumulando. Corren aquí dentro y al soltarlo se borra ESTA carpeta, la que se
/// creó aquí — jamás por patrón ni nada fuera de ella (falla #2571).
struct TemporalPropio(Option<std::path::PathBuf>);

impl TemporalPropio {
    fn nuevo() -> Self {
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let d = std::env::temp_dir().join(format!("gotv-herramientas-{}-{n}", std::process::id()));
        // si no se puede crear, los comandos usan el temporal de siempre
        Self(std::fs::create_dir(&d).ok().map(|_| d))
    }
}

impl Drop for TemporalPropio {
    fn drop(&mut self) {
        if let Some(d) = &self.0 {
            let _ = std::fs::remove_dir_all(d);
        }
    }
}

/// Una herramienta SIN ejecutarla: el primer `nombre` del PATH, y su versión si la ruta real
/// (resueltos los enlaces) la trae (`…/Versions/3.14/bin/python3.14` → `3.14`); si no, `✓`.
fn sin_ejecutar(nombre: &str, path: &str) -> Option<String> {
    let exe = std::env::split_paths(path).map(|d| d.join(nombre)).find(|p| p.is_file())?;
    let real = std::fs::canonicalize(&exe).unwrap_or(exe);
    let es_version = |s: &str| {
        let mut partes = s.split('.');
        partes.next().is_some_and(|a| !a.is_empty() && a.chars().all(|c| c.is_ascii_digit()))
            && partes.next().is_some_and(|b| !b.is_empty() && b.chars().all(|c| c.is_ascii_digit()))
    };
    let v = real.iter().rev().filter_map(|c| c.to_str()).find_map(|c| {
        let c = c.trim_start_matches(|ch: char| ch.is_ascii_alphabetic());
        es_version(c).then(|| c.to_string())
    });
    Some(v.unwrap_or_else(|| "✓".into()))
}

async fn version(cmd: &[&str], tmp: Option<&Path>) -> Option<String> {
    let mut c = crate::entorno::comando_tokio(cmd[0]);
    c.args(&cmd[1..]).kill_on_drop(true); // el que pasa del tope muere: no escribe tras borrar `tmp`
    if let Some(t) = tmp {
        c.env("TMPDIR", t);
    }
    let o = tokio::time::timeout(Duration::from_secs(4), c.output())
    .await
    .ok()?
    .ok()?;
    if !o.status.success() {
        return None;
    }
    let mut t = String::from_utf8_lossy(&o.stdout).to_string();
    if t.trim().is_empty() {
        t = String::from_utf8_lossy(&o.stderr).to_string();
    }
    // la primera línea, y de ella el primer número de versión
    let first = t.lines().next().unwrap_or("").to_string();
    let v = first
        .split(|c: char| c.is_whitespace() || c == ',')
        .find(|w| {
            w.trim_start_matches(['v', 'V'])
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_digit())
                && w.contains('.')
        })
        .map(|w| {
            w.trim_start_matches(['v', 'V'])
                .trim_start_matches("go")
                .to_string()
        })
        .or_else(|| {
            first.split_whitespace().find_map(|w| {
                w.strip_prefix("go")
                    .filter(|r| r.contains('.'))
                    .map(String::from)
            })
        });
    Some(v.unwrap_or_else(|| "✓".into()))
}

type Detected = (Vec<(String, String)>, Vec<String>);

fn detected() -> &'static std::sync::Mutex<Option<Detected>> {
    static D: std::sync::OnceLock<std::sync::Mutex<Option<Detected>>> = std::sync::OnceLock::new();
    D.get_or_init(Default::default)
}

/// Lo instalado (nombre, versión) y lo que falta: medido UNA vez por proceso (no depende del
/// proyecto) y de nuevo tras instalar algo.
pub async fn detect() -> Detected {
    if let Some(d) = detected().lock().unwrap().clone() {
        return d;
    }
    let d = measure().await;
    *detected().lock().unwrap() = Some(d.clone());
    d
}

async fn measure() -> Detected {
    let tmp = TemporalPropio::nuevo();
    let t = tmp.0.as_deref();
    let path = std::env::var("PATH").unwrap_or_default();
    let path = &path;
    let futs = TOOLS.iter().map(|(n, c)| async move {
        let v = if c.is_empty() { sin_ejecutar(n, path) } else { version(c, t).await };
        (n.to_string(), v)
    });
    let all = futures_util::future::join_all(futs).await;
    drop(tmp);
    let mut have = Vec::new();
    let mut miss = Vec::new();
    for (n, v) in all {
        match v {
            Some(v) => have.push((n, v)),
            None => miss.push(n),
        }
    }
    (have, miss)
}

/// El bloque de entorno para las instrucciones (como el de Claude Code).
pub async fn env_block(root: &Path) -> String {
    let (have, miss) = detect().await;
    let os = std::process::Command::new("sw_vers")
        .arg("-productVersion")
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();
    let arch = std::env::consts::ARCH;
    let branch = std::process::Command::new("git")
        .args([
            "-C",
            &root.to_string_lossy(),
            "rev-parse",
            "--abbrev-ref",
            "HEAD",
        ])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string());
    let mut s = crate::tr!(
        "\n\nEntorno: macOS {os} ({arch})",
        "\n\nEnvironment: macOS {os} ({arch})"
    );
    s.push_str(&match branch {
        Some(b) => crate::tr!(" · repo git (rama {b})", " · git repo (branch {b})"),
        None => crate::i18n::t(" · sin repo git", " · not a git repo").to_string(),
    });
    s.push_str(&crate::tr!(
        "\nInstalado: {}",
        "\nInstalled: {}",
        have.iter()
            .map(|(n, v)| format!("{n} {v}"))
            .collect::<Vec<_>>()
            .join(" · ")
    ));
    if !miss.is_empty() {
        s.push_str(&crate::tr!(
            "\nNo instalado: {} (si hace falta, la herramienta `toolchain` —búscala con tool_search— lo instala con permiso)",
            "\nNot installed: {} (if needed, the `toolchain` tool —find it with tool_search— installs it with permission)",
            miss.join(", ")
        ));
    }
    // F5: el modelo sabe qué parte de su motor de lenguaje está encendida aquí
    let servidores: Vec<String> = crate::lsp::instalados()
        .into_iter()
        .map(|(l, b)| format!("{l} ({b})"))
        .collect();
    let servidores = if servidores.is_empty() {
        crate::i18n::t(
            "ninguno instalado (se activan instalándolos, con permiso del usuario: p. ej. `npm i -g pyright`)",
            "none installed (they turn on once installed, with the user's permission: e.g. `npm i -g pyright`)",
        )
        .to_string()
    } else {
        servidores.join(", ")
    };
    let externos = crate::revisor::externos_disponibles();
    let externos = if externos.is_empty() { "—".to_string() } else { externos.join(", ") };
    s.push_str(&crate::tr!(
        "\nMotor de lenguaje de GOTV: tras cada write_file/edit te devuelve los problemas del archivo con su línea. Nativo, siempre encendido: {} (sintaxis, nombres sin definir ni importar, imports del proyecto que no existen — también en los archivos que importaban del que cambiaste) y {} (sintaxis; HTML también ids repetidos). Sintaxis con el revisor de cada lenguaje: {externos}. Servidores completos (tipos y la herramienta `lsp`): {servidores}. Corrige lo que señale ANTES de seguir; «sin errores» no sustituye a las pruebas.",
        "\nGOTV language engine: after every write_file/edit it returns the file's problems with their line. Native, always on: {} (syntax, names neither defined nor imported, project imports that do not exist — also in the files that imported from the one you changed) and {} (syntax; HTML also repeated ids). Syntax with each language's own checker: {externos}. Full servers (types and the `lsp` tool): {servidores}. Fix what it reports BEFORE going on; «no errors» does not replace the tests.",
        crate::revisor::NATIVOS.join(", "),
        crate::revisor::NATIVOS_MARCADO.join(", ")
    ));
    s
}

fn cache() -> &'static std::sync::Mutex<std::collections::HashMap<std::path::PathBuf, String>> {
    static C: std::sync::OnceLock<
        std::sync::Mutex<std::collections::HashMap<std::path::PathBuf, String>>,
    > = std::sync::OnceLock::new();
    C.get_or_init(Default::default)
}

/// Mide el entorno de este proyecto UNA vez (las instrucciones lo leen de la caché).
pub async fn warm(root: &Path) {
    if cache().lock().unwrap().contains_key(root) {
        return;
    }
    let b = env_block(root).await;
    cache().lock().unwrap().insert(root.to_path_buf(), b);
}

/// El bloque ya medido (vacío si aún no se midió).
pub fn cached(root: &Path) -> String {
    cache()
        .lock()
        .unwrap()
        .get(root)
        .cloned()
        .unwrap_or_default()
}

/// Tras instalar algo: se vuelve a medir la próxima vez.
pub fn invalidate() {
    cache().lock().unwrap().clear();
    *detected().lock().unwrap() = None;
}

/// La herramienta `toolchain`: `status` informa; `install` devuelve el comando a correr
/// (el agente lo corre FUERA de la caja tras el permiso del usuario).
pub async fn status() -> String {
    let (have, miss) = detect().await;
    let mut s = have
        .iter()
        .map(|(n, v)| format!("✓ {n} {v}"))
        .collect::<Vec<_>>()
        .join("\n");
    if !miss.is_empty() {
        s.push_str(&format!(
            "\n{}",
            miss.iter()
                .map(|n| format!("— {n}"))
                .collect::<Vec<_>>()
                .join("\n")
        ));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cada_herramienta_sabe_como_instalarse() {
        assert_eq!(
            install_command("uv", true).as_deref(),
            Some("brew install uv")
        );
        assert!(install_command("uv", false).unwrap().contains("astral.sh"));
        assert!(install_command("cargo", true)
            .unwrap()
            .contains("rustup-init -y"));
        assert_eq!(
            install_command("brew", true),
            None,
            "Homebrew lo instala la persona"
        );
        assert_eq!(install_command("algo-raro", true), None);
    }

    #[test]
    fn python_se_ve_sin_ejecutarlo() {
        let c = gotv_engine::prueba::CarpetaPrueba::nueva("gotv-sin-ejecutar");
        let r = c.ruta();
        std::fs::create_dir_all(r.join("Versions/3.14/bin")).unwrap();
        std::fs::create_dir_all(r.join("bin")).unwrap();
        // un «python» que ni siquiera es ejecutable: si se ejecutara, la prueba fallaría
        std::fs::write(r.join("Versions/3.14/bin/python3.14"), "no soy un ejecutable").unwrap();
        std::os::unix::fs::symlink(r.join("Versions/3.14/bin/python3.14"), r.join("bin/python3")).unwrap();
        let path = r.join("bin").to_string_lossy().to_string();
        assert_eq!(sin_ejecutar("python3", &path).as_deref(), Some("3.14"));
        std::fs::write(r.join("bin/uv"), "").unwrap();
        assert_eq!(sin_ejecutar("uv", &path).as_deref(), Some("✓"), "sin versión en la ruta: sólo «está»");
        assert_eq!(sin_ejecutar("nada", &path), None);
        assert!(TOOLS.iter().any(|(n, c)| *n == "python3" && c.is_empty()), "python3 jamás se ejecuta");
    }

    #[tokio::test]
    async fn detecta_lo_instalado_con_su_version() {
        let (have, _miss) = detect().await;
        let git = have
            .iter()
            .find(|(n, _)| n == "git")
            .expect("git viene con macOS");
        assert!(git.1.chars().next().unwrap().is_ascii_digit(), "{git:?}");
        let b = env_block(Path::new(env!("CARGO_MANIFEST_DIR"))).await;
        assert!(b.contains("macOS") && b.contains("git "), "{b}");
        // F5: el modelo sabe qué parte de su motor de lenguaje está encendida
        assert!(b.contains("Python (") && (b.contains("revisor") || b.contains("Motor de lenguaje") || b.contains("language engine")), "{b}");
    }
}
