//! Pruebas de F10.1 con el agente de verdad y un motor de GUION (sin modelo): cada evento dispara una vez y
//! con su entrada. El motor y el frente de aquí los usan también las pruebas de ACP y del control remoto.
//! Todo este archivo es código de prueba (`#[cfg(test)]` en `hooks_eventos.rs`): el juez de idioma no lo mira.
use crate::agent::Agent;
use crate::checkpoint::Checkpoint;
use crate::frontend::Frontend;
use crate::hooks::tests::{hook_falso, volcadas};
use crate::permissions::{AskAnswer, Mode, Permissions};
use async_trait::async_trait;
use gotv_engine::{Capabilities, ChatRequest, ChatResponse, Engine, StreamEvent, ToolCall};
use serde_json::{json, Value};
use std::path::Path;
use std::sync::Mutex;

/// Motor de guion: devuelve respuestas preparadas (texto y llamadas) y guarda lo que recibió. Los pedidos de
/// resumen de la compactación se contestan aparte (no gastan el guion).
pub(crate) struct Guion {
    pub respuestas: Mutex<Vec<ChatResponse>>,
    pub vistos: Mutex<Vec<ChatRequest>>,
    /// espera antes de cada respuesta (para cancelar a mitad)
    pub demora: std::time::Duration,
}

impl Guion {
    pub fn nuevo(r: Vec<ChatResponse>) -> Self {
        Guion { respuestas: Mutex::new(r), vistos: Mutex::new(vec![]), demora: std::time::Duration::ZERO }
    }
}

fn es_resumen(req: &ChatRequest) -> bool {
    req.messages.iter().rev().take(2).any(|m| (m.content.contains("MEMORIA DE TRABAJO") || m.content.contains("WORKING MEMORY")) && (m.content.contains("SIGUIENTE:") || m.content.contains("NEXT:")))
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
        let resumen = es_resumen(&req);
        self.vistos.lock().unwrap().push(req);
        tokio::time::sleep(self.demora).await;
        if resumen {
            return Ok(texto("TAREA: la del usuario\nHECHO: x.txt\nPRUEBAS: sin correr\nREGLAS: -\nDECISIONES: -\nINTENTADO: -\nFALTA: nada\nSIGUIENTE: responder"));
        }
        let mut r = self.respuestas.lock().unwrap();
        Ok(if r.is_empty() { texto("fin") } else { r.remove(0) })
    }
    async fn complete_stream(&self, req: ChatRequest, on: &(dyn Fn(StreamEvent) + Send + Sync)) -> anyhow::Result<ChatResponse> {
        let r = self.complete(req).await?;
        if !r.content.is_empty() {
            on(StreamEvent::Text(r.content.clone()));
        }
        Ok(r)
    }
    async fn stop(&mut self) -> anyhow::Result<()> {
        Ok(())
    }
}

/// Frente de prueba: guarda el registro y las preguntas; contesta `respuesta` (sí por omisión).
#[derive(Default)]
pub(crate) struct Mudo {
    pub lineas: Mutex<Vec<String>>,
    pub preguntas: Mutex<Vec<String>>,
    pub respuesta: Mutex<Option<AskAnswer>>,
}

#[async_trait]
impl Frontend for Mudo {
    async fn log(&self, l: String) {
        self.lineas.lock().unwrap().push(l);
    }
    async fn ask(&self, d: &str) -> AskAnswer {
        self.preguntas.lock().unwrap().push(d.to_string());
        self.respuesta.lock().unwrap().unwrap_or(AskAnswer::Yes)
    }
}

pub(crate) fn texto(t: &str) -> ChatResponse {
    ChatResponse { content: t.into(), finish_reason: "stop".into(), ..Default::default() }
}

pub(crate) fn llamadas(cs: &[(&str, Value)]) -> ChatResponse {
    ChatResponse {
        finish_reason: "tool_calls".into(),
        tool_calls: cs.iter().enumerate().map(|(i, (n, a))| ToolCall { id: format!("c{i}-{n}"), name: n.to_string(), arguments: a.clone() }).collect(),
        ..Default::default()
    }
}

/// Un agente completo sobre `root` con el motor de guion y el frente que se le dé.
pub(crate) fn agente<F: Frontend>(root: &Path, modo: Mode, respuestas: Vec<ChatResponse>, front: F) -> Agent<Guion, F> {
    agente_con(root, modo, Guion::nuevo(respuestas), front)
}

