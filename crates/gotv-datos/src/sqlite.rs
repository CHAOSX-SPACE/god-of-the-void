//! SQLite en proceso (`rusqlite`, el SQLite que ya trae GOTV dentro): nada que instalar.
//!
//! Sólo lectura de VERDAD, por dos vías a la vez: la conexión se abre con `SQLITE_OPEN_READ_ONLY`
//! (el motor no escribe aunque se lo pidan) y un AUTORIZADOR niega, sentencia por sentencia, todo lo
//! que no sea leer: escribir, `ATTACH`, `PRAGMA` que cambian algo, crear o borrar. Además el
//! clasificador ([`crate::clasificar`]) ya rechazó antes lo que no es lectura.
//! Topes: filas (se pide una de más para saber si hay más), tiempo (un manejador de progreso corta
//! la consulta al pasar `ms`) y caracteres de la tabla.
//! Escribir: en una conexión aparte, dentro de `BEGIN`; se cuenta lo tocado y SÓLO entonces se decide
//! `COMMIT` o `ROLLBACK` (quien llama decide, con el número delante).
use crate::{clasificar, Limites, Motor, Resultado, Valor};
use anyhow::{Context, Result};
use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
use rusqlite::types::ValueRef;
use rusqlite::{Connection, OpenFlags};
use std::path::Path;
use std::time::{Duration, Instant};

/// El autorizador de sólo lectura: lo que no sea leer (o funciones de lectura), se niega.
fn solo_leer(ctx: AuthContext<'_>) -> Authorization {
    match ctx.action {
        AuthAction::Select | AuthAction::Read { .. } | AuthAction::Function { .. } | AuthAction::Recursive => Authorization::Allow,
        // PRAGMA de sólo lectura (`table_info`, `index_list`…): sin argumento de valor
        // los que reciben una tabla o un índice como argumento (`table_info(t)`): el argumento no cambia nada
        AuthAction::Pragma { pragma_name, .. } if clasificar::PRAGMAS_CON_TABLA.contains(&pragma_name.to_ascii_lowercase().as_str()) => Authorization::Allow,
        // los que sólo informan un valor: sin argumento (con él, lo CAMBIARÍAN)
        AuthAction::Pragma { pragma_name, pragma_value: None } if clasificar::PRAGMAS_DE_LECTURA.contains(&pragma_name.to_ascii_lowercase().as_str()) => Authorization::Allow,
        _ => Authorization::Deny,
    }
}

fn abrir_lectura(archivo: &Path) -> Result<Connection> {
    if !archivo.is_file() {
        gotv_engine::bail_tr!("no existe la base {}", "the database {} does not exist", archivo.display());
    }
    let c = Connection::open_with_flags(archivo, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX | OpenFlags::SQLITE_OPEN_URI)
        .with_context(|| gotv_engine::tr!("no pude abrir {}", "could not open {}", archivo.display()))?;
    c.busy_timeout(Duration::from_secs(5))?;
    c.authorizer(Some(solo_leer));
    Ok(c)
}

fn valor(v: ValueRef<'_>) -> Valor {
    match v {
        ValueRef::Null => Valor::Nulo,
        ValueRef::Integer(n) => Valor::Entero(n),
        ValueRef::Real(x) => Valor::Real(x),
        ValueRef::Text(t) => Valor::Texto(String::from_utf8_lossy(t).into_owned()),
        ValueRef::Blob(b) => Valor::Binario(b.len()),
    }
}

/// Corre una sentencia de LECTURA en una conexión ya abierta, con los topes.
fn leer(c: &Connection, sql: &str, l: &Limites) -> Result<Resultado> {
    let inicio = Instant::now();
    let tope = Duration::from_millis(l.ms);
    // cada 1 000 instrucciones de la máquina virtual de SQLite se mira el reloj
    c.progress_handler(1_000, Some(move || inicio.elapsed() > tope));
    let r = (|| {
        let mut st = c.prepare(sql)?;
        let columnas: Vec<(String, String)> = st.columns().iter().map(|col| (col.name().to_string(), col.decl_type().unwrap_or("").to_string())).collect();
        let n = columnas.len();
        let mut filas = Vec::new();
        let mut cortado = false;
        let mut rows = st.query([])?;
        while let Some(row) = rows.next()? {
            if filas.len() == l.filas {
                cortado = true;
                break;
            }
            filas.push((0..n).map(|i| row.get_ref(i).map(valor).unwrap_or(Valor::Nulo)).collect());
        }
        Ok::<_, rusqlite::Error>(Resultado { columnas, filas, cortado, contadas: None })
    })();
    c.progress_handler(0, None::<fn() -> bool>);
    r.map_err(|e| match e {
        rusqlite::Error::SqliteFailure(f, _) if f.code == rusqlite::ErrorCode::OperationInterrupted => {
            gotv_engine::anyhow_tr!("la consulta pasó de {} ms y se cortó: acótala (WHERE, índice, LIMIT)", "the query took over {} ms and was stopped: narrow it (WHERE, index, LIMIT)", l.ms)
        }
        rusqlite::Error::SqliteFailure(f, m) if f.code == rusqlite::ErrorCode::AuthorizationForStatementDenied => {
            gotv_engine::anyhow_tr!("negado: «consultar» sólo lee ({})", "denied: «consultar» only reads ({})", m.unwrap_or_default())
        }
        otro => anyhow::Error::from(otro),
    })
}

