//! Adjuntos de la GUI: imágenes, audio, PDF, Word/RTF/ODT/HTML y texto.
//! - imagen → medio para un modelo con visión (si no ve, se pasa la ruta);
//! - audio → medio si el modelo OYE; si no, Whisper (NO nativo) a texto;
//! - PDF → texto con PDFKit (JXA, viene con macOS); Word & co. → `textutil` (de macOS);
//! - texto → tal cual.
//!
//! Un documento corto viaja dentro de la orden; uno largo se guarda en
//! `.gotv/adjuntos/` y el agente lo lee con `read_file` (no ahoga el contexto).
//!
//! Voz: grabación NATIVA con AVFoundation por JXA (más fiable que el micrófono del
//! webview); para parar, se crea un archivo «stop» que el grabador vigila.

use anyhow::{anyhow, bail, Result};
use serde::Serialize;
use std::path::{Path, PathBuf};

pub const INLINE_MAX: usize = 12_000;

#[derive(Clone, Debug, Serialize)]
pub struct Chip {
    pub id: u64,
    pub name: String,
    /// image · audio · doc · voice
    pub kind: String,
    pub detail: String,
}

/// Lo que un adjunto aporta a la próxima orden.
#[derive(Clone, Debug)]
pub enum Payload {
    /// medio nativo (imagen o audio): (nombre, data URL)
    Media(String, String),
    /// texto que se agrega a la orden
    Text(String),
}

fn ext(p: &Path) -> String {
    p.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default()
}

fn pdf_text(p: &Path) -> Result<String> {
    let script = format!(
        "ObjC.import('PDFKit'); var d=$.PDFDocument.alloc.initWithURL($.NSURL.fileURLWithPath({})); d ? ObjC.unwrap(d.string) : ''",
        serde_json::to_string(&p.to_string_lossy())?
    );
    let o = std::process::Command::new("osascript").args(["-l", "JavaScript", "-e", &script]).output()?;
    let t = String::from_utf8_lossy(&o.stdout).trim().to_string();
    if t.is_empty() {
        bail!("{}", gotv_core::i18n::t("el PDF no tiene texto extraíble (¿es una imagen escaneada?)", "the PDF has no extractable text (is it a scanned image?)"));
    }
    Ok(t)
}

fn textutil(p: &Path) -> Result<String> {
    let o = std::process::Command::new("textutil").args(["-convert", "txt", "-stdout"]).arg(p).output()?;
    if !o.status.success() {
        bail!("textutil: {}", String::from_utf8_lossy(&o.stderr).trim());
    }
    Ok(String::from_utf8_lossy(&o.stdout).to_string())
}

/// Documento → texto para la orden (dentro si es corto; si no, a `.gotv/adjuntos/`).
fn doc_payload(root: &Path, name: &str, text: String, origin: &str) -> Result<(Payload, String)> {
    let n = text.chars().count();
    if n <= INLINE_MAX {
        let body = gotv_core::tr!(
            "[adjunto: {name} — {origin}]\n```\n{text}\n```",
            "[attachment: {name} — {origin}]\n```\n{text}\n```"
        );
        return Ok((Payload::Text(body), gotv_core::tr!("{n} caracteres", "{n} characters")));
    }
    let dir = root.join(".gotv/adjuntos");
    std::fs::create_dir_all(&dir)?;
    let safe: String = name.chars().map(|c| if c.is_alphanumeric() || ".-_".contains(c) { c } else { '_' }).collect();
    let dest = dir.join(format!("{safe}.txt"));
    std::fs::write(&dest, &text)?;
    let rel = dest.strip_prefix(root).unwrap_or(&dest).display().to_string();
    let body = gotv_core::tr!(
        "[adjunto: {name} — {origin}, {n} caracteres: demasiado largo para la orden; está en `{rel}`, léelo con read_file por partes]",
        "[attachment: {name} — {origin}, {n} characters: too long for the order; it is at `{rel}`, read it with read_file in parts]"
    );
    Ok((Payload::Text(body), gotv_core::tr!("{n} caracteres → {rel}", "{n} characters → {rel}")))
}

/// Procesa un archivo. `vision` / `hears`: lo que el motor actual puede.
pub fn process(root: &Path, p: &Path, vision: bool, hears: bool) -> Result<(Payload, String, String)> {
    if !p.is_file() {
        bail!("{}", gotv_core::tr!("no existe {}", "{} does not exist", p.display()));
    }
    let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let e = ext(p);
    if gotv_core::images::is_image(p) {
        if vision {
            let (url, n) = gotv_core::images::data_url(p)?;
            return Ok((Payload::Media(name, url), "image".into(), format!("{} KB", n / 1024)));
        }
        let note = gotv_core::tr!(
            "[imagen adjunta: {} — el modelo actual NO ve imágenes; elige uno con visión para que la mire]",
            "[attached image: {} — the current model CANNOT see images; pick a vision model so it can look at it]",
            p.display()
        );
        return Ok((Payload::Text(note), "image".into(), gotv_core::i18n::t("sin visión", "no vision").into()));
    }
    if gotv_core::audio::is_audio(p) {
        if hears {
            let (url, secs) = gotv_core::audio::data_url(p)?;
            return Ok((Payload::Media(name, url), "audio".into(), format!("{secs:.1} s")));
        }
        let t = gotv_core::audio::transcribe(p)?;
        let body = gotv_core::tr!("[audio {name} transcrito por Whisper — NO nativo]\n{t}", "[audio {name} transcribed by Whisper — NOT native]\n{t}");
        return Ok((Payload::Text(body), "audio".into(), "Whisper".into()));
    }
    let (text, origin) = match e.as_str() {
        "pdf" => (pdf_text(p)?, "PDF".to_string()),
        "doc" | "docx" | "rtf" | "rtfd" | "odt" | "html" | "htm" | "webarchive" | "wordml" => (textutil(p)?, e.to_uppercase()),
        _ => {
            let bytes = std::fs::read(p)?;
            if bytes.len() > 4 * 1024 * 1024 {
                bail!("{}", gotv_core::i18n::t("archivo de más de 4 MB: muévelo al proyecto y pídele al agente que lo lea", "file over 4 MB: move it into the project and ask the agent to read it"));
            }
            let t = String::from_utf8(bytes).map_err(|_| anyhow!("{}", gotv_core::i18n::t("formato no soportado (binario)", "unsupported format (binary)")))?;
            (t, gotv_core::i18n::t("texto", "text").to_string())
        }
    };
    let (payload, detail) = doc_payload(root, &name, text, &origin)?;
    Ok((payload, "doc".into(), detail))
}

