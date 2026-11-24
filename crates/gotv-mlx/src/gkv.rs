//! `GKV2`: la caché de la conversación en disco, comprimida SIN PÉRDIDA y en Rust puro (F2 del plano
//! de la memoria rápida). Sólo bytes: nada de MLX aquí, así se prueba sin modelo.
//!
//! Medido 2026-11-11 sobre la caché real del Portador (K/V en BF16): el byte BAJO de cada número es
//! ruido (entropía 7,97 de 8 bits: la mantisa) y el byte ALTO (signo + exponente) toma ~30 valores con
//! 2,81 bits de entropía. Separarlos y pasar el alto por Huffman da 66–67 % del tamaño, que es el techo
//! sin pérdida (diferencias con el token o la dimensión vecina salieron PEORES: 68–71 %). zstd directo
//! daba 79 %; esto lo iguala sin meter una librería en C.
//!
//! ```text
//! "GKV2" · u64 LE n · cabecera en el formato propio (códec GKVC, n bytes) · datos de los trozos, uno tras otro
//! JSON = {"tensores":[{"nombre","dtype","forma","codec":"h16"|"crudo",
//!                      "trozos":[{"pos","largo","crudos","alto":"huff"|"crudo"}]}]}
//! trozo h16 = [bytes bajos, crudos/2] [byte alto: Huffman (256 largos · u64 bits · flujo) o crudo]
//! ```

use anyhow::{anyhow, bail, Context, Result};
use gotv_engine::{anyhow_tr, bail_tr};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// GKV2 (2026-11-17): la cabecera va en el formato propio (códec, tipo GKVC), ya no en JSON. Un GKV1 viejo
/// se rechaza como «no es un archivo GKV» y la memoria rápida lo trata como caché vacía.
const MAGIA: &[u8; 4] = b"GKV2";

fn cabecera_bytes(c: &Cabecera) -> Result<Vec<u8>> {
    let mut e = gotv_engine::codec::Escritor::nuevo(b"GKVC", 1);
    e.valor(&serde_json::to_value(c)?);
    Ok(e.bytes())
}

fn cabecera_de(b: &[u8]) -> Result<Cabecera> {
    let (mut l, _) = gotv_engine::codec::Lector::nuevo(b, b"GKVC", 1)?;
    Ok(serde_json::from_value(l.valor()?)?)
}
/// Bytes crudos por trozo (par). Trozos independientes = compresión y lectura en paralelo.
const TROZO: usize = 8 << 20;
/// Largo máximo de un código de Huffman (la tabla de decodificación tiene 2^15 entradas).
const MAX_BITS: u32 = 15;
/// El Huffman tiene que ganar al menos esto o el byte alto va crudo.
const GANANCIA_MIN: f64 = 0.03;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
struct TrozoMeta {
    pos: u64,
    largo: u64,
    crudos: u64,
    alto: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
struct TensorMeta {
    nombre: String,
    dtype: String,
    forma: Vec<i64>,
    codec: String,
    trozos: Vec<TrozoMeta>,
}

#[derive(Serialize, Deserialize, Debug)]
struct Cabecera {
    tensores: Vec<TensorMeta>,
}

/// Un tensor para escribir: sus bytes tal como están en memoria (little endian).
pub struct Tensor<'a> {
    pub nombre: String,
    pub dtype: String,
    pub forma: Vec<i64>,
    pub datos: &'a [u8],
}

/// Un tensor leído.
#[derive(Debug, Clone, PartialEq)]
pub struct Leido {
    pub nombre: String,
    pub dtype: String,
    pub forma: Vec<i64>,
    pub datos: Vec<u8>,
}

/// Bytes por elemento de un dtype de safetensors.
pub fn bytes_dtype(d: &str) -> Option<usize> {
    Some(match d {
        "BOOL" | "U8" | "I8" => 1,
        "BF16" | "F16" | "U16" | "I16" => 2,
        "F32" | "U32" | "I32" => 4,
        "F64" | "U64" | "I64" => 8,
        _ => return None,
    })
}

fn elementos(forma: &[i64]) -> Option<usize> {
    forma.iter().try_fold(1usize, |a, &d| a.checked_mul(usize::try_from(d).ok()?))
}

// ── Huffman canónico ────────────────────────────────────────────────────────────────────────

