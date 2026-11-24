//! Lector de los `.pth` de PyTorch (zip + pickle) en Rust puro, SIN ejecutar el pickle: un intérprete
//! del subconjunto que escribe `torch.save` para un `state_dict` (diccionarios, tuplas, listas,
//! números, textos, `OrderedDict` y `_rebuild_tensor_v2` con su almacén persistente). Cualquier otra
//! clase o función del pickle queda como un valor opaco que no se llama nunca: un `.pth` ajeno no
//! puede ejecutar nada aquí. Los datos de cada tensor se leen del zip por desplazamiento.
//!
//! M7c: el VAE DAC de MOSS-SoundEffect viene así (`vae_128d_48k.pth`: `{"state_dict": …,
//! "metadata": {"kwargs": …}}`).

use anyhow::{anyhow, bail, Context, Result};
use gotv_engine::{anyhow_tr, bail_tr};
use mlx_rs::Array;
use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

/// Un valor del pickle.
#[derive(Clone, Debug)]
pub enum Valor {
    Nada,
    Bool(bool),
    Int(i64),
    Real(f64),
    Texto(String),
    Tupla(Vec<Valor>),
    Lista(Vec<Valor>),
    Dic(Vec<(Valor, Valor)>),
    /// `módulo nombre` de un GLOBAL: nunca se resuelve ni se llama
    Global(String),
    /// el id persistente (el almacén de un tensor)
    Persistente(Box<Valor>),
    /// un tensor por reconstruir: almacén, desplazamiento, forma y pasos
    Tensor { almacen: Box<Valor>, desde: usize, forma: Vec<usize>, pasos: Vec<usize> },
    /// cualquier otra llamada: queda opaca
    Opaco(Box<Valor>, Box<Valor>),
    Marca,
}

impl Valor {
    fn como_usizes(&self) -> Result<Vec<usize>> {
        match self {
            Valor::Tupla(v) | Valor::Lista(v) => v.iter().map(|x| match x {
                Valor::Int(i) if *i >= 0 => Ok(*i as usize),
                o => bail!("se esperaba un entero ≥ 0, no {o:?}"),
            }).collect(),
            o => bail_tr!("se esperaba una tupla de enteros, no {o:?}", "expected a tuple of integers, not {o:?}"),
        }
    }

    pub fn get(&self, clave: &str) -> Option<&Valor> {
        match self {
            Valor::Dic(v) => v.iter().find(|(k, _)| matches!(k, Valor::Texto(t) if t == clave)).map(|(_, v)| v),
            _ => None,
        }
    }

    /// A JSON (para los metadatos: `kwargs` del modelo). Los tensores y lo opaco quedan en `null`.
    pub fn json(&self) -> serde_json::Value {
        use serde_json::Value as J;
        match self {
            Valor::Nada | Valor::Marca | Valor::Global(_) | Valor::Persistente(_) | Valor::Tensor { .. } | Valor::Opaco(..) => J::Null,
            Valor::Bool(b) => J::Bool(*b),
            Valor::Int(i) => J::from(*i),
            Valor::Real(r) => serde_json::Number::from_f64(*r).map(J::Number).unwrap_or(J::Null),
            Valor::Texto(t) => J::String(t.clone()),
            Valor::Tupla(v) | Valor::Lista(v) => J::Array(v.iter().map(Valor::json).collect()),
            Valor::Dic(v) => J::Object(v.iter().filter_map(|(k, x)| if let Valor::Texto(k) = k { Some((k.clone(), x.json())) } else { None }).collect()),
        }
    }
}

