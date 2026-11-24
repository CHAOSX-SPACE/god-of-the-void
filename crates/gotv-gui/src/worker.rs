//! El trabajador de la GUI: dueño del motor y del agente. Atiende, en orden:
//! órdenes, cambio de proyecto (mismo motor, agente nuevo, retoma su última
//! conversación), cambio de modelo (apaga → espera la RAM → carga; si falla, vuelve
//! al anterior), ajustes en vivo y `/compact`.

use crate::attach::Payload;
use crate::{emit, GuiFrontend, Hub};
use gotv_core::{Agent, Checkpoint, Config, Mode, Permissions};
use gotv_engine::{AnyEngine, Engine};
use serde_json::json;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::mpsc;

pub enum Msg {
    /// Una orden del chat; `bool` = el check «Control Mac» estaba encendido; el texto = el
    /// perfil elegido junto al botón de adjuntar (auto · programar · diseno · imagen · video · musica).
    Task(String, bool, Option<String>, Option<(String, usize)>, Box<gotv_core::pro::Pro>),
    Open(PathBuf),
    /// F4: se encendió «Reabrir rápido» en el proyecto abierto: precargar ya
    Rapido,
    Model(String),
    Apply(serde_json::Value),
    /// Ola EXPERTO · EXP-5: una orden del chat del estudio (no pasa por el candado ni por el historial)
    Experto(String, tokio::sync::oneshot::Sender<Result<serde_json::Value, String>>),
    Close,
    /// F8.1: pasar a otro chat del proyecto abierto (`None` = uno nuevo)
    Chat(Option<i64>),
    /// F8.3: los recursos de los servidores MCP activos, para ofrecerlos con @ (sólo con el agente libre)
    Recursos(tokio::sync::oneshot::Sender<Vec<(String, String, String, String)>>),
}

/// Sin proyecto: el motor ya está cargado, pero no hay carpeta donde trabajar. Espera
/// a que el usuario cree o abra uno; lo demás se responde con el aviso.
async fn wait_project(hub: &Hub, rx: &mut mpsc::UnboundedReceiver<Msg>) -> Option<PathBuf> {
    *hub.root.lock().await = PathBuf::new();
    // el motor YA está cargado: la ventana debe saberlo aunque no haya proyecto
    // (medido 2026-11-09: sin este aviso el pie quedaba en «…» y parecía que el
    // modelo no cargaba, con 18 GB ya en memoria)
    let snapshot = {
        let mut s = hub.status.lock().await;
        s["root"] = json!("");
        s["ready"] = json!(true);
        s.clone()
    };
    emit(&hub.app, json!({"kind": "status", "status": snapshot}));
    emit(&hub.app, json!({"kind": "noproject"}));
    while let Some(m) = rx.recv().await {
        match m {
            Msg::Open(p) => return Some(p),
            _ => emit(&hub.app, json!({"kind": "done", "text": gotv_core::i18n::t("Primero crea un proyecto (＋ Nuevo proyecto).", "First create a project (＋ New project)."), "info": true})),
        }
    }
    None
}

/// Al abrir la app: SIN proyecto elegido y SIN motor (pedido del Portador 2026-11-15: «quiero que al abrir
/// no preseleccione ningún chat ni inicie cargas… así la app abre más ligera»). El modelo, su ventana y la
/// memoria del chat se cargan cuando se elige un proyecto. Elegir modelo aquí sólo lo recuerda.
async fn en_reposo(hub: &Hub, rx: &mut mpsc::UnboundedReceiver<Msg>) -> Option<PathBuf> {
    *hub.root.lock().await = PathBuf::new();
    let snapshot = {
        let mut s = hub.status.lock().await;
        s["root"] = json!("");
        s["ready"] = json!(true);
        s["reposo"] = json!(true);
        s.clone()
    };
    emit(&hub.app, json!({"kind": "status", "status": snapshot}));
    emit(&hub.app, json!({"kind": "reposo"}));
    while let Some(m) = rx.recv().await {
        match m {
            Msg::Open(p) => {
                hub.status.lock().await["reposo"] = json!(false);
                return Some(p);
            }
            Msg::Model(path) => {
                let mut st = crate::store::Store::load();
                st.model = Some(path);
                st.save();
                emit(&hub.app, json!({"kind": "done", "text": gotv_core::i18n::t("Modelo elegido: se carga al abrir un proyecto.", "Model chosen: it loads when you open a project."), "info": true}));
            }
            _ => emit(&hub.app, json!({"kind": "done", "text": gotv_core::i18n::t("Primero abre un proyecto: su modelo se carga al abrirlo.", "First open a project: its model loads when you open it."), "info": true})),
        }
    }
    None
}

pub(crate) type Ag = Agent<AnyEngine, GuiFrontend>;

fn new_agent(engine: AnyEngine, root: &std::path::Path, cfg: &Config, front: GuiFrontend, cancel: Arc<std::sync::atomic::AtomicBool>, resume: bool) -> Ag {
let mut perms = Permissions::new(Mode::parse(&cfg.mode), root, cfg.allow.clone(), cfg.deny.clone(), cfg.ask.clone());
    perms.system_changes = cfg.system_changes;
    // C2: la caja de arena y los dominios extra de la config
    perms.sandbox = cfg.sandbox;
    perms.net_extra = cfg.net_allow.clone();
    perms.sockets_extra = cfg.sockets_allow.clone();
    perms.juez_auto = cfg.auto_juez.clone();
    perms.credenciales_extra = cfg.credenciales_extra.clone();
    perms.entorno_extra = cfg.entorno_allow.clone();
    perms.desktop_control = cfg.desktop_control;
    let (session, history) = match resume.then(|| gotv_core::session::Session::resume_last(root, None)).and_then(|r| r.ok()).flatten() {
        Some((s, h)) => (Some(s), h),
        None => (gotv_core::session::Session::open_new(root, &cfg.model).ok(), vec![]),
    };
    Agent {
        engine,
        root: root.to_path_buf(),
        max_steps: cfg.max_steps,
        max_tokens: cfg.max_tokens,
        sampling: cfg.sampling,
        context_budget: cfg.context_budget,
        perms,
        checkpoint: Some(Checkpoint::new(root)),
        front,
        history,
        session,
        cancel,
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
    }
}

/// La conversación que retomó el proyecto, para pintarla (sólo usuario y respuestas).
/// Tope de las imágenes y audios que viajan enteros al reabrir (cada tarjeta lleva su PNG o WAV en base64): pasado
/// este total, las más antiguas salen como una línea con su ruta para no ahogar la ventana.
const VISTAS_MAX_BYTES: usize = 48 << 20;

