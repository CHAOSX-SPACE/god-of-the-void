//! F16 · Paridad con la app de escritorio, en la ventana. La lógica vive en `gotv_core::escritorio` y
//! `gotv_core::lugar`; aquí sólo se cose con la ventana (órdenes Tauri, eventos `gotv`) y con el turno del
//! trabajador (`run_order`: tarea en el panel, stop que siempre gana).
//!
//! Eventos que emite (todos por `gotv`): `cola` (los mensajes que esperan), `revision` (la tarjeta de
//! `/revisar`), `terminal` (`{id, vista}` o `{id, fin}`), `pr` (el informe de cada vuelta).
use crate::worker::{publish_status, run_order, Ag, Msg};
use crate::{emit, Hub};
use gotv_core::escritorio::{agentes, archivo, cola, pr, revisar, terminal, variables};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::Arc;
use tauri::State;

/// Al arrancar la ventana: la fábrica de modelos para los agentes a medida y el vigía de la cola.
pub fn arrancar(hub: Hub) {
    agentes::registrar_fabrica(fabrica());
    tauri::async_runtime::spawn(async move {
        let mut ultimo = String::new();
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
            let dueno = hub.procs_owner.lock().unwrap().clone();
            if dueno.is_empty() {
                continue;
            }
            // F16.1 + F10.5: con el agente trabajando, lo que llega del teléfono también entra a mitad
            if hub.turn.lock().await.is_some() {
                let root = hub.root.lock().await.clone();
                if let Some(m) = hub.api.remoto.tomar_mensaje(&crate::remoto::sesion_id(&root)) {
                    if cola::encolar(&dueno, &m, gotv_core::i18n::t("teléfono", "phone")).is_ok() {
                        emit(&hub.app, json!({"kind": "line", "text": gotv_core::tr!("📱 del teléfono (entra entre dos pasos): {m}", "📱 from the phone (goes in between two steps): {m}")}));
                    }
                }
            }
            // la ventana ve qué sigue esperando (lo que desaparece, entró)
            let p: Vec<Value> = cola::pendientes(&dueno).into_iter().map(|e| json!({"id": e.id, "texto": e.texto, "origen": e.origen})).collect();
            let clave = serde_json::to_string(&p).unwrap_or_default();
            if clave != ultimo {
                ultimo = clave;
                emit(&hub.app, json!({"kind": "cola", "pendientes": p}));
            }
        }
    });
}

/// Cómo cargar el modelo de un agente a medida: de nube con su llave; local con el núcleo de GOTV (si cabe
/// junto al del chat: `launch::prepare` mide la memoria libre AHORA, con el del chat ya cargado).
fn fabrica() -> agentes::Fabrica {
    agentes::fabrica(|modelo: String| {
        async move {
            let mut cfg = gotv_core::Config::load(&std::env::temp_dir());
            cfg.model = modelo.clone();
            if gotv_core::nube::is_cloud(&modelo, &cfg) {
                let (e, _, _) = gotv_core::nube::launch(&mut cfg).await?;
                return Ok(Arc::new(e) as agentes::Motor);
            }
            let (cfg, r) = tokio::task::spawn_blocking(move || {
                let r = gotv_core::launch::prepare(&mut cfg);
                (cfg, r)
            })
            .await?;
            let l = r?;
            let dir = l.card.as_ref().map(|c| c.path.clone()).unwrap_or_else(|| PathBuf::from(&cfg.model));
            gotv_mlx::NativoEngine::puede(&dir).map_err(|por| anyhow::anyhow!("{}", gotv_core::tr!("el núcleo de GOTV aún no corre este modelo: {por}", "GOTV's core does not run this model yet: {por}")))?;
            let mut e = gotv_mlx::NativoEngine::new(&dir);
            gotv_engine::Engine::start(&mut e).await?;
            Ok(Arc::new(e) as agentes::Motor)
        }
    })
}

// ───────────────────────── F16.1 ─────────────────────────

