//! F10.5 en la app: el chat abierto se ve desde el teléfono (rutas `/gotv/` del servicio de API), sus permisos
//! se contestan donde llegue primero, lo que manda el teléfono es la próxima orden, y Configuración → API
//! empareja (código de un solo uso) y revoca teléfonos. Nada de esto escucha fuera de esta Mac si el Portador
//! no abrió el servicio a la red.
use crate::Hub;
use gotv_core::remoto::EstadoSesion;
use serde_json::{json, Value};
use tauri::State;

/// El id del chat abierto en el registro remoto: su proyecto (la app tiene un chat a la vez).
pub fn sesion_id(root: &std::path::Path) -> String {
    root.display().to_string()
}

/// El chat del proyecto pasa a verse (o cambia de título).
pub fn abrir(hub: &Hub, root: &std::path::Path) {
    let titulo = root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| root.display().to_string());
    hub.api.remoto.abrir(&sesion_id(root), root, &titulo);
}

pub fn estado(hub: &Hub, root: &std::path::Path, e: EstadoSesion) {
    hub.api.remoto.estado(&sesion_id(root), e);
}

/// El chat se cierra: deja de verse (un permiso pendiente del teléfono queda negado).
pub fn cerrar(hub: &Hub, root: &std::path::Path) {
    hub.api.remoto.cerrar(&sesion_id(root));
}

/// Un código de un solo uso para emparejar un teléfono, y las direcciones de la página.
#[tauri::command]
pub async fn remoto_codigo(h: State<'_, Hub>) -> Result<Value, String> {
    let run = h.api_run.lock().await;
    let Some(r) = run.as_ref() else {
        return Err(gotv_core::i18n::t("enciende el servicio de API (y ábrelo a la red de casa) para usar el teléfono", "turn on the API service (and open it to the home network) to use the phone").into());
    };
    let lan = !gotv_core::serve::is_loopback(&r.host);
    let urls: Vec<String> = if lan { gotv_core::serve::lan_addresses().into_iter().map(|ip| format!("http://{ip}:{}/gotv/movil", r.port)).collect() } else { vec![format!("http://127.0.0.1:{}/gotv/movil", r.port)] };
    let codigo = h.api.remoto.codigo_nuevo();
    // el QR lleva la dirección con el código en el fragmento: la cámara del teléfono abre la página y empareja
    let qr = urls.first().and_then(|u| gotv_core::remoto::qr_svg(&gotv_core::remoto::url_con_codigo(u, &codigo)));
    Ok(json!({"codigo": codigo, "urls": urls, "lan": lan, "qr": qr, "minutos": gotv_core::remoto::CODIGO_VIGENCIA.as_secs() / 60}))
}

/// Los teléfonos emparejados.
#[tauri::command]
pub async fn remoto_tokens(h: State<'_, Hub>) -> Result<Value, String> {
    Ok(json!(h.api.remoto.tokens().into_iter().map(|t| json!({"id": t.id, "nombre": t.nombre, "creado": t.creado})).collect::<Vec<_>>()))
}

/// Revoca un teléfono: sin acceso al momento.
#[tauri::command]
pub async fn remoto_revocar(h: State<'_, Hub>, id: String) -> Result<Value, String> {
    if !h.api.remoto.revocar(&id) {
        return Err(gotv_core::i18n::t("ese teléfono ya no estaba emparejado", "that phone was no longer paired").into());
    }
    remoto_tokens(h).await
}
