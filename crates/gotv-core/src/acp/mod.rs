//! F10.3 · `gotv acp`: GOTV para los editores que hablan el **Agent Client Protocol** (ACP), por stdio y en
//! Rust: sin escribir una extensión en otro lenguaje (L1).
//!
//! **Versión del protocolo (leída de agentclientprotocol.com el 2026-11-17):** `protocolVersion` = **1**
//! (entero; se sube sólo con cambios que rompen). Transporte: JSON-RPC 2.0, UTF-8, **un mensaje por línea**
//! (`\n`, sin saltos dentro); la salida estándar ES el canal (el registro va a `stderr`). Mensajes que usa
//! GOTV, con sus nombres exactos:
//!
//! | Del editor | GOTV responde |
//! |---|---|
//! | `initialize` (`protocolVersion`, `clientCapabilities.fs.readTextFile/writeTextFile`, `clientInfo`) | `protocolVersion`, `agentCapabilities` (`loadSession: false`, `promptCapabilities.embeddedContext: true`), `agentInfo`, `authMethods: []` |
//! | `authenticate` | `{}` (GOTV no pide cuenta: corre en esta Mac) |
//! | `session/new` (`cwd`, `mcpServers`) | `sessionId` (un chat = una `Session` de GOTV en ese proyecto) |
//! | `session/prompt` (`sessionId`, `prompt`: `text`, `resource_link`, `resource`) | `stopReason`: `end_turn` · `cancelled` |
//! | `session/cancel` (aviso) | corta la orden; la respuesta pendiente sale con `cancelled` |
//!
//! | De GOTV al editor | Cuándo |
//! |---|---|
//! | `session/update` · `agent_message_chunk` / `agent_thought_chunk` | texto y razonamiento en vivo |
//! | `session/update` · `tool_call` / `tool_call_update` (`toolCallId`, `title`, `kind`, `status`, `content`, `locations`, `rawInput`) | cada herramienta, con su resultado o el `diff` (`path`, `oldText`, `newText`) de lo editado |
//! | `session/request_permission` (`toolCall`, `options`: `allow_once` · `allow_always` · `reject_once` · `reject_always`) | lo que GOTV pregunta (el comando entero o el diff); `cancelled` o lo que no se entienda = **NEGADO** |
//! | `fs/read_text_file` · `fs/write_text_file` | si el editor los ofrece: las herramientas de archivos leen su búfer sin guardar y escriben por él |
//!
//! **Trampas que se cumplen:** nada sale por stdout que no sea del protocolo (ni un `println!`); una línea
//! que no es JSON no tumba el proceso (el canal la salta); el motor local atiende UNA orden a la vez: la
//! segunda se encola y se le dice al editor; un permiso que el editor no sabe mostrar se niega. Los permisos
//! son los del proyecto, como en la ventana (el editor no abre ninguna puerta más).
//!
//! **No hace (dicho):** `session/load` (no se anuncia), modos (`session/set_mode`), los `mcpServers` que mande
//! el editor (GOTV usa los suyos y los del proyecto; se avisa por stderr), imágenes y audio en el prompt.
use crate::agent::Agent;
use crate::canal::{Canal, Marco};
use anyhow::Result;
use futures_util::future::BoxFuture;
use gotv_engine::{Capabilities, ChatRequest, ChatResponse, Engine, Precarga, StreamEvent};
use serde_json::{json, Value};
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

pub mod archivos;
mod frente;
/// `gotv acp --guion <archivo>`: probar un editor de verdad sin modelo
pub mod guion;
pub use frente::FrenteAcp;

/// La versión del protocolo que habla GOTV.
pub const VERSION: u64 = 1;

/// Hace el agente de un chat nuevo del editor en `cwd` (la CLI, con su configuración y el motor de la Mac).
pub type Fabrica<E> = Arc<dyn Fn(PathBuf, FrenteAcp) -> BoxFuture<'static, Result<Agent<E, FrenteAcp>>> + Send + Sync>;

/// UN motor para todos los chats del editor (el modelo se carga una vez): cada chat tiene su asa; las
/// peticiones van de a una.
pub struct Compartido<E: Engine> {
    motor: Arc<tokio::sync::Mutex<E>>,
    nombre: String,
    caps: Capabilities,
    local: bool,
}

impl<E: Engine> Clone for Compartido<E> {
    fn clone(&self) -> Self {
        Compartido { motor: self.motor.clone(), nombre: self.nombre.clone(), caps: self.caps, local: self.local }
    }
}

