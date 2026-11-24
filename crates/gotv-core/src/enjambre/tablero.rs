//! F9 · el tablero común de un equipo: notas cortas que se dejan los subagentes y el coordinador («ya
//! existe validar() en util.js», «toco sólo rutas/pedidos.js»), EN ORDEN. Vive en la carpeta de la
//! conversación del agente principal (`.gotv/chats/<chat>/tablero.gotv`), en el códec propio de GOTV (tipo
//! `TABL`); cada nota lleva su número, quién la dejó y cuándo. Varios escriben a la vez: un candado
//! (`flock`) ordena las escrituras, así el número es el orden real y nunca se pierde una nota.
//!
//! Los subagentes trabajan en OTRA carpeta (su rama): para que todos escriban en el MISMO tablero, el
//! agente principal registra el de cada uno ([`asignar`]) por su dueño de procesos, y lo suelta al terminar.

use crate::programar::codec::{self, Candado, Escritor, Lector};
use anyhow::Result;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

const TIPO: &[u8; 4] = b"TABL";
const VERSION: u8 = 1;
/// Lo más que se guarda (las más viejas se van): un tablero no es un registro.
pub const MAX_NOTAS: usize = 400;
/// El largo de una nota (lo que pase se recorta).
pub const MAX_CHARS: usize = 2_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Nota {
    pub n: u32,
    pub de: String,
    pub texto: String,
    /// segundos desde 1970
    pub cuando: i64,
}

/// El tablero de la conversación cuya carpeta es `dir_chat`.
pub fn ruta(dir_chat: &Path) -> PathBuf {
    dir_chat.join("tablero.gotv")
}

fn ahora() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

fn decodificar(b: &[u8]) -> Result<Vec<Nota>> {
    let (mut l, _) = Lector::nuevo(b, TIPO, VERSION)?;
    let total = l.u32()? as usize;
    let mut v = Vec::with_capacity(total.min(MAX_NOTAS));
    for _ in 0..total {
        v.push(Nota { n: l.u32()?, de: l.texto()?, texto: l.texto()?, cuando: l.i64()? });
    }
    Ok(v)
}

fn codificar(notas: &[Nota]) -> Vec<u8> {
    let mut e = Escritor::nuevo(TIPO, VERSION);
    e.u32(notas.len() as u32);
    for n in notas {
        e.u32(n.n);
        e.texto(&n.de);
        e.texto(&n.texto);
        e.i64(n.cuando);
    }
    e.bytes()
}

/// Las notas, en orden (vacío si aún no hay tablero o no se puede leer).
pub fn leer(ruta: &Path) -> Vec<Nota> {
    std::fs::read(ruta).ok().and_then(|b| decodificar(&b).ok()).unwrap_or_default()
}

/// Deja una nota y devuelve su número.
pub fn anotar(ruta: &Path, de: &str, texto: &str) -> Result<u32> {
    let _c = Candado::esperar(&ruta.with_extension("candado"))?;
    let mut notas = leer(ruta);
    let n = notas.last().map(|x| x.n + 1).unwrap_or(1);
    let texto: String = texto.trim().chars().take(MAX_CHARS).collect();
    notas.push(Nota { n, de: de.trim().chars().take(80).collect(), texto, cuando: ahora() });
    if notas.len() > MAX_NOTAS {
        let sobran = notas.len() - MAX_NOTAS;
        notas.drain(..sobran);
    }
    codec::guardar(ruta, &codificar(&notas))?;
    Ok(n)
}

/// Las notas desde la `desde` (exclusive) como texto para el modelo.
pub fn texto(notas: &[Nota], desde: u32) -> String {
    let v: Vec<String> = notas.iter().filter(|x| x.n > desde).map(|x| format!("#{} [{}] {}", x.n, x.de, x.texto)).collect();
    if v.is_empty() {
        return crate::i18n::t("(el tablero está vacío)", "(the board is empty)").to_string();
    }
    v.join("\n")
}

// ── qué tablero usa cada agente ────────────────────────────────────────────────────────────────────

/// dueño de procesos de un subagente → (tablero del equipo, su nombre en el tablero)
static REGISTRO: Mutex<Option<HashMap<String, (PathBuf, String)>>> = Mutex::new(None);

/// El subagente cuyo dueño de procesos es `dueno` escribe en `ruta` como `nombre`.
pub fn asignar(dueno: &str, ruta: PathBuf, nombre: &str) {
    REGISTRO.lock().unwrap().get_or_insert_with(HashMap::new).insert(dueno.to_string(), (ruta, nombre.to_string()));
}

/// Al terminar el subagente.
pub fn soltar(dueno: &str) {
    if let Some(m) = REGISTRO.lock().unwrap().as_mut() {
        m.remove(dueno);
    }
}

/// El tablero y el nombre de un subagente (`None`: es un agente principal, usa el de su conversación).
pub fn de(dueno: &str) -> Option<(PathBuf, String)> {
    REGISTRO.lock().unwrap().as_ref().and_then(|m| m.get(dueno).cloned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn el_tablero_guarda_el_orden_aunque_escriban_a_la_vez() {
        let d = crate::CarpetaPrueba::nueva("gotv-tablero-orden");
        let r = ruta(&d);
        // 4 escritores a la vez, 25 notas cada uno
        let hilos: Vec<_> = (0..4)
            .map(|h| {
                let r = r.clone();
                std::thread::spawn(move || (0..25).map(|i| anotar(&r, &format!("sub-{h}"), &format!("nota {i}")).unwrap()).collect::<Vec<u32>>())
            })
            .collect();
        let mut numeros: Vec<u32> = hilos.into_iter().flat_map(|h| h.join().unwrap()).collect();
        numeros.sort();
        assert_eq!(numeros, (1..=100).collect::<Vec<u32>>(), "ninguna nota se pierde ni repite número");
        let notas = leer(&r);
        assert_eq!(notas.len(), 100);
        assert!(notas.windows(2).all(|w| w[0].n + 1 == w[1].n), "se leen en orden");
        // lo de cada escritor, en SU orden
        for h in 0..4 {
            let suyas: Vec<&str> = notas.iter().filter(|x| x.de == format!("sub-{h}")).map(|x| x.texto.as_str()).collect();
            assert_eq!(suyas, (0..25).map(|i| format!("nota {i}")).collect::<Vec<_>>().iter().map(String::as_str).collect::<Vec<_>>());
        }
        assert!(texto(&notas, 98).starts_with("#99 ["), "{}", texto(&notas, 98));
        // es el códec propio, no texto
        assert_eq!(&std::fs::read(&r).unwrap()[..8], b"GOTVTABL");
    }

    #[test]
    fn el_tablero_tiene_tope_y_el_registro_se_suelta() {
        let d = crate::CarpetaPrueba::nueva("gotv-tablero-tope");
        let r = ruta(&d);
        for i in 0..(MAX_NOTAS + 5) {
            anotar(&r, "a", &i.to_string()).unwrap();
        }
        let notas = leer(&r);
        assert_eq!(notas.len(), MAX_NOTAS);
        assert_eq!(notas[0].n, 6, "se van las más viejas, los números siguen");
        asignar("dueno-x", r.clone(), "sub-1");
        assert_eq!(de("dueno-x"), Some((r, "sub-1".to_string())));
        soltar("dueno-x");
        assert_eq!(de("dueno-x"), None);
    }
}
