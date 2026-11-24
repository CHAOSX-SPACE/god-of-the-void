//! F10.3 · Un motor de GUION para probar un editor de verdad sin cargar ningún modelo (`gotv acp --guion
//! <archivo>`). El archivo es texto: bloques separados por una línea `---`; un bloque que empieza por
//! `herramienta <nombre> <json>` es una llamada a herramienta, cualquier otro es la respuesta en texto. Cada
//! petición al motor toma el siguiente bloque; acabados, responde «(guion terminado)». Nunca se usa solo:
//! hay que pedirlo con `--guion`.
use anyhow::Result;
use gotv_engine::{Capabilities, ChatRequest, ChatResponse, Engine, StreamEvent, ToolCall};
use std::collections::VecDeque;
use std::sync::Mutex;

pub struct MotorGuion {
    pasos: Mutex<VecDeque<ChatResponse>>,
    n: Mutex<u32>,
}

impl MotorGuion {
    pub fn de_texto(t: &str) -> Result<Self> {
        let mut pasos = VecDeque::new();
        for (i, b) in t.split("\n---\n").map(str::trim).filter(|b| !b.is_empty()).enumerate() {
            if let Some(resto) = b.strip_prefix("herramienta ") {
                let (nombre, json) = resto.split_once(' ').unwrap_or((resto, "{}"));
                let args: serde_json::Value = serde_json::from_str(json.trim()).map_err(|e| gotv_engine::anyhow_tr!("guion, bloque {}: argumentos no válidos: {e}", "script, block {}: invalid arguments: {e}", i + 1))?;
                pasos.push_back(ChatResponse { finish_reason: "tool_calls".into(), tool_calls: vec![ToolCall { id: format!("guion{}", i + 1), name: nombre.trim().to_string(), arguments: args }], ..Default::default() });
            } else {
                pasos.push_back(ChatResponse { content: b.to_string(), finish_reason: "stop".into(), ..Default::default() });
            }
        }
        Ok(MotorGuion { pasos: Mutex::new(pasos), n: Mutex::new(0) })
    }
}

#[async_trait::async_trait]
impl Engine for MotorGuion {
    fn name(&self) -> String {
        "guion".into()
    }
    fn capabilities(&self) -> Capabilities {
        Capabilities { text: true, vision: false, audio_in: false }
    }
    async fn start(&mut self) -> Result<()> {
        Ok(())
    }
    async fn complete(&self, _req: ChatRequest) -> Result<ChatResponse> {
        *self.n.lock().unwrap() += 1;
        Ok(self.pasos.lock().unwrap().pop_front().unwrap_or_else(|| ChatResponse { content: crate::i18n::t("(guion terminado)", "(script finished)").into(), finish_reason: "stop".into(), ..Default::default() }))
    }
    /// El texto sale a trozos, como de un modelo (así se ve «en vivo» en el editor).
    async fn complete_stream(&self, req: ChatRequest, on: &(dyn Fn(StreamEvent) + Send + Sync)) -> Result<ChatResponse> {
        let r = self.complete(req).await?;
        for trozo in r.content.split_inclusive(' ') {
            on(StreamEvent::Text(trozo.to_string()));
            tokio::time::sleep(std::time::Duration::from_millis(40)).await;
        }
        Ok(r)
    }
    async fn stop(&mut self) -> Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req() -> ChatRequest {
        ChatRequest { messages: vec![], max_tokens: 1, sampling: gotv_engine::Sampling::GREEDY, tools: None, conversacion: None, herramientas_aparte: None }
    }

    #[tokio::test]
    async fn lee_texto_y_llamadas() {
        let m = MotorGuion::de_texto("herramienta edit {\"path\": \"a.txt\", \"old\": \"x\", \"new\": \"y\"}\n---\nListo.\n").unwrap();
        let r = m.complete(req()).await.unwrap();
        assert_eq!((r.tool_calls[0].name.as_str(), r.tool_calls[0].arguments["new"].as_str()), ("edit", Some("y")));
        assert_eq!(m.complete(req()).await.unwrap().content, "Listo.");
        assert!(m.complete(req()).await.unwrap().content.contains("guion"));
        assert!(MotorGuion::de_texto("herramienta bash {roto").is_err());
    }
}
