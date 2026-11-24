//! GOD OF THE VOID · Dios del Vacío — motor (Engine).
//!
//! El `trait Engine` es la ESPINA del proyecto: cualquier backend de tokens
//! (el núcleo MLX en Rust dentro del proceso, la nube, o NUESTROS propios modelos)
//! implementa este mismo contrato. La UI y el agente nunca ven el backend.

pub mod i18n;

/// El formato propio de los datos de GOTV (todos los crates lo usan; `gotv_core::programar::codec` lo reexporta).
pub mod codec;
pub mod cloud;
pub mod memoria_tope;
/// Carpetas temporales de las pruebas (se borran solas): sólo para `#[cfg(test)]`.
#[doc(hidden)]
pub mod prueba;

use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

pub use cloud::{AnthropicEngine, AnyEngine, CloudModel, OpenAiEngine};

/// La caché de GOTV (`~/.cache/gotv`): modelos, licencias, prefijos…
pub fn cache_dir() -> std::path::PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    std::path::PathBuf::from(home).join(".cache/gotv")
}

/// Capacidades NATIVAS de un modelo. Deciden qué herramientas se encienden
/// (visión/audio sólo si el modelo las trae de fábrica — decisión del Portador).
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct Capabilities {
    pub text: bool,
    pub vision: bool,
    pub audio_in: bool,
}

/// Llamada a herramienta pedida por el modelo (tool-calling NATIVO).
#[derive(Debug, Clone, PartialEq)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    /// Argumentos ya parseados (objeto JSON; si el modelo mandó basura, un String).
    pub arguments: serde_json::Value,
}

impl ToolCall {
    /// Forma OpenAI para guardar la llamada en el historial.
    pub fn to_openai(&self) -> serde_json::Value {
        serde_json::json!({
            "id": self.id,
            "type": "function",
            "function": {
                "name": self.name,
                "arguments": match &self.arguments {
                    serde_json::Value::String(s) => s.clone(),
                    v => v.to_string(),
                },
            }
        })
    }
}

/// Un mensaje de la conversación (formato OpenAI, con herramientas).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<serde_json::Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Imágenes adjuntas (S1), como `data:` URL. Viajan como partes `image_url`
    /// del contenido sólo a un motor con visión (ver `wire_messages`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<String>,
    /// Audio adjunto (S2), `data:audio/wav;base64,…`: viaja como `input_audio`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub audio: Vec<String>,
    /// A3: contenido CRUDO de un turno del asistente en un motor de Claude (bloques
    /// `thinking` con su firma, `text`, `tool_use`). Se devuelve INTACTO: la firma ata
    /// el pensamiento a la conversación («pensamiento preservado»). Los motores OpenAI
    /// no lo ven (`wire_messages` lo quita).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocks: Option<serde_json::Value>,
    /// Lo que el usuario ESCRIBIÓ, cuando `content` lleva además lo que se le añade para el modelo
    /// (skill elegida, perfil, notas, adjuntos, la instrucción de voz). Se guarda en la sesión para
    /// que el chat muestre y rebobine sólo tus palabras; NINGÚN motor lo ve (`wire_messages` lo quita
    /// y los demás formatos leen campos sueltos).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visible: Option<String>,
}

/// N4: la marca (`name`) de un aviso del programa a mitad de una orden. Ver [`Message::aviso`].
pub const AVISO: &str = "gotv";

/// Mensajes en el formato OpenAI del cable: las imágenes pasan a ser partes
/// `{"type":"image_url"}` del contenido; sin imágenes, el contenido sigue siendo texto.
pub fn wire_messages(msgs: &[Message]) -> serde_json::Value {
    serde_json::Value::Array(
        msgs.iter()
            .map(|m| {
                let mut v = serde_json::to_value(m).unwrap_or_default();
                if let Some(o) = v.as_object_mut() {
                    // N4: la marca de aviso es cosa del núcleo nativo; a un servidor no le llega
                    if m.es_aviso() {
                        o.remove("name");
                    }
                    o.remove("images");
                    o.remove("audio");
                    o.remove("blocks");
                    o.remove("visible");
                    if !m.images.is_empty() || !m.audio.is_empty() {
                        let mut parts = vec![serde_json::json!({"type": "text", "text": m.content})];
                        parts.extend(m.images.iter().map(|u| serde_json::json!({"type": "image_url", "image_url": {"url": u}})));
                        parts.extend(m.audio.iter().map(|u| serde_json::json!({"type": "input_audio", "input_audio": {"data": u, "format": "wav"}})));
                        o.insert("content".into(), serde_json::Value::Array(parts));
                    }
                }
                v
            })
            .collect(),
    )
}

