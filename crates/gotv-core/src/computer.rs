//! Control del PC (G1) — DENEGADO por defecto: las herramientas ni existen sin
//! `computer_use = true`, y aun así cada acción pide permiso (también en bypass) salvo un
//! `allow` explícito; en modo plan, nunca.
//!
//! Sin dependencias nuevas, con lo que trae macOS:
//! - mirar: `screencapture -x` → `sips` a ≤1280 px → imagen para un modelo con visión (S1);
//! - ratón: CoreGraphics por JXA (`osascript -l JavaScript`), coordinadas en PUNTOS;
//! - teclado: System Events (`keystroke` / `key code`).
//!
//! Medido en esta Mac: la captura sale en píxeles Retina (3456×2234) y la pantalla mide
//! 1728×1117 puntos; el modelo ve la imagen reducida, así que GOTV convierte SUS
//! coordenadas (de la última captura) a puntos de pantalla. Necesita los permisos de
//! macOS «Grabación de pantalla» (mirar) y «Accesibilidad» (ratón y teclado).

use anyhow::{anyhow, bail, Result};
use std::sync::Mutex;

pub const MAX_SIDE: u32 = 1280;

/// Relación de la última captura: (ancho, alto) de la imagen que vio el modelo y
/// (ancho, alto) de la pantalla en puntos.
/// (ancho, alto)
type Size = (f64, f64);
static LAST: Mutex<Option<(Size, Size)>> = Mutex::new(None);

fn jxa(script: &str) -> Result<String> {
    let o = std::process::Command::new("osascript")
        .args(["-l", "JavaScript", "-e", script])
        .output()?;
    if !o.status.success() {
        bail!("osascript: {}", String::from_utf8_lossy(&o.stderr).trim());
    }
    Ok(String::from_utf8_lossy(&o.stdout).trim().to_string())
}

/// Tamaño de la pantalla principal en puntos.
pub fn screen_points() -> Result<(f64, f64)> {
    let s = jxa("ObjC.import('CoreGraphics'); var b=$.CGDisplayBounds($.CGMainDisplayID()); b.size.width+' '+b.size.height")?;
    let mut it = s.split_whitespace().filter_map(|x| x.parse::<f64>().ok());
    Ok((
        it.next().ok_or_else(|| anyhow!("ancho"))?,
        it.next().ok_or_else(|| anyhow!("alto"))?,
    ))
}

/// Captura de pantalla lista para el modelo: (`data:` URL, ancho, alto de la imagen).
pub fn screenshot() -> Result<(String, u32, u32)> {
    let dir = std::env::temp_dir();
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let (png, jpg) = (
        dir.join(format!("gotv-pant-{stamp}.png")),
        dir.join(format!("gotv-pant-{stamp}.jpg")),
    );
    let ok = std::process::Command::new("screencapture")
        .args(["-x", "-t", "png"])
        .arg(&png)
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if !ok || !png.exists() {
        bail!("{}", crate::i18n::t(
            "no pude capturar la pantalla (¿permiso de «Grabación de pantalla» para la terminal?)",
            "could not capture the screen (is «Screen Recording» allowed for the terminal?)"
        ));
    }
    let _ = std::process::Command::new("sips")
        .args([
            "-Z",
            &MAX_SIDE.to_string(),
            "-s",
            "format",
            "jpeg",
            "-s",
            "formatOptions",
            "75",
        ])
        .arg(&png)
        .arg("--out")
        .arg(&jpg)
        .output();
    let _ = std::fs::remove_file(&png); // la pantalla del usuario no se queda en disco
    let dims = std::process::Command::new("sips")
        .args(["-g", "pixelWidth", "-g", "pixelHeight"])
        .arg(&jpg)
        .output()?;
    let txt = String::from_utf8_lossy(&dims.stdout);
    let num = |k: &str| {
        txt.lines()
            .find(|l| l.contains(k))
            .and_then(|l| l.split(':').nth(1))
            .and_then(|v| v.trim().parse::<u32>().ok())
    };
    let (w, h) = (
        num("pixelWidth").unwrap_or(0),
        num("pixelHeight").unwrap_or(0),
    );
    let bytes = std::fs::read(&jpg)?;
    let _ = std::fs::remove_file(&jpg);
    if w == 0 || h == 0 {
        bail!("sips");
    }
    let pts = screen_points()?;
    *LAST.lock().unwrap() = Some(((w as f64, h as f64), pts));
    Ok((
        format!("data:image/jpeg;base64,{}", crate::images::base64(&bytes)),
        w,
        h,
    ))
}

