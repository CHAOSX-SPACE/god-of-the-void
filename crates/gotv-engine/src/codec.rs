//! El formato en el que GOTV guarda lo SUYO (las sesiones y sus mensajes, el registro de tareas, la agenda de
//! un chat): binario, escrito y leído con `std`, sin formatos intermedios (regla del Portador: lo nuestro,
//! sólo Rust). Cabecera `GOTV` + tipo + versión; enteros en little-endian; textos con su largo delante; un
//! árbol de valores (las llamadas a herramientas tal como las da el motor) con una etiqueta por nodo.
use anyhow::{bail, Result};
use std::path::Path;

const MAGIA: &[u8; 4] = b"GOTV";

pub struct Escritor {
    buf: Vec<u8>,
}

impl Escritor {
    /// `tipo`: qué se guarda (4 bytes, p. ej. `b"TARE"`); `version`: la del formato de ese tipo.
    pub fn nuevo(tipo: &[u8; 4], version: u8) -> Self {
        let mut buf = Vec::with_capacity(256);
        buf.extend_from_slice(MAGIA);
        buf.extend_from_slice(tipo);
        buf.push(version);
        Escritor { buf }
    }
    pub fn u8(&mut self, v: u8) {
        self.buf.push(v);
    }
    pub fn bool(&mut self, v: bool) {
        self.buf.push(v as u8);
    }
    pub fn u32(&mut self, v: u32) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    pub fn i64(&mut self, v: i64) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    pub fn texto(&mut self, s: &str) {
        self.u32(s.len() as u32);
        self.buf.extend_from_slice(s.as_bytes());
    }
    pub fn opt_i64(&mut self, v: Option<i64>) {
        self.bool(v.is_some());
        if let Some(x) = v {
            self.i64(x);
        }
    }
    pub fn opt_u64(&mut self, v: Option<u64>) {
        self.opt_i64(v.map(|x| x as i64));
    }
    pub fn opt_texto(&mut self, v: Option<&str>) {
        self.bool(v.is_some());
        if let Some(x) = v {
            self.texto(x);
        }
    }
    pub fn textos(&mut self, v: &[String]) {
        self.u32(v.len() as u32);
        for s in v {
            self.texto(s);
        }
    }
    /// Un árbol de valores, en binario: 0 nulo · 1 falso · 2 verdadero · 3 entero · 4 natural · 5 real ·
    /// 6 texto · 7 lista · 8 mapa (claves en su orden).
    pub fn valor(&mut self, v: &serde_json::Value) {
        use serde_json::Value as V;
        match v {
            V::Null => self.u8(0),
            V::Bool(b) => self.u8(1 + *b as u8),
            V::Number(n) => {
                if let Some(i) = n.as_i64() {
                    self.u8(3);
                    self.i64(i);
                } else if let Some(u) = n.as_u64() {
                    self.u8(4);
                    self.buf.extend_from_slice(&u.to_le_bytes());
                } else {
                    self.u8(5);
                    self.buf.extend_from_slice(&n.as_f64().unwrap_or(0.0).to_le_bytes());
                }
            }
            V::String(s) => {
                self.u8(6);
                self.texto(s);
            }
            V::Array(a) => {
                self.u8(7);
                self.u32(a.len() as u32);
                for x in a {
                    self.valor(x);
                }
            }
            V::Object(o) => {
                self.u8(8);
                self.u32(o.len() as u32);
                for (k, x) in o {
                    self.texto(k);
                    self.valor(x);
                }
            }
        }
    }
    pub fn opt_valor(&mut self, v: Option<&serde_json::Value>) {
        self.bool(v.is_some());
        if let Some(x) = v {
            self.valor(x);
        }
    }
    /// Bytes tal cual (el texto de los trozos del índice, que se lee luego por posición).
    pub fn crudo(&mut self, b: &[u8]) {
        self.buf.extend_from_slice(b);
    }
    /// Cuánto lleva escrito (la posición de lo siguiente).
    pub fn largo(&self) -> usize {
        self.buf.len()
    }
    pub fn bytes(self) -> Vec<u8> {
        self.buf
    }
}

pub struct Lector<'a> {
    b: &'a [u8],
    i: usize,
}