/// Largos de código (0 = el símbolo no aparece), limitados a `MAX_BITS`.
fn largos(frec: &[u64; 256]) -> [u8; 256] {
    let mut l = [0u8; 256];
    let usados: Vec<usize> = (0..256).filter(|&s| frec[s] > 0).collect();
    match usados.len() {
        0 => return l,
        1 => {
            l[usados[0]] = 1;
            return l;
        }
        _ => {}
    }
    // árbol de Huffman con un montículo: nodos 0..256 hojas, después los internos
    use std::cmp::Reverse;
    use std::collections::BinaryHeap;
    let mut padre: Vec<usize> = vec![usize::MAX; 512];
    let mut monticulo: BinaryHeap<Reverse<(u64, usize)>> = usados.iter().map(|&s| Reverse((frec[s], s))).collect();
    let mut siguiente = 256;
    while monticulo.len() > 1 {
        let Reverse((fa, a)) = monticulo.pop().unwrap();
        let Reverse((fb, b)) = monticulo.pop().unwrap();
        padre[a] = siguiente;
        padre[b] = siguiente;
        monticulo.push(Reverse((fa + fb, siguiente)));
        siguiente += 1;
    }
    for &s in &usados {
        let (mut d, mut n) = (0u32, s);
        while padre[n] != usize::MAX {
            n = padre[n];
            d += 1;
        }
        l[s] = d.min(255) as u8;
    }
    // límite de largo: los que pasan se recortan y se reparan las cuentas de Kraft alargando los
    // más largos que aún caben (con ~30 símbolos casi nunca hace falta)
    if l.iter().any(|&x| x as u32 > MAX_BITS) {
        let tope = 1u64 << MAX_BITS;
        for x in l.iter_mut().filter(|x| **x as u32 > MAX_BITS) {
            *x = MAX_BITS as u8;
        }
        let mut kraft: u64 = l.iter().filter(|&&x| x > 0).map(|&x| 1u64 << (MAX_BITS - x as u32)).sum();
        while kraft > tope {
            let s = (0..256).filter(|&s| l[s] > 0 && (l[s] as u32) < MAX_BITS).max_by_key(|&s| (l[s], frec[s] == 0, std::cmp::Reverse(frec[s]))).expect("Kraft siempre se puede reparar");
            kraft -= 1u64 << (MAX_BITS - l[s] as u32 - 1);
            l[s] += 1;
        }
    }
    l
}

/// Códigos canónicos a partir de los largos (orden: largo, luego símbolo).
fn codigos(l: &[u8; 256]) -> [u32; 256] {
    let mut orden: Vec<usize> = (0..256).filter(|&s| l[s] > 0).collect();
    orden.sort_by_key(|&s| (l[s], s));
    let mut c = [0u32; 256];
    let (mut codigo, mut previo) = (0u32, 0u8);
    for (i, &s) in orden.iter().enumerate() {
        if i > 0 {
            codigo += 1;
        }
        codigo <<= l[s] - previo;
        previo = l[s];
        c[s] = codigo;
    }
    c
}

/// `[256 largos][u64 bits][flujo]`. `None` si no hay nada que codificar.
fn huff_codificar(datos: &[u8]) -> Option<Vec<u8>> {
    if datos.is_empty() {
        return None;
    }
    let mut frec = [0u64; 256];
    for &b in datos {
        frec[b as usize] += 1;
    }
    let l = largos(&frec);
    let c = codigos(&l);
    let bits: u64 = (0..256).map(|s| frec[s] * l[s] as u64).sum();
    let mut out = Vec::with_capacity(256 + 8 + bits.div_ceil(8) as usize);
    out.extend_from_slice(&l);
    out.extend_from_slice(&bits.to_le_bytes());
    let (mut acc, mut n) = (0u64, 0u32);
    for &b in datos {
        let (cod, lar) = (c[b as usize] as u64, l[b as usize] as u32);
        acc = (acc << lar) | cod;
        n += lar;
        while n >= 8 {
            out.push((acc >> (n - 8)) as u8);
            n -= 8;
        }
    }
    if n > 0 {
        out.push((acc << (8 - n)) as u8);
    }
    Some(out)
}

