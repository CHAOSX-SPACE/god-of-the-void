//! W1 — Navegador propio: agent-browser de Vercel Labs (Apache-2.0, vendorizado en
//! `terceros/agent-browser`, compilado y empaquetado JUNTO a GOTV, como el ayudante de
//! agent-desktop). La herramienta `browser` le pasa un comando y devuelve su salida.
//!
//! - Una SESIÓN por chat (`AGENT_BROWSER_SESSION`): dos chats no se pisan la página.
//! - Usa el Chrome instalado con un perfil TEMPORAL y aislado (nunca el del usuario: sus
//!   sesiones, contraseñas y pestañas no se tocan); si no hay Chrome, `install`.
//! - Al cerrarse el chat, su navegador se cierra.

use anyhow::{anyhow, Result};
use std::path::{Path, PathBuf};
use std::time::Duration;

const MAX_OUT: usize = 24_000;

/// Dónde está el ejecutable: junto a GOTV (la app), en la compilación, o en el PATH.
pub fn binary() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("GOTV_AGENT_BROWSER")
        .map(PathBuf::from)
        .filter(|p| p.is_file())
    {
        return Some(p);
    }
    // junto a GOTV (la app) o, en desarrollo, subiendo desde el ejecutable hasta el repo; sin rutas
    // de compilación grabadas (la app publicada no lleva las carpetas de quien la armó)
    if let Ok(exe) = std::env::current_exe() {
        let junto = exe.parent().map(|d| d.join("agent-browser"));
        let repo = exe
            .ancestors()
            .skip(1)
            .map(|d| d.join("terceros/agent-browser/cli/target/release/agent-browser"));
        if let Some(c) = junto.into_iter().chain(repo).find(|c| c.is_file()) {
            return Some(c);
        }
    }
    // F0.2: en el PATH del usuario (desde el Dock, `which` no veía Homebrew ni Cargo)
    crate::entorno::donde("agent-browser")
}

/// La sesión de un chat: su dueño de procesos (proyecto#chat) → nombre corto y estable.
pub fn session_for(owner: &str) -> String {
    let chat = owner.rsplit('#').next().unwrap_or(owner);
    let clean: String = chat
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect();
    format!("gotv-{clean}")
}

/// ¿Este comando abre una dirección de FUERA (no localhost)? → cuenta como red.
pub fn external_target(args: &[String]) -> Option<String> {
    let first = args.iter().find(|a| !a.starts_with('-'))?.as_str();
    if !matches!(first, "open" | "goto" | "navigate" | "tab") {
        return None;
    }
    let url = args
        .iter()
        .skip_while(|a| a.as_str() != first)
        .skip(1)
        .find(|a| !a.starts_with('-'))?;
    let host = url
        .split("://")
        .nth(1)
        .unwrap_or(url)
        .split(['/', ':', '?', '#'])
        .next()
        .unwrap_or("")
        .to_lowercase();
    let local = host.is_empty()
        || matches!(
            host.as_str(),
            "localhost" | "127.0.0.1" | "0.0.0.0" | "[::1]"
        )
        || host.ends_with(".localhost")
        || url.starts_with("file:")
        || url.starts_with("about:")
        || url.starts_with("data:");
    (!local).then(|| url.clone())
}

fn used() -> &'static std::sync::Mutex<std::collections::HashSet<String>> {
    static U: std::sync::OnceLock<std::sync::Mutex<std::collections::HashSet<String>>> =
        std::sync::OnceLock::new();
    U.get_or_init(Default::default)
}