fn history_view(a: &Ag) -> serde_json::Value {
    let esquemas = a.all_tool_schemas();
    let h = &a.history;
    // las imágenes generadas, de la más nueva a la más vieja, hasta el tope
    let mut vistas: Vec<Option<serde_json::Value>> = vec![None; h.len()];
    let mut bytes = 0usize;
    for i in (0..h.len()).rev() {
        // la imagen o el audio (música, efecto) que se generó en ese mensaje
        let vista = gotv_core::mensajes::vista_de_imagen(h, i, &a.root).or_else(|| gotv_core::mensajes::vista_de_audio(h, i, &a.root)).or_else(|| gotv_core::mensajes::vista_de_video(h, i, &a.root))
            // F4: la última tarjeta de cada notebook, con su estado de hoy
            .or_else(|| gotv_core::cuaderno::vista::de_historial(h, i, &a.root));
        if let Some(mut v) = vista {
            let n = ["html", "audio", "imagen"].iter().map(|k| v[*k].as_str().map_or(0, str::len)).sum::<usize>();
            if bytes + n <= VISTAS_MAX_BYTES {
                bytes += n;
                v["role"] = json!("vista");
            } else {
                let icono = if v["tipo"] == "audio" { "♪" } else { "🖼" };
                v = json!({"role": "assistant", "text": format!("{icono} `{}`", v["ruta"].as_str().unwrap_or(""))});
            }
            // F8.1: su lugar en el historial (para bifurcar desde ahí)
            v["i"] = json!(i);
            vistas[i] = Some(v);
        }
    }
    // lo que se vio en vivo: tu mensaje sin las skills ni la instrucción de voz; la respuesta sin las llamadas
    // escritas como texto ni la ubicación/licencia de lo generado. `None` = no se pinta (herramientas, adjuntos…)
    let texto = |m: &gotv_engine::Message| -> Option<String> {
        if !((m.role == "user" || m.role == "assistant") && m.tool_calls.is_none() && !m.content.trim().is_empty())
            || gotv_core::mensajes::es_adjuntos_pedidos(m)
            || gotv_core::mensajes::es_memoria_de_trabajo(m)
        {
            return None;
        }
        let t = if m.role == "assistant" {
            gotv_core::mensajes::sin_ubicaciones(&gotv_core::recover::strip_calls_en(m.content.split("\n\n(Skills").next().unwrap_or(&m.content), &esquemas))
        } else {
            gotv_core::mensajes::visible(m).to_string()
        };
        (!t.trim().is_empty()).then_some(t)
    };
    let mut items = Vec::new();
    // lo que la compactación sacó de la vista del modelo, arriba y SIN `i` (es historia: no se bifurca ni se
    // rebobina desde ahí). Antes, al reabrir la app, desaparecía (medido por el Portador, 2026-11-18).
    if let Some(s) = &a.session {
        for m in s.archivo() {
            if let Some(t) = texto(&m) {
                items.push(json!({"role": m.role, "text": t}));
            }
        }
        if !items.is_empty() {
            items.push(json!({"role": "compactado"}));
        }
    }
    for (i, m) in h.iter().enumerate() {
        // la tarjeta de la imagen, donde se vio en vivo (tras su `generar_imagen`)
        if let Some(v) = vistas[i].take() {
            items.push(v);
            continue;
        }
        if let Some(t) = texto(m) {
            // F8.1: `i`, su lugar en el historial (bifurcar desde ahí)
            items.push(json!({"role": m.role, "text": t, "i": i}));
        }
    }
    serde_json::Value::Array(items)
}

async fn hub_model_path(hub: &Hub) -> String {
    hub.model_path.lock().await.clone()
}

pub use gotv_core::launch::{avail_with_loaded, loaded_card};

pub(crate) async fn publish_status(hub: &Hub, a: &Ag, model: &str) {
    // C1: la lista de procesos de la ventana es la de ESTE chat
    *hub.procs_owner.lock().unwrap() = a.proc_owner();
    let (used, budget) = a.context_usage();
    let caps = a.engine.capabilities();
    *hub.caps.lock().await = (caps.vision, caps.audio_in);
    // A4: el servicio de API sirve SIEMPRE el modelo local que la app tiene cargado, por el asa del
    // núcleo (el mismo modelo en memoria, sin cargar otro)
    let local = match a.engine.compartir().filter(|_| a.engine.is_local()) {
        Some(asa) => {
            let path = hub_model_path(hub).await;
            Some(gotv_core::serve::local_upstream(&gotv_core::serve::public_name(&path), &path, asa, caps.vision, 0))
        }
        None => None,
    };
    hub.api.set_local(local);
    let mut s = hub.status.lock().await;
    s["model"] = json!(model);
    s["mode"] = json!(a.perms.mode.label());
    s["root"] = json!(a.root.display().to_string());
    // F8.1: el chat abierto (la lista de la izquierda lo marca)
    s["chat"] = json!(a.session.as_ref().map(|x| x.id));
    s["ctx_used"] = json!(used);
    s["ctx_budget"] = json!(budget);
    s["vision"] = json!(caps.vision);
    s["hears"] = json!(caps.audio_in);
    s["chat_only"] = json!(a.chat_only);
    s["model_path"] = json!(hub_model_path(hub).await);
    s["max_steps"] = json!(a.max_steps);
    let snapshot = s.clone();
    drop(s);
    emit(&hub.app, json!({"kind": "status", "status": snapshot}));
}