// ── Voz ──────────────────────────────────────────────────────────────────────

pub struct Recording {
    pub out: PathBuf,
    stop: PathBuf,
    child: std::process::Child,
}

/// Empieza a grabar (AAC 16 kHz mono, máx. 120 s). macOS pide permiso de micrófono a
/// «GOD OF THE VOID» la primera vez (por eso el Info.plist trae NSMicrophoneUsageDescription).
pub fn record_start() -> Result<Recording> {
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let dir = std::env::temp_dir();
    let out = dir.join(format!("gotv-voz-{stamp}.m4a"));
    let stop = dir.join(format!("gotv-voz-{stamp}.stop"));
    let script = format!(
        r#"ObjC.import('AVFoundation'); ObjC.import('Foundation');
var url = $.NSURL.fileURLWithPath({out});
var s = $.NSMutableDictionary.alloc.init;
s.setObjectForKey($(1633772320), $('AVFormatIDKey'));
s.setObjectForKey($(16000), $('AVSampleRateKey'));
s.setObjectForKey($(1), $('AVNumberOfChannelsKey'));
var r = $.AVAudioRecorder.alloc.initWithURLSettingsError(url, s, null);
r.record;
var fm = $.NSFileManager.defaultManager; var t = 0;
while (!fm.fileExistsAtPath({stop}) && t < 1200) {{ delay(0.1); t++; }}
r.stop; 'ok'"#,
        out = serde_json::to_string(&out.to_string_lossy())?,
        stop = serde_json::to_string(&stop.to_string_lossy())?
    );
    let child = std::process::Command::new("osascript").args(["-l", "JavaScript", "-e", &script]).spawn()?;
    Ok(Recording { out, stop, child })
}

impl Recording {
    /// Para y devuelve el archivo grabado.
    pub fn finish(mut self) -> Result<PathBuf> {
        std::fs::write(&self.stop, b"")?;
        for _ in 0..50 {
            if let Ok(Some(_)) = self.child.try_wait() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        let _ = self.child.kill();
        let _ = std::fs::remove_file(&self.stop);
        let ok = std::fs::metadata(&self.out).map(|m| m.len() > 1000).unwrap_or(false);
        if !ok {
            bail!("{}", gotv_core::i18n::t(
                "no se grabó audio (¿permiso de micrófono para GOD OF THE VOID en Configuración › Privacidad?)",
                "no audio was recorded (microphone permission for GOD OF THE VOID in Settings › Privacy?)"
            ));
        }
        Ok(self.out.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn documentos_cortos_dentro_y_largos_a_archivo() {
        let d = gotv_engine::prueba::CarpetaPrueba::sin_crear("gotv-adj");
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("nota.md"), "# Hola\nuna nota").unwrap();
        let (p, kind, _) = process(&d, &d.join("nota.md"), false, false).unwrap();
        assert_eq!(kind, "doc");
        assert!(matches!(p, Payload::Text(t) if t.contains("una nota")));
        std::fs::write(d.join("largo.txt"), "x".repeat(INLINE_MAX + 10)).unwrap();
        let (p, _, detail) = process(&d, &d.join("largo.txt"), false, false).unwrap();
        assert!(matches!(p, Payload::Text(t) if t.contains(".gotv/adjuntos/largo.txt.txt")), "{detail}");
        assert!(d.join(".gotv/adjuntos/largo.txt.txt").exists());
        // RTF real por textutil (de macOS)
        std::fs::write(d.join("carta.rtf"), r"{\rtf1\ansi Estimado Portador\par}").unwrap();
        let (p, _, _) = process(&d, &d.join("carta.rtf"), false, false).unwrap();
        assert!(matches!(p, Payload::Text(t) if t.contains("Estimado Portador")));
        // binario: se niega con razón
        std::fs::write(d.join("x.bin"), [0u8, 159, 146, 150]).unwrap();
        assert!(process(&d, &d.join("x.bin"), false, false).is_err());
    }

    #[test]
    fn pdf_por_pdfkit() {
        // un PDF REAL hecho con `cupsfilter` (viene con macOS), con un texto conocido
        let d = gotv_engine::prueba::CarpetaPrueba::sin_crear("gotv-pdf");
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("src.txt"), "Factura del Vacio numero 7341\n").unwrap();
        let Ok(o) = std::process::Command::new("/usr/sbin/cupsfilter").arg(d.join("src.txt")).output() else { return };
        if o.stdout.len() < 100 {
            return; // sin cupsfilter: no aplica
        }
        std::fs::write(d.join("factura.pdf"), &o.stdout).unwrap();
        let (p, kind, _) = process(&d, &d.join("factura.pdf"), false, false).unwrap();
        assert_eq!(kind, "doc");
        assert!(matches!(p, Payload::Text(t) if t.contains("7341") && t.contains("PDF")));
    }
}
