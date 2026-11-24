//! F1 paso 5 · MySQL y MariaDB por la red, con `mysql_async` (Rust puro) y TLS de `rustls`.
//!
//! Lectura: `START TRANSACTION READ ONLY` + tope de tiempo del servidor (MySQL: `max_execution_time`, en
//! ms y SÓLO para `SELECT`; MariaDB: `max_statement_time`, en segundos, para todo) + un reloj propio que,
//! si vence, manda `KILL QUERY` desde una segunda conexión. Siempre por el protocolo BINARIO (sentencia
//! preparada): una sola sentencia por fuerza (el de texto admite varias, y MySQL ejecuta lo que va dentro de
//! `/*! … */`, que el clasificador ya rechaza). El `LIMIT` lo pone GOTV en la sentencia: un resultado
//! dejado a medias se drena ENTERO.
//! Escribir: transacción, como Postgres, salvo dos casos sin red: el DDL (MySQL lo confirma al instante,
//! no hay `ROLLBACK`) y las tablas no transaccionales (MyISAM, Aria…: el ensayo las cambiaría de verdad).
//! En esos casos NO se ensaya: se ejecuta sólo con la confirmación explícita de quien llama.
use crate::postgres::Destino;
use crate::sqlite::Escritura;
use crate::{clasificar, Limites, Motor, Resultado, Valor};
use anyhow::Result;
use mysql_async::prelude::Queryable;
use mysql_async::{Conn, OptsBuilder, TxOpts};
use std::time::Duration;

pub fn destino(d: &str) -> Result<Destino> {
    Destino::leer_con(d, 3306, "root", "")
}

pub async fn conectar(d: &Destino, clave: Option<&str>) -> Result<Conn> {
    // TLS: local, sin TLS salvo que se pida; remoto, con TLS salvo `sslmode=disable` (MySQL no tiene
    // «preferir»: o se exige o no se usa)
    let tls = match d.sslmode.as_str() {
        "disable" => false,
        "require" | "verify-ca" | "verify-full" => true,
        _ => !d.es_local(),
    };
    let opts = OptsBuilder::default()
        .ip_or_hostname(d.host.clone())
        .tcp_port(d.puerto)
        .user(Some(d.usuario.clone()))
        .pass(clave.map(String::from))
        .db_name((!d.base.is_empty()).then(|| d.base.clone()))
        .prefer_socket(false)
        .stmt_cache_size(0)
        .ssl_opts(tls.then(mysql_async::SslOpts::default));
    tokio::time::timeout(Duration::from_secs(12), Conn::new(opts))
        .await
        .map_err(|_| gotv_engine::anyhow_tr!("{}:{} no respondió en 12 s", "{}:{} did not answer within 12 s", d.host, d.puerto))?
        .map_err(|e| gotv_engine::anyhow_tr!("no conecté a {}:{} ({e})", "could not connect to {}:{} ({e})", d.host, d.puerto))
}

async fn es_mariadb(c: &mut Conn) -> bool {
    c.query_first::<String, _>("SELECT VERSION()").await.ok().flatten().is_some_and(|v| v.contains("MariaDB"))
}

/// Tope de tiempo del servidor para esta sesión.
async fn tope_de_tiempo(c: &mut Conn, ms: u64) -> Result<()> {
    let s = if es_mariadb(c).await { format!("SET SESSION max_statement_time = {:.3}", ms as f64 / 1000.0) } else { format!("SET SESSION max_execution_time = {ms}") };
    c.query_drop(s).await?;
    Ok(())
}