/// Interpreta el pickle (protocolo 2, el de `torch.save`).
pub fn desempaquetar(b: &[u8]) -> Result<Valor> {
    let mut pila: Vec<Valor> = Vec::new();
    let mut memo: HashMap<u32, Valor> = HashMap::new();
    let mut i = 0usize;
    let leer = |i: &mut usize, n: usize| -> Result<&[u8]> {
        let s = b.get(*i..*i + n).ok_or_else(|| anyhow!("pickle corto"))?;
        *i += n;
        Ok(s)
    };
    let hasta_marca = |pila: &mut Vec<Valor>| -> Result<Vec<Valor>> {
        let m = pila.iter().rposition(|v| matches!(v, Valor::Marca)).ok_or_else(|| anyhow_tr!("pickle sin MARK", "pickle without MARK"))?;
        let v = pila.split_off(m + 1);
        pila.pop();
        Ok(v)
    };
    let sacar = |pila: &mut Vec<Valor>| pila.pop().ok_or_else(|| anyhow_tr!("pickle: pila vacía", "pickle: empty stack"));
    loop {
        let op = *b.get(i).ok_or_else(|| anyhow_tr!("pickle sin STOP", "pickle without STOP"))?;
        i += 1;
        match op {
            0x80 => {
                leer(&mut i, 1)?; // PROTO
            }
            b'.' => return sacar(&mut pila), // STOP
            b'}' => pila.push(Valor::Dic(Vec::new())),
            b']' => pila.push(Valor::Lista(Vec::new())),
            b')' => pila.push(Valor::Tupla(Vec::new())),
            b'(' => pila.push(Valor::Marca),
            b'N' => pila.push(Valor::Nada),
            0x88 => pila.push(Valor::Bool(true)),
            0x89 => pila.push(Valor::Bool(false)),
            b'K' => {
                let v = leer(&mut i, 1)?[0];
                pila.push(Valor::Int(v as i64));
            }
            b'M' => {
                let s = leer(&mut i, 2)?;
                pila.push(Valor::Int(u16::from_le_bytes([s[0], s[1]]) as i64));
            }
            b'J' => {
                let s = leer(&mut i, 4)?;
                pila.push(Valor::Int(i32::from_le_bytes([s[0], s[1], s[2], s[3]]) as i64));
            }
            0x8a => {
                // LONG1: entero de n bytes, complemento a dos
                let n = leer(&mut i, 1)?[0] as usize;
                let s = leer(&mut i, n)?;
                if n > 8 {
                    bail!("pickle: entero de {n} bytes");
                }
                let mut v: i64 = 0;
                for (k, x) in s.iter().enumerate() {
                    v |= (*x as i64) << (8 * k);
                }
                if n > 0 && n < 8 && s[n - 1] & 0x80 != 0 {
                    v -= 1i64 << (8 * n);
                }
                pila.push(Valor::Int(v));
            }
            b'G' => {
                let s = leer(&mut i, 8)?;
                pila.push(Valor::Real(f64::from_be_bytes(s.try_into()?)));
            }
            b'X' => {
                let s = leer(&mut i, 4)?;
                let n = u32::from_le_bytes([s[0], s[1], s[2], s[3]]) as usize;
                pila.push(Valor::Texto(String::from_utf8(leer(&mut i, n)?.to_vec())?));
            }
            0x8c => {
                let n = leer(&mut i, 1)?[0] as usize;
                pila.push(Valor::Texto(String::from_utf8(leer(&mut i, n)?.to_vec())?));
            }
            b'c' => {
                // GLOBAL «módulo\nnombre\n»: se guarda el nombre; NUNCA se importa
                let fin1 = b[i..].iter().position(|&x| x == b'\n').ok_or_else(|| anyhow!("GLOBAL roto"))?;
                let m = String::from_utf8(b[i..i + fin1].to_vec())?;
                i += fin1 + 1;
                let fin2 = b[i..].iter().position(|&x| x == b'\n').ok_or_else(|| anyhow!("GLOBAL roto"))?;
                let n = String::from_utf8(b[i..i + fin2].to_vec())?;
                i += fin2 + 1;
                pila.push(Valor::Global(format!("{m} {n}")));
            }
            b'q' => {
                let k = leer(&mut i, 1)?[0] as u32;
                memo.insert(k, pila.last().cloned().ok_or_else(|| anyhow_tr!("BINPUT sin valor", "BINPUT without a value"))?);
            }
            b'r' => {
                let s = leer(&mut i, 4)?;
                memo.insert(u32::from_le_bytes([s[0], s[1], s[2], s[3]]), pila.last().cloned().ok_or_else(|| anyhow_tr!("BINPUT sin valor", "BINPUT without a value"))?);
            }
            b'h' => {
                let k = leer(&mut i, 1)?[0] as u32;
                pila.push(memo.get(&k).cloned().ok_or_else(|| anyhow_tr!("BINGET {k} sin memo", "BINGET {k} without memo"))?);
            }
            b'j' => {
                let s = leer(&mut i, 4)?;
                let k = u32::from_le_bytes([s[0], s[1], s[2], s[3]]);
                pila.push(memo.get(&k).cloned().ok_or_else(|| anyhow_tr!("BINGET {k} sin memo", "BINGET {k} without memo"))?);
            }
            b't' => {
                let v = hasta_marca(&mut pila)?;
                pila.push(Valor::Tupla(v));
            }
            0x85 => {
                let a = sacar(&mut pila)?;
                pila.push(Valor::Tupla(vec![a]));
            }
            0x86 => {
                let b2 = sacar(&mut pila)?;
                let a = sacar(&mut pila)?;
                pila.push(Valor::Tupla(vec![a, b2]));
            }
            0x87 => {
                let c = sacar(&mut pila)?;
                let b2 = sacar(&mut pila)?;
                let a = sacar(&mut pila)?;
                pila.push(Valor::Tupla(vec![a, b2, c]));
            }
            b'a' => {
                let v = sacar(&mut pila)?;
                match pila.last_mut() {
                    Some(Valor::Lista(l)) => l.push(v),
                    _ => bail_tr!("APPEND fuera de una lista", "APPEND outside a list"),
                }
            }
            b'e' => {
                let v = hasta_marca(&mut pila)?;
                match pila.last_mut() {
                    Some(Valor::Lista(l)) => l.extend(v),
                    _ => bail_tr!("APPENDS fuera de una lista", "APPENDS outside a list"),
                }
            }
            b's' => {
                let v = sacar(&mut pila)?;
                let k = sacar(&mut pila)?;
                match pila.last_mut() {
                    Some(Valor::Dic(d)) => d.push((k, v)),
                    _ => bail!("SETITEM fuera de un diccionario"),
                }
            }
            b'u' => {
                let v = hasta_marca(&mut pila)?;
                match pila.last_mut() {
                    Some(Valor::Dic(d)) => {
                        let mut it = v.into_iter();
                        while let (Some(k), Some(x)) = (it.next(), it.next()) {
                            d.push((k, x));
                        }
                    }
                    _ => bail!("SETITEMS fuera de un diccionario"),
                }
            }
            b'Q' => {
                let id = sacar(&mut pila)?;
                pila.push(Valor::Persistente(Box::new(id)));
            }
            b'R' => {
                // REDUCE: sólo DOS llamadas se interpretan (sin ejecutar nada); el resto, opaco
                let args = sacar(&mut pila)?;
                let f = sacar(&mut pila)?;
                let v = match (&f, &args) {
                    (Valor::Global(g), _) if g == "collections OrderedDict" => Valor::Dic(Vec::new()),
                    (Valor::Global(g), Valor::Tupla(a)) if g == "torch._utils _rebuild_tensor_v2" && a.len() >= 4 => match &a[0] {
                        Valor::Persistente(id) => Valor::Tensor {
                            almacen: id.clone(),
                            desde: match a[1] {
                                Valor::Int(x) if x >= 0 => x as usize,
                                _ => bail_tr!("tensor con desplazamiento raro", "tensor with an odd offset"),
                            },
                            forma: a[2].como_usizes()?,
                            pasos: a[3].como_usizes()?,
                        },
                        o => bail_tr!("tensor sin almacén persistente: {o:?}", "tensor without a persistent storage: {o:?}"),
                    },
                    _ => Valor::Opaco(Box::new(f), Box::new(args)),
                };
                pila.push(v);
            }
            b'b' => {
                // BUILD: el estado de un objeto (p. ej. `_metadata` del OrderedDict): se descarta
                sacar(&mut pila)?;
            }
            x => bail_tr!("pickle: código 0x{x:02x} no admitido (el lector sólo entiende state_dicts)", "pickle: opcode 0x{x:02x} not supported (the reader only understands state_dicts)"),
        }
    }
}

