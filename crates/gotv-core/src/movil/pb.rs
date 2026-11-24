//! F18 · Protobuf a mano: el formato de cable (varint, 64 bits, con largo) para los POCOS mensajes que GOTV
//! cruza con `idb_companion` y con el emulador de Android.
//!
//! Por qué no `prost` + `tonic-build`: generar el cliente obliga a un `build.rs` en `gotv-core` (todo GOTV
//! compilaría `protox`, `logos`, `petgraph`… en cada compilación limpia) para usar ~15 mensajes de los cientos
//! que trae `idb.proto`. El formato de cable de protobuf es estable y corto; los números de campo se toman de
//! los `.proto` copiados en `crates/gotv-core/proto/` y una prueba (`idb::tests::campos_casan_con_el_proto`)
//! comprueba cada uno contra el archivo, para que no se desvíen.
use anyhow::{bail, Result};

fn poner_varint(b: &mut Vec<u8>, mut v: u64) {
    loop {
        let x = (v & 0x7f) as u8;
        v >>= 7;
        if v == 0 {
            b.push(x);
            return;
        }
        b.push(x | 0x80);
    }
}

/// Un mensaje en construcción (los campos se escriben en el orden en que se llaman).
#[derive(Default, Clone, Debug, PartialEq)]
pub struct Msg(pub Vec<u8>);

