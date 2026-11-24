//! F2.1 · Notificaciones del sistema: GOTV avisa cuando algo termina o te necesita y no estás mirando.
//!
//! - **La decisión es una función pura** ([`debe_avisar`]): el tipo de evento, si la ventana está al frente
//!   y los interruptores `[avisos] turno, permiso, programadas` (anexo A; encendidos por omisión).
//! - Turno terminado, permiso pedido, vigilancia disparada y tarea larga: sólo con la ventana en segundo
//!   plano. Tarea programada terminada o detenida: siempre (corre sin ventana).
//! - El notificador real es `UserNotifications` de macOS hablado desde Rust con `objc2` (sin compilar
//!   Objective-C ni lanzar otro programa); las pruebas usan uno falso. Vive en el núcleo (no sólo en la
//!   ventana) porque las tareas programadas corren desde la CLI, con la app cerrada. macOS sólo deja avisar
//!   a un proceso dentro de un paquete de app: fuera de él (un `gotv` suelto) se dice y no se avisa.
use anyhow::Result;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Evento {
    TurnoTerminado,
    PermisoPedido,
    ProgramadaTerminada,
    ProgramadaDetenida,
    VigilanciaDisparada,
    /// compilación o instalación larga (F18)
    LargaTerminada,
}

/// Los interruptores de Configuración → General (`[avisos]` en la config).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Interruptores {
    pub turno: bool,
    pub permiso: bool,
    pub programadas: bool,
}

impl Default for Interruptores {
    fn default() -> Self {
        Self { turno: true, permiso: true, programadas: true }
    }
}

impl Interruptores {
    /// `~/.config/gotv/config.toml` ← `.gotv/config.toml` del proyecto (manda el proyecto).
    pub fn cargar(proyecto: Option<&Path>) -> Self {
        let mut i = Self::default();
        let mut capas = vec![crate::config::global_path()];
        if let Some(p) = proyecto {
            capas.push(p.join(".gotv/config.toml"));
        }
        for c in capas {
            i.aplicar(&std::fs::read_to_string(c).unwrap_or_default());
        }
        i
    }

    /// Lo que diga la tabla `[avisos]` de un archivo de config.
    pub fn aplicar(&mut self, toml_texto: &str) {
        let Ok(v) = toml_texto.parse::<toml::Table>() else { return };
        let Some(a) = v.get("avisos").and_then(|a| a.as_table()) else { return };
        let b = |k: &str| a.get(k).and_then(|x| x.as_bool());
        if let Some(x) = b("turno") {
            self.turno = x;
        }
        if let Some(x) = b("permiso") {
            self.permiso = x;
        }
        if let Some(x) = b("programadas") {
            self.programadas = x;
        }
    }
}

/// ¿Se avisa? (pura: la prueba la recorre entera)
pub fn debe_avisar(ev: Evento, ventana_al_frente: bool, i: &Interruptores) -> bool {
    match ev {
        Evento::ProgramadaTerminada | Evento::ProgramadaDetenida => i.programadas,
        Evento::PermisoPedido => i.permiso && !ventana_al_frente,
        Evento::TurnoTerminado | Evento::VigilanciaDisparada | Evento::LargaTerminada => i.turno && !ventana_al_frente,
    }
}

pub trait Notificador: Send + Sync {
    fn notificar(&self, titulo: &str, cuerpo: &str) -> Result<()>;
}

/// `UserNotifications` de macOS: la notificación sale a nombre del paquete de app en el que corre GOTV.
pub struct Sistema;

/// ¿Corre dentro de un paquete de app? (`UNUserNotificationCenter` lanza una excepción si no.)
pub fn en_paquete() -> Option<String> {
    objc2_foundation::NSBundle::mainBundle().bundleIdentifier().map(|s| s.to_string())
}

impl Notificador for Sistema {
    fn notificar(&self, titulo: &str, cuerpo: &str) -> Result<()> {
        use objc2_foundation::NSString;
        use objc2_user_notifications::{UNAuthorizationOptions, UNMutableNotificationContent, UNNotificationRequest, UNUserNotificationCenter};
        if en_paquete().is_none() {
            anyhow::bail!("{}", crate::i18n::t(
                "este gotv no corre dentro de la app (sin paquete): macOS no le deja mostrar avisos",
                "this gotv does not run inside the app (no bundle): macOS does not let it show notifications"
            ));
        }
        let centro = UNUserNotificationCenter::currentNotificationCenter();
        // la primera vez macOS pregunta al usuario; después recuerda su respuesta
        static PERMISO: std::sync::Once = std::sync::Once::new();
        PERMISO.call_once(|| {
            let nada = block2::RcBlock::new(|_ok: objc2::runtime::Bool, _e: *mut objc2_foundation::NSError| {});
            centro.requestAuthorizationWithOptions_completionHandler(UNAuthorizationOptions::Alert | UNAuthorizationOptions::Sound, &nada);
        });
        let contenido = UNMutableNotificationContent::new();
        contenido.setTitle(&NSString::from_str(titulo));
        contenido.setBody(&NSString::from_str(cuerpo));
        static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let id = format!("gotv-{}-{}", std::process::id(), N.fetch_add(1, std::sync::atomic::Ordering::Relaxed));
        let pedido = UNNotificationRequest::requestWithIdentifier_content_trigger(&NSString::from_str(&id), &contenido, None);
        centro.addNotificationRequest_withCompletionHandler(&pedido, None);
        Ok(())
    }
}

