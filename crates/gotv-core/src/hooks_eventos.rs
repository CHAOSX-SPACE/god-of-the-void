//! F10.1 · Dónde dispara cada evento de los hooks dentro del agente. Cada función de aquí se llama desde
//! UN punto de `agent.rs` (anotado en la entrega de F10); la lógica vive aquí para no engordar el agente.
//!
//! | Evento | Punto de disparo |
//! |---|---|
//! | `SessionStart` | primera orden de la conversación en este proceso (`startup` si no hay órdenes previas, `resume` si se retomó) y tras compactar (`compact`) |
//! | `UserPromptSubmit` | cada orden, antes de que llegue al modelo: puede bloquearla o sumarle contexto |
//! | `FileChanged` | al empezar cada orden y tras cada herramienta: los archivos que nombran sus `matcher` se comparan con la última foto (`add`, `change`, `unlink`) |
//! | `PermissionRequest` | cuando la puerta de permisos va a PREGUNTAR (una llamada o el lote) |
//! | `Notification` | `permission_prompt` cuando se le pregunta a la persona; `agent_completed` al terminar la orden |
//! | `PreCompact` / `PostCompact` | antes y después de resumir la conversación (`manual` o `auto`) |
//! | `SubagentStart` / `SubagentStop` | al lanzar y al volver un subagente (`task`); `SubagentStop` puede pedirle que siga |
//! | `SessionEnd` | `Agent::cerrar_sesion` (la CLI al salir, el chat del editor al cerrarse) |
//!
//! **Ningún hook abre una puerta que la ventana no abriría** (regla del Portador): un `PermissionRequest`
//! puede NEGAR siempre, pero su «allow» no vale para lo irreversible ni para lo que toca el sistema (eso lo
//! sigue contestando una persona); y en bypass no hay preguntas, así que el evento no llega a dispararse.
use crate::agent::Agent;
use crate::frontend::Frontend;
use crate::hooks::{Comunes, Entrada, Event, Hooks, Juez, Permiso, Salida};
use gotv_engine::{ChatRequest, Engine, Message, ToolCall};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

/// Los hooks que la orden en curso decidió usar (con o sin los del proyecto, según la confianza), por
/// proyecto: los puntos que no reciben `hooks` (la compactación, el lote de permisos) usan los mismos.
static VIGENTES: Mutex<Option<HashMap<PathBuf, Hooks>>> = Mutex::new(None);
/// Las conversaciones que ya dispararon `SessionStart` en este proceso.
static ARRANCADAS: Mutex<Option<HashSet<String>>> = Mutex::new(None);
/// La última foto de los archivos vigilados por `FileChanged`: (proyecto, ruta) → (fecha, tamaño).
type Foto = HashMap<(PathBuf, PathBuf), Option<(SystemTime, u64)>>;
static FOTOS: Mutex<Option<Foto>> = Mutex::new(None);

/// Los modos de GOTV con el nombre de Claude Code (`permission_mode`).
fn modo(m: crate::permissions::Mode) -> &'static str {
    use crate::permissions::Mode;
    match m {
        Mode::Plan => "plan",
        Mode::Default => "default",
        Mode::AcceptEdits => "acceptEdits",
        Mode::Auto => "auto",
        Mode::Bypass => "bypassPermissions",
    }
}

/// ¿Lo pide una persona sí o sí? Lo irreversible (modo automático) y lo que toca otra app o el sistema.
pub fn solo_una_persona(desc: &str) -> bool {
    ["IRREVERSIBLE", "⚠ SISTEMA", "⚠ SYSTEM"].iter().any(|m| desc.contains(m)) // juez-idioma: datos (marcas que pone la puerta de permisos)
}

/// La consulta de un hook `prompt`, con el motor de la sesión (corta, sin herramientas, aparte).
struct JuezMotor<'a, E: Engine> {
    engine: &'a E,
    sampling: gotv_engine::Sampling,
}

#[async_trait::async_trait]
impl<E: Engine> Juez for JuezMotor<'_, E> {
    async fn preguntar(&self, prompt: &str, tope: Duration) -> Option<String> {
        let sistema = crate::i18n::t(
            "Evalúas un hook de GOTV. Lee la entrada y responde SÓLO un objeto JSON: {\"ok\": true} si se cumple lo que pide el hook, o {\"ok\": false, \"reason\": \"una frase\"} si no.",
            "You evaluate a GOTV hook. Read the input and answer ONLY a JSON object: {\"ok\": true} if what the hook asks holds, or {\"ok\": false, \"reason\": \"one sentence\"} if not.",
        );
        let req = ChatRequest { messages: vec![Message::system(sistema), Message::user(prompt.to_string())], max_tokens: 200, sampling: self.sampling, tools: None, conversacion: None, herramientas_aparte: None };
        let r = tokio::time::timeout(tope, self.engine.complete(req)).await.ok()?.ok()?;
        let t = match r.content.rfind("</think>") {
            Some(i) => r.content[i + 8..].to_string(),
            None => r.content,
        };
        Some(t.trim().to_string()).filter(|t| !t.is_empty())
    }
}

