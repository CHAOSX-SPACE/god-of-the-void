//! A4 · GOD OF THE VOID como servicio (lo que hace Ollama, y más).
//!
//! Una API HTTP para que OTRA app —un cliente OpenAI, uno de Ollama, `curl`, otra
//! Mac de la red— use los modelos de GOTV:
//!
//! - **compatible con OpenAI**: `GET /v1/models`, `POST /v1/chat/completions`
//!   (con y sin streaming, herramientas, imágenes);
//! - **compatible con Ollama**: `GET /api/tags`, `POST /api/chat`,
//!   `POST /api/generate`, `POST /api/show`, `GET /api/version`, `GET /api/ps`;
//! - el modelo se elige por NOMBRE: el local cargado (también `default`/`gotv`) o
//!   cualquier modelo de nube de A3 (`anthropic:claude-…`, `openrouter:…`).
//!
//! El local va por PASARELA al motor MLX vivo: lo que el cliente mande (herramientas,
//! imágenes, `logprobs`…) llega intacto y el streaming sale byte a byte. Los de nube
//! OpenAI-compatibles, igual (con la llave del Llavero); Claude se traduce por el
//! `AnthropicEngine` de A3.
//!
//! Seguridad: por defecto sólo `127.0.0.1`. Abrirlo a la red EXIGE una llave propia
//! (`Authorization: Bearer …` o `x-api-key`), comparada en tiempo constante; sin ella,
//! 401. `check_bind` se niega a escuchar fuera del equipo sin llave.

use crate::llaves;
use anyhow::{anyhow, bail, Result};
use axum::body::Body;
use axum::extract::{DefaultBodyLimit, State};
use axum::http::{header, HeaderMap, Request, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use gotv_engine::{ChatRequest, ChatResponse, Engine, Message, Sampling, StreamEvent};
use serde_json::{json, Value};
use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::mpsc;

/// Puerto por defecto: el de Ollama + 1 (11434 suele estar ocupado por Ollama).
pub const DEFAULT_PORT: u16 = 11435;
/// Nombre de la llave del servicio en el Llavero.
pub const KEY_NAME: &str = "gotv-serve";

/// Cómo se habla con el modelo de atrás.
#[derive(Clone, Debug, PartialEq)]
pub enum Wire {
    /// API OpenAI (los proveedores compatibles y los servidores de la red): pasarela.
    OpenAi,
    /// Messages API de Claude: se traduce con `AnthropicEngine`.
    Anthropic,
    /// El núcleo de GOTV en ESTE proceso (el modelo que la app o `gotv serve` tienen cargado), por
    /// su asa compartida: sin puerto ni proceso aparte.
    Motor,
}

/// El asa del núcleo que sirve el modelo local (`Engine::compartir`).
#[derive(Clone)]
pub struct MotorLocal(pub Arc<dyn gotv_engine::Engine>);

impl std::fmt::Debug for MotorLocal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "MotorLocal({})", self.0.name())
    }
}

/// Un modelo que el servicio ofrece.
#[derive(Clone, Debug)]
pub struct Upstream {
    /// el nombre con que lo piden los clientes
    pub id: String,
    /// `…/v1` del servidor real
    pub base_url: String,
    pub key: Option<String>,
    /// el nombre que espera el servidor real
    pub remote: String,
    pub wire: Wire,
    pub owned_by: String,
    pub vision: bool,
    /// bytes de los pesos (para `/api/tags`; 0 en la nube)
    pub size: u64,
    /// `Wire::Motor`: el núcleo que atiende
    pub motor: Option<MotorLocal>,
}

/// Estado compartido del servicio: lo actualiza la app al cambiar de modelo.
#[derive(Clone)]
pub struct Shared {
    pub local: Arc<RwLock<Option<Upstream>>>,
    pub cloud: Arc<RwLock<Vec<Upstream>>>,
    key: Arc<RwLock<Option<String>>>,
    http: reqwest::Client,
    /// peticiones atendidas (se ve en la app)
    pub hits: Arc<AtomicU64>,
    /// F10.5: las sesiones que se ven desde el teléfono y los teléfonos emparejados
    pub remoto: crate::remoto::Registro,
}

impl Shared {
    pub fn new(key: Option<String>) -> Self {
        Self {
            local: Arc::default(),
            cloud: Arc::default(),
            key: Arc::new(RwLock::new(key)),
            http: reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(20))
                .build()
                .unwrap_or_default(),
            hits: Arc::default(),
            remoto: crate::remoto::Registro::default(),
        }
    }

    /// F10.5: ¿trae la llave del servicio? (sin llave configurada, NO: el control remoto exige una u otro token)
    pub fn llave_valida(&self, h: &HeaderMap) -> bool {
        self.key.read().unwrap().is_some() && self.authorized(h)
    }

    pub fn set_local(&self, up: Option<Upstream>) {
        *self.local.write().unwrap() = up;
    }

    pub fn set_cloud(&self, ups: Vec<Upstream>) {
        *self.cloud.write().unwrap() = ups;
    }

    pub fn set_key(&self, key: Option<String>) {
        *self.key.write().unwrap() = key;
    }

    pub fn models(&self) -> Vec<Upstream> {
        let mut v: Vec<Upstream> = self.local.read().unwrap().iter().cloned().collect();
        v.extend(self.cloud.read().unwrap().iter().cloned());
        v
    }

    /// El modelo pedido: el local por su nombre, su carpeta o un alias
    /// (`default`, `gotv`, `local`, vacío); la nube por su `proveedor:modelo`.
    pub fn pick(&self, model: &str) -> Option<Upstream> {
        let m = model.trim();
        let local = self.local.read().unwrap().clone();
        if m.is_empty()
            || ["default", "gotv", "local", "gotv:latest", "default:latest"]
                .contains(&m.to_lowercase().as_str())
        {
            return local;
        }
        let bare = m.strip_suffix(":latest").unwrap_or(m);
        let all = self.models();
        all.iter()
            .find(|u| u.id == bare)
            .or_else(|| all.iter().find(|u| u.id.eq_ignore_ascii_case(bare)))
            // el local también por el final de su nombre (`Qwen3-32B-4bit-DWQ` sin la org)
            .or_else(|| {
                local.as_ref().filter(|u| {
                    u.id.rsplit('/')
                        .next()
                        .is_some_and(|n| n.eq_ignore_ascii_case(bare))
                })
            })
            .cloned()
    }

    fn authorized(&self, h: &HeaderMap) -> bool {
        let Some(key) = self.key.read().unwrap().clone() else {
            return true;
        };
        let given = h
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| {
                v.strip_prefix("Bearer ")
                    .or_else(|| v.strip_prefix("bearer "))
            })
            .or_else(|| h.get("x-api-key").and_then(|v| v.to_str().ok()))
            .unwrap_or("");
        ct_eq(given.trim().as_bytes(), key.as_bytes())
    }
}

