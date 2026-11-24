//! F4 · El protocolo de mensajes de Jupyter (v5.3) sobre ZeroMQ: el formato de cable y su FIRMA.
//!
//! Un mensaje viaja como varias tramas: `[identidades…] <IDS|MSG> firma cabecera padre metadatos contenido
//! [búferes…]`. La firma es HMAC-SHA256 (hex) de las cuatro partes JSON con la clave del archivo de conexión.
//! Trampa 3 de la ficha: sin firma (o con una mala) el kernel IGNORA el mensaje en silencio; y lo que llega
//! con una firma mala tampoco se cree aquí.

use anyhow::{bail, Result};
use bytes::Bytes;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

/// El separador entre las identidades de ZeroMQ y el mensaje.
pub const DELIM: &[u8] = b"<IDS|MSG>";
pub const VERSION: &str = "5.3";

/// HMAC-SHA256 (RFC 2104) sobre varias partes seguidas.
pub fn hmac_sha256(clave: &[u8], partes: &[&[u8]]) -> [u8; 32] {
    let mut k = [0u8; 64];
    if clave.len() > 64 {
        k[..32].copy_from_slice(&Sha256::digest(clave));
    } else {
        k[..clave.len()].copy_from_slice(clave);
    }
    let mut dentro = Sha256::new();
    dentro.update(k.map(|b| b ^ 0x36));
    for p in partes {
        dentro.update(p);
    }
    let h = dentro.finalize();
    let mut fuera = Sha256::new();
    fuera.update(k.map(|b| b ^ 0x5c));
    fuera.update(h);
    fuera.finalize().into()
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// La firma de una sesión (la clave del archivo de conexión; vacía = sin firma, como permite Jupyter).
#[derive(Clone)]
pub struct Firma {
    clave: Vec<u8>,
}

impl Firma {
    pub fn new(clave: &str) -> Firma {
        Firma { clave: clave.as_bytes().to_vec() }
    }

    pub fn firmar(&self, partes: &[&[u8]]) -> String {
        if self.clave.is_empty() {
            return String::new();
        }
        hex(&hmac_sha256(&self.clave, partes))
    }

    /// Comparación en tiempo constante (no se filtra cuánto de la firma acertó).
    pub fn valida(&self, firma: &[u8], partes: &[&[u8]]) -> bool {
        let buena = self.firmar(partes);
        buena.len() == firma.len() && buena.bytes().zip(firma).fold(0u8, |a, (x, y)| a | (x ^ y)) == 0
    }
}

/// Un mensaje de Jupyter.
#[derive(Clone, Debug)]
pub struct Mensaje {
    pub ids: Vec<Bytes>,
    pub header: Value,
    pub parent: Value,
    pub metadata: Value,
    pub content: Value,
    pub buffers: Vec<Bytes>,
}

/// Un id de mensaje (32 hex, como el `uuid4().hex` de Jupyter).
pub fn id_nuevo() -> String {
    let mut b = [0u8; 16];
    super::formato::aleatorio(&mut b);
    hex(&b)
}

impl Mensaje {
    /// Un mensaje nuevo de esta sesión.
    pub fn nuevo(tipo: &str, sesion: &str, content: Value) -> Mensaje {
        Mensaje {
            ids: vec![],
            header: json!({
                "msg_id": id_nuevo(),
                "session": sesion,
                "username": "gotv",
                "date": chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Micros, true),
                "msg_type": tipo,
                "version": VERSION,
            }),
            parent: json!({}),
            metadata: json!({}),
            content,
            buffers: vec![],
        }
    }

    /// La respuesta a `padre` (para el kernel falso de las pruebas: lleva su cabecera como padre y sus ids).
    pub fn respuesta(tipo: &str, sesion: &str, padre: &Mensaje, content: Value) -> Mensaje {
        let mut m = Mensaje::nuevo(tipo, sesion, content);
        m.parent = padre.header.clone();
        m.ids = padre.ids.clone();
        m
    }

    pub fn tipo(&self) -> &str {
        self.header["msg_type"].as_str().unwrap_or("")
    }

    pub fn id(&self) -> &str {
        self.header["msg_id"].as_str().unwrap_or("")
    }

    /// El id del mensaje al que responde.
    pub fn padre(&self) -> &str {
        self.parent["msg_id"].as_str().unwrap_or("")
    }

    /// Las tramas para ZeroMQ, firmadas.
    pub fn tramas(&self, f: &Firma) -> Vec<Bytes> {
        let partes = [&self.header, &self.parent, &self.metadata, &self.content].map(|v| v.to_string().into_bytes());
        let firma = f.firmar(&partes.iter().map(|p| p.as_slice()).collect::<Vec<_>>());
        let mut t: Vec<Bytes> = self.ids.clone();
        t.push(Bytes::from_static(DELIM));
        t.push(Bytes::from(firma));
        t.extend(partes.into_iter().map(Bytes::from));
        t.extend(self.buffers.iter().cloned());
        t
    }

    /// Lee las tramas de un mensaje recibido; una firma que no casa es un error (y el mensaje no se cree).
    pub fn de_tramas(t: Vec<Bytes>, f: &Firma) -> Result<Mensaje> {
        let Some(k) = t.iter().position(|x| x.as_ref() == DELIM) else {
            bail!("{}", crate::i18n::t("mensaje de Jupyter sin separador <IDS|MSG>", "Jupyter message without <IDS|MSG> delimiter"));
        };
        if t.len() < k + 6 {
            bail!("{}", crate::i18n::t("mensaje de Jupyter incompleto", "incomplete Jupyter message"));
        }
        let partes: Vec<&[u8]> = t[k + 2..k + 6].iter().map(|x| x.as_ref()).collect();
        if !f.valida(&t[k + 1], &partes) {
            bail!("{}", crate::i18n::t("mensaje de Jupyter con la firma mala: descartado", "Jupyter message with a bad signature: discarded"));
        }
        let json = |i: usize| serde_json::from_slice::<Value>(&t[k + 2 + i]).unwrap_or(Value::Null);
        Ok(Mensaje {
            ids: t[..k].to_vec(),
            header: json(0),
            parent: json(1),
            metadata: json(2),
            content: json(3),
            buffers: t[k + 6..].to_vec(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hmac_como_el_rfc_4231() {
        // RFC 4231, caso 2: clave «Jefe»
        let h = hmac_sha256(b"Jefe", &[b"what do ya want ", b"for nothing?"]);
        assert_eq!(hex(&h), "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843");
        // caso 6: una clave más larga que el bloque se resume primero
        let h = hmac_sha256(&[0xaa; 131], &[b"Test Using Larger Than Block-Size Key - Hash Key First"]);
        assert_eq!(hex(&h), "60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54");
    }

    #[test]
    fn ida_y_vuelta_firmada_y_la_firma_mala_se_descarta() {
        let f = Firma::new("a0436f6c-1916-498b-8eb9-e81ab9368e84");
        let mut m = Mensaje::nuevo("execute_request", "s1", json!({"code": "1+1"}));
        m.ids = vec![Bytes::from_static(b"cliente")];
        let t = m.tramas(&f);
        assert_eq!(t[1].as_ref(), DELIM);
        assert_eq!(t[2].len(), 64, "la firma es hex de 32 bytes");
        let r = Mensaje::de_tramas(t.clone(), &f).unwrap();
        assert_eq!((r.tipo(), r.content["code"].as_str(), r.ids.len()), ("execute_request", Some("1+1"), 1));
        // otra clave: no se cree
        assert!(Mensaje::de_tramas(t.clone(), &Firma::new("otra")).is_err());
        // contenido cambiado con la firma vieja: tampoco
        let mut t2 = t;
        t2[6] = Bytes::from_static(b"{\"code\":\"rm -rf\"}");
        assert!(Mensaje::de_tramas(t2, &f).is_err());
        // sin clave no hay firma (Jupyter lo permite)
        let sin = Firma::new("");
        assert!(Mensaje::nuevo("x", "s", json!({})).tramas(&sin)[1].is_empty());
    }
}