/// Coordenadas de la ÚLTIMA captura (píxeles de la imagen) → puntos de pantalla.
pub fn to_points(x: f64, y: f64) -> Result<(f64, f64)> {
    let Some(((iw, ih), (pw, ph))) = *LAST.lock().unwrap() else {
        bail!(
            "{}",
            crate::i18n::t(
                "primero mira la pantalla con `screen`: las coordenadas son de esa imagen",
                "look at the screen with `screen` first: coordinates are from that image"
            )
        );
    };
    map((x, y), (iw, ih), (pw, ph))
}

pub fn map((x, y): (f64, f64), (iw, ih): (f64, f64), (pw, ph): (f64, f64)) -> Result<(f64, f64)> {
    if !(0.0..=iw).contains(&x) || !(0.0..=ih).contains(&y) {
        bail!(
            "{}",
            crate::tr!(
                "({x}, {y}) está fuera de la imagen ({iw}×{ih})",
                "({x}, {y}) is outside the image ({iw}×{ih})"
            )
        );
    }
    Ok(((x * pw / iw).round(), (y * ph / ih).round()))
}

/// Script JXA de una acción de ratón en PUNTOS.
pub fn mouse_script(action: &str, x: f64, y: f64, amount: i64) -> Result<String> {
    let base = "ObjC.import('CoreGraphics'); function ev(t,b,n){var e=$.CGEventCreateMouseEvent(null,t,{x:__X__,y:__Y__},b);if(n){$.CGEventSetIntegerValueField(e,$.kCGMouseEventClickState,n);}$.CGEventPost($.kCGHIDEventTap,e);}";
    let body = match action {
        "move" => "ev($.kCGEventMouseMoved,$.kCGMouseButtonLeft);",
        "click" => "ev($.kCGEventMouseMoved,0);ev($.kCGEventLeftMouseDown,$.kCGMouseButtonLeft);ev($.kCGEventLeftMouseUp,$.kCGMouseButtonLeft);",
        "double_click" => "ev($.kCGEventMouseMoved,0);ev($.kCGEventLeftMouseDown,$.kCGMouseButtonLeft,1);ev($.kCGEventLeftMouseUp,$.kCGMouseButtonLeft,1);ev($.kCGEventLeftMouseDown,$.kCGMouseButtonLeft,2);ev($.kCGEventLeftMouseUp,$.kCGMouseButtonLeft,2);",
        "right_click" => "ev($.kCGEventMouseMoved,0);ev($.kCGEventRightMouseDown,$.kCGMouseButtonRight);ev($.kCGEventRightMouseUp,$.kCGMouseButtonRight);",
        "scroll" => "ev($.kCGEventMouseMoved,0);$.CGEventPost($.kCGHIDEventTap,$.CGEventCreateScrollWheelEvent(null,0,1,AMOUNT));",
        other => bail!("{}", crate::tr!("acción de ratón desconocida: {other} (move | click | double_click | right_click | scroll)", "unknown mouse action: {other} (move | click | double_click | right_click | scroll)")),
    };
    Ok(format!("{base}{body}")
        .replace("__X__", &format!("{x}"))
        .replace("__Y__", &format!("{y}"))
        .replace("AMOUNT", &(-amount).to_string()))
}

/// Teclas con nombre → `key code` de macOS.
fn key_code(name: &str) -> Option<u32> {
    Some(match name {
        "return" | "enter" => 36,
        "tab" => 48,
        "space" => 49,
        "delete" | "backspace" => 51,
        "escape" | "esc" => 53,
        "left" => 123,
        "right" => 124,
        "down" => 125,
        "up" => 126,
        "home" => 115,
        "end" => 119,
        "pageup" => 116,
        "pagedown" => 121,
        _ => return None,
    })
}

/// AppleScript de teclado: `text` se escribe tal cual; `keys` es un atajo como
/// «cmd+shift+s», «return» o «ctrl+c».
pub fn keyboard_script(text: Option<&str>, keys: Option<&str>) -> Result<String> {
    if let Some(t) = text.filter(|t| !t.is_empty()) {
        let esc = t.replace('\\', "\\\\").replace('"', "\\\"");
        return Ok(format!(
            "tell application \"System Events\" to keystroke \"{esc}\""
        ));
    }
    let Some(k) = keys.filter(|k| !k.trim().is_empty()) else {
        bail!(
            "{}",
            crate::i18n::t(
                "keyboard necesita `text` o `keys`",
                "keyboard needs `text` or `keys`"
            )
        );
    };
    let parts: Vec<String> = k
        .to_lowercase()
        .split('+')
        .map(|p| p.trim().to_string())
        .collect();
    let (mods, key) = parts.split_at(parts.len() - 1);
    let mut using = Vec::new();
    for m in mods {
        using.push(match m.as_str() {
            "cmd" | "command" => "command down",
            "shift" => "shift down",
            "alt" | "option" | "opt" => "option down",
            "ctrl" | "control" => "control down",
            other => bail!(
                "{}",
                crate::tr!(
                    "modificador desconocido: {other}",
                    "unknown modifier: {other}"
                )
            ),
        });
    }
    let using = if using.is_empty() {
        String::new()
    } else {
        format!(" using {{{}}}", using.join(", "))
    };
    let key = &key[0];
    Ok(match key_code(key) {
        Some(code) => format!("tell application \"System Events\" to key code {code}{using}"),
        None if key.chars().count() == 1 => format!(
            "tell application \"System Events\" to keystroke \"{}\"{using}",
            key.replace('"', "\\\"")
        ),
        None => bail!(
            "{}",
            crate::tr!("tecla desconocida: {key}", "unknown key: {key}")
        ),
    })
}

