//! Qué es una sentencia SQL ANTES de mandarla: `sqlparser` la analiza y GOTV decide si cabe en
//! «consultar» (sólo lectura) o si es una escritura (y cuán peligrosa). Ante la duda, se rechaza:
//! un dialecto que el analizador no entiende no pasa por «consultar».
use sqlparser::ast::{Expr, LimitClause, Query, SetExpr, Statement, Value};
use sqlparser::dialect::{Dialect, GenericDialect, MySqlDialect, PostgreSqlDialect, SQLiteDialect};
use sqlparser::parser::Parser;

use crate::Motor;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Clase {
    /// `SELECT`, `WITH` sin escritura, `EXPLAIN` de un `SELECT`, `SHOW`, `DESCRIBE`.
    Lectura,
    /// Cambia datos o estructura. `peligrosa` = `DROP`, `TRUNCATE`, `DELETE`/`UPDATE` sin `WHERE`.
    Escritura { ddl: bool, peligrosa: Option<String> },
    /// `BEGIN`, `COMMIT`, `ROLLBACK`, `SAVEPOINT`: la transacción la lleva GOTV, no el guion (un
    /// `.sql` «de prueba» con su propio `COMMIT` confirmó de verdad sobre una base de esta casa).
    Transaccion,
    /// Lo que no debe pasar nunca por aquí: `ATTACH`, `SET`/`PRAGMA` de sesión, `COPY … TO PROGRAM`…
    Prohibida(String),
}

fn dialecto(m: Motor) -> Box<dyn Dialect> {
    match m {
        Motor::Sqlite => Box::new(SQLiteDialect {}),
        Motor::Postgres => Box::new(PostgreSqlDialect {}),
        Motor::Mysql => Box::new(MySqlDialect {}),
        Motor::Duckdb => Box::new(GenericDialect {}),
    }
}

/// PRAGMA de SQLite que reciben una tabla o un índice y sólo informan.
pub const PRAGMAS_CON_TABLA: &[&str] = &["table_info", "table_xinfo", "index_list", "index_info", "index_xinfo", "foreign_key_list", "foreign_key_check", "integrity_check", "quick_check"];
/// PRAGMA de SQLite que, SIN argumento, sólo informan un valor (con `= x` o `(x)` lo cambiarían).
pub const PRAGMAS_DE_LECTURA: &[&str] = &["table_list", "database_list", "collation_list", "function_list", "module_list", "pragma_list", "compile_options", "user_version", "schema_version", "application_id", "page_count", "page_size", "freelist_count", "encoding", "foreign_keys", "journal_mode", "synchronous", "auto_vacuum", "cache_size", "busy_timeout", "data_version", "integrity_check", "quick_check", "foreign_key_check"];

/// `PRAGMA` de SQLite, a mano: el analizador no entiende todos sus valores (`journal_mode = DELETE`).
fn pragma_sqlite(sql: &str) -> Option<Clase> {
    let s = sql.trim().trim_end_matches(';').trim();
    let resto = s.get(..6).filter(|p| p.eq_ignore_ascii_case("pragma")).map(|_| s[6..].trim())?;
    let nombre: String = resto.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '.').collect();
    let nombre = nombre.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    let tras = resto[resto.find(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '.')).unwrap_or(resto.len())..].trim();
    let lectura = if tras.is_empty() {
        PRAGMAS_DE_LECTURA.contains(&nombre.as_str()) || PRAGMAS_CON_TABLA.contains(&nombre.as_str())
    } else {
        tras.starts_with('(') && tras.ends_with(')') && PRAGMAS_CON_TABLA.contains(&nombre.as_str())
    };
    Some(if lectura { Clase::Lectura } else { Clase::Prohibida(gotv_engine::i18n::t("un PRAGMA que cambia la conexión o la base", "a PRAGMA that changes the connection or the database").into()) })
}

