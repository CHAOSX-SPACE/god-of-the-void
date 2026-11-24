//! GOD OF THE VOID · GUI (G2) — Tauri 2, sin Electron y sin paso de compilación web.
//!
//! La ventana habla con el MISMO agente del núcleo que el CLI y la TUI. Además de chatear:
//! proyectos (recientes, nuevo, abrir con el selector de macOS, confianza por carpeta),
//! adjuntos (imagen, audio, PDF, Word, texto) y voz, configuración en vivo, selector de
//! modelo (apaga el actual y carga el nuevo), tareas en segundo plano (panel que se
//! despega a su ventana), medidor de contexto y autocompletado de comandos y skills.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod acceso;
mod attach;
mod avisos;
mod dispositivos;
mod escritorio;
mod formas;
mod libreria;
/// F10.5 · el chat abierto, visible y contestable desde el teléfono
mod remoto;
mod servicio;
mod store;
// F8 · la ventana: estructura (F8.0) y órdenes de chats, cambios, menciones, ⌘K, charlas y vista
mod ventana;
mod worker;

use async_trait::async_trait;
use gotv_core::{AskAnswer, Frontend};
use gotv_engine::StreamEvent;
use serde_json::json;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::sync::{mpsc, oneshot, Mutex};
use worker::Msg;

pub fn emit(app: &AppHandle, v: serde_json::Value) {
    let _ = app.emit("gotv", v);
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// Estado compartido entre la ventana (comandos) y el trabajador.
#[derive(Clone)]
pub struct Hub {
    pub app: AppHandle,
    pub tx: mpsc::UnboundedSender<Msg>,
    pub asks: Arc<Mutex<HashMap<u64, oneshot::Sender<AskAnswer>>>>,
    /// C7: preguntas con opciones del agente (`ask_user`, aprobar un plan)
    pub choices: Arc<Mutex<HashMap<u64, oneshot::Sender<Option<String>>>>>,
    pub cancel: Arc<AtomicBool>,
    pub root: Arc<Mutex<PathBuf>>,
    pub status: Arc<Mutex<serde_json::Value>>,
    /// (ve, oye) del motor actual
    pub caps: Arc<Mutex<(bool, bool)>>,
    pub pending: Arc<Mutex<Vec<(attach::Chip, attach::Payload)>>>,
    pub tasks: Arc<Mutex<Vec<serde_json::Value>>>,
    pub turn: Arc<Mutex<Option<u64>>>,
    pub recording: Arc<Mutex<Option<attach::Recording>>>,
    /// M2: la lectura en voz alta en curso (para callarla)
    pub voz: Arc<std::sync::Mutex<Option<gotv_mlx::Callar>>>,
    /// M3: la charla por voz (manos libres) si está encendida
    pub charla: Arc<std::sync::Mutex<Option<Arc<gotv_voz::Charla>>>>,
    /// M3: la instrucción de voz viaja con la PRIMERA frase dicha de cada sesión de manos libres
    pub voz_instruccion: Arc<AtomicBool>,
    /// El check «Control Mac» y el perfil tal como están en el chat (la interfaz los avisa al
    /// cambiar): las órdenes DICHAS en manos libres los usan igual que las escritas. Medido
    /// 2026-11-11 (captura del Portador): la voz mandaba siempre Control Mac apagado y sin perfil.
    pub chat_mac: Arc<AtomicBool>,
    pub chat_perfil: Arc<std::sync::Mutex<Option<String>>>,
    /// Ola PRO: el modo profesional del perfil del chat, para las órdenes dichas en manos libres
    pub chat_pro: Arc<std::sync::Mutex<gotv_core::pro::Pro>>,
    /// carpeta del modelo cargado (para descontar su memoria al recalcular)
    pub model_path: Arc<Mutex<String>>,
    /// descargas de la Biblioteca en curso, por repo
    pub pulls: Arc<Mutex<HashMap<String, libreria::Pull>>>,
    /// A4: el servicio de API (su estado compartido vive siempre; el servidor, al encenderlo)
    pub api: gotv_core::serve::Shared,
    pub api_run: Arc<Mutex<Option<servicio::Running>>>,
    /// C1: dueño (proyecto + chat) de los procesos que la ventana muestra
    pub procs_owner: Arc<std::sync::Mutex<String>>,
    next_id: Arc<AtomicU64>,
}

/// C1: los procesos del chat abierto, para el panel de tareas.
fn procs_view(owner: &str) -> serde_json::Value {
    let list: Vec<serde_json::Value> = gotv_core::exec::list(owner)
        .iter()
        .map(|p| {
            json!({
                "id": p.id,
                "name": p.name,
                "cmd": p.cmd,
                "tty": p.tty,
                "alive": p.alive(),
                "exit": p.exit_label(),
                "secs": p.started.elapsed().as_secs(),
                "urls": p.urls(),
                "screen": p.screen_tail(6),
                "log": p.log_path.display().to_string(),
            })
        })
        .collect();
    serde_json::Value::Array(list)
}

#[tauri::command]
async fn proc_stop(h: State<'_, Hub>, id: u32) -> Result<(), String> {
    let owner = h.procs_owner.lock().unwrap().clone();
    let p = gotv_core::exec::get(&owner, id).ok_or_else(|| gotv_core::tr!("no existe el proceso #{id}", "process #{id} does not exist"))?;
    p.stop().await;
    gotv_core::exec::forget(&owner, id);
    emit(&h.app, json!({"kind": "procs", "procs": procs_view(&owner)}));
    Ok(())
}

impl Hub {
    pub fn id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::SeqCst)
    }

    async fn publish_tasks(&self) {
        let list = self.tasks.lock().await.clone();
        emit(&self.app, json!({"kind": "tasks", "tasks": list}));
    }

    pub async fn task_begin(&self, kind: &str, title: &str) -> u64 {
        let id = self.id();
        {
            let mut t = self.tasks.lock().await;
            t.insert(0, json!({"id": id, "kind": kind, "title": title, "state": "running", "detail": "", "t0": now(), "t1": null}));
            t.truncate(60);
        }
        self.publish_tasks().await;
        id
    }

    pub async fn task_update(&self, id: u64, detail: &str) {
        if let Some(t) = self.tasks.lock().await.iter_mut().find(|t| t["id"] == id) {
            t["detail"] = json!(detail);
        }
        self.publish_tasks().await;
    }

    /// A2: tokens que lleva gastados la tarea (un turno o un subagente).
    pub async fn task_tokens(&self, id: u64, usage: &serde_json::Value) {
        if let Some(t) = self.tasks.lock().await.iter_mut().find(|t| t["id"] == id) {
            t["tokens"] = usage.clone();
        }
        self.publish_tasks().await;
    }

    pub async fn task_end(&self, id: u64, state: &str, detail: &str) {
        if let Some(t) = self.tasks.lock().await.iter_mut().find(|t| t["id"] == id) {
            t["state"] = json!(state);
            if !detail.is_empty() {
                t["detail"] = json!(detail);
            }
            t["t1"] = json!(now());
        }
        self.publish_tasks().await;
    }

    /// Al cambiar de proyecto, los servicios del anterior (LSP, embedder) se apagaron.
    pub async fn services_reset(&self) {
        for t in self.tasks.lock().await.iter_mut() {
            if t["state"] == "running" && (t["kind"] == "lsp" || t["kind"] == "embedder") {
                t["state"] = json!("done");
                t["t1"] = json!(now());
            }
        }
        self.publish_tasks().await;
    }
}

#[derive(Clone)]
pub struct GuiFrontend {
    hub: Hub,
    subs: Arc<std::sync::Mutex<HashMap<String, u64>>>,
    /// El subagente que corre ahora (corren de a uno): a él van sus tokens.
    cur_sub: Arc<std::sync::Mutex<Option<u64>>>,
    /// F10.5: el id del chat abierto en el registro remoto (lo fija el worker al abrir un proyecto)
    remoto: Arc<std::sync::Mutex<String>>,
}

impl GuiFrontend {
    pub fn new(hub: Hub) -> Self {
        GuiFrontend { hub, subs: Arc::default(), cur_sub: Arc::default(), remoto: Arc::default() }
    }

    /// F10.5: este frente cuenta ahora lo del chat `id` al registro remoto.
    pub fn fijar_remoto(&self, id: &str) {
        *self.remoto.lock().unwrap() = id.to_string();
    }

    fn sesion_remota(&self) -> String {
        self.remoto.lock().unwrap().clone()
    }
}

#[async_trait]
impl Frontend for GuiFrontend {
    async fn log(&self, line: String) {
        emit(&self.hub.app, json!({"kind": "line", "text": line}));
    }
    async fn ask(&self, desc: &str) -> AskAnswer {
        let id = self.hub.id();
        let (tx, rx) = oneshot::channel();
        self.hub.asks.lock().await.insert(id, tx);
        emit(&self.hub.app, json!({"kind": "ask", "id": id, "desc": desc, "partes": gotv_core::permissions::pregunta_en_partes(desc)}));
        // F2.1: con la ventana en segundo plano, el permiso pedido avisa
        let raiz = self.hub.root.lock().await.clone();
        avisos::avisar(&self.hub.app, Some(&raiz), gotv_core::avisos::Evento::PermisoPedido, &gotv_core::tr!("pide permiso: {desc}", "asks for permission: {desc}"));
        // F10.5: también desde el teléfono; vale lo que llegue PRIMERO y el otro lado se entera
        let sesion = self.sesion_remota();
        let (idp, telefono) = self.hub.api.remoto.pedir_permiso(&sesion, desc);
        tokio::pin!(rx);
        let r = tokio::select! {
            a = &mut rx => {
                let a = a.unwrap_or(AskAnswer::No);
                self.hub.api.remoto.permiso_resuelto(&sesion, &idp, a);
                a
            }
            t = telefono => match t {
                Ok(a) => {
                    self.hub.asks.lock().await.remove(&id);
                    emit(&self.hub.app, json!({"kind": "ask_cerrado", "id": id, "aprobado": a == AskAnswer::Yes}));
                    a
                }
                // sin chat en el registro remoto: sólo la ventana
                Err(_) => rx.await.unwrap_or(AskAnswer::No),
            },
        };
        self.hub.api.remoto.estado(&sesion, gotv_core::remoto::EstadoSesion::Trabajando);
        r
    }
    async fn choose(&self, question: &str, options: &[String]) -> Option<String> {
        let id = self.hub.id();
        let (tx, rx) = oneshot::channel();
        self.hub.choices.lock().await.insert(id, tx);
        emit(&self.hub.app, json!({"kind": "choose", "id": id, "question": question, "options": options}));
        // F2.1: un permiso con alcance (`pedir_permiso`, empieza por 🔑) también avisa en segundo plano
        if question.starts_with('🔑') {
            let raiz = self.hub.root.lock().await.clone();
            avisos::avisar(&self.hub.app, Some(&raiz), gotv_core::avisos::Evento::PermisoPedido, question);
        }
        rx.await.ok().flatten()
    }
    // F1: la clave de una base, en un cuadro tapado; el valor vuelve por `answer_choice` y va al Llavero
    async fn secreto(&self, etiqueta: &str) -> Option<String> {
        let id = self.hub.id();
        let (tx, rx) = oneshot::channel();
        self.hub.choices.lock().await.insert(id, tx);
        emit(&self.hub.app, json!({"kind": "secreto", "id": id, "etiqueta": etiqueta}));
        rx.await.ok().flatten()
    }
    fn delta(&self, ev: &StreamEvent) {
        match ev {
            StreamEvent::Text(t) => {
                emit(&self.hub.app, json!({"kind": "delta", "text": t}));
                self.hub.api.remoto.evento(&self.sesion_remota(), "texto", json!({"texto": t})); // F10.5
                // M3: con manos libres, lo que escribe también se DICE (frase a frase, mientras escribe)
                if let Some(c) = self.hub.charla.lock().ok().and_then(|g| g.clone()) {
                    c.texto(t);
                }
            }
            StreamEvent::Prefill { done, total } => emit(&self.hub.app, json!({"kind": "prefill", "done": done, "total": total})),
            // M3: una llamada a herramienta = trabajo del agente: la voz no lo dice
            StreamEvent::ToolArgs(_) => {
                if let Some(c) = self.hub.charla.lock().ok().and_then(|g| g.clone()) {
                    c.herramienta();
                }
            }
            StreamEvent::Reasoning(_) => {}
        }
    }
    fn stats(&self, s: &gotv_core::TurnStats) {
        emit(&self.hub.app, json!({"kind": "stats", "text": s.summary(), "used": s.ctx_used, "budget": s.ctx_budget}));
    }
    fn activity(&self, kind: &str, detail: &str) {
        // A1: la velocidad va directo a la ventana (sin pasar por las tareas)
        if kind == "rate" {
            emit(&self.hub.app, json!({"kind": "rate", "tok_s": detail.parse::<f32>().unwrap_or(0.0)}));
            return;
        }
        // D6/D7: una página o isométrica recién hecha va directo al chat (marco aislado)
        if kind == "vista" {
            if let Ok(mut v) = serde_json::from_str::<serde_json::Value>(detail) {
                v["kind"] = json!("vista");
                emit(&self.hub.app, v);
            }
            return;
        }
        // F10.5: los pasos se ven en el teléfono (no el registro de herramientas)
        if matches!(kind, "step" | "subagent_start" | "subagent_end" | "compact") {
            self.hub.api.remoto.evento(&self.sesion_remota(), "actividad", json!({"tipo": kind, "detalle": detail}));
        }
        let (hub, kind, detail, subs, cur_sub) = (self.hub.clone(), kind.to_string(), detail.to_string(), self.subs.clone(), self.cur_sub.clone());
        tauri::async_runtime::spawn(async move {
            match kind.as_str() {
                "step" => {
                    if let Some(id) = *hub.turn.lock().await {
                        hub.task_update(id, &detail).await;
                    }
                }
                "subagent_start" => {
                    let id = hub.task_begin("subagent", &detail).await;
                    subs.lock().unwrap().insert(detail, id);
                    *cur_sub.lock().unwrap() = Some(id);
                }
                "usage" => {
                    let Ok(u) = serde_json::from_str::<serde_json::Value>(&detail) else { return };
                    let id = if u["depth"].as_u64().unwrap_or(0) == 0 { *hub.turn.lock().await } else { *cur_sub.lock().unwrap() };
                    if let Some(id) = id {
                        hub.task_tokens(id, &u).await;
                    }
                    if u["depth"].as_u64().unwrap_or(0) == 0 {
                        emit(&hub.app, json!({"kind": "usage", "usage": u}));
                    }
                }
                "subagent_end" => {
                    cur_sub.lock().unwrap().take();
                    let id = subs.lock().unwrap().remove(&detail);
                    if let Some(id) = id {
                        hub.task_end(id, "done", "").await;
                    }
                }
                "compact" => {
                    let id = hub.task_begin("compact", gotv_core::i18n::t("contexto compactado", "context compacted")).await;
                    hub.task_end(id, "done", &detail).await;
                    emit(&hub.app, json!({"kind": "compacted", "detail": detail}));
                }
                "lsp" | "embedder" => {
                    hub.task_begin(&kind, &detail).await;
                }
                _ => {}
            }
        });
    }
}

// ── Comandos de la ventana ──────────────────────────────────────────────────

#[tauri::command]
fn send(task: String, mac: Option<bool>, perfil: Option<String>, rebobinar: Option<serde_json::Value>, pro: Option<serde_json::Value>, h: State<'_, Hub>) -> Result<(), String> {
    // ↺ {texto, n}: el n-ésimo mensaje del usuario con ese texto; se rebobina hasta antes de él
    let rebobinar = rebobinar.and_then(|r| Some((r["texto"].as_str()?.to_string(), r["n"].as_u64().unwrap_or(0) as usize)));
    h.tx.send(Msg::Task(task, mac.unwrap_or(false), perfil, rebobinar, Box::new(leer_pro(pro)))).map_err(|e| e.to_string())
}

/// Ola PRO: los ajustes que manda la interfaz (sólo el perfil con el switch encendido); lo que no
/// se entienda se ignora (= sin modo profesional), nunca rompe la orden.
fn leer_pro(pro: Option<serde_json::Value>) -> gotv_core::pro::Pro {
    pro.and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default()
}

/// Ola PRO: el vocabulario de los paneles (una sola fuente: el núcleo), en el idioma de la app.
#[tauri::command]
fn pro_opciones() -> serde_json::Value {
    gotv_core::pro::opciones()
}

/// Ola PRO · Música: qué modelos del motor están en esta Mac (el panel marca ✓ o ↓ en cada opción).
#[tauri::command]
async fn musica_listos() -> serde_json::Value {
    tokio::task::spawn_blocking(gotv_core::generadores::musica_listos).await.unwrap_or_default()
}

