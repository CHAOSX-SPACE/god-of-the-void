//! F23.3 · El índice propio, enfocado en lo del usuario, en `~/.cache/gotv/indice/indice.db`.
//!
//! Texto completo con FTS5 de SQLite (BM25), que ya viene dentro de `rusqlite` «bundled»
//! (`-DSQLITE_ENABLE_FTS5`) — en lugar de `tantivy` (el plan): cero crates nuevos, el mismo motor que
//! ya usa `rag.rs`, y la latencia medida en la prueba `mil_paginas_en_milisegundos`.
//!
//! Qué entra (F23.L-3/5): páginas semilla que elige el usuario o páginas que GOTV leyó para él (clase
//! B, con `robots.txt` y `noindex` respetados) y resultados de buscadores cuya ficha dice `si`. Nunca
//! los de una ficha con `no` ni con caché corta. Cada fila lleva su procedencia (F23.L-4) y vence.
//! Funciona sin conexión y es la fuente `local` del metabuscador.

pub mod rastreador;

use crate::buscar::legal::{Ficha, Guardar, Procedencia};
use crate::buscar::{Buscador, Hallazgo};
use anyhow::{anyhow, Result};
use rusqlite::{params, Connection};
use std::path::{Path, PathBuf};

/// Días que vive una página indexada antes de volver a leerse (o borrarse).
pub const DIAS_PAGINA: i64 = 30;
/// `[indice] tope_gb` por omisión (Anexo A).
pub const TOPE_GB: f64 = 5.0;

const ESQUEMA: &str = "CREATE VIRTUAL TABLE IF NOT EXISTS paginas USING fts5(
    url UNINDEXED, titulo, texto, dominio UNINDEXED, origen UNINDEXED,
    fecha UNINDEXED, vence UNINDEXED, regla UNINDEXED,
    tokenize = 'unicode61 remove_diacritics 2'
)"; // juez-idioma: datos (esquema SQL)

/// Una página para el índice.
#[derive(Debug, Clone)]
pub struct Pagina {
    pub url: String,
    pub titulo: String,
    pub texto: String,
}

pub struct Indice {
    db: PathBuf,
    /// Bytes como máximo (L6: tope de disco).
    pub tope_bytes: u64,
}

pub fn dominio_de(url: &str) -> String {
    crate::buscar::legal::host_de(url).map(|h| h.trim_start_matches("www.").to_string()).unwrap_or_default()
}

impl Indice {
    pub fn abrir(dir: &Path) -> Result<Indice> {
        std::fs::create_dir_all(dir)?;
        let i = Indice { db: dir.join("indice.db"), tope_bytes: (TOPE_GB * 1e9) as u64 };
        i.con()?;
        Ok(i)
    }

    /// Sólo si ya existe (la búsqueda no crea un índice vacío por consultarlo).
    pub fn abrir_si_existe(dir: &Path) -> Option<Indice> {
        dir.join("indice.db").is_file().then(|| Indice { db: dir.join("indice.db"), tope_bytes: (TOPE_GB * 1e9) as u64 })
    }

    fn con(&self) -> Result<Connection> {
        let c = Connection::open(&self.db)?;
        c.busy_timeout(std::time::Duration::from_secs(5))?;
        c.execute_batch(ESQUEMA)?;
        Ok(c)
    }

    pub fn ocupa(&self) -> u64 {
        ["", "-wal", "-shm", "-journal"]
            .iter()
            .filter_map(|s| {
                let mut p = self.db.as_os_str().to_os_string();
                p.push(s);
                std::fs::metadata(PathBuf::from(p)).ok()
            })
            .map(|m| m.len())
            .sum()
    }

    pub fn paginas(&self) -> Result<u64> {
        Ok(self.con()?.query_row("SELECT count(*) FROM paginas", [], |r| r.get::<_, i64>(0))? as u64)
    }