fn valor(v: Option<&mysql_async::Value>, binario: bool) -> Valor {
    use mysql_async::Value as V;
    match v {
        None | Some(V::NULL) => Valor::Nulo,
        Some(V::Int(n)) => Valor::Entero(*n),
        Some(V::UInt(n)) => i64::try_from(*n).map(Valor::Entero).unwrap_or_else(|_| Valor::Texto(n.to_string())),
        Some(V::Float(x)) => Valor::Real(*x as f64),
        Some(V::Double(x)) => Valor::Real(*x),
        Some(V::Bytes(b)) if binario => Valor::Binario(b.len()),
        Some(V::Bytes(b)) => Valor::Texto(String::from_utf8_lossy(b).into_owned()),
        Some(V::Date(a, m, d, h, mi, s, us)) => Valor::Texto(if (*h, *mi, *s, *us) == (0, 0, 0, 0) { format!("{a:04}-{m:02}-{d:02}") } else { format!("{a:04}-{m:02}-{d:02} {h:02}:{mi:02}:{s:02}") }),
        Some(V::Time(neg, d, h, mi, s, _)) => Valor::Texto(format!("{}{:02}:{mi:02}:{s:02}", if *neg { "-" } else { "" }, *d * 24 + *h as u32)),
    }
}

fn error(e: mysql_async::Error) -> anyhow::Error {
    match &e {
        mysql_async::Error::Server(s) => match s.code {
            // 3024 (MySQL) / 1969 (MariaDB): se pasó del tope de tiempo
            3024 | 1969 => gotv_engine::anyhow_tr!("se pasó del tope de tiempo y el servidor la canceló ({})", "it exceeded the time cap and the server canceled it ({})", s.message),
            // 1792: escritura dentro de una transacción de sólo lectura
            1792 => gotv_engine::anyhow_tr!("negado: «consultar» sólo lee ({})", "denied: «consultar» only reads ({})", s.message),
            _ => anyhow::anyhow!("{} ({}): {}", s.code, s.state, s.message),
        },
        _ => anyhow::anyhow!("{e}"),
    }
}

/// Lee con el tope de filas ya puesto en la sentencia; devuelve también si había más.
async fn leer(tx: &mut mysql_async::Transaction<'_>, sql: &str, tope: usize) -> Result<(Vec<(String, String)>, Vec<Vec<Valor>>, bool)> {
    let mut r = tx.exec_iter(sql, ()).await.map_err(error)?;
    let cols = r.columns_ref().to_vec();
    let columnas: Vec<(String, String)> = cols.iter().map(|c| (c.name_str().into_owned(), format!("{:?}", c.column_type()).trim_start_matches("MYSQL_TYPE_").to_lowercase())).collect();
    // juego de caracteres 63 = binario: BLOB, BINARY… salen como `<n bytes>`
    let binarias: Vec<bool> = cols.iter().map(|c| c.character_set() == 63 && matches!(c.column_type(), mysql_async::consts::ColumnType::MYSQL_TYPE_BLOB | mysql_async::consts::ColumnType::MYSQL_TYPE_TINY_BLOB | mysql_async::consts::ColumnType::MYSQL_TYPE_MEDIUM_BLOB | mysql_async::consts::ColumnType::MYSQL_TYPE_LONG_BLOB | mysql_async::consts::ColumnType::MYSQL_TYPE_STRING | mysql_async::consts::ColumnType::MYSQL_TYPE_VAR_STRING)).collect();
    let mut filas = Vec::new();
    let mut cortado = false;
    while let Some(f) = r.next().await.map_err(error)? {
        if filas.len() == tope {
            cortado = true;
            break;
        }
        filas.push((0..f.len()).map(|i| valor(f.as_ref(i), binarias.get(i) == Some(&true))).collect());
    }
    r.drop_result().await.map_err(error)?;
    Ok((columnas, filas, cortado))
}

/// Si el reloj propio vence, `KILL QUERY` desde otra conexión (el tope del servidor de MySQL no cubre
/// lo que no es `SELECT`).
async fn con_reloj<T>(d: &Destino, clave: Option<&str>, id: u32, ms: u64, f: impl std::future::Future<Output = Result<T>>) -> Result<T> {
    match tokio::time::timeout(Duration::from_millis(ms + 2_000), f).await {
        Ok(r) => r,
        Err(_) => {
            if let Ok(mut otra) = conectar(d, clave).await {
                let _ = otra.query_drop(format!("KILL QUERY {id}")).await;
                let _ = otra.disconnect().await;
            }
            Err(gotv_engine::anyhow_tr!("la consulta pasó de {ms} ms y se cortó (KILL QUERY): acótala (WHERE, índice, LIMIT)", "the query took over {ms} ms and was stopped (KILL QUERY): narrow it (WHERE, index, LIMIT)"))
        }
    }
}

