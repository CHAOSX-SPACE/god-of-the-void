//! W3 — «GOTV en Chrome»: el puente LOCAL entre GOTV y su extensión (`extension-chrome/`).
//!
//! Chrome ── mensajería nativa (4 bytes de largo + JSON) ──> `gotv __chrome-host`
//!        ── socket de Unix `~/.config/gotv/chrome.sock` (líneas JSON) ──> GOTV
//!
//! Nada pasa por la nube. La extensión sólo actúa en las pestañas del grupo «GOTV», bloquea
//! bancos/pagos/correo/cuentas y jamás escribe en un campo de contraseña: esas reglas viven
//! en el navegador del usuario, no en el modelo.

use anyhow::{anyhow, Result};
use serde_json::Value;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

/// El nombre del host de mensajería nativa (el manifiesto de la extensión lo usa).
pub const HOST_NAME: &str = "space.chaosx.gotv";
/// La identidad fija de la extensión (de la clave pública de su manifiesto).
pub const EXTENSION_ID: &str = "ciamcpgldifbohhbdcpifphcmnbnkmoa";

/// La extensión de Chrome: en la app (`Contents/Resources`) o, en desarrollo, en el repo, subiendo
/// desde el ejecutable. Sin rutas de compilación grabadas: la app publicada no lleva carpetas de
/// quien la armó (lo vigila el juez de `gotv-empaquetar`).
pub fn carpeta_extension() -> PathBuf {
    let me = std::env::current_exe().unwrap_or_default();
    me.ancestors()
        .skip(1)
        .flat_map(|d| [d.join("Resources/extension-chrome"), d.join("extension-chrome")])
        .find(|d| d.join("manifest.json").exists())
        .unwrap_or_else(|| PathBuf::from("extension-chrome"))
}

pub fn socket_path() -> PathBuf {
    if let Some(p) = std::env::var_os("GOTV_CHROME_SOCK") {
        return PathBuf::from(p);
    }
    PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".config/gotv/chrome.sock")
}

// ── lado GOTV: servidor del puente ────────────────────────────────────────

type Pending = Arc<Mutex<HashMap<u64, tokio::sync::oneshot::Sender<Value>>>>;

struct Bridge {
    /// Escritor hacia el host conectado (el último gana).
    tx: Mutex<Option<tokio::sync::mpsc::UnboundedSender<String>>>,
    pending: Pending,
}

fn bridge() -> &'static Bridge {
    static B: OnceLock<Bridge> = OnceLock::new();
    B.get_or_init(|| Bridge {
        tx: Mutex::new(None),
        pending: Arc::new(Mutex::new(HashMap::new())),
    })
}

/// Arranca el servidor (una vez, en un hilo con runtime propio: sobrevive a quien lo pide).
pub fn ensure_server() -> Result<()> {
    static STARTED: OnceLock<std::result::Result<(), String>> = OnceLock::new();
    STARTED
        .get_or_init(|| {
            let path = socket_path();
            if let Some(d) = path.parent() {
                let _ = std::fs::create_dir_all(d);
            }
            let _ = std::fs::remove_file(&path);
            let l = std::os::unix::net::UnixListener::bind(&path)
                .map_err(|e| format!("{}: {e}", path.display()))?;
            // sólo el usuario puede hablar con su GOTV
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
            l.set_nonblocking(true).map_err(|e| e.to_string())?;
            std::thread::Builder::new()
                .name("gotv-chrome".into())
                .spawn(move || {
                    let rt = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .expect("chrome runtime");
                    rt.block_on(async move {
                        let l = tokio::net::UnixListener::from_std(l).expect("chrome listener");
                        loop {
                            let Ok((s, _)) = l.accept().await else {
                                continue;
                            };
                            let (r, mut w) = s.into_split();
                            let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
                            *bridge().tx.lock().unwrap() = Some(tx);
                            tokio::spawn(async move {
                                while let Some(line) = rx.recv().await {
                                    if w.write_all(line.as_bytes()).await.is_err()
                                        || w.write_all(b"\n").await.is_err()
                                    {
                                        break;
                                    }
                                }
                            });
                            let pending = bridge().pending.clone();
                            tokio::spawn(async move {
                                let mut lines = BufReader::new(r).lines();
                                while let Ok(Some(l)) = lines.next_line().await {
                                    let Ok(v) = serde_json::from_str::<Value>(&l) else {
                                        continue;
                                    };
                                    if let Some(id) = v["id"].as_u64() {
                                        if let Some(tx) = pending.lock().unwrap().remove(&id) {
                                            let _ = tx.send(v);
                                        }
                                    }
                                }
                            });
                        }
                    });
                })
                .map_err(|e| e.to_string())?;
            Ok(())
        })
        .clone()
        .map_err(|e| anyhow!("{e}"))
}