/// Con el agente trabajando, lo escrito se encola y entra entre dos pasos (no lo detiene).
#[tauri::command]
pub fn mensaje_encolar(texto: String, h: State<'_, Hub>) -> Result<u64, String> {
    let dueno = h.procs_owner.lock().unwrap().clone();
    if dueno.is_empty() {
        return Err(gotv_core::i18n::t("no hay un chat abierto", "there is no open chat").into());
    }
    cola::encolar(&dueno, &texto, gotv_core::i18n::t("ventana", "window"))
}

/// Se arrepintió antes de que entrara.
#[tauri::command]
pub fn mensaje_quitar(id: u64, h: State<'_, Hub>) -> bool {
    cola::quitar(&h.procs_owner.lock().unwrap().clone(), id)
}

/// Enganche de `run_order` (al terminar un turno): lo que llegó tras el último paso es la orden siguiente.
pub fn tras_orden(hub: &Hub, agent: &Ag) {
    if let Some(t) = cola::sobrantes(&agent.proc_owner()) {
        emit(&hub.app, json!({"kind": "cola", "pendientes": [], "sigue": t}));
        let _ = hub.tx.send(Msg::Task(t, false, None, None, Box::default()));
    }
}

/// Enganche de `cambiar_de_chat`: lo de ese chat se suelta (su cola, sus terminales).
pub fn soltar(agent: &Ag) {
    cola::soltar(&agent.proc_owner());
    terminal::cerrar_de(&agent.root);
}

// ───────────────────── F16.2 · F16.5 · F16.6 en el chat ─────────────────────

/// Las órdenes de barra de F16. `true` si la atendió.
pub async fn atender(hub: &Hub, agent: &mut Ag, model: &str, task: &str, nombre: &str, arg: &str) -> bool {
    // juez-idioma: datos (los nombres de las órdenes en los dos idiomas)
    match nombre {
        "revisar" | "review" => revisar_en_ventana(hub, agent, model, task, arg).await,
        "arreglar" | "fix" => match arg.trim().parse::<u32>().ok().and_then(|id| revisar::orden_arreglar(&agent.proc_owner(), id)) {
            Some((orden, visible)) => run_order(hub, agent, model, orden, visible).await,
            None => emit(&hub.app, json!({"kind": "error", "text": gotv_core::i18n::t("no hay ese hallazgo en la última revisión (/revisar primero)", "there is no such finding in the last review (/review first)")})),
        },
        "pr" => pr_en_ventana(hub, agent, model, arg).await,
        "agentes" | "agents" => emit(&hub.app, json!({"kind": "done", "text": agentes::lista(&agent.root), "info": true})),
        _ => return false,
    }
    true
}

/// `/revisar`: un turno de la ventana en sólo lectura y, al final, la tarjeta.
async fn revisar_en_ventana(hub: &Hub, agent: &mut Ag, model: &str, task: &str, arg: &str) {
    let (tipo, resto) = revisar::tipo_de(arg);
    let (diff, de) = match revisar::que_revisar(&agent.root, resto) {
        Ok(x) => x,
        Err(e) => return emit(&hub.app, json!({"kind": "done", "text": e.to_string(), "info": true})),
    };
    let desde = agent.history.len();
    let antes = revisar::solo_lectura(agent);
    run_order(hub, agent, model, revisar::orden(tipo, &diff, &de), task.trim().to_string()).await;
    let ultima = |agent: &Ag, desde: usize| agent.history.get(desde..).and_then(|h| h.iter().rev().find(|m| m.role == "assistant" && m.tool_calls.is_none())).map(|m| m.content.clone()).unwrap_or_default();
    let mut rev = revisar::validar(&agent.root, &ultima(agent, desde));
    if rev.sin_json {
        // terminó en prosa: se le pide UNA vez el JSON, aún en sólo lectura (la tarjeta no sale vacía)
        let otra = agent.history.len();
        run_order(hub, agent, model, revisar::orden_solo_json(), gotv_core::i18n::t("↺ pidiendo los hallazgos en su formato", "↺ asking for the findings in their format").to_string()).await;
        rev = revisar::validar(&agent.root, &ultima(agent, otra));
    }
    revisar::restaurar(agent, antes);
    revisar::guardar(&agent.proc_owner(), &rev);
    emit(&hub.app, rev.tarjeta(tipo));
}

