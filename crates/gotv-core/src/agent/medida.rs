//! F16.2 · `task { agente: "…" }`: el agente a medida corre como un SUBAGENTE PROPIO hecho con
//! [`Agent::hijo`] (F9): su motor (el de su `modelo`, o el del chat prestado), SUS permisos —los del chat con
//! `solo_herramientas` = su lista: `permissions.rs` niega lo demás, también en bypass— y la cara del chat con su
//! nombre delante. El agente principal no cambia nada suyo.
use super::paralelo::{FrenteHijo, Prestado};
use super::Agent;
use crate::checkpoint::Checkpoint;
use crate::escritorio::agentes;
use crate::frontend::Frontend;
use anyhow::Result;
use gotv_engine::{Engine, Message, ToolCall};

/// El bucle del hijo con su tipo BORRADO (como en F9: sin esto el compilador no decide si el futuro es `Send`).
type Bucle<'x> = std::pin::Pin<Box<dyn std::future::Future<Output = Result<String>> + Send + 'x>>;

fn bucle<'x, 'a: 'x>(h: &'x mut Agent<Prestado<'a>, FrenteHijo<'a>>, msgs: &'x mut Vec<Message>, hooks: &'x crate::hooks::Hooks, depth: u8) -> Bucle<'x> {
    Box::pin(h.run_loop(msgs, hooks, depth))
}

impl<E: Engine, F: Frontend> Agent<E, F> {
    /// `None` si la llamada no nombra un agente (sigue el subagente de siempre); si no, su respuesta.
    pub(super) async fn subagente_a_medida(&self, call: &ToolCall, hooks: &crate::hooks::Hooks, depth: u8, prompt: &str, label: &str) -> Option<String> {
        let nombre = call.arguments["agente"].as_str().map(str::trim).filter(|s| !s.is_empty())?;
        let Some(m) = agentes::buscar(&self.root, nombre) else {
            return Some(crate::tr!("ERROR: no hay un agente a medida «{nombre}».\n{}", "ERROR: there is no custom agent «{nombre}».\n{}", agentes::lista(&self.root)));
        };
        // su modelo, si es otro que el del chat
        let mut otro: Option<agentes::Motor> = None;
        if let Some(modelo) = m.modelo.clone().filter(|x| *x != self.engine.name()) {
            match agentes::motor_para(&self.proc_owner(), &self.root, &modelo).await {
                Ok(motor) => otro = Some(motor),
                // un agente de Claude Code con su alias (haiku, sonnet…) y sin llave: con el modelo del chat, y se dice
                Err(e) if m.formato == agentes::Formato::ClaudeCode => {
                    self.front.log(crate::tr!("  ⚠ el agente «{}» pide {modelo}: {e}. Trabaja con el modelo del chat", "  ⚠ agent «{}» asks for {modelo}: {e}. It works with the chat's model", m.nombre)).await;
                }
                Err(e) => return Some(crate::tr!("ERROR: el agente «{}» pide el modelo {modelo} y no se pudo cargar: {e}", "ERROR: agent «{}» asks for model {modelo} and it could not be loaded: {e}", m.nombre)),
            }
        }
        let motor: &(dyn Engine + '_) = match &otro {
            Some(a) => &**a,
            None => &self.engine,
        };
        let h_txt = m.herramientas.as_ref().map(|h| h.join(", ")).unwrap_or_else(|| crate::i18n::t("todas", "all").into());
        self.front.activity("subagent_start", label);
        self.front.log(crate::tr!("⤷ agente a medida «{}» · modelo: {} · herramientas: {h_txt}", "⤷ custom agent «{}» · model: {} · tools: {h_txt}", m.nombre, motor.name())).await;
        let mut perms = self.perms.clone();
        perms.solo_herramientas = m.herramientas.clone();
        let checkpoint = self.checkpoint.as_ref().map(|_| Checkpoint::new(&self.root));
        let turno = tokio::sync::Mutex::new(());
        let front = FrenteHijo { padre: &self.front, nombre: m.nombre.clone(), turno: &turno };
        let mut h = self.hijo(motor, self.root.clone(), perms, checkpoint, front);
        let mut msgs = vec![Message::system(self.system_prompt()), Message::user(agentes::orden(&m, prompt))];
        let id = format!("{}-{}", call.id, 2);
        h.hook_subagente_empieza(hooks, &id).await;
        let mut r = bucle(&mut h, &mut msgs, hooks, depth + 1).await;
        // F10.1: SubagentStop puede pedirle que siga (con tope), como en el camino de siempre
        for seguidas in 0..3u32 {
            let Ok(informe) = &r else { break };
            let Some(razon) = h.hook_subagente_termina(hooks, &id, informe, seguidas).await else { break };
            msgs.push(Message::assistant(informe.clone()));
            msgs.push(Message::aviso(crate::tr!("Aún no terminas — un hook del usuario dice: {razon}", "You are not done yet — a user hook says: {razon}")));
            r = bucle(&mut h, &mut msgs, hooks, depth + 1).await;
        }
        let pasos = msgs.iter().filter(|x| x.role == "tool").count();
        drop(h); // sus procesos mueren con él
        self.front.activity("subagent_end", label);
        self.front.log(crate::tr!("⤶ agente «{}» terminó ({pasos} herramienta(s) usadas)", "⤶ agent «{}» finished ({pasos} tool call(s))", m.nombre)).await;
        Some(match r {
            Ok(informe) => crate::tr!("[informe del agente «{}»]\n{informe}", "[report of agent «{}»]\n{informe}", m.nombre),
            Err(e) => crate::tr!("ERROR del agente «{}»: {e}", "agent «{}» ERROR: {e}", m.nombre),
        })
    }
}