pub fn connected() -> bool {
    bridge()
        .tx
        .lock()
        .unwrap()
        .as_ref()
        .is_some_and(|t| !t.is_closed())
}

/// Manda una orden a la extensión y espera su respuesta.
pub async fn call(mut cmd: Value, timeout: Duration) -> Result<Value> {
    ensure_server()?;
    static ID: AtomicU64 = AtomicU64::new(1);
    let id = ID.fetch_add(1, Ordering::Relaxed);
    cmd["id"] = Value::from(id);
    let tx = bridge().tx.lock().unwrap().clone().filter(|t| !t.is_closed()).ok_or_else(|| {
        anyhow!("{}", crate::i18n::t(
            "la extensión «GOD OF THE VOID» de Chrome no está conectada: instálala (`gotv chrome install` y cárgala en chrome://extensions) y abre Chrome",
            "the «GOD OF THE VOID» Chrome extension is not connected: install it (`gotv chrome install` and load it in chrome://extensions) and open Chrome",
        ))
    })?;
    let (otx, orx) = tokio::sync::oneshot::channel();
    bridge().pending.lock().unwrap().insert(id, otx);
    tx.send(cmd.to_string()).map_err(|_| {
        anyhow!(
            "{}",
            crate::i18n::t("puente de Chrome cerrado", "Chrome bridge closed")
        )
    })?;
    let r = tokio::time::timeout(timeout, orx).await.map_err(|_| {
        bridge().pending.lock().unwrap().remove(&id);
        anyhow!(
            "{}",
            crate::i18n::t(
                "Chrome no respondió a tiempo",
                "Chrome did not answer in time"
            )
        )
    })??;
    if r["ok"].as_bool() == Some(true) {
        Ok(r["result"].clone())
    } else {
        Err(anyhow!(
            "{}",
            r["error"]
                .as_str()
                .unwrap_or(crate::i18n::t("error de la extensión", "extension error"))
        ))
    }
}

// ── lado Chrome: el host de mensajería nativa (`gotv __chrome-host`) ─────

/// Lee un mensaje de Chrome: 4 bytes (largo, little-endian nativo) + JSON.
pub fn read_native(r: &mut impl Read) -> std::io::Result<Option<Vec<u8>>> {
    let mut len = [0u8; 4];
    match r.read_exact(&mut len) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }
    let n = u32::from_ne_bytes(len) as usize;
    if n > 64 * 1024 * 1024 {
        return Err(std::io::Error::other("mensaje demasiado grande"));
    }
    let mut buf = vec![0u8; n];
    r.read_exact(&mut buf)?;
    Ok(Some(buf))
}

pub fn write_native(w: &mut impl Write, msg: &[u8]) -> std::io::Result<()> {
    w.write_all(&(msg.len() as u32).to_ne_bytes())?;
    w.write_all(msg)?;
    w.flush()
}