/// Consulta de sólo lectura. Lo que no sea lectura se rechaza ANTES de tocar la base.
pub fn consultar(archivo: &Path, sql: &str, l: &Limites) -> Result<Resultado> {
    match clasificar::clasificar(sql, Motor::Sqlite)? {
        clasificar::Clase::Lectura => {}
        otra => gotv_engine::bail_tr!("«consultar» sólo lee y esto no es una lectura ({otra:?}); para cambiar datos, «ejecutar»", "«consultar» only reads and this is not a read ({otra:?}); to change data, «ejecutar»"),
    }
    let c = abrir_lectura(archivo)?;
    let mut r = leer(&c, sql, l)?;
    if r.cortado {
        // cuántas hay en total, si se puede contar rápido (con el mismo tope de tiempo)
        let contar = format!("SELECT COUNT(*) FROM ({})", sql.trim().trim_end_matches(';'));
        if let Ok(t) = leer(&c, &contar, &Limites { filas: 1, ms: l.ms.min(3_000), caracteres: 100 }) {
            if let Some(Some(Valor::Entero(n))) = t.filas.first().map(|f| f.first().cloned()) {
                r.contadas = Some(n as usize);
            }
        }
    }
    Ok(r)
}

/// El esquema: las tablas y vistas con sus columnas (o una sola tabla), en texto compacto.
pub fn esquema(archivo: &Path, tabla: Option<&str>) -> Result<String> {
    let c = abrir_lectura(archivo)?;
    let l = Limites { filas: 500, ms: 5_000, caracteres: 40_000 };
    let objetos = leer(&c, "SELECT type, name, sql FROM sqlite_master WHERE type IN ('table','view') AND name NOT LIKE 'sqlite_%' ORDER BY type, name", &l)?;
    let mut s = String::new();
    for f in &objetos.filas {
        let (Valor::Texto(tipo), Valor::Texto(nombre)) = (&f[0], &f[1]) else { continue };
        if tabla.is_some_and(|t| !t.eq_ignore_ascii_case(nombre)) {
            continue;
        }
        let cols = leer(&c, &format!("PRAGMA table_info(\"{}\")", nombre.replace('"', "\"\"")), &l)?;
        let desc: Vec<String> = cols
            .filas
            .iter()
            .map(|r| {
                let mut d = format!("{} {}", r[1], r[2]);
                if matches!(r[3], Valor::Entero(1)) {
                    d.push_str(" NOT NULL");
                }
                if !matches!(r[5], Valor::Entero(0)) {
                    d.push_str(" PK");
                }
                d
            })
            .collect();
        let filas = if tipo == "table" {
            leer(&c, &format!("SELECT COUNT(*) FROM \"{}\"", nombre.replace('"', "\"\"")), &Limites { filas: 1, ms: 2_000, caracteres: 100 }).ok().and_then(|r| r.filas.first().map(|f| f[0].to_string())).unwrap_or_else(|| "?".into())
        } else {
            "—".into()
        };
        s.push_str(&gotv_engine::tr!("{tipo} {nombre} ({filas} filas): {}\n", "{tipo} {nombre} ({filas} rows): {}\n", desc.join(", ")));
        if tabla.is_some() {
            if let Valor::Texto(sql) = &f[2] {
                s.push_str(&format!("{sql}\n"));
            }
            let idx = leer(&c, &format!("PRAGMA index_list(\"{}\")", nombre.replace('"', "\"\"")), &l)?;
            for i in &idx.filas {
                s.push_str(&format!("  índice {}{}\n", i[1], if matches!(i[2], Valor::Entero(1)) { " (único)" } else { "" })); // juez-idioma: datos (rótulos de un esquema, se lee igual)
            }
        }
    }
    if s.is_empty() {
        return Ok(match tabla {
            Some(t) => gotv_engine::tr!("no hay tabla ni vista «{t}»", "there is no table or view «{t}»"),
            None => gotv_engine::i18n::t("(la base está vacía)", "(the database is empty)").into(),
        });
    }
    Ok(s)
}