/// Comparación en tiempo constante (la llave no se adivina midiendo cuánto tarda).
fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    let mut diff = (a.len() ^ b.len()) as u8 | u8::from(a.len() != b.len());
    for (i, x) in b.iter().enumerate() {
        diff |= x ^ a.get(i).copied().unwrap_or(0);
    }
    diff == 0
}

/// ¿Es una dirección sólo de este equipo?
pub fn is_loopback(host: &str) -> bool {
    host == "localhost"
        || host
            .parse::<IpAddr>()
            .map(|ip| ip.is_loopback())
            .unwrap_or(false)
}

/// Escuchar fuera del equipo sin llave = cualquiera de la red usa tus modelos (y tus
/// créditos de nube). Se niega.
pub fn check_bind(host: &str, key: Option<&str>) -> Result<()> {
    if is_loopback(host) {
        return Ok(());
    }
    match key {
        Some(k) if k.len() >= 16 => Ok(()),
        _ => bail!(
            "{}",
            crate::tr!(
            "abrir el servicio a la red ({host}) exige una llave de al menos 16 caracteres",
            "opening the service to the network ({host}) requires a key of at least 16 characters"
        )
        ),
    }
}

/// Una llave nueva y aleatoria (`gotv-` + 48 hex), de `/dev/urandom`.
pub fn new_key() -> Result<String> {
    use std::io::Read;
    let mut b = [0u8; 24];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut b)?;
    Ok(format!(
        "gotv-{}",
        b.iter().map(|x| format!("{x:02x}")).collect::<String>()
    ))
}

/// La llave del servicio en el Llavero (la crea si no existe).
/// `GOTV_SERVE_KEY` en el entorno manda primero: no toca el Llavero (un binario recién
/// compilado, con otra firma, haría que macOS pidiera permiso y esperara a alguien).
pub fn stored_key(create: bool) -> Result<Option<String>> {
    if let Ok(k) = std::env::var("GOTV_SERVE_KEY") {
        if k.trim().len() >= 16 {
            return Ok(Some(k.trim().to_string()));
        }
    }
    if let Some(k) = llaves::get(KEY_NAME) {
        return Ok(Some(k));
    }
    if !create {
        return Ok(None);
    }
    let k = new_key()?;
    llaves::set(KEY_NAME, &k)?;
    Ok(Some(k))
}

/// Las IPv4 de esta Mac en la red local (para decir a qué dirección conectarse).
pub fn lan_addresses() -> Vec<String> {
    let mut out = Vec::new();
    for ifc in ["en0", "en1", "en2", "bridge0"] {
        if let Ok(o) = std::process::Command::new("ipconfig")
            .args(["getifaddr", ifc])
            .output()
        {
            let ip = String::from_utf8_lossy(&o.stdout).trim().to_string();
            if !ip.is_empty() && !out.contains(&ip) {
                out.push(ip);
            }
        }
    }
    out
}

/// El modelo local cargado, servido por el NÚCLEO en este proceso (su asa compartida).
pub fn local_upstream(name: &str, model_path: &str, motor: Arc<dyn gotv_engine::Engine>, vision: bool, size: u64) -> Upstream {
    Upstream {
        id: name.to_string(),
        base_url: String::new(),
        key: None,
        remote: model_path.to_string(),
        wire: Wire::Motor,
        owned_by: "gotv".into(),
        vision,
        size,
        motor: Some(MotorLocal(motor)),
    }
}

/// El nombre PÚBLICO de un modelo local (el mismo en la app y en `gotv serve`):
/// `org/nombre` en LM Studio, la caché de GOTV y la de Hugging Face; si no, su carpeta.
pub fn public_name(path: &str) -> String {
    let p = std::path::Path::new(path);
    let hf = crate::models::display_name(p);
    let dir = p
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    if hf != dir {
        return hf; // caché de Hugging Face: models--org--nombre
    }
    for root in ["/.lmstudio/models/", "/.cache/gotv/models/"] {
        if let Some(rest) = path.split(root).nth(1) {
            let parts: Vec<&str> = rest.trim_end_matches('/').split('/').collect();
            if parts.len() == 2 {
                return parts.join("/");
            }
        }
    }
    dir
}

/// Los modelos de nube que se pueden servir: los de cada proveedor con llave en el
/// Llavero (o un servidor de la red sin llave). Un proveedor que falla se salta.
pub async fn cloud_upstreams(cfg: &crate::Config) -> Vec<Upstream> {
    let mut out = Vec::new();
    for p in crate::nube::providers(cfg) {
        let key = llaves::get(&p.name);
        if key.is_none() && !(p.kind == "openai" && p.base_url.starts_with("http://")) {
            continue;
        }
        let Ok(models) = crate::nube::models_of(&p).await else {
            continue;
        };
        let anthropic = p.kind == "anthropic";
        for m in models {
            out.push(Upstream {
                id: format!("{}:{}", p.name, m.id),
                base_url: p.base_url.clone(),
                key: key.clone(),
                remote: m.id,
                wire: if anthropic {
                    Wire::Anthropic
                } else {
                    Wire::OpenAi
                },
                owned_by: p.name.clone(),
                vision: p.vision,
                size: 0,
                motor: None,
            });
        }
    }
    out
}

// ── El servidor ──────────────────────────────────────────────────────────────

pub fn router(s: Shared) -> Router {
    Router::new()
        .route("/", get(|| async { "GOD OF THE VOID is running" }))
        .route("/v1/models", get(v1_models))
        .route("/v1/chat/completions", post(v1_chat))
        .route(
            "/api/version",
            get(|| async { Json(json!({"version": "0.9.0", "server": "GOD OF THE VOID"})) }),
        )
        .route("/api/tags", get(api_tags))
        .route("/api/ps", get(api_ps))
        .route("/api/show", post(api_show))
        .route("/api/chat", post(api_chat))
        .route("/api/generate", post(api_generate))
        // F10.5: el control remoto (con su propia guardia: red local + llave o token del teléfono)
        .layer(middleware::from_fn_with_state(s.clone(), guard))
        // imágenes en base64 pesan: 64 MB por petición
        .layer(DefaultBodyLimit::max(64 << 20))
        .with_state(s.clone())
        // F10.5: el control remoto, fuera de la guardia de la API de modelos y con la suya (red local + la
        // llave del servicio o el token de un teléfono emparejado)
        .merge({
            let k = s.clone();
            crate::remoto::rutas(s.remoto.clone(), Arc::new(move |h: &HeaderMap| k.llave_valida(h)))
        })
}

