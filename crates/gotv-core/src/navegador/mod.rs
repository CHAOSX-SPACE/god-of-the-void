//! F23.4 · El navegador hablado DIRECTO desde el motor, por el protocolo de depuración de Chrome
//! (CDP), dentro del mismo proceso: `cdp.rs`. Sin capturas: el árbol de accesibilidad como texto con
//! las mismas referencias `@e1`, `@e2`… que el `browser` de hoy.
//!
//! Siempre un Chrome/Chromium AISLADO: perfil temporal propio (`--user-data-dir`), que se borra por su
//! ruta exacta al cerrar; jamás el perfil, las cookies ni las sesiones del usuario.
//!
//! Medido el 2026-11-16 (L10, primer paso de F23.4), agent-browser con chrome-headless-shell en esta
//! Mac: abrir en frío 2 313 ms; `snapshot -i` 16–21 ms; `get text` 16 ms; `click` 15–27 ms; `fill`
//! 20 ms. agent-browser ya mantiene un daemon: el «proceso por acción» del plan cuesta ~15 ms, no más.

pub mod cdp;

use std::path::Path;

/// Los sitios del proyecto (`.gotv/navegador`, F7) vistos desde una lectura SIN nadie a quien preguntar
/// (`investigar` y el navegador CDP): sólo pesa «abrir negar». Un solo lector para los dos hilos
/// (`artefactos::sitios`): antes había dos, y éste ignoraba un archivo roto (abría TODO); ahora una
/// lista rota no deja abrir nada, como en el `browser`. Las descargas, en el navegador CDP, están
/// siempre negadas.
#[derive(Debug, Clone, Default)]
pub struct Listas {
    pub abrir: Abrir,
}

#[derive(Debug, Clone, Default)]
pub struct Abrir {
    sitios: crate::artefactos::sitios::Sitios,
    /// Por qué la lista no vale (archivo roto): entonces no se abre nada.
    rota: Option<String>,
}

impl Abrir {
    pub fn deja(&self, url: &str) -> bool {
        if self.rota.is_some() {
            return false;
        }
        if url == "about:blank" || url.starts_with("data:") || url.starts_with("file:") {
            return true;
        }
        crate::artefactos::sitios::dominio(url).is_some() && !self.sitios.abrir_negado(url)
    }

    /// Por qué no se dejó (para el mensaje).
    pub fn porque(&self, url: &str) -> String {
        match &self.rota {
            Some(e) => crate::tr!("«{url}»: la lista de sitios (.gotv/navegador) está rota y no se abre nada hasta arreglarla: {e}", "«{url}»: the site list (.gotv/navegador) is broken and nothing opens until it is fixed: {e}"),
            None => crate::tr!("«{url}» está prohibido en .gotv/navegador", "«{url}» is forbidden in .gotv/navegador"),
        }
    }
}

impl Listas {
    pub fn del_proyecto(root: &Path) -> Listas {
        match crate::artefactos::sitios::Sitios::leer(root) {
            Ok(sitios) => Listas { abrir: Abrir { sitios, rota: None } },
            Err(e) => Listas { abrir: Abrir { sitios: Default::default(), rota: Some(format!("{e:#}")) } },
        }
    }

    pub fn de_texto(t: &str) -> Listas {
        match crate::artefactos::sitios::Sitios::desde_texto(t) {
            Ok(sitios) => Listas { abrir: Abrir { sitios, rota: None } },
            Err(e) => Listas { abrir: Abrir { sitios: Default::default(), rota: Some(format!("{e:#}")) } },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// «negar» manda (dominio y subdominios); «permitir» NO vuelve la lista exclusiva (sólo quita preguntas
    /// en el `browser`); una lista rota no deja abrir nada; sin lista, todo.
    #[test]
    fn las_listas_de_f7() {
        let l = Listas::de_texto("abrir negar malo.io");
        assert!(l.abrir.deja("https://docs.rs/x"));
        assert!(!l.abrir.deja("https://malo.io/") && !l.abrir.deja("https://sub.malo.io/"));
        assert!(l.abrir.deja("https://nomalo.io/"));
        let l = Listas::de_texto("abrir permitir docs.rs\nabrir negar x.docs.rs");
        assert!(l.abrir.deja("https://a.docs.rs/") && !l.abrir.deja("https://x.docs.rs/"));
        assert!(l.abrir.deja("https://otro.io/"), "permitir no excluye a los demás");
        let rota = Listas::de_texto("abrir prohibir malo.io");
        assert!(!rota.abrir.deja("https://docs.rs/") && rota.abrir.porque("https://docs.rs/").contains("rota"));
        assert!(Listas::default().abrir.deja("about:blank") && Listas::default().abrir.deja("https://cualquiera.io/"));
    }
}
