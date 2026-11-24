//! F1 paso 9 · DuckDB: archivos `.duckdb` del proyecto y, sobre todo, CSV y Parquet del proyecto
//! consultados con SQL (`SELECT … FROM 'datos/ventas.csv'`), sin cargarlos en ninguna base.
//!
//! **Arquitectura (2026-11-15):** DuckDB NO vive en la app. Corre en el proceso aparte `gotv-duckdb`
//! (junto al ejecutable), que esta conexión lanza al abrir y que muere al soltarla: la app no carga sus
//! 62 MB de C++ en cada binario, un fallo de DuckDB no la tumba y su memoria vuelve entera al terminar.
//! Este módulo es el cliente: [`Duck`] con las mismas acciones de antes, por líneas JSON ([`Pedido`] y
//! [`Respuesta`]). La cerca (sólo el proyecto, sin red, configuración bloqueada) y los topes de memoria
//! e hilos los pone el proceso: ver `crates/gotv-duckdb/src/motor.rs`.
use crate::sqlite::Escritura;
use crate::{Limites, Resultado};
use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::Duration;

/// Un literal SQL entre comillas simples.
pub fn texto_sql(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// La cerca de DuckDB compara la ruta LITERAL antes de aplicar `file_search_path`: una relativa del propio
/// proyecto se negaría (medido). GOTV pone absoluta la ruta de los literales que nombran archivos —tras
/// `FROM`/`JOIN` o como primer argumento de `read_*`, `glob`, `parquet_*`, `sniff_csv`—; lo demás no se toca
/// (un `WHERE nombre = 'a.csv'` sigue igual), y DuckDB sigue siendo quien decide si cae dentro.
pub fn rutas_absolutas(sql: &str, root: &Path) -> String {
    let raiz = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let mut out = String::with_capacity(sql.len() + 64);
    let mut resto = sql;
    while let Some(i) = resto.find('\'') {
        let (antes, desde) = resto.split_at(i);
        out.push_str(antes);
        // el literal entero (con '' escapadas)
        let b = desde.as_bytes();
        let mut j = 1;
        while j < b.len() {
            if b[j] == b'\'' {
                if j + 1 < b.len() && b[j + 1] == b'\'' {
                    j += 2;
                    continue;
                }
                break;
            }
            j += 1;
        }
        let fin = (j + 1).min(desde.len());
        let literal = &desde[..fin];
        let previo = out.trim_end().to_ascii_lowercase();
        let palabra = previo.rsplit(|ch: char| !(ch.is_alphanumeric() || ch == '_')).next().unwrap_or("");
        let funcion = previo.strip_suffix('(').map(|p| p.trim_end().rsplit(|ch: char| !(ch.is_alphanumeric() || ch == '_')).next().unwrap_or("").to_string());
        let es_archivo = matches!(palabra, "from" | "join") || funcion.is_some_and(|f| f.starts_with("read_") || f == "glob" || f.starts_with("parquet_") || f == "sniff_csv");
        let contenido = literal.trim_start_matches('\'').trim_end_matches('\'').replace("''", "'");
        if es_archivo && fin > 1 && !contenido.starts_with('/') && !contenido.contains("://") && !contenido.is_empty() {
            out.push_str(&texto_sql(&raiz.join(&contenido).display().to_string()));
        } else {
            out.push_str(literal);
        }
        resto = &desde[fin..];
    }
    out.push_str(resto);
    out
}

/// Lo que la app le pide al proceso de DuckDB (una línea JSON cada uno).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "lowercase")]
pub enum Pedido {
    /// La primera línea: el proyecto (la única carpeta alcanzable), el `.duckdb` (o ninguno: en memoria,
    /// para los CSV/Parquet), el modo y el idioma de los mensajes.
    Abrir { root: PathBuf, archivo: Option<PathBuf>, escribir: bool, en: bool },
    Consultar { sql: String, limites: Limites },
    Esquema { tabla: Option<String> },
    Columnas { archivo: String },
    Explicar { sql: String, analizar: bool },
    /// Escribe dentro de una transacción: responde lo que HARÍA y espera [`Pedido::Decidir`].
    Ejecutar { sql: String },
    Decidir { confirmar: bool },
}

/// Lo que responde el proceso.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Respuesta {
    Texto(String),
    Tabla(Resultado),
    Escritura(Escritura),
    Error(String),
}

/// Dónde está `gotv-duckdb`: `GOTV_DUCKDB`, o junto al ejecutable (la app: `Contents/MacOS`; el banco de
/// pruebas: `target/release`, un nivel arriba de `deps`).
pub fn binario() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("GOTV_DUCKDB") {
        return Some(PathBuf::from(p));
    }
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    std::iter::once(dir.join("gotv-duckdb")).chain(dir.parent().map(|d| d.join("gotv-duckdb"))).find(|p| p.is_file())
}