/// Analiza y clasifica. Error si no se entiende o si trae VARIAS sentencias (una por llamada).
pub fn clasificar(sql: &str, motor: Motor) -> anyhow::Result<Clase> {
    // MySQL EJECUTA lo que va dentro de `/*! … */` (y MariaDB de `/*M! … */`): el analizador lo ve
    // como comentario. Nunca pasa.
    if motor == Motor::Mysql && (sql.contains("/*!") || sql.contains("/*M!")) {
        return Ok(Clase::Prohibida(gotv_engine::i18n::t("comentario ejecutable de MySQL (/*! … */): lo que va dentro se ejecuta", "MySQL executable comment (/*! … */): its content runs").into()));
    }
    if motor == Motor::Sqlite && !sql.trim().trim_end_matches(';').contains(';') {
        if let Some(c) = pragma_sqlite(sql) {
            return Ok(c);
        }
    }
    let d = dialecto(motor);
    let sentencias = Parser::parse_sql(d.as_ref(), sql).map_err(|e| gotv_engine::anyhow_tr!("no entiendo esa sentencia SQL ({e}); si es una escritura, va por «ejecutar»", "I do not understand that SQL statement ({e}); if it is a write, use «ejecutar»"))?;
    match sentencias.as_slice() {
        [] => gotv_engine::bail_tr!("no hay ninguna sentencia", "there is no statement"),
        [una] => Ok(clase_de(una)),
        _ => gotv_engine::bail_tr!(
            "{} sentencias en una sola llamada: manda UNA por vez (así cada una pasa por su permiso)",
            "{} statements in a single call: send ONE at a time (so each one goes through its permission)",
            sentencias.len()
        ),
    }
}

/// ¿Escribe una consulta? (un `WITH … INSERT/UPDATE/DELETE` también escribe, y un `SELECT … INTO`
/// crea una tabla o un archivo)
fn consulta_escribe(q: &Query) -> bool {
    fn cuerpo_escribe(s: &SetExpr) -> bool {
        match s {
            SetExpr::Insert(_) | SetExpr::Update(_) | SetExpr::Delete(_) | SetExpr::Merge(_) => true,
            SetExpr::Select(s) => s.into.is_some(),
            SetExpr::SetOperation { left, right, .. } => cuerpo_escribe(left) || cuerpo_escribe(right),
            SetExpr::Query(q) => consulta_escribe(q),
            _ => false,
        }
    }
    cuerpo_escribe(&q.body) || q.with.as_ref().is_some_and(|w| w.cte_tables.iter().any(|c| consulta_escribe(&c.query))) || q.locks.iter().next().is_some()
}

fn clase_de(s: &Statement) -> Clase {
    match s {
        Statement::Query(q) if !consulta_escribe(q) => Clase::Lectura,
        Statement::Query(_) => Clase::Escritura { ddl: false, peligrosa: None },
        Statement::Explain { statement, analyze, .. } => match clase_de(statement) {
            Clase::Lectura => Clase::Lectura,
            // EXPLAIN ANALYZE de una escritura la EJECUTA
            _ if *analyze => Clase::Prohibida(gotv_engine::i18n::t("EXPLAIN ANALYZE de una escritura la ejecuta de verdad", "EXPLAIN ANALYZE of a write really runs it").into()),
            _ => Clase::Lectura,
        },
        Statement::ExplainTable { .. } | Statement::ShowTables { .. } | Statement::ShowColumns { .. } | Statement::ShowCreate { .. } | Statement::ShowVariable { .. } | Statement::ShowDatabases { .. } | Statement::ShowSchemas { .. } | Statement::ShowViews { .. } => Clase::Lectura,
        Statement::StartTransaction { .. } | Statement::Commit { .. } | Statement::Rollback { .. } | Statement::Savepoint { .. } | Statement::ReleaseSavepoint { .. } => Clase::Transaccion,
        Statement::AttachDatabase { .. } | Statement::AttachDuckDBDatabase { .. } | Statement::DetachDuckDBDatabase { .. } => Clase::Prohibida(gotv_engine::i18n::t("ATTACH abre otra base fuera de la conexión", "ATTACH opens another database outside the connection").into()),
        Statement::Pragma { value: Some(_), .. } => Clase::Prohibida(gotv_engine::i18n::t("un PRAGMA que cambia la conexión", "a PRAGMA that changes the connection").into()),
        Statement::Pragma { .. } => Clase::Lectura,
        Statement::Set(_) => Clase::Prohibida(gotv_engine::i18n::t("SET cambia la sesión", "SET changes the session").into()),
        Statement::Copy { .. } => Clase::Prohibida(gotv_engine::i18n::t("COPY lee o escribe archivos del servidor", "COPY reads or writes server files").into()),
        Statement::Drop { .. } | Statement::Truncate { .. } => Clase::Escritura { ddl: true, peligrosa: Some(s.to_string().split_whitespace().take(3).collect::<Vec<_>>().join(" ")) },
        Statement::Delete(d) if d.selection.is_none() => Clase::Escritura { ddl: false, peligrosa: Some(gotv_engine::i18n::t("DELETE sin WHERE: borra TODAS las filas", "DELETE without WHERE: deletes ALL rows").into()) },
        Statement::Update(u) if u.selection.is_none() => Clase::Escritura { ddl: false, peligrosa: Some(gotv_engine::i18n::t("UPDATE sin WHERE: cambia TODAS las filas", "UPDATE without WHERE: changes ALL rows").into()) },
        Statement::Insert(_) | Statement::Update(_) | Statement::Delete(_) | Statement::Merge { .. } => Clase::Escritura { ddl: false, peligrosa: None },
        Statement::CreateTable(_) | Statement::CreateIndex(_) | Statement::CreateView { .. } | Statement::AlterTable { .. } | Statement::CreateTrigger { .. } | Statement::CreateSchema { .. } | Statement::CreateSequence { .. } | Statement::AlterIndex { .. } | Statement::AlterView { .. } => Clase::Escritura { ddl: true, peligrosa: None },
        // lo demás (funciones, permisos, roles, VACUUM…): escritura, y se trata como peligrosa
        otra => Clase::Escritura { ddl: true, peligrosa: Some(otra.to_string().split_whitespace().take(2).collect::<Vec<_>>().join(" ")) },
    }
}

