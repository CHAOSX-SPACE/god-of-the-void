//! F2.3 paso 5 · El `LaunchAgent` que corre las tareas programadas con la app cerrada:
//! `~/Library/LaunchAgents/space.chaosx.gotv.programadas.plist` lanza `gotv programadas pendientes` cada 60 s.
//!
//! - **Instalarlo cambia la configuración del sistema:** [`instalar`] exige `confirmado` (lo pregunta la
//!   CLI o la ventana al crear la primera tarea, SIEMPRE, y lo responde una persona); se quita igual.
//! - Trampa (F0.2): `launchd` no ve el `PATH` de la terminal → el `.plist` lleva el `PATH` del usuario
//!   resuelto por [`crate::entorno::path`].
//! - Rust puro, sin otro programa: GOTV sólo ESCRIBE el `.plist` (el formato de launchd, que es externo) en
//!   `~/Library/LaunchAgents`, que launchd lee solo al iniciar sesión. No se llama a `launchctl`: instalar
//!   vale desde el próximo inicio de sesión, y quitar vale YA, porque la corrida lanzada por launchd lleva
//!   [`MARCA`] en su entorno y no hace nada si su `.plist` ya no está ([`vigente`]).
//! - Con la Mac dormida no corre nada: al despertar, lo atrasado corre UNA vez (ver [`super::tareas`]).
use anyhow::{bail, Result};
use std::path::{Path, PathBuf};

pub const ETIQUETA: &str = "space.chaosx.gotv.programadas";
/// Variable de entorno que el `.plist` le pone a la corrida que lanza launchd.
pub const MARCA: &str = "GOTV_LANZADO_POR_LAUNCHD";
/// Cada cuánto lo lanza `launchd` (segundos).
pub const INTERVALO: u32 = 60;

pub fn ruta_plist(casa: &Path) -> PathBuf {
    casa.join("Library/LaunchAgents").join(format!("{ETIQUETA}.plist"))
}

fn xml(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&apos;")
}

/// El `.plist` (texto). `gotv` = la ruta ABSOLUTA del binario; `path` = el `PATH` del usuario.
pub fn plist(gotv: &Path, path: &str, registro: &Path) -> String {
    let (g, p, r) = (xml(&gotv.display().to_string()), xml(path), xml(&registro.display().to_string()));
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>Label</key>
	<string>{ETIQUETA}</string>
	<key>ProgramArguments</key>
	<array>
		<string>{g}</string>
		<string>programadas</string>
		<string>pendientes</string>
	</array>
	<key>StartInterval</key>
	<integer>{INTERVALO}</integer>
	<key>RunAtLoad</key>
	<false/>
	<key>ProcessType</key>
	<string>Background</string>
	<key>EnvironmentVariables</key>
	<dict>
		<key>PATH</key>
		<string>{p}</string>
		<key>{MARCA}</key>
		<string>1</string>
	</dict>
	<key>StandardOutPath</key>
	<string>{r}</string>
	<key>StandardErrorPath</key>
	<string>{r}</string>
</dict>
</plist>
"#
    )
}

/// Escribe el `.plist` en `casa` (sin cargarlo). Lo usan [`instalar`] y las pruebas (en una carpeta temporal).
pub fn escribir(casa: &Path, gotv: &Path, path: &str) -> Result<PathBuf> {
    if !gotv.is_absolute() {
        bail!("{}", crate::tr!("la ruta de gotv debe ser absoluta ({})", "the gotv path must be absolute ({})", gotv.display()));
    }
    let ruta = ruta_plist(casa);
    if let Some(d) = ruta.parent() {
        std::fs::create_dir_all(d)?;
    }
    let registro = casa.join(".cache/gotv/programadas.log");
    if let Some(d) = registro.parent() {
        std::fs::create_dir_all(d)?;
    }
    std::fs::write(&ruta, plist(gotv, path, &registro))?;
    Ok(ruta)
}

fn casa() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_default())
}

/// Escribe el agente. SÓLO con `confirmado` (una persona dijo que sí, con el texto delante). launchd lo
/// carga solo en el próximo inicio de sesión.
pub fn instalar(confirmado: bool) -> Result<String> {
    if !confirmado {
        bail!("{}", crate::i18n::t(
            "instalar el agente de launchd cambia la configuración de tu Mac: hace falta tu sí explícito (`gotv programadas agente instalar` lo pregunta)",
            "installing the launchd agent changes your Mac's configuration: your explicit yes is needed (`gotv programadas agente instalar` asks for it)"
        ));
    }
    let ruta = escribir(&casa(), &std::env::current_exe()?, crate::entorno::path())?;
    Ok(crate::tr!(
        "✅ agente escrito: {} · launchd lo carga en tu próximo inicio de sesión y desde ahí corre lo pendiente cada {INTERVALO} s (hasta entonces: `gotv programadas pendientes`)",
        "✅ agent written: {} · launchd loads it at your next login and from then on runs what is due every {INTERVALO} s (until then: `gotv programadas pendientes`)",
        ruta.display()
    ))
}

