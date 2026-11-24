//! F23.L-3 · F23.L-4 · F23.L-12 · Dónde y cómo se guarda (y se borra) lo que trae la búsqueda.
//!
//! - `~/.cache/gotv/busquedas.db`: la caché de resultados (clase A). Sólo recibe lo de un servicio
//!   cuya ficha dice `cache_horas = N` o `si`; con `no` el archivo ni se crea (F23.L-3). Cada fila
//!   lleva su procedencia completa, y la base la exige con un CHECK (F23.L-4).
//! - `~/.config/gotv/buscadores.db`: el contador de consultas por servicio y mes (F23.1) y la última
//!   huella vista de las condiciones de cada servicio (F23.L-2). Ni consultas ni resultados.
//! - `~/.cache/gotv/indice/`: el índice propio (F23.3).
//!
//! Borrar es fácil (F23.L-12) y es por ruta EXACTA (ley L6): nunca por patrón.

use super::legal::{Ficha, Guardar, Procedencia, HORAS_SI, REGLA_CACHE};
use super::Hallazgo;
use anyhow::{anyhow, Result};
use rusqlite::{params, Connection, OptionalExtension};
use std::path::{Path, PathBuf};

/// Las tres rutas de la búsqueda. En las pruebas, dentro de una carpeta temporal.
#[derive(Debug, Clone)]
pub struct Almacen {
    pub cache_db: PathBuf,
    pub uso_db: PathBuf,
    pub indice_dir: PathBuf,
}

impl Almacen {
    pub fn del_usuario() -> Self {
        let h = crate::paths::home();
        Self {
            cache_db: h.join(".cache/gotv/busquedas.db"),
            uso_db: h.join(".config/gotv/buscadores.db"),
            indice_dir: h.join(".cache/gotv/indice"),
        }
    }

    /// Todo debajo de `base` (pruebas): `base/cache/busquedas.db`, `base/config/buscadores.db`,
    /// `base/cache/indice`.
    pub fn en(base: &Path) -> Self {
        Self {
            cache_db: base.join("cache/busquedas.db"),
            uso_db: base.join("config/buscadores.db"),
            indice_dir: base.join("cache/indice"),
        }
    }

    pub fn cache(&self) -> Cache {
        Cache { ruta: self.cache_db.clone() }
    }

    pub fn uso(&self) -> Uso {
        Uso { ruta: self.uso_db.clone() }
    }

    /// Lo que ocupa cada parte en bytes: (caché, índice).
    pub fn ocupa(&self) -> (u64, u64) {
        let cache: u64 = archivos_de_la_base(&self.cache_db).iter().filter_map(|p| p.metadata().ok()).map(|m| m.len()).sum();
        (cache, tam_carpeta(&self.indice_dir))
    }

    /// F23.L-12 · «Borrar caché e índice»: sólo `busquedas.db` (con su diario `-wal`/`-shm`/`-journal`)
    /// y la carpeta `indice/`, por sus rutas exactas. Devuelve lo liberado (caché, índice).
    pub fn borrar_cache_e_indice(&self) -> Result<(u64, u64)> {
        let (c, i) = self.ocupa();
        for p in archivos_de_la_base(&self.cache_db) {
            std::fs::remove_file(&p).map_err(|e| anyhow!("{}: {e}", p.display()))?;
        }
        if self.indice_dir.is_dir() {
            std::fs::remove_dir_all(&self.indice_dir).map_err(|e| anyhow!("{}: {e}", self.indice_dir.display()))?;
        }
        Ok((c, i))
    }
}

/// La base y sus archivos de diario de SQLite, los que existan (nombres exactos, no un patrón).
fn archivos_de_la_base(db: &Path) -> Vec<PathBuf> {
    let mut v = vec![db.to_path_buf()];
    for suf in ["-wal", "-shm", "-journal"] {
        let mut s = db.as_os_str().to_os_string();
        s.push(suf);
        v.push(PathBuf::from(s));
    }
    v.into_iter().filter(|p| p.is_file()).collect()
}

