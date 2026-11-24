//! F0.2 · El `PATH` real del usuario.
//!
//! Una app abierta desde el Dock (o el Finder) la lanza `launchd`, que NO le da el `PATH` de la
//! terminal: busca programas sólo en `/usr/bin:/bin:/usr/sbin:/sbin`. Medido el 2026-11-15:
//! `launchctl print gui/<uid>` no trae `PATH`, mientras la shell de inicio de sesión tiene
//! `/opt/homebrew/bin`, `~/.cargo/bin` y `~/go/bin`. Así, desde la app, el servidor de lenguaje, los
//! servidores MCP y la medición de herramientas (`toolchain`) no encontraban `rust-analyzer`, `node`,
//! `typescript-language-server`, `gopls`… aunque estuvieran instalados. (`bash` no lo sufre: corre
//! con `bash -lc`, que carga el perfil.)
//!
//! Aquí se resuelve UNA vez por proceso: se le pregunta a la shell de inicio de sesión del usuario
//! (`$SHELL -ilc`) con un tope de 3 s, y se le suman los lugares de siempre. Todo programa que GOTV
//! lance por su nombre lo hace con este `PATH` (`entorno::comando`, `entorno::comando_tokio`).
use std::path::PathBuf;
use std::time::Duration;

/// Las marcas que encierran la respuesta: un `.zshrc` que imprime cosas no ensucia el resultado.
const A: &str = "__GOTV_PATH_A__";
const B: &str = "__GOTV_PATH_B__";

/// Lo que se añade siempre al final si no está (por si la shell falla o no los pone).
fn de_siempre() -> Vec<PathBuf> {
    let casa = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    vec![
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/opt/homebrew/sbin"),
        PathBuf::from("/usr/local/bin"),
        casa.join(".cargo/bin"),
        casa.join(".local/bin"),
        casa.join("go/bin"),
        casa.join(".bun/bin"),
        casa.join(".deno/bin"),
        PathBuf::from("/usr/bin"),
        PathBuf::from("/bin"),
        PathBuf::from("/usr/sbin"),
        PathBuf::from("/sbin"),
    ]
}

/// Lo que dice la shell de inicio de sesión, entre las marcas. `None` si no respondió a tiempo.
fn de_la_shell(shell: &str, tope: Duration) -> Option<String> {
    let mut hijo = std::process::Command::new(shell)
        .args(["-ilc", &format!("printf '%s%s%s' '{A}' \"$PATH\" '{B}'")])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;
    let inicio = std::time::Instant::now();
    loop {
        match hijo.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if inicio.elapsed() < tope => std::thread::sleep(Duration::from_millis(20)),
            _ => {
                // una shell que se cuelga (un `.zshrc` que pregunta algo) no detiene a GOTV
                let _ = hijo.kill();
                let _ = hijo.wait();
                return None;
            }
        }
    }
    let mut salida = String::new();
    use std::io::Read;
    hijo.stdout.take()?.read_to_string(&mut salida).ok()?;
    let desde = salida.find(A)? + A.len();
    let hasta = desde + salida[desde..].find(B)?;
    Some(salida[desde..hasta].to_string()).filter(|p| !p.trim().is_empty())
}

/// Une las listas sin repetir, en orden: primero lo de la shell, luego lo heredado, luego lo de
/// siempre (sólo lo que existe).
fn unir(shell: Option<&str>, heredado: Option<&str>) -> String {
    let mut vistos = std::collections::HashSet::new();
    let mut out: Vec<String> = Vec::new();
    let mut poner = |d: &str| {
        let d = d.trim();
        if !d.is_empty() && vistos.insert(d.to_string()) {
            out.push(d.to_string());
        }
    };
    for p in [shell, heredado].into_iter().flatten() {
        p.split(':').for_each(&mut poner);
    }
    for d in de_siempre() {
        if d.is_dir() {
            poner(&d.to_string_lossy());
        }
    }
    out.join(":")
}

/// El `PATH` del usuario, resuelto una vez por proceso.
pub fn path() -> &'static str {
    static P: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    P.get_or_init(|| {
        let heredado = std::env::var("PATH").ok();
        let shell = std::env::var("SHELL").ok().filter(|s| s.starts_with('/')).unwrap_or_else(|| "/bin/zsh".into());
        // en las pruebas y en la terminal ya se hereda el bueno: no hace falta preguntar
        let shell_path = if std::env::var_os("GOTV_SIN_SHELL_PATH").is_some() { None } else { de_la_shell(&shell, Duration::from_secs(3)) };
        unir(shell_path.as_deref(), heredado.as_deref())
    })
}

/// ¿Está `programa` en el `PATH` del usuario? (Para los que se buscan por su nombre.)
pub fn en_path(programa: &str) -> bool {
    donde(programa).is_some()
}

/// La ruta completa de `programa` según el `PATH` del usuario.
pub fn donde(programa: &str) -> Option<PathBuf> {
    if programa.contains('/') {
        let p = PathBuf::from(programa);
        return p.is_file().then_some(p);
    }
    path().split(':').map(|d| PathBuf::from(d).join(programa)).find(|p| p.is_file())
}

