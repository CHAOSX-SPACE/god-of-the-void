//! F1 paso 4 · PostgreSQL por la red, con `tokio-postgres` (Rust puro) y TLS de `rustls`.
//!
//! Lectura: `BEGIN TRANSACTION READ ONLY` + `SET LOCAL statement_timeout` + UNA sentencia (el clasificador
//! ya rechazó varias). Las consultas que devuelven filas van por un CURSOR (`DECLARE … NO SCROLL CURSOR`,
//! `FETCH n+1`): el servidor no manda más filas de las pedidas, aunque la tabla tenga millones. Los valores
//! llegan como texto (protocolo simple de `FETCH`): JSONB, arreglos, geometría o `numeric` se leen igual.
//! Lo que la transacción de sólo lectura NO frena (funciones con efectos: `pg_terminate_backend`,
//! `nextval`, `dblink`): sólo un usuario de sólo lectura. `probar` dice si la conexión puede escribir y
//! da el SQL para crear uno.
//! Escribir: transacción, se cuentan las filas y quien llama decide `COMMIT` o `ROLLBACK` (como SQLite).
use crate::sqlite::Escritura;
use crate::{clasificar, Limites, Motor, Resultado, Valor};
use anyhow::{Context, Result};
use std::time::Duration;
use tokio_postgres::{Client, SimpleQueryMessage};

/// `usuario@host:puerto/base?sslmode=…` (el destino del registro, ya sin clave).
#[derive(Debug, Clone, PartialEq)]
pub struct Destino {
    pub usuario: String,
    pub host: String,
    pub puerto: u16,
    pub base: String,
    pub sslmode: String,
}

impl Destino {
    pub fn leer(destino: &str) -> Result<Destino> {
        Destino::leer_con(destino, 5432, "postgres", "postgres")
    }

    /// Con los valores por defecto de cada motor (MySQL: 3306, `root`, sin base).
    pub fn leer_con(destino: &str, puerto_defecto: u16, usuario_defecto: &str, base_defecto: &str) -> Result<Destino> {
        let d = destino.split_once("://").map(|(_, r)| r).unwrap_or(destino);
        let (d, params) = d.split_once('?').unwrap_or((d, ""));
        let (usuario, resto) = d.rsplit_once('@').map(|(u, r)| (u.split(':').next().unwrap_or(u).to_string(), r)).unwrap_or((String::new(), d));
        let (hostpuerto, base) = resto.split_once('/').unwrap_or((resto, ""));
        let (host, puerto) = if let Some(h) = hostpuerto.strip_prefix('[') {
            // IPv6 entre corchetes: `[::1]:5432`
            let (h, p) = h.split_once(']').unwrap_or((h, ""));
            (h.to_string(), p.trim_start_matches(':'))
        } else {
            let (h, p) = hostpuerto.split_once(':').unwrap_or((hostpuerto, ""));
            (h.to_string(), p)
        };
        let puerto = if puerto.is_empty() { puerto_defecto } else { puerto.parse().with_context(|| gotv_engine::tr!("puerto inválido «{puerto}»", "invalid port «{puerto}»"))? };
        let sslmode = params.split('&').find_map(|kv| kv.strip_prefix("sslmode=")).unwrap_or("prefer").to_string();
        if host.is_empty() {
            gotv_engine::bail_tr!("el destino no tiene servidor: usuario@host:puerto/base", "the destination has no server: user@host:port/database");
        }
        Ok(Destino { usuario: if usuario.is_empty() { usuario_defecto.into() } else { usuario }, host, puerto, base: if base.is_empty() { base_defecto.into() } else { base.to_string() }, sslmode })
    }

    pub fn es_local(&self) -> bool {
        matches!(self.host.as_str(), "localhost" | "127.0.0.1" | "::1") || self.host.starts_with('/')
    }
}

fn tls() -> Result<tokio_postgres_rustls::MakeRustlsConnect> {
    let raices = rustls::RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let config = rustls::ClientConfig::builder_with_provider(std::sync::Arc::new(rustls::crypto::ring::default_provider()))
        .with_safe_default_protocol_versions()?
        .with_root_certificates(raices)
        .with_no_client_auth();
    Ok(tokio_postgres_rustls::MakeRustlsConnect::new(config))
}