impl Message {
    fn plain(role: &str, c: impl Into<String>) -> Self {
        Self {
            role: role.into(),
            content: c.into(),
            tool_calls: None,
            tool_call_id: None,
            name: None,
            images: Vec::new(),
            audio: Vec::new(),
            blocks: None,
            visible: None,
        }
    }
    /// Mensaje del usuario con imágenes (`data:` URLs).
    pub fn user_with_images(c: impl Into<String>, images: Vec<String>) -> Self {
        let mut m = Self::plain("user", c);
        m.images = images;
        m
    }
    /// Mensaje del usuario con medios: los `data:image/…` van como imagen y los
    /// `data:audio/…` como audio.
    pub fn user_with_media(c: impl Into<String>, media: Vec<String>) -> Self {
        let mut m = Self::plain("user", c);
        let (audio, images): (Vec<String>, Vec<String>) = media.into_iter().partition(|u| u.starts_with("data:audio/"));
        m.images = images;
        m.audio = audio;
        m
    }
    pub fn system(c: impl Into<String>) -> Self {
        Self::plain("system", c)
    }
    pub fn user(c: impl Into<String>) -> Self {
        Self::plain("user", c)
    }
    /// N4 (2026-11-14): un AVISO del programa a mitad de una orden (un rescate, un recordatorio, un
    /// pedido de medir). Para todo el mundo es un mensaje de usuario; lleva la marca [`AVISO`] para
    /// que el núcleo nativo lo escriba como resultado de herramienta. Medido con las plantillas de
    /// Qwen3.5-9B y Qwen3.6-35B: un mensaje de usuario nuevo cambia cómo se escriben TODOS los turnos
    /// de la IA desde el anterior (pierden su `<think>`), el prompt ya no continúa el que se leyó y
    /// hay que releerlo entero (9 171 tokens por un aviso, en el Air); como resultado, nada cambia.
    pub fn aviso(c: impl Into<String>) -> Self {
        let mut m = Self::plain("user", c);
        m.name = Some(AVISO.into());
        m
    }
    /// ¿Es un aviso del programa ([`Message::aviso`]) y no algo que escribió el usuario?
    pub fn es_aviso(&self) -> bool {
        self.role == "user" && self.name.as_deref() == Some(AVISO)
    }
    pub fn assistant(c: impl Into<String>) -> Self {
        Self::plain("assistant", c)
    }
    /// Turno del asistente que pidió herramientas.
    pub fn assistant_calls(c: impl Into<String>, calls: &[ToolCall]) -> Self {
        let mut m = Self::plain("assistant", c);
        m.tool_calls = Some(calls.iter().map(ToolCall::to_openai).collect());
        m
    }
    /// Resultado de una herramienta, atado a su llamada.
    pub fn tool_result(call: &ToolCall, c: impl Into<String>) -> Self {
        let mut m = Self::plain("tool", c);
        m.tool_call_id = Some(call.id.clone());
        m.name = Some(call.name.clone());
        m
    }
}

/// Cómo elegir cada token. Lo dicta el MODELO (`generation_config.json`):
/// medido, el voraz (`temperature 0`) que usábamos hacía que Qwen3-32B
/// escribiera `edit(…)` como texto en vez de llamarla (Qwen lo advierte:
/// «no uses decodificación voraz»).
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Sampling {
    pub temperature: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_k: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_p: Option<f32>,
}

impl Sampling {
    /// Determinista: sólo para pedidos de 1 token (precalentar) o pruebas.
    pub const GREEDY: Sampling = Sampling { temperature: 0.0, top_p: None, top_k: None, min_p: None };

