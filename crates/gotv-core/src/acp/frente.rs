//! F10.3 · El frente del editor: traduce lo que hace el agente a avisos de ACP (`session/update`) y sus
//! preguntas a `session/request_permission`. Los avisos salen EN ORDEN por una sola tarea escritora; una
//! petición de permiso espera a que salga todo lo anterior.
use super::archivos::Editor;
use crate::canal::Canal;
use crate::frontend::{Frontend, Herramienta};
use crate::permissions::AskAnswer;
use async_trait::async_trait;
use gotv_engine::StreamEvent;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};

enum Salida {
    Aviso(Value),
    Marca(oneshot::Sender<()>),
}

/// Lo que se sabe de la herramienta en curso.
struct EnCurso {
    acp: String,
    nombre: String,
    args: Value,
    /// (ruta, texto de antes) de lo que se va a editar
    antes: Option<(PathBuf, Option<String>)>,
}

pub struct FrenteAcp {
    canal: Arc<Canal>,
    sesion: String,
    caps: Arc<Mutex<Value>>,
    /// la carpeta del proyecto del chat
    raiz: PathBuf,
    tx: mpsc::UnboundedSender<Salida>,
    n: std::sync::atomic::AtomicU64,
    en_curso: Mutex<HashMap<String, EnCurso>>,
    /// la última que empezó (a ella se refiere un permiso)
    ultima: Mutex<Option<String>>,
}

fn es_edicion(n: &str) -> bool {
    matches!(n, "write_file" | "edit" | "multi_edit")
}

fn tipo(n: &str) -> &'static str {
    match n {
        "read_file" | "list_dir" | "view_image" => "read",
        "write_file" | "edit" | "multi_edit" => "edit",
        "bash" | "process" | "pruebas" => "execute",
        "grep" | "glob" | "rag_search" | "lsp" | "mapa" => "search",
        "web_fetch" | "web_search" | "investigar" => "fetch",
        "task" | "todo" => "think",
        _ => "other",
    }
}

fn titulo(n: &str, a: &Value) -> String {
    let detalle = a["cmd"].as_str().or(a["path"].as_str()).or(a["pattern"].as_str()).or(a["url"].as_str()).or(a["query"].as_str()).or(a["description"].as_str()).unwrap_or("");
    let d: String = detalle.lines().next().unwrap_or("").chars().take(160).collect();
    if d.is_empty() {
        n.to_string()
    } else {
        format!("{n}: {d}")
    }
}

/// Lo que dejaría la edición, sin hacerla (para enseñar el diff ANTES de pedir permiso).
fn propuesto(nombre: &str, args: &Value, antes: &str) -> Option<String> {
    match nombre {
        "write_file" if args["append"].as_bool() == Some(true) => Some(format!("{antes}{}", args["content"].as_str().unwrap_or(""))),
        "write_file" => args["content"].as_str().map(String::from),
        "edit" => {
            let (o, n) = (args["old"].as_str()?, args["new"].as_str()?);
            antes.contains(o).then(|| antes.replacen(o, n, if args["replace_all"].as_bool() == Some(true) { usize::MAX } else { 1 }))
        }
        "multi_edit" => {
            let mut t = antes.to_string();
            for e in args["edits"].as_array()? {
                let (o, n) = (e["old"].as_str()?, e["new"].as_str()?);
                if !t.contains(o) {
                    return None;
                }
                t = t.replacen(o, n, if e["replace_all"].as_bool() == Some(true) { usize::MAX } else { 1 });
            }
            Some(t)
        }
        _ => None,
    }
}

impl FrenteAcp {
    pub fn new(canal: Arc<Canal>, sesion: &str, caps: Arc<Mutex<Value>>, raiz: &Path) -> Self {
        let (tx, mut rx) = mpsc::unbounded_channel::<Salida>();
        let c = canal.clone();
        tokio::spawn(async move {
            while let Some(s) = rx.recv().await {
                match s {
                    Salida::Aviso(v) => {
                        let _ = c.avisar("session/update", v).await;
                    }
                    Salida::Marca(t) => {
                        let _ = t.send(());
                    }
                }
            }
        });
        FrenteAcp { canal, sesion: sesion.to_string(), caps, raiz: raiz.to_path_buf(), tx, n: Default::default(), en_curso: Mutex::new(HashMap::new()), ultima: Mutex::new(None) }
    }

