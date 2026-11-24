//! Carpetas temporales de las PRUEBAS de todo el workspace.
//!
//! Cada prueba que necesita disco pide una `CarpetaPrueba`: vive en el temporal del
//! usuario (`$TMPDIR`), lleva el pid en el nombre y se BORRA SOLA al salir de alcance
//! (Drop), pase la prueba o falle una aserción. Antes, cada corrida de
//! `cargo test --workspace` dejaba ~150 carpetas `gotv-*` en el temporal de macOS.
//!
//! Regla (falla #2571, la poda que borró carpetas del temporal de macOS): esto borra
//! SÓLO su ruta exacta, la que construyó él mismo — jamás por patrón, jamás el padre,
//! jamás algo fuera de `temp_dir()`. El nombre se valida: empieza por `gotv-`, sin `/`
//! ni `..`, así que la ruta es siempre un hijo directo del temporal.
//!
//! Vive aquí (y no bajo `#[cfg(test)]`) porque gotv-core, gotv-mlx y gotv-gui la usan
//! en sus pruebas y todos dependen de gotv-engine; sólo la tocan las pruebas.

use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

/// Cada guardia del proceso lleva su número: dos pruebas PARALELAS que piden la misma
/// base (`agent("plan")` en dos pruebas) no comparten carpeta, así que el Drop de una
/// jamás borra la carpeta viva de la otra.
static SIGUIENTE: AtomicUsize = AtomicUsize::new(0);

/// `<base>-<pid>-<n>`: único en el proceso y entre procesos.
fn unico(base: &str) -> String {
    format!("{base}-{}-{}", std::process::id(), SIGUIENTE.fetch_add(1, Ordering::Relaxed))
}

/// Una ruta del temporal que es de UNA prueba y se borra al soltarla.
#[derive(Debug)]
pub struct CarpetaPrueba {
    ruta: PathBuf,
}

impl CarpetaPrueba {
    /// `temp_dir()/<base>-<pid>-<n>` creada y VACÍA.
    pub fn nueva(base: &str) -> Self {
        let c = Self::sin_crear(base);
        std::fs::create_dir_all(&c.ruta).expect("no pude crear la carpeta de la prueba");
        c
    }

    /// `temp_dir()/<base>-<pid>-<n>` sin crearla: para la prueba que la crea ella misma
    /// (o que necesita que NO exista todavía).
    pub fn sin_crear(base: &str) -> Self {
        Self::con_nombre(&unico(base))
    }

    /// Un ARCHIVO suelto del temporal: `temp_dir()/<base>-<pid>-<n>.<ext>`, sin crearlo.
    pub fn archivo(base: &str, ext: &str) -> Self {
        Self::con_nombre(&format!("{}.{ext}", unico(base)))
    }

    fn con_nombre(nombre: &str) -> Self {
        assert!(
            nombre.starts_with("gotv-") && !nombre.contains('/') && !nombre.contains('\\') && !nombre.contains(".."),
            "nombre de carpeta de prueba inválido: {nombre:?} (debe empezar por gotv- y no llevar / ni ..)"
        );
        let c = Self { ruta: std::env::temp_dir().join(nombre) };
        c.borrar();
        c
    }

    /// La ruta (también por `Deref`/`AsRef`).
    pub fn ruta(&self) -> &Path {
        &self.ruta
    }

    /// Borra la ruta exacta: carpeta con su contenido, o archivo/enlace suelto.
    /// Un enlace simbólico se borra como enlace, jamás se sigue.
    fn borrar(&self) {
        match std::fs::symlink_metadata(&self.ruta) {
            Ok(m) if m.is_dir() => {
                let _ = std::fs::remove_dir_all(&self.ruta);
            }
            Ok(_) => {
                let _ = std::fs::remove_file(&self.ruta);
            }
            Err(_) => {}
        }
    }
}

impl Drop for CarpetaPrueba {
    fn drop(&mut self) {
        self.borrar();
    }
}

