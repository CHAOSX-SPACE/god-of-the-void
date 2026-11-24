//! Rutas canónicas (auditoría C4). El modelo pide rutas a su manera: relativas,
//! con `..`, con `~/`, o con una `/` inicial inventada (medido: pidió `/main.rs`
//! queriendo decir el `main.rs` del proyecto). Aquí se resuelven contra la raíz
//! y se decide, sin engaños de symlinks, si caen dentro o fuera del proyecto.

use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};

pub fn home() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| "/".into()))
}

/// Resuelve la ruta pedida por el modelo contra la raíz del proyecto.
/// Las carpetas del usuario que Finder MUESTRA traducidas y que en disco están en inglés (prueba del Portador,
/// 2026-11-18: «créalo en el escritorio» → el modelo escribió `~/Escritorio`, que no existe, y acabó creando una
/// carpeta nueva con ese nombre en vez de usar el escritorio de verdad, `~/Desktop`).
pub const CARPETAS_TRADUCIDAS: &[(&str, &str)] = &[
    ("Escritorio", "Desktop"),
    ("Documentos", "Documents"),
    ("Descargas", "Downloads"),
    ("Imágenes", "Pictures"), // juez-idioma: datos (nombre que muestra Finder)
    ("Imagenes", "Pictures"),
    ("Música", "Music"), // juez-idioma: datos (nombre que muestra Finder)
    ("Musica", "Music"),
    ("Películas", "Movies"), // juez-idioma: datos (nombre que muestra Finder)
    ("Peliculas", "Movies"),
    ("Público", "Public"), // juez-idioma: datos (nombre que muestra Finder)
    ("Publico", "Public"),
]; // juez-idioma: datos (nombres que muestra Finder)

/// `~/Escritorio/x` → `~/Desktop/x` si la traducida NO existe en disco y la real sí (si alguien creó de verdad una
/// carpeta «Escritorio», se respeta). `None` = la ruta no empieza por una carpeta traducida.
pub fn carpeta_real(p: &Path) -> Option<PathBuf> {
    carpeta_real_en(&home(), p)
}

fn carpeta_real_en(casa: &Path, p: &Path) -> Option<PathBuf> {
    let resto = p.strip_prefix(casa).ok()?;
    let primera = resto.components().next()?.as_os_str().to_str()?;
    let (_, real) = CARPETAS_TRADUCIDAS.iter().find(|(t, _)| *t == primera)?;
    let traducida = casa.join(primera);
    let dentro = resto.strip_prefix(primera).ok()?;
    (!traducida.exists() && casa.join(real).is_dir()).then(|| casa.join(real).join(dentro))
}

pub fn resolve(root: &Path, requested: &str) -> PathBuf {
    let p = requested.trim();
    if p.is_empty() || p == "." {
        return normalize(root);
    }
    // F8.4: `@nombre/…` va a la carpeta extra de ese nombre (si el proyecto la tiene)
    if p.starts_with('@') {
        if let Some(x) = crate::raices::resolver_extra(root, p) {
            return x;
        }
    }
    let expanded = if p == "~" {
        home()
    } else if let Some(rest) = p.strip_prefix("~/") {
        home().join(rest)
    } else {
        PathBuf::from(p)
    };
    // `~/Escritorio/…` es `~/Desktop/…` (Finder lo muestra traducido)
    let expanded = carpeta_real(&expanded).unwrap_or(expanded);
    let candidate = if expanded.is_absolute() {
        // Una absoluta es real si existe o si su carpeta padre existe (y no es
        // la raíz del disco). Si no, es la `/x` inventada: relativa al proyecto.
        let parent_real = expanded
            .parent()
            .map(|q| q != Path::new("/") && q.exists())
            .unwrap_or(false);
        if expanded.exists() || parent_real {
            expanded
        } else {
            root.join(p.trim_start_matches('/'))
        }
    } else {
        root.join(expanded)
    };
    normalize(&candidate)
}