pub fn run_mouse(action: &str, x: f64, y: f64, amount: i64) -> Result<String> {
    let (px, py) = to_points(x, y)?;
    jxa(&mouse_script(action, px, py, amount)?)?;
    Ok(crate::tr!(
        "ratón: {action} en ({x}, {y}) de la captura = ({px}, {py}) puntos",
        "mouse: {action} at ({x}, {y}) of the screenshot = ({px}, {py}) points"
    ))
}

pub fn run_keyboard(text: Option<&str>, keys: Option<&str>) -> Result<String> {
    let s = keyboard_script(text, keys)?;
    let o = std::process::Command::new("osascript")
        .args(["-e", &s])
        .output()?;
    if !o.status.success() {
        bail!(
            "{}",
            crate::tr!(
                "teclado: {} (¿permiso de «Accesibilidad» para la terminal?)",
                "keyboard: {} (is «Accessibility» allowed for the terminal?)",
                String::from_utf8_lossy(&o.stderr).trim()
            )
        );
    }
    Ok(crate::i18n::t("teclado: hecho", "keyboard: done").into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coordenadas_de_la_imagen_a_puntos() {
        // lo medido: imagen 1280×827 de una pantalla de 1728×1117 puntos
        let (x, y) = map((640.0, 413.5), (1280.0, 827.0), (1728.0, 1117.0)).unwrap();
        assert_eq!((x, y), (864.0, 559.0));
        assert!(
            map((1300.0, 10.0), (1280.0, 827.0), (1728.0, 1117.0)).is_err(),
            "fuera de la imagen"
        );
    }

    #[test]
    fn atajos_y_texto_seguros() {
        assert_eq!(keyboard_script(None, Some("cmd+shift+s")).unwrap(), "tell application \"System Events\" to keystroke \"s\" using {command down, shift down}");
        assert_eq!(
            keyboard_script(None, Some("return")).unwrap(),
            "tell application \"System Events\" to key code 36"
        );
        assert_eq!(
            keyboard_script(Some("dice \"hola\""), None).unwrap(),
            "tell application \"System Events\" to keystroke \"dice \\\"hola\\\"\""
        );
        assert!(keyboard_script(None, Some("hyper+x")).is_err());
        assert!(mouse_script("arrastrar", 1.0, 1.0, 0).is_err());
        let d = mouse_script("double_click", 10.0, 20.0, 0).unwrap();
        assert!(d.contains("{x:10,y:20}") && !d.contains("__X__") && d.contains("ClickState"));
    }

    /// Lo que se puede probar sin tocar la sesión del usuario: los scripts COMPILAN
    /// (`osacompile`) y mover el ratón a donde YA está no cambia nada.
    #[test]
    fn scripts_reales_sin_molestar() {
        if std::process::Command::new("osacompile")
            .arg("-h")
            .output()
            .is_err()
        {
            return;
        }
        for s in [
            keyboard_script(None, Some("cmd+s")).unwrap(),
            keyboard_script(Some("x"), None).unwrap(),
        ] {
            let o = std::process::Command::new("osacompile")
                .args(["-o", "/dev/null", "-e", &s])
                .output()
                .unwrap();
            assert!(
                o.status.success(),
                "{s}: {}",
                String::from_utf8_lossy(&o.stderr)
            );
        }
        // el usuario puede estar moviendo el ratón mientras corre la prueba (medido
        // 2026-11-09: 779,382 → 713,698 entre dos lecturas): basta con que UNA de
        // varias veces quede donde estaba; falla sólo si nunca
        let pos = || {
            jxa("ObjC.import('CoreGraphics'); var p=$.CGEventGetLocation($.CGEventCreate(null)); p.x+' '+p.y")
        };
        let mut seen = vec![];
        for _ in 0..5 {
            let Ok(here) = pos() else { return };
            let v: Vec<f64> = here
                .split_whitespace()
                .filter_map(|x| x.parse().ok())
                .collect();
            jxa(&mouse_script("move", v[0], v[1], 0).unwrap()).unwrap();
            let after = pos().unwrap();
            if here == after {
                return;
            }
            seen.push((here, after));
        }
        panic!("el ratón nunca quedó donde estaba: {seen:?}");
    }
}
