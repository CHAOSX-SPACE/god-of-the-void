//! GOD OF THE VOID · F1 — bases de datos con herramienta propia: mirar, consultar y, con permiso,
//! cambiar una base local o remota, sin instalar nada y sin riesgo de romperla.
//!
//! - [`clasificar`]: qué es una sentencia ANTES de mandarla (lectura, escritura, peligrosa, prohibida).
//! - [`sqlite`]: SQLite en proceso: sólo lectura forzada por el propio motor (apertura de sólo lectura
//!   + autorizador), topes de filas y de tiempo, escritura dentro de una transacción que se confirma
//!   o se deshace, y respaldo en caliente.
//! - [`registro`]: las conexiones del proyecto (`.gotv/bases.gotv`, formato propio, sin secretos).
//! - [`detectar`]: lo que el proyecto ya declara (archivos SQLite, `DATABASE_URL`).
//!
//! - [`postgres`]: PostgreSQL por la red (TLS con rustls): lectura en `BEGIN READ ONLY` con
//!   `statement_timeout` y cursor (el servidor no manda más filas de las pedidas), escritura en
//!   transacción.
//!
//! - [`mysql`]: MySQL y MariaDB (TLS con rustls): `START TRANSACTION READ ONLY`, protocolo binario (una
//!   sentencia por fuerza), `LIMIT` puesto por GOTV, tope de tiempo del servidor y `KILL QUERY`.
//!
//! - [`duckdb`]: DuckDB en su PROCESO APARTE (`gotv-duckdb`): archivos `.duckdb` y CSV/Parquet del proyecto con SQL, con el proyecto
//!   como ÚNICA carpeta alcanzable (`allowed_directories` + `enable_external_access = false`, bloqueado).
pub mod clasificar;
pub mod detectar;
pub mod duckdb;
pub mod mysql;
pub mod postgres;
pub mod referencias;
pub mod registro;
pub mod sqlite;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Motor {
    Sqlite,
    Postgres,
    Mysql,
    Duckdb,
}

impl Motor {
    pub fn nombre(self) -> &'static str {
        match self {
            Motor::Sqlite => "sqlite",
            Motor::Postgres => "postgres",
            Motor::Mysql => "mysql",
            Motor::Duckdb => "duckdb",
        }
    }
}

/// Topes de una consulta.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Limites {
    pub filas: usize,
    pub ms: u64,
    pub caracteres: usize,
}

impl Default for Limites {
    fn default() -> Self {
        Limites { filas: 200, ms: 15_000, caracteres: 20_000 }
    }
}

/// Un valor tal como lo verá el modelo.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Valor {
    Nulo,
    Entero(i64),
    Real(f64),
    Texto(String),
    Binario(usize),
}

impl std::fmt::Display for Valor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Valor::Nulo => write!(f, "NULL"),
            Valor::Entero(n) => write!(f, "{n}"),
            Valor::Real(x) => write!(f, "{x}"),
            Valor::Texto(t) => write!(f, "{}", t.replace(['\n', '\r', '\t'], " ")),
            Valor::Binario(n) => write!(f, "<{n} bytes>"),
        }
    }
}

/// El resultado de una consulta: columnas (nombre, tipo declarado), filas, y si se cortó.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Resultado {
    pub columnas: Vec<(String, String)>,
    pub filas: Vec<Vec<Valor>>,
    /// Había más filas que el tope (y cuántas se contaron, si se contaron).
    pub cortado: bool,
    pub contadas: Option<usize>,
}

/// Columnas que JAMÁS se le muestran al modelo, aunque no se hayan marcado: contraseñas, tokens,
/// secretos. Su contenido sale como «oculto».
pub fn columna_secreta(nombre: &str) -> bool {
    let n = nombre.to_lowercase();
    ["password", "passwd", "contrasena", "contraseña", "clave_hash", "hash_clave", "token", "secret", "secreto", "api_key", "apikey", "private_key"].iter().any(|s| n.contains(s)) // juez-idioma: datos (nombres de columna que se reconocen)
}

/// F1 paso 7: columnas con datos PERSONALES por su nombre (correo, teléfono, documento, dirección): con un
/// modelo de nube salen ocultas, porque las filas viajan al proveedor.
pub fn columna_personal(nombre: &str) -> bool {
    let n = nombre.to_lowercase();
    ["email", "correo", "mail", "phone", "telefono", "teléfono", "celular", "movil", "móvil", "mobile", "cedula", "cédula", "documento", "dni", "nit", "ssn", "passport", "pasaporte", "address", "direccion", "dirección", "iban", "tarjeta", "card_number"] // juez-idioma: datos (nombres de columna que se reconocen)
        .iter()
        .any(|s| n == *s || n.starts_with(&format!("{s}_")) || n.ends_with(&format!("_{s}")) || n.contains(&format!("_{s}_")))
}

impl Resultado {
    /// Las columnas personales de este resultado: por su nombre, o porque más de la mitad de sus valores
    /// parecen un correo.
    pub fn personales(&self) -> Vec<String> {
        let correo = |t: &str| t.split_once('@').is_some_and(|(u, d)| !u.is_empty() && d.contains('.') && !d.starts_with('.') && !t.contains(char::is_whitespace));
        self.columnas
            .iter()
            .enumerate()
            .filter(|(i, (n, _))| {
                if columna_personal(n) {
                    return true;
                }
                let textos: Vec<&str> = self.filas.iter().filter_map(|f| if let Some(Valor::Texto(t)) = f.get(*i) { Some(t.as_str()) } else { None }).collect();
                !textos.is_empty() && textos.iter().filter(|t| correo(t)).count() * 2 > textos.len()
            })
            .map(|(_, (n, _))| n.clone())
            .collect()
    }

