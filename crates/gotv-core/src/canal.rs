//! F0.3 · Un solo cliente para los programas que hablan por mensajes JSON-RPC: servidores de
//! lenguaje (LSP, marco `Content-Length`), servidores MCP por stdio (una línea por mensaje) y, más
//! adelante, depuradores (DAP) y kernels. Antes `lsp.rs` y `mcp.rs` traían cada uno su lector, su
//! mapa de peticiones pendientes y su arranque, y tiraban el `stderr` del hijo a `/dev/null`.
//!
//! - Un lector en su tarea: las RESPUESTAS van a quien las espera; las notificaciones y las
//!   peticiones del otro lado salen por `eventos()`.
//! - Topes: un mensaje de más de 32 MiB o una cabecera de más de 64 KiB cuentan como caída (un
//!   hijo desbocado no se come la memoria). Con marco de línea, una línea que no es JSON se ignora
//!   (hay servidores MCP que escriben su registro por stdout); con `Content-Length`, una cabecera
//!   rota cuenta como caída (el flujo ya no se puede resincronizar).
//! - El `stderr` del hijo va a `.gotv/registros/<nombre>.log` (se rehace en cada arranque).
//! - Cuando el hijo cae, `vivo()` lo dice y toda petición pendiente vuelve con error al momento.
//! - `sobre(lector, escritor, …)` monta el mismo canal sobre extremos cualesquiera: así se prueba
//!   con un servidor dentro del proceso (`tokio::io::duplex`), sin lanzar nada.
//!
//! Lo que NO hace (y por qué): reiniciar al hijo. Volver a lanzarlo es fácil; dejarlo en el estado
//! en que estaba (la inicialización del protocolo, los archivos abiertos de un LSP) sólo lo sabe
//! quien lo usa: `lsp.rs` y `mcp.rs` miran `vivo()` y deciden.
use anyhow::{anyhow, bail, Result};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::{mpsc, oneshot, Mutex};

/// Cómo van enmarcados los mensajes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Marco {
    /// `Content-Length: N\r\n\r\n{json}` (LSP, DAP).
    ContentLength,
    /// Una línea por mensaje (MCP por stdio).
    Linea,
}

/// Qué protocolo va dentro del marco.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Protocolo {
    /// `{"jsonrpc":"2.0","id":…,"method":…}` (LSP, MCP).
    JsonRpc,
    /// F15.2 · el de los depuradores: `{"seq":…,"type":"request","command":…,"arguments":…}`; la respuesta
    /// trae `request_seq`, `success` y `body`; los `event` y las peticiones inversas salen por `eventos()`.
    Dap,
}

/// Tope de un mensaje.
pub const MAX_MENSAJE: usize = 32 * 1024 * 1024;
/// Tope de una cabecera `Content-Length`.
pub const MAX_CABECERA: usize = 64 * 1024;

type Escritor = Arc<Mutex<Box<dyn AsyncWrite + Send + Unpin>>>;
type Pendientes = Arc<std::sync::Mutex<HashMap<String, oneshot::Sender<Value>>>>;

/// Para contestar peticiones del otro lado desde otra tarea (un LSP pregunta por su configuración).
#[derive(Clone)]
pub struct Respondedor {
    escritor: Escritor,
    marco: Marco,
}

impl Respondedor {
    pub async fn responder(&self, id: Value, resultado: Value) -> Result<()> {
        escribir(&self.escritor, self.marco, &json!({"jsonrpc": "2.0", "id": id, "result": resultado})).await
    }

    /// F13.6: una petición del otro lado que no se atiende (método desconocido, rechazo).
    pub async fn error(&self, id: Value, codigo: i64, mensaje: &str) -> Result<()> {
        escribir(&self.escritor, self.marco, &json!({"jsonrpc": "2.0", "id": id, "error": {"code": codigo, "message": mensaje}})).await
    }
}

pub struct Canal {
    nombre: String,
    marco: Marco,
    protocolo: Protocolo,
    escritor: Escritor,
    pendientes: Pendientes,
    eventos: std::sync::Mutex<Option<mpsc::UnboundedReceiver<Value>>>,
    vivo: Arc<AtomicBool>,
    siguiente: AtomicU64,
    _hijo: Option<tokio::process::Child>,
}

