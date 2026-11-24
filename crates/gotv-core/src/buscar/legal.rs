//! F23.L · Las reglas legales del guardado (PLAN-CUERPO, F23.L-1 a F23.L-12).
//!
//! Tres clases de datos: A (resultado de un buscador: del servicio, según sus condiciones), B (página
//! leída: del sitio) y C (lo que escribe el agente: del usuario). Este archivo es la LEY en código:
//! - la **ficha legal** de cada servicio (`recursos/buscadores/<id>.toml`), escrita leyendo sus
//!   condiciones oficiales (F23.L-1); sin ficha completa y revisada el servicio NO se activa: lo impide
//!   [`Ficha::activable`], no un aviso;
//! - la **caducidad** de una ficha: 6 meses o un texto de condiciones distinto (F23.L-2);
//! - la **procedencia** que lleva todo registro guardado (F23.L-4).
//!
//! Este código no es asesoría legal: antes del lanzamiento público, un abogado revisa las fichas.

use anyhow::{anyhow, Result};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicI64, Ordering};

/// Días que una revisión de condiciones sigue valiendo (F23.L-2: «pasan 6 meses»).
pub const VIGENCIA_DIAS: i64 = 183;

// ── reloj (las pruebas simulan el paso de las horas) ─────────────────────

/// Segundos Unix «ahora». La caché vence por este reloj, no por el del sistema, para que
/// `legal_guardado.rs` pueda adelantar una hora sin esperarla (F23.L-11).
pub trait Reloj: Send + Sync {
    fn ahora(&self) -> i64;
}

pub struct RelojSistema;

impl Reloj for RelojSistema {
    fn ahora(&self) -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0)
    }
}

/// Reloj de las pruebas: se adelanta a mano.
pub struct RelojFijo(pub AtomicI64);

impl RelojFijo {
    pub fn en(t: i64) -> Self {
        Self(AtomicI64::new(t))
    }
    pub fn adelantar(&self, segundos: i64) {
        self.0.fetch_add(segundos, Ordering::SeqCst);
    }
}

impl Reloj for RelojFijo {
    fn ahora(&self) -> i64 {
        self.0.load(Ordering::SeqCst)
    }
}

// ── fechas civiles sin dependencias (algoritmo de H. Hinnant, dominio público) ─

/// Días desde 1970-01-01 de una fecha civil.
pub fn dias_desde_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m as i64 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// (año, mes, día) de un número de días desde 1970-01-01.
pub fn civil_de_dias(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// `AAAA-MM-DD` → segundos Unix a medianoche UTC.
pub fn fecha_a_unix(s: &str) -> Option<i64> {
    let mut p = s.trim().splitn(3, '-');
    let y: i64 = p.next()?.parse().ok()?;
    let m: u32 = p.next()?.parse().ok()?;
    let d: u32 = p.next()?.parse().ok()?;
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    Some(dias_desde_civil(y, m, d) * 86_400)
}

pub fn unix_a_fecha(t: i64) -> String {
    let (y, m, d) = civil_de_dias(t.div_euclid(86_400));
    format!("{y:04}-{m:02}-{d:02}")
}

/// `AAAA-MM` del instante: la llave del contador mensual de uso.
pub fn mes_de(t: i64) -> String {
    let (y, m, _) = civil_de_dias(t.div_euclid(86_400));
    format!("{y:04}-{m:02}")
}

// ── la ficha legal (F23.L-1) ─────────────────────────────────────────────

/// Qué permite guardar un servicio de sus resultados (clase A).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Guardar {
    /// Nada al disco: sólo en memoria del turno (F23.L-3, la regla por omisión).
    No,
    /// Caché local que vence a las N horas.
    Cache { horas: u32 },
    /// Se puede guardar (con procedencia y vencimiento igual: F23.L-4), también en el índice propio.
    Si,
}

/// Horas que vive un resultado de un servicio que permite guardar sin plazo (`si`): un registro sin
/// vencimiento no se escribe (F23.L-4), así que también estos caducan.
pub const HORAS_SI: u32 = 24 * 30;

/// Un texto que se ve, en los dos idiomas (L5).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Par {
    pub es: &'static str,
    pub en: &'static str,
}

impl Par {
    /// El del idioma activo.
    pub fn ver(&self) -> &'static str {
        if crate::i18n::is_en() { self.en } else { self.es }
    }
    /// Falta alguno de los dos.
    pub fn incompleto(&self) -> bool {
        self.es.trim().is_empty() || self.en.trim().is_empty()
    }
}