    /// Escribe los parámetros en el cuerpo de la petición OpenAI.
    pub fn put(&self, body: &mut serde_json::Value) {
        body["temperature"] = serde_json::json!(self.temperature);
        if let Some(v) = self.top_p {
            body["top_p"] = serde_json::json!(v);
        }
        if let Some(v) = self.top_k {
            body["top_k"] = serde_json::json!(v);
        }
        if let Some(v) = self.min_p {
            body["min_p"] = serde_json::json!(v);
        }
    }

    pub fn summary(&self) -> String {
        let mut s = format!("temp {}", self.temperature);
        if let Some(v) = self.top_p {
            s += &format!(" · top_p {v}");
        }
        if let Some(v) = self.top_k {
            s += &format!(" · top_k {v}");
        }
        if let Some(v) = self.min_p {
            s += &format!(" · min_p {v}");
        }
        s
    }
}

/// Petición de completado.
#[derive(Debug, Clone, Serialize)]
pub struct ChatRequest {
    pub messages: Vec<Message>,
    pub max_tokens: u32,
    pub sampling: Sampling,
    /// Esquemas de herramientas (formato OpenAI). `None` = sin herramientas.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<serde_json::Value>,
    /// Carpeta donde el núcleo nativo guarda y lee la caché de ESTA conversación (para que reabrirla
    /// no obligue a releer todo el historial). `None` = no se guarda. No viaja a servidores.
    #[serde(skip)]
    pub conversacion: Option<std::path::PathBuf>,
    /// F1-A de la memoria rápida: esquemas de herramientas que se ANUNCIARON dentro de la
    /// conversación (no van en `tools`, que queda fija al principio del prompt). El núcleo los usa
    /// sólo para tipar los argumentos; los motores de servidor y de nube los necesitan como
    /// definiciones: para ellos se usa [`ChatRequest::herramientas_todas`].
    #[serde(skip)]
    pub herramientas_aparte: Option<serde_json::Value>,
}

impl ChatRequest {
    /// `tools` + `herramientas_aparte` (sin repetir nombres): las definiciones completas.
    pub fn herramientas_todas(&self) -> Option<serde_json::Value> {
        let aparte = self.herramientas_aparte.as_ref().and_then(|a| a.as_array()).filter(|a| !a.is_empty());
        let Some(aparte) = aparte else { return self.tools.clone() };
        let mut todas: Vec<serde_json::Value> = self.tools.as_ref().and_then(|t| t.as_array()).cloned().unwrap_or_default();
        let nombre = |t: &serde_json::Value| t["function"]["name"].as_str().unwrap_or("").to_string();
        let ya: std::collections::HashSet<String> = todas.iter().map(nombre).collect();
        todas.extend(aparte.iter().filter(|t| !ya.contains(&nombre(t))).cloned());
        Some(serde_json::Value::Array(todas))
    }
}

/// Respuesta del motor.
#[derive(Debug, Clone, Default)]
pub struct ChatResponse {
    pub content: String,
    pub tokens: u32,
    /// Tokens del prompt (para medir el prellenado en tok/s, P11).
    pub prompt_tokens: u32,
    /// De esos, cuántos salieron de la caché de prompts (no se recalcularon).
    pub cached_tokens: u32,
    /// `true` si `tokens`/`prompt_tokens` vienen del `usage` del motor (exactos) y no
    /// de contar trozos del stream (aproximado).
    pub usage_exact: bool,
    /// A3: contenido crudo del turno (Claude) para guardarlo en `Message::blocks`.
    pub blocks: Option<serde_json::Value>,
    /// `stop`, `length` (se cortó por max_tokens), `tool_calls`…
    pub finish_reason: String,
    /// Llamadas a herramientas parseadas por el servidor (puede haber varias).
    pub tool_calls: Vec<ToolCall>,
    /// Memoria rápida (caché de la conversación en disco): por qué NO se pudo usar, si la petición la
    /// pedía y hubo que releer. `None` = se usó, no hacía falta o el motor no la tiene.
    pub memoria_rapida: Option<String>,
    /// Streaming: argumentos crudos de cada llamada mientras llegan en trozos, y su `index`.
    #[doc(hidden)]
    pub raw_args: Vec<(Option<u64>, String)>,
}