/// Decodifica `n` símbolos de un bloque de `huff_codificar` en `salida` (uno de cada `paso` bytes
/// desde `salida[0]`: así se escribe el byte alto directamente en su lugar).
fn huff_decodificar(bloque: &[u8], n: usize, salida: &mut [u8], paso: usize) -> Result<()> {
    if bloque.len() < 264 {
        bail!("bloque de Huffman truncado");
    }
    let mut l = [0u8; 256];
    l.copy_from_slice(&bloque[..256]);
    if l.iter().any(|&x| x as u32 > MAX_BITS) {
        bail_tr!("largo de código imposible", "impossible code length");
    }
    let bits = u64::from_le_bytes(bloque[256..264].try_into().unwrap());
    let flujo = &bloque[264..];
    if bits.div_ceil(8) as usize > flujo.len() {
        bail!("flujo de Huffman truncado");
    }
    let c = codigos(&l);
    // tabla: los 15 bits siguientes → (símbolo, largo)
    let mut tabla = vec![(0u8, 0u8); 1 << MAX_BITS];
    for s in 0..256 {
        if l[s] == 0 {
            continue;
        }
        let corre = MAX_BITS - l[s] as u32;
        let inicio = (c[s] << corre) as usize;
        for e in &mut tabla[inicio..inicio + (1usize << corre)] {
            *e = (s as u8, l[s]);
        }
    }
    let (mut acc, mut nacc, mut pos) = (0u64, 0u32, 0usize);
    for i in 0..n {
        while nacc < MAX_BITS {
            let b = flujo.get(pos).copied().unwrap_or(0);
            pos += 1;
            acc = (acc << 8) | b as u64;
            nacc += 8;
        }
        let (s, lar) = tabla[((acc >> (nacc - MAX_BITS)) & ((1 << MAX_BITS) - 1)) as usize];
        if lar == 0 {
            bail_tr!("código de Huffman inválido", "invalid Huffman code");
        }
        salida[i * paso] = s;
        nacc -= lar as u32;
    }
    let usados = pos as u64 * 8 - nacc as u64;
    if usados > bits {
        bail_tr!("el flujo de Huffman se acabó antes de tiempo", "the Huffman stream ended early");
    }
    Ok(())
}

// ── trozos ──────────────────────────────────────────────────────────────────────────────────

/// (bytes en disco, ¿el byte alto va con Huffman?)
fn comprimir_trozo(crudo: &[u8]) -> (Vec<u8>, bool) {
    let n = crudo.len() / 2;
    let mut bajo = Vec::with_capacity(crudo.len());
    let mut alto = Vec::with_capacity(n);
    for [b, a] in crudo.as_chunks::<2>().0 {
        bajo.push(*b);
        alto.push(*a);
    }
    match huff_codificar(&alto) {
        Some(h) if (h.len() as f64) < n as f64 * (1.0 - GANANCIA_MIN) => {
            bajo.extend_from_slice(&h);
            (bajo, true)
        }
        _ => {
            bajo.extend_from_slice(&alto);
            (bajo, false)
        }
    }
}

fn descomprimir_trozo(dat: &[u8], huff: bool, salida: &mut [u8]) -> Result<()> {
    let n = salida.len() / 2;
    if dat.len() < n {
        bail!("trozo truncado");
    }
    let (bajo, resto) = dat.split_at(n);
    for (i, &b) in bajo.iter().enumerate() {
        salida[2 * i] = b;
    }
    if huff {
        huff_decodificar(resto, n, &mut salida[1..], 2)
    } else {
        if resto.len() != n {
            bail!("trozo de largo inesperado");
        }
        for (i, &b) in resto.iter().enumerate() {
            salida[2 * i + 1] = b;
        }
        Ok(())
    }
}

// ── archivo ─────────────────────────────────────────────────────────────────────────────────

