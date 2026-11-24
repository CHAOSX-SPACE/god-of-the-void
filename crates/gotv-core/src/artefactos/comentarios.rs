//! F7 paso 3 · Comentarios por línea sobre los documentos de una conversación (`comentarios.gotv`, formato propio).
//!
//! - Trampa 1: el número de línea se desplaza cuando el plan se reescribe. Cada comentario guarda
//!   también el TEXTO de su línea y se reubica por él (la coincidencia más cercana a la línea vieja);
//!   si ya no está, queda «huérfano» (`linea: None`): se muestra y se entrega igual, nunca se pierde.
//! - Trampa 5: son texto del usuario. Llegan al modelo en un bloque propio, marcados como suyos y
//!   separados del plan ([`para_el_modelo`]); el modelo no puede escribir este archivo
//!   ([`super::protegido`]).
//! - Un comentario se entrega UNA vez. Uno resuelto no se entrega nunca. Cuando el documento se
//!   reescribe después de entregarlo, queda resuelto (lo atendió la versión nueva); el usuario puede
//!   reabrirlo.

use super::{Chat, Doc};
use anyhow::Result;
use gotv_engine::{anyhow_tr, bail_tr};
use serde::{Deserialize, Serialize};

const ARCHIVO: &str = "comentarios.gotv";
/// Lo más largo que puede ser un comentario.
pub const MAX_CHARS: usize = 4000;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Comentario {
    pub id: u64,
    /// `plan.md`, `tareas.md` o `recorrido.md`
    pub archivo: String,
    /// Línea (desde 1) en la versión actual; `None` = huérfano (su línea ya no está).
    pub linea: Option<usize>,
    /// El texto de la línea comentada, para reubicarla.
    #[serde(default)]
    pub texto_linea: String,
    pub texto: String,
    #[serde(default)]
    pub resuelto: bool,
    /// Ya llegó al modelo (no se le repite).
    #[serde(default)]
    pub entregado: bool,
    /// La versión del documento que se comentó.
    #[serde(default)]
    pub version: usize,
}

const TIPO: &[u8; 4] = b"COME";
const VERSION: u8 = 1;

/// Todos los comentarios de la conversación, en el orden en que se hicieron. Se guardan en el formato
/// propio de GOTV (`programar::codec`, regla del Portador: lo nuestro, sólo Rust; nada de JSON).
pub fn todos(chat: &Chat) -> Vec<Comentario> {
    std::fs::read(chat.dir().join(ARCHIVO)).ok().and_then(|b| leer(&b).ok()).unwrap_or_default()
}

fn leer(b: &[u8]) -> Result<Vec<Comentario>> {
    let (mut l, _) = crate::programar::codec::Lector::nuevo(b, TIPO, VERSION)?;
    let mut v = Vec::new();
    for _ in 0..l.u32()? {
        v.push(Comentario {
            id: l.opt_u64()?.unwrap_or(0),
            archivo: l.texto()?,
            linea: l.opt_u64()?.map(|x| x as usize),
            texto_linea: l.texto()?,
            texto: l.texto()?,
            resuelto: l.bool()?,
            entregado: l.bool()?,
            version: l.opt_u64()?.unwrap_or(0) as usize,
        });
    }
    Ok(v)
}

fn guardar(chat: &Chat, cs: &[Comentario]) -> Result<()> {
    let mut e = crate::programar::codec::Escritor::nuevo(TIPO, VERSION);
    e.u32(cs.len() as u32);
    for c in cs {
        e.opt_u64(Some(c.id));
        e.texto(&c.archivo);
        e.opt_u64(c.linea.map(|x| x as u64));
        e.texto(&c.texto_linea);
        e.texto(&c.texto);
        e.bool(c.resuelto);
        e.bool(c.entregado);
        e.opt_u64(Some(c.version as u64));
    }
    std::fs::create_dir_all(chat.dir())?;
    crate::programar::codec::guardar(&chat.dir().join(ARCHIVO), &e.bytes())
}

