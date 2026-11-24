//! A3 · Motores en la NUBE con los créditos del usuario.
//!
//! - [`OpenAiEngine`]: cualquier API compatible con OpenAI (`/chat/completions`):
//!   OpenAI, OpenRouter, Groq, DeepSeek, Mistral, Together, Ollama o LM Studio en
//!   otra máquina… Mismo cable que el motor local, así que reutiliza `apply_sse_line`.
//! - [`AnthropicEngine`]: la Messages API NATIVA de Claude (no un adaptador OpenAI):
//!   herramientas `tool_use`/`tool_result`, streaming SSE, caché de prompt y
//!   «pensamiento preservado» — los bloques `thinking` vuelven INTACTOS (con su firma)
//!   en `Message::blocks`.
//!
//! Formas verificadas contra la documentación oficial (2026-11-09):
//! `anthropic-version: 2026-10-21` (en esta versión `temperature`/`top_p`/`top_k`
//! dan 400: NO se envían); eventos `message_start` → `content_block_start` /
//! `content_block_delta` (`text_delta`, `thinking_delta`, `signature_delta`,
//! `input_json_delta`) / `content_block_stop` → `message_delta` (usage ACUMULADO) →
//! `message_stop`, más `ping` y `error`. La compactación de GOTV reescribe el
//! historial, y un bloque de pensamiento atado a otro prefijo daría 400: se envía
//! `thinking.block_binding.prefix_mismatch_behavior = "drop_block"` (beta
//! `thinking-binding-controls-2026-08-01`) para que el API lo descarte en vez de fallar.
//!
//! La llave NUNCA se imprime: ningún tipo de aquí deriva `Debug`.

use crate::{apply_sse_line, wire_messages, Capabilities, ChatRequest, ChatResponse, Engine, Message, StreamEvent, ToolCall};
use anyhow::{anyhow, bail, Context, Result};
use async_trait::async_trait;
use serde_json::{json, Value};

pub const ANTHROPIC_VERSION: &str = "2026-10-21";
const ANTHROPIC_URL: &str = "https://api.anthropic.com";

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(20))
        // sin tope total: una respuesta larga en streaming puede durar minutos. Pero sí tope de
        // SILENCIO entre lecturas: medido 2026-11-11, una petición a la nube se quedó sin datos y la
        // app esperó para siempre. Anthropic manda `ping` cada pocos segundos aun pensando; 180 s
        // callado es una conexión muerta.
        .read_timeout(std::time::Duration::from_secs(180))
        .build()
        .unwrap_or_default()
}

/// Un modelo que ofrece un proveedor (para el selector).
#[derive(Clone)]
pub struct CloudModel {
    pub id: String,
    pub name: String,
    /// Ventana de contexto (tokens), si el proveedor la publica.
    pub ctx: Option<u64>,
    /// Tope de salida (tokens), si lo publica.
    pub max_out: Option<u64>,
}

/// Error HTTP legible (sin la llave: el cuerpo de error no la contiene).
async fn http_error(r: reqwest::Response) -> anyhow::Error {
    let status = r.status();
    let body = r.text().await.unwrap_or_default();
    let msg = serde_json::from_str::<Value>(&body)
        .ok()
        .and_then(|v| v["error"]["message"].as_str().map(String::from).or_else(|| v["message"].as_str().map(String::from)))
        .unwrap_or_else(|| body.chars().take(300).collect());
    let hint = match status.as_u16() {
        401 | 403 => crate::i18n::t(" (llave inválida o sin permiso)", " (invalid key or no permission)"),
        402 => crate::i18n::t(" (sin saldo o créditos)", " (no balance or credits)"),
        429 => crate::i18n::t(" (límite de uso: espera y reintenta)", " (rate limit: wait and retry)"),
        529 | 503 => crate::i18n::t(" (servicio saturado: reintenta)", " (service overloaded: retry)"),
        _ => "",
    };
    anyhow!("HTTP {status}{hint}: {msg}")
}

// ───────────────────────────── OpenAI-compatible ─────────────────────────────

pub struct OpenAiEngine {
    provider: String,
    base_url: String,
    key: String,
    model: String,
    caps: Capabilities,
    client: reqwest::Client,
}

impl OpenAiEngine {
    /// `base_url` termina en `/v1` (p. ej. `https://api.openai.com/v1`). `key` vacía =
    /// sin cabecera de autorización (un Ollama/LM Studio de la red).
    pub fn new(provider: &str, base_url: &str, key: String, model: &str, vision: bool) -> Self {
        Self {
            provider: provider.into(),
            base_url: base_url.trim_end_matches('/').into(),
            key,
            model: model.into(),
            caps: Capabilities { text: true, vision, audio_in: false },
            client: client(),
        }
    }

