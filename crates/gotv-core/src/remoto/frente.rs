//! F10.5 · El frente que deja ver un chat desde el teléfono: envuelve el de la ventana (o cualquiera) y le
//! cuenta al registro lo que pasa. Un permiso se contesta donde se conteste PRIMERO (la ventana o el
//! teléfono); el otro lado se entera y su botón deja de valer.
use super::{EstadoSesion, Registro};
use crate::frontend::{Frontend, TurnStats};
use crate::permissions::AskAnswer;
use async_trait::async_trait;
use gotv_engine::StreamEvent;
use serde_json::json;

pub struct FrenteRemoto<F: Frontend> {
    pub interno: F,
    pub registro: Registro,
    pub sesion: String,
}

impl<F: Frontend> FrenteRemoto<F> {
    pub fn new(interno: F, registro: Registro, sesion: &str) -> Self {
        FrenteRemoto { interno, registro, sesion: sesion.to_string() }
    }
}

#[async_trait]
impl<F: Frontend> Frontend for FrenteRemoto<F> {
    /// El registro de herramientas NO va al teléfono (puede llevar contenido de archivos).
    async fn log(&self, line: String) {
        self.interno.log(line).await
    }

    async fn ask(&self, desc: &str) -> AskAnswer {
        let (idp, rx) = self.registro.pedir_permiso(&self.sesion, desc);
        let ventana = self.interno.ask(desc);
        tokio::pin!(ventana);
        let r = tokio::select! {
            a = &mut ventana => {
                self.registro.permiso_resuelto(&self.sesion, &idp, a);
                a
            }
            r = rx => match r {
                Ok(a) => a,
                // el chat se cerró en el registro: sólo queda la ventana
                Err(_) => ventana.await,
            },
        };
        self.registro.estado(&self.sesion, EstadoSesion::Trabajando);
        r
    }

    fn delta(&self, ev: &StreamEvent) {
        if let StreamEvent::Text(t) = ev {
            self.registro.evento(&self.sesion, "texto", json!({"texto": t}));
        }
        self.interno.delta(ev)
    }

    fn stats(&self, s: &TurnStats) {
        self.interno.stats(s)
    }

    fn activity(&self, kind: &str, detail: &str) {
        if matches!(kind, "step" | "subagent_start" | "subagent_end" | "compact") {
            self.registro.evento(&self.sesion, "actividad", json!({"tipo": kind, "detalle": detail}));
        }
        self.interno.activity(kind, detail)
    }

    async fn choose(&self, question: &str, options: &[String]) -> Option<String> {
        self.interno.choose(question, options).await
    }

    async fn secreto(&self, etiqueta: &str) -> Option<String> {
        self.interno.secreto(etiqueta).await
    }

    async fn herramienta(&self, h: crate::frontend::Herramienta<'_>) {
        self.interno.herramienta(h).await
    }
}