impl<E: Engine> Compartido<E> {
    pub fn new(e: E) -> Self {
        Compartido { nombre: e.name(), caps: e.capabilities(), local: e.local(), motor: Arc::new(tokio::sync::Mutex::new(e)) }
    }
}

#[async_trait::async_trait]
impl<E: Engine> Engine for Compartido<E> {
    fn name(&self) -> String {
        self.nombre.clone()
    }
    fn capabilities(&self) -> Capabilities {
        self.caps
    }
    async fn start(&mut self) -> Result<()> {
        self.motor.lock().await.start().await
    }
    async fn complete(&self, req: ChatRequest) -> Result<ChatResponse> {
        self.motor.lock().await.complete(req).await
    }
    async fn complete_stream(&self, req: ChatRequest, on: &(dyn Fn(StreamEvent) + Send + Sync)) -> Result<ChatResponse> {
        self.motor.lock().await.complete_stream(req, on).await
    }
    async fn stop(&mut self) -> Result<()> {
        self.motor.lock().await.stop().await
    }
    fn local(&self) -> bool {
        self.local
    }
    async fn precargar(&self, req: ChatRequest, on: &(dyn Fn(StreamEvent) + Send + Sync)) -> Result<Precarga> {
        self.motor.lock().await.precargar(req, on).await
    }
}

struct Chat<E: Engine + 'static> {
    agente: Arc<tokio::sync::Mutex<Agent<E, FrenteAcp>>>,
    cancel: Arc<AtomicBool>,
}

struct Pedido {
    id: Value,
    sesion: String,
    texto: String,
}

struct Servidor<E: Engine + 'static> {
    canal: Arc<Canal>,
    fabrica: Fabrica<E>,
    chats: Mutex<HashMap<String, Chat<E>>>,
    /// `clientCapabilities` del `initialize`
    caps: Arc<Mutex<Value>>,
    /// la cola y si el motor está ocupado, bajo el MISMO candado (una orden nunca queda varada en la cola)
    cola: Mutex<(bool, VecDeque<Pedido>)>,
    n: AtomicU64,
    /// cuántas órdenes terminaron (para las pruebas y el registro)
    hechas: AtomicU64,
}

/// El texto de una orden a partir de sus bloques (`text`, `resource_link` → `@ruta`, `resource` → su texto).
pub fn texto_de_prompt(bloques: &[Value]) -> (String, Vec<String>) {
    let mut t = String::new();
    let mut avisos = Vec::new();
    for b in bloques {
        match b["type"].as_str() {
            Some("text") => t.push_str(b["text"].as_str().unwrap_or("")),
            Some("resource_link") => {
                let uri = b["uri"].as_str().unwrap_or("");
                t.push_str(&format!(" @{}", uri.strip_prefix("file://").unwrap_or(uri)));
            }
            Some("resource") => {
                let r = &b["resource"];
                let uri = r["uri"].as_str().unwrap_or("");
                t.push_str(&format!("\n\n[{}]\n{}\n", uri.strip_prefix("file://").unwrap_or(uri), r["text"].as_str().unwrap_or("")));
            }
            Some(otro) => avisos.push(crate::tr!("bloque «{otro}» en la orden: GOTV no lo anuncia y lo ignora", "block «{otro}» in the prompt: GOTV does not advertise it and ignores it")),
            None => {}
        }
    }
    (t.trim().to_string(), avisos)
}

/// Sirve el protocolo por `lector`/`escritor` (stdin/stdout en `gotv acp`; un `duplex` en las pruebas) hasta
/// que el editor cierre. Al terminar, cierra cada chat (`SessionEnd`).
pub async fn servir<E, R, W>(lector: R, escritor: W, fabrica: Fabrica<E>) -> Result<()>
where
    E: Engine + 'static,
    R: tokio::io::AsyncRead + Send + Unpin + 'static,
    W: tokio::io::AsyncWrite + Send + Unpin + 'static,
{
    let canal = Arc::new(Canal::sobre(lector, escritor, Marco::Linea, "ACP"));
    let mut entrada = canal.eventos().ok_or_else(|| gotv_engine::anyhow_tr!("ACP: el canal no da eventos", "ACP: the channel gives no events"))?;
    let srv = Arc::new(Servidor { canal, fabrica, chats: Mutex::new(HashMap::new()), caps: Arc::new(Mutex::new(json!({}))), cola: Mutex::new((false, VecDeque::new())), n: AtomicU64::new(0), hechas: AtomicU64::new(0) });
    while let Some(m) = entrada.recv().await {
        atender(&srv, m).await;
    }
    // el editor cerró: cada chat termina con su SessionEnd
    let chats: Vec<_> = srv.chats.lock().unwrap().drain().map(|(_, c)| c).collect();
    for c in chats {
        c.cancel.store(true, Ordering::Relaxed);
        c.agente.lock().await.cerrar_sesion("other").await;
    }
    Ok(())
}