/// La llave, antes que nada (también para listar modelos).
async fn guard(State(s): State<Shared>, req: Request<Body>, next: Next) -> Response {
    if req.uri().path() != "/" && !s.authorized(req.headers()) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error": {"message": "missing or invalid API key (Authorization: Bearer …)", "type": "invalid_request_error", "code": "invalid_api_key"}})),
        )
            .into_response();
    }
    s.hits.fetch_add(1, Ordering::Relaxed);
    next.run(req).await
}

/// Escucha en `host:port`. Puerto 0 = uno libre (pruebas).
pub async fn bind(host: &str, port: u16) -> Result<tokio::net::TcpListener> {
    let ip: IpAddr = if host == "localhost" {
        IpAddr::from([127, 0, 0, 1])
    } else {
        host.parse().map_err(|_| {
            anyhow!(
                "{}",
                crate::tr!("host inválido: {host}", "invalid host: {host}")
            )
        })?
    };
    tokio::net::TcpListener::bind(SocketAddr::new(ip, port))
        .await
        .map_err(|e| {
            anyhow!(
                "{}",
                crate::tr!(
                    "no pude escuchar en {host}:{port}: {e}",
                    "could not listen on {host}:{port}: {e}"
                )
            )
        })
}

/// Sirve hasta que `stop` se dispare (o se suelte).
pub async fn run(
    listener: tokio::net::TcpListener,
    s: Shared,
    stop: tokio::sync::oneshot::Receiver<()>,
) -> Result<()> {
    // F10.5: con la dirección del par (el control remoto sólo responde a la red local)
    axum::serve(listener, router(s).into_make_service_with_connect_info::<crate::remoto::Conexion>())
        .with_graceful_shutdown(async move {
            let _ = stop.await;
        })
        .await?;
    Ok(())
}

/// El cuerpo como JSON, venga o no con `Content-Type: application/json` (los clientes
/// de Ollama y `curl -d` no lo ponen; el Ollama real los acepta igual).
#[allow(clippy::result_large_err)] // el Err ES la respuesta HTTP: se devuelve tal cual
fn parse_json(b: &[u8]) -> Result<Value, Response> {
    serde_json::from_slice(b).map_err(|e| {
        err(
            StatusCode::BAD_REQUEST,
            format!("invalid JSON body: {e}"),
            "invalid_request_error",
        )
    })
}

fn err(code: StatusCode, msg: impl Into<String>, kind: &str) -> Response {
    (
        code,
        Json(json!({"error": {"message": msg.into(), "type": kind}})),
    )
        .into_response()
}

fn no_model(m: &str, s: &Shared) -> Response {
    let ids: Vec<String> = s.models().into_iter().map(|u| u.id).collect();
    let msg = if ids.is_empty() {
        "GOD OF THE VOID has no model loaded yet".to_string()
    } else {
        format!("model «{m}» not found; available: {}", ids.join(", "))
    };
    err(StatusCode::NOT_FOUND, msg, "model_not_found")
}

async fn v1_models(State(s): State<Shared>) -> Json<Value> {
    let data: Vec<Value> = s
        .models()
        .iter()
        .map(|u| json!({"id": u.id, "object": "model", "created": 0, "owned_by": u.owned_by}))
        .collect();
    Json(json!({"object": "list", "data": data}))
}

/// `POST /v1/chat/completions`: pasarela al motor real (o traducción para Claude).
async fn v1_chat(State(s): State<Shared>, raw: bytes::Bytes) -> Response {
    let mut body = match parse_json(&raw) {
        Ok(v) => v,
        Err(r) => return r,
    };
    let model = body["model"].as_str().unwrap_or("").to_string();
    let Some(up) = s.pick(&model) else {
        return no_model(&model, &s);
    };
    if up.wire != Wire::OpenAi {
        return v1_via_engine(&up, &body).await;
    }
    body["model"] = json!(up.remote);
    let mut req = s
        .http
        .post(format!(
            "{}/chat/completions",
            up.base_url.trim_end_matches('/')
        ))
        .json(&body);
    if let Some(k) = &up.key {
        req = req.bearer_auth(k);
    }
    let resp = match req.send().await {
        Ok(r) => r,
        Err(e) => {
            return err(
                StatusCode::BAD_GATEWAY,
                format!("upstream: {e}"),
                "upstream_error",
            )
        }
    };
    let status = StatusCode::from_u16(resp.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
    let ctype = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/json")
        .to_string();
    // el streaming SSE del motor llega al cliente LÍNEA a línea (sin esperar al final),
    // con una sola corrección: el `model` lleva la RUTA local del motor
    // (`/Users/<usuario>/…`) y se cambia por el nombre público
    let (from, to) = (
        serde_json::to_string(&up.remote).unwrap_or_default(),
        serde_json::to_string(&up.id).unwrap_or_default(),
    );
    let (tx, rx) = mpsc::channel::<Result<bytes::Bytes, std::io::Error>>(32);
    tokio::spawn(async move {
        let mut resp = resp;
        let mut pending: Vec<u8> = Vec::new();
        let fix = |b: &[u8]| -> bytes::Bytes {
            let t = String::from_utf8_lossy(b);
            bytes::Bytes::from(if from.len() > 2 && t.contains(&from) {
                t.replace(&from, &to)
            } else {
                t.into_owned()
            })
        };
        loop {
            match resp.chunk().await {
                Ok(Some(c)) => {
                    pending.extend_from_slice(&c);
                    let Some(cut) = pending.iter().rposition(|b| *b == b'\n') else {
                        continue;
                    };
                    let ready: Vec<u8> = pending.drain(..=cut).collect();
                    if tx.send(Ok(fix(&ready))).await.is_err() {
                        break; // el cliente colgó: se deja de leer (y el motor lo nota)
                    }
                }
                Ok(None) => {
                    if !pending.is_empty() {
                        let _ = tx.send(Ok(fix(&pending))).await;
                    }
                    break;
                }
                Err(e) => {
                    let _ = tx.send(Err(std::io::Error::other(e))).await;
                    break;
                }
            }
        }
    });
    let stream =
        futures_util::stream::unfold(rx, |mut rx| async move { rx.recv().await.map(|x| (x, rx)) });
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, ctype)
        .header(header::CACHE_CONTROL, "no-cache")
        .body(Body::from_stream(stream))
        .unwrap_or_else(|_| {
            err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "response",
                "server_error",
            )
        })
}

