//! Audio (S2). NATIVO si el modelo trae oído (`audio_config`, p. ej. gemma-4-e4b): el audio
//! viaja en el mensaje y el modelo lo OYE (en el núcleo desde M1b: «siete, tres, cuatro, uno» →
//! «7341»). Si no, respaldo: Whisper transcribe —en Rust desde M1 (`registrar_transcriptor`); sin
//! Python desde 2026-11-12— y el texto se marca como transcripción — decisión #4 del Portador: lo
//! no nativo PARA EL MODELO se declara.
//!
//! Cualquier audio que macOS lea (wav, m4a, mp3, aiff, caf…) se pasa con `afconvert`
//! a WAV 16 kHz mono. Tope de duración para no ahogar el contexto.

use anyhow::{anyhow, bail, Result};
use std::path::{Path, PathBuf};

pub const MAX_SECONDS: f64 = 120.0;
/// Whisper de respaldo (se baja con `gotv setup audio`).
/// Medido: mlx-audio exige los archivos de PROCESADOR (tokenizador, preprocesador);
/// los repos viejos `whisper-*` (sólo config + pesos) fallan con «Processor not found».
pub const STT_REPO: &str = "mlx-community/whisper-large-v3-turbo-asr-fp16";
const EXTS: &[&str] = &[
    "wav", "m4a", "mp3", "aiff", "aif", "caf", "aac", "flac", "mp4",
];

pub fn is_audio(p: &Path) -> bool {
    p.extension()
        .map(|e| EXTS.contains(&e.to_string_lossy().to_lowercase().as_str()))
        .unwrap_or(false)
}

/// Duración en segundos (vía `afinfo`, de macOS).
pub fn seconds(p: &Path) -> Option<f64> {
    let o = std::process::Command::new("afinfo").arg(p).output().ok()?;
    let t = String::from_utf8_lossy(&o.stdout);
    t.lines()
        .find(|l| l.contains("estimated duration"))
        .and_then(|l| l.split(':').nth(1))
        .and_then(|v| v.split_whitespace().next())
        .and_then(|v| v.parse().ok())
}

