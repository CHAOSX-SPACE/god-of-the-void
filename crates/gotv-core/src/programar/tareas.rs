//! F2.3 · Tareas programadas: `~/.config/gotv/programadas.bin` (formato propio de [`super::codec`]) con `id`,
//! `nombre`, `cron` (5 campos + zona), `proyecto`, `orden`, `modelo`, `permisos` (reglas con alcance de
//! F14.4), `activa`, `ultima`, `proxima` y el resultado de la última corrida.
//!
//! - **Quién las corre:** `gotv programadas pendientes` (lo lanza el `LaunchAgent` cada 60 s, ver
//!   [`super::launchd`]); con un **candado por `id`** (`flock`): dos corridas a la vez → corre una.
//! - **Atrasada tras dormir corre UNA vez:** la próxima se calcula desde AHORA, no desde la que tocaba.
//! - **Sin ventana y sólo con los permisos concedidos:** modo `default` (nunca `bypass`), lo concedido
//!   (config, «siempre» del proyecto) más SUS reglas con alcance; lo que habría que preguntar se niega, la
//!   orden se DETIENE ahí, queda su motivo y se avisa ([`crate::avisos`]). Nunca decide sola.
//! - El resultado queda como un chat del proyecto cuyo título empieza por «⏰ [programada · nombre]».
use super::codec::{Escritor, Lector};
use anyhow::{anyhow, bail, Result};
use std::path::{Path, PathBuf};

/// El tipo y la versión del archivo del registro.
const TIPO: &[u8; 4] = b"TARE";
const VERSION: u8 = 1;
/// Lo que se guarda de la respuesta (o el error) de la última corrida; entera queda en su chat.
const ULTIMO_MAX: usize = 2_000;

#[derive(Clone, Debug, PartialEq)]
pub struct Tarea {
    pub id: String,
    pub nombre: String,
    pub cron: String,
    /// IANA; vacía = la de esta Mac
    pub zona: String,
    pub proyecto: PathBuf,
    pub orden: String,
    /// vacío = el del proyecto (o el automático)
    pub modelo: String,
    /// reglas con alcance: `bash(cargo test)`, `leer(docs/*)`, `red(api.github.com)`…
    pub permisos: Vec<String>,
    pub activa: bool,
    pub ultima: Option<i64>,
    pub proxima: Option<i64>,
    /// `hecho` · `detenida: …` · `fallo: …`
    pub estado: Option<String>,
    /// el chat que dejó la última corrida
    pub sesion: Option<i64>,
    /// la respuesta (o el error) de la última corrida, acotada
    pub ultimo: Option<String>,
}

impl Tarea {
    /// La próxima desde `despues` (o `None` si el cron no vuelve a ocurrir).
    pub fn calcular_proxima(&self, despues: i64) -> Result<Option<i64>> {
        Ok(super::proxima(&super::horario(&self.cron)?, super::zona(&self.zona)?, despues))
    }

    /// La orden tal como la recibe el agente (y el título del chat que deja).
    pub fn orden_marcada(&self) -> String {
        crate::tr!("⏰ [programada · {}] {}", "⏰ [scheduled · {}] {}", self.nombre, self.orden)
    }

    fn escribir(&self, e: &mut Escritor) {
        e.texto(&self.id);
        e.texto(&self.nombre);
        e.texto(&self.cron);
        e.texto(&self.zona);
        e.texto(&self.proyecto.to_string_lossy());
        e.texto(&self.orden);
        e.texto(&self.modelo);
        e.textos(&self.permisos);
        e.bool(self.activa);
        e.opt_i64(self.ultima);
        e.opt_i64(self.proxima);
        e.opt_texto(self.estado.as_deref());
        e.opt_i64(self.sesion);
        e.opt_texto(self.ultimo.as_deref());
    }

