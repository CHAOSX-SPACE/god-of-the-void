//! GOD OF THE VOID · TUI (F3) — la cara viva en la terminal.
//! Chat + tool-calls + resultados inline, prompts de permiso DENTRO de la UI,
//! barra de estado. El agente corre en un worker; UI y worker hablan por canales.
//! Config por capas (F4): ~/.config/gotv/config.toml ← .gotv/config.toml ← GOTV_*.

use anyhow::Result;
use gotv_core::tr;
use async_trait::async_trait;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use gotv_core::{Agent, AskAnswer, Checkpoint, Config, Frontend, Mode, Permissions};
use gotv_engine::{AnyEngine, Engine};
use ratatui::prelude::*;
use ratatui::widgets::{Block, Paragraph, Wrap};
use std::io::IsTerminal;
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};

/// Eventos del worker (agente) → UI.
enum UiEvent {
    Line(String),
    Ask {
        desc: String,
        reply: oneshot::Sender<AskAnswer>,
    },
    /// C7: pregunta con opciones; la respuesta (número, texto libre o nada) vuelve al agente.
    Choose {
        question: String,
        options: Vec<String>,
        reply: oneshot::Sender<Option<String>>,
    },
    Done(String),
    Error(String),
    EngineUp,
    /// Texto de la respuesta mientras se genera (streaming, P3).
    Delta(String),
    /// Avance del prellenado: hecho de total.
    Prefill(u32, u32),
    /// A1: velocidad de generación en vivo (tok/s).
    Rate(f32),
    /// Métricas de la última respuesta (P6).
    Stats(gotv_core::TurnStats),
    /// Cambió el modo de permisos (`/mode`).
    Mode(String),
    /// Salida de un comando de barra: varias líneas, deja la UI lista.
    Info(String),
}

#[derive(Clone)]
struct TuiFrontend {
    tx: mpsc::UnboundedSender<UiEvent>,
}

#[async_trait]
impl Frontend for TuiFrontend {
    async fn log(&self, line: String) {
        let _ = self.tx.send(UiEvent::Line(line));
    }
    async fn ask(&self, desc: &str) -> AskAnswer {
        let (rtx, rrx) = oneshot::channel();
        // P10: la lista de un lote va a la transcripción; la barra lleva el resumen
        let head = match desc.split_once('\n') {
            Some((head, list)) => {
                let _ = self.tx.send(UiEvent::Line(format!("❓ {head}")));
                for l in list.lines() {
                    let _ = self.tx.send(UiEvent::Line(format!("  {l}")));
                }
                head.to_string()
            }
            None => desc.to_string(),
        };
        let _ = self.tx.send(UiEvent::Ask { desc: head, reply: rtx });
        rrx.await.unwrap_or(AskAnswer::No)
    }
    async fn choose(&self, question: &str, options: &[String]) -> Option<String> {
        let (rtx, rrx) = oneshot::channel();
        let _ = self.tx.send(UiEvent::Choose { question: question.to_string(), options: options.to_vec(), reply: rtx });
        rrx.await.ok().flatten()
    }
    fn delta(&self, ev: &gotv_engine::StreamEvent) {
        let _ = match ev {
            gotv_engine::StreamEvent::Text(t) => self.tx.send(UiEvent::Delta(t.clone())),
            gotv_engine::StreamEvent::Prefill { done, total } => {
                self.tx.send(UiEvent::Prefill(*done, *total))
            }
            gotv_engine::StreamEvent::Reasoning(_) | gotv_engine::StreamEvent::ToolArgs(_) => Ok(()),
        };
    }
    fn stats(&self, s: &gotv_core::TurnStats) {
        let _ = self.tx.send(UiEvent::Stats(*s));
    }
    fn activity(&self, kind: &str, detail: &str) {
        if kind == "rate" {
            if let Ok(r) = detail.parse() {
                let _ = self.tx.send(UiEvent::Rate(r));
            }
        }
    }
}

struct App {
    /// A1: velocidad en vivo de la respuesta que se está escribiendo.
    rate: Option<f32>,
    mode: Mode,
    transcript: Vec<String>,
    input: String,
    status: String,
    busy: bool,
    pending: Option<(String, oneshot::Sender<AskAnswer>)>,
    /// C7: pregunta con opciones esperando respuesta (se escribe en la misma entrada).
    choosing: Option<(Vec<String>, oneshot::Sender<Option<String>>)>,
    quit: bool,
    /// ¿La última línea es una respuesta que se está escribiendo en vivo?
    streaming: bool,
    /// Interrumpir el turno en curso (compartida con el agente).
    cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// Cuándo se pulsó Ctrl+C (dos veces seguidas = salir).
    ctrl_c_at: Option<std::time::Instant>,
    /// Métricas de la última respuesta, para la barra de estado.
    stats: Option<gotv_core::TurnStats>,
}

impl App {
    fn push(&mut self, s: impl Into<String>) {
        self.streaming = false;
        self.transcript.push(s.into());
    }