    fn post(&self, path: &str) -> reqwest::RequestBuilder {
        let r = self.client.post(format!("{}{path}", self.base_url));
        if self.key.is_empty() {
            r
        } else {
            r.bearer_auth(&self.key)
        }
    }

    fn body(&self, req: &ChatRequest, stream: bool, usage: bool) -> Value {
        // OpenAI (y sus modelos de razonamiento) piden `max_completion_tokens`; el resto
        // del ecosistema compatible sigue con `max_tokens`. Sin muestreo: varios modelos
        // de razonamiento rechazan `temperature` distinta de la suya.
        let field = if self.base_url.contains("api.openai.com") { "max_completion_tokens" } else { "max_tokens" };
        let mut b = json!({"model": self.model, "messages": wire_messages(&req.messages), "stream": stream});
        b[field] = json!(req.max_tokens);
        // F1-A: la nube necesita TODAS las definiciones (también las anunciadas en la conversación)
        if let Some(t) = req.herramientas_todas() {
            b["tools"] = t;
        }
        if stream && usage {
            b["stream_options"] = json!({"include_usage": true});
        }
        b
    }

    pub async fn models(&self) -> Result<Vec<CloudModel>> {
        let mut r = self.client.get(format!("{}/models", self.base_url));
        if !self.key.is_empty() {
            r = r.bearer_auth(&self.key);
        }
        let r = r.send().await.context(crate::i18n::t("no hay conexión con el proveedor", "no connection to the provider"))?;
        if !r.status().is_success() {
            return Err(http_error(r).await);
        }
        let v: Value = r.json().await?;
        Ok(v["data"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|m| {
                let id = m["id"].as_str()?.to_string();
                Some(CloudModel {
                    name: m["name"].as_str().unwrap_or(&id).to_string(),
                    // OpenRouter publica `context_length`; otros, nada
                    ctx: m["context_length"].as_u64().or_else(|| m["context_window"].as_u64()),
                    max_out: m["top_provider"]["max_completion_tokens"].as_u64(),
                    id,
                })
            })
            .collect())
    }
}

#[async_trait]
impl Engine for OpenAiEngine {
    fn name(&self) -> String {
        format!("{}:{}", self.provider, self.model)
    }
    fn capabilities(&self) -> Capabilities {
        self.caps
    }
    async fn start(&mut self) -> Result<()> {
        // la llave y la red se prueban AL ELEGIR el modelo, no en la primera orden
        self.models().await.map(|_| ())
    }
    async fn complete(&self, req: ChatRequest) -> Result<ChatResponse> {
        self.complete_stream(req, &|_| {}).await
    }
    async fn complete_stream(&self, req: ChatRequest, on: &(dyn Fn(StreamEvent) + Send + Sync)) -> Result<ChatResponse> {
        let mut usage = true;
        let resp = loop {
            let r = self.post("/chat/completions").json(&self.body(&req, true, usage)).send().await.context(crate::i18n::t("no hay conexión con el proveedor", "no connection to the provider"))?;
            if r.status().as_u16() == 400 && usage {
                // algún compatible rechaza `stream_options`: se reintenta sin él
                let e = http_error(r).await;
                if e.to_string().contains("stream_options") {
                    usage = false;
                    continue;
                }
                return Err(e);
            }
            if !r.status().is_success() {
                return Err(http_error(r).await);
            }
            break r;
        };
        let mut out = ChatResponse::default();
        let mut resp = resp;
        let mut buf = String::new();
        while let Some(chunk) = resp.chunk().await.context(crate::i18n::t("se cortó el flujo del proveedor", "the provider stream was cut"))? {
            buf.push_str(&String::from_utf8_lossy(&chunk));
            while let Some(i) = buf.find('\n') {
                let line: String = buf.drain(..=i).collect();
                apply_sse_line(line.trim_end(), &mut out, on);
            }
        }
        if !buf.trim().is_empty() {
            apply_sse_line(buf.trim_end(), &mut out, on);
        }
        Ok(out)
    }
    async fn stop(&mut self) -> Result<()> {
        Ok(())
    }
    fn local(&self) -> bool {
        false
    }
}

// ─────────────────────────────── Anthropic ───────────────────────────────────

pub struct AnthropicEngine {
    model: String,
    key: String,
    base_url: String,
    /// `output_config.effort` (low · medium · high · xhigh · max); `None` = el del modelo.
    effort: Option<String>,
    max_out: u32,
    client: reqwest::Client,
}

/// Modelos con pensamiento ADAPTATIVO (4.6 en adelante): aceptan
/// `thinking: {type: "adaptive"}` y el `block_binding`.
fn adaptive(model: &str) -> bool {
    ["claude-opus-4-6", "claude-opus-4-7", "claude-opus-4-8", "claude-sonnet-4-6", "claude-opus-5", "claude-sonnet-5", "claude-haiku-5", "claude-fable", "claude-mythos"]
        .iter()
        .any(|p| model.starts_with(p))
}