    fn leer(l: &mut Lector) -> Result<Tarea> {
        Ok(Tarea {
            id: l.texto()?,
            nombre: l.texto()?,
            cron: l.texto()?,
            zona: l.texto()?,
            proyecto: PathBuf::from(l.texto()?),
            orden: l.texto()?,
            modelo: l.texto()?,
            permisos: l.textos()?,
            activa: l.bool()?,
            ultima: l.opt_i64()?,
            proxima: l.opt_i64()?,
            estado: l.opt_texto()?,
            sesion: l.opt_i64()?,
            ultimo: l.opt_texto()?,
        })
    }
}

/// Lo que hace falta para crear una.
#[derive(Debug, Clone, Default)]
pub struct Nueva {
    pub nombre: String,
    pub cron: String,
    pub zona: String,
    pub proyecto: PathBuf,
    pub orden: String,
    pub modelo: String,
    pub permisos: Vec<String>,
}

pub fn ruta_por_omision() -> PathBuf {
    crate::config::global_path().with_file_name("programadas.bin")
}

/// La carpeta de candados, junto al registro.
pub fn carpeta_de(registro: &Path) -> PathBuf {
    registro.with_file_name("programadas")
}

/// El candado por archivo (`flock`), compartido con las sesiones.
pub use super::codec::Candado;

/// El registro de tareas.
pub struct Registro {
    pub ruta: PathBuf,
    pub tareas: Vec<Tarea>,
}

