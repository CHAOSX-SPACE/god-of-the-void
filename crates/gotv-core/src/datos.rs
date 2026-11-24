//! F1 · La herramienta `base_datos`: el puente entre el agente y `gotv-datos`.
//!
//! Leer (esquema, consultar, explicar) pasa sin preguntar: la conexión es de sólo lectura por el
//! propio motor. Escribir (`ejecutar`) pregunta SIEMPRE (también en bypass, ver `permissions.rs`),
//! corre en una transacción y, si es peligrosa (DROP, TRUNCATE, DELETE/UPDATE sin WHERE), pasa de
//! 1 000 filas o la base es de producción, NO se confirma hasta que el modelo repite la orden con
//! `confirmar_filas` = el número exacto de filas que tocaría. Antes de confirmar una peligrosa,
//! respaldo automático en `.gotv/respaldos/`.
//! Las bases deben vivir DENTRO del proyecto (una base de otra app del Mac no se lee por aquí).
use anyhow::{anyhow, Result};
use gotv_datos::registro::{self, Conexion};
/// Para la ventana (panel «Bases de datos»): el registro y la detección, tal cual.
pub use gotv_datos::{detectar, registro as registro_bases};
use gotv_datos::{clasificar, sqlite, Limites, Motor};
use std::path::{Path, PathBuf};

/// F1 paso 9: la conexión reservada para consultar con SQL los CSV, Parquet y JSON del proyecto (DuckDB).
pub const ARCHIVOS: &str = "archivos";

/// Lo que se puede tocar sin pedir confirmación de filas.
const FILAS_SIN_CONFIRMAR: usize = 1_000;
/// Respaldos automáticos que se guardan por base.
const RESPALDOS_POR_BASE: usize = 5;

fn s<'a>(args: &'a serde_json::Value, k: &str) -> Option<&'a str> {
    args.get(k).and_then(|v| v.as_str()).map(str::trim).filter(|v| !v.is_empty())
}

/// La ruta dentro del proyecto, o error (también si un enlace la saca de él).
fn dentro(root: &Path, ruta: &str) -> Result<PathBuf> {
    let p = if Path::new(ruta).is_absolute() { PathBuf::from(ruta) } else { root.join(ruta) };
    let raiz = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    // si aún no existe (destino de un respaldo), se mira su carpeta
    let real = p.canonicalize().or_else(|_| p.parent().map(|d| d.canonicalize().map(|d| d.join(p.file_name().unwrap_or_default()))).unwrap_or(Err(std::io::ErrorKind::NotFound.into())));
    match real {
        Ok(r) if r.starts_with(&raiz) => Ok(r),
        Ok(_) => Err(anyhow!("{}", crate::tr!("«{ruta}» está fuera del proyecto: sólo se abren bases del proyecto", "«{ruta}» is outside the project: only project databases are opened"))),
        Err(_) => Err(anyhow!("{}", crate::tr!("no existe «{ruta}»", "«{ruta}» does not exist"))),
    }
}

/// La conexión pedida: por nombre registrado, por ruta de un archivo SQLite del proyecto, o la
/// única registrada.
fn resolver(root: &Path, args: &serde_json::Value) -> Result<Conexion> {
    let todas = registro::cargar(root);
    match s(args, "conexion") {
        Some(n) => {
            if let Some(c) = todas.iter().find(|c| c.nombre == n) {
                return Ok(c.clone());
            }
            // F1 paso 9: «archivos» = los CSV, Parquet y JSON del proyecto, consultados con DuckDB
            if n == ARCHIVOS {
                return Ok(Conexion { nombre: ARCHIVOS.into(), motor: Motor::Duckdb, archivo: None, destino: None, secreto: None, produccion: false, mascara: vec![], tunel: None });
            }
            let p = dentro(root, n)?;
            let motor = if sqlite::es_sqlite(&p) {
                Motor::Sqlite
            } else if gotv_datos::detectar::es_duckdb(&p) {
                Motor::Duckdb
            } else {
                return Err(anyhow!("{}", crate::tr!("«{n}» no es una conexión registrada ni un archivo SQLite o DuckDB; mira action=conexiones (los CSV y Parquet se consultan con conexion=archivos)", "«{n}» is neither a registered connection nor a SQLite or DuckDB file; see action=conexiones (CSV and Parquet are queried with conexion=archivos)")));
            };
            Ok(Conexion { nombre: n.to_string(), motor, archivo: Some(p.display().to_string()), destino: None, secreto: None, produccion: false, mascara: vec![], tunel: None })
        }
        None if todas.len() == 1 => Ok(todas[0].clone()),
        None => Err(anyhow!("{}", crate::tr!(
            "falta `conexion` (el nombre registrado o la ruta del archivo); action=conexiones las lista y action=detectar busca las del proyecto",
            "missing `conexion` (the registered name or the file path); action=conexiones lists them and action=detectar finds the project's"
        ))),
    }
}

/// La ruta del archivo de una conexión SQLite, comprobando que siga dentro del proyecto.
fn archivo_sqlite(root: &Path, c: &Conexion) -> Result<PathBuf> {
    match c.motor {
        Motor::Sqlite => {}
        otro => {
            return Err(anyhow!("{}", crate::tr!(
                "{} aún no (DuckDB llega en el paso 9 de F1)",
                "{} not yet (DuckDB arrives in F1 step 9)",
                otro.nombre()
            )))
        }
    }
    let a = c.archivo.as_deref().ok_or_else(|| anyhow!("{}", crate::i18n::t("la conexión no tiene archivo", "the connection has no file")))?;
    dentro(root, a)
}

fn limites(args: &serde_json::Value) -> Limites {
    let d = Limites::default();
    Limites { filas: args.get("filas").and_then(|v| v.as_u64()).map(|n| (n as usize).clamp(1, 1_000)).unwrap_or(d.filas), ..d }
}

fn slug(n: &str) -> String {
    let s: String = n.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
    s.trim_matches('-').chars().take(40).collect()
}

/// Respaldo con fecha en `.gotv/respaldos/`; deja sólo los últimos de esa base.
fn respaldar(root: &Path, c: &Conexion, ruta: &Path, destino: Option<&str>) -> Result<(PathBuf, u64)> {
    let destino = match destino {
        Some(d) => dentro(root, d)?,
        None => {
            let dir = root.join(".gotv").join("respaldos");
            std::fs::create_dir_all(&dir)?;
            let hora = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
            let prefijo = format!("{}-", slug(&c.nombre));
            let destino = dir.join(format!("{prefijo}{hora}.db"));
            // poda: sólo los de ESTA base, por su ruta exacta, los más viejos primero (L6)
            podar_respaldos(&dir, &prefijo);
            destino
        }
    };
    let bytes = sqlite::respaldo(ruta, &destino)?;
    Ok((destino, bytes))
}

fn rel(root: &Path, p: &Path) -> String {
    let raiz = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    p.strip_prefix(&raiz).or_else(|_| p.strip_prefix(root)).unwrap_or(p).display().to_string()
}