pub async fn run(
    owner: &str,
    root: &Path,
    args: &[String],
    stdin: Option<&str>,
    timeout_s: u64,
) -> Result<String> {
    if args.is_empty() {
        return Err(anyhow!(
            "{}",
            crate::i18n::t(
                "'args' vacío: p. ej. [\"open\",\"http://localhost:5173\"]",
                "empty 'args': e.g. [\"open\",\"http://localhost:5173\"]"
            )
        ));
    }
    let bin = binary().ok_or_else(|| anyhow!("{}", crate::i18n::t(
        // banco 2026-11-17: el consejo de compilar mandó al modelo a `find /` por todo el disco; el
        // mensaje es para el MODELO: no hay nada que buscar, sigue sin navegador y avisa al usuario
        "el navegador de GOTV no está en esta instalación: no lo busques en el disco ni intentes instalarlo. Sigue sin él (web_fetch lee páginas) y, si de verdad lo necesitas, dile al usuario que reinstale la app de GOTV",
        "GOTV's browser is not in this installation: do not search the disk for it or try to install it. Carry on without it (web_fetch reads pages) and, if you truly need it, tell the user to reinstall the GOTV app",
    )))?;
    used().lock().unwrap().insert(session_for(owner));
    let mut c = tokio::process::Command::new(&bin);
    c.args(args)
        .current_dir(root)
        .env("AGENT_BROWSER_SESSION", session_for(owner))
        .env("AGENT_BROWSER_SOCKET_DIR", carpeta_sockets())
        // el daemon se cierra solo si nadie lo usa en 30 min
        .env("AGENT_BROWSER_IDLE_TIMEOUT_MS", "1800000")
        .stdin(if stdin.is_some() {
            std::process::Stdio::piped()
        } else {
            std::process::Stdio::null()
        })
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    let mut child = c.spawn().map_err(|e| anyhow!("agent-browser: {e}"))?;
    if let (Some(text), Some(mut w)) = (stdin, child.stdin.take()) {
        use tokio::io::AsyncWriteExt;
        let _ = w.write_all(text.as_bytes()).await;
    }
    let o = tokio::time::timeout(
        Duration::from_secs(timeout_s.clamp(5, 600)),
        child.wait_with_output(),
    )
    .await
    .map_err(|_| {
        anyhow!(
            "{}",
            crate::tr!(
                "el navegador pasó de {timeout_s} s",
                "the browser went past {timeout_s} s"
            )
        )
    })??;
    let mut out = String::from_utf8_lossy(&o.stdout).to_string();
    let err = String::from_utf8_lossy(&o.stderr);
    if !err.trim().is_empty() {
        out.push_str("\n[stderr]\n");
        out.push_str(err.trim_end());
    }
    if out.chars().count() > MAX_OUT {
        out = out.chars().take(MAX_OUT).collect::<String>()
            + crate::i18n::t(
                "\n…[recortado: usa snapshot -i o --selector para acotar]",
                "\n…[truncated: use snapshot -i or --selector to narrow it]",
            );
    }
    if o.status.success() {
        Ok(out.trim_end().to_string())
    } else if out.trim().is_empty() {
        // medido (D5): `click @e43` falló 15 veces con el error VACÍO y el modelo no supo por qué
        let cmd = args.join(" ");
        let pista = if args.iter().any(|a| a.starts_with("@e")) {
            crate::i18n::t(
                "esa referencia @e ya no existe o no se puede usar (las @e cambian cada vez que la página cambia): toma un `snapshot -i` NUEVO y usa sus referencias",
                "that @e reference no longer exists or cannot be used (@e refs change every time the page changes): take a NEW `snapshot -i` and use its refs",
            )
        } else {
            crate::i18n::t(
                "revisa el comando o toma un `snapshot -i` para ver el estado real de la página",
                "check the command or take a `snapshot -i` to see the page's real state",
            )
        };
        Err(anyhow!(
            "{}",
            crate::tr!(
                "el navegador falló sin decir por qué ({}) en «{cmd}»: {pista}",
                "the browser failed without saying why ({}) on «{cmd}»: {pista}",
                o.status
            )
        ))
    } else {
        // con stdout vacío la causa vive en stderr: que sea la PRIMERA línea (el registro sólo
        // muestra ésa y se veía «ERROR: » como si el fallo fuera mudo)
        Err(anyhow!(
            "{}",
            out.trim().trim_start_matches("[stderr]").trim_start()
        ))
    }
}

/// La carpeta de los sockets de agent-browser. Medido 2026-11-18 (banco F7): con un `HOME` largo el socket
/// (`~/.agent-browser/<sesión>.sock`) pasaba de los 103 bytes que admite macOS y el navegador NO arrancaba
/// («Session name is too long»; el modelo gastó 40 pasos en eso). La temporal PROPIA del usuario
/// (`/var/folders/…/T/`, privada, ≈50 bytes) siempre cabe. Respeta `AGENT_BROWSER_SOCKET_DIR` si ya viene.
pub fn carpeta_sockets() -> PathBuf {
    if let Some(d) = std::env::var_os("AGENT_BROWSER_SOCKET_DIR").filter(|d| !d.is_empty()) {
        return PathBuf::from(d);
    }
    let d = std::env::temp_dir().join("gotv-ab");
    let _ = std::fs::create_dir_all(&d);
    d
}

/// Al salir por una señal (SIGTERM, Ctrl+C doble): cierra TODOS los navegadores abiertos.
/// Medido (D1, 2026-11-09): un `gotv` matado con SIGTERM dejó 15 procesos de Chrome huérfanos
/// con su carpeta de trabajo en el proyecto.
pub fn close_all() {
    let sesiones: Vec<String> = used().lock().unwrap().drain().collect();
    let Some(bin) = binary() else { return };
    for s in sesiones {
        let _ = std::process::Command::new(&bin)
            .args(["close"])
            .env("AGENT_BROWSER_SESSION", &s)
            .env("AGENT_BROWSER_SOCKET_DIR", carpeta_sockets())
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
        let _ = std::fs::remove_file(carpeta_sockets().join(format!("{s}.target")));
    }
}