/// F4 de la memoria rápida: deja la conversación del proyecto LISTA antes del primer mensaje si el
/// proyecto tiene «Reabrir rápido» y el motor es el núcleo nativo (lee del disco o, si no hay memoria,
/// la construye con el modelo y la guarda). Si no, el calentamiento de siempre (sistema + «hola»).
/// Emite `{kind:"precarga", fase:"inicio"|"avance"|"lista"|"cancelada"|"error", …}` para la ventana de
/// progreso. «Cancelar» = `stop_turn` (la bandera `cancel` del hub).
async fn calentar(hub: &Hub, agent: &mut Ag) {
    // ANTES de precargar: la conversación lista para ESTE modelo (reglas al día y compactada si no
    // cabe cómoda). Así la memoria rápida se arma sobre lo que la primera orden va a usar, y la
    // primera orden no compacta. Pedido del Portador tras cambiar 9B ↔ Sonnet (2026-11-11).
    hub.cancel.store(false, std::sync::atomic::Ordering::Relaxed);
    // el modelo de chat se descargó para un video: vuelve antes de calentar
    if agent.motor_dormido && agent.engine.start().await.is_ok() {
        agent.motor_dormido = false;
    }
    if agent.preparar_conversacion().await {
        emit(&hub.app, json!({"kind": "line", "text": gotv_core::i18n::t("· conversación preparada para este modelo (compactada y guardada)", "· conversation prepared for this model (compacted and saved)")}));
    }
    let nativo = matches!(agent.engine, AnyEngine::Otro(_)) && agent.engine.local();
    if !(nativo && gotv_core::conversaciones::activa(&agent.root) && !agent.history.is_empty()) {
        let _ = agent.prewarm().await;
        return;
    }
    let app = hub.app.clone();
    let t = std::time::Instant::now();
    emit(&app, json!({"kind": "precarga", "fase": "inicio"}));
    hub.cancel.store(false, std::sync::atomic::Ordering::Relaxed);
    let cancel = hub.cancel.clone();
    let avance = {
        let app = app.clone();
        move |e: gotv_engine::StreamEvent| {
            if let gotv_engine::StreamEvent::Prefill { done, total } = e {
                emit(&app, json!({"kind": "precarga", "fase": "avance", "done": done, "total": total}));
            }
        }
    };
    let cancelada = async move {
        while !cancel.load(std::sync::atomic::Ordering::Relaxed) {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    };
    let r = tokio::select! {
        r = agent.precargar(&avance) => Some(r),
        _ = cancelada => None,
    };
    hub.cancel.store(false, std::sync::atomic::Ordering::Relaxed);
    let seg = t.elapsed().as_secs_f32();
    match r {
        None => emit(&app, json!({"kind": "precarga", "fase": "cancelada"})),
        Some(Err(e)) => emit(&app, json!({"kind": "precarga", "fase": "error", "error": e.to_string()})),
        Some(Ok(p)) if !p.admitida => {
            // esta plantilla no deja precargar: el calentamiento de siempre
            let _ = agent.prewarm().await;
            emit(&app, json!({"kind": "precarga", "fase": "lista", "tokens": 0, "seg": seg}));
        }
        Some(Ok(p)) => emit(
            &app,
            json!({"kind": "precarga", "fase": "lista", "tokens": p.tokens, "del_disco": p.del_disco, "calculados": p.calculados, "bytes": p.bytes, "seg": seg, "nota": p.nota}),
        ),
    }
}

/// Arranca un motor con esta config (prepara memoria/contexto, como el CLI).
async fn boot(cfg: Config, hub: &Hub) -> Result<(Config, AnyEngine, String), String> {
    // A3: un modelo en la nube no carga nada en esta Mac
    if gotv_core::nube::is_cloud(&cfg.model, &cfg) {
        let tid = hub.task_begin("engine", &gotv_core::tr!("conectando {}", "connecting {}", cfg.model)).await;
        let mut c = cfg;
        return match gotv_core::nube::launch(&mut c).await {
            Ok((engine, name, notes)) => {
                for n in notes {
                    emit(&hub.app, json!({"kind": "line", "text": format!("· {n}")}));
                }
                hub.task_end(tid, "done", gotv_core::i18n::t("conectado", "connected")).await;
                gotv_core::launch::remember_model(&c.model);
                Ok((c, engine, name))
            }
            Err(e) => {
                hub.task_end(tid, "error", &e.to_string()).await;
                Err(gotv_core::tr!("no conecté con {}: {e}", "could not connect to {}: {e}", c.model))
            }
        };
    }
    let (cfg, prepared) = tokio::task::spawn_blocking(move || {
        let mut c = cfg;
        // ABRIR LA APP NO PIDE NINGÚN PERMISO: el arranque nunca recorre una carpeta de
        // modelos protegida (Documentos/Escritorio/Descargas/iCloud/discos), ni aunque el
        // usuario ya tenga un proyecto confiado ahí. macOS sólo pregunta al CREAR/ABRIR un
        // proyecto. Los modelos de ~/.lmstudio y ~/.cache/huggingface (no protegidos) sí se
        // ven; los que el usuario guarda en Documentos aparecen al abrir Configuración →
        // Modelo. (Medido 2026-11-09: el proyecto en Documentos concedía toda la carpeta y el
        // arranque escaneaba los modelos de ahí → pedía Documentos en CADA inicio.)
        let all_dirs = c.model_dirs.clone();
        c.model_dirs = crate::acceso::model_dirs_allowed(&all_dirs, &Default::default());
        let r = gotv_core::launch::prepare(&mut c);
        c.model_dirs = all_dirs;
        (c, r)
    })
    .await
    .map_err(|e| e.to_string())?;
    let launch = prepared.map_err(|e| e.to_string())?;
    for n in &launch.notes {
        emit(&hub.app, json!({"kind": "line", "text": format!("· {n}")}));
    }
    // el motor es SIEMPRE el núcleo Rust (sin Python desde 2026-11-12): lo que aún no corre, se dice
    let dir = launch.card.as_ref().map(|c| c.path.clone()).unwrap_or_else(|| std::path::PathBuf::from(&cfg.model));
    gotv_mlx::NativoEngine::puede(&dir).map_err(|por| gotv_core::tr!("el núcleo de GOTV aún no corre este modelo: {por}", "GOTV's core does not run this model yet: {por}"))?;
    // un sentido que el modelo trae y el núcleo aún no le da con esa familia: trabaja sin él (el audio
    // le llega como transcripción de Whisper)
    if let Some(por) = launch.card.as_ref().and_then(|c| gotv_mlx::sentido_que_falta(&dir, c.vision, c.audio)) {
        emit(&hub.app, json!({"kind": "line", "text": gotv_core::tr!("· {por}: trabaja sin ese sentido", "· {por}: it works without that sense")}));
    }
    let mut engine = AnyEngine::Otro(Box::new(gotv_mlx::NativoEngine::new(&dir)));
    let name = launch.card.as_ref().map(|c| c.name.clone()).unwrap_or_else(|| cfg.model.clone());
    let tid = hub.task_begin("engine", &gotv_core::tr!("cargando {name}", "loading {name}")).await;
    let arranque = engine.start().await;
    match arranque {
        Ok(()) => {
            hub.task_end(tid, "done", gotv_core::i18n::t("vivo", "up")).await;
            gotv_core::launch::remember_model(&cfg.model);
            Ok((cfg, engine, name))
        }
        Err(e) => {
            hub.task_end(tid, "error", &e.to_string()).await;
            Err(gotv_core::tr!("no arrancó el motor: {e}", "the engine did not start: {e}"))
        }
    }
}

/// Apaga el motor, espera a que macOS libere la memoria y arranca otro con `next`;
/// si falla, vuelve a `previous`. Deja el agente apuntando al motor que quedó vivo.
async fn restart(hub: &Hub, agent: &mut Ag, next: Config, previous: Config) -> Result<(Config, String), (String, String)> {
    let say = |t: String| emit(&hub.app, json!({"kind": "line", "text": t}));
    let _ = agent.engine.stop().await;
    gotv_core::launch::wait_memory_settles(std::time::Duration::from_secs(20)).await;
    let (c, engine, name) = match boot(next, hub).await {
        Ok(x) => x,
        Err(e) => {
            say(format!("❌ {e}"));
            say(gotv_core::i18n::t("↩ vuelvo a lo anterior…", "↩ going back to the previous setup…").into());
            gotv_core::launch::wait_memory_settles(std::time::Duration::from_secs(20)).await;
            boot(previous, hub).await.map_err(|e2| (e, e2))?
        }
    };
    agent.engine = engine;
    agent.sampling = c.sampling;
    agent.context_budget = c.context_budget;
    *hub.model_path.lock().await = c.model.clone();
    Ok((c, name))
}

/// El modelo elegido en la app, si sigue siendo usable: uno de nube con su proveedor
/// configurado, o una carpeta local que todavía existe.
fn chosen_model(saved: Option<String>, cfg: &Config) -> Option<String> {
    let m = saved.filter(|m| !m.trim().is_empty())?;
    if gotv_core::nube::is_cloud(&m, cfg) || std::path::Path::new(&m).is_dir() {
        Some(m)
    } else {
        None
    }
}

/// F5: el modelo con el que se abre un proyecto. Manda un `model =` explícito en su config (otra
/// elección tuya: entonces `None`, se queda el de la config); si no, el del PROYECTO; si no, el último
/// que elegiste. Un modelo que ya no está (borrado, proveedor sin llave) cae al siguiente.
fn modelo_de(root: Option<&std::path::Path>, cfg: &Config, st: &crate::store::Store) -> Option<String> {
    if !cfg.model.is_empty() {
        return None;
    }
    root.and_then(|r| chosen_model(st.project_model(r), cfg)).or_else(|| chosen_model(st.model.clone(), cfg))
}

#[cfg(test)]
mod pruebas_modelo_del_proyecto {
    use super::*;

    #[test]
    fn el_modelo_del_proyecto_manda_y_uno_borrado_cae_al_siguiente() {
        // la guardia borra la raíz al soltarse, pase la prueba o falle
        let raiz = gotv_engine::prueba::CarpetaPrueba::nueva("gotv-modelo-proyecto");
        let (m9, m32) = (raiz.join("9B"), raiz.join("32B"));
        std::fs::create_dir_all(&m9).unwrap();
        std::fs::create_dir_all(&m32).unwrap();
        let (web, api) = (std::path::Path::new("/p/web"), std::path::Path::new("/p/api"));
        let mut st = crate::store::Store::default();
        st.set_project_model(web, &m32.to_string_lossy());
        st.model = Some(m9.to_string_lossy().into());
        let cfg = Config::default();
        assert_eq!(modelo_de(Some(web), &cfg, &st), Some(m32.to_string_lossy().into()), "el del proyecto");
        assert_eq!(modelo_de(Some(api), &cfg, &st), Some(m9.to_string_lossy().into()), "sin modelo propio: el último elegido");
        assert_eq!(modelo_de(None, &cfg, &st), Some(m9.to_string_lossy().into()), "sin proyecto: el último elegido");
        // un `model =` explícito en la config es otra elección del Portador: no se toca
        let explicito = Config { model: "/otro".into(), ..Config::default() };
        assert_eq!(modelo_de(Some(web), &explicito, &st), None);
        // el modelo del proyecto se borró del disco: cae al último elegido
        std::fs::remove_dir_all(&m32).unwrap();
        assert_eq!(modelo_de(Some(web), &cfg, &st), Some(m9.to_string_lossy().into()));
    }
}


/// F5: al abrir un proyecto con OTRO modelo, se cambia (un solo modelo cargado a la vez: `restart`
/// apaga el actual, espera la memoria y enciende el del proyecto; si no carga, vuelve al anterior y NO
/// se pisa el modelo guardado del proyecto). Devuelve si cambió.
async fn ajustar_modelo(hub: &Hub, agent: &mut Ag, cfg: &mut Config, model: &mut String, engine_ctx: &mut usize) -> bool {
    let pc = Config::load(&agent.root);
    let st = crate::store::Store::load();
    let Some(quiere) = modelo_de(Some(&agent.root), &pc, &st).filter(|m| *m != cfg.model) else { return false };
    let say = |t: String| emit(&hub.app, json!({"kind": "line", "text": t}));
    say(gotv_core::tr!("⏳ este proyecto usa {quiere}: cambio de modelo…", "⏳ this project uses {quiere}: switching model…"));
    let mut next = cfg.clone();
    next.model = quiere;
    // con la ventana de ESTE proyecto desde el arranque (si no, cargaría y luego recargaría)
    next.context_preset = contexto_de(&agent.root, &st);
    match restart(hub, agent, next, cfg.clone()).await {
        Ok((c, name)) => {
            // el modo (agente/chat) y el muestreo son del MODELO, no del proyecto anterior
            agent.chat_only = c.chat_only;
            *cfg = c;
            *engine_ctx = cfg.context_budget;
            *model = name;
            true
        }
        Err((e, e2)) => {
            say(gotv_core::tr!("❌ no pude cargar el modelo del proyecto ({e}) ni volver ({e2})", "❌ could not load the project model ({e}) nor go back ({e2})"));
            false
        }
    }
}

/// La ventana de contexto con la que se abre un proyecto: la que se le eligió (cada proyecto puede usar
/// otra IA y otra ventana: pedido del Portador 2026-11-13); si no, la de la configuración.
fn contexto_de(root: &std::path::Path, st: &crate::store::Store) -> String {
    st.project_context(root).unwrap_or_else(|| Config::load(root).context_preset)
}

/// Aplica una ventana de contexto al modelo cargado y devuelve lo que se le dice al usuario (`None` si
/// falló; el error ya se dijo). Nube: se reconecta con la nueva (al instante, nada se recarga aquí).
/// Local: el contexto REAL con este modelo es el pedido, acotado por su tope y por la memoria (contando
/// la que ya ocupa él mismo), y se DICE si se recortó; CRECER exige recargar (la caché de prompts del
/// motor, P11, se ató al contexto de arranque: con uno mayor cada orden volvería a prellenarse entera,
/// minutos con un 32B); achicar es al momento.
async fn aplicar_contexto(hub: &Hub, agent: &mut Ag, cfg: &mut Config, model: &mut String, engine_ctx: &mut usize, p: &str) -> Option<String> {
    let say = |t: String| emit(&hub.app, json!({"kind": "line", "text": t}));
    if gotv_core::nube::is_cloud(&cfg.model, cfg) {
        let mut next = cfg.clone();
        next.context_preset = p.to_string();
        return match restart(hub, agent, next, cfg.clone()).await {
            Ok((c, n)) => {
                *cfg = c;
                *model = n;
                *engine_ctx = cfg.context_budget;
                Some(gotv_core::tr!("Contexto: {} tokens ✅", "Context: {} tokens ✅", cfg.context_budget))
            }
            Err((e, e2)) => {
                say(gotv_core::tr!("❌ no pude reconectar ({e}) ni volver ({e2})", "❌ could not reconnect ({e}) nor go back ({e2})"));
                None
            }
        };
    }
    let c2 = cfg.clone();
    let preset = gotv_core::config::preset_to_budget(p);
    let plan = tokio::task::spawn_blocking(move || {
        let card = loaded_card(&c2).ok_or_else(|| anyhow::anyhow!("{}", gotv_core::tr!("no pude leer la ficha del modelo cargado ({})", "could not read the loaded model's card ({})", c2.model)))?;
        let hw = gotv_core::Hardware::detect();
        let plan = gotv_core::launch::resolve_context(&card, &hw, avail_with_loaded(&c2), preset)?;
        anyhow::Ok((plan, card.name))
    })
    .await
    .unwrap_or_else(|e| Err(anyhow::anyhow!("{e}")));
    let (plan, name) = match plan {
        Ok(x) => x,
        Err(e) => {
            say(format!("⚠ {e}"));
            return None;
        }
    };
    let mut ctx = plan.ctx as usize;
    if ctx > *engine_ctx {
        say(gotv_core::tr!(
            "⏳ recargo {name} con contexto {ctx} (arrancó con {engine_ctx}; su caché de prompts debe crecer)…",
            "⏳ reloading {name} with context {ctx} (it started with {engine_ctx}; its prompt cache must grow)…"
        ));
        let mut next = cfg.clone();
        next.context_preset = p.to_string();
        match restart(hub, agent, next, cfg.clone()).await {
            Ok((c, n)) => {
                *cfg = c;
                *model = n;
                *engine_ctx = cfg.context_budget;
                ctx = cfg.context_budget;
                calentar(hub, agent).await;
            }
            Err((e, e2)) => {
                say(gotv_core::tr!("❌ no pude recargar ({e}) ni volver ({e2})", "❌ could not reload ({e}) nor go back ({e2})"));
                return None;
            }
        }
    } else {
        agent.context_budget = ctx;
        cfg.context_preset = p.to_string();
        cfg.context_budget = ctx;
    }
    let msg = if ctx == plan.ctx as usize { plan.explain(&name) } else { None }.unwrap_or_else(|| gotv_core::tr!("Contexto: {} tokens ✅", "Context: {} tokens ✅", ctx));
    emit(&hub.app, json!({"kind": "line", "text": format!("· {msg}")}));
    Some(msg)
}

/// Al abrir un proyecto (con su modelo ya puesto): si su ventana de contexto es otra, se aplica.
async fn ajustar_contexto(hub: &Hub, agent: &mut Ag, cfg: &mut Config, model: &mut String, engine_ctx: &mut usize) {
    let quiere = contexto_de(&agent.root, &crate::store::Store::load());
    if quiere.trim() != cfg.context_preset.trim() {
        aplicar_contexto(hub, agent, cfg, model, engine_ctx, &quiere).await;
    }
}

/// Guarda llaves en `~/.config/gotv/config.toml` sin pisar las demás.
pub fn save_config(keys: &[(&str, toml::Value)]) {
    let p = gotv_core::config::global_path();
    let mut t: toml::Table = std::fs::read_to_string(&p).ok().and_then(|s| s.parse().ok()).unwrap_or_default();
    for (k, v) in keys {
        t.insert(k.to_string(), v.clone());
    }
    if let Some(d) = p.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let _ = std::fs::write(p, toml::to_string_pretty(&t).unwrap_or_default());
}

pub async fn run(hub: Hub, mut rx: mpsc::UnboundedReceiver<Msg>, root: Option<PathBuf>) {
    let app = hub.app.clone();
    let say = |t: String| emit(&app, json!({"kind": "line", "text": t}));
    // sin un proyecto nombrado al lanzar, la app abre en reposo: nada se carga hasta elegir uno
    let root = match root {
        Some(r) => r,
        None => match en_reposo(&hub, &mut rx).await {
            Some(r) => {
                *hub.root.lock().await = r.clone();
                hub.services_reset().await;
                r
            }
            None => return,
        },
    };
    say(gotv_core::i18n::t("⏳ leyendo la máquina y los modelos…", "⏳ reading the machine and the models…").into());
    let mut cfg = Config::load(&root);
    // el modelo del PROYECTO (o el último que elegiste) manda, salvo un `model =` explícito en tu
    // config, que es otra elección tuya
    let chosen = modelo_de(Some(&root), &cfg, &crate::store::Store::load());
    if let Some(m) = &chosen {
        cfg.model = m.clone();
        say(gotv_core::tr!("· modelo del proyecto: {m}", "· project model: {m}"));
    }
    // y su ventana de contexto, ANTES de cargar (si no, cargaría con otra y recargaría)
    cfg.context_preset = contexto_de(&root, &crate::store::Store::load());
    let first = boot(cfg.clone(), &hub).await;
    let booted = match first {
        Err(e) if chosen.is_some() => {
            // borrado, sin memoria, sin llave…: se avisa y arranca el automático
            say(gotv_core::tr!("⚠ no pude cargar tu último modelo ({e}); uso el automático", "⚠ could not load your last model ({e}); using the automatic one"));
            gotv_core::launch::wait_memory_settles(std::time::Duration::from_secs(20)).await;
            let mut c = cfg;
            c.model.clear();
            boot(c, &hub).await
        }
        other => other,
    };
    let (mut cfg, engine, mut model) = match booted {
        Ok(x) => x,
        Err(e) => {
            emit(&app, json!({"kind": "error", "text": e}));
            return;
        }
    };
    *hub.model_path.lock().await = cfg.model.clone();
    // contexto con el que arrancó el motor: su caché de prompts se dimensionó para él
    let mut engine_ctx = cfg.context_budget;
    let front = GuiFrontend::new(hub.clone());
    // el motor ya arrancó con el modelo y la ventana de ESTE proyecto
    let mut agent = new_agent(engine, &root, &cfg, front.clone(), hub.cancel.clone(), true);
    let t = std::time::Instant::now();
    calentar(&hub, &mut agent).await;
    say(gotv_core::tr!("listo en {:.1} s ✅", "ready in {:.1} s ✅", t.elapsed().as_secs_f32()));
    hub.status.lock().await["ready"] = json!(true);
    publish_status(&hub, &agent, &model).await;
    emit(&app, json!({"kind": "project", "root": agent.root.display().to_string(), "history": history_view(&agent)}));
    emit(&app, json!({"kind": "ready"}));
    // F10.5: el chat abierto se ve desde el teléfono
    front.fijar_remoto(&crate::remoto::sesion_id(&agent.root));
    crate::remoto::abrir(&hub, &agent.root);

    // F2.2 + F3: entre órdenes (el agente libre) también despierta lo programado y lo vigilado
    while let Some(msg) = recibir(&hub, &mut rx, &mut agent).await {
        match msg {
            Msg::Close => {
                // el proyecto abierto se quitó de la lista: se suelta y se espera otro
                cambiar_de_chat(&hub, &mut agent).await; // F10.1/F10.5
                let Agent { engine, .. } = agent;
                hub.services_reset().await;
                let Some(new_root) = wait_project(&hub, &mut rx).await else { return };
                let pc = Config::load(&new_root);
                cfg.allow = pc.allow;
                cfg.deny = pc.deny;
                cfg.ask = pc.ask;
                agent = new_agent(engine, &new_root, &cfg, front.clone(), hub.cancel.clone(), true);
                *hub.root.lock().await = new_root.clone();
                hub.services_reset().await;
                // F5: el modelo de ESTE proyecto
                ajustar_modelo(&hub, &mut agent, &mut cfg, &mut model, &mut engine_ctx).await;
                // y su ventana de contexto (cada proyecto la suya)
                ajustar_contexto(&hub, &mut agent, &mut cfg, &mut model, &mut engine_ctx).await;
                publish_status(&hub, &agent, &model).await;
                emit(&app, json!({"kind": "project", "root": new_root.display().to_string(), "history": history_view(&agent)}));
                front.fijar_remoto(&crate::remoto::sesion_id(&new_root)); // F10.5
                crate::remoto::abrir(&hub, &new_root);
                calentar(&hub, &mut agent).await;
                emit(&app, json!({"kind": "done", "text": "", "silent": true}));
            }
            Msg::Experto(texto, tx) => {
                let r = agent.experto_orden(&texto).await.map_err(|e| format!("{e:#}"));
                let _ = tx.send(r);
                publish_status(&hub, &agent, &model).await;
            }
            Msg::Chat(id) => {
                // F8.1: otro chat del MISMO proyecto (el de ahora quedó guardado al cerrar su turno)
                cambiar_de_chat(&hub, &mut agent).await; // F10.1/F10.5
                let r = match id {
                    Some(id) => agent.abrir_chat(id),
                    None => {
                        agent.clear();
                        Ok(())
                    }
                };
                if let Err(e) = r {
                    emit(&app, json!({"kind": "error", "text": format!("{e:#}")}));
                }
                publish_status(&hub, &agent, &model).await;
                emit(&app, json!({"kind": "project", "root": agent.root.display().to_string(), "history": history_view(&agent), "chat": agent.session.as_ref().map(|s| s.id)}));
                crate::remoto::abrir(&hub, &agent.root);
                calentar(&hub, &mut agent).await;
                emit(&app, json!({"kind": "done", "text": "", "silent": true}));
            }
            Msg::Recursos(tx) => {
                let v = match agent.mcp.as_mut() {
                    Some(m) => gotv_core::menciones::recursos_de(m).await,
                    None => vec![],
                };
                let _ = tx.send(v);
            }
            Msg::Rapido => {
                // se encendió «Reabrir rápido»: la memoria de esta conversación se construye YA
                calentar(&hub, &mut agent).await;
                publish_status(&hub, &agent, &model).await;
            }
            Msg::Open(new_root) => {
                // agente nuevo en la otra carpeta; el motor sigue salvo que el proyecto tenga OTRO modelo
                cambiar_de_chat(&hub, &mut agent).await; // F10.1/F10.5
                let Agent { engine, .. } = agent;
                let pc = Config::load(&new_root);
                cfg.allow = pc.allow;
                cfg.deny = pc.deny;
                cfg.ask = pc.ask;
                agent = new_agent(engine, &new_root, &cfg, front.clone(), hub.cancel.clone(), true);
                *hub.root.lock().await = new_root.clone();
                hub.services_reset().await;
                // F5: el modelo de ESTE proyecto
                ajustar_modelo(&hub, &mut agent, &mut cfg, &mut model, &mut engine_ctx).await;
                // y su ventana de contexto (cada proyecto la suya)
                ajustar_contexto(&hub, &mut agent, &mut cfg, &mut model, &mut engine_ctx).await;
                publish_status(&hub, &agent, &model).await;
                emit(&app, json!({"kind": "project", "root": new_root.display().to_string(), "history": history_view(&agent)}));
                front.fijar_remoto(&crate::remoto::sesion_id(&new_root)); // F10.5
                crate::remoto::abrir(&hub, &new_root);
                calentar(&hub, &mut agent).await;
                emit(&app, json!({"kind": "done", "text": "", "silent": true}));
            }
            Msg::Model(path) => {
                if path == cfg.model {
                    emit(&app, json!({"kind": "done", "text": gotv_core::i18n::t("ese modelo ya está cargado", "that model is already loaded"), "info": true}));
                    continue;
                }
                say(gotv_core::tr!("⏳ apagando {model} para liberar la memoria…", "⏳ shutting down {model} to free the memory…"));
                let mut next = cfg.clone();
                next.model = path;
                match restart(&hub, &mut agent, next, cfg.clone()).await {
                    Ok((c, name)) => {
                        let mode_changed = c.chat_only != agent.chat_only;
                        if mode_changed {
                            agent.chat_only = c.chat_only;
                            agent.clear();
                            say(gotv_core::i18n::t("· cambio entre modo agente y modo chat: conversación nueva", "· switch between agent and chat mode: new conversation").into());
                        }
                        cfg = c;
                        engine_ctx = cfg.context_budget;
                        model = name.clone();
                        // F5: tu elección es la de ESTE proyecto (y la que se propone a los nuevos)
                        let mut st = crate::store::Store::load();
                        if agent.root.as_os_str().is_empty() {
                            st.model = Some(cfg.model.clone());
                        } else {
                            st.set_project_model(&agent.root, &cfg.model);
                        }
                        st.save();
                        // con la memoria rápida: la de ESTE modelo (otro modelo = otra memoria)
                        calentar(&hub, &mut agent).await;
                        publish_status(&hub, &agent, &model).await;
                        emit(&app, json!({"kind": "done", "text": gotv_core::tr!("modelo: {name} ✅", "model: {name} ✅"), "info": true}));
                    }
                    Err((e, e2)) => {
                        emit(&app, json!({"kind": "error", "text": gotv_core::tr!("no pude cambiar ({e}) ni volver ({e2}): elige otro modelo", "could not switch ({e}) nor go back ({e2}): pick another model")}));
                    }
                }
            }
            Msg::Apply(s) => {
                let mut keys: Vec<(&str, toml::Value)> = vec![];
                let mut notes_ctx: Option<String> = None;
                if let Some(m) = s["mode"].as_str() {
                    agent.perms.mode = Mode::parse(m);
                    cfg.mode = m.to_string();
                    keys.push(("mode", toml::Value::String(m.into())));
                }
                if let Some(n) = s["max_tokens"].as_u64() {
                    agent.max_tokens = n.clamp(256, 32768) as u32;
                    cfg.max_tokens = agent.max_tokens;
                    keys.push(("max_tokens", toml::Value::Integer(agent.max_tokens as i64)));
                }
                if let Some(t) = s["temperature"].as_f64() {
                    let t = t.clamp(0.0, 2.0) as f32;
                    agent.sampling.temperature = t;
                    keys.push(("temperature", toml::Value::Float(t as f64)));
                }
                if let Some(b) = s["computer_use"].as_bool() {
                    agent.computer_use = b;
                    cfg.computer_use = b;
                    keys.push(("computer_use", toml::Value::Boolean(b)));
                }
                // administrador de pasos: el tope por orden, en vivo (5–500)
                if let Some(n) = s["max_steps"].as_u64() {
                    let n = n.clamp(5, 500) as u32;
                    agent.max_steps = n;
                    cfg.max_steps = n;
                    keys.push(("max_steps", toml::Value::Integer(n as i64)));
                }
                // F9: cuántos subagentes trabajan a la vez (1–16); se lee al repartir, no hace falta en vivo
                if let Some(n) = s["subagentes"].as_u64() {
                    let n = n.clamp(1, 16) as u32;
                    cfg.subagentes = n;
                    keys.push(("subagentes", toml::Value::Integer(n as i64)));
                }
                // C2: caja de arena y dominios extra (en vivo y guardados)
                if let Some(b) = s["sandbox"].as_bool() {
                    agent.perms.sandbox = b;
                    cfg.sandbox = b;
                    keys.push(("sandbox", toml::Value::Boolean(b)));
                }
                if let Some(a) = s["net_allow"].as_array() {
                    let v: Vec<String> = a.iter().filter_map(|x| x.as_str()).map(|h| h.trim().to_lowercase()).filter(|h| !h.is_empty() && !h.contains(char::is_whitespace)).collect();
                    agent.perms.net_extra = v.clone();
                    cfg.net_allow = v.clone();
                    keys.push(("net_allow", toml::Value::Array(v.into_iter().map(toml::Value::String).collect())));
                }
                if let Some(b) = s["desktop_control"].as_bool() {
                    agent.perms.desktop_control = b;
                    cfg.desktop_control = b;
                    keys.push(("desktop_control", toml::Value::Boolean(b)));
                }
                if let Some(b) = s["system_changes"].as_bool() {
                    // la autenticación de admin ya la hizo el comando set_system_changes;
                    // aquí sólo se aplica a los permisos vivos y se guarda
                    agent.perms.system_changes = b;
                    cfg.system_changes = b;
                    keys.push(("system_changes", toml::Value::Boolean(b)));
                }
                if let Some(l) = s["lang"].as_str() {
                    gotv_core::i18n::set(gotv_core::i18n::resolve(l));
                    keys.push(("lang", toml::Value::String(l.into())));
                }
                // A3: esfuerzo de Claude
                let new_effort = s["effort"].as_str().map(String::from);
                if let Some(e) = &new_effort {
                    cfg.effort = Some(e.clone()).filter(|e| !e.is_empty());
                    keys.push(("effort", toml::Value::String(e.clone())));
                }
                let cloud = gotv_core::nube::is_cloud(&cfg.model, &cfg);
                if let Some(p) = s["context_preset"].as_str().map(str::trim).filter(|p| !p.is_empty()) {
                    // la ventana es del PROYECTO abierto (cada uno puede usar otra IA); la elegida queda
                    // además como la de los proyectos que aún no tienen la suya
                    keys.push(("context_preset", toml::Value::String(p.into())));
                    if !agent.root.as_os_str().is_empty() {
                        let mut st = crate::store::Store::load();
                        st.set_project_context(&agent.root, p);
                        st.save();
                    }
                    // en la nube reconecta con todo lo nuevo (también el esfuerzo, ya en `cfg`)
                    notes_ctx = aplicar_contexto(&hub, &mut agent, &mut cfg, &mut model, &mut engine_ctx, p).await;
                } else if cloud && new_effort.is_some() {
                    // sólo el esfuerzo: en la nube se reconecta con él (al instante)
                    match restart(&hub, &mut agent, cfg.clone(), cfg.clone()).await {
                        Ok((c, n)) => {
                            cfg = c;
                            model = n;
                            engine_ctx = cfg.context_budget;
                        }
                        Err((e, e2)) => say(gotv_core::tr!("❌ no pude reconectar ({e}) ni volver ({e2})", "❌ could not reconnect ({e}) nor go back ({e2})")),
                    }
                }
                save_config(&keys);
                publish_status(&hub, &agent, &model).await;
                let text = match notes_ctx {
                    Some(m) => format!("{} {m}", gotv_core::i18n::t("Configuración guardada.", "Settings saved.")),
                    None => gotv_core::i18n::t("configuración guardada ✅", "settings saved ✅").to_string(),
                };
                emit(&app, json!({"kind": "done", "text": text, "info": true}));
            }
            Msg::Task(task, mac, perfil, rebobinar, pro) => {
                // ↺ rebobinar (como Claude Desktop): la conversación se corta ANTES de ese mensaje justo
                // al enviar el editado; los archivos no se tocan
                if let Some((texto, n)) = rebobinar {
                    match agent.rebobinar(&texto, n) {
                        Ok(k) => emit(&app, json!({"kind": "rebobinado", "quitados": k})),
                        Err(e) => {
                            emit(&app, json!({"kind": "error", "text": e.to_string()}));
                            continue;
                        }
                    }
                }
                // control nativo del Mac sólo en esta orden y sólo si Seguridad lo permite
                agent.perms.desktop_turn = mac;
                // D7: el perfil de los botones junto a adjuntar
                if let Some(p) = &perfil {
                    agent.perfil = gotv_core::perfil::Perfil::parse(p);
                }
                // Ola PRO: los ajustes del modo profesional valen sólo para ESTA orden
                agent.pro = *pro;
                // comandos de barra y skills (`/nombre resto`)
                if let Some(cmd) = task.trim().strip_prefix('/') {
                    let (name, arg) = cmd.split_once(' ').unwrap_or((cmd, ""));
                    let arg = arg.trim();
                    // F12 · formas de trabajar: /objetivo, /aparte, /aprender, /entrevista, /reglas
                    if crate::formas::atender(&hub, &mut agent, &model, &task, name, arg, &rx).await {
                        continue;
                    }
                    // F16: /revisar, /arreglar, /pr, /agentes
                    if crate::escritorio::atender(&hub, &mut agent, &model, &task, name, arg).await {
                        continue;
                    }
                    let builtin = match name {
                        "clear" => {
                            agent.clear();
                            emit(&app, json!({"kind": "project", "root": agent.root.display().to_string(), "history": []}));
                            Some(gotv_core::i18n::t("conversación nueva ✅", "new conversation ✅").to_string())
                        }
                        "compact" => Some(if agent.compact_now().await {
                            gotv_core::i18n::t("contexto compactado ✅", "context compacted ✅").to_string()
                        } else {
                            gotv_core::i18n::t("aún no hay nada que compactar", "nothing to compact yet").to_string()
                        }),
                        "mode" if !arg.is_empty() => {
                            agent.perms.mode = Mode::parse(arg);
                            Some(gotv_core::tr!("modo de permisos: {} ✅", "permission mode: {} ✅", agent.perms.mode.label()))
                        }
                        "context" => {
                            let (u, b) = agent.context_usage();
                            Some(gotv_core::tr!("contexto: ~{u} de {b} tokens · se compacta solo al pasar el 75 %", "context: ~{u} of {b} tokens · compacts itself past 75 %"))
                        }
                        // F16.3: en un proyecto remoto se deshace ALLÁ el último turno que escribió
                        "rewind" if gotv_core::lugar::ficha(&agent.root).is_some() => Some(gotv_core::lugar::deshacer_ultimo(&agent.root).await.unwrap_or_else(|e| e.to_string())),
                        "rewind" => Some(match &agent.checkpoint {
                            // el último TURNO entero; `/rewind paso` sólo el último paso
                            Some(cp) if arg.split_whitespace().any(|a| matches!(a, "paso" | "--paso" | "step" | "--step")) => cp.rewind_paso(arg.contains("--force")).unwrap_or_else(|e| e.to_string()),
                            Some(cp) => cp.rewind(arg.contains("--force")).unwrap_or_else(|e| e.to_string()),
                            None => "—".into(),
                        }),
                        "modelos" | "models" => Some(crate::libreria::chat_list(&hub, arg).await),
                        "descargar" | "download" | "pull" => Some(if arg.is_empty() {
                            gotv_core::i18n::t("uso: /descargar org/nombre (p. ej. mlx-community/Qwen3-8B-4bit) · busca con /modelos <texto>", "usage: /download org/name (e.g. mlx-community/Qwen3-8B-4bit) · search with /models <text>").to_string()
                        } else {
                            crate::libreria::start_pull(&hub, arg.trim_matches('`')).await.unwrap_or_else(|e| format!("✗ {e}"))
                        }),
                        "help" | "?" => Some(format!("{}\n{}", gotv_core::i18n::t(
                            "Comandos: /clear · /compact · /context · /mode <plan|default|acceptEdits|bypass> · /rewind [--force] · /modelos <texto> · /descargar <org/nombre> · /<skill> <tarea>",
                            "Commands: /clear · /compact · /context · /mode <plan|default|acceptEdits|bypass> · /rewind [--force] · /models <text> · /download <org/name> · /<skill> <task>",
                        ), gotv_core::formas::ayuda())),
                        _ => None,
                    };
                    if let Some(text) = builtin {
                        publish_status(&hub, &agent, &model).await;
                        // la lista de modelos es una tabla: va como respuesta (markdown), no como nota
                        let table = matches!(name, "modelos" | "models") && text.starts_with("**");
                        emit(&app, json!({"kind": "done", "text": text, "info": !table}));
                        continue;
                    }
                    // ¿es una skill? → su cuerpo viaja con la orden
                    if let Ok(body) = gotv_core::skills::load(&agent.root, name) {
                        let body: String = body.chars().take(14_000).collect();
                        let order = if arg.is_empty() { gotv_core::i18n::t("Aplica esta skill.", "Apply this skill.").to_string() } else { arg.to_string() };
                        let full = format!("{order}\n\n{}\n{body}", gotv_core::tr!("[skill «{name}» elegida por el usuario]", "[skill «{name}» chosen by the user]"));
                        // lo escrito (`/skill orden`) es lo que el chat muestra; la skill entera, para el modelo
                        run_order(&hub, &mut agent, &model, full, gotv_core::mensajes::tu_mensaje(&task).to_string()).await;
                        continue;
                    }
                }
                // sin la instrucción de voz de manos libres, si la lleva
                let escrito = gotv_core::mensajes::tu_mensaje(&task).to_string();
                run_order(&hub, &mut agent, &model, task, escrito).await;
            }
        }
    }
    // F10.1 / F10.5: la app se cierra: SessionEnd y el chat deja de verse en el teléfono
    agent.cerrar_sesion("other").await;
    crate::remoto::cerrar(&hub, &agent.root);
    let _ = agent.engine.stop().await;
}

/// F10.1 / F10.5: el chat que se deja (otro proyecto, o se quitó): SessionEnd y deja de verse en el teléfono.
async fn cambiar_de_chat(hub: &Hub, agent: &mut Ag) {
    crate::escritorio::soltar(agent); // F16: su cola y sus terminales
    agent.cerrar_sesion("other").await;
    crate::remoto::cerrar(hub, &agent.root);
}

/// F2.2 + F3 · El siguiente mensaje del bucle. Este bucle sólo espera ENTRE órdenes (el agente libre): ahí,
/// cada segundo, lo programado que venció o lo que vieron las vigilancias del chat es la orden siguiente.
/// Con el agente ocupado nadie lo mira, así que lo vencido espera a que termine.
async fn recibir(hub: &Hub, rx: &mut mpsc::UnboundedReceiver<Msg>, agent: &mut Ag) -> Option<Msg> {
    let mut tic = tokio::time::interval(std::time::Duration::from_secs(1));
    tic.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            m = rx.recv() => return m,
            _ = tic.tick() => {
                // F10.5: lo que mandó el teléfono es la próxima orden (se ve en el chat como del teléfono)
                if let Some(m) = hub.api.remoto.tomar_mensaje(&crate::remoto::sesion_id(&agent.root)) {
                    emit(&hub.app, json!({"kind": "line", "text": gotv_core::tr!("📱 del teléfono: {m}", "📱 from the phone: {m}")}));
                    return Some(Msg::Task(m, false, None, None, Box::default()));
                }
                if let Some(orden) = gotv_core::programar::chat::turno_pendiente(agent, gotv_core::programar::chat::ahora()) {
                    // se ve en el chat como lo que es: algo que llegó solo, no algo que escribiste
                    emit(&hub.app, json!({"kind": "line", "text": orden.lines().next().unwrap_or("")}));
                    let vigilancia = !orden.starts_with('⏰');
                    if vigilancia {
                        crate::avisos::avisar(&hub.app, Some(&agent.root), gotv_core::avisos::Evento::VigilanciaDisparada, orden.lines().next().unwrap_or(""));
                    }
                    return Some(Msg::Task(orden, false, None, None, Box::default()));
                }
            }
        }
    }
}