#[cfg(test)]
mod tests {
    /// D7: el `data: {"error": …}` de mlx_vlm (su lector de llamadas reventó) no puede terminar
    /// el turno en silencio: queda marcado para que el motor lo convierta en error visible.
    #[test]
    fn un_error_del_flujo_queda_marcado() {
        let mut out = ChatResponse::default();
        apply_sse_line(r#"data: {"error": "closing parenthesis '}' does not match opening parenthesis '['"}"#, &mut out, &|_| {});
        assert!(out.finish_reason.starts_with("error: closing parenthesis"), "{}", out.finish_reason);
        assert!(out.tool_calls.is_empty() && out.content.is_empty());
    }

    /// El trozo final de `usage` (forma real de mlx_lm con include_usage) corrige el
    /// conteo aproximado de trozos.
    #[test]
    fn usage_exacto_del_stream_manda_sobre_el_conteo() {
        let mut out = super::ChatResponse::default();
        let on = |_: super::StreamEvent| {};
        for l in [
            r#"data: {"choices": [{"index": 0, "delta": {"content": "Hola"}}]}"#,
            r#"data: {"choices": [{"index": 0, "delta": {"content": " mundo"}, "finish_reason": "stop"}]}"#,
            r#"data: {"id": "x", "object": "chat.completion", "choices": [], "usage": {"prompt_tokens": 812, "completion_tokens": 3, "total_tokens": 815, "prompt_tokens_details": {"cached_tokens": 790}}}"#,
            "data: [DONE]",
        ] {
            super::apply_sse_line(l, &mut out, &on);
        }
        assert_eq!((out.content.as_str(), out.tokens, out.prompt_tokens, out.cached_tokens, out.usage_exact), ("Hola mundo", 3, 812, 790, true));
    }

    #[test]
    fn llamadas_en_trozos_al_estilo_openai() {
        // líneas reales de mlx_vlm (recortadas): los argumentos llegan en 6 trozos
        let lines = [
            r#"data: {"choices":[{"index":0,"finish_reason":null,"delta":{"role":"assistant","content":null,"tool_calls":[{"index":0,"id":"6e4c","type":"function","function":{"name":"view_image","arguments":"{"}}]}}]}"#,
            r#"data: {"choices":[{"index":0,"finish_reason":null,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"\"path\": \""}}]}}]}"#,
            r#"data: {"choices":[{"index":0,"finish_reason":null,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"num"}}]}}]}"#,
            r#"data: {"choices":[{"index":0,"finish_reason":null,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"."}}]}}]}"#,
            r#"data: {"choices":[{"index":0,"finish_reason":null,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"png"}}]}}]}"#,
            r#"data: {"choices":[{"index":0,"finish_reason":null,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"\"}"}}]}}]}"#,
            r#"data: {"choices":[{"index":0,"finish_reason":"tool_calls","delta":{}}]}"#,
        ];
        let mut out = ChatResponse::default();
        for l in lines {
            apply_sse_line(l, &mut out, &|_| {});
        }
        assert_eq!(out.tool_calls.len(), 1);
        assert_eq!(out.tool_calls[0].id, "6e4c");
        assert_eq!(out.tool_calls[0].arguments, serde_json::json!({"path": "num.png"}));
        // y dos llamadas enteras seguidas (estilo mlx_lm, mismo index) siguen siendo dos
        let mut two = ChatResponse::default();
        for name in ["read_file", "list_dir"] {
            let l = format!(r#"data: {{"choices":[{{"delta":{{"tool_calls":[{{"index":0,"id":"x","function":{{"name":"{name}","arguments":"{{}}"}}}}]}}}}]}}"#);
            apply_sse_line(&l, &mut two, &|_| {});
        }
        assert_eq!(two.tool_calls.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), vec!["read_file", "list_dir"]);
    }

    #[test]
    fn imagenes_viajan_como_partes_del_contenido() {
        let w = crate::wire_messages(&[
            Message::user("sin imagen"),
            Message::user_with_images("mira", vec!["data:image/jpeg;base64,AAA".into()]),
        ]);
        assert_eq!(w[0]["content"], "sin imagen");
        assert!(w[0].get("images").is_none());
        assert_eq!(w[1]["content"][0], serde_json::json!({"type": "text", "text": "mira"}));
        assert_eq!(w[1]["content"][1]["image_url"]["url"], "data:image/jpeg;base64,AAA");
        let a = crate::wire_messages(&[Message::user_with_media("oye", vec!["data:audio/wav;base64,BBB".into(), "data:image/jpeg;base64,CCC".into()])]);
        assert_eq!(a[0]["content"][1]["image_url"]["url"], "data:image/jpeg;base64,CCC");
        assert_eq!(a[0]["content"][2], serde_json::json!({"type": "input_audio", "input_audio": {"data": "data:audio/wav;base64,BBB", "format": "wav"}}));
    }

    use super::*;
    use std::sync::Mutex;

    /// Líneas reales capturadas de mlx_lm.server 0.31.3 en modo stream.
    #[test]
    fn sse_real_de_mlx() {
        let ev = Mutex::new(Vec::new());
        let on = |e: StreamEvent| ev.lock().unwrap().push(e);
        let mut out = ChatResponse::default();
        for l in [
            ": keepalive 23/24",
            r#"data: {"choices": [{"index": 0, "finish_reason": null, "delta": {"role": "assistant", "reasoning": "\n\n"}}]}"#,
            r#"data: {"choices": [{"index": 0, "finish_reason": null, "delta": {"role": "assistant", "content": "1"}}]}"#,
            r#"data: {"choices": [{"index": 0, "finish_reason": null, "delta": {"role": "assistant", "content": ", ñandú"}}]}"#,
            r#"data: {"choices": [{"index": 0, "finish_reason": null, "delta": {"role": "assistant", "tool_calls": [{"function": {"name": "read_file", "arguments": "{\"path\": \"/main.rs\"}"}, "type": "function", "id": "db6b", "index": 0}]}}]}"#,
            r#"data: {"choices": [{"index": 0, "finish_reason": "tool_calls", "delta": {"role": "assistant"}}]}"#,
            "data: [DONE]",
        ] {
            apply_sse_line(l, &mut out, &on);
        }
        assert_eq!(out.content, "1, ñandú");
        assert_eq!(out.finish_reason, "tool_calls");
        assert_eq!(out.tool_calls.len(), 1);
        assert_eq!(out.tool_calls[0].arguments["path"], "/main.rs");
        let ev = ev.lock().unwrap();
        assert_eq!(ev[0], StreamEvent::Prefill { done: 23, total: 24 });
        assert!(ev.contains(&StreamEvent::Text(", ñandú".into())));
    }
}

/// Convierte el `tool_calls` de una respuesta OpenAI en `ToolCall`s.
pub fn parse_tool_calls(v: &serde_json::Value) -> Vec<ToolCall> {
    v.as_array()
        .map(|a| {
            a.iter()
                .enumerate()
                .filter_map(|(i, c)| {
                    let f = c.get("function")?;
                    let name = f.get("name")?.as_str()?.to_string();
                    let arguments = match f.get("arguments") {
                        Some(serde_json::Value::String(s)) => serde_json::from_str(s)
                            .unwrap_or_else(|_| serde_json::Value::String(s.clone())),
                        Some(v) => v.clone(),
                        None => serde_json::json!({}),
                    };
                    let id = c
                        .get("id")
                        .and_then(|x| x.as_str())
                        .map(String::from)
                        .unwrap_or_else(|| format!("call_{i}"));
                    Some(ToolCall { id, name, arguments })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Lo que llega mientras el modelo trabaja (streaming, P3).
#[derive(Debug, Clone, PartialEq)]
pub enum StreamEvent {
    /// Avance del prellenado (el servidor lee el contexto): `hecho` de `total`.
    Prefill { done: u32, total: u32 },
    /// Razonamiento visible del modelo (si lo emite).
    Reasoning(String),
    /// Texto de la respuesta, token a token.
    Text(String),
    /// Trozo de los ARGUMENTOS de una llamada a herramienta mientras llega. Medido (D5, 9B): un
    /// `write_file` degeneró repitiendo «Pistacho Chía Coco…» 11 k caracteres y nadie lo veía:
    /// el tope de tokens y el detector de bucles sólo miraban el texto.
    ToolArgs(String),
}

/// Interpreta UNA línea SSE de `mlx_lm.server` y la acumula en `out`.
/// Formato medido: `: keepalive N/M` durante el prellenado; `data: {chunk}` con
/// `delta.content` / `delta.reasoning` / `delta.tool_calls` (enteras); `data: [DONE]`.
pub fn apply_sse_line(line: &str, out: &mut ChatResponse, on: &(dyn Fn(StreamEvent) + Send + Sync)) {
    let line = line.trim_end_matches('\r');
    if let Some(rest) = line.strip_prefix(": keepalive ") {
        if let Some((a, b)) = rest.split_once('/') {
            if let (Ok(done), Ok(total)) = (a.trim().parse(), b.trim().parse()) {
                on(StreamEvent::Prefill { done, total });
            }
        }
        return;
    }
    let Some(data) = line.strip_prefix("data: ") else { return };
    if data == "[DONE]" {
        return;
    }
    let Ok(v) = serde_json::from_str::<serde_json::Value>(data) else { return };
    // Medido (D7, 2026-11-09): mlx_vlm manda `data: {"error": "…"}` cuando su lector de
    // llamadas revienta, y el turno terminaba vacío y MUDO. Ahora queda como error visible.
    if let Some(e) = v.get("error").filter(|e| !e.is_null()) {
        let msg = e.as_str().map(String::from).or_else(|| e["message"].as_str().map(String::from)).unwrap_or_else(|| e.to_string());
        out.finish_reason = format!("error: {msg}");
        return;
    }
    let choice = &v["choices"][0];
    let delta = &choice["delta"];
    if let Some(t) = delta["content"].as_str().filter(|t| !t.is_empty()) {
        out.content.push_str(t);
        out.tokens += 1;
        on(StreamEvent::Text(t.to_string()));
    }
    if let Some(r) = delta["reasoning"].as_str().filter(|r| !r.is_empty()) {
        out.tokens += 1;
        on(StreamEvent::Reasoning(r.to_string()));
    }
    // Llamadas en streaming. Medido (S1): `mlx_vlm` manda los argumentos EN TROZOS
    // («{», «"path": "», «num», «.png»…, estándar de OpenAI) y GOTV se quedaba con
    // «{»; `mlx_lm` manda cada llamada entera. Regla: un trozo con `name` abre una
    // llamada nueva; sin `name`, se concatena a la abierta con ese `index`.
    for frag in delta["tool_calls"].as_array().into_iter().flatten() {
        let idx = frag["index"].as_u64();
        let f = &frag["function"];
        let piece = match &f["arguments"] {
            serde_json::Value::String(t) => t.clone(),
            serde_json::Value::Null => String::new(),
            v => v.to_string(),
        };
        if !piece.is_empty() {
            on(StreamEvent::ToolArgs(piece.clone()));
        }
        match f["name"].as_str().filter(|n| !n.is_empty()) {
            Some(name) => {
                let n = out.tool_calls.len();
                let id = frag["id"].as_str().map(String::from).unwrap_or_else(|| format!("call_{n}"));
                out.tool_calls.push(ToolCall { id, name: name.to_string(), arguments: serde_json::json!({}) });
                out.raw_args.push((idx, piece));
            }
            None => {
                if let Some(slot) = out.raw_args.iter_mut().rev().find(|(i, _)| *i == idx || idx.is_none()) {
                    slot.1.push_str(&piece);
                }
            }
        }
    }
    // argumentos al día: JSON si ya cierra; si aún no, el texto crudo
    for (call, (_, raw)) in out.tool_calls.iter_mut().zip(&out.raw_args) {
        if !raw.trim().is_empty() {
            call.arguments = serde_json::from_str(raw).unwrap_or_else(|_| serde_json::Value::String(raw.clone()));
        }
    }
    if let Some(f) = choice["finish_reason"].as_str() {
        out.finish_reason = f.to_string();
    }
    // A2: el `usage` EXACTO llega en un trozo final sin `choices` cuando se pide con
    // `stream_options.include_usage` (mlx_lm y mlx_vlm, medido en su código): manda
    // sobre el conteo de trozos, que es aproximado.
    if let Some(u) = v["usage"].as_object() {
        let n = |k: &str| u.get(k).and_then(|x| x.as_u64()).unwrap_or(0) as u32;
        out.tokens = n("completion_tokens");
        out.prompt_tokens = n("prompt_tokens");
        out.cached_tokens = u.get("prompt_tokens_details").and_then(|d| d["cached_tokens"].as_u64()).unwrap_or(0) as u32;
        out.usage_exact = true;
    }
}

/// La espina. Etapa 1 = MLX por servidor (`MlxServerEngine`);
/// etapa 2 (futuro) = in-process con `mlxrs`; y `NativeTinyEngine` para los
/// modelos que entrenamos en `proyectos/ia`.
#[async_trait]
pub trait Engine: Send + Sync {
    /// Nombre legible del backend/modelo.
    fn name(&self) -> String;
    /// Capacidades nativas (texto/visión/audio) del modelo cargado.
    fn capabilities(&self) -> Capabilities;
    /// Enciende el motor (arranca el proceso hijo si hace falta) y espera a que viva.
    async fn start(&mut self) -> Result<()>;
    /// Una respuesta completa a una petición.
    async fn complete(&self, req: ChatRequest) -> Result<ChatResponse>;
    /// Igual, pero emitiendo eventos mientras genera. Por defecto (motores sin
    /// streaming) emite el texto entero al final.
    async fn complete_stream(
        &self,
        req: ChatRequest,
        on: &(dyn Fn(StreamEvent) + Send + Sync),
    ) -> Result<ChatResponse> {
        let r = self.complete(req).await?;
        if !r.content.is_empty() {
            on(StreamEvent::Text(r.content.clone()));
        }
        Ok(r)
    }
    /// Apaga el motor (mata el hijo, libera RAM).
    async fn stop(&mut self) -> Result<()>;
    /// ¿Corre en esta Mac? (en la nube cada petición gasta créditos del usuario)
    fn local(&self) -> bool {
        true
    }
    /// F4 de la memoria rápida: deja lista EN MEMORIA la conversación de `req` (su último mensaje es
    /// un postizo: se lee todo lo de ANTES, sin generar) y la guarda en disco. Por omisión, un motor
    /// sin memoria rápida: `admitida: false`.
    async fn precargar(&self, _req: ChatRequest, _on: &(dyn Fn(StreamEvent) + Send + Sync)) -> Result<Precarga> {
        Ok(Precarga::default())
    }
    /// Un ASA al MISMO motor ya encendido (el mismo modelo cargado, sin cargar otro): el servicio de
    /// API la usa para servir el modelo que la app tiene en memoria. Soltarla no apaga el motor.
    /// `None` = este motor no se comparte.
    fn compartir(&self) -> Option<std::sync::Arc<dyn Engine>> {
        None
    }
    /// F9 · cuántas conversaciones genera JUNTAS en un mismo paso sobre los mismos pesos (generación en
    /// lote). Un motor LOCAL que no la tiene responde 1: sus subagentes van por turnos, uno tras otro
    /// (intercalarlos obligaría a releer la conversación de cada uno en su única caché). La nube no
    /// pregunta esto: allí cada conversación es una petición aparte y van en paralelo.
    fn lote_max(&self) -> usize {
        1
    }
}

/// Lo que dejó una precarga (F4 de la memoria rápida).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Precarga {
    /// false: este motor no tiene memoria rápida (o la plantilla no deja saber dónde cortar)
    pub admitida: bool,
    /// tokens de la conversación listos en memoria
    pub tokens: usize,
    /// cuántos vinieron del disco
    pub del_disco: usize,
    /// cuántos hubo que leer con el modelo
    pub calculados: usize,
    /// bytes de la memoria en disco tras la precarga
    pub bytes: u64,
    /// por qué no se usó el disco, si no se usó (la causa de F1)
    pub nota: Option<String>,
}
