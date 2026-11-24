//! F2 · Automatizaciones: que GOTV trabaje sin que estés delante, y te avise.
//!
//! - [`chat`] (F2.2): `programar` dentro del chat («avísame en 2 minutos…»): vive lo que vive la sesión.
//! - [`tareas`] (F2.3): tareas programadas del proyecto con `cron` y zona horaria, que corren SIN ventana y
//!   SÓLO con los permisos que se les concedieron (reglas con alcance de F14.4).
//! - [`launchd`] (F2.3 paso 5): el `LaunchAgent` que las corre con la app cerrada (generarlo; instalarlo
//!   pregunta SIEMPRE).
//!
//! Este archivo: el cálculo de la próxima hora de un `cron` de 5 campos en una zona horaria.
use anyhow::{anyhow, bail, Result};
use chrono::{NaiveDateTime, TimeZone, Utc};
use std::str::FromStr;

pub mod chat;
pub mod codec;
pub mod launchd;
pub mod tareas;
pub mod ventana;

/// El día de la semana del cron de siempre (0–6, domingo = 0 o 7) en nombres, que el crate `cron` lee
/// sin ambigüedad (él numera 1–7 con domingo = 1: «1-5» habría sido domingo a jueves).
fn dias_en_nombres(campo: &str) -> Result<String> {
    const NOMBRES: [&str; 7] = ["SUN", "MON", "TUE", "WED", "THU", "FRI", "SAT"]; // juez-idioma: datos (nombres que lee el crate cron)
    if campo == "*" || campo == "?" {
        return Ok(campo.to_string());
    }
    let num = |s: &str| -> Result<usize> {
        let n: usize = s.parse().map_err(|_| anyhow!("{}", crate::tr!("día de la semana inválido «{s}» (0–7 o SUN–SAT)", "invalid day of week «{s}» (0–7 or SUN–SAT)")))?;
        if n > 7 {
            bail!("{}", crate::tr!("día de la semana fuera de rango «{n}» (0–7)", "day of week out of range «{n}» (0–7)"));
        }
        Ok(n % 7)
    };
    let mut dias = std::collections::BTreeSet::new();
    for parte in campo.split(',') {
        // con nombres (MON-FRI) se deja tal cual
        if parte.chars().any(|c| c.is_ascii_alphabetic()) {
            return Ok(campo.to_uppercase());
        }
        let (rango, paso) = match parte.split_once('/') {
            Some((r, p)) => (r, p.parse::<usize>().map_err(|_| anyhow!("{}", crate::tr!("paso inválido «{p}»", "invalid step «{p}»")))?.max(1)),
            None => (parte, 1),
        };
        let (a, b) = match rango {
            "*" => (0, 6),
            r => match r.split_once('-') {
                Some((a, b)) => {
                    let (a, b): (usize, usize) = (a.parse().map_err(|_| anyhow!("«{a}»"))?, b.parse().map_err(|_| anyhow!("«{b}»"))?);
                    if a > 7 || b > 7 || a > b {
                        bail!("{}", crate::tr!("rango de días inválido «{r}»", "invalid day range «{r}»"));
                    }
                    (a, b)
                }
                None => {
                    let n = num(r)?;
                    (n, if paso > 1 { 6 } else { n })
                }
            },
        };
        let mut d = a;
        while d <= b {
            dias.insert(d % 7);
            d += paso;
        }
    }
    Ok(dias.into_iter().map(|d| NOMBRES[d]).collect::<Vec<_>>().join(","))
}

/// Un `cron` de 5 campos (minuto hora día mes día-de-semana) → el horario del crate (con segundos).
pub fn horario(cron5: &str) -> Result<cron::Schedule> {
    let campos: Vec<&str> = cron5.split_whitespace().collect();
    if campos.len() != 5 {
        bail!("{}", crate::tr!(
            "«{cron5}» no es un cron de 5 campos (minuto hora día mes día-de-semana), p. ej. «*/5 * * * *» o «0 9 * * 1-5»",
            "«{cron5}» is not a 5-field cron (minute hour day month weekday), e.g. «*/5 * * * *» or «0 9 * * 1-5»"
        ));
    }
    let dow = dias_en_nombres(campos[4])?;
    let seis = format!("0 {} {} {} {} {dow}", campos[0], campos[1], campos[2], campos[3]);
    cron::Schedule::from_str(&seis).map_err(|e| anyhow!("{}", crate::tr!("cron inválido «{cron5}»: {e}", "invalid cron «{cron5}»: {e}")))
}

/// La zona: un nombre IANA (`America/Bogota`), o la de esta Mac si viene vacía.
pub fn zona(nombre: &str) -> Result<chrono_tz::Tz> {
    let n = if nombre.trim().is_empty() { zona_local() } else { nombre.trim().to_string() };
    n.parse::<chrono_tz::Tz>().map_err(|_| anyhow!("{}", crate::tr!("zona horaria desconocida «{n}» (usa un nombre IANA, p. ej. America/Bogota)", "unknown time zone «{n}» (use an IANA name, e.g. America/Bogota)")))
}

/// Un instante (segundos Unix) en la hora local de esta Mac, `AAAA-MM-DD HH:MM` (o «—»).
pub fn hora_local(t: Option<i64>) -> String {
    t.and_then(|t| chrono::Local.timestamp_opt(t, 0).single()).map(|d| d.format("%Y-%m-%d %H:%M").to_string()).unwrap_or_else(|| "—".into())
}

/// El nombre IANA de la zona de esta Mac (UTC si no se sabe).
pub fn zona_local() -> String {
    iana_time_zone::get_timezone().unwrap_or_else(|_| "UTC".into())
}