    /// Guarda (o renueva) una página con su procedencia. Sin procedencia completa, no (F23.L-4).
    // legal: F23.L-4 (procedencia) · F23.L-5 (quien llama ya miró robots.txt/noindex/sesión)
    pub fn agregar(&self, p: &Pagina, pr: &Procedencia) -> Result<()> {
        pr.validar()?;
        if pr.url != p.url {
            return Err(anyhow!("{}", crate::i18n::t("la procedencia no es de esta página", "the provenance is not this page's")));
        }
        let mut c = self.con()?;
        let tx = c.transaction()?;
        tx.execute("DELETE FROM paginas WHERE url = ?1", params![p.url])?;
        tx.execute(
            "INSERT INTO paginas (url, titulo, texto, dominio, origen, fecha, vence, regla) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
            params![p.url, p.titulo, p.texto, dominio_de(&p.url), pr.origen, pr.fecha, pr.vence, pr.regla],
        )?;
        tx.commit()?;
        drop(c);
        self.respetar_tope()?;
        Ok(())
    }

    /// F23.L-3: resultados de un buscador entran SÓLO si su ficha dice `si`.
    // legal: F23.L-3
    pub fn agregar_hallazgos(&self, ficha: &Ficha, hallazgos: &[Hallazgo], ahora: i64) -> Result<usize> {
        if ficha.guardar() != Guardar::Si {
            return Ok(0);
        }
        let vence = ahora + i64::from(crate::buscar::legal::HORAS_SI) * 3_600;
        for h in hallazgos {
            let pr = Procedencia { origen: ficha.id.clone(), url: h.url.clone(), fecha: ahora, vence, regla: crate::buscar::legal::REGLA_CACHE.into() };
            self.agregar(&Pagina { url: h.url.clone(), titulo: h.titulo.clone(), texto: h.resumen.clone() }, &pr)?;
        }
        Ok(hallazgos.len())
    }

    /// Por encima del tope de disco, salen primero las más viejas (cada una por su fila exacta).
    fn respetar_tope(&self) -> Result<()> {
        if self.ocupa() <= self.tope_bytes {
            return Ok(());
        }
        let c = self.con()?;
        while self.ocupa() > self.tope_bytes {
            let viejas: Vec<i64> = {
                let mut st = c.prepare("SELECT rowid FROM paginas ORDER BY CAST(fecha AS INTEGER) ASC LIMIT 20")?;
                let v = st.query_map([], |r| r.get(0))?.flatten().collect();
                v
            };
            if viejas.is_empty() {
                break;
            }
            for id in viejas {
                c.execute("DELETE FROM paginas WHERE rowid = ?1", params![id])?;
            }
            c.execute_batch("INSERT INTO paginas(paginas) VALUES('optimize'); VACUUM;")?;
        }
        Ok(())
    }

    /// Borra lo vencido, fila por fila (L6).
    pub fn barrer(&self, ahora: i64) -> Result<usize> {
        let c = self.con()?;
        let v: Vec<i64> = {
            let mut st = c.prepare("SELECT rowid FROM paginas WHERE CAST(vence AS INTEGER) <= ?1")?;
            let v = st.query_map(params![ahora], |r| r.get(0))?.flatten().collect();
            v
        };
        for id in &v {
            c.execute("DELETE FROM paginas WHERE rowid = ?1", params![id])?;
        }
        Ok(v.len())
    }

    /// Todas las procedencias (para comprobar F23.L-4).
    pub fn procedencias(&self) -> Result<Vec<Procedencia>> {
        let c = self.con()?;
        let mut st = c.prepare("SELECT origen, url, fecha, vence, regla FROM paginas")?;
        let v = st
            .query_map([], |r| {
                Ok(Procedencia { origen: r.get(0)?, url: r.get(1)?, fecha: r.get(2)?, vence: r.get(3)?, regla: r.get(4)? })
            })?
            .flatten()
            .collect();
        Ok(v)
    }