/// Una conexión a DuckDB: el proceso `gotv-duckdb` vivo mientras dure. Soltarla lo cierra (y DuckDB
/// cierra el archivo, deshace lo no confirmado y devuelve su memoria).
pub struct Duck {
    hijo: Child,
    entrada: Option<ChildStdin>,
    salida: Receiver<String>,
    /// la versión de DuckDB que abrió
    pub version: String,
}

/// Tope para las acciones sin tope propio (esquema, explicar sin ANALYZE): el proceso no debería tardar.
const TOPE_ACCION: Duration = Duration::from_secs(120);
/// Una escritura no tiene tope de tiempo en DuckDB; aquí, 10 min como último recurso.
const TOPE_ESCRITURA: Duration = Duration::from_secs(600);

impl Duck {
    /// Abre DuckDB en su proceso: el `.duckdb` dado (o en memoria, para consultar CSV/Parquet), con el
    /// proyecto como única carpeta alcanzable.
    pub fn abrir(root: &Path, archivo: Option<&Path>, escribir: bool) -> Result<Duck> {
        let bin = binario().ok_or_else(|| anyhow!("{}", gotv_engine::i18n::t(
            "no encuentro `gotv-duckdb` junto a GOTV: la instalación está incompleta (vuelve a instalar la app)",
            "cannot find `gotv-duckdb` next to GOTV: the installation is incomplete (reinstall the app)"
        )))?;
        Duck::abrir_con(&bin, root, archivo, escribir)
    }

    /// Igual, con la ruta del binario dada.
    pub fn abrir_con(bin: &Path, root: &Path, archivo: Option<&Path>, escribir: bool) -> Result<Duck> {
        let mut hijo = Command::new(bin).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().map_err(|e| {
            anyhow!("{}", gotv_engine::tr!("no pude lanzar {}: {e}", "could not start {}: {e}", bin.display()))
        })?;
        let entrada = hijo.stdin.take();
        let salida = hijo.stdout.take().ok_or_else(|| anyhow!("stdout"))?;
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            for l in BufReader::new(salida).lines() {
                let Ok(l) = l else { break };
                if tx.send(l).is_err() {
                    break;
                }
            }
        });
        let mut d = Duck { hijo, entrada, salida: rx, version: String::new() };
        let p = Pedido::Abrir { root: root.to_path_buf(), archivo: archivo.map(Path::to_path_buf), escribir, en: gotv_engine::i18n::is_en() };
        d.version = match d.pedir(&p, Duration::from_secs(60))? {
            Respuesta::Texto(v) => v,
            otra => return Err(inesperada(otra)),
        };
        Ok(d)
    }

    /// El proceso de DuckDB de esta conexión.
    pub fn pid(&self) -> u32 {
        self.hijo.id()
    }

    fn mandar(&mut self, p: &Pedido) -> Result<()> {
        let e = self.entrada.as_mut().ok_or_else(|| anyhow!("{}", gotv_engine::i18n::t("la conexión a DuckDB ya se cerró", "the DuckDB connection is already closed")))?;
        let mut linea = serde_json::to_vec(p)?;
        linea.push(b'\n');
        e.write_all(&linea).and_then(|_| e.flush()).map_err(|_| self.caido())
    }

    fn recibir(&mut self, tope: Duration) -> Result<Respuesta> {
        match self.salida.recv_timeout(tope) {
            Ok(l) => match serde_json::from_str::<Respuesta>(&l)? {
                Respuesta::Error(e) => Err(anyhow!("{e}")),
                r => Ok(r),
            },
            Err(RecvTimeoutError::Timeout) => {
                let _ = self.hijo.kill();
                Err(anyhow!("{}", gotv_engine::tr!(
                    "DuckDB no respondió en {} s y GOTV cerró su proceso",
                    "DuckDB did not answer within {} s and GOTV closed its process",
                    tope.as_secs()
                )))
            }
            Err(RecvTimeoutError::Disconnected) => Err(self.caido()),
        }
    }

    /// El proceso murió: el error dice cómo (la app sigue viva).
    fn caido(&mut self) -> anyhow::Error {
        let como = self.hijo.wait().map(|s| s.to_string()).unwrap_or_default();
        anyhow!("{}", gotv_engine::tr!(
            "el proceso de DuckDB se cayó ({como}); GOTV sigue bien: repite la acción o acótala",
            "the DuckDB process crashed ({como}); GOTV is fine: repeat the action or narrow it"
        ))
    }

    fn pedir(&mut self, p: &Pedido, tope: Duration) -> Result<Respuesta> {
        self.mandar(p)?;
        self.recibir(tope)
    }

    fn texto(&mut self, p: &Pedido) -> Result<String> {
        match self.pedir(p, TOPE_ACCION)? {
            Respuesta::Texto(t) => Ok(t),
            otra => Err(inesperada(otra)),
        }
    }

    /// Consulta de sólo lectura con topes (el de tiempo lo aplica DuckDB; aquí, un margen sobre él).
    pub fn consultar(&mut self, sql: &str, l: &Limites) -> Result<Resultado> {
        let tope = Duration::from_millis(l.ms) + Duration::from_secs(30);
        match self.pedir(&Pedido::Consultar { sql: sql.into(), limites: *l }, tope)? {
            Respuesta::Tabla(t) => Ok(t),
            otra => Err(inesperada(otra)),
        }
    }

    /// El esquema: las tablas y vistas de la base (`.duckdb`).
    pub fn esquema(&mut self, tabla: Option<&str>) -> Result<String> {
        self.texto(&Pedido::Esquema { tabla: tabla.map(Into::into) })
    }

    /// Las columnas de un archivo CSV/Parquet/JSON del proyecto (ruta relativa al proyecto).
    pub fn columnas_de_archivo(&mut self, archivo: &str) -> Result<String> {
        self.texto(&Pedido::Columnas { archivo: archivo.into() })
    }

    pub fn explicar(&mut self, sql: &str, analizar: bool) -> Result<String> {
        self.texto(&Pedido::Explicar { sql: sql.into(), analizar })
    }

    /// Escribe en un `.duckdb` dentro de una transacción; `confirmar` decide `COMMIT` o `ROLLBACK`.
    pub fn ejecutar(&mut self, sql: &str, confirmar: impl FnOnce(&Escritura) -> bool) -> Result<Escritura> {
        let previa = match self.pedir(&Pedido::Ejecutar { sql: sql.into() }, TOPE_ESCRITURA)? {
            Respuesta::Escritura(e) => e,
            otra => return Err(inesperada(otra)),
        };
        let si = confirmar(&previa);
        match self.pedir(&Pedido::Decidir { confirmar: si }, TOPE_ACCION)? {
            Respuesta::Escritura(e) => Ok(e),
            otra => Err(inesperada(otra)),
        }
    }
}