/// WAV 16 kHz mono en un archivo temporal (lo borra quien llama).
fn to_wav(p: &Path) -> Result<PathBuf> {
    if !p.is_file() {
        bail!(
            "{}",
            crate::tr!(
                "no existe el audio {}",
                "audio {} does not exist",
                p.display()
            )
        );
    }
    if !is_audio(p) {
        bail!(
            "{}",
            crate::tr!(
                "{} no parece un audio",
                "{} does not look like audio",
                p.display()
            )
        );
    }
    if let Some(s) = seconds(p) {
        if s > MAX_SECONDS {
            bail!(
                "{}",
                crate::tr!(
                "{} dura {s:.0} s; el tope es {MAX_SECONDS:.0} s (córtalo o pásalo por partes)",
                "{} lasts {s:.0} s; the limit is {MAX_SECONDS:.0} s (trim it or send it in parts)",
                p.display()
            )
            );
        }
    }
    let tmp = std::env::temp_dir().join(format!(
        "gotv-aud-{}-{}.wav",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    let ok = std::process::Command::new("afconvert")
        .args(["-f", "WAVE", "-d", "LEI16@16000", "-c", "1"])
        .arg(p)
        .arg(&tmp)
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if !ok || !tmp.exists() {
        bail!(
            "{}",
            crate::tr!(
                "no pude convertir {} (afconvert)",
                "could not convert {} (afconvert)",
                p.display()
            )
        );
    }
    Ok(tmp)
}

/// Audio listo para un modelo que OYE: (`data:audio/wav;base64,…`, segundos).
pub fn data_url(p: &Path) -> Result<(String, f64)> {
    let wav = to_wav(p)?;
    let bytes = std::fs::read(&wav)?;
    let _ = std::fs::remove_file(&wav);
    let secs = bytes.len() as f64 / 32_000.0;
    Ok((
        format!("data:audio/wav;base64,{}", crate::images::base64(&bytes)),
        secs,
    ))
}

/// El Whisper elegido: `GOTV_STT_REPO` o large-v3-turbo (el mejor en español).
pub fn stt_repo() -> String {
    std::env::var("GOTV_STT_REPO")
        .ok()
        .filter(|r| crate::hub::valid_repo(r))
        .unwrap_or_else(|| STT_REPO.to_string())
}

/// Carpeta local de Whisper, si se bajó. El del catálogo exige su licencia ACEPTADA (la casilla de
/// Configuración → Generadores, como todo modelo); uno propio por `GOTV_STT_REPO` queda a cargo de
/// quien lo elige.
pub fn whisper_dir() -> Option<PathBuf> {
    let repo = stt_repo();
    if crate::generadores::ficha(&repo).is_some() && crate::generadores::aceptacion(&repo).is_none() {
        return None;
    }
    let d = crate::hub::models_dir().join(&repo);
    d.join("config.json").exists().then_some(d)
}

/// Qué hacer si falta Whisper: aceptarlo y bajarlo en la app (o con la CLI).
pub fn falta_whisper() -> String {
    crate::tr!(
        "falta Whisper ({}): acéptalo y bájalo en Configuración → Generadores (o `gotv setup audio`)",
        "Whisper is missing ({}): accept and download it in Settings → Generators (or `gotv setup audio`)",
        stt_repo()
    )
}

/// M1: el Whisper NATIVO (Rust + MLX, sin Python) — `(carpeta de Whisper, WAV 16 kHz mono) → texto`.
/// Lo registran el CLI y la GUI al arrancar (`gotv-core` no depende del núcleo MLX).
pub type Transcriptor = fn(&Path, &[u8]) -> Result<String>;
static NATIVO: std::sync::OnceLock<Transcriptor> = std::sync::OnceLock::new();

pub fn registrar_transcriptor(f: Transcriptor) {
    let _ = NATIVO.set(f);
}

/// Transcribe con Whisper para un modelo que no oye (decisión #4: se declara como transcripción).
/// SÓLO el Whisper del núcleo (Rust + MLX): desde 2026-11-12 GOTV no tiene respaldo en Python.
pub fn transcribe(p: &Path) -> Result<String> {
    let model = whisper_dir().ok_or_else(|| anyhow!("{}", falta_whisper()))?;
    let f = NATIVO.get().ok_or_else(|| anyhow!("{}", crate::i18n::t("este binario no trae el Whisper del núcleo", "this binary has no core Whisper")))?;
    let wav = to_wav(p)?;
    let r = std::fs::read(&wav).map_err(anyhow::Error::from).and_then(|b| f(&model, &b));
    let _ = std::fs::remove_file(&wav);
    match r {
        Ok(t) if !t.trim().is_empty() => Ok(t.trim().to_string()),
        Ok(_) => bail!("{}", crate::i18n::t("Whisper no oyó nada que transcribir", "Whisper heard nothing to transcribe")),
        Err(e) => bail!("{}", crate::tr!("Whisper no transcribió: {e:#}", "Whisper did not transcribe: {e:#}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn convierte_mide_y_acota() {
        let d = crate::CarpetaPrueba::sin_crear("gotv-audio");
        std::fs::create_dir_all(&d).unwrap();
        let aiff = d.join("v.aiff");
        // `say` viene con macOS: 1-2 s de voz real
        if !std::process::Command::new("say")
            .args(["-o"])
            .arg(&aiff)
            .arg("siete tres")
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
        {
            return;
        }
        let (url, secs) = data_url(&aiff).unwrap();
        assert!(
            url.starts_with("data:audio/wav;base64,UklGR"),
            "WAV real (RIFF)"
        );
        assert!(secs > 0.3 && secs < 10.0, "{secs}");
        assert!(data_url(Path::new("/no/existe.wav")).is_err());
        assert!(data_url(Path::new("Cargo.toml")).is_err());
    }
}