/// El plan de una consulta (`EXPLAIN QUERY PLAN`): por qué es lenta.
pub fn explicar(archivo: &Path, sql: &str) -> Result<String> {
    if clasificar::clasificar(sql, Motor::Sqlite)? != clasificar::Clase::Lectura {
        gotv_engine::bail_tr!("«explicar» es para una lectura", "«explicar» is for a read");
    }
    let c = abrir_lectura(archivo)?;
    let r = leer(&c, &format!("EXPLAIN QUERY PLAN {}", sql.trim().trim_end_matches(';')), &Limites::default())?;
    Ok(r.filas.iter().map(|f| f.last().map(|v| v.to_string()).unwrap_or_default()).collect::<Vec<_>>().join("\n"))
}

/// Lo que haría una escritura: filas tocadas, sin confirmar nada.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Escritura {
    pub filas: usize,
    pub ddl: bool,
    pub peligrosa: Option<String>,
    pub confirmada: bool,
}

/// Escribe dentro de `BEGIN`. Cuenta lo tocado y confirma SÓLO si `confirmar(filas)` dice que sí;
/// si no, `ROLLBACK` y la base queda como estaba. Las transacciones del guion se rechazan.
pub fn ejecutar(archivo: &Path, sql: &str, confirmar: impl FnOnce(&Escritura) -> bool) -> Result<Escritura> {
    let (ddl, peligrosa) = match clasificar::clasificar(sql, Motor::Sqlite)? {
        clasificar::Clase::Escritura { ddl, peligrosa } => (ddl, peligrosa),
        clasificar::Clase::Lectura => gotv_engine::bail_tr!("eso es una lectura: va por «consultar»", "that is a read: use «consultar»"),
        clasificar::Clase::Transaccion => gotv_engine::bail_tr!("la transacción la lleva GOTV: quita BEGIN/COMMIT/ROLLBACK del SQL", "GOTV runs the transaction: remove BEGIN/COMMIT/ROLLBACK from the SQL"),
        clasificar::Clase::Prohibida(por) => gotv_engine::bail_tr!("no se ejecuta: {por}", "not run: {por}"),
    };
    let mut c = Connection::open_with_flags(archivo, OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX)
        .with_context(|| gotv_engine::tr!("no pude abrir {}", "could not open {}", archivo.display()))?;
    c.busy_timeout(Duration::from_secs(5))?;
    let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let filas = tx.execute(sql, [])?;
    let mut e = Escritura { filas, ddl, peligrosa, confirmada: false };
    if confirmar(&e) {
        tx.commit()?;
        e.confirmada = true;
    } else {
        tx.rollback()?;
    }
    Ok(e)
}