/// Modelos con reserva del lado del servidor (`fallbacks: "default"`): si un
/// clasificador declina, el API reintenta en otro modelo dentro de la misma llamada.
pub fn server_fallback(model: &str) -> bool {
    // Fable 5.2 la lleva también (guía de la API, 2026-11-03); Haiku 5.5 no tiene reserva del servidor
    matches!(model, "claude-fable-5-2" | "claude-fable-5-1" | "claude-opus-5-5" | "claude-opus-5" | "claude-sonnet-5-5")
}

impl AnthropicEngine {
    pub fn new(model: &str, key: String, effort: Option<String>) -> Self {
        Self { model: model.into(), key, base_url: ANTHROPIC_URL.into(), effort, max_out: 64_000, client: client() }
    }

    /// Para pruebas o un proxy propio.
    pub fn with_base_url(mut self, url: &str) -> Self {
        self.base_url = url.trim_end_matches('/').into();
        self
    }

    fn betas(&self) -> Vec<&'static str> {
        let mut b = vec![];
        if adaptive(&self.model) {
            b.push("thinking-binding-controls-2026-08-01");
        }
        if server_fallback(&self.model) {
            b.push("server-side-fallback-2026-07-01");
        }
        b
    }

    fn request(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        let mut r = self
            .client
            .request(method, format!("{}{path}", self.base_url))
            .header("x-api-key", &self.key)
            .header("anthropic-version", ANTHROPIC_VERSION);
        let betas = self.betas();
        if !betas.is_empty() {
            r = r.header("anthropic-beta", betas.join(","));
        }
        r
    }

    pub fn body(&self, req: &ChatRequest) -> Value {
        let (system, messages) = to_anthropic(&req.messages);
        // el pensamiento cuenta dentro de `max_tokens`: 4 096 (lo de un modelo local)
        // lo ahogaría. Con streaming no hay riesgo de tiempo de espera.
        let max_tokens = req.max_tokens.max(32_000).min(self.max_out);
        let mut b = json!({
            "model": self.model,
            "max_tokens": max_tokens,
            "messages": messages,
            "stream": true,
            // caché automática sobre el último bloque cacheable (herramientas + sistema +
            // historial: lo que se reenvía en cada paso del agente)
            "cache_control": {"type": "ephemeral"},
        });
        if !system.is_empty() {
            b["system"] = json!(system);
        }
        if let Some(t) = req.herramientas_todas().as_ref().and_then(|t| t.as_array()) {
            b["tools"] = Value::Array(t.iter().filter_map(tool_to_anthropic).collect());
        }
        if adaptive(&self.model) {
            b["thinking"] = json!({"type": "adaptive", "block_binding": {"prefix_mismatch_behavior": "drop_block"}});
            if let Some(e) = &self.effort {
                b["output_config"] = json!({"effort": e});
            }
        }
        if server_fallback(&self.model) {
            b["fallbacks"] = json!("default");
        }
        b
    }

    /// Modelos de la cuenta, con su ventana de contexto y su tope de salida.
    pub async fn models(&self) -> Result<Vec<CloudModel>> {
        let r = self.request(reqwest::Method::GET, "/v1/models?limit=100").send().await.context(crate::i18n::t("no hay conexión con Anthropic", "no connection to Anthropic"))?;
        if !r.status().is_success() {
            return Err(http_error(r).await);
        }
        let v: Value = r.json().await?;
        Ok(v["data"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|m| {
                Some(CloudModel {
                    id: m["id"].as_str()?.into(),
                    name: m["display_name"].as_str().or(m["id"].as_str())?.into(),
                    ctx: m["max_input_tokens"].as_u64(),
                    max_out: m["max_tokens"].as_u64(),
                })
            })
            .collect())
    }
}

/// Herramienta en formato OpenAI → Anthropic. `eager_input_streaming`: los argumentos
/// grandes (un archivo entero) llegan mientras se generan en vez de en un bloque al
/// final; GOTV valida el JSON él mismo (argumento inválido → texto, y el agente lo
/// devuelve como error).
fn tool_to_anthropic(t: &Value) -> Option<Value> {
    let f = &t["function"];
    Some(json!({
        "name": f["name"].as_str()?,
        "description": f["description"].as_str().unwrap_or(""),
        "input_schema": if f["parameters"].is_object() { f["parameters"].clone() } else { json!({"type": "object", "properties": {}}) },
        "eager_input_streaming": true,
    }))
}

/// `data:image/png;base64,XXXX` → bloque `image` de Anthropic.
fn image_block(url: &str) -> Option<Value> {
    let rest = url.strip_prefix("data:")?;
    let (meta, data) = rest.split_once(',')?;
    let media = meta.strip_suffix(";base64")?;
    media.starts_with("image/").then(|| json!({"type": "image", "source": {"type": "base64", "media_type": media, "data": data}}))
}