    /// Agrega texto en vivo: continúa la respuesta en curso, respetando saltos.
    fn stream_text(&mut self, t: &str) {
        if !self.streaming {
            if t.trim().is_empty() {
                return; // los saltos del arranque de turno no se pintan
            }
            self.transcript.push("🧠 ".to_string());
            self.streaming = true;
            self.append(t.trim_start());
        } else {
            self.append(t);
        }
    }

    fn append(&mut self, t: &str) {
        for (i, part) in t.split('\n').enumerate() {
            if i == 0 {
                if let Some(last) = self.transcript.last_mut() {
                    last.push_str(part);
                }
            } else {
                self.transcript.push(part.to_string());
            }
        }
    }
}

/// Arranque de un motor (al abrir y en `/model`): máquina, modelo, contexto,
/// muestreo y memoria, luego carga. Devuelve la config resuelta y el motor vivo.
async fn boot(cfg: Config, ui: &mpsc::UnboundedSender<UiEvent>) -> Result<(Config, AnyEngine), String> {
    let (cfg, prepared) = tokio::task::spawn_blocking(move || {
        let mut c = cfg;
        let r = gotv_core::launch::prepare(&mut c);
        (c, r)
    })
    .await
    .map_err(|e| e.to_string())?;
    let launch = prepared.map_err(|e| e.to_string())?;
    for n in &launch.notes {
        let _ = ui.send(UiEvent::Line(format!("· {n}")));
    }
    // el núcleo Rust dentro del proceso (sin Python): lo que aún no corre, se dice
    let dir = launch.card.as_ref().map(|c| c.path.clone()).unwrap_or_else(|| std::path::PathBuf::from(&cfg.model));
    gotv_mlx::NativoEngine::puede(&dir).map_err(|por| tr!("el núcleo de GOTV aún no corre este modelo: {por}", "GOTV's core does not run this model yet: {por}"))?;
    let mut engine = AnyEngine::Otro(Box::new(gotv_mlx::NativoEngine::new(&dir)));
    let _ = ui.send(UiEvent::Line(tr!("⏳ cargando {} …", "⏳ loading {} …", engine.name())));
    engine.start().await.map_err(|e| tr!("no arrancó el motor: {e}", "the engine did not start: {e}"))?;
    gotv_core::launch::remember_model(&cfg.model);
    let _ = ui.send(UiEvent::Line(tr!("motor {} vivo ✅ (dentro del proceso)", "engine {} up ✅ (in-process)", engine.name())));
    Ok((cfg, engine))
}

/// Precalienta y avisa cuánto tardó (P3).
async fn prewarm<F: Frontend>(agent: &mut Agent<AnyEngine, F>, ui: &mpsc::UnboundedSender<UiEvent>) {
    let _ = ui.send(UiEvent::Line(gotv_core::i18n::t("⏳ preparando el contexto del agente (una sola vez)…", "⏳ preparing the agent context (once)…").into()));
    let t = std::time::Instant::now();
    let _ = ui.send(UiEvent::Line(match agent.prewarm().await {
        Ok(()) => tr!("contexto listo en {:.1} s: las órdenes empiezan al instante ✅", "context ready in {:.1} s: orders now start instantly ✅", t.elapsed().as_secs_f32()),
        Err(e) => tr!("⚠ sin precalentar: {e}", "⚠ not prewarmed: {e}"),
    }));
}