impl Msg {
    pub fn new() -> Self {
        Self::default()
    }
    fn clave(&mut self, campo: u32, tipo: u8) {
        poner_varint(&mut self.0, ((campo as u64) << 3) | tipo as u64);
    }
    /// Enteros sin signo, enumeraciones y booleanos (tipo 0). Se escribe aunque valga 0: en un `oneof`
    /// el 0 también dice «este es el caso».
    pub fn entero(mut self, campo: u32, v: u64) -> Self {
        self.clave(campo, 0);
        poner_varint(&mut self.0, v);
        self
    }
    pub fn booleano(self, campo: u32, v: bool) -> Self {
        self.entero(campo, v as u64)
    }
    /// `double` (tipo 1, 8 bytes little-endian).
    pub fn doble(mut self, campo: u32, v: f64) -> Self {
        self.clave(campo, 1);
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    /// `bytes`, `string` y mensajes anidados (tipo 2, con su largo delante).
    pub fn bytes(mut self, campo: u32, v: &[u8]) -> Self {
        self.clave(campo, 2);
        poner_varint(&mut self.0, v.len() as u64);
        self.0.extend_from_slice(v);
        self
    }
    pub fn texto(self, campo: u32, v: &str) -> Self {
        self.bytes(campo, v.as_bytes())
    }
    pub fn msg(self, campo: u32, m: Msg) -> Self {
        self.bytes(campo, &m.0)
    }
    pub fn into_bytes(self) -> Vec<u8> {
        self.0
    }
}

/// Un valor leído del cable.
#[derive(Debug, Clone, PartialEq)]
pub enum Valor {
    Entero(u64),
    Fijo64(u64),
    Bytes(Vec<u8>),
    Fijo32(u32),
}

/// Un mensaje leído: sus campos en orden (un campo repetido aparece varias veces).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Leido(pub Vec<(u32, Valor)>);

fn leer_varint(b: &[u8], i: &mut usize) -> Result<u64> {
    let mut v = 0u64;
    for k in 0..10 {
        let Some(&x) = b.get(*i) else { bail!("{}", crate::i18n::t("protobuf: varint cortado", "protobuf: truncated varint")) };
        *i += 1;
        v |= ((x & 0x7f) as u64) << (7 * k);
        if x & 0x80 == 0 {
            return Ok(v);
        }
    }
    bail!("{}", crate::i18n::t("protobuf: varint de más de 10 bytes", "protobuf: varint longer than 10 bytes"))
}

/// Lee un mensaje entero. Un campo desconocido no estorba (se guarda con su número, como hace protobuf).
pub fn leer(b: &[u8]) -> Result<Leido> {
    let mut i = 0;
    let mut out = Vec::new();
    while i < b.len() {
        let clave = leer_varint(b, &mut i)?;
        let (campo, tipo) = ((clave >> 3) as u32, (clave & 7) as u8);
        let v = match tipo {
            0 => Valor::Entero(leer_varint(b, &mut i)?),
            1 => {
                let Some(s) = b.get(i..i + 8) else { bail!("{}", crate::i18n::t("protobuf: campo de 64 bits cortado", "protobuf: truncated 64-bit field")) };
                i += 8;
                Valor::Fijo64(u64::from_le_bytes(s.try_into().unwrap_or_default()))
            }
            2 => {
                let n = leer_varint(b, &mut i)? as usize;
                let Some(s) = i.checked_add(n).and_then(|f| b.get(i..f)) else { bail!("{}", crate::i18n::t("protobuf: campo con largo cortado", "protobuf: truncated length-delimited field")) };
                i += n;
                Valor::Bytes(s.to_vec())
            }
            5 => {
                let Some(s) = b.get(i..i + 4) else { bail!("{}", crate::i18n::t("protobuf: campo de 32 bits cortado", "protobuf: truncated 32-bit field")) };
                i += 4;
                Valor::Fijo32(u32::from_le_bytes(s.try_into().unwrap_or_default()))
            }
            t => bail!("{}", crate::tr!("protobuf: tipo de cable {t} no admitido (campo {campo})", "protobuf: unsupported wire type {t} (field {campo})")),
        };
        out.push((campo, v));
    }
    Ok(Leido(out))
}

impl Leido {
    fn ultimo(&self, campo: u32) -> Option<&Valor> {
        self.0.iter().rev().find(|(c, _)| *c == campo).map(|(_, v)| v)
    }
    pub fn entero(&self, campo: u32) -> Option<u64> {
        match self.ultimo(campo)? {
            Valor::Entero(v) => Some(*v),
            _ => None,
        }
    }
    pub fn doble(&self, campo: u32) -> Option<f64> {
        match self.ultimo(campo)? {
            Valor::Fijo64(v) => Some(f64::from_bits(*v)),
            _ => None,
        }
    }
    pub fn bytes(&self, campo: u32) -> Option<&[u8]> {
        match self.ultimo(campo)? {
            Valor::Bytes(v) => Some(v),
            _ => None,
        }
    }
    pub fn texto(&self, campo: u32) -> Option<String> {
        self.bytes(campo).map(|b| String::from_utf8_lossy(b).into_owned())
    }
    pub fn msg(&self, campo: u32) -> Option<Leido> {
        leer(self.bytes(campo)?).ok()
    }
    /// Todas las apariciones de un campo repetido de mensajes.
    pub fn msgs(&self, campo: u32) -> Vec<Leido> {
        self.0.iter().filter(|(c, _)| *c == campo).filter_map(|(_, v)| if let Valor::Bytes(b) = v { leer(b).ok() } else { None }).collect()
    }
    pub fn tiene(&self, campo: u32) -> bool {
        self.ultimo(campo).is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Bytes conocidos del formato de cable (los ejemplos de la documentación de protobuf: 150 en el
    /// campo 1 = 08 96 01; «testing» en el campo 2 = 12 07 74 65 73 74 69 6e 67).
    #[test]
    fn el_cable_casa_con_los_ejemplos_de_protobuf() {
        assert_eq!(Msg::new().entero(1, 150).into_bytes(), vec![0x08, 0x96, 0x01]);
        assert_eq!(Msg::new().texto(2, "testing").into_bytes(), vec![0x12, 0x07, b't', b'e', b's', b't', b'i', b'n', b'g']);
        // un mensaje anidado (campo 3 con el mensaje {1: 150}) = 1a 03 08 96 01
        assert_eq!(Msg::new().msg(3, Msg::new().entero(1, 150)).into_bytes(), vec![0x1a, 0x03, 0x08, 0x96, 0x01]);
        // double 1.5 en el campo 1 = 09 + 8 bytes little-endian
        assert_eq!(Msg::new().doble(1, 1.5).into_bytes(), [vec![0x09], 1.5f64.to_le_bytes().to_vec()].concat());
    }

    #[test]
    fn ida_y_vuelta_con_utf8_y_campos_repetidos() {
        let b = Msg::new().texto(1, "mañana ñandú 日本").doble(2, 201.5).entero(3, u64::MAX).msg(4, Msg::new().entero(1, 7)).msg(4, Msg::new().entero(1, 8)).into_bytes();
        let l = leer(&b).unwrap();
        assert_eq!(l.texto(1).unwrap(), "mañana ñandú 日本");
        assert_eq!(l.doble(2), Some(201.5));
        assert_eq!(l.entero(3), Some(u64::MAX));
        assert_eq!(l.msgs(4).iter().map(|m| m.entero(1).unwrap()).collect::<Vec<_>>(), vec![7, 8]);
        assert!(!l.tiene(9));
    }

    #[test]
    fn lo_cortado_es_error_y_no_panico() {
        assert!(leer(&[0x08]).is_err());
        assert!(leer(&[0x12, 0x05, b'a']).is_err());
        assert!(leer(&[0x09, 0, 0]).is_err());
        assert!(leer(&[0x0b]).is_err(), "tipo 3 (grupos) no se admite");
        assert!(leer(&[0xff; 11]).is_err());
    }
}