async fn atender<E: Engine + 'static>(srv: &Arc<Servidor<E>>, m: Value) {
    let metodo = m["method"].as_str().unwrap_or("").to_string();
    let id = m.get("id").cloned();
    let p = &m["params"];
    let resp = srv.canal.respondedor();
    match (metodo.as_str(), id) {
        ("initialize", Some(id)) => {
            *srv.caps.lock().unwrap() = p["clientCapabilities"].clone();
            // si pide la que habla GOTV, se repite; si no, la última que GOTV sabe
            let _ = resp
                .responder(
                    id,
                    json!({
                        "protocolVersion": VERSION,
                        "agentCapabilities": {"loadSession": false, "promptCapabilities": {"image": false, "audio": false, "embeddedContext": true}, "mcpCapabilities": {"http": false, "sse": false}},
                        "agentInfo": {"name": "gotv", "title": "GOD OF THE VOID", "version": env!("CARGO_PKG_VERSION")},
                        "authMethods": [],
                    }),
                )
                .await;
        }
        ("authenticate", Some(id)) => {
            let _ = resp.responder(id, json!({})).await;
        }
        ("session/new", Some(id)) => {
            let cwd = PathBuf::from(p["cwd"].as_str().unwrap_or(""));
            if !cwd.is_absolute() || !cwd.is_dir() {
                let _ = resp.error(id, -32602, &crate::tr!("cwd debe ser una carpeta absoluta que exista: {}", "cwd must be an existing absolute folder: {}", cwd.display())).await;
                return;
            }
            if p["mcpServers"].as_array().is_some_and(|a| !a.is_empty()) {
                eprintln!("{}", crate::i18n::t("· ACP: GOTV usa sus servidores MCP y los del proyecto; los que manda el editor no se arrancan", "· ACP: GOTV uses its own and the project's MCP servers; the ones sent by the editor are not started"));
            }
            let sesion = format!("gotv-{}-{}", std::process::id(), srv.n.fetch_add(1, Ordering::Relaxed) + 1);
            let front = FrenteAcp::new(srv.canal.clone(), &sesion, srv.caps.clone(), &cwd);
            match (srv.fabrica)(cwd, front).await {
                Ok(a) => {
                    // la bandera del EDITOR (la del agente la repone `run`; un vigía las une mientras corre)
                    let cancel = Arc::new(AtomicBool::new(false));
                    srv.chats.lock().unwrap().insert(sesion.clone(), Chat { agente: Arc::new(tokio::sync::Mutex::new(a)), cancel });
                    let _ = resp.responder(id, json!({"sessionId": sesion})).await;
                }
                Err(e) => {
                    let _ = resp.error(id, -32603, &format!("{e:#}")).await;
                }
            }
        }
        ("session/prompt", Some(id)) => {
            let sesion = p["sessionId"].as_str().unwrap_or("").to_string();
            if !srv.chats.lock().unwrap().contains_key(&sesion) {
                let _ = srv.canal.respondedor().error(id, -32002, &crate::tr!("no hay sesión «{sesion}»", "no session «{sesion}»")).await;
                return;
            }
            let (texto, avisos) = texto_de_prompt(p["prompt"].as_array().map(|a| a.as_slice()).unwrap_or(&[]));
            for a in avisos {
                eprintln!("· {a}");
            }
            let pedido = Pedido { id, sesion: sesion.clone(), texto };
            let en_cola = {
                let mut c = srv.cola.lock().unwrap();
                if c.0 {
                    c.1.push_back(pedido);
                    Ok(c.1.len())
                } else {
                    c.0 = true;
                    Err(pedido)
                }
            };
            if let Ok(n) = en_cola {
                let aviso = crate::tr!("⏳ En cola ({n}): el motor local atiende una orden a la vez; ésta empieza cuando termine la anterior.\n", "⏳ Queued ({n}): the local engine handles one order at a time; this one starts when the previous one ends.\n");
                let _ = srv.canal.avisar("session/update", json!({"sessionId": sesion, "update": {"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": aviso}}})).await;
            } else if let Err(pedido) = en_cola {
                lanzar(srv.clone(), pedido);
            }
        }
        ("session/cancel", None) => {
            let sesion = p["sessionId"].as_str().unwrap_or("");
            // una en cola de ese chat se contesta ya, cancelada
            let quitadas: Vec<Pedido> = {
                let mut c = srv.cola.lock().unwrap();
                let (fuera, quedan): (Vec<_>, Vec<_>) = c.1.drain(..).partition(|x| x.sesion == sesion);
                c.1.extend(quedan);
                fuera
            };
            for q in quitadas {
                let _ = resp.responder(q.id, json!({"stopReason": "cancelled"})).await;
            }
            if let Some(c) = srv.chats.lock().unwrap().get(sesion) {
                c.cancel.store(true, Ordering::Relaxed);
            }
        }
        ("session/close", Some(id)) => {
            let sesion = p["sessionId"].as_str().unwrap_or("").to_string();
            let chat = srv.chats.lock().unwrap().remove(&sesion);
            if let Some(c) = chat {
                c.cancel.store(true, Ordering::Relaxed);
                c.agente.lock().await.cerrar_sesion("other").await;
            }
            let _ = resp.responder(id, json!({})).await;
        }
        (_, Some(id)) if metodo.is_empty() => {
            let _ = resp.error(id, -32600, "Invalid request").await;
        }
        (_, Some(id)) => {
            let _ = resp.error(id, -32601, &format!("Method not found: {metodo}")).await;
        }
        // avisos desconocidos: se ignoran (así lo pide JSON-RPC)
        (_, None) => {}
    }
}

