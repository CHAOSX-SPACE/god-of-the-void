//! F8 · La ventana: dónde va cada cosa (F8.0) y las órdenes de sus piezas (F8.1–F8.7).
//!
//! **F8.0 se decide AQUÍ**, en Rust: la ventana tiene TRES zonas y ninguna más (izquierda: proyectos con sus
//! chats; centro: el chat; derecha: el panel de trabajo con pestañas). Configuración tiene sus pestañas y,
//! dentro de algunas, secciones. La página (`ui/panel.js`) sólo PINTA esta estructura: cada pestaña o
//! sección la llena el `.js` que se registra con su id (`gotvPanel.registrar`, `gotvAjustes.registrar`);
//! una pestaña sin quien la pinte no se enseña (así F16 y F18 se enganchan sin tocar el armazón).
//!
//! Lo demás son órdenes delgadas sobre el núcleo: `session.rs` (chats), `cambios.rs` (panel de cambios),
//! `menciones.rs` (@), `hallar.rs` (⌘K), `sueltas.rs` (charla fuera de proyecto). El estado de la ventana que
//! no es del núcleo (los comentarios del panel que esperan al siguiente mensaje) vive aquí, en Rust.

use crate::worker::{Ag, Msg};
use crate::Hub;
use gotv_core::cambios::Comentario;
use serde::Serialize;
use serde_json::json;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use tauri::State;

// ── F8.0 · la estructura ───────────────────────────────────────────────────────────────────────────────

/// Una pestaña o sección: su id (lo que registra quien la pinta) y la clave de su nombre en `i18n`.
#[derive(Debug, Clone, Copy, Serialize, PartialEq)]
pub struct Hueco {
    pub id: &'static str,
    pub i18n: &'static str,
    /// la fase que la llena (para la entrega y la ayuda; la página no lo usa)
    pub fase: &'static str,
}

const fn h(id: &'static str, i18n: &'static str, fase: &'static str) -> Hueco {
    Hueco { id, i18n, fase }
}

// juez-idioma: datos (de aquí a `ORDENES`: ids de zonas, pestañas y secciones y claves de i18n; el texto
// que se lee está en `ui/i18n-ventana.js`, en los dos idiomas)
/// Las TRES zonas de la ventana. Ninguna zona nueva: lo que venga entra en éstas.
pub const ZONAS: [Hueco; 3] = [h("proyectos", "zonaProyectos", "F8.1"), h("chat", "zonaChat", "—"), h("trabajo", "zonaTrabajo", "F8.0")];

/// El panel de trabajo (zona derecha): una pestaña visible a la vez; se puede despegar.
pub const PANEL: [Hueco; 5] = [
    // juez-idioma: bloque — datos (ids y claves)
    h("tareas", "pTareas", "—"),
    h("cambios", "pCambios", "F8.2"),
    // servidor (F8.7), plan (F7), dispositivo (F18)
    h("vista", "pVista", "F8.7"),
    h("archivo", "pArchivo", "F16.7"),
    h("terminal", "pTerminal", "F16.7"),
];

/// Configuración: las 11 de antes y UNA nueva (Automatizaciones). «Bases de datos» y «Conectores» dejan de
/// ser pestañas: son secciones de Conexiones.
pub const AJUSTES: [Hueco; 12] = [
    // juez-idioma: bloque — datos (ids y claves)
    h("general", "sGeneral", "—"),
    h("model", "sModel", "—"),
    h("library", "sLibrary", "—"),
    h("gen", "sGen", "—"),
    h("context", "sContext", "—"),
    h("prov", "sProv", "—"),
    h("pasos", "sSteps", "—"),
    h("perms", "sPerms", "—"),
    h("seguridad", "sSecurity", "—"),
    h("conex", "sConex", "—"),
    h("automatizaciones", "sAutomat", "F2.3"),
    h("api", "sApi", "—"),
];

/// Las secciones de las pestañas que las tienen.
pub const SECCIONES: [(&str, &[Hueco]); 3] = [
    // juez-idioma: bloque — datos (ids y claves)
    (
        "conex",
        &[
            h("ssh", "cxSsh", "Ola H"),
            h("bases", "sBases", "F1"),
            h("conectores", "cxMcp", "F13.8"),
            // F23 llegó después del plan: los buscadores son conexiones a servicios de fuera, como los demás
            h("buscadores", "sBuscadores", "F23"),
            h("dispositivos", "cxDispositivos", "F18"),
            h("remoto", "cxRemoto", "F16.3"),
        ],
    ),
    // credenciales/sockets (F14.0, F14.1) y modo automático (F14.2): su sitio queda aquí; se ven cuando su
    // hilo registre quien las pinte. Pulido (2026-11-18): «Caja de arena y acceso» se parte; `caja` queda
    // sólo con la caja de arena y salen tres secciones con lo que traía dentro (Chrome, el control del Mac y
    // el acceso a otras apps y al sistema), cada una en su página.
    (
        "seguridad",
        &[
            h("caja", "sgCaja", "C2"),
            h("credenciales", "sgCredenciales", "F14.0/F14.1"),
            h("auto", "sgAuto", "F14.2"),
            h("sitios", "sgSitios", "F7"),
            h("variables", "sgVariables", "F16.4"),
            h("chrome", "sgChrome", "W3"),
            h("mac", "sgMac", "—"),
            h("sistema", "sgSistema", "—"),
        ],
    ),
    ("general", &[h("aspecto", "gAspecto", "—"), h("avisos", "gAvisos", "F2.1"), h("agentes", "gAgentes", "F16.2")]),
];

/// Una página de Configuración (lo que se elige en la barra): su id, su nombre y las piezas que pinta, una
/// debajo de otra. Cada pieza es la clave de un hueco de arriba: `"model"` (pestaña sin secciones) o
/// `"conex/ssh"` (sección); quien pinta se sigue registrando con esa clave (`gotvAjustes.registrar`).
#[derive(Debug, Clone, Copy, Serialize, PartialEq)]
pub struct Pagina {
    pub id: &'static str,
    pub i18n: &'static str,
    pub piezas: &'static [&'static str],
}

const fn pg(id: &'static str, i18n: &'static str, piezas: &'static [&'static str]) -> Pagina {
    Pagina { id, i18n, piezas }
}

/// Un grupo de la barra de Configuración: un título pequeño y sus páginas (como los ajustes de Claude Desktop).
#[derive(Debug, Clone, Copy, Serialize, PartialEq)]
pub struct Grupo {
    pub id: &'static str,
    pub i18n: &'static str,
    pub paginas: &'static [Pagina],
}