/// Al cerrarse el chat: su navegador también (sin esperar).
pub fn close_session(chat_id: &str) {
    // sólo si este chat abrió un navegador
    if !used().lock().unwrap().remove(&session_for(chat_id)) {
        return;
    }
    // el «.target» que agent-browser guarda por sesión sobreviviría para siempre (una sesión
    // por chat): se borra al cerrar el chat
    let target = carpeta_sockets().join(format!("{}.target", session_for(chat_id)));
    // síncrono (≈100 ms): en un hilo aparte no alcanzaba a correr si GOTV salía justo después
    if let Some(bin) = binary() {
        let _ = std::process::Command::new(bin)
            .args(["close"])
            .env("AGENT_BROWSER_SESSION", session_for(chat_id))
            .env("AGENT_BROWSER_SOCKET_DIR", carpeta_sockets())
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }
    let _ = std::fs::remove_file(&target);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn una_sesion_por_chat_y_estable() {
        assert_eq!(session_for("/p/proy#chat-123-4"), "gotv-chat-123-4");
        assert_eq!(
            session_for("/otro/proy#chat-123-4"),
            session_for("/p/proy#chat-123-4"),
            "depende del chat, no de la ruta"
        );
        assert_ne!(session_for("/p#chat-1"), session_for("/p#chat-2"));
    }

    #[test]
    fn abrir_fuera_cuenta_como_red_y_localhost_no() {
        let a = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            external_target(&a(&["open", "http://localhost:5173"])),
            None
        );
        assert_eq!(
            external_target(&a(&["open", "http://127.0.0.1:8000/x"])),
            None
        );
        assert_eq!(
            external_target(&a(&["--headed", "open", "https://github.com/x"])),
            Some("https://github.com/x".into())
        );
        assert_eq!(external_target(&a(&["snapshot", "-i"])), None);
    }

    /// Con el binario de verdad (si está compilado): abrir un servidor local, leer por ref,
    /// actuar, leer la consola y cerrar; cada chat en su sesión.
    #[tokio::test]
    async fn el_navegador_de_verdad_por_la_herramienta() {
        if binary().is_none() || !Path::new("/Applications/Google Chrome.app").exists() {
            return;
        }
        let d = crate::CarpetaPrueba::sin_crear("gotv-nav");
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("index.html"), "<!doctype html><meta charset=utf-8><title>GOTV</title><input placeholder=Nombre><button onclick=\"document.querySelector('p').textContent='Hola '+document.querySelector('input').value\">Saludar</button><p></p>").unwrap();
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let html = std::fs::read_to_string(d.join("index.html")).unwrap();
        std::thread::spawn(move || {
            use std::io::{Read, Write};
            for c in l.incoming().flatten().take(20) {
                let mut c = c;
                let mut b = [0u8; 1024];
                let _ = c.read(&mut b);
                let _ = c.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", html.len(), html).as_bytes());
            }
        });
        let yo = format!("/p#chat-prueba-nav-{}", std::process::id());
        let a = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let r = run(
            &yo,
            &d,
            &a(&["open", &format!("http://127.0.0.1:{port}/")]),
            None,
            60,
        )
        .await
        .unwrap();
        assert!(r.contains("GOTV"), "{r}");
        let snap = run(&yo, &d, &a(&["snapshot", "-i"]), None, 30)
            .await
            .unwrap();
        assert!(
            snap.contains("textbox") && snap.contains("button"),
            "{snap}"
        );
        let field = snap
            .lines()
            .find(|l| l.contains("textbox"))
            .and_then(|l| l.split("ref=").nth(1))
            .map(|r| format!("@{}", r.trim_end_matches(']')))
            .unwrap();
        let btn = snap
            .lines()
            .find(|l| l.contains("button"))
            .and_then(|l| l.split("ref=").nth(1))
            .map(|r| format!("@{}", r.trim_end_matches(']')))
            .unwrap();
        run(&yo, &d, &a(&["fill", &field, "Vacío"]), None, 30)
            .await
            .unwrap();
        run(&yo, &d, &a(&["click", &btn]), None, 30).await.unwrap();
        let t = run(&yo, &d, &a(&["get", "text", "p"]), None, 30)
            .await
            .unwrap();
        assert!(t.contains("Hola Vacío"), "{t}");
        close_session(&yo);
    }
}