/// `/pr N [--arreglar]`: cada ronda un turno de la ventana; empujar pregunta (salvo bypass); jamás fusiona.
async fn pr_en_ventana(hub: &Hub, agent: &mut Ag, model: &str, arg: &str) {
    let mut s = match pr::Seguimiento::nuevo(&agent.root, arg) {
        Ok(s) => s,
        Err(e) => return emit(&hub.app, json!({"kind": "error", "text": e.to_string()})),
    };
    emit(&hub.app, json!({"kind": "line", "text": gotv_core::tr!("👁 siguiendo el PR #{} ({}){}", "👁 following PR #{} ({}){}", s.numero, s.repo, if s.arreglar { gotv_core::i18n::t(" · arreglará lo rojo con tu permiso para empujar", " · will fix red with your permission to push") } else { "" })}));
    loop {
        let paso = tokio::select! {
            p = s.siguiente() => p,
            _ = esperar_stop(hub) => pr::Paso::Fin(gotv_core::agent::interrupted().to_string()),
        };
        match paso {
            pr::Paso::Fin(t) => {
                publish_status(hub, agent, model).await;
                return emit(&hub.app, json!({"kind": "done", "text": t, "info": true}));
            }
            pr::Paso::Ronda { orden, visible } => {
                emit(&hub.app, json!({"kind": "pr", "text": visible}));
                run_order(hub, agent, model, orden, visible).await;
                match s.empujar(agent).await {
                    Ok(true) => {}
                    Ok(false) => return emit(&hub.app, json!({"kind": "done", "text": gotv_core::tr!("PR #{}: la ronda no cambió nada; paro aquí", "PR #{}: the round changed nothing; stopping here", s.numero), "info": true})),
                    Err(e) => return emit(&hub.app, json!({"kind": "done", "text": format!("⏹ PR #{}: {e}", s.numero), "info": true})),
                }
            }
        }
    }
}

async fn esperar_stop(hub: &Hub) {
    hub.cancel.store(false, std::sync::atomic::Ordering::Relaxed);
    while !hub.cancel.load(std::sync::atomic::Ordering::Relaxed) {
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }
}

#[tauri::command]
pub async fn agentes_lista(h: State<'_, Hub>) -> Result<Value, String> {
    let root = h.root.lock().await.clone();
    let (ms, malos) = agentes::todos(&root);
    // juez-idioma: datos (claves fijas del JSON que pinta la ventana) — bloque
    Ok(json!({
        "agentes": ms.iter().map(|m| json!({"nombre": m.nombre, "descripcion": m.descripcion, "herramientas": m.herramientas, "modelo": m.modelo, "archivo": m.archivo.display().to_string()})).collect::<Vec<_>>(),
        "malos": malos,
    }))
}

// ───────────────────────── F16.4 ─────────────────────────

#[tauri::command]
pub async fn entorno_lista(h: State<'_, Hub>) -> Result<Value, String> {
    let root = h.root.lock().await.clone();
    Ok(json!(variables::almacen().lista(&root).iter().map(|v| json!({"nombre": v.nombre, "caja": v.caja})).collect::<Vec<_>>()))
}