/// Corre la herramienta (síncrona: SQLite en proceso; quien llama la saca del hilo de tokio).
pub fn ejecutar(args: &serde_json::Value, root: &Path) -> Result<String> {
    let accion = s(args, "action").unwrap_or("conexiones");
    if matches!(accion, "probar" | "esquema" | "consultar" | "explicar" | "ejecutar" | "respaldo") {
        if let Ok(c) = resolver(root, args) {
            if c.motor == Motor::Duckdb {
                return duck(root, args, &c, accion);
            }
        }
    }
    match accion {
        "conexiones" => {
            let todas = registro::cargar(root);
            if todas.is_empty() {
                return Ok(crate::i18n::t("No hay conexiones registradas. action=detectar busca las bases del proyecto; action=agregar registra una.", "No connections registered. action=detectar finds the project's databases; action=agregar registers one.").into());
            }
            Ok(todas.iter().map(|c| format!("- {}", c.resumen())).collect::<Vec<_>>().join("\n"))
        }
        "detectar" => {
            let h = gotv_datos::detectar::detectar(root);
            if h.is_empty() {
                return Ok(crate::i18n::t("No encontré bases en el proyecto (archivos SQLite/DuckDB ni DATABASE_URL en .env).", "I found no databases in the project (SQLite/DuckDB files or DATABASE_URL in .env).").into());
            }
            let mut out: Vec<String> = h.iter().map(|c| format!("- {}", c.resumen())).collect();
            out.push(crate::i18n::t("(no se registró nada: action=agregar con la que quieras usar; los archivos SQLite también se abren directo por su ruta)", "(nothing was registered: action=agregar the one you want to use; SQLite files also open directly by path)").into());
            Ok(out.join("\n"))
        }
        "agregar" => {
            let nombre = s(args, "nombre").ok_or_else(|| anyhow!("{}", crate::i18n::t("falta `nombre`", "missing `nombre`")))?;
            let mascara: Vec<String> = args.get("mascara").and_then(|v| v.as_array()).map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default();
            let produccion = args.get("produccion").and_then(|v| v.as_bool()).unwrap_or(false);
            let c = if let Some(a) = s(args, "archivo") {
                let p = dentro(root, a)?;
                let motor = if sqlite::es_sqlite(&p) {
                    Motor::Sqlite
                } else if gotv_datos::detectar::es_duckdb(&p) {
                    Motor::Duckdb
                } else {
                    return Err(anyhow!("{}", crate::tr!("«{a}» no es una base SQLite ni DuckDB", "«{a}» is not a SQLite or DuckDB database")));
                };
                Conexion { nombre: nombre.into(), motor, archivo: Some(rel(root, &p)), destino: None, secreto: None, produccion, mascara, tunel: None }
            } else if let Some(u) = s(args, "url") {
                let motor = match u.split("://").next().unwrap_or("") {
                    "postgres" | "postgresql" => Motor::Postgres,
                    "mysql" | "mariadb" => Motor::Mysql,
                    otro => return Err(anyhow!("{}", crate::tr!("esquema «{otro}» desconocido (postgres:// o mysql://)", "unknown scheme «{otro}» (postgres:// or mysql://)"))),
                };
                let (destino, tenia) = registro::sin_clave(u);
                // la clave NO se guarda: se dice dónde está (variable de entorno o Llavero)
                let secreto = s(args, "secreto").map(String::from).or_else(|| tenia.then(|| "env:DATABASE_URL".to_string()));
                // por un túnel: la conexión SSH tiene que existir ya en el proyecto
                let tunel = s(args, "tunel").map(String::from);
                if let Some(t) = &tunel {
                    let reg = crate::conexiones::Registro::cargar(root)?;
                    if reg.buscar(t).is_none() {
                        return Err(anyhow!("{}", crate::tr!("no hay una conexión SSH «{t}» en este proyecto: primero `gotv conexiones agregar`", "there is no SSH connection «{t}» in this project: first `gotv conexiones agregar`")));
                    }
                }
                Conexion { nombre: nombre.into(), motor, archivo: None, destino: Some(destino), secreto, produccion, mascara, tunel }
            } else {
                return Err(anyhow!("{}", crate::i18n::t("falta `archivo` (SQLite) o `url` (Postgres/MySQL)", "missing `archivo` (SQLite) or `url` (Postgres/MySQL)")));
            };
            let r = c.resumen();
            registro::agregar(root, c)?;
            Ok(crate::tr!("✓ registrada: {r} (en el registro del proyecto, sin claves)", "✓ registered: {r} (in the project registry, without passwords)"))
        }
        "probar" => {
            let c = resolver(root, args)?;
            let ruta = archivo_sqlite(root, &c)?;
            let tablas = sqlite::consultar(&ruta, "SELECT sqlite_version(), (SELECT count(*) FROM sqlite_master WHERE type = 'table')", &Limites::default())?;
            let f = tablas.filas.first().cloned().unwrap_or_default();
            Ok(crate::tr!("✓ SQLite {} · {} tablas · {} (abierta en sólo lectura para leer)", "✓ SQLite {} · {} tables · {} (opened read-only for reading)", f.first().map(|v| v.to_string()).unwrap_or_default(), f.get(1).map(|v| v.to_string()).unwrap_or_default(), rel(root, &ruta)))
        }
        "esquema" => {
            let c = resolver(root, args)?;
            sqlite::esquema(&archivo_sqlite(root, &c)?, s(args, "tabla"))
        }
        "consultar" => {
            let c = resolver(root, args)?;
            let sql = s(args, "sql").ok_or_else(|| anyhow!("{}", crate::i18n::t("falta `sql`", "missing `sql`")))?;
            let l = limites(args);
            let r = sqlite::consultar(&archivo_sqlite(root, &c)?, sql, &l)?;
            Ok(tabla_para(&r, &c, args, &l))
        }
        "explicar" => {
            let c = resolver(root, args)?;
            let sql = s(args, "sql").ok_or_else(|| anyhow!("{}", crate::i18n::t("falta `sql`", "missing `sql`")))?;
            sqlite::explicar(&archivo_sqlite(root, &c)?, sql)
        }
        "secreto" => Err(anyhow!("{}", crate::i18n::t("la clave sólo se pide dentro de una conversación (la teclea el usuario)", "the password is only asked inside a conversation (the user types it)"))),
        "respaldo" => {
            let c = resolver(root, args)?;
            let ruta = archivo_sqlite(root, &c)?;
            let (destino, bytes) = respaldar(root, &c, &ruta, s(args, "archivo"))?; // juez-idioma: datos (nombre de un argumento de la herramienta)
            Ok(crate::tr!("✓ respaldo: {} ({} KB)", "✓ backup: {} ({} KB)", rel(root, &destino), bytes / 1024))
        }
        "ejecutar" => {
            let c = resolver(root, args)?;
            let sql = s(args, "sql").ok_or_else(|| anyhow!("{}", crate::i18n::t("falta `sql`", "missing `sql`")))?;
            let ruta = archivo_sqlite(root, &c)?;
            let confirmar = args.get("confirmar_filas").and_then(|v| v.as_u64()).map(|n| n as usize);
            // una peligrosa que ya trae su número: respaldo ANTES de tocar nada
            let mut respaldo = None;
            if let (Ok(clasificar::Clase::Escritura { peligrosa: Some(_), .. }), Some(_)) = (clasificar::clasificar(sql, Motor::Sqlite), confirmar) {
                respaldo = Some(respaldar(root, &c, &ruta, None)?.0);
            }
            let e = sqlite::ejecutar(&ruta, sql, |e| se_confirma(e, &c, confirmar))?;
            if !e.confirmada {
                // se deshizo: el respaldo previo sobra
                if let Some(r) = respaldo.take() {
                    let _ = std::fs::remove_file(r);
                }
            }
            Ok(informe(root, &e, &c, respaldo.as_deref()))
        }
        otra => Err(anyhow!("{}", crate::tr!(
            "acción «{otra}» desconocida: conexiones · detectar · agregar · secreto · probar · esquema · consultar · explicar · ejecutar · respaldo",
            "unknown action «{otra}»: conexiones · detectar · agregar · secreto · probar · esquema · consultar · explicar · ejecutar · respaldo"
        ))),
    }
}