    /// Búsqueda de texto completo (BM25; el título pesa 4 veces más que el cuerpo). Primero todas las
    /// palabras; si no hay nada, cualquiera de ellas.
    pub fn consultar(&self, q: &str, n: usize, ahora: i64) -> Result<Vec<Hallazgo>> {
        let palabras: Vec<String> = q
            .split(|c: char| !c.is_alphanumeric())
            .filter(|w| w.chars().count() >= 2)
            .map(|w| format!("\"{}\"", w.replace('"', "")))
            .collect();
        if palabras.is_empty() {
            return Ok(Vec::new());
        }
        let c = self.con()?;
        for union in [" ", " OR "] {
            let expr = palabras.join(union);
            let mut st = c.prepare(
                "SELECT url, titulo, snippet(paginas, 2, '', '', '…', 24) FROM paginas
                 WHERE paginas MATCH ?1 AND CAST(vence AS INTEGER) > ?2
                 ORDER BY bm25(paginas, 0.0, 4.0, 1.0) LIMIT ?3",
            )?;
            let v: Vec<Hallazgo> = st
                .query_map(params![expr, ahora, n as i64], |r| {
                    Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?))
                })?
                .flatten()
                .enumerate()
                .map(|(i, (url, titulo, resumen))| Hallazgo { titulo, url, resumen, fuente: "local".into(), puesto: i + 1 })
                .collect();
            if !v.is_empty() {
                return Ok(v);
            }
        }
        Ok(Vec::new())
    }

    /// El texto entero de una página indexada (para `investigar` sin conexión).
    pub fn texto_de(&self, url: &str) -> Option<(String, String)> {
        self.con().ok()?.query_row("SELECT titulo, texto FROM paginas WHERE url = ?1", params![url], |r| Ok((r.get(0)?, r.get(1)?))).ok()
    }
}

#[async_trait::async_trait]
impl Buscador for Indice {
    fn id(&self) -> &str {
        "local"
    }

    async fn buscar(&self, consulta: &str, n: usize) -> Result<Vec<Hallazgo>> {
        let ahora = crate::buscar::legal::RelojSistema;
        use crate::buscar::legal::Reloj;
        let (db, tope, q) = (self.db.clone(), self.tope_bytes, consulta.to_string());
        let t = ahora.ahora();
        tokio::task::spawn_blocking(move || Indice { db, tope_bytes: tope }.consultar(&q, n, t))
            .await
            .map_err(|e| anyhow!("{e}"))?
    }
}

/// `[indice]` del config del usuario: tope_gb, ritmo_ms, semillas.
#[derive(Debug, Clone)]
pub struct AjustesIndice {
    pub tope_gb: f64,
    pub ritmo_ms: u64,
    pub semillas: Vec<String>,
}

impl Default for AjustesIndice {
    fn default() -> Self {
        Self { tope_gb: TOPE_GB, ritmo_ms: 1_000, semillas: Vec::new() }
    }
}

impl AjustesIndice {
    pub fn cargar() -> Self {
        let t = std::fs::read_to_string(crate::config::global_path()).ok().and_then(|t| t.parse::<toml::Table>().ok());
        let mut a = Self::default();
        if let Some(i) = t.as_ref().and_then(|t| t.get("indice")) {
            a.tope_gb = i.get("tope_gb").and_then(|v| v.as_float().or_else(|| v.as_integer().map(|n| n as f64))).unwrap_or(a.tope_gb).max(0.01);
            // F23.L-5: nunca más rápido que 1 por segundo por dominio... salvo que el usuario lo pida más lento
            a.ritmo_ms = i.get("ritmo_ms").and_then(|v| v.as_integer()).map(|n| n.max(1_000) as u64).unwrap_or(a.ritmo_ms);
            a.semillas = i
                .get("semillas")
                .and_then(|v| v.as_array())
                .map(|x| x.iter().filter_map(|s| s.as_str().map(String::from)).filter(|s| s.starts_with("http")).collect())
                .unwrap_or_default();
        }
        a
    }
}