impl<E: Engine, F: Frontend> Agent<E, F> {
    fn hooks_comunes(&self) -> Comunes {
        Comunes { session_id: self.session.as_ref().map(|s| s.id.to_string()).unwrap_or_default(), cwd: self.root.clone(), permission_mode: modo(self.perms.mode).to_string() }
    }

    /// Los hooks de la orden en curso (los que eligió `load_hooks`); sin orden en curso, los que valen sin
    /// preguntar: los del usuario, los de plugins confiados y los del proyecto sólo si ya se confió en ellos.
    pub(crate) fn hooks_vigentes(&self) -> Hooks {
        if let Some(h) = VIGENTES.lock().unwrap().as_ref().and_then(|m| m.get(&self.root)) {
            return h.clone();
        }
        let (h, fp) = crate::hooks::load(&self.root);
        match fp {
            Some(fp) if !crate::hooks::is_trusted(&self.root, &fp) => h.without_project(),
            _ => h,
        }
    }

    /// Dispara y deja constancia: cada corrida y cada aviso van a la transcripción.
    pub(crate) async fn hooks_disparar(&self, hooks: &Hooks, evento: Event, objetivo: &str, campos: Value) -> Salida {
        if !hooks.tiene(evento) {
            return Salida::default();
        }
        let juez = JuezMotor { engine: &self.engine, sampling: self.sampling };
        let s = hooks.disparar(&Entrada { evento, objetivo: objetivo.to_string(), campos }, &self.hooks_comunes(), Some(&juez)).await;
        for c in &s.corridas {
            self.front.log(format!("  ⚓ hook {c}")).await;
        }
        for a in &s.avisos {
            self.front.log(format!("  ⚠ {a}")).await;
        }
        s
    }

    /// Al empezar una orden: fija los hooks de la orden, `SessionStart` (si es la primera de la conversación
    /// en este proceso), `FileChanged` y `UserPromptSubmit`. El contexto que devuelvan va con la orden.
    /// `Some(razón)` = un hook bloqueó la orden: no llega al modelo.
    pub(crate) async fn hooks_al_empezar(&mut self, hooks: &Hooks, task: &str, msg: &mut String) -> Option<String> {
        VIGENTES.lock().unwrap().get_or_insert_with(HashMap::new).insert(self.root.clone(), hooks.clone());
        for a in &hooks.avisos {
            self.front.log(format!("· ⚠ {a}")).await;
        }
        let mut contexto = Vec::new();
        let clave = format!("{}#{}", self.root.display(), self.session.as_ref().map(|s| s.id).unwrap_or(-1));
        let primera = ARRANCADAS.lock().unwrap().get_or_insert_with(HashSet::new).insert(clave);
        if primera {
            let previas = self.history.iter().any(|m| m.role == "user");
            let source = if previas { "resume" } else { "startup" };
            let s = self.hooks_disparar(hooks, Event::SessionStart, source, json!({"source": source, "model": self.engine.name()})).await;
            contexto.extend(s.contexto);
        }
        self.hooks_archivos(hooks).await;
        let s = self.hooks_disparar(hooks, Event::UserPromptSubmit, "", json!({"prompt": task})).await;
        if let Some(r) = s.bloqueo {
            self.front.log(crate::tr!("  🚫 la orden la bloqueó un hook: {r}", "  🚫 a hook blocked the order: {r}")).await;
            return Some(r);
        }
        contexto.extend(s.contexto);
        if !contexto.is_empty() {
            msg.push_str(&crate::tr!("\n\n(contexto que añadieron los hooks del usuario:)\n{}", "\n\n(context added by the user's hooks:)\n{}", contexto.join("\n")));
        }
        None
    }