/// Una lectura con el `LIMIT` puesto por GOTV (si no lo trae, o si trae uno mayor): en MySQL un resultado
/// que se deja a medias se drena ENTERO, así que el tope tiene que ir en la propia sentencia. `None` si
/// no es una consulta que lo admita (`SHOW`, `EXPLAIN`…): ésas devuelven poco.
pub fn con_limite(sql: &str, motor: Motor, n: usize) -> Option<String> {
    let d = dialecto(motor);
    let mut v = Parser::parse_sql(d.as_ref(), sql).ok()?;
    let [Statement::Query(q)] = v.as_mut_slice() else { return None };
    let tope = || Expr::value(Value::Number(n.to_string(), false));
    let mayor = |e: &Expr| match e {
        Expr::Value(v) => match &v.value {
            Value::Number(x, _) => x.parse::<u64>().map_or(true, |x| x > n as u64),
            _ => true,
        },
        _ => true,
    };
    match &mut q.limit_clause {
        None => q.limit_clause = Some(LimitClause::LimitOffset { limit: Some(tope()), offset: None, limit_by: vec![] }),
        Some(LimitClause::LimitOffset { limit, .. }) => {
            if limit.as_ref().map_or(true, mayor) {
                *limit = Some(tope());
            }
        }
        Some(LimitClause::OffsetCommaLimit { limit, .. }) => {
            if mayor(limit) {
                *limit = tope();
            }
        }
    }
    Some(v[0].to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn la_lectura_y_la_escritura_se_distinguen() {
        let c = |s: &str| clasificar(s, Motor::Sqlite).unwrap();
        assert_eq!(c("SELECT * FROM t WHERE id = 1"), Clase::Lectura);
        assert_eq!(c("WITH x AS (SELECT 1) SELECT * FROM x"), Clase::Lectura);
        assert_eq!(c("EXPLAIN QUERY PLAN SELECT * FROM t"), Clase::Lectura);
        assert_eq!(c("INSERT INTO t VALUES (1)"), Clase::Escritura { ddl: false, peligrosa: None });
        assert_eq!(c("UPDATE t SET a = 1 WHERE id = 2"), Clase::Escritura { ddl: false, peligrosa: None });
        assert!(matches!(c("DELETE FROM t"), Clase::Escritura { peligrosa: Some(_), .. }));
        assert!(matches!(c("UPDATE t SET a = 1"), Clase::Escritura { peligrosa: Some(_), .. }));
        assert!(matches!(c("DROP TABLE t"), Clase::Escritura { ddl: true, peligrosa: Some(_) }));
        assert_eq!(c("CREATE TABLE t (id INTEGER)"), Clase::Escritura { ddl: true, peligrosa: None });
        assert_eq!(c("COMMIT"), Clase::Transaccion);
        assert_eq!(c("BEGIN"), Clase::Transaccion);
        assert!(matches!(c("ATTACH DATABASE '/tmp/x.db' AS x"), Clase::Prohibida(_)));
        assert!(matches!(c("PRAGMA journal_mode = DELETE"), Clase::Prohibida(_)));
        assert_eq!(c("PRAGMA table_info(t)"), Clase::Lectura);
        assert_eq!(c("pragma user_version;"), Clase::Lectura);
        assert!(matches!(c("PRAGMA user_version = 9"), Clase::Prohibida(_)));
        assert!(matches!(c("PRAGMA wal_checkpoint(TRUNCATE)"), Clase::Prohibida(_)));
        assert!(matches!(c("PRAGMA main.journal_mode=WAL"), Clase::Prohibida(_)));
        assert!(clasificar("PRAGMA user_version; DELETE FROM t", Motor::Sqlite).is_err());
    }

    #[test]
    fn el_limite_lo_pone_gotv() {
        let l = |s: &str| con_limite(s, Motor::Mysql, 201);
        assert_eq!(l("SELECT * FROM t").as_deref(), Some("SELECT * FROM t LIMIT 201"));
        assert_eq!(l("SELECT * FROM t LIMIT 5").as_deref(), Some("SELECT * FROM t LIMIT 5"));
        assert_eq!(l("SELECT * FROM t LIMIT 100000 OFFSET 10").as_deref(), Some("SELECT * FROM t LIMIT 201 OFFSET 10"));
        assert_eq!(l("SELECT * FROM t LIMIT 10, 100000").as_deref(), Some("SELECT * FROM t LIMIT 10, 201"));
        assert!(l("WITH x AS (SELECT 1) SELECT * FROM x").unwrap().ends_with("LIMIT 201"));
        assert_eq!(l("SHOW TABLES"), None);
    }

    /// La batería de ataque de la ficha: lo que intenta escribir disfrazado de lectura.
    #[test]
    fn la_bateria_de_ataque_no_pasa_como_lectura() {
        // multi-sentencia
        assert!(clasificar("SELECT 1; DROP TABLE t", Motor::Sqlite).is_err());
        assert!(clasificar("SELECT 1; DELETE FROM t", Motor::Postgres).is_err());
        // SET de sesión
        assert!(matches!(clasificar("SET default_transaction_read_only = off", Motor::Postgres).unwrap(), Clase::Prohibida(_)));
        // ATTACH
        assert!(matches!(clasificar("ATTACH 'otra.db' AS o", Motor::Sqlite).unwrap(), Clase::Prohibida(_)));
        // WITH … DELETE … RETURNING
        assert_ne!(clasificar("WITH borrados AS (DELETE FROM t RETURNING *) SELECT * FROM borrados", Motor::Postgres).unwrap(), Clase::Lectura);
        // EXPLAIN ANALYZE de una escritura
        assert!(matches!(clasificar("EXPLAIN ANALYZE DELETE FROM t", Motor::Postgres).unwrap(), Clase::Prohibida(_)));
        // una función con efectos (`SELECT pg_terminate_backend(…)`) SÍ es lectura para el analizador:
        // la frena la transacción de sólo lectura o un usuario de sólo lectura (la ficha lo dice)
        assert_eq!(clasificar("SELECT nextval('s')", Motor::Postgres).unwrap(), Clase::Lectura);
        // SELECT … INTO crea una tabla (Postgres) o un archivo (MySQL)
        assert_ne!(clasificar("SELECT * INTO copia FROM t", Motor::Postgres).unwrap(), Clase::Lectura);
        // el comentario ejecutable de MySQL
        assert!(matches!(clasificar("SELECT 1 /*! ; DROP TABLE t */", Motor::Mysql).unwrap(), Clase::Prohibida(_)));
        // lo que no se entiende, no pasa
        assert!(clasificar("SELEC * FORM t", Motor::Sqlite).is_err());
    }
}
