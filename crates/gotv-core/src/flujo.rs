//! F10.2 · `gotv -p`: una orden sin nadie a quien preguntar (un guion, un CI, las tareas programadas de F2).
//!
//! - **Nadie responde:** un permiso que habría que preguntar se NIEGA y se registra; con `detener` (lo que
//!   usan `-p` y las tareas programadas) además se corta la orden ahí (si no, el modelo intenta rodearlo). Lo
//!   que pide `pedir_permiso` (F14.4, la pregunta con 🔑) cuenta igual.
//! - **Códigos de salida:** 0 hecho · 1 falló · 2 falta un permiso · 3 interrumpido.
//! - El resultado es un [`Resultado`] de Rust. Decisión del Portador (2026-11-17): lo que GOTV emite o guarda
//!   es sólo Rust, sin JSON propio; por eso `--salida json|flujo` del plan NO existe: la salida estándar lleva
//!   la respuesta y el registro va a stderr.
use crate::permissions::AskAnswer;
use async_trait::async_trait;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Estado {
    Hecho,
    Fallo,
    FaltaPermiso,
    Interrumpido,
}

impl Estado {
    pub fn codigo(self) -> i32 {
        match self {
            Estado::Hecho => 0,
            Estado::Fallo => 1,
            Estado::FaltaPermiso => 2,
            Estado::Interrumpido => 3,
        }
    }
    pub fn de_codigo(c: i32) -> Estado {
        match c {
            0 => Estado::Hecho,
            2 => Estado::FaltaPermiso,
            3 => Estado::Interrumpido,
            _ => Estado::Fallo,
        }
    }
}

/// Lo que dejó una orden sin ventana.
#[derive(Debug, Clone, PartialEq)]
pub struct Resultado {
    pub estado: Estado,
    pub respuesta: Option<String>,
    pub error: Option<String>,
    /// los permisos que se negaron por no haber a quién preguntar
    pub negados: Vec<String>,
    pub segundos: f32,
}

impl Resultado {
    pub fn codigo(&self) -> i32 {
        self.estado.codigo()
    }
}

/// Adónde van las líneas del registro mientras corre (stderr en `-p`; nada en las programadas).
pub type Escritor = Box<dyn Fn(&str) + Send + Sync>;

/// La cara del agente cuando nadie mira.
pub struct SinVentana {
    escribir: Option<Escritor>,
    /// cortar la orden en el primer permiso que falte
    detener: Option<Arc<AtomicBool>>,
    negados: Mutex<Vec<String>>,
    /// el registro en texto (lo que se habría visto en la ventana)
    pub registro: Mutex<Vec<String>>,
}

impl SinVentana {
    pub fn new(escribir: Option<Escritor>, detener: Option<Arc<AtomicBool>>) -> Self {
        Self { escribir, detener, negados: Default::default(), registro: Default::default() }
    }

    pub fn negados(&self) -> Vec<String> {
        self.negados.lock().unwrap().clone()
    }

    fn anotar(&self, linea: String) {
        if let Some(e) = &self.escribir {
            e(&linea);
        }
        self.registro.lock().unwrap().push(linea);
    }

    fn negar(&self, que: &str) {
        self.negados.lock().unwrap().push(que.to_string());
        self.anotar(crate::tr!("🚫 sin permiso (nadie a quien preguntar): {que}", "🚫 no permission (nobody to ask): {que}"));
        if let Some(c) = &self.detener {
            c.store(true, Ordering::Relaxed);
        }
    }

    /// El resultado de una corrida.
    pub fn resultado(&self, r: &anyhow::Result<String>, segundos: f32) -> Resultado {
        let negados = self.negados();
        let estado = match r {
            _ if !negados.is_empty() => Estado::FaltaPermiso,
            Err(_) => Estado::Fallo,
            Ok(a) if a == crate::agent::interrupted() => Estado::Interrumpido,
            Ok(_) => Estado::Hecho,
        };
        Resultado {
            estado,
            respuesta: r.as_ref().ok().cloned(),
            error: r.as_ref().err().map(|e| format!("{e:#}")),
            negados,
            segundos,
        }
    }
}