/// El relevo: Chrome (stdin/stdout, enmarcado) ⇄ GOTV (socket, líneas). Corre hasta que
/// Chrome cierre. Si GOTV no está abierto, reintenta cada 2 s.
pub fn run_host() -> std::io::Result<()> {
    let sock = loop {
        match std::os::unix::net::UnixStream::connect(socket_path()) {
            Ok(s) => break s,
            Err(_) => std::thread::sleep(Duration::from_secs(2)),
        }
    };
    let mut to_gotv = sock.try_clone()?;
    let from_gotv = sock;
    // GOTV → Chrome
    std::thread::spawn(move || {
        let mut out = std::io::stdout().lock();
        for line in std::io::BufReader::new(from_gotv)
            .lines()
            .map_while(Result::ok)
        {
            if write_native(&mut out, line.as_bytes()).is_err() {
                break;
            }
        }
        std::process::exit(0);
    });
    // Chrome → GOTV
    let mut input = std::io::stdin().lock();
    while let Some(msg) = read_native(&mut input)? {
        to_gotv.write_all(&msg)?;
        to_gotv.write_all(b"\n")?;
    }
    Ok(())
}

use std::io::BufRead;

// ── instalación ───────────────────────────────────────────────────────────

/// Escribe el lanzador del host y su manifiesto de mensajería nativa en `nm_dir`
/// (la carpeta `NativeMessagingHosts` del Chrome del usuario o de un perfil de prueba).
pub fn install_host(
    nm_dir: &std::path::Path,
    gotv_bin: &std::path::Path,
    extra_env: &[(&str, &str)],
) -> Result<PathBuf> {
    std::fs::create_dir_all(nm_dir)?;
    let launcher = nm_dir.join("gotv-chrome-host.sh");
    let env: String = extra_env
        .iter()
        .map(|(k, v)| format!("export {k}='{v}'\n"))
        .collect();
    std::fs::write(
        &launcher,
        format!(
            "#!/bin/sh\n{env}exec '{}' __chrome-host \"$@\"\n",
            gotv_bin.display()
        ),
    )?;
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&launcher, std::fs::Permissions::from_mode(0o755))?;
    let manifest = serde_json::json!({
        "name": HOST_NAME,
        "description": "GOD OF THE VOID — local bridge with its Chrome extension",
        "path": launcher,
        "type": "stdio",
        "allowed_origins": [format!("chrome-extension://{EXTENSION_ID}/")]
    });
    let f = nm_dir.join(format!("{HOST_NAME}.json"));
    std::fs::write(&f, serde_json::to_string_pretty(&manifest)?)?;
    Ok(f)
}