    pub fn sesion(&self) -> &str {
        &self.sesion
    }

    fn aviso(&self, update: Value) {
        let _ = self.tx.send(Salida::Aviso(json!({"sessionId": self.sesion, "update": update})));
    }

    /// Espera a que salga todo lo encolado.
    pub async fn vaciar(&self) {
        let (t, r) = oneshot::channel();
        if self.tx.send(Salida::Marca(t)).is_ok() {
            let _ = r.await;
        }
    }

    /// Las rutas de las herramientas son relativas al proyecto; el editor las quiere absolutas.
    fn absoluta(&self, p: &str) -> PathBuf {
        let q = Path::new(p);
        if q.is_absolute() {
            q.to_path_buf()
        } else {
            self.raiz.join(q)
        }
    }

    async fn leer(&self, ruta: &Path) -> Option<String> {
        let caps = self.caps.lock().unwrap().clone();
        EditorAcp::new(self.canal.clone(), &self.sesion, caps).leer(ruta).await.or_else(|| std::fs::read_to_string(ruta).ok())
    }

    fn nuevo_id(&self) -> String {
        format!("t{}", self.n.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1)
    }
}

#[async_trait]
impl Frontend for FrenteAcp {
    /// El registro va a stderr: la salida estándar es el canal.
    async fn log(&self, line: String) {
        eprintln!("{line}");
    }

    async fn ask(&self, desc: &str) -> AskAnswer {
        // la herramienta a la que se refiere (o una propia si la pregunta no es de una herramienta)
        let ultima = self.ultima.lock().unwrap().clone();
        let datos = ultima.as_ref().and_then(|u| self.en_curso.lock().unwrap().get(u).map(|e| (e.acp.clone(), e.nombre.clone(), e.args.clone(), e.antes.clone())));
        // `propia` = la pregunta no es de una herramienta en curso: su tarjeta lleva la pregunta como título. Si es de
        // una herramienta, su título se queda (medido en Zed 1.28.1: un `title` en el permiso pisaba «edit: …»)
        let (acp, diff, propia) = match datos {
            Some((acp, nombre, args, antes)) => {
                let diff = antes.and_then(|(ruta, previo)| {
                    let nuevo = propuesto(&nombre, &args, previo.as_deref().unwrap_or(""))?;
                    Some(json!({"type": "diff", "path": ruta, "oldText": previo, "newText": nuevo}))
                });
                (acp, diff, false)
            }
            None => {
                let acp = self.nuevo_id();
                self.aviso(json!({"sessionUpdate": "tool_call", "toolCallId": acp, "title": desc.lines().next().unwrap_or(""), "kind": "other", "status": "pending"}));
                (acp, None, true)
            }
        };
        let mut contenido = vec![json!({"type": "content", "content": {"type": "text", "text": desc}})];
        contenido.extend(diff);
        let opciones = json!([
            {"optionId": "si", "name": crate::i18n::t("Permitir", "Allow"), "kind": "allow_once"},
            {"optionId": "siempre", "name": crate::i18n::t("Permitir siempre", "Always allow"), "kind": "allow_always"},
            {"optionId": "no", "name": crate::i18n::t("Negar", "Deny"), "kind": "reject_once"},
            {"optionId": "nunca", "name": crate::i18n::t("Negar siempre", "Always deny"), "kind": "reject_always"},
        ]);
        // lo anterior sale antes que la pregunta
        self.vaciar().await;
        let mut tool_call = json!({"toolCallId": acp, "content": contenido});
        if propia {
            tool_call["title"] = json!(desc.lines().next().unwrap_or(""));
        }
        let params = json!({"sessionId": self.sesion, "toolCall": tool_call, "options": opciones});
        // una persona puede tardar: un día de tope; si el editor no contesta o no se entiende, NEGADO
        match self.canal.pedir("session/request_permission", params, Duration::from_secs(86_400)).await {
            Ok(r) if r["outcome"]["outcome"] == "selected" => match r["outcome"]["optionId"].as_str() {
                Some("si") => AskAnswer::Yes,
                Some("siempre") => AskAnswer::Always,
                Some("nunca") => AskAnswer::Never,
                _ => AskAnswer::No,
            },
            _ => AskAnswer::No,
        }
    }

