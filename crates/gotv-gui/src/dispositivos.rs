//! F18 · M9 · Configuración → Conexiones → «Dispositivos» (F18.I): instalar el SDK de Android con un clic y
//! sus avisos legales, y el estado de Xcode. Órdenes delgadas sobre `gotv_core::movil::instalador`:
//! - el texto de una licencia SIEMPRE sale del catálogo oficial, nunca de la ventana (como en Generadores);
//! - «Acepto» acepta UNA licencia (jamás en bloque); instalar exige la suya aceptada;
//! - sólo se desinstala lo que instaló GOTV;
//! - Xcode: GOTV sólo abre su página de la Mac App Store; la cuenta y la licencia de Apple son de Apple.
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

/// Las descargas en curso: paquete → (bajado, total, error si acabó mal).
fn avances() -> &'static Mutex<HashMap<String, (u64, u64, Option<String>)>> {
    static A: OnceLock<Mutex<HashMap<String, (u64, u64, Option<String>)>>> = OnceLock::new();
    A.get_or_init(Default::default)
}

/// Lo que pinta la sección: el plan con su origen y tamaño, las licencias con su texto, lo instalado, los
/// dispositivos virtuales, las descargas en curso y Xcode.
#[tauri::command]
pub async fn devices_state() -> Result<serde_json::Value, String> {
    let android = gotv_core::movil::instalador::estado().await.map_err(|e| e.to_string());
    let xcode = gotv_core::movil::instalador::estado_xcode().await;
    let progreso: Vec<serde_json::Value> = avances()
        .lock()
        .map(|m| m.iter().map(|(p, (b, t, e))| serde_json::json!({"paquete": p, "bajado": b, "total": t, "error": e})).collect())
        .unwrap_or_default();
    Ok(serde_json::json!({
        "android": android.as_ref().ok(),
        "android_error": android.as_ref().err(),
        "xcode": xcode,
        "progreso": progreso,
    }))
}

/// El usuario pulsó «Acepto» en UNA licencia (`id` del catálogo).
#[tauri::command]
pub async fn devices_accept(id: String) -> Result<(), String> {
    gotv_core::movil::instalador::aceptar_licencia(&id).await.map_err(|e| e.to_string())
}

/// Baja e instala UN paquete del plan en segundo plano; su avance sale en `devices_state`.
#[tauri::command]
pub async fn devices_install(paquete: String) -> Result<(), String> {
    {
        let mut m = avances().lock().map_err(|e| e.to_string())?;
        if m.get(&paquete).is_some_and(|(_, _, e)| e.is_none()) {
            return Err(gotv_core::i18n::t("ya se está bajando", "it is already downloading").into());
        }
        m.insert(paquete.clone(), (0, 0, None));
    }
    tokio::spawn(async move {
        let p = paquete.clone();
        let avance = move |b: u64, t: u64| {
            if let Ok(mut m) = avances().lock() {
                m.insert(p.clone(), (b, t, None));
            }
        };
        let r = gotv_core::movil::instalador::instalar_paquete(&paquete, &avance).await;
        if let Ok(mut m) = avances().lock() {
            match r {
                Ok(_) => {
                    m.remove(&paquete);
                }
                Err(e) => {
                    let (b, t, _) = m.get(&paquete).cloned().unwrap_or((0, 0, None));
                    m.insert(paquete, (b, t, Some(format!("{e:#}"))));
                }
            }
        }
    });
    Ok(())
}

/// Desinstala un paquete que instaló GOTV.
#[tauri::command]
pub fn devices_uninstall(paquete: String) -> Result<(), String> {
    gotv_core::movil::instalador::desinstalar_paquete(&paquete).map_err(|e| e.to_string())
}

/// Xcode, paso 3: el acuerdo de Xcode y los SDK de Apple, abierto ENTERO (el archivo que trae Xcode).
#[tauri::command]
pub async fn devices_xcode_license() -> Result<(), String> {
    let p = gotv_core::movil::xcode::preparacion().await;
    let l = p.licencia.ok_or_else(|| gotv_core::i18n::t("no encuentro el acuerdo de Xcode", "the Xcode agreement was not found").to_string())?;
    std::process::Command::new("open").arg(&l).spawn().map(|_| ()).map_err(|e| e.to_string())
}

/// Xcode, paso 3: el humano pulsó «Acepto»: licencia y primer arranque con el cuadro de administrador de
/// macOS (GOTV no ve la contraseña).
#[tauri::command]
pub async fn devices_xcode_prepare() -> Result<String, String> {
    gotv_core::movil::xcode::preparar().await.map_err(|e| e.to_string())
}

/// Xcode, paso 4: baja la plataforma del simulador de iOS de Apple (en segundo plano; el estado lo dice).
#[tauri::command]
pub async fn devices_xcode_platform() -> Result<(), String> {
    let clave = "ios-platform".to_string();
    if avances().lock().map_err(|e| e.to_string())?.get(&clave).is_some_and(|(_, _, e)| e.is_none()) {
        return Err(gotv_core::i18n::t("ya se está bajando", "it is already downloading").into());
    }
    avances().lock().map_err(|e| e.to_string())?.insert(clave.clone(), (0, 0, None));
    tokio::spawn(async move {
        let r = gotv_core::movil::xcode::bajar_plataforma_ios().await;
        if let Ok(mut m) = avances().lock() {
            match r {
                Ok(_) => {
                    m.remove(&clave);
                }
                Err(e) => {
                    m.insert(clave, (0, 0, Some(format!("{e:#}"))));
                }
            }
        }
    });
    Ok(())
}

/// Xcode, paso 5: idb (Meta, MIT), versión fijada con su SHA-256.
#[tauri::command]
pub async fn devices_idb_install() -> Result<String, String> {
    gotv_core::movil::xcode::instalar_idb().await.map(|p| p.display().to_string()).map_err(|e| e.to_string())
}

/// Xcode, paso 1: su página en la Mac App Store (dirección fija, no la que mande la ventana).
#[tauri::command]
pub fn devices_xcode_store() -> Result<(), String> {
    let url = "macappstore://apps.apple.com/app/xcode/id497799835";
    std::process::Command::new("open").arg(url).spawn().map(|_| ()).map_err(|e| e.to_string())
}