/// P7: cambia de modelo SIN salir y sin dos modelos en RAM a la vez. Apaga el
/// actual, espera a que macOS devuelva su memoria, arranca el nuevo y conserva
/// la conversación. Si el nuevo no cabe o no arranca, vuelve al anterior.
async fn switch_model<F: Frontend>(
    agent: &mut Agent<AnyEngine, F>,
    cfg: &mut Config,
    query: &str,
    ui: &mpsc::UnboundedSender<UiEvent>,
) -> String {
    let cards = gotv_core::discover(&cfg.model_dirs);
    let card = match gotv_core::launch::find_model(&cards, query) {
        Ok(c) => c,
        Err(many) if many.is_empty() => return tr!("no hay ningún modelo ejecutable que diga «{query}» (mira /model)", "no runnable model matches «{query}» (see /model)"),
        Err(many) => return tr!("«{query}» coincide con varios; sé más preciso:\n  {}", "«{query}» matches several; be more specific:\n  {}", many.join("\n  ")),
    };
    let target = card.path.to_string_lossy().to_string();
    if target == cfg.model {
        return tr!("{} ya es el modelo cargado", "{} is already the loaded model", card.name);
    }
    let previous = cfg.clone();
    let _ = ui.send(UiEvent::Line(tr!("⏳ apagando {} para liberar la memoria…", "⏳ shutting down {} to free the memory…", agent.engine.name())));
    let _ = agent.engine.stop().await;
    gotv_core::launch::wait_memory_settles(Duration::from_secs(20)).await;

    let mut next = cfg.clone();
    next.model = target;
    match boot(next, ui).await {
        Ok((c, engine)) => {
            agent.engine = engine;
            agent.sampling = c.sampling;
            agent.context_budget = c.context_budget;
            // S3: entre agente y chat el sistema cambia; el historial del otro modo no sirve
            if agent.chat_only != c.chat_only {
                agent.chat_only = c.chat_only;
                agent.clear();
                let _ = ui.send(UiEvent::Line(gotv_core::i18n::t(
                    "· cambio entre modo agente y modo chat: conversación nueva",
                    "· switch between agent and chat mode: new conversation",
                ).into()));
            }
            *cfg = c;
            prewarm(agent, ui).await;
            let (used, budget) = agent.context_usage();
            tr!("modelo: {} ✅ · la conversación sigue ({used} de {budget} tokens)", "model: {} ✅ · the conversation continues ({used} of {budget} tokens)", card.name)
        }
        Err(e) => {
            let _ = ui.send(UiEvent::Line(format!("❌ {e}")));
            let _ = ui.send(UiEvent::Line(gotv_core::i18n::t("↩ vuelvo al modelo anterior…", "↩ going back to the previous model…").into()));
            gotv_core::launch::wait_memory_settles(Duration::from_secs(20)).await;
            match boot(previous, ui).await {
                Ok((c, engine)) => {
                    agent.engine = engine;
                    agent.sampling = c.sampling;
                    agent.context_budget = c.context_budget;
                    if agent.chat_only != c.chat_only {
                        agent.chat_only = c.chat_only;
                        agent.clear();
                    }
                    *cfg = c;
                    prewarm(agent, ui).await;
                    tr!("no pude cambiar a {}: {e}. Sigo con el anterior ✅", "could not switch to {}: {e}. Staying with the previous one ✅", card.name)
                }
                Err(e2) => tr!("no pude cambiar ({e}) ni volver al anterior ({e2}); usa /model <nombre>", "could not switch ({e}) nor go back ({e2}); use /model <name>"),
            }
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    gotv_desktop::run_if_requested();
    // los motores del núcleo (sin Python), como el CLI y la app: Whisper, el embebedor del RAG,
    // imagen, música y efectos
    gotv_core::audio::registrar_transcriptor(gotv_mlx::transcribir_wav);
    gotv_core::rag::registrar_incrustador(gotv_mlx::incrustar_textos);
    gotv_core::generadores::registrar_motor_imagen(gotv_mlx::imagen::generar_png);
    gotv_core::generadores::registrar_motor_musica(gotv_mlx::musica::generar_wav);
    gotv_core::generadores::registrar_motor_efecto(gotv_mlx::efectos::generar_wav);
    gotv_core::generadores::registrar_motor_separar(gotv_mlx::separador::separar_archivo);
    if !std::io::stdout().is_terminal() {
        eprintln!("{}", gotv_core::i18n::t("gotv-tui necesita una terminal real (tty). Usa `gotv` para el modo CLI.", "gotv-tui needs a real terminal (tty). Use `gotv` for CLI mode."));
        std::process::exit(2);
    }

    let root = std::env::current_dir().unwrap_or_default();
    let mut cfg = Config::load(&root);
    // --mode <m> override
    let a: Vec<String> = std::env::args().collect();
    if let Some(m) = a.windows(2).find(|w| w[0] == "--mode").map(|w| w[1].clone()) {
        cfg.mode = m;
    }
    if let Some(l) = a.windows(2).find(|w| w[0] == "--lang").map(|w| w[1].clone()) {
        gotv_core::i18n::set(gotv_core::i18n::resolve(&l));
        cfg.lang = l;
    }
    let mode = Mode::parse(&cfg.mode);

    let (ui_tx, mut ui_rx) = mpsc::unbounded_channel::<UiEvent>();
    let (task_tx, mut task_rx) = mpsc::unbounded_channel::<String>();
    // P5: la UI enciende esta bandera (Esc / Ctrl+C) y el agente la vigila.
    let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));

    // ── Worker del agente ───────────────────────────────────────────
    {
        let ui_tx = ui_tx.clone();
        let cfg = cfg.clone();
        let root = root.clone();
        let root_for_session = root.clone();
        let cancel_w = cancel.clone();
        tokio::spawn(async move {
            let front = TuiFrontend { tx: ui_tx.clone() };
            let _ = ui_tx.send(UiEvent::Line(gotv_core::i18n::t("⏳ leyendo la máquina y los modelos…", "⏳ reading the machine and the models…").into()));
            // El mismo arranque que el CLI (E7): máquina, modelo, contexto, memoria.
            let (mut cfg, engine) = match boot(cfg, &ui_tx).await {
                Ok(x) => x,
                Err(e) => {
                    let _ = ui_tx.send(UiEvent::Error(tr!("no puedo arrancar: {e}", "cannot start: {e}")));
                    return;
                }
            };

            let mut perms = Permissions::new(
                Mode::parse(&cfg.mode),
                &root,
                cfg.allow.clone(),
                cfg.deny.clone(),
                cfg.ask.clone(),
            );
            perms.system_changes = cfg.system_changes;
            // C2: la caja de arena y los dominios extra de la config
            perms.sandbox = cfg.sandbox;
            perms.net_extra = cfg.net_allow.clone();
            perms.sockets_extra = cfg.sockets_allow.clone();
            perms.juez_auto = cfg.auto_juez.clone();
            perms.credenciales_extra = cfg.credenciales_extra.clone();
            perms.entorno_extra = cfg.entorno_allow.clone();
            let checkpoint = Some(Checkpoint::new(&root));
            let mut agent = Agent {
                engine,
                root,
                max_steps: cfg.max_steps,
                max_tokens: cfg.max_tokens,
                sampling: cfg.sampling,
                context_budget: cfg.context_budget,
                perms,
                checkpoint,
                front,
                history: vec![],
                session: gotv_core::session::Session::open_new(&root_for_session, &cfg.model).ok(),
                cancel: cancel_w,
                mcp: None,
                attachments: Vec::new(),
                user_media: Vec::new(),
                chat_only: cfg.chat_only,
                embedder: None,
                lsp: None,
                computer_use: cfg.computer_use,
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

            // Una sola vez: calcular sistema + herramientas en la caché del motor,
            // para que cada orden empiece en < 1 s en vez de ~20 s (medido).
            prewarm(&mut agent, &ui_tx).await;
            let _ = ui_tx.send(UiEvent::EngineUp);

            while let Some(task) = task_rx.recv().await {
                // Comandos de barra (P4 + P6). Lo demás es una orden para el agente.
                if let Some(cmd) = task.trim().strip_prefix('/') {
                    let (name, arg) = cmd.split_once(' ').unwrap_or((cmd, ""));
                    let arg = arg.trim();
                    // F12 · formas de trabajar: /objetivo, /aparte, /aprender, /entrevista, /reglas. En la TUI no
                    // se escribe mientras trabaja: el objetivo se para con Esc (el turno vuelve «interrumpido»)
                    if gotv_core::formas::es_comando(name) {
                        let r = gotv_core::formas::orden(&mut agent, &task, || !task_rx.is_empty()).await;
                        let _ = ui_tx.send(match r {
                            Some(Ok(t)) => UiEvent::Info(t),
                            Some(Err(e)) => UiEvent::Error(e.to_string()),
                            None => UiEvent::Info(String::new()),
                        });
                        continue;
                    }
                    let info = match name {
                        "clear" => {
                            agent.clear();
                            gotv_core::i18n::t("conversación nueva: olvidé lo anterior ✅", "new conversation: forgot the previous one ✅").to_string()
                        }
                        "resume" => {
                            let current = agent.session.as_ref().map(|s| s.id);
                            match gotv_core::session::Session::resume_last(&root_for_session, current) {
                                Ok(Some((s, h))) => {
                                    let (n, id) = (h.len(), s.id);
                                    agent.history = h;
                                    agent.session = Some(s);
                                    tr!("retomé la conversación #{id} ({n} mensajes) ✅", "resumed conversation #{id} ({n} messages) ✅")
                                }
                                Ok(None) => gotv_core::i18n::t("no hay otra conversación guardada en este proyecto", "no other saved conversation in this project").into(),
                                Err(e) => tr!("no pude retomar: {e}", "could not resume: {e}"),
                            }
                        }
                        "mode" if arg.is_empty() => tr!("modo actual: {}", "current mode: {}", agent.perms.mode.label()),
                        "mode" => {
                            let m = Mode::parse(arg);
                            agent.perms.mode = m;
                            let _ = ui_tx.send(UiEvent::Mode(m.label().to_string()));
                            tr!("modo de permisos: {} ✅", "permission mode: {} ✅", m.label())
                        }
                        "context" => {
                            let (used, budget) = agent.context_usage();
                            tr!(
                                "contexto: ~{used} de {budget} tokens ({:.0} %) · se compacta al pasar el 75 %",
                                "context: ~{used} of {budget} tokens ({:.0} %) · compacts past 75 %",
                                used as f32 * 100.0 / budget.max(1) as f32
                            )
                        }
                        "rewind" => match &agent.checkpoint {
                            Some(cp) => match cp.rewind(arg == "--force") {
                                Ok(m) => m,
                                Err(e) => e.to_string(),
                            },
                            None => gotv_core::i18n::t("sin checkpoints en esta sesión", "no checkpoints in this session").into(),
                        },
                        "checkpoints" => match &agent.checkpoint {
                            Some(cp) => {
                                let l = cp.list();
                                if l.is_empty() {
                                    gotv_core::i18n::t("(sin pasos guardados)", "(no saved steps)").into()
                                } else {
                                    l.iter()
                                        .rev()
                                        .map(|(id, label, n)| tr!("#{id:<4} {label}  ({n} archivo(s))", "#{id:<4} {label}  ({n} file(s))"))
                                        .collect::<Vec<_>>()
                                        .join("\n")
                                }
                            }
                            None => gotv_core::i18n::t("sin checkpoints", "no checkpoints").into(),
                        },
                        // S2: adjuntar un audio a la PRÓXIMA orden (nativo si el modelo oye)
                        "audio" if !arg.is_empty() => {
                            let p = gotv_core::paths::resolve(&root_for_session, arg);
                            if !agent.engine.capabilities().audio_in {
                                gotv_core::i18n::t("este modelo/motor no oye (prueba /model con uno que oiga; o pídele al agente listen_audio: con Whisper, NO nativo)", "this model/engine cannot hear (try /model with one that hears; or ask the agent for listen_audio: with Whisper, NOT native)").to_string()
                            } else {
                                match gotv_core::audio::data_url(&p) {
                                    Ok((url, secs)) => {
                                        agent.user_media.push((arg.to_string(), url));
                                        tr!("🎧 {} adjunto ({secs:.1} s): va con tu próxima orden", "🎧 {} attached ({secs:.1} s): it goes with your next order", p.display())
                                    }
                                    Err(e) => e.to_string(),
                                }
                            }
                        }
                        // S1: adjuntar una imagen a la PRÓXIMA orden
                        "image" if !arg.is_empty() => {
                            let p = gotv_core::paths::resolve(&root_for_session, arg);
                            if !agent.engine.capabilities().vision {
                                gotv_core::i18n::t("este modelo no ve imágenes en el núcleo (prueba /model con un Qwen3.5 o Gemma 4 con su torre de visión)", "this model cannot see images in the core (try /model with a Qwen3.5 or Gemma 4 with its vision tower)").to_string()
                            } else {
                                match gotv_core::images::data_url(&p) {
                                    Ok((url, n)) => {
                                        agent.user_media.push((arg.to_string(), url));
                                        tr!("🖼  {} adjunta ({} KB): va con tu próxima orden", "🖼  {} attached ({} KB): it goes with your next order", p.display(), n / 1024)
                                    }
                                    Err(e) => e.to_string(),
                                }
                            }
                        }
                        "model" if !arg.is_empty() => {
                            let _ = ui_tx.send(UiEvent::Line(tr!("· cambiando de modelo a «{arg}»…", "· switching model to «{arg}»…")));
                            switch_model(&mut agent, &mut cfg, arg, &ui_tx).await
                        }
                        "model" => {
                            let hw = gotv_core::Hardware::detect();
                            // libre + lo del modelo cargado: al cambiar, ESO se libera
                            let avail = gotv_core::launch::avail_with_loaded(&cfg);
                            let mut out = vec![tr!("{} · disponible para un modelo {:.1} GB", "{} · available for a model {:.1} GB", hw.device, avail.unwrap_or(0) as f64 / 1e9)];
                            for c in gotv_core::discover(&cfg.model_dirs).iter().filter(|c| c.runnable) {
                                let f = gotv_core::fit(c, &hw, avail, 16384u64.min(c.max_ctx));
                                let actual = if c.path.to_string_lossy() == cfg.model { "▶ " } else { "  " };
                                let v = if f.max_ctx >= 4096 { tr!("✅ hasta {} tok", "✅ up to {} tok", f.max_ctx) } else { gotv_core::i18n::t("❌ no cabe ahora", "❌ does not fit now").into() };
                                out.push(format!("{actual}{} ({}) {v}", c.name, c.kind()));
                            }
                            out.push(gotv_core::i18n::t("/model <nombre> cambia en caliente (la conversación sigue)", "/model <name> switches live (the conversation continues)").into());
                            out.join("\n")
                        }
                        _ if gotv_core::i18n::is_en() => [
                            "Commands:",
                            "  /help                     this help",
                            "  /mode <plan|default|acceptEdits|auto|bypass>  changes permissions live",
                            "  /context                  context used and budget",
                            "  /clear                    new conversation",
                            "  /resume                   resumes the previous conversation",
                            "  /rewind [--force]         undoes the last step that changed files",
                            "  /checkpoints              saved steps",
                            "  /model                    models on this Mac and whether they fit",
                            "  /model <name>             switches model without leaving (e.g. /model 0.6b)",
                            "  /image <path>             attaches an image to the next order (vision models)",
                            "  /audio <path>             attaches audio to the next order (models that hear)",
                            "Keys: Enter sends · Esc stops the turn · Ctrl+C ×2 exits · y/n/a/x answer permissions",
                        ]
                        .join("\n")
                            + "\n"
                            + &gotv_core::formas::ayuda(),
                        // juez-idioma: par (rama ES de is_en) — bloque
                        _ => [
                            "Comandos:",
                            "  /help                     esta ayuda",
                            "  /mode <plan|default|acceptEdits|auto|bypass>  cambia los permisos en vivo",
                            "  /context                  contexto ocupado y presupuesto",
                            "  /clear                    conversación nueva",
                            "  /resume                   retoma la conversación anterior",
                            "  /rewind [--force]         deshace el último paso que cambió archivos",
                            "  /checkpoints              pasos guardados",
                            "  /model                    modelos de esta Mac y si caben",
                            "  /model <nombre>           cambia de modelo sin salir (p. ej. /model 0.6b)",
                            "  /image <ruta>             adjunta una imagen a la próxima orden (modelos con visión)",
                            "  /audio <ruta>             adjunta un audio a la próxima orden (modelos que oyen)",
                            "Teclas: Enter envía · Esc detiene el turno · Ctrl+C ×2 sale · s/n/a/x responden permisos",
                        ]
                        .join("\n")
                            + "\n"
                            + &gotv_core::formas::ayuda(),
                    };
                    let _ = ui_tx.send(UiEvent::Info(info));
                    continue;
                }
                match agent.run(&task).await {
                    Ok(ans) => {
                        let _ = ui_tx.send(UiEvent::Done(ans));
                    }
                    Err(e) => {
                        let _ = ui_tx.send(UiEvent::Error(e.to_string()));
                    }
                }
            }
            agent.cerrar_sesion("prompt_input_exit").await; // F10.1: SessionEnd al salir de la TUI
            let _ = agent.engine.stop().await;
        });
    }

    // ── UI ──────────────────────────────────────────────────────────
    let mut terminal = ratatui::init();
    let mut app = App {
        mode,
        transcript: vec![
            gotv_core::i18n::t("🕳️  GOD OF THE VOID · Dios del Vacío", "🕳️  GOD OF THE VOID").into(),
            gotv_core::i18n::t("Escribe una tarea y Enter. /help para los comandos. Esc detiene el turno. Salir: Ctrl+C dos veces.", "Type a task and press Enter. /help for commands. Esc stops the turn. Exit: Ctrl+C twice.").into(),
        ],
        input: String::new(),
        status: gotv_core::i18n::t("arrancando…", "starting…").into(),
        busy: true,
        pending: None,
        choosing: None,
        quit: false,
        streaming: false,
        cancel: cancel.clone(),
        ctrl_c_at: None,
        stats: None,
        rate: None,
    };

    let res = run_ui(&mut terminal, &mut app, &mut ui_rx, &task_tx).await;
    ratatui::restore();
    gotv_core::exec::kill_all(); // C1: ningún proceso del agente sobrevive a la TUI
    res
}

async fn run_ui(
    terminal: &mut ratatui::DefaultTerminal,
    app: &mut App,
    ui_rx: &mut mpsc::UnboundedReceiver<UiEvent>,
    task_tx: &mpsc::UnboundedSender<String>,
) -> Result<()> {
    loop {
        while let Ok(ev) = ui_rx.try_recv() {
            match ev {
                UiEvent::Line(l) => app.push(l),
                UiEvent::EngineUp => {
                    app.busy = false;
                    app.status = gotv_core::i18n::t("listo", "ready").into();
                }
                UiEvent::Ask { desc, reply } => {
                    app.pending = Some((desc, reply));
                }
                UiEvent::Choose { question, options, reply } => {
                    app.push(format!("❔ {question}"));
                    for (i, o) in options.iter().enumerate() {
                        app.push(format!("   {}. {o}", i + 1));
                    }
                    app.input.clear();
                    app.choosing = Some((options, reply));
                }
                UiEvent::Done(ans) if ans == gotv_core::agent::interrupted() => {
                    app.push(gotv_core::i18n::t("⏹ turno interrumpido: la sesión sigue, escribe otra orden", "⏹ turn interrupted: the session continues, type another order"));
                    app.busy = false;
                    app.status = gotv_core::i18n::t("listo", "ready").into();
                }
                UiEvent::Done(ans) => {
                    if app.streaming {
                        app.streaming = false; // ya se vio en vivo
                    } else {
                        app.push(format!("🧠 {ans}"));
                    }
                    app.busy = false;
                    app.status = gotv_core::i18n::t("listo", "ready").into();
                }
                UiEvent::Delta(t) => {
                    app.status = match app.rate {
                        Some(r) => tr!("escribiendo · ≈ {:.1} tok/s", "writing · ≈ {:.1} tok/s", r),
                        None => gotv_core::i18n::t("escribiendo…", "writing…").into(),
                    };
                    app.stream_text(&t);
                }
                UiEvent::Rate(r) => app.rate = Some(r),
                UiEvent::Prefill(done, total) if total > 1 => {
                    app.status = tr!("leyendo contexto {} %", "reading context {} %", done as u64 * 100 / total as u64);
                }
                UiEvent::Prefill(..) => {}
                UiEvent::Stats(st) => {
                    app.stats = Some(st);
                    app.rate = None; // la próxima respuesta mide la suya
                }
                UiEvent::Mode(m) => app.mode = Mode::parse(&m),
                UiEvent::Info(text) => {
                    // F12: un /objetivo o /entrevista termina con Info tras turnos que se vieron en vivo
                    app.streaming = false;
                    for line in text.lines() {
                        app.push(line.to_string());
                    }
                    app.busy = false;
                    app.status = gotv_core::i18n::t("listo", "ready").into();
                }
                UiEvent::Error(e) => {
                    app.push(format!("✗ error: {e}"));
                    app.busy = false;
                    app.status = "error".into();
                }
            }
        }

        terminal.draw(|f| draw(f, app))?;

        if event::poll(Duration::from_millis(50))? {
            if let Event::Key(k) = event::read()? {
                if k.kind != KeyEventKind::Press {
                    continue;
                }
                let ctrl_c = k.code == KeyCode::Char('c') && k.modifiers.contains(KeyModifiers::CONTROL);
                if ctrl_c {
                    // 1.ª vez: interrumpe (si hay turno) y avisa; 2.ª en 3 s: sale.
                    let again = app
                        .ctrl_c_at
                        .map(|t| t.elapsed() < Duration::from_secs(3))
                        .unwrap_or(false);
                    if again {
                        app.quit = true;
                    } else {
                        app.ctrl_c_at = Some(std::time::Instant::now());
                        if app.busy {
                            app.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                            app.push(gotv_core::i18n::t("⏹ interrumpiendo el turno…", "⏹ interrupting the turn…"));
                        }
                        app.status = gotv_core::i18n::t("Ctrl+C otra vez para salir", "Ctrl+C again to exit").into();
                    }
                    if let Some((_, reply)) = app.pending.take() {
                        let _ = reply.send(AskAnswer::No);
                    }
                    if let Some((_, reply)) = app.choosing.take() {
                        let _ = reply.send(None);
                    }
                } else if app.choosing.is_some() {
                    match k.code {
                        KeyCode::Enter | KeyCode::Esc => {
                            let (opts, reply) = app.choosing.take().expect("choosing");
                            let typed = if k.code == KeyCode::Esc { String::new() } else { app.input.trim().to_string() };
                            app.input.clear();
                            let ans = pick(&opts, &typed);
                            app.push(match &ans {
                                Some(a) => tr!("   ➜ respondiste: {}", "   ➜ you answered: {}", a),
                                None => gotv_core::i18n::t("   ➜ sin respuesta: el agente decide", "   ➜ no answer: the agent decides").to_string(),
                            });
                            let _ = reply.send(ans);
                        }
                        KeyCode::Backspace => {
                            app.input.pop();
                        }
                        KeyCode::Char(c) if !k.modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) => app.input.push(c),
                        _ => {}
                    }
                } else if let Some((_, reply)) = app.pending.take() {
                    let ans = match k.code {
                        KeyCode::Char('s') | KeyCode::Char('y') => AskAnswer::Yes,
                        KeyCode::Char('a') => AskAnswer::Always,
                        KeyCode::Char('x') => AskAnswer::Never,
                        _ => AskAnswer::No,
                    };
                    app.push(tr!("   ➜ respondiste: {}", "   ➜ you answered: {}", ans_label(&ans)));
                    let _ = reply.send(ans);
                } else {
                    // Medido (TUI manejada por expect): un Enter que llega como Ctrl+J
                    // se escribía como «j» y se mandaba de orden. Con Ctrl/Alt una letra
                    // NO es texto: Ctrl+J / Ctrl+M son Enter; Ctrl+U y Ctrl+W, como en la shell.
                    let ctrl = k.modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT);
                    let code = match k.code {
                        KeyCode::Char('j') | KeyCode::Char('m') if ctrl => KeyCode::Enter,
                        other => other,
                    };
                    match code {
                        KeyCode::Char('u') if ctrl => app.input.clear(),
                        KeyCode::Char('w') if ctrl => {
                            let t = app.input.trim_end().to_string();
                            let cut = t.rfind(' ').map(|i| i + 1).unwrap_or(0);
                            app.input.truncate(cut);
                        }
                        KeyCode::Char(_) if ctrl => {}
                        // Esc: detiene el turno (como Claude Code); en reposo limpia la entrada.
                        KeyCode::Esc if app.busy => {
                            app.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                            app.push(gotv_core::i18n::t("⏹ interrumpiendo el turno…", "⏹ interrupting the turn…"));
                        }
                        KeyCode::Esc => app.input.clear(),
                        KeyCode::Enter => {
                            let t = app.input.trim().to_string();
                            if !t.is_empty() && !app.busy {
                                app.push(format!("» {t}"));
                                app.busy = true;
                                app.status = gotv_core::i18n::t("trabajando…", "working…").into();
                                let _ = task_tx.send(t);
                                app.input.clear();
                            }
                        }
                        KeyCode::Backspace => {
                            app.input.pop();
                        }
                        KeyCode::Char(c) => app.input.push(c),
                        _ => {}
                    }
                }
            }
        }

        if app.quit {
            break;
        }
    }
    Ok(())
}

