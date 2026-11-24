//! F7 · Artefactos verificables: lo que el Portador revisa y comenta, en archivos de la conversación.
//!
//! Carpeta por conversación: `.gotv/chats/<sesión>/` con `plan.md`, `tareas.md` y `recorrido.md`. Cada
//! escritura deja su versión en `historial/<n>-<doc>.md` (tope [`MAX_VERSIONES`] por documento; lo viejo
//! se borra por su RUTA EXACTA, ley L6). Los escribe GOTV, nunca el modelo con sus herramientas
//! ([`protegido`]): así el recorrido no puede mentir y nadie falsifica un comentario del Portador.
//!
//! - [`comentarios`]: comentarios por línea (`comentarios.gotv`), reubicados por el texto de la línea.
//! - [`recorrido`]: qué se cambió (de los puntos de restauración, no del modelo), capturas y video.
//! - [`grabacion`]: `browser record start|stop` → tarjeta de video servida como archivo local.
//! - [`sitios`]: `.gotv/navegador` (texto, una regla por línea), sitios permitidos y prohibidos para abrir y para descargar.

pub mod comentarios;
pub mod grabacion;
pub mod recorrido;
pub mod sitios;

#[cfg(test)]
pub(crate) mod pruebas;

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

/// Versiones que se guardan de cada documento de una conversación (trampa 3: la carpeta crece).
pub const MAX_VERSIONES: usize = 30;
/// Tope de un documento: lo que pase se recorta con aviso (un plan de 256 KiB ya no es un plan).
pub const MAX_BYTES_DOC: usize = 256 * 1024;

/// Los tres documentos de una conversación.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Doc {
    Plan,
    Tareas,
    Recorrido,
}

impl Doc {
    pub const TODOS: [Doc; 3] = [Doc::Plan, Doc::Tareas, Doc::Recorrido];

    pub fn archivo(self) -> &'static str {
        // juez-idioma: datos (nombres de archivo fijos del formato)
        match self {
            Doc::Plan => "plan.md",
            Doc::Tareas => "tareas.md",
            Doc::Recorrido => "recorrido.md",
        }
    }

    pub fn de_archivo(nombre: &str) -> Option<Doc> {
        Doc::TODOS.into_iter().find(|d| d.archivo() == nombre)
    }
}

/// Lo que hay de un documento: cuántas versiones y cuál es la última.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct Resumen {
    pub archivo: &'static str,
    pub versiones: Vec<usize>,
    pub ruta: PathBuf,
}

/// La clave de la carpeta de una conversación: su id de sesión; sin sesión (pruebas, `gotv -p` sin
/// persistir), la del dueño de procesos del chat. Sólo letras, cifras, `-` y `_` (nunca `..` ni `/`).
pub fn clave_chat(sesion: Option<i64>, dueno: &str) -> String {
    match sesion {
        Some(id) => id.to_string(),
        // juez-idioma: datos (nombre de carpeta, no texto para una persona)
        None => format!("sin-sesion-{}", dueno.rsplit('#').next().unwrap_or(dueno)),
    }
}

fn limpia(clave: &str) -> String {
    let s: String = clave
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '-' })
        .take(64)
        .collect();
    if s.trim_matches('-').is_empty() {
        // juez-idioma: datos (nombre de carpeta, no texto para una persona)
        "sin-sesion".into()
    } else {
        s
    }
}

/// La carpeta de la conversación de un agente (su sesión, o su dueño de procesos si no persiste).
pub fn chat_de(root: &Path, sesion: Option<i64>, dueno: &str) -> Chat {
    Chat::new(root, &clave_chat(sesion, dueno))
}