/// Historial de GOTV (forma OpenAI) → `system` + `messages` de Anthropic.
/// Reglas: el sistema inicial va arriba; los `tool` consecutivos se juntan en UN
/// mensaje del usuario (llamadas en paralelo); roles iguales seguidos se funden;
/// un turno del asistente con `blocks` se devuelve INTACTO; una llamada sin
/// resultado (turno interrumpido) recibe uno sintético o el API daría 400.
pub fn to_anthropic(msgs: &[Message]) -> (String, Vec<Value>) {
    let mut system = String::new();
    let mut out: Vec<(String, Vec<Value>)> = Vec::new();
    let push = |role: &str, blocks: Vec<Value>, out: &mut Vec<(String, Vec<Value>)>| {
        if blocks.is_empty() {
            return;
        }
        match out.last_mut() {
            Some((r, b)) if r == role => b.extend(blocks),
            _ => out.push((role.into(), blocks)),
        }
    };
    let text = |t: &str| json!({"type": "text", "text": t});
    for m in msgs {
        match m.role.as_str() {
            "system" if out.is_empty() => {
                if !system.is_empty() {
                    system.push_str("\n\n");
                }
                system.push_str(&m.content);
            }
            // un aviso de sistema a mitad de la charla: como texto del usuario (anexar
            // nunca rompe el prefijo del que depende el pensamiento preservado)
            "system" => push("user", vec![text(&format!("[GOTV] {}", m.content))], &mut out),
            "user" => {
                let mut b = vec![];
                if !m.content.trim().is_empty() {
                    b.push(text(&m.content));
                }
                b.extend(m.images.iter().filter_map(|u| image_block(u)));
                if b.is_empty() {
                    b.push(text("(…)"));
                }
                push("user", b, &mut out);
            }
            "assistant" => {
                if let Some(Value::Array(raw)) = &m.blocks {
                    push("assistant", raw.clone(), &mut out);
                    continue;
                }
                let mut b = vec![];
                if !m.content.trim().is_empty() {
                    b.push(text(&m.content));
                }
                for c in m.tool_calls.iter().flatten() {
                    let args = &c["function"]["arguments"];
                    let input = match args {
                        Value::String(s) => serde_json::from_str::<Value>(s).ok().filter(Value::is_object).unwrap_or_else(|| json!({})),
                        Value::Object(_) => args.clone(),
                        _ => json!({}),
                    };
                    b.push(json!({"type": "tool_use", "id": c["id"], "name": c["function"]["name"], "input": input}));
                }
                push("assistant", b, &mut out);
            }
            "tool" => {
                let content = if m.content.is_empty() { crate::i18n::t("(sin salida)", "(no output)").to_string() } else { m.content.clone() };
                let mut r = json!({"type": "tool_result", "tool_use_id": m.tool_call_id.clone().unwrap_or_default(), "content": content});
                if m.content.starts_with("ERROR") {
                    r["is_error"] = json!(true);
                }
                push("user", vec![r], &mut out);
            }
            _ => {}
        }
    }
    // cada tool_use necesita su tool_result al COMIENZO del siguiente mensaje
    let mut i = 0;
    while i < out.len() {
        if out[i].0 == "assistant" {
            let ids: Vec<String> = out[i].1.iter().filter(|b| b["type"] == "tool_use").filter_map(|b| b["id"].as_str().map(String::from)).collect();
            if !ids.is_empty() {
                if out.get(i + 1).is_none_or(|(r, _)| r != "user") {
                    out.insert(i + 1, ("user".into(), vec![]));
                }
                let next = &mut out[i + 1].1;
                for id in ids.iter().rev() {
                    if !next.iter().any(|b| b["type"] == "tool_result" && b["tool_use_id"] == id.as_str()) {
                        next.insert(0, json!({"type": "tool_result", "tool_use_id": id, "content": crate::i18n::t("(sin resultado: el turno se interrumpió)", "(no result: the turn was interrupted)"), "is_error": true}));
                    }
                }
                // los resultados van primero; el texto del usuario, después
                next.sort_by_key(|b| b["type"] != "tool_result");
            }
        }
        i += 1;
    }
    if out.first().is_some_and(|(r, _)| r == "assistant") {
        out.insert(0, ("user".into(), vec![json!({"type": "text", "text": crate::i18n::t("(continúa)", "(continue)")})]));
    }
    (system, out.into_iter().map(|(role, content)| json!({"role": role, "content": content})).collect())
}

/// Estado del stream de Anthropic.
#[derive(Default)]
pub struct AnthropicStream {
    pub blocks: Vec<Value>,
    json: Vec<String>,
    input: u32,
    cache_read: u32,
    cache_write: u32,
    output: u32,
    pub stop: String,
    stop_category: Option<String>,
    pub error: Option<String>,
    /// Bloques de pensamiento que el API descartó (`drop_block`): se informa.
    pub dropped: usize,
}

