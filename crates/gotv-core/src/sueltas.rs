//! F8.5 · Charlas fuera de proyecto: una pregunta suelta no necesita una carpeta del Portador.
//!
//! Cada charla vive en su propia carpeta, `~/Library/Application Support/gotv/sueltas/<id>/` (fuera de los
//! proyectos, nada del Portador dentro), y abre como un proyecto más con PERMISOS MÍNIMOS: su
//! `.gotv/config.toml` pide el modo `default` (pregunta antes de cada edición o comando) y sin control del
//! Mac. Un `.gotv/config.toml` de proyecto sólo puede ENDURECER (`config.rs`), así que si tu modo general ya
//! es más estricto (`plan`), manda el tuyo.
//!
//! No entran en la lista de proyectos: la ventana las enseña aparte, en la misma zona izquierda.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

/// Lo que se escribe en la config de cada charla (TOML de `config.rs`, como la de cualquier proyecto).
const CONFIG: &str = "mode = \"default\"\ncomputer_use = false\nsystem_changes = false\n"; // juez-idioma: datos (config TOML)

/// La carpeta de todas las charlas sueltas (`GOTV_SUELTAS` la cambia, para pruebas).
pub fn base() -> PathBuf {
    if let Some(p) = std::env::var_os("GOTV_SUELTAS").filter(|p| !p.is_empty()) {
        return PathBuf::from(p);
    }
    PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join("Library/Application Support/gotv/sueltas")
}

/// ¿Es `p` una charla suelta (una carpeta directa de [`base`])?
pub fn es_suelta(p: &Path) -> bool {
    let b = base();
    let canon = |x: &Path| x.canonicalize().unwrap_or_else(|_| x.to_path_buf());
    p.parent().is_some_and(|d| canon(d) == canon(&b))
}

/// Crea una charla nueva (su carpeta y su config de permisos mínimos) y devuelve su ruta.
pub fn nueva() -> Result<PathBuf> {
    let b = base();
    std::fs::create_dir_all(&b).with_context(|| crate::tr!("no pude crear {}", "could not create {}", b.display()))?;
    // una charla en BLANCO (sin ningún mensaje) se reutiliza: con el cambiador Chat/Code cada vuelta a «Chat»
    // abre una charla nueva, y sin esto cada clic dejaba una carpeta vacía (como Claude: una sola en blanco)
    if let Some(vacia) = en_blanco(&b) {
        return Ok(vacia);
    }
    let sello = chrono::Local::now().format("%Y%m%d-%H%M%S").to_string();
    let mut d = b.join(&sello);
    let mut n = 2;
    // dos en el mismo segundo: la segunda lleva número (create_dir falla si ya existe: nunca se reutiliza una)
    while std::fs::create_dir(&d).is_err() {
        d = b.join(format!("{sello}-{n}"));
        n += 1;
        if n > 1000 {
            anyhow::bail!("{}", crate::i18n::t("no pude crear la carpeta de la charla", "could not create the chat folder"));
        }
    }
    std::fs::create_dir_all(d.join(".gotv"))?;
    std::fs::write(d.join(".gotv/config.toml"), CONFIG)?;
    Ok(d)
}

/// La charla en blanco más nueva de `b`: una carpeta de charla (con su `.gotv/config.toml`) sin ninguna sesión.
fn en_blanco(b: &Path) -> Option<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(b)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir() && p.join(".gotv/config.toml").is_file())
        .filter(|p| crate::session::Session::list(p).map(|s| s.iter().all(|x| x.messages == 0)).unwrap_or(true))
        .collect();
    v.sort();
    v.pop()
}

/// Una charla suelta de la lista.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Suelta {
    pub ruta: String,
    pub titulo: String,
    pub creada: String,
    pub mensajes: usize,
}

/// Las charlas sueltas, de la más nueva a la más vieja. Las que nunca recibieron un mensaje no se listan.
pub fn lista() -> Vec<Suelta> {
    let mut v: Vec<(String, Suelta)> = std::fs::read_dir(base())
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().is_dir())
        .filter_map(|e| {
            let ruta = e.path();
            let s = crate::session::Session::list(&ruta).ok()?.into_iter().next()?;
            let nombre = e.file_name().to_string_lossy().to_string();
            Some((nombre, Suelta { ruta: ruta.display().to_string(), titulo: s.title, creada: s.created, mensajes: s.messages }))
        })
        .collect();
    v.sort_by(|a, b| b.0.cmp(&a.0));
    v.into_iter().map(|x| x.1).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Una charla nueva: su carpeta bajo la base, permisos mínimos que la config respeta, y no se lista
    /// hasta que tiene un mensaje.
    #[test]
    fn una_charla_suelta_tiene_su_carpeta_y_permisos_minimos() {
        let b = crate::CarpetaPrueba::sin_crear("gotv-sueltas");
        // SAFETY: única prueba que usa esta variable
        unsafe { std::env::set_var("GOTV_SUELTAS", &*b) };
        let a = nueva().unwrap();
        assert_eq!(nueva().unwrap(), a, "una charla en blanco se reutiliza (el cambiador Chat/Code no deja carpetas vacías)");
        // con un mensaje ya no está en blanco: la siguiente es otra (y en el mismo segundo no se pisan)
        crate::session::Session::open_new(&a, "m").unwrap().save(&[gotv_engine::Message::user("hola")]).unwrap();
        let c = nueva().unwrap();
        assert_ne!(a, c, "dos en el mismo segundo no se pisan");
        assert!(es_suelta(&a) && !es_suelta(&b) && !es_suelta(Path::new("/tmp")));
        let cfg = crate::Config::load(&a);
        // nunca más laxo que `default` (si tu modo general es `plan`, manda el tuyo)
        assert!(matches!(cfg.mode.as_str(), "default" | "plan"), "{}", cfg.mode);
        assert!(!cfg.computer_use);
        assert_eq!(lista().len(), 1, "las vacías no se listan");
        let s = crate::session::Session::open_new(&c, "m").unwrap();
        s.save(&[gotv_engine::Message::user("¿qué hora es en Tokio?")]).unwrap();
        let l = lista();
        assert_eq!((l.len(), l[0].titulo.as_str()), (2, "¿qué hora es en Tokio?"));
        // SAFETY: única prueba que usa esta variable
        unsafe { std::env::remove_var("GOTV_SUELTAS") };
    }
}