/// El valor llega aquí y se va al Llavero: no vuelve nunca a la ventana.
#[tauri::command]
pub async fn entorno_guardar(nombre: String, valor: String, caja: bool, h: State<'_, Hub>) -> Result<(), String> {
    let root = h.root.lock().await.clone();
    variables::almacen().guardar(&root, nombre.trim(), &valor, caja).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn entorno_caja(nombre: String, caja: bool, h: State<'_, Hub>) -> Result<(), String> {
    let root = h.root.lock().await.clone();
    variables::almacen().marcar_caja(&root, &nombre, caja).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn entorno_quitar(nombre: String, h: State<'_, Hub>) -> Result<(), String> {
    let root = h.root.lock().await.clone();
    variables::almacen().quitar(&root, &nombre).map_err(|e| e.to_string())
}

// ───────────────────────── F16.7 ─────────────────────────

#[tauri::command]
pub async fn archivo_leer(ruta: String, h: State<'_, Hub>) -> Result<Value, String> {
    let root = h.root.lock().await.clone();
    archivo::leer(&root, &ruta).map(|a| a.json()).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn archivo_guardar(ruta: String, texto: String, version: Option<String>, h: State<'_, Hub>) -> Result<String, String> {
    let root = h.root.lock().await.clone();
    archivo::guardar(&root, &ruta, &texto, version.as_deref().filter(|v| !v.is_empty())).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn terminal_abrir(cols: u16, filas: u16, h: State<'_, Hub>) -> Result<u32, String> {
    let root = h.root.lock().await.clone();
    if root.as_os_str().is_empty() {
        return Err(gotv_core::i18n::t("abre un proyecto primero", "open a project first").into());
    }
    let app = h.app.clone();
    terminal::abrir(&root, cols, filas, Arc::new(move |id, vista| emit(&app, json!({"kind": "terminal", "id": id, "vista": vista})))).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn terminal_escribir(id: u32, datos: String) -> Result<(), String> {
    terminal::escribir(id, datos.as_bytes()).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn terminal_tamano(id: u32, cols: u16, filas: u16) -> Result<(), String> {
    terminal::tamano(id, cols, filas).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn terminal_vista(id: u32) -> Result<Value, String> {
    terminal::vista(id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn terminal_cerrar(id: u32) {
    terminal::cerrar(id);
}

// ───────────────────────── F16.3 ─────────────────────────

/// Los proyectos remotos (la lista de proyectos los marca con su conexión).
#[tauri::command]
pub fn remotos_lista() -> Value {
    json!(gotv_core::lugar::remotos().into_iter().map(|(a, f)| json!({"path": a.display().to_string(), "conexion": f.conexion, "raiz": f.raiz})).collect::<Vec<_>>())
}

/// Crea un proyecto remoto: su ancla aquí y su conexión (`usuario@host[:puerto]`). La huella se fija después en
/// Configuración → Conexiones con el proyecto ya abierto (como cualquier conexión).
#[tauri::command]
pub fn remoto_crear(nombre: String, destino: String, raiz: String, llave: Option<String>) -> Result<String, String> {
    use gotv_core::conexiones as cx;
    let nombre = nombre.trim().to_string();
    let ancla = gotv_core::lugar::crear_ancla(&nombre, &gotv_core::lugar::Ficha { conexion: nombre.clone(), raiz: raiz.trim().to_string() }).map_err(|e| e.to_string())?;
    let (usuario, resto) = match destino.trim().split_once('@') {
        Some((u, x)) => (Some(u.to_string()), x.to_string()),
        None => (None, destino.trim().to_string()),
    };
    let (host, puerto) = match resto.rsplit_once(':') {
        Some((x, p)) if !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()) => (x.to_string(), p.parse().ok()),
        _ => (resto, None),
    };
    let mut reg = cx::Registro::cargar(&ancla).map_err(|e| e.to_string())?;
    let _ = reg.quitar(&nombre);
    reg.agregar(cx::Conexion { nombre: nombre.clone(), host, usuario, puerto, llave: llave.filter(|l| !l.trim().is_empty()), nota: Some(gotv_core::tr!("proyecto remoto en {}", "remote project at {}", raiz.trim())), ..Default::default() })
        .map_err(|e| e.to_string())?;
    reg.guardar(&ancla).map_err(|e| e.to_string())?;
    Ok(ancla.display().to_string())
}
