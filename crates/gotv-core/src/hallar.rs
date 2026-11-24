//! F8.6 · Buscar en todos los chats (⌘K): lo que se habló en cualquier proyecto, sin abrirlos uno a uno.
//!
//! **Rust puro, sin base aparte.** El plan pedía un índice FTS5 en `~/.gotv/gotv.db`; las conversaciones
//! ya viven en el códec propio (`session.rs`, orden del Portador: los datos de GOTV, en Rust), y otra base
//! sería una segunda copia de todo lo hablado que hay que mantener al día. En su lugar: se leen los
//! archivos de mensajes y se guarda en memoria su texto VISIBLE normalizado (minúsculas, sin tildes), por
//! archivo, con su fecha de cambio: la segunda búsqueda sólo relee lo que cambió. Medido en la prueba
//! `doscientos_chats_en_milisegundos`.
//!
//! Sólo se busca en lo que se ve en la ventana (tus mensajes sin anexos y las respuestas): nunca en
//! llamadas, resultados de herramientas ni avisos internos.

use serde::Serialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::SystemTime;

/// Lo más que se devuelve.
pub const MAX: usize = 50;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Hallazgo {
    pub proyecto: String,
    pub raiz: String,
    pub chat: i64,
    pub titulo: String,
    /// el índice del mensaje en el historial (para abrir el chat en ese punto)
    pub mensaje: usize,
    pub rol: String,
    /// un trozo alrededor de lo encontrado, con lo encontrado entre » «
    pub fragmento: String,
    pub fecha: String,
}

/// Un mensaje visible: índice, rol, texto tal cual y normalizado.
type Visible = (usize, String, String, String);

struct EnMemoria {
    sello: (SystemTime, u64),
    mensajes: Vec<Visible>,
}

fn memoria() -> &'static Mutex<HashMap<PathBuf, EnMemoria>> {
    static M: OnceLock<Mutex<HashMap<PathBuf, EnMemoria>>> = OnceLock::new();
    M.get_or_init(Default::default)
}

/// Minúsculas y sin tildes (la búsqueda de «cancion» encuentra «Canción»). Conserva el largo en caracteres,
/// para que las posiciones valgan en el texto original.
pub fn normaliza(t: &str) -> String {
    t.chars()
        .map(|c| match c {
            'á' | 'à' | 'ä' | 'â' | 'Á' | 'À' | 'Ä' | 'Â' => 'a',
            'é' | 'è' | 'ë' | 'ê' | 'É' | 'È' | 'Ë' | 'Ê' => 'e',
            'í' | 'ì' | 'ï' | 'î' | 'Í' | 'Ì' | 'Ï' | 'Î' => 'i',
            'ó' | 'ò' | 'ö' | 'ô' | 'Ó' | 'Ò' | 'Ö' | 'Ô' => 'o',
            'ú' | 'ù' | 'ü' | 'û' | 'Ú' | 'Ù' | 'Ü' | 'Û' => 'u',
            'ñ' | 'Ñ' => 'n',
            'ç' | 'Ç' => 'c',
            c => c.to_lowercase().next().unwrap_or(c),
        })
        .collect()
}

/// Lo visible de un historial.
fn visibles(h: &[gotv_engine::Message]) -> Vec<Visible> {
    h.iter()
        .enumerate()
        .filter_map(|(i, m)| {
            let t = if m.role == "user" && !m.es_aviso() && !crate::mensajes::es_adjuntos_pedidos(m) {
                crate::mensajes::visible(m).to_string()
            } else if m.role == "assistant" && m.tool_calls.is_none() {
                crate::mensajes::sin_ubicaciones(m.content.split("\n\n(Skills").next().unwrap_or(&m.content))
            } else {
                return None;
            };
            (!t.trim().is_empty()).then(|| (i, m.role.clone(), normaliza(&t), t))
        })
        .map(|(i, r, n, t)| (i, r, t, n))
        .collect()
}

/// Los mensajes visibles de un chat, de la memoria si el archivo no cambió.
fn de_chat(raiz: &Path, id: i64) -> Vec<Visible> {
    let archivo = raiz.join(format!(".gotv/sesiones/{id}.msj"));
    let sello = std::fs::metadata(&archivo).ok().map(|m| (m.modified().unwrap_or(SystemTime::UNIX_EPOCH), m.len()));
    let Some(sello) = sello else { return vec![] };
    if let Some(e) = memoria().lock().unwrap_or_else(|e| e.into_inner()).get(&archivo) {
        if e.sello == sello {
            return e.mensajes.clone();
        }
    }
    let mensajes = crate::session::Session::open(raiz, id).map(|(_, h)| visibles(&h)).unwrap_or_default();
    memoria().lock().unwrap_or_else(|e| e.into_inner()).insert(archivo, EnMemoria { sello, mensajes: mensajes.clone() });
    mensajes
}

/// El trozo alrededor de la primera aparición de `termino` (normalizado) en `texto`, marcado » «.
fn fragmento(texto: &str, norm: &str, termino: &str) -> String {
    let cs: Vec<char> = texto.chars().collect();
    let Some(pos_b) = norm.find(termino) else { return cs.iter().take(120).collect() };
    let inicio = norm[..pos_b].chars().count();
    let fin = inicio + termino.chars().count();
    let (a, b) = (inicio.saturating_sub(50), (fin + 70).min(cs.len()));
    let s = |r: std::ops::Range<usize>| cs[r].iter().collect::<String>().replace('\n', " ");
    format!("{}{}»{}«{}{}", if a > 0 { "…" } else { "" }, s(a..inicio), s(inicio..fin), s(fin..b), if b < cs.len() { "…" } else { "" })
}