#[derive(Debug, Clone, Default)]
pub struct Limites {
    /// Peticiones por segundo como máximo (arXiv: 1 cada 3 s = 0.333…). Se hace cumplir en el código.
    pub por_segundo: Option<f64>,
    /// Peticiones por mes de la capa de entrada, si la hay (dato informativo; el tope lo pone el usuario).
    pub por_mes: Option<u64>,
    pub nota: Par,
}

#[derive(Debug, Clone, Default)]
pub struct Precio {
    pub texto: Par,
    /// Fecha en que se leyó el precio en la página oficial.
    pub fecha: String,
    pub fuente: String,
}

/// La ficha legal de un servicio. Las de fábrica están escritas en Rust (`fichas.rs`).
#[derive(Debug, Clone, Default)]
pub struct Ficha {
    pub id: String,
    pub nombre: String,
    pub publica: String,
    pub condiciones_url: String,
    /// `AAAA-MM-DD` de la lectura de las condiciones.
    pub condiciones_revisadas: String,
    /// `sha256:<hex>` del texto normalizado de las condiciones ([`huella_de_texto`]).
    pub condiciones_hash: String,
    /// Quién leyó las condiciones (y si un abogado ya lo revisó).
    pub revisada_por: String,
    /// `revisada` · `sin_revisar` · `retirada`.
    pub estado: String,
    /// Sólo `F23.L-9`: el DuckDuckGo de hoy sigue funcionando «sin revisar» mientras el Portador
    /// decide (la única excepción escrita en el plan; ninguna otra ficha la puede usar).
    pub excepcion: String,
    /// `no` · `cache` · `si`.
    pub guardar_resultados: String,
    pub cache_horas: Option<u32>,
    pub mostrar_al_usuario: bool,
    /// Texto que exige mostrar junto a sus resultados (vacío si no exige nada).
    pub atribucion: String,
    /// `si` · `no` · `sin_dato`.
    pub prohibe_entrenar_ia: String,
    pub limites: Limites,
    pub precio: Precio,
    /// El ÚNICO dominio al que se manda la llave (F23.L-7).
    pub dominio_api: String,
    pub necesita_llave: bool,
    /// Dónde saca el usuario su llave (sólo los servicios con llave).
    pub donde_llave: String,
    /// «Qué permite guardar», en lenguaje claro, para la ventana (F23.2).
    pub resumen: Par,
    /// De qué cláusula sale cada regla (citas cortas de las condiciones).
    pub notas_legales: Par,
    /// Las URL oficiales de las que sale cada regla (citadas en `notas_legales`).
    pub fuentes: Vec<String>,
}

/// Todas las fichas de fábrica, por id.
pub fn fichas() -> BTreeMap<String, Ficha> {
    super::fichas::de_fabrica().into_iter().map(|f| (f.id.clone(), f)).collect()
}

pub fn ficha(id: &str) -> Option<Ficha> {
    fichas().remove(id)
}

impl Ficha {
    /// Lo que la ficha permite guardar. Una ficha mal escrita cae en lo más estricto: `No`.
    pub fn guardar(&self) -> Guardar {
        match (self.guardar_resultados.as_str(), self.cache_horas) {
            ("cache", Some(h)) if h > 0 => Guardar::Cache { horas: h },
            ("si", _) => Guardar::Si,
            _ => Guardar::No,
        }
    }

