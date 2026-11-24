//! F18 · M8 · F18.I · Xcode: lo que se puede y lo que no (pasos 1–5 de la ficha).
//!
//! 1. Obtener Xcode: sólo se abre su página de la Mac App Store. Apple lo entrega con una cuenta de Apple;
//!    GOTV **nunca** pide ni toca esa cuenta: la descarga y su licencia las maneja Apple.
//! 2. Detectarlo: `xcode-select -p` y `xcodebuild -version` (`instalador::estado_xcode`).
//! 3. Prepararlo: `xcodebuild -license check` y `-checkFirstLaunchStatus` dicen si falta (medido: código 0 =
//!    hecho). Si falta, se muestra el acuerdo (`Contents/Resources/en.lproj/License.rtf`) y, si el usuario
//!    acepta, `xcodebuild -license accept && xcodebuild -runFirstLaunch` con el cuadro de administrador de
//!    macOS (`tools::run_elevated`): GOTV no ve la contraseña.
//! 4. Simulador de iOS: `xcodebuild -downloadPlatform iOS` (baja de Apple, varios GB).
//! 5. idb (Meta, MIT) para leer y tocar el simulador: versión FIJADA a mano tras probarla, con el SHA-256 que
//!    Meta publica junto al archivo; se instala con el mismo instalador que los servidores de lenguaje
//!    (`lsp::descargas`) en `~/Library/Application Support/gotv/idb/<versión>/`, donde `idb::companion` lo busca.
use crate::lsp::descargas::{Descarga, Formato};
use anyhow::{bail, Result};
use std::path::PathBuf;

/// idb_companion fijado (2026-11-17: v1.7.8, el último de github.com/meta/idb; 30,9 MB; suma del
/// `.sha256` que Meta publica junto al archivo).
pub const IDB: Descarga = Descarga {
    id: "idb",
    programa: "idb_companion",
    version: "1.7.8",
    url: "https://github.com/meta/idb/releases/download/v1.7.8/idb-companion.macos-arm64.tar.gz",
    sha256: "44369a2df0b62e3a1764de61bd09280999c7e9b27ea8f20bff1ea3a9db0302a2",
    formato: Formato::TarGz,
    ruta: "bin/idb_companion",
    requiere: &[],
};
pub const IDB_TAMANO: u64 = 30_893_766;
pub const IDB_LICENCIA: &str = "https://github.com/meta/idb/blob/main/LICENSE";

/// La carpeta de lo que baja GOTV para el móvil.
pub fn base() -> PathBuf {
    crate::paths::home().join("Library/Application Support/gotv")
}

/// Baja idb fijado, comprueba su SHA-256 y lo deja donde `idb::companion` lo busca.
pub async fn instalar_idb() -> Result<PathBuf> {
    crate::lsp::descargas::instalar_en(&base(), &IDB).await
}

/// Lo que falta para que Xcode compile para el simulador (paso 3).
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct Preparacion {
    pub licencia_aceptada: bool,
    pub primer_arranque_hecho: bool,
    /// El acuerdo de Xcode y los SDK de Apple, para mostrarlo ENTERO antes de aceptar.
    pub licencia: Option<PathBuf>,
}

async fn ok(args: &[&str]) -> bool {
    crate::entorno::comando_tokio("xcodebuild").args(args).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status().await.is_ok_and(|s| s.success())
}

pub async fn preparacion() -> Preparacion {
    let licencia = crate::entorno::comando_tokio("xcode-select").arg("-p").output().await.ok().filter(|o| o.status.success()).map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).map(|dev| PathBuf::from(dev).join("../Resources/en.lproj/License.rtf")).filter(|p| p.is_file());
    Preparacion { licencia_aceptada: ok(&["-license", "check"]).await, primer_arranque_hecho: ok(&["-checkFirstLaunchStatus"]).await, licencia }
}

/// La orden que aprueba el humano en el cuadro de macOS (se ve EXACTA en ese cuadro).
pub const ORDEN_PREPARAR: &str = "xcodebuild -license accept && xcodebuild -runFirstLaunch";

/// Acepta la licencia de Xcode y hace su primer arranque, SÓLO tras el «Acepto» del humano en la ventana
/// (el modelo no tiene herramienta para esto) y con el cuadro de administrador de macOS.
pub async fn preparar() -> Result<String> {
    let p = preparacion().await;
    if p.licencia_aceptada && p.primer_arranque_hecho {
        return Ok(crate::i18n::t("Xcode ya está preparado.", "Xcode is already prepared.").into());
    }
    crate::tools::run_elevated(ORDEN_PREPARAR, &crate::paths::home(), 1800).await
}

/// Baja la plataforma del simulador de iOS de Apple (varios GB; puede tardar mucho).
pub async fn bajar_plataforma_ios() -> Result<String> {
    let o = crate::entorno::comando_tokio("xcodebuild").args(["-downloadPlatform", "iOS"]).stdin(std::process::Stdio::null()).output().await?;
    let t = format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr));
    if !o.status.success() {
        bail!("{}", crate::tr!("xcodebuild -downloadPlatform iOS falló: {}", "xcodebuild -downloadPlatform iOS failed: {}", t.lines().rev().take(6).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join("\n")));
    }
    Ok(t.lines().last().unwrap_or("").to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// idb fijado queda EXACTAMENTE donde `idb::companion` lo busca, viene de Meta por https y trae una suma
    /// SHA-256 de 64 cifras hexadecimales.
    #[test]
    fn idb_fijado_y_donde_se_busca() {
        assert!(IDB.url.starts_with("https://github.com/meta/idb/releases/download/v1.7.8/"));
        assert!(IDB.sha256.len() == 64 && IDB.sha256.chars().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(base().join(IDB.id).join(IDB.version).join(IDB.ruta), crate::paths::home().join("Library/Application Support/gotv/idb/1.7.8/bin/idb_companion"));
        assert!(ORDEN_PREPARAR.contains("-license accept") && ORDEN_PREPARAR.contains("-runFirstLaunch"));
    }

    /// De verdad: idb bajado de Meta a una carpeta temporal, con la suma comprobada, y que arranca; y la
    /// preparación de Xcode de esta Mac (sólo lectura). `cargo test -p gotv-core idb_y_xcode_de_verdad --
    /// --ignored --nocapture`
    #[tokio::test]
    #[ignore]
    async fn idb_y_xcode_de_verdad() {
        let d = crate::CarpetaPrueba::nueva("gotv-idb-real");
        let t = std::time::Instant::now();
        let exe = crate::lsp::descargas::instalar_en(&d, &IDB).await.unwrap();
        eprintln!("idb instalado en {:.1} s: {}", t.elapsed().as_secs_f32(), exe.display());
        let o = std::process::Command::new(&exe).arg("--help").output().unwrap();
        let s = format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr));
        assert!(s.contains("udid") || s.contains("Usage"), "{s}");
        let p = preparacion().await;
        eprintln!("{p:?}");
        assert!(p.licencia.is_some());
    }
}