impl Registro {
    pub fn cargar(ruta: &Path) -> Result<Registro> {
        let bytes = match std::fs::read(ruta) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Registro { ruta: ruta.to_path_buf(), tareas: vec![] }),
            Err(e) => return Err(e.into()),
        };
        let leer = || -> Result<Vec<Tarea>> {
            let (mut l, _) = Lector::nuevo(&bytes, TIPO, VERSION)?;
            let n = l.u32()? as usize;
            let v = (0..n).map(|_| Tarea::leer(&mut l)).collect::<Result<Vec<_>>>()?;
            if !l.fin() {
                bail!("{}", crate::i18n::t("bytes de más al final", "extra bytes at the end"));
            }
            Ok(v)
        };
        let tareas = leer().map_err(|e| anyhow!("{}", crate::tr!("{} está dañado ({e}): no lo toco", "{} is damaged ({e}): I will not touch it", ruta.display())))?;
        Ok(Registro { ruta: ruta.to_path_buf(), tareas })
    }

    /// Guarda de forma atómica.
    pub fn guardar(&self) -> Result<()> {
        let mut e = Escritor::nuevo(TIPO, VERSION);
        e.u32(self.tareas.len() as u32);
        for t in &self.tareas {
            t.escribir(&mut e);
        }
        super::codec::guardar(&self.ruta, &e.bytes())
    }

    /// Leer, cambiar y guardar bajo el candado del registro (la CLI y el `LaunchAgent` pueden coincidir).
    pub fn con<T>(ruta: &Path, f: impl FnOnce(&mut Registro) -> Result<T>) -> Result<T> {
        let mut candado = ruta.as_os_str().to_owned();
        candado.push(".lock");
        let _c = Candado::esperar(Path::new(&candado))?;
        let mut r = Registro::cargar(ruta)?;
        let x = f(&mut r)?;
        r.guardar()?;
        Ok(x)
    }

    pub fn buscar(&self, id: &str) -> Option<&Tarea> {
        self.tareas.iter().find(|t| t.id == id)
    }

    pub fn crear(&mut self, n: Nueva, ahora: i64) -> Result<Tarea> {
        let nombre = n.nombre.trim();
        if nombre.is_empty() || n.orden.trim().is_empty() {
            bail!("{}", crate::i18n::t("faltan `nombre` y `orden`", "missing `nombre` and `orden`"));
        }
        if !n.proyecto.is_dir() {
            bail!("{}", crate::tr!("el proyecto {} no existe", "project {} does not exist", n.proyecto.display()));
        }
        for p in &n.permisos {
            if crate::permissions::Alcance::parse(p).is_none() {
                bail!("{}", crate::tr!("permiso «{p}» inválido: una regla con alcance, p. ej. bash(cargo test), leer(docs/*), red(api.github.com); nunca «*»", "invalid permission «{p}»: a scoped rule, e.g. bash(cargo test), leer(docs/*), red(api.github.com); never «*»"));
            }
        }
        let mut t = Tarea {
            id: String::new(),
            nombre: nombre.to_string(),
            cron: n.cron.trim().to_string(),
            zona: n.zona.trim().to_string(),
            proyecto: n.proyecto.canonicalize().unwrap_or(n.proyecto.clone()),
            orden: n.orden.trim().to_string(),
            modelo: n.modelo.trim().to_string(),
            permisos: n.permisos.clone(),
            activa: true,
            ultima: None,
            proxima: None,
            estado: None,
            sesion: None,
            ultimo: None,
        };
        t.proxima = t.calcular_proxima(ahora)?;
        if t.proxima.is_none() {
            bail!("{}", crate::tr!("«{}» no vuelve a ocurrir", "«{}» never happens again", t.cron));
        }
        // un id corto y estable: base 36 del instante + lo que haga falta para no repetir
        let mut n_id = ahora.max(0) as u64;
        loop {
            let id = format!("t{}", base36(n_id));
            if self.buscar(&id).is_none() {
                t.id = id;
                break;
            }
            n_id += 1;
        }
        self.tareas.push(t.clone());
        Ok(t)
    }

    pub fn quitar(&mut self, id: &str) -> Result<Tarea> {
        let i = self.tareas.iter().position(|t| t.id == id).ok_or_else(|| anyhow!("{}", crate::tr!("no existe la tarea {id}", "task {id} does not exist")))?;
        Ok(self.tareas.remove(i))
    }

    pub fn activar(&mut self, id: &str, si: bool, ahora: i64) -> Result<()> {
        let t = self.tareas.iter_mut().find(|t| t.id == id).ok_or_else(|| anyhow!("{}", crate::tr!("no existe la tarea {id}", "task {id} does not exist")))?;
        t.activa = si;
        if si {
            t.proxima = t.calcular_proxima(ahora)?;
        }
        Ok(())
    }

    /// Las que tocan ahora (activas y con `proxima` vencida), en orden de vencimiento.
    pub fn pendientes(&self, ahora: i64) -> Vec<String> {
        let mut v: Vec<&Tarea> = self.tareas.iter().filter(|t| t.activa && t.proxima.is_some_and(|p| p <= ahora)).collect();
        v.sort_by_key(|t| t.proxima);
        v.into_iter().map(|t| t.id.clone()).collect()
    }

    /// Tras correr: la próxima se calcula desde AHORA (lo atrasado corre una sola vez).
    pub fn marcar(&mut self, id: &str, ahora: i64, c: &Corrida) {
        if let Some(t) = self.tareas.iter_mut().find(|t| t.id == id) {
            t.ultima = Some(ahora);
            t.proxima = t.calcular_proxima(ahora).ok().flatten();
            t.estado = Some(c.estado.clone());
            t.sesion = c.sesion;
            t.ultimo = c.resultado.respuesta.as_ref().or(c.resultado.error.as_ref()).map(|s| s.chars().take(ULTIMO_MAX).collect());
        }
    }
}

fn base36(mut n: u64) -> String {
    const D: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let mut s = Vec::new();
    loop {
        s.push(D[(n % 36) as usize]);
        n /= 36;
        if n == 0 {
            break;
        }
    }
    s.reverse();
    String::from_utf8(s).unwrap_or_default()
}

/// Lo que dejó una corrida.
#[derive(Debug, Clone)]
pub struct Corrida {
    pub resultado: crate::flujo::Resultado,
    /// en palabras, para el registro y el aviso
    pub estado: String,
    pub sesion: Option<i64>,
}

impl Corrida {
    pub fn codigo(&self) -> i32 {
        self.resultado.codigo()
    }

    /// Una corrida que no llegó a empezar (sin motor, por ejemplo).
    pub fn fallida(e: &anyhow::Error) -> Corrida {
        let r = crate::flujo::Resultado { estado: crate::flujo::Estado::Fallo, respuesta: None, error: Some(format!("{e:#}")), negados: vec![], segundos: 0.0 };
        Corrida { estado: crate::tr!("fallo: {e:#}", "failure: {e:#}"), resultado: r, sesion: None }
    }
}