// ── Motor genérico: eventos de cualquier modelo (para Ollama y Claude) ───────

enum Ev {
    Text(String),
    Reasoning(String),
    Done(Box<ChatResponse>),
    Fail(String),
}

/// Mensajes OpenAI (contenido texto o por partes) → los del núcleo.
pub fn to_messages(v: &Value) -> Vec<Message> {
    v.as_array()
        .into_iter()
        .flatten()
        .map(|m| {
            let role = m["role"].as_str().unwrap_or("user").to_string();
            let (mut text, mut images) = (String::new(), Vec::new());
            match &m["content"] {
                Value::String(t) => text = t.clone(),
                Value::Array(parts) => {
                    for p in parts {
                        match p["type"].as_str() {
                            Some("text") => text.push_str(p["text"].as_str().unwrap_or("")),
                            Some("image_url") => {
                                let u = p["image_url"]["url"]
                                    .as_str()
                                    .or(p["image_url"].as_str())
                                    .unwrap_or("");
                                if !u.is_empty() {
                                    images.push(u.to_string());
                                }
                            }
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
            let mut out = Message {
                role,
                content: text,
                ..Message::user("")
            };
            out.images = std::mem::take(&mut images);
            out.tool_calls = m["tool_calls"].as_array().cloned();
            out.tool_call_id = m["tool_call_id"].as_str().map(String::from);
            out.name = m["name"].as_str().map(String::from);
            out
        })
        .collect()
}

fn sampling_of(body: &Value) -> Sampling {
    Sampling {
        temperature: body["temperature"].as_f64().unwrap_or(0.7) as f32,
        top_p: body["top_p"].as_f64().map(|x| x as f32),
        top_k: body["top_k"].as_u64().map(|x| x as u32),
        min_p: body["min_p"].as_f64().map(|x| x as f32),
    }
}

/// Corre la petición contra el modelo y devuelve sus eventos por un canal.
fn events(up: Upstream, http: reqwest::Client, body: Value) -> mpsc::UnboundedReceiver<Ev> {
    let (tx, rx) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        let send = tx.clone();
        let on = move |e: StreamEvent| {
            let _ = match e {
                StreamEvent::Text(t) => send.send(Ev::Text(t)),
                StreamEvent::Reasoning(t) => send.send(Ev::Reasoning(t)),
                StreamEvent::Prefill { .. } | StreamEvent::ToolArgs(_) => Ok(()),
            };
        };
        let pedido = || ChatRequest {
            messages: to_messages(&body["messages"]),
            max_tokens: body["max_tokens"].as_u64().or(body["max_completion_tokens"].as_u64()).unwrap_or(4096) as u32,
            sampling: sampling_of(&body),
            tools: body.get("tools").filter(|t| t.is_array()).cloned(),
            conversacion: None,
            herramientas_aparte: None,
        };
        let r = match up.wire {
            Wire::Anthropic => {
                let eng = gotv_engine::AnthropicEngine::new(&up.remote, up.key.clone().unwrap_or_default(), None);
                eng.complete_stream(pedido(), &on).await
            }
            // el núcleo en este proceso, por su asa (el modelo ya cargado)
            Wire::Motor => match &up.motor {
                Some(m) => m.0.complete_stream(pedido(), &on).await,
                None => Err(gotv_engine::anyhow_tr!("el modelo local no tiene motor", "the local model has no engine")),
            },
            Wire::OpenAi => openai_stream(&up, &http, body.clone(), &on).await,
        };
        let _ = match r {
            Ok(resp) => tx.send(Ev::Done(Box::new(resp))),
            Err(e) => tx.send(Ev::Fail(e.to_string())),
        };
    });
    rx
}

/// Streaming contra un servidor OpenAI (el motor local o un proveedor), leído con el
/// mismo intérprete SSE del núcleo (`apply_sse_line`): usage exacto incluido.
async fn openai_stream(
    up: &Upstream,
    http: &reqwest::Client,
    mut body: Value,
    on: &(dyn Fn(StreamEvent) + Send + Sync),
) -> Result<ChatResponse> {
    body["model"] = json!(up.remote);
    body["stream"] = json!(true);
    body["stream_options"] = json!({"include_usage": true});
    let mut req = http
        .post(format!(
            "{}/chat/completions",
            up.base_url.trim_end_matches('/')
        ))
        .json(&body);
    if let Some(k) = &up.key {
        req = req.bearer_auth(k);
    }
    let mut resp = req.send().await?;
    if !resp.status().is_success() {
        let st = resp.status();
        bail!(
            "upstream {st}: {}",
            resp.text()
                .await
                .unwrap_or_default()
                .chars()
                .take(400)
                .collect::<String>()
        );
    }
    let mut out = ChatResponse::default();
    let mut buf: Vec<u8> = Vec::new();
    while let Some(chunk) = resp.chunk().await? {
        buf.extend_from_slice(&chunk);
        while let Some(i) = buf.iter().position(|b| *b == b'\n') {
            let line: Vec<u8> = buf.drain(..=i).collect();
            gotv_engine::apply_sse_line(String::from_utf8_lossy(&line).trim_end(), &mut out, on);
        }
    }
    if !buf.is_empty() {
        gotv_engine::apply_sse_line(String::from_utf8_lossy(&buf).trim_end(), &mut out, on);
    }
    if out.finish_reason.is_empty() {
        out.finish_reason = "stop".into();
    }
    Ok(out)
}

fn openai_calls(r: &ChatResponse) -> Vec<Value> {
    r.tool_calls
        .iter()
        .map(|c| json!({"id": c.id, "type": "function", "function": {"name": c.name, "arguments": c.arguments.to_string()}}))
        .collect()
}

/// `/v1/chat/completions` para Claude: la respuesta en forma OpenAI.
async fn v1_via_engine(up: &Upstream, body: &Value) -> Response {
    let stream = body["stream"].as_bool().unwrap_or(false);
    let mut rx = events(up.clone(), reqwest::Client::new(), body.clone());
    let id = format!("chatcmpl-gotv{}", now_ms());
    let model = up.id.clone();
    if !stream {
        let (mut text, mut done) = (String::new(), None);
        while let Some(e) = rx.recv().await {
            match e {
                Ev::Text(t) => text.push_str(&t),
                Ev::Reasoning(_) => {}
                Ev::Done(r) => done = Some(r),
                Ev::Fail(m) => return err(StatusCode::BAD_GATEWAY, m, "upstream_error"),
            }
        }
        let r = done.unwrap_or_default();
        let calls = openai_calls(&r);
        let mut msg = json!({"role": "assistant", "content": if r.content.is_empty() { text } else { r.content.clone() }});
        if !calls.is_empty() {
            msg["tool_calls"] = json!(calls);
        }
        return Json(json!({"id": id, "object": "chat.completion", "created": now_ms() / 1000, "model": model,
            "choices": [{"index": 0, "message": msg, "finish_reason": fin_para_api(&r.finish_reason)}],
            "usage": {"prompt_tokens": r.prompt_tokens, "completion_tokens": r.tokens, "total_tokens": r.prompt_tokens + r.tokens}}))
        .into_response();
    }
    let (tx, out) = mpsc::channel::<Result<String, std::io::Error>>(64);
    tokio::spawn(async move {
        let chunk = |delta: Value, finish: Value| {
            format!(
                "data: {}\n\n",
                json!({"id": id, "object": "chat.completion.chunk", "created": now_ms() / 1000, "model": model, "choices": [{"index": 0, "delta": delta, "finish_reason": finish}]})
            )
        };
        let _ = tx
            .send(Ok(chunk(
                json!({"role": "assistant", "content": ""}),
                Value::Null,
            )))
            .await;
        while let Some(e) = rx.recv().await {
            let line = match e {
                Ev::Text(t) => chunk(json!({"content": t}), Value::Null),
                Ev::Reasoning(t) => chunk(json!({"reasoning": t}), Value::Null),
                Ev::Done(r) => {
                    let calls = openai_calls(&r);
                    let mut s = String::new();
                    if !calls.is_empty() {
                        let indexed: Vec<Value> = calls
                            .into_iter()
                            .enumerate()
                            .map(|(i, mut c)| {
                                c["index"] = json!(i);
                                c
                            })
                            .collect();
                        s.push_str(&chunk(json!({"tool_calls": indexed}), Value::Null));
                    }
                    s.push_str(&chunk(
                        json!({}),
                        json!(fin_para_api(&r.finish_reason)),
                    ));
                    s.push_str(&format!("data: {}\n\n", json!({"id": id, "object": "chat.completion.chunk", "model": model, "choices": [],
                        "usage": {"prompt_tokens": r.prompt_tokens, "completion_tokens": r.tokens, "total_tokens": r.prompt_tokens + r.tokens}})));
                    s.push_str("data: [DONE]\n\n");
                    s
                }
                Ev::Fail(m) => format!(
                    "data: {}\n\ndata: [DONE]\n\n",
                    json!({"error": {"message": m}})
                ),
            };
            if tx.send(Ok(line)).await.is_err() {
                break;
            }
        }
    });
    sse_body(out, "text/event-stream")
}

fn sse_body(rx: mpsc::Receiver<Result<String, std::io::Error>>, ctype: &str) -> Response {
    let stream =
        futures_util::stream::unfold(rx, |mut rx| async move { rx.recv().await.map(|x| (x, rx)) });
    Response::builder()
        .header(header::CONTENT_TYPE, ctype)
        .header(header::CACHE_CONTROL, "no-cache")
        .body(Body::from_stream(stream))
        .unwrap_or_else(|_| {
            err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "response",
                "server_error",
            )
        })
}

// ── Ollama ───────────────────────────────────────────────────────────────────

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// `2026-11-09T05:00:00.123Z` sin dependencias (días civiles de Howard Hinnant).
pub fn rfc3339(ms: u64) -> String {
    let secs = (ms / 1000) as i64;
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.{:03}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60,
        ms % 1000
    )
}