/// Respaldo en caliente con la API de SQLite (consistente aunque otro la esté usando).
pub fn respaldo(archivo: &Path, destino: &Path) -> Result<u64> {
    if destino.exists() {
        gotv_engine::bail_tr!("{} ya existe: no lo piso", "{} already exists: I will not overwrite it", destino.display());
    }
    let origen = abrir_lectura(archivo)?;
    origen.authorizer(None::<fn(AuthContext<'_>) -> Authorization>);
    let mut copia = Connection::open(destino)?;
    {
        let b = rusqlite::backup::Backup::new(&origen, &mut copia)?;
        b.run_to_completion(256, Duration::from_millis(5), None)?;
    }
    Ok(std::fs::metadata(destino)?.len())
}

/// ¿Es SQLite este archivo? (por su cabecera, no por la extensión)
pub fn es_sqlite(archivo: &Path) -> bool {
    use std::io::Read;
    let mut cab = [0u8; 16];
    std::fs::File::open(archivo).and_then(|mut f| f.read_exact(&mut cab)).is_ok() && &cab == b"SQLite format 3\0"
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base(nombre: &str) -> (gotv_engine::prueba::CarpetaPrueba, std::path::PathBuf) {
        let d = gotv_engine::prueba::CarpetaPrueba::nueva(&format!("gotv-datos-{nombre}"));
        let f = d.join("tienda.db");
        let c = Connection::open(&f).unwrap();
        c.execute_batch(
            "CREATE TABLE clientes (id INTEGER PRIMARY KEY, nombre TEXT NOT NULL, correo TEXT, clave_hash TEXT);
             CREATE INDEX clientes_correo ON clientes (correo);
             CREATE VIEW activos AS SELECT * FROM clientes WHERE id < 100;",
        )
        .unwrap();
        for i in 1..=500 {
            c.execute("INSERT INTO clientes (nombre, correo, clave_hash) VALUES (?1, ?2, 'h')", rusqlite::params![format!("cliente {i}"), format!("c{i}@x.co")]).unwrap();
        }
        (d, f)
    }

    #[test]
    fn consulta_con_topes_y_esquema() {
        let (_d, f) = base("consulta");
        let r = consultar(&f, "SELECT id, nombre FROM clientes ORDER BY id", &Limites::default()).unwrap();
        assert_eq!((r.filas.len(), r.cortado, r.contadas), (200, true, Some(500)));
        assert_eq!(r.columnas[0], ("id".into(), "INTEGER".into()));
        let e = esquema(&f, None).unwrap();
        assert!(e.contains("table clientes (500 filas): id INTEGER PK, nombre TEXT NOT NULL, correo TEXT, clave_hash TEXT") && e.contains("view activos"), "{e}");
        let t = esquema(&f, Some("clientes")).unwrap();
        assert!(t.contains("CREATE TABLE clientes") && t.contains("clientes_correo"), "{t}");
        let p = explicar(&f, "SELECT * FROM clientes WHERE correo = 'c7@x.co'").unwrap();
        assert!(p.contains("clientes_correo"), "usa el índice: {p}");
    }

    /// La lectura no escribe aunque el clasificador se equivocara: el motor y el autorizador lo niegan.
    #[test]
    fn la_lectura_no_escribe_ni_por_debajo_del_clasificador() {
        let (_d, f) = base("solo-lectura");
        assert!(consultar(&f, "DELETE FROM clientes", &Limites::default()).is_err());
        assert!(consultar(&f, "ATTACH DATABASE '/tmp/x.db' AS x", &Limites::default()).is_err());
        // saltándose el clasificador, directo al motor: igual se niega
        let c = abrir_lectura(&f).unwrap();
        assert!(c.execute("DELETE FROM clientes", []).is_err());
        assert!(c.execute_batch("ATTACH DATABASE ':memory:' AS m").is_err());
        assert!(c.execute_batch("PRAGMA user_version = 9").is_err());
        let n: i64 = Connection::open(&f).unwrap().query_row("SELECT COUNT(*) FROM clientes", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 500, "nada se borró");
    }

    /// Una consulta que no acaba se corta en su tope de tiempo.
    #[test]
    fn una_consulta_eterna_se_corta_a_tiempo() {
        let (_d, f) = base("eterna");
        let t = Instant::now();
        let e = consultar(&f, "WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM n) SELECT COUNT(*) FROM n", &Limites { ms: 300, ..Limites::default() }).unwrap_err().to_string();
        assert!(t.elapsed() < Duration::from_secs(3), "{:?}", t.elapsed());
        assert!(e.contains("300 ms"), "{e}");
    }

    /// Escribir: dentro de una transacción; sin confirmar, la base queda igual; con el guion trayendo
    /// su propio COMMIT, se rechaza.
    #[test]
    fn escribir_en_transaccion_que_se_confirma_o_se_deshace() {
        let (_d, f) = base("escribe");
        let contar = || -> i64 { Connection::open(&f).unwrap().query_row("SELECT COUNT(*) FROM clientes", [], |r| r.get(0)).unwrap() };
        let e = ejecutar(&f, "DELETE FROM clientes WHERE id > 450", |e| {
            assert_eq!(e.filas, 50);
            false
        })
        .unwrap();
        assert!(!e.confirmada);
        assert_eq!(contar(), 500, "sin confirmar, nada cambió");
        let e = ejecutar(&f, "DELETE FROM clientes WHERE id > 450", |_| true).unwrap();
        assert!(e.confirmada && e.filas == 50);
        assert_eq!(contar(), 450);
        assert!(ejecutar(&f, "DELETE FROM clientes", |e| e.peligrosa.is_none()).unwrap().peligrosa.is_some(), "sin WHERE: peligrosa");
        assert_eq!(contar(), 450, "la peligrosa no se confirmó");
        assert!(ejecutar(&f, "COMMIT", |_| true).is_err());
        assert!(ejecutar(&f, "SELECT 1", |_| true).is_err());
    }

    #[test]
    fn respaldo_en_caliente_y_no_pisa() {
        let (_d, f) = base("respaldo");
        let destino = f.with_file_name("copia.db");
        assert!(respaldo(&f, &destino).unwrap() > 0);
        let n: i64 = Connection::open(&destino).unwrap().query_row("SELECT COUNT(*) FROM clientes", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 500);
        assert!(respaldo(&f, &destino).is_err(), "no pisa");
        assert!(es_sqlite(&f) && !es_sqlite(&f.with_file_name("no-existe.db")));
    }
}
