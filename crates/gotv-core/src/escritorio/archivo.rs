//! F16.7 · Pestaña «Archivo» del panel de trabajo: ver y editar un archivo del proyecto desde la ventana.
//!
//! - Sólo dentro del proyecto (ni `..`, ni rutas absolutas de fuera, ni un enlace que apunte fuera); nunca
//!   dentro de `.git/`.
//! - Cada lectura lleva su **versión** (sha256 del contenido). Guardar exige la versión que se leyó: si el
//!   agente (o cualquiera) cambió el archivo mientras el Portador lo editaba, NO se pisa: se dice el conflicto.
//! - Se escribe de forma atómica (temporal + renombrar) conservando los permisos del archivo.
//! - Texto de hasta 2 MB; un binario no se abre como texto.
use anyhow::{anyhow, bail, Result};
use sha2::{Digest, Sha256};
use std::path::{Component, Path, PathBuf};

pub const TOPE: u64 = 2 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq)]
pub struct Abierto {
    /// relativa a la raíz
    pub ruta: String,
    pub texto: String,
    pub version: String,
    /// por la extensión (para que la ventana elija el resaltado)
    pub lenguaje: String,
}

impl Abierto {
    pub fn json(&self) -> serde_json::Value {
        serde_json::json!({"ruta": self.ruta, "texto": self.texto, "version": self.version, "lenguaje": self.lenguaje})
    }
}

pub fn version_de(b: &[u8]) -> String {
    let h = Sha256::digest(b);
    h.iter().take(16).map(|x| format!("{x:02x}")).collect()
}

/// La ruta absoluta de `ruta` si queda DENTRO del proyecto (exista o no el archivo).
pub fn dentro(root: &Path, ruta: &str) -> Result<PathBuf> {
    let fuera = || anyhow!("{}", crate::tr!("«{ruta}» queda fuera del proyecto", "«{ruta}» is outside the project"));
    let p = Path::new(ruta.trim());
    let raiz = root.canonicalize()?;
    let rel = if p.is_absolute() { p.strip_prefix(&raiz).or_else(|_| p.strip_prefix(root)).map_err(|_| fuera())?.to_path_buf() } else { p.to_path_buf() };
    if rel.as_os_str().is_empty() || rel.components().any(|c| !matches!(c, Component::Normal(_))) {
        return Err(fuera());
    }
    if rel.components().next().is_some_and(|c| c.as_os_str() == ".git") {
        bail!("{}", crate::i18n::t("los archivos internos de git no se editan aquí", "git's internal files are not edited here"));
    }
    let abs = raiz.join(&rel);
    // lo que exista del camino, resuelto, debe seguir dentro (un enlace a fuera no vale)
    let mut existente = abs.clone();
    while !existente.exists() {
        existente = existente.parent().map(Path::to_path_buf).ok_or_else(fuera)?;
    }
    if !existente.canonicalize()?.starts_with(&raiz) {
        return Err(fuera());
    }
    Ok(abs)
}

fn lenguaje(ruta: &Path) -> String {
    ruta.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default()
}

pub fn leer(root: &Path, ruta: &str) -> Result<Abierto> {
    let abs = dentro(root, ruta)?;
    let meta = std::fs::metadata(&abs).map_err(|e| anyhow!("{ruta}: {e}"))?;
    if !meta.is_file() {
        bail!("{}", crate::tr!("«{ruta}» no es un archivo", "«{ruta}» is not a file"));
    }
    if meta.len() > TOPE {
        bail!("{}", crate::tr!("«{ruta}» pesa {} KB: el editor abre hasta {} KB", "«{ruta}» weighs {} KB: the editor opens up to {} KB", meta.len() / 1024, TOPE / 1024));
    }
    let b = std::fs::read(&abs)?;
    if b.iter().take(8192).any(|&x| x == 0) {
        bail!("{}", crate::tr!("«{ruta}» es binario: no se abre como texto", "«{ruta}» is binary: it does not open as text"));
    }
    let texto = String::from_utf8(b.clone()).map_err(|_| anyhow!("{}", crate::tr!("«{ruta}» no es UTF-8", "«{ruta}» is not UTF-8")))?;
    let raiz = root.canonicalize()?;
    Ok(Abierto { ruta: abs.strip_prefix(&raiz).unwrap_or(&abs).display().to_string(), texto, version: version_de(&b), lenguaje: lenguaje(&abs) })
}