fn ollama_details(u: &Upstream) -> Value {
    let quant = ["2bit", "3bit", "4bit", "5bit", "6bit", "8bit", "bf16"]
        .iter()
        .find(|q| u.id.to_lowercase().contains(*q))
        .map(|q| q.to_uppercase())
        .unwrap_or_default();
    json!({"format": if u.wire == Wire::OpenAi && u.owned_by == "gotv" { "mlx" } else { "api" }, "family": u.owned_by, "families": [u.owned_by],
        "parameter_size": "", "quantization_level": quant})
}

async fn api_tags(State(s): State<Shared>) -> Json<Value> {
    let models: Vec<Value> = s
        .models()
        .iter()
        .map(|u| json!({"name": u.id, "model": u.id, "modified_at": rfc3339(now_ms()), "size": u.size, "digest": "", "details": ollama_details(u)}))
        .collect();
    Json(json!({"models": models}))
}

async fn api_ps(State(s): State<Shared>) -> Json<Value> {
    let models: Vec<Value> = s
        .local
        .read()
        .unwrap()
        .iter()
        .map(|u| json!({"name": u.id, "model": u.id, "size": u.size, "size_vram": u.size, "digest": "", "details": ollama_details(u), "expires_at": "0001-01-01T00:00:00Z"}))
        .collect();
    Json(json!({"models": models}))
}

async fn api_show(State(s): State<Shared>, raw: bytes::Bytes) -> Response {
    let body = match parse_json(&raw) {
        Ok(v) => v,
        Err(r) => return r,
    };
    let name = body["model"]
        .as_str()
        .or(body["name"].as_str())
        .unwrap_or("");
    let Some(u) = s.pick(name) else {
        return no_model(name, &s);
    };
    let mut caps = vec!["completion", "tools"];
    if u.vision {
        caps.push("vision");
    }
    Json(json!({"modelfile": "", "parameters": "", "template": "", "details": ollama_details(&u), "model_info": {}, "capabilities": caps})).into_response()
}