/// La carpeta de hosts nativos del Chrome del usuario.
pub fn user_nm_dir() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_default())
        .join("Library/Application Support/Google/Chrome/NativeMessagingHosts")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn el_marco_de_mensajeria_nativa_va_y_vuelve() {
        let mut buf = Vec::new();
        write_native(&mut buf, br#"{"id":1,"cmd":"ping"}"#).unwrap();
        assert_eq!(&buf[..4], &(21u32).to_ne_bytes());
        let mut r = std::io::Cursor::new(buf);
        assert_eq!(
            read_native(&mut r).unwrap().unwrap(),
            br#"{"id":1,"cmd":"ping"}"#
        );
        assert!(read_native(&mut r).unwrap().is_none(), "fin limpio");
    }

    #[test]
    fn el_manifiesto_del_host_apunta_a_gotv_y_a_la_extension() {
        let d = crate::CarpetaPrueba::sin_crear("gotv-nm");
        let f = install_host(
            &d,
            std::path::Path::new("/Apps/gotv"),
            &[("GOTV_CHROME_SOCK", "/tmp/x.sock")],
        )
        .unwrap();
        let m: Value = serde_json::from_str(&std::fs::read_to_string(&f).unwrap()).unwrap();
        assert_eq!(m["name"], HOST_NAME);
        assert_eq!(
            m["allowed_origins"][0],
            format!("chrome-extension://{EXTENSION_ID}/")
        );
        let sh = std::fs::read_to_string(d.join("gotv-chrome-host.sh")).unwrap();
        assert!(
            sh.contains("'/Apps/gotv' __chrome-host")
                && sh.contains("GOTV_CHROME_SOCK='/tmp/x.sock'")
        );
    }

    #[test]
    fn la_identidad_de_la_extension_sale_de_su_clave() {
        // el id de una extensión = sha256 de su clave pública, 32 «letras» a–p
        let man: Value =
            serde_json::from_str(include_str!("../../../extension-chrome/manifest.json")).unwrap();
        use base64::Engine;
        use sha2::Digest;
        let der = base64::engine::general_purpose::STANDARD
            .decode(man["key"].as_str().unwrap())
            .unwrap();
        let h = sha2::Sha256::digest(&der);
        let id: String = h
            .iter()
            .take(16)
            .flat_map(|b| [b >> 4, b & 15])
            .map(|n| (b'a' + n) as char)
            .collect();
        assert_eq!(id, EXTENSION_ID);
    }

    /// En VIVO y aislado (no corre por defecto): un Chrome aparte con perfil temporal que
    /// carga la extensión y el puente; el Chrome del usuario no se toca.
    #[tokio::test]
    #[ignore]
    async fn extension_en_vivo_en_un_chrome_aislado() {
        let chrome =
            std::path::Path::new(&std::env::var("GOTV_TEST_CHROME").unwrap_or_else(|_| {
                "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome".into()
            }))
            .to_path_buf();
        let base = crate::CarpetaPrueba::sin_crear("gotv-chrome-vivo");
        let profile = base.join("perfil");
        let sock = base.join("chrome.sock");
        std::fs::create_dir_all(&profile).unwrap();
        std::env::set_var("GOTV_CHROME_SOCK", &sock);
        let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .unwrap();
        let gotv = repo.join("target/debug/gotv");
        install_host(
            &profile.join("NativeMessagingHosts"),
            &gotv,
            &[("GOTV_CHROME_SOCK", &sock.to_string_lossy())],
        )
        .unwrap();
        ensure_server().unwrap();
        // una página local
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let html = "<!doctype html><meta charset=utf-8><title>GOTV vivo</title><input id=n placeholder=Nombre><input id=p type=password placeholder=Clave><button onclick=\"document.querySelector('p').textContent='Hola '+n.value\">Saludar</button><p></p>";
            for c in l.incoming().flatten() {
                let mut c = c;
                let mut b = [0u8; 2048];
                let _ = c.read(&mut b);
                let _ = c.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{html}", html.len()).as_bytes());
            }
        });
        // al salir (también si la prueba falla) muere TODO el grupo de Chrome
        struct Guarda(std::process::Child);
        impl Drop for Guarda {
            fn drop(&mut self) {
                unsafe {
                    libc::killpg(self.0.id() as i32, libc::SIGKILL);
                }
                let _ = self.0.wait(); // sin procesos zombis
                                       // el crashpad de Chrome for Testing se separa del grupo: también se cierra
                let _ = std::process::Command::new("pkill")
                    .args(["-f", "Chrome for Testing.*chrome_crashpad_handler"])
                    .status();
            }
        }
        use std::os::unix::process::CommandExt;
        let child = std::process::Command::new(&chrome)
            .args([
                format!("--user-data-dir={}", profile.display()),
                format!(
                    "--load-extension={}",
                    repo.join("extension-chrome").display()
                ),
                "--disable-features=DisableLoadExtensionCommandLineSwitch".into(),
                "--headless=new".into(),
                "--no-first-run".into(),
                "--no-default-browser-check".into(),
                "about:blank".into(),
            ])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .process_group(0)
            .spawn()
            .unwrap();
        let _guarda = Guarda(child);
        let mut ok = false;
        for _ in 0..60 {
            if connected() {
                ok = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
        let result = async {
            assert!(ok, "la extensión no se conectó al puente en 30 s");
            assert_eq!(
                call(serde_json::json!({"cmd": "ping"}), Duration::from_secs(10))
                    .await
                    .unwrap(),
                "pong"
            );
            let t = call(
                serde_json::json!({"cmd": "new_tab", "url": format!("http://127.0.0.1:{port}/")}),
                Duration::from_secs(20),
            )
            .await
            .unwrap();
            let tab = t["tab"].as_i64().unwrap();
            tokio::time::sleep(Duration::from_secs(2)).await;
            let snap = call(
                serde_json::json!({"cmd": "snapshot", "tab": tab}),
                Duration::from_secs(20),
            )
            .await
            .unwrap();
            let snap = snap.as_str().unwrap().to_string();
            let refs: Vec<String> = snap
                .lines()
                .filter_map(|l| {
                    l.split("[ref=")
                        .nth(1)
                        .map(|r| r.trim_end_matches(']').to_string())
                })
                .collect();
            assert!(refs.len() >= 3, "{snap}");
            let line_ref = |role: &str, name: &str| {
                snap.lines()
                    .find(|l| l.contains(role) && l.contains(name))
                    .and_then(|l| l.split("[ref=").nth(1))
                    .map(|r| r.trim_end_matches(']').to_string())
                    .unwrap()
            };
            let campo = line_ref("textbox", "Nombre");
            let clave = line_ref("textbox", "Clave");
            let boton = line_ref("button", "Saludar");
            call(
                serde_json::json!({"cmd": "type", "tab": tab, "ref": campo, "text": "Vacío"}),
                Duration::from_secs(10),
            )
            .await
            .unwrap();
            let e = call(
                serde_json::json!({"cmd": "type", "tab": tab, "ref": clave, "text": "secreto"}),
                Duration::from_secs(10),
            )
            .await;
            assert!(
                e.unwrap_err().to_string().contains("contraseña"),
                "jamás escribe contraseñas"
            );
            call(
                serde_json::json!({"cmd": "click", "tab": tab, "ref": boton}),
                Duration::from_secs(10),
            )
            .await
            .unwrap();
            let txt = call(
                serde_json::json!({"cmd": "text", "tab": tab}),
                Duration::from_secs(10),
            )
            .await
            .unwrap();
            assert!(txt.as_str().unwrap().contains("Hola Vacío"), "{txt}");
            let b = call(
                serde_json::json!({"cmd": "new_tab", "url": "https://www.bancolombia.com/"}),
                Duration::from_secs(10),
            )
            .await;
            assert!(
                b.unwrap_err().to_string().contains("bloqueado"),
                "bancos bloqueados"
            );
            let tabs = call(serde_json::json!({"cmd": "tabs"}), Duration::from_secs(10))
                .await
                .unwrap();
            assert_eq!(
                tabs.as_array().unwrap().len(),
                1,
                "sólo las del grupo GOTV: {tabs}"
            );
            // prueba NEGATIVA: cualquier otra pestaña (la about:blank del arranque) es inalcanzable
            let mut fuera = 0;
            // los números de pestaña pueden ser grandes: se miran las 20 anteriores
            for other in (tab - 20).max(1)..tab {
                if let Err(e) = call(
                    serde_json::json!({"cmd": "text", "tab": other}),
                    Duration::from_secs(5),
                )
                .await
                {
                    let e = e.to_string();
                    assert!(e.contains("grupo") || e.contains("No tab"), "{e}");
                    if e.contains("grupo") {
                        fuera += 1;
                    }
                } else {
                    panic!("la pestaña {other} (fuera del grupo) respondió");
                }
            }
            assert!(
                fuera >= 1,
                "había al menos una pestaña fuera del grupo y se negó"
            );
        }
        .await;
        let _ = std::fs::remove_dir_all(&base);
        result
    }
}
