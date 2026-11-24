//! Imágenes para la visión nativa (S1): cualquier formato que macOS lea (png, jpg,
//! heic, webp, gif, tiff…) se reduce con `sips` (viene con macOS) a ≤1024 px en JPEG
//! y viaja como `data:` URL. Así una foto de 12 MP no se come el contexto.

use anyhow::{anyhow, bail, Result};
use std::path::Path;

pub const MAX_SIDE: u32 = 1024;
const EXTS: &[&str] = &[
    "png", "jpg", "jpeg", "heic", "heif", "webp", "gif", "tif", "tiff", "bmp",
];

pub fn is_image(p: &Path) -> bool {
    p.extension()
        .map(|e| EXTS.contains(&e.to_string_lossy().to_lowercase().as_str()))
        .unwrap_or(false)
}

/// La imagen lista para el modelo: (`data:image/jpeg;base64,…`, bytes enviados).
pub fn data_url(p: &Path) -> Result<(String, usize)> {
    if !p.is_file() {
        bail!(
            "{}",
            crate::tr!(
                "no existe la imagen {}",
                "image {} does not exist",
                p.display()
            )
        );
    }
    if !is_image(p) {
        bail!(
            "{}",
            crate::tr!(
                "{} no parece una imagen",
                "{} does not look like an image",
                p.display()
            )
        );
    }
    // medido 2026-11-16 (banco `mcp-imagen`): una imagen ROTA de un servidor MCP (su PNG venía corrupto)
    // salió de `sips` como un JPEG sin datos; el motor no pudo leerlo y el turno entero se cayó. Lo
    // convertido se comprueba: si no se lee, va la original si ésa sí se lee; si ninguna, un error claro
    let original = std::fs::read(p)?;
    let tmp = std::env::temp_dir().join(format!(
        "gotv-img-{}-{}.jpg",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    // `sips -Z` también AMPLÍA (medido 2026-11-10: un icono de 512 salía de 1024×1024 → 4× tokens
    // de imagen sin un detalle más): sólo se reduce si el lado mayor pasa de MAX_SIDE
    let mut args: Vec<String> = Vec::new();
    if lado_mayor(p).is_none_or(|l| l > MAX_SIDE) {
        args.extend(["-Z".to_string(), MAX_SIDE.to_string()]);
    }
    args.extend(["-s", "format", "jpeg", "-s", "formatOptions", "80"].map(String::from));
    let out = std::process::Command::new("sips")
        .args(&args)
        .arg(p)
        .arg("--out")
        .arg(&tmp)
        .output()
        .map_err(|e| anyhow!("sips: {e}"))?;
    if !out.status.success() || !tmp.exists() {
        bail!(
            "{}",
            crate::tr!(
                "no pude convertir {} (sips)",
                "could not convert {} (sips)",
                p.display()
            )
        );
    }
    let bytes = std::fs::read(&tmp)?;
    let _ = std::fs::remove_file(&tmp);
    if mime_legible(&bytes).is_none() {
        // la conversión salió rota: la original, si el modelo la puede leer
        if let Some(mime) = mime_legible(&original) {
            return Ok((format!("data:{mime};base64,{}", base64(&original)), original.len()));
        }
        bail!("{}", crate::tr!("no pude preparar la imagen {} (la conversión salió ilegible)", "could not prepare the image {} (the conversion came out unreadable)", p.display()));
    }
    Ok((
        format!("data:image/jpeg;base64,{}", base64(&bytes)),
        bytes.len(),
    ))
}

/// El tipo de una imagen PNG, JPEG o WebP que se puede DECODIFICAR de verdad (`None` si no es de esas o
/// está rota): lo que el motor de visión lee.
fn mime_legible(bytes: &[u8]) -> Option<&'static str> {
    let mime = match image::guess_format(bytes).ok()? {
        image::ImageFormat::Png => "image/png",
        image::ImageFormat::Jpeg => "image/jpeg",
        image::ImageFormat::WebP => "image/webp",
        _ => return None,
    };
    image::load_from_memory(bytes).ok().map(|_| mime)
}

/// El lado mayor de una imagen en píxeles, según `sips -g` (None si no se pudo leer).
fn lado_mayor(p: &Path) -> Option<u32> {
    let out = std::process::Command::new("sips").args(["-g", "pixelWidth", "-g", "pixelHeight"]).arg(p).output().ok()?;
    let t = String::from_utf8_lossy(&out.stdout);
    let medida = |clave: &str| t.lines().find_map(|l| l.trim().strip_prefix(clave).and_then(|v| v.trim().parse::<u32>().ok()));
    Some(medida("pixelWidth:")?.max(medida("pixelHeight:")?))
}

/// Base64 estándar (sin dependencias).
pub fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for c in data.chunks(3) {
        let n = (c[0] as u32) << 16
            | (*c.get(1).unwrap_or(&0) as u32) << 8
            | *c.get(2).unwrap_or(&0) as u32;
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if c.len() > 1 {
            T[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if c.len() > 2 {
            T[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Banco `mcp-imagen` (2026-11-16): lo que sale de `data_url` se puede leer; una imagen rota no se
    /// manda como si fuera buena (error claro), y una cabecera sin datos no pasa por imagen.
    #[test]
    fn lo_que_va_al_modelo_se_puede_leer() {
        use base64::Engine as _;
        let png = base64::engine::general_purpose::STANDARD.decode("iVBORw0KGgoAAAANSUhEUgAAAEAAAABACAIAAAAlC+aJAAAACXBIWXMAAAABAAAAAQBPJcTWAAAAeklEQVR4nO3YQQqAMAwAQQv+/8v15r2IjNKdF2TJKRlzzuPPTj3AUwVoBWgFaAVoBWgFaAVoywFzjDfmuI3F+2S/DXxNAVoBWgFaAVoBWgFaAVoBWgFaAVoBWgFaAVoB2n4Bq//7t+23ga8pQCtAK0ArQCtAK0D7fcAFRE8I+kk7AoEAAAAASUVORK5CYII=").unwrap();
        let d = crate::CarpetaPrueba::nueva("gotv-img-chica");
        let p = d.join("cuadro.png");
        std::fs::write(&p, &png).unwrap();
        let (url, _) = data_url(&p).unwrap();
        let b = base64::engine::general_purpose::STANDARD.decode(url.split_once(',').unwrap().1).unwrap();
        assert!(mime_legible(&b).is_some(), "{}", &url[..40]);
        // un PNG con la cabecera bien y el cuerpo roto: ni la conversión ni la original se leen → error
        let roto = d.join("roto.png");
        let mut malo = png.clone();
        let n = malo.len();
        malo[40..n - 12].iter_mut().for_each(|x| *x = 0x5a);
        std::fs::write(&roto, &malo).unwrap();
        assert!(data_url(&roto).is_err(), "una imagen rota no va al modelo como si fuera buena");
        assert!(mime_legible(b"\xff\xd8\xff\xe0 no es un jpeg").is_none(), "una cabecera sin datos no pasa");
    }

    #[test]
    fn base64_estandar() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"Vac\xc3\xado"), "VmFjw61v");
    }

    #[test]
    fn convierte_con_sips_y_acota_el_lado() {
        // un PNG real de 2000×100 hecho con sips desde la nada no es trivial: usamos el
        // ícono del sistema que trae macOS
        let src = Path::new("/System/Library/CoreServices/CoreTypes.bundle/Contents/Resources/GenericDocumentIcon.icns");
        if !src.exists() {
            return; // otra versión de macOS: la prueba no aplica
        }
        let png = crate::CarpetaPrueba::archivo("gotv-icon", "png");
        std::process::Command::new("sips")
            .args(["-s", "format", "png"])
            .arg(src)
            .arg("--out")
            .arg(&png)
            .output()
            .unwrap();
        let (url, n) = data_url(&png).unwrap();
        assert!(url.starts_with("data:image/jpeg;base64,/9j/"), "JPEG real");
        assert!(n > 0 && n < 400_000);
        assert!(data_url(Path::new("/no/existe.png")).is_err());
        assert!(data_url(Path::new("Cargo.toml")).is_err());
    }

    /// Una imagen pequeña NO se amplía (sips -Z amplía: 512 → 1024 cuadruplicaba los tokens).
    #[test]
    fn no_amplia_las_imagenes_pequenas() {
        let src = Path::new("/System/Library/CoreServices/CoreTypes.bundle/Contents/Resources/GenericDocumentIcon.icns");
        if !src.exists() {
            return;
        }
        let d = crate::CarpetaPrueba::sin_crear("gotv-pequena");
        std::fs::create_dir_all(&d).unwrap();
        let png = d.join("p.png");
        std::process::Command::new("sips").args(["-s", "format", "png", "-z", "300", "200"]).arg(src).arg("--out").arg(&png).output().unwrap();
        assert_eq!(lado_mayor(&png), Some(300));
        let (url, _) = data_url(&png).unwrap();
        let jpg = d.join("salida.jpg");
        std::fs::write(&jpg, decodifica(&url)).unwrap();
        assert_eq!(lado_mayor(&jpg), Some(300), "se queda en 300, no sube a {MAX_SIDE}");
        let _ = std::fs::remove_dir_all(&d);
    }

    fn decodifica(url: &str) -> Vec<u8> {
        let b = url.split_once(',').unwrap().1.as_bytes();
        let val = |c: u8| match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            _ => 63,
        } as u32;
        let mut out = Vec::new();
        for c in b.chunks(4) {
            let n = c.iter().enumerate().fold(0u32, |n, (i, &x)| if x == b'=' { n } else { n | val(x) << (18 - 6 * i) });
            out.push((n >> 16) as u8);
            if c.get(2).is_some_and(|&x| x != b'=') {
                out.push((n >> 8) as u8);
            }
            if c.get(3).is_some_and(|&x| x != b'=') {
                out.push(n as u8);
            }
        }
        out
    }
}
