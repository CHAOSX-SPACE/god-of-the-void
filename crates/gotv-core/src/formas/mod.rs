//! F12 · formas de trabajar: lo que el Portador ya usa en Claude Code para dirigir el trabajo, sin que
//! ninguna gaste contexto cuando no se usa.
//!
//! | Forma | Módulo | Cuándo cuesta contexto |
//! |---|---|---|
//! | Reglas por ruta (`.gotv/reglas/`, `.claude/rules/`) | [`reglas`] | sólo al tocar un archivo que casa, una vez, como aviso |
//! | `/aparte` (pregunta al margen) | [`aparte`] | nunca en el historial: es una COPIA |
//! | `/objetivo` (seguir hasta cumplir) | [`objetivo`] | sólo lo que el propio trabajo gasta |
//! | `/aprender` · `/entrevista` (skills incluidas) | [`aprender`], [`incluidas`] | sólo cuando se piden |
//!
//! Las órdenes de barra las atiende [`orden`] igual en la CLI (`gotv -p`), la TUI y la ventana (ésta,
//! para que cada ronda de `/objetivo` sea un turno suyo, usa [`objetivo::Objetivo`] directamente).
use crate::frontend::Frontend;
use crate::Agent;
use anyhow::Result;
use gotv_engine::{ChatResponse, Engine, Message};

pub mod aparte;
pub mod aprender;
pub mod incluidas;
pub mod objetivo;
pub mod reglas;

/// Las formas que se piden con una orden de barra.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Forma {
    Aparte,
    Objetivo,
    Aprender,
    Entrevista,
    Reglas,
    /// F9 · el agente principal en su propia rama (`agent/paralelo.rs`)
    Rama,
}

/// `/nombre` → su forma (en español y en inglés).
pub fn forma(nombre: &str) -> Option<Forma> {
    // juez-idioma: datos (los nombres de las órdenes en los dos idiomas)
    Some(match nombre.trim().to_lowercase().as_str() {
        "aparte" | "aside" | "btw" => Forma::Aparte,
        "objetivo" | "goal" => Forma::Objetivo,
        "aprender" | "learn" => Forma::Aprender,
        "entrevista" | "interview" => Forma::Entrevista,
        "reglas" | "rules" => Forma::Reglas,
        "rama" | "branch" => Forma::Rama,
        _ => return None,
    })
}

/// ¿`nombre` es una de estas órdenes? (la ventana no las lista además como skills)
pub fn es_comando(nombre: &str) -> bool {
    forma(nombre).is_some()
}

/// Las órdenes para la ayuda y el autocompletado: `(nombre, qué hace)` en el idioma activo.
pub fn comandos() -> Vec<(&'static str, &'static str)> {
    vec![
        (crate::i18n::t("objetivo", "goal"), crate::i18n::t("sigue hasta cumplir una condición: /objetivo [--turnos N] [--minutos M] todas las pruebas pasan", "keeps going until a condition holds: /goal [--turns N] [--minutes M] all tests pass")),
        (crate::i18n::t("aparte", "aside"), crate::i18n::t("pregunta al margen: no entra en la conversación ni rompe su memoria rápida", "side question: it does not enter the conversation nor break its fast memory")),
        (crate::i18n::t("aprender", "learn"), crate::i18n::t("propone una regla o skill a partir de tus correcciones; sólo se escribe si la apruebas", "proposes a rule or skill from your corrections; it is written only if you approve it")),
        (crate::i18n::t("entrevista", "interview"), crate::i18n::t("te entrevista para precisar una tarea: una pregunta a la vez, con respuesta recomendada", "interviews you to pin down a task: one question at a time, with a recommended answer")),
        (crate::i18n::t("reglas", "rules"), crate::i18n::t("las reglas por ruta del proyecto (.gotv/reglas, .claude/rules)", "the project's path rules (.gotv/reglas, .claude/rules)")),
        (crate::i18n::t("rama", "branch"), crate::i18n::t("trabaja en tu propia rama git sin tocar tu árbol: /rama nueva [tema] · /rama cerrar · /rama fusionar", "work on your own git branch without touching your tree: /branch new [topic] · /branch close · /branch merge")),
    ]
}

