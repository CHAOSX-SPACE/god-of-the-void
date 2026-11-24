//! El motor de `gotv-duckdb`: DuckDB en ESTE proceso aparte (nunca dentro de la app). Archivos `.duckdb`
//! del proyecto y, sobre todo, CSV y Parquet del proyecto consultados con SQL, sin cargarlos en ninguna base.
//!
//! Riesgo que el modo de sólo lectura NO cubre: DuckDB lee archivos por sí mismo (`read_csv`, `read_text`,
//! `glob`…), así que una consulta podría leer cualquier archivo del Mac (`~/.ssh/…`). Defensa, en la
//! propia configuración y bloqueada: `allowed_directories = [proyecto]` + `enable_external_access = false`
//! (fuera de esa carpeta no se lee ni se escribe nada), sin instalar ni cargar extensiones (nada de red),
//! y `lock_configuration = true` (la consulta no puede deshacerlo). Las rutas relativas se resuelven
//! desde el proyecto (`file_search_path`). Además, un tope de memoria y de hilos: DuckDB por omisión toma
//! el 80 % de la RAM y todos los núcleos, y en esta Mac los comparte con el modelo.
use anyhow::Result;
use duckdb::{AccessMode, Config, Connection};
use gotv_datos::duckdb::{rutas_absolutas, texto_sql};
use gotv_datos::sqlite::Escritura;
use gotv_datos::{clasificar, Limites, Motor, Resultado, Valor};
use std::path::Path;
use std::time::Duration;

/// La RAM del equipo, en bytes (0 si no se pudo leer).
fn ram() -> u64 {
    let mut v: u64 = 0;
    let mut n = std::mem::size_of::<u64>();
    let nombre = c"hw.memsize";
    // SAFETY: sysctlbyname escribe como mucho `n` bytes en `v`, que mide exactamente eso
    let r = unsafe { libc::sysctlbyname(nombre.as_ptr(), (&mut v as *mut u64).cast(), &mut n, std::ptr::null_mut(), 0) };
    if r == 0 { v } else { 0 }
}

/// Tope de memoria de DuckDB: 1/8 de la RAM (lo mismo que un servidor de lenguaje), entre 1 y 16 GB.
pub fn tope_memoria_mb() -> u64 {
    (ram() / 8 / (1 << 20)).clamp(1024, 16 * 1024)
}

/// Hilos de DuckDB: la mitad de los núcleos (el resto, para la app y el modelo).
pub fn hilos() -> usize {
    std::thread::available_parallelism().map(|n| n.get() / 2).unwrap_or(2).max(1)
}

/// Abre DuckDB: el archivo `.duckdb` dado (o en memoria, para consultar CSV/Parquet) con el proyecto como
/// única carpeta alcanzable.
pub fn abrir(root: &Path, archivo: Option<&Path>, escribir: bool) -> Result<Connection> {
    let raiz = root.canonicalize()?;
    let mut cfg = Config::default().enable_autoload_extension(false)?;
    if archivo.is_some() {
        cfg = cfg.access_mode(if escribir { AccessMode::ReadWrite } else { AccessMode::ReadOnly })?;
    }
    let c = match archivo {
        Some(a) => Connection::open_with_flags(a, cfg)?,
        None => Connection::open_in_memory_with_flags(cfg)?,
    };
    let r = texto_sql(&raiz.display().to_string());
    c.execute_batch(&format!(
        "SET allowed_directories = [{r}]; SET file_search_path = {r}; SET enable_external_access = false; SET allow_community_extensions = false; SET autoinstall_known_extensions = false; SET autoload_known_extensions = false; SET memory_limit = '{}MB'; SET threads = {}; SET lock_configuration = true;",
        tope_memoria_mb(),
        hilos()
    ))?;
    Ok(c)
}

fn valor(tipo: &str, v: Option<String>) -> Valor {
    let t = tipo.to_ascii_uppercase();
    match v {
        None => Valor::Nulo,
        Some(x) if t == "BLOB" => Valor::Binario(x.matches("\\x").count().max(x.len() / 4)),
        Some(x) if ["TINYINT", "SMALLINT", "INTEGER", "BIGINT", "UTINYINT", "USMALLINT", "UINTEGER"].contains(&t.as_str()) => x.parse().map(Valor::Entero).unwrap_or(Valor::Texto(x)),
        // NaN e infinito no viajan en JSON: van como texto
        Some(x) if t == "DOUBLE" || t == "FLOAT" => x.parse::<f64>().ok().filter(|f| f.is_finite()).map(Valor::Real).unwrap_or(Valor::Texto(x)),
        Some(x) => Valor::Texto(x),
    }
}