/// Mensajes de Ollama (`images` en base64 crudo) → forma OpenAI.
fn ollama_messages(v: &Value) -> Vec<Value> {
    v.as_array()
        .into_iter()
        .flatten()
        .map(|m| {
            let text = m["content"].as_str().unwrap_or("");
            let imgs: Vec<&str> = m["images"].as_array().map(|a| a.iter().filter_map(|x| x.as_str()).collect()).unwrap_or_default();
            let content = if imgs.is_empty() {
                json!(text)
            } else {
                let mut parts = vec![json!({"type": "text", "text": text})];
                for b in imgs {
                    let mime = if b.starts_with("/9j/") { "image/jpeg" } else if b.starts_with("R0lG") { "image/gif" } else if b.starts_with("UklG") { "image/webp" } else { "image/png" };
                    parts.push(json!({"type": "image_url", "image_url": {"url": format!("data:{mime};base64,{b}")}}));
                }
                json!(parts)
            };
            let mut o = json!({"role": m["role"].as_str().unwrap_or("user"), "content": content});
            // llamadas de herramientas de Ollama → OpenAI (argumentos como texto JSON)
            if let Some(calls) = m["tool_calls"].as_array() {
                let c: Vec<Value> = calls.iter().enumerate().map(|(i, c)| json!({"id": format!("call_{i}"), "type": "function",
                    "function": {"name": c["function"]["name"], "arguments": c["function"]["arguments"].to_string()}})).collect();
                o["tool_calls"] = json!(c);
            }
            o
        })
        .collect()
}

fn ollama_body(model: &str, messages: Vec<Value>, req: &Value) -> Value {
    let o = &req["options"];
    let mut b = json!({"model": model, "messages": messages, "max_tokens": o["num_predict"].as_i64().filter(|n| *n > 0).unwrap_or(4096)});
    for (from, to) in [
        ("temperature", "temperature"),
        ("top_p", "top_p"),
        ("top_k", "top_k"),
        ("min_p", "min_p"),
        ("seed", "seed"),
    ] {
        if !o[from].is_null() {
            b[to] = o[from].clone();
        }
    }
    if req["tools"].is_array() {
        b["tools"] = req["tools"].clone();
    }
    if req["format"] == "json" {
        b["response_format"] = json!({"type": "json_object"});
    }
    b
}

/// Respuesta Ollama: NDJSON en vivo (por defecto) o un solo objeto con `stream:false`.
fn ollama_reply(
    s: &Shared,
    up: Upstream,
    body: Value,
    stream: bool,
    generate: bool,
    think: bool,
) -> Response {
    let mut rx = events(up.clone(), s.http.clone(), body);
    let model = up.id.clone();
    let t0 = Instant::now();
    let (tx, out) = mpsc::channel::<Result<String, std::io::Error>>(64);
    tokio::spawn(async move {
        let piece = |text: &str, thinking: &str| -> Value {
            if generate {
                let mut v = json!({"model": model, "created_at": rfc3339(now_ms()), "response": text, "done": false});
                if !thinking.is_empty() {
                    v["thinking"] = json!(thinking);
                }
                v
            } else {
                let mut msg = json!({"role": "assistant", "content": text});
                if !thinking.is_empty() {
                    msg["thinking"] = json!(thinking);
                }
                json!({"model": model, "created_at": rfc3339(now_ms()), "message": msg, "done": false})
            }
        };
        let (mut all, mut all_think) = (String::new(), String::new());
        while let Some(e) = rx.recv().await {
            match e {
                Ev::Text(t) => {
                    if stream {
                        if tx.send(Ok(format!("{}\n", piece(&t, "")))).await.is_err() {
                            return;
                        }
                    } else {
                        all.push_str(&t);
                    }
                }
                Ev::Reasoning(t) if think => {
                    if stream {
                        let _ = tx.send(Ok(format!("{}\n", piece("", &t)))).await;
                    } else {
                        all_think.push_str(&t);
                    }
                }
                Ev::Reasoning(_) => {}
                Ev::Done(r) => {
                    let ns = t0.elapsed().as_nanos() as u64;
                    let mut last = if stream {
                        piece("", "")
                    } else {
                        piece(
                            if r.content.is_empty() {
                                &all
                            } else {
                                &r.content
                            },
                            &all_think,
                        )
                    };
                    if !r.tool_calls.is_empty() && !generate {
                        let calls: Vec<Value> = r
                            .tool_calls
                            .iter()
                            .map(
                                |c| json!({"function": {"name": c.name, "arguments": c.arguments}}),
                            )
                            .collect();
                        last["message"]["tool_calls"] = json!(calls);
                    }
                    last["done"] = json!(true);
                    last["done_reason"] = json!(if r.finish_reason == "length" {
                        "length"
                    } else {
                        "stop"
                    });
                    last["total_duration"] = json!(ns);
                    last["load_duration"] = json!(0);
                    last["prompt_eval_count"] = json!(r.prompt_tokens);
                    last["prompt_eval_duration"] = json!(0);
                    last["eval_count"] = json!(r.tokens);
                    last["eval_duration"] = json!(ns);
                    if generate {
                        last["context"] = json!([]);
                    }
                    let _ = tx.send(Ok(format!("{last}\n"))).await;
                }
                Ev::Fail(m) => {
                    let _ = tx.send(Ok(format!("{}\n", json!({"error": m})))).await;
                }
            }
        }
    });
    sse_body(
        out,
        if stream {
            "application/x-ndjson"
        } else {
            "application/json"
        },
    )
}

async fn api_chat(State(s): State<Shared>, raw: bytes::Bytes) -> Response {
    let req = match parse_json(&raw) {
        Ok(v) => v,
        Err(r) => return r,
    };
    let model = req["model"].as_str().unwrap_or("").to_string();
    let Some(up) = s.pick(&model) else {
        return no_model(&model, &s);
    };
    let body = ollama_body(&up.remote, ollama_messages(&req["messages"]), &req);
    let stream = req["stream"].as_bool().unwrap_or(true);
    ollama_reply(
        &s,
        up,
        body,
        stream,
        false,
        req["think"].as_bool().unwrap_or(false),
    )
}