/// Escribe los tensores (temporal + rename: nunca queda uno a medias). Devuelve los bytes en disco.
pub fn escribir(ruta: &Path, tensores: &[Tensor]) -> Result<u64> {
    for t in tensores {
        let e = bytes_dtype(&t.dtype).ok_or_else(|| anyhow!("dtype desconocido: {}", t.dtype))?;
        if elementos(&t.forma).and_then(|n| n.checked_mul(e)) != Some(t.datos.len()) {
            bail_tr!("{}: la forma {:?} no cuadra con {} bytes", "{}: shape {:?} does not match {} bytes", t.nombre, t.forma, t.datos.len());
        }
    }
    // todos los trozos de todos los tensores, en paralelo
    let tareas: Vec<(usize, &[u8], bool)> = tensores
        .iter()
        .enumerate()
        .flat_map(|(i, t)| {
            let h16 = bytes_dtype(&t.dtype) == Some(2);
            t.datos.chunks(TROZO).map(move |c| (i, c, h16))
        })
        .collect();
    let hechos: Vec<(usize, Vec<u8>, &'static str, u64)> = tareas
        .par_iter()
        .map(|&(i, c, h16)| {
            if h16 {
                let (d, huff) = comprimir_trozo(c);
                (i, d, if huff { "huff" } else { "crudo" }, c.len() as u64)
            } else {
                (i, c.to_vec(), "crudo", c.len() as u64)
            }
        })
        .collect();
    let mut metas: Vec<TensorMeta> = tensores
        .iter()
        .map(|t| TensorMeta { nombre: t.nombre.clone(), dtype: t.dtype.clone(), forma: t.forma.clone(), codec: if bytes_dtype(&t.dtype) == Some(2) { "h16".into() } else { "crudo".into() }, trozos: vec![] })
        .collect();
    let mut pos = 0u64;
    for (i, d, alto, crudos) in &hechos {
        metas[*i].trozos.push(TrozoMeta { pos, largo: d.len() as u64, crudos: *crudos, alto: alto.to_string() });
        pos += d.len() as u64;
    }
    let cab = cabecera_bytes(&Cabecera { tensores: metas })?;
    let base = 4 + 8 + cab.len() as u64;
    let tmp = ruta.with_extension(format!("{}.tmp", std::process::id()));
    {
        use std::io::Write;
        let mut f = std::io::BufWriter::with_capacity(8 << 20, std::fs::File::create(&tmp).with_context(|| format!("crear {}", tmp.display()))?);
        f.write_all(MAGIA)?;
        f.write_all(&(cab.len() as u64).to_le_bytes())?;
        f.write_all(&cab)?;
        for (_, d, _, _) in &hechos {
            f.write_all(d)?;
        }
        f.flush()?;
    }
    std::fs::rename(&tmp, ruta).with_context(|| format!("renombrar {}", ruta.display()))?;
    Ok(base + pos)
}

fn cabecera(datos: &[u8]) -> Result<(Cabecera, usize)> {
    if datos.len() < 12 || &datos[..4] != MAGIA {
        bail_tr!("no es un archivo GKV2", "not a GKV2 file");
    }
    let n = u64::from_le_bytes(datos[4..12].try_into().unwrap()) as usize;
    if n > 64 << 20 || 12 + n > datos.len() {
        bail!("cabecera GKV2 truncada");
    }
    Ok((cabecera_de(&datos[12..12 + n]).context("cabecera GKV2")?, 12 + n))
}

/// Lee todos los tensores de un archivo `GKV2` (en paralelo por trozos).
pub fn leer(ruta: &Path) -> Result<Vec<Leido>> {
    let datos = std::fs::read(ruta).with_context(|| format!("leer {}", ruta.display()))?;
    leer_bytes(&datos)
}

fn leer_bytes(datos: &[u8]) -> Result<Vec<Leido>> {
    let (cab, base) = cabecera(datos)?;
    let mut salidas: Vec<Vec<u8>> = Vec::with_capacity(cab.tensores.len());
    for t in &cab.tensores {
        let e = bytes_dtype(&t.dtype).ok_or_else(|| anyhow!("dtype desconocido: {}", t.dtype))?;
        let total = elementos(&t.forma).and_then(|n| n.checked_mul(e)).ok_or_else(|| anyhow!("forma imposible"))?;
        if total > 64 << 30 || t.trozos.iter().map(|z| z.crudos).sum::<u64>() != total as u64 {
            bail_tr!("{}: los trozos no suman el tensor", "{}: the chunks do not add up to the tensor", t.nombre);
        }
        salidas.push(vec![0u8; total]);
    }
    // cada trozo escribe en su pedazo de la salida: tareas disjuntas, en paralelo
    let mut tareas: Vec<(&mut [u8], &TrozoMeta, bool)> = Vec::new();
    for (t, salida) in cab.tensores.iter().zip(salidas.iter_mut()) {
        let h16 = t.codec == "h16";
        let mut resto: &mut [u8] = salida.as_mut_slice();
        for z in &t.trozos {
            let (este, sigue) = std::mem::take(&mut resto).split_at_mut(z.crudos as usize);
            tareas.push((este, z, h16));
            resto = sigue;
        }
    }
    tareas.into_par_iter().try_for_each(|(salida, z, h16)| -> Result<()> {
        let ini = base.checked_add(z.pos as usize).ok_or_else(|| anyhow_tr!("posición imposible", "impossible position"))?;
        let dat = datos.get(ini..ini + z.largo as usize).ok_or_else(|| anyhow_tr!("archivo GKV2 truncado", "truncated GKV2 file"))?;
        if !h16 {
            if dat.len() != salida.len() {
                bail!("trozo crudo de largo inesperado");
            }
            salida.copy_from_slice(dat);
            return Ok(());
        }
        if salida.len() % 2 != 0 {
            bail!("trozo h16 impar");
        }
        descomprimir_trozo(dat, z.alto == "huff", salida)
    })?;
    Ok(cab.tensores.into_iter().zip(salidas).map(|(t, datos)| Leido { nombre: t.nombre, dtype: t.dtype, forma: t.forma, datos }).collect())
}

/// Bytes sin comprimir que guarda un archivo (sólo lee la cabecera).
pub fn crudos(ruta: &Path) -> Result<u64> {
    use std::io::Read;
    let mut f = std::fs::File::open(ruta)?;
    let mut cab = [0u8; 12];
    f.read_exact(&mut cab)?;
    if &cab[..4] != MAGIA {
        bail_tr!("no es un archivo GKV2", "not a GKV2 file");
    }
    let n = u64::from_le_bytes(cab[4..12].try_into().unwrap()) as usize;
    if n > 64 << 20 {
        bail!("cabecera GKV2 imposible");
    }
    let mut j = vec![0u8; n];
    f.read_exact(&mut j)?;
    let c = cabecera_de(&j)?;
    Ok(c.tensores.iter().flat_map(|t| &t.trozos).map(|z| z.crudos).sum())
}

/// Lee un `.safetensors` (el formato en que MLX guarda) y lo escribe como `GKV2`. Devuelve los bytes
/// en disco y los crudos. `metadatos` del safetensors se descartan (el índice de la memoria los lleva).
pub fn desde_safetensors(origen: &Path, destino: &Path) -> Result<(u64, u64)> {
    let datos = std::fs::read(origen).with_context(|| format!("leer {}", origen.display()))?;
    if datos.len() < 8 {
        bail!("safetensors truncado");
    }
    let n = u64::from_le_bytes(datos[..8].try_into().unwrap()) as usize;
    if n > 100 << 20 || 8 + n > datos.len() {
        bail!("cabecera de safetensors imposible");
    }
    let cab: serde_json::Map<String, serde_json::Value> = serde_json::from_slice(&datos[8..8 + n]).context("cabecera de safetensors")?;
    let base = 8 + n;
    let mut tensores = Vec::new();
    for (nombre, v) in &cab {
        if nombre == "__metadata__" {
            continue;
        }
        let dtype = v["dtype"].as_str().ok_or_else(|| anyhow_tr!("{nombre}: sin dtype", "{nombre}: no dtype"))?.to_string();
        let forma: Vec<i64> = v["shape"].as_array().ok_or_else(|| anyhow_tr!("{nombre}: sin forma", "{nombre}: no shape"))?.iter().filter_map(|x| x.as_i64()).collect();
        let off = v["data_offsets"].as_array().ok_or_else(|| anyhow_tr!("{nombre}: sin data_offsets", "{nombre}: no data_offsets"))?;
        let (a, b) = (off[0].as_u64().unwrap_or(0) as usize, off[1].as_u64().unwrap_or(0) as usize);
        let trozo = datos.get(base + a..base + b).ok_or_else(|| anyhow_tr!("{nombre}: fuera del archivo", "{nombre}: outside the file"))?;
        tensores.push(Tensor { nombre: nombre.clone(), dtype, forma, datos: trozo });
    }
    let crudos: u64 = tensores.iter().map(|t| t.datos.len() as u64).sum();
    Ok((escribir(destino, &tensores)?, crudos))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Carpeta vacía de la prueba; se borra sola al soltarse.
    fn tmp(tag: &str) -> gotv_engine::prueba::CarpetaPrueba {
        gotv_engine::prueba::CarpetaPrueba::nueva(&format!("gotv-gkv-{tag}"))
    }

    /// Un PRNG mínimo y determinista (sin dependencias).
    struct Azar(u64);
    impl Azar {
        fn sig(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
        /// ~normal(0, 1) con la suma de 4 uniformes
        fn normal(&mut self) -> f32 {
            (0..4).map(|_| (self.sig() % 10_000) as f32 / 10_000.0).sum::<f32>() - 2.0
        }
    }

    /// BF16 «realista»: como la caché K/V (valores de escala variada, signo al azar).
    fn bf16_realista(n: usize, semilla: u64) -> Vec<u8> {
        let mut a = Azar(semilla);
        (0..n).flat_map(|i| {
            let x = a.normal() * (1.0 + (i % 7) as f32);
            let b = (x.to_bits() >> 16) as u16;
            b.to_le_bytes()
        }).collect()
    }

    fn ida_y_vuelta(tag: &str, tensores: Vec<(String, String, Vec<i64>, Vec<u8>)>) -> u64 {
        let d = tmp(tag);
        let ruta = d.join("x.gkv");
        let refs: Vec<Tensor> = tensores.iter().map(|(n, t, f, b)| Tensor { nombre: n.clone(), dtype: t.clone(), forma: f.clone(), datos: b }).collect();
        let bytes = escribir(&ruta, &refs).unwrap();
        assert_eq!(bytes, std::fs::metadata(&ruta).unwrap().len());
        let leidos = leer(&ruta).unwrap();
        assert_eq!(leidos.len(), tensores.len());
        for ((n, t, f, b), l) in tensores.iter().zip(&leidos) {
            assert_eq!((&l.nombre, &l.dtype, &l.forma), (n, t, f));
            assert!(l.datos == *b, "{n}: los bytes NO son idénticos");
        }
        assert_eq!(crudos(&ruta).unwrap(), tensores.iter().map(|t| t.3.len() as u64).sum::<u64>());
        let _ = std::fs::remove_dir_all(&d);
        bytes
    }

    #[test]
    fn ida_y_vuelta_bit_a_bit_y_comprime_lo_realista() {
        // realista: 2 trozos y pico (el último corto), como un tensor K de la caché
        let n = (TROZO / 2) * 2 + 12_345;
        let real = bf16_realista(n, 7);
        let bytes = ida_y_vuelta("real", vec![("8.k".into(), "BF16".into(), vec![1, 1, n as i64], real.clone())]);
        let razon = bytes as f64 / real.len() as f64;
        assert!(razon < 0.75, "lo realista debe comprimir: {razon:.3}");
    }

    #[test]
    fn casos_borde_bit_a_bit() {
        let azar: Vec<u8> = { let mut a = Azar(99); (0..200_001 * 2).map(|_| a.sig() as u8).collect() };
        ida_y_vuelta("borde", vec![
            // todo ceros (un solo símbolo en el byte alto)
            ("ceros".into(), "BF16".into(), vec![1000], vec![0u8; 2000]),
            // un solo valor distinto de cero
            ("uno".into(), "F16".into(), vec![3, 5], [0x00u8, 0x3c].repeat(15)),
            // aleatorio puro: el byte alto no gana y va crudo
            ("azar".into(), "BF16".into(), vec![200_001], azar),
            // vacío
            ("vacio".into(), "BF16".into(), vec![1, 4, 0, 256], vec![]),
            // otro tamaño de elemento (el estado recurrente es F32): va crudo
            ("estado".into(), "F32".into(), vec![2, 3], (0..24u8).collect()),
        ]);
    }

    #[test]
    fn el_huffman_limita_los_largos_y_sigue_siendo_exacto() {
        // frecuencias de Fibonacci: sin límite, los códigos pasarían de 15 bits
        let mut datos = Vec::new();
        let (mut a, mut b) = (1usize, 1usize);
        for s in 0..24u8 {
            datos.extend(std::iter::repeat_n(s, a));
            (a, b) = (b, a + b);
        }
        let mut frec = [0u64; 256];
        for &x in &datos {
            frec[x as usize] += 1;
        }
        let l = largos(&frec);
        assert!(l.iter().all(|&x| x as u32 <= MAX_BITS), "{l:?}");
        let kraft: f64 = l.iter().filter(|&&x| x > 0).map(|&x| 0.5f64.powi(x as i32)).sum();
        assert!(kraft <= 1.0 + 1e-12, "Kraft: {kraft}");
        let h = huff_codificar(&datos).unwrap();
        let mut salida = vec![0u8; datos.len()];
        huff_decodificar(&h, datos.len(), &mut salida, 1).unwrap();
        assert_eq!(salida, datos);
    }

    #[test]
    fn un_archivo_roto_es_error_y_nunca_panico() {
        let d = tmp("roto");
        let ruta = d.join("x.gkv");
        let datos = bf16_realista(50_000, 3);
        escribir(&ruta, &[Tensor { nombre: "k".into(), dtype: "BF16".into(), forma: vec![50_000], datos: &datos }]).unwrap();
        let bueno = std::fs::read(&ruta).unwrap();
        // truncado en varios puntos, magia mala, basura en el flujo
        for corte in [0, 3, 11, 40, bueno.len() / 2, bueno.len() - 1] {
            assert!(leer_bytes(&bueno[..corte]).is_err(), "truncado en {corte}");
        }
        let mut malo = bueno.clone();
        malo[0] = b'X';
        assert!(leer_bytes(&malo).is_err());
        let mut basura = bueno.clone();
        let n = basura.len();
        for b in &mut basura[n - 2000..] {
            *b = 0xFF;
        }
        let _ = leer_bytes(&basura); // error o datos distintos, pero jamás pánico
        // una forma que no cuadra con los bytes no se escribe
        assert!(escribir(&ruta, &[Tensor { nombre: "k".into(), dtype: "BF16".into(), forma: vec![7], datos: &datos }]).is_err());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn desde_un_safetensors() {
        let d = tmp("st");
        let k = bf16_realista(10_000, 5);
        let e: Vec<u8> = (0..64u8).collect();
        // un safetensors a mano: cabecera JSON + datos
        let cab = serde_json::json!({
            "__metadata__": {"largo": "10000"},
            "0.k": {"dtype": "BF16", "shape": [1, 10_000], "data_offsets": [0, k.len()]},
            "1.estado": {"dtype": "F32", "shape": [16], "data_offsets": [k.len(), k.len() + e.len()]},
        }).to_string();
        let mut st = (cab.len() as u64).to_le_bytes().to_vec();
        st.extend_from_slice(cab.as_bytes());
        st.extend_from_slice(&k);
        st.extend_from_slice(&e);
        std::fs::write(d.join("a.safetensors"), &st).unwrap();
        let (bytes, crudos_) = desde_safetensors(&d.join("a.safetensors"), &d.join("a.gkv")).unwrap();
        assert_eq!(crudos_, (k.len() + e.len()) as u64);
        assert!(bytes < crudos_);
        let l = leer(&d.join("a.gkv")).unwrap();
        let por = |n: &str| l.iter().find(|x| x.nombre == n).unwrap().datos.clone();
        assert_eq!(por("0.k"), k);
        assert_eq!(por("1.estado"), e);
        let _ = std::fs::remove_dir_all(&d);
    }

    /// Con la caché REAL del Portador: comprimir y descomprimir, bit a bit, con tiempos.
    /// `GOTV_GKV_REAL=<archivo .safetensors> cargo test -p gotv-mlx --release -- --ignored --nocapture gkv_con_la_cache_real`
    #[test]
    #[ignore]
    fn gkv_con_la_cache_real() {
        let Ok(origen) = std::env::var("GOTV_GKV_REAL") else { return };
        let d = tmp("real-portador");
        let destino = d.join("real.gkv");
        let t = std::time::Instant::now();
        let (bytes, crudos_) = desde_safetensors(Path::new(&origen), &destino).unwrap();
        let t_comprimir = t.elapsed();
        let t = std::time::Instant::now();
        let leidos = leer(&destino).unwrap();
        let t_leer = t.elapsed();
        println!("{:.2} GB → {:.2} GB ({:.1} %) · comprimir+escribir {:?} · leer+descomprimir {:?}", crudos_ as f64 / 1e9, bytes as f64 / 1e9, bytes as f64 / crudos_ as f64 * 100.0, t_comprimir, t_leer);
        // bit a bit contra el original
        let datos = std::fs::read(&origen).unwrap();
        let n = u64::from_le_bytes(datos[..8].try_into().unwrap()) as usize;
        let cab: serde_json::Map<String, serde_json::Value> = serde_json::from_slice(&datos[8..8 + n]).unwrap();
        for l in &leidos {
            let off = cab[&l.nombre]["data_offsets"].as_array().unwrap();
            let (a, b) = (off[0].as_u64().unwrap() as usize, off[1].as_u64().unwrap() as usize);
            assert!(l.datos[..] == datos[8 + n + a..8 + n + b], "{} no es idéntico", l.nombre);
        }
        assert!((bytes as f64) < crudos_ as f64 * 0.70, "≤ 70 %");
        let _ = std::fs::remove_dir_all(&d);
    }
}