impl<'a> Lector<'a> {
    /// Comprueba cabecera, tipo y versión (una versión más NUEVA que la que sabe leer no se toca).
    pub fn nuevo(b: &'a [u8], tipo: &[u8; 4], version_max: u8) -> Result<(Self, u8)> {
        if b.len() < 9 || &b[..4] != MAGIA || &b[4..8] != tipo {
            bail!("{}", crate::i18n::t("no es un archivo de GOTV de este tipo", "not a GOTV file of this kind"));
        }
        let v = b[8];
        if v == 0 || v > version_max {
            bail!("{}", crate::tr!("versión {v} desconocida (esta GOTV lee hasta la {version_max})", "unknown version {v} (this GOTV reads up to {version_max})"));
        }
        Ok((Lector { b, i: 9 }, v))
    }
    fn tomar(&mut self, n: usize) -> Result<&'a [u8]> {
        if self.i + n > self.b.len() {
            bail!("{}", crate::i18n::t("archivo cortado", "truncated file"));
        }
        let s = &self.b[self.i..self.i + n];
        self.i += n;
        Ok(s)
    }
    pub fn u8(&mut self) -> Result<u8> {
        Ok(self.tomar(1)?[0])
    }
    pub fn bool(&mut self) -> Result<bool> {
        Ok(self.u8()? != 0)
    }
    pub fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.tomar(4)?.try_into()?))
    }
    pub fn i64(&mut self) -> Result<i64> {
        Ok(i64::from_le_bytes(self.tomar(8)?.try_into()?))
    }
    pub fn texto(&mut self) -> Result<String> {
        let n = self.u32()? as usize;
        Ok(std::str::from_utf8(self.tomar(n)?)?.to_string())
    }
    pub fn opt_i64(&mut self) -> Result<Option<i64>> {
        Ok(if self.bool()? { Some(self.i64()?) } else { None })
    }
    pub fn opt_u64(&mut self) -> Result<Option<u64>> {
        Ok(self.opt_i64()?.map(|x| x as u64))
    }
    pub fn opt_texto(&mut self) -> Result<Option<String>> {
        Ok(if self.bool()? { Some(self.texto()?) } else { None })
    }
    pub fn textos(&mut self) -> Result<Vec<String>> {
        let n = self.u32()? as usize;
        (0..n).map(|_| self.texto()).collect()
    }
    pub fn valor(&mut self) -> Result<serde_json::Value> {
        self.valor_en(0)
    }
    fn valor_en(&mut self, hondo: usize) -> Result<serde_json::Value> {
        use serde_json::Value as V;
        if hondo > 128 {
            bail!("{}", crate::i18n::t("árbol demasiado hondo", "tree too deep"));
        }
        Ok(match self.u8()? {
            0 => V::Null,
            1 => V::Bool(false),
            2 => V::Bool(true),
            3 => V::from(self.i64()?),
            4 => V::from(u64::from_le_bytes(self.tomar(8)?.try_into()?)),
            5 => serde_json::Number::from_f64(f64::from_le_bytes(self.tomar(8)?.try_into()?)).map(V::Number).unwrap_or(V::Null),
            6 => V::String(self.texto()?),
            7 => {
                let n = self.u32()? as usize;
                V::Array((0..n).map(|_| self.valor_en(hondo + 1)).collect::<Result<_>>()?)
            }
            8 => {
                let n = self.u32()? as usize;
                let mut m = serde_json::Map::new();
                for _ in 0..n {
                    let k = self.texto()?;
                    m.insert(k, self.valor_en(hondo + 1)?);
                }
                V::Object(m)
            }
            e => bail!("{}", crate::tr!("etiqueta de valor desconocida {e}", "unknown value tag {e}")),
        })
    }
    pub fn opt_valor(&mut self) -> Result<Option<serde_json::Value>> {
        Ok(if self.bool()? { Some(self.valor()?) } else { None })
    }
    pub fn fin(&self) -> bool {
        self.i == self.b.len()
    }
}

/// La versión con la que se escribió un archivo de este tipo (aunque sea más nueva de lo que se sabe leer).
pub fn version_de(b: &[u8], tipo: &[u8; 4]) -> Option<u8> {
    (b.len() >= 9 && &b[..4] == MAGIA && &b[4..8] == tipo).then(|| b[8])
}