    fn delta(&self, ev: &StreamEvent) {
        match ev {
            StreamEvent::Text(t) if !t.is_empty() => self.aviso(json!({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": t}})),
            StreamEvent::Reasoning(t) if !t.is_empty() => self.aviso(json!({"sessionUpdate": "agent_thought_chunk", "content": {"type": "text", "text": t}})),
            _ => {}
        }
    }

    async fn herramienta(&self, h: Herramienta<'_>) {
        match h {
            Herramienta::Empieza { id, nombre, args } => {
                let acp = self.nuevo_id();
                let ruta = args["path"].as_str().map(|p| self.absoluta(p));
                let antes = match (&ruta, es_edicion(nombre)) {
                    (Some(r), true) => Some((r.clone(), self.leer(r).await)),
                    _ => None,
                };
                let mut v = json!({"sessionUpdate": "tool_call", "toolCallId": acp, "title": titulo(nombre, args), "kind": tipo(nombre), "status": "in_progress", "rawInput": args});
                if let Some(r) = &ruta {
                    v["locations"] = json!([{"path": r}]);
                }
                self.aviso(v);
                self.en_curso.lock().unwrap().insert(id.to_string(), EnCurso { acp, nombre: nombre.to_string(), args: args.clone(), antes });
                *self.ultima.lock().unwrap() = Some(id.to_string());
            }
            Herramienta::Termina { id, nombre, salida, .. } => {
                let Some(e) = self.en_curso.lock().unwrap().remove(id) else { return };
                let fallo = ["ERROR", "DENEGADO", "DENIED", "BLOQUEADO", "BLOCKED"].iter().any(|p| salida.starts_with(p)); // juez-idioma: datos (cómo empiezan las salidas de error en los dos idiomas)
                let mut contenido = Vec::new();
                if let (false, true, Some((ruta, previo))) = (fallo, es_edicion(nombre), e.antes.clone()) {
                    let nuevo = self.leer(&ruta).await;
                    contenido.push(json!({"type": "diff", "path": ruta, "oldText": previo, "newText": nuevo.unwrap_or_default()}));
                }
                let corto: String = salida.chars().take(4000).collect();
                contenido.push(json!({"type": "content", "content": {"type": "text", "text": corto}}));
                self.aviso(json!({"sessionUpdate": "tool_call_update", "toolCallId": e.acp, "status": if fallo { "failed" } else { "completed" }, "content": contenido}));
            }
        }
    }
}

/// Los archivos por el editor (`fs/read_text_file`, `fs/write_text_file`), sólo si los ofreció.
pub struct EditorAcp {
    canal: Arc<Canal>,
    sesion: String,
    caps: Value,
}

impl EditorAcp {
    pub fn new(canal: Arc<Canal>, sesion: &str, caps: Value) -> Self {
        EditorAcp { canal, sesion: sesion.to_string(), caps }
    }
}

#[async_trait]
impl Editor for EditorAcp {
    async fn leer(&self, ruta: &Path) -> Option<String> {
        if self.caps["fs"]["readTextFile"].as_bool() != Some(true) || !ruta.is_absolute() {
            return None;
        }
        let r = self.canal.pedir("fs/read_text_file", json!({"sessionId": self.sesion, "path": ruta}), Duration::from_secs(30)).await.ok()?;
        r["content"].as_str().map(String::from)
    }

    async fn escribir(&self, ruta: &Path, texto: &str) -> anyhow::Result<()> {
        if self.caps["fs"]["writeTextFile"].as_bool() != Some(true) || !ruta.is_absolute() {
            std::fs::write(ruta, texto)?;
            return Ok(());
        }
        self.canal.pedir("fs/write_text_file", json!({"sessionId": self.sesion, "path": ruta, "content": texto}), Duration::from_secs(30)).await?;
        Ok(())
    }
}
