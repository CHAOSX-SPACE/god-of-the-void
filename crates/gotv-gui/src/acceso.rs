//! Carpetas PROTEGIDAS de macOS (Documentos, Escritorio, Descargas, iCloud Drive,
//! discos externos): tocarlas dispara el permiso del sistema. Regla del Portador
//! (2026-11-09): abrir la app NO pide nada; el permiso se pide al CREAR/ABRIR un
//! proyecto que vive ahí, y queda guardado (con firma estable sobrevive a reinicios).

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

fn home() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_default())
}

/// La carpeta protegida que contiene `p` (p. ej. `~/Documents`), si hay.
pub fn protected_root(p: &Path) -> Option<PathBuf> {
    let h = home();
    for sub in ["Documents", "Desktop", "Downloads", "Library/Mobile Documents"] {
        let r = h.join(sub);
        if p.starts_with(&r) {
            return Some(r);
        }
    }
    // discos externos y de red: /Volumes/<nombre>
    let mut it = p.components();
    if let (Some(std::path::Component::RootDir), Some(v), Some(name)) = (it.next(), it.next(), it.next()) {
        if v.as_os_str() == "Volumes" {
            return Some(Path::new("/Volumes").join(name.as_os_str()));
        }
    }
    None
}

/// Raíces protegidas a las que el usuario YA dio acceso: las de sus proyectos confiados.
pub fn granted_roots(trusted: &[String]) -> BTreeSet<PathBuf> {
    trusted.iter().filter_map(|t| protected_root(Path::new(t))).collect()
}

fn expand(d: &str) -> PathBuf {
    match d.strip_prefix("~/") {
        Some(rest) => home().join(rest),
        None if d == "~" => home(),
        None => PathBuf::from(d),
    }
}

/// Las carpetas de modelos que se pueden recorrer SIN pedir permiso: las no protegidas
/// y las protegidas cuya raíz ya fue concedida.
pub fn model_dirs_allowed(dirs: &[String], granted: &BTreeSet<PathBuf>) -> Vec<String> {
    dirs.iter()
        .filter(|d| protected_root(&expand(d)).is_none_or(|r| granted.contains(&r)))
        .cloned()
        .collect()
}

/// Pide (si hace falta) el acceso PERSISTENTE a la raíz protegida del proyecto. Elegir
/// la carpeta en el panel da acceso sólo a esa sesión; leer la raíz hace que macOS
/// pregunte AHORA —al crear el proyecto— y guarde la respuesta.
pub fn request_for_project(p: &Path) -> Result<(), String> {
    let Some(root) = protected_root(p) else { return Ok(()) };
    std::fs::read_dir(&root).map(|_| ()).map_err(|e| {
        gotv_core::tr!(
            "macOS no dio acceso a {}: {e}. Actívalo en Ajustes del Sistema → Privacidad y seguridad → Archivos y carpetas → GOD OF THE VOID.",
            "macOS denied access to {}: {e}. Turn it on in System Settings → Privacy & Security → Files and Folders → GOD OF THE VOID.",
            root.display()
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sabe_que_es_protegido_y_que_no() {
        let h = home();
        assert_eq!(protected_root(&h.join("Documents/proyectos/web")), Some(h.join("Documents")));
        assert_eq!(protected_root(&h.join("Desktop")), Some(h.join("Desktop")));
        assert_eq!(protected_root(Path::new("/Volumes/USB/x")), Some(PathBuf::from("/Volumes/USB")));
        assert_eq!(protected_root(&h.join(".lmstudio/models")), None);
        assert_eq!(protected_root(&h.join("DocumentsX/a")), None, "por componente, no por texto");
    }

    #[test]
    fn modelos_en_documentos_solo_con_permiso_concedido() {
        let dirs = vec!["~/Documents/proyectos/ia/checkpoints".to_string(), "~/modelos".to_string()];
        assert_eq!(model_dirs_allowed(&dirs, &BTreeSet::new()), vec!["~/modelos".to_string()]);
        let granted = granted_roots(&[home().join("Documents/proyectos/php").display().to_string()]);
        assert_eq!(model_dirs_allowed(&dirs, &granted).len(), 2);
    }
}