async fn api_generate(State(s): State<Shared>, raw: bytes::Bytes) -> Response {
    let req = match parse_json(&raw) {
        Ok(v) => v,
        Err(r) => return r,
    };
    let model = req["model"].as_str().unwrap_or("").to_string();
    let Some(up) = s.pick(&model) else {
        return no_model(&model, &s);
    };
    let prompt = req["prompt"].as_str().unwrap_or("");
    // Ollama «carga» el modelo con un prompt vacío: se responde hecho, sin generar
    if prompt.is_empty() && req["images"].is_null() {
        return Json(json!({"model": up.id, "created_at": rfc3339(now_ms()), "response": "", "done": true, "done_reason": "load"})).into_response();
    }
    let mut msgs = Vec::new();
    if let Some(sys) = req["system"].as_str() {
        msgs.push(json!({"role": "system", "content": sys}));
    }
    msgs.push(json!({"role": "user", "content": prompt, "images": req["images"].clone()}));
    let body = ollama_body(&up.remote, ollama_messages(&json!(msgs)), &req);
    let stream = req["stream"].as_bool().unwrap_or(true);
    ollama_reply(
        &s,
        up,
        body,
        stream,
        true,
        req["think"].as_bool().unwrap_or(false),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Un «motor» OpenAI falso que contesta en streaming SSE como `mlx_lm.server`.
    async fn fake_upstream() -> String {
        async fn chat(Json(b): Json<Value>) -> Response {
            let model = b["model"].as_str().unwrap_or("").to_string();
            if b["stream"].as_bool().unwrap_or(false) {
                let mut s = String::new();
                for t in ["Ho", "la ", "Portador"] {
                    s.push_str(&format!("data: {}\n\n", json!({"choices": [{"delta": {"content": t}, "finish_reason": null}], "model": model})));
                }
                s.push_str(&format!(
                    "data: {}\n\n",
                    json!({"choices": [{"delta": {}, "finish_reason": "stop"}]})
                ));
                s.push_str(&format!(
                    "data: {}\n\n",
                    json!({"choices": [], "usage": {"prompt_tokens": 11, "completion_tokens": 3}})
                ));
                s.push_str("data: [DONE]\n\n");
                ([(header::CONTENT_TYPE, "text/event-stream")], s).into_response()
            } else {
                Json(json!({"choices": [{"message": {"role": "assistant", "content": format!("eco:{model}")}, "finish_reason": "stop"}]})).into_response()
            }
        }
        let l = bind("127.0.0.1", 0).await.unwrap();
        let addr = l.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(l, Router::new().route("/v1/chat/completions", post(chat)))
                .await
                .unwrap()
        });
        format!("http://{addr}/v1")
    }

    async fn gotv(key: Option<&str>) -> (String, tokio::sync::oneshot::Sender<()>) {
        let up = fake_upstream().await;
        let s = Shared::new(key.map(String::from));
        s.set_local(Some(Upstream {
            id: "mlx-community/Qwen3-32B-4bit-DWQ".into(),
            base_url: up,
            key: None,
            remote: "/m/32b".into(),
            wire: Wire::OpenAi,
            owned_by: "gotv".into(),
            vision: false,
            size: 18 << 30,
            motor: None,
        }));
        let l = bind("127.0.0.1", 0).await.unwrap();
        let addr = l.local_addr().unwrap();
        let (tx, rx) = tokio::sync::oneshot::channel();
        tokio::spawn(run(l, s, rx));
        (format!("http://{addr}"), tx)
    }

    #[tokio::test]
    async fn openai_y_ollama_contra_un_motor_de_verdad() {
        let (base, _stop) = gotv(None).await;
        let c = reqwest::Client::new();
        // modelos
        let m: Value = c
            .get(format!("{base}/v1/models"))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(m["data"][0]["id"], "mlx-community/Qwen3-32B-4bit-DWQ");
        let t: Value = c
            .get(format!("{base}/api/tags"))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(t["models"][0]["name"], "mlx-community/Qwen3-32B-4bit-DWQ");
        // OpenAI sin streaming: pasarela, con el nombre REAL del motor (el alias se traduce)
        let r: Value = c
            .post(format!("{base}/v1/chat/completions"))
            .json(&json!({"model": "default", "messages": [{"role": "user", "content": "hola"}]}))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(r["choices"][0]["message"]["content"], "eco:/m/32b");
        // OpenAI en streaming: los bytes del motor llegan tal cual
        let sse = c.post(format!("{base}/v1/chat/completions")).json(&json!({"model": "Qwen3-32B-4bit-DWQ", "stream": true, "messages": [{"role": "user", "content": "hola"}]}))
            .send().await.unwrap().text().await.unwrap();
        assert!(
            sse.contains("\"Portador\"") && sse.contains("[DONE]"),
            "{sse}"
        );
        // la ruta interna del motor no sale de la Mac: el cliente ve el nombre público
        assert!(
            !sse.contains("\"/m/32b\"") && sse.contains("\"mlx-community/Qwen3-32B-4bit-DWQ\""),
            "{sse}"
        );
        // Ollama /api/chat en NDJSON: trozos + cierre con conteos exactos
        // como `curl -d` o un cliente de Ollama: SIN Content-Type
        let nd = c.post(format!("{base}/api/chat")).body(json!({"model": "mlx-community/Qwen3-32B-4bit-DWQ:latest", "messages": [{"role": "user", "content": "hola"}]}).to_string())
            .send().await.unwrap().text().await.unwrap();
        let lines: Vec<Value> = nd
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        let text: String = lines
            .iter()
            .filter_map(|l| l["message"]["content"].as_str())
            .collect();
        assert_eq!(text, "Hola Portador");
        let last = lines.last().unwrap();
        assert_eq!(last["done"], true);
        assert_eq!(last["prompt_eval_count"], 11);
        assert_eq!(last["eval_count"], 3);
        // Ollama /api/generate sin streaming: un solo objeto
        let g: Value = c
            .post(format!("{base}/api/generate"))
            .json(&json!({"model": "gotv", "prompt": "hola", "stream": false}))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(g["response"], "Hola Portador");
        assert_eq!(g["done"], true);
        // un modelo que no existe: 404 que dice cuáles hay
        let nf = c
            .post(format!("{base}/v1/chat/completions"))
            .json(&json!({"model": "nada", "messages": []}))
            .send()
            .await
            .unwrap();
        assert_eq!(nf.status(), 404);
        assert!(nf.text().await.unwrap().contains("Qwen3-32B-4bit-DWQ"));
    }

    /// El modelo local se sirve desde el NÚCLEO en este proceso (su asa), sin puerto ni servidor
    /// aparte: OpenAI con y sin streaming, y Ollama.
    #[tokio::test]
    async fn el_nucleo_se_sirve_sin_puerto() {
        struct Falso;
        #[async_trait::async_trait]
        impl gotv_engine::Engine for Falso {
            fn name(&self) -> String {
                "falso".into()
            }
            fn capabilities(&self) -> gotv_engine::Capabilities {
                gotv_engine::Capabilities { text: true, vision: false, audio_in: false }
            }
            async fn start(&mut self) -> anyhow::Result<()> {
                Ok(())
            }
            async fn complete(&self, req: ChatRequest) -> anyhow::Result<ChatResponse> {
                self.complete_stream(req, &|_| {}).await
            }
            async fn complete_stream(&self, req: ChatRequest, on: &(dyn Fn(StreamEvent) + Send + Sync)) -> anyhow::Result<ChatResponse> {
                on(StreamEvent::Text("Hola ".into()));
                on(StreamEvent::Text("núcleo".into()));
                Ok(ChatResponse { content: format!("Hola núcleo ({} msj)", req.messages.len()), tokens: 2, prompt_tokens: 5, usage_exact: true, finish_reason: "stop".into(), ..Default::default() })
            }
            async fn stop(&mut self) -> anyhow::Result<()> {
                Ok(())
            }
        }
        let s = Shared::new(None);
        s.set_local(Some(local_upstream("mlx-community/Qwen3.5-9B-4bit", "/m/9b", Arc::new(Falso), true, 5 << 30)));
        let l = bind("127.0.0.1", 0).await.unwrap();
        let base = format!("http://{}", l.local_addr().unwrap());
        let (_tx, rx) = tokio::sync::oneshot::channel();
        tokio::spawn(run(l, s, rx));
        let c = reqwest::Client::new();
        let r: Value = c.post(format!("{base}/v1/chat/completions")).json(&json!({"model": "default", "messages": [{"role": "user", "content": "hola"}]})).send().await.unwrap().json().await.unwrap();
        assert_eq!(r["choices"][0]["message"]["content"], "Hola núcleo (1 msj)");
        assert_eq!(r["model"], "mlx-community/Qwen3.5-9B-4bit", "el nombre público, nunca la ruta");
        let sse = c.post(format!("{base}/v1/chat/completions")).json(&json!({"model": "Qwen3.5-9B-4bit", "stream": true, "messages": [{"role": "user", "content": "hola"}]})).send().await.unwrap().text().await.unwrap();
        assert!(sse.contains("\"núcleo\"") && sse.contains("[DONE]") && !sse.contains("/m/9b"), "{sse}");
        let g: Value = c.post(format!("{base}/api/generate")).json(&json!({"model": "gotv", "prompt": "hola", "stream": false})).send().await.unwrap().json().await.unwrap();
        assert_eq!(g["response"], "Hola núcleo (1 msj)", "la respuesta final del motor");
        assert_eq!(g["prompt_eval_count"], 5);
    }

    #[tokio::test]
    async fn con_llave_sin_ella_401() {
        let key = "gotv-0123456789abcdef0123";
        let (base, _stop) = gotv(Some(key)).await;
        let c = reqwest::Client::new();
        assert_eq!(
            c.get(format!("{base}/v1/models"))
                .send()
                .await
                .unwrap()
                .status(),
            401
        );
        assert_eq!(
            c.get(format!("{base}/api/tags"))
                .bearer_auth("gotv-mala")
                .send()
                .await
                .unwrap()
                .status(),
            401
        );
        assert_eq!(
            c.get(format!("{base}/v1/models"))
                .bearer_auth(key)
                .send()
                .await
                .unwrap()
                .status(),
            200
        );
        assert_eq!(
            c.get(format!("{base}/api/tags"))
                .header("x-api-key", key)
                .send()
                .await
                .unwrap()
                .status(),
            200
        );
        // la raíz responde siempre (los clientes de Ollama la usan para ver si vive)
        assert_eq!(c.get(&base).send().await.unwrap().status(), 200);
    }

    #[test]
    fn abrir_a_la_red_exige_llave() {
        assert!(check_bind("127.0.0.1", None).is_ok());
        assert!(check_bind("localhost", None).is_ok());
        assert!(check_bind("::1", None).is_ok());
        assert!(check_bind("0.0.0.0", None).is_err());
        assert!(check_bind("0.0.0.0", Some("corta")).is_err());
        assert!(check_bind("0.0.0.0", Some("gotv-0123456789abcdef")).is_ok());
        let k = new_key().unwrap();
        assert!(k.starts_with("gotv-") && k.len() == 53 && k != new_key().unwrap());
        assert!(
            ct_eq(b"abc", b"abc")
                && !ct_eq(b"abc", b"abd")
                && !ct_eq(b"ab", b"abc")
                && !ct_eq(b"", b"a")
        );
    }

    #[test]
    fn el_nombre_publico_no_lleva_la_ruta() {
        assert_eq!(
            public_name("/Users/x/.lmstudio/models/vanch007/Huihui-35B-mlx-4bit"),
            "vanch007/Huihui-35B-mlx-4bit"
        );
        assert_eq!(
            public_name("/Users/x/.cache/gotv/models/mlx-community/Qwen3-8B-4bit"),
            "mlx-community/Qwen3-8B-4bit"
        );
        assert_eq!(public_name("/Users/x/.cache/huggingface/hub/models--mlx-community--Qwen3-0.6B-8bit/snapshots/abc123"), "mlx-community/Qwen3-0.6B-8bit");
        assert_eq!(
            public_name("/Users/x/proyectos/ia/checkpoints/vacio-sft"),
            "vacio-sft"
        );
    }

    #[test]
    fn fechas_y_mensajes() {
        assert_eq!(rfc3339(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(rfc3339(1_794_218_400_123), "2026-11-09T10:00:00.123Z");
        let m = to_messages(
            &json!([{"role": "user", "content": [{"type": "text", "text": "mira"}, {"type": "image_url", "image_url": {"url": "data:image/png;base64,AAA"}}]}]),
        );
        assert_eq!(m[0].content, "mira");
        assert_eq!(m[0].images, vec!["data:image/png;base64,AAA".to_string()]);
        let o = ollama_messages(&json!([{"role": "user", "content": "x", "images": ["/9j/abc"]}]));
        assert_eq!(
            o[0]["content"][1]["image_url"]["url"],
            "data:image/jpeg;base64,/9j/abc"
        );
    }
}

/// Las razones de fin propias del núcleo (tope elástico y bucle cortado, F0.9) llegan a los clientes de
/// la API como lo que son para ellos: una respuesta cortada (`length`).
fn fin_para_api(fin: &str) -> &str {
    match fin {
        "" => "stop",
        "tope_texto" | "repeticion" => "length",
        otro => otro,
    }
}