/// Decodifica `%XX` de una URL (una clave con `@` o `:` llega así).
fn sin_porcentajes(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Some(n) = std::str::from_utf8(&b[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(n);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// La clave de una URL de conexión (`esquema://usuario:clave@host/…` o `?password=`).
fn clave_de_url(url: &str) -> Option<String> {
    let resto = url.split_once("://").map(|(_, r)| r).unwrap_or(url);
    let (base, params) = resto.split_once('?').unwrap_or((resto, ""));
    if let Some((cred, _)) = base.rsplit_once('@') {
        if let Some((_, k)) = cred.split_once(':') {
            return Some(sin_porcentajes(k));
        }
    }
    params.split('&').find_map(|kv| kv.strip_prefix("password=")).map(sin_porcentajes)
}

/// Una variable de los `.env` del proyecto (la app abierta desde el Dock no hereda el entorno del shell).
fn variable_del_env(root: &Path, var: &str) -> Option<String> {
    for env in [".env", ".env.local", ".env.development"] {
        let Ok(t) = std::fs::read_to_string(root.join(env)) else { continue };
        for l in t.lines() {
            let l = l.trim().trim_start_matches("export ").trim();
            if let Some((k, v)) = l.split_once('=') {
                if k.trim() == var {
                    return Some(v.trim().trim_matches('"').trim_matches('\'').to_string());
                }
            }
        }
    }
    None
}

/// La clave de una conexión, de donde dice el registro (Llavero o variable); nunca se guarda ni se muestra.
fn clave_de(root: &Path, c: &Conexion) -> Option<String> {
    let s = c.secreto.as_deref()?;
    if let Some(cuenta) = s.strip_prefix("llavero:") {
        return crate::llaves::get(cuenta);
    }
    let var = s.strip_prefix("env:")?;
    let url = std::env::var(var).ok().filter(|v| !v.is_empty()).or_else(|| variable_del_env(root, var))?;
    clave_de_url(&url)
}

/// La herramienta entera: Postgres por la red (async); lo demás (SQLite en proceso), fuera del hilo de tokio.
/// F5 ML9 · dónde se guardan las fotos de los esquemas (una por base registrada).
pub fn dir_esquemas(root: &Path) -> PathBuf {
    root.join(".gotv").join("esquemas")
}

/// F5 ML9 · guarda la foto del esquema de `c` (sin recuentos) para el puente de datos: así el puente
/// compara las consultas del código con la base SIN conectarse en cada edición.
async fn fotografiar(root: &Path, c: &Conexion) {
    let texto = match c.motor {
        Motor::Duckdb => {
            let Some(ruta) = c.archivo.as_deref().and_then(|a| dentro(root, a).ok()) else { return };
            let r = root.to_path_buf();
            tokio::task::spawn_blocking(move || gotv_datos::duckdb::Duck::abrir(&r, Some(&ruta), false).and_then(|mut k| k.esquema(None))).await.ok().and_then(|x| x.ok()).map(|t| sin_recuentos(&t))
        }
        _ => esquema_comparable(root, c).await.ok(),
    };
    if let Some(t) = texto {
        let d = dir_esquemas(root);
        if std::fs::create_dir_all(&d).is_ok() {
            let _ = std::fs::write(d.join(format!("{}.txt", slug(&c.nombre))), t);
        }
    }
}

pub async fn ejecutar_async(args: serde_json::Value, root: PathBuf) -> Result<String> {
    let accion = s(&args, "action").unwrap_or("conexiones").to_string();
    let conexion = resolver(&root, &args).ok().filter(|c| c.motor != Motor::Duckdb || c.archivo.is_some());
    // F5 ML9: antes de CAMBIAR una base, la base de los puentes con su esquema de antes
    if let (true, Some(c)) = (accion == "ejecutar", &conexion) {
        let foto = dir_esquemas(&root).join(format!("{}.txt", slug(&c.nombre)));
        if !foto.is_file() {
            fotografiar(&root, c).await;
            let r = root.clone();
            let _ = tokio::task::spawn_blocking(move || crate::puentes::rebasar(&r)).await;
        } else {
            let r = root.clone();
            let _ = tokio::task::spawn_blocking(move || crate::puentes::asegurar(&r)).await;
        }
    }
    let mut r = ejecutar_async_sin_foto(args.clone(), root.clone()).await;
    // F5 ML9: tras leer o cambiar el esquema, su foto al día (para el puente de datos)
    if let (true, Some(c)) = (r.is_ok() && matches!(accion.as_str(), "probar" | "esquema" | "ejecutar"), &conexion) {
        fotografiar(&root, c).await;
        let rr = root.clone();
        if accion == "ejecutar" {
            // las consultas del código que este cambio dejó sin casar (p. ej. una columna quitada)
            if let Ok(Some(aviso)) = tokio::task::spawn_blocking(move || crate::puentes::tras_cambio_de_esquema(&rr)).await {
                if let Ok(t) = r.as_mut() {
                    t.push('\n');
                    t.push_str(&aviso);
                }
            }
        } else {
            let _ = tokio::task::spawn_blocking(move || crate::puentes::rebasar(&rr)).await;
        }
    }
    r
}

async fn ejecutar_async_sin_foto(args: serde_json::Value, root: PathBuf) -> Result<String> {
    let accion = s(&args, "action").unwrap_or("conexiones").to_string();
    if accion == "comparar" {
        return comparar(&root, &args).await;
    }
    if matches!(accion.as_str(), "probar" | "esquema" | "consultar" | "explicar" | "ejecutar" | "respaldo") {
        if let Ok(c) = resolver(&root, &args) {
            if accion == "respaldo" && matches!(c.motor, Motor::Postgres | Motor::Mysql) {
                let (p, metodo) = respaldo_red(&root, &c, None).await?;
                return Ok(crate::tr!("✓ respaldo: {} ({metodo})", "✓ backup: {} ({metodo})", rel(&root, &p)));
            }
            match c.motor {
                Motor::Postgres => return postgres(&root, &args, c, &accion).await,
                Motor::Mysql => return mysql(&root, &args, c, &accion).await,
                _ => {}
            }
        }
    }
    tokio::task::spawn_blocking(move || ejecutar(&args, &root)).await.map_err(|e| anyhow!("{e}"))?
}

/// F1 paso 6: si la conexión va por un túnel SSH del proyecto, lo abre y devuelve el destino LOCAL del
/// túnel (que vive mientras viva el guardián). Dentro del túnel SSH no se pide TLS salvo que el
/// destino lo exija: el certificado nombra al servidor, no a 127.0.0.1, y el canal ya va cifrado. Una
/// conexión SSH de sólo lectura no deja escribir en la base.
async fn por_tunel(root: &Path, c: &Conexion, accion: &str, mut d: gotv_datos::postgres::Destino) -> Result<(gotv_datos::postgres::Destino, Option<crate::tunel::Tunel>)> {
    let Some(nombre) = &c.tunel else { return Ok((d, None)) };
    if accion == "ejecutar" {
        let reg = crate::conexiones::Registro::cargar(root)?;
        if reg.buscar(nombre).is_some_and(|s| s.solo_lectura) {
            return Err(anyhow!("{}", crate::tr!("la conexión SSH «{nombre}» es de sólo lectura: por ella no se escribe en la base", "SSH connection «{nombre}» is read-only: no database writes go through it")));
        }
    }
    let t = crate::tunel::abrir(root, nombre, &d.host, d.puerto).await?;
    d.host = "127.0.0.1".into();
    d.puerto = t.puerto;
    if d.sslmode == "prefer" {
        d.sslmode = "disable".into();
    }
    Ok((d, Some(t)))
}

async fn postgres(root: &Path, args: &serde_json::Value, c: Conexion, accion: &str) -> Result<String> {
    use gotv_datos::postgres as pg;
    let (d, _tunel) = por_tunel(root, &c, accion, pg::Destino::leer(c.destino.as_deref().unwrap_or(""))?).await?;
    let clave = clave_de(root, &c);
    let mut cli = pg::conectar(&d, clave.as_deref()).await.map_err(|e| {
        if clave.is_none() {
            anyhow!("{e}{}", crate::tr!(" — si pide clave: action=secreto conexion={} (la teclea el usuario)", " — if it needs a password: action=secreto conexion={} (the user types it)", c.nombre))
        } else {
            e
        }
    })?;
    drop(clave);
    let l = limites(args);
    let sql = || s(args, "sql").ok_or_else(|| anyhow!("{}", crate::i18n::t("falta `sql`", "missing `sql`")));
    match accion {
        "probar" => pg::probar(&mut cli).await,
        "esquema" => pg::esquema(&mut cli, s(args, "tabla")).await,
        "explicar" => pg::explicar(&mut cli, sql()?, args.get("analizar").and_then(|v| v.as_bool()).unwrap_or(false), l.ms).await,
        "consultar" => {
            let r = pg::consultar(&mut cli, sql()?, &l).await?;
            Ok(tabla_para(&r, &c, args, &l))
        }
        _ => {
            let confirmar = args.get("confirmar_filas").and_then(|v| v.as_u64()).map(|n| n as usize);
            let previo = respaldo_previo(root, &c, sql()?, confirmar).await?;
            let e = pg::ejecutar(&mut cli, sql()?, |e| se_confirma(e, &c, confirmar)).await?;
            Ok(informe(root, &e, &c, sin_sobrante(previo, e.confirmada).as_deref()))
        }
    }
}

async fn mysql(root: &Path, args: &serde_json::Value, c: Conexion, accion: &str) -> Result<String> {
    use gotv_datos::mysql as my;
    let (d, _tunel) = por_tunel(root, &c, accion, my::destino(c.destino.as_deref().unwrap_or(""))?).await?;
    let clave = clave_de(root, &c);
    let mut cli = my::conectar(&d, clave.as_deref()).await.map_err(|e| {
        if clave.is_none() {
            anyhow!("{e}{}", crate::tr!(" — si pide clave: action=secreto conexion={} (la teclea el usuario)", " — if it needs a password: action=secreto conexion={} (the user types it)", c.nombre))
        } else {
            e
        }
    })?;
    let l = limites(args);
    let sql = || s(args, "sql").ok_or_else(|| anyhow!("{}", crate::i18n::t("falta `sql`", "missing `sql`")));
    let r = match accion {
        "probar" => my::probar(&mut cli).await,
        "esquema" => my::esquema(&mut cli, s(args, "tabla")).await,
        "explicar" => my::explicar(&mut cli, sql()?, args.get("analizar").and_then(|v| v.as_bool()).unwrap_or(false), l.ms).await,
        "consultar" => my::consultar(&mut cli, &d, clave.as_deref(), sql()?, &l).await.map(|r| tabla_para(&r, &c, args, &l)),
        _ => {
            let confirmar = args.get("confirmar_filas").and_then(|v| v.as_u64()).map(|n| n as usize);
            // DDL o tablas sin transacción: no hay ensayo; se ejecuta sólo con confirmar_filas=0 explícito
            match respaldo_previo(root, &c, sql()?, confirmar).await {
                Err(e) => Err(e),
                Ok(previo) => my::ejecutar(&mut cli, sql()?, confirmar == Some(0), |e| se_confirma(e, &c, confirmar)).await.map(|e| informe(root, &e, &c, sin_sobrante(previo, e.confirmada).as_deref())),
            }
        }
    };
    drop(clave);
    let _ = cli.disconnect().await;
    r
}

/// La tabla que ve el modelo: columnas de `mascara` y secretas ocultas; con un modelo de NUBE (`nube`, lo pone
/// el agente), también las personales (correo, teléfono, documento…); y la marca de producción.
fn tabla_para(r: &gotv_datos::Resultado, c: &Conexion, args: &serde_json::Value, l: &Limites) -> String {
    let nube = args.get("nube").and_then(|v| v.as_bool()).unwrap_or(false);
    let mut mascara = c.mascara.clone();
    let personales = if nube { r.personales() } else { vec![] };
    mascara.extend(personales.iter().cloned());
    let mut t = r.tabla(&mascara, l.caracteres);
    if c.produccion {
        t.insert_str(0, crate::i18n::t("[PRODUCCIÓN · sólo lectura]\n", "[PRODUCTION · read-only]\n"));
    }
    if !personales.is_empty() {
        t.push_str(&crate::tr!("\n(modelo de nube: ocultas por ser personales: {})", "\n(cloud model: hidden as personal data: {})", personales.join(", ")));
    }
    t
}

/// Tope de filas por consulta con un modelo de nube (las filas viajan al proveedor).
pub const FILAS_NUBE: u64 = 50;

/// F1 paso 7 · lo que decide el USUARIO antes de que la herramienta corra (lo llama el agente, que tiene el
/// frente y sabe si el modelo es local):
/// - con un modelo de NUBE, la primera `consultar` de cada base en el chat pide permiso (las filas viajan
///   al proveedor); después, como mucho `FILAS_NUBE` filas y las columnas personales ocultas;
/// - en una base de PRODUCCIÓN, la escritura que ya trae `confirmar_filas` (la que haría `COMMIT`) exige
///   que el usuario TECLEE el nombre de la base.
/// Devuelve los argumentos con los que correr, o el motivo para no correr.
pub async fn antes_de(root: &Path, args: &serde_json::Value, local: bool, front: &(impl crate::frontend::Frontend + ?Sized), aprobadas: &mut std::collections::BTreeSet<String>) -> std::result::Result<serde_json::Value, String> {
    let accion = s(args, "action").unwrap_or("");
    let Ok(c) = resolver(root, args) else { return Ok(args.clone()) };
    let mut a = args.clone();
    if !local && accion == "consultar" {
        if !aprobadas.contains(&c.nombre) {
            let si = crate::i18n::t("Sí, para esta base en este chat", "Yes, for this database in this chat").to_string();
            let no = crate::i18n::t("No", "No").to_string();
            let q = crate::tr!(
                "El modelo es de NUBE: las filas que lea de la base «{}» viajarán al proveedor. Irán como mucho {FILAS_NUBE} por consulta y las columnas personales (correo, teléfono, documento…) ocultas. ¿Sigo?",
                "The model runs in the CLOUD: the rows it reads from database «{}» will travel to the provider. At most {FILAS_NUBE} per query, with personal columns (email, phone, ID…) hidden. Go ahead?",
                c.nombre
            );
            match front.choose(&q, &[si.clone(), no]).await {
                Some(r) if r == si => {
                    aprobadas.insert(c.nombre.clone());
                }
                _ => {
                    return Err(crate::tr!(
                        "el usuario NO autorizó enviar filas de «{}» al modelo de nube: no la consultes; si hace falta, que cambie a un modelo local",
                        "the user did NOT authorize sending rows of «{}» to the cloud model: do not query it; if needed, they can switch to a local model",
                        c.nombre
                    ))
                }
            }
        }
        a["nube"] = serde_json::json!(true);
        let f = a.get("filas").and_then(|v| v.as_u64()).unwrap_or(FILAS_NUBE).min(FILAS_NUBE);
        a["filas"] = serde_json::json!(f);
    }
    if accion == "ejecutar" && c.produccion && args.get("confirmar_filas").is_some() {
        let sql: String = s(args, "sql").unwrap_or("").chars().take(300).collect();
        let q = crate::tr!(
            "⚠ PRODUCCIÓN · «{}»\n{sql}\nPara aplicarlo, escribe el nombre exacto de la base.",
            "⚠ PRODUCTION · «{}»\n{sql}\nTo apply it, type the exact database name.",
            c.nombre
        );
        match front.choose(&q, &[]).await {
            Some(r) if r.trim() == c.nombre => {}
            _ => {
                return Err(crate::tr!(
                    "no se aplicó: en una base de PRODUCCIÓN el usuario debe teclear su nombre «{}» y no lo hizo",
                    "not applied: on a PRODUCTION database the user must type its name «{}» and did not",
                    c.nombre
                ))
            }
        }
    }
    Ok(a)
}

/// Los CSV, Parquet y JSON del proyecto (sin dependencias ni carpetas ocultas; 4 niveles, 60 como mucho).
fn archivos_de_datos(root: &Path) -> Vec<String> {
    fn recorrer(root: &Path, dir: &Path, nivel: usize, out: &mut Vec<String>) {
        if nivel > 4 || out.len() >= 60 {
            return;
        }
        let Ok(e) = std::fs::read_dir(dir) else { return };
        let mut e: Vec<_> = e.flatten().collect();
        e.sort_by_key(|x| x.file_name());
        for x in e {
            let n = x.file_name().to_string_lossy().into_owned();
            let p = x.path();
            if p.is_dir() {
                if !n.starts_with('.') && !["node_modules", "target", "vendor", "dist", "build", "venv"].contains(&n.as_str()) {
                    recorrer(root, &p, nivel + 1, out);
                }
            } else if ["csv", "tsv", "parquet", "json", "jsonl", "ndjson"].contains(&p.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase().as_str()) {
                out.push(p.strip_prefix(root).unwrap_or(&p).display().to_string());
            }
        }
    }
    let mut out = Vec::new();
    recorrer(root, root, 0, &mut out);
    out
}

/// F1 paso 9 · las acciones sobre DuckDB: un `.duckdb` del proyecto, o `archivos` (CSV/Parquet/JSON con SQL).
fn duck(root: &Path, args: &serde_json::Value, c: &Conexion, accion: &str) -> Result<String> {
    use gotv_datos::duckdb::Duck;
    let ruta = match &c.archivo {
        Some(a) => Some(dentro(root, a)?),
        None => None,
    };
    let l = limites(args);
    let sql = || s(args, "sql").ok_or_else(|| anyhow!("{}", crate::i18n::t("falta `sql`", "missing `sql`")));
    match accion {
        "probar" => {
            let k = Duck::abrir(root, ruta.as_deref(), false)?;
            let v = k.version.clone();
            Ok(match &ruta {
                Some(p) => crate::tr!("✓ DuckDB {v} · {} (abierta en sólo lectura para leer; sólo alcanza el proyecto)", "✓ DuckDB {v} · {} (opened read-only for reading; only reaches the project)", rel(root, p)),
                None => crate::tr!("✓ DuckDB {v} · los CSV, Parquet y JSON del proyecto, con SQL: SELECT * FROM 'datos/archivo.csv' (sólo alcanza el proyecto)", "✓ DuckDB {v} · the project's CSV, Parquet and JSON files, with SQL: SELECT * FROM 'data/file.csv' (only reaches the project)"),
            })
        }
        "esquema" => {
            let mut k = Duck::abrir(root, ruta.as_deref(), false)?;
            match (s(args, "tabla"), &ruta) {
                // un archivo de datos: sus columnas
                (Some(t), _) if root.join(t).is_file() => k.columnas_de_archivo(t),
                (t, Some(_)) => k.esquema(t),
                (None, None) => {
                    let lista = archivos_de_datos(root);
                    if lista.is_empty() {
                        return Ok(crate::i18n::t("No hay CSV, Parquet ni JSON en el proyecto.", "There are no CSV, Parquet or JSON files in the project.").into());
                    }
                    Ok(lista.iter().map(|a| k.columnas_de_archivo(a).unwrap_or_else(|e| format!("{a}: ✗ {e}"))).collect::<Vec<_>>().join("\n"))
                }
                (Some(t), None) => Err(anyhow!("{}", crate::tr!("no existe el archivo «{t}» en el proyecto", "file «{t}» does not exist in the project"))),
            }
        }
        "consultar" => {
            let mut k = Duck::abrir(root, ruta.as_deref(), false)?;
            let r = k.consultar(sql()?, &l)?;
            Ok(tabla_para(&r, c, args, &l))
        }
        "explicar" => {
            let mut k = Duck::abrir(root, ruta.as_deref(), false)?;
            k.explicar(sql()?, args.get("analizar").and_then(|v| v.as_bool()).unwrap_or(false))
        }
        "respaldo" | "ejecutar" => {
            let Some(p) = ruta else {
                return Err(anyhow!("{}", crate::i18n::t(
                    "los CSV, Parquet y JSON no se cambian con SQL: para crear uno nuevo, una base .duckdb o write_file",
                    "CSV, Parquet and JSON files are not changed with SQL: to make a new one, a .duckdb database or write_file"
                )));
            };
            let copia = |destino: Option<&str>| -> Result<PathBuf> {
                let dir = root.join(".gotv").join("respaldos");
                std::fs::create_dir_all(&dir)?;
                let prefijo = format!("{}-", slug(&c.nombre));
                let destino = match destino {
                    Some(d) => dentro(root, d)?,
                    None => {
                        podar_respaldos(&dir, &prefijo);
                        let hora = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
                        dir.join(format!("{prefijo}{hora}.duckdb"))
                    }
                };
                if destino.exists() {
                    anyhow::bail!("{}", crate::tr!("{} ya existe: no lo piso", "{} already exists: I will not overwrite it", rel(root, &destino)));
                }
                std::fs::copy(&p, &destino)?;
                Ok(destino)
            };
            if accion == "respaldo" {
                let d = copia(s(args, "archivo"))?; // juez-idioma: datos (nombre de un argumento de la herramienta)
                return Ok(crate::tr!("✓ respaldo: {}", "✓ backup: {}", rel(root, &d)));
            }
            let confirmar = args.get("confirmar_filas").and_then(|v| v.as_u64()).map(|n| n as usize);
            let peligrosa = matches!(clasificar::clasificar(sql()?, Motor::Duckdb), Ok(clasificar::Clase::Escritura { peligrosa: Some(_), .. }));
            // la copia previa, con el archivo CERRADO (DuckDB es un solo archivo)
            let previo = if peligrosa && confirmar.is_some() { Some(copia(None)?) } else { None };
            let mut k = Duck::abrir(root, Some(&p), true)?;
            let e = k.ejecutar(sql()?, |e| se_confirma(e, c, confirmar))?;
            drop(k);
            Ok(informe(root, &e, c, sin_sobrante(previo, e.confirmada).as_deref()))
        }
        _ => unreachable!(),
    }
}

/// ¿Es `archivo` un respaldo de la base con este `prefijo`? (`<prefijo><segundos>[.ext]`: así `app-` no
/// confunde los respaldos de `app` con los de `app-2`)
fn es_respaldo_de(archivo: &str, prefijo: &str) -> bool {
    archivo.strip_prefix(prefijo).is_some_and(|r| {
        let n = r.split('.').next().unwrap_or("");
        !n.is_empty() && n.chars().all(|c| c.is_ascii_digit())
    })
}

/// Deja sitio para un respaldo nuevo de la base: quita los más viejos de ESA base (archivos o carpetas,
/// por su ruta exacta, sólo dentro de `.gotv/respaldos`) hasta quedar por debajo del tope.
fn podar_respaldos(dir: &Path, prefijo: &str) {
    let mut previos: Vec<PathBuf> = std::fs::read_dir(dir).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| es_respaldo_de(n, prefijo))).collect();
    previos.sort();
    while previos.len() >= RESPALDOS_POR_BASE {
        let p = previos.remove(0);
        let _ = if p.is_dir() { std::fs::remove_dir_all(&p) } else { std::fs::remove_file(&p) };
    }
}

/// Un archivo temporal 0600 dentro de `.gotv` (la clave para `pg_dump`/`mysqldump`: nunca en la línea de
/// órdenes, que cualquiera ve con `ps`, ni en el entorno). Se borra al soltarlo.
struct ArchivoClave(PathBuf);
impl ArchivoClave {
    fn nuevo(root: &Path, contenido: &str) -> Result<ArchivoClave> {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let dir = root.join(".gotv");
        std::fs::create_dir_all(&dir)?;
        let p = dir.join(format!(".clave-{}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0)));
        let mut f = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&p)?;
        f.write_all(contenido.as_bytes())?;
        Ok(ArchivoClave(p))
    }
}
impl Drop for ArchivoClave {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Corre un programa de respaldo con tope de 10 minutos; error con lo que dijo si falla.
async fn correr_respaldo(mut k: tokio::process::Command) -> Result<()> {
    k.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::piped()).kill_on_drop(true);
    let o = tokio::time::timeout(std::time::Duration::from_secs(600), k.output()).await.map_err(|_| anyhow!("{}", crate::i18n::t("el respaldo pasó de 10 minutos y se cortó", "the backup went past 10 minutes and was cut")))??;
    if o.status.success() {
        Ok(())
    } else {
        Err(anyhow!("{}", String::from_utf8_lossy(&o.stderr).trim().chars().take(600).collect::<String>()))
    }
}

/// F1 paso 10 · respaldo de una base de RED, por la escalera de L1: el binario nativo del motor si está
/// (`pg_dump`; `mysqldump` o `mariadb-dump`), con la clave en un archivo 0600; si no está o falla (p. ej.
/// versión menor que la del servidor), exportación propia de GOTV: esquema + un CSV por tabla. Con `solo`,
/// sólo esas tablas (y siempre por exportación). Devuelve la ruta y el método.
async fn respaldo_red(root: &Path, c: &Conexion, solo: Option<Vec<String>>) -> Result<(PathBuf, String)> {
    let dir = root.join(".gotv").join("respaldos");
    std::fs::create_dir_all(&dir)?;
    let prefijo = format!("{}-", slug(&c.nombre));
    podar_respaldos(&dir, &prefijo);
    let hora = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let base = dir.join(format!("{prefijo}{hora}"));
    let clave = clave_de(root, c);
    let mut aviso = String::new();
    match c.motor {
        Motor::Postgres => {
            use gotv_datos::postgres as pg;
            let (d, _tunel) = por_tunel(root, c, "respaldo", pg::Destino::leer(c.destino.as_deref().unwrap_or(""))?).await?;
            if let (None, Some(prog)) = (&solo, crate::entorno::donde("pg_dump")) {
                let esc = |s: &str| s.replace('\\', "\\\\").replace(':', "\\:");
                let pass = ArchivoClave::nuevo(root, &format!("{}:{}:{}:{}:{}\n", esc(&d.host), d.puerto, esc(&d.base), esc(&d.usuario), esc(clave.as_deref().unwrap_or(""))))?;
                let destino = base.with_extension("dump");
                let mut k = tokio::process::Command::new(prog);
                k.args(["-h", &d.host, "-p", &d.puerto.to_string(), "-U", &d.usuario, "-d", &d.base, "-Fc", "--no-password", "-f"]).arg(&destino);
                k.env("PATH", crate::entorno::path()).env("PGPASSFILE", &pass.0).env("PGSSLMODE", &d.sslmode).env("PGCONNECT_TIMEOUT", "10");
                match correr_respaldo(k).await {
                    Ok(()) => return Ok((destino, "pg_dump".into())),
                    Err(e) => {
                        let _ = std::fs::remove_file(&destino);
                        aviso = crate::tr!(" (pg_dump falló: {e})", " (pg_dump failed: {e})");
                    }
                }
            }
            let mut cli = pg::conectar(&d, clave.as_deref()).await?;
            let hechos = pg::exportar(&mut cli, &base, solo.as_deref()).await?;
            Ok((base, crate::tr!("exportación de GOTV: {} tablas{aviso}", "GOTV export: {} tables{aviso}", hechos.len())))
        }
        Motor::Mysql => {
            use gotv_datos::mysql as my;
            let (d, _tunel) = por_tunel(root, c, "respaldo", my::destino(c.destino.as_deref().unwrap_or(""))?).await?;
            let prog = crate::entorno::donde("mysqldump").or_else(|| crate::entorno::donde("mariadb-dump"));
            if let (None, Some(prog), false) = (&solo, prog, d.base.is_empty()) {
                let k_esc = clave.as_deref().unwrap_or("").replace('\\', "\\\\").replace('"', "\\\"");
                let cnf = ArchivoClave::nuevo(root, &format!("[client]\npassword=\"{k_esc}\"\n"))?;
                let destino = base.with_extension("sql");
                let mut k = tokio::process::Command::new(prog);
                // --defaults-extra-file tiene que ir PRIMERO
                k.arg(format!("--defaults-extra-file={}", cnf.0.display()));
                k.args(["-h", &d.host, "-P", &d.puerto.to_string(), "-u", &d.usuario, "--protocol=TCP", "--single-transaction", "--routines", "--triggers"]).arg(format!("--result-file={}", destino.display())).arg(&d.base);
                k.env("PATH", crate::entorno::path());
                match correr_respaldo(k).await {
                    Ok(()) => return Ok((destino, "mysqldump".into())),
                    Err(e) => {
                        let _ = std::fs::remove_file(&destino);
                        aviso = crate::tr!(" (mysqldump falló: {e})", " (mysqldump failed: {e})");
                    }
                }
            }
            let mut cli = my::conectar(&d, clave.as_deref()).await?;
            let hechos = my::exportar(&mut cli, &base, solo.as_deref()).await?;
            let _ = cli.disconnect().await;
            Ok((base, crate::tr!("exportación de GOTV: {} tablas{aviso}", "GOTV export: {} tables{aviso}", hechos.len())))
        }
        otro => Err(anyhow!("{}", crate::tr!("{} no es una base de red", "{} is not a network database", otro.nombre()))),
    }
}

/// Antes de una escritura PELIGROSA que ya trae su número (la que haría COMMIT) en una base de red: respaldo
/// de las tablas que nombra. Si falla, la escritura no se hace.
async fn respaldo_previo(root: &Path, c: &Conexion, sql: &str, confirmar: Option<usize>) -> Result<Option<PathBuf>> {
    let peligrosa = matches!(clasificar::clasificar(sql, c.motor), Ok(clasificar::Clase::Escritura { peligrosa: Some(_), .. }));
    if !peligrosa || confirmar.is_none() {
        return Ok(None);
    }
    let (p, _) = respaldo_red(root, c, Some(palabras(sql))).await.map_err(|e| anyhow!("{}", crate::tr!("no se escribió: el respaldo previo falló ({e})", "nothing was written: the prior backup failed ({e})")))?;
    Ok(Some(p))
}

/// Si la escritura no se confirmó, el respaldo previo sobra: se quita (su ruta exacta).
fn sin_sobrante(previo: Option<PathBuf>, confirmada: bool) -> Option<PathBuf> {
    match previo {
        Some(p) if !confirmada => {
            let _ = if p.is_dir() { std::fs::remove_dir_all(&p) } else { std::fs::remove_file(&p) };
            None
        }
        otro => otro,
    }
}

/// Las palabras de una sentencia (para saber qué tablas nombra: el respaldo automático sólo copia ésas).
fn palabras(sql: &str) -> Vec<String> {
    let mut v: Vec<String> = sql.split(|ch: char| !(ch.is_alphanumeric() || ch == '_' || ch == '$')).filter(|w| !w.is_empty()).map(|w| w.to_string()).collect();
    v.sort();
    v.dedup();
    v
}

/// F1 paso 10 · el esquema como texto comparable (sin los recuentos de filas, que cambian solos).
async fn esquema_comparable(root: &Path, c: &Conexion) -> Result<String> {
    let texto = match c.motor {
        Motor::Postgres => {
            use gotv_datos::postgres as pg;
            let (d, _t) = por_tunel(root, c, "esquema", pg::Destino::leer(c.destino.as_deref().unwrap_or(""))?).await?;
            let mut cli = pg::conectar(&d, clave_de(root, c).as_deref()).await?;
            pg::esquema(&mut cli, None).await?
        }
        Motor::Mysql => {
            use gotv_datos::mysql as my;
            let (d, _t) = por_tunel(root, c, "esquema", my::destino(c.destino.as_deref().unwrap_or(""))?).await?;
            let mut cli = my::conectar(&d, clave_de(root, c).as_deref()).await?;
            let e = my::esquema(&mut cli, None).await?;
            let _ = cli.disconnect().await;
            e
        }
        _ => {
            let ruta = archivo_sqlite(root, c)?;
            tokio::task::spawn_blocking(move || sqlite::esquema(&ruta, None)).await.map_err(|e| anyhow!("{e}"))??
        }
    };
    Ok(sin_recuentos(&texto))
}

/// Quita el «(~5000 filas)» de cada línea de un esquema.
fn sin_recuentos(t: &str) -> String {
    t.lines()
        .map(|l| match (l.find(" ("), l.find("):")) {
            (Some(a), Some(b)) if a < b && (l[a..b].contains("filas") || l[a..b].contains("rows")) => format!("{}{}", &l[..a], &l[b + 1..]),
            _ => l.to_string(),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// F1 paso 10 · `comparar`: diferencia de esquemas entre la base y `con` (otra base registrada, o un archivo
/// del proyecto con un esquema guardado, p. ej. el `esquema.txt` de un respaldo).
async fn comparar(root: &Path, args: &serde_json::Value) -> Result<String> {
    let a = resolver(root, args)?;
    let con = s(args, "con").ok_or_else(|| anyhow!("{}", crate::i18n::t("falta `con`: otra base registrada o un archivo de esquema del proyecto", "missing `con`: another registered database or a schema file in the project")))?;
    let ta = esquema_comparable(root, &a).await?;
    let (nombre_b, tb) = match registro::buscar(root, con) {
        Some(b) => (b.nombre.clone(), esquema_comparable(root, &b).await?),
        None => {
            let p = dentro(root, con)?;
            (rel(root, &p), sin_recuentos(&std::fs::read_to_string(&p)?))
        }
    };
    let diff = similar::TextDiff::from_lines(&ta, &tb);
    if diff.ratio() == 1.0 {
        return Ok(crate::tr!("✓ «{}» y «{nombre_b}» tienen el mismo esquema", "✓ «{}» and «{nombre_b}» have the same schema", a.nombre));
    }
    Ok(diff.unified_diff().context_radius(1).header(&a.nombre, &nombre_b).to_string())
}

/// ¿Se confirma esta escritura? Lo peligroso, lo masivo y lo de producción, sólo con el número exacto.
fn se_confirma(e: &sqlite::Escritura, c: &Conexion, confirmar: Option<usize>) -> bool {
    let pide = e.peligrosa.is_some() || e.filas > FILAS_SIN_CONFIRMAR || c.produccion;
    !pide || confirmar == Some(e.filas)
}

/// Lo que ve el modelo tras una escritura: confirmada, o deshecha con el número a repetir.
fn informe(root: &Path, e: &sqlite::Escritura, c: &Conexion, respaldo: Option<&Path>) -> String {
    if e.confirmada {
        let mut out = crate::tr!("✓ COMMIT · {} filas", "✓ COMMIT · {} rows", e.filas);
        if e.ddl {
            out.push_str(crate::i18n::t(" (cambió la estructura)", " (structure changed)"));
        }
        if let Some(r) = respaldo {
            out.push_str(&crate::tr!(" · respaldo previo: {}", " · prior backup: {}", rel(root, r)));
        }
        return out;
    }
    let motivo = e.peligrosa.clone().unwrap_or_else(|| {
        if c.produccion { crate::i18n::t("base de PRODUCCIÓN", "PRODUCTION database").into() } else { crate::tr!("más de {FILAS_SIN_CONFIRMAR} filas", "more than {FILAS_SIN_CONFIRMAR} rows") }
    });
    crate::tr!(
        "⚠ NO se aplicó (ROLLBACK): tocaría {} filas · {motivo}. Si es exactamente lo que quieres, repite la MISMA orden con confirmar_filas={}; si el número no es el esperado, revisa el WHERE con action=consultar.",
        "⚠ NOT applied (ROLLBACK): it would touch {} rows · {motivo}. If that is exactly what you want, repeat the SAME command with confirmar_filas={}; if the number is not what you expected, check the WHERE with action=consultar.",
        e.filas,
        e.filas
    )
}

/// F1 paso 8: la clave tecleada en el panel «Bases de datos» de la ventana (acción directa del usuario):
/// al Llavero, y el registro sólo apunta a ella.
pub fn guardar_secreto(root: &Path, nombre: &str, valor: &str) -> Result<String> {
    let mut c = registro::buscar(root, nombre).ok_or_else(|| anyhow!("{}", crate::tr!("«{nombre}» no está registrada", "«{nombre}» is not registered")))?;
    if matches!(c.motor, Motor::Sqlite | Motor::Duckdb) {
        return Err(anyhow!("{}", crate::i18n::t("un archivo SQLite o DuckDB no lleva clave", "a SQLite or DuckDB file has no password")));
    }
    let cuenta = format!("bd/{}/{nombre}", crate::conexiones::id_proyecto(root));
    crate::llaves::set(&cuenta, valor)?;
    c.secreto = Some(format!("llavero:{cuenta}"));
    registro::agregar(root, c)?;
    Ok(cuenta)
}

/// F1 paso 3: la clave de una conexión de red la teclea el USUARIO en un campo tapado (`Frontend::secreto`)
/// y va directo al Llavero (`bd/<id del proyecto>/<nombre>`); el registro sólo guarda DÓNDE está. El
/// modelo ve «guardada», nunca el valor. El rótulo lo arma GOTV con los datos de la conexión: el
/// modelo no puede poner su propio texto en el cuadro (no sirve de anzuelo para pedir otras claves).
pub async fn pedir_secreto(root: &Path, args: &serde_json::Value, front: &(impl crate::frontend::Frontend + ?Sized)) -> Result<String> {
    pedir_secreto_con(root, args, front, |cuenta, valor| crate::llaves::set(cuenta, valor)).await
}

async fn pedir_secreto_con(root: &Path, args: &serde_json::Value, front: &(impl crate::frontend::Frontend + ?Sized), guardar: impl FnOnce(&str, &str) -> Result<()>) -> Result<String> {
    let nombre = s(args, "conexion").ok_or_else(|| anyhow!("{}", crate::i18n::t("falta `conexion` (el nombre registrado)", "missing `conexion` (the registered name)")))?;
    let mut c = registro::buscar(root, nombre).ok_or_else(|| anyhow!("{}", crate::tr!("«{nombre}» no está registrada: primero action=agregar", "«{nombre}» is not registered: action=agregar first")))?;
    if matches!(c.motor, Motor::Sqlite | Motor::Duckdb) {
        return Err(anyhow!("{}", crate::i18n::t("un archivo SQLite o DuckDB no lleva clave", "a SQLite or DuckDB file has no password")));
    }
    let destino = c.destino.clone().unwrap_or_default();
    let etiqueta = crate::tr!(
        "Clave de la base «{nombre}» ({}, {destino}). Va directo al Llavero de macOS: el modelo no la ve.",
        "Password for database «{nombre}» ({}, {destino}). It goes straight to the macOS Keychain: the model never sees it.",
        c.motor.nombre(),
    );
    let Some(valor) = front.secreto(&etiqueta).await else {
        return Err(anyhow!("{}", crate::i18n::t(
            "no se tecleó ninguna clave (o esta interfaz no puede pedirla tapada): que el usuario la teclee en la ventana o en la terminal de GOTV; NUNCA se la pidas por el chat",
            "no password was typed (or this interface cannot ask for it masked): let the user type it in the window or in the GOTV terminal; NEVER ask for it in the chat"
        )));
    };
    let cuenta = format!("bd/{}/{nombre}", crate::conexiones::id_proyecto(root));
    guardar(&cuenta, &valor)?;
    drop(valor);
    c.secreto = Some(format!("llavero:{cuenta}"));
    registro::agregar(root, c)?;
    Ok(crate::tr!("🔑 clave guardada en el Llavero ({cuenta}); el valor no pasó por el chat", "🔑 password saved in the Keychain ({cuenta}); the value did not go through the chat"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::time::Duration;

    fn proyecto() -> crate::CarpetaPrueba {
        let d = crate::CarpetaPrueba::nueva("gotv-base-datos");
        let c = rusqlite::Connection::open(d.join("app.db")).unwrap();
        c.execute_batch("CREATE TABLE usuarios (id INTEGER PRIMARY KEY, nombre TEXT, password TEXT);").unwrap();
        for i in 1..=1_500 {
            c.execute("INSERT INTO usuarios (nombre, password) VALUES (?1, 'x')", [format!("u{i}")]).unwrap();
        }
        d
    }

    fn contar(d: &Path) -> i64 {
        rusqlite::Connection::open(d.join("app.db")).unwrap().query_row("SELECT COUNT(*) FROM usuarios", [], |r| r.get(0)).unwrap()
    }

    /// De punta a punta: detectar, registrar, leer (lo secreto oculto), y escribir con el freno de
    /// las filas y el respaldo de la peligrosa.
    #[test]
    fn base_datos_de_punta_a_punta() {
        let d = proyecto();
        assert!(ejecutar(&json!({"action": "detectar"}), &d).unwrap().contains("app.db"));
        ejecutar(&json!({"action": "agregar", "nombre": "app", "archivo": "app.db"}), &d).unwrap();
        // con una sola registrada, `conexion` sobra
        let t = ejecutar(&json!({"action": "consultar", "sql": "SELECT * FROM usuarios WHERE id <= 3"}), &d).unwrap();
        assert!(t.contains("u1") && !t.contains("| x"), "la columna password va oculta: {t}");
        assert!(ejecutar(&json!({"action": "consultar", "sql": "DELETE FROM usuarios"}), &d).is_err());
        assert!(ejecutar(&json!({"action": "esquema"}), &d).unwrap().contains("usuarios (1500 filas)"));

        // pocas filas, con WHERE: pasa directo
        assert!(ejecutar(&json!({"action": "ejecutar", "sql": "DELETE FROM usuarios WHERE id > 1490"}), &d).unwrap().starts_with("✓ COMMIT · 10"));
        assert_eq!(contar(&d), 1490);
        // más de 1 000: se deshace y pide el número
        let r = ejecutar(&json!({"action": "ejecutar", "sql": "UPDATE usuarios SET nombre = 'z' WHERE id > 100"}), &d).unwrap();
        assert!(r.contains("1390") && r.contains("confirmar_filas=1390"), "{r}");
        // con un número equivocado, tampoco
        assert!(ejecutar(&json!({"action": "ejecutar", "sql": "UPDATE usuarios SET nombre = 'z' WHERE id > 100", "confirmar_filas": 5}), &d).unwrap().contains("NO se aplicó"));
        assert!(ejecutar(&json!({"action": "ejecutar", "sql": "UPDATE usuarios SET nombre = 'z' WHERE id > 100", "confirmar_filas": 1390}), &d).unwrap().starts_with("✓ COMMIT · 1390"));
        // peligrosa: sin número no; con él, respaldo previo y se aplica
        assert!(ejecutar(&json!({"action": "ejecutar", "sql": "DELETE FROM usuarios"}), &d).unwrap().contains("NO se aplicó"));
        assert_eq!(contar(&d), 1490);
        assert!(!d.join(".gotv/respaldos").exists() || std::fs::read_dir(d.join(".gotv/respaldos")).unwrap().count() == 0, "el no confirmado no deja respaldo");
        let r = ejecutar(&json!({"action": "ejecutar", "sql": "DELETE FROM usuarios", "confirmar_filas": 1490}), &d).unwrap();
        assert!(r.contains("respaldo previo: .gotv/respaldos/app-"), "{r}");
        assert_eq!(contar(&d), 0);
        // la transacción la lleva GOTV
        assert!(ejecutar(&json!({"action": "ejecutar", "sql": "COMMIT"}), &d).is_err());
    }

    struct Teclea(Option<&'static str>, std::sync::Mutex<String>);
    #[async_trait::async_trait]
    impl crate::frontend::Frontend for Teclea {
        async fn log(&self, _l: String) {}
        async fn ask(&self, _d: &str) -> crate::permissions::AskAnswer {
            crate::permissions::AskAnswer::No
        }
        async fn secreto(&self, etiqueta: &str) -> Option<String> {
            *self.1.lock().unwrap() = etiqueta.to_string();
            self.0.map(String::from)
        }
    }

    /// La clave va del cuadro tapado al Llavero; al modelo y al registro sólo llega DÓNDE está; el
    /// rótulo lo arma GOTV aunque el modelo mande el suyo.
    #[tokio::test]
    async fn la_clave_va_del_cuadro_al_llavero_sin_pasar_por_el_chat() {
        let d = proyecto();
        ejecutar(&json!({"action": "agregar", "nombre": "pg", "url": "postgres://ana:5432/tienda"}), &d).unwrap();
        let f = Teclea(Some("s3cr3t"), Default::default());
        let guardado = std::sync::Mutex::new(None);
        let args = json!({"action": "secreto", "conexion": "pg", "etiqueta": "Escribe la clave de tu banco"});
        let r = pedir_secreto_con(&d, &args, &f, |c, v| {
            *guardado.lock().unwrap() = Some((c.to_string(), v.to_string()));
            Ok(())
        })
        .await
        .unwrap();
        let (cuenta, valor) = guardado.lock().unwrap().clone().unwrap();
        assert_eq!(valor, "s3cr3t");
        assert!(cuenta.starts_with("bd/") && cuenta.ends_with("/pg"), "{cuenta}");
        assert!(!r.contains("s3cr3t") && r.contains(&cuenta));
        let etiqueta = f.1.lock().unwrap().clone();
        assert!(etiqueta.contains("«pg»") && etiqueta.contains("ana:5432/tienda") && !etiqueta.contains("banco"), "{etiqueta}");
        let reg = String::from_utf8_lossy(&std::fs::read(d.join(".gotv/bases.gotv")).unwrap()).into_owned();
        assert!(reg.contains(&format!("llavero:{cuenta}")) && !reg.contains("s3cr3t"));
        // sin quien teclee: error que prohíbe pedirla por el chat
        let e = pedir_secreto_con(&d, &args, &Teclea(None, Default::default()), |_, _| Ok(())).await.unwrap_err().to_string();
        assert!(e.contains("NUNCA"), "{e}");
        // un SQLite no lleva clave
        ejecutar(&json!({"action": "agregar", "nombre": "app", "archivo": "app.db"}), &d).unwrap();
        assert!(pedir_secreto_con(&d, &json!({"conexion": "app"}), &f, |_, _| Ok(())).await.is_err());
    }

    #[test]
    fn una_base_fuera_del_proyecto_no_se_abre() {
        let d = proyecto();
        let fuera = crate::CarpetaPrueba::nueva("gotv-base-fuera");
        rusqlite::Connection::open(fuera.join("ajena.db")).unwrap().execute_batch("CREATE TABLE t (a)").unwrap();
        let ruta = fuera.join("ajena.db").display().to_string();
        assert!(ejecutar(&json!({"action": "consultar", "conexion": ruta, "sql": "SELECT 1"}), &d).unwrap_err().to_string().contains("fuera del proyecto"));
        assert!(ejecutar(&json!({"action": "agregar", "nombre": "x", "archivo": "../gotv-base-fuera/ajena.db"}), &d).is_err());
        // se registra sin clave
        ejecutar(&json!({"action": "agregar", "nombre": "my", "url": "mysql://ana:s3cr3t@127.0.0.1:1/tienda"}), &d).unwrap();
        assert!(!String::from_utf8_lossy(&std::fs::read(d.join(".gotv/bases.gotv")).unwrap()).contains("s3cr3t"));
    }

    /// La herramienta contra un Postgres REAL: `GOTV_PRUEBA_PG=postgres://usuario:clave@127.0.0.1:puerto/base`
    /// (uno desechable; la prueba crea y borra su propia tabla). La clave se lee de la variable y no
    /// llega al registro ni a la salida.
    #[tokio::test]
    #[ignore = "necesita GOTV_PRUEBA_PG apuntando a un Postgres desechable"]
    async fn base_datos_contra_postgres() {
        let Ok(url) = std::env::var("GOTV_PRUEBA_PG") else { return };
        let d = proyecto();
        let r = d.to_path_buf();
        let h = |a: serde_json::Value| ejecutar_async(a, r.clone());
        h(json!({"action": "agregar", "nombre": "pg", "url": url, "secreto": "env:GOTV_PRUEBA_PG"})).await.unwrap();
        let clave = clave_de_url(&url).unwrap();
        assert!(!String::from_utf8_lossy(&std::fs::read(d.join(".gotv/bases.gotv")).unwrap()).contains(&clave));
        let p = h(json!({"action": "probar", "conexion": "pg"})).await.unwrap();
        assert!(p.contains("PostgreSQL") && !p.contains(&clave), "{p}");
        h(json!({"action": "ejecutar", "conexion": "pg", "sql": "CREATE TABLE gotv_prueba (id serial PRIMARY KEY, nombre text, token text)"})).await.unwrap();
        let w = h(json!({"action": "ejecutar", "conexion": "pg", "sql": "INSERT INTO gotv_prueba (nombre, token) SELECT 'n' || i, 't' FROM generate_series(1, 1500) i"})).await.unwrap();
        assert!(w.contains("NO se aplicó") && w.contains("1500"), "{w}");
        let w = h(json!({"action": "ejecutar", "conexion": "pg", "sql": "INSERT INTO gotv_prueba (nombre, token) SELECT 'n' || i, 't' FROM generate_series(1, 1500) i", "confirmar_filas": 1500})).await.unwrap();
        assert!(w.starts_with("✓ COMMIT · 1500"), "{w}");
        let t = h(json!({"action": "consultar", "conexion": "pg", "sql": "SELECT * FROM gotv_prueba ORDER BY id", "filas": 5})).await.unwrap();
        assert!(t.contains("n1") && t.contains("«oculto»") && t.contains("de 1500"), "{t}");
        assert!(h(json!({"action": "esquema", "conexion": "pg", "tabla": "gotv_prueba"})).await.unwrap().contains("gotv_prueba_pkey"));
        // paso 10: respaldo (pg_dump si está, si no exportación), comparar consigo misma y con el esquema guardado
        let r = h(json!({"action": "respaldo", "conexion": "pg"})).await.unwrap();
        assert!(r.starts_with("✓ respaldo: .gotv/respaldos/pg-"), "{r}");
        assert!(h(json!({"action": "comparar", "conexion": "pg", "con": "pg"})).await.unwrap().contains("mismo esquema"));
        // la peligrosa confirmada deja antes un respaldo de la tabla que nombra
        let w = h(json!({"action": "ejecutar", "conexion": "pg", "sql": "DROP TABLE gotv_prueba", "confirmar_filas": 0})).await.unwrap();
        assert!(w.contains("COMMIT") && w.contains("respaldo previo"), "{w}");
        let previo = std::fs::read_dir(d.join(".gotv/respaldos")).unwrap().flatten().map(|e| e.path()).filter(|p| p.is_dir()).last().unwrap();
        assert!(std::fs::read_to_string(previo.join("public.gotv_prueba.csv")).unwrap().lines().count() == 1501);
        // y ahora el esquema difiere del guardado en el respaldo previo
        let guardado = format!("{}/esquema.txt", rel(&d, &previo));
        assert!(h(json!({"action": "comparar", "conexion": "pg", "con": guardado})).await.unwrap().contains("+table gotv_prueba"));
    }

    /// La herramienta contra un MySQL/MariaDB REAL: `GOTV_PRUEBA_MYSQL=mysql://usuario:clave@127.0.0.1:puerto/base`
    /// (uno desechable; la prueba crea y borra su propia tabla).
    #[tokio::test]
    #[ignore = "necesita GOTV_PRUEBA_MYSQL apuntando a un MySQL o MariaDB desechable"]
    async fn base_datos_contra_mysql() {
        let Ok(url) = std::env::var("GOTV_PRUEBA_MYSQL") else { return };
        let d = proyecto();
        let r = d.to_path_buf();
        let h = |a: serde_json::Value| ejecutar_async(a, r.clone());
        h(json!({"action": "agregar", "nombre": "my", "url": url, "secreto": "env:GOTV_PRUEBA_MYSQL"})).await.unwrap();
        assert!(h(json!({"action": "probar", "conexion": "my"})).await.unwrap().contains("PUEDE escribir"));
        // DDL: sin ensayo posible; sólo con confirmar_filas=0
        let w = h(json!({"action": "ejecutar", "conexion": "my", "sql": "CREATE TABLE gotv_prueba (id INT AUTO_INCREMENT PRIMARY KEY, nombre VARCHAR(20), password VARCHAR(20)) ENGINE=InnoDB"})).await.unwrap();
        assert!(w.contains("NO se aplicó") && w.contains("confirmar_filas=0"), "{w}");
        assert!(h(json!({"action": "ejecutar", "conexion": "my", "sql": "CREATE TABLE gotv_prueba (id INT AUTO_INCREMENT PRIMARY KEY, nombre VARCHAR(20), password VARCHAR(20)) ENGINE=InnoDB", "confirmar_filas": 0})).await.unwrap().contains("COMMIT"));
        let w = h(json!({"action": "ejecutar", "conexion": "my", "sql": "INSERT INTO gotv_prueba (nombre, password) VALUES ('a', 'x'), ('b', 'y')"})).await.unwrap();
        assert!(w.starts_with("✓ COMMIT · 2"), "{w}");
        let t = h(json!({"action": "consultar", "conexion": "my", "sql": "SELECT * FROM gotv_prueba ORDER BY id"})).await.unwrap();
        assert!(t.contains("| a |") && t.contains("«oculto»") && !t.contains("| x"), "{t}");
        assert!(h(json!({"action": "ejecutar", "conexion": "my", "sql": "DELETE FROM gotv_prueba"})).await.unwrap().contains("NO se aplicó"));
        // paso 10: sin mysqldump en esta Mac, exportación propia
        assert!(h(json!({"action": "respaldo", "conexion": "my"})).await.unwrap().contains("✓ respaldo: .gotv/respaldos/my-"));
        let w = h(json!({"action": "ejecutar", "conexion": "my", "sql": "DROP TABLE gotv_prueba", "confirmar_filas": 0})).await.unwrap();
        assert!(w.contains("COMMIT") && w.contains("respaldo previo"), "{w}");
    }

    /// Contenedores desechables de la prueba del túnel: se paran (y se borran, `--rm`) y la red se quita.
    struct Contenedores(Vec<String>, String);
    impl Drop for Contenedores {
        fn drop(&mut self) {
            for n in &self.0 {
                let _ = std::process::Command::new("docker").args(["stop", "-t", "1", n]).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status();
            }
            let _ = std::process::Command::new("docker").args(["network", "rm", &self.1]).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status();
        }
    }

    /// F1 paso 6 de punta a punta: un MariaDB SIN puerto publicado, alcanzable sólo desde un servidor
    /// SSH desechable (los dos en una red Docker propia); GOTV llega por el túnel de la conexión SSH del
    /// proyecto (llave de prueba, huella fijada). Necesita Docker con `mariadb:11.4` y `almalinux:10`
    /// (el contenedor SSH instala `openssh-server` con dnf: requiere red).
    #[tokio::test]
    #[ignore = "levanta dos contenedores desechables (sshd + MariaDB) en Docker"]
    async fn base_datos_por_tunel_ssh() {
        let corre = |a: &[&str]| std::process::Command::new("docker").args(a).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status().is_ok_and(|s| s.success());
        if !corre(&["image", "inspect", "mariadb:11.4"]) || !corre(&["image", "inspect", "almalinux:10"]) {
            eprintln!("sin Docker o sin las imágenes: prueba omitida");
            return;
        }
        let d = proyecto();
        let id = std::process::id();
        let (red, db, sshd) = (format!("gotv-prueba-red-{id}"), format!("gotv-prueba-db-{id}"), format!("gotv-prueba-ssh-{id}"));
        let _guarda = Contenedores(vec![sshd.clone(), db.clone()], red.clone());
        assert!(std::process::Command::new("ssh-keygen").args(["-q", "-t", "ed25519", "-N", "", "-f"]).arg(d.join("llave")).status().unwrap().success());
        let publica = std::fs::read_to_string(d.join("llave.pub")).unwrap();
        assert!(corre(&["network", "create", &red]));
        assert!(corre(&["run", "-d", "--rm", "--name", &db, "--network", &red, "-e", "MARIADB_ROOT_PASSWORD=s3cr3t", "-e", "MARIADB_DATABASE=tienda", "mariadb:11.4"]));
        let puerto = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let guion = format!("dnf -y -q install openssh-server >/dev/null 2>&1 && ssh-keygen -A && mkdir -p /root/.ssh && echo '{}' > /root/.ssh/authorized_keys && chmod 700 /root/.ssh && chmod 600 /root/.ssh/authorized_keys && exec /usr/sbin/sshd -D -e -o AllowTcpForwarding=yes -o PermitRootLogin=prohibit-password", publica.trim());
        assert!(corre(&["run", "-d", "--rm", "--name", &sshd, "--network", &red, "-p", &format!("127.0.0.1:{puerto}:22"), "almalinux:10", "sh", "-c", &guion]));
        // sshd listo cuando saluda con «SSH-» (dnf tarda)
        let mut listo = false;
        for _ in 0..240 {
            if let Ok(mut s) = std::net::TcpStream::connect(("127.0.0.1", puerto)) {
                use std::io::Read;
                let _ = s.set_read_timeout(Some(Duration::from_secs(2)));
                let mut b = [0u8; 4];
                if s.read_exact(&mut b).is_ok() && &b == b"SSH-" {
                    listo = true;
                    break;
                }
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
        assert!(listo, "sshd no arrancó en 240 s");
        let c = crate::conexiones::Conexion { nombre: "servidor".into(), host: "127.0.0.1".into(), usuario: Some("root".into()), puerto: Some(puerto), llave: Some(d.join("llave").display().to_string()), ..Default::default() };
        let (lineas, _) = crate::conexiones::escanear(&c).unwrap();
        crate::conexiones::fijar_huella(&d, &c, &lineas).unwrap();
        crate::conexiones::Registro { conexiones: vec![c] }.guardar(&d).unwrap();
        std::fs::write(d.join(".env"), "DB_TUNEL=mysql://root:s3cr3t@x/tienda\n").unwrap();
        let r = d.to_path_buf();
        let h = |a: serde_json::Value| ejecutar_async(a, r.clone());
        h(json!({"action": "agregar", "nombre": "db", "url": format!("mysql://root@{db}:3306/tienda"), "tunel": "servidor", "secreto": "env:DB_TUNEL"})).await.unwrap();
        assert!(h(json!({"action": "conexiones"})).await.unwrap().contains("por el túnel servidor"));
        // MariaDB tarda en aceptar: hasta 60 s, siempre por el túnel
        let mut p = Err(anyhow!("sin intentos"));
        for _ in 0..60 {
            p = h(json!({"action": "probar", "conexion": "db"})).await;
            if p.is_ok() {
                break;
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
        assert!(p.unwrap().contains("MariaDB"));
        assert!(h(json!({"action": "ejecutar", "conexion": "db", "sql": "CREATE TABLE t (id INT PRIMARY KEY, nombre VARCHAR(10)) ENGINE=InnoDB", "confirmar_filas": 0})).await.unwrap().contains("COMMIT"));
        assert!(h(json!({"action": "ejecutar", "conexion": "db", "sql": "INSERT INTO t VALUES (1, 'uno')"})).await.unwrap().starts_with("✓ COMMIT · 1"));
        assert!(h(json!({"action": "consultar", "conexion": "db", "sql": "SELECT * FROM t"})).await.unwrap().contains("uno"));
        // el túnel quedó en la bitácora y no sobrevive a la llamada
        assert!(std::fs::read_to_string(d.join(".gotv/conexiones.log")).unwrap().contains("tunel -> "));
        tokio::time::sleep(Duration::from_millis(300)).await;
        let vivos = std::process::Command::new("pgrep").args(["-f", &format!("127.0.0.1:[0-9]*:{db}:3306")]).output().unwrap();
        assert!(vivos.stdout.is_empty(), "ningún ssh -L queda vivo tras la llamada");
        // una conexión SSH de sólo lectura no deja escribir por el túnel
        let mut reg = crate::conexiones::Registro::cargar(&d).unwrap();
        reg.conexiones[0].solo_lectura = true;
        reg.guardar(&d).unwrap();
        assert!(h(json!({"action": "ejecutar", "conexion": "db", "sql": "DELETE FROM t WHERE id = 1"})).await.unwrap_err().to_string().contains("sólo lectura"));
    }

    /// Un frente que responde desde una cola y cuenta lo que se le preguntó.
    struct Responde(std::sync::Mutex<Vec<Option<String>>>, std::sync::Mutex<Vec<String>>);
    #[async_trait::async_trait]
    impl crate::frontend::Frontend for Responde {
        async fn log(&self, _l: String) {}
        async fn ask(&self, _d: &str) -> crate::permissions::AskAnswer {
            crate::permissions::AskAnswer::No
        }
        async fn choose(&self, q: &str, _o: &[String]) -> Option<String> {
            self.1.lock().unwrap().push(q.to_string());
            self.0.lock().unwrap().remove(0)
        }
    }

    /// F1 paso 7: con un modelo de nube, la primera consulta de cada base pide permiso UNA vez, baja el tope
    /// a 50 y oculta lo personal; en producción, escribir exige teclear el nombre; en local, nada pregunta.
    #[tokio::test]
    async fn la_nube_y_la_produccion_las_decide_el_usuario() {
        let d = proyecto();
        rusqlite::Connection::open(d.join("app.db")).unwrap().execute_batch("ALTER TABLE usuarios ADD COLUMN correo TEXT; UPDATE usuarios SET correo = nombre || '@x.co';").unwrap();
        ejecutar(&json!({"action": "agregar", "nombre": "app", "archivo": "app.db", "produccion": true}), &d).unwrap();
        let si = "Sí, para esta base en este chat".to_string();
        let f = Responde(std::sync::Mutex::new(vec![Some(si), Some("otra".into()), Some("app".into())]), Default::default());
        let mut ok = std::collections::BTreeSet::new();
        let consulta = json!({"action": "consultar", "sql": "SELECT * FROM usuarios", "filas": 500});
        // local: ni pregunta ni toca los argumentos
        assert_eq!(antes_de(&d, &consulta, true, &f, &mut ok).await.unwrap(), consulta);
        // nube: pregunta una vez; tope 50; lo personal oculto
        let a = antes_de(&d, &consulta, false, &f, &mut ok).await.unwrap();
        assert_eq!((a["nube"].as_bool(), a["filas"].as_u64()), (Some(true), Some(50)));
        antes_de(&d, &consulta, false, &f, &mut ok).await.unwrap();
        assert_eq!(f.1.lock().unwrap().len(), 1, "una sola pregunta por base y chat");
        let t = ejecutar(&a, &d).unwrap();
        assert!(!t.contains("u1@x.co") && t.contains("ocultas por ser personales: correo") && t.contains("PRODUCCIÓN") && t.contains("de 1500"), "{t}");
        // producción: el COMMIT exige el nombre tecleado
        let w = json!({"action": "ejecutar", "sql": "DELETE FROM usuarios WHERE id = 1", "confirmar_filas": 1});
        assert!(antes_de(&d, &w, true, &f, &mut ok).await.unwrap_err().contains("teclear"));
        assert!(antes_de(&d, &w, true, &f, &mut ok).await.is_ok());
        // el primer intento (sin confirmar_filas) no pregunta: se ensaya y se deshace
        assert!(antes_de(&d, &json!({"action": "ejecutar", "sql": "DELETE FROM usuarios WHERE id = 1"}), true, &f, &mut ok).await.is_ok());
        assert_eq!(f.1.lock().unwrap().len(), 3);
        // sin permiso para la nube en otra base: no se consulta
        ejecutar(&json!({"action": "agregar", "nombre": "otra", "archivo": "app.db"}), &d).unwrap();
        let f2 = Responde(std::sync::Mutex::new(vec![Some("No".into())]), Default::default());
        assert!(antes_de(&d, &json!({"action": "consultar", "conexion": "otra", "sql": "SELECT 1"}), false, &f2, &mut ok).await.unwrap_err().contains("NO autorizó"));
    }

    /// Sin servidor: el error lo dice y señala `secreto` si no hay clave; nada se cuelga.
    #[tokio::test]
    async fn mysql_sin_servidor_falla_claro() {
        let d = proyecto();
        ejecutar(&json!({"action": "agregar", "nombre": "my", "url": "mysql://ana@127.0.0.1:1/tienda"}), &d).unwrap();
        let e = ejecutar_async(json!({"action": "consultar", "conexion": "my", "sql": "SELECT 1"}), d.to_path_buf()).await.unwrap_err().to_string();
        assert!(e.contains("no conecté") && e.contains("action=secreto"), "{e}");
    }

    /// F1 paso 9 por la herramienta: `archivos` consulta los CSV del proyecto con SQL; un `.duckdb` se registra,
    /// se lee y se escribe con el mismo freno y con copia previa; los CSV no se cambian con SQL.
    #[test]
    fn duckdb_por_la_herramienta() {
        let d = proyecto();
        std::fs::create_dir_all(d.join("datos")).unwrap();
        let mut csv = String::from("id,ciudad,email\n");
        for i in 1..=300 {
            csv.push_str(&format!("{i},{},u{i}@x.co\n", ["Cali", "Pasto"][i % 2]));
        }
        std::fs::write(d.join("datos/v.csv"), csv).unwrap();
        let t = ejecutar(&json!({"action": "consultar", "conexion": "archivos", "sql": "SELECT ciudad, count(*) AS n FROM 'datos/v.csv' GROUP BY ciudad ORDER BY ciudad"}), &d).unwrap();
        assert!(t.contains("Cali | 150") && t.contains("Pasto | 150"), "{t}");
        assert!(ejecutar(&json!({"action": "esquema", "conexion": "archivos"}), &d).unwrap().contains("datos/v.csv (300 filas): id BIGINT, ciudad VARCHAR, email VARCHAR"));
        assert!(ejecutar(&json!({"action": "ejecutar", "conexion": "archivos", "sql": "DELETE FROM x"}), &d).unwrap_err().to_string().contains("no se cambian con SQL"));
        assert!(ejecutar(&json!({"action": "probar", "conexion": "archivos"}), &d).unwrap().contains("DuckDB"));
        // un .duckdb del proyecto
        {
            // por el proceso aparte, como lo usa la app
            let mut k = gotv_datos::duckdb::Duck::abrir(&d, Some(&d.join("datos/a.duckdb")), true).unwrap();
            assert!(k.ejecutar("CREATE TABLE v AS SELECT * FROM 'datos/v.csv'", |_| true).unwrap().confirmada);
        }
        ejecutar(&json!({"action": "agregar", "nombre": "dk", "archivo": "datos/a.duckdb"}), &d).unwrap();
        assert!(ejecutar(&json!({"action": "esquema", "conexion": "dk"}), &d).unwrap().contains("table v (300 filas)"));
        assert!(ejecutar(&json!({"action": "ejecutar", "conexion": "dk", "sql": "DELETE FROM v"}), &d).unwrap().contains("NO se aplicó"));
        let w = ejecutar(&json!({"action": "ejecutar", "conexion": "dk", "sql": "DELETE FROM v", "confirmar_filas": 300}), &d).unwrap();
        assert!(w.contains("COMMIT · 300") && w.contains("respaldo previo: .gotv/respaldos/dk-"), "{w}");
        assert!(ejecutar(&json!({"action": "consultar", "conexion": "dk", "sql": "SELECT count(*) AS n FROM v"}), &d).unwrap().contains("\n0\n"));
    }

    /// F5 ML9 · el «hecho cuando» del plan: quitar una columna en la base avisa de las consultas del
    /// código que la usan (y sólo de lo nuevo: la tabla que ya no existía antes no se repite).
    #[tokio::test]
    async fn quitar_una_columna_avisa_de_las_consultas_que_la_usan() {
        let d = proyecto();
        std::fs::write(d.join("pyproject.toml"), "[project]\nname = \"x\"\n").unwrap();
        std::fs::write(d.join("app.py"), "def leer(c):\n    c.execute(\"SELECT id, nombre FROM usuarios\")\n    c.execute(\"SELECT * FROM vieja\")\n").unwrap();
        ejecutar(&json!({"action": "agregar", "nombre": "app", "archivo": "app.db"}), &d).unwrap();
        let h = |a: serde_json::Value| ejecutar_async(a, d.to_path_buf());
        // leer el esquema deja su foto y la base de los puentes (la tabla «vieja» ya no casaba: no es nuevo)
        h(json!({"action": "esquema"})).await.unwrap();
        assert!(dir_esquemas(&d).join("app.txt").is_file());
        let w = h(json!({"action": "ejecutar", "sql": "ALTER TABLE usuarios DROP COLUMN nombre", "confirmar_filas": 0})).await.unwrap();
        assert!(w.contains("COMMIT"), "{w}");
        assert!(w.contains("[Puentes]") || w.contains("[Bridges]"), "{w}");
        assert!(w.contains("app.py:2") && w.contains("usuarios.nombre") && !w.contains("vieja"), "{w}");
        // repetirlo (otra escritura) no lo vuelve a decir
        let w = h(json!({"action": "ejecutar", "sql": "UPDATE usuarios SET password = 'y' WHERE id = 1", "confirmar_filas": 1})).await.unwrap();
        assert!(!w.contains("[Puentes]") && !w.contains("[Bridges]"), "{w}");
    }

    #[test]
    fn los_respaldos_de_una_base_no_tocan_los_de_otra() {
        assert!(es_respaldo_de("app-1731650000.db", "app-") && es_respaldo_de("app-1731650000", "app-") && es_respaldo_de("app-1731650000.dump", "app-"));
        assert!(!es_respaldo_de("app-2-1731650000.db", "app-") && !es_respaldo_de("app-.db", "app-"));
        let d = crate::CarpetaPrueba::nueva("gotv-respaldos-poda");
        for n in ["app-1.db", "app-2", "app-3.dump", "app-4.db", "app-5.db", "app-2-9.db"] {
            if n == "app-2" { std::fs::create_dir_all(d.join(n)).unwrap() } else { std::fs::write(d.join(n), "x").unwrap() }
        }
        podar_respaldos(&d, "app-");
        let mut quedan: Vec<String> = std::fs::read_dir(&*d).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        quedan.sort();
        assert_eq!(quedan, ["app-2", "app-2-9.db", "app-3.dump", "app-4.db", "app-5.db"], "deja sitio para uno nuevo (tope 5) y no toca los de app-2");
        assert_eq!(sin_recuentos("table t (~5000 filas): id int\nview v (—) x"), "table t: id int\nview v (—) x");
    }

    #[test]
    fn la_clave_se_lee_de_la_url_o_del_env_sin_guardarse() {
        assert_eq!(clave_de_url("postgres://ana:p%40ss%3Aw@h/b").as_deref(), Some("p@ss:w"));
        assert_eq!(clave_de_url("postgres://h/b?sslmode=require&password=k").as_deref(), Some("k"));
        assert_eq!(clave_de_url("postgres://ana@h/b"), None);
        assert_eq!(sin_porcentajes("100%"), "100%");
        let d = crate::CarpetaPrueba::nueva("gotv-base-env");
        std::fs::write(d.join(".env"), "X=1\nexport GOTV_PRUEBA_URL_BD=\"postgres://u:desde-env@h/b\"\n").unwrap();
        let c = Conexion { nombre: "pg".into(), motor: Motor::Postgres, archivo: None, destino: Some("u@h/b".into()), secreto: Some("env:GOTV_PRUEBA_URL_BD".into()), produccion: false, mascara: vec![], tunel: None };
        assert_eq!(clave_de(&d, &c).as_deref(), Some("desde-env"));
    }
}
