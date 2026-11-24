//! F2.2 · `programar` dentro del chat: «avísame en 2 minutos si el servidor sigue arriba».
//!
//! - `crear` (`en_segundos` o `cron`, `orden`, `veces`), `listar`, `cancelar`. Tope: 50 por sesión; mínimo 60 s.
//! - Vive lo que vive la sesión: se guarda en `.gotv/programadas/sesion-<id>.bin` (formato propio,
//!   [`super::codec`]) y se restaura al reabrir el chat.
//! - Dispara como un turno NUEVO cuando el agente está libre ([`turno_pendiente`], que la ventana pregunta
//!   desde su bucle sólo cuando no hay una orden corriendo: con el agente ocupado, espera).
//! - El turno que dispara corre con los permisos de la sesión, como si lo hubieras escrito tú.
use anyhow::{anyhow, bail, Result};
use super::codec::{Escritor, Lector};
// los argumentos y la ficha de la herramienta son el protocolo del MODELO (no se guardan)
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

pub const MAX_POR_SESION: usize = 50;
pub const MIN_SEGUNDOS: u64 = 60;
/// Repeticiones como máximo de una programada del chat.
pub const MAX_VECES: u32 = 100;

#[derive(Clone, Debug, PartialEq)]
pub struct Programada {
    pub id: u32,
    pub orden: String,
    /// `en_segundos`: la primera a los N s y, si se repite, cada N s
    pub cada: Option<u64>,
    pub cron: Option<String>,
    pub zona: Option<String>,
    pub veces: u32,
    pub hechas: u32,
    /// segundos Unix
    pub proxima: i64,
}

#[derive(Default, Debug, PartialEq)]
pub struct Agenda {
    pub items: Vec<Programada>,
    pub siguiente: u32,
    /// la sesión de la que se cargó (`None` = sin sesión o aún sin cargar)
    pub sesion: Option<i64>,
    pub cargada: bool,
}

const TIPO: &[u8; 4] = b"AGEN";
const VERSION: u8 = 1;

impl Agenda {
    pub fn bytes(&self) -> Vec<u8> {
        let mut e = Escritor::nuevo(TIPO, VERSION);
        e.u32(self.siguiente);
        e.u32(self.items.len() as u32);
        for p in &self.items {
            e.u32(p.id);
            e.texto(&p.orden);
            e.opt_u64(p.cada);
            e.opt_texto(p.cron.as_deref());
            e.opt_texto(p.zona.as_deref());
            e.u32(p.veces);
            e.u32(p.hechas);
            e.i64(p.proxima);
        }
        e.bytes()
    }

    pub fn de_bytes(b: &[u8]) -> Result<Agenda> {
        let (mut l, _) = Lector::nuevo(b, TIPO, VERSION)?;
        let siguiente = l.u32()?;
        let n = l.u32()? as usize;
        let mut items = Vec::with_capacity(n.min(MAX_POR_SESION));
        for _ in 0..n {
            items.push(Programada { id: l.u32()?, orden: l.texto()?, cada: l.opt_u64()?, cron: l.opt_texto()?, zona: l.opt_texto()?, veces: l.u32()?, hechas: l.u32()?, proxima: l.i64()? });
        }
        Ok(Agenda { items, siguiente, sesion: None, cargada: false })
    }
}

/// Dónde vive la agenda de una sesión.
pub fn ruta_de(raiz: &std::path::Path, sesion: i64) -> std::path::PathBuf {
    raiz.join(".gotv/programadas").join(format!("sesion-{sesion}.bin"))
}

fn hora(t: i64) -> String {
    super::hora_local(Some(t))
}