/// Busca en los chats de estos proyectos `(nombre, raíz)`. Todas las palabras de la consulta deben estar en
/// el mismo mensaje (sin importar tildes ni mayúsculas). Lo más reciente primero.
pub fn en_chats(proyectos: &[(String, PathBuf)], consulta: &str, max: usize) -> Vec<Hallazgo> {
    let q = normaliza(consulta.trim());
    let terminos: Vec<&str> = q.split_whitespace().collect();
    if terminos.is_empty() {
        return vec![];
    }
    let mut out = Vec::new();
    for (nombre, raiz) in proyectos {
        // TODOS los chats vivos (la lista de la ventana sólo enseña los 50 últimos)
        let Ok((chats, _)) = crate::session::Session::page(raiz, 0, usize::MAX) else { continue };
        for s in chats {
            for (i, rol, texto, norm) in de_chat(raiz, s.id).into_iter().rev() {
                if terminos.iter().all(|t| norm.contains(t)) {
                    out.push((s.created.clone(), i, Hallazgo { proyecto: nombre.clone(), raiz: raiz.display().to_string(), chat: s.id, titulo: s.title.clone(), mensaje: i, rol, fragmento: fragmento(&texto, &norm, terminos[0]), fecha: s.created.clone() }));
                }
            }
        }
    }
    // lo más nuevo primero: por la fecha del chat y, dentro, por el mensaje
    out.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)));
    out.into_iter().take(max).map(|x| x.2).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::Session;
    use gotv_engine::{Message, ToolCall};

    fn proyecto(tag: &str) -> crate::CarpetaPrueba {
        let d = crate::CarpetaPrueba::sin_crear(&format!("gotv-hallar-{tag}"));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn encuentra_en_varios_proyectos_sin_tildes_y_sin_lo_interno() {
        let (a, b) = (proyecto("a"), proyecto("b"));
        let call = ToolCall { id: "c".into(), name: "read_file".into(), arguments: serde_json::json!({"path": "canción.txt"}) };
        let s = Session::open_new(&a, "m").unwrap();
        s.save(&[Message::system("Canción del sistema"), Message::user("Escribe la CANCIÓN del pedido"), Message::assistant_calls("", std::slice::from_ref(&call)), Message::tool_result(&call, "canción secreta"), Message::aviso("canción aviso"), Message::assistant("Listo: la canción tiene tres estrofas.")]).unwrap();
        let t = Session::open_new(&b, "m").unwrap();
        t.save(&[Message::user("otra canción, de cuna"), Message::assistant("vale")]).unwrap();
        let ps = vec![("A".to_string(), a.to_path_buf()), ("B".to_string(), b.to_path_buf())];
        let h = en_chats(&ps, "cancion", MAX);
        assert_eq!(h.len(), 3, "dos en A (tú y la respuesta) y uno en B: {h:#?}");
        assert!(h.iter().all(|x| !x.fragmento.contains("secreta") && !x.fragmento.contains("aviso") && !x.fragmento.contains("sistema")));
        let r = h.iter().find(|x| x.proyecto == "A" && x.rol == "assistant").unwrap();
        assert_eq!(r.mensaje, 5);
        assert!(r.fragmento.contains("»canción«"), "{}", r.fragmento);
        // todas las palabras, en el mismo mensaje
        assert_eq!(en_chats(&ps, "canción cuna", MAX).len(), 1);
        assert!(en_chats(&ps, "   ", MAX).is_empty());
        // la memoria se renueva cuando el chat cambia
        std::thread::sleep(std::time::Duration::from_millis(20));
        t.save(&[Message::user("ya no hay nada de eso"), Message::assistant("bien")]).unwrap();
        assert_eq!(en_chats(&ps, "cuna", MAX).len(), 0);
    }

    /// 200 chats de 60 mensajes (12 000 mensajes): la primera búsqueda lee todo; la segunda, de la memoria.
    #[test]
    fn doscientos_chats_en_milisegundos() {
        let r = proyecto("muchos");
        for c in 0..200 {
            let s = Session::open_new(&r, "m").unwrap();
            let h: Vec<Message> = (0..60).map(|i| if i % 2 == 0 { Message::user(format!("orden {c}-{i}: revisa el módulo de facturación y los impuestos del pedido")) } else { Message::assistant(format!("respuesta {c}-{i}: hecho, con las pruebas en verde y la documentación al día")) }).collect();
            s.save(&h).unwrap();
        }
        let ps = vec![("p".to_string(), r.to_path_buf())];
        let t0 = std::time::Instant::now();
        let h = en_chats(&ps, "orden 137-58", MAX);
        let primera = t0.elapsed();
        let t1 = std::time::Instant::now();
        let h2 = en_chats(&ps, "facturacion impuestos", MAX);
        let segunda = t1.elapsed();
        println!("primera {:.1} ms · segunda {:.1} ms", primera.as_secs_f64() * 1e3, segunda.as_secs_f64() * 1e3);
        assert_eq!(h.len(), 1);
        assert_eq!(h2.len(), MAX);
        assert!(segunda.as_millis() < 500, "de la memoria: {segunda:?}");
    }
}