/// Los permisos de una corrida sin ventana: lo de `cfg` (nunca más laxo que `default`) + sus alcances.
pub fn permisos_de(t: &Tarea, cfg: &crate::Config) -> crate::Permissions {
    use crate::permissions::{Alcance, Mode, Vigencia};
    let modo = match Mode::parse(&cfg.mode) {
        Mode::Plan => Mode::Plan,
        _ => Mode::Default,
    };
    let mut p = crate::Permissions::new(modo, &t.proyecto, cfg.allow.clone(), cfg.deny.clone(), cfg.ask.clone());
    p.sandbox = cfg.sandbox;
    p.net_extra = cfg.net_allow.clone();
    p.sockets_extra = cfg.sockets_allow.clone();
    p.credenciales_extra = cfg.credenciales_extra.clone();
    p.entorno_extra = cfg.entorno_allow.clone();
    // nada de control del Mac ni cambios de sistema sin nadie mirando
    p.desktop_control = false;
    p.desktop_turn = false;
    p.system_changes = false;
    // vigencia de SESIÓN: cada corrida es un agente nuevo, y `run` olvida lo «de la orden» al empezar
    // (medido: con `Orden` la regla se borraba antes de usarse y la orden pedía permiso)
    for r in &t.permisos {
        if let Some(a) = Alcance::parse(r) {
            p.conceder(a, Vigencia::Sesion);
        }
    }
    p
}

/// Corre la tarea con un agente YA armado (la CLI lo arma con su motor; las pruebas, con el de guion) cuya
/// cara es [`crate::flujo::SinVentana`] con `detener`. Prepara permisos, sesión y corte; devuelve la corrida.
pub async fn correr_con<E: gotv_engine::Engine>(a: &mut crate::Agent<E, crate::flujo::SinVentana>, t: &Tarea, cfg: &crate::Config, detener: std::sync::Arc<std::sync::atomic::AtomicBool>) -> Corrida {
    use crate::flujo::Estado;
    a.perms = permisos_de(t, cfg);
    a.cancel = detener;
    a.computer_use = false;
    if a.session.is_none() {
        a.session = crate::session::Session::open_new(&t.proyecto, if t.modelo.is_empty() { "auto" } else { &t.modelo }).ok();
    }
    let inicio = std::time::Instant::now();
    let r = a.run(&t.orden_marcada()).await;
    let resultado = a.front.resultado(&r, inicio.elapsed().as_secs_f32());
    let estado = match resultado.estado {
        Estado::Hecho => "hecho".to_string(), // juez-idioma: datos (estado guardado que se compara)
        Estado::FaltaPermiso => crate::tr!(
            "detenida: falta permiso para «{}» (concédelo con una regla en `permisos` y vuelve a correrla)",
            "stopped: missing permission for «{}» (grant it with a rule in `permisos` and run it again)",
            resultado.negados.first().map(|n| n.chars().take(200).collect::<String>()).unwrap_or_default()
        ),
        Estado::Interrumpido => crate::i18n::t("interrumpida", "interrupted").to_string(),
        Estado::Fallo => crate::tr!("fallo: {}", "failure: {}", resultado.error.as_deref().unwrap_or("?").chars().take(300).collect::<String>()),
    };
    Corrida { resultado, estado, sesion: a.session.as_ref().map(|s| s.id) }
}

/// Avisa del final de la corrida (terminada o detenida).
pub fn avisar_corrida(t: &Tarea, c: &Corrida, n: &dyn crate::avisos::Notificador, i: &crate::avisos::Interruptores) {
    let (ev, cuerpo) = if c.codigo() == 0 {
        (crate::avisos::Evento::ProgramadaTerminada, crate::tr!("«{}» terminó", "«{}» finished", t.nombre))
    } else {
        (crate::avisos::Evento::ProgramadaDetenida, crate::tr!("«{}» se detuvo — {}", "«{}» stopped — {}", t.nombre, c.estado))
    };
    crate::avisos::avisar(n, ev, false, i, crate::i18n::t("GOTV · tarea programada", "GOTV · scheduled task"), &cuerpo);
}