/// Abre una conexión (10 s de tope). `clave` viene del Llavero o del `.env`; nunca se registra.
pub async fn conectar(d: &Destino, clave: Option<&str>) -> Result<Client> {
    let mut c = tokio_postgres::Config::new();
    c.host(&d.host).port(d.puerto).user(&d.usuario).dbname(&d.base).application_name("gotv").connect_timeout(Duration::from_secs(10));
    if let Some(k) = clave {
        c.password(k);
    }
    c.ssl_mode(match d.sslmode.as_str() {
        "disable" => tokio_postgres::config::SslMode::Disable,
        "require" | "verify-ca" | "verify-full" => tokio_postgres::config::SslMode::Require,
        _ => tokio_postgres::config::SslMode::Prefer,
    });
    let (cliente, conexion) = tokio::time::timeout(Duration::from_secs(12), c.connect(tls()?))
        .await
        .map_err(|_| gotv_engine::anyhow_tr!("{}:{} no respondió en 12 s", "{}:{} did not answer within 12 s", d.host, d.puerto))?
        .map_err(|e| gotv_engine::anyhow_tr!("no conecté a {}:{} ({e})", "could not connect to {}:{} ({e})", d.host, d.puerto))?;
    tokio::spawn(async move {
        let _ = conexion.await;
    });
    Ok(cliente)
}

fn texto_a_valor(tipo: &str, v: Option<&str>) -> Valor {
    match v {
        None => Valor::Nulo,
        Some(t) if tipo == "bytea" => Valor::Binario(t.len().saturating_sub(2) / 2),
        Some(t) if matches!(tipo, "int2" | "int4" | "int8") => t.parse().map(Valor::Entero).unwrap_or_else(|_| Valor::Texto(t.into())),
        Some(t) if matches!(tipo, "float4" | "float8") => t.parse().map(Valor::Real).unwrap_or_else(|_| Valor::Texto(t.into())),
        Some(t) => Valor::Texto(t.into()),
    }
}

fn filas_de(mensajes: Vec<SimpleQueryMessage>, tipos: &[String], tope: usize) -> (Vec<Vec<Valor>>, bool) {
    let mut filas = Vec::new();
    let mut cortado = false;
    for m in mensajes {
        if let SimpleQueryMessage::Row(r) = m {
            if filas.len() == tope {
                cortado = true;
                break;
            }
            filas.push((0..r.len()).map(|i| texto_a_valor(tipos.get(i).map(String::as_str).unwrap_or(""), r.get(i))).collect());
        }
    }
    (filas, cortado)
}

/// ¿Devuelve filas que un cursor puede recorrer? (`SHOW` y `EXPLAIN` no van en un cursor)
fn va_en_cursor(sql: &str) -> bool {
    let s = sql.trim_start().trim_start_matches('(').trim_start().to_ascii_lowercase();
    ["select", "with", "values", "table"].iter().any(|p| s.starts_with(p))
}

/// Consulta de sólo lectura, con topes de filas y tiempo.
pub async fn consultar(c: &mut Client, sql: &str, l: &Limites) -> Result<Resultado> {
    match clasificar::clasificar(sql, Motor::Postgres)? {
        clasificar::Clase::Lectura => {}
        otra => gotv_engine::bail_tr!("«consultar» sólo lee y esto no es una lectura ({otra:?}); para cambiar datos, «ejecutar»", "«consultar» only reads and this is not a read ({otra:?}); to change data, «ejecutar»"),
    }
    let sql = sql.trim().trim_end_matches(';').trim();
    let tope = Duration::from_millis(l.ms + 3_000);
    tokio::time::timeout(tope, async {
        let tx = c.build_transaction().read_only(true).start().await?;
        tx.batch_execute(&format!("SET LOCAL statement_timeout = {}", l.ms)).await?;
        let st = tx.prepare(sql).await.map_err(|e| anyhow::anyhow!("{}", pg_error(&e)))?;
        let columnas: Vec<(String, String)> = st.columns().iter().map(|c| (c.name().to_string(), c.type_().name().to_string())).collect();
        let tipos: Vec<String> = columnas.iter().map(|c| c.1.clone()).collect();
        let (filas, cortado) = if va_en_cursor(sql) {
            tx.batch_execute(&format!("DECLARE gotv_c NO SCROLL CURSOR FOR {sql}")).await.map_err(|e| anyhow::anyhow!("{}", pg_error(&e)))?;
            let m = tx.simple_query(&format!("FETCH {} FROM gotv_c", l.filas + 1)).await.map_err(|e| anyhow::anyhow!("{}", pg_error(&e)))?;
            filas_de(m, &tipos, l.filas)
        } else {
            filas_de(tx.simple_query(sql).await.map_err(|e| anyhow::anyhow!("{}", pg_error(&e)))?, &tipos, l.filas)
        };
        let mut contadas = None;
        if cortado && va_en_cursor(sql) {
            // el total, si sale rápido (otro tope, más corto)
            tx.batch_execute("CLOSE gotv_c; SET LOCAL statement_timeout = 3000").await?;
            if let Ok(m) = tx.simple_query(&format!("SELECT count(*) FROM ({sql}) AS gotv_t")).await {
                contadas = m.iter().find_map(|x| if let SimpleQueryMessage::Row(r) = x { r.get(0).and_then(|n| n.parse().ok()) } else { None });
            }
        }
        tx.rollback().await?;
        Ok::<_, anyhow::Error>(Resultado { columnas, filas, cortado, contadas })
    })
    .await
    .map_err(|_| gotv_engine::anyhow_tr!("la consulta pasó de {} ms y se cortó: acótala (WHERE, índice, LIMIT)", "the query took over {} ms and was stopped: narrow it (WHERE, index, LIMIT)", l.ms))?
}