    /// Todo lo que impide activar el servicio HOY (vacío = se puede). `hash_visto`: la última huella
    /// medida del texto de sus condiciones, si se volvió a leer (F23.L-2).
    pub fn problemas(&self, ahora: i64, hash_visto: Option<&str>) -> Vec<String> {
        let mut p = Vec::new();
        let vacios: Vec<&str> = [
            ("nombre", &self.nombre),
            ("publica", &self.publica),
            ("condiciones_url", &self.condiciones_url),
            ("condiciones_hash", &self.condiciones_hash),
            ("revisada_por", &self.revisada_por),
            ("dominio_api", &self.dominio_api),
            ("prohibe_entrenar_ia", &self.prohibe_entrenar_ia),
        ]
        .into_iter()
        .filter(|(_, v)| v.trim().is_empty())
        .map(|(k, _)| k)
        .chain(self.notas_legales.incompleto().then_some("notas_legales"))
        .chain(self.resumen.incompleto().then_some("resumen"))
        .collect();
        if !vacios.is_empty() {
            p.push(crate::tr!("ficha incompleta: falta {}", "incomplete sheet: missing {}", vacios.join(", ")));
        }
        if self.fuentes.is_empty() {
            p.push(crate::i18n::t("la ficha no cita sus fuentes oficiales", "the sheet cites no official sources").to_string());
        }
        if !matches!(self.guardar_resultados.as_str(), "no" | "cache" | "si")
            || (self.guardar_resultados == "cache" && self.cache_horas.unwrap_or(0) == 0)
        {
            p.push(crate::tr!(
                "guardar_resultados inválido («{}»): debe ser no, si o cache con cache_horas > 0",
                "invalid guardar_resultados («{}»): must be no, si or cache with cache_horas > 0",
                self.guardar_resultados
            ));
        }
        if self.necesita_llave && self.donde_llave.trim().is_empty() {
            p.push(crate::i18n::t("falta dónde sacar la llave (donde_llave)", "missing where to get the key (donde_llave)").to_string());
        }
        match self.estado.as_str() {
            "revisada" => {}
            // legal: F23.L-9 (sólo DuckDuckGo, como hoy y marcado; ver `aviso`)
            "sin_revisar" if self.excepcion == "F23.L-9" && self.id == "duckduckgo" => {}
            "retirada" => p.push(crate::i18n::t("servicio retirado por su ficha legal", "service withdrawn by its legal sheet").to_string()),
            _ => p.push(crate::i18n::t("ficha legal sin revisar", "legal sheet not reviewed").to_string()),
        }
        match fecha_a_unix(&self.condiciones_revisadas) {
            None => p.push(crate::i18n::t("condiciones_revisadas no es una fecha AAAA-MM-DD", "condiciones_revisadas is not a YYYY-MM-DD date").to_string()),
            Some(t) if ahora - t > VIGENCIA_DIAS * 86_400 => p.push(crate::tr!(
                "ficha vencida: las condiciones se revisaron el {} (hace más de 6 meses)",
                "expired sheet: the terms were reviewed on {} (more than 6 months ago)",
                self.condiciones_revisadas
            )),
            Some(_) => {}
        }
        if let Some(h) = hash_visto {
            if h != self.condiciones_hash {
                p.push(crate::i18n::t(
                    "las condiciones de uso cambiaron desde la revisión: hace falta revisarlas de nuevo",
                    "the terms of use changed since the review: they need a new review",
                ).to_string());
            }
        }
        p
    }

    /// F23.L-1/2: el servicio se puede usar HOY, o el porqué no.
    pub fn activable(&self, ahora: i64, hash_visto: Option<&str>) -> Result<()> {
        let p = self.problemas(ahora, hash_visto);
        if p.is_empty() {
            Ok(())
        } else {
            Err(anyhow!("{}: {}", self.id, p.join("; ")))
        }
    }

    /// La marca que acompaña a un servicio que funciona sin ficha revisada (F23.L-9).
    pub fn aviso(&self) -> Option<String> {
        (self.estado == "sin_revisar").then(|| crate::i18n::t("sin revisar", "not reviewed").to_string())
    }

    /// «Qué permite guardar» en lenguaje claro (F23.2), en el idioma de la ventana.
    pub fn resumen(&self) -> String {
        let r = self.resumen.ver();
        if !r.trim().is_empty() {
            return r.to_string();
        }
        match self.guardar() {
            Guardar::No => crate::tr!(
                "{} no permite guardar sus resultados: GOTV no los guardará",
                "{} does not allow storing its results: GOTV will not store them",
                self.nombre
            ),
            Guardar::Cache { horas } => crate::tr!(
                "{} permite una caché local de {horas} h: GOTV la borra al vencer",
                "{} allows a local cache of {horas} h: GOTV deletes it when it expires",
                self.nombre
            ),
            Guardar::Si => crate::tr!(
                "{} permite guardar sus resultados (con su procedencia y vencimiento)",
                "{} allows storing its results (with their provenance and expiry)",
                self.nombre
            ),
        }
    }

    /// F23.L-7: ¿se puede mandar la llave a esta URL? Sólo a `dominio_api`, exacto (sin subdominios
    /// ni parecidos: `api.brave.com.evil.io` no es `api.search.brave.com`).
    pub fn llave_puede_ir_a(&self, url: &str) -> bool {
        host_de(url).is_some_and(|h| h.eq_ignore_ascii_case(self.dominio_api.trim()))
    }
}