/// Borra SU `.plist` (ruta exacta). Pide la misma confirmación. Vale al instante: la siguiente corrida que
/// lance launchd ve que no está y no hace nada; launchd lo olvida al cerrar sesión.
pub fn quitar(confirmado: bool) -> Result<String> {
    if !confirmado {
        bail!("{}", crate::i18n::t("quitar el agente de launchd también pide tu sí explícito", "removing the launchd agent also needs your explicit yes"));
    }
    let ruta = ruta_plist(&casa());
    if ruta.is_file() {
        std::fs::remove_file(&ruta)?;
    }
    Ok(crate::tr!("agente quitado ({}): desde ya no corre nada", "agent removed ({}): nothing runs from now on", ruta.display()))
}

#[derive(Debug, PartialEq)]
pub enum Estado {
    NoInstalado,
    Instalado,
}

pub fn estado(casa: &Path) -> Estado {
    if ruta_plist(casa).is_file() {
        Estado::Instalado
    } else {
        Estado::NoInstalado
    }
}

/// ¿Esta corrida puede seguir? Una que lanzó launchd (lleva [`MARCA`]) sólo si su `.plist` sigue en `casa`;
/// una que pidió una persona (`gotv programadas pendientes` a mano), siempre.
pub fn vigente(marca: Option<&str>, casa: &Path) -> bool {
    marca.is_none() || ruta_plist(casa).is_file()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn el_plist_se_genera_valido_en_una_carpeta_temporal_y_sin_cargar() {
        let casa = crate::CarpetaPrueba::nueva("gotv-launchd");
        let path = "/opt/homebrew/bin:/usr/bin:/bin:/Users/x/.cargo/bin";
        let ruta = escribir(&casa, Path::new("/Applications/GOTV & co/gotv"), path).unwrap();
        assert_eq!(ruta, casa.join("Library/LaunchAgents/space.chaosx.gotv.programadas.plist"));
        let t = std::fs::read_to_string(&ruta).unwrap();
        assert!(t.contains("<string>programadas</string>") && t.contains("<string>pendientes</string>"));
        assert!(t.contains("<integer>60</integer>") && t.contains(path), "el PATH del usuario va dentro (launchd no lo ve)");
        assert!(t.contains("GOTV &amp; co"), "escapado XML");
        // bien formado (comprobado en Rust): cada etiqueta abierta se cierra en orden y cada <key> lleva valor
        let mut pila: Vec<String> = Vec::new();
        for trozo in t.split('<').skip(1) {
            let etiqueta = trozo.split('>').next().unwrap();
            if etiqueta.starts_with('?') || etiqueta.starts_with('!') || etiqueta.ends_with('/') {
                continue;
            }
            match etiqueta.strip_prefix('/') {
                Some(cierre) => assert_eq!(pila.pop().as_deref(), Some(cierre), "cierre fuera de orden en {etiqueta}"),
                None => pila.push(etiqueta.split_whitespace().next().unwrap().to_string()),
            }
        }
        assert!(pila.is_empty(), "sin cerrar: {pila:?}");
        assert_eq!(t.matches("<key>").count(), 10, "Label, ProgramArguments, StartInterval, RunAtLoad, ProcessType, EnvironmentVariables, PATH, la marca, StandardOutPath, StandardErrorPath");
        assert!(t.contains(&format!("<key>{MARCA}</key>")));
        // instalado; la corrida de launchd sigue mientras el .plist esté, y se detiene en cuanto se borra
        assert_eq!(estado(&casa), Estado::Instalado);
        assert!(vigente(Some("1"), &casa) && vigente(None, &casa));
        std::fs::remove_file(&ruta).unwrap();
        assert_eq!(estado(&casa), Estado::NoInstalado);
        assert!(!vigente(Some("1"), &casa), "quitado: la de launchd ya no hace nada");
        assert!(vigente(None, &casa), "a mano, sí");
        assert_eq!(estado(&casa.join("otra-casa")), Estado::NoInstalado);
        assert!(escribir(&casa, Path::new("gotv"), path).is_err(), "ruta relativa: launchd no la encontraría");
        // instalar o quitar sin el sí explícito: nada
        assert!(instalar(false).is_err() && quitar(false).is_err());
    }
}