fn tam_carpeta(d: &Path) -> u64 {
    let Ok(rd) = std::fs::read_dir(d) else { return 0 };
    rd.flatten()
        .map(|e| match e.file_type() {
            Ok(t) if t.is_dir() => tam_carpeta(&e.path()),
            Ok(t) if t.is_file() => e.metadata().map(|m| m.len()).unwrap_or(0),
            _ => 0,
        })
        .sum()
}

fn abrir(ruta: &Path) -> Result<Connection> {
    if let Some(d) = ruta.parent() {
        std::fs::create_dir_all(d)?;
    }
    let c = Connection::open(ruta)?;
    c.busy_timeout(std::time::Duration::from_secs(5))?;
    Ok(c)
}

/// La llave de la caché: huella de (servicio, consulta normalizada, n). La consulta NO se guarda en
/// claro (F23.L-8: lo que el usuario busca es suyo).
pub fn clave(servicio: &str, consulta: &str, n: usize) -> String {
    use sha2::{Digest, Sha256};
    let q = consulta.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase();
    let h = Sha256::digest(format!("{servicio}\u{1f}{q}\u{1f}{n}").as_bytes());
    h.iter().take(16).map(|b| format!("{b:02x}")).collect()
}

// ── la caché de resultados (clase A) ─────────────────────────────────────

pub struct Cache {
    ruta: PathBuf,
}

/// Una fila de la caché con su procedencia (F23.L-4).
#[derive(Debug, Clone)]
pub struct Registro {
    pub clave: String,
    pub puesto: i64,
    pub titulo: String,
    pub resumen: String,
    pub procedencia: Procedencia,
}

const ESQUEMA: &str = "CREATE TABLE IF NOT EXISTS resultados (
    clave TEXT NOT NULL,
    puesto INTEGER NOT NULL,
    titulo TEXT NOT NULL,
    url TEXT NOT NULL,
    resumen TEXT NOT NULL,
    origen TEXT NOT NULL,
    fecha INTEGER NOT NULL,
    vence INTEGER NOT NULL,
    regla TEXT NOT NULL,
    PRIMARY KEY (clave, puesto),
    CHECK (origen <> '' AND url <> '' AND regla LIKE 'F23.L-%' AND fecha > 0 AND vence > fecha)
)"; // juez-idioma: datos (esquema SQL)

impl Cache {
    pub fn ruta(&self) -> &Path {
        &self.ruta
    }

    /// Guarda los hallazgos de `ficha` SI su ficha lo permite (F23.L-3). Con `no` no toca el disco
    /// (ni crea el archivo) y devuelve 0.
    // legal: F23.L-3 (sólo con cache_horas o si) · F23.L-4 (procedencia en cada fila)
    pub fn guardar(&self, ficha: &Ficha, consulta: &str, n: usize, hallazgos: &[Hallazgo], ahora: i64) -> Result<usize> {
        let horas = match ficha.guardar() {
            Guardar::No => return Ok(0),
            Guardar::Cache { horas } => horas,
            Guardar::Si => HORAS_SI,
        };
        if hallazgos.is_empty() {
            return Ok(0);
        }
        let k = clave(&ficha.id, consulta, n);
        let vence = ahora + i64::from(horas) * 3_600;
        let filas: Vec<(&Hallazgo, Procedencia)> = hallazgos
            .iter()
            .map(|h| (h, Procedencia { origen: ficha.id.clone(), url: h.url.clone(), fecha: ahora, vence, regla: REGLA_CACHE.into() }))
            .collect();
        for (_, p) in &filas {
            p.validar()?;
        }
        let mut c = abrir(&self.ruta)?;
        c.execute_batch(ESQUEMA)?;
        let tx = c.transaction()?;
        tx.execute("DELETE FROM resultados WHERE clave = ?1", params![k])?;
        for (h, p) in &filas {
            tx.execute(
                "INSERT INTO resultados (clave, puesto, titulo, url, resumen, origen, fecha, vence, regla) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
                params![k, h.puesto as i64, h.titulo, p.url, h.resumen, p.origen, p.fecha, p.vence, p.regla],
            )?;
        }
        tx.commit()?;
        Ok(filas.len())
    }