    /// Tabla compacta para el modelo: cabecera con tipos, `NULL` explícito, binarios como `<n bytes>`,
    /// las columnas de `mascara` (y las secretas) como «oculto», y el aviso de corte.
    pub fn tabla(&self, mascara: &[String], tope_caracteres: usize) -> String {
        if self.columnas.is_empty() {
            return gotv_engine::i18n::t("(sin columnas)", "(no columns)").into();
        }
        let oculta: Vec<bool> = self.columnas.iter().map(|(n, _)| columna_secreta(n) || mascara.iter().any(|m| m.eq_ignore_ascii_case(n) || m.rsplit('.').next().is_some_and(|c| c.eq_ignore_ascii_case(n)))).collect();
        let cab: Vec<String> = self.columnas.iter().map(|(n, t)| if t.is_empty() { n.clone() } else { format!("{n} ({})", t.to_lowercase()) }).collect();
        let mut s = cab.join(" | ");
        s.push('\n');
        let oculto = gotv_engine::i18n::t("«oculto»", "«hidden»");
        for f in &self.filas {
            let celdas: Vec<String> = f.iter().enumerate().map(|(i, v)| if oculta.get(i) == Some(&true) && *v != Valor::Nulo { oculto.to_string() } else { let t = v.to_string(); if t.chars().count() > 200 { format!("{}…", t.chars().take(200).collect::<String>()) } else { t } }).collect();
            let linea = celdas.join(" | ");
            if s.chars().count() + linea.chars().count() > tope_caracteres {
                s.push_str(&gotv_engine::tr!("… (la tabla pasa de {tope_caracteres} caracteres: acota las columnas o las filas)\n", "… (the table exceeds {tope_caracteres} characters: narrow the columns or rows)\n"));
                break;
            }
            s.push_str(&linea);
            s.push('\n');
        }
        let n = self.filas.len();
        if self.cortado {
            s.push_str(&match self.contadas {
                Some(t) => gotv_engine::tr!("(cortado en {n} de {t} filas: filtra o pagina con LIMIT/OFFSET)", "(cut at {n} of {t} rows: filter or page with LIMIT/OFFSET)"),
                None => gotv_engine::tr!("(cortado en {n} filas; hay más: filtra o pagina con LIMIT/OFFSET)", "(cut at {n} rows; there are more: filter or page with LIMIT/OFFSET)"),
            });
        } else {
            s.push_str(&gotv_engine::tr!("({n} fila(s))", "({n} row(s))"));
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lo_personal_se_reconoce_por_nombre_o_por_contenido() {
        let r = Resultado {
            columnas: vec![("id".into(), "".into()), ("user_email".into(), "".into()), ("contacto".into(), "".into()), ("nota".into(), "".into()), ("nitrogeno".into(), "".into())],
            filas: vec![
                vec![Valor::Entero(1), Valor::Texto("a@x.co".into()), Valor::Texto("b@y.org".into()), Valor::Texto("hola @todos".into()), Valor::Real(1.0)],
                vec![Valor::Entero(2), Valor::Nulo, Valor::Texto("c@z.com".into()), Valor::Texto("nada".into()), Valor::Real(2.0)],
            ],
            cortado: false,
            contadas: None,
        };
        assert_eq!(r.personales(), vec!["user_email".to_string(), "contacto".to_string()]);
        assert!(columna_personal("telefono") && columna_personal("billing_address") && !columna_personal("nitrogeno") && !columna_personal("domain"));
    }

    #[test]
    fn la_tabla_oculta_lo_secreto_y_dice_el_corte() {
        let r = Resultado {
            columnas: vec![("id".into(), "INTEGER".into()), ("correo".into(), "TEXT".into()), ("password_hash".into(), "TEXT".into()), ("foto".into(), "BLOB".into())],
            filas: vec![vec![Valor::Entero(1), Valor::Texto("ana@x.co".into()), Valor::Texto("$2b$12$abc".into()), Valor::Binario(2048)], vec![Valor::Entero(2), Valor::Nulo, Valor::Nulo, Valor::Nulo]],
            cortado: true,
            contadas: Some(5000),
        };
        gotv_engine::i18n::set_thread(Some(gotv_engine::i18n::Lang::Es));
        let t = r.tabla(&["usuarios.correo".into()], 20_000);
        gotv_engine::i18n::set_thread(None);
        assert!(t.starts_with("id (integer) | correo (text) | password_hash (text) | foto (blob)\n"), "{t}");
        assert!(t.contains("1 | «oculto» | «oculto» | <2048 bytes>"), "{t}");
        assert!(t.contains("2 | NULL | NULL | NULL"), "un NULL se dice NULL, no se oculta: {t}");
        assert!(!t.contains("ana@x.co") && !t.contains("$2b$"));
        assert!(t.contains("cortado en 2 de 5000 filas"), "{t}");
    }
}

/// F1 paso 10: un campo de CSV (comillas si hace falta). `None` = NULL.
pub fn campo_csv(v: Option<&str>, nulo: &str) -> String {
    match v {
        None => nulo.to_string(),
        Some(t) if t.contains([',', '"', '\n', '\r']) || t.is_empty() => format!("\"{}\"", t.replace('"', "\"\"")),
        Some(t) => t.to_string(),
    }
}

/// Un nombre de tabla como nombre de archivo (sin `/`, sin `..`).
pub fn nombre_archivo(t: &str) -> String {
    t.chars().map(|c| if c.is_alphanumeric() || matches!(c, '.' | '_' | '-') { c } else { '_' }).collect::<String>().trim_start_matches('.').to_string()
}