/// El texto útil de un error de Postgres (mensaje, detalle, pista), no sólo «db error».
fn pg_error(e: &tokio_postgres::Error) -> String {
    match e.as_db_error() {
        Some(d) => {
            let mut s = format!("{}: {}", d.code().code(), d.message());
            if d.code() == &tokio_postgres::error::SqlState::QUERY_CANCELED {
                s = gotv_engine::tr!("se pasó del tope de tiempo y Postgres la canceló ({s})", "it exceeded the time cap and Postgres canceled it ({s})");
            }
            if d.code() == &tokio_postgres::error::SqlState::READ_ONLY_SQL_TRANSACTION {
                s = gotv_engine::tr!("negado: «consultar» sólo lee ({s})", "denied: «consultar» only reads ({s})");
            }
            if let Some(x) = d.detail() {
                s.push_str(&format!(" · {x}"));
            }
            if let Some(x) = d.hint() {
                s.push_str(&format!(" · {x}"));
            }
            s
        }
        None => e.to_string(),
    }
}

/// El esquema: tablas, vistas y vistas materializadas fuera del catálogo, con columnas y filas
/// ESTIMADAS (`reltuples`: sin recorrer la tabla). Con `tabla`: además índices y llaves.
pub async fn esquema(c: &mut Client, tabla: Option<&str>) -> Result<String> {
    let tx = c.build_transaction().read_only(true).start().await?;
    tx.batch_execute("SET LOCAL statement_timeout = 10000").await?;
    let q = "SELECT n.nspname::text, c.relname::text, c.relkind::text, c.reltuples::float8,
        coalesce((SELECT string_agg(a.attname || ' ' || format_type(a.atttypid, a.atttypmod)
              || CASE WHEN a.attnotnull THEN ' NOT NULL' ELSE '' END
              || CASE WHEN a.attnum = ANY(coalesce((SELECT k.conkey FROM pg_constraint k WHERE k.conrelid = c.oid AND k.contype = 'p'), '{}')) THEN ' PK' ELSE '' END,
              ', ' ORDER BY a.attnum)
           FROM pg_attribute a WHERE a.attrelid = c.oid AND a.attnum > 0 AND NOT a.attisdropped), ''), c.oid::bigint
        FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace
        WHERE c.relkind IN ('r', 'v', 'm', 'p', 'f') AND n.nspname NOT IN ('pg_catalog', 'information_schema') AND n.nspname NOT LIKE 'pg_toast%'
          AND ($1::text IS NULL OR c.relname = $1 OR n.nspname || '.' || c.relname = $1)
        ORDER BY 1, 2 LIMIT 500";
    let filas = tx.query(q, &[&tabla]).await.map_err(|e| anyhow::anyhow!("{}", pg_error(&e)))?;
    let mut s = String::new();
    for f in &filas {
        let (esquema_, nombre, tipo, n, cols, oid): (String, String, String, f64, String, i64) = (f.get(0), f.get(1), f.get(2), f.get(3), f.get(4), f.get(5));
        let tipo = match tipo.as_str() {
            "r" | "p" => "table",
            "v" => "view",
            "m" => "matview",
            _ => "foreign",
        };
        let n = if n < 0.0 { "?".to_string() } else { format!("~{}", n as i64) };
        let nombre_c = if esquema_ == "public" { nombre.clone() } else { format!("{esquema_}.{nombre}") };
        s.push_str(&gotv_engine::tr!("{tipo} {nombre_c} ({n} filas): {cols}\n", "{tipo} {nombre_c} ({n} rows): {cols}\n"));
        if tabla.is_some() {
            let oid = oid as u32;
            for r in tx.query("SELECT pg_get_indexdef(indexrelid) FROM pg_index WHERE indrelid = $1::oid", &[&oid]).await? {
                s.push_str(&format!("  {}\n", r.get::<_, String>(0)));
            }
            for r in tx.query("SELECT conname::text, pg_get_constraintdef(oid) FROM pg_constraint WHERE conrelid = $1::oid AND contype IN ('f', 'c', 'u')", &[&oid]).await? {
                s.push_str(&format!("  {} {}\n", r.get::<_, String>(0), r.get::<_, String>(1)));
            }
        }
    }
    tx.rollback().await?;
    if s.is_empty() {
        return Ok(match tabla {
            Some(t) => gotv_engine::tr!("no hay tabla ni vista «{t}»", "there is no table or view «{t}»"),
            None => gotv_engine::i18n::t("(la base no tiene tablas fuera del catálogo)", "(the database has no tables outside the catalog)").into(),
        });
    }
    Ok(s)
}