/// Un `Command` que busca `programa` y corre con el `PATH` del usuario.
pub fn comando(programa: &str) -> std::process::Command {
    let mut c = std::process::Command::new(donde(programa).unwrap_or_else(|| PathBuf::from(programa)));
    c.env("PATH", path());
    c
}

/// Lo mismo, para `tokio`.
pub fn comando_tokio(programa: &str) -> tokio::process::Command {
    let mut c = tokio::process::Command::new(donde(programa).unwrap_or_else(|| PathBuf::from(programa)));
    c.env("PATH", path());
    c
}

#[cfg(test)]
mod tests {
    use super::*;

    /// La unión no repite, respeta el orden y añade sólo lo que existe.
    #[test]
    fn unir_no_repite_y_respeta_el_orden() {
        let u = unir(Some("/a:/usr/bin:/b"), Some("/usr/bin:/c"));
        let partes: Vec<&str> = u.split(':').collect();
        assert_eq!(&partes[..4], ["/a", "/usr/bin", "/b", "/c"]);
        assert_eq!(partes.iter().filter(|p| **p == "/usr/bin").count(), 1);
        assert!(partes.contains(&"/bin"), "lo de siempre que existe entra");
        assert!(!partes.iter().any(|p| p.is_empty()));
        // sin shell ni herencia, al menos lo de siempre
        assert!(unir(None, None).contains("/usr/bin"));
    }

    /// Con un `PATH` mínimo —el de una app abierta desde el Dock— la shell de inicio de sesión
    /// devuelve el del usuario, y un `.zshrc` que imprime cosas no lo ensucia (marcas). Necesita la
    /// shell del usuario: en esta Mac, zsh con Homebrew y Cargo.
    #[test]
    fn la_shell_de_inicio_da_el_path_del_usuario() {
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into());
        let Some(p) = de_la_shell(&shell, Duration::from_secs(5)) else { return };
        assert!(!p.contains(A) && !p.contains(B) && !p.contains('\n'), "{p}");
        assert!(p.split(':').count() >= 4, "{p}");
        let casa = std::env::var("HOME").unwrap();
        if std::path::Path::new(&format!("{casa}/.cargo/bin")).is_dir() {
            assert!(p.contains(".cargo/bin"), "la shell de inicio trae Cargo: {p}");
        }
    }

    /// Una shell que se cuelga no detiene a GOTV: se mata al tope.
    #[test]
    fn una_shell_colgada_se_corta_al_tope() {
        let d = crate::CarpetaPrueba::nueva("gotv-entorno-colgada");
        let s = d.join("shell-lenta");
        std::fs::write(&s, "#!/bin/sh\nsleep 30\n").unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&s, std::fs::Permissions::from_mode(0o755)).unwrap();
        let t = std::time::Instant::now();
        assert!(de_la_shell(s.to_str().unwrap(), Duration::from_millis(300)).is_none());
        assert!(t.elapsed() < Duration::from_secs(3), "{:?}", t.elapsed());
    }

    /// `comando` lleva el PATH del usuario aunque el proceso tenga uno mínimo: un hijo lanzado con
    /// el PATH del Dock ve lo mismo que la terminal.
    #[test]
    fn un_hijo_ve_el_path_del_usuario() {
        let o = comando("sh").args(["-c", "printf %s \"$PATH\""]).env_remove("GOTV_X").output().unwrap();
        let visto = String::from_utf8_lossy(&o.stdout).to_string();
        assert_eq!(visto, path());
        assert!(donde("sh").is_some_and(|p| p.is_absolute()));
        assert!(donde("gotv-no-existe-este-programa").is_none());
        assert!(!en_path("gotv-no-existe-este-programa"));
    }
}

#[cfg(test)]
mod dock {
    /// A MANO, con el entorno de una app abierta desde el Dock (sin el PATH de la terminal):
    /// `env -i HOME=$HOME SHELL=$SHELL PATH=/usr/bin:/bin <binario de pruebas> --ignored con_el_path_del_dock`
    /// Antes de F0.2, `lsp::detect` no encontraba `rust-analyzer` así y la app no tenía servidor de lenguaje.
    #[test]
    #[ignore]
    fn con_el_path_del_dock_se_encuentra_el_servidor_de_lenguaje() {
        assert_eq!(std::env::var("PATH").as_deref(), Ok("/usr/bin:/bin"), "córrela con el PATH del Dock");
        assert!(super::path().contains(".cargo/bin"), "{}", super::path());
        let d = crate::CarpetaPrueba::nueva("gotv-entorno-dock");
        std::fs::create_dir_all(d.join("src")).unwrap();
        std::fs::write(d.join("Cargo.toml"), "[package]\nname = \"x\"\nversion = \"0.1.0\"\nedition = \"2021\"\n").unwrap();
        std::fs::write(d.join("src/main.rs"), "fn main() {}\n").unwrap();
        let s = crate::lsp::detect(&d).unwrap_or_else(|e| panic!("sin servidor de lenguaje con el PATH del Dock: {e:?}"));
        assert_eq!(s.cmd[0], "rust-analyzer");
        assert!(crate::entorno::donde("rust-analyzer").is_some_and(|p| p.to_string_lossy().contains(".cargo/bin") || p.to_string_lossy().contains("homebrew")));
    }
}
