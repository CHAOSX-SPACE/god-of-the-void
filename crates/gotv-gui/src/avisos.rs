//! F2.1 · Los avisos de la ventana: con GOTV en segundo plano, terminar un turno o pedir un permiso avisa
//! en el Centro de notificaciones; al frente, no. La decisión es la función pura del núcleo
//! (`gotv_core::avisos::debe_avisar`) con los interruptores `[avisos]` de la config.
use gotv_core::avisos::{Evento, Interruptores, Sistema};
use tauri::{AppHandle, Manager};

/// ¿Alguna ventana de GOTV tiene el foco?
pub fn al_frente(app: &AppHandle) -> bool {
    app.webview_windows().values().any(|w| w.is_focused().unwrap_or(false))
}

/// Avisa si toca (en otro hilo: el sistema de avisos no detiene a nadie).
pub fn avisar(app: &AppHandle, proyecto: Option<&std::path::Path>, ev: Evento, cuerpo: &str) {
    let i = Interruptores::cargar(proyecto);
    let frente = al_frente(app);
    if !gotv_core::avisos::debe_avisar(ev, frente, &i) {
        return;
    }
    let cuerpo: String = cuerpo.chars().take(180).collect();
    std::thread::spawn(move || {
        gotv_core::avisos::avisar(&Sistema, ev, frente, &i, "GOD OF THE VOID", &cuerpo);
    });
}