/// La tarjeta del plan para la ventana: el texto por líneas, sus comentarios y la pregunta con la que
/// se pide aprobarlo (la ventana la reconoce y contesta desde la tarjeta: «Aprobar» o «Pedir cambios»).
pub fn vista_plan(chat: &Chat, version: usize, pregunta: &str) -> serde_json::Value {
    serde_json::json!({
        "tipo": "plan",
        "titulo": crate::tr!("Plan · versión {version}", "Plan · version {version}"),
        "sesion": chat.clave(),
        // juez-idioma: datos (claves de la tarjeta que lee la ventana)
        "archivo": Doc::Plan.archivo(),
        "version": version,
        "ruta": chat.ruta(Doc::Plan).to_string_lossy(),
        "texto": chat.leer(Doc::Plan).unwrap_or_default(),
        "comentarios": comentarios::todos(chat).into_iter().filter(|c| c.archivo == Doc::Plan.archivo()).collect::<Vec<_>>(),
        "pregunta": pregunta,
    })
}

/// F7 paso 5, al cerrar un turno: si tocó archivos, su recorrido (con las capturas y videos anotados
/// en el turno). Devuelve la ruta y la versión escritas.
pub fn cerrar_turno(root: &Path, sesion: Option<i64>, dueno: &str, cp: &crate::checkpoint::Checkpoint, desde: Option<usize>, resumen: &str) -> Option<(PathBuf, usize)> {
    let medios = recorrido::tomar_medios(dueno);
    let chat = chat_de(root, sesion, dueno);
    recorrido::cerrar_turno(&chat, cp, desde, root, resumen, &medios).ok().flatten().map(|v| (chat.ruta(Doc::Recorrido), v))
}

/// La carpeta de una conversación.
#[derive(Clone, Debug)]
pub struct Chat {
    dir: PathBuf,
    clave: String,
}

impl Chat {
    pub fn new(root: &Path, clave: &str) -> Chat {
        let clave = limpia(clave);
        Chat { dir: root.join(".gotv/chats").join(&clave), clave }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn clave(&self) -> &str {
        &self.clave
    }

    pub fn ruta(&self, doc: Doc) -> PathBuf {
        self.dir.join(doc.archivo())
    }

    fn historial(&self) -> PathBuf {
        self.dir.join("historial")
    }

    fn ruta_version(&self, doc: Doc, n: usize) -> PathBuf {
        self.historial().join(format!("{n}-{}", doc.archivo()))
    }

    /// El texto actual del documento.
    pub fn leer(&self, doc: Doc) -> Option<String> {
        std::fs::read_to_string(self.ruta(doc)).ok()
    }

    /// Una versión guardada.
    pub fn version(&self, doc: Doc, n: usize) -> Option<String> {
        std::fs::read_to_string(self.ruta_version(doc, n)).ok()
    }

    /// Los números de versión guardados de un documento, de menor a mayor.
    pub fn versiones(&self, doc: Doc) -> Vec<usize> {
        let sufijo = format!("-{}", doc.archivo());
        let mut v: Vec<usize> = std::fs::read_dir(self.historial())
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| e.file_name().to_str().and_then(|n| n.strip_suffix(&sufijo)).and_then(|n| n.parse().ok()))
            .collect();
        v.sort_unstable();
        v
    }

    /// Escribe el documento y guarda su versión. Devuelve el número de versión. Si el texto es igual
    /// al de la última versión no se crea otra (devuelve la misma).
    pub fn escribir(&self, doc: Doc, texto: &str) -> Result<usize> {
        let texto = recortar(texto);
        let versiones = self.versiones(doc);
        if let Some(&ult) = versiones.last() {
            if self.version(doc, ult).as_deref() == Some(texto.as_str()) {
                return Ok(ult);
            }
        }
        let n = versiones.last().map_or(1, |u| u + 1);
        std::fs::create_dir_all(self.historial()).with_context(|| self.historial().display().to_string())?;
        escribir_atomico(&self.ruta_version(doc, n), &texto)?;
        escribir_atomico(&self.ruta(doc), &texto)?;
        // trampa 3: tope por documento; lo que sobra se borra por su ruta exacta (creada aquí)
        for viejo in versiones.iter().copied().filter(|v| *v + MAX_VERSIONES <= n) {
            let _ = std::fs::remove_file(self.ruta_version(doc, viejo));
        }
        // los comentarios siguen a sus líneas en la versión nueva (trampa 1)
        comentarios::reubicar(self, doc)?;
        Ok(n)
    }