/// Corre una orden en su tarea; al terminar contesta y saca la siguiente de la cola.
fn lanzar<E: Engine + 'static>(srv: Arc<Servidor<E>>, p: Pedido) {
    tokio::spawn(async move {
        let chat = srv.chats.lock().unwrap().get(&p.sesion).map(|c| (c.agente.clone(), c.cancel.clone()));
        let respuesta = match chat {
            None => Err((-32002, crate::tr!("no hay sesión «{}»", "no session «{}»", p.sesion))),
            Some((agente, cancelado)) => {
                let mut a = agente.lock().await;
                cancelado.store(false, Ordering::Relaxed);
                // `run` repone la bandera del agente al empezar: un `session/cancel` que llegue mientras la orden
                // arranca se perdería. Un vigía la vuelve a poner mientras la orden siga viva
                let cancel = a.cancel.clone();
                let viva = Arc::new(AtomicBool::new(true));
                {
                    let (cancelado, viva) = (cancelado.clone(), viva.clone());
                    tokio::spawn(async move {
                        while viva.load(Ordering::Relaxed) {
                            if cancelado.load(Ordering::Relaxed) {
                                cancel.store(true, Ordering::Relaxed);
                            }
                            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
                        }
                    });
                }
                let caps = srv.caps.lock().unwrap().clone();
                let r = if caps["fs"]["readTextFile"].as_bool() == Some(true) || caps["fs"]["writeTextFile"].as_bool() == Some(true) {
                    let ed: Arc<dyn archivos::Editor> = Arc::new(frente::EditorAcp::new(srv.canal.clone(), &p.sesion, caps));
                    archivos::EDITOR.scope(ed, a.run(&p.texto)).await
                } else {
                    a.run(&p.texto).await
                };
                viva.store(false, Ordering::Relaxed);
                a.front.vaciar().await;
                match r {
                    Ok(t) if t == crate::agent::interrupted() || cancelado.load(Ordering::Relaxed) => Ok("cancelled"),
                    Ok(_) => Ok("end_turn"),
                    Err(e) => Err((-32603, format!("{e:#}"))),
                }
            }
        };
        let resp = srv.canal.respondedor();
        let _ = match respuesta {
            Ok(stop) => resp.responder(p.id, json!({"stopReason": stop})).await,
            Err((c, m)) => resp.error(p.id, c, &m).await,
        };
        srv.hechas.fetch_add(1, Ordering::Relaxed);
        let siguiente = {
            let mut c = srv.cola.lock().unwrap();
            let s = c.1.pop_front();
            if s.is_none() {
                c.0 = false;
            }
            s
        };
        if let Some(s) = siguiente {
            lanzar(srv.clone(), s);
        }
    });
}

#[cfg(test)]
mod tests;