/// El plan de una consulta. `analizar` = `EXPLAIN (ANALYZE, BUFFERS)`: la EJECUTA, sólo sobre una
/// lectura y dentro de la transacción de sólo lectura.
pub async fn explicar(c: &mut Client, sql: &str, analizar: bool, ms: u64) -> Result<String> {
    if clasificar::clasificar(sql, Motor::Postgres)? != clasificar::Clase::Lectura {
        gotv_engine::bail_tr!("«explicar» es para una lectura", "«explicar» is for a read");
    }
    let sql = sql.trim().trim_end_matches(';');
    let tx = c.build_transaction().read_only(true).start().await?;
    tx.batch_execute(&format!("SET LOCAL statement_timeout = {ms}")).await?;
    let pre = if analizar { "EXPLAIN (ANALYZE, BUFFERS)" } else { "EXPLAIN" };
    let m = tx.simple_query(&format!("{pre} {sql}")).await.map_err(|e| anyhow::anyhow!("{}", pg_error(&e)))?;
    tx.rollback().await?;
    Ok(m.iter().filter_map(|x| if let SimpleQueryMessage::Row(r) = x { r.get(0).map(String::from) } else { None }).collect::<Vec<_>>().join("\n"))
}

/// Versión, usuario, base y si la conexión PUEDE escribir (para avisar).
pub async fn probar(c: &mut Client) -> Result<String> {
    let f = c
        .query_one(
            "SELECT version(), current_user::text, current_database()::text, r.rolsuper,
               EXISTS (SELECT 1 FROM pg_class t JOIN pg_namespace n ON n.oid = t.relnamespace
                       WHERE t.relkind = 'r' AND n.nspname NOT IN ('pg_catalog', 'information_schema')
                         AND has_table_privilege(t.oid, 'INSERT, UPDATE, DELETE'))
             FROM pg_roles r WHERE r.rolname = current_user",
            &[],
        )
        .await
        .map_err(|e| anyhow::anyhow!("{}", pg_error(&e)))?;
    let (version, usuario, base, superusuario, escribe): (String, String, String, bool, bool) = (f.get(0), f.get(1), f.get(2), f.get(3), f.get(4));
    let version = version.split(" on ").next().unwrap_or(&version).to_string();
    let mut s = gotv_engine::tr!("✓ {version} · usuario {usuario} · base {base}", "✓ {version} · user {usuario} · database {base}");
    if superusuario || escribe {
        s.push_str(&gotv_engine::tr!(
            "\n⚠ este usuario PUEDE escribir{}. «consultar» lo frena con una transacción de sólo lectura, pero una función con efectos (pg_terminate_backend, nextval, dblink) no se frena así. Para blindarla, que el dueño cree un usuario de sólo lectura:\n  CREATE ROLE gotv_lector LOGIN PASSWORD '…';\n  GRANT CONNECT ON DATABASE {base} TO gotv_lector;\n  GRANT USAGE ON SCHEMA public TO gotv_lector;\n  GRANT SELECT ON ALL TABLES IN SCHEMA public TO gotv_lector;\n  ALTER DEFAULT PRIVILEGES IN SCHEMA public GRANT SELECT ON TABLES TO gotv_lector;",
            "\n⚠ this user CAN write{}. «consultar» stops it with a read-only transaction, but a function with side effects (pg_terminate_backend, nextval, dblink) is not stopped that way. To lock it down, let the owner create a read-only user:\n  CREATE ROLE gotv_lector LOGIN PASSWORD '…';\n  GRANT CONNECT ON DATABASE {base} TO gotv_lector;\n  GRANT USAGE ON SCHEMA public TO gotv_lector;\n  GRANT SELECT ON ALL TABLES IN SCHEMA public TO gotv_lector;\n  ALTER DEFAULT PRIVILEGES IN SCHEMA public GRANT SELECT ON TABLES TO gotv_lector;",
            if superusuario { gotv_engine::i18n::t(" (es superusuario)", " (it is a superuser)") } else { "" }
        ));
    } else {
        s.push_str(gotv_engine::i18n::t("\n✓ usuario de sólo lectura", "\n✓ read-only user"));
    }
    Ok(s)
}