/// Consulta de sólo lectura, con topes de filas y tiempo.
pub async fn consultar(c: &mut Conn, d: &Destino, clave: Option<&str>, sql: &str, l: &Limites) -> Result<Resultado> {
    match clasificar::clasificar(sql, Motor::Mysql)? {
        clasificar::Clase::Lectura => {}
        otra => gotv_engine::bail_tr!("«consultar» sólo lee y esto no es una lectura ({otra:?}); para cambiar datos, «ejecutar»", "«consultar» only reads and this is not a read ({otra:?}); to change data, «ejecutar»"),
    }
    let sql = sql.trim().trim_end_matches(';').trim();
    let limitada = clasificar::con_limite(sql, Motor::Mysql, l.filas + 1);
    tope_de_tiempo(c, l.ms).await?;
    let id = c.id();
    con_reloj(d, clave, id, l.ms, async {
        let mut o = TxOpts::default();
        o.with_readonly(true);
        let mut tx = c.start_transaction(o).await.map_err(error)?;
        let (columnas, filas, cortado) = leer(&mut tx, limitada.as_deref().unwrap_or(sql), l.filas).await?;
        let mut contadas = None;
        if cortado && limitada.is_some() {
            contadas = tx.exec_first::<i64, _, _>(format!("SELECT COUNT(*) FROM ({sql}) AS gotv_t"), ()).await.ok().flatten().map(|n| n as usize);
        }
        tx.rollback().await.map_err(error)?;
        Ok(Resultado { columnas, filas, cortado, contadas })
    })
    .await
}

/// El esquema de la base elegida: tablas y vistas con columnas y filas ESTIMADAS (`TABLE_ROWS`). Con
/// `tabla`, su `SHOW CREATE TABLE` (índices, llaves y motor incluidos).
pub async fn esquema(c: &mut Conn, tabla: Option<&str>) -> Result<String> {
    if c.query_first::<Option<String>, _>("SELECT DATABASE()").await.map_err(error)?.flatten().is_none() {
        gotv_engine::bail_tr!("la conexión no eligió base: agrégala al final del destino (usuario@host:3306/base)", "the connection chose no database: add it at the end of the destination (user@host:3306/database)");
    }
    c.query_drop("SET SESSION group_concat_max_len = 65536").await.map_err(error)?;
    let q = "SELECT t.TABLE_NAME, t.TABLE_TYPE, COALESCE(t.TABLE_ROWS, -1), t.ENGINE,
             GROUP_CONCAT(CONCAT(c.COLUMN_NAME, ' ', c.COLUMN_TYPE, IF(c.IS_NULLABLE = 'NO', ' NOT NULL', ''), IF(c.COLUMN_KEY = 'PRI', ' PK', '')) ORDER BY c.ORDINAL_POSITION SEPARATOR ', ')
             FROM information_schema.TABLES t JOIN information_schema.COLUMNS c ON c.TABLE_SCHEMA = t.TABLE_SCHEMA AND c.TABLE_NAME = t.TABLE_NAME
             WHERE t.TABLE_SCHEMA = DATABASE() AND (? IS NULL OR t.TABLE_NAME = ?)
             GROUP BY t.TABLE_NAME, t.TABLE_TYPE, t.TABLE_ROWS, t.ENGINE ORDER BY t.TABLE_NAME LIMIT 500";
    let filas: Vec<(String, String, i64, Option<String>, String)> = c.exec(q, (tabla, tabla)).await.map_err(error)?;
    let mut s = String::new();
    for (nombre, tipo, n, motor, cols) in &filas {
        let tipo = if tipo.contains("VIEW") { "view" } else { "table" };
        let n = if *n < 0 { "—".to_string() } else { format!("~{n}") };
        let motor = motor.as_deref().map(|m| format!(" [{m}]")).unwrap_or_default();
        s.push_str(&gotv_engine::tr!("{tipo} {nombre}{motor} ({n} filas): {cols}\n", "{tipo} {nombre}{motor} ({n} rows): {cols}\n"));
        if tabla.is_some() {
            if let Ok(Some((_, def))) = c.query_first::<(String, String), _>(format!("SHOW CREATE TABLE `{}`", nombre.replace('`', "``"))).await {
                s.push_str(&def);
                s.push('\n');
            }
        }
    }
    if s.is_empty() {
        return Ok(match tabla {
            Some(t) => gotv_engine::tr!("no hay tabla ni vista «{t}»", "there is no table or view «{t}»"),
            None => gotv_engine::i18n::t("(la base no tiene tablas)", "(the database has no tables)").into(),
        });
    }
    Ok(s)
}