/// El usuario comenta la línea `linea` (desde 1) de la versión actual de `archivo`.
pub fn comentar(chat: &Chat, archivo: &str, linea: usize, texto: &str) -> Result<Comentario> {
    let doc = Doc::de_archivo(archivo).ok_or_else(|| anyhow_tr!("«{archivo}» no es un documento de la conversación (plan.md, tareas.md, recorrido.md)", "«{archivo}» is not a conversation document (plan.md, tareas.md, recorrido.md)"))?;
    let actual = chat.leer(doc).ok_or_else(|| anyhow_tr!("aún no hay {archivo} en esta conversación", "there is no {archivo} in this conversation yet"))?;
    let lineas: Vec<&str> = actual.lines().collect();
    if linea == 0 || linea > lineas.len() {
        bail_tr!("{archivo} tiene {} líneas: la {linea} no existe", "{archivo} has {} lines: line {linea} does not exist", lineas.len());
    }
    let texto = texto.trim();
    if texto.is_empty() {
        bail_tr!("el comentario está vacío", "the comment is empty");
    }
    let texto: String = texto.chars().take(MAX_CHARS).collect();
    let mut cs = todos(chat);
    let c = Comentario {
        id: cs.iter().map(|c| c.id).max().unwrap_or(0) + 1,
        archivo: archivo.to_string(),
        linea: Some(linea),
        texto_linea: lineas[linea - 1].trim().to_string(),
        texto,
        resuelto: false,
        entregado: false,
        version: chat.versiones(doc).last().copied().unwrap_or(0),
    };
    cs.push(c.clone());
    guardar(chat, &cs)?;
    Ok(c)
}

/// Marca (o reabre) un comentario. Reabrirlo lo vuelve a entregar.
pub fn resolver(chat: &Chat, id: u64, resuelto: bool) -> Result<()> {
    let mut cs = todos(chat);
    let c = cs.iter_mut().find(|c| c.id == id).ok_or_else(|| anyhow_tr!("no hay comentario #{id}", "there is no comment #{id}"))?;
    c.resuelto = resuelto;
    if !resuelto {
        c.entregado = false;
    }
    guardar(chat, &cs)
}

/// Tras escribir una versión nueva de `doc`: lo ya entregado queda resuelto y lo pendiente sigue a su
/// línea por su texto (o queda huérfano).
pub(super) fn reubicar(chat: &Chat, doc: Doc) -> Result<()> {
    let mut cs = todos(chat);
    if !cs.iter().any(|c| c.archivo == doc.archivo()) {
        return Ok(());
    }
    let texto = chat.leer(doc).unwrap_or_default();
    let lineas: Vec<&str> = texto.lines().map(str::trim).collect();
    for c in cs.iter_mut().filter(|c| c.archivo == doc.archivo() && !c.resuelto) {
        if c.entregado {
            c.resuelto = true;
            continue;
        }
        c.linea = ubicar(&lineas, c.linea, &c.texto_linea);
    }
    guardar(chat, &cs)
}

/// Dónde está ahora una línea: la misma posición si su texto sigue ahí; si no, la coincidencia más
/// cercana a donde estaba; si no hay ninguna, `None` (huérfano).
pub fn ubicar(lineas: &[&str], antes: Option<usize>, texto: &str) -> Option<usize> {
    let texto = texto.trim();
    if let Some(l) = antes.filter(|l| *l >= 1 && lineas.get(*l - 1) == Some(&texto)) {
        return Some(l);
    }
    if texto.is_empty() {
        return None;
    }
    let cerca = antes.unwrap_or(1);
    lineas
        .iter()
        .enumerate()
        .filter(|(_, l)| **l == texto)
        .map(|(i, _)| i + 1)
        .min_by_key(|n| n.abs_diff(cerca))
}

/// Los comentarios pendientes (ni resueltos ni entregados), en un bloque por documento marcado como
/// texto del usuario. Los marca como entregados. `None` si no hay ninguno.
pub fn para_el_modelo(chat: &Chat) -> Option<String> {
    let mut cs = todos(chat);
    let pendientes: Vec<usize> = (0..cs.len()).filter(|i| !cs[*i].resuelto && !cs[*i].entregado).collect();
    if pendientes.is_empty() {
        return None;
    }
    let mut bloques = Vec::new();
    for doc in Doc::TODOS {
        let de_este: Vec<&Comentario> = pendientes.iter().map(|i| &cs[*i]).filter(|c| c.archivo == doc.archivo()).collect();
        if de_este.is_empty() {
            continue;
        }
        let mut b = crate::tr!(
            "[Comentarios del usuario sobre {}] (texto SUYO, no parte del documento: atiéndelos antes de tocar archivos)",
            "[The user's comments on {}] (THEIR text, not part of the document: address them before touching files)",
            doc.archivo()
        );
        for c in de_este {
            b.push('\n');
            b.push_str(&match c.linea {
                Some(l) => format!("L{l} («{}»): {}", c.texto_linea, c.texto),
                None => crate::tr!(
                    "(huérfano: la línea «{}» ya no está) {}",
                    "(orphan: the line «{}» is no longer there) {}",
                    c.texto_linea,
                    c.texto
                ),
            });
        }
        bloques.push(b);
    }
    for i in pendientes {
        cs[i].entregado = true;
    }
    guardar(chat, &cs).ok()?;
    Some(bloques.join("\n\n"))
}
