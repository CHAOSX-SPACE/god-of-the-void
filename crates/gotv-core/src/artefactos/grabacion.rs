//! F7 paso 6 · Grabar la prueba con el navegador propio y verla en el chat.
//!
//! `browser ["record","start","prueba.webm"]` … `["record","stop"]` (motor: agent-browser, que necesita
//! ffmpeg con libvpx/libx264). Trampa 2: el video NO entra en base64 al chat ni al contexto: la ventana
//! recibe una tarjeta `vista` de tipo `video` con su RUTA y lo lee como archivo local (`leer_video`); al
//! modelo sólo le llega la ruta. Las capturas y el video quedan anotados para el recorrido del turno.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

/// Extensiones de video que graba el motor y que la ventana reproduce.
pub const EXT_VIDEO: [&str; 2] = ["webm", "mp4"];

fn activas() -> &'static Mutex<HashMap<String, PathBuf>> {
    static M: OnceLock<Mutex<HashMap<String, PathBuf>>> = OnceLock::new();
    M.get_or_init(Default::default)
}

fn absoluta(root: &Path, p: &str) -> PathBuf {
    let p = Path::new(p);
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        root.join(p)
    }
}

fn es_video(p: &Path) -> bool {
    p.extension().and_then(|e| e.to_str()).is_some_and(|e| EXT_VIDEO.iter().any(|v| e.eq_ignore_ascii_case(v)))
}

/// La primera palabra de la salida que parezca un archivo con alguna de estas extensiones.
fn ruta_en(salida: &str, ext: &[&str]) -> Option<String> {
    salida
        .split_whitespace()
        .map(|w| w.trim_matches(|c: char| matches!(c, '`' | '"' | '\'' | '(' | ')' | ',' | ':' | '«' | '»')))
        .find(|w| w.rsplit_once('.').is_some_and(|(_, e)| ext.iter().any(|x| e.eq_ignore_ascii_case(x))))
        .map(String::from)
}

/// Tras una llamada a `browser` que SALIÓ BIEN: anota lo grabado y lo capturado y, al parar una
/// grabación, devuelve la tarjeta de video para la ventana. Le añade a `salida` lo que el modelo debe
/// saber (sólo rutas).
pub fn tras_browser(dueno: &str, root: &Path, args: &[String], salida: &mut String) -> Option<serde_json::Value> {
    let mut palabras = args.iter().map(String::as_str).filter(|a| !a.starts_with('-'));
    match (palabras.next(), palabras.next()) {
        (Some("record"), Some("start" | "restart")) => {
            let ruta = absoluta(root, palabras.next()?);
            salida.push_str(&crate::tr!(
                "\n(grabando en {}: al terminar la prueba, [\"record\",\"stop\"]; el video se verá en el chat)",
                "\n(recording to {}: when the test is done, [\"record\",\"stop\"]; the video will show in the chat)",
                ruta.display()
            ));
            activas().lock().unwrap_or_else(|e| e.into_inner()).insert(dueno.to_string(), ruta);
            None
        }
        (Some("record"), Some("stop")) => {
            let guardada = activas().lock().unwrap_or_else(|e| e.into_inner()).remove(dueno);
            let ruta = guardada.or_else(|| ruta_en(salida, &EXT_VIDEO).map(|p| absoluta(root, &p))).filter(|p| es_video(p) && p.is_file())?;
            super::recorrido::anotar_medio(dueno, ruta.clone());
            salida.push_str(&crate::tr!(
                "\n(video guardado en {}: el usuario lo ve en el chat. No lo leas ni lo conviertas: basta su ruta)",
                "\n(video saved to {}: the user sees it in the chat. Do not read or convert it: its path is enough)",
                ruta.display()
            ));
            let titulo = ruta.file_name().and_then(|n| n.to_str()).unwrap_or("video").to_string();
            Some(serde_json::json!({"titulo": titulo, "tipo": "video", "ruta": ruta.to_string_lossy(), "origen": "browser"}))
        }
        (Some("screenshot"), _) => {
            if let Some(p) = ruta_en(salida, &["png", "jpg", "jpeg"]) {
                super::recorrido::anotar_medio(dueno, absoluta(root, &p));
            }
            None
        }
        _ => None,
    }
}