/// Diseño (2026-11-18, pedido del Portador: «más organizada, bien agrupada, con mejores flujos»). Lo que se
/// VE en la barra: seis grupos y, en cada uno, sus páginas. Los ids de `AJUSTES`/`SECCIONES` NO cambian (otros
/// hilos se registran con ellos); esto sólo decide en qué grupo y en qué página sale cada uno. Lo frecuente
/// arriba (General, Modelos), lo peligroso y lo técnico al final (Seguridad, Avanzado). Una página reúne piezas
/// pequeñas que se leen juntas (idioma y avisos; modo de permisos y pasos) para no tener páginas de un solo
/// control. Una página sin ninguna pieza pintada no sale (como antes una sección sin pintor).
pub const GRUPOS: [Grupo; 6] = [
    // juez-idioma: bloque — datos (ids y claves)
    Grupo { id: "general", i18n: "grpGeneral", paginas: &[pg("general", "sGeneral", &["general/aspecto", "general/avisos"])] },
    Grupo {
        id: "modelos",
        i18n: "grpModelos",
        paginas: &[
            pg("model", "sModel", &["model"]),
            pg("context", "sContext", &["context"]),
            pg("library", "sLibrary", &["library"]),
            pg("gen", "sGen", &["gen"]),
            pg("prov", "sProv", &["prov"]),
        ],
    },
    Grupo {
        id: "agente",
        i18n: "grpAgente",
        paginas: &[
            pg("perms", "pgConducta", &["perms", "pasos"]),
            pg("general/agentes", "gAgentes", &["general/agentes"]),
            pg("automatizaciones", "sAutomat", &["automatizaciones"]),
        ],
    },
    Grupo {
        id: "conex",
        i18n: "sConex",
        paginas: &[
            pg("conex/conectores", "cxMcp", &["conex/conectores"]),
            pg("conex/buscadores", "sBuscadores", &["conex/buscadores"]),
            pg("conex/bases", "sBases", &["conex/bases"]),
            pg("conex/ssh", "cxSsh", &["conex/ssh"]),
            pg("conex/remoto", "cxRemoto", &["conex/remoto"]),
            pg("conex/dispositivos", "cxDispositivos", &["conex/dispositivos"]),
        ],
    },
    Grupo {
        id: "seguridad",
        i18n: "sSecurity",
        // pulido (2026-11-18): de lo que protege a lo que abre la puerta. La caja de arena (encendida de serie)
        // y los secretos del proyecto arriba; luego lo que deja al agente salir de su carpeta (el navegador con
        // tus sesiones, manejar el Mac, el modo automático) y AL FINAL modificar otras apps y el sistema.
        paginas: &[
            pg("seguridad", "sgCaja", &["seguridad/caja"]),
            pg("seguridad/variables", "sgVariables", &["seguridad/variables"]),
            pg("seguridad/credenciales", "sgCredenciales", &["seguridad/credenciales"]),
            pg("seguridad/navegador", "sgNavegador", &["seguridad/chrome", "seguridad/sitios"]),
            pg("seguridad/mac", "sgMac", &["seguridad/mac"]),
            pg("seguridad/auto", "sgAuto", &["seguridad/auto"]),
            pg("seguridad/sistema", "sgSistema", &["seguridad/sistema"]),
        ],
    },
    Grupo { id: "avanzado", i18n: "grpAvanzado", paginas: &[pg("api", "sApi", &["api"])] },
];

/// Un ajuste suelto (una FILA de una página): lo que encuentra el buscador de Configuración. `id` es lo que
/// el pintor pone en la fila (`setFila("id", …)` o `data-fila="id"`) para que el buscador lleve a ella y la
/// resalte; `i18n` su nombre y `nota` (o "") su explicación, que también se buscan.
#[derive(Debug, Clone, Copy, Serialize, PartialEq)]
pub struct Fila {
    pub pagina: &'static str,
    pub id: &'static str,
    pub i18n: &'static str,
    pub nota: &'static str,
}

const fn fl(pagina: &'static str, id: &'static str, i18n: &'static str, nota: &'static str) -> Fila {
    Fila { pagina, id, i18n, nota }
}

/// Pulido (2026-11-18, pedido del Portador: «el buscador encuentra páginas, no ajustes sueltos»). Los ajustes
/// que se pueden buscar uno por uno, página por página y en el orden en que salen. Las listas que llegan del
/// núcleo (cada conexión, cada modelo, cada servidor MCP…) no van aquí: el buscador encuentra su página.
pub const FILAS: &[Fila] = &[
    // juez-idioma: bloque — datos (ids y claves)
    fl("general", "idioma", "lang", "langNota"),
    fl("general", "tema", "theme", "themeNota"),
    fl("general", "aviso-turno", "avTurno", "avAyuda"),
    fl("general", "aviso-permiso", "avPermiso", "avAyuda"),
    fl("general", "aviso-programadas", "avProgramadas", "avAyuda"),
    fl("model", "modelo", "flModelo", "flModeloNota"),
    fl("context", "contexto", "ctxPreset", "ctxNote"),
    fl("context", "max-tokens", "maxTok", "flMaxTokNota"),
    fl("context", "temperatura", "temp", "tempNote"),
    fl("context", "esfuerzo", "effort", "effortNote"),
    fl("library", "lib-mac", "libMeasure", "flLibMacNota"),
    fl("library", "lib-filtros", "flLibFiltros", "flLibFiltrosNota"),
    fl("gen", "hf-token", "genTokTitle", "genTokNone"),
    fl("prov", "prov-nuevo", "provAdd", "flProvNuevoNota"),
    fl("perms", "modo", "defMode", "flModoNota"),
    fl("perms", "pasos", "flPasos", "flPasosNota"),
    fl("perms", "subagentes", "stepsParalelo", "stepsParaleloA"),
    fl("conex/conectores", "mcp-nuevo", "mcNuevo", "flMcpNuevoNota"),
    fl("conex/buscadores", "bs-paralelo", "bsParalelo", ""),
    fl("conex/buscadores", "bs-local", "bsSoloLocal", ""),
    fl("conex/buscadores", "bs-contacto", "bsContacto", "flBsContactoNota"),
    fl("conex/buscadores", "bs-indice", "bsIndice", "bsSemillas"),
    fl("conex/buscadores", "bs-borrar", "flBsBorrar", "flBsBorrarNota"),
    fl("conex/bases", "bd-nueva", "bdNueva", "flBdNuevaNota"),
    fl("conex/ssh", "ssh-nueva", "cxNueva", "flSshNuevaNota"),
    fl("conex/remoto", "remoto-crear", "remTitulo", "remNota"),
    fl("conex/dispositivos", "android", "dSdk", "flAndroidNota"),
    fl("conex/dispositivos", "xcode", "dXcode", "flXcodeNota"),
    fl("conex/dispositivos", "simuladores", "dSims", "dBajarIos"),
    fl("conex/dispositivos", "idb", "flIdb", "dIdb"),
    fl("seguridad", "caja", "sbTitle", "flCajaNota"),
    fl("seguridad", "caja-dominios", "flCajaDominios", "sbExtra"),
    fl("seguridad/variables", "var-nueva", "flVarNueva", "varNota"),
    fl("seguridad/navegador", "chrome", "flChromeFila", "flChromeNota"),
    fl("seguridad/navegador", "sitios", "sgSitios", "sitAyuda"),
    fl("seguridad/mac", "mac-nativo", "dkTitle", "flMacNota"),
    fl("seguridad/mac", "mac-capturas", "computerOld", "computerNote"),
    fl("seguridad/sistema", "sistema", "secSysTitle", "flSistemaNota"),
    fl("api", "api-activar", "apiEnable", "apiEnableNote"),
    fl("api", "api-red", "apiLan", "apiLanNote"),
    fl("api", "api-nube", "apiCloud", "apiCloudNote"),
    fl("api", "api-puerto", "apiPort", "flApiPuertoNota"),
    fl("api", "api-llave", "apiKey", "apiKeyNote"),
    fl("api", "telefono", "telTitle", "flTelNota"),
];

/// Cada clave que alguien puede pintar: las pestañas sin secciones y cada `pestaña/sección`.
pub fn claves_de_huecos() -> Vec<String> {
    let con_secciones = |id: &str| SECCIONES.iter().any(|s| s.0 == id);
    let mut v: Vec<String> = AJUSTES.iter().filter(|a| !con_secciones(a.id)).map(|a| a.id.to_string()).collect();
    v.extend(SECCIONES.iter().flat_map(|(p, s)| s.iter().map(move |x| format!("{p}/{}", x.id))));
    v
}