/// Un candado exclusivo sobre un archivo (`flock`): se suelta al soltarlo (o si el proceso muere). Lo usan
/// el registro de tareas, cada tarea y la carpeta de sesiones (la app y la CLI pueden coincidir).
pub struct Candado(#[allow(dead_code)] std::fs::File);

impl Candado {
    /// `Ok(None)` = otro lo tiene. Insiste ~250 ms antes de rendirse: `flock` va por DESCRIPTOR abierto, y
    /// cuando otro hilo del proceso lanza un programa, el hijo hereda una copia entre su `fork` y su `exec`;
    /// mientras tanto el candado sigue tomado aunque ya se haya soltado (medido 2026-11-17: la corrida 2 de
    /// una tarea se saltaba como «la tiene otra» 2 de cada 6 veces, sólo con otra prueba lanzando procesos a
    /// la vez). Una corrida de verdad dura minutos: sigue siendo «la tiene otra».
    pub fn intentar(ruta: &Path) -> Result<Option<Candado>> {
        for i in 0..10 {
            if let Some(c) = Self::abrir(ruta, false)? {
                return Ok(Some(c));
            }
            if i < 9 {
                std::thread::sleep(std::time::Duration::from_millis(25));
            }
        }
        Ok(None)
    }

    /// Espera a tenerlo.
    pub fn esperar(ruta: &Path) -> Result<Candado> {
        Self::abrir(ruta, true)?.ok_or_else(|| anyhow::anyhow!("flock"))
    }

    fn abrir(ruta: &Path, esperar: bool) -> Result<Option<Candado>> {
        use std::os::fd::AsRawFd;
        if let Some(d) = ruta.parent() {
            std::fs::create_dir_all(d)?;
        }
        let f = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(ruta)?;
        let modo = libc::LOCK_EX | if esperar { 0 } else { libc::LOCK_NB };
        // SAFETY: flock sobre un descriptor abierto aquí y vivo mientras viva el File
        if unsafe { libc::flock(f.as_raw_fd(), modo) } == 0 {
            return Ok(Some(Candado(f)));
        }
        let e = std::io::Error::last_os_error();
        if e.raw_os_error() == Some(libc::EWOULDBLOCK) {
            return Ok(None);
        }
        Err(e.into())
    }
}

/// Escribe de forma atómica (temporal + renombrar): un corte a mitad nunca deja un archivo a medias.
pub fn guardar(ruta: &std::path::Path, bytes: &[u8]) -> Result<()> {
    if let Some(d) = ruta.parent() {
        std::fs::create_dir_all(d)?;
    }
    let mut tmp = ruta.as_os_str().to_owned();
    tmp.push(".tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, ruta)?;
    Ok(())
}

/// Un mapa texto → texto en el formato propio (`tipo`, versión 1). Lo que no se pueda leer = vacío.
pub fn leer_mapa(ruta: &Path, tipo: &[u8; 4]) -> std::collections::BTreeMap<String, String> {
    let Ok(b) = std::fs::read(ruta) else { return Default::default() };
    let leer = || -> Result<std::collections::BTreeMap<String, String>> {
        let (mut l, _) = Lector::nuevo(&b, tipo, 1)?;
        let (k, v) = (l.textos()?, l.textos()?);
        Ok(k.into_iter().zip(v).collect())
    };
    leer().unwrap_or_default()
}

pub fn guardar_mapa(ruta: &Path, tipo: &[u8; 4], m: &std::collections::BTreeMap<String, String>) -> Result<()> {
    let mut e = Escritor::nuevo(tipo, 1);
    e.textos(&m.keys().cloned().collect::<Vec<_>>());
    e.textos(&m.values().cloned().collect::<Vec<_>>());
    guardar(ruta, &e.bytes())
}

/// Migración de un almacén que antes era JSON (decisión del Portador 2026-11-17: los datos PROPIOS de GOTV
/// van en formato propio). Si `propio` no existe y `viejo` sí, lo lee UNA vez, lo guarda en lo propio y
/// renombra el viejo a `*.migrado` (no se borra). Devuelve lo migrado; `None` si no había nada que migrar.
pub fn migrar_json<T: serde::de::DeserializeOwned>(propio: &Path, viejo: &Path, guardar: impl FnOnce(&T) -> Result<()>) -> Option<T> {
    if propio.exists() || !viejo.is_file() {
        return None;
    }
    let t: T = std::fs::read_to_string(viejo).ok().and_then(|t| serde_json::from_str(&t).ok())?;
    guardar(&t).ok()?;
    let mut fuera = viejo.as_os_str().to_owned();
    fuera.push(".migrado");
    let _ = std::fs::rename(viejo, fuera);
    Some(t)
}

/// Cualquier struct de GOTV en su formato propio: el valor con la codificación binaria del códec.
pub fn guardar_serde<T: serde::Serialize>(ruta: &Path, tipo: &[u8; 4], v: &T) -> Result<()> {
    let mut e = Escritor::nuevo(tipo, 1);
    e.valor(&serde_json::to_value(v)?);
    guardar(ruta, &e.bytes())
}

pub fn leer_serde<T: serde::de::DeserializeOwned>(ruta: &Path, tipo: &[u8; 4]) -> Option<T> {
    let b = std::fs::read(ruta).ok()?;
    let (mut l, _) = Lector::nuevo(&b, tipo, 1).ok()?;
    serde_json::from_value(l.valor().ok()?).ok()
}

/// `leer_serde`, migrando antes el JSON viejo si es lo único que hay.
pub fn leer_serde_migrado<T: serde::Serialize + serde::de::DeserializeOwned>(propio: &Path, viejo: &Path, tipo: &[u8; 4]) -> Option<T> {
    migrar_json::<T>(propio, viejo, |t| guardar_serde(propio, tipo, t)).or_else(|| leer_serde(propio, tipo))
}

/// Un mapa texto → texto que antes era un objeto JSON plano: lee lo propio o migra el viejo.
pub fn mapa_migrado(propio: &Path, viejo: &Path, tipo: &[u8; 4]) -> std::collections::BTreeMap<String, String> {
    migrar_json::<std::collections::BTreeMap<String, serde_json::Value>>(propio, viejo, |m| guardar_mapa(propio, tipo, &a_textos(m)))
        .map(|m| a_textos(&m))
        .unwrap_or_else(|| leer_mapa(propio, tipo))
}

fn a_textos(m: &std::collections::BTreeMap<String, serde_json::Value>) -> std::collections::BTreeMap<String, String> {
    m.iter().map(|(k, v)| (k.clone(), v.as_str().map(String::from).unwrap_or_else(|| v.to_string()))).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn el_mapa_viejo_en_json_se_migra_una_vez() {
        let d = crate::prueba::CarpetaPrueba::nueva("gotv-codec-migrar");
        let (propio, viejo) = (d.join("x.gotv"), d.join("x.json"));
        std::fs::write(&viejo, r#"{"/a": "h1", "/b": "h2"}"#).unwrap();
        let m = mapa_migrado(&propio, &viejo, b"PRUE");
        assert_eq!(m.get("/a").map(String::as_str), Some("h1"));
        assert!(propio.is_file() && !viejo.exists() && d.join("x.json.migrado").is_file());
        assert_eq!(leer_mapa(&propio, b"PRUE"), m, "lo propio dice lo mismo");
        // un viejo que reaparece no pisa lo propio
        std::fs::write(&viejo, r#"{"/a": "otro"}"#).unwrap();
        assert_eq!(mapa_migrado(&propio, &viejo, b"PRUE").get("/a").map(String::as_str), Some("h1"));
    }

    #[test]
    fn ida_y_vuelta_y_lo_danado_no_se_lee() {
        let mut e = Escritor::nuevo(b"PRUE", 1);
        e.texto("ñandú ⏰");
        e.i64(-5);
        e.opt_i64(None);
        e.opt_texto(Some("x"));
        e.textos(&["a".into(), "".into()]);
        e.bool(true);
        let b = e.bytes();
        let (mut l, v) = Lector::nuevo(&b, b"PRUE", 1).unwrap();
        assert_eq!(v, 1);
        assert_eq!(l.texto().unwrap(), "ñandú ⏰");
        assert_eq!(l.i64().unwrap(), -5);
        assert_eq!(l.opt_i64().unwrap(), None);
        assert_eq!(l.opt_texto().unwrap().as_deref(), Some("x"));
        assert_eq!(l.textos().unwrap(), ["a", ""]);
        assert!(l.bool().unwrap() && l.fin());
        assert!(Lector::nuevo(&b, b"OTRO", 1).is_err(), "otro tipo");
        assert!(Lector::nuevo(&b[..12], b"PRUE", 1).and_then(|(mut l, _)| l.texto()).is_err(), "cortado");
        let mut nueva = b.clone();
        nueva[8] = 9;
        assert!(Lector::nuevo(&nueva, b"PRUE", 1).is_err(), "versión futura");
        assert_eq!(version_de(&nueva, b"PRUE"), Some(9));
    }

    #[test]
    fn el_arbol_de_valores_va_y_vuelve_igual() {
        let v = serde_json::json!({"id": "c0", "type": "function", "function": {"name": "read_file", "arguments": "{\"path\":\"a\"}"},
            "n": [0, -7, 18446744073709551615u64, 1.5, null, true, false, "ñ ⏰", [], {}]});
        let mut e = Escritor::nuevo(b"VALO", 1);
        e.valor(&v);
        e.opt_valor(None);
        let b = e.bytes();
        let (mut l, _) = Lector::nuevo(&b, b"VALO", 1).unwrap();
        assert_eq!(l.valor().unwrap(), v);
        assert_eq!(l.opt_valor().unwrap(), None);
        assert!(l.fin());
        // una etiqueta desconocida no se inventa nada
        let mut mala = Escritor::nuevo(b"VALO", 1);
        mala.u8(42);
        let m = mala.bytes();
        assert!(Lector::nuevo(&m, b"VALO", 1).unwrap().0.valor().is_err());
    }
}