/// Escribe dentro de una transacción; confirma SÓLO si `confirmar` dice que sí (si no, `ROLLBACK`).
/// El DDL de Postgres también es transaccional.
pub async fn ejecutar(c: &mut Client, sql: &str, confirmar: impl FnOnce(&Escritura) -> bool) -> Result<Escritura> {
    let (ddl, peligrosa) = match clasificar::clasificar(sql, Motor::Postgres)? {
        clasificar::Clase::Escritura { ddl, peligrosa } => (ddl, peligrosa),
        clasificar::Clase::Lectura => gotv_engine::bail_tr!("eso es una lectura: va por «consultar»", "that is a read: use «consultar»"),
        clasificar::Clase::Transaccion => gotv_engine::bail_tr!("la transacción la lleva GOTV: quita BEGIN/COMMIT/ROLLBACK del SQL", "GOTV runs the transaction: remove BEGIN/COMMIT/ROLLBACK from the SQL"),
        clasificar::Clase::Prohibida(por) => gotv_engine::bail_tr!("no se ejecuta: {por}", "not run: {por}"),
    };
    let tx = c.transaction().await?;
    tx.batch_execute("SET LOCAL statement_timeout = 60000").await?;
    let filas = tx.execute(sql.trim().trim_end_matches(';'), &[]).await.map_err(|e| anyhow::anyhow!("{}", pg_error(&e)))? as usize;
    let mut e = Escritura { filas, ddl, peligrosa, confirmada: false };
    if confirmar(&e) {
        tx.commit().await?;
        e.confirmada = true;
    } else {
        tx.rollback().await?;
    }
    Ok(e)
}