fn inesperada(r: Respuesta) -> anyhow::Error {
    anyhow!("{}", gotv_engine::tr!("respuesta inesperada de DuckDB: {r:?}", "unexpected answer from DuckDB: {r:?}"))
}

impl Drop for Duck {
    fn drop(&mut self) {
        // cerrar su entrada = fin de pedidos: DuckDB cierra el archivo y el proceso sale solo
        self.entrada.take();
        let fin = std::time::Instant::now() + Duration::from_secs(5);
        while std::time::Instant::now() < fin {
            if let Ok(Some(_)) = self.hijo.try_wait() {
                return;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        let _ = self.hijo.kill();
        let _ = self.hijo.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn las_rutas_de_archivo_se_vuelven_absolutas_y_lo_demas_no() {
        let r = Path::new("/p");
        assert_eq!(rutas_absolutas("SELECT * FROM 'a.csv' WHERE x = 'b.csv'", r), "SELECT * FROM '/p/a.csv' WHERE x = 'b.csv'");
        assert_eq!(rutas_absolutas("SELECT * FROM read_csv( 'd/*.csv', header = true) JOIN 'e.parquet' USING (id)", r), "SELECT * FROM read_csv( '/p/d/*.csv', header = true) JOIN '/p/e.parquet' USING (id)");
        assert_eq!(rutas_absolutas("SELECT * FROM '/etc/hosts', 'https://x/y.csv'", r), "SELECT * FROM '/etc/hosts', 'https://x/y.csv'");
        assert_eq!(rutas_absolutas("SELECT 'it''s' FROM 'o''k.csv'", r), "SELECT 'it''s' FROM '/p/o''k.csv'");
    }

    /// El protocolo viaja por JSON sin perder nada (incluido el valor real y el NULL).
    #[test]
    fn el_protocolo_va_y_vuelve() {
        let p = Pedido::Consultar { sql: "SELECT 1".into(), limites: Limites::default() };
        let j = serde_json::to_string(&p).unwrap();
        assert!(j.contains("\"op\":\"consultar\""), "{j}");
        assert!(matches!(serde_json::from_str::<Pedido>(&j).unwrap(), Pedido::Consultar { .. }));
        let t = Resultado { columnas: vec![("x".into(), "DOUBLE".into())], filas: vec![vec![crate::Valor::Real(1.5), crate::Valor::Nulo]], cortado: false, contadas: None };
        let r: Respuesta = serde_json::from_str(&serde_json::to_string(&Respuesta::Tabla(t)).unwrap()).unwrap();
        let Respuesta::Tabla(t) = r else { panic!() };
        assert_eq!(t.filas[0], vec![crate::Valor::Real(1.5), crate::Valor::Nulo]);
    }
}