    /// Lo guardado para esa consulta, si existe, no venció y la ficha HOY aún permite guardar (si una
    /// ficha pasa a `no`, lo viejo deja de servirse aunque no haya vencido).
    pub fn leer(&self, ficha: &Ficha, consulta: &str, n: usize, ahora: i64) -> Option<Vec<Hallazgo>> {
        if ficha.guardar() == Guardar::No || !self.ruta.is_file() {
            return None;
        }
        let c = abrir(&self.ruta).ok()?;
        let mut st = c
            .prepare("SELECT titulo, url, resumen, puesto FROM resultados WHERE clave = ?1 AND vence > ?2 ORDER BY puesto")
            .ok()?;
        let v: Vec<Hallazgo> = st
            .query_map(params![clave(&ficha.id, consulta, n), ahora], |r| {
                Ok(Hallazgo { titulo: r.get(0)?, url: r.get(1)?, resumen: r.get(2)?, fuente: ficha.id.clone(), puesto: r.get::<_, i64>(3)? as usize })
            })
            .ok()?
            .flatten()
            .collect();
        (!v.is_empty()).then_some(v)
    }

    /// F23.L-3: borra lo vencido, fila por fila por su clave exacta (ley L6). Devuelve cuántas.
    pub fn barrer(&self, ahora: i64) -> Result<usize> {
        if !self.ruta.is_file() {
            return Ok(0);
        }
        let mut c = abrir(&self.ruta)?;
        c.execute_batch(ESQUEMA)?;
        let vencidas: Vec<(String, i64)> = {
            let mut st = c.prepare("SELECT clave, puesto FROM resultados WHERE vence <= ?1")?;
            let v = st.query_map(params![ahora], |r| Ok((r.get(0)?, r.get(1)?)))?.flatten().collect();
            v
        };
        let tx = c.transaction()?;
        for (k, p) in &vencidas {
            tx.execute("DELETE FROM resultados WHERE clave = ?1 AND puesto = ?2", params![k, p])?;
        }
        tx.commit()?;
        Ok(vencidas.len())
    }

    /// Todas las filas, con su procedencia (para la ventana y para `legal_guardado.rs`).
    pub fn registros(&self) -> Result<Vec<Registro>> {
        if !self.ruta.is_file() {
            return Ok(Vec::new());
        }
        let c = abrir(&self.ruta)?;
        c.execute_batch(ESQUEMA)?;
        let mut st = c.prepare("SELECT clave, puesto, titulo, resumen, origen, url, fecha, vence, regla FROM resultados")?;
        let v = st
            .query_map([], |r| {
                Ok(Registro {
                    clave: r.get(0)?,
                    puesto: r.get(1)?,
                    titulo: r.get(2)?,
                    resumen: r.get(3)?,
                    procedencia: Procedencia { origen: r.get(4)?, url: r.get(5)?, fecha: r.get(6)?, vence: r.get(7)?, regla: r.get(8)? },
                })
            })?
            .flatten()
            .collect();
        Ok(v)
    }
}

// ── el contador de uso y las huellas vistas ──────────────────────────────

pub struct Uso {
    ruta: PathBuf,
}

/// El servicio llegó a su tope del mes: se salta (nunca se pasa en silencio).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Agotado {
    pub usado: u64,
    pub tope: u64,
}

const ESQUEMA_USO: &str = "CREATE TABLE IF NOT EXISTS uso (servicio TEXT NOT NULL, mes TEXT NOT NULL, consultas INTEGER NOT NULL, PRIMARY KEY (servicio, mes));
CREATE TABLE IF NOT EXISTS condiciones (servicio TEXT PRIMARY KEY, hash_visto TEXT NOT NULL, fecha INTEGER NOT NULL);"; // juez-idioma: datos (esquema SQL)

impl Uso {
    fn con(&self) -> Result<Connection> {
        let c = abrir(&self.ruta)?;
        c.execute_batch(ESQUEMA_USO)?;
        Ok(c)
    }