/// Las órdenes de la ventana que ofrece ⌘K (además de chats y archivos).
pub const ORDENES: [Hueco; 8] = [
    // juez-idioma: bloque — datos (ids y claves)
    h("nuevo_chat", "oNuevoChat", "F8.1"),
    h("nueva_charla", "oNuevaCharla", "F8.5"),
    h("nuevo_proyecto", "newProject", "—"),
    h("ajustes", "settings", "—"),
    h("panel_tareas", "oPanelTareas", "F8.0"),
    h("panel_cambios", "oPanelCambios", "F8.2"),
    h("panel_vista", "oPanelVista", "F8.7"),
    h("papelera", "oPapelera", "F8.1"),
];

/// El nombre de una orden en los dos idiomas (⌘K filtra por lo que la persona lee, no por el id).
fn texto_orden(id: &str) -> [&'static str; 2] {
    match id {
        // juez-idioma: bloque — datos (el nombre de cada orden en los DOS idiomas, para buscarla en cualquiera)
        "nuevo_chat" => ["chat nuevo en este proyecto", "new chat in this project"],
        "nueva_charla" => ["charla sin proyecto", "chat without a project"],
        "nuevo_proyecto" => ["nuevo proyecto", "new project"],
        "ajustes" => ["configuración", "settings"],
        "panel_tareas" => ["ver las tareas", "show the tasks"],
        "panel_cambios" => ["ver los cambios", "show the changes"],
        "panel_vista" => ["ver la vista del servidor", "show the server view"],
        "papelera" => ["papelera de chats", "chat trash"],
        _ => ["", ""],
    }
}

/// Las órdenes que casan con lo escrito (todas las palabras, en cualquiera de los dos idiomas).
pub fn ordenes_que_casan(consulta: &str) -> Vec<&'static Hueco> {
    let q = gotv_core::hallar::normaliza(consulta.trim());
    ORDENES.iter().filter(|o| q.split_whitespace().all(|p| texto_orden(o.id).iter().any(|t| gotv_core::hallar::normaliza(t).contains(p)))).collect()
}

#[tauri::command]
pub fn ventana_estructura() -> serde_json::Value {
    json!({
        "zonas": ZONAS,
        "panel": PANEL,
        "ajustes": AJUSTES,
        "secciones": SECCIONES.iter().map(|(p, s)| (p.to_string(), json!(s))).collect::<serde_json::Map<_, _>>(),
        "grupos": GRUPOS,
        "filas": FILAS,
        "ordenes": ORDENES,
    })
}

// ── F8.1 · chats del proyecto ──────────────────────────────────────────────────────────────────────────

/// Cuántos chats se mandan por página.
const PAGINA: usize = 30;

async fn raiz(h: &Hub) -> PathBuf {
    h.root.lock().await.clone()
}

fn info_chat(s: &gotv_core::session::SessionInfo) -> serde_json::Value {
    json!({"id": s.id, "title": s.title, "created": s.created, "messages": s.messages, "parent": s.parent.map(|p| p.0)})
}

/// Una página de los chats de un proyecto (el abierto, si no se dice), con el que está abierto marcado.
#[tauri::command]
pub async fn chats_lista(proyecto: Option<String>, desde: Option<usize>, h: State<'_, Hub>) -> Result<serde_json::Value, String> {
    let actual_raiz = raiz(&h).await;
    let root = proyecto.map(PathBuf::from).unwrap_or_else(|| actual_raiz.clone());
    if root.as_os_str().is_empty() {
        return Ok(json!({"chats": [], "total": 0, "actual": null, "papelera": 0}));
    }
    let (v, total) = gotv_core::session::Session::page(&root, desde.unwrap_or(0), PAGINA).map_err(|e| e.to_string())?;
    let actual = if same(&root, &actual_raiz) { h.status.lock().await["chat"].as_i64() } else { None };
    let papelera = gotv_core::session::Session::papelera(&root).map(|p| p.len()).unwrap_or(0);
    // F8.5: una charla sin proyecto no se nombra por su carpeta (un sello de fecha)
    let suelta = gotv_core::sueltas::es_suelta(&root);
    Ok(json!({"chats": v.iter().map(info_chat).collect::<Vec<_>>(), "total": total, "actual": actual, "papelera": papelera, "pagina": PAGINA, "suelta": suelta}))
}

fn same(a: &Path, b: &Path) -> bool {
    let c = |p: &Path| p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
    !a.as_os_str().is_empty() && c(a) == c(b)
}

