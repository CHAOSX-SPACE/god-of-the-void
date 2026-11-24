//! F9 · subagentes de verdad: cada uno es un `Agent` PROPIO —su conversación, sus permisos, su punto de
//! restauración, su rama— que toma PRESTADO el motor del chat ([`Prestado`]: el mismo modelo, sin cargar
//! otro ni copiarlo; `Engine::complete` es `&self`, así que varios generan a la vez si el motor puede) y
//! habla con el usuario por la cara del chat ([`FrenteHijo`]: sus líneas llevan su nombre; sus preguntas,
//! de a una).
//!
//! Cuándo: cuando el modelo pide VARIOS `task` en un mismo turno (y nada más en ese turno). El planificador
//! (`enjambre::planificador`) decide cuántos a la vez: en la nube, en paralelo con tope; en local, por lotes
//! si el motor genera en lote, si no por turnos. A la vez (más de uno) → cada uno en su rama git
//! (`worktree.rs`), y al terminar lo de cada rama vuelve al árbol si no choca (`enjambre::ramas::al_arbol`);
//! lo que choca se reporta y su rama queda. Un `task` solo sigue por el camino de siempre (`run_subagent`).
//!
//! Aquí también: la herramienta `enjambre` (tablero, fusionar, verificador, mensajes), el buzón entre pasos y
//! la rama propia del agente principal (`/rama`).

use super::{first_line, Agent};
use crate::checkpoint::Checkpoint;
use crate::enjambre::{buzon, herramienta, planificador, ramas, tablero, verificador};
use crate::frontend::Frontend;
use crate::permissions::{AskAnswer, Permissions};
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use gotv_engine::{Capabilities, ChatRequest, ChatResponse, Engine, Message, StreamEvent, ToolCall};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// El motor del chat, PRESTADO a un subagente: el mismo modelo ya encendido. Encenderlo o apagarlo desde el
/// subagente no hace nada (es del chat).
pub(crate) struct Prestado<'a>(pub(crate) &'a (dyn Engine + 'a));

#[async_trait]
impl<'a> Engine for Prestado<'a> {
    fn name(&self) -> String {
        self.0.name()
    }
    fn capabilities(&self) -> Capabilities {
        self.0.capabilities()
    }
    async fn start(&mut self) -> Result<()> {
        Ok(())
    }
    async fn complete(&self, req: ChatRequest) -> Result<ChatResponse> {
        self.0.complete(req).await
    }
    async fn complete_stream(&self, req: ChatRequest, on: &(dyn Fn(StreamEvent) + Send + Sync)) -> Result<ChatResponse> {
        self.0.complete_stream(req, on).await
    }
    async fn stop(&mut self) -> Result<()> {
        Ok(())
    }
    fn local(&self) -> bool {
        self.0.local()
    }
    async fn precargar(&self, req: ChatRequest, on: &(dyn Fn(StreamEvent) + Send + Sync)) -> Result<gotv_engine::Precarga> {
        self.0.precargar(req, on).await
    }
    fn compartir(&self) -> Option<std::sync::Arc<dyn Engine>> {
        self.0.compartir()
    }
    fn lote_max(&self) -> usize {
        self.0.lote_max()
    }
}

/// La cara de un subagente: la del chat, con su nombre delante. Su texto en vivo no se muestra (varios a
/// la vez serían un revoltijo: vuelve su informe) y sus preguntas al usuario van DE A UNA.
pub(crate) struct FrenteHijo<'a> {
    pub(crate) padre: &'a (dyn Frontend + 'a),
    pub(crate) nombre: String,
    pub(crate) turno: &'a tokio::sync::Mutex<()>,
}

#[async_trait]
impl<'a> Frontend for FrenteHijo<'a> {
    async fn log(&self, line: String) {
        self.padre.log(format!("  ⎸{} · {}", self.nombre, line.trim_start())).await;
    }
    async fn ask(&self, desc: &str) -> AskAnswer {
        let _uno = self.turno.lock().await;
        self.padre.ask(&format!("[{}] {desc}", self.nombre)).await
    }
    fn activity(&self, kind: &str, detail: &str) {
        if kind == "step" {
            self.padre.activity("step", &format!("{}: {detail}", self.nombre));
        }
    }
}