/// Cómo acabó un turno.
enum Fin {
    Hecho(anyhow::Result<String>),
    Panico(String),
    Forzado,
}

/// Segundos que el turno tiene para atender el stop antes de abandonarlo.
const STOP_GRACIA_S: u64 = 8;

/// Termina sólo si el stop se pulsó y pasaron `STOP_GRACIA_S` sin que el turno lo atendiera
/// (el turno que lo atiende devuelve antes «interrumpido» y esta espera se descarta).
async fn stop_sin_respuesta(cancel: &std::sync::atomic::AtomicBool) {
    loop {
        if cancel.load(std::sync::atomic::Ordering::Relaxed) {
            tokio::time::sleep(std::time::Duration::from_secs(STOP_GRACIA_S)).await;
            if cancel.load(std::sync::atomic::Ordering::Relaxed) {
                return;
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }
}

/// Todo pánico de la app queda en `~/.cache/gotv/gui-fallos.log` (fecha, hilo, lugar y backtrace)
/// además del aviso de siempre: abierta desde el Finder, la app no tiene stderr.
pub fn registrar_panicos() {
    let previo = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let ruta = std::path::PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".cache/gotv/gui-fallos.log");
        let ahora = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let hilo = std::thread::current().name().unwrap_or("?").to_string();
        let traza = std::backtrace::Backtrace::force_capture();
        if let Some(dir) = ruta.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&ruta) {
            use std::io::Write;
            let _ = writeln!(f, "── {ahora} (unix) · hilo {hilo} · v{}\n{info}\n{traza}\n", env!("CARGO_PKG_VERSION"));
        }
        previo(info);
    }));
}

