//! Banco de las pruebas de F16: un motor de guion con un GANCHO por llamada (para simular lo que el Portador
//! hace mientras el modelo genera: escribir, cambiar de modelo…) y un agente sobre una carpeta que se borra
//! sola. Sin modelo ni GPU (regla 2 del Portador). Todo este archivo es código de prueba (`#[cfg(test)]` en
//! `escritorio/mod.rs`): el juez de idioma no lo mira.
use crate::checkpoint::Checkpoint;
use crate::permissions::{AskAnswer, Mode, Permissions};
use crate::Agent;
use anyhow::Result;
use async_trait::async_trait;
use gotv_engine::{Capabilities, ChatRequest, ChatResponse, Engine, ToolCall};
use std::sync::{Arc, Mutex};

pub type Gancho = Box<dyn Fn(usize, &ChatRequest) + Send + Sync>;

/// Motor de guion: contesta `turnos` en orden y, ANTES de contestar la llamada `n` (desde 0), corre el gancho.
pub struct Guion {
    pub nombre: String,
    pub turnos: Mutex<Vec<ChatResponse>>,
    pub visto: Arc<Mutex<Vec<ChatRequest>>>,
    pub gancho: Option<Gancho>,
    pub local: bool,
}

impl Guion {
    pub fn nuevo(nombre: &str, turnos: Vec<ChatResponse>) -> Self {
        Guion { nombre: nombre.into(), turnos: Mutex::new(turnos), visto: Default::default(), gancho: None, local: true }
    }
}

#[async_trait]
impl Engine for Guion {
    fn name(&self) -> String {
        self.nombre.clone()
    }
    fn capabilities(&self) -> Capabilities {
        Capabilities { text: true, vision: false, audio_in: false }
    }
    async fn start(&mut self) -> Result<()> {
        Ok(())
    }
    async fn complete(&self, req: ChatRequest) -> Result<ChatResponse> {
        // el resumen al compactar (sin herramientas) no gasta el guion
        if req.tools.is_none() {
            return Ok(texto("RESUMEN: lo hecho"));
        }
        let n = self.visto.lock().unwrap().len();
        if let Some(g) = &self.gancho {
            g(n, &req);
        }
        self.visto.lock().unwrap().push(req);
        let mut t = self.turnos.lock().unwrap();
        Ok(if t.is_empty() { texto("fin") } else { t.remove(0) })
    }
    async fn stop(&mut self) -> Result<()> {
        Ok(())
    }
    fn local(&self) -> bool {
        self.local
    }
}

/// Frontend de prueba: guarda lo que se le dice y contesta `respuesta` a los permisos.
#[derive(Default)]
pub struct Mudo {
    pub lineas: Mutex<Vec<String>>,
    pub actividad: Mutex<Vec<(String, String)>>,
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
    fn activity(&self, k: &str, d: &str) {
        self.actividad.lock().unwrap().push((k.into(), d.into()));
    }
}

pub fn texto(t: &str) -> ChatResponse {
    ChatResponse { content: t.into(), finish_reason: "stop".into(), ..Default::default() }
}

/// Un turno de la IA que pide herramientas; los ids llevan `prefijo` para distinguir de quién son.
pub fn llamadas(prefijo: &str, cs: &[(&str, serde_json::Value)]) -> ChatResponse {
    ChatResponse {
        finish_reason: "tool_calls".into(),
        tool_calls: cs.iter().enumerate().map(|(i, (n, a))| ToolCall { id: format!("{prefijo}{i}"), name: n.to_string(), arguments: a.clone() }).collect(),
        ..Default::default()
    }
}

/// `git -C dir args…` que debe salir bien; devuelve su salida.
pub fn git(dir: &std::path::Path, args: &[&str]) -> String {
    let o = std::process::Command::new("git").arg("-C").arg(dir).args(args).output().unwrap();
    assert!(o.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&o.stderr));
    String::from_utf8_lossy(&o.stdout).trim().to_string()
}