impl Agenda {
    pub fn crear(&mut self, args: &Value, ahora: i64) -> Result<String> {
        let orden = args["orden"].as_str().map(str::trim).filter(|s| !s.is_empty()).ok_or_else(|| anyhow!("{}", crate::i18n::t("falta `orden`: lo que harás cuando toque, como si te lo escribieran", "missing `orden`: what you will do when it is time, as if someone wrote it to you")))?;
        if self.items.len() >= MAX_POR_SESION {
            bail!("{}", crate::tr!("ya hay {MAX_POR_SESION} programadas en esta sesión: cancela alguna antes", "there are already {MAX_POR_SESION} scheduled in this session: cancel one first"));
        }
        let veces = args["veces"].as_u64().unwrap_or(1).clamp(1, MAX_VECES as u64) as u32;
        let segundos = args["en_segundos"].as_u64().or_else(|| args["en_segundos"].as_str().and_then(|s| s.trim().parse().ok()));
        let cron = args["cron"].as_str().map(str::trim).filter(|s| !s.is_empty());
        let (cada, cron, zona, proxima) = match (segundos, cron) {
            (Some(_), Some(_)) => bail!("{}", crate::i18n::t("una de dos: `en_segundos` o `cron`", "one or the other: `en_segundos` or `cron`")),
            (None, None) => bail!("{}", crate::i18n::t("falta cuándo: `en_segundos` (mínimo 60) o `cron` (5 campos)", "missing when: `en_segundos` (at least 60) or `cron` (5 fields)")),
            (Some(s), None) => {
                if s < MIN_SEGUNDOS {
                    bail!("{}", crate::tr!("mínimo {MIN_SEGUNDOS} s (pediste {s})", "at least {MIN_SEGUNDOS} s (you asked for {s})"));
                }
                (Some(s), None, None, ahora + s as i64)
            }
            (None, Some(c)) => {
                let z = args["zona"].as_str().unwrap_or("").to_string();
                let tz = super::zona(&z)?;
                let h = super::horario(c)?;
                let p = super::proxima(&h, tz, ahora).ok_or_else(|| anyhow!("{}", crate::tr!("«{c}» no vuelve a ocurrir", "«{c}» never happens again")))?;
                (None, Some(c.to_string()), (!z.is_empty()).then_some(z), p)
            }
        };
        self.siguiente += 1;
        let id = self.siguiente;
        self.items.push(Programada { id, orden: orden.to_string(), cada, cron, zona, veces, hechas: 0, proxima });
        let rep = if veces > 1 { crate::tr!(", {veces} veces", ", {veces} times") } else { String::new() };
        Ok(crate::tr!(
            "⏰ programada #{id} para las {} (en {} s{rep}): «{orden}». Llegará como una orden nueva cuando estés libre. Cancélala con action=cancelar.",
            "⏰ scheduled #{id} for {} (in {} s{rep}): «{orden}». It will arrive as a new order when you are idle. Cancel it with action=cancelar.",
            hora(proxima),
            proxima - ahora
        ))
    }

    pub fn cancelar(&mut self, id: u32) -> Result<String> {
        let n = self.items.len();
        self.items.retain(|p| p.id != id);
        if self.items.len() == n {
            bail!("{}", crate::tr!("no existe la programada #{id} (mira action=listar)", "scheduled #{id} does not exist (see action=listar)"));
        }
        Ok(crate::tr!("programada #{id} cancelada", "scheduled #{id} cancelled"))
    }