fn ans_label(a: &AskAnswer) -> &'static str {
    match a {
        AskAnswer::Yes => gotv_core::i18n::t("sí", "yes"),
        AskAnswer::No => "no",
        AskAnswer::Always => gotv_core::i18n::t("siempre", "always"),
        AskAnswer::Never => gotv_core::i18n::t("nunca", "never"),
    }
}

fn draw(f: &mut Frame, app: &App) {
    let chunks = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(3),
    ])
    .split(f.area());

    let status = Line::from(vec![
        Span::styled(
            " GOD OF THE VOID ",
            Style::new().fg(Color::Black).bg(Color::Magenta).bold(),
        ),
        Span::raw(tr!(" modo {} · {} ", " mode {} · {} ", app.mode.label(), app.status)),
        Span::styled(
            app.stats.map(|s| format!("· {}", s.summary())).unwrap_or_default(),
            Style::new().fg(Color::DarkGray),
        ),
    ]);
    f.render_widget(Paragraph::new(status), chunks[0]);

    let height = chunks[1].height.saturating_sub(2) as usize;
    let total = app.transcript.len();
    let start = total.saturating_sub(height);
    let text: Vec<Line> = app.transcript[start..].iter().map(|l| style_line(l)).collect();
    f.render_widget(
        Paragraph::new(text)
            .block(Block::bordered().title(gotv_core::i18n::t(" vacío ", " void ")))
            .wrap(Wrap { trim: false }),
        chunks[1],
    );

    let bottom = if let Some((opts, _)) = &app.choosing {
        Paragraph::new(format!("> {}", app.input)).block(Block::bordered().title(tr!(
            " elige 1-{} o escribe otra respuesta · Esc = que decida ",
            " pick 1-{} or type another answer · Esc = let it decide ",
            opts.len()
        )))
    } else if let Some((desc, _)) = &app.pending {
        Paragraph::new(Line::from(vec![
            Span::styled("❓ ", Style::new().fg(Color::Yellow).bold()),
            Span::raw(tr!("permitir [{desc}]  ", "allow [{desc}]  ")),
            Span::styled(gotv_core::i18n::t("s", "y"), Style::new().fg(Color::Green)),
            Span::raw(gotv_core::i18n::t("=sí ", "=yes ")),
            Span::styled("n", Style::new().fg(Color::Red)),
            Span::raw("=no "),
            Span::styled("a", Style::new().fg(Color::Green)),
            Span::raw(gotv_core::i18n::t("=siempre ", "=always ")),
            Span::styled("x", Style::new().fg(Color::Red)),
            Span::raw(gotv_core::i18n::t("=nunca", "=never")),
        ]))
        .block(Block::bordered().title(gotv_core::i18n::t(" permiso ", " permission ")))
    } else {
        Paragraph::new(format!("> {}", app.input))
            .block(Block::bordered().title(if app.busy { gotv_core::i18n::t(" trabajando… ", " working… ") } else { gotv_core::i18n::t(" tarea ", " task ") }))
    };
    f.render_widget(bottom, chunks[2]);
}