/// El plan. `analizar`: MySQL `EXPLAIN ANALYZE` / MariaDB `ANALYZE` (la EJECUTAN: sólo lecturas, en la
/// transacción de sólo lectura).
pub async fn explicar(c: &mut Conn, sql: &str, analizar: bool, ms: u64) -> Result<String> {
    if clasificar::clasificar(sql, Motor::Mysql)? != clasificar::Clase::Lectura {
        gotv_engine::bail_tr!("«explicar» es para una lectura", "«explicar» is for a read");
    }
    let sql = sql.trim().trim_end_matches(';');
    let maria = es_mariadb(c).await;
    tope_de_tiempo(c, ms).await?;
    let pre = match (analizar, maria) {
        (false, _) => "EXPLAIN",
        (true, true) => "ANALYZE",
        (true, false) => "EXPLAIN ANALYZE",
    };
    let mut o = TxOpts::default();
    o.with_readonly(true);
    let mut tx = c.start_transaction(o).await.map_err(error)?;
    let (columnas, filas, cortado) = leer(&mut tx, &format!("{pre} {sql}"), 200).await?;
    tx.rollback().await.map_err(error)?;
    Ok(Resultado { columnas, filas, cortado, contadas: None }.tabla(&[], 20_000))
}

/// Versión, usuario, base y si la conexión PUEDE escribir (por sus permisos).
pub async fn probar(c: &mut Conn) -> Result<String> {
    let (version, usuario, base): (String, String, Option<String>) = c.query_first("SELECT VERSION(), CURRENT_USER(), DATABASE()").await.map_err(error)?.unwrap_or_default();
    let permisos: Vec<String> = c.query("SHOW GRANTS").await.map_err(error)?;
    let escribe = permisos.iter().any(|p| {
        let p = p.to_uppercase();
        ["ALL PRIVILEGES", "INSERT", "UPDATE", "DELETE", "DROP", "ALTER", "CREATE"].iter().any(|x| p.contains(x))
    });
    let base_txt = base.clone().unwrap_or_else(|| "—".into());
    let mut s = gotv_engine::tr!("✓ {version} · usuario {usuario} · base {base_txt}", "✓ {version} · user {usuario} · database {base_txt}");
    if escribe {
        let b = base.unwrap_or_else(|| "<base>".into());
        s.push_str(&gotv_engine::tr!(
            "\n⚠ este usuario PUEDE escribir. «consultar» lo frena con una transacción de sólo lectura y el protocolo de una sola sentencia, pero funciones con efectos (GET_LOCK, SLEEP, LOAD_FILE con permiso FILE) no se frenan así. Para blindarla, que el dueño cree un usuario de sólo lectura:\n  CREATE USER 'gotv_lector'@'%' IDENTIFIED BY '…';\n  GRANT SELECT, SHOW VIEW ON `{b}`.* TO 'gotv_lector'@'%';",
            "\n⚠ this user CAN write. «consultar» stops it with a read-only transaction and the single-statement protocol, but functions with side effects (GET_LOCK, SLEEP, LOAD_FILE with the FILE privilege) are not stopped that way. To lock it down, let the owner create a read-only user:\n  CREATE USER 'gotv_lector'@'%' IDENTIFIED BY '…';\n  GRANT SELECT, SHOW VIEW ON `{b}`.* TO 'gotv_lector'@'%';"
        ));
    } else {
        s.push_str(gotv_engine::i18n::t("\n✓ usuario de sólo lectura", "\n✓ read-only user"));
    }
    Ok(s)
}