impl AnthropicStream {
    /// Una línea SSE. Los tipos desconocidos se ignoran (el API puede añadir más).
    pub fn line(&mut self, line: &str, on: &(dyn Fn(StreamEvent) + Send + Sync)) {
        let Some(data) = line.strip_prefix("data:").map(str::trim_start) else { return };
        let Ok(v) = serde_json::from_str::<Value>(data) else { return };
        let idx = v["index"].as_u64().unwrap_or(0) as usize;
        match v["type"].as_str().unwrap_or("") {
            "message_start" => {
                let u = &v["message"]["usage"];
                self.input = u["input_tokens"].as_u64().unwrap_or(0) as u32;
                self.cache_read = u["cache_read_input_tokens"].as_u64().unwrap_or(0) as u32;
                self.cache_write = u["cache_creation_input_tokens"].as_u64().unwrap_or(0) as u32;
                self.output = u["output_tokens"].as_u64().unwrap_or(0) as u32;
                self.count_drops(&v["message"]["input_transformations"]);
            }
            "content_block_start" => {
                while self.blocks.len() <= idx {
                    self.blocks.push(Value::Null);
                    self.json.push(String::new());
                }
                self.blocks[idx] = v["content_block"].clone();
            }
            "content_block_delta" => {
                let Some(b) = self.blocks.get_mut(idx) else { return };
                let d = &v["delta"];
                match d["type"].as_str().unwrap_or("") {
                    "text_delta" => {
                        let t = d["text"].as_str().unwrap_or("");
                        append(b, "text", t);
                        if !t.is_empty() {
                            on(StreamEvent::Text(t.into()));
                        }
                    }
                    "thinking_delta" => {
                        let t = d["thinking"].as_str().unwrap_or("");
                        append(b, "thinking", t);
                        if !t.is_empty() {
                            on(StreamEvent::Reasoning(t.into()));
                        }
                    }
                    "signature_delta" => b["signature"] = d["signature"].clone(),
                    "input_json_delta" => self.json[idx].push_str(d["partial_json"].as_str().unwrap_or("")),
                    _ => {}
                }
            }
            "content_block_stop" => {
                if let Some(b) = self.blocks.get_mut(idx) {
                    if b["type"] == "tool_use" && !self.json[idx].trim().is_empty() {
                        // estricto: un JSON roto NO se arregla en silencio
                        b["input"] = serde_json::from_str::<Value>(&self.json[idx]).unwrap_or_else(|_| json!({}));
                    }
                }
            }
            "message_delta" => {
                if let Some(s) = v["delta"]["stop_reason"].as_str() {
                    self.stop = s.into();
                }
                if let Some(c) = v["delta"]["stop_details"]["category"].as_str() {
                    self.stop_category = Some(c.into());
                }
                // el usage de message_delta es ACUMULADO
                let u = &v["usage"];
                if let Some(o) = u["output_tokens"].as_u64() {
                    self.output = o as u32;
                }
                if let Some(i) = u["input_tokens"].as_u64() {
                    self.input = i as u32;
                }
                if let Some(c) = u["cache_read_input_tokens"].as_u64() {
                    self.cache_read = c as u32;
                }
                if let Some(c) = u["cache_creation_input_tokens"].as_u64() {
                    self.cache_write = c as u32;
                }
                self.count_drops(&v["input_transformations"]);
            }
            "error" => self.error = Some(format!("{}: {}", v["error"]["type"].as_str().unwrap_or("error"), v["error"]["message"].as_str().unwrap_or(""))),
            _ => {}
        }
    }

    fn count_drops(&mut self, t: &Value) {
        if let Some(a) = t.as_array() {
            self.dropped = a.iter().filter(|x| x["type"] == "thinking_dropped").count();
        }
    }