/// La próxima vez (segundos Unix) ESTRICTAMENTE después de `despues`, en la hora de pared de `tz`.
///
/// Se recorre el horario en hora de PARED y cada candidata se traduce a un instante real:
/// - hora que se repite (al atrasar el reloj): vale la primera; la segunda ya quedó atrás → corre UNA vez;
/// - hora que no existe (al adelantar el reloj): corre en el primer minuto que existe tras el hueco
///   (02:30 → 03:00, como el cron de Vixie), no se salta el día.
pub fn proxima(h: &cron::Schedule, tz: chrono_tz::Tz, despues: i64) -> Option<i64> {
    let desde = Utc.timestamp_opt(despues, 0).single()?.with_timezone(&tz).naive_local();
    // el horario recorrido sobre la hora de pared (como si fuera UTC: sin saltos)
    let mut it = h.after(&Utc.from_utc_datetime(&desde));
    for _ in 0..1000 {
        let pared: NaiveDateTime = it.next()?.naive_utc();
        let real = match tz.from_local_datetime(&pared) {
            chrono::LocalResult::Single(t) => t,
            chrono::LocalResult::Ambiguous(a, b) => a.min(b),
            chrono::LocalResult::None => {
                // dentro del hueco: el primer minuto que existe después
                let mut p = pared;
                loop {
                    p += chrono::Duration::minutes(1);
                    if let Some(t) = tz.from_local_datetime(&p).earliest() {
                        break t;
                    }
                }
            }
        };
        let s = real.timestamp();
        if s > despues {
            return Some(s);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unix(s: &str) -> i64 {
        chrono::DateTime::parse_from_rfc3339(s).unwrap().timestamp()
    }

    #[test]
    fn dias_de_la_semana_como_el_cron_de_siempre() {
        assert_eq!(dias_en_nombres("1-5").unwrap(), "MON,TUE,WED,THU,FRI");
        assert_eq!(dias_en_nombres("0").unwrap(), "SUN");
        assert_eq!(dias_en_nombres("7").unwrap(), "SUN");
        assert_eq!(dias_en_nombres("5-7").unwrap(), "SUN,FRI,SAT");
        assert_eq!(dias_en_nombres("*/2").unwrap(), "SUN,TUE,THU,SAT");
        assert_eq!(dias_en_nombres("mon-fri").unwrap(), "MON-FRI");
        assert!(dias_en_nombres("8").is_err() && dias_en_nombres("5-2").is_err());
        assert!(horario("* * * *").is_err() && horario("99 * * * *").is_err());
    }

    #[test]
    fn proxima_con_zona_sin_cambio_de_hora() {
        let tz = zona("America/Bogota").unwrap();
        // viernes 2026-11-13 10:00 en Bogotá → lunes 16 a las 9:00 (-05:00), no el sábado
        let h = horario("0 9 * * 1-5").unwrap();
        assert_eq!(proxima(&h, tz, unix("2026-11-13T10:00:00-05:00")), Some(unix("2026-11-16T09:00:00-05:00")));
        let h = horario("*/5 * * * *").unwrap();
        let a = unix("2026-11-16T10:02:30-05:00");
        assert_eq!(proxima(&h, tz, a), Some(unix("2026-11-16T10:05:00-05:00")));
        // estrictamente DESPUÉS: en la hora justa, la siguiente
        assert_eq!(proxima(&h, tz, unix("2026-11-16T10:05:00-05:00")), Some(unix("2026-11-16T10:10:00-05:00")));
        assert!(zona("Marte/Olimpo").is_err());
        assert!(!zona_local().is_empty());
    }

    #[test]
    fn proxima_cuando_el_reloj_se_adelanta_no_se_salta_el_dia() {
        // Nueva York, 2026-03-08: a las 2:00 pasa a las 3:00; las 2:30 no existen
        let tz = zona("America/New_York").unwrap();
        let h = horario("30 2 * * *").unwrap();
        let p = proxima(&h, tz, unix("2026-03-08T00:00:00-05:00")).unwrap();
        assert_eq!(p, unix("2026-03-08T03:00:00-04:00"), "corre al salir del hueco");
        assert_eq!(proxima(&h, tz, p), Some(unix("2026-03-09T02:30:00-04:00")));
    }

    #[test]
    fn proxima_cuando_el_reloj_se_atrasa_corre_una_sola_vez() {
        // Nueva York, 2026-11-01: a las 2:00 vuelve a la 1:00; las 1:30 pasan DOS veces
        let tz = zona("America/New_York").unwrap();
        let h = horario("30 1 * * *").unwrap();
        let p = proxima(&h, tz, unix("2026-11-01T00:00:00-04:00")).unwrap();
        assert_eq!(p, unix("2026-11-01T01:30:00-04:00"));
        // la segunda 1:30 (la de -05:00) NO es otra corrida: la siguiente es el día 2
        assert_eq!(proxima(&h, tz, p), Some(unix("2026-11-02T01:30:00-05:00")));
        // y una cada 15 min atraviesa la hora repetida sin repetir horas de pared: decisión declarada,
        // «una hora de pared, una corrida» (en la segunda vuelta de la 1:xx no corre: pausa de 1 h real)
        let h = horario("*/15 * * * *").unwrap();
        let mut t = unix("2026-11-01T00:50:00-04:00");
        let mut vistas = vec![];
        for _ in 0..6 {
            t = proxima(&h, tz, t).unwrap();
            vistas.push(t);
        }
        assert!(vistas.windows(2).all(|w| w[1] > w[0]));
        assert_eq!(vistas[1], unix("2026-11-01T01:15:00-04:00"));
        assert_eq!(vistas[3], unix("2026-11-01T01:45:00-04:00"));
        assert_eq!(vistas[4], unix("2026-11-01T02:00:00-05:00"));
    }
}