/// La línea de ayuda de estas órdenes (para `/help`).
pub fn ayuda() -> String {
    comandos().iter().map(|(n, d)| format!("  /{n:<24} {d}")).collect::<Vec<_>>().join("\n")
}

/// Atiende `texto` si es una de estas órdenes de barra; `None` si no lo es (sigue su camino de siempre).
/// `parar` dice si el Portador escribió algo (lo consulta `/objetivo` entre ronda y ronda).
pub async fn orden<E: Engine, F: Frontend, P: FnMut() -> bool + Send>(agent: &mut Agent<E, F>, texto: &str, parar: P) -> Option<Result<String>> {
    let resto = texto.trim().strip_prefix('/')?;
    let (nombre, arg) = resto.split_once(char::is_whitespace).unwrap_or((resto, ""));
    let arg = arg.trim();
    Some(match forma(nombre)? {
        Forma::Aparte => aparte::responder(agent, arg).await,
        Forma::Objetivo => objetivo::perseguir(agent, arg, parar).await,
        Forma::Aprender => aprender::aprender(agent).await,
        Forma::Reglas => Ok(reglas::listado(&agent.root)),
        Forma::Rama => agent.rama_propia(arg).await,
        Forma::Entrevista => {
            agent.visible = Some(texto.trim().to_string());
            agent.run(&orden_de_entrevista(arg)).await
        }
    })
}

/// La orden de `/entrevista <tema>`: el tema y la skill entera (como cualquier skill elegida a mano).
pub fn orden_de_entrevista(tema: &str) -> String {
    let tema = if tema.trim().is_empty() { crate::i18n::t("Entrevístame para precisar lo que quiero hacer en este proyecto.", "Interview me to pin down what I want to do in this project.").to_string() } else { tema.trim().to_string() };
    let cuerpo = incluidas::cuerpo("entrevista").unwrap_or("");
    format!("{tema}\n\n{}\n{cuerpo}", crate::i18n::t("[skill «entrevista» elegida por el usuario]", "[skill «entrevista» chosen by the user]"))
}