/// Para la ventana: páginas, lo que ocupa, el tope y las semillas.
pub fn estado(dir: &Path) -> serde_json::Value {
    let a = AjustesIndice::cargar();
    let (paginas, ocupa) = Indice::abrir_si_existe(dir).map(|i| (i.paginas().unwrap_or(0), i.ocupa())).unwrap_or((0, 0));
    serde_json::json!({ "paginas": paginas, "ocupa": ocupa, "tope_gb": a.tope_gb, "ritmo_ms": a.ritmo_ms, "semillas": a.semillas })
}

/// `[indice] semillas` en el config del usuario (sólo direcciones http/https).
pub fn poner_semillas(config: &Path, semillas: &[String]) -> Result<()> {
    let mut t = match std::fs::read_to_string(config) {
        Ok(s) => s.parse::<toml::Table>().map_err(|e| anyhow!("{}: {e}", config.display()))?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => toml::Table::new(),
        Err(e) => return Err(e.into()),
    };
    let i = t.entry("indice").or_insert_with(|| toml::Value::Table(Default::default()));
    let Some(i) = i.as_table_mut() else { return Err(anyhow!("[indice] no es una tabla")) }; // juez-idioma: datos (clave de config)
    let v: Vec<toml::Value> = semillas.iter().map(|s| s.trim()).filter(|s| s.starts_with("http://") || s.starts_with("https://")).map(|s| toml::Value::String(s.to_string())).collect();
    i.insert("semillas".into(), toml::Value::Array(v));
    if let Some(d) = config.parent() {
        std::fs::create_dir_all(d)?;
    }
    std::fs::write(config, toml::to_string_pretty(&t)?)?;
    Ok(())
}