    /// Cuenta UNA consulta del mes si cabe bajo el tope; si no, `Err(Agotado)` sin contar. Se llama
    /// ANTES de mandar la petición: ni en paralelo se pasa del tope.
    pub fn reservar(&self, servicio: &str, tope: Option<u64>, ahora: i64) -> Result<std::result::Result<u64, Agotado>> {
        let mes = super::legal::mes_de(ahora);
        let mut c = self.con()?;
        let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let usado: u64 = tx
            .query_row("SELECT consultas FROM uso WHERE servicio = ?1 AND mes = ?2", params![servicio, mes], |r| r.get::<_, i64>(0))
            .optional()?
            .unwrap_or(0) as u64;
        if let Some(t) = tope {
            if usado >= t {
                return Ok(Err(Agotado { usado, tope: t }));
            }
        }
        tx.execute(
            "INSERT INTO uso (servicio, mes, consultas) VALUES (?1, ?2, 1) ON CONFLICT(servicio, mes) DO UPDATE SET consultas = consultas + 1",
            params![servicio, mes],
        )?;
        tx.commit()?;
        Ok(Ok(usado + 1))
    }

    /// Devuelve una reserva que al final no salió (búsqueda en serie que ya tuvo resultados).
    pub fn devolver(&self, servicio: &str, ahora: i64) -> Result<()> {
        self.con()?.execute(
            "UPDATE uso SET consultas = consultas - 1 WHERE servicio = ?1 AND mes = ?2 AND consultas > 0",
            params![servicio, super::legal::mes_de(ahora)],
        )?;
        Ok(())
    }

    /// Consultas de este mes (sin abrir nada si la base no existe).
    pub fn del_mes(&self, servicio: &str, ahora: i64) -> u64 {
        if !self.ruta.is_file() {
            return 0;
        }
        self.con()
            .and_then(|c| {
                Ok(c.query_row("SELECT consultas FROM uso WHERE servicio = ?1 AND mes = ?2", params![servicio, super::legal::mes_de(ahora)], |r| r.get::<_, i64>(0))
                    .optional()?
                    .unwrap_or(0) as u64)
            })
            .unwrap_or(0)
    }

    /// F23.L-2: la última huella medida del texto de las condiciones de un servicio.
    pub fn hash_visto(&self, servicio: &str) -> Option<String> {
        if !self.ruta.is_file() {
            return None;
        }
        self.con().ok()?.query_row("SELECT hash_visto FROM condiciones WHERE servicio = ?1", params![servicio], |r| r.get(0)).optional().ok()?
    }