/// El bucle de un subagente, con su tipo BORRADO: el bucle del agente llama a éste y éste al bucle del
/// agente; sin borrar el tipo, el compilador no termina de decidir si el futuro es `Send` (E0275).
type Bucle<'x> = std::pin::Pin<Box<dyn std::future::Future<Output = Result<String>> + Send + 'x>>;

fn bucle<'x, 'a: 'x>(h: &'x mut Agent<Prestado<'a>, FrenteHijo<'a>>, msgs: &'x mut Vec<Message>, hooks: &'x crate::hooks::Hooks, depth: u8) -> Bucle<'x> {
    Box::pin(h.run_loop(msgs, hooks, depth))
}

/// Una subtarea pedida con `task`.
struct Pieza<'c> {
    call: &'c ToolCall,
    prompt: String,
    nombre: String,
    aislar: bool,
}

/// Una pieza lista para correr: su rama (si va aislada) y si la rama la puso GOTV (y entonces la trae al
/// árbol al terminar) o la pidió el modelo (y entonces se queda para que él decida).
struct Preparada<'c> {
    pieza: Pieza<'c>,
    wt: Option<crate::worktree::Worktree>,
    forzada: bool,
}

/// El pedido de un subagente que trabaja con otros.
fn pedido_de_pieza(prompt: &str, total: usize, rama: Option<&str>) -> String {
    let donde = match rama {
        Some(r) => crate::tr!(
            " Trabajas en tu propia rama git ({r}), en una carpeta aparte: tus cambios vuelven al proyecto cuando termines.",
            " You work on your own git branch ({r}), in a separate folder: your changes go back to the project when you finish."
        ),
        None => String::new(),
    };
    crate::tr!(
        "Eres un SUBAGENTE del agente principal, uno de {total} que trabajan en partes distintas de la misma tarea.{donde} Haz SÓLO esta subtarea, con las herramientas, y termina con un informe breve y concreto para él (qué encontraste o hiciste, con archivos y líneas). No toques archivos que no son de tu parte: si hace falta cambiar algo común, no lo cambies; dilo en tu informe o anótalo en el tablero del equipo (herramienta `enjambre`, accion «anotar»; léelo con accion «tablero»). No le preguntes nada al usuario.\n\nSubtarea: {prompt}",
        "You are a SUBAGENT of the main agent, one of {total} working on different parts of the same task.{donde} Do ONLY this subtask, with the tools, and finish with a short, concrete report for it (what you found or did, with files and lines). Do not touch files that are not part of your piece: if something shared needs changing, do not change it; say so in your report or note it on the team board (tool `enjambre`, accion «anotar»; read it with accion «tablero»). Do not ask the user anything.\n\nSubtask: {prompt}"
    )
}

/// La rama propia del agente principal (`/rama`): lo que se apartó para volver.
struct RamaPrincipal {
    wt: crate::worktree::Worktree,
    raiz: PathBuf,
    raiz_permisos: PathBuf,
    checkpoint: Option<Checkpoint>,
}

/// chat (su dueño de procesos sin raíz) → su rama propia
static RAMAS: Mutex<Option<HashMap<String, RamaPrincipal>>> = Mutex::new(None);