    pub fn listar(&self, ahora: i64) -> String {
        if self.items.is_empty() {
            return crate::i18n::t("(no hay nada programado en esta sesión)", "(nothing scheduled in this session)").into();
        }
        self.items
            .iter()
            .map(|p| {
                let cuando = p.cron.as_ref().map(|c| format!("cron «{c}»")).unwrap_or_else(|| crate::tr!("cada {} s", "every {} s", p.cada.unwrap_or(0)));
                crate::tr!(
                    "#{} {} · próxima {} (en {} s) · {}/{} · «{}»",
                    "#{} {} · next {} (in {} s) · {}/{} · «{}»",
                    p.id,
                    cuando,
                    hora(p.proxima),
                    (p.proxima - ahora).max(0),
                    p.hechas,
                    p.veces,
                    p.orden
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// La primera vencida, si el agente está libre: cuenta la vez, calcula la próxima (desde AHORA: lo que se
    /// atrasó corre una sola vez) o la quita si ya hizo sus `veces`. Devuelve la orden con su marca.
    pub fn tomar_vencida(&mut self, ahora: i64, ocupado: bool) -> Option<String> {
        if ocupado {
            return None;
        }
        let i = self.items.iter().enumerate().filter(|(_, p)| p.proxima <= ahora).min_by_key(|(_, p)| p.proxima).map(|(i, _)| i)?;
        let p = &mut self.items[i];
        p.hechas += 1;
        let texto = if p.veces > 1 {
            crate::tr!("⏰ [Programada #{} · {}/{}] {}", "⏰ [Scheduled #{} · {}/{}] {}", p.id, p.hechas, p.veces, p.orden)
        } else {
            crate::tr!("⏰ [Programada #{}] {}", "⏰ [Scheduled #{}] {}", p.id, p.orden)
        };
        let siguiente = if p.hechas >= p.veces {
            None
        } else if let Some(s) = p.cada {
            Some(ahora + s as i64)
        } else {
            p.cron.as_deref().and_then(|c| super::horario(c).ok()).zip(super::zona(p.zona.as_deref().unwrap_or("")).ok()).and_then(|(h, tz)| super::proxima(&h, tz, ahora))
        };
        match siguiente {
            Some(t) => p.proxima = t,
            None => {
                self.items.remove(i);
            }
        }
        Some(texto)
    }
}

/// Los modos en que nadie se queda esperando (`gotv -p`, una tarea programada): lo programado en el chat no
/// correría NUNCA, así que `crear` se niega y dice por qué (medido 2026-11-17: la herramienta se ofrecía en
/// todos los modos y sólo la ventana la disparaba).
static SIN_ESPERA: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Lo llama el modo que no espera, al arrancar (vale para todo el proceso).
pub fn sin_espera() {
    SIN_ESPERA.store(true, std::sync::atomic::Ordering::Relaxed);
}

/// ¿Puede dispararse lo programado en este proceso? (la ventana o la CLI que espera; no `-p` ni una tarea)
pub fn espera_posible() -> bool {
    !SIN_ESPERA.load(std::sync::atomic::Ordering::Relaxed)
}

/// Agendas por chat (la clave del dueño de sus procesos).
fn agendas() -> &'static Mutex<HashMap<String, Agenda>> {
    static A: OnceLock<Mutex<HashMap<String, Agenda>>> = OnceLock::new();
    A.get_or_init(Default::default)
}

pub fn ahora() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// Que la agenda del chat sea la de su sesión ACTUAL (al reabrir un chat, se restaura de su archivo).
fn asegurar<E: gotv_engine::Engine, F: crate::Frontend>(a: &crate::Agent<E, F>) -> String {
    let clave = a.proc_owner();
    let sesion = a.session.as_ref().map(|s| s.id);
    let mut m = agendas().lock().unwrap();
    let ag = m.entry(clave.clone()).or_default();
    if !ag.cargada || ag.sesion != sesion {
        let guardada = sesion.and_then(|s| std::fs::read(ruta_de(&a.root, s)).ok()).and_then(|b| Agenda::de_bytes(&b).ok()).unwrap_or_default();
        *ag = guardada;
        ag.sesion = sesion;
        ag.cargada = true;
    }
    clave
}

/// Guarda la agenda de la sesión (sin sesión, vive sólo en memoria).
fn persistir<E: gotv_engine::Engine, F: crate::Frontend>(a: &crate::Agent<E, F>, clave: &str) {
    let Some(s) = &a.session else { return };
    let bytes = agendas().lock().unwrap().get(clave).map(Agenda::bytes);
    if let Some(b) = bytes {
        let _ = super::codec::guardar(&ruta_de(&a.root, s.id), &b);
    }
}

/// La herramienta `programar` (la despacha el agente: necesita su sesión).
pub fn herramienta<E: gotv_engine::Engine, F: crate::Frontend>(a: &crate::Agent<E, F>, args: &Value) -> crate::tools::ToolResult {
    herramienta_en(a, args, ahora())
}

pub(crate) fn herramienta_en<E: gotv_engine::Engine, F: crate::Frontend>(a: &crate::Agent<E, F>, args: &Value, t: i64) -> crate::tools::ToolResult {
    let clave = asegurar(a);
    let accion = args["action"].as_str().unwrap_or(if args["orden"].is_string() { "crear" } else { "listar" });
    if matches!(accion, "crear" | "create") && SIN_ESPERA.load(std::sync::atomic::Ordering::Relaxed) {
        return crate::tools::ToolResult {
            ok: false,
            output: crate::i18n::t(
                "ERROR: en este modo (una orden sin ventana: `gotv -p` o una tarea programada) nadie se queda esperando: lo programado aquí no correría nunca. Haz ahora lo que se pueda y dile al usuario que, para algo que deba correr más tarde sin la ventana, use `gotv programadas crear`.",
                "ERROR: in this mode (an order without a window: `gotv -p` or a scheduled task) nobody stays waiting: what is scheduled here would never run. Do now what can be done and tell the user that, for something that must run later without the window, they should use `gotv programadas crear`.",
            )
            .into(),
        };
    }
    let r = {
        let mut m = agendas().lock().unwrap();
        let ag = m.entry(clave.clone()).or_default();
        match accion {
            "crear" | "create" => ag.crear(args, t),
            "listar" | "list" => Ok(ag.listar(t)),
            "cancelar" | "cancel" => match args["id"].as_u64().or_else(|| args["id"].as_str().and_then(|s| s.trim_start_matches('#').parse().ok())) {
                Some(id) => ag.cancelar(id as u32),
                None => Err(anyhow!("{}", crate::i18n::t("falta 'id' (el número de la programada)", "missing 'id' (the scheduled number)"))),
            },
            otra => Err(anyhow!("{}", crate::tr!("acción desconocida «{otra}»: crear, listar o cancelar", "unknown action «{otra}»: crear, listar or cancelar"))),
        }
    };
    if r.is_ok() && accion != "listar" && accion != "list" {
        persistir(a, &clave);
    }
    match r {
        Ok(output) => crate::tools::ToolResult { ok: true, output },
        Err(e) => crate::tools::ToolResult { ok: false, output: format!("ERROR: {e}") },
    }
}

/// ¿Qué orden toca ahora, con el agente LIBRE? Lo programado vencido primero; si no, lo que vieron las
/// vigilancias del chat (F3). La ventana lo pregunta desde su bucle cuando no hay orden corriendo.
pub fn turno_pendiente<E: gotv_engine::Engine, F: crate::Frontend>(a: &mut crate::Agent<E, F>, t: i64) -> Option<String> {
    a.sincronizar_estado();
    let clave = asegurar(a);
    let vencida = agendas().lock().unwrap().get_mut(&clave).and_then(|ag| ag.tomar_vencida(t, false));
    if let Some(orden) = vencida {
        persistir(a, &clave);
        return Some(orden);
    }
    crate::vigilar::tomar(&clave)
}

/// Lo programado del chat (para la ventana: verlo y cancelarlo en Tareas).
pub fn lista<E: gotv_engine::Engine, F: crate::Frontend>(a: &crate::Agent<E, F>) -> Vec<Programada> {
    let clave = asegurar(a);
    agendas().lock().unwrap().get(&clave).map(|ag| ag.items.clone()).unwrap_or_default()
}

/// La ficha de la herramienta (va en `agent.rs::all_tool_schemas`; nace diferida).
pub fn esquema() -> Value {
    use serde_json::json;
    let s = |d: &str| json!({ "type": "string", "description": d });
    let i = |d: &str| json!({ "type": "integer", "description": d });
    json!({
        "type": "function",
        "function": {
            "name": "programar",
            "description": crate::i18n::t(
                "Programa una orden para MÁS TARDE en este chat («avísame en 2 minutos si…», «cada lunes a las 9…»): cuando toque te llega sola como una orden nueva. action: crear (orden + en_segundos (mín. 60) o cron de 5 campos con zona opcional; veces, 1 por omisión) · listar · cancelar (id). Hasta 50 por sesión; vive lo que vive la sesión.",
                "Schedules an order for LATER in this chat («tell me in 2 minutes if…», «every Monday at 9…»): when it is time it reaches you as a new order. action: crear (orden + en_segundos (min 60) or a 5-field cron with optional zona; veces, 1 by default) · listar · cancelar (id). Up to 50 per session; lives as long as the session.",
            ),
            "parameters": {
                "type": "object",
                "properties": {
                    "action": { "type": "string", "enum": ["crear", "listar", "cancelar"] },
                    "orden": s(crate::i18n::t("lo que harás cuando toque, como si te lo escribieran", "what you will do when it is time, as if someone wrote it to you")),
                    "en_segundos": i(crate::i18n::t("dentro de cuántos segundos (mínimo 60); con veces > 1, cada cuánto", "in how many seconds (at least 60); with veces > 1, how often")),
                    "cron": s(crate::i18n::t("cron de 5 campos: minuto hora día mes día-de-semana (0 = domingo)", "5-field cron: minute hour day month weekday (0 = Sunday)")),
                    "zona": s(crate::i18n::t("zona horaria IANA (vacío = la de esta Mac)", "IANA time zone (empty = this Mac's)")),
                    "veces": i(crate::i18n::t("cuántas veces (1; máx. 100)", "how many times (1; max 100)")),
                    "id": i(crate::i18n::t("con cancelar: el número", "with cancelar: the number")),
                },
                "required": ["action"],
            },
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flujo::tests::{agente, llamadas, texto};
    use crate::permissions::Mode;
    use serde_json::json;

    #[derive(Default)]
    struct Mudo;
    #[async_trait::async_trait]
    impl crate::Frontend for Mudo {
        async fn log(&self, _: String) {}
        async fn ask(&self, _: &str) -> crate::AskAnswer {
            crate::AskAnswer::Yes
        }
    }

    #[test]
    fn la_agenda_dispara_a_su_hora_respeta_veces_y_espera_si_esta_ocupado() {
        let mut ag = Agenda::default();
        let t0 = 1_000_000;
        assert!(ag.crear(&json!({"orden": "x", "en_segundos": 59}), t0).unwrap_err().to_string().contains("60"));
        assert!(ag.crear(&json!({"orden": "x"}), t0).is_err());
        assert!(ag.crear(&json!({"orden": "x", "en_segundos": 60, "cron": "* * * * *"}), t0).is_err());
        ag.crear(&json!({"orden": "mira el servidor", "en_segundos": 120, "veces": 3}), t0).unwrap();
        assert_eq!(ag.tomar_vencida(t0 + 119, false), None, "aún no");
        assert_eq!(ag.tomar_vencida(t0 + 120, true), None, "ocupado: espera");
        let o = ag.tomar_vencida(t0 + 125, false).unwrap();
        assert!(o.contains("1/3") && o.contains("mira el servidor"), "{o}");
        assert_eq!(ag.tomar_vencida(t0 + 125, false), None, "una sola vez por vencimiento");
        // la próxima cuenta desde que corrió (t0+125), no desde la hora prevista
        assert_eq!(ag.items[0].proxima, t0 + 245);
        // atrasada tras dormir 1 h: corre UNA vez, no las que se perdió
        let o = ag.tomar_vencida(t0 + 3_600, false).unwrap();
        assert!(o.contains("2/3"), "{o}");
        assert_eq!(ag.tomar_vencida(t0 + 3_600, false), None);
        assert!(ag.tomar_vencida(t0 + 3_720, false).unwrap().contains("3/3"));
        assert!(ag.items.is_empty(), "hechas sus veces, se quita");
        // tope por sesión
        for _ in 0..MAX_POR_SESION {
            ag.crear(&json!({"orden": "y", "en_segundos": 60}), t0).unwrap();
        }
        assert!(ag.crear(&json!({"orden": "y", "en_segundos": 60}), t0).unwrap_err().to_string().contains("50"));
        let id = ag.items[3].id;
        ag.cancelar(id).unwrap();
        assert!(ag.cancelar(id).is_err());
        assert_eq!(ag.items.len(), MAX_POR_SESION - 1);
    }

    #[test]
    fn con_cron_y_zona() {
        let mut ag = Agenda::default();
        let lunes_8 = chrono::DateTime::parse_from_rfc3339("2026-11-16T08:00:00-05:00").unwrap().timestamp();
        ag.crear(&json!({"orden": "resumen del día", "cron": "0 9 * * 1-5", "zona": "America/Bogota", "veces": 2}), lunes_8).unwrap();
        assert_eq!(ag.items[0].proxima, lunes_8 + 3_600);
        assert!(ag.tomar_vencida(lunes_8 + 3_600, false).is_some());
        assert_eq!(ag.items[0].proxima, lunes_8 + 3_600 + 86_400, "el martes a las 9");
    }

    /// «Hecho cuando» de F2.2: «avísame en 2 minutos si el servidor sigue arriba» dispara UNA vez y
    /// responde; y la agenda sobrevive a reabrir el chat (otra instancia del agente, la misma sesión).
    #[tokio::test]
    async fn avisame_en_2_minutos_dispara_una_vez_y_se_restaura_al_reabrir() {
        let raiz = crate::CarpetaPrueba::nueva("gotv-programar-chat");
        let t0 = ahora();
        let mut a = agente(
            &raiz,
            Mode::Default,
            vec![
                llamadas(&[("programar", json!({"action": "crear", "en_segundos": 120, "orden": "comprueba si el servidor sigue arriba y avísame"}))]),
                texto("Listo: te aviso en 2 minutos."),
            ],
            Mudo,
        );
        a.session = Some(crate::session::Session::open_new(&raiz, "guion").unwrap());
        // la herramienta nace diferida: aquí ya está cargada (como tras `tool_search`)
        a.tools_loaded.insert("programar".into());
        a.run("avísame en 2 minutos si el servidor sigue arriba").await.unwrap();
        assert!(a.history.iter().any(|m| m.role == "tool" && m.content.contains("programada #1")), "{:?}", a.history.iter().map(|m| &m.content).collect::<Vec<_>>());
        let sid = a.session.as_ref().unwrap().id;
        // reabrir el chat: otro agente, la misma sesión (lo que hace la ventana al volver a él)
        drop(a);
        let mut b = agente(&raiz, Mode::Default, vec![texto("Sí, el servidor sigue arriba (puerto 3000 responde).")], Mudo);
        b.session = crate::session::Session::resume_last(&raiz, None).unwrap().map(|(s, _)| s);
        assert_eq!(b.session.as_ref().unwrap().id, sid);
        assert_eq!(turno_pendiente(&mut b, t0 + 100), None, "aún no son 2 minutos");
        let orden = turno_pendiente(&mut b, t0 + 125).expect("a los 2 minutos dispara");
        assert!(orden.contains("comprueba si el servidor sigue arriba"), "{orden}");
        let r = b.run(&orden).await.unwrap();
        assert!(r.contains("sigue arriba"), "{r}");
        assert_eq!(turno_pendiente(&mut b, t0 + 600), None, "una sola vez");
        assert!(lista(&b).is_empty());
        // y quedó en SU archivo binario (ida y vuelta), no en el estado de la sesión
        let guardada = Agenda::de_bytes(&std::fs::read(ruta_de(&raiz, sid)).unwrap()).unwrap();
        assert!(guardada.items.is_empty() && guardada.siguiente == 1, "{guardada:?}");
        assert!(b.session.as_ref().unwrap().load_state().tools_loaded.contains(&"programar".to_string()), "el estado de la sesión sigue siendo suyo");
    }

    #[test]
    fn la_agenda_va_y_vuelve_por_su_formato() {
        let mut ag = Agenda::default();
        ag.crear(&json!({"orden": "ñ ⏰ uno", "en_segundos": 90, "veces": 4}), 1_000).unwrap();
        ag.crear(&json!({"orden": "dos", "cron": "*/5 * * * *", "zona": "America/Bogota"}), 1_000).unwrap();
        ag.items[0].hechas = 2;
        let de_vuelta = Agenda::de_bytes(&ag.bytes()).unwrap();
        assert_eq!((de_vuelta.items.clone(), de_vuelta.siguiente), (ag.items.clone(), ag.siguiente));
        assert!(Agenda::de_bytes(&ag.bytes()[..20]).is_err());
    }
}