/// Tablas no transaccionales de la base cuyo nombre aparece en la sentencia (MyISAM, Aria, MEMORY…):
/// ahí un `ROLLBACK` no deshace nada.
async fn sin_transaccion(c: &mut Conn, sql: &str) -> Result<Vec<String>> {
    let tablas: Vec<(String, String)> = c.query("SELECT TABLE_NAME, ENGINE FROM information_schema.TABLES WHERE TABLE_SCHEMA = DATABASE() AND TABLE_TYPE = 'BASE TABLE' AND ENGINE IS NOT NULL AND ENGINE NOT IN ('InnoDB')").await.map_err(error)?;
    let palabras: std::collections::HashSet<String> = sql.split(|ch: char| !(ch.is_alphanumeric() || ch == '_' || ch == '$')).map(|w| w.to_lowercase()).collect();
    Ok(tablas.into_iter().filter(|(t, _)| palabras.contains(&t.to_lowercase())).map(|(t, m)| format!("{t} ({m})")).collect())
}

/// Escribe. DML sobre tablas transaccionales: dentro de una transacción, se cuentan las filas y
/// `confirmar` decide `COMMIT` o `ROLLBACK`. DDL o tablas sin transacción: NO se ensaya; se ejecuta sólo
/// si `sin_ensayo` es `true` (la confirmación explícita), y si no, vuelve sin tocar nada con el motivo.
pub async fn ejecutar(c: &mut Conn, sql: &str, sin_ensayo: bool, confirmar: impl FnOnce(&Escritura) -> bool) -> Result<Escritura> {
    let (ddl, peligrosa) = match clasificar::clasificar(sql, Motor::Mysql)? {
        clasificar::Clase::Escritura { ddl, peligrosa } => (ddl, peligrosa),
        clasificar::Clase::Lectura => gotv_engine::bail_tr!("eso es una lectura: va por «consultar»", "that is a read: use «consultar»"),
        clasificar::Clase::Transaccion => gotv_engine::bail_tr!("la transacción la lleva GOTV: quita BEGIN/COMMIT/ROLLBACK del SQL", "GOTV runs the transaction: remove BEGIN/COMMIT/ROLLBACK from the SQL"),
        clasificar::Clase::Prohibida(por) => gotv_engine::bail_tr!("no se ejecuta: {por}", "not run: {por}"),
    };
    let sql = sql.trim().trim_end_matches(';');
    let sin_red = if ddl {
        Some(gotv_engine::i18n::t("MySQL confirma el DDL al instante: no hay ROLLBACK", "MySQL commits DDL instantly: there is no ROLLBACK").to_string())
    } else {
        let t = sin_transaccion(c, sql).await?;
        (!t.is_empty()).then(|| gotv_engine::tr!("tablas sin transacción ({}): un ensayo las cambiaría de verdad", "tables without transactions ({}): a dry run would really change them", t.join(", ")))
    };
    if let Some(motivo) = sin_red {
        let peligrosa = Some(match peligrosa {
            Some(p) => format!("{p} · {motivo}"),
            None => motivo,
        });
        if !sin_ensayo {
            return Ok(Escritura { filas: 0, ddl, peligrosa, confirmada: false });
        }
        c.exec_drop(sql, ()).await.map_err(error)?;
        return Ok(Escritura { filas: c.affected_rows() as usize, ddl, peligrosa, confirmada: true });
    }
    let mut tx = c.start_transaction(TxOpts::default()).await.map_err(error)?;
    tx.exec_drop(sql, ()).await.map_err(error)?;
    let mut e = Escritura { filas: tx.affected_rows() as usize, ddl, peligrosa, confirmada: false };
    if confirmar(&e) {
        tx.commit().await.map_err(error)?;
        e.confirmada = true;
    } else {
        tx.rollback().await.map_err(error)?;
    }
    Ok(e)
}