/// Abre un chat. Si es de OTRO proyecto (de la lista, ya confiado), abre antes ese proyecto.
#[tauri::command]
pub async fn chat_abrir(proyecto: Option<String>, id: Option<i64>, h: State<'_, Hub>) -> Result<(), String> {
    let actual = raiz(&h).await;
    if let Some(p) = proyecto.map(PathBuf::from).filter(|p| !same(p, &actual)) {
        let p = p.canonicalize().map_err(|e| e.to_string())?;
        let st = crate::store::Store::load();
        if !st.is_trusted(&p) && !gotv_core::sueltas::es_suelta(&p) {
            return Err("untrusted".into());
        }
        h.tx.send(Msg::Open(p)).map_err(|e| e.to_string())?;
    }
    h.tx.send(Msg::Chat(id)).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn chat_renombrar(proyecto: Option<String>, id: i64, titulo: String, h: State<'_, Hub>) -> Result<(), String> {
    let root = proyecto.map(PathBuf::from).unwrap_or(raiz(&h).await);
    gotv_core::session::Session::de(&root, id).and_then(|s| s.rename(&titulo)).map_err(|e| e.to_string())
}

/// Bifurca el chat abierto en el mensaje `hasta` (su lugar en el historial) y abre el nuevo. Devuelve el aviso
/// de que el nuevo no hereda la memoria rápida.
#[tauri::command]
pub async fn chat_bifurcar(id: i64, hasta: usize, h: State<'_, Hub>) -> Result<String, String> {
    let root = raiz(&h).await;
    let s = gotv_core::session::Session::fork(&root, id, hasta).map_err(|e| e.to_string())?;
    h.tx.send(Msg::Chat(Some(s.id))).map_err(|e| e.to_string())?;
    Ok(gotv_core::tr!(
        "chat #{} creado desde el mensaje {hasta}: su primera orden relee el historial (la memoria rápida es de cada chat)",
        "chat #{} created from message {hasta}: its first order rereads the history (fast memory is per chat)",
        s.id
    ))
}

/// Exporta un chat a Markdown o JSON con el diálogo NATIVO de guardar (abre en Descargas). `None` = cancelado.
#[tauri::command]
pub async fn chat_exportar(app: tauri::AppHandle, proyecto: Option<String>, id: i64, formato: String, h: State<'_, Hub>) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    let root = proyecto.map(PathBuf::from).unwrap_or(raiz(&h).await);
    let f = gotv_core::session::Formato::parse(&formato).ok_or_else(|| gotv_core::tr!("formato desconocido: {formato}", "unknown format: {formato}"))?;
    let texto = gotv_core::session::Session::export(&root, id, f).map_err(|e| e.to_string())?;
    let mut d = app.dialog().file().set_file_name(format!("chat-{id}.{}", f.extension())).add_filter(f.extension().to_uppercase(), &[f.extension()]);
    if let Some(casa) = std::env::var_os("HOME") {
        d = d.set_directory(PathBuf::from(casa).join("Downloads"));
    }
    let (tx, rx) = tokio::sync::oneshot::channel();
    d.save_file(move |p| {
        let _ = tx.send(p.and_then(|p| p.into_path().ok()));
    });
    let Some(destino) = rx.await.ok().flatten() else { return Ok(None) };
    std::fs::write(&destino, texto).map_err(|e| e.to_string())?;
    Ok(Some(destino.display().to_string()))
}

/// Manda un chat a la papelera (sólo lo marca). Si era el abierto, se pasa a uno nuevo.
#[tauri::command]
pub async fn chat_papelera(proyecto: Option<String>, id: i64, h: State<'_, Hub>) -> Result<(), String> {
    let actual_raiz = raiz(&h).await;
    let root = proyecto.map(PathBuf::from).unwrap_or_else(|| actual_raiz.clone());
    gotv_core::session::Session::trash(&root, id).map_err(|e| e.to_string())?;
    if same(&root, &actual_raiz) && h.status.lock().await["chat"].as_i64() == Some(id) {
        h.tx.send(Msg::Chat(None)).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub async fn chat_restaurar(id: i64, h: State<'_, Hub>) -> Result<(), String> {
    gotv_core::session::Session::restore(&raiz(&h).await, id).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn papelera_lista(h: State<'_, Hub>) -> Result<serde_json::Value, String> {
    let v = gotv_core::session::Session::papelera(&raiz(&h).await).map_err(|e| e.to_string())?;
    Ok(json!({"chats": v.iter().map(|s| { let mut j = info_chat(s); j["deleted_at"] = json!(s.deleted_at); j }).collect::<Vec<_>>(), "dias": gotv_core::session::DIAS_PAPELERA}))
}

/// «Vaciar»: borra de verdad los chats que llevan más de 30 días en la papelera. Devuelve cuántos.
#[tauri::command]
pub async fn papelera_vaciar(h: State<'_, Hub>) -> Result<usize, String> {
    gotv_core::session::Session::vaciar(&raiz(&h).await, gotv_core::session::DIAS_PAPELERA).map_err(|e| e.to_string())
}

// ── F8.2 · panel de cambios ────────────────────────────────────────────────────────────────────────────

/// Los turnos con cambios del proyecto abierto (`todos = false`: sólo los del chat abierto).
#[tauri::command]
pub async fn cambios_turnos(todos: Option<bool>, h: State<'_, Hub>) -> Result<serde_json::Value, String> {
    let root = raiz(&h).await;
    if root.as_os_str().is_empty() {
        return Ok(json!({"turnos": [], "git": false}));
    }
    let chat = if todos.unwrap_or(false) { None } else { h.status.lock().await["chat"].as_i64() };
    let cp = gotv_core::checkpoint::Checkpoint::new(&root);
    let git = gotv_core::git::leer::abrir(&root).is_ok();
    Ok(json!({"turnos": gotv_core::cambios::turnos(&cp, chat), "git": git}))
}

#[tauri::command]
pub async fn cambios_turno(n: u32, h: State<'_, Hub>) -> Result<Vec<gotv_core::cambios::Cambio>, String> {
    let cp = gotv_core::checkpoint::Checkpoint::new(&raiz(&h).await);
    gotv_core::cambios::por_turno(&cp, n).map_err(|e| e.to_string())
}

/// Con git: `vista` = `sin_confirmar` o `rama` (contra `base`, `main` si no se dice).
#[tauri::command]
pub async fn cambios_git(vista: String, base: Option<String>, h: State<'_, Hub>) -> Result<Vec<gotv_core::cambios::Cambio>, String> {
    let root = raiz(&h).await;
    let r = match vista.as_str() {
        "rama" => gotv_core::cambios::de_rama(&root, base.as_deref().filter(|b| !b.trim().is_empty()).unwrap_or("main")),
        _ => gotv_core::cambios::sin_confirmar(&root),
    };
    r.map_err(|e| format!("{e:#}"))
}

#[tauri::command]
pub async fn cambios_revertir(n: u32, ruta: String, forzar: Option<bool>, h: State<'_, Hub>) -> Result<String, String> {
    let root = raiz(&h).await;
    let cp = gotv_core::checkpoint::Checkpoint::new(&root);
    gotv_core::cambios::revertir_archivo(&cp, n, &root.join(&ruta), forzar.unwrap_or(false)).map_err(|e| format!("{e:#}"))
}

/// F8.2 · Confirmar (commit) lo marcado en «Sin confirmar». Lo decide la persona; sin push (en pausa).
#[tauri::command]
pub async fn cambios_confirmar(rutas: Vec<String>, mensaje: String, h: State<'_, Hub>) -> Result<String, String> {
    gotv_core::cambios::confirmar(&raiz(&h).await, &rutas, &mensaje).await.map_err(|e| format!("{e:#}"))
}

/// Los comentarios del panel que esperan al siguiente mensaje, por proyecto.
fn comentarios() -> &'static Mutex<HashMap<PathBuf, Vec<Comentario>>> {
    static C: OnceLock<Mutex<HashMap<PathBuf, Vec<Comentario>>>> = OnceLock::new();
    C.get_or_init(Default::default)
}

fn comentarios_json(v: &[Comentario]) -> serde_json::Value {
    json!(v.iter().map(|c| json!({"ruta": c.ruta, "linea": c.linea_nueva, "texto": c.texto, "como_viaja": c.para_el_mensaje()})).collect::<Vec<_>>())
}

/// Comenta una línea (del lado NUEVO): viaja en tu siguiente mensaje.
#[tauri::command]
pub async fn cambios_comentar(ruta: String, linea: u32, texto: String, h: State<'_, Hub>) -> Result<serde_json::Value, String> {
    if texto.trim().is_empty() {
        return Err(gotv_core::i18n::t("el comentario está vacío", "the comment is empty").into());
    }
    let root = raiz(&h).await;
    let mut m = comentarios().lock().unwrap_or_else(|e| e.into_inner());
    let v = m.entry(root).or_default();
    v.push(Comentario { ruta, linea_nueva: linea, texto: texto.trim().to_string() });
    Ok(comentarios_json(v))
}

#[tauri::command]
pub async fn cambios_comentarios(quitar: Option<usize>, h: State<'_, Hub>) -> Result<serde_json::Value, String> {
    let root = raiz(&h).await;
    let mut m = comentarios().lock().unwrap_or_else(|e| e.into_inner());
    let v = m.entry(root).or_default();
    if let Some(i) = quitar.filter(|i| *i < v.len()) {
        v.remove(i);
    }
    Ok(comentarios_json(v))
}

/// Lo que se añade a una orden antes de mandarla al agente: los comentarios del panel (delante, y se vacían)
/// y los anexos de lo mencionado con @ (chats y recursos MCP, detrás). Devuelve (para el modelo, lo escrito).
pub async fn preparar_envio(hub: &Hub, agent: &mut Ag, texto: String, escrito: String) -> (String, String) {
    let cs = comentarios().lock().unwrap_or_else(|e| e.into_inner()).remove(&agent.root).unwrap_or_default();
    let (texto, escrito) = if cs.is_empty() { (texto, escrito) } else { (gotv_core::cambios::con_comentarios(&texto, &cs), gotv_core::cambios::con_comentarios(&escrito, &cs)) };
    if !cs.is_empty() {
        crate::emit(&hub.app, json!({"kind": "comentarios", "lista": [], "enviados": cs.len()}));
    }
    let mut anexos = gotv_core::menciones::anexos_de_chats(&agent.root, &texto);
    if let Some(m) = agent.mcp.as_mut() {
        for f in gotv_core::menciones::fichas(&texto).into_iter().filter(|f| f.starts_with("mcp:")) {
            let Some((servidor, uri)) = f[4..].split_once(':') else { continue };
            let t = m.leer_recurso(servidor, uri).await.unwrap_or_else(|e| format!("{e:#}"));
            anexos.push(gotv_core::menciones::anexo(&f, servidor, &t));
        }
    }
    (gotv_core::menciones::con_anexos(&texto, &anexos), escrito)
}

// ── F8.3 · menciones con @ ─────────────────────────────────────────────────────────────────────────────

/// Los recursos MCP vistos por última vez (se piden al worker con el agente libre; valen un minuto).
fn recursos_cache() -> &'static Mutex<Option<(std::time::Instant, PathBuf, Vec<(String, String, String, String)>)>> {
    static R: OnceLock<Mutex<Option<(std::time::Instant, PathBuf, Vec<(String, String, String, String)>)>>> = OnceLock::new();
    R.get_or_init(Default::default)
}

async fn recursos_mcp(h: &Hub, root: &Path) -> Vec<(String, String, String, String)> {
    if let Some((t, r, v)) = recursos_cache().lock().unwrap_or_else(|e| e.into_inner()).clone() {
        if r == root && t.elapsed().as_secs() < 60 {
            return v;
        }
    }
    let (tx, rx) = tokio::sync::oneshot::channel();
    if h.tx.send(Msg::Recursos(tx)).is_err() {
        return vec![];
    }
    // con el agente trabajando no hay respuesta pronto: la caja no espera (se ofrecen los demás)
    match tokio::time::timeout(std::time::Duration::from_millis(1200), rx).await {
        Ok(Ok(v)) => {
            *recursos_cache().lock().unwrap_or_else(|e| e.into_inner()) = Some((std::time::Instant::now(), root.to_path_buf(), v.clone()));
            v
        }
        _ => vec![],
    }
}

/// Lo que casa con lo escrito tras la @: archivos y carpetas (sin lo ignorado), chats, reglas y recursos MCP.
#[tauri::command]
pub async fn menciones(consulta: String, h: State<'_, Hub>) -> Result<Vec<gotv_core::menciones::Mencion>, String> {
    let root = raiz(&h).await;
    if root.as_os_str().is_empty() {
        return Ok(vec![]);
    }
    let mcp = recursos_mcp(&h, &root).await;
    let q = consulta.clone();
    tokio::task::spawn_blocking(move || gotv_core::menciones::buscar(&root, &q, &mcp)).await.map_err(|e| e.to_string())
}

/// Citar un trozo seleccionado (de una respuesta, un archivo o un diff), listo para la caja.
#[tauri::command]
pub fn citar(origen: String, texto: String) -> String {
    gotv_core::menciones::cita(&origen, &texto)
}

// ── F8.6 · ⌘K: chats de todos los proyectos, archivos del abierto y órdenes ────────────────────────────────

#[tauri::command]
pub async fn buscar_todo(consulta: String, h: State<'_, Hub>) -> Result<serde_json::Value, String> {
    let root = raiz(&h).await;
    let q = consulta.trim().to_string();
    let ordenes = ordenes_que_casan(&q);
    let mut proyectos: Vec<(String, PathBuf)> = crate::store::Store::load().live_projects().into_iter().map(|p| (p.name, PathBuf::from(p.path))).collect();
    proyectos.extend(gotv_core::sueltas::lista().into_iter().map(|s| (gotv_core::i18n::t("charla", "chat").to_string(), PathBuf::from(s.ruta))));
    let r2 = root.clone();
    let (chats, archivos) = tokio::task::spawn_blocking(move || {
        let chats = if q.chars().count() >= 2 { gotv_core::hallar::en_chats(&proyectos, &q, gotv_core::hallar::MAX) } else { vec![] };
        let archivos = if r2.as_os_str().is_empty() || q.is_empty() { vec![] } else { gotv_core::menciones::archivos(&r2, &q, 8) };
        (chats, archivos)
    })
    .await
    .map_err(|e| e.to_string())?;
    Ok(json!({"chats": chats, "archivos": archivos, "ordenes": ordenes, "proyecto": root.display().to_string()}))
}

// ── F8.5 · charla fuera de proyecto ────────────────────────────────────────────────────────────────────

/// Una charla nueva en su propia carpeta, con permisos mínimos. No entra en la lista de proyectos.
#[tauri::command]
pub async fn suelta_nueva(h: State<'_, Hub>) -> Result<String, String> {
    let p = gotv_core::sueltas::nueva().map_err(|e| e.to_string())?;
    h.tx.send(Msg::Open(p.clone())).map_err(|e| e.to_string())?;
    Ok(p.display().to_string())
}

#[tauri::command]
pub fn sueltas_lista() -> Vec<gotv_core::sueltas::Suelta> {
    gotv_core::sueltas::lista()
}

/// Abre una charla suelta que ya existe (sólo de la carpeta de charlas).
#[tauri::command]
pub async fn suelta_abrir(ruta: String, h: State<'_, Hub>) -> Result<(), String> {
    let p = PathBuf::from(&ruta);
    if !gotv_core::sueltas::es_suelta(&p) || !p.is_dir() {
        return Err(gotv_core::i18n::t("no es una charla de GOTV", "not a GOTV chat").into());
    }
    h.tx.send(Msg::Open(p)).map_err(|e| e.to_string())
}

// ── F8.7 · vista previa del servidor ───────────────────────────────────────────────────────────────────

/// ¿Es una URL de ESTA Mac (localhost, 127.0.0.1, [::1]) por http(s)? Sólo esas se abren en «Vista».
pub fn url_local(url: &str) -> Option<String> {
    let u = url.trim();
    let resto = u.strip_prefix("http://").or_else(|| u.strip_prefix("https://"))?;
    let host = resto.split(['/', '?', '#']).next()?;
    let sin_puerto = if host.starts_with('[') { host.split(']').next().map(|x| format!("{x}]"))? } else { host.split(':').next()?.to_string() };
    if host.contains('@') {
        return None; // usuario@host: nada de credenciales ni engaños
    }
    // la CSP de la ventana (frame-src) nombra localhost y 127.0.0.1: [::1] y 0.0.0.0 se dicen así
    matches!(sin_puerto.to_ascii_lowercase().as_str(), "localhost" | "127.0.0.1" | "[::1]" | "0.0.0.0").then(|| u.replacen("://0.0.0.0", "://127.0.0.1", 1).replacen("://[::1]", "://localhost", 1))
}

/// La URL lista para la pestaña Vista (o el error si no es de esta Mac).
#[tauri::command]
pub fn vista_url(url: String) -> Result<String, String> {
    url_local(&url).ok_or_else(|| gotv_core::tr!("«Vista» sólo abre direcciones de esta Mac (localhost): {url}", "«View» only opens addresses on this Mac (localhost): {url}"))
}

/// Lo que se le pide al agente para revisar la página con su navegador.
#[tauri::command]
pub fn vista_pedir_revision(url: String) -> Result<String, String> {
    let u = vista_url(url)?;
    Ok(gotv_core::tr!(
        "Revisa {u} con el navegador (browser): ábrela, mira la consola y dime qué ves y si algo falla.",
        "Review {u} with the browser: open it, check the console and tell me what you see and whether anything fails."
    ))
}

// ── F8.4 · varias carpetas por proyecto ────────────────────────────────────────────────────────────────

fn carpetas_json(r: &gotv_core::raices::Raices) -> serde_json::Value {
    json!(r.extra.iter().map(|(n, p)| json!({"nombre": n, "ruta": p.display().to_string()})).collect::<Vec<_>>())
}

/// Las carpetas extra del proyecto abierto.
#[tauri::command]
pub async fn carpetas_lista(h: State<'_, Hub>) -> Result<serde_json::Value, String> {
    let root = raiz(&h).await;
    if root.as_os_str().is_empty() {
        return Ok(json!([]));
    }
    Ok(carpetas_json(&gotv_core::raices::Raices::de_proyecto(&root)))
}

/// Añade una carpeta extra (la eligió el Portador con el selector de macOS). El modelo la conoce desde la
/// orden siguiente (`@nombre/…`).
#[tauri::command]
pub async fn carpetas_agregar(ruta: String, nombre: Option<String>, h: State<'_, Hub>) -> Result<serde_json::Value, String> {
    let root = raiz(&h).await;
    let mut r = gotv_core::raices::Raices::de_proyecto(&root);
    r.agregar(Path::new(&ruta), nombre.as_deref()).map_err(|e| e.to_string())?;
    r.guardar().map_err(|e| e.to_string())?;
    Ok(carpetas_json(&r))
}

/// Quita una carpeta extra de la lista (no borra nada de ella).
#[tauri::command]
pub async fn carpetas_quitar(nombre: String, h: State<'_, Hub>) -> Result<serde_json::Value, String> {
    let root = raiz(&h).await;
    let mut r = gotv_core::raices::Raices::de_proyecto(&root);
    r.quitar(&nombre).map_err(|e| e.to_string())?;
    r.guardar().map_err(|e| e.to_string())?;
    Ok(carpetas_json(&r))
}

// ── Qué proyectos quedan desplegados en la zona izquierda (F8.1) ──────────────────────────────────────
// En el formato propio de GOTV (regla del Portador: lo nuestro, sólo Rust; antes vivía en el `localStorage`
// del navegador, como JSON).

fn ruta_abiertos() -> std::path::PathBuf {
    gotv_core::paths::home().join("Library/Application Support/gotv/ventana-abiertos.gotv")
}

fn leer_abiertos(ruta: &std::path::Path) -> Vec<String> {
    let Ok(b) = std::fs::read(ruta) else { return vec![] };
    gotv_core::programar::codec::Lector::nuevo(&b, b"ABIE", 1).and_then(|(mut l, _)| l.textos()).unwrap_or_default()
}

fn guardar_abiertos(ruta: &std::path::Path, rutas: &[String]) -> Result<(), String> {
    let mut e = gotv_core::programar::codec::Escritor::nuevo(b"ABIE", 1);
    // sin repetir y con tope: una lista de rutas de proyecto, no un registro que crezca sin fin
    let mut v: Vec<String> = Vec::new();
    for r in rutas.iter().filter(|r| !r.is_empty()) {
        if !v.contains(r) {
            v.push(r.clone());
        }
    }
    v.truncate(200);
    e.textos(&v);
    if let Some(d) = ruta.parent() {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    gotv_core::programar::codec::guardar(ruta, &e.bytes()).map_err(|e| e.to_string())
}

// ── Preferencias pequeñas de la ventana (clave → texto), en el formato propio: lo que antes guardaba el
// navegador en su `localStorage` ──────────────────────────────────────────────────────────────────────

fn ruta_prefs() -> std::path::PathBuf {
    gotv_core::paths::home().join("Library/Application Support/gotv/ventana-prefs.gotv")
}

fn leer_prefs(ruta: &std::path::Path) -> Vec<(String, String)> {
    let Ok(b) = std::fs::read(ruta) else { return vec![] };
    let Ok((mut l, _)) = gotv_core::programar::codec::Lector::nuevo(&b, b"PREF", 1) else { return vec![] };
    let (Ok(k), Ok(v)) = (l.textos(), l.textos()) else { return vec![] };
    k.into_iter().zip(v).collect()
}

fn poner_pref(ruta: &std::path::Path, clave: &str, valor: &str) -> Result<(), String> {
    if clave.is_empty() || clave.len() > 80 || valor.len() > 4096 {
        return Err(gotv_core::i18n::t("preferencia no válida", "invalid preference").into());
    }
    let mut p = leer_prefs(ruta);
    p.retain(|(k, _)| k != clave);
    p.push((clave.to_string(), valor.to_string()));
    let sobra = p.len().saturating_sub(200);
    p.drain(..sobra);
    let mut e = gotv_core::programar::codec::Escritor::nuevo(b"PREF", 1);
    e.textos(&p.iter().map(|(k, _)| k.clone()).collect::<Vec<_>>());
    e.textos(&p.iter().map(|(_, v)| v.clone()).collect::<Vec<_>>());
    if let Some(d) = ruta.parent() {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    gotv_core::programar::codec::guardar(ruta, &e.bytes()).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn ventana_pref(clave: String) -> Option<String> {
    leer_prefs(&ruta_prefs()).into_iter().find(|(k, _)| *k == clave).map(|(_, v)| v)
}

#[tauri::command]
pub fn ventana_pref_poner(clave: String, valor: String) -> Result<(), String> {
    poner_pref(&ruta_prefs(), &clave, &valor)
}

#[tauri::command]
pub fn chats_abiertos() -> Vec<String> {
    leer_abiertos(&ruta_abiertos())
}

#[tauri::command]
pub fn chats_abiertos_guardar(rutas: Vec<String>) -> Result<(), String> {
    guardar_abiertos(&ruta_abiertos(), &rutas)
}

// ── Configuración: secciones nuevas de F8.0 ────────────────────────────────────────────────────────────

/// Automatizaciones (F2.3): las tareas programadas con su estado, y si el agente de inicio de sesión está
/// (sin él sólo corren cuando se lanzan). Todo sale de `gotv_core::programar::ventana` (probado allí).
#[tauri::command]
pub fn automatizaciones_lista() -> Result<serde_json::Value, String> {
    let e = gotv_core::programar::ventana::estado().map_err(|e| e.to_string())?;
    Ok(json!({"tareas": e.tareas, "agente": e.agente}))
}

#[tauri::command]
pub fn automatizacion_activar(id: String, activa: bool) -> Result<(), String> {
    gotv_core::programar::ventana::activar(&gotv_core::programar::tareas::ruta_por_omision(), &id, activa).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn automatizacion_quitar(id: String) -> Result<(), String> {
    gotv_core::programar::ventana::quitar(&gotv_core::programar::tareas::ruta_por_omision(), &id).map_err(|e| e.to_string())
}

/// Seguridad → sitios del navegador (F7): el texto de reglas del proyecto abierto.
#[tauri::command]
pub async fn sitios_texto(h: State<'_, Hub>) -> Result<String, String> {
    gotv_core::artefactos::sitios::Sitios::leer(&raiz(&h).await).map(|s| s.a_texto()).map_err(|e| format!("{e:#}"))
}

/// Guarda las reglas de sitios. Una línea que no se entiende: no se guarda nada (falla cerrado).
#[tauri::command]
pub async fn sitios_guardar(texto: String, h: State<'_, Hub>) -> Result<(), String> {
    let s = gotv_core::artefactos::sitios::Sitios::desde_texto(&texto).map_err(|e| format!("{e:#}"))?;
    s.guardar(&raiz(&h).await).map_err(|e| format!("{e:#}"))
}

/// General → avisos (F2.1): los interruptores que valen para el proyecto abierto.
#[tauri::command]
pub async fn avisos_estado(h: State<'_, Hub>) -> Result<serde_json::Value, String> {
    let root = raiz(&h).await;
    let i = gotv_core::avisos::Interruptores::cargar((!root.as_os_str().is_empty()).then_some(root.as_path()));
    Ok(json!({"turno": i.turno, "permiso": i.permiso, "programadas": i.programadas}))
}

/// Guarda los interruptores en la config GENERAL (`[avisos]`), sin tocar el resto del archivo. Si la config
/// está ROTA no se escribe nada (antes se leía como vacía y se reescribía sólo con los avisos: se perdía todo
/// lo demás).
#[tauri::command]
pub fn avisos_guardar(turno: bool, permiso: bool, programadas: bool) -> Result<(), String> {
    let ruta = gotv_core::config::global_path();
    for (k, v) in [("turno", turno), ("permiso", permiso), ("programadas", programadas)] {
        gotv_core::programar::ventana::poner_aviso(&ruta, k, v).map_err(|e| format!("{e:#}"))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Lo desplegado va y vuelve en el formato propio (cabecera GOTV, no JSON), sin repetir ni vacíos;
    /// sin archivo o dañado, nada desplegado.
    #[test]
    fn los_proyectos_desplegados_en_formato_propio() {
        let d = std::env::temp_dir().join(format!("gotv-abiertos-{}", std::process::id()));
        let r = d.join("abiertos.gotv");
        assert!(leer_abiertos(&r).is_empty());
        guardar_abiertos(&r, &["/a".into(), "".into(), "/b".into(), "/a".into()]).unwrap();
        assert_eq!(&std::fs::read(&r).unwrap()[..4], b"GOTV");
        assert_eq!(leer_abiertos(&r), ["/a", "/b"]);
        std::fs::write(&r, b"{\"no\": 1}").unwrap();
        assert!(leer_abiertos(&r).is_empty());
        // las preferencias: una por clave (la última manda), con límites
        let p = d.join("prefs.gotv");
        poner_pref(&p, "proyectos", "1").unwrap();
        poner_pref(&p, "proyectos", "0").unwrap();
        poner_pref(&p, "otra", "ñ").unwrap();
        assert_eq!(leer_prefs(&p), [("proyectos".to_string(), "0".to_string()), ("otra".to_string(), "ñ".to_string())]);
        assert!(poner_pref(&p, "", "x").is_err() && poner_pref(&p, "k", &"x".repeat(5000)).is_err());
        let _ = std::fs::remove_dir_all(&d);
    }

    const INDEX: &str = include_str!("../ui/index.html");
    const I18N: &str = include_str!("../ui/i18n.js");
    const I18N_V: &str = include_str!("../ui/i18n-ventana.js");
    const PANEL_JS: &str = include_str!("../ui/panel.js");
    const I18N_E: &str = include_str!("../ui/escritorio-i18n.js");
    const DISP_JS: &str = include_str!("../ui/dispositivos.js");
    /// Los pintores de Configuración (donde cada fila lleva su id).
    const PINTORES: [&str; 4] = [include_str!("../ui/app.js"), include_str!("../ui/ajustes-f8.js"), include_str!("../ui/escritorio-ajustes.js"), DISP_JS];

    /// F8.0: la ventana sigue teniendo TRES zonas (y el panel de trabajo es la derecha).
    #[test]
    fn la_ventana_tiene_tres_zonas_y_ninguna_mas() {
        let cuenta = |s: &str| INDEX.matches(s).count();
        assert_eq!(cuenta("<aside class=\"side\""), 1);
        assert_eq!(cuenta("<main class=\"chat\""), 1);
        assert_eq!(cuenta("<aside class=\"panel\""), 1);
        assert_eq!(cuenta("<aside"), 2, "dos aside (izquierda y derecha) y ninguno más");
        assert_eq!(cuenta("<main"), 1);
        assert_eq!(cuenta("data-zona="), 3);
        for z in ZONAS {
            assert!(INDEX.contains(&format!("data-zona=\"{}\"", z.id)), "falta la zona {}", z.id);
        }
    }

    /// El panel de trabajo tiene las cinco pestañas, en orden; Configuración gana UNA pestaña (12) y Conexiones
    /// tiene sus secciones (SSH · Bases de datos · Conectores MCP · … · Dispositivos).
    #[test]
    fn panel_y_ajustes_como_dice_f8_0() {
        assert_eq!(PANEL.map(|p| p.id), ["tareas", "cambios", "vista", "archivo", "terminal"]);
        assert_eq!(AJUSTES.len(), 12);
        assert_eq!(AJUSTES.iter().filter(|a| a.fase != "—").count(), 1, "una sola nueva");
        assert!(AJUSTES.iter().all(|a| a.id != "bases" && a.id != "conectores" && a.id != "buscadores"), "pasan a ser secciones");
        let conex = SECCIONES.iter().find(|s| s.0 == "conex").unwrap().1;
        for s in ["ssh", "bases", "conectores", "dispositivos"] {
            assert!(conex.iter().any(|x| x.id == s), "Conexiones → {s}");
        }
        // la barra de Configuración ya no está escrita a mano: la pinta panel.js desde aquí
        assert!(!INDEX.contains("data-p=\"bases\""));
        let e = ventana_estructura();
        assert_eq!(e["panel"].as_array().unwrap().len(), 5);
        assert_eq!(e["secciones"]["conex"][4]["id"], "dispositivos");
    }

    /// Cada nombre que la estructura promete existe en los dos idiomas.
    #[test]
    fn cada_clave_tiene_su_texto_en_los_dos_idiomas() {
        let mut todas: Vec<&str> = ZONAS.iter().chain(PANEL.iter()).chain(AJUSTES.iter()).chain(ORDENES.iter()).chain(SECCIONES.iter().flat_map(|s| s.1.iter())).map(|x| x.i18n).collect();
        todas.extend(GRUPOS.iter().map(|g| g.i18n).chain(GRUPOS.iter().flat_map(|g| g.paginas.iter().map(|p| p.i18n))));
        todas.extend(FILAS.iter().flat_map(|f| [f.i18n, f.nota]).filter(|k| !k.is_empty()));
        // i18n.js pone a veces varias claves en una línea: se mira sólo el par de ESTA clave
        let par_de = |k: &str| -> Option<String> {
            let marca = format!("{k}: [");
            I18N.lines().chain(I18N_V.lines()).chain(I18N_E.lines()).chain(DISP_JS.lines()).find_map(|l| {
                let i = l.match_indices(&marca).map(|(i, _)| i).find(|i| !l[..*i].chars().next_back().is_some_and(|c| c.is_alphanumeric() || c == '_'))?;
                let resto = &l[i + marca.len()..];
                Some(resto[..resto.find(']')?].to_string())
            })
        };
        for k in todas {
            let par = par_de(k).unwrap_or_else(|| panic!("falta {k} en i18n"));
            assert_eq!(par.matches("\", \"").count() + par.matches("\",\"").count(), 1, "{k}: un par [es, en]: [{par}]");
        }
    }

    /// Diseño (2026-11-18): la barra de Configuración son seis grupos con sus páginas. Cada hueco que alguien
    /// puede pintar sale en UNA página y sólo en una (ni perdido ni repetido en dos sitios); cada pieza de una
    /// página es un hueco que existe; los ids de página no se repiten y una página que abre otro sitio por su
    /// nombre de antes («model», «seguridad», «automatizaciones», «conex/dispositivos») sigue llegando.
    #[test]
    fn la_barra_de_configuracion_agrupa_cada_hueco_una_vez() {
        let huecos = claves_de_huecos();
        let piezas: Vec<&str> = GRUPOS.iter().flat_map(|g| g.paginas.iter().flat_map(|p| p.piezas.iter().copied())).collect();
        for h in &huecos {
            assert_eq!(piezas.iter().filter(|p| *p == h).count(), 1, "{h}: una página y sólo una");
        }
        for p in &piezas {
            assert!(huecos.iter().any(|h| h == p), "{p}: pieza sin hueco en AJUSTES/SECCIONES");
        }
        let ids: Vec<&str> = GRUPOS.iter().flat_map(|g| g.paginas.iter().map(|p| p.id)).collect();
        let mut sin_rep = ids.clone();
        sin_rep.sort();
        sin_rep.dedup();
        assert_eq!(sin_rep.len(), ids.len(), "ids de página repetidos");
        // los que abren Configuración por su nombre de antes: página propia o primera que lo contiene
        for k in ["general", "model", "seguridad", "automatizaciones", "conex/dispositivos", "api"] {
            let llega = ids.contains(&k) || piezas.iter().any(|p| *p == k || p.starts_with(&format!("{k}/")));
            assert!(llega, "openSettings(\"{k}\") ya no llega a ninguna página");
        }
        // grupos ordenados como se decidió: lo frecuente arriba, lo técnico al final
        assert_eq!(GRUPOS.map(|g| g.id), ["general", "modelos", "agente", "conex", "seguridad", "avanzado"]);
        assert!(GRUPOS.iter().all(|g| !g.paginas.is_empty()));
        assert_eq!(ventana_estructura()["grupos"][1]["paginas"][0]["piezas"][0], "model");
    }

    /// Pulido (2026-11-18): «Caja de arena y acceso» quedó partida en páginas propias dentro de Seguridad; la
    /// caja de arena sigue siendo la primera (`openSettings("seguridad")`) y modificar otras apps y el sistema
    /// la ÚLTIMA. Ninguna página de Seguridad reúne ya la caja con lo demás.
    #[test]
    fn la_caja_de_arena_quedo_partida_y_el_sistema_al_final() {
        let seg = GRUPOS.iter().find(|g| g.id == "seguridad").unwrap().paginas;
        assert_eq!(seg[0].id, "seguridad");
        assert_eq!(seg[0].piezas, ["seguridad/caja"]);
        assert_eq!(seg.last().unwrap().piezas, ["seguridad/sistema"]);
        for p in ["seguridad/chrome", "seguridad/mac", "seguridad/sistema"] {
            let n = seg.iter().filter(|pg| pg.piezas.contains(&p)).count();
            assert_eq!(n, 1, "{p} en una página de Seguridad");
            assert!(seg.iter().find(|pg| pg.piezas.contains(&p)).unwrap().piezas.iter().all(|x| *x != "seguridad/caja"));
        }
        // quien las pinta las registra (panel.js reparte las piezas de app.js)
        for k in ["seguridad/caja", "seguridad/chrome", "seguridad/mac", "seguridad/sistema"] {
            assert!(PANEL_JS.contains(&format!("\"{k}\":")), "{k} sin pintor en panel.js");
        }
    }

    /// Pulido: el buscador encuentra AJUSTES sueltos. Cada fila declarada vive en una página que existe, su id
    /// no se repite y algún pintor la marca (`setFila("id"` o `data-fila="id"`) para poder llevar a ella.
    #[test]
    fn cada_ajuste_buscable_tiene_pagina_y_fila_pintada() {
        let paginas: Vec<&str> = GRUPOS.iter().flat_map(|g| g.paginas.iter().map(|p| p.id)).collect();
        let mut ids: Vec<&str> = FILAS.iter().map(|f| f.id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), FILAS.len(), "ids de fila repetidos");
        for f in FILAS {
            assert!(paginas.contains(&f.pagina), "{}: la página {} no existe", f.id, f.pagina);
            let marcada = PINTORES.iter().any(|js| js.contains(&format!("setFila(\"{}\"", f.id)) || js.contains(&format!("data-fila=\"{}\"", f.id)));
            assert!(marcada, "{}: ningún pintor pinta esa fila", f.id);
        }
        // la copia del simulador (para ver la ventana sin la app) no se desvía de aquí
        const MOCK: &str = include_str!("../ui/mock-ventana.js");
        for f in FILAS {
            let fila = format!("[\"{}\", \"{}\", \"{}\", \"{}\"]", f.pagina, f.id, f.i18n, f.nota);
            assert!(MOCK.contains(&fila), "mock-ventana.js no trae {fila}");
        }
        // y el buscador las usa
        assert!(PANEL_JS.contains("E.filas") && PANEL_JS.contains("data-fila"));
        assert_eq!(ventana_estructura()["filas"].as_array().unwrap().len(), FILAS.len());
    }

    /// Pulido: la interfaz de Configuración no lleva ✓/✗/⚠ ni emojis; los estados van en texto o con un punto
    /// de color (`setEstado`). Se mira el texto de las páginas de Configuración que este hilo repintó.
    #[test]
    fn configuracion_sin_marcas_ni_emojis() {
        let marcas = ['✓', '✗', '✕', '⚠', '◐', '🔒', '🔑', '🙈', '💾', '💲', '🔧', '🧠', '👁', '⬇', '✦', '↗', '⌘', '⌕'];
        let tramo = |src: &'static str, desde: &str, hasta: &str| -> &'static str {
            let i = src.find(desde).unwrap_or_else(|| panic!("no encuentro {desde}"));
            let j = src[i..].find(hasta).map(|j| i + j).unwrap_or(src.len());
            &src[i..j]
        };
        let app = PINTORES[0];
        let tramos = [
            tramo(app, "async function drawConexiones", "// ── Control Mac"),
            tramo(app, "// ── Configuración ──", "// ── Eventos del núcleo"),
            PINTORES[1],
            PINTORES[2],
            DISP_JS,
        ];
        for t in tramos {
            for (n, l) in t.lines().enumerate() {
                // los comentarios del código no son interfaz
                if l.trim_start().starts_with("//") {
                    continue;
                }
                assert!(!l.chars().any(|c| marcas.contains(&c)), "marca en la interfaz de Configuración (línea {n} del tramo): {l}");
            }
        }
    }

    /// El armazón ofrece el registro para que otros hilos se enganchen sin tocarlo.
    #[test]
    fn hay_registro_de_pestanas_y_secciones() {
        assert!(PANEL_JS.contains("window.gotvPanel") && PANEL_JS.contains("registrar("));
        assert!(PANEL_JS.contains("window.gotvAjustes"));
        assert!(PANEL_JS.contains("ventana_estructura"));
    }

    /// ⌘K: las órdenes casan por su nombre, en los dos idiomas y sin tildes; sin consulta, todas.
    #[test]
    fn las_ordenes_casan_por_su_nombre() {
        assert_eq!(ordenes_que_casan("").len(), ORDENES.len());
        assert_eq!(ordenes_que_casan("papelera")[0].id, "papelera");
        assert_eq!(ordenes_que_casan("chat nuevo")[0].id, "nuevo_chat");
        assert_eq!(ordenes_que_casan("configuracion")[0].id, "ajustes");
        assert_eq!(ordenes_que_casan("settings")[0].id, "ajustes");
        assert!(ordenes_que_casan("zzz").is_empty());
        assert!(ORDENES.iter().all(|o| !texto_orden(o.id)[0].is_empty()), "cada orden tiene su nombre");
    }

    #[test]
    fn la_vista_solo_abre_direcciones_de_esta_mac() {
        for (u, ok) in [
            ("http://localhost:5173/", true),
            ("http://127.0.0.1:8080/a?b", true),
            ("https://localhost", true),
            ("http://[::1]:3000", true),
            ("http://0.0.0.0:8000", true),
            ("http://localhost.evil.com/", false),
            ("http://user@localhost:5173", false),
            ("https://example.com", false),
            ("file:///etc/passwd", false),
            ("javascript:alert(1)", false),
        ] {
            assert_eq!(url_local(u).is_some(), ok, "{u}");
        }
        assert_eq!(url_local("http://0.0.0.0:8000/x").unwrap(), "http://127.0.0.1:8000/x");
    }
}