/// Una generación sobre una COPIA de la conversación con `pedido` al final, con el mismo principio que un
/// paso del bucle (la memoria rápida del motor sigue valiendo) y sin la caché en disco de la conversación.
/// Ni `history` ni la sesión guardada cambian: el agente se presta sólo para leer.
pub(crate) async fn generar_aparte<E: Engine, F: Frontend>(agent: &Agent<E, F>, pedido: String, max_tokens: u32) -> Result<ChatResponse> {
    let mut copia = if agent.history.is_empty() { vec![Message::system(agent.system_prompt())] } else { agent.history.clone() };
    copia.push(Message::user(pedido));
    let (req, enmascarados) = agent.peticion_aparte(copia, max_tokens);
    agent.avisar_secretos(enmascarados).await;
    let cancel = agent.cancel.clone();
    cancel.store(false, std::sync::atomic::Ordering::Relaxed);
    let parado = async {
        while !cancel.load(std::sync::atomic::Ordering::Relaxed) {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    };
    tokio::select! {
        r = agent.engine.complete(req) => r,
        _ = parado => gotv_engine::bail_tr!("detenido por el usuario", "stopped by the user"),
        _ = tokio::time::sleep(std::time::Duration::from_secs(APARTE_TOPE_S)) => gotv_engine::bail_tr!("el modelo no respondió en {} s", "the model did not answer within {} s", APARTE_TOPE_S),
    }
}

/// Lo más que espera una generación aparte.
const APARTE_TOPE_S: u64 = 300;

/// El texto sin el razonamiento `<think>…</think>`.
pub(crate) fn sin_pensamiento(s: &str) -> String {
    let mut t = s.to_string();
    while let (Some(a), Some(b)) = (t.find("<think>"), t.find("</think>")) {
        if b < a {
            break;
        }
        t.replace_range(a..b + "</think>".len(), "");
    }
    t.trim().to_string()
}

/// El primer objeto JSON de una respuesta (los modelos lo envuelven en ```json … ``` o en una frase).
pub(crate) fn objeto_json(respuesta: &str) -> Option<serde_json::Value> {
    let r = sin_pensamiento(respuesta);
    let (i, f) = (r.find('{')?, r.rfind('}')?);
    serde_json::from_str(r.get(i..=f)?).ok().filter(|v: &serde_json::Value| v.is_object())
}

/// Para las pruebas de las formas: un motor de guion (el bucle sin modelo) y un agente sobre una carpeta
/// que se borra sola. Las generaciones APARTE se contestan de su propia cola (`aparte`), así un empujón
/// del bucle no se come la respuesta del evaluador.
#[cfg(test)]
pub(crate) mod prueba {
    use crate::checkpoint::Checkpoint;
    use crate::permissions::{AskAnswer, Mode, Permissions};
    use crate::Agent;
    use anyhow::Result;
    use async_trait::async_trait;
    use gotv_engine::{Capabilities, ChatRequest, ChatResponse, Engine, ToolCall};
    use std::sync::Mutex;

    pub struct Guion {
        pub turnos: Mutex<Vec<ChatResponse>>,
        pub aparte: Mutex<Vec<String>>,
        pub visto: Mutex<Vec<ChatRequest>>,
    }

    /// ¿Es una generación aparte de las formas? (pregunta al margen, evaluador o `/aprender`)
    pub fn es_aparte(req: &ChatRequest) -> bool {
        let u = req.messages.last().map(|m| m.content.as_str()).unwrap_or("");
        ["(Pregunta al margen:", "(Side question:", "GOTV · comprobación del objetivo", "GOTV · goal check", "# Aprender de las correcciones", "# Learning from corrections"].iter().any(|m| u.contains(m))
    }

    #[async_trait]
    impl Engine for Guion {
        fn name(&self) -> String {
            "guion".into()
        }
        fn capabilities(&self) -> Capabilities {
            Capabilities { text: true, vision: false, audio_in: false }
        }
        async fn start(&mut self) -> Result<()> {
            Ok(())
        }
        async fn complete(&self, req: ChatRequest) -> Result<ChatResponse> {
            let aparte = es_aparte(&req);
            // el pedido de resumen o de memoria al compactar no gasta el guion
            // (el resumen aparte va sin herramientas; la memoria dentro de la conversación empieza así)
            let compacta = req.tools.is_none() || req.messages.last().is_some_and(|m| m.content.starts_with("GOTV: el contexto") || m.content.starts_with("GOTV: the context"));
            self.visto.lock().unwrap().push(req);
            if aparte {
                let mut a = self.aparte.lock().unwrap();
                return Ok(texto(&if a.is_empty() { String::new() } else { a.remove(0) }));
            }
            if compacta {
                return Ok(texto("RESUMEN: lo hecho\nSIGUIENTE: seguir"));
            }
            let mut t = self.turnos.lock().unwrap();
            Ok(if t.is_empty() { texto("fin") } else { t.remove(0) })
        }
        async fn stop(&mut self) -> Result<()> {
            Ok(())
        }
    }

    /// Frontend de prueba: guarda lo que se le dice y contesta `respuesta` a los permisos.
    #[derive(Default)]
    pub struct Mudo {
        pub lineas: Mutex<Vec<String>>,
        pub preguntas: Mutex<Vec<String>>,
        pub respuesta: Mutex<Option<AskAnswer>>,
    }

    #[async_trait]
    impl crate::Frontend for Mudo {
        async fn log(&self, l: String) {
            self.lineas.lock().unwrap().push(l);
        }
        async fn ask(&self, d: &str) -> AskAnswer {
            self.preguntas.lock().unwrap().push(d.to_string());
            self.respuesta.lock().unwrap().unwrap_or(AskAnswer::No)
        }
    }

    pub fn texto(t: &str) -> ChatResponse {
        ChatResponse { content: t.into(), finish_reason: "stop".into(), ..Default::default() }
    }

    pub fn llamadas(cs: &[(&str, serde_json::Value)]) -> ChatResponse {
        ChatResponse {
            finish_reason: "tool_calls".into(),
            tool_calls: cs.iter().enumerate().map(|(i, (n, a))| ToolCall { id: format!("c{i}"), name: n.to_string(), arguments: a.clone() }).collect(),
            ..Default::default()
        }
    }

    pub struct ConCarpeta {
        pub agente: Agent<Guion, Mudo>,
        pub _raiz: crate::CarpetaPrueba,
    }

    pub fn agente(tag: &str, modo: Mode, turnos: Vec<ChatResponse>, aparte: Vec<&str>) -> ConCarpeta {
        let raiz = crate::CarpetaPrueba::nueva(&format!("gotv-formas-{tag}"));
        let root = raiz.canonicalize().unwrap();
        let agente = Agent {
            engine: Guion { turnos: Mutex::new(turnos), aparte: Mutex::new(aparte.into_iter().map(String::from).collect()), visto: Mutex::new(vec![]) },
            perms: Permissions::new(modo, &root, vec![], vec![], vec![]),
            checkpoint: Some(Checkpoint::new(&root)),
            root,
            max_steps: 8,
            max_tokens: 512,
            sampling: gotv_engine::Sampling::GREEDY,
            context_budget: 32768,
            front: Mudo::default(),
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
        };
        ConCarpeta { agente, _raiz: raiz }
    }

    /// La conversación como texto (para comparar «idéntica» byte a byte).
    pub fn foto(a: &Agent<Guion, Mudo>) -> String {
        serde_json::to_string(&a.history).unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn las_ordenes_se_reconocen_en_los_dos_idiomas() {
        assert_eq!(forma("objetivo"), Some(Forma::Objetivo));
        assert_eq!(forma("GOAL"), Some(Forma::Objetivo));
        assert_eq!(forma("btw"), Some(Forma::Aparte));
        assert_eq!(forma("learn"), Some(Forma::Aprender));
        assert_eq!(forma("compact"), None);
        assert_eq!(comandos().len(), 6);
        assert_eq!(forma("branch"), Some(Forma::Rama));
        assert!(objeto_json("<think>{x}</think>```json\n{\"a\": 1}\n```").is_some_and(|v| v["a"] == 1));
        assert!(objeto_json("nada").is_none());
    }

    #[tokio::test]
    async fn una_orden_que_no_es_de_estas_sigue_su_camino() {
        let mut c = prueba::agente("no-es", crate::permissions::Mode::Default, vec![], vec![]);
        assert!(orden(&mut c.agente, "/compact", || false).await.is_none());
        assert!(orden(&mut c.agente, "objetivo sin barra", || false).await.is_none());
        let r = orden(&mut c.agente, "/reglas", || false).await.unwrap().unwrap();
        assert!(r.contains(".gotv/reglas"), "{r}");
    }

    /// La regla de la ficha: ninguna forma gasta contexto cuando no se usa. Se mide el prompt REAL que recibe
    /// el motor (sistema + herramientas + la orden) para la misma orden, sin y con: una regla por ruta, las
    /// dos skills incluidas y las órdenes nuevas. Debe ser idéntico byte a byte. Y lo que cuesta al usarse:
    /// el anuncio de la regla (una vez) y las dos líneas de más en `skill("?")`.
    #[tokio::test]
    async fn ninguna_forma_gasta_contexto_sin_usarse() {
        use prueba::{agente, llamadas, texto};
        let turno = || vec![llamadas(&[("read_file", serde_json::json!({"path": "main.rs"}))]), texto("listo")];
        let mut c = agente("contexto", crate::permissions::Mode::Default, [turno(), turno(), vec![llamadas(&[("read_file", serde_json::json!({"path": "db/schema.sql"}))]), texto("listo")]].concat(), vec![]);
        let root = c.agente.root.clone();
        std::fs::write(root.join("main.rs"), "fn main() {}\n").unwrap();
        std::fs::create_dir_all(root.join("db")).unwrap();
        std::fs::write(root.join("db/schema.sql"), "create table t (id int);\n").unwrap();
        let prompt = |a: &crate::Agent<prueba::Guion, prueba::Mudo>, i: usize| {
            let v = a.engine.visto.lock().unwrap();
            let r = &v[i];
            let orden = r.messages.iter().find(|m| m.role == "user").map(|m| m.content.clone()).unwrap_or_default();
            (r.messages[0].content.clone(), serde_json::to_string(&r.tools).unwrap(), orden)
        };
        let est = |(s, t, o): &(String, String, String)| (s.chars().count() + t.chars().count() + o.chars().count()) / 4;
        c.agente.run("lee main.rs y dime qué hace").await.unwrap();
        let sin = prompt(&c.agente, 0);
        // con: una regla por ruta, y las skills incluidas (ya viajan en el binario)
        std::fs::create_dir_all(root.join(".gotv/reglas")).unwrap();
        std::fs::write(root.join(".gotv/reglas/sql.md"), "---\nrutas: [\"**/*.sql\"]\n---\nLas migraciones van numeradas: 0001_x.sql. Nunca edites una aplicada.\n").unwrap();
        c.agente.clear();
        c.agente.run("lee main.rs y dime qué hace").await.unwrap();
        let con = prompt(&c.agente, 2);
        eprintln!("[F12 contexto] prompt sin formas ≈ {} tok · con regla por ruta y skills incluidas ≈ {} tok", est(&sin), est(&con));
        assert_eq!(sin.0, con.0, "el mensaje de sistema no cambia");
        assert_eq!(sin.1, con.1, "la lista de herramientas no cambia");
        assert_eq!(sin.2, con.2, "la orden no lleva nada de más");
        assert!(!c.agente.history.iter().any(|m| m.content.contains("regla del proyecto") || m.content.contains("project rule")), "sin tocar un .sql, la regla no aparece");
        // las órdenes con palabras de las skills incluidas no las sugieren
        for t in ["aprende cómo funciona el login y aprender a usarlo", "prepara la entrevista con el cliente"] {
            assert!(!crate::skills::note(t, &root).unwrap_or_default().contains("aprender") && !crate::skills::note(t, &root).unwrap_or_default().contains("entrevista"), "{t}");
        }
        // al usarse: tocar el .sql trae la regla UNA vez
        c.agente.run("lee db/schema.sql").await.unwrap();
        let anuncio = c.agente.history.iter().find(|m| m.es_aviso() && m.content.contains(".gotv/reglas/sql.md")).map(|m| m.content.clone()).expect("la regla se anunció");
        let lista = crate::skills::load(&root, "?").unwrap();
        let extra: usize = lista.lines().filter(|l| l.starts_with("- entrevista") || l.starts_with("- aprender")).map(|l| l.chars().count() + 1).sum();
        eprintln!("[F12 contexto] al usarse: anuncio de la regla ≈ {} tok (una vez) · skill(\"?\") +{} tok por las dos incluidas", anuncio.chars().count() / 4, extra / 4);
        assert!(anuncio.chars().count() / 4 < 120);
    }

    #[tokio::test]
    async fn entrevista_lleva_la_skill_y_lo_escrito_es_lo_visible() {
        let mut c = prueba::agente("entrevista", crate::permissions::Mode::Default, vec![prueba::texto("Resumen: el login usará SQLite, como ya hace el proyecto.")], vec![]);
        let r = orden(&mut c.agente, "/entrevista quiero un login", || false).await.unwrap().unwrap();
        assert!(r.contains("SQLite"), "{r}");
        let pedido = c.agente.engine.visto.lock().unwrap()[0].messages.iter().rev().find(|m| m.role == "user").unwrap().content.clone();
        assert!(pedido.starts_with("quiero un login") && pedido.contains("ask_user"), "{pedido}");
        let u = c.agente.history.iter().find(|m| m.role == "user").unwrap();
        assert_eq!(crate::mensajes::visible(u), "/entrevista quiero un login");
    }
}