    /// `FileChanged`: los archivos que nombran sus `matcher` (relativos al proyecto) contra la última foto.
    /// La primera vez sólo se toma la foto (no hay cambio que contar).
    pub(crate) async fn hooks_archivos(&self, hooks: &Hooks) {
        if !hooks.tiene(Event::FileChanged) {
            return;
        }
        let nombres: HashSet<String> = hooks.list.iter().filter(|h| h.event == Event::FileChanged).flat_map(|h| h.matcher.split('|').map(|n| n.trim().to_string()).collect::<Vec<_>>()).filter(|n| !n.is_empty() && n != "*").collect();
        let mut cambios = Vec::new();
        {
            let mut g = FOTOS.lock().unwrap();
            let fotos = g.get_or_insert_with(HashMap::new);
            for n in nombres {
                let ruta = self.root.join(&n);
                let ahora = std::fs::metadata(&ruta).ok().map(|m| (m.modified().unwrap_or(SystemTime::UNIX_EPOCH), m.len()));
                let clave = (self.root.clone(), ruta.clone());
                match fotos.insert(clave, ahora) {
                    None => {}
                    Some(antes) if antes == ahora => {}
                    Some(antes) => {
                        let ev = match (antes, ahora) {
                            (None, Some(_)) => "add",
                            (Some(_), None) => "unlink",
                            _ => "change",
                        };
                        cambios.push((ruta, n, ev));
                    }
                }
            }
        }
        for (ruta, n, ev) in cambios {
            let base = Path::new(&n).file_name().map(|b| b.to_string_lossy().into_owned()).unwrap_or(n.clone());
            // el matcher puede nombrar la ruta relativa o sólo el nombre: se prueban los dos
            let objetivo = if hooks.list.iter().any(|h| h.event == Event::FileChanged && crate::hooks::casa(Event::FileChanged, &h.matcher, &n)) { n.clone() } else { base };
            self.hooks_disparar(hooks, Event::FileChanged, &objetivo, json!({"file_path": ruta, "event": ev})).await;
        }
    }

    /// `PermissionRequest` antes de preguntar UNA llamada: `Some(None)` = un hook la permitió, `Some(Some(r))`
    /// = un hook la negó, `None` = la decide una persona (y entonces se avisa con `Notification`).
    pub(crate) async fn permiso_por_hooks(&self, hooks: &Hooks, tool: &str, args: &Value, desc: &str) -> Option<Option<String>> {
        let r = self.permiso_de_un_hook(hooks, tool, args, desc).await;
        if r.is_none() {
            self.hooks_disparar(hooks, Event::Notification, "permission_prompt", json!({"message": desc, "title": crate::i18n::t("GOTV pide permiso", "GOTV asks for permission"), "notification_type": "permission_prompt"})).await;
        }
        r
    }

    async fn permiso_de_un_hook(&self, hooks: &Hooks, tool: &str, args: &Value, desc: &str) -> Option<Option<String>> {
        if !hooks.tiene(Event::PermissionRequest) {
            return None;
        }
        let a = crate::hooks::alias(tool);
        let nombre_cc = if a.is_empty() { tool } else { a };
        let s = self.hooks_disparar(hooks, Event::PermissionRequest, tool, json!({"tool_name": nombre_cc, "gotv_tool_name": tool, "tool_input": args, "gotv_description": desc})).await;
        match s.permiso {
            Some(Permiso::Negar(r)) => {
                self.front.log(crate::tr!("  🚫 un hook negó el permiso: {r}", "  🚫 a hook denied the permission: {r}")).await;
                Some(Some(crate::tr!("un hook del usuario lo negó: {r}", "a user hook denied it: {r}")))
            }
            Some(Permiso::Permitir) if solo_una_persona(desc) => {
                self.front.log(crate::i18n::t("  · un hook quiso permitirlo, pero esto lo contesta una persona (irreversible o del sistema)", "  · a hook wanted to allow it, but a person answers this (irreversible or system)").into()).await;
                None
            }
            Some(Permiso::Permitir) => {
                self.front.log(crate::i18n::t("  ✓ permitido por un hook del usuario", "  ✓ allowed by a user hook").into()).await;
                Some(None)
            }
            None => None,
        }
    }

    /// El lote de permisos (P10): lo que deciden los hooks sale del lote; el resto se pregunta junto.
    pub(crate) async fn permisos_en_lote_por_hooks(&self, calls: &[ToolCall], asks: &mut Vec<(usize, String)>, pre: &mut [Option<Option<String>>]) {
        // una sola pregunta no es un lote: la contesta la puerta de una en una (y su hook corre allí, una vez)
        if asks.len() < 2 {
            return;
        }
        let hooks = self.hooks_vigentes();
        if !hooks.tiene(Event::PermissionRequest) {
            return;
        }
        let mut quedan = Vec::new();
        for (i, d) in asks.drain(..) {
            match self.permiso_de_un_hook(&hooks, &calls[i].name, &calls[i].arguments, &d).await {
                Some(decision) => pre[i] = Some(decision),
                None => quedan.push((i, d)),
            }
        }
        *asks = quedan;
    }