/// Corre `f` con un tope de tiempo: si vence, interrumpe la consulta en curso.
fn con_tope<T>(c: &Connection, ms: u64, f: impl FnOnce() -> Result<T>) -> Result<T> {
    let h = c.interrupt_handle();
    let (tx, rx) = std::sync::mpsc::channel::<()>();
    let vigia = std::thread::spawn(move || {
        if rx.recv_timeout(Duration::from_millis(ms)).is_err() {
            h.interrupt();
            return true;
        }
        false
    });
    let r = f();
    let _ = tx.send(());
    let vencio = vigia.join().unwrap_or(false);
    match r {
        Err(_) if vencio => Err(gotv_engine::anyhow_tr!("la consulta pasó de {ms} ms y se cortó: acótala (WHERE, LIMIT)", "the query took over {ms} ms and was stopped: narrow it (WHERE, LIMIT)")),
        otro => otro,
    }
}

/// Consulta de sólo lectura con topes. Los valores llegan como texto (`COLUMNS(*)::VARCHAR`) y se tipan con
/// `DESCRIBE`, así cualquier tipo de DuckDB (listas, structs, fechas, decimales) se ve.
pub fn consultar(c: &Connection, root: &Path, sql: &str, l: &Limites) -> Result<Resultado> {
    let sql = &rutas_absolutas(sql, root);
    match clasificar::clasificar(sql, Motor::Duckdb)? {
        clasificar::Clase::Lectura => {}
        otra => gotv_engine::bail_tr!("«consultar» sólo lee y esto no es una lectura ({otra:?}); para cambiar datos, «ejecutar»", "«consultar» only reads and this is not a read ({otra:?}); to change data, «ejecutar»"),
    }
    let sql = sql.trim().trim_end_matches(';').trim();
    con_tope(c, l.ms, || {
        let mut d = c.prepare(&format!("DESCRIBE ({sql})"))?;
        let columnas: Vec<(String, String)> = d.query_map([], |f| Ok((f.get::<_, String>(0)?, f.get::<_, String>(1)?)))?.collect::<std::result::Result<_, _>>()?;
        let mut st = c.prepare(&format!("SELECT COLUMNS(*)::VARCHAR FROM ({sql}) AS gotv_t LIMIT {}", l.filas + 1))?;
        let n = columnas.len();
        let mut filas = Vec::new();
        let mut rows = st.query([])?;
        let mut cortado = false;
        while let Some(f) = rows.next()? {
            if filas.len() == l.filas {
                cortado = true;
                break;
            }
            filas.push((0..n).map(|i| valor(&columnas[i].1, f.get::<_, Option<String>>(i).ok().flatten())).collect());
        }
        let contadas = if cortado { c.query_row(&format!("SELECT count(*) FROM ({sql}) AS gotv_t"), [], |f| f.get::<_, i64>(0)).ok().map(|n| n as usize) } else { None };
        Ok(Resultado { columnas, filas, cortado, contadas })
    })
}

/// El esquema: las tablas y vistas de la base (`.duckdb`) o, con `archivo`, las columnas de un CSV/Parquet.
pub fn esquema(c: &Connection, tabla: Option<&str>) -> Result<String> {
    let mut s = String::new();
    let mut st = c.prepare(
        "SELECT t.table_name, t.table_type, string_agg(c.column_name || ' ' || c.data_type || CASE WHEN c.is_nullable = 'NO' THEN ' NOT NULL' ELSE '' END, ', ' ORDER BY c.ordinal_position)
         FROM information_schema.tables t JOIN information_schema.columns c ON c.table_schema = t.table_schema AND c.table_name = t.table_name
         WHERE t.table_schema NOT IN ('information_schema', 'pg_catalog') AND (? IS NULL OR t.table_name = ?)
         GROUP BY t.table_name, t.table_type ORDER BY t.table_name LIMIT 500",
    )?;
    let filas = st.query_map(duckdb::params![tabla, tabla], |f| Ok((f.get::<_, String>(0)?, f.get::<_, String>(1)?, f.get::<_, String>(2)?)))?;
    for f in filas {
        let (nombre, tipo, cols) = f?;
        let tipo = if tipo.contains("VIEW") { "view" } else { "table" };
        let n = c.query_row(&format!("SELECT count(*) FROM \"{}\"", nombre.replace('"', "\"\"")), [], |r| r.get::<_, i64>(0)).map(|n| n.to_string()).unwrap_or_else(|_| "?".into());
        s.push_str(&gotv_engine::tr!("{tipo} {nombre} ({n} filas): {cols}\n", "{tipo} {nombre} ({n} rows): {cols}\n"));
    }
    if s.is_empty() {
        return Ok(match tabla {
            Some(t) => gotv_engine::tr!("no hay tabla ni vista «{t}»", "there is no table or view «{t}»"),
            None => gotv_engine::i18n::t("(la base no tiene tablas; los CSV y Parquet del proyecto se consultan directo: SELECT * FROM 'datos/archivo.csv')", "(the database has no tables; project CSV and Parquet files are queried directly: SELECT * FROM 'data/file.csv')").into(),
        });
    }
    Ok(s)
}