/// F16.6 · el proyecto de un PR: `raiz` con `calc.txt` = «2+2=5» en la rama `arreglo`, empujada a un remoto git
/// LOCAL (`fuera/remoto.git`, que se devuelve). Nada sale a GitHub.
pub fn repo_del_pr(raiz: &std::path::Path, fuera: &std::path::Path) -> std::path::PathBuf {
    let bare = fuera.join("remoto.git");
    git(fuera, &["init", "-q", "--bare", "remoto.git"]);
    git(raiz, &["init", "-q"]);
    git(raiz, &["config", "user.email", "f16@prueba"]);
    git(raiz, &["config", "user.name", "f16"]);
    std::fs::write(raiz.join("calc.txt"), "2+2=5\n").unwrap();
    std::fs::write(raiz.join(".gitignore"), ".gotv/\n").unwrap();
    git(raiz, &["add", "."]);
    git(raiz, &["commit", "-qm", "base"]);
    git(raiz, &["branch", "-M", "arreglo"]);
    git(raiz, &["remote", "add", "origin", bare.to_str().unwrap()]);
    git(raiz, &["push", "-q", "origin", "arreglo"]);
    bare
}

/// F16.6 · un GitHub FALSO local (axum): el PR #7 de `yo/repo` sobre la rama `arreglo` de `bare`; su «CI» pasa si
/// `calc.txt` de la rama dice «2+2=4». Devuelve su dirección y cuántas veces le pidieron FUSIONAR.
pub async fn github_falso(bare: std::path::PathBuf) -> (std::net::SocketAddr, Arc<std::sync::atomic::AtomicU32>) {
    use axum::extract::{Path as Ruta, State};
    use axum::routing::{get, put};
    use axum::{Json, Router};
    use serde_json::json;
    use std::sync::atomic::Ordering;
    #[derive(Clone)]
    struct Est {
        bare: std::path::PathBuf,
        fusiones: Arc<std::sync::atomic::AtomicU32>,
    }
    let est = Est { bare, fusiones: Default::default() };
    let fusiones = est.fusiones.clone();
    let sha_de = |b: &std::path::Path| String::from_utf8_lossy(&std::process::Command::new("git").arg("-C").arg(b).args(["rev-parse", "refs/heads/arreglo"]).output().unwrap().stdout).trim().to_string();
    let app = Router::new()
        .route("/repos/yo/repo/pulls/7", get(move |State(e): State<Est>| async move { Json(json!({"state": "open", "merged": false, "head": {"sha": sha_de(&e.bare), "ref": "arreglo"}})) }))
        .route("/repos/yo/repo/commits/{sha}/check-runs", get(|State(e): State<Est>, Ruta(sha): Ruta<String>| async move {
            let o = std::process::Command::new("git").arg("-C").arg(&e.bare).args(["show", &format!("{sha}:calc.txt")]).output().unwrap();
            let ok = String::from_utf8_lossy(&o.stdout).contains("2+2=4");
            Json(json!({"check_runs": [{"name": "pruebas", "status": "completed", "conclusion": if ok { "success" } else { "failure" }, "output": {"title": "1 prueba falló", "text": "esperaba 2+2=4 en calc.txt y hay 2+2=5"}}]}))
        }))
        .route("/repos/yo/repo/pulls/7/comments", get(|| async { Json(json!([])) }))
        .route("/repos/yo/repo/pulls/7/merge", put(|State(e): State<Est>| async move {
            e.fusiones.fetch_add(1, Ordering::SeqCst);
            Json(json!({"merged": true}))
        }))
        .with_state(est);
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let dir = l.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    (dir, fusiones)
}

pub struct ConCarpeta<E: Engine = Guion> {
    pub agente: Agent<E, Mudo>,
    pub raiz: crate::CarpetaPrueba,
}

pub fn agente(tag: &str, modo: Mode, motor: Guion) -> ConCarpeta {
    agente_con(tag, modo, motor)
}

/// El mismo agente de prueba con cualquier motor (las mediciones con un modelo de verdad, `con_modelo`).
pub fn agente_con<E: Engine>(tag: &str, modo: Mode, motor: E) -> ConCarpeta<E> {
    let raiz = crate::CarpetaPrueba::nueva(&format!("gotv-f16-{tag}"));
    let root = raiz.canonicalize().unwrap();
    let agente = Agent {
        engine: motor,
        perms: Permissions::new(modo, &root, vec![], vec![], vec![]),
        checkpoint: Some(Checkpoint::new(&root)),
        root,
        max_steps: 10,
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
    ConCarpeta { agente, raiz }
}