/// C7: «2» → la opción 2; texto → respuesta libre; vacío o número fuera de rango → libre/nada.
fn pick(options: &[String], typed: &str) -> Option<String> {
    let t = typed.trim();
    if t.is_empty() {
        return None;
    }
    match t.parse::<usize>() {
        Ok(n) if (1..=options.len()).contains(&n) => Some(options[n - 1].clone()),
        _ => Some(t.to_string()),
    }
}

fn style_line(l: &str) -> Line<'_> {
    let style = if l.starts_with('»') {
        Style::new().fg(Color::Cyan).bold()
    } else if l.starts_with("🧠") {
        Style::new().fg(Color::Green).bold()
    } else if l.contains("🚫") || l.starts_with("✗") {
        Style::new().fg(Color::Red)
    } else if l.starts_with("  │+") {
        Style::new().fg(Color::Green)
    } else if l.starts_with("  │-") {
        Style::new().fg(Color::Red)
    } else if l.starts_with("  ┌") || l.starts_with("  │ @@") {
        Style::new().fg(Color::Cyan)
    } else if l.starts_with("  ↩") || l.starts_with('⏹') {
        Style::new().fg(Color::Magenta)
    } else if l.trim_start().starts_with('·') {
        Style::new().fg(Color::Gray)
    } else {
        Style::new()
    };
    Line::styled(l.to_string(), style)
}

#[cfg(test)]
mod tests {
    use super::pick;

    #[test]
    fn elegir_por_numero_texto_o_nada() {
        let o = vec!["React".to_string(), "Vue".to_string()];
        assert_eq!(pick(&o, "2").as_deref(), Some("Vue"));
        assert_eq!(pick(&o, " 1 ").as_deref(), Some("React"));
        assert_eq!(pick(&o, "Svelte").as_deref(), Some("Svelte"), "respuesta libre");
        assert_eq!(pick(&o, "7").as_deref(), Some("7"), "fuera de rango = texto tal cual");
        assert_eq!(pick(&o, "  "), None, "vacío = que decida el agente");
    }
}