    /// Lo que hay de los tres documentos (los que existen).
    pub fn listar(&self) -> Vec<Resumen> {
        Doc::TODOS
            .into_iter()
            .filter(|d| self.ruta(*d).is_file())
            .map(|d| Resumen { archivo: d.archivo(), versiones: self.versiones(d), ruta: self.ruta(d) })
            .collect()
    }
}

fn recortar(texto: &str) -> String {
    if texto.len() <= MAX_BYTES_DOC {
        return texto.to_string();
    }
    let mut corte = MAX_BYTES_DOC;
    while !texto.is_char_boundary(corte) {
        corte -= 1;
    }
    format!(
        "{}\n\n{}",
        &texto[..corte],
        crate::tr!("…[recortado: el documento pasaba de {} KiB]", "…[truncated: the document went past {} KiB]", MAX_BYTES_DOC / 1024)
    )
}

pub(crate) fn escribir_atomico(ruta: &Path, texto: &str) -> Result<()> {
    if let Some(p) = ruta.parent() {
        std::fs::create_dir_all(p)?;
    }
    let tmp = ruta.with_extension("tmp");
    std::fs::write(&tmp, texto).with_context(|| tmp.display().to_string())?;
    std::fs::rename(&tmp, ruta).with_context(|| ruta.display().to_string())?;
    Ok(())
}

/// Lo que el MODELO no puede escribir con sus herramientas: la carpeta de las conversaciones (planes,
/// recorridos y, sobre todo, los comentarios del Portador) y la lista de sitios del navegador. Leerlos
/// sí. `resuelta` es la ruta ya resuelta de una herramienta de archivos; `crudo`, el sujeto (un bash).
pub fn protegido(root: &Path, herramienta: &str, edita: bool, crudo: Option<&str>, resuelta: Option<&Path>) -> Option<String> {
    // APFS no distingue mayúsculas: `.GOTV/Chats` es la misma carpeta
    let min = |p: &Path| crate::paths::normalize(p).to_string_lossy().to_lowercase();
    let chats = min(&root.join(".gotv/chats"));
    let sitios = min(&root.join(sitios::ARCHIVO));
    let cae = |p: &Path| {
        let p = min(p);
        p == chats || p.starts_with(&format!("{chats}/")) || p == sitios
    };
    let por_ruta = edita && resuelta.is_some_and(cae);
    let por_comando = herramienta == "bash"
        && crudo.map(str::to_lowercase).is_some_and(|c| [".gotv/chats", ".gotv/navegador", "comentarios.gotv"].iter().any(|x| c.contains(x)) && escribe(&c));
    (por_ruta || por_comando).then(|| {
        crate::i18n::t(
            "DENEGADO: `.gotv/chats/` (planes, recorridos y comentarios del usuario) y `.gotv/navegador` los escribe GOTV o el usuario, nunca el modelo. Para presentar un plan usa `exit_plan`; el recorrido lo arma GOTV al cerrar el turno.",
            "DENIED: `.gotv/chats/` (plans, walkthroughs and the user's comments) and `.gotv/navegador` are written by GOTV or the user, never by the model. To present a plan use `exit_plan`; GOTV builds the walkthrough when the turn closes.",
        )
        .to_string()
    })
}

/// ¿Este comando puede escribir? (redirección, copiar, mover, borrar, editar en sitio, intérpretes)
fn escribe(c: &str) -> bool {
    c.contains('>')
        || c.split(|ch: char| ch.is_whitespace() || matches!(ch, ';' | '|' | '&' | '(' | ')' | '`'))
            .any(|p| {
                let p = p.rsplit('/').next().unwrap_or(p);
                matches!(
                    p,
                    "tee" | "cp" | "mv" | "rm" | "ln" | "touch" | "truncate" | "dd" | "install" | "rsync" | "sed" | "perl" | "python" | "python3" | "node" | "ruby" | "awk" | "ed" | "ex" | "vi" | "vim" | "nano" | "unlink" | "shred" | "chmod" | "osascript"
                )
            })
}