/// El host de una URL `http(s)://host[:puerto]/…`, sin puerto ni credenciales.
pub fn host_de(url: &str) -> Option<String> {
    let resto = url.split_once("://")?.1;
    let autoridad = resto.split(['/', '?', '#']).next()?;
    let autoridad = autoridad.rsplit('@').next()?;
    let host = if autoridad.starts_with('[') {
        autoridad.split(']').next()?.trim_start_matches('[').to_string()
    } else {
        autoridad.split(':').next()?.to_string()
    };
    (!host.is_empty()).then(|| host.to_ascii_lowercase())
}

/// F23.L-2: la huella del texto de unas condiciones: el HTML pasado a texto (`web::html_to_text`) y
/// con los espacios colapsados, para que un cambio de maquetación no cuente como cambio de texto.
pub fn huella_de_texto(html_o_texto: &str) -> String {
    use sha2::{Digest, Sha256};
    let texto = if html_o_texto.contains('<') { crate::web::html_to_text(html_o_texto) } else { html_o_texto.to_string() };
    let normal = texto.split_whitespace().collect::<Vec<_>>().join(" ");
    let h = Sha256::digest(normal.as_bytes());
    format!("sha256:{}", h.iter().map(|b| format!("{b:02x}")).collect::<String>())
}

// ── procedencia (F23.L-4) ────────────────────────────────────────────────

/// La regla F23.L-n que permitió guardar un registro.
pub const REGLA_CACHE: &str = "F23.L-3";
pub const REGLA_INDICE: &str = "F23.L-5";

/// Todo registro guardado lleva su procedencia: sin estos campos no se escribe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Procedencia {
    /// Servicio (`brave`) o sitio (`docs.rs`) de donde vino.
    pub origen: String,
    pub url: String,
    /// Segundos Unix de cuando se obtuvo.
    pub fecha: i64,
    /// Segundos Unix de cuando vence (siempre después de `fecha`).
    pub vence: i64,
    /// La regla F23.L-n que lo permitió.
    pub regla: String,
}