/// Las columnas de un archivo CSV/Parquet/JSON del proyecto (ruta relativa al proyecto).
pub fn columnas_de_archivo(c: &Connection, root: &Path, archivo: &str) -> Result<String> {
    let ruta = root.canonicalize()?.join(archivo).display().to_string();
    let mut st = c.prepare(&format!("DESCRIBE SELECT * FROM {}", texto_sql(&ruta)))?;
    let cols: Vec<String> = st.query_map([], |f| Ok(format!("{} {}", f.get::<_, String>(0)?, f.get::<_, String>(1)?)))?.collect::<std::result::Result<_, _>>()?;
    let n: i64 = c.query_row(&format!("SELECT count(*) FROM {}", texto_sql(&ruta)), [], |r| r.get(0))?;
    Ok(gotv_engine::tr!("{archivo} ({n} filas): {}", "{archivo} ({n} rows): {}", cols.join(", ")))
}

pub fn explicar(c: &Connection, root: &Path, sql: &str, analizar: bool) -> Result<String> {
    let sql = &rutas_absolutas(sql, root);
    if clasificar::clasificar(sql, Motor::Duckdb)? != clasificar::Clase::Lectura {
        gotv_engine::bail_tr!("«explicar» es para una lectura", "«explicar» is for a read");
    }
    let pre = if analizar { "EXPLAIN ANALYZE" } else { "EXPLAIN" };
    let mut st = c.prepare(&format!("{pre} {}", sql.trim().trim_end_matches(';')))?;
    let lineas: Vec<String> = st.query_map([], |f| f.get::<_, String>(1))?.collect::<std::result::Result<_, _>>()?;
    Ok(lineas.join("\n"))
}