/// F1 paso 10 · exporta tablas (todas, o las de `solo`) a `dir`: `esquema.sql` (`SHOW CREATE TABLE`) + un CSV por
/// tabla con cabecera, fila a fila (no carga la tabla en memoria). NULL = `\N` (lo que entiende `LOAD DATA`);
/// binarios en hexadecimal `0x…`. Devuelve (tabla, bytes).
pub async fn exportar(c: &mut Conn, dir: &std::path::Path, solo: Option<&[String]>) -> Result<Vec<(String, u64)>> {
    use std::io::Write;
    std::fs::create_dir_all(dir)?;
    let tablas: Vec<String> = c.query("SELECT TABLE_NAME FROM information_schema.TABLES WHERE TABLE_SCHEMA = DATABASE() AND TABLE_TYPE = 'BASE TABLE' ORDER BY 1").await.map_err(error)?;
    let mut esquema_sql = String::new();
    let mut hechos = Vec::new();
    for t in tablas {
        if let Some(solo) = solo {
            if !solo.iter().any(|x| x.eq_ignore_ascii_case(&t)) {
                continue;
            }
        }
        let q = format!("`{}`", t.replace('`', "``"));
        if let Some((_, def)) = c.query_first::<(String, String), _>(format!("SHOW CREATE TABLE {q}")).await.map_err(error)? {
            esquema_sql.push_str(&def);
            esquema_sql.push_str(";\n\n");
        }
        let archivo = dir.join(format!("{}.csv", crate::nombre_archivo(&t)));
        let mut out = std::io::BufWriter::new(std::fs::File::create(&archivo)?);
        // protocolo de texto: la sentencia la arma GOTV (nombre entre acentos graves, escapado)
        let mut r = c.query_iter(format!("SELECT * FROM {q}")).await.map_err(error)?;
        let cols = r.columns_ref().to_vec();
        let binaria: Vec<bool> = cols.iter().map(|x| x.character_set() == 63 && !matches!(x.column_type(), mysql_async::consts::ColumnType::MYSQL_TYPE_LONG | mysql_async::consts::ColumnType::MYSQL_TYPE_LONGLONG | mysql_async::consts::ColumnType::MYSQL_TYPE_SHORT | mysql_async::consts::ColumnType::MYSQL_TYPE_TINY | mysql_async::consts::ColumnType::MYSQL_TYPE_INT24 | mysql_async::consts::ColumnType::MYSQL_TYPE_DOUBLE | mysql_async::consts::ColumnType::MYSQL_TYPE_FLOAT | mysql_async::consts::ColumnType::MYSQL_TYPE_NEWDECIMAL | mysql_async::consts::ColumnType::MYSQL_TYPE_DATE | mysql_async::consts::ColumnType::MYSQL_TYPE_DATETIME | mysql_async::consts::ColumnType::MYSQL_TYPE_TIMESTAMP | mysql_async::consts::ColumnType::MYSQL_TYPE_TIME | mysql_async::consts::ColumnType::MYSQL_TYPE_YEAR)).collect();
        writeln!(out, "{}", cols.iter().map(|x| crate::campo_csv(Some(&x.name_str()), "")).collect::<Vec<_>>().join(","))?;
        while let Some(f) = r.next().await.map_err(error)? {
            let celdas: Vec<String> = (0..f.len())
                .map(|i| match f.as_ref(i) {
                    None | Some(mysql_async::Value::NULL) => "\\N".to_string(),
                    Some(mysql_async::Value::Bytes(b)) if binaria.get(i) == Some(&true) => format!("0x{}", b.iter().map(|x| format!("{x:02x}")).collect::<String>()),
                    Some(mysql_async::Value::Bytes(b)) => crate::campo_csv(Some(&String::from_utf8_lossy(b)), ""),
                    Some(otro) => crate::campo_csv(Some(&valor(Some(otro), false).to_string()), ""),
                })
                .collect();
            writeln!(out, "{}", celdas.join(","))?;
        }
        out.flush()?;
        hechos.push((t.clone(), std::fs::metadata(&archivo)?.len()));
    }
    std::fs::write(dir.join("esquema.sql"), esquema_sql)?;
    Ok(hechos)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Un MariaDB PROPIO y desechable en Docker (imagen ya bajada), en un puerto libre de 127.0.0.1;
    /// el contenedor se borra solo al pararlo (`--rm`).
    struct Desechable {
        nombre: String,
        puerto: u16,
    }
    impl Desechable {
        fn levantar() -> Option<Desechable> {
            let hay = std::process::Command::new("docker").args(["image", "inspect", "mariadb:11.4"]).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status().is_ok_and(|s| s.success());
            if !hay {
                return None;
            }
            let puerto = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
            let nombre = format!("gotv-prueba-mariadb-{}", std::process::id());
            let ok = std::process::Command::new("docker")
                .args(["run", "-d", "--rm", "--name", &nombre, "-e", "MARIADB_ROOT_PASSWORD=s3cr3t", "-e", "MARIADB_DATABASE=tienda", "-p", &format!("127.0.0.1:{puerto}:3306"), "mariadb:11.4"])
                .stdout(std::process::Stdio::null())
                .status()
                .is_ok_and(|s| s.success());
            ok.then_some(Desechable { nombre, puerto })
        }
    }
    impl Drop for Desechable {
        fn drop(&mut self) {
            let _ = std::process::Command::new("docker").args(["stop", "-t", "1", &self.nombre]).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status();
        }
    }

    async fn contar(c: &mut Conn) -> i64 {
        c.query_first::<i64, _>("SELECT COUNT(*) FROM clientes").await.unwrap().unwrap()
    }

    #[tokio::test]
    #[ignore = "levanta un MariaDB desechable en Docker: cargo test -p gotv-datos mysql -- --ignored"]
    async fn mariadb_de_punta_a_punta() {
        let Some(m) = Desechable::levantar() else {
            eprintln!("sin Docker o sin la imagen mariadb:11.4: prueba omitida");
            return;
        };
        let d = destino(&format!("root@127.0.0.1:{}/tienda", m.puerto)).unwrap();
        // el servidor tarda en arrancar: hasta 60 s
        let mut c = None;
        for _ in 0..60 {
            if let Ok(x) = conectar(&d, Some("s3cr3t")).await {
                c = Some(x);
                break;
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
        let mut c = c.expect("MariaDB no arrancó en 60 s");
        assert!(conectar(&d, Some("mala")).await.is_err(), "la clave se exige");
        c.query_drop("CREATE TABLE clientes (id INT AUTO_INCREMENT PRIMARY KEY, nombre VARCHAR(50) NOT NULL, foto BLOB, alta DATE, token VARCHAR(20)) ENGINE=InnoDB").await.unwrap();
        c.query_drop("INSERT INTO clientes (nombre, foto, alta, token) SELECT CONCAT('c', seq), X'0102', '2026-11-15', 't' FROM seq_1_to_5000").await.unwrap();
        c.query_drop("CREATE TABLE bitacora (id INT PRIMARY KEY, txt TEXT) ENGINE=MyISAM").await.unwrap();
        c.query_drop("CREATE INDEX clientes_nombre ON clientes (nombre)").await.unwrap();

        let r = consultar(&mut c, &d, Some("s3cr3t"), "SELECT id, nombre, foto, alta FROM clientes ORDER BY id", &Limites::default()).await.unwrap();
        assert_eq!((r.filas.len(), r.cortado, r.contadas), (200, true, Some(5000)));
        assert_eq!(r.filas[0], vec![Valor::Entero(1), Valor::Texto("c1".into()), Valor::Binario(2), Valor::Texto("2026-11-15".into())]);
        assert_eq!(consultar(&mut c, &d, None, "SHOW TABLES", &Limites::default()).await.unwrap().filas.len(), 2);

        let e = esquema(&mut c, None).await.unwrap();
        assert!(e.contains("table clientes [InnoDB]") && e.contains("id int(11) NOT NULL PK"), "{e}");
        assert!(esquema(&mut c, Some("clientes")).await.unwrap().contains("KEY `clientes_nombre`"));
        assert!(explicar(&mut c, "SELECT * FROM clientes WHERE nombre = 'c7'", false, 5_000).await.unwrap().contains("clientes_nombre"));

        // ataque
        for sql in ["SELECT 1; DELETE FROM clientes", "SELECT 1 /*! ; DELETE FROM clientes */", "SET autocommit = 1", "SELECT * FROM clientes INTO OUTFILE '/tmp/x'"] {
            assert!(consultar(&mut c, &d, None, sql, &Limites::default()).await.is_err(), "{sql}");
        }
        assert!(probar(&mut c).await.unwrap().contains("PUEDE escribir"));
        // tope de tiempo del servidor
        let t = std::time::Instant::now();
        assert!(consultar(&mut c, &d, Some("s3cr3t"), "SELECT SLEEP(5)", &Limites { ms: 500, ..Limites::default() }).await.map(|r| r.filas.len()).unwrap_or(1) <= 1);
        assert!(t.elapsed() < Duration::from_secs(4), "{:?}", t.elapsed());

        // escribir en InnoDB: ensayo en transacción
        let e = ejecutar(&mut c, "DELETE FROM clientes WHERE id > 4990", false, |_| false).await.unwrap();
        assert_eq!((e.filas, e.confirmada), (10, false));
        assert_eq!(contar(&mut c).await, 5000);
        assert!(ejecutar(&mut c, "DELETE FROM clientes WHERE id > 4990", false, |_| true).await.unwrap().confirmada);
        assert_eq!(contar(&mut c).await, 4990);
        // exportar: SHOW CREATE + CSV fila a fila (BLOB en hexadecimal, NULL como \N)
        let dir = gotv_engine::prueba::CarpetaPrueba::nueva("gotv-mysql-exporta");
        let e = exportar(&mut c, &dir, Some(&["clientes".to_string()])).await.unwrap();
        assert_eq!(e.len(), 1);
        let csv = std::fs::read_to_string(dir.join("clientes.csv")).unwrap();
        assert!(csv.starts_with("id,nombre,foto,alta,token\n1,c1,0x0102,2026-11-15,t\n") && csv.lines().count() == 4991, "{}", &csv[..80]);
        assert!(std::fs::read_to_string(dir.join("esquema.sql")).unwrap().contains("CREATE TABLE `clientes`"));
        // DDL y MyISAM: sin ensayo, no se toca nada sin la confirmación explícita
        let e = ejecutar(&mut c, "DROP TABLE clientes", false, |_| true).await.unwrap();
        assert!(!e.confirmada && e.peligrosa.unwrap().contains("ROLLBACK"));
        assert_eq!(contar(&mut c).await, 4990);
        let e = ejecutar(&mut c, "INSERT INTO bitacora VALUES (1, 'x')", false, |_| true).await.unwrap();
        assert!(!e.confirmada && e.peligrosa.unwrap().contains("bitacora (MyISAM)"));
        assert_eq!(c.query_first::<i64, _>("SELECT COUNT(*) FROM bitacora").await.unwrap(), Some(0), "el ensayo no tocó la MyISAM");
        assert!(ejecutar(&mut c, "INSERT INTO bitacora VALUES (1, 'x')", true, |_| false).await.unwrap().confirmada);
        assert!(ejecutar(&mut c, "COMMIT", true, |_| true).await.is_err());
    }
}