#[async_trait]
impl crate::Frontend for SinVentana {
    async fn log(&self, line: String) {
        self.anotar(line);
    }

    async fn ask(&self, desc: &str) -> AskAnswer {
        self.negar(desc);
        AskAnswer::No
    }

    /// Nadie elige: `None`. Un permiso pedido con `pedir_permiso` (empieza por 🔑) cuenta como negado.
    async fn choose(&self, question: &str, _options: &[String]) -> Option<String> {
        if question.starts_with('🔑') {
            self.negar(question);
        } else {
            self.anotar(crate::tr!("? {question} (sin respuesta: decide el agente)", "? {question} (no answer: the agent decides)"));
        }
        None
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::permissions::{Mode, Permissions};
    use gotv_engine::{Capabilities, ChatRequest, ChatResponse, Engine, ToolCall};
    use serde_json::Value;

    /// Motor de guion para los agentes de MIS pruebas (F10.2, F2, F3): respuestas preparadas, sin modelo.
    pub(crate) struct Guion {
        pub respuestas: Mutex<Vec<ChatResponse>>,
        pub vistos: Mutex<Vec<ChatRequest>>,
        /// cada respuesta tarda esto (un paso «largo» del modelo)
        pub demora: std::time::Duration,
    }

    impl Guion {
        pub(crate) fn de(respuestas: Vec<ChatResponse>) -> Self {
            Self { respuestas: Mutex::new(respuestas), vistos: Mutex::new(vec![]), demora: std::time::Duration::ZERO }
        }
    }

    #[async_trait]
    impl Engine for Guion {
        fn name(&self) -> String {
            "guion".into()
        }
        fn capabilities(&self) -> Capabilities {
            Capabilities { text: true, vision: false, audio_in: false }
        }
        async fn start(&mut self) -> anyhow::Result<()> {
            Ok(())
        }
        async fn complete(&self, req: ChatRequest) -> anyhow::Result<ChatResponse> {
            self.vistos.lock().unwrap().push(req);
            tokio::time::sleep(self.demora).await;
            let mut r = self.respuestas.lock().unwrap();
            Ok(if r.is_empty() { texto("fin") } else { r.remove(0) })
        }
        async fn stop(&mut self) -> anyhow::Result<()> {
            Ok(())
        }
    }

    pub(crate) fn texto(t: &str) -> ChatResponse {
        ChatResponse { content: t.into(), finish_reason: "stop".into(), ..Default::default() }
    }

    /// Una respuesta del modelo que llama herramientas (los argumentos van como los manda el modelo).
    pub(crate) fn llamadas(cs: &[(&str, Value)]) -> ChatResponse {
        ChatResponse {
            finish_reason: "tool_calls".into(),
            tool_calls: cs.iter().enumerate().map(|(i, (n, a))| ToolCall { id: format!("c{i}"), name: n.to_string(), arguments: a.clone() }).collect(),
            ..Default::default()
        }
    }

    /// Un agente con el motor de guion y la cara dada, sobre `raiz`. Si `Agent` gana un campo, se añade aquí
    /// (como en `agent.rs::tests::agent` y en `gotv-cli::armar_agente`).
    pub(crate) fn agente<F: crate::Frontend>(raiz: &std::path::Path, modo: Mode, respuestas: Vec<ChatResponse>, front: F) -> crate::Agent<Guion, F> {
        crate::Agent {
            engine: Guion::de(respuestas),
            perms: Permissions::new(modo, raiz, vec![], vec![], vec![]),
            checkpoint: None,
            root: raiz.to_path_buf(),
            max_steps: 8,
            max_tokens: 512,
            sampling: gotv_engine::Sampling::GREEDY,
            context_budget: 32768,
            front,
            history: vec![],
            session: None,
            cancel: Default::default(),
            mcp: None,
            attachments: Vec::new(),
            user_media: Vec::new(),
            chat_only: false,
            embedder: None,
            lsp: None,
            computer_use: false,
            procs: Default::default(),
            tools_loaded: Default::default(),
            bases_nube: Default::default(),
            guardia: Default::default(),
            perfil: Default::default(),
            pro: Default::default(),
            motor_dormido: false,
            visible: None,
            estado_de: None,
            calibre: None,
            recorto_ubicaciones: false,
            tope_paso: 0,
        }
    }

    /// Un escritor que guarda las líneas (para leerlas en la prueba).
    pub(crate) fn bufer() -> (Escritor, Arc<Mutex<Vec<String>>>) {
        let b: Arc<Mutex<Vec<String>>> = Default::default();
        let b2 = b.clone();
        (Box::new(move |l: &str| b2.lock().unwrap().push(l.to_string())), b)
    }

    #[tokio::test]
    async fn hecho_da_codigo_0_y_el_registro_ve_la_herramienta() {
        let raiz = crate::CarpetaPrueba::nueva("gotv-flujo-0");
        std::fs::write(raiz.join("nota.txt"), "hola\n").unwrap();
        let (esc, lineas) = bufer();
        let mut a = agente(&raiz, Mode::Default, vec![llamadas(&[("read_file", serde_json::json!({"path": "nota.txt"}))]), texto("La nota dice hola.")], SinVentana::new(Some(esc), None));
        let r = a.run("lee nota.txt").await;
        let res = a.front.resultado(&r, 0.1);
        assert_eq!((res.codigo(), res.estado), (0, Estado::Hecho));
        assert_eq!(res.respuesta.as_deref(), Some("La nota dice hola."));
        assert!(lineas.lock().unwrap().iter().any(|l| l.contains("read_file")), "{:?}", lineas.lock().unwrap());
    }

    #[tokio::test]
    async fn falta_permiso_da_2_y_corta_la_orden() {
        let raiz = crate::CarpetaPrueba::nueva("gotv-flujo-2");
        let detener = Arc::new(AtomicBool::new(false));
        let mut a = agente(
            &raiz,
            Mode::Default,
            vec![llamadas(&[("bash", serde_json::json!({"cmd": "git push origin main"}))]), texto("no debería llegar")],
            SinVentana::new(None, Some(detener.clone())),
        );
        a.cancel = detener.clone();
        let r = a.run("sube los cambios").await;
        let res = a.front.resultado(&r, 0.0);
        assert_eq!(res.codigo(), 2);
        assert!(res.negados[0].contains("git push"), "{res:?}");
        // se cortó ahí: el modelo no recibió otra vuelta para «rodear» el permiso
        assert_eq!(a.engine.vistos.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn pedir_permiso_sin_nadie_tambien_es_falta_de_permiso() {
        let raiz = crate::CarpetaPrueba::nueva("gotv-flujo-pedir");
        let mut a = agente(
            &raiz,
            Mode::Default,
            vec![llamadas(&[("pedir_permiso", serde_json::json!({"accion": "red", "objetivo": "registry.npmjs.org", "motivo": "instalar"}))]), texto("sigo sin red")],
            SinVentana::new(None, None),
        );
        let r = a.run("instala").await;
        let res = a.front.resultado(&r, 0.0);
        assert_eq!(res.codigo(), 2, "{res:?}");
        assert!(res.negados[0].contains("registry.npmjs.org"));
    }

    #[test]
    fn codigos_de_fallo_e_interrumpido() {
        let s = SinVentana::new(None, None);
        assert_eq!(s.resultado(&Err(anyhow::anyhow!("motor caído")), 0.0).codigo(), 1);
        assert_eq!(s.resultado(&Ok(crate::agent::interrupted().into()), 0.0).codigo(), 3);
        assert_eq!(s.resultado(&Ok("listo".into()), 0.0).codigo(), 0);
        for e in [Estado::Hecho, Estado::Fallo, Estado::FaltaPermiso, Estado::Interrumpido] {
            assert_eq!(Estado::de_codigo(e.codigo()), e);
        }
    }
}