/// Lo que le falta a una función (manos libres, un perfil, lo elegido en su panel PRO): el repo cuya fila de
/// Configuración → Generadores hay que abrir, o nada si está todo en esta Mac.
#[tauri::command]
async fn generador_falta(uso: String, pro: Option<serde_json::Value>) -> Option<String> {
    tokio::task::spawn_blocking(move || gotv_core::generadores::falta_para(&uso, &pro.unwrap_or_default()).map(String::from)).await.ok().flatten()
}

/// El generador que nombra un aviso de «falta» (para ofrecer ir a su fila).
#[tauri::command]
fn generador_de_aviso(texto: String) -> Option<String> {
    gotv_core::generadores::repo_de_aviso(&texto).map(String::from)
}

/// Ola PRO: los minutos estimados de un video con estos ajustes (con lo medido en V7).
#[tauri::command]
fn pro_video_estimado(video: serde_json::Value, segundos: Option<f32>) -> f32 {
    let v: gotv_core::pro::ProVideo = serde_json::from_value(video).unwrap_or_default();
    let s = segundos.filter(|s| *s > 0.0).or(Some(v.segundos).filter(|s| *s > 0.0)).unwrap_or(2.0).clamp(1.0, 5.0);
    v.minutos_estimados(s)
}

/// Ola PRO: guarda los paneles del modo profesional de un proyecto (qué switch está encendido y los
/// ajustes de cada perfil), sin tocar el resto del chat.
#[tauri::command]
fn guardar_pro(proyecto: String, pro: serde_json::Value) {
    let mut st = store::Store::load();
    let p = Path::new(&proyecto);
    let chat = store::Chat { pro: Some(pro).filter(|v| v.is_object()), ..st.project_chat(p) };
    st.set_project_chat(p, chat);
    st.save();
}

/// Ola EXPERTO: la carpeta de un proyecto CONOCIDO (la sesión sólo se escribe en sus `.gotv/`).
fn raiz_experto(proyecto: &str) -> Result<std::path::PathBuf, String> {
    let st = store::Store::load();
    if st.projects.iter().any(|p| p.path == proyecto) && Path::new(proyecto).is_dir() {
        Ok(Path::new(proyecto).to_path_buf())
    } else {
        Err(gotv_core::i18n::t("ese proyecto no está en la lista de GOTV", "that project is not in GOTV's list").into())
    }
}

/// Ola EXPERTO: la sesión del estudio del proyecto (estado, candado, deshacer/rehacer).
#[tauri::command]
fn experto_sesion(proyecto: String) -> Result<serde_json::Value, String> {
    Ok(gotv_core::experto::cargar(&raiz_experto(&proyecto)?).vista())
}

/// Abre o cierra el Experto (el candado del chat y del Pro de ESE proyecto) o lo minimiza.
#[tauri::command]
fn experto_activar(proyecto: String, activa: bool, minimizada: Option<bool>) -> Result<serde_json::Value, String> {
    let root = raiz_experto(&proyecto)?;
    let mut s = gotv_core::experto::activar(&root, activa).map_err(|e| e.to_string())?;
    if let Some(m) = minimizada.filter(|_| activa) {
        s.minimizada = m;
        gotv_core::experto::guardar(&root, &s).map_err(|e| e.to_string())?;
    }
    Ok(s.vista())
}

/// Aplica una operación de la sesión (validada por el núcleo; lo nuevo con `id` 0 recibe número).
#[tauri::command]
fn experto_aplicar(proyecto: String, op: serde_json::Value) -> Result<serde_json::Value, String> {
    let root = raiz_experto(&proyecto)?;
    let op: gotv_core::experto::Op = serde_json::from_value(op).map_err(|e| e.to_string())?;
    let mut s = gotv_core::experto::cargar(&root);
    let op = s.numerar(op);
    s.aplicar(op).map_err(|e| e.to_string())?;
    gotv_core::experto::guardar(&root, &s).map_err(|e| e.to_string())?;
    Ok(s.vista())
}

/// Deshacer (o rehacer, con `rehacer`) la última operación de la sesión.
#[tauri::command]
fn experto_deshacer(proyecto: String, rehacer: Option<bool>) -> Result<serde_json::Value, String> {
    let root = raiz_experto(&proyecto)?;
    let mut s = gotv_core::experto::cargar(&root);
    if rehacer.unwrap_or(false) { s.rehacer() } else { s.deshacer() };
    gotv_core::experto::guardar(&root, &s).map_err(|e| e.to_string())?;
    Ok(s.vista())
}