async fn escribir(escritor: &Escritor, marco: Marco, v: &Value) -> Result<()> {
    let cuerpo = v.to_string();
    let mut w = escritor.lock().await;
    match marco {
        Marco::ContentLength => w.write_all(format!("Content-Length: {}\r\n\r\n{cuerpo}", cuerpo.len()).as_bytes()).await?,
        Marco::Linea => w.write_all(format!("{cuerpo}\n").as_bytes()).await?,
    }
    w.flush().await?;
    Ok(())
}

/// Lee un mensaje del flujo. `Ok(None)` = el otro lado cerró o el flujo se rompió (caída).
async fn leer_mensaje<R: AsyncRead + Unpin>(r: &mut BufReader<R>, marco: Marco) -> Option<Value> {
    match marco {
        Marco::Linea => loop {
            let mut linea = Vec::new();
            let n = (&mut *r).take(MAX_MENSAJE as u64 + 1).read_until(b'\n', &mut linea).await.ok()?;
            if n == 0 || linea.len() > MAX_MENSAJE {
                return None;
            }
            // ruido por stdout (un registro, un aviso): se ignora la línea, no se cae
            if let Ok(v) = serde_json::from_slice::<Value>(trim(&linea)) {
                if v.is_object() {
                    return Some(v);
                }
            }
        },
        Marco::ContentLength => {
            let mut largo: Option<usize> = None;
            let mut leidos = 0usize;
            loop {
                let mut l = String::new();
                let n = r.read_line(&mut l).await.ok()?;
                leidos += n;
                if n == 0 || leidos > MAX_CABECERA {
                    return None;
                }
                let t = l.trim();
                if t.is_empty() {
                    // una línea en blanco SIN cabecera antes es ruido (un servidor que escribe algo por
                    // stdout): se sigue leyendo; antes daba el canal por cerrado
                    if largo.is_some() {
                        break;
                    }
                    continue;
                }
                if let Some(v) = t.strip_prefix("Content-Length:") {
                    largo = Some(v.trim().parse().ok()?);
                }
            }
            let largo = largo.filter(|l| *l <= MAX_MENSAJE)?;
            let mut buf = vec![0u8; largo];
            r.read_exact(&mut buf).await.ok()?;
            // un cuerpo que no es JSON con la cabecera bien: se descarta ese mensaje, el flujo sigue
            Some(serde_json::from_slice::<Value>(&buf).unwrap_or(Value::Null))
        }
    }
}

fn trim(b: &[u8]) -> &[u8] {
    let ini = b.iter().position(|c| !c.is_ascii_whitespace()).unwrap_or(b.len());
    let fin = b.iter().rposition(|c| !c.is_ascii_whitespace()).map(|i| i + 1).unwrap_or(ini);
    &b[ini..fin.max(ini)]
}

/// La clave de un id en el mapa de pendientes (número o texto, como lo mande el otro lado).
fn clave(id: &Value) -> String {
    id.to_string()
}

impl Canal {
    /// Lanza `cmd` con stdin y stdout tomados por el canal y su stderr a `registro` (si se da).
    pub async fn lanzar(cmd: tokio::process::Command, marco: Marco, nombre: &str, registro: Option<PathBuf>) -> Result<Canal> {
        Canal::lanzar_con(cmd, marco, Protocolo::JsonRpc, nombre, registro).await
    }