    pub fn anotar_hash(&self, servicio: &str, hash: &str, ahora: i64) -> Result<()> {
        self.con()?.execute(
            "INSERT INTO condiciones (servicio, hash_visto, fecha) VALUES (?1, ?2, ?3) ON CONFLICT(servicio) DO UPDATE SET hash_visto = ?2, fecha = ?3",
            params![servicio, hash, ahora],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buscar::legal::Ficha;

    fn ficha(guardar: &str, horas: Option<u32>) -> Ficha {
        let mut f = crate::buscar::legal::ficha("brave").unwrap();
        f.id = "falso".into();
        f.guardar_resultados = guardar.into();
        f.cache_horas = horas;
        f
    }

    fn h(i: usize) -> Hallazgo {
        Hallazgo { titulo: format!("T{i}"), url: format!("https://x.io/{i}"), resumen: format!("R{i}"), fuente: "falso".into(), puesto: i }
    }

    #[test]
    fn con_no_ni_se_crea_el_archivo() {
        let d = crate::CarpetaPrueba::nueva("gotv-buscar-cache-no");
        let a = Almacen::en(d.ruta());
        let n = a.cache().guardar(&ficha("no", None), "hola", 5, &[h(1), h(2)], 1_000).unwrap();
        assert_eq!(n, 0);
        assert!(!a.cache_db.exists());
        assert!(a.cache().leer(&ficha("no", None), "hola", 5, 1_000).is_none());
    }

    #[test]
    fn la_cache_vence_y_el_barrido_borra_lo_vencido() {
        let d = crate::CarpetaPrueba::nueva("gotv-buscar-cache-hora");
        let a = Almacen::en(d.ruta());
        let f = ficha("cache", Some(1));
        assert_eq!(a.cache().guardar(&f, "Hola  Mundo", 5, &[h(1), h(2)], 1_000).unwrap(), 2);
        // la misma consulta con otros espacios y mayúsculas es la misma llave
        assert_eq!(a.cache().leer(&f, "hola mundo", 5, 1_000 + 3_599).unwrap().len(), 2);
        assert!(a.cache().leer(&f, "hola mundo", 5, 1_000 + 3_600).is_none(), "vence a la hora");
        // si la ficha pasa a «no», lo viejo deja de servirse
        assert!(a.cache().leer(&ficha("no", None), "hola mundo", 5, 1_001).is_none());
        let regs = a.cache().registros().unwrap();
        assert!(regs.iter().all(|r| r.procedencia.validar().is_ok() && r.procedencia.origen == "falso"));
        assert_eq!(a.cache().barrer(1_000 + 3_599).unwrap(), 0);
        assert_eq!(a.cache().barrer(1_000 + 3_600).unwrap(), 2);
        assert!(a.cache().registros().unwrap().is_empty());
        // la consulta no está en claro en ningún lado de la base
        let crudo = std::fs::read(&a.cache_db).unwrap();
        assert!(!String::from_utf8_lossy(&crudo).to_lowercase().contains("hola mundo"));
    }

    #[test]
    fn la_base_rechaza_una_fila_sin_procedencia() {
        let d = crate::CarpetaPrueba::nueva("gotv-buscar-cache-check");
        let a = Almacen::en(d.ruta());
        a.cache().guardar(&ficha("si", None), "x", 1, &[h(1)], 1_000).unwrap();
        let c = Connection::open(&a.cache_db).unwrap();
        let malo = c.execute(
            "INSERT INTO resultados VALUES ('k', 1, 't', 'https://u', 'r', '', 1, 2, 'F23.L-3')",
            [],
        );
        assert!(malo.is_err(), "origen vacío");
        let malo = c.execute("INSERT INTO resultados VALUES ('k', 1, 't', 'https://u', 'r', 'o', 5, 5, 'F23.L-3')", []);
        assert!(malo.is_err(), "sin vencimiento");
    }

    #[test]
    fn el_tope_del_mes_no_se_pasa_y_cambia_de_mes() {
        let d = crate::CarpetaPrueba::nueva("gotv-buscar-uso");
        let u = Almacen::en(d.ruta()).uso();
        let nov = crate::buscar::legal::fecha_a_unix("2026-11-30").unwrap();
        assert_eq!(u.reservar("brave", Some(2), nov).unwrap(), Ok(1));
        assert_eq!(u.reservar("brave", Some(2), nov).unwrap(), Ok(2));
        assert_eq!(u.reservar("brave", Some(2), nov).unwrap(), Err(Agotado { usado: 2, tope: 2 }));
        assert_eq!(u.del_mes("brave", nov), 2, "lo agotado no se cuenta");
        let dic = crate::buscar::legal::fecha_a_unix("2026-12-01").unwrap();
        assert_eq!(u.reservar("brave", Some(2), dic).unwrap(), Ok(1));
        assert_eq!(u.reservar("wikipedia", None, dic).unwrap(), Ok(1));
        u.anotar_hash("brave", "sha256:x", dic).unwrap();
        assert_eq!(u.hash_visto("brave").as_deref(), Some("sha256:x"));
    }

    #[test]
    fn borrar_es_por_ruta_exacta_y_dice_lo_que_libera() {
        let d = crate::CarpetaPrueba::nueva("gotv-buscar-borrar");
        let a = Almacen::en(d.ruta());
        a.cache().guardar(&ficha("si", None), "x", 1, &[h(1)], 1_000).unwrap();
        std::fs::create_dir_all(a.indice_dir.join("sub")).unwrap();
        std::fs::write(a.indice_dir.join("sub/a.bin"), [0u8; 100]).unwrap();
        // vecinos que NO son suyos: se quedan
        let vecino = a.cache_db.with_file_name("busquedas.db.respaldo");
        std::fs::write(&vecino, "mío").unwrap();
        assert_eq!(a.uso().reservar("brave", None, 1_000).unwrap(), Ok(1));
        let (c, i) = a.borrar_cache_e_indice().unwrap();
        assert!(c > 0 && i == 100);
        assert!(!a.cache_db.exists() && !a.indice_dir.exists());
        assert!(vecino.exists() && a.uso_db.exists());
        assert_eq!(a.borrar_cache_e_indice().unwrap(), (0, 0), "dos veces no falla");
    }
}