/// Las entradas de un zip (nombre → (desplazamiento de los datos, tamaño)), leyendo SÓLO el
/// directorio central y los encabezados locales.
fn entradas_zip(f: &mut std::fs::File) -> Result<HashMap<String, (u64, u64)>> {
    let largo = f.seek(SeekFrom::End(0))?;
    let cola = largo.min(66_000);
    f.seek(SeekFrom::Start(largo - cola))?;
    let mut b = vec![0u8; cola as usize];
    f.read_exact(&mut b)?;
    let fin = (0..b.len().saturating_sub(21)).rev().find(|&i| b[i..i + 4] == [0x50, 0x4b, 0x05, 0x06]).ok_or_else(|| anyhow!("no es un zip"))?;
    let le16 = |s: &[u8], o: usize| u16::from_le_bytes([s[o], s[o + 1]]) as u64;
    let le32 = |s: &[u8], o: usize| u32::from_le_bytes([s[o], s[o + 1], s[o + 2], s[o + 3]]) as u64;
    let (n, tam_cd, ini_cd) = (le16(&b, fin + 10), le32(&b, fin + 12), le32(&b, fin + 16));
    if ini_cd == 0xFFFF_FFFF || n == 0xFFFF {
        bail_tr!("zip64 aún no admitido", "zip64 not supported yet");
    }
    f.seek(SeekFrom::Start(ini_cd))?;
    let mut cd = vec![0u8; tam_cd as usize];
    f.read_exact(&mut cd)?;
    let mut m = HashMap::new();
    let mut c = 0usize;
    for _ in 0..n {
        if cd.get(c..c + 4) != Some(&[0x50, 0x4b, 0x01, 0x02]) {
            bail_tr!("directorio central dañado", "damaged central directory");
        }
        let (metodo, tam, nl, el, cl, local) = (le16(&cd, c + 10), le32(&cd, c + 20), le16(&cd, c + 28) as usize, le16(&cd, c + 30) as usize, le16(&cd, c + 32) as usize, le32(&cd, c + 42));
        let nombre = String::from_utf8_lossy(&cd[c + 46..c + 46 + nl]).to_string();
        if metodo != 0 {
            bail_tr!("{nombre}: comprimido (método {metodo}); torch.save no comprime", "{nombre}: compressed (method {metodo}); torch.save does not compress");
        }
        // torch escribe con «descriptor de datos»: el tamaño fiable es el del directorio central
        let mut h = [0u8; 30];
        f.seek(SeekFrom::Start(local))?;
        f.read_exact(&mut h)?;
        m.insert(nombre, (local + 30 + le16(&h, 26) + le16(&h, 28), tam));
        c += 46 + nl + el + cl;
    }
    Ok(m)
}

