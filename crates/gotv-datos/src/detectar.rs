//! Las bases que ya tiene un proyecto: archivos SQLite/DuckDB (por su cabecera, no por la
//! extensión) y la `DATABASE_URL` de los `.env` (sin su clave: se recuerda DÓNDE está).
use crate::registro::{sin_clave, Conexion};
use crate::Motor;
use std::path::Path;

/// Carpetas que nunca se recorren: dependencias, compilados, ocultas.
const SALTAR: &[&str] = &["node_modules", "target", "vendor", "dist", "build", "__pycache__", "venv", "Pods", "DerivedData"];
const PROFUNDIDAD: usize = 4;
const MAX: usize = 40;

pub fn es_duckdb(f: &Path) -> bool {
    use std::io::Read;
    let mut cab = [0u8; 12];
    std::fs::File::open(f).and_then(|mut a| a.read_exact(&mut cab)).is_ok() && &cab[8..12] == b"DUCK"
}

fn recorrer(root: &Path, dir: &Path, nivel: usize, out: &mut Vec<Conexion>) {
    if nivel > PROFUNDIDAD || out.len() >= MAX {
        return;
    }
    let Ok(entradas) = std::fs::read_dir(dir) else { return };
    let mut entradas: Vec<_> = entradas.flatten().collect();
    entradas.sort_by_key(|e| e.file_name());
    for e in entradas {
        let nombre = e.file_name().to_string_lossy().into_owned();
        let Ok(t) = e.file_type() else { continue };
        if t.is_dir() {
            if !nombre.starts_with('.') && !SALTAR.contains(&nombre.as_str()) {
                recorrer(root, &e.path(), nivel + 1, out);
            }
            continue;
        }
        if !t.is_file() {
            continue;
        }
        let ext = Path::new(&nombre).extension().and_then(|x| x.to_str()).unwrap_or("").to_ascii_lowercase();
        let p = e.path();
        let motor = match ext.as_str() {
            "sqlite" | "sqlite3" | "db" | "db3" if crate::sqlite::es_sqlite(&p) => Motor::Sqlite,
            "duckdb" | "db" if es_duckdb(&p) => Motor::Duckdb,
            _ => continue,
        };
        let rel = p.strip_prefix(root).unwrap_or(&p).display().to_string();
        out.push(Conexion { nombre: rel.clone(), motor, archivo: Some(rel), destino: None, secreto: None, produccion: false, mascara: vec![], tunel: None });
        if out.len() >= MAX {
            return;
        }
    }
}

/// Las URL de conexión de los `.env` del proyecto (raíz): `DATABASE_URL` y parecidas.
fn de_env(root: &Path, out: &mut Vec<Conexion>) {
    for env in [".env", ".env.local", ".env.development"] {
        let Ok(t) = std::fs::read_to_string(root.join(env)) else { continue };
        for l in t.lines() {
            let l = l.trim().trim_start_matches("export ").trim();
            let Some((k, v)) = l.split_once('=') else { continue };
            let k = k.trim();
            if !(k.ends_with("DATABASE_URL") || k.ends_with("DB_URL")) {
                continue;
            }
            let v = v.trim().trim_matches('"').trim_matches('\'');
            let motor = match v.split("://").next().unwrap_or("") {
                "postgres" | "postgresql" => Motor::Postgres,
                "mysql" | "mariadb" => Motor::Mysql,
                "sqlite" | "file" => {
                    let ruta = v.split_once("://").map(|(_, r)| r).unwrap_or(v).trim_start_matches('/');
                    let ruta = if v.contains(":///") { format!("/{ruta}") } else { ruta.to_string() };
                    out.push(Conexion { nombre: format!("{env}:{k}"), motor: Motor::Sqlite, archivo: Some(ruta), destino: None, secreto: None, produccion: false, mascara: vec![], tunel: None });
                    continue;
                }
                _ => continue,
            };
            let (destino, tenia) = sin_clave(v);
            out.push(Conexion {
                nombre: format!("{env}:{k}"),
                motor,
                archivo: None,
                destino: Some(destino),
                secreto: tenia.then(|| format!("env:{k}")),
                produccion: false,
                mascara: vec![],
                tunel: None,
            });
        }
    }
}

/// Lo hallado en el proyecto (no se guarda: quien llama decide si lo registra).
pub fn detectar(root: &Path) -> Vec<Conexion> {
    let mut out = Vec::new();
    de_env(root, &mut out);
    recorrer(root, root, 0, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn halla_las_bases_por_su_cabecera_y_la_url_sin_clave() {
        let d = gotv_engine::prueba::CarpetaPrueba::nueva("gotv-datos-detectar");
        std::fs::create_dir_all(d.join("datos")).unwrap();
        std::fs::create_dir_all(d.join("node_modules/x")).unwrap();
        rusqlite::Connection::open(d.join("datos/app.db")).unwrap().execute_batch("CREATE TABLE t (a)").unwrap();
        rusqlite::Connection::open(d.join("node_modules/x/dep.sqlite")).unwrap().execute_batch("CREATE TABLE t (a)").unwrap();
        std::fs::write(d.join("falso.db"), "no soy una base").unwrap();
        std::fs::write(d.join(".env"), "PORT=3000\nDATABASE_URL=\"postgres://ana:s3cr3t@localhost:5432/tienda\"\n").unwrap();
        let h = detectar(&d);
        let nombres: Vec<&str> = h.iter().map(|c| c.nombre.as_str()).collect();
        assert_eq!(nombres, [".env:DATABASE_URL", "datos/app.db"], "{h:?}");
        assert_eq!(h[0].destino.as_deref(), Some("ana@localhost:5432/tienda"));
        assert_eq!(h[0].secreto.as_deref(), Some("env:DATABASE_URL"));
        assert!(!format!("{h:?}").contains("s3cr3t"));
    }
}