/// Normalización léxica: quita `.` y resuelve `..` sin tocar el disco.
pub fn normalize(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Canoniza el tramo existente más largo (resuelve symlinks) y le añade el resto.
fn canonical_lenient(p: &Path) -> PathBuf {
    let mut cur = p.to_path_buf();
    let mut rest: Vec<OsString> = Vec::new();
    loop {
        if let Ok(c) = cur.canonicalize() {
            let mut out = c;
            for r in rest.iter().rev() {
                out.push(r);
            }
            return out;
        }
        match cur.file_name() {
            Some(name) => {
                rest.push(name.to_os_string());
                cur.pop();
            }
            None => return p.to_path_buf(),
        }
    }
}

/// ¿La ruta (ya resuelta) cae dentro del proyecto? Sigue symlinks: un enlace
/// dentro del proyecto que apunta afuera cuenta como AFUERA.
pub fn is_inside(root: &Path, p: &Path) -> bool {
    canonical_lenient(p).starts_with(canonical_lenient(root))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Prueba del Portador: «Escritorio» es `Desktop` en disco; si alguien creó de verdad una carpeta con el nombre
    /// traducido, se respeta; y lo que no empieza por una carpeta traducida no se toca.
    #[test]
    fn las_carpetas_que_finder_traduce() {
        let casa = crate::CarpetaPrueba::nueva("gotv-casa-traducida");
        std::fs::create_dir_all(casa.join("Desktop")).unwrap();
        std::fs::create_dir_all(casa.join("Documents")).unwrap();
        assert_eq!(carpeta_real_en(&casa, &casa.join("Escritorio/prueba.txt")), Some(casa.join("Desktop/prueba.txt")));
        assert_eq!(carpeta_real_en(&casa, &casa.join("Documentos")), Some(casa.join("Documents")));
        assert_eq!(carpeta_real_en(&casa, &casa.join("Proyectos/x")), None);
        assert_eq!(carpeta_real_en(&casa, Path::new("/tmp/Escritorio/x")), None, "sólo en la casa del usuario");
        std::fs::create_dir_all(casa.join("Documentos")).unwrap();
        assert_eq!(carpeta_real_en(&casa, &casa.join("Documentos/a")), None, "una carpeta real con ese nombre se respeta");
    }

    /// (guardia, raíz canónica con `src/`): la guardia borra la carpeta al soltarse.
    fn tmp() -> (crate::CarpetaPrueba, PathBuf) {
        let d = crate::CarpetaPrueba::nueva("gotv-paths");
        std::fs::create_dir_all(d.join("src")).unwrap();
        let r = d.canonicalize().unwrap();
        (d, r)
    }

    #[test]
    fn relativa_va_a_la_raiz() {
        let (_raiz, r) = tmp();
        assert_eq!(resolve(&r, "src/main.rs"), r.join("src/main.rs"));
    }

    #[test]
    fn barra_inventada_es_relativa() {
        let (_raiz, r) = tmp();
        assert_eq!(resolve(&r, "/main.rs"), r.join("main.rs"));
        assert_eq!(resolve(&r, "/src/lib.rs"), r.join("src/lib.rs"));
    }

    #[test]
    fn absoluta_real_se_respeta() {
        let (_raiz, r) = tmp();
        assert_eq!(resolve(&r, "/etc/hosts"), PathBuf::from("/etc/hosts"));
        assert_eq!(
            resolve(&r, "/tmp/nuevo.txt"),
            PathBuf::from("/tmp/nuevo.txt")
        );
    }

    #[test]
    fn puntos_dobles_se_normalizan() {
        let (_raiz, r) = tmp();
        assert_eq!(resolve(&r, "src/../a.txt"), r.join("a.txt"));
        assert!(!is_inside(&r, &resolve(&r, "../../fuera.txt")));
    }

    #[test]
    fn dentro_y_fuera() {
        let (_raiz, r) = tmp();
        assert!(is_inside(&r, &resolve(&r, "src/x.rs")));
        assert!(is_inside(&r, &resolve(&r, "nuevo/dir/x.rs")));
        assert!(!is_inside(&r, &resolve(&r, "~/.zshrc")));
        assert!(!is_inside(&r, &resolve(&r, "/etc/hosts")));
    }

    #[test]
    fn symlink_que_sale_cuenta_como_fuera() {
        let (_raiz, r) = tmp();
        let link = r.join("salida");
        let _ = std::fs::remove_file(&link);
        std::os::unix::fs::symlink("/tmp", &link).unwrap();
        assert!(!is_inside(&r, &resolve(&r, "salida/x.txt")));
    }
}