/// Rastrea las semillas del usuario con sus ajustes (ritmo, tope de disco) y barre lo vencido.
pub async fn rastrear_semillas(dir: &Path) -> Result<rastreador::Informe> {
    let a = AjustesIndice::cargar();
    let mut i = Indice::abrir(dir)?;
    i.tope_bytes = (a.tope_gb * 1e9) as u64;
    let ahora = crate::buscar::legal::Reloj::ahora(&crate::buscar::legal::RelojSistema);
    let _ = i.barrer(ahora);
    let op = rastreador::Opciones { ritmo: std::time::Duration::from_millis(a.ritmo_ms), ..Default::default() };
    rastreador::rastrear(&i, &a.semillas, &op, &crate::buscar::cortesia::Ritmo::global(), ahora).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buscar::legal::REGLA_INDICE;

    fn pr(url: &str, fecha: i64) -> Procedencia {
        Procedencia { origen: dominio_de(url), url: url.into(), fecha, vence: fecha + DIAS_PAGINA * 86_400, regla: REGLA_INDICE.into() }
    }

    #[test]
    fn indexa_busca_y_vence() {
        let d = crate::CarpetaPrueba::nueva("gotv-indice-basico");
        let i = Indice::abrir(d.ruta()).unwrap();
        let p = |u: &str, t: &str, x: &str| Pagina { url: u.into(), titulo: t.into(), texto: x.into() };
        i.agregar(&p("https://docs.rs/tokio", "tokio - Rust", "Un runtime asíncrono para Rust con tareas y canales"), &pr("https://docs.rs/tokio", 1_000)).unwrap();
        i.agregar(&p("https://serde.rs", "Serde", "Serialización en Rust: derive Serialize y Deserialize"), &pr("https://serde.rs", 1_000)).unwrap();
        let v = i.consultar("runtime asincrono", 5, 2_000).unwrap();
        assert_eq!(v[0].url, "https://docs.rs/tokio", "sin tildes también encuentra");
        assert_eq!(v[0].fuente, "local");
        // el título pesa más
        let v = i.consultar("serde rust", 5, 2_000).unwrap();
        assert_eq!(v[0].url, "https://serde.rs");
        // renovar no duplica
        i.agregar(&p("https://serde.rs", "Serde", "otra versión"), &pr("https://serde.rs", 1_500)).unwrap();
        assert_eq!(i.paginas().unwrap(), 2);
        // nada de sintaxis FTS5 rota con comillas o guiones
        assert!(i.consultar("\"tokio\" -- OR (", 5, 2_000).is_ok());
        // vencidas no salen y el barrido las borra
        let tarde = 1_500 + DIAS_PAGINA * 86_400;
        assert!(i.consultar("tokio", 5, tarde).unwrap().is_empty());
        assert_eq!(i.barrer(tarde).unwrap(), 2);
        // sin procedencia no entra
        let mut malo = pr("https://x.io", 10);
        malo.regla = String::new();
        assert!(i.agregar(&p("https://x.io", "x", "x"), &malo).is_err());
        assert!(i.agregar(&p("https://y.io", "y", "y"), &pr("https://x.io", 10)).is_err(), "procedencia de otra URL");
    }

    #[test]
    fn solo_entran_resultados_de_fichas_que_dicen_si() {
        let d = crate::CarpetaPrueba::nueva("gotv-indice-fichas");
        let i = Indice::abrir(d.ruta()).unwrap();
        let h = Hallazgo { titulo: "T".into(), url: "https://t.io".into(), resumen: "resumen".into(), fuente: "x".into(), puesto: 1 };
        let fichas = crate::buscar::legal::fichas();
        for id in ["brave", "exa", "tavily", "serper", "github", "duckduckgo"] {
            assert_eq!(i.agregar_hallazgos(&fichas[id], &[h.clone()], 1_000).unwrap(), 0, "{id}");
        }
        assert_eq!(i.agregar_hallazgos(&fichas["wikipedia"], &[h.clone()], 1_000).unwrap(), 1);
        assert_eq!(i.paginas().unwrap(), 1);
        assert!(i.procedencias().unwrap().iter().all(|p| p.validar().is_ok()));
    }

    #[test]
    fn el_tope_de_disco_saca_lo_mas_viejo() {
        let d = crate::CarpetaPrueba::nueva("gotv-indice-tope");
        let mut i = Indice::abrir(d.ruta()).unwrap();
        i.tope_bytes = 200_000;
        let relleno = "palabra ".repeat(2_000);
        for k in 0..60 {
            let u = format!("https://x.io/{k}");
            i.agregar(&Pagina { url: u.clone(), titulo: format!("p{k}"), texto: format!("{relleno} unico{k}") }, &pr(&u, 1_000 + k)).unwrap();
        }
        assert!(i.ocupa() <= 200_000, "{}", i.ocupa());
        assert!(i.consultar("unico59", 1, 2_000).unwrap().len() == 1, "la más nueva sigue");
        assert!(i.consultar("unico0", 1, 2_000).unwrap().is_empty(), "la más vieja salió");
    }

    #[test]
    fn mil_paginas_en_milisegundos() {
        let d = crate::CarpetaPrueba::nueva("gotv-indice-mil");
        let i = Indice::abrir(d.ruta()).unwrap();
        let temas = ["tokio", "serde", "axum", "rusqlite", "reqwest", "tantivy", "anyhow", "clap"];
        for k in 0..1_000 {
            let u = format!("https://docs.ejemplo/{k}");
            let t = temas[k % temas.len()];
            i.agregar(&Pagina { url: u.clone(), titulo: format!("{t} {k}"), texto: format!("{t} documentación página {k} con texto de relleno sobre {t} y Rust") }, &pr(&u, 1_000)).unwrap();
        }
        let t0 = std::time::Instant::now();
        for _ in 0..20 {
            assert!(!i.consultar("axum rust", 10, 2_000).unwrap().is_empty());
        }
        let ms = t0.elapsed().as_secs_f64() * 1_000.0 / 20.0;
        eprintln!("indice: 1000 páginas, {ms:.2} ms por consulta");
        assert!(ms < 50.0, "{ms} ms");
    }
}
