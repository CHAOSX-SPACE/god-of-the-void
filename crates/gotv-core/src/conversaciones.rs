//! La caché de la CONVERSACIÓN en disco, por proyecto y con interruptor (pedido del Portador
//! 2026-11-10: «un check arriba de cada proyecto para activarla o desactivarla, así no toca almacenar
//! tanto peso en todo»).
//!
//! Medido antes: al reabrir la app, el proyecto Php (126 mensajes, ~130 000 caracteres) obligaba al 9B
//! a releer TODO el historial antes de la primera palabra: 140 s. Con esto el núcleo guarda la caché
//! de la conversación (K/V y estado recurrente) y al reabrir la LEE del disco.
//!
//! - el interruptor vive EN el proyecto: `.gotv/cache-conversacion` (un archivo marca);
//! - el peso vive FUERA: `~/.cache/gotv/conversaciones/<proyecto>/s<sesión>/` (no ensucia el repo);
//! - apagarlo borra lo guardado de ese proyecto;
//! - como mucho [`MAX_PROYECTOS`] proyectos la tienen encendida a la vez (pedido del Portador
//!   2026-11-11: «máximo 3 proyectos… así no llenamos el disco»): cada uno se REGISTRA con su ruta en
//!   su carpeta de la memoria (`<…>/<proyecto>/proyecto`) y se cuentan los que siguen encendidos.

use std::path::{Path, PathBuf};

/// Cuántos proyectos pueden tener la memoria rápida encendida a la vez.
pub const MAX_PROYECTOS: usize = 3;

fn marca(root: &Path) -> PathBuf {
    root.join(".gotv/cache-conversacion")
}

/// ¿Está activa en este proyecto?
pub fn activa(root: &Path) -> bool {
    marca(root).is_file()
}

/// Donde viven las memorias de todos los proyectos.
fn base() -> Option<PathBuf> {
    match std::env::var_os("GOTV_CONVERSACIONES_DIR") {
        Some(d) => Some(PathBuf::from(d)),
        None => Some(PathBuf::from(std::env::var_os("HOME")?).join(".cache/gotv/conversaciones")),
    }
}

fn canonica(root: &Path) -> PathBuf {
    root.canonicalize().unwrap_or_else(|_| root.to_path_buf())
}

/// La carpeta de cachés de este proyecto (exista o no).
pub fn carpeta(root: &Path) -> Option<PathBuf> {
    use std::hash::{Hash, Hasher};
    let canon = canonica(root);
    let mut h = std::collections::hash_map::DefaultHasher::new();
    canon.hash(&mut h);
    let nombre: String = canon.file_name().map(|n| n.to_string_lossy().chars().filter(|c| c.is_alphanumeric() || *c == '-').take(24).collect()).unwrap_or_default();
    Some(base()?.join(format!("{nombre}-{:016x}", h.finish())))
}

/// Anota en su carpeta de la memoria de qué proyecto es (para contar los encendidos).
fn registrar(root: &Path) {
    if let Some(c) = carpeta(root) {
        if std::fs::create_dir_all(&c).is_ok() {
            let _ = std::fs::write(c.join("proyecto"), canonica(root).to_string_lossy().as_bytes());
        }
    }
}

/// Los proyectos con la memoria rápida ENCENDIDA (registrados y con su marca todavía puesta).
pub fn proyectos_activos() -> Vec<PathBuf> {
    let Some(b) = base() else { return vec![] };
    let mut v: Vec<PathBuf> = std::fs::read_dir(b)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| std::fs::read_to_string(e.path().join("proyecto")).ok())
        .map(|p| PathBuf::from(p.trim()))
        .filter(|p| activa(p))
        .collect();
    v.sort();
    v.dedup();
    v
}

/// La carpeta de ESTA sesión, si la caché está activa en el proyecto.
pub fn para_sesion(root: &Path, sesion: Option<i64>) -> Option<PathBuf> {
    if !activa(root) {
        return None;
    }
    let c = carpeta(root)?;
    // un proyecto encendido antes del límite se registra la primera vez que se usa
    if !c.join("proyecto").is_file() {
        registrar(root);
    }
    Some(c.join(format!("s{}", sesion?)))
}

/// Enciende o apaga la caché del proyecto; al apagarla se borra lo guardado (es nuestra: se
/// regenera sola si se vuelve a encender). Encenderla en un proyecto más allá de
/// [`MAX_PROYECTOS`] es un error que nombra los que ya la tienen.
pub fn activar(root: &Path, on: bool) -> std::io::Result<()> {
    let m = marca(root);
    if on {
        if !activa(root) {
            let yo = canonica(root);
            let otros: Vec<PathBuf> = proyectos_activos().into_iter().filter(|p| *p != yo).collect();
            if otros.len() >= MAX_PROYECTOS {
                let nombres: Vec<String> = otros.iter().map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| p.display().to_string())).collect();
                return Err(std::io::Error::other(crate::tr!(
                    "ya hay {MAX_PROYECTOS} proyectos con «Reabrir rápido» ({}): apágalo en uno de ellos para encenderlo aquí",
                    "{MAX_PROYECTOS} projects already have «Fast reopen» ({}): turn it off in one of them to turn it on here",
                    nombres.join(", ")
                )));
            }
        }
        std::fs::create_dir_all(m.parent().unwrap_or(root))?;
        std::fs::write(&m, crate::tr!("la caché de la conversación de GOTV está activa en este proyecto\n", "the GOTV conversation cache is active in this project\n"))?;
        registrar(root);
        Ok(())
    } else {
        let _ = std::fs::remove_file(&m);
        if let Some(d) = carpeta(root) {
            let _ = std::fs::remove_dir_all(d);
        }
        Ok(())
    }
}