/// Avisa si toca. Devuelve si avisó (un fallo del sistema de avisos no tumba lo que avisa: se dice en stderr).
pub fn avisar(n: &dyn Notificador, ev: Evento, ventana_al_frente: bool, i: &Interruptores, titulo: &str, cuerpo: &str) -> bool {
    if !debe_avisar(ev, ventana_al_frente, i) {
        return false;
    }
    match n.notificar(titulo, cuerpo) {
        Ok(()) => true,
        Err(e) => {
            eprintln!("{}", crate::tr!("⚠ no pude mostrar el aviso ({e})", "⚠ could not show the notification ({e})"));
            false
        }
    }
}

/// Un notificador que guarda lo que habría mostrado (pruebas; nada sale a la pantalla).
#[derive(Default)]
pub struct Falso {
    pub enviados: std::sync::Mutex<Vec<(String, String)>>,
}

impl Notificador for Falso {
    fn notificar(&self, titulo: &str, cuerpo: &str) -> Result<()> {
        self.enviados.lock().unwrap().push((titulo.to_string(), cuerpo.to_string()));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn la_decision_de_avisar_es_pura_y_completa() {
        let todo = Interruptores::default();
        // con la ventana en segundo plano, terminar un turno y pedir un permiso avisan; al frente, no
        for ev in [Evento::TurnoTerminado, Evento::PermisoPedido, Evento::VigilanciaDisparada, Evento::LargaTerminada] {
            assert!(debe_avisar(ev, false, &todo), "{ev:?} en segundo plano");
            assert!(!debe_avisar(ev, true, &todo), "{ev:?} al frente");
        }
        // lo programado corre sin ventana: avisa siempre
        for ev in [Evento::ProgramadaTerminada, Evento::ProgramadaDetenida] {
            assert!(debe_avisar(ev, true, &todo) && debe_avisar(ev, false, &todo));
        }
        // cada interruptor apaga lo suyo y sólo lo suyo
        let sin_turno = Interruptores { turno: false, ..todo };
        assert!(!debe_avisar(Evento::TurnoTerminado, false, &sin_turno) && debe_avisar(Evento::PermisoPedido, false, &sin_turno));
        let sin_permiso = Interruptores { permiso: false, ..todo };
        assert!(!debe_avisar(Evento::PermisoPedido, false, &sin_permiso) && debe_avisar(Evento::TurnoTerminado, false, &sin_permiso));
        let sin_prog = Interruptores { programadas: false, ..todo };
        assert!(!debe_avisar(Evento::ProgramadaDetenida, false, &sin_prog) && debe_avisar(Evento::PermisoPedido, false, &sin_prog));
    }

    #[test]
    fn interruptores_de_la_config_y_el_notificador_falso() {
        let mut i = Interruptores::default();
        i.aplicar("mode = \"default\"\n[avisos]\nturno = false\nprogramadas = true\n");
        assert_eq!(i, Interruptores { turno: false, permiso: true, programadas: true });
        i.aplicar("[avisos\nroto");
        assert!(!i.turno, "un archivo roto no cambia nada");
        let f = Falso::default();
        assert!(!avisar(&f, Evento::TurnoTerminado, false, &i, "GOTV", "listo"));
        assert!(avisar(&f, Evento::ProgramadaDetenida, true, &i, "GOTV · programada", "falta un permiso"));
        assert_eq!(f.enviados.lock().unwrap().as_slice(), [("GOTV · programada".to_string(), "falta un permiso".to_string())]);
        // y la config con [avisos] sigue valiendo entera (no se ignora por la tabla nueva)
        let d = crate::CarpetaPrueba::nueva("gotv-avisos-config");
        std::fs::create_dir_all(d.join(".gotv")).unwrap();
        std::fs::write(d.join(".gotv/config.toml"), "mode = \"plan\"\n[avisos]\npermiso = false\n").unwrap();
        let c = crate::Config::load(&d);
        assert_eq!(c.mode, "plan", "{:?}", c.warnings);
        assert!(!c.warnings.iter().any(|w| w.contains("avisos")), "{:?}", c.warnings);
        assert!(!Interruptores::cargar(Some(&d)).permiso);
    }

    /// El notificador REAL fuera de un paquete de app (el binario de pruebas no lo es): lo dice y no toca
    /// `UNUserNotificationCenter` (que lanzaría una excepción de Objective-C). Nada sale a la pantalla.
    #[test]
    fn el_notificador_real_sin_paquete_se_niega_sin_romper() {
        assert_eq!(en_paquete(), None);
        let e = Sistema.notificar("GOTV", "no debe salir").unwrap_err().to_string();
        assert!(e.contains("paquete") || e.contains("bundle"), "{e}");
    }
}