impl Procedencia {
    pub fn validar(&self) -> Result<()> {
        let ok = !self.origen.trim().is_empty()
            && !self.url.trim().is_empty()
            && self.regla.starts_with("F23.L-")
            && self.fecha > 0
            && self.vence > self.fecha;
        if ok {
            Ok(())
        } else {
            Err(anyhow!("{}", crate::i18n::t(
                "registro sin procedencia completa (origen, url, fecha, vence, regla): no se guarda (F23.L-4)",
                "record without complete provenance (origen, url, fecha, vence, regla): not stored (F23.L-4)"
            )))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fechas_civiles_ida_y_vuelta() {
        assert_eq!(fecha_a_unix("1970-01-01"), Some(0));
        assert_eq!(fecha_a_unix("2026-11-16"), Some(1_794_787_200));
        assert_eq!(unix_a_fecha(1_794_787_200 + 3_600), "2026-11-16");
        assert_eq!(mes_de(1_794_787_200), "2026-11");
        assert_eq!(unix_a_fecha(fecha_a_unix("2024-02-29").unwrap()), "2024-02-29");
        assert!(fecha_a_unix("2026-13-01").is_none() && fecha_a_unix("ayer").is_none());
    }

    #[test]
    fn las_fichas_de_fabrica_se_leen_todas_y_citan_fuentes() {
        let f = fichas();
        for id in ["duckduckgo", "wikipedia", "arxiv", "github", "brave", "serper", "tavily", "exa"] {
            let x = f.get(id).unwrap_or_else(|| panic!("falta o no se lee la ficha {id}"));
            assert!(!x.fuentes.is_empty() && x.fuentes.iter().all(|u| u.starts_with("https://")), "{id}: fuentes");
            assert!(x.condiciones_hash.starts_with("sha256:") && x.condiciones_hash.len() == 71, "{id}: huella");
            assert!(!x.condiciones_hash.contains("00000000000000000000"), "{id}: huella medida, no de relleno");
            assert!(fecha_a_unix(&x.condiciones_revisadas).is_some(), "{id}: fecha");
            assert!(!x.dominio_api.contains('/') && !x.dominio_api.contains(':'), "{id}: dominio_api es un host");
        }
    }

    fn base() -> Ficha {
        let s = String::from;
        Ficha {
            id: s("falso"),
            nombre: s("Falso"),
            publica: s("Nadie S.A."),
            condiciones_url: s("https://falso.test/terms"),
            condiciones_revisadas: s("2026-11-16"),
            condiciones_hash: s("sha256:aaaa"),
            revisada_por: s("prueba"),
            estado: s("revisada"),
            guardar_resultados: s("no"),
            mostrar_al_usuario: true,
            prohibe_entrenar_ia: s("sin_dato"),
            dominio_api: s("api.falso.test"),
            necesita_llave: true,
            donde_llave: s("https://falso.test/llave"),
            resumen: Par { es: "prueba", en: "test" },
            notas_legales: Par { es: "prueba", en: "test" },
            fuentes: vec![s("https://falso.test/terms")],
            ..Default::default()
        }
    }

    #[test]
    fn sin_ficha_revisada_y_al_dia_no_se_activa() {
        let hoy = fecha_a_unix("2026-11-20").unwrap();
        let f = base();
        assert!(f.activable(hoy, None).is_ok());
        assert!(f.activable(hoy, Some("sha256:aaaa")).is_ok());
        // F23.L-2: otro texto de condiciones → vencida
        assert!(f.activable(hoy, Some("sha256:bbbb")).is_err());
        // F23.L-2: seis meses después → vencida
        assert!(f.activable(fecha_a_unix("2027-06-01").unwrap(), None).is_err());
        let mut g = f.clone();
        g.estado = "sin_revisar".into();
        assert!(g.activable(hoy, None).is_err());
        let mut g = f.clone();
        g.dominio_api = " ".into();
        assert!(g.activable(hoy, None).unwrap_err().to_string().contains("dominio_api"));
        let mut g = f.clone();
        g.fuentes.clear();
        assert!(g.activable(hoy, None).is_err());
        // L5: sus textos en los dos idiomas o no se activa
        let mut g = f.clone();
        g.notas_legales.en = "";
        assert!(g.activable(hoy, None).unwrap_err().to_string().contains("notas_legales"));
        let mut g = f.clone();
        g.guardar_resultados = "cache".into();
        assert!(g.activable(hoy, None).is_err(), "cache sin horas");
        g.cache_horas = Some(1);
        assert!(g.activable(hoy, None).is_ok());
        assert_eq!(g.guardar(), Guardar::Cache { horas: 1 });
        g.guardar_resultados = "quizas".into();
        assert_eq!(g.guardar(), Guardar::No, "lo mal escrito cae en lo más estricto");
    }

    #[test]
    fn la_llave_sale_solo_a_su_dominio_exacto() {
        let f = base();
        assert!(f.llave_puede_ir_a("https://api.falso.test/v1/buscar?q=x"));
        assert!(f.llave_puede_ir_a("https://API.falso.test:443/v1"));
        for malo in [
            "https://api.falso.test.evil.io/v1",
            "https://evil.io/?u=https://api.falso.test",
            "https://api.falso.test@evil.io/",
            "https://otro.falso.test/",
            "api.falso.test/v1",
        ] {
            assert!(!f.llave_puede_ir_a(malo), "{malo}");
        }
        assert_eq!(host_de("http://[::1]:8080/x").as_deref(), Some("::1"));
    }

    #[test]
    fn la_huella_ignora_la_maquetacion_y_ve_el_texto() {
        let a = huella_de_texto("<html><body><p>No guardes   nada.</p></body></html>");
        let b = huella_de_texto("<html><body>\n<div><p>No guardes nada.</p></div></body></html>");
        let c = huella_de_texto("<html><body><p>Guarda todo.</p></body></html>");
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn sin_procedencia_completa_no_se_guarda() {
        let ok = Procedencia { origen: "brave".into(), url: "https://x.io".into(), fecha: 10, vence: 20, regla: REGLA_CACHE.into() };
        assert!(ok.validar().is_ok());
        for malo in [
            Procedencia { origen: "".into(), ..ok.clone() },
            Procedencia { url: " ".into(), ..ok.clone() },
            Procedencia { regla: "porque sí".into(), ..ok.clone() },
            Procedencia { vence: 10, ..ok.clone() },
            Procedencia { fecha: 0, ..ok.clone() },
        ] {
            assert!(malo.validar().is_err(), "{malo:?}");
        }
    }
}