/// Guarda `texto` si el archivo sigue en `version` (la que se leyó). `None` = archivo nuevo (no debe existir).
/// Devuelve la versión nueva.
pub fn guardar(root: &Path, ruta: &str, texto: &str, version: Option<&str>) -> Result<String> {
    let abs = dentro(root, ruta)?;
    if texto.len() as u64 > TOPE {
        bail!("{}", crate::i18n::t("el texto supera el tope del editor", "the text exceeds the editor's limit"));
    }
    let actual = std::fs::read(&abs).ok();
    match (&actual, version) {
        (Some(b), Some(v)) if version_de(b) != v => bail!(
            "{}",
            crate::tr!(
                "CONFLICTO: «{ruta}» cambió desde que lo abriste (quizá lo editó el agente). No se pisó: vuelve a abrirlo y aplica tu cambio encima",
                "CONFLICT: «{ruta}» changed since you opened it (maybe the agent edited it). It was not overwritten: reopen it and apply your change on top"
            )
        ),
        (Some(_), None) => bail!("{}", crate::tr!("«{ruta}» ya existe: ábrelo antes de guardarlo", "«{ruta}» already exists: open it before saving it")),
        (None, Some(_)) => bail!("{}", crate::tr!("«{ruta}» ya no existe (¿lo borró el agente?)", "«{ruta}» no longer exists (did the agent delete it?)")),
        _ => {}
    }
    let dir = abs.parent().ok_or_else(|| anyhow!("?"))?;
    std::fs::create_dir_all(dir)?;
    let permisos = std::fs::metadata(&abs).ok().map(|m| m.permissions());
    let tmp = dir.join(format!(".{}.gotv-{}", abs.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default(), std::process::id()));
    std::fs::write(&tmp, texto.as_bytes())?;
    if let Some(p) = permisos {
        let _ = std::fs::set_permissions(&tmp, p);
    }
    std::fs::rename(&tmp, &abs).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })?;
    Ok(version_de(texto.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn abrir_editar_y_conflicto() {
        let d = crate::CarpetaPrueba::nueva("gotv-archivo-editar");
        std::fs::create_dir_all(d.join("src")).unwrap();
        std::fs::write(d.join("src/a.rs"), "fn a() {}\n").unwrap();
        let a = leer(&d, "src/a.rs").unwrap();
        assert_eq!((a.ruta.as_str(), a.lenguaje.as_str()), ("src/a.rs", "rs"));
        let v2 = guardar(&d, "src/a.rs", "fn a() { 1; }\n", Some(&a.version)).unwrap();
        assert_eq!(std::fs::read_to_string(d.join("src/a.rs")).unwrap(), "fn a() { 1; }\n");
        // el agente lo cambia mientras el Portador edita: no se pisa
        std::fs::write(d.join("src/a.rs"), "fn a() { 2; }\n").unwrap();
        let e = guardar(&d, "src/a.rs", "fn a() { 3; }\n", Some(&v2)).unwrap_err().to_string();
        assert!(e.contains("CONFLICT"), "{e}");
        assert_eq!(std::fs::read_to_string(d.join("src/a.rs")).unwrap(), "fn a() { 2; }\n");
        // nuevo
        assert!(guardar(&d, "src/a.rs", "x", None).is_err(), "ya existe");
        guardar(&d, "nuevo/b.txt", "hola", None).unwrap();
        assert_eq!(std::fs::read_to_string(d.join("nuevo/b.txt")).unwrap(), "hola");
        assert!(std::fs::read_dir(d.join("nuevo")).unwrap().count() == 1, "sin temporales sueltos");
    }

    #[test]
    fn no_sale_del_proyecto() {
        let d = crate::CarpetaPrueba::nueva("gotv-archivo-fuera");
        let f = crate::CarpetaPrueba::nueva("gotv-archivo-fuera-otro");
        std::fs::write(f.join("secreto.txt"), "no").unwrap();
        std::os::unix::fs::symlink(&*f, d.join("enlace")).unwrap();
        for r in ["../x", "/etc/passwd", "enlace/secreto.txt", ".git/config", ""] {
            assert!(leer(&d, r).is_err(), "{r}");
            assert!(guardar(&d, r, "x", None).is_err(), "{r}");
        }
        std::fs::write(d.join("bin"), [0u8, 1, 2]).unwrap();
        assert!(leer(&d, "bin").unwrap_err().to_string().contains("binari"));
    }
}