/// Ola EXPERTO · EXP-6 / X2-7: exporta la sesión a `<proyecto>/exportado/<fecha>/`: la mezcla (con buses y
/// maestro), las partes que la suman y el `.dawproject`. `opciones`: `lufs`, `desde`/`hasta` (un tramo),
/// `bits` (16, 24, 32), `pistas`, `dawproject`, `formato` («flac», «m4a»).
#[tauri::command]
async fn experto_exportar(proyecto: String, lufs: Option<f64>, opciones: Option<serde_json::Value>) -> Result<serde_json::Value, String> {
    let root = raiz_experto(&proyecto)?;
    tokio::task::spawn_blocking(move || {
        let s = gotv_core::experto::cargar(&root);
        let ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0);
        let fecha = gotv_core::serve::rfc3339(ms).replace(':', "-");
        let fecha = fecha.split('.').next().unwrap_or(&fecha).to_string();
        let nombre = root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "sesion".into());
        let destino = root.join("exportado").join(&fecha);
        let o = opciones.unwrap_or_default();
        let num = |k: &str| o.get(k).and_then(|v| v.as_f64()).filter(|v| v.is_finite());
        let si = |k: &str| o.get(k).and_then(|v| v.as_bool()).unwrap_or(true);
        let op = gotv_core::experto_export::Opciones {
            nombre,
            normalizar_lufs: lufs.filter(|v| v.is_finite()).or(num("lufs")),
            tramo: num("desde").zip(num("hasta")),
            bits: num("bits").map(|b| b as u8).filter(|b| [16, 24, 32].contains(b)).unwrap_or(24),
            pistas: si("pistas"),
            dawproject: si("dawproject"),
            formato: o.get("formato").and_then(|v| v.as_str()).unwrap_or_default().to_string(),
        };
        let x = gotv_core::experto_export::exportar(&root, &s, &destino, &op).map_err(|e| format!("{e:#}"))?;
        Ok(serde_json::json!({"carpeta": destino.display().to_string(), "exportado": x}))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Ola EXPERTO: trae un audio (del proyecto o de fuera) a la sesión como pista nueva; un `.dawproject`
/// trae TODAS sus pistas, grupos, mezclador, tempo y marcadores.
#[tauri::command]
async fn experto_importar(proyecto: String, ruta: String, nombre: Option<String>) -> Result<serde_json::Value, String> {
    let root = raiz_experto(&proyecto)?;
    tokio::task::spawn_blocking(move || {
        let mut s = gotv_core::experto::cargar(&root);
        if ruta.to_lowercase().ends_with(".dawproject") {
            gotv_core::experto_export::importar_dawproject(&root, &mut s, Path::new(&ruta)).map_err(|e| format!("{e:#}"))?;
            gotv_core::experto::guardar(&root, &s).map_err(|e| e.to_string())?;
            return Ok(s.vista());
        }
        let op = gotv_core::experto::op_importar(&root, &s, Path::new(&ruta), nombre.as_deref(), gotv_core::experto::Origen::Importada).map_err(|e| e.to_string())?;
        s.aplicar(op).map_err(|e| e.to_string())?;
        gotv_core::experto::guardar(&root, &s).map_err(|e| e.to_string())?;
        Ok(s.vista())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Ola EXPERTO II: UN trabajo de IA a la vez por la ventana del estudio, con su avance («experto-avance»)
/// y su botón de cancelar (`experto_cancelar`).
static EXPERTO_OCUPADO: AtomicBool = AtomicBool::new(false);
static EXPERTO_CANCELAR: AtomicBool = AtomicBool::new(false);

async fn experto_trabajo<T: Send + 'static>(app: tauri::AppHandle, f: impl FnOnce(&(dyn Fn(String) + Sync), &AtomicBool) -> Result<T, String> + Send + 'static) -> Result<T, String> {
    if EXPERTO_OCUPADO.swap(true, Ordering::SeqCst) {
        return Err(gotv_core::i18n::t("ya hay un trabajo de IA en curso en el estudio: espera a que termine o cancélalo", "an AI job is already running in the studio: wait for it or cancel it").into());
    }
    EXPERTO_CANCELAR.store(false, Ordering::SeqCst);
    let r = tokio::task::spawn_blocking(move || {
        let avance = |t: String| {
            let _ = tauri::Emitter::emit(&app, "experto-avance", t);
        };
        f(&avance, &EXPERTO_CANCELAR)
    })
    .await;
    EXPERTO_OCUPADO.store(false, Ordering::SeqCst);
    r.map_err(|e| e.to_string())?
}

/// Cancela el trabajo de IA en curso del estudio (se detiene en su próximo paso).
#[tauri::command]
fn experto_cancelar() -> bool {
    EXPERTO_CANCELAR.store(true, Ordering::SeqCst);
    EXPERTO_OCUPADO.load(Ordering::SeqCst)
}

/// Un trabajo de IA que cambia la sesión: la carga, la cambia, la guarda y devuelve su vista.
async fn experto_ia<T: serde::Serialize + Send + 'static>(
    proyecto: String,
    app: tauri::AppHandle,
    f: impl FnOnce(&Path, &mut gotv_core::experto::Sesion, &(dyn Fn(String) + Sync), &AtomicBool) -> anyhow::Result<T> + Send + 'static,
) -> Result<serde_json::Value, String> {
    let root = raiz_experto(&proyecto)?;
    experto_trabajo(app, move |avance, cancelar| {
        let mut s = gotv_core::experto::cargar(&root);
        let r = f(&root, &mut s, avance, cancelar).map_err(|e| format!("{e:#}"))?;
        gotv_core::experto::guardar(&root, &s).map_err(|e| e.to_string())?;
        Ok(serde_json::json!({"vista": s.vista(), "resultado": r}))
    })
    .await
}

/// Ola EXPERTO: divide una pista con HTDemucs («4», «6» o «ft»); el avance va como «experto-avance».
#[tauri::command]
async fn experto_separar(proyecto: String, pista: u32, modelo: Option<String>, app: tauri::AppHandle) -> Result<serde_json::Value, String> {
    let modelo = modelo.filter(|m| matches!(m.as_str(), "4" | "6" | "ft" | "bateria")).unwrap_or_else(|| "4".into());
    let r = experto_ia(proyecto, app, move |root, s, avance, cancelar| gotv_core::experto::separar(root, s, pista, &modelo, avance, cancelar)).await?;
    Ok(r["vista"].clone())
}

/// X2-6: una pista NUEVA desde un texto, con el tempo y la tonalidad de la sesión.
#[tauri::command]
async fn experto_generar(proyecto: String, pedido: serde_json::Value, app: tauri::AppHandle) -> Result<serde_json::Value, String> {
    let p: gotv_core::experto_ia::PedidoPista = serde_json::from_value(pedido).map_err(|e| e.to_string())?;
    experto_ia(proyecto, app, move |root, s, avance, cancelar| gotv_core::experto_ia::generar_pista(root, s, &p, avance, cancelar)).await
}

/// X2-6: la canción (o una pista) sigue `segundos` más.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
async fn experto_extender(proyecto: String, pista: Option<u32>, segundos: f64, descripcion: String, letra: Option<String>, idioma: Option<String>, opciones: Option<serde_json::Value>, app: tauri::AppHandle) -> Result<serde_json::Value, String> {
    experto_ia(proyecto, app, move |root, s, avance, cancelar| {
        gotv_core::experto_ia::extender(root, s, pista, segundos, &descripcion, letra.as_deref().unwrap_or(""), idioma.as_deref().unwrap_or(""), &opciones.unwrap_or_default(), avance, cancelar)
    })
    .await
}

/// X2-6: otra versión (cover) de una pista con otro estilo.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
async fn experto_versionar(proyecto: String, pista: u32, descripcion: String, fuerza: Option<f32>, letra: Option<String>, idioma: Option<String>, opciones: Option<serde_json::Value>, app: tauri::AppHandle) -> Result<serde_json::Value, String> {
    experto_ia(proyecto, app, move |root, s, avance, cancelar| {
        gotv_core::experto_ia::versionar(root, s, pista, &descripcion, fuerza.unwrap_or(0.6), letra.as_deref().unwrap_or(""), idioma.as_deref().unwrap_or(""), &opciones.unwrap_or_default(), avance, cancelar)
    })
    .await
}

/// X2-6: rehacer un tramo de una pista con otro sonido (EXP-4), desde el panel de IA.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
async fn experto_rehacer(proyecto: String, pista: u32, desde: f64, hasta: f64, descripcion: String, letra: Option<String>, idioma: Option<String>, app: tauri::AppHandle) -> Result<serde_json::Value, String> {
    experto_ia(proyecto, app, move |root, s, avance, cancelar| {
        gotv_core::experto::rehacer_tramo(root, s, pista, desde, hasta, &descripcion, letra.as_deref().unwrap_or(""), idioma.as_deref().unwrap_or(""), avance, cancelar)
    })
    .await
}

/// X2-6: la letra de una pista (o de la mezcla) con Whisper. No cambia la sesión: devuelve el texto.
#[tauri::command]
async fn experto_letra(proyecto: String, pista: Option<u32>, app: tauri::AppHandle) -> Result<String, String> {
    let root = raiz_experto(&proyecto)?;
    experto_trabajo(app, move |avance, _| gotv_core::experto_ia::transcribir(&root, &gotv_core::experto::cargar(&root), pista, avance).map_err(|e| format!("{e:#}"))).await
}

/// X2-6: masteriza la mezcla a `lufs` (−14 por omisión) con techo `techo_db` (−1).
#[tauri::command]
async fn experto_masterizar(proyecto: String, lufs: Option<f64>, techo_db: Option<f32>, app: tauri::AppHandle) -> Result<serde_json::Value, String> {
    experto_ia(proyecto, app, move |root, s, avance, _| {
        avance(gotv_core::i18n::t("midiendo la mezcla", "measuring the mix").to_string());
        gotv_core::experto_ia::masterizar(root, s, lufs.unwrap_or(-14.0), techo_db.unwrap_or(-1.0))
    })
    .await
}

/// X2-3: el análisis (tempo, tonalidad, sonoridad, picos) de una pista o, sin `pista`, de la mezcla.
#[tauri::command]
async fn experto_analizar(proyecto: String, pista: Option<u32>) -> Result<serde_json::Value, String> {
    let root = raiz_experto(&proyecto)?;
    tokio::task::spawn_blocking(move || {
        let a = gotv_core::experto_ia::analizar(&root, &gotv_core::experto::cargar(&root), pista).map_err(|e| format!("{e:#}"))?;
        serde_json::to_value(a).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// X2-5: la curva del EQ (dB en cada frecuencia) con la MISMA fórmula que suena.
#[tauri::command]
fn experto_curva_eq(bandas: serde_json::Value, frecuencias: Vec<f64>, frecuencia: Option<u32>) -> Result<Vec<f64>, String> {
    let b: Vec<gotv_core::experto::Banda> = serde_json::from_value(bandas).map_err(|e| e.to_string())?;
    Ok(frecuencias.iter().map(|f| gotv_core::experto_fx::respuesta_eq(&b, *f, frecuencia.unwrap_or(48_000))).collect())
}

/// X2-7: el historial se recorre: deja la sesión con `n` operaciones hechas.
#[tauri::command]
fn experto_ir_a(proyecto: String, n: usize) -> Result<serde_json::Value, String> {
    let root = raiz_experto(&proyecto)?;
    let mut s = gotv_core::experto::cargar(&root);
    s.ir_a(n);
    gotv_core::experto::guardar(&root, &s).map_err(|e| e.to_string())?;
    Ok(s.vista())
}

/// X2-2: la MEZCLA FINAL (buses y maestro) en binario: lo que suena cuando hay efectos de bus o de
/// maestro, y lo mismo que se exporta. De paso poda la caché.
#[tauri::command]
async fn experto_mezcla_audio(proyecto: String) -> Result<tauri::ipc::Response, String> {
    let root = raiz_experto(&proyecto)?;
    tokio::task::spawn_blocking(move || {
        let s = gotv_core::experto::cargar(&root);
        let ruta = gotv_core::experto_dsp::mezcla_en_cache(&root, &s.estado(), s.frecuencia).map_err(|e| format!("{e:#}"))?;
        let datos = std::fs::read(&ruta).map_err(|e| e.to_string())?;
        gotv_core::experto_dsp::podar_cache(&root, 60);
        Ok(tauri::ipc::Response::new(datos))
    })
    .await
    .map_err(|e| e.to_string())?
}


/// X2-7: la grabación en curso del estudio (una a la vez): el hilo que lee el micrófono y su señal de parar.
struct Grabacion {
    proyecto: String,
    parar: Arc<AtomicBool>,
    hilo: std::thread::JoinHandle<Vec<f32>>,
}
static EXPERTO_GRABACION: std::sync::Mutex<Option<Grabacion>> = std::sync::Mutex::new(None);

/// X2-7: empieza a grabar del micrófono (mono, a la frecuencia de la sesión). macOS pide permiso la
/// primera vez. Devuelve cuando el micrófono ya está abierto: desde ahí cuenta el tiempo.
#[tauri::command]
fn experto_grabar(proyecto: String) -> Result<(), String> {
    let root = raiz_experto(&proyecto)?;
    let mut g = EXPERTO_GRABACION.lock().unwrap_or_else(|e| e.into_inner());
    if g.is_some() {
        return Err(gotv_core::i18n::t("ya se está grabando", "already recording").into());
    }
    let fs = gotv_core::experto::cargar(&root).frecuencia;
    let mic = gotv_sonido::Microfono::abrir(fs, 2400).map_err(|e| format!("{e:#}"))?;
    let parar = Arc::new(AtomicBool::new(false));
    let p2 = parar.clone();
    let hilo = std::thread::spawn(move || {
        let mut v: Vec<f32> = Vec::new();
        // tope de 20 minutos: una grabación olvidada no se come la memoria
        while !p2.load(Ordering::SeqCst) && v.len() < fs as usize * 1200 {
            if let Some(b) = mic.leer(std::time::Duration::from_millis(100)) {
                v.extend(b);
            }
        }
        v
    });
    *g = Some(Grabacion { proyecto, parar, hilo });
    Ok(())
}

/// X2-7: termina la grabación y la deja como pista nueva, con su clip en `inicio` (donde estaba el
/// cabezal al empezar, ya compensada la latencia por la ventana). Con `descartar`, sólo se tira.
#[tauri::command]
async fn experto_grabar_parar(proyecto: String, inicio: Option<f64>, descartar: Option<bool>) -> Result<serde_json::Value, String> {
    let root = raiz_experto(&proyecto)?;
    let g = EXPERTO_GRABACION.lock().unwrap_or_else(|e| e.into_inner()).take().ok_or_else(|| gotv_core::i18n::t("no hay ninguna grabación en curso", "there is no recording in progress").to_string())?;
    g.parar.store(true, Ordering::SeqCst);
    tokio::task::spawn_blocking(move || {
        let x = g.hilo.join().map_err(|_| gotv_core::i18n::t("la grabación se interrumpió", "the recording was interrupted").to_string())?;
        let mut s = gotv_core::experto::cargar(&root);
        if descartar.unwrap_or(false) || g.proyecto != proyecto {
            return Ok(s.vista());
        }
        let fs = s.frecuencia;
        if x.len() < fs as usize / 5 {
            return Err(gotv_core::i18n::t("no se grabó nada (¿el micrófono tiene permiso?)", "nothing was recorded (does the microphone have permission?)").to_string());
        }
        let ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0);
        let ruta = root.join(format!(".gotv/experto/audio/grabacion-{ms}.wav"));
        gotv_core::experto_dsp::escribir_wav(&ruta, &[x.clone(), x], fs, false).map_err(|e| e.to_string())?;
        let n = s.estado().pistas.iter().filter(|p| p.archivo.contains("grabacion-")).count() + 1;
        let nombre = format!("{} {n}", gotv_core::i18n::t("Grabación", "Recording"));
        let op = gotv_core::experto::op_importar(&root, &s, &ruta, Some(&nombre), gotv_core::experto::Origen::Importada).map_err(|e| e.to_string())?;
        let id = match &op {
            gotv_core::experto::Op::AgregarPista { pista } => pista.id,
            _ => 0,
        };
        s.aplicar(op).map_err(|e| e.to_string())?;
        let t = inicio.filter(|t| t.is_finite()).unwrap_or(0.0).max(0.0);
        if t > 0.0 {
            if let Some(c) = s.estado().clips_de(id).next().map(|c| c.id) {
                s.aplicar(gotv_core::experto::Op::MoverClip { clip: c, inicio: t, pista: None }).map_err(|e| e.to_string())?;
            }
        }
        gotv_core::experto::guardar(&root, &s).map_err(|e| e.to_string())?;
        Ok(s.vista())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// X2-7: muestra en Finder una carpeta de exportación del proyecto (sólo dentro de `exportado/`).
#[tauri::command]
fn experto_mostrar(proyecto: String, carpeta: String) -> Result<(), String> {
    let root = raiz_experto(&proyecto)?;
    let base = std::fs::canonicalize(root.join("exportado")).map_err(|e| e.to_string())?;
    let p = std::fs::canonicalize(&carpeta).map_err(|e| e.to_string())?;
    if !p.starts_with(&base) || !p.is_dir() {
        return Err(gotv_core::i18n::t("sólo se abren las carpetas exportadas del proyecto", "only the project's exported folders are opened").into());
    }
    std::process::Command::new("open").arg(&p).spawn().map(|_| ()).map_err(|e| e.to_string())
}

/// Ola EXPERTO: transponer un tramo a una PISTA NUEVA (la original queda en silencio en ese tramo).
#[tauri::command]
async fn experto_tono(proyecto: String, pista: u32, desde: f64, hasta: f64, semitonos: f32) -> Result<serde_json::Value, String> {
    let root = raiz_experto(&proyecto)?;
    tokio::task::spawn_blocking(move || {
        let mut s = gotv_core::experto::cargar(&root);
        gotv_core::experto::tono_a_pista(&root, &mut s, pista, desde, hasta, semitonos, 0.03).map_err(|e| format!("{e:#}"))?;
        gotv_core::experto::guardar(&root, &s).map_err(|e| e.to_string())?;
        Ok(s.vista())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Ola EXPERTO · EXP-5: una orden del chat del estudio, sólo para el proyecto ABIERTO (el modelo está en
/// el hilo del agente).
#[tauri::command]
async fn experto_chat(proyecto: String, texto: String, h: State<'_, Hub>) -> Result<serde_json::Value, String> {
    let root = raiz_experto(&proyecto)?;
    if std::fs::canonicalize(&root).ok() != std::fs::canonicalize(&*h.root.lock().await).ok() {
        return Err(gotv_core::i18n::t("el chat del Experto trabaja sobre el proyecto abierto", "the Expert chat works on the open project").into());
    }
    let (tx, rx) = oneshot::channel();
    h.tx.send(Msg::Experto(texto, tx)).map_err(|e| e.to_string())?;
    rx.await.map_err(|e| e.to_string())?
}

/// Ola EXPERTO: el audio de una pista con sus regiones aplicadas (antes del fader), en binario (WAV en
/// coma flotante): la ventana lo reproduce y le aplica en vivo el fader y el paneo de Web Audio.
#[tauri::command]
async fn experto_pista_audio(proyecto: String, pista: u32) -> Result<tauri::ipc::Response, String> {
    let root = raiz_experto(&proyecto)?;
    tokio::task::spawn_blocking(move || {
        let s = gotv_core::experto::cargar(&root);
        let e = s.estado();
        let p = e.pistas.iter().find(|p| p.id == pista).ok_or_else(|| gotv_core::i18n::t("esa pista no existe", "that track does not exist").to_string())?;
        let ruta = gotv_core::experto_dsp::pista_en_cache(&root, &e, p, s.frecuencia).map_err(|e| e.to_string())?;
        std::fs::read(ruta).map(tauri::ipc::Response::new).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// D6/D7: «Abrir» de una vista (página o isométrica) — sólo un .html/.svg DENTRO del proyecto.
#[tauri::command]
async fn open_vista(path: String, h: State<'_, Hub>) -> Result<(), String> {
    let root = h.root.lock().await.clone();
    let p = std::fs::canonicalize(&path).map_err(|e| e.to_string())?;
    let raiz = std::fs::canonicalize(&root).map_err(|e| e.to_string())?;
    let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
    // la ficha de licencia de lo generado (botón ⓘ de la tarjeta) también se abre
    let ficha = p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.ends_with(".licencia.txt"));
    if !p.starts_with(&raiz) || !(ficha || matches!(ext.as_str(), "html" | "svg" | "png" | "wav" | "mp4" | "webm" | "md" | "ipynb")) {
        return Err(gotv_core::i18n::t("sólo se abren páginas, imágenes, audios, videos, notebooks y licencias del proyecto", "only project pages, images, audio, videos, notebooks and licenses are opened").into());
    }
    std::process::Command::new("open").arg(&p).spawn().map(|_| ()).map_err(|e| e.to_string())
}

/// F7 · la tarjeta del plan: comentar una línea de un documento de la conversación (`.gotv/chats/<sesión>/`).
/// La sesión la trae la tarjeta; la carpeta es SIEMPRE la del proyecto abierto (la clave se limpia).
#[tauri::command]
async fn plan_comentar(sesion: String, archivo: String, linea: usize, texto: String, h: State<'_, Hub>) -> Result<serde_json::Value, String> {
    let root = h.root.lock().await.clone();
    let chat = gotv_core::artefactos::Chat::new(&root, &sesion);
    let c = gotv_core::artefactos::comentarios::comentar(&chat, &archivo, linea, &texto).map_err(|e| format!("{e:#}"))?;
    serde_json::to_value(c).map_err(|e| e.to_string())
}

/// F7 · los comentarios de un documento de la conversación (con su línea actual o huérfanos).
#[tauri::command]
async fn plan_comentarios(sesion: String, archivo: String, h: State<'_, Hub>) -> Result<serde_json::Value, String> {
    let root = h.root.lock().await.clone();
    let chat = gotv_core::artefactos::Chat::new(&root, &sesion);
    let cs: Vec<_> = gotv_core::artefactos::comentarios::todos(&chat).into_iter().filter(|c| c.archivo == archivo).collect();
    serde_json::to_value(cs).map_err(|e| e.to_string())
}

/// F7 · marcar (o reabrir) un comentario.
#[tauri::command]
async fn plan_resolver(sesion: String, id: u64, resuelto: bool, h: State<'_, Hub>) -> Result<(), String> {
    let root = h.root.lock().await.clone();
    let chat = gotv_core::artefactos::Chat::new(&root, &sesion);
    gotv_core::artefactos::comentarios::resolver(&chat, id, resuelto).map_err(|e| format!("{e:#}"))
}

/// M6: el MP4 de un video generado, para el reproductor del chat. Va en binario crudo
/// (`ipc::Response`: llega como ArrayBuffer, sin JSON ni base64) y NO se guarda en la conversación:
/// la tarjeta lleva sólo la ruta. Sólo `.mp4` del proyecto, como «Abrir».
#[tauri::command]
async fn leer_video(path: String, h: State<'_, Hub>) -> Result<tauri::ipc::Response, String> {
    let root = h.root.lock().await.clone();
    let p = std::fs::canonicalize(&path).map_err(|e| e.to_string())?;
    let raiz = std::fs::canonicalize(&root).map_err(|e| e.to_string())?;
    // F7: también el .webm que graba el navegador propio (`browser record`)
    let mp4 = p.extension().and_then(|e| e.to_str()).is_some_and(|e| gotv_core::artefactos::grabacion::EXT_VIDEO.iter().any(|x| e.eq_ignore_ascii_case(x)));
    if !p.starts_with(&raiz) || !mp4 || !p.is_file() {
        return Err(gotv_core::i18n::t("sólo se leen videos .mp4 o .webm del proyecto", "only project .mp4 or .webm videos are read").into());
    }
    let b = tokio::fs::read(&p).await.map_err(|e| e.to_string())?;
    Ok(tauri::ipc::Response::new(b))
}

/// D6: la página de una vista y su `ficha.json`, sólo DENTRO del proyecto.
async fn ficha_de_vista(path: &str, h: &State<'_, Hub>) -> Result<(std::path::PathBuf, gotv_diseno::pagina::Ficha), String> {
    let root = h.root.lock().await.clone();
    let p = std::fs::canonicalize(path).map_err(|e| e.to_string())?;
    let raiz = std::fs::canonicalize(&root).map_err(|e| e.to_string())?;
    let dir = p.parent().map(|d| d.to_path_buf()).unwrap_or_default();
    if !p.starts_with(&raiz) || p.file_name().and_then(|n| n.to_str()) != Some("index.html") {
        return Err(gotv_core::i18n::t("sólo páginas del proyecto", "only project pages").into());
    }
    let txt = std::fs::read_to_string(dir.join("ficha.json")).map_err(|_| gotv_core::i18n::t("esta página no tiene ficha.json (no la hizo `pagina`)", "this page has no ficha.json (not made by `pagina`)").to_string())?;
    let (f, _) = gotv_diseno::pagina::validar_tolerante(&txt).map_err(|e| e.join("; "))?;
    Ok((dir, f))
}

/// D6: la MISMA ficha en todas las direcciones — sin modelo, sólo se recompila (nada se escribe).
#[tauri::command]
async fn vista_variantes(path: String, h: State<'_, Hub>) -> Result<serde_json::Value, String> {
    let (_, f) = ficha_de_vista(&path, &h).await?;
    let actual = f.direccion.clone();
    let mut out = Vec::new();
    for d in std::iter::once(actual.as_str()).chain(gotv_diseno::pagina::DIRECCIONES.iter().map(|d| d.id).filter(|d| *d != actual)) {
        let mut g = f.clone();
        g.direccion = d.to_string();
        if let Ok(p) = gotv_diseno::pagina::Pagina::new(g) {
            out.push(json!({"direccion": d, "html": p.html_suelto(), "actual": d == actual}));
        }
    }
    Ok(json!(out))
}

/// D6: «Usar esta» — la variante elegida se escribe en el proyecto (ficha.json incluida).
#[tauri::command]
async fn vista_usar(path: String, direccion: String, h: State<'_, Hub>) -> Result<serde_json::Value, String> {
    let (dir, f) = ficha_de_vista(&path, &h).await?;
    let (g, _) = gotv_diseno::pagina::aplicar_cambio(&f, None, &json!({"direccion": direccion})).map_err(|e| e.join("; "))?;
    let p = gotv_diseno::pagina::Pagina::new(g).map_err(|e| e.to_string())?;
    let inf = gotv_diseno::pagina::escribir_proyecto(&dir, &p.archivos()).map_err(|e| e.to_string())?;
    Ok(json!({"html": p.html_suelto(), "direccion": p.dir.id, "respetados": inf.respetados.len()}))
}

// ── F13.8: «Conectores» (servidores MCP) del proyecto ABIERTO ─────────────────────────────────────
#[tauri::command]
async fn mcp_lista(h: State<'_, Hub>) -> Result<serde_json::Value, String> {
    use gotv_core::mcp::{self, Tipo};
    let root = h.root.lock().await.clone();
    let (cfgs, avisos, _) = mcp::load(&root);
    let lista: Vec<serde_json::Value> = cfgs
        .iter()
        .map(|c| {
            let origen = match (&c.tipo, &c.url) {
                (Tipo::Stdio, _) => format!("{} {}", c.command, c.args.join(" ")),
                (_, Some(u)) => u.clone(),
                _ => String::new(),
            };
            let seguro: String = c.name.chars().map(|x| if x.is_ascii_alphanumeric() || x == '-' { x } else { '_' }).collect();
            let registro: Vec<String> = std::fs::read_to_string(root.join(".gotv/registros").join(format!("mcp-{seguro}.log"))).unwrap_or_default().lines().rev().take(12).map(String::from).collect::<Vec<_>>().into_iter().rev().collect();
            json!({"nombre": c.name, "ambito": format!("{:?}", c.ambito), "tipo": format!("{:?}", c.tipo), "origen": origen, "activo": c.activo, "caja": c.caja, "remoto": c.tipo != Tipo::Stdio,
                   "sesion": c.tipo != Tipo::Stdio && gotv_core::mcp_oauth::leer(&c.name).is_some(), "registro": registro})
        })
        .collect();
    let importables: Vec<serde_json::Value> = mcp::importables(&root).iter().map(|i| json!({"nombre": i.nombre, "origen": i.origen, "secretos": i.secretos.len()})).collect();
    Ok(json!({"proyecto": root.display().to_string(), "servidores": lista, "avisos": avisos, "importables": importables}))
}

/// Arranca UN servidor aparte (dentro de la caja si es stdio) y dice qué ofrece y cuánto pesa.
#[tauri::command]
async fn mcp_probar(nombre: String, h: State<'_, Hub>) -> Result<serde_json::Value, String> {
    use gotv_core::mcp;
    let root = h.root.lock().await.clone();
    let (cfgs, _, _) = mcp::load(&root);
    let c = cfgs.into_iter().find(|c| c.name == nombre).ok_or_else(|| gotv_core::tr!("no hay un servidor «{nombre}»", "there is no server «{nombre}»"))?;
    let caja = gotv_core::sandbox::Spec { root: root.clone(), hosts: vec![], all_net: false, sockets: vec![], credenciales: vec![], entorno: vec![] };
    let (m, notas) = mcp::Mcp::start_con(&[mcp::ServerConfig { activo: true, ..c }], &root, Some(caja)).await;
    let esquemas = m.schemas();
    let nombres: Vec<String> = esquemas.iter().filter_map(|t| t["function"]["name"].as_str().map(String::from)).collect();
    Ok(json!({"notas": notas, "herramientas": nombres, "tokens_si_se_cargan": serde_json::Value::Array(esquemas).to_string().len() / 4}))
}

#[tauri::command]
async fn mcp_agregar(nombre: String, destino: String, ambito: String, dominios: Vec<String>, h: State<'_, Hub>) -> Result<(), String> {
    use gotv_core::mcp::{self, Ambito};
    let root = h.root.lock().await.clone();
    let n = nombre.trim();
    if n.is_empty() || !n.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '_' || c == ' ') {
        return Err(gotv_core::i18n::t("nombre inválido: letras, números, - y _", "invalid name: letters, digits, - and _").into());
    }
    let d = destino.trim();
    let mut def = if d.starts_with("http://") || d.starts_with("https://") {
        json!({"type": "http", "url": d})
    } else if d.starts_with("ws://") || d.starts_with("wss://") {
        json!({"type": "ws", "url": d})
    } else {
        let mut partes = d.split_whitespace().map(String::from);
        let cmd = partes.next().ok_or_else(|| gotv_core::i18n::t("falta el comando o la URL", "missing the command or the URL").to_string())?;
        json!({"command": cmd, "args": partes.collect::<Vec<_>>()})
    };
    let dominios: Vec<String> = dominios.into_iter().map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect();
    if !dominios.is_empty() {
        def["dominios"] = json!(dominios);
    }
    let ambito = if ambito == "local" { Ambito::Local } else { Ambito::Usuario };
    mcp::escribir_servidor(&mcp::archivo_de(&root, ambito), n, def).map_err(|e| e.to_string())
}

fn mcp_archivo(root: &std::path::Path, nombre: &str) -> Result<std::path::PathBuf, String> {
    use gotv_core::mcp;
    let (cfgs, _, _) = mcp::load(root);
    let c = cfgs.into_iter().find(|c| c.name == nombre).ok_or_else(|| gotv_core::tr!("no hay un servidor «{nombre}»", "there is no server «{nombre}»"))?;
    Ok(mcp::archivo_de(root, c.ambito))
}

#[tauri::command]
async fn mcp_quitar(nombre: String, h: State<'_, Hub>) -> Result<(), String> {
    let root = h.root.lock().await.clone();
    gotv_core::mcp::quitar_servidor(&mcp_archivo(&root, &nombre)?, &nombre).map(|_| ()).map_err(|e| e.to_string())
}

#[tauri::command]
async fn mcp_activar(nombre: String, activo: bool, h: State<'_, Hub>) -> Result<(), String> {
    let root = h.root.lock().await.clone();
    gotv_core::mcp::activar(&mcp_archivo(&root, &nombre)?, &nombre, activo).map_err(|e| e.to_string())
}

/// OAuth: abre el navegador y espera la vuelta (5 min); los tokens van al Llavero.
#[tauri::command]
async fn mcp_entrar(nombre: String, h: State<'_, Hub>) -> Result<(), String> {
    use gotv_core::mcp;
    let root = h.root.lock().await.clone();
    let (cfgs, _, _) = mcp::load(&root);
    let c = cfgs.into_iter().find(|c| c.name == nombre).ok_or_else(|| gotv_core::tr!("no hay un servidor «{nombre}»", "there is no server «{nombre}»"))?;
    let url = c.url.clone().ok_or_else(|| gotv_core::i18n::t("un servidor local no lleva sesión", "a local server has no sign-in").to_string())?;
    gotv_core::mcp_oauth::entrar(&nombre, &url, None, |u| {
        std::process::Command::new("open").arg(u).status()?;
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())
}

#[tauri::command]
async fn mcp_salir(nombre: String) -> Result<(), String> {
    gotv_core::mcp_oauth::olvidar(&nombre);
    Ok(())
}

/// Importa los elegidos de Claude Code / Claude Desktop al archivo del usuario (secretos al Llavero).
#[tauri::command]
async fn mcp_importar(nombres: Vec<String>, h: State<'_, Hub>) -> Result<usize, String> {
    use gotv_core::mcp;
    let root = h.root.lock().await.clone();
    let mut n = 0;
    for i in mcp::importables(&root).iter().filter(|i| nombres.contains(&i.nombre)) {
        mcp::importar(i, &mcp::user_file()).map_err(|e| e.to_string())?;
        n += 1;
    }
    Ok(n)
}

// ── F23.2: Configuración → Conexiones → «Buscadores» (fichas legales F23.L; llaves al Llavero) ──
async fn ctx_buscar(h: &State<'_, Hub>) -> gotv_core::buscar::Contexto {
    let root = h.root.lock().await.clone();
    gotv_core::buscar::Contexto::del_usuario(&root)
}
#[tauri::command]
async fn buscadores_lista(h: State<'_, Hub>) -> Result<serde_json::Value, String> {
    let ctx = ctx_buscar(&h).await;
    let mut v = gotv_core::buscar::ventana::lista(&ctx);
    v["indice"] = gotv_core::indice::estado(&ctx.almacen.indice_dir);
    Ok(v)
}
/// La llave va al Llavero y no vuelve; vacía = quitarla.
#[tauri::command]
async fn buscadores_llave(h: State<'_, Hub>, id: String, llave: Option<String>) -> Result<(), String> {
    let ctx = ctx_buscar(&h).await;
    match llave.filter(|k| !k.trim().is_empty()) {
        Some(k) => gotv_core::buscar::ventana::poner_llave(&ctx, &id, &k).map_err(|e| e.to_string()),
        None => {
            gotv_core::buscar::ventana::quitar_llave(&id);
            let _ = gotv_core::buscar::ventana::activar(&ctx, &gotv_core::config::global_path(), &id, false);
            Ok(())
        }
    }
}
#[tauri::command]
async fn buscadores_activar(h: State<'_, Hub>, id: String, activo: bool) -> Result<(), String> {
    let ctx = ctx_buscar(&h).await;
    gotv_core::buscar::ventana::activar(&ctx, &gotv_core::config::global_path(), &id, activo).map_err(|e| e.to_string())
}
#[tauri::command]
async fn buscadores_probar(h: State<'_, Hub>, id: String) -> Result<serde_json::Value, String> {
    let ctx = ctx_buscar(&h).await;
    gotv_core::buscar::ventana::probar(&ctx, &id).await.map_err(|e| e.to_string())
}
#[tauri::command]
async fn buscadores_tope(id: String, tope: Option<u64>) -> Result<(), String> {
    gotv_core::buscar::ventana::poner_tope(&gotv_core::config::global_path(), &id, tope).map_err(|e| e.to_string())
}
#[tauri::command]
async fn buscadores_orden(h: State<'_, Hub>, orden: Vec<String>, respaldo: Vec<String>, paralelo: bool, solo_local: bool, contacto: Option<String>) -> Result<(), String> {
    let ctx = ctx_buscar(&h).await;
    let cfg = gotv_core::config::global_path();
    gotv_core::buscar::ventana::poner_orden(&ctx, &cfg, &orden, &respaldo, paralelo, solo_local).map_err(|e| e.to_string())?;
    if let Some(c) = contacto {
        gotv_core::buscar::ventana::poner_contacto(&cfg, &c).map_err(|e| e.to_string())?;
    }
    Ok(())
}
#[tauri::command]
async fn buscadores_ficha(h: State<'_, Hub>, id: String) -> Result<serde_json::Value, String> {
    gotv_core::buscar::ventana::ficha(&ctx_buscar(&h).await, &id).map_err(|e| e.to_string())
}
/// F23.L-2: vuelve a leer las condiciones del servicio y compara su huella.
#[tauri::command]
async fn buscadores_revisar(h: State<'_, Hub>, id: String) -> Result<serde_json::Value, String> {
    gotv_core::buscar::ventana::revisar_condiciones(&ctx_buscar(&h).await, &id).await.map_err(|e| e.to_string())
}
#[tauri::command]
async fn buscadores_uso(h: State<'_, Hub>) -> Result<serde_json::Value, String> {
    Ok(gotv_core::buscar::ventana::uso(&ctx_buscar(&h).await))
}
/// Guarda las semillas y rastrea en segundo plano (robots.txt, noindex y 1 página por segundo por dominio).
#[tauri::command]
async fn indice_semillas(h: State<'_, Hub>, semillas: Vec<String>) -> Result<(), String> {
    let ctx = ctx_buscar(&h).await;
    gotv_core::indice::poner_semillas(&gotv_core::config::global_path(), &semillas).map_err(|e| e.to_string())?;
    let dir = ctx.almacen.indice_dir.clone();
    tokio::spawn(async move {
        let _ = gotv_core::indice::rastrear_semillas(&dir).await;
    });
    Ok(())
}
#[tauri::command]
async fn indice_estado(h: State<'_, Hub>) -> Result<serde_json::Value, String> {
    Ok(gotv_core::indice::estado(&ctx_buscar(&h).await.almacen.indice_dir))
}
/// F23.L-12: borra `~/.cache/gotv/busquedas.db` y `~/.cache/gotv/indice/` (rutas exactas).
#[tauri::command]
async fn busquedas_borrar(h: State<'_, Hub>) -> Result<serde_json::Value, String> {
    gotv_core::buscar::ventana::borrar(&ctx_buscar(&h).await).map_err(|e| e.to_string())
}

// ── F1 paso 8: bases de datos del proyecto ABIERTO (registro .gotv/bases.gotv; claves en el Llavero) ──
#[tauri::command]
async fn bases_lista(h: State<'_, Hub>) -> Result<serde_json::Value, String> {
    use gotv_core::datos::registro_bases as registro;
    let root = h.root.lock().await.clone();
    let bases: Vec<serde_json::Value> = registro::cargar(&root)
        .iter()
        .map(|c| {
            let donde = c.archivo.clone().or(c.destino.clone()).unwrap_or_default();
            // dónde está la clave (nunca su valor)
            json!({"nombre": c.nombre, "motor": c.motor.nombre(), "donde": donde, "tunel": c.tunel, "produccion": c.produccion, "mascara": c.mascara, "clave": c.secreto}) // juez-idioma: datos (claves del JSON que lee la ventana)
        })
        .collect();
    let r2 = root.clone();
    let detectadas = tokio::task::spawn_blocking(move || gotv_core::datos::detectar::detectar(&r2)).await.map_err(|e| e.to_string())?;
    let detectadas: Vec<serde_json::Value> = detectadas
        .iter()
        .map(|c| {
            let donde = c.archivo.clone().or(c.destino.clone()).unwrap_or_default();
            let url = c.destino.as_ref().map(|d| format!("{}://{d}", c.motor.nombre()));
            json!({"nombre": c.nombre, "motor": c.motor.nombre(), "donde": donde, "url": url, "secreto": c.secreto}) // juez-idioma: datos (claves del JSON que lee la ventana)
        })
        .collect();
    let ssh: Vec<String> = gotv_core::conexiones::Registro::cargar(&root).map(|r| r.conexiones.into_iter().map(|c| c.nombre).collect()).unwrap_or_default();
    Ok(json!({"proyecto": root.display().to_string(), "bases": bases, "detectadas": detectadas, "ssh": ssh}))
}

#[tauri::command]
async fn bases_agregar(nombre: String, destino: String, tunel: Option<String>, produccion: bool, mascara: Vec<String>, secreto: Option<String>, h: State<'_, Hub>) -> Result<String, String> {
    let root = h.root.lock().await.clone();
    let d = destino.trim().to_string();
    let mut a = json!({"action": "agregar", "nombre": nombre.trim(), "produccion": produccion, "mascara": mascara});
    if d.contains("://") {
        a["url"] = json!(d);
        if let Some(t) = tunel.filter(|t| !t.is_empty()) {
            a["tunel"] = json!(t);
        }
        if let Some(s) = secreto.filter(|s| s.starts_with("env:")) {
            a["secreto"] = json!(s);
        }
    } else {
        a["archivo"] = json!(d); // juez-idioma: datos (claves del JSON que lee la ventana)
    }
    tokio::task::spawn_blocking(move || gotv_core::datos::ejecutar(&a, &root)).await.map_err(|e| e.to_string())?.map_err(|e| e.to_string())
}

#[tauri::command]
async fn bases_probar(nombre: String, h: State<'_, Hub>) -> Result<String, String> {
    let root = h.root.lock().await.clone();
    gotv_core::datos::ejecutar_async(json!({"action": "probar", "conexion": nombre}), root).await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn bases_quitar(nombre: String, h: State<'_, Hub>) -> Result<(), String> {
    let root = h.root.lock().await.clone();
    gotv_core::datos::registro_bases::quitar(&root, &nombre).map(|_| ()).map_err(|e| e.to_string())
}

/// La clave tecleada en el panel: directo al Llavero; nunca se devuelve ni se registra.
#[tauri::command]
async fn bases_secreto(nombre: String, valor: String, h: State<'_, Hub>) -> Result<String, String> {
    let root = h.root.lock().await.clone();
    let r = gotv_core::datos::guardar_secreto(&root, &nombre, &valor).map_err(|e| e.to_string());
    drop(valor);
    r
}

#[tauri::command]
async fn bases_marcar(nombre: String, produccion: bool, h: State<'_, Hub>) -> Result<(), String> {
    use gotv_core::datos::registro_bases as registro;
    let root = h.root.lock().await.clone();
    let mut c = registro::buscar(&root, &nombre).ok_or_else(|| gotv_core::tr!("«{nombre}» no está registrada", "«{nombre}» is not registered"))?;
    c.produccion = produccion;
    registro::agregar(&root, c).map_err(|e| e.to_string())
}

// ── Ola H: conexiones SSH del proyecto ABIERTO (cada proyecto ve sólo las suyas) ──────────
#[tauri::command]
async fn conexiones_lista(h: State<'_, Hub>) -> Result<serde_json::Value, String> {
    use gotv_core::conexiones as cx;
    let root = h.root.lock().await.clone();
    let reg = cx::Registro::cargar(&root).map_err(|e| e.to_string())?;
    let kh = std::fs::read_to_string(cx::ruta_known_hosts(&root)).unwrap_or_default();
    let lista: Vec<serde_json::Value> = reg
        .conexiones
        .iter()
        .map(|c| {
            let fijada = kh.lines().any(|l| l.split_whitespace().next().is_some_and(|x| x == c.host || x == format!("[{}]:{}", c.host, c.puerto.unwrap_or(22))));
            json!({"nombre": c.nombre, "destino": c.destino(), "puerto": c.puerto, "salto": c.salto, "carpeta": c.carpeta, "nota": c.nota,
                   "solo_lectura": c.solo_lectura, "llave": c.llave.is_some(), "fijada": fijada || c.salto.is_some() && kh.contains(&c.host)})
        })
        .collect();
    // la bitácora la lee el núcleo (texto propio; las líneas JSON de antes también): la más nueva primero
    let log: Vec<cx::Bitacora> = cx::registro(&root, 8).into_iter().rev().collect();
    Ok(json!({"proyecto": root.display().to_string(), "conexiones": lista, "registro": log}))
}

#[tauri::command]
async fn conexiones_agregar(nombre: String, destino: String, salto: Option<String>, nota: Option<String>, solo_lectura: bool, h: State<'_, Hub>) -> Result<(), String> {
    use gotv_core::conexiones as cx;
    let root = h.root.lock().await.clone();
    let mut reg = cx::Registro::cargar(&root).map_err(|e| e.to_string())?;
    let (usuario, resto) = match destino.trim().split_once('@') {
        Some((u, x)) => (Some(u.to_string()), x.to_string()),
        None => (None, destino.trim().to_string()),
    };
    let (host, puerto) = match resto.rsplit_once(':') {
        Some((x, p)) if !x.contains(':') && p.chars().all(|c| c.is_ascii_digit()) && !p.is_empty() => (x.to_string(), p.parse().ok()),
        _ => (resto, None),
    };
    let vacio = |o: Option<String>| o.filter(|x| !x.trim().is_empty());
    reg.agregar(cx::Conexion { nombre: nombre.trim().to_string(), host, usuario, puerto, salto: vacio(salto), nota: vacio(nota), solo_lectura, ..Default::default() }).map_err(|e| e.to_string())?;
    reg.guardar(&root).map_err(|e| e.to_string())
}

#[tauri::command]
async fn conexiones_quitar(nombre: String, h: State<'_, Hub>) -> Result<(), String> {
    use gotv_core::conexiones as cx;
    let root = h.root.lock().await.clone();
    let mut reg = cx::Registro::cargar(&root).map_err(|e| e.to_string())?;
    reg.quitar(&nombre).map_err(|e| e.to_string())?;
    reg.guardar(&root).map_err(|e| e.to_string())
}

/// Crea (si falta) la llave de la conexión y devuelve la línea PÚBLICA para autorizarla.
#[tauri::command]
async fn conexiones_llave(nombre: String, h: State<'_, Hub>) -> Result<String, String> {
    use gotv_core::conexiones as cx;
    let root = h.root.lock().await.clone();
    let mut reg = cx::Registro::cargar(&root).map_err(|e| e.to_string())?;
    let mut c = reg.buscar(&nombre).cloned().ok_or("no existe")?;
    let (privada, publica) = cx::generar_llave(&root, &nombre).map_err(|e| e.to_string())?;
    c.llave = Some(privada.display().to_string());
    reg.agregar(c).map_err(|e| e.to_string())?;
    reg.guardar(&root).map_err(|e| e.to_string())?;
    Ok(publica)
}

/// Las huellas que ofrece el servidor, para que el usuario las compare ANTES de fijarlas.
#[tauri::command]
async fn conexiones_escanear(nombre: String, h: State<'_, Hub>) -> Result<serde_json::Value, String> {
    use gotv_core::conexiones as cx;
    let root = h.root.lock().await.clone();
    let reg = cx::Registro::cargar(&root).map_err(|e| e.to_string())?;
    let c = reg.buscar(&nombre).cloned().ok_or("no existe")?;
    let objetivo = match &c.salto {
        Some(j) => reg.buscar(j).cloned().ok_or("salto inexistente")?,
        None => c.clone(),
    };
    let (_, huellas) = tokio::task::spawn_blocking(move || cx::escanear(&objetivo)).await.map_err(|e| e.to_string())?.map_err(|e| e.to_string())?;
    Ok(json!({"host": c.salto.clone().unwrap_or(c.host.clone()), "huellas": huellas}))
}

/// El usuario confirmó las huellas: se fijan y se prueba la conexión con una orden inocua.
#[tauri::command]
async fn conexiones_fijar_y_probar(nombre: String, h: State<'_, Hub>) -> Result<String, String> {
    use gotv_core::conexiones as cx;
    let root = h.root.lock().await.clone();
    let reg = cx::Registro::cargar(&root).map_err(|e| e.to_string())?;
    let c = reg.buscar(&nombre).cloned().ok_or("no existe")?;
    let objetivo = match &c.salto {
        Some(j) => reg.buscar(j).cloned().ok_or("salto inexistente")?,
        None => c.clone(),
    };
    let o2 = objetivo.clone();
    let (lineas, _) = tokio::task::spawn_blocking(move || cx::escanear(&o2)).await.map_err(|e| e.to_string())?.map_err(|e| e.to_string())?;
    cx::fijar_huella(&root, &objetivo, &lineas).map_err(|e| e.to_string())?;
    if c.salto.is_some() {
        // la máquina de atrás se aprende por el bastión ya verificado (accept-new sólo esta vez)
        let mut o = cx::opciones(&root, &reg, &c, false).map_err(|e| e.to_string())?;
        if let Some(i) = o.iter().position(|a| a == "StrictHostKeyChecking=yes") {
            o[i] = "StrictHostKeyChecking=accept-new".into();
        }
        o.extend([c.destino(), "--".into(), "true".into()]);
        let st = tokio::process::Command::new("ssh").args(&o).status().await.map_err(|e| e.to_string())?;
        if !st.success() {
            return Err(gotv_core::i18n::t("no pude llegar por el salto", "could not get through the jump").into());
        }
    }
    let (rc, out) = cx::ejecutar(&root, &nombre, "echo \"$(whoami)@$(hostname) · $(uname -sr)\"", 30).await.map_err(|e| e.to_string())?;
    if rc == 0 { Ok(out.trim().to_string()) } else { Err(out) }
}

/// W3: la extensión de Chrome — dónde está, si el puente está instalado y si conectó.
fn chrome_ext_dir() -> std::path::PathBuf {
    gotv_core::chrome::carpeta_extension()
}

#[tauri::command]
async fn chrome_status() -> Result<serde_json::Value, String> {
    let _ = gotv_core::chrome::ensure_server();
    let manifest = gotv_core::chrome::user_nm_dir().join(format!("{}.json", gotv_core::chrome::HOST_NAME));
    let ext = chrome_ext_dir();
    Ok(json!({
        "installed": manifest.exists(),
        "connected": gotv_core::chrome::connected(),
        "ext_dir": ext.canonicalize().unwrap_or(ext).display().to_string(),
    }))
}

#[tauri::command]
async fn chrome_install() -> Result<serde_json::Value, String> {
    let me = std::env::current_exe().map_err(|e| e.to_string())?;
    // el CLI viaja junto a la app; en desarrollo, el de target/
    let gotv = me.parent().map(|d| d.join("gotv")).filter(|p| p.exists()).ok_or_else(|| gotv_core::i18n::t("no encuentro el ejecutable gotv junto a la app", "cannot find the gotv executable next to the app").to_string())?;
    gotv_core::chrome::install_host(&gotv_core::chrome::user_nm_dir(), &gotv, &[]).map_err(|e| e.to_string())?;
    chrome_status().await
}

/// Muestra una carpeta en Finder (sólo carpetas que GOTV conoce: la de la extensión).
#[tauri::command]
async fn reveal_ext() -> Result<(), String> {
    std::process::Command::new("open").arg(chrome_ext_dir()).spawn().map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
async fn answer_choice(id: u64, value: Option<String>, h: State<'_, Hub>) -> Result<(), String> {
    if let Some(tx) = h.choices.lock().await.remove(&id) {
        let _ = tx.send(value.filter(|v| !v.trim().is_empty()));
    }
    Ok(())
}

#[tauri::command]
async fn answer(id: u64, ans: String, h: State<'_, Hub>) -> Result<(), String> {
    let a = match ans.as_str() {
        "yes" => AskAnswer::Yes,
        "always" => AskAnswer::Always,
        "never" => AskAnswer::Never,
        _ => AskAnswer::No,
    };
    if let Some(tx) = h.asks.lock().await.remove(&id) {
        let _ = tx.send(a);
    }
    Ok(())
}

#[tauri::command]
fn stop_turn(h: State<'_, Hub>) {
    h.cancel.store(true, Ordering::Relaxed);
}

#[tauri::command]
async fn info(h: State<'_, Hub>) -> Result<serde_json::Value, String> {
    let mut v = h.status.lock().await.clone();
    let st = store::Store::load();
    v["lang"] = json!(if gotv_core::i18n::is_en() { "en" } else { "es" });
    v["theme"] = json!(st.theme);
    v["desktop_control"] = json!(gotv_core::Config::load(&h.root.lock().await.clone()).desktop_control);
    v["projects"] = json!(st.live_projects());
    v["root"] = json!(h.root.lock().await.display().to_string());
    v["home"] = json!(std::env::var("HOME").unwrap_or_default());
    // la carpeta de las charlas sin proyecto: con ella la barra sabe si lo abierto es «Chat» o «Code»
    v["sueltas"] = json!(gotv_core::sueltas::base().display().to_string());
    v["tasks"] = json!(h.tasks.lock().await.clone());
    v["procs"] = procs_view(&h.procs_owner.lock().unwrap().clone());
    Ok(v)
}

/// Selector NATIVO de carpetas de macOS (elegir una carpeta ahí ya le da acceso a la app).
#[tauri::command]
async fn pick_folder(app: AppHandle) -> Option<String> {
    use tauri_plugin_dialog::DialogExt;
    let (tx, rx) = oneshot::channel();
    app.dialog().file().pick_folder(move |f| {
        let _ = tx.send(f.and_then(|p| p.into_path().ok()).map(|p| p.display().to_string()));
    });
    rx.await.ok().flatten()
}

/// Selector NATIVO de archivos (varios) para adjuntar.
#[tauri::command]
async fn pick_files(app: AppHandle) -> Vec<String> {
    use tauri_plugin_dialog::DialogExt;
    let (tx, rx) = oneshot::channel();
    app.dialog().file().pick_files(move |f| {
        let paths = f.unwrap_or_default().into_iter().filter_map(|p| p.into_path().ok()).map(|p| p.display().to_string()).collect();
        let _ = tx.send(paths);
    });
    rx.await.unwrap_or_default()
}

/// «Guardar como…» de lo generado (clic derecho en una imagen o un audio del chat): el diálogo
/// NATIVO de macOS (abre en Descargas, con el nombre del archivo) y una copia. Sólo archivos del
/// proyecto, como «Abrir». `None` si la persona cancela.
#[tauri::command]
async fn guardar_como(app: AppHandle, path: String, h: State<'_, Hub>) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    let root = h.root.lock().await.clone();
    let p = std::fs::canonicalize(&path).map_err(|e| e.to_string())?;
    let raiz = std::fs::canonicalize(&root).map_err(|e| e.to_string())?;
    if !p.starts_with(&raiz) || !p.is_file() {
        return Err(gotv_core::i18n::t("sólo se guardan archivos del proyecto", "only project files are saved").into());
    }
    let nombre = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let mut d = app.dialog().file().set_file_name(&nombre);
    if let Some(ext) = p.extension().and_then(|e| e.to_str()) {
        d = d.add_filter(ext.to_uppercase(), &[ext]);
    }
    if let Some(casa) = std::env::var_os("HOME") {
        d = d.set_directory(PathBuf::from(casa).join("Downloads"));
    }
    let (tx, rx) = oneshot::channel();
    d.save_file(move |f| {
        let _ = tx.send(f.and_then(|p| p.into_path().ok()));
    });
    let Some(destino) = rx.await.ok().flatten() else { return Ok(None) };
    std::fs::copy(&p, &destino).map_err(|e| e.to_string())?;
    Ok(Some(destino.display().to_string()))
}

/// El icono VIVO del Dock: la página pinta la X con el motor del logo y manda cada cuadro (PNG en
/// base64, ~10 por segundo, sólo al nacer y mientras GOTV trabaja). `None` devuelve el icono del
/// paquete. AppKit exige el hilo principal; fuera de macOS no hace nada.
#[tauri::command]
fn dock_icono(app: AppHandle, png: Option<String>) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        use base64::Engine as _;
        let bytes = match png {
            Some(b) => Some(base64::engine::general_purpose::STANDARD.decode(b.as_bytes()).map_err(|e| e.to_string())?),
            None => None,
        };
        app.run_on_main_thread(move || {
            use objc2::AllocAnyThread as _;
            use objc2_app_kit::{NSApplication, NSImage};
            let Some(mtm) = objc2::MainThreadMarker::new() else { return };
            let nsapp = NSApplication::sharedApplication(mtm);
            let imagen = bytes.and_then(|b| NSImage::initWithData(NSImage::alloc(), &objc2_foundation::NSData::with_bytes(&b)));
            // SAFETY: en el hilo principal (lo garantiza `mtm`); `None` restaura el icono del paquete
            unsafe { nsapp.setApplicationIconImage(imagen.as_deref()) };
        })
        .map_err(|e| e.to_string())?;
    }
    #[cfg(not(target_os = "macos"))]
    let _ = (app, png);
    Ok(())
}

/// ¿El usuario ya confió en esta carpeta?
#[tauri::command]
fn is_trusted(path: String) -> bool {
    store::Store::load().is_trusted(&PathBuf::from(path))
}

/// Abre un proyecto (`trust` = el usuario aceptó el diálogo de confianza).
#[tauri::command]
async fn open_project(path: String, trust: bool, h: State<'_, Hub>) -> Result<(), String> {
    let p = PathBuf::from(&path);
    let p = p.canonicalize().map_err(|e| gotv_core::tr!("no puedo abrir {path}: {e}", "cannot open {path}: {e}"))?;
    if !p.is_dir() {
        return Err(gotv_core::tr!("{path} no es una carpeta", "{path} is not a folder"));
    }
    // probar el acceso de verdad: aquí macOS pide su permiso (Documentos, Escritorio…)
    acceso::request_for_project(&p)?;
    std::fs::read_dir(&p).map_err(|e| gotv_core::tr!("macOS no dio acceso a {path}: {e}", "macOS denied access to {path}: {e}"))?;
    let mut st = store::Store::load();
    if !st.is_trusted(&p) {
        if !trust {
            return Err("untrusted".into());
        }
        st.trust(&p);
    }
    st.touch(&p);
    st.save();
    h.tx.send(Msg::Open(p)).map_err(|e| e.to_string())
}

/// Un proyecto = la carpeta que el usuario eligió, TAL CUAL (no se crea nada dentro).
/// El nombre es sólo para la app (vacío → el de la carpeta). Confiada: la eligió él.
/// (Medido 2026-11-09: antes se creaba `<ruta>/<nombre>` vacía, sin sus archivos.)
#[tauri::command]
async fn new_project(path: String, name: String, model: Option<String>, h: State<'_, Hub>) -> Result<String, String> {
    let p = PathBuf::from(path.trim());
    if path.trim().is_empty() {
        return Err(gotv_core::i18n::t("elige la carpeta del proyecto", "choose the project folder").into());
    }
    if !p.is_dir() {
        return Err(gotv_core::tr!(
            "{} no existe o no es una carpeta (en «Elegir…» puedes crear una nueva)",
            "{} does not exist or is not a folder (you can create a new one in «Choose…»)",
            p.display()
        ));
    }
    let p = p.canonicalize().map_err(|e| e.to_string())?;
    // probar el acceso de verdad: aquí macOS pide su permiso (Documentos, Escritorio…)
    // el permiso de macOS se pide AHORA, al crear el proyecto, y queda guardado
    acceso::request_for_project(&p)?;
    std::fs::read_dir(&p).map_err(|e| gotv_core::tr!("macOS no dio acceso a {}: {e}", "macOS denied access to {}: {e}", p.display()))?;
    let mut st = store::Store::load();
    st.trust(&p);
    st.touch_named(&p, Some(&name));
    // F5: el modelo de este proyecto, elegido al crearlo (al abrirlo se carga éste)
    if let Some(m) = model.as_deref().map(str::trim).filter(|m| !m.is_empty()) {
        st.set_project_model(&p, m);
    }
    st.save();
    h.tx.send(Msg::Open(p.clone())).map_err(|e| e.to_string())?;
    Ok(p.display().to_string())
}

/// Cierra el proyecto abierto (se quitó de la lista): el agente se suelta y la app
/// queda sin proyecto hasta que se cree o abra otro. El motor sigue cargado.
#[tauri::command]
fn close_project(h: State<'_, Hub>) -> Result<(), String> {
    h.tx.send(Msg::Close).map_err(|e| e.to_string())
}

#[tauri::command]
fn forget_project(path: String) {
    let mut st = store::Store::load();
    st.projects.retain(|p| p.path != path);
    // quitarlo de la lista retira también la confianza: volver a abrirlo vuelve a preguntar
    st.trusted.retain(|t| t != &path);
    st.save();
}

/// Comandos y skills para el autocompletado de `/`.
#[tauri::command]
async fn slash_items(h: State<'_, Hub>) -> Result<serde_json::Value, String> {
    let mut items = vec![
        json!({"name": "clear", "desc": gotv_core::i18n::t("conversación nueva", "new conversation"), "kind": "cmd"}),
        json!({"name": "compact", "desc": gotv_core::i18n::t("resume el contexto ahora", "summarize the context now"), "kind": "cmd"}),
        json!({"name": "context", "desc": gotv_core::i18n::t("uso de la ventana de contexto", "context window usage"), "kind": "cmd"}),
        json!({"name": "mode", "desc": gotv_core::i18n::t("plan · default · acceptEdits · bypass", "plan · default · acceptEdits · bypass"), "kind": "cmd"}),
        json!({"name": "rewind", "desc": gotv_core::i18n::t("deshace el último turno que cambió archivos (/rewind paso: sólo el último paso)", "undoes the last turn that changed files (/rewind step: only the last step)"), "kind": "cmd"}),
        json!({"name": gotv_core::i18n::t("modelos", "models"), "desc": gotv_core::i18n::t("modelos de Hugging Face que corren en esta Mac, con su velocidad", "Hugging Face models that run on this Mac, with their speed"), "kind": "cmd"}),
        json!({"name": gotv_core::i18n::t("descargar", "download"), "desc": gotv_core::i18n::t("baja un modelo: /descargar org/nombre", "downloads a model: /download org/name"), "kind": "cmd"}),
        json!({"name": "help", "desc": gotv_core::i18n::t("ayuda", "help"), "kind": "cmd"}),
    ];
    // F12 · formas de trabajar (`entrevista` y `aprender` son skills incluidas: se listan una vez, como orden)
    items.extend(gotv_core::formas::comandos().into_iter().map(|(n, d)| json!({"name": n, "desc": d, "kind": "cmd"})));
    let root = h.root.lock().await.clone();
    for s in gotv_core::skills::discover(&root).into_iter().filter(|s| !gotv_core::formas::es_comando(&s.name)) {
        items.push(json!({"name": s.name, "desc": s.description.chars().take(140).collect::<String>(), "kind": "skill"}));
    }
    Ok(json!(items))
}

/// Adjunta un archivo de disco a la próxima orden.
#[tauri::command]
async fn attach_path(path: String, h: State<'_, Hub>) -> Result<attach::Chip, String> {
    let root = h.root.lock().await.clone();
    let (vision, hears) = *h.caps.lock().await;
    let p = PathBuf::from(&path);
    let (payload, kind, detail) = tokio::task::spawn_blocking(move || attach::process(&root, &p, vision, hears))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;
    let chip = attach::Chip {
        id: h.id(),
        name: PathBuf::from(&path).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or(path),
        kind,
        detail,
    };
    h.pending.lock().await.push((chip.clone(), payload));
    Ok(chip)
}

/// Adjunta bytes (arrastrar y soltar desde la ventana, pegar una imagen…).
#[tauri::command]
async fn attach_bytes(name: String, b64: String, h: State<'_, Hub>) -> Result<attach::Chip, String> {
    let bytes = base64_decode(&b64).ok_or("base64")?;
    let safe: String = name.chars().map(|c| if c.is_alphanumeric() || ".-_".contains(c) { c } else { '_' }).collect();
    let tmp = std::env::temp_dir().join(format!("gotv-adj-{}-{safe}", now()));
    std::fs::write(&tmp, bytes).map_err(|e| e.to_string())?;
    let r = attach_path(tmp.display().to_string(), h).await;
    let _ = std::fs::remove_file(&tmp);
    r.map(|mut c| {
        c.name = name;
        c
    })
}

#[tauri::command]
async fn detach(id: u64, h: State<'_, Hub>) -> Result<(), String> {
    h.pending.lock().await.retain(|(c, _)| c.id != id);
    Ok(())
}

/// 🎙 empieza a grabar.
#[tauri::command]
async fn voice_start(h: State<'_, Hub>) -> Result<(), String> {
    let r = attach::record_start().map_err(|e| e.to_string())?;
    *h.recording.lock().await = Some(r);
    Ok(())
}

/// 🔊 M2: lee en voz alta una respuesta (markdown) con la voz de GOTV, en el núcleo. Suena mientras se
/// genera; otra lectura o `voz_callar` corta la anterior. Al terminar emite `{kind:"voz", fin:true, id}`:
/// el `id` de la lectura, para que el fin TARDÍO de una lectura callada no apague la que ya suena.
#[tauri::command]
async fn voz_leer(h: State<'_, Hub>, texto: String, id: Option<u64>) -> Result<(), String> {
    let dir = gotv_core::generadores::voz_dir().map_err(|e| e.to_string())?;
    let callar = gotv_mlx::Callar::default();
    if let Some(vieja) = h.voz.lock().map_err(|e| e.to_string())?.replace(callar.clone()) {
        vieja.ya();
    }
    let (app, estado) = (h.app.clone(), h.voz.clone());
    let es = !gotv_core::i18n::is_en();
    tokio::task::spawn_blocking(move || {
        let r = (|| -> anyhow::Result<()> {
            let altavoz = gotv_sonido::Altavoz::abrir(24000)?;
            let idioma = if es { "spanish" } else { "english" };
            for trozo in gotv_core::lectura::trozos(&texto, es) {
                if callar.pedido() {
                    break;
                }
                gotv_mlx::hablar(&dir, &trozo, idioma, false, &callar, &mut |w| {
                    if callar.pedido() {
                        altavoz.callar();
                    } else {
                        altavoz.empujar(w);
                    }
                    Ok(())
                })?;
            }
            // que termine de sonar, salvo que la callen
            while altavoz.sonando() {
                if callar.pedido() {
                    altavoz.callar();
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(30));
            }
            if !callar.pedido() {
                altavoz.esperar();
            }
            Ok(())
        })();
        // sólo la lectura VIGENTE limpia el estado (otra pudo reemplazarla)
        if let Ok(mut g) = estado.lock() {
            if g.as_ref().is_some_and(|c| std::sync::Arc::ptr_eq(&c.bandera(), &callar.bandera())) {
                *g = None;
            }
        }
        emit(&app, json!({"kind": "voz", "fin": true, "id": id, "error": r.err().map(|e| e.to_string())}));
    });
    Ok(())
}

/// 🎧 M3 · manos libres: GOTV te escucha (micrófono + detector de voz + Whisper, en el núcleo), lo que
/// dices entra al chat como tu mensaje, y te responde HABLANDO mientras escribe; si le hablas encima,
/// calla y corta el turno. Es la misma charla de `gotv conversar` (crate `gotv-voz`). Estados por el
/// evento `{kind:"manos", estado}`: saludando · escuchando · oyendo · pensando · apagado.
#[tauri::command]
async fn manos_libres(h: State<'_, Hub>, on: bool) -> Result<(), String> {
    if !on {
        if let Some(c) = h.charla.lock().map_err(|e| e.to_string())?.take() {
            c.cerrar();
        }
        emit(&h.app, json!({"kind": "manos", "estado": "apagado"}));
        return Ok(());
    }
    if h.charla.lock().map_err(|e| e.to_string())?.is_some() {
        return Ok(());
    }
    // una lectura en voz alta en curso se calla: la voz es una sola
    if let Some(c) = h.voz.lock().map_err(|e| e.to_string())?.take() {
        c.ya();
    }
    let dir = gotv_core::generadores::voz_dir().map_err(|e| e.to_string())?;
    let es = !gotv_core::i18n::is_en();
    let (charla, rx) = gotv_voz::Charla::abrir(dir, es).map_err(|e| e.to_string())?;
    let charla = Arc::new(charla);
    *h.charla.lock().map_err(|e| e.to_string())? = Some(charla.clone());
    h.voz_instruccion.store(true, Ordering::Relaxed);
    tauri::async_runtime::spawn(bucle_de_voz(h.inner().clone(), charla, rx));
    Ok(())
}

async fn bucle_de_voz(hub: Hub, charla: Arc<gotv_voz::Charla>, mut rx: tokio::sync::mpsc::UnboundedReceiver<gotv_voz::Oido>) {
    use gotv_voz::Oido;
    let estado = |e: &str| emit(&hub.app, json!({"kind": "manos", "estado": e}));
    estado("saludando");
    {
        let c = charla.clone();
        let saludo = gotv_core::i18n::t("Te escucho, Portador.", "I'm listening.");
        let _ = tokio::task::spawn_blocking(move || c.saludar(saludo)).await;
    }
    estado("escuchando");
    let mut en_curso: Option<(u64, tokio::task::JoinHandle<anyhow::Result<String>>)> = None;
    while let Some(ev) = rx.recv().await {
        match ev {
            Oido::Inicio => estado("oyendo"),
            Oido::Borrador(id, a) => en_curso = Some((id, tokio::task::spawn_blocking(move || gotv_voz::transcribir(&a)))),
            Oido::Interrumpir => {
                hub.cancel.store(true, Ordering::Relaxed);
                charla.interrumpir();
                estado("oyendo");
            }
            Oido::Frase(a, vale) => {
                let texto = match (en_curso.take(), vale) {
                    (Some((id, j)), Some(v)) if id == v => j.await.map_err(anyhow::Error::from).and_then(|r| r),
                    _ => tokio::task::spawn_blocking(move || gotv_voz::transcribir(&a)).await.map_err(anyhow::Error::from).and_then(|r| r),
                };
                let texto = match texto {
                    Ok(t) if t.chars().filter(|c| c.is_alphanumeric()).count() >= 2 => t,
                    Ok(_) => {
                        estado("escuchando");
                        continue;
                    }
                    Err(e) => {
                        emit(&hub.app, json!({"kind": "manos", "estado": "escuchando", "error": e.to_string()}));
                        continue;
                    }
                };
                // K5: una transcripción dudosa («Śwesne! Po») NO se obedece: se pide repetir
                if gotv_voz::transcripcion_dudosa(&texto) {
                    emit(&hub.app, json!({"kind": "voz_tu", "text": gotv_core::tr!("{texto}  ⟵ (no te entendí bien: repítelo)", "{texto}  ⟵ (I did not catch that: say it again)")}));
                    let c = charla.clone();
                    let frase = gotv_core::i18n::t("No te entendí bien. ¿Me lo repites?", "I did not catch that. Can you say it again?");
                    let _ = tokio::task::spawn_blocking(move || c.saludar(frase)).await;
                    estado("escuchando");
                    continue;
                }
                // tu frase entra al chat como tu mensaje; la instrucción de voz, sólo con la primera
                emit(&hub.app, json!({"kind": "voz_tu", "text": texto}));
                let orden = if hub.voz_instruccion.swap(false, Ordering::Relaxed) {
                    // al reabrir, el chat no la muestra (`gotv_core::mensajes::tu_mensaje`)
                    format!("{}\n\n{texto}", gotv_core::mensajes::instruccion_manos_libres())
                } else {
                    texto
                };
                estado("pensando");
                // con el check «Control Mac» y el perfil del chat, como una orden escrita
                let mac = hub.chat_mac.load(Ordering::Relaxed);
                let perfil = hub.chat_perfil.lock().ok().and_then(|p| p.clone());
                let pro = hub.chat_pro.lock().map(|p| p.clone()).unwrap_or_default();
                if hub.tx.send(Msg::Task(orden, mac, perfil, None, Box::new(pro))).is_err() {
                    break;
                }
            }
        }
    }
    estado("apagado");
}

/// ⚡ La caché de la conversación del proyecto abierto: si está activa y cuánto ocupa en disco.
#[tauri::command]
async fn cache_conversacion(h: State<'_, Hub>, on: Option<bool>, tope_gb: Option<u64>) -> Result<serde_json::Value, String> {
    let root = h.root.lock().await.clone();
    if root.as_os_str().is_empty() {
        return Ok(json!({"on": false, "bytes": 0, "tope": gotv_engine::memoria_tope::TOPE_POR_OMISION}));
    }
    if let Some(on) = on {
        gotv_core::conversaciones::activar(&root, on).map_err(|e| e.to_string())?;
        // F4: encendida → la memoria de esta conversación se construye ya (con su barra de progreso)
        if on {
            let _ = h.tx.send(worker::Msg::Rapido);
        }
    }
    // F3: el tope por proyecto (3 · 6 · 12 · 24 GB); al bajarlo se desaloja lo que no cabe
    if let Some(gb) = tope_gb.filter(|g| gotv_engine::memoria_tope::TOPES_GB.contains(g)) {
        gotv_core::conversaciones::fijar_tope(&root, gb).map_err(|e| e.to_string())?;
    }
    Ok(json!({
        "on": gotv_core::conversaciones::activa(&root),
        "bytes": gotv_core::conversaciones::bytes(&root),
        "tope": gotv_core::conversaciones::tope(&root),
        "topes_gb": gotv_engine::memoria_tope::TOPES_GB,
    }))
}

/// El check «Control Mac» y el perfil del chat, para las órdenes dichas en manos libres (la interfaz
/// lo llama al cambiarlos y al arrancar).
#[tauri::command]
fn chat_estado(mac: bool, perfil: Option<String>, pro: Option<serde_json::Value>, h: State<'_, Hub>) {
    h.chat_mac.store(mac, Ordering::Relaxed);
    if let Ok(mut p) = h.chat_perfil.lock() {
        *p = perfil.filter(|x| !x.trim().is_empty());
    }
    if let Ok(mut p) = h.chat_pro.lock() {
        *p = leer_pro(pro);
    }
}

/// Guarda cómo dejó el Portador el chat de un proyecto (Control Mac, perfil, manos libres): la
/// interfaz lo llama cuando ÉL los cambia, y al abrir el proyecto los vuelve a poner (vienen con
/// `info().projects[].chat`).
#[tauri::command]
fn guardar_chat(proyecto: String, mac: bool, perfil: Option<String>, manos: bool) {
    let mut st = store::Store::load();
    // los paneles del modo profesional se guardan aparte (`guardar_pro`): aquí se conservan
    let pro = st.project_chat(Path::new(&proyecto)).pro;
    st.set_project_chat(Path::new(&proyecto), store::Chat { mac, perfil, manos, pro });
    st.save();
}

/// 🔊 calla la lectura en curso (si hay).
#[tauri::command]
fn voz_callar(h: State<'_, Hub>) -> Result<(), String> {
    if let Some(c) = h.voz.lock().map_err(|e| e.to_string())?.take() {
        c.ya();
    }
    Ok(())
}

/// 🎙 para: si el modelo OYE, el audio va como adjunto nativo; si no, Whisper lo
/// dicta (NO nativo) y el texto vuelve al cuadro para editarlo.
#[tauri::command]
async fn voice_stop(h: State<'_, Hub>) -> Result<serde_json::Value, String> {
    let rec = h.recording.lock().await.take().ok_or("no grabando")?;
    let file = tokio::task::spawn_blocking(move || rec.finish()).await.map_err(|e| e.to_string())?.map_err(|e| e.to_string())?;
    let (_, hears) = *h.caps.lock().await;
    let r = if hears {
        let f = file.clone();
        let (url, secs) = tokio::task::spawn_blocking(move || gotv_core::audio::data_url(&f)).await.map_err(|e| e.to_string())?.map_err(|e| e.to_string())?;
        let chip = attach::Chip { id: h.id(), name: gotv_core::i18n::t("nota de voz", "voice note").into(), kind: "voice".into(), detail: format!("{secs:.1} s") };
        h.pending.lock().await.push((chip.clone(), attach::Payload::Media("voz.m4a".into(), url)));
        json!({"chip": chip})
    } else {
        let f = file.clone();
        match tokio::task::spawn_blocking(move || gotv_core::audio::transcribe(&f)).await.map_err(|e| e.to_string())? {
            Ok(t) => json!({"text": t}),
            Err(e) => {
                let _ = std::fs::remove_file(&file);
                return Err(gotv_core::tr!(
                    "este modelo no oye y no se pudo dictar con Whisper: {e}",
                    "this model cannot hear and Whisper could not dictate: {e}"
                ));
            }
        }
    };
    let _ = std::fs::remove_file(&file);
    Ok(r)
}

/// Abre un enlace de una respuesta en el navegador (sólo http/https: nada de
/// `file://` ni esquemas de apps que un modelo pudiera colar).
#[tauri::command]
fn open_link(url: String) -> Result<(), String> {
    let ok = (url.starts_with("https://") || url.starts_with("http://")) && !url.chars().any(|c| c.is_whitespace() || c.is_control());
    if !ok {
        return Err(gotv_core::i18n::t("sólo se abren enlaces http(s)", "only http(s) links are opened").into());
    }
    std::process::Command::new("open").arg(&url).spawn().map(|_| ()).map_err(|e| e.to_string())
}

/// A4: estado del servicio de API (direcciones, llave, modelos, peticiones).
#[tauri::command]
async fn api_status(h: State<'_, Hub>) -> Result<serde_json::Value, String> {
    let mut v = servicio::status(&h).await;
    let cfg = gotv_core::Config::load(&std::env::temp_dir());
    v["enabled"] = json!(cfg.serve_enabled);
    v["lan_wanted"] = json!(cfg.serve_lan);
    v["port_wanted"] = json!(cfg.serve_port);
    v["cloud"] = json!(cfg.serve_cloud);
    Ok(v)
}

/// A4: enciende, apaga o reconfigura el servicio, y lo deja guardado.
#[tauri::command]
async fn api_apply(enabled: bool, lan: bool, port: u16, cloud: bool, h: State<'_, Hub>) -> Result<serde_json::Value, String> {
    if !(1024..=65535).contains(&port) {
        return Err(gotv_core::i18n::t("el puerto va de 1024 a 65535", "the port goes from 1024 to 65535").into());
    }
    worker::save_config(&[
        ("serve_enabled", toml::Value::Boolean(enabled)),
        ("serve_lan", toml::Value::Boolean(lan)),
        ("serve_port", toml::Value::Integer(port as i64)),
        ("serve_cloud", toml::Value::Boolean(cloud)),
    ]);
    let cfg = gotv_core::Config::load(&std::env::temp_dir());
    if enabled {
        servicio::start(&h, &cfg).await?;
    } else {
        servicio::stop(&h).await;
    }
    api_status(h).await
}

/// A4: llave nueva para el servicio (la anterior deja de valer al instante).
#[tauri::command]
async fn api_new_key(h: State<'_, Hub>) -> Result<serde_json::Value, String> {
    let k = gotv_core::serve::new_key().map_err(|e| e.to_string())?;
    gotv_core::llaves::set(gotv_core::serve::KEY_NAME, &k).map_err(|e| e.to_string())?;
    if h.api_run.lock().await.as_ref().is_some_and(|r| !gotv_core::serve::is_loopback(&r.host)) {
        h.api.set_key(Some(k));
    }
    api_status(h).await
}

/// Biblioteca: el catálogo MLX de Hugging Face medido contra esta Mac (o `mac`
/// simulada: `"M4 Pro|48"`), con velocidad estimada, si cabe y si ya está.
#[tauri::command]
async fn library_search(query: String, org: String, sort: String, mac: String, h: State<'_, Hub>) -> Result<serde_json::Value, String> {
    libreria::search(&h, &query, &org, &sort, &mac).await
}

/// Baja un modelo de la Biblioteca en segundo plano (comprueba antes que quepa).
#[tauri::command]
async fn library_pull(repo: String, h: State<'_, Hub>) -> Result<String, String> {
    libreria::start_pull(&h, &repo).await
}

/// M0 · Generadores (voz, imagen, video, música) con su licencia y lo que hay en esta Mac.
#[tauri::command]
async fn generators_list(h: State<'_, Hub>) -> Result<serde_json::Value, String> {
    let estados = tokio::task::spawn_blocking(gotv_core::generadores::estados).await.map_err(|e| e.to_string())?;
    let pulls: Vec<serde_json::Value> = h.pulls.lock().await.iter().map(|(r, p)| serde_json::json!({"repo": r, "done": p.done, "total": p.total})).collect();
    Ok(serde_json::json!({"items": estados, "pulls": pulls, "verificado": gotv_core::generadores::VERIFICADO}))
}

/// La casilla de la licencia: marcarla guarda la aceptación (licencia, commit, fecha); quitarla la retira.
#[tauri::command]
fn generator_accept(repo: String, accept: bool) -> Result<bool, String> {
    if accept {
        // no se acepta a ciegas: antes hay que haber abierto la licencia con «Leer»
        gotv_core::generadores::aceptar_leida(&repo).map(|_| true).map_err(|e| e.to_string())
    } else {
        gotv_core::generadores::revocar(&repo).map(|_| false).map_err(|e| e.to_string())
    }
}

/// «Leer» la licencia de un generador: abre la página de SU licencia (la del catálogo, no una URL que
/// mande la ventana) y lo anota; eso desbloquea «Aceptar».
#[tauri::command]
fn generator_read(repo: String) -> Result<(), String> {
    let url = gotv_core::generadores::marcar_leida(&repo).map_err(|e| e.to_string())?;
    open_link(url.to_string())
}

/// Baja un generador en segundo plano (exige la licencia aceptada y, si es restringido, el token).
#[tauri::command]
async fn generator_pull(repo: String, h: State<'_, Hub>) -> Result<String, String> {
    libreria::start_pull_de(&h, &repo, libreria::Origen::Generador).await
}

/// El token de Hugging Face va DIRECTO al Llavero; la ventana sólo sabe si hay uno. Vacío = borrarlo.
#[tauri::command]
fn hf_token(token: String) -> Result<bool, String> {
    if token.trim().is_empty() {
        gotv_core::llaves::delete(gotv_core::generadores::LLAVE_HF);
        Ok(false)
    } else {
        gotv_core::llaves::set(gotv_core::generadores::LLAVE_HF, &token).map(|_| true).map_err(|e| e.to_string())
    }
}

/// Corta una descarga (lo bajado se reanuda la próxima vez).
#[tauri::command]
async fn library_cancel(repo: String, h: State<'_, Hub>) -> Result<bool, String> {
    Ok(libreria::cancel_pull(&h, &repo).await)
}

/// Configuración + modelos de esta Mac (con si caben) para el panel de ajustes.
#[tauri::command]
async fn settings(h: State<'_, Hub>) -> Result<serde_json::Value, String> {
    let root = h.root.lock().await.clone();
    let cfg = gotv_core::Config::load(&root);
    let status = h.status.lock().await.clone();
    let models = modelos_locales(&h).await?;
    // A3: los modelos de cada proveedor con llave (en paralelo, con tope de espera:
    // un proveedor caído no congela la ventana)
    let mut models = models;
    let provs = gotv_core::nube::providers(&cfg);
    let usable: Vec<_> = provs.iter().filter(|p| gotv_core::llaves::has(&p.name) || (p.kind == "openai" && p.base_url.starts_with("http://"))).cloned().collect();
    let lists = futures_join(usable.iter().map(|p| {
        let p = p.clone();
        async move { (p.clone(), tokio::time::timeout(std::time::Duration::from_secs(8), gotv_core::nube::models_of(&p)).await) }
    }))
    .await;
    let mut cloud_errors = vec![];
    for (p, r) in lists {
        match r {
            Ok(Ok(ms)) => {
                for m in ms {
                    let id = format!("{}:{}", p.name, m.id);
                    models.push(json!({"name": id, "label": m.name, "path": id, "kind": format!("nube · {}", p.name), "cloud": true, "provider": p.name,
                        "fits": true, "ctx_top": m.ctx, "model_max": m.ctx, "max_ctx": m.ctx, "ctx_auto": gotv_core::nube::CLOUD_DEFAULT_CTX, "vision": p.vision, "audio": false}));
                }
            }
            Ok(Err(e)) => cloud_errors.push(json!({"provider": p.name, "error": e.to_string()})),
            Err(_) => cloud_errors.push(json!({"provider": p.name, "error": gotv_core::i18n::t("no respondió en 8 s", "no answer within 8 s")})),
        }
    }
    let providers: Vec<_> = provs
        .iter()
        .map(|p| json!({"name": p.name, "kind": p.kind, "base_url": p.base_url, "vision": p.vision, "has_key": gotv_core::llaves::has(&p.name), "builtin": p.name == "anthropic" || p.name == "openai"}))
        .collect();
    let presets: Vec<_> = gotv_core::nube::PRESETS.iter().map(|(n, l, u)| json!({"name": n, "label": l, "base_url": u})).collect();
    Ok(json!({
        // la ventana del PROYECTO abierto (si no eligió una, la de la configuración)
        "lang": cfg.lang, "theme": store::Store::load().theme, "context_preset": store::Store::load().project_context(&root).unwrap_or_else(|| cfg.context_preset.clone()),
        "max_tokens": cfg.max_tokens, "temperature": cfg.temperature, "mode": status["mode"], "computer_use": cfg.computer_use, "system_changes": cfg.system_changes, "desktop_control": cfg.desktop_control, "max_steps": cfg.max_steps, "subagentes": cfg.subagentes,
        "model": status["model"], "model_path": status["model_path"], "models": models, "ctx_budget": status["ctx_budget"],
        "providers": providers, "presets": presets, "cloud_errors": cloud_errors, "effort": cfg.effort.unwrap_or_else(|| "high".into()),
        // C2: la caja de arena
        "sandbox": cfg.sandbox, "sandbox_available": gotv_core::sandbox::available(), "net_allow": cfg.net_allow,
        "net_default": gotv_core::sandbox::DEFAULT_HOSTS,
        "net_learned": gotv_core::permissions::Permissions::new(gotv_core::permissions::Mode::Default, &root, vec![], vec![], vec![]).net_hosts(),
    }))
}

/// F5: el menú de modelo del encabezado: los locales (rápido, sin preguntar a la nube) y cuál está
/// cargado.
#[tauri::command]
async fn models_list(h: State<'_, Hub>) -> Result<serde_json::Value, String> {
    let models = modelos_locales(&h).await?;
    let status = h.status.lock().await.clone();
    Ok(json!({"models": models, "model": status["model"], "model_path": status["model_path"]}))
}

/// Las ventanas de contexto que admite el modelo cargado, para el menú del medidor de arriba (pedido
/// del Portador 2026-11-13: la ventana es POR PROYECTO, cada uno puede usar otra IA). Sólo el modelo
/// cargado: local, su tope sale de la MISMA función que decide al arrancar (como Configuración →
/// Contexto); nube, de su proveedor (un pedido, tope 8 s). `preset` = el elegido para ESTE proyecto.
#[tauri::command]
async fn contexto_opciones(h: State<'_, Hub>) -> Result<serde_json::Value, String> {
    let root = h.root.lock().await.clone();
    let cfg = gotv_core::Config::load(&root);
    let status = h.status.lock().await.clone();
    let path = h.model_path.lock().await.clone();
    let preset = store::Store::load().project_context(&root).unwrap_or_else(|| cfg.context_preset.clone());
    let base = json!({"preset": preset, "ctx_budget": status["ctx_budget"], "model": status["model"]});
    let mut r = base;
    if let Some((p, id)) = gotv_core::nube::split(&path, &cfg) {
        let top = match tokio::time::timeout(std::time::Duration::from_secs(8), gotv_core::nube::models_of(&p)).await {
            Ok(Ok(ms)) => ms.into_iter().find(|m| m.id == id).and_then(|m| m.ctx).unwrap_or(0),
            _ => 0,
        };
        r["cloud"] = json!(true);
        r["ctx_top"] = json!(top);
        r["model_max"] = json!(top);
        r["ctx_auto"] = json!(gotv_core::nube::CLOUD_DEFAULT_CTX);
        return Ok(r);
    }
    let ms = modelos_locales(&h).await?;
    let cur = ms.iter().find(|m| m["path"].as_str() == Some(path.as_str())).or_else(|| ms.iter().find(|m| m["name"] == status["model"]));
    r["cloud"] = json!(false);
    r["ctx_top"] = cur.map(|m| m["ctx_top"].clone()).unwrap_or(json!(0));
    r["model_max"] = cur.map(|m| m["model_max"].clone()).unwrap_or(json!(0));
    Ok(r)
}

/// F5: los modelos LOCALES que se pueden correr (con si caben en la memoria de ahora). Rápido: sin
/// preguntar a la nube. Lo usan Configuración y el menú de modelo del encabezado.
async fn modelos_locales(h: &Hub) -> Result<Vec<serde_json::Value>, String> {
    let root = h.root.lock().await.clone();
    let cfg = gotv_core::Config::load(&root);
    let status = h.status.lock().await.clone();
    // sin tocar carpetas protegidas a las que el usuario no dio acceso (no pedir permiso
    // por abrir Configuración)
    let dirs = acceso::model_dirs_allowed(&cfg.model_dirs, &acceso::granted_roots(&store::Store::load().trusted));
    // la memoria cuenta lo que ocupa el modelo cargado: al cambiar, se libera
    let mut loaded = cfg.clone();
    loaded.model = h.model_path.lock().await.clone();
    loaded.context_budget = status["ctx_budget"].as_u64().unwrap_or(0) as usize;
    tokio::task::spawn_blocking(move || {
        let hw = gotv_core::Hardware::detect();
        let avail = worker::avail_with_loaded(&loaded);
        gotv_core::discover(&dirs)
            .into_iter()
            .filter(|c| c.runnable)
            .map(|c| {
                let f = gotv_core::fit(&c, &hw, avail, 8192u64.min(c.max_ctx));
                let mem_max = gotv_core::fit(&c, &hw, avail, c.max_ctx).max_ctx;
                // EL tope: lo que GOTV aplicaría con «Máximo» — la MISMA función que decide al
                // arrancar y al guardar, así la pantalla Modelo y la de Contexto dicen igual
                let top = gotv_core::launch::resolve_context(&c, &hw, avail, None).map(|p| p.ctx).unwrap_or(0);
                json!({"ctx_top": top, "name": c.name, "source": c.source, "path": c.path.display().to_string(), "kind": c.kind(), "gb": (c.weights as f64 / 1e9 * 10.0).round() / 10.0,
                       "fits": f.ok || f.max_ctx >= 2048.min(c.max_ctx), "why": f.why, "max_ctx": mem_max, "model_max": c.max_ctx,
                       "vision": c.vision, "audio": c.audio, "chat_only": c.chat_only})
            })
            .collect::<Vec<_>>()
    })
    .await
    .map_err(|e| e.to_string())
}

/// Espera varias tareas a la vez (sin traer otra dependencia).
async fn futures_join<F: std::future::Future + Send + 'static>(fs: impl Iterator<Item = F>) -> Vec<F::Output>
where
    F::Output: Send + 'static,
{
    let hs: Vec<_> = fs.map(tokio::spawn).collect();
    let mut out = vec![];
    for h in hs {
        if let Ok(v) = h.await {
            out.push(v);
        }
    }
    out
}

// ── A3 · proveedores de IA por API ──────────────────────────────────────────

fn provider_name_ok(n: &str) -> bool {
    !n.is_empty() && n.len() <= 32 && n.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// Guarda la llave en el Llavero de macOS. Nunca vuelve a la ventana.
#[tauri::command]
fn provider_key(name: String, key: String) -> Result<(), String> {
    if !provider_name_ok(&name) {
        return Err(gotv_core::i18n::t("nombre de proveedor inválido", "invalid provider name").into());
    }
    gotv_core::llaves::set(&name, &key).map_err(|e| e.to_string())
}

#[tauri::command]
fn provider_forget_key(name: String) {
    gotv_core::llaves::delete(&name);
}

fn save_providers(list: &[gotv_core::config::Provider]) -> Result<(), String> {
    let v = toml::Value::try_from(list).map_err(|e| e.to_string())?;
    worker::save_config(&[("providers", v)]);
    Ok(())
}

#[tauri::command]
fn provider_add(name: String, kind: String, base_url: String, vision: bool, h: State<'_, Hub>) -> Result<(), String> {
    let name = name.trim().to_lowercase();
    if !provider_name_ok(&name) {
        return Err(gotv_core::i18n::t("nombre: minúsculas, números y guiones (máx. 32)", "name: lowercase, digits and dashes (max 32)").into());
    }
    let base_url = base_url.trim().trim_end_matches('/').to_string();
    if kind == "openai" && !(base_url.starts_with("https://") || base_url.starts_with("http://")) {
        return Err(gotv_core::i18n::t("la URL debe empezar con https:// (o http:// en tu red)", "the URL must start with https:// (or http:// on your network)").into());
    }
    let root = h.root.try_lock().map(|r| r.clone()).unwrap_or_default();
    let mut list = gotv_core::Config::load(&root).providers;
    list.retain(|p| p.name != name);
    list.push(gotv_core::config::Provider { name, kind, base_url, vision });
    save_providers(&list)
}

#[tauri::command]
fn provider_remove(name: String, h: State<'_, Hub>) -> Result<(), String> {
    let root = h.root.try_lock().map(|r| r.clone()).unwrap_or_default();
    let mut list = gotv_core::Config::load(&root).providers;
    list.retain(|p| p.name != name);
    gotv_core::llaves::delete(&name);
    save_providers(&list)
}

/// Prueba la llave y la conexión: cuántos modelos ofrece.
#[tauri::command]
async fn provider_test(name: String, h: State<'_, Hub>) -> Result<usize, String> {
    let root = h.root.lock().await.clone();
    let cfg = gotv_core::Config::load(&root);
    let p = gotv_core::nube::providers(&cfg).into_iter().find(|p| p.name == name).ok_or("?")?;
    match tokio::time::timeout(std::time::Duration::from_secs(12), gotv_core::nube::models_of(&p)).await {
        Ok(r) => r.map(|m| m.len()).map_err(|e| e.to_string()),
        Err(_) => Err(gotv_core::i18n::t("no respondió en 12 s", "no answer within 12 s").into()),
    }
}

#[tauri::command]
fn apply_settings(s: serde_json::Value, h: State<'_, Hub>) -> Result<(), String> {
    // los cambios de sistema SÓLO se encienden por `set_system_changes` (que autentica
    // como administrador); por aquí jamás
    let mut s = s;
    if let Some(o) = s.as_object_mut() {
        o.remove("system_changes");
    }
    if let Some(t) = s["theme"].as_str() {
        let mut st = store::Store::load();
        st.theme = t.to_string();
        st.save();
    }
    h.tx.send(Msg::Apply(s)).map_err(|e| e.to_string())
}

/// Elegir modelo: apaga el actual, espera la RAM y carga el nuevo (vuelve si falla).
#[tauri::command]
fn set_model(path: String, h: State<'_, Hub>) -> Result<(), String> {
    h.tx.send(Msg::Model(path)).map_err(|e| e.to_string())
}

/// Seguridad: ACTIVAR que GOTV modifique apps/sistema pide autenticación de administrador
/// (Touch ID/contraseña, por macOS). Si el usuario autoriza, se guarda y aplica; si cancela,
/// se queda apagado. Apagar no pide nada.
#[tauri::command]
async fn set_system_changes(on: bool, h: State<'_, Hub>) -> Result<(), String> {
    if on {
        // prueba de admin: corre un comando inofensivo COMO ADMINISTRADOR. macOS muestra
        // Touch ID/contraseña; si cancela o falla, no se activa nada.
        let tmp = std::env::temp_dir();
        gotv_core::tools::run_elevated("/usr/bin/true", &tmp, 120).await.map_err(|e| e.to_string())?;
    }
    h.tx.send(Msg::Apply(json!({"system_changes": on}))).map_err(|e| e.to_string())
}

/// Control nativo del Mac: estado de los permisos de macOS que necesita (Accesibilidad,
/// Grabación de pantalla), leídos por agent-desktop en un hijo de GOTV.
#[tauri::command]
async fn desktop_status() -> Result<serde_json::Value, String> {
    let out = gotv_core::tools::run_desktop(&["status".into()], None, &std::env::temp_dir(), 20).await.map_err(|e| e.to_string())?;
    let v: serde_json::Value = serde_json::from_str(&out).map_err(|e| e.to_string())?;
    Ok(v["data"]["permissions"].clone())
}

/// Abre el asistente de permisos de agent-desktop (o el panel de macOS indicado).
#[tauri::command]
async fn desktop_permissions(pane: String) -> Result<(), String> {
    if pane == "guide" {
        gotv_core::tools::run_desktop(&["permissions".into(), "--guide".into()], None, &std::env::temp_dir(), 30).await.map_err(|e| e.to_string())?;
        return Ok(());
    }
    let anchor = if pane == "screen" { "Privacy_ScreenCapture" } else { "Privacy_Accessibility" };
    let _ = std::process::Command::new("open").arg(format!("x-apple.systempreferences:com.apple.preference.security?{anchor}")).spawn();
    Ok(())
}

/// Abre Ajustes del Sistema → Privacidad y seguridad → Gestión de apps, para conceder el
/// permiso de macOS de modificar otras apps.
#[tauri::command]
fn open_app_management() {
    let _ = std::process::Command::new("open")
        .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_AppBundles")
        .spawn();
}

/// Despega el panel de tareas a su propia ventana.
#[tauri::command]
fn tasks_window(app: AppHandle) -> Result<(), String> {
    if let Some(w) = app.get_webview_window("tareas") {
        let _ = w.set_focus();
        return Ok(());
    }
    tauri::WebviewWindowBuilder::new(&app, "tareas", tauri::WebviewUrl::App("tasks.html".into()))
        .title(gotv_core::i18n::t("Tareas — GOD OF THE VOID", "Tasks — GOD OF THE VOID"))
        .inner_size(420.0, 560.0)
        .min_inner_size(300.0, 240.0)
        .build()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

fn base64_decode(s: &str) -> Option<Vec<u8>> {
    let s = s.split(',').next_back()?; // admite «data:…;base64,»
    let val = |c: u8| -> Option<u32> {
        Some(match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        } as u32)
    };
    let clean: Vec<u8> = s.bytes().filter(|c| !c.is_ascii_whitespace() && *c != b'=').collect();
    let mut out = Vec::with_capacity(clean.len() * 3 / 4);
    for chunk in clean.chunks(4) {
        let mut n = 0u32;
        for (i, c) in chunk.iter().enumerate() {
            n |= val(*c)? << (18 - 6 * i);
        }
        out.push((n >> 16) as u8);
        if chunk.len() > 2 {
            out.push((n >> 8) as u8);
        }
        if chunk.len() > 3 {
            out.push(n as u8);
        }
    }
    Some(out)
}

/// Proyecto inicial: SÓLO uno que el usuario eligió — el que se pasó explícito
/// (argumento o `GOTV_ROOT`) o el último de su lista. Nunca la carpeta desde la que se
/// lanzó la app (medido 2026-11-09: era `~`, se re-añadía a la lista en cada arranque
/// —«borro el proyecto zero y vuelve»— y además se CONFIABA sin preguntar). Sin
/// ninguno, la app arranca sin proyecto y pide crear uno.
fn project_root() -> Option<PathBuf> {
    let explicit = std::env::args()
        .nth(1)
        .filter(|a| !a.starts_with('-'))
        .or_else(|| std::env::var("GOTV_ROOT").ok())
        .map(PathBuf::from)
        .and_then(|p| p.canonicalize().ok())
        .filter(|p| p.is_dir());
    if let Some(p) = explicit {
        let mut st = store::Store::load();
        st.trust(&p); // lo nombró el usuario al lanzar
        st.touch(&p);
        st.save();
        return Some(p);
    }
    // Pedido del Portador (2026-11-15): la app abre LIGERA, sin proyecto elegido ni nada cargando; el
    // modelo y la memoria del chat se cargan al elegir uno (`worker::en_reposo`)
    None
}

/// Menú nativo en el idioma de la app (el de fábrica de Tauri sale en inglés). Edición
/// se conserva entera: sin ella, ⌘C / ⌘V no llegan al WebView.
fn native_menu<R: tauri::Runtime>(app: &AppHandle<R>) -> tauri::Result<tauri::menu::Menu<R>> {
    use gotv_core::i18n;
    use tauri::menu::{Menu, PredefinedMenuItem as P, Submenu};
    let name = "GOD OF THE VOID";
    let app_menu = Submenu::with_items(
        app,
        name,
        true,
        &[
            &P::about(app, Some(&format!("{} {name}", i18n::t("Acerca de", "About"))), None)?,
            &P::separator(app)?,
            &P::services(app, Some(i18n::t("Servicios", "Services")))?,
            &P::separator(app)?,
            &P::hide(app, Some(&format!("{} {name}", i18n::t("Ocultar", "Hide"))))?,
            &P::hide_others(app, Some(i18n::t("Ocultar otros", "Hide Others")))?,
            &P::show_all(app, Some(i18n::t("Mostrar todo", "Show All")))?,
            &P::separator(app)?,
            &P::quit(app, Some(&format!("{} {name}", i18n::t("Salir de", "Quit"))))?,
        ],
    )?;
    let edit = Submenu::with_items(
        app,
        i18n::t("Edición", "Edit"),
        true,
        &[
            &P::undo(app, Some(i18n::t("Deshacer", "Undo")))?,
            &P::redo(app, Some(i18n::t("Rehacer", "Redo")))?,
            &P::separator(app)?,
            &P::cut(app, Some(i18n::t("Cortar", "Cut")))?,
            &P::copy(app, Some(i18n::t("Copiar", "Copy")))?,
            &P::paste(app, Some(i18n::t("Pegar", "Paste")))?,
            &P::select_all(app, Some(i18n::t("Seleccionar todo", "Select All")))?,
        ],
    )?;
    let window = Submenu::with_items(
        app,
        i18n::t("Ventana", "Window"),
        true,
        &[
            &P::minimize(app, Some(i18n::t("Minimizar", "Minimize")))?,
            &P::maximize(app, Some(i18n::t("Zoom", "Zoom")))?,
            &P::separator(app)?,
            &P::fullscreen(app, Some(i18n::t("Pantalla completa", "Enter Full Screen")))?,
            &P::separator(app)?,
            &P::close_window(app, Some(i18n::t("Cerrar ventana", "Close Window")))?,
        ],
    )?;
    Menu::with_items(app, &[&app_menu, &edit, &window])
}

fn main() {
    // M1: Whisper nativo (sin Python) para los modelos que no oyen
    gotv_core::audio::registrar_transcriptor(gotv_mlx::transcribir_wav);
    // M11: el embebedor del RAG en el núcleo (sin el servidor de mlx_vlm)
    gotv_core::rag::registrar_incrustador(gotv_mlx::incrustar_textos);
    // M4: el motor de imagen (FLUX.2 klein) para la herramienta `generar_imagen`
    gotv_core::generadores::registrar_motor_imagen(gotv_mlx::imagen::generar_png);
    // M7: el motor de música (ACE-Step 1.5) para la herramienta `generar_musica`
    gotv_core::generadores::registrar_motor_musica(gotv_mlx::musica::generar_wav);
    gotv_core::generadores::registrar_motor_efecto(gotv_mlx::efectos::generar_wav);
    gotv_core::generadores::registrar_motor_separar(gotv_mlx::separador::separar_archivo);
    // M6: el motor de video (Wan 2.2 TI2V 5B) para la herramienta `generar_video`
    gotv_core::generadores::registrar_motor_video(gotv_mlx::video::generar_mp4);
    // el vigilante del motor es este mismo binario relanzado: debe ir PRIMERO
    // control nativo del Mac: hijo de un comando o auxiliar de agent-desktop
    gotv_desktop::run_if_requested();
    // 2026-11-11 (captura del Portador: «trabajando…» eterno, el stop no responde, sin CPU ni red):
    // una app abierta desde el Finder no tiene stderr, así que un pánico no dejaba rastro. Ahora
    // queda escrito con su lugar exacto en ~/.cache/gotv/gui-fallos.log.
    worker::registrar_panicos();
    let root = project_root();
    let (tx, rx) = mpsc::unbounded_channel::<Msg>();
    // el idioma se fija ANTES de armar el menú nativo (si no, sale en inglés)
    let cfg_lang = gotv_core::Config::load(&root.clone().unwrap_or_else(|| PathBuf::from("/"))).lang;
    gotv_core::i18n::set(gotv_core::i18n::resolve(&cfg_lang));
    tauri::Builder::default()
        .menu(native_menu)
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            send, answer, answer_choice, chrome_status, chrome_install, reveal_ext, stop_turn, info, pick_folder, pick_files, guardar_como, dock_icono, leer_video, is_trusted, open_project, new_project, forget_project, close_project,
            provider_key, provider_forget_key, provider_add, provider_remove, provider_test,
            buscadores_lista, buscadores_llave, buscadores_activar, buscadores_probar, buscadores_tope, buscadores_orden, buscadores_ficha, buscadores_revisar, buscadores_uso, indice_semillas, indice_estado, busquedas_borrar,
            slash_items, open_link, open_vista, vista_variantes, vista_usar, conexiones_lista, conexiones_agregar, conexiones_quitar, conexiones_llave, conexiones_escanear, conexiones_fijar_y_probar, bases_lista, bases_agregar, bases_probar, bases_quitar, bases_secreto, bases_marcar, mcp_lista, mcp_probar, mcp_agregar, mcp_quitar, mcp_activar, mcp_entrar, mcp_salir, mcp_importar, proc_stop, api_status, api_apply, api_new_key, remoto::remoto_codigo, remoto::remoto_tokens, remoto::remoto_revocar, library_search, library_pull, library_cancel, generators_list, generator_read, generator_accept, generator_pull, experto_grabar, experto_grabar_parar, experto_mostrar, experto_cancelar, experto_generar, experto_extender, experto_versionar, experto_rehacer, experto_letra, experto_masterizar, experto_analizar, experto_curva_eq, experto_ir_a, experto_mezcla_audio, experto_sesion, experto_activar, experto_aplicar, experto_deshacer, experto_importar, experto_separar, experto_pista_audio, experto_chat, experto_exportar, experto_tono, hf_token, attach_path, attach_bytes, detach, voice_start, voice_stop, voz_leer, voz_callar, manos_libres, chat_estado, guardar_chat, guardar_pro, pro_opciones, musica_listos, generador_falta, generador_de_aviso, pro_video_estimado, cache_conversacion, settings, contexto_opciones, models_list, apply_settings, set_model, tasks_window,
            set_system_changes, open_app_management, desktop_status, desktop_permissions,
            // F18 · M9 · Configuración → Dispositivos
            dispositivos::devices_state, dispositivos::devices_accept, dispositivos::devices_install, dispositivos::devices_uninstall, dispositivos::devices_xcode_store, dispositivos::devices_xcode_license, dispositivos::devices_xcode_prepare, dispositivos::devices_xcode_platform, dispositivos::devices_idb_install,
            // F7 · la tarjeta del plan
            plan_comentar, plan_comentarios, plan_resolver,
            // F16 · paridad con la app de escritorio (escritorio.rs)
            escritorio::mensaje_encolar, escritorio::mensaje_quitar, escritorio::agentes_lista, escritorio::entorno_lista, escritorio::entorno_guardar, escritorio::entorno_caja, escritorio::entorno_quitar, escritorio::archivo_leer, escritorio::archivo_guardar, escritorio::terminal_abrir, escritorio::terminal_escribir, escritorio::terminal_tamano, escritorio::terminal_vista, escritorio::terminal_cerrar, escritorio::remotos_lista, escritorio::remoto_crear,
            // F8 · la ventana (estructura, chats, cambios, menciones, ⌘K, charlas, vista, Configuración)
            ventana::ventana_estructura, ventana::chats_lista, ventana::chat_abrir, ventana::chat_renombrar, ventana::chat_bifurcar, ventana::chat_exportar, ventana::chat_papelera, ventana::chat_restaurar, ventana::papelera_lista, ventana::papelera_vaciar,
            ventana::cambios_turnos, ventana::cambios_turno, ventana::cambios_git, ventana::cambios_revertir, ventana::cambios_comentar, ventana::cambios_comentarios,
            ventana::carpetas_lista, ventana::carpetas_agregar, ventana::carpetas_quitar,
            ventana::menciones, ventana::citar, ventana::buscar_todo, ventana::suelta_nueva, ventana::sueltas_lista, ventana::suelta_abrir, ventana::vista_url, ventana::vista_pedir_revision,
            ventana::chats_abiertos, ventana::chats_abiertos_guardar, ventana::cambios_confirmar, ventana::ventana_pref, ventana::ventana_pref_poner, ventana::automatizaciones_lista, ventana::automatizacion_activar, ventana::automatizacion_quitar, ventana::sitios_texto, ventana::sitios_guardar, ventana::avisos_estado, ventana::avisos_guardar
        ])
        .setup(move |app| {
            let hub = Hub {
                app: app.handle().clone(),
                tx,
                asks: Arc::default(),
                choices: Arc::default(),
                cancel: Arc::new(AtomicBool::new(false)),
                root: Arc::new(Mutex::new(root.clone().unwrap_or_default())),
                status: Arc::new(Mutex::new(json!({"ready": false}))),
                caps: Arc::new(Mutex::new((false, false))),
                pending: Arc::default(),
                tasks: Arc::default(),
                turn: Arc::default(),
                recording: Arc::default(),
                voz: Arc::default(),
                charla: Arc::default(),
                voz_instruccion: Arc::default(),
                chat_mac: Arc::default(),
                chat_perfil: Arc::default(),
                chat_pro: Arc::default(),
                model_path: Arc::default(),
                pulls: Arc::default(),
                api: gotv_core::serve::Shared::new(None),
                api_run: Arc::default(),
                procs_owner: Arc::default(),
                next_id: Arc::new(AtomicU64::new(1)),
            };
            app.manage(hub.clone());
            // A4: si el usuario dejó el servicio encendido, vuelve a encenderse
            {
                let h = hub.clone();
                tauri::async_runtime::spawn(async move {
                    let cfg = gotv_core::Config::load(&std::env::temp_dir());
                    if cfg.serve_enabled {
                        if let Err(e) = servicio::start(&h, &cfg).await {
                            emit(&h.app, json!({"kind": "line", "text": gotv_core::tr!("⚠ servicio de API: {e}", "⚠ API service: {e}")}));
                        }
                    }
                });
            }
            // C1: el panel ve los procesos del chat en vivo (sólo se emite si cambió algo)
            {
                let h = hub.clone();
                tauri::async_runtime::spawn(async move {
                    let mut last = String::new();
                    loop {
                        tokio::time::sleep(std::time::Duration::from_millis(1000)).await;
                        let owner = h.procs_owner.lock().unwrap().clone();
                        let v = procs_view(&owner);
                        // los segundos cambian siempre: se comparan sin ellos
                        let key: String = v.as_array().map(|a| a.iter().map(|p| format!("{}{}{}{}{}", p["id"], p["alive"], p["exit"], p["urls"], p["screen"])).collect()).unwrap_or_default();
                        if key != last {
                            last = key;
                            emit(&h.app, json!({"kind": "procs", "procs": v}));
                        }
                    }
                });
            }
            // F16: la cola de mensajes a mitad de turno y los modelos de los agentes a medida
            escritorio::arrancar(hub.clone());
            tauri::async_runtime::spawn(worker::run(hub, rx, root));
            Ok(())
        })
        .build(tauri::generate_context!())
        .unwrap_or_else(|e| panic!("{}: {e}", gotv_core::i18n::t("no pude abrir la ventana de GOTV", "could not open the GOTV window")))
        .run(|_, ev| {
            // C1: al cerrar GOTV no queda vivo ningún proceso que lanzó el agente
            if let tauri::RunEvent::Exit = ev {
                gotv_core::exec::kill_all();
            }
        });
}

#[cfg(test)]
mod tests {
    /// El núcleo relanza GOTV con una bandera y el `main` la atiende con otra
    /// constante: si se separan, el control del Mac deja de funcionar EN SILENCIO.
    #[test]
    fn la_bandera_del_control_del_mac_coincide() {
        assert_eq!(gotv_desktop::FLAG, gotv_core::tools::DESKTOP_FLAG);
    }

    /// Ola PRO: el modo profesional suma el negativo de la persona al oficial copiado en el núcleo.
    #[test]
    fn negativo_de_video_igual_al_del_motor() {
        assert_eq!(gotv_core::generadores::NEGATIVO_VIDEO, gotv_mlx::video::NEGATIVO);
    }

    #[test]
    fn base64_ida_y_vuelta() {
        let src = b"Vac\xc3\xado del GOTV!";
        let enc = gotv_core::images::base64(src);
        assert_eq!(super::base64_decode(&enc).unwrap(), src);
        assert_eq!(super::base64_decode(&format!("data:audio/wav;base64,{enc}")).unwrap(), src);
    }
}