    pub fn finish(self) -> Result<ChatResponse> {
        if let Some(e) = self.error {
            bail!("Anthropic: {e}");
        }
        let blocks: Vec<Value> = self.blocks.into_iter().filter(|b| !b.is_null()).collect();
        let mut content: String = blocks.iter().filter(|b| b["type"] == "text").filter_map(|b| b["text"].as_str()).collect();
        let tool_calls: Vec<ToolCall> = blocks
            .iter()
            .filter(|b| b["type"] == "tool_use")
            .map(|b| ToolCall { id: b["id"].as_str().unwrap_or("").into(), name: b["name"].as_str().unwrap_or("").into(), arguments: b["input"].clone() })
            .collect();
        let finish_reason = match self.stop.as_str() {
            "end_turn" | "stop_sequence" => "stop",
            "tool_use" => "tool_calls",
            "max_tokens" => "length",
            other => other,
        }
        .to_string();
        if finish_reason == "refusal" {
            let why = self.stop_category.map(|c| format!(" ({c})")).unwrap_or_default();
            content.push_str(&crate::tr!("\n\n[Claude declinó continuar{why}]", "\n\n[Claude declined to continue{why}]"));
        }
        // el banco (GOTV_TIEMPOS) cuenta lo que COBRA la nube: cada petición, exacta, separada por tipo de token
        if std::env::var_os("GOTV_TIEMPOS").is_some() {
            // juez-idioma: datos (línea fija que lee el banco, como las de [tiempos])
            eprintln!("[uso] entrada {} · caché leída {} · caché escrita {} · salida {}", self.input, self.cache_read, self.cache_write, self.output);
        }
        Ok(ChatResponse {
            content,
            tokens: self.output,
            prompt_tokens: self.input + self.cache_read + self.cache_write,
            cached_tokens: self.cache_read,
            usage_exact: true,
            // un rechazo puede cortar una llamada a medias: jamás se ejecuta
            tool_calls: if finish_reason == "refusal" { vec![] } else { tool_calls },
            finish_reason,
            raw_args: vec![],
            blocks: Some(Value::Array(blocks)),
            memoria_rapida: None,
        })
    }
}

fn append(b: &mut Value, field: &str, t: &str) {
    let cur = b[field].as_str().unwrap_or("").to_string();
    b[field] = json!(cur + t);
}

#[async_trait]
impl Engine for AnthropicEngine {
    fn name(&self) -> String {
        format!("anthropic:{}", self.model)
    }
    fn capabilities(&self) -> Capabilities {
        Capabilities { text: true, vision: true, audio_in: false }
    }
    async fn start(&mut self) -> Result<()> {
        // llave + modelo + su tope de salida, al elegirlo
        let r = self.request(reqwest::Method::GET, &format!("/v1/models/{}", self.model)).send().await.context(crate::i18n::t("no hay conexión con Anthropic", "no connection to Anthropic"))?;
        if !r.status().is_success() {
            return Err(http_error(r).await);
        }
        let v: Value = r.json().await?;
        if let Some(m) = v["max_tokens"].as_u64() {
            self.max_out = (m as u32).min(64_000);
        }
        Ok(())
    }
    async fn complete(&self, req: ChatRequest) -> Result<ChatResponse> {
        self.complete_stream(req, &|_| {}).await
    }
    async fn complete_stream(&self, req: ChatRequest, on: &(dyn Fn(StreamEvent) + Send + Sync)) -> Result<ChatResponse> {
        let r = self
            .request(reqwest::Method::POST, "/v1/messages")
            .json(&self.body(&req))
            .send()
            .await
            .context(crate::i18n::t("no hay conexión con Anthropic", "no connection to Anthropic"))?;
        if !r.status().is_success() {
            return Err(http_error(r).await);
        }
        let mut r = r;
        let mut st = AnthropicStream::default();
        let mut buf = String::new();
        while let Some(chunk) = r.chunk().await.context(crate::i18n::t("se cortó el flujo de Anthropic", "the Anthropic stream was cut"))? {
            buf.push_str(&String::from_utf8_lossy(&chunk));
            while let Some(i) = buf.find('\n') {
                let line: String = buf.drain(..=i).collect();
                st.line(line.trim_end(), on);
            }
        }
        st.line(buf.trim_end(), on);
        st.finish()
    }
    async fn stop(&mut self) -> Result<()> {
        Ok(())
    }
    fn local(&self) -> bool {
        false
    }
}

// ───────────────────────────── Motor cualquiera ──────────────────────────────

/// El motor del agente: el núcleo local (Rust + MLX) o la nube. Cambiar de uno a otro en caliente
/// es reemplazar este valor. (Sin Python desde 2026-11-12: ya no hay servidor `mlx_lm`.)
pub enum AnyEngine {
    OpenAi(OpenAiEngine),
    Anthropic(AnthropicEngine),
    /// G4: el núcleo nativo (MLX dentro del proceso, crate `gotv-mlx`) u otro motor que no
    /// conviene que este crate conozca (evita dependencias circulares).
    Otro(Box<dyn Engine>),
}

impl AnyEngine {
    pub fn is_local(&self) -> bool {
        match self {
            AnyEngine::Otro(e) => e.local(),
            _ => false,
        }
    }
}

macro_rules! each {
    ($s:expr, $e:ident => $body:expr) => {
        match $s {
            AnyEngine::OpenAi($e) => $body,
            AnyEngine::Anthropic($e) => $body,
            AnyEngine::Otro($e) => $body,
        }
    };
}

