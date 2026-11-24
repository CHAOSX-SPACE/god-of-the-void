//! F7 paso 5 · `recorrido.md` al cerrar un turno que tocó archivos.
//!
//! Lo cuenta el agente (su respuesta final: qué hizo y cómo lo probó), pero la LISTA de archivos no la
//! redacta el modelo: la pone GOTV desde los puntos de restauración del turno (`checkpoint.rs`), que
//! ven también lo que cambió la terminal. Así el recorrido no puede mentir. Las capturas y los videos
//! del navegador que se tomaron en el turno van con su ruta (nunca incrustados).

use super::{Chat, Doc};
use crate::checkpoint::{Checkpoint, Step};
use anyhow::Result;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

/// Cómo quedó un archivo al final del turno respecto de su comienzo.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cambio {
    Nuevo,
    Cambiado,
    Borrado,
}

/// El cambio NETO de cada archivo en una serie de pasos (primer «antes» contra último «después»). Lo
/// que se creó y se borró dentro del mismo turno no cuenta: al final no cambió nada.
pub fn cambios_netos(pasos: &[Step]) -> Vec<(String, Cambio)> {
    versiones_netas(pasos)
        .into_iter()
        .filter_map(|(p, antes, despues)| match (antes, despues) {
            (None, Some(_)) => Some((p, Cambio::Nuevo)),
            (Some(_), None) => Some((p, Cambio::Borrado)),
            (Some(a), Some(d)) if a != d => Some((p, Cambio::Cambiado)),
            _ => None,
        })
        .collect()
}

/// Las versiones netas de cada archivo en una serie de pasos: (ruta, primer «antes», último «después»), por
/// hash de los puntos de restauración. La base de [`cambios_netos`] y del panel de cambios (F8.2).
pub fn versiones_netas(pasos: &[Step]) -> Vec<(String, Option<String>, Option<String>)> {
    let mut neto: BTreeMap<String, (Option<String>, Option<String>)> = BTreeMap::new();
    for paso in pasos {
        for c in &paso.changes {
            neto.entry(c.path.clone()).and_modify(|e| e.1 = c.after.clone()).or_insert((c.before.clone(), c.after.clone()));
        }
    }
    neto.into_iter().map(|(p, (a, d))| (p, a, d)).collect()
}

fn medios() -> &'static Mutex<HashMap<String, Vec<PathBuf>>> {
    static M: OnceLock<Mutex<HashMap<String, Vec<PathBuf>>>> = OnceLock::new();
    M.get_or_init(Default::default)
}

/// Una captura o un video que se tomó en el turno de este chat (`dueno` = dueño de procesos).
pub fn anotar_medio(dueno: &str, ruta: PathBuf) {
    let mut m = medios().lock().unwrap_or_else(|e| e.into_inner());
    let v = m.entry(dueno.to_string()).or_default();
    if !v.contains(&ruta) {
        v.push(ruta);
    }
}

/// Lo anotado en el turno (y se vacía).
pub fn tomar_medios(dueno: &str) -> Vec<PathBuf> {
    medios().lock().unwrap_or_else(|e| e.into_inner()).remove(dueno).unwrap_or_default()
}

/// El texto del recorrido.
pub fn armar(root: &Path, resumen: &str, cambios: &[(String, Cambio)], medios: &[PathBuf], fecha: &str) -> String {
    let mut t = crate::tr!("# Recorrido · {fecha}\n\n", "# Walkthrough · {fecha}\n\n");
    t.push_str(crate::i18n::t(
        "## Qué se hizo y cómo se probó\n_(lo cuenta el agente)_\n\n",
        "## What was done and how it was tested\n_(told by the agent)_\n\n",
    ));
    let resumen = resumen.trim();
    t.push_str(if resumen.is_empty() { crate::i18n::t("(el agente no dejó resumen)", "(the agent left no summary)") } else { resumen });
    t.push_str(crate::i18n::t(
        "\n\n## Archivos que cambiaron\n_(de los puntos de restauración de GOTV, también lo que cambió la terminal; no lo escribe el modelo)_\n\n",
        "\n\n## Files that changed\n_(from GOTV's restore points, including what the terminal changed; not written by the model)_\n\n",
    ));
    for (ruta, c) in cambios {
        t.push_str(&match c {
            Cambio::Nuevo => crate::tr!("- `+` `{ruta}` (nuevo)\n", "- `+` `{ruta}` (new)\n"),
            Cambio::Cambiado => format!("- `~` `{ruta}`\n"),
            Cambio::Borrado => crate::tr!("- `−` `{ruta}` (borrado)\n", "- `−` `{ruta}` (deleted)\n"),
        });
    }
    let medios: Vec<&PathBuf> = medios.iter().filter(|p| p.is_file()).collect();
    if !medios.is_empty() {
        t.push_str(crate::i18n::t("\n## Capturas y videos\n\n", "\n## Screenshots and videos\n\n"));
        for p in medios {
            let r = p.strip_prefix(root).unwrap_or(p).display().to_string();
            let video = matches!(p.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref(), Some("webm" | "mp4" | "mov"));
            t.push_str(&if video { crate::tr!("- video: `{r}`\n", "- video: `{r}`\n") } else { crate::tr!("- captura: `{r}`\n", "- screenshot: `{r}`\n") });
        }
    }
    t
}

/// Cierra el turno: si cambió algún archivo desde el paso `desde`, escribe y versiona `recorrido.md`.
/// Devuelve la versión escrita (`None` = el turno no tocó archivos).
pub fn cerrar_turno(chat: &Chat, cp: &Checkpoint, desde: Option<usize>, root: &Path, resumen: &str, medios: &[PathBuf]) -> Result<Option<usize>> {
    let cambios = cambios_netos(&cp.pasos_desde(desde));
    if cambios.is_empty() {
        return Ok(None);
    }
    let fecha = chrono::Local::now().format("%Y-%m-%d %H:%M").to_string();
    chat.escribir(Doc::Recorrido, &armar(root, resumen, &cambios, medios, &fecha)).map(Some)
}