/// Una vuelta de `pendientes` para UNA tarea vencida: candado, re-comprobar, correr (con lo que `correr`
/// haga), marcar y avisar. `Ok(None)` = otra corrida la tiene o ya no tocaba. `forzar` = correrla ya aunque
/// no toque (`gotv programadas correr <id>`; el candado vale igual).
pub async fn una_pendiente<Fut>(registro: &Path, id: &str, ahora: i64, forzar: bool, correr: impl FnOnce(Tarea) -> Fut, n: &dyn crate::avisos::Notificador, i: &crate::avisos::Interruptores) -> Result<Option<Corrida>>
where
    Fut: std::future::Future<Output = Corrida>,
{
    let Some(_candado) = Candado::intentar(&carpeta_de(registro).join(format!("{id}.lock")))? else {
        return Ok(None);
    };
    // con el candado, ¿sigue tocando? (otra corrida pudo terminarla justo antes)
    let reg = Registro::cargar(registro)?;
    let Some(t) = reg.buscar(id).cloned().filter(|t| forzar || t.activa && t.proxima.is_some_and(|p| p <= ahora)) else {
        return Ok(None);
    };
    let c = correr(t.clone()).await;
    let fin = crate::programar::chat::ahora().max(ahora);
    Registro::con(registro, |r| {
        r.marcar(id, fin, &c);
        Ok(())
    })?;
    avisar_corrida(&t, &c, n, i);
    Ok(Some(c))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flujo::tests::{agente, llamadas, texto};
    use crate::flujo::SinVentana;
    use crate::permissions::Mode;
    use serde_json::json;
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;

    fn nueva(proyecto: &Path, cron: &str, orden: &str, permisos: &[&str]) -> Nueva {
        Nueva { nombre: "contar TODO".into(), cron: cron.into(), zona: "America/Bogota".into(), proyecto: proyecto.to_path_buf(), orden: orden.into(), modelo: String::new(), permisos: permisos.iter().map(|s| s.to_string()).collect() }
    }

    fn cfg() -> crate::Config {
        crate::Config { mode: "bypass".into(), ..Default::default() }
    }

    fn hecha() -> Corrida {
        let r = crate::flujo::Resultado { estado: crate::flujo::Estado::Hecho, respuesta: Some("ok".into()), error: None, negados: vec![], segundos: 0.0 };
        Corrida { resultado: r, estado: "hecho".into(), sesion: Some(3) }
    }

    #[test]
    fn registro_crear_validar_y_atrasada_corre_una_vez() {
        let d = crate::CarpetaPrueba::nueva("gotv-programadas-registro");
        let ruta = d.join("config/programadas.bin");
        let t0 = chrono::DateTime::parse_from_rfc3339("2026-11-16T10:02:00-05:00").unwrap().timestamp();
        let t = Registro::con(&ruta, |r| r.crear(nueva(&d, "*/5 * * * *", "cuenta los TODO", &["bash(grep -rn TODO)"]), t0)).unwrap();
        assert_eq!(t.proxima, Some(t0 + 180), "10:05");
        assert!(Registro::con(&ruta, |r| r.crear(nueva(&d, "*/5 * * *", "x", &[]), t0)).is_err(), "cron de 4 campos");
        assert!(Registro::con(&ruta, |r| r.crear(nueva(&d, "*/5 * * * *", "x", &["bash(*)"]), t0)).is_err(), "«todo» no es un alcance");
        assert!(Registro::con(&ruta, |r| r.crear(nueva(&d.join("no-existe"), "*/5 * * * *", "x", &[]), t0)).is_err());
        // ida y vuelta por el archivo binario: la misma tarea, campo por campo
        let r = Registro::cargar(&ruta).unwrap();
        assert_eq!(r.tareas, [t.clone()]);
        assert!(r.pendientes(t0 + 179).is_empty());
        assert_eq!(r.pendientes(t0 + 180), [t.id.clone()]);
        // la Mac durmió 2 horas (24 vencimientos perdidos): UNA corrida, y la próxima desde ahora
        let despierta = t0 + 2 * 3600 + 30;
        let mut r = Registro::cargar(&ruta).unwrap();
        assert_eq!(r.pendientes(despierta).len(), 1);
        r.marcar(&t.id, despierta, &hecha());
        assert!(r.pendientes(despierta).is_empty());
        assert_eq!(r.buscar(&t.id).unwrap().proxima, Some(t0 + 2 * 3600 + 180), "12:05, no 10:10");
        assert_eq!(r.buscar(&t.id).unwrap().ultimo.as_deref(), Some("ok"));
        // desactivada no toca
        r.activar(&t.id, false, despierta).unwrap();
        assert!(r.pendientes(despierta + 10_000).is_empty());
        r.guardar().unwrap();
        assert_eq!(Registro::cargar(&ruta).unwrap().tareas, r.tareas);
        // un registro dañado no se pisa
        std::fs::write(&ruta, b"GOTVTARE\x01roto").unwrap();
        assert!(Registro::con(&ruta, |r| r.crear(nueva(&d, "* * * * *", "x", &[]), t0)).is_err());
        assert_eq!(std::fs::read(&ruta).unwrap(), b"GOTVTARE\x01roto");
    }

    #[test]
    fn candado_dos_corridas_a_la_vez_corre_una() {
        let d = crate::CarpetaPrueba::nueva("gotv-programadas-candado");
        let r = d.join("t1.lock");
        let a = Candado::intentar(&r).unwrap();
        assert!(a.is_some());
        // otro descriptor (como otro proceso: flock es por apertura)
        assert!(Candado::intentar(&r).unwrap().is_none(), "la segunda no corre");
        drop(a);
        assert!(Candado::intentar(&r).unwrap().is_some(), "soltado, vuelve a poderse");
    }

    async fn corrida_con_guion(t: Tarea, respuestas: Vec<gotv_engine::ChatResponse>) -> Corrida {
        let detener = Arc::new(AtomicBool::new(false));
        let mut a = agente(&t.proyecto, Mode::Bypass, respuestas, SinVentana::new(None, Some(detener.clone())));
        correr_con(&mut a, &t, &cfg(), detener).await
    }

    /// «Hecho cuando» de F2.3 (con el motor de guion): una tarea «cuenta los TODO» corre 3 veces y deja 3
    /// chats; una que intenta `git push` sin permiso se detiene, deja su motivo y avisa. El modo `bypass` de
    /// la config NO se hereda.
    #[tokio::test]
    async fn tres_corridas_dejan_tres_chats_y_git_push_sin_permiso_se_detiene() {
        let d = crate::CarpetaPrueba::nueva("gotv-programadas-correr");
        let proyecto = d.join("proyecto");
        std::fs::create_dir_all(&proyecto).unwrap();
        std::fs::write(proyecto.join("a.rs"), "// TODO uno\n// TODO dos\nfn main() {}\n").unwrap();
        let ruta = d.join("config/programadas.bin");
        let t0 = chrono::DateTime::parse_from_rfc3339("2026-11-16T10:02:00-05:00").unwrap().timestamp();
        let t = Registro::con(&ruta, |r| r.crear(nueva(&proyecto, "*/5 * * * *", "cuenta los TODO del proyecto", &["leer(a.rs)"]), t0)).unwrap();
        let falso = crate::avisos::Falso::default();
        let todo = crate::avisos::Interruptores::default();
        for vuelta in 0..3 {
            let ahora = Registro::cargar(&ruta).unwrap().buscar(&t.id).unwrap().proxima.unwrap();
            assert_eq!(Registro::cargar(&ruta).unwrap().pendientes(ahora), [t.id.clone()], "vuelta {vuelta}");
            // el `grep` de GOTV (Rust, en proceso), no un programa de fuera
            let c = una_pendiente(&ruta, &t.id, ahora, false, |t| corrida_con_guion(t, vec![llamadas(&[("grep", json!({"pattern": "TODO", "path": "a.rs"}))]), texto("Hay 2 TODO.")]), &falso, &todo).await.unwrap().unwrap();
            assert_eq!((c.codigo(), c.estado.as_str()), (0, "hecho"), "{c:?}");
            assert_eq!(c.resultado.respuesta.as_deref(), Some("Hay 2 TODO."));
        }
        let chats = crate::session::Session::list(&proyecto).unwrap();
        assert_eq!(chats.len(), 3, "3 corridas, 3 chats");
        assert!(chats.iter().all(|c| c.title.starts_with("⏰ [programada · contar TODO]") || c.title.starts_with("⏰ [scheduled · contar TODO]")), "{:?}", chats.iter().map(|c| &c.title).collect::<Vec<_>>());
        assert_eq!(falso.enviados.lock().unwrap().len(), 3);
        assert_eq!(Registro::cargar(&ruta).unwrap().buscar(&t.id).unwrap().ultimo.as_deref(), Some("Hay 2 TODO."));

        // git push sin permiso: se detiene, deja su motivo y avisa
        let tp = Registro::con(&ruta, |r| r.crear(Nueva { nombre: "publicar".into(), ..nueva(&proyecto, "0 * * * *", "sube los cambios", &["leer(a.rs)"]) }, t0)).unwrap();
        let cuando = tp.proxima.unwrap();
        let c = una_pendiente(&ruta, &tp.id, cuando, false, |t| corrida_con_guion(t, vec![llamadas(&[("bash", json!({"cmd": "git push origin main"}))]), texto("no debería llegar")]), &falso, &todo).await.unwrap().unwrap();
        assert_eq!(c.codigo(), 2, "{c:?}");
        let guardada = Registro::cargar(&ruta).unwrap().buscar(&tp.id).cloned().unwrap();
        assert!(guardada.estado.as_deref().unwrap().contains("git push"), "{guardada:?}");
        let (titulo, cuerpo) = falso.enviados.lock().unwrap().last().cloned().unwrap();
        assert!(titulo.contains("GOTV") && cuerpo.contains("publicar") && cuerpo.contains("git push"), "{cuerpo}");
        // y quedó dónde se detuvo: el chat de esa corrida
        assert_eq!(guardada.sesion, c.sesion);
        assert!(c.sesion.is_some());
        // con el interruptor apagado, no se avisa
        let mut sin = todo;
        sin.programadas = false;
        let antes = falso.enviados.lock().unwrap().len();
        una_pendiente(&ruta, &tp.id, 0, true, |_| async { hecha() }, &falso, &sin).await.unwrap().unwrap();
        assert_eq!(falso.enviados.lock().unwrap().len(), antes);
    }

    #[tokio::test]
    async fn una_pendiente_con_el_candado_tomado_no_corre() {
        let d = crate::CarpetaPrueba::nueva("gotv-programadas-ocupada");
        let ruta = d.join("programadas.bin");
        let t = Registro::con(&ruta, |r| r.crear(nueva(&d, "* * * * *", "x", &[]), 0)).unwrap();
        let todo = crate::avisos::Interruptores::default();
        let falso = crate::avisos::Falso::default();
        let otra = Candado::intentar(&carpeta_de(&ruta).join(format!("{}.lock", t.id))).unwrap().unwrap();
        let corrio = std::sync::atomic::AtomicBool::new(false);
        let r = una_pendiente(&ruta, &t.id, t.proxima.unwrap(), true, |_| async {
            corrio.store(true, std::sync::atomic::Ordering::Relaxed);
            hecha()
        }, &falso, &todo)
        .await
        .unwrap();
        assert!(r.is_none() && !corrio.load(std::sync::atomic::Ordering::Relaxed), "ni forzada corre con el candado tomado");
        // sin candado y sin tocar: no corre; forzada, sí
        drop(otra);
        assert!(una_pendiente(&ruta, &t.id, 0, false, |_| async { hecha() }, &falso, &todo).await.unwrap().is_none());
        assert!(una_pendiente(&ruta, &t.id, 0, true, |_| async { hecha() }, &falso, &todo).await.unwrap().is_some());
    }
}