/// Carga un `.pth` de `torch.save`: los tensores de `state_dict` (o del diccionario raíz si no lo
/// hay) como arrays f32, y el valor raíz entero (para leer `metadata`).
pub fn cargar(ruta: &Path) -> Result<(HashMap<String, Array>, Valor)> {
    let mut f = std::fs::File::open(ruta).with_context(|| format!("abrir {}", ruta.display()))?;
    let zip = entradas_zip(&mut f).with_context(|| ruta.display().to_string())?;
    let pkl = zip.keys().find(|k| k.ends_with("/data.pkl") || k.as_str() == "data.pkl").cloned().ok_or_else(|| anyhow_tr!("{}: sin data.pkl", "{}: no data.pkl", ruta.display()))?;
    let prefijo = pkl.trim_end_matches("data.pkl").to_string();
    let leer = |f: &mut std::fs::File, (o, n): (u64, u64)| -> Result<Vec<u8>> {
        f.seek(SeekFrom::Start(o))?;
        let mut b = vec![0u8; n as usize];
        f.read_exact(&mut b)?;
        Ok(b)
    };
    let raiz = desempaquetar(&leer(&mut f, zip[&pkl])?)?;
    let sd = raiz.get("state_dict").unwrap_or(&raiz);
    let Valor::Dic(pares) = sd else { bail_tr!("{}: el pickle no es un state_dict", "{}: the pickle is not a state_dict", ruta.display()) };
    let mut out = HashMap::new();
    for (k, v) in pares {
        let (Valor::Texto(k), Valor::Tensor { almacen, desde, forma, pasos }) = (k, v) else { continue };
        let Valor::Tupla(id) = almacen.as_ref() else { bail_tr!("{k}: almacén raro", "{k}: odd storage") };
        let (Some(Valor::Global(tipo)), Some(Valor::Texto(clave))) = (id.get(1), id.get(2)) else { bail_tr!("{k}: almacén sin tipo o clave", "{k}: storage without type or key") };
        if tipo != "torch FloatStorage" {
            bail_tr!("{k}: almacén {tipo} (el lector sólo lee f32)", "{k}: {tipo} storage (the reader only reads f32)");
        }
        // sólo contiguos (los de un state_dict lo son)
        let mut esperado = 1usize;
        for (d, p) in forma.iter().zip(pasos).rev() {
            if *d > 1 && *p != esperado {
                bail!("{k}: tensor no contiguo (forma {forma:?}, pasos {pasos:?})");
            }
            esperado *= d;
        }
        let n: usize = forma.iter().product();
        let datos = zip.get(&format!("{prefijo}data/{clave}")).copied().ok_or_else(|| anyhow!("{k}: falta data/{clave}"))?;
        let b = leer(&mut f, (datos.0 + 4 * *desde as u64, 4 * n as u64))?;
        let v: Vec<f32> = b.as_chunks::<4>().0.iter().map(|x| f32::from_le_bytes(*x)).collect();
        let forma_i: Vec<i32> = forma.iter().map(|&d| d as i32).collect();
        out.insert(k.clone(), Array::from_slice(&v, &forma_i));
    }
    Ok((out, raiz))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interpreta_un_state_dict_y_no_ejecuta_nada() {
        // {"state_dict": OrderedDict(w = tensor f32 [2,3] en el almacén "0"), "metadata": {"kwargs": {"a": [1, 2], "b": True}}}
        // + un REDUCE de os.system que debe quedar OPACO
        let mut p: Vec<u8> = vec![0x80, 2, b'}', b'('];
        let txt = |p: &mut Vec<u8>, s: &str| {
            p.push(b'X');
            p.extend((s.len() as u32).to_le_bytes());
            p.extend(s.as_bytes());
        };
        txt(&mut p, "state_dict");
        p.extend(b"ccollections\nOrderedDict\n)R(");
        txt(&mut p, "w");
        p.extend(b"ctorch._utils\n_rebuild_tensor_v2\n((");
        txt(&mut p, "storage");
        p.extend(b"ctorch\nFloatStorage\n");
        txt(&mut p, "0");
        txt(&mut p, "cpu");
        p.extend([b'K', 6, b't', b'Q', b'K', 0, b'K', 2, b'K', 3, 0x86, b'K', 3, b'K', 1, 0x86, 0x89, b'}', b't', b'R', b'u']);
        txt(&mut p, "metadata");
        p.push(b'}');
        txt(&mut p, "kwargs");
        p.extend(*b"}(");
        txt(&mut p, "a");
        p.extend([b']', b'(', b'K', 1, b'K', 2, b'e']);
        txt(&mut p, "b");
        p.extend([0x88, b'u', b's']);
        txt(&mut p, "malo");
        p.extend(b"cos\nsystem\n(");
        txt(&mut p, "rm -rf /");
        p.extend(*b"tRu.");
        let v = desempaquetar(&p).unwrap();
        let sd = v.get("state_dict").unwrap();
        match sd.get("w").unwrap() {
            Valor::Tensor { forma, pasos, desde, .. } => {
                assert_eq!((forma.as_slice(), pasos.as_slice(), *desde), (&[2usize, 3][..], &[3usize, 1][..], 0));
            }
            o => panic!("{o:?}"),
        }
        assert_eq!(v.get("metadata").unwrap().json(), serde_json::json!({"kwargs": {"a": [1, 2], "b": true}}));
        assert!(matches!(v.get("malo"), Some(Valor::Opaco(..))), "os.system queda opaco, jamás se llama");
    }
}