    /// `Notification` antes de preguntar el lote a una persona.
    pub(crate) async fn avisar_pregunta(&self, desc: &str) {
        let hooks = self.hooks_vigentes();
        self.hooks_disparar(&hooks, Event::Notification, "permission_prompt", json!({"message": desc, "title": crate::i18n::t("GOTV pide permiso", "GOTV asks for permission"), "notification_type": "permission_prompt"})).await;
    }

    /// `PreCompact`: `Err(())` = un hook la bloqueó (no se compacta); `Ok(texto)` = instrucciones para el
    /// resumen (vacío si no hay).
    pub(crate) async fn hook_antes_de_compactar(&self, manual: bool) -> Result<String, ()> {
        let hooks = self.hooks_vigentes();
        let trigger = if manual { "manual" } else { "auto" };
        let s = self.hooks_disparar(&hooks, Event::PreCompact, trigger, json!({"trigger": trigger, "custom_instructions": Value::Null})).await;
        if let Some(r) = s.bloqueo {
            self.front.log(crate::tr!("· un hook bloqueó la compactación: {r}", "· a hook blocked the compaction: {r}")).await;
            return Err(());
        }
        Ok(s.instrucciones.join("\n"))
    }

    /// `PostCompact` y `SessionStart` (`compact`). Devuelve el contexto de este último (va con la memoria).
    pub(crate) async fn hook_tras_compactar(&self, manual: bool, resumen: &str) -> String {
        let hooks = self.hooks_vigentes();
        let trigger = if manual { "manual" } else { "auto" };
        self.hooks_disparar(&hooks, Event::PostCompact, trigger, json!({"trigger": trigger, "compact_summary": resumen})).await;
        let s = self.hooks_disparar(&hooks, Event::SessionStart, "compact", json!({"source": "compact", "model": self.engine.name()})).await;
        if s.contexto.is_empty() {
            String::new()
        } else {
            crate::tr!("\n\n(contexto que añadieron los hooks del usuario:)\n{}", "\n\n(context added by the user's hooks:)\n{}", s.contexto.join("\n"))
        }
    }

    pub(crate) async fn hook_subagente_empieza(&self, hooks: &Hooks, id: &str) {
        self.hooks_disparar(hooks, Event::SubagentStart, "task", json!({"agent_id": id, "agent_type": "task"})).await;
    }

    /// `SubagentStop`: `Some(razón)` = un hook pide que el subagente siga.
    pub(crate) async fn hook_subagente_termina(&self, hooks: &Hooks, id: &str, informe: &str, seguidas: u32) -> Option<String> {
        let s = self.hooks_disparar(hooks, Event::SubagentStop, "task", json!({"agent_id": id, "agent_type": "task", "stop_hook_active": seguidas > 0, "last_assistant_message": informe})).await;
        s.bloqueo
    }

    /// `Notification` (`agent_completed`) al terminar la orden.
    pub(crate) async fn hook_orden_terminada(&self, hooks: &Hooks, r: &anyhow::Result<String>) {
        let mensaje = match r {
            Ok(a) => a.chars().take(300).collect::<String>(),
            Err(e) => format!("{e:#}"),
        };
        self.hooks_disparar(hooks, Event::Notification, "agent_completed", json!({"message": mensaje, "title": crate::i18n::t("GOTV terminó", "GOTV finished"), "notification_type": "agent_completed"})).await;
    }

    /// `SessionEnd`: lo llama quien cierra la conversación (la CLI al salir, el editor al cerrar el chat).
    /// `razon`: `clear`, `logout`, `prompt_input_exit` u `other` (los de Claude Code).
    pub async fn cerrar_sesion(&mut self, razon: &str) {
        let hooks = self.hooks_vigentes();
        self.hooks_disparar(&hooks, Event::SessionEnd, razon, json!({"reason": razon})).await;
        let clave = format!("{}#{}", self.root.display(), self.session.as_ref().map(|s| s.id).unwrap_or(-1));
        if let Some(a) = ARRANCADAS.lock().unwrap().as_mut() {
            a.remove(&clave);
        }
        if let Some(v) = VIGENTES.lock().unwrap().as_mut() {
            v.remove(&self.root);
        }
    }
}

#[cfg(test)]
pub(crate) mod tests;
