//! F12 · paso 2 · `/aparte`: una pregunta al margen. Se contesta con UNA generación sobre una copia de la
//! conversación (`formas::generar_aparte`) y no entra al historial: ni la pregunta ni la respuesta quedan,
//! la sesión guardada no cambia y la caché en disco de la conversación no se toca (trampa 3 de la ficha).
//!
//! Por qué la copia lleva el MISMO principio que un paso del bucle (sistema, herramientas, anunciadas): el
//! núcleo nativo reutiliza el prefijo común que ya tiene en memoria, así que la pregunta sólo cuesta sus
//! propios tokens; la orden siguiente comparte con lo que quedó en memoria todo el historial (lo que va
//! después de él se recorta). Sin herramientas cambiaría el principio del prompt y se releería TODO.
use crate::frontend::Frontend;
use crate::Agent;
use anyhow::Result;
use gotv_engine::Engine;

/// Lo más que escribe una respuesta al margen.
pub const APARTE_MAX_TOKENS: u32 = 1_500;

/// La respuesta a `pregunta`, sin tocar la conversación (`&Agent`: el tipo garantiza que no se escribe).
pub async fn responder<E: Engine, F: Frontend>(agent: &Agent<E, F>, pregunta: &str) -> Result<String> {
    let pregunta = pregunta.trim();
    if pregunta.is_empty() {
        gotv_engine::bail_tr!("uso: /aparte <pregunta> — se responde al margen y no entra en la conversación", "usage: /aside <question> — answered on the side, it does not enter the conversation");
    }
    agent.front.activity("step", crate::i18n::t("respondiendo al margen", "answering on the side"));
    let pedido = crate::tr!(
        "{pregunta}\n\n(Pregunta al margen: respóndela sólo con texto y en pocas líneas, con lo que ya sabes de esta conversación. No llames herramientas. Ni la pregunta ni tu respuesta quedan en la conversación.)",
        "{pregunta}\n\n(Side question: answer it with text only and in a few lines, with what you already know from this conversation. Do not call tools. Neither the question nor your answer stays in the conversation.)"
    );
    let r = super::generar_aparte(agent, pedido, agent.max_tokens.clamp(256, APARTE_MAX_TOKENS)).await?;
    let t = super::sin_pensamiento(&r.content);
    if !t.is_empty() {
        return Ok(t);
    }
    if let Some(c) = r.tool_calls.first() {
        gotv_engine::bail_tr!(
            "el modelo quiso usar `{}` para contestar; /aparte sólo responde con lo que ya sabe: si hace falta mirar algo, pídelo como orden normal",
            "the model wanted to use `{}` to answer; /aside only answers with what it already knows: if something must be looked at, ask it as a normal order",
            c.name
        );
    }
    gotv_engine::bail_tr!("el modelo no respondió nada", "the model answered nothing")
}

#[cfg(test)]
mod tests {
    use super::super::prueba::{agente, foto, llamadas, texto};
    use super::*;
    use crate::permissions::Mode;
    use serde_json::json;

    /// La prueba de la ficha: `/aparte` deja `history` idéntico (mismos mensajes, mismo conteo de tokens) y
    /// la pregunta viaja con el mismo principio que un paso del bucle, sin caché de conversación en disco.
    #[tokio::test]
    async fn aparte_deja_el_historial_identico() {
        let mut c = agente("aparte", Mode::Default, vec![llamadas(&[("read_file", json!({"path": "a.txt"}))]), texto("a.txt dice hola")], vec!["Porque a.txt sólo tiene «hola»."]);
        std::fs::write(c.agente.root.join("a.txt"), "hola\n").unwrap();
        c.agente.run("lee a.txt").await.unwrap();
        let (antes, tokens_antes) = (foto(&c.agente), c.agente.context_usage());
        let paso = c.agente.engine.visto.lock().unwrap().last().cloned().unwrap();
        let r = responder(&c.agente, "¿por qué dijiste hola?").await.unwrap();
        assert_eq!(r, "Porque a.txt sólo tiene «hola».");
        assert_eq!(foto(&c.agente), antes, "el historial no cambia");
        assert_eq!(c.agente.context_usage(), tokens_antes, "ni su conteo de tokens");
        let visto = c.agente.engine.visto.lock().unwrap();
        let al_margen = visto.last().unwrap();
        // la copia: el historial entero + la pregunta, con el MISMO principio (herramientas) que el bucle
        assert_eq!(al_margen.messages.len(), c.agente.history.len() + 1);
        assert_eq!(serde_json::to_string(&al_margen.messages[..c.agente.history.len()]).unwrap(), antes);
        assert_eq!(al_margen.tools, paso.tools, "mismas herramientas: el prefijo en memoria sigue valiendo");
        assert!(al_margen.conversacion.is_none(), "la caché en disco de la conversación no se toca");
        assert!(al_margen.messages.last().unwrap().content.starts_with("¿por qué dijiste hola?"));
    }

    #[tokio::test]
    async fn sin_conversacion_y_con_herramientas_pedidas() {
        let c = agente("aparte-vacio", Mode::Default, vec![], vec!["", "dos"]);
        assert!(responder(&c.agente, "  ").await.is_err(), "sin pregunta: se explica el uso");
        // respuesta vacía: error claro, y el historial sigue vacío (no se crea ni el sistema)
        assert!(responder(&c.agente, "¿qué es esto?").await.is_err());
        assert!(c.agente.history.is_empty());
        assert_eq!(responder(&c.agente, "¿uno más uno?").await.unwrap(), "dos");
    }
}