/// Escribe en un `.duckdb` dentro de una transacción; `confirmar` decide `COMMIT` o `ROLLBACK`.
pub fn ejecutar(c: &mut Connection, root: &Path, sql: &str, confirmar: impl FnOnce(&Escritura) -> bool) -> Result<Escritura> {
    let sql = &rutas_absolutas(sql, root);
    let (ddl, peligrosa) = match clasificar::clasificar(sql, Motor::Duckdb)? {
        clasificar::Clase::Escritura { ddl, peligrosa } => (ddl, peligrosa),
        clasificar::Clase::Lectura => gotv_engine::bail_tr!("eso es una lectura: va por «consultar»", "that is a read: use «consultar»"),
        clasificar::Clase::Transaccion => gotv_engine::bail_tr!("la transacción la lleva GOTV: quita BEGIN/COMMIT/ROLLBACK del SQL", "GOTV runs the transaction: remove BEGIN/COMMIT/ROLLBACK from the SQL"),
        clasificar::Clase::Prohibida(por) => gotv_engine::bail_tr!("no se ejecuta: {por}", "not run: {por}"),
    };
    let tx = c.transaction()?;
    let filas = tx.execute(sql.trim().trim_end_matches(';'), [])?;
    let mut e = Escritura { filas, ddl, peligrosa, confirmada: false };
    if confirmar(&e) {
        tx.commit()?;
        e.confirmada = true;
    } else {
        tx.rollback()?;
    }
    Ok(e)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn proyecto() -> gotv_engine::prueba::CarpetaPrueba {
        let d = gotv_engine::prueba::CarpetaPrueba::nueva("gotv-duckdb");
        std::fs::create_dir_all(d.join("datos")).unwrap();
        let mut csv = String::from("id,ciudad,monto,correo\n");
        for i in 1..=500 {
            csv.push_str(&format!("{i},{},{}.5,u{i}@x.co\n", ["Bogotá", "Lima", "Quito"][i % 3], i));
        }
        std::fs::write(d.join("datos/ventas.csv"), csv).unwrap();
        d
    }

    /// CSV del proyecto con SQL, topes, y la CERCA: nada fuera del proyecto, ni por `read_csv`, ni por
    /// `read_text`, ni por `COPY … TO`, ni deshaciendo la configuración.
    #[test]
    fn csv_del_proyecto_si_lo_demas_del_mac_no() {
        let d = proyecto();
        let fuera = gotv_engine::prueba::CarpetaPrueba::nueva("gotv-duckdb-fuera");
        std::fs::write(fuera.join("secreto.csv"), "clave\nnadie-debe-verla\n").unwrap();
        let c = abrir(&d, None, false).unwrap();
        let r = consultar(&c, &d, "SELECT ciudad, count(*) AS n, sum(monto) AS total FROM 'datos/ventas.csv' GROUP BY ciudad ORDER BY ciudad", &Limites::default()).unwrap();
        assert_eq!(r.filas.len(), 3);
        assert_eq!(r.filas[0][0], Valor::Texto("Bogotá".into()));
        assert_eq!(r.columnas[1].1, "BIGINT");
        let r = consultar(&c, &d, "SELECT * FROM 'datos/ventas.csv'", &Limites::default()).unwrap();
        assert_eq!((r.filas.len(), r.cortado, r.contadas), (200, true, Some(500)));
        assert!(columnas_de_archivo(&c, &d, "datos/ventas.csv").unwrap().contains("(500 filas): id BIGINT, ciudad VARCHAR"));
        let ruta = fuera.join("secreto.csv").display().to_string();
        // subiendo con «..» desde el proyecto, relativa o absoluta
        let nombre_fuera = fuera.file_name().unwrap().to_string_lossy().into_owned();
        let truco_abs = format!("{}/../{nombre_fuera}/secreto.csv", d.canonicalize().unwrap().display());
        for sql in [format!("SELECT * FROM '../{nombre_fuera}/secreto.csv'"), format!("SELECT * FROM read_csv('{truco_abs}')")] {
            let e = consultar(&c, &d, &sql, &Limites::default());
            assert!(e.is_err(), "{sql} → {:?}", e.map(|r| r.filas));
        }
        for sql in [format!("SELECT * FROM read_csv('{ruta}')"), format!("SELECT * FROM read_text('{ruta}')"), "SELECT * FROM read_text('/etc/hosts')".to_string()] {
            let e = consultar(&c, &d, &sql, &Limites::default());
            assert!(e.is_err(), "{sql} → {:?}", e.map(|r| r.filas));
        }
        // por debajo del clasificador, directo al motor: igual se niega
        assert!(c.execute_batch(&format!("COPY (SELECT 1) TO '{}'", fuera.join("x.csv").display())).is_err());
        assert!(c.execute_batch("SET enable_external_access = true").is_err(), "la configuración está bloqueada");
        assert!(c.execute_batch("INSTALL httpfs").is_err());
        assert!(c.execute_batch("SET memory_limit = '1TB'").is_err(), "el tope de memoria tampoco se deshace");
        assert!(!fuera.join("x.csv").exists());
    }

    #[test]
    fn un_archivo_duckdb_en_solo_lectura_y_escritura_en_transaccion() {
        let d = proyecto();
        let f = d.join("datos/almacen.duckdb");
        {
            let mut c = abrir(&d, Some(&f), true).unwrap();
            c.execute_batch(&rutas_absolutas("CREATE TABLE ventas AS SELECT * FROM 'datos/ventas.csv'", &d)).unwrap();
            let e = ejecutar(&mut c, &d, "DELETE FROM ventas WHERE id > 490", |_| false).unwrap();
            assert_eq!((e.filas, e.confirmada), (10, false));
            assert!(ejecutar(&mut c, &d, "DELETE FROM ventas WHERE id > 490", |_| true).unwrap().confirmada);
        }
        let c = abrir(&d, Some(&f), false).unwrap();
        assert!(esquema(&c, None).unwrap().contains("table ventas (490 filas): id BIGINT"));
        assert!(c.execute_batch("DELETE FROM ventas").is_err(), "abierta en sólo lectura");
        let t = std::time::Instant::now();
        let e = consultar(&c, &d, "SELECT count(*) FROM range(10000000000) a, range(1000) b", &Limites { ms: 300, ..Limites::default() }).unwrap_err().to_string();
        assert!(t.elapsed() < Duration::from_secs(5) && e.contains("300 ms"), "{e} {:?}", t.elapsed());
        assert!(explicar(&c, &d, "SELECT * FROM ventas WHERE id = 3", false).unwrap().to_uppercase().contains("SCAN"));
    }
}