impl<E: Engine, F: Frontend> Agent<E, F> {
    /// Un subagente: un `Agent` propio sobre `motor` (prestado), en `root`, con `perms` y `checkpoint`
    /// suyos. Sin servidores MCP propios (no arranca procesos ni pide confianza otra vez), sin manejar la
    /// pantalla del usuario, con lo cargado del chat más `enjambre`.
    pub(crate) fn hijo<'a>(&'a self, motor: &'a (dyn Engine + 'a), root: PathBuf, mut perms: Permissions, checkpoint: Option<Checkpoint>, front: FrenteHijo<'a>) -> Agent<Prestado<'a>, FrenteHijo<'a>> {
        perms.set_root(root.clone());
        perms.desktop_turn = false;
        let mut tools_loaded = self.tools_loaded.clone();
        tools_loaded.insert("enjambre".into());
        Agent {
            engine: Prestado(motor),
            root,
            max_steps: self.max_steps,
            max_tokens: self.max_tokens,
            sampling: self.sampling,
            context_budget: self.context_budget,
            perms,
            checkpoint,
            front,
            history: Vec::new(),
            session: None,
            cancel: self.cancel.clone(),
            // F9: los servidores MCP del principal, compartidos (sin procesos nuevos ni confianza otra vez)
            mcp: Some(self.mcp.as_ref().map(crate::mcp::Mcp::compartido).unwrap_or_default()),
            attachments: Vec::new(),
            user_media: Vec::new(),
            chat_only: false,
            embedder: None,
            lsp: None,
            computer_use: false,
            procs: Default::default(),
            tools_loaded,
            bases_nube: self.bases_nube.clone(),
            guardia: crate::guardia::Guardia::nueva(),
            perfil: self.perfil,
            pro: self.pro.clone(),
            motor_dormido: false,
            visible: None,
            estado_de: None,
            calibre: self.calibre,
            recorto_ubicaciones: false,
            tope_paso: 0,
        }
    }

    /// La raíz del proyecto del chat (con `/rama`, la de antes de apartarse: el tablero, el buzón y lo que
    /// se fusiona viven ahí).
    pub(crate) fn raiz_del_chat(&self) -> PathBuf {
        let clave = self.procs.key(Path::new(""));
        RAMAS.lock().unwrap().as_ref().and_then(|m| m.get(&clave).map(|r| r.raiz.clone())).unwrap_or_else(|| self.root.clone())
    }

    /// F9: los `task` de UN turno, juntos (ver el encabezado del módulo). Devuelve la salida de cada uno por
    /// el id de su llamada; vacío si no toca (menos de dos `task`, un subagente, otras herramientas en el
    /// mismo turno o una llamada cortada): entonces siguen por el camino de siempre, de a uno.
    pub(super) async fn subagentes_juntos(&self, calls: &[ToolCall], hooks: &crate::hooks::Hooks, depth: u8, cortada: Option<&str>) -> HashMap<String, String> {
        let mut salidas = HashMap::new();
        if depth > 0 || calls.len() < 2 || !calls.iter().all(|c| c.name == "task") || calls.iter().any(|c| Some(c.id.as_str()) == cortada) {
            return salidas;
        }
        let piezas: Vec<Pieza> = calls
            .iter()
            .filter_map(|call| {
                let prompt = call.arguments["prompt"].as_str().unwrap_or("").trim().to_string();
                (!prompt.is_empty()).then(|| Pieza {
                    call,
                    nombre: call.arguments["description"].as_str().unwrap_or(&prompt).chars().take(80).collect(),
                    aislar: call.arguments["aislar"].as_bool().unwrap_or(false) || call.arguments["aislar"].as_str() == Some("true"),
                    prompt,
                })
            })
            .collect();
        if piezas.len() != calls.len() {
            // alguno sin `prompt`: el camino de siempre da el error de cada uno
            return salidas;
        }
        let total = piezas.len();
        let modo = planificador::decidir(self.engine.local(), self.engine.lote_max(), total, planificador::tope_en(&self.root));
        let hay_git = ramas::hay_repo(&self.root);
        // a la vez sin git no se puede aislar: dos en el mismo árbol se pisarían
        let a_la_vez = if hay_git { modo.a_la_vez() } else { 1 };
        let sin_git = if !hay_git && modo.a_la_vez() > 1 {
            crate::i18n::t(" — sin repositorio git no hay ramas que aislar: van de a uno, en tu árbol", " — without a git repository there are no branches to isolate: one at a time, in your tree")
        } else {
            ""
        };
        let modo_txt = if a_la_vez > 1 { modo.describir() } else { planificador::Modo::Turnos.describir() };
        self.front.log(crate::tr!("⇉ {total} subagentes {modo_txt}{sin_git}", "⇉ {total} subagents {modo_txt}{sin_git}")).await;
        // el tablero del equipo: el de esta conversación
        let sid = self.session.as_ref().map(|s| s.id);
        let ruta_tablero = tablero::ruta(crate::artefactos::chat_de(&self.raiz_del_chat(), sid, &self.proc_owner()).dir());
        // las ramas, de a una (dos `git worktree add` a la vez pelean por el candado del repo)
        let mut prep: Vec<Preparada> = Vec::new();
        for p in piezas {
            let aislar = p.aislar || a_la_vez > 1;
            if !aislar {
                prep.push(Preparada { pieza: p, wt: None, forzada: false });
                continue;
            }
            match crate::worktree::create(&self.root, &p.nombre) {
                Ok(wt) => {
                    let forzada = !p.aislar;
                    prep.push(Preparada { pieza: p, wt: Some(wt), forzada });
                }
                Err(e) => {
                    salidas.insert(p.call.id.clone(), crate::tr!("ERROR: {e}", "ERROR: {e}"));
                }
            }
        }
        let turno = tokio::sync::Mutex::new(());
        let mut hijos = Vec::with_capacity(prep.len());
        for (i, p) in prep.iter().enumerate() {
            let root = p.wt.as_ref().map(|w| w.path.clone()).unwrap_or_else(|| self.root.clone());
            // su deshacer es su rama; sin rama, el punto de restauración del proyecto (van de a uno)
            let checkpoint = if p.wt.is_some() { None } else { self.checkpoint.as_ref().map(|_| Checkpoint::new(&self.root)) };
            let corto = format!("sub-{}", i + 1);
            let front = FrenteHijo { padre: &self.front, nombre: corto.clone(), turno: &turno };
            let h = self.hijo(&self.engine, root, self.perms.clone(), checkpoint, front);
            tablero::asignar(&h.proc_owner(), ruta_tablero.clone(), &corto);
            let rama = p.wt.as_ref().map(|w| w.branch.clone());
            let en_rama = rama.as_ref().map(|r| crate::tr!(" (rama {r})", " (branch {r})")).unwrap_or_default();
            self.front.activity("subagent_start", &p.pieza.nombre);
            self.front.log(crate::tr!("⤷ subagente {corto}: {}{en_rama}", "⤷ subagent {corto}: {}{en_rama}", p.pieza.nombre)).await;
            let msgs = vec![Message::system(self.system_prompt()), Message::user(pedido_de_pieza(&p.pieza.prompt, total, rama.as_deref()))];
            hijos.push((h, msgs, format!("{}-{}", p.pieza.call.id, 2)));
        }
        // a la vez, como mucho `a_la_vez` (el semáforo de tokio es FIFO: por turnos, en el orden pedido)
        let sem = tokio::sync::Semaphore::new(a_la_vez);
        let futuros: Vec<Bucle> = hijos
            .iter_mut()
            .map(|(h, msgs, id)| -> Bucle {
                let sem = &sem;
                Box::pin(async move {
                    let _cupo = sem.acquire().await;
                    h.hook_subagente_empieza(hooks, id).await;
                    let mut r = bucle(h, msgs, hooks, depth + 1).await;
                    // F10.1: SubagentStop puede pedirle que siga (con tope), como en el camino de siempre
                    for seguidas in 0..3u32 {
                        let Ok(informe) = &r else { break };
                        let Some(razon) = h.hook_subagente_termina(hooks, id, informe, seguidas).await else { break };
                        h.front.log(crate::tr!("  · un hook pide que el subagente siga: {razon}", "  · a hook asks the subagent to continue: {razon}")).await;
                        msgs.push(Message::assistant(informe.clone()));
                        msgs.push(Message::aviso(crate::tr!("Aún no terminas — un hook del usuario dice: {razon}", "You are not done yet — a user hook says: {razon}")));
                        r = bucle(h, msgs, hooks, depth + 1).await;
                    }
                    r
                })
            })
            .collect();
        // Medido (prueba de reloj, 2026-11-17): en UNA tarea (`join_all`) coincidían en vuelo, pero la CPU y lo
        // síncrono de uno (armar la petición, git, archivos) frenaba a los otros: el reloj generando bajaba
        // sólo de 1,51 s a 1,17 s. Con un runtime multihilo (la app y la CLI) cada uno corre en su hilo, sobre
        // los MISMOS manejadores de red y tiempo del runtime (`Handle::block_on`); en uno de un hilo, juntos.
        let multihilo = matches!(tokio::runtime::Handle::try_current().map(|h| h.runtime_flavor()), Ok(tokio::runtime::RuntimeFlavor::MultiThread));
        let resultados: Vec<Result<String>> = if multihilo && a_la_vez > 1 {
            let rt = tokio::runtime::Handle::current();
            tokio::task::block_in_place(|| {
                std::thread::scope(|s| {
                    let hilos: Vec<_> = futuros
                        .into_iter()
                        .map(|f| {
                            let rt = &rt;
                            s.spawn(move || rt.block_on(f))
                        })
                        .collect();
                    hilos.into_iter().map(|h| h.join().unwrap_or_else(|_| Err(anyhow!("{}", crate::i18n::t("el subagente se cayó", "the subagent crashed"))))).collect()
                })
            })
        } else {
            futures_util::future::join_all(futuros).await
        };
        let pasos: Vec<usize> = hijos.iter().map(|(_, m, _)| m.iter().filter(|x| x.role == "tool").count()).collect();
        for (h, _, _) in &hijos {
            tablero::soltar(&h.proc_owner());
        }
        // al soltarse, cada subagente mata SUS procesos (su dueño es suyo)
        drop(hijos);
        // lo de las ramas que puso GOTV, al árbol: un solo punto de restauración para todo (se deshace junto)
        let pendiente = self.checkpoint.as_ref().filter(|_| prep.iter().any(|p| p.forzada)).and_then(|c| c.begin().ok());
        // cerrar cada rama (su commit, quitar su carpeta) a la vez: cada una es su carpeta (medido: de a una,
        // ~5 lanzamientos de `git` por rama se comían lo que ganaba el paralelo con respuestas cortas)
        let cierres: Vec<Option<Result<String>>> = std::thread::scope(|s| {
            let hilos: Vec<_> = prep
                .iter()
                .map(|p| {
                    p.wt.as_ref().map(|wt| {
                        let mensaje = format!("gotv: {}", p.pieza.nombre);
                        s.spawn(move || wt.finish(&mensaje))
                    })
                })
                .collect();
            hilos.into_iter().map(|h| h.map(|h| h.join().unwrap_or_else(|_| Err(anyhow!("worktree"))))).collect()
        });
        let mut entro = false;
        for (((p, r), pasos), cierre) in prep.into_iter().zip(resultados).zip(pasos).zip(cierres) {
            let mut cola = String::new();
            if let (Some(wt), Some(cierre)) = (&p.wt, cierre) {
                cola = match cierre {
                    Err(e) => crate::tr!("\n\n⚠ no pude cerrar el worktree: {e}", "\n\n⚠ could not close the worktree: {e}"),
                    Ok(resumen) if !p.forzada => format!("\n\n{resumen}"),
                    Ok(_) if !ramas::existe(&self.root, &wt.branch) => crate::i18n::t("\n\n(no cambió ningún archivo)", "\n\n(no file changed)").to_string(),
                    Ok(_) => match ramas::al_arbol(&self.root, &wt.branch) {
                        Ok(f) => {
                            entro |= matches!(f, ramas::Fusion::Hecha { .. });
                            format!("\n\n{}", ramas::informe(&[f]))
                        }
                        Err(e) => crate::tr!("\n\n⚠ no pude traer sus cambios ({e}): quedan en la rama {}", "\n\n⚠ could not bring its changes ({e}): they stay on branch {}", wt.branch),
                    },
                };
            }
            self.front.activity("subagent_end", &p.pieza.nombre);
            self.front.log(crate::tr!("⤶ subagente terminó ({pasos} herramienta(s) usadas)", "⤶ subagent finished ({pasos} tool call(s))")).await;
            let out = match r {
                Ok(report) => crate::tr!("[informe del subagente]\n{report}{cola}", "[subagent report]\n{report}{cola}"),
                Err(e) => crate::tr!("ERROR del subagente: {e}{cola}", "subagent ERROR: {e}{cola}"),
            };
            salidas.insert(p.pieza.call.id.clone(), out);
        }
        if let (Some(cp), Some(pend), true) = (&self.checkpoint, pendiente, entro) {
            let _ = cp.commit(pend, crate::i18n::t("subagentes en paralelo", "parallel subagents"));
        }
        salidas
    }

    /// Lo que la herramienta `enjambre` sabe de quien la llama.
    fn contexto_enjambre(&self) -> herramienta::Contexto {
        let plan = self.perms.mode == crate::permissions::Mode::Plan;
        match tablero::de(&self.proc_owner()) {
            Some((ruta, yo)) => herramienta::Contexto { raiz: self.root.clone(), tablero: ruta, yo, subagente: true, sesion: None, plan },
            None => {
                let raiz = self.raiz_del_chat();
                let sesion = self.session.as_ref().map(|s| s.id);
                let chat = crate::artefactos::chat_de(&raiz, sesion, &self.proc_owner());
                herramienta::Contexto { tablero: tablero::ruta(chat.dir()), raiz, yo: crate::i18n::t("principal", "main").to_string(), subagente: false, sesion, plan }
            }
        }
    }

    /// F9: la herramienta `enjambre`.
    pub(super) async fn herramienta_enjambre(&self, args: &serde_json::Value, hooks: &crate::hooks::Hooks) -> crate::tools::ToolResult {
        let ctx = self.contexto_enjambre();
        match herramienta::simple(args, &ctx) {
            Some(Ok(output)) => crate::tools::ToolResult { ok: true, output },
            Some(Err(e)) => crate::tools::ToolResult { ok: false, output: format!("ERROR: {e}") },
            None => self.verificar(args, hooks, &ctx).await,
        }
    }

    async fn verificar(&self, args: &serde_json::Value, hooks: &crate::hooks::Hooks, ctx: &herramienta::Contexto) -> crate::tools::ToolResult {
        let error = |s: String| crate::tools::ToolResult { ok: false, output: format!("ERROR: {s}") };
        if ctx.subagente {
            return error(crate::i18n::t("el verificador lo lanza el agente principal, al final", "the verifier is launched by the main agent, at the end").into());
        }
        let Some(criterio) = args["criterio"].as_str().map(str::trim).filter(|s| !s.is_empty()) else {
            return error(crate::i18n::t("falta `criterio`: qué pruebas correr y qué debe cumplirse", "missing `criterio`: which tests to run and what must hold").into());
        };
        let modelo = args["modelo"].as_str().map(str::trim).filter(|s| !s.is_empty());
        let mut otro = None;
        if let Some(m) = modelo {
            match verificador::motor_de(m, &self.root).await {
                Ok(x) => otro = Some(x),
                Err(e) => return error(e.to_string()),
            }
        }
        let en = modelo.map(|m| crate::tr!(" en {m}", " on {m}")).unwrap_or_default();
        self.front.log(crate::tr!("⚖ verificador{en}: {}", "⚖ verifier{en}: {}", first_line(criterio))).await;
        let r = self.correr_verificador(criterio, otro.as_ref().map(|(e, c)| (e as &dyn Engine, *c)), hooks).await;
        if let Some((mut e, _)) = otro {
            let _ = e.stop().await;
        }
        match r {
            Ok(informe) => {
                let cabeza = match verificador::veredicto(&informe) {
                    Some(true) => crate::i18n::t("✓ PASA", "✓ PASS"),
                    Some(false) => crate::i18n::t("✗ FALLA", "✗ FAIL"),
                    None => crate::i18n::t("? sin veredicto", "? no verdict"),
                };
                let _ = tablero::anotar(&ctx.tablero, crate::i18n::t("verificador", "verifier"), &format!("{cabeza} · {}", first_line(criterio)));
                self.front.log(crate::tr!("⚖ verificador: {cabeza}", "⚖ verifier: {cabeza}")).await;
                crate::tools::ToolResult { ok: true, output: crate::tr!("[verificador{en}] {cabeza}\n{informe}", "[verifier{en}] {cabeza}\n{informe}") }
            }
            Err(e) => error(crate::tr!("el verificador no terminó: {e}", "the verifier did not finish: {e}")),
        }
    }

    /// El verificador: otra conversación sobre el árbol del proyecto, en `otro` motor (con su presupuesto de
    /// contexto) o en el del chat, sin herramientas de edición.
    pub(crate) async fn correr_verificador(&self, criterio: &str, otro: Option<(&(dyn Engine + '_), usize)>, hooks: &crate::hooks::Hooks) -> Result<String> {
        let turno = tokio::sync::Mutex::new(());
        let raiz = self.raiz_del_chat();
        let perms = verificador::permisos(&self.perms, raiz.clone());
        // lo que cambie por la terminal, el punto de restauración del proyecto lo deshace
        let checkpoint = self.checkpoint.as_ref().map(|_| Checkpoint::new(&raiz));
        let front = FrenteHijo { padre: &self.front, nombre: crate::i18n::t("verificador", "verifier").into(), turno: &turno };
        let motor: &dyn Engine = match otro {
            Some((e, _)) => e,
            None => &self.engine,
        };
        let mut h = self.hijo(motor, raiz, perms, checkpoint, front);
        if let Some((_, ctx)) = otro {
            h.context_budget = ctx;
        }
        h.tools_loaded.remove("enjambre");
        let mut msgs = vec![Message::system(self.system_prompt()), Message::user(verificador::pedido(criterio))];
        bucle(&mut h, &mut msgs, hooks, 1).await
    }

    /// F9 · entre pasos: lo que otra sesión de GOTV le dejó a ésta, como aviso.
    pub(super) async fn mirar_buzon(&self, messages: &mut Vec<Message>) {
        if let Some(a) = buzon::entre_pasos(&self.raiz_del_chat(), self.session.as_ref().map(|s| s.id)) {
            self.front.log(crate::i18n::t("  ✉ llegó un mensaje de otra sesión de GOTV", "  ✉ a message arrived from another GOTV session").into()).await;
            messages.push(Message::aviso(a));
        }
    }

    /// F9 · `/rama`: el agente PRINCIPAL trabaja en su propia rama git, en una carpeta aparte; el árbol del
    /// usuario no cambia hasta cerrarla. `nueva [tema]` · `cerrar` (los cambios quedan en la rama) ·
    /// `fusionar` (con su commit de fusión en la rama de antes) · sin nada: dónde está.
    pub async fn rama_propia(&mut self, arg: &str) -> Result<String> {
        let clave = self.procs.key(Path::new(""));
        let (accion, tema) = arg.trim().split_once(char::is_whitespace).map(|(a, b)| (a, b.trim())).unwrap_or((arg.trim(), ""));
        // juez-idioma: datos (las acciones de /rama en los dos idiomas)
        let accion = match accion.to_lowercase().as_str() {
            "nueva" | "new" => "nueva",
            "cerrar" | "close" => "cerrar",
            "fusionar" | "merge" => "fusionar",
            "" => "",
            _ => return Err(anyhow!("{}", crate::i18n::t("uso: /rama nueva [tema] · /rama cerrar · /rama fusionar · /rama", "usage: /branch new [topic] · /branch close · /branch merge · /branch"))),
        };
        let activa = RAMAS.lock().unwrap().as_ref().and_then(|m| m.get(&clave).map(|r| r.wt.branch.clone()));
        match (accion, activa) {
            ("", Some(rama)) => Ok(crate::tr!("trabajas en la rama {rama}, en {} (tu árbol no cambia hasta /rama cerrar o /rama fusionar)", "you are working on branch {rama}, in {} (your tree does not change until /branch close or /branch merge)", self.root.display())),
            ("", None) => Ok(crate::i18n::t("trabajas en tu árbol, sin rama propia (/rama nueva [tema] para apartarte)", "you are working in your tree, without a branch of your own (/branch new [topic] to step aside)").into()),
            ("nueva", Some(rama)) => Err(anyhow!("{}", crate::tr!("ya trabajas en la rama {rama}: ciérrala antes", "you are already on branch {rama}: close it first"))),
            ("nueva", None) => {
                let tema = if tema.is_empty() { crate::i18n::t("principal", "main") } else { tema };
                let wt = crate::worktree::create(&self.root, tema)?;
                let raiz = std::mem::replace(&mut self.root, wt.path.clone());
                let raiz_permisos = self.perms.set_root(wt.path.clone());
                // su deshacer es la rama (el punto de restauración del proyecto no mira esa carpeta)
                let checkpoint = self.checkpoint.take();
                let txt = crate::tr!(
                    "ahora trabajas en la rama {} (carpeta {}); tu árbol no cambia hasta /rama cerrar (los cambios quedan en la rama) o /rama fusionar",
                    "you now work on branch {} (folder {}); your tree does not change until /branch close (the changes stay on the branch) or /branch merge",
                    wt.branch,
                    wt.path.display()
                );
                RAMAS.lock().unwrap().get_or_insert_with(HashMap::new).insert(clave, RamaPrincipal { wt, raiz, raiz_permisos, checkpoint });
                Ok(txt)
            }
            (_, None) => Err(anyhow!("{}", crate::i18n::t("no trabajas en una rama propia (/rama nueva [tema])", "you are not on a branch of your own (/branch new [topic])"))),
            (accion, Some(_)) => {
                let r = RAMAS.lock().unwrap().as_mut().and_then(|m| m.remove(&clave)).ok_or_else(|| anyhow!("rama"))?;
                self.root = r.raiz.clone();
                self.perms.set_root(r.raiz_permisos);
                self.checkpoint = r.checkpoint;
                let resumen = r.wt.finish(&format!("gotv: {}", r.wt.branch))?;
                if accion != "fusionar" || !ramas::existe(&self.root, &r.wt.branch) {
                    return Ok(resumen);
                }
                let f = ramas::fusionar(&self.root, &r.wt.branch, &format!("gotv: {}", r.wt.branch))?;
                Ok(ramas::informe(&[f]))
            }
        }
    }
}

#[cfg(test)]
mod tests;