/// El tope de la memoria rápida de este proyecto (bytes; por omisión 3 GiB). Vive en la carpeta de la
/// memoria (`<…>/conversaciones/<proyecto>/tope`) para que el núcleo lo lea sin saber del proyecto.
pub fn tope(root: &Path) -> u64 {
    carpeta(root).map(|c| gotv_engine::memoria_tope::tope(&c)).unwrap_or(gotv_engine::memoria_tope::TOPE_POR_OMISION)
}

/// Fija el tope (GiB) y, si baja, desaloja YA lo usado hace más tiempo hasta caber con un 10 % libre.
pub fn fijar_tope(root: &Path, gb: u64) -> std::io::Result<()> {
    let Some(c) = carpeta(root) else { return Ok(()) };
    let bytes = gb.max(1) << 30;
    gotv_engine::memoria_tope::fijar_tope(&c, bytes)?;
    // sin entrada protegida: la conversación activa es la usada más recientemente y sale la última
    gotv_engine::memoria_tope::hacer_sitio(&c, &c.join("-"), 0, 0, bytes);
    Ok(())
}

/// Bytes que ocupa en disco la memoria de este proyecto (sólo las conversaciones guardadas: no los
/// archivos de control `tope` y `proyecto`).
pub fn bytes(root: &Path) -> u64 {
    carpeta(root).map(|d| gotv_engine::memoria_tope::ocupado(&d)).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn el_interruptor_marca_el_proyecto_y_apagar_borra_el_peso() {
        let raiz = crate::CarpetaPrueba::sin_crear("gotv-conv");
        let base = raiz.join("cache");
        let proyecto = raiz.join("mi-proyecto");
        std::fs::create_dir_all(&proyecto).unwrap();
        // SAFETY: única prueba que usa esta variable
        unsafe { std::env::set_var("GOTV_CONVERSACIONES_DIR", &base) };
        assert!(!activa(&proyecto) && para_sesion(&proyecto, Some(3)).is_none(), "apagada por omisión");
        activar(&proyecto, true).unwrap();
        let d = para_sesion(&proyecto, Some(3)).unwrap();
        assert!(d.starts_with(&base) && d.ends_with("s3"), "{d:?}");
        assert!(d.to_string_lossy().contains("mi-proyecto-"), "{d:?}");
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("x.safetensors"), vec![0u8; 1000]).unwrap();
        assert_eq!(bytes(&proyecto), 1000);
        // el tope: 3 GiB por omisión; al bajarlo se desaloja lo que no cabe con el 10 % libre
        assert_eq!(tope(&proyecto), 3 << 30);
        std::fs::write(d.join("indice.gotv"), b"{}").unwrap();
        fijar_tope(&proyecto, 6).unwrap();
        assert_eq!(tope(&proyecto), 6 << 30);
        assert!(d.join("x.safetensors").exists(), "con 6 GiB cabe");
        activar(&proyecto, false).unwrap();
        assert!(!activa(&proyecto) && !d.exists() && bytes(&proyecto) == 0, "apagar borra el peso");
        // como mucho MAX_PROYECTOS encendidos a la vez (en la MISMA prueba: la variable es global)
        let p: Vec<PathBuf> = (0..=MAX_PROYECTOS).map(|i| raiz.join(format!("p{i}"))).collect();
        for x in &p {
            std::fs::create_dir_all(x).unwrap();
        }
        for x in &p[..MAX_PROYECTOS] {
            activar(x, true).unwrap();
        }
        assert_eq!(proyectos_activos().len(), MAX_PROYECTOS);
        let e = activar(&p[MAX_PROYECTOS], true).unwrap_err().to_string();
        assert!(!activa(&p[MAX_PROYECTOS]) && e.contains("p0") && e.contains("p2"), "{e}");
        activar(&p[0], true).unwrap(); // volver a encender uno ya encendido no cuenta doble
        activar(&p[1], false).unwrap();
        activar(&p[MAX_PROYECTOS], true).unwrap();
        assert_eq!(proyectos_activos().len(), MAX_PROYECTOS);
        // uno encendido ANTES del límite (sin registro) se registra al usarse
        let viejo = raiz.join("viejo");
        std::fs::create_dir_all(viejo.join(".gotv")).unwrap();
        std::fs::write(marca(&viejo), "x").unwrap();
        assert!(!proyectos_activos().iter().any(|x| x.ends_with("viejo")));
        para_sesion(&viejo, Some(1));
        assert!(proyectos_activos().iter().any(|x| x.ends_with("viejo")));
        let _ = std::fs::remove_dir_all(&raiz);
    }
}