/// F1 paso 10 · exporta tablas (todas, o las de `solo`) a `dir`: `esquema.txt` + un CSV por tabla con cabecera,
/// por `COPY … TO STDOUT` en flujo (no carga la tabla en memoria). Devuelve (tabla, bytes).
pub async fn exportar(c: &mut Client, dir: &std::path::Path, solo: Option<&[String]>) -> Result<Vec<(String, u64)>> {
    use futures_util::StreamExt;
    use tokio::io::AsyncWriteExt;
    std::fs::create_dir_all(dir)?;
    std::fs::write(dir.join("esquema.txt"), esquema(c, None).await?)?;
    let tablas = c
        .query("SELECT n.nspname::text, c.relname::text FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace WHERE c.relkind IN ('r', 'p') AND n.nspname NOT IN ('pg_catalog', 'information_schema') AND n.nspname NOT LIKE 'pg_toast%' ORDER BY 1, 2", &[])
        .await
        .map_err(|e| anyhow::anyhow!("{}", pg_error(&e)))?;
    let mut hechos = Vec::new();
    for f in tablas {
        let (s, t): (String, String) = (f.get(0), f.get(1));
        if let Some(solo) = solo {
            if !solo.iter().any(|x| x.eq_ignore_ascii_case(&t) || x.eq_ignore_ascii_case(&format!("{s}.{t}"))) {
                continue;
            }
        }
        let ident = format!("\"{}\".\"{}\"", s.replace('"', "\"\""), t.replace('"', "\"\""));
        let sql = format!("COPY {ident} TO STDOUT WITH (FORMAT csv, HEADER true)");
        let flujo = c.copy_out(sql.as_str()).await.map_err(|e| anyhow::anyhow!("{}", pg_error(&e)))?;
        futures_util::pin_mut!(flujo);
        let mut out = tokio::fs::File::create(dir.join(format!("{}.csv", crate::nombre_archivo(&format!("{s}.{t}"))))).await?;
        let mut n = 0u64;
        while let Some(b) = flujo.next().await {
            let b = b?;
            n += b.len() as u64;
            out.write_all(&b).await?;
        }
        out.flush().await?;
        hechos.push((format!("{s}.{t}"), n));
    }
    Ok(hechos)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn el_destino_se_lee_con_sus_valores_por_defecto() {
        assert_eq!(Destino::leer("ana@db.local:6543/tienda?sslmode=require").unwrap(), Destino { usuario: "ana".into(), host: "db.local".into(), puerto: 6543, base: "tienda".into(), sslmode: "require".into() });
        let d = Destino::leer("postgres://h").unwrap();
        assert_eq!((d.usuario.as_str(), d.puerto, d.base.as_str(), d.sslmode.as_str()), ("postgres", 5432, "postgres", "prefer"));
        assert_eq!(Destino::leer("u@[::1]:5433/b").unwrap().host, "::1");
        assert!(Destino::leer("u@[::1]:5433/b").unwrap().es_local());
        assert!(Destino::leer("u@/b").is_err());
    }

    /// Un Postgres PROPIO y desechable: `initdb` en una carpeta temporal, otro puerto, clave con
    /// SCRAM. No toca ningún Postgres que ya esté corriendo en el equipo.
    struct Desechable {
        dir: gotv_engine::prueba::CarpetaPrueba,
        puerto: u16,
    }
    impl Desechable {
        fn levantar() -> Option<Desechable> {
            let ok = |c: &mut std::process::Command| c.stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status().is_ok_and(|s| s.success());
            if !ok(std::process::Command::new("initdb").env("LC_ALL", "C").arg("--version")) {
                return None;
            }
            let dir = gotv_engine::prueba::CarpetaPrueba::nueva("gotv-pg");
            std::fs::write(dir.join("clave"), "s3cr3t\n").unwrap();
            assert!(ok(std::process::Command::new("initdb").env("LC_ALL", "C").arg("-D").arg(dir.join("datos")).args(["-U", "gotv", "--locale=C", "-E", "UTF8", "--auth=scram-sha-256", "--pwfile"]).arg(dir.join("clave"))));
            let puerto = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
            // sin socket Unix: la ruta de una carpeta temporal pasa del límite de macOS (~103 bytes)
            let opciones = format!("-p {puerto} -c listen_addresses=127.0.0.1 -c unix_socket_directories=''");
            assert!(ok(std::process::Command::new("pg_ctl").env("LC_ALL", "C").arg("-D").arg(dir.join("datos")).args(["-o", &opciones, "-l"]).arg(dir.join("log")).args(["-w", "start"])));
            Some(Desechable { dir, puerto })
        }
    }
    impl Drop for Desechable {
        fn drop(&mut self) {
            let _ = std::process::Command::new("pg_ctl").env("LC_ALL", "C").arg("-D").arg(self.dir.join("datos")).args(["-m", "immediate", "stop"]).stdout(std::process::Stdio::null()).status();
        }
    }

    async fn contar(c: &Client) -> i64 {
        c.query_one("SELECT count(*) FROM clientes", &[]).await.unwrap().get::<_, i64>(0)
    }

    /// De punta a punta contra un Postgres real (el suyo propio): clave, esquema, lectura con cursor
    /// y topes, la batería de ataque y la escritura en transacción.
    #[tokio::test]
    #[ignore = "levanta un Postgres desechable con initdb (Homebrew): cargo test -p gotv-datos -- --ignored"]
    async fn postgres_de_punta_a_punta() {
        let Some(pg) = Desechable::levantar() else {
            eprintln!("sin initdb: prueba omitida");
            return;
        };
        let d = Destino::leer(&format!("gotv@127.0.0.1:{}/postgres", pg.puerto)).unwrap();
        assert!(conectar(&d, Some("mala")).await.is_err(), "la clave se exige");
        let mut c = conectar(&d, Some("s3cr3t")).await.unwrap();
        c.batch_execute("CREATE TABLE clientes (id serial PRIMARY KEY, nombre text NOT NULL, datos jsonb, foto bytea, password_hash text);
                         INSERT INTO clientes (nombre, datos, foto, password_hash) SELECT 'c' || i, '{\"a\": 1}', '\\x0102', 'h' FROM generate_series(1, 5000) i;
                         CREATE INDEX clientes_nombre ON clientes (nombre); ANALYZE clientes;
                         CREATE SEQUENCE s;").await.unwrap();

        let r = consultar(&mut c, "SELECT id, nombre, datos, foto FROM clientes ORDER BY id", &Limites::default()).await.unwrap();
        assert_eq!((r.filas.len(), r.cortado, r.contadas), (200, true, Some(5000)));
        assert_eq!(r.filas[0], vec![Valor::Entero(1), Valor::Texto("c1".into()), Valor::Texto("{\"a\": 1}".into()), Valor::Binario(2)]);
        let r = consultar(&mut c, "SHOW server_version", &Limites::default()).await.unwrap();
        assert_eq!(r.filas.len(), 1);

        let e = esquema(&mut c, None).await.unwrap();
        assert!(e.contains("table clientes (~5000 filas): id integer NOT NULL PK, nombre text NOT NULL, datos jsonb"), "{e}");
        let t = esquema(&mut c, Some("clientes")).await.unwrap();
        assert!(t.contains("CREATE INDEX clientes_nombre"), "{t}");
        assert!(explicar(&mut c, "SELECT * FROM clientes WHERE nombre = 'c7'", false, 5_000).await.unwrap().contains("clientes_nombre"));

        // ataque: cada vía, rechazada por GOTV o por la transacción de sólo lectura
        for sql in ["SELECT 1; DELETE FROM clientes", "SET default_transaction_read_only = off", "WITH b AS (DELETE FROM clientes RETURNING *) SELECT * FROM b", "EXPLAIN ANALYZE DELETE FROM clientes"] {
            assert!(consultar(&mut c, sql, &Limites::default()).await.is_err(), "{sql}");
        }
        // la quinta (función con efectos) pasa el clasificador: la frena la transacción de sólo lectura
        let e = consultar(&mut c, "SELECT nextval('s')", &Limites::default()).await.unwrap_err().to_string();
        assert!(e.contains("sólo lee") || e.contains("read-only") || e.contains("25006"), "{e}");
        assert!(probar(&mut c).await.unwrap().contains("PUEDE escribir"), "gotv es superusuario: avisa");

        // tope de tiempo del servidor
        let e = consultar(&mut c, "SELECT pg_sleep(3)", &Limites { ms: 300, ..Limites::default() }).await.unwrap_err().to_string();
        assert!(e.contains("57014") || e.contains("tope"), "{e}");

        // escribir: en transacción, se deshace si no se confirma
        let e = ejecutar(&mut c, "DELETE FROM clientes WHERE id > 4990", |_| false).await.unwrap();
        assert_eq!((e.filas, e.confirmada), (10, false));
        assert_eq!(contar(&c).await, 5000);
        assert!(ejecutar(&mut c, "DELETE FROM clientes WHERE id > 4990", |_| true).await.unwrap().confirmada);
        assert_eq!(contar(&c).await, 4990);
        // exportar: esquema + CSV en flujo
        let dir = pg.dir.join("respaldo");
        let e = exportar(&mut c, &dir, None).await.unwrap();
        assert_eq!(e.len(), 1);
        let csv = std::fs::read_to_string(dir.join("public.clientes.csv")).unwrap();
        assert!(csv.starts_with("id,nombre,datos,foto,password_hash\n") && csv.lines().count() == 4991, "{}", csv.lines().count());
        assert!(std::fs::read_to_string(dir.join("esquema.txt")).unwrap().contains("table clientes"));
        // el DDL también se deshace
        assert!(!ejecutar(&mut c, "DROP TABLE clientes", |e| e.peligrosa.is_none()).await.unwrap().confirmada);
        assert_eq!(contar(&c).await, 4990);
        assert!(ejecutar(&mut c, "COMMIT", |_| true).await.is_err());
    }
}