/// El mismo agente de prueba con cualquier motor (el de guion, o el modelo REAL por `gotv serve`).
pub(crate) fn agente_con<E: gotv_engine::Engine, F: Frontend>(root: &Path, modo: Mode, engine: E, front: F) -> Agent<E, F> {
    Agent {
        engine,
        perms: Permissions::new(modo, root, vec![], vec![], vec![]),
        checkpoint: Some(Checkpoint::new(root)),
        root: root.to_path_buf(),
        max_steps: 12,
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

/// «Cada evento dispara una vez y con su entrada (un hook de prueba la vuelca a un archivo)»: una orden que
/// pide permiso, escribe `.env`, delega en un subagente, lee y termina; luego `/compact` y cerrar la sesión.
#[tokio::test]
async fn cada_evento_dispara_una_vez_con_su_entrada() {
    let d = crate::CarpetaPrueba::nueva("gotv-f10-eventos");
    let root = d.canonicalize().unwrap();
    let v = |n: &str| root.join(format!("vol-{n}.jsonl"));
    let h = |n: &str, salida: &str| json!([{"type": "command", "command": hook_falso(&v(n), 0, salida)}]);
    let hooks = json!({"hooks": {
        "SessionStart": [{"matcher": "startup", "hooks": h("ss-startup", "contexto de arranque")}, {"matcher": "compact", "hooks": h("ss-compact", "")}],
        "UserPromptSubmit": [{"hooks": h("ups", "contexto de la orden")}],
        "PreToolUse": [{"matcher": "Bash", "hooks": h("pre", "")}],
        "PermissionRequest": [{"matcher": "Bash", "hooks": h("perm", "")}],
        "PostToolUse": [{"matcher": "Bash", "hooks": h("post", "")}],
        "Notification": [{"matcher": "permission_prompt", "hooks": h("not-perm", "")}, {"matcher": "agent_completed", "hooks": h("not-fin", "")}],
        "FileChanged": [{"matcher": ".env", "hooks": h("fc", "")}],
        "SubagentStart": [{"hooks": h("sa-start", "")}],
        "SubagentStop": [{"hooks": h("sa-stop", "")}],
        "Stop": [{"hooks": h("stop", "")}],
        "PreCompact": [{"hooks": h("prec", "conserva los nombres de archivo")}],
        "PostCompact": [{"hooks": h("postc", "")}],
        "SessionEnd": [{"hooks": h("end", "")}]
    }});
    std::fs::create_dir_all(root.join(".gotv")).unwrap();
    std::fs::write(root.join(".gotv/hooks.json"), hooks.to_string()).unwrap();
    let guion = vec![
        llamadas(&[("bash", json!({"cmd": "echo hola"}))]),
        llamadas(&[("write_file", json!({"path": ".env", "content": "A=1\n"}))]),
        llamadas(&[("task", json!({"description": "mirar", "prompt": "Mira .env"}))]),
        texto("informe: .env tiene A"),
        llamadas(&[("read_file", json!({"path": ".env"}))]),
        texto("listo"),
    ];
    let mut a = agente(&root, Mode::AcceptEdits, guion, Mudo::default());
    let r = a.run("haz la tarea").await.unwrap();
    assert_eq!(r, "listo");
    assert!(a.compact_now().await, "se compacta a mano");
    a.cerrar_sesion("other").await;

    let una = |n: &str| {
        let l = volcadas(&v(n));
        assert_eq!(l.len(), 1, "{n} disparó {} veces: {l:?}", l.len());
        l.into_iter().next().unwrap()
    };
    let e = una("ss-startup");
    assert_eq!((e["hook_event_name"].as_str(), e["source"].as_str(), e["model"].as_str()), (Some("SessionStart"), Some("startup"), Some("guion")));
    assert_eq!(e["permission_mode"], "acceptEdits");
    assert_eq!(e["cwd"], json!(root));
    let e = una("ups");
    assert_eq!(e["prompt"], "haz la tarea");
    let e = una("pre");
    assert_eq!((e["tool_name"].as_str(), e["tool_input"]["cmd"].as_str()), (Some("Bash"), Some("echo hola")));
    let e = una("perm");
    assert_eq!((e["hook_event_name"].as_str(), e["tool_name"].as_str(), e["gotv_tool_name"].as_str()), (Some("PermissionRequest"), Some("Bash"), Some("bash")));
    let e = una("not-perm");
    assert_eq!(e["notification_type"], "permission_prompt");
    assert!(e["message"].as_str().unwrap().contains("echo hola"));
    let e = una("post");
    assert!(e["tool_response"].as_str().unwrap().contains("hola"));
    let e = una("fc");
    assert_eq!((e["event"].as_str(), e["file_path"].as_str()), (Some("add"), Some(root.join(".env").to_str().unwrap())));
    let e = una("sa-start");
    assert_eq!(e["agent_type"], "task");
    let e = una("sa-stop");
    assert_eq!((e["last_assistant_message"].as_str(), e["stop_hook_active"].as_bool()), (Some("informe: .env tiene A"), Some(false)));
    let e = una("stop");
    assert_eq!(e["last_assistant_message"], "listo");
    let e = una("not-fin");
    assert_eq!((e["notification_type"].as_str(), e["message"].as_str()), (Some("agent_completed"), Some("listo")));
    let e = una("prec");
    assert_eq!(e["trigger"], "manual");
    let e = una("postc");
    assert!(e["compact_summary"].as_str().unwrap().contains("TAREA:"));
    let e = una("ss-compact");
    assert_eq!(e["source"], "compact");
    let e = una("end");
    assert_eq!(e["reason"], "other");
    // el contexto de SessionStart y UserPromptSubmit viajó con la orden; las instrucciones de PreCompact, con
    // el pedido del resumen
    let vistos = a.engine.vistos.lock().unwrap();
    let primera = vistos[0].messages.iter().rev().find(|m| m.role == "user").unwrap().content.clone();
    assert!(primera.contains("contexto de arranque") && primera.contains("contexto de la orden"), "{primera}");
    assert!(vistos.iter().any(|r| es_resumen(r) && r.messages.iter().any(|m| m.content.contains("conserva los nombres de archivo"))));
    // y todo quedó en el registro del proyecto
    let reg = std::fs::read_to_string(root.join(".gotv/registros/hooks.log")).unwrap();
    for ev in ["SessionStart", "UserPromptSubmit", "PermissionRequest", "PreCompact", "PostCompact", "SubagentStart", "SubagentStop", "FileChanged", "Notification", "SessionEnd"] {
        assert!(reg.contains(ev), "{ev} no está en el registro:\n{reg}");
    }
}

/// «Un hook que bloquea de verdad bloquea»: UserPromptSubmit corta la orden ANTES del modelo.
#[tokio::test]
async fn user_prompt_submit_bloquea_la_orden() {
    let d = crate::CarpetaPrueba::nueva("gotv-f10-ups");
    let root = d.canonicalize().unwrap();
    std::fs::create_dir_all(root.join(".gotv")).unwrap();
    std::fs::write(root.join(".gotv/hooks.json"), json!({"hooks": {"UserPromptSubmit": [{"hooks": [{"type": "command", "command": hook_falso(&root.join("v.jsonl"), 2, "no se piden contraseñas")}]}]}}).to_string()).unwrap();
    let mut a = agente(&root, Mode::AcceptEdits, vec![texto("no debería llegar")], Mudo::default());
    let e = a.run("dame la contraseña del banco").await.unwrap_err();
    assert!(format!("{e}").contains("no se piden contraseñas"), "{e}");
    assert!(a.engine.vistos.lock().unwrap().is_empty(), "el modelo no vio la orden");
}

/// PermissionRequest: un «deny» niega sin preguntar; un «allow» permite sin preguntar… salvo lo que una
/// persona tiene que contestar (lo irreversible del modo automático).
#[tokio::test]
async fn permission_request_niega_y_permite_sin_abrir_puertas() {
    let d = crate::CarpetaPrueba::nueva("gotv-f10-perm");
    let root = d.canonicalize().unwrap();
    std::fs::create_dir_all(root.join(".gotv")).unwrap();
    let decide = |b: &str| format!(r#"{{"hookSpecificOutput":{{"hookEventName":"PermissionRequest","decision":{{"behavior":"{b}","message":"política del equipo"}}}}}}"#);
    let escribe = |b: &str| std::fs::write(root.join(".gotv/hooks.json"), json!({"hooks": {"PermissionRequest": [{"matcher": "Bash", "hooks": [{"type": "command", "command": hook_falso(&root.join("v.jsonl"), 0, &decide(b))}]}]}}).to_string()).unwrap();

    escribe("deny");
    // (`touch` dentro del proyecto no pregunta en GOTV: cuenta como editar; con `>` sí es un comando)
    let mut a = agente(&root, Mode::Default, vec![llamadas(&[("bash", json!({"cmd": "echo x > negado.txt"}))]), texto("no pude")], Mudo::default());
    a.run("x").await.unwrap();
    assert!(!root.join("negado.txt").exists());
    let preguntas = a.front.preguntas.lock().unwrap().clone();
    assert_eq!(preguntas.len(), 1, "sólo la confianza en los hooks del proyecto: {preguntas:?}");
    assert!(a.history.iter().any(|m| m.role == "tool" && m.content.contains("política del equipo")));

    escribe("allow");
    let mut b = agente(&root, Mode::Default, vec![llamadas(&[("bash", json!({"cmd": "echo x > permitido.txt"}))]), texto("hecho")], Mudo::default());
    b.run("x").await.unwrap();
    assert!(root.join("permitido.txt").exists());
    assert_eq!(b.front.preguntas.lock().unwrap().len(), 1, "sólo la confianza");

    // lo irreversible en modo automático: el «allow» del hook NO vale, pregunta a la persona (que dice no)
    let c = agente(&root, Mode::Auto, vec![], Mudo::default());
    *c.front.respuesta.lock().unwrap() = Some(AskAnswer::No);
    let hooks = crate::hooks::load(&root).0;
    let desc = "🔴 IRREVERSIBLE · bash: git push --force";
    assert_eq!(c.permiso_por_hooks(&hooks, "bash", &json!({"cmd": "git push --force"}), desc).await, None, "lo contesta una persona");
    assert!(crate::hooks_eventos::solo_una_persona(desc) && crate::hooks_eventos::solo_una_persona("⚠ SISTEMA: defaults write"));
}

/// En bypass no hay preguntas: PermissionRequest no llega a dispararse (control total).
#[tokio::test]
async fn en_bypass_no_hay_permission_request() {
    let d = crate::CarpetaPrueba::nueva("gotv-f10-bypass");
    let root = d.canonicalize().unwrap();
    std::fs::create_dir_all(root.join(".gotv")).unwrap();
    let vol = root.join("v.jsonl");
    std::fs::write(root.join(".gotv/hooks.json"), json!({"hooks": {"PermissionRequest": [{"hooks": [{"type": "command", "command": hook_falso(&vol, 0, "")}]}]}}).to_string()).unwrap();
    let mut a = agente(&root, Mode::Bypass, vec![llamadas(&[("bash", json!({"cmd": "touch b.txt"}))]), texto("ok")], Mudo::default());
    a.run("x").await.unwrap();
    assert!(root.join("b.txt").exists() && volcadas(&vol).is_empty() && a.front.preguntas.lock().unwrap().is_empty());
}

/// SubagentStop bloquea: el subagente sigue con la razón del hook (una vez; la segunda vez el hook deja ir).
#[tokio::test]
async fn subagent_stop_pide_que_siga() {
    let d = crate::CarpetaPrueba::nueva("gotv-f10-sastop");
    let root = d.canonicalize().unwrap();
    std::fs::create_dir_all(root.join(".gotv")).unwrap();
    // bloquea mientras el informe no diga «verificado» (el hook es un programa de prueba en Rust: cuenta)
    let cuenta = root.join("cuenta");
    let cmd = format!("GOTV_FALSO='c|{}|1|2' '{}' --exact vigilar::tests::programa_falso --nocapture --quiet >/dev/null 2>&1 || {{ echo 'falta verificar' >&2; exit 2; }}", cuenta.display(), std::env::current_exe().unwrap().display());
    std::fs::write(root.join(".gotv/hooks.json"), json!({"hooks": {"SubagentStop": [{"hooks": [{"type": "command", "command": cmd}]}]}}).to_string()).unwrap();
    let guion = vec![llamadas(&[("task", json!({"description": "d", "prompt": "p"}))]), texto("informe sin verificar"), texto("informe verificado"), texto("fin")];
    let mut a = agente(&root, Mode::Bypass, guion, Mudo::default());
    assert_eq!(a.run("x").await.unwrap(), "fin");
    assert!(a.history.iter().any(|m| m.role == "tool" && m.content.contains("informe verificado")), "{:?}", a.history.iter().map(|m| &m.content).collect::<Vec<_>>());
    assert!(a.engine.vistos.lock().unwrap().iter().any(|r| r.messages.iter().any(|m| m.content.contains("falta verificar"))));
}

/// PreCompact bloquea: no se compacta.
#[tokio::test]
async fn pre_compact_bloquea() {
    let d = crate::CarpetaPrueba::nueva("gotv-f10-prec");
    let root = d.canonicalize().unwrap();
    std::fs::create_dir_all(root.join(".gotv")).unwrap();
    std::fs::write(root.join(".gotv/hooks.json"), json!({"hooks": {"PreCompact": [{"hooks": [{"type": "command", "command": hook_falso(&root.join("v.jsonl"), 2, "ahora no")}]}]}}).to_string()).unwrap();
    let guion = (0..4).map(|i| llamadas(&[("read_file", json!({"path": format!("f{i}.txt")}))])).chain([texto("listo")]).collect();
    for i in 0..4 {
        std::fs::write(root.join(format!("f{i}.txt")), "x").unwrap();
    }
    let mut a = agente(&root, Mode::Bypass, guion, Mudo::default());
    a.run("lee").await.unwrap();
    let antes = a.history.len();
    assert!(!a.compact_now().await, "bloqueada: nada cambió");
    assert_eq!(a.history.len(), antes);
    assert!(a.front.lineas.lock().unwrap().iter().any(|l| l.contains("ahora no")));
}

/// «Un hook SessionStart y uno http corren y quedan en el registro» (el «hecho cuando» de F10), con el agente:
/// un SessionStart `http` contra un servidor falso (axum) recibe la orden de arranque.
#[tokio::test]
async fn session_start_por_http_queda_en_el_registro() {
    use axum::{routing::post, Router};
    let d = crate::CarpetaPrueba::nueva("gotv-f10-http");
    let root = d.canonicalize().unwrap();
    let visto: std::sync::Arc<Mutex<Vec<Value>>> = Default::default();
    let v2 = visto.clone();
    let app = Router::new().route(
        "/inicio",
        post(move |cuerpo: String| {
            let v2 = v2.clone();
            async move {
                v2.lock().unwrap().push(serde_json::from_str(&cuerpo).unwrap());
                r#"{"hookSpecificOutput": {"hookEventName": "SessionStart", "additionalContext": "rama: main"}}"#
            }
        }),
    );
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://127.0.0.1:{}/inicio", l.local_addr().unwrap().port());
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    std::fs::create_dir_all(root.join(".gotv")).unwrap();
    std::fs::write(root.join(".gotv/hooks.json"), json!({"hooks": {"SessionStart": [{"hooks": [{"type": "http", "url": url}]}]}}).to_string()).unwrap();
    let mut a = agente(&root, Mode::Bypass, vec![texto("ok")], Mudo::default());
    a.run("hola").await.unwrap();
    a.run("otra").await.unwrap();
    let v = visto.lock().unwrap().clone();
    assert_eq!(v.len(), 1, "una vez por conversación");
    assert_eq!((v[0]["hook_event_name"].as_str(), v[0]["source"].as_str()), (Some("SessionStart"), Some("startup")));
    let reg = std::fs::read_to_string(root.join(".gotv/registros/hooks.log")).unwrap();
    assert!(reg.contains("SessionStart [http]") && reg.contains("HTTP 200"), "{reg}");
    assert!(a.front.lineas.lock().unwrap().iter().any(|l| l.contains("⚓ hook SessionStart [http]")));
    assert!(a.engine.vistos.lock().unwrap()[0].messages.iter().any(|m| m.content.contains("rama: main")));
}

/// FileChanged ve también los cambios hechos POR FUERA entre una orden y otra (`change` y `unlink`).
#[tokio::test]
async fn file_changed_entre_ordenes() {
    let d = crate::CarpetaPrueba::nueva("gotv-f10-fc");
    let root = d.canonicalize().unwrap();
    std::fs::create_dir_all(root.join(".gotv")).unwrap();
    let vol = root.join("v.jsonl");
    std::fs::write(root.join(".envrc"), "export A=1\n").unwrap();
    std::fs::write(root.join(".gotv/hooks.json"), json!({"hooks": {"FileChanged": [{"matcher": ".envrc|.env", "hooks": [{"type": "command", "command": hook_falso(&vol, 0, "")}]}]}}).to_string()).unwrap();
    let mut a = agente(&root, Mode::Bypass, vec![texto("1"), texto("2"), texto("3")], Mudo::default());
    a.run("uno").await.unwrap();
    assert!(volcadas(&vol).is_empty(), "la primera vez sólo se toma la foto");
    std::fs::write(root.join(".envrc"), "export A=2 # más largo\n").unwrap();
    a.run("dos").await.unwrap();
    std::fs::remove_file(root.join(".envrc")).unwrap();
    a.run("tres").await.unwrap();
    let l = volcadas(&vol);
    assert_eq!(l.iter().map(|e| e["event"].as_str().unwrap()).collect::<Vec<_>>(), vec!["change", "unlink"]);
}
