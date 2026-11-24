//! Las conexiones de un proyecto: `.gotv/bases.gotv` (formato propio). Nunca guarda una contraseña: un destino de
//! red se guarda SIN su clave y `secreto` dice DÓNDE está (`env:DATABASE_URL`, `llavero:<nombre>`),
//! jamás su valor.
use crate::Motor;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Conexion {
    pub nombre: String,
    pub motor: Motor,
    /// SQLite/DuckDB: la ruta del archivo (relativa al proyecto si está dentro).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archivo: Option<String>,
    /// Postgres/MySQL: `usuario@host:puerto/base`, sin clave.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub destino: Option<String>,
    /// Dónde está la clave (`env:VAR` o `llavero:nombre`); nunca la clave.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secreto: Option<String>,
    /// Producción: toda escritura pregunta, aun con permisos amplios, y se avisa en cada respuesta.
    #[serde(default)]
    pub produccion: bool,
    /// Columnas que nunca se muestran (además de las que ya parecen secretas por su nombre).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mascara: Vec<String>,
    /// Postgres/MySQL: nombre de una conexión SSH del proyecto (`conexiones.json`) por la que se llega
    /// a la base; `destino` es entonces el host y puerto VISTOS DESDE ese servidor.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tunel: Option<String>,
}

impl Conexion {
    /// La ruta absoluta del archivo de la base (SQLite/DuckDB).
    pub fn ruta(&self, root: &Path) -> Option<PathBuf> {
        self.archivo.as_ref().map(|a| {
            let p = PathBuf::from(a);
            if p.is_absolute() { p } else { root.join(p) }
        })
    }

    /// Una línea para listarla.
    pub fn resumen(&self) -> String {
        let donde = self.archivo.as_deref().or(self.destino.as_deref()).unwrap_or("?");
        let prod = if self.produccion { gotv_engine::i18n::t(" · PRODUCCIÓN", " · PRODUCTION") } else { "" };
        let tunel = self.tunel.as_deref().map(|t| gotv_engine::tr!(" · por el túnel {t}", " · through tunnel {t}")).unwrap_or_default();
        format!("{} ({}): {donde}{tunel}{prod}", self.nombre, self.motor.nombre())
    }
}

/// Formato propio (códec, BASE); hasta 2026-11-17 era `bases.json` (se migra solo la primera vez).
pub fn archivo(root: &Path) -> PathBuf {
    root.join(".gotv").join("bases.gotv")
}

pub fn cargar(root: &Path) -> Vec<Conexion> {
    let f = archivo(root);
    gotv_engine::codec::leer_serde_migrado(&f, &f.with_extension("json"), b"BASE").unwrap_or_default()
}

/// Escritura atómica (temporal + renombrar): un archivo a medias no debe perder las conexiones.
pub fn guardar(root: &Path, conexiones: &[Conexion]) -> Result<()> {
    gotv_engine::codec::guardar_serde(&archivo(root), b"BASE", &conexiones)
}

/// Agrega o reemplaza (por nombre). La que llega se limpia: si su destino trae clave, se quita.
pub fn agregar(root: &Path, mut c: Conexion) -> Result<()> {
    if c.nombre.trim().is_empty() {
        gotv_engine::bail_tr!("la conexión necesita un nombre", "the connection needs a name");
    }
    if let Some(d) = &c.destino {
        let (limpio, tenia) = sin_clave(d);
        if tenia {
            c.destino = Some(limpio);
        }
    }
    let mut todas = cargar(root);
    todas.retain(|x| x.nombre != c.nombre);
    todas.push(c);
    todas.sort_by(|a, b| a.nombre.cmp(&b.nombre));
    guardar(root, &todas)
}

/// Quita una conexión del registro (la base no se toca). Error si no estaba.
pub fn quitar(root: &Path, nombre: &str) -> Result<Conexion> {
    let mut todas = cargar(root);
    let i = todas.iter().position(|c| c.nombre == nombre).ok_or_else(|| gotv_engine::anyhow_tr!("no hay una base «{nombre}» registrada", "there is no registered database «{nombre}»"))?;
    let c = todas.remove(i);
    guardar(root, &todas)?;
    Ok(c)
}

pub fn buscar(root: &Path, nombre: &str) -> Option<Conexion> {
    cargar(root).into_iter().find(|c| c.nombre == nombre)
}

/// Quita la clave de una URL de conexión (`esquema://usuario:clave@host/base` → `usuario@host/base`,
/// sin esquema). Devuelve si tenía clave.
pub fn sin_clave(url: &str) -> (String, bool) {
    let resto = url.split_once("://").map(|(_, r)| r).unwrap_or(url);
    // los parámetros (`?password=…&sslmode=…`) también pueden traerla: fuera los que la tengan
    let (base, params) = resto.split_once('?').map(|(b, p)| (b, Some(p))).unwrap_or((resto, None));
    let mut tenia = false;
    let base = match base.rsplit_once('@') {
        Some((cred, host)) => match cred.split_once(':') {
            Some((usuario, _)) => {
                tenia = true;
                format!("{usuario}@{host}")
            }
            None => format!("{cred}@{host}"),
        },
        None => base.to_string(),
    };
    let params: Vec<&str> = params
        .map(|p| {
            p.split('&')
                .filter(|kv| {
                    let k = kv.split('=').next().unwrap_or("").to_ascii_lowercase();
                    let secreto = k.contains("pass") || k.contains("secret") || k.contains("token") || k == "pwd" || k.contains("key");
                    tenia |= secreto;
                    !secreto && !kv.is_empty()
                })
                .collect()
        })
        .unwrap_or_default();
    (if params.is_empty() { base } else { format!("{base}?{}", params.join("&")) }, tenia)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn la_clave_no_llega_al_registro() {
        assert_eq!(sin_clave("postgres://ana:s3cr3t@db.local:5432/tienda"), ("ana@db.local:5432/tienda".into(), true));
        assert_eq!(sin_clave("mysql://root@localhost/app"), ("root@localhost/app".into(), false));
        assert_eq!(sin_clave("postgresql://h/b?sslmode=require&password=x"), ("h/b?sslmode=require".into(), true));
        // una arroba DENTRO de la clave
        assert_eq!(sin_clave("postgres://ana:p@ss@h/b").0, "ana@h/b");

        let d = gotv_engine::prueba::CarpetaPrueba::nueva("gotv-datos-registro");
        let c = Conexion { nombre: "tienda".into(), motor: Motor::Postgres, archivo: None, destino: Some("postgres://ana:s3cr3t@h/tienda".into()), secreto: Some("env:DATABASE_URL".into()), produccion: true, mascara: vec![], tunel: None };
        agregar(&d, c.clone()).unwrap();
        let texto = String::from_utf8_lossy(&std::fs::read(archivo(&d)).unwrap()).into_owned();
        assert!(!texto.contains("s3cr3t"), "{texto}");
        assert_eq!(buscar(&d, "tienda").unwrap().destino.as_deref(), Some("ana@h/tienda"));
        // reemplaza por nombre, no duplica
        agregar(&d, Conexion { produccion: false, ..c }).unwrap();
        assert_eq!(cargar(&d).len(), 1);
        assert!(!buscar(&d, "tienda").unwrap().produccion);
        assert!(quitar(&d, "tienda").is_ok() && cargar(&d).is_empty() && quitar(&d, "tienda").is_err());
    }
}