impl Deref for CarpetaPrueba {
    type Target = Path;
    fn deref(&self) -> &Path {
        &self.ruta
    }
}

impl AsRef<Path> for CarpetaPrueba {
    fn as_ref(&self) -> &Path {
        &self.ruta
    }
}

/// Para pasarla tal cual como argumento de un `Command` (`git -C <carpeta>`).
impl AsRef<std::ffi::OsStr> for CarpetaPrueba {
    fn as_ref(&self) -> &std::ffi::OsStr {
        self.ruta.as_os_str()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn la_carpeta_se_borra_al_soltarla_aunque_la_prueba_falle() {
        let ruta;
        {
            let c = CarpetaPrueba::nueva("gotv-prueba-guardia");
            ruta = c.to_path_buf();
            std::fs::create_dir_all(c.join("a/b")).unwrap();
            std::fs::write(c.join("a/b/x.txt"), "x").unwrap();
            assert!(ruta.is_dir());
        }
        assert!(!ruta.exists(), "la carpeta sobrevivió al Drop");

        // una aserción que falla dentro de la prueba: el Drop corre al desenrollar
        let quedo = std::sync::Mutex::new(PathBuf::new());
        let r = std::panic::catch_unwind(|| {
            let c = CarpetaPrueba::nueva("gotv-prueba-guardia-panico");
            *quedo.lock().unwrap() = c.to_path_buf();
            std::fs::write(c.join("y.txt"), "y").unwrap();
            assert_eq!(1, 2, "falla a propósito");
        });
        assert!(r.is_err());
        let quedo = quedo.into_inner().unwrap_or_else(|e| e.into_inner());
        assert!(quedo.starts_with(std::env::temp_dir()) && quedo != std::env::temp_dir());
        assert!(!quedo.exists(), "el Drop no corrió al desenrollar");
    }

    #[test]
    fn la_misma_base_en_dos_pruebas_da_dos_carpetas() {
        let (a, b) = (CarpetaPrueba::nueva("gotv-prueba-guardia-par"), CarpetaPrueba::nueva("gotv-prueba-guardia-par"));
        assert_ne!(a.ruta(), b.ruta());
        let ruta_a = a.to_path_buf();
        drop(a);
        assert!(!ruta_a.exists() && b.is_dir(), "soltar una no toca la otra");
    }

    #[test]
    fn el_archivo_suelto_tambien_se_borra() {
        let ruta;
        {
            let f = CarpetaPrueba::archivo("gotv-prueba-guardia-archivo", "txt");
            std::fs::write(&f, "x").unwrap();
            ruta = f.to_path_buf();
            assert!(ruta.is_file());
        }
        assert!(!ruta.exists());
    }

    #[test]
    fn solo_toca_hijos_directos_del_temporal_con_nombre_gotv() {
        for malo in ["", "..", "../x", "gotv-a/b", "otra-cosa", "gotv-..", "/tmp"] {
            let r = std::panic::catch_unwind(|| CarpetaPrueba::con_nombre(malo));
            assert!(r.is_err(), "aceptó {malo:?}");
        }
        // el temporal mismo jamás es su ruta
        let c = CarpetaPrueba::sin_crear("gotv-prueba-guardia-hijo");
        assert_eq!(c.parent(), Some(std::env::temp_dir().as_path()));
    }

    #[cfg(unix)]
    #[test]
    fn un_enlace_se_borra_como_enlace_sin_seguirlo() {
        let destino = CarpetaPrueba::nueva("gotv-prueba-guardia-destino");
        std::fs::write(destino.join("vivo.txt"), "no me borres").unwrap();
        {
            let enlace = CarpetaPrueba::sin_crear("gotv-prueba-guardia-enlace");
            std::os::unix::fs::symlink(destino.ruta(), enlace.ruta()).unwrap();
        }
        assert!(destino.join("vivo.txt").is_file(), "siguió el enlace y borró el destino");
    }
}