/// Una orden: adjuntos pendientes → agente → respuesta, con su tarea en el panel. `escrito` = lo que
/// tecleaste o dijiste (lo que el chat muestra al reabrir y rebobina), sin lo añadido para el modelo.
pub(crate) async fn run_order(hub: &Hub, agent: &mut Ag, model: &str, task: String, escrito: String) {
    let pending = std::mem::take(&mut *hub.pending.lock().await);
    let mut text = task.clone();
    for (_, p) in pending {
        match p {
            Payload::Media(name, url) => agent.user_media.push((name, url)),
            Payload::Text(t) => text = format!("{text}\n\n{t}"),
        }
    }
    // F8.2/F8.3: los comentarios del panel de cambios (delante) y lo mencionado con @ (chats y recursos MCP)
    let (text, escrito) = crate::ventana::preparar_envio(hub, agent, text, escrito).await;
    let title: String = task.lines().next().unwrap_or("").chars().take(60).collect();
    let tid = hub.task_begin("turn", &title).await;
    *hub.turn.lock().await = Some(tid);
    // M3: con manos libres, la respuesta también se DICE (el frontend le pasa el texto en vivo)
    let charla = hub.charla.lock().ok().and_then(|g| g.clone());
    if let Some(c) = &charla {
        c.empezar();
        emit(&hub.app, json!({"kind": "manos", "estado": "pensando"}));
    }
    // F10.5 (prueba del Portador, 2026-11-18): el teléfono ve también la orden que se dio en la Mac
    hub.api.remoto.evento(&crate::remoto::sesion_id(&agent.root), "orden", json!({"texto": &escrito}));
    agent.visible = Some(escrito);
    // 2026-11-11 (captura del Portador: «trabajando…» eterno, el stop muerto, sin CPU ni red ni hijos):
    // el turno corría DENTRO del bucle del worker; si reventaba o se quedaba esperando algo que no
    // llega, no quedaba nadie que oyera el stop. Ahora:
    // - un pánico se atrapa (y queda escrito en ~/.cache/gotv/gui-fallos.log);
    // - el stop SIEMPRE gana: si el turno no lo atiende en STOP_GRACIA_S, se abandona;
    // - en ambos casos la conversación vuelve a como estaba antes de la orden (el turno se la
    //   lleva mientras corre: sin la copia se perdería entera).
    let antes = agent.history.clone();
    crate::remoto::estado(hub, &agent.root, gotv_core::remoto::EstadoSesion::Trabajando); // F10.5
    let fin = {
        let turno = futures_util::FutureExt::catch_unwind(std::panic::AssertUnwindSafe(agent.run(&text)));
        tokio::pin!(turno);
        tokio::select! {
            r = &mut turno => match r {
                Ok(r) => Fin::Hecho(r),
                Err(p) => Fin::Panico(p.downcast_ref::<String>().cloned().or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string())).unwrap_or_else(|| "?".into())),
            },
            _ = stop_sin_respuesta(&hub.cancel) => Fin::Forzado,
        }
    };
    crate::remoto::estado(hub, &agent.root, gotv_core::remoto::EstadoSesion::Libre); // F10.5
    let r = match fin {
        Fin::Hecho(r) => r,
        Fin::Panico(msg) => {
            agent.history = antes;
            Err(anyhow::anyhow!(
                "{}",
                gotv_core::tr!(
                    "el turno se cayó por un fallo interno ({msg}); quedó escrito en ~/.cache/gotv/gui-fallos.log. La conversación sigue como antes de esta orden.",
                    "the turn crashed with an internal error ({msg}); it was written to ~/.cache/gotv/gui-fallos.log. The conversation is as it was before this order.",
                )
            ))
        }
        Fin::Forzado => {
            agent.history = antes;
            Ok(gotv_core::tr!(
                "{} (forzado: no respondió al stop en {STOP_GRACIA_S} s; la conversación sigue como antes de esta orden)",
                "{} (forced: it did not answer the stop in {STOP_GRACIA_S} s; the conversation is as it was before this order)",
                gotv_core::agent::interrupted()
            ))
        }
    };
    *hub.turn.lock().await = None;
    if let Some(c) = charla {
        // dice lo que quede y, al acabar de sonar, vuelve a escuchar
        let listo = c.terminar();
        let app = hub.app.clone();
        std::thread::spawn(move || {
            let _ = listo.recv();
            emit(&app, json!({"kind": "manos", "estado": "escuchando"}));
        });
    }
    match r {
        Ok(ans) => {
            let state = if ans == gotv_core::agent::interrupted() { "stopped" } else { "done" };
            hub.task_end(tid, state, "").await;
            // F2.1: con la ventana en segundo plano, el turno terminado avisa
            if state == "done" {
                crate::avisos::avisar(&hub.app, Some(&agent.root), gotv_core::avisos::Evento::TurnoTerminado, &gotv_core::tr!("terminó: {title}", "done: {title}"));
            }
            publish_status(hub, agent, model).await;
            // `limpia`: la respuesta perdió la ubicación/licencia de lo generado (va en la tarjeta): manda
            // F8.1: dónde quedaron tu mensaje y la respuesta en el historial (bifurcar desde ellos sin reabrir)
            let h = &agent.history;
            let iu = h.iter().rposition(|m| m.role == "user" && !m.es_aviso() && !gotv_core::mensajes::es_adjuntos_pedidos(m));
            let ia = h.iter().rposition(|m| m.role == "assistant" && m.tool_calls.is_none()).filter(|a| iu.is_none_or(|u| *a > u));
            emit(&hub.app, json!({"kind": "done", "text": ans, "limpia": agent.recorto_ubicaciones, "iu": iu, "ia": ia}));
        }
        Err(e) => {
            hub.task_end(tid, "error", &e.to_string()).await;
            emit(&hub.app, json!({"kind": "error", "text": e.to_string()}));
        }
    }
    // F16.1: lo que el Portador escribió cuando el turno ya cerraba es la orden siguiente
    crate::escritorio::tras_orden(hub, agent);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// El turno colgado para siempre (sin CPU, sin red: lo que vio el Portador) se abandona cuando
    /// el stop no se atiende; un stop viejo que el turno ya limpió no fuerza nada.
    #[tokio::test(start_paused = true)]
    async fn el_stop_siempre_gana_a_un_turno_colgado() {
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::Arc;
        let cancel = Arc::new(AtomicBool::new(false));
        let c = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(30)).await;
            c.store(true, Ordering::Relaxed); // el Portador pulsa stop a los 30 s
        });
        let t0 = tokio::time::Instant::now();
        let colgado = std::future::pending::<()>();
        let forzado = tokio::select! {
            _ = colgado => false,
            _ = stop_sin_respuesta(&cancel) => true,
        };
        assert!(forzado);
        let s = t0.elapsed().as_secs();
        assert!((30 + STOP_GRACIA_S..30 + STOP_GRACIA_S + 2).contains(&s), "{s} s");

        // un stop VIEJO que el turno limpia al empezar (agent.run pone cancel=false) no fuerza
        let viejo = Arc::new(AtomicBool::new(true));
        let v = viejo.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            v.store(false, Ordering::Relaxed);
        });
        let r = tokio::time::timeout(std::time::Duration::from_secs(60), stop_sin_respuesta(&viejo)).await;
        assert!(r.is_err(), "no debe forzar un turno que sigue sano");
    }

    #[test]
    fn el_modelo_elegido_vuelve_si_sigue_existiendo() {
        let cfg = Config::default();
        let dir = gotv_engine::prueba::CarpetaPrueba::sin_crear("gotv-modelo");
        std::fs::create_dir_all(&dir).unwrap();
        let local = dir.display().to_string();
        assert_eq!(chosen_model(Some(local.clone()), &cfg), Some(local), "carpeta local que existe");
        assert_eq!(chosen_model(Some("anthropic:claude-opus-5-5".into()), &cfg).as_deref(), Some("anthropic:claude-opus-5-5"), "nube");
        assert_eq!(chosen_model(Some("/no/existe/modelo".into()), &cfg), None, "borrado → automático");
        assert_eq!(chosen_model(Some("desconocido:x".into()), &cfg), None, "proveedor que ya no está");
        assert_eq!(chosen_model(None, &cfg), None);
        assert_eq!(chosen_model(Some("  ".into()), &cfg), None);
    }
}