#[async_trait]
impl Engine for AnyEngine {
    fn name(&self) -> String {
        each!(self, e => e.name())
    }
    fn capabilities(&self) -> Capabilities {
        each!(self, e => e.capabilities())
    }
    async fn start(&mut self) -> Result<()> {
        each!(self, e => e.start().await)
    }
    async fn complete(&self, req: ChatRequest) -> Result<ChatResponse> {
        each!(self, e => e.complete(req).await)
    }
    async fn complete_stream(&self, req: ChatRequest, on: &(dyn Fn(StreamEvent) + Send + Sync)) -> Result<ChatResponse> {
        each!(self, e => e.complete_stream(req, on).await)
    }
    async fn stop(&mut self) -> Result<()> {
        each!(self, e => e.stop().await)
    }
    fn local(&self) -> bool {
        each!(self, e => e.local())
    }
    async fn precargar(&self, req: ChatRequest, on: &(dyn Fn(StreamEvent) + Send + Sync)) -> Result<crate::Precarga> {
        each!(self, e => e.precargar(req, on).await)
    }
    fn compartir(&self) -> Option<std::sync::Arc<dyn Engine>> {
        each!(self, e => e.compartir())
    }
    fn lote_max(&self) -> usize {
        each!(self, e => e.lote_max())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool_call(id: &str, name: &str, args: &str) -> Value {
        json!({"id": id, "type": "function", "function": {"name": name, "arguments": args}})
    }

    /// El historial de GOTV pasa a la forma de Anthropic sin perder nada.
    #[test]
    fn historial_a_anthropic() {
        let mut asst = Message::assistant("Leo los dos.");
        asst.tool_calls = Some(vec![tool_call("t1", "read_file", r#"{"path":"a.rs"}"#), tool_call("t2", "read_file", r#"{"path":"b.rs"}"#)]);
        let call = |id: &str| ToolCall { id: id.into(), name: "read_file".into(), arguments: json!({}) };
        let r1 = Message::tool_result(&call("t1"), "fn a() {}");
        let r2 = Message::tool_result(&call("t2"), "ERROR: no existe b.rs");
        let msgs = vec![Message::system("Eres GOTV."), Message::user("revisa a y b"), asst, r1, r2, Message::user_with_images("¿y esta captura?", vec!["data:image/png;base64,QUJD".into()])];
        let (system, m) = to_anthropic(&msgs);
        assert_eq!(system, "Eres GOTV.");
        assert_eq!(m.len(), 3, "user · assistant · user (resultados en paralelo + la siguiente orden)");
        assert_eq!(m[1]["content"][1], json!({"type": "tool_use", "id": "t1", "name": "read_file", "input": {"path": "a.rs"}}));
        let u = m[2]["content"].as_array().unwrap();
        assert_eq!((u[0]["type"].as_str(), u[1]["type"].as_str()), (Some("tool_result"), Some("tool_result")));
        assert_eq!(u[1]["is_error"], true);
        assert_eq!(u[3], json!({"type": "image", "source": {"type": "base64", "media_type": "image/png", "data": "QUJD"}}));
    }

    /// Un turno con `blocks` (pensamiento firmado) vuelve INTACTO; una llamada sin
    /// resultado (turno interrumpido) recibe uno sintético.
    #[test]
    fn bloques_crudos_intactos_y_llamada_huerfana() {
        let raw = json!([{"type": "thinking", "thinking": "", "signature": "EqQB…"}, {"type": "tool_use", "id": "t9", "name": "bash", "input": {"cmd": "ls"}}]);
        let mut asst = Message::assistant("");
        asst.blocks = Some(raw.clone());
        let (_, m) = to_anthropic(&[Message::user("lista"), asst, Message::user("¿y?")]);
        assert_eq!(m[1]["content"], raw);
        let u = m[2]["content"].as_array().unwrap();
        assert_eq!(u[0]["tool_use_id"], "t9");
        assert_eq!(u[0]["is_error"], true);
        assert_eq!(u[1]["text"], "¿y?", "el texto va DESPUÉS de los resultados");
    }

    /// Eventos calcados de la documentación oficial de streaming (2026-11-09).
    #[test]
    fn stream_de_anthropic_con_pensamiento_texto_y_llamada() {
        let lines = [
            r#"event: message_start"#,
            r#"data: {"type":"message_start","message":{"id":"msg_01","type":"message","role":"assistant","model":"claude-opus-5-5","content":[],"stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":2679,"cache_creation_input_tokens":0,"cache_read_input_tokens":1500,"output_tokens":3},"input_transformations":[]}}"#,
            r#"data: {"type": "ping"}"#,
            r#"data: {"type": "content_block_start", "index": 0, "content_block": {"type": "thinking", "thinking": "", "signature": ""}}"#,
            r#"data: {"type": "content_block_delta", "index": 0, "delta": {"type": "thinking_delta", "thinking": ""}}"#,
            r#"data: {"type": "content_block_delta", "index": 0, "delta": {"type": "signature_delta", "signature": "EqQBCgIYAhIM"}}"#,
            r#"data: {"type": "content_block_stop", "index": 0}"#,
            r#"data: {"type":"content_block_start","index":1,"content_block":{"type":"text","text":""}}"#,
            r#"data: {"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"Miro el clima."}}"#,
            r#"data: {"type":"content_block_stop","index":1}"#,
            r#"data: {"type":"content_block_start","index":2,"content_block":{"type":"tool_use","id":"toolu_01T1","name":"get_weather","input":{}}}"#,
            r#"data: {"type":"content_block_delta","index":2,"delta":{"type":"input_json_delta","partial_json":""}}"#,
            r#"data: {"type":"content_block_delta","index":2,"delta":{"type":"input_json_delta","partial_json":"{\"location\":"}}"#,
            r#"data: {"type":"content_block_delta","index":2,"delta":{"type":"input_json_delta","partial_json":" \"San Francisco, CA\"}"}}"#,
            r#"data: {"type":"content_block_stop","index":2}"#,
            r#"data: {"type":"message_delta","delta":{"stop_reason":"tool_use","stop_sequence":null},"usage":{"output_tokens":89}}"#,
            r#"data: {"type":"message_stop"}"#,
        ];
        let texts = std::sync::Mutex::new(String::new());
        let on = |e: StreamEvent| {
            if let StreamEvent::Text(t) = e {
                texts.lock().unwrap().push_str(&t);
            }
        };
        let mut st = AnthropicStream::default();
        for l in lines {
            st.line(l, &on);
        }
        let r = st.finish().unwrap();
        assert_eq!(texts.into_inner().unwrap(), "Miro el clima.");
        assert_eq!((r.content.as_str(), r.finish_reason.as_str()), ("Miro el clima.", "tool_calls"));
        assert_eq!(r.tool_calls[0].arguments, json!({"location": "San Francisco, CA"}));
        assert_eq!((r.tokens, r.prompt_tokens, r.cached_tokens), (89, 2679 + 1500, 1500));
        let b = r.blocks.unwrap();
        assert_eq!(b[0], json!({"type": "thinking", "thinking": "", "signature": "EqQBCgIYAhIM"}), "la firma se guarda tal cual");
        assert_eq!(b[2]["input"], json!({"location": "San Francisco, CA"}));
    }

    #[test]
    fn error_en_el_stream_y_rechazo() {
        let mut st = AnthropicStream::default();
        st.line(r#"data: {"type": "error", "error": {"type": "overloaded_error", "message": "Overloaded"}}"#, &|_| {});
        assert!(st.finish().unwrap_err().to_string().contains("overloaded_error"));
        let mut st = AnthropicStream::default();
        st.line(r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"t","name":"bash","input":{}}}"#, &|_| {});
        st.line(r#"data: {"type":"message_delta","delta":{"stop_reason":"refusal","stop_details":{"type":"refusal","category":"cyber"}},"usage":{"output_tokens":4}}"#, &|_| {});
        let r = st.finish().unwrap();
        assert!(r.tool_calls.is_empty(), "una llamada cortada por un rechazo jamás se ejecuta");
        assert!(r.content.contains("cyber"));
    }

    #[test]
    fn cuerpo_de_la_peticion() {
        let e = AnthropicEngine::new("claude-opus-5-5", "x".into(), Some("high".into()));
        let tools = json!([{"type": "function", "function": {"name": "bash", "description": "corre", "parameters": {"type": "object", "properties": {"cmd": {"type": "string"}}, "required": ["cmd"]}}}]);
        let req = ChatRequest { messages: vec![Message::system("s"), Message::user("hola")], max_tokens: 4096, sampling: crate::Sampling::GREEDY, tools: Some(tools), conversacion: None, herramientas_aparte: None };
        let b = e.body(&req);
        assert_eq!(b["max_tokens"], 32_000, "el pensamiento cuenta dentro de max_tokens");
        assert!(b.get("temperature").is_none() && b.get("top_p").is_none(), "la versión 2026-10-21 rechaza el muestreo");
        assert_eq!(b["thinking"]["block_binding"]["prefix_mismatch_behavior"], "drop_block");
        assert_eq!(b["output_config"]["effort"], "high");
        assert_eq!(b["fallbacks"], "default");
        assert_eq!(b["tools"][0]["input_schema"]["required"][0], "cmd");
        assert_eq!(b["tools"][0]["eager_input_streaming"], true);
        assert_eq!(e.betas(), vec!["thinking-binding-controls-2026-08-01", "server-side-fallback-2026-07-01"]);
        // un modelo viejo: sin pensamiento adaptativo ni sus betas
        let old = AnthropicEngine::new("claude-haiku-4-5", "x".into(), None);
        assert!(old.body(&req).get("thinking").is_none() && old.betas().is_empty());
    }
}