    /// Igual, con el protocolo dado (F15.2: un adaptador de depuración habla DAP).
    pub async fn lanzar_con(mut cmd: tokio::process::Command, marco: Marco, protocolo: Protocolo, nombre: &str, registro: Option<PathBuf>) -> Result<Canal> {
        let err = match registro.as_ref().and_then(|r| {
            r.parent().map(std::fs::create_dir_all);
            std::fs::File::create(r).ok()
        }) {
            Some(f) => std::process::Stdio::from(f),
            None => std::process::Stdio::null(),
        };
        let mut hijo = cmd
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(err)
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| anyhow!("{nombre}: {e}"))?;
        let entrada = hijo.stdin.take().ok_or_else(|| anyhow!("{nombre}: stdin"))?;
        let salida = hijo.stdout.take().ok_or_else(|| anyhow!("{nombre}: stdout"))?;
        let mut c = Canal::sobre_con(salida, entrada, marco, protocolo, nombre);
        c._hijo = Some(hijo);
        Ok(c)
    }

    /// El canal sobre extremos cualesquiera (un hijo, un `duplex` en memoria, un socket).
    pub fn sobre<R, W>(lector: R, escritor: W, marco: Marco, nombre: &str) -> Canal
    where
        R: AsyncRead + Send + Unpin + 'static,
        W: AsyncWrite + Send + Unpin + 'static,
    {
        Canal::sobre_con(lector, escritor, marco, Protocolo::JsonRpc, nombre)
    }

    /// Igual, con el protocolo dado.
    pub fn sobre_con<R, W>(lector: R, escritor: W, marco: Marco, protocolo: Protocolo, nombre: &str) -> Canal
    where
        R: AsyncRead + Send + Unpin + 'static,
        W: AsyncWrite + Send + Unpin + 'static,
    {
        let pendientes: Pendientes = Arc::default();
        let vivo = Arc::new(AtomicBool::new(true));
        let (tx, rx) = mpsc::unbounded_channel();
        {
            let (pendientes, vivo) = (pendientes.clone(), vivo.clone());
            tokio::spawn(async move {
                let mut r = BufReader::new(lector);
                while let Some(m) = leer_mensaje(&mut r, marco).await {
                    // la respuesta a una petición nuestra, y por qué clave se la espera
                    let respuesta = match protocolo {
                        Protocolo::JsonRpc => (m.get("id").is_some() && m.get("method").is_none() && (m.get("result").is_some() || m.get("error").is_some())).then(|| clave(&m["id"])),
                        Protocolo::Dap => (m["type"] == "response").then(|| clave(&m["request_seq"])),
                    };
                    if let Some(k) = respuesta {
                        let quien = pendientes.lock().unwrap().remove(&k);
                        if let Some(q) = quien {
                            let _ = q.send(m);
                        }
                    } else if m.get("method").is_some() || (protocolo == Protocolo::Dap && matches!(m["type"].as_str(), Some("event" | "request"))) {
                        let _ = tx.send(m);
                    }
                }
                // caída: nadie se queda esperando una respuesta que no llegará
                vivo.store(false, Ordering::SeqCst);
                pendientes.lock().unwrap().clear();
            });
        }
        Canal {
            nombre: nombre.to_string(),
            marco,
            protocolo,
            escritor: Arc::new(Mutex::new(Box::new(escritor))),
            pendientes,
            eventos: std::sync::Mutex::new(Some(rx)),
            vivo,
            siguiente: AtomicU64::new(0),
            _hijo: None,
        }
    }

    pub fn vivo(&self) -> bool {
        self.vivo.load(Ordering::SeqCst)
    }

    /// Las notificaciones y peticiones del otro lado. Se entregan UNA vez (a quien las atiende).
    pub fn eventos(&self) -> Option<mpsc::UnboundedReceiver<Value>> {
        self.eventos.lock().unwrap().take()
    }

    /// F5 ML1: el PID del proceso (si el canal lanzó uno), para medir su memoria.
    pub fn pid(&self) -> Option<u32> {
        self._hijo.as_ref().and_then(|h| h.id())
    }

    pub fn respondedor(&self) -> Respondedor {
        Respondedor { escritor: self.escritor.clone(), marco: self.marco }
    }

    /// Una petición y su respuesta (el `result`). Un `error` del otro lado vuelve como error.
    pub async fn pedir(&self, metodo: &str, params: Value, tope: Duration) -> Result<Value> {
        if !self.vivo() {
            bail!("{}", crate::tr!("{}: se cerró", "{}: closed", self.nombre));
        }
        let id = self.siguiente.fetch_add(1, Ordering::SeqCst) + 1;
        let (tx, rx) = oneshot::channel();
        self.pendientes.lock().unwrap().insert(clave(&json!(id)), tx);
        let mut msg = match self.protocolo {
            Protocolo::JsonRpc => json!({"jsonrpc": "2.0", "id": id, "method": metodo}),
            Protocolo::Dap => json!({"seq": id, "type": "request", "command": metodo}),
        };
        if !params.is_null() {
            msg[if self.protocolo == Protocolo::Dap { "arguments" } else { "params" }] = params;
        }
        if let Err(e) = escribir(&self.escritor, self.marco, &msg).await {
            self.pendientes.lock().unwrap().remove(&clave(&json!(id)));
            return Err(e);
        }
        let m = match tokio::time::timeout(tope, rx).await {
            Err(_) => {
                self.pendientes.lock().unwrap().remove(&clave(&json!(id)));
                bail!("{}", crate::tr!("{}: «{metodo}» no respondió en {} s", "{}: «{metodo}» did not answer in {} s", self.nombre, tope.as_secs()))
            }
            Ok(Err(_)) => bail!("{}", crate::tr!("{}: se cerró mientras esperaba «{metodo}»", "{}: closed while waiting for «{metodo}»", self.nombre)),
            Ok(Ok(m)) => m,
        };
        if self.protocolo == Protocolo::Dap {
            if m["success"] == false {
                let msg = m["body"]["error"]["format"].as_str().or(m["message"].as_str()).unwrap_or("error");
                bail!("{} {metodo}: {msg}", self.nombre);
            }
            return Ok(m["body"].clone());
        }
        if let Some(e) = m.get("error") {
            bail!("{} {metodo}: {}", self.nombre, e["message"].as_str().unwrap_or("error"));
        }
        Ok(m["result"].clone())
    }

    /// F15.2 · contesta una petición INVERSA de un adaptador DAP (`runInTerminal`, `startDebugging`).
    pub async fn responder_dap(&self, peticion: &Value, exito: bool, cuerpo: Value) -> Result<()> {
        let seq = self.siguiente.fetch_add(1, Ordering::SeqCst) + 1;
        escribir(&self.escritor, self.marco, &json!({"seq": seq, "type": "response", "request_seq": peticion["seq"], "command": peticion["command"], "success": exito, "body": cuerpo})).await
    }

    /// Una notificación (sin respuesta). `params` nulo = sin `params`.
    pub async fn avisar(&self, metodo: &str, params: Value) -> Result<()> {
        let mut msg = json!({"jsonrpc": "2.0", "method": metodo});
        if !params.is_null() {
            msg["params"] = params;
        }
        escribir(&self.escritor, self.marco, &msg).await
    }

    /// Contesta una petición del otro lado.
    pub async fn responder(&self, id: Value, resultado: Value) -> Result<()> {
        self.respondedor().responder(id, resultado).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Un «servidor» en memoria: contesta `eco` con lo que recibe, manda una notificación y una
    /// petición propia antes de responder, y se cae con `caer`.
    fn servidor(marco: Marco) -> Canal {
        let (cliente, servidor) = tokio::io::duplex(1 << 16);
        let (sr, sw) = tokio::io::split(servidor);
        tokio::spawn(async move {
            let mut r = BufReader::new(sr);
            let w: Escritor = Arc::new(Mutex::new(Box::new(sw)));
            while let Some(m) = leer_mensaje(&mut r, marco).await {
                let Some(id) = m.get("id").cloned() else { continue };
                match m["method"].as_str() {
                    Some("eco") => {
                        let _ = escribir(&w, marco, &json!({"jsonrpc": "2.0", "method": "aviso", "params": {"n": 1}})).await;
                        let _ = escribir(&w, marco, &json!({"jsonrpc": "2.0", "id": "srv-1", "method": "pregunta", "params": {}})).await;
                        let _ = escribir(&w, marco, &json!({"jsonrpc": "2.0", "id": id, "result": m["params"].clone()})).await;
                    }
                    Some("mal") => {
                        let _ = escribir(&w, marco, &json!({"jsonrpc": "2.0", "id": id, "error": {"code": -1, "message": "no se puede"}})).await;
                    }
                    Some("lento") => {}
                    Some("caer") => return,
                    _ => {}
                }
            }
        });
        let (cr, cw) = tokio::io::split(cliente);
        Canal::sobre(cr, cw, marco, "prueba")
    }

    #[tokio::test]
    async fn pide_avisa_y_recibe_eventos_en_los_dos_marcos() {
        for marco in [Marco::Linea, Marco::ContentLength] {
            let c = servidor(marco);
            let mut ev = c.eventos().unwrap();
            assert!(c.eventos().is_none(), "los eventos se entregan una vez");
            let r = c.pedir("eco", json!({"hola": "ñandú"}), Duration::from_secs(5)).await.unwrap();
            assert_eq!(r, json!({"hola": "ñandú"}), "{marco:?}");
            // antes de la respuesta llegaron una notificación y una petición del otro lado
            assert_eq!(ev.recv().await.unwrap()["method"], "aviso");
            let p = ev.recv().await.unwrap();
            assert_eq!((p["method"].as_str(), p["id"].as_str()), (Some("pregunta"), Some("srv-1")));
            c.responder(p["id"].clone(), json!({"ok": true})).await.unwrap();
            let e = c.pedir("mal", Value::Null, Duration::from_secs(5)).await.unwrap_err().to_string();
            assert!(e.contains("no se puede") && e.contains("prueba"), "{e}");
            let t = c.pedir("lento", Value::Null, Duration::from_millis(100)).await.unwrap_err().to_string();
            assert!(t.contains("lento"), "{t}");
            assert!(c.vivo());
        }
    }

    /// Si el otro lado se cae, lo pendiente vuelve con error al momento (no al tope) y `vivo` lo dice.
    #[tokio::test]
    async fn la_caida_desbloquea_lo_pendiente() {
        let c = servidor(Marco::Linea);
        let t = std::time::Instant::now();
        let e = c.pedir("caer", Value::Null, Duration::from_secs(20)).await.unwrap_err().to_string();
        assert!(t.elapsed() < Duration::from_secs(5), "{:?}", t.elapsed());
        assert!(e.contains("se cerró") || e.contains("closed"), "{e}");
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(!c.vivo());
        assert!(c.pedir("eco", Value::Null, Duration::from_secs(1)).await.is_err());
    }

    /// Ruido por stdout con marco de línea: se ignora; una cabecera rota o un mensaje gigante con
    /// `Content-Length`: caída.
    #[tokio::test]
    async fn ruido_y_topes() {
        let (a, b) = tokio::io::duplex(1 << 16);
        let (_, mut bw) = tokio::io::split(b);
        bw.write_all(b"iniciando el servidor...\n{\"jsonrpc\":\"2.0\",\"method\":\"listo\"}\n").await.unwrap();
        let (ar, aw) = tokio::io::split(a);
        let c = Canal::sobre(ar, aw, Marco::Linea, "ruido");
        assert_eq!(c.eventos().unwrap().recv().await.unwrap()["method"], "listo");
        assert!(c.vivo());
        // Content-Length que pasa del tope
        let mut r = BufReader::new(std::io::Cursor::new(format!("Content-Length: {}\r\n\r\n{{}}", MAX_MENSAJE + 1).into_bytes()));
        assert!(leer_mensaje(&mut r, Marco::ContentLength).await.is_none());
        // cabecera sin fin
        let mut r = BufReader::new(std::io::Cursor::new(vec![b'x'; MAX_CABECERA + 10]));
        assert!(leer_mensaje(&mut r, Marco::ContentLength).await.is_none());
    }

    /// Lanzar un hijo de verdad: su stderr queda en el registro.
    #[tokio::test]
    async fn el_stderr_del_hijo_queda_en_su_registro() {
        let d = crate::CarpetaPrueba::nueva("gotv-canal");
        let reg = d.join(".gotv/registros/hijo.log");
        let mut cmd = tokio::process::Command::new("/bin/sh");
        cmd.args(["-c", "echo 'se rompió algo' >&2; read l; echo '{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":7}'"]);
        let c = Canal::lanzar(cmd, Marco::Linea, "hijo", Some(reg.clone())).await.unwrap();
        assert_eq!(c.pedir("x", Value::Null, Duration::from_secs(5)).await.unwrap(), json!(7));
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(std::fs::read_to_string(&reg).unwrap().contains("se rompió algo"));
    }
}
