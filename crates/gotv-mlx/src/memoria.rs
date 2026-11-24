//! La memoria rápida en disco por TRAMOS (F2 del plano de la memoria rápida).
//!
//! Antes (M10b) cada guardado reescribía la caché ENTERA (2,5 GB por turno con 77 000 tokens del 9B) y
//! la lectura era todo-o-nada: si un token del principio cambiaba, no se aprovechaba nada. Lo guardado
//! de los tokens viejos NUNCA cambia; basta AÑADIR lo nuevo. Medido 2026-11-11: guardar sólo el tramo de
//! un turno (1 500 tokens, 49 MB) son 21 ms frente a 0,43 s del todo: 51× menos escritura.
//!
//! ```text
//! <sesión>/<clave del modelo>/
//!     indice.gotv     ← LA VERDAD; se escribe el ÚLTIMO (lo que no esté en él es basura). Formato propio
//!                       (códec, tipo MRIX; hasta 2026-11-17 era `indice.json`: lo viejo es caché y se desaloja)
//!     ids             ← u32 LE de los tokens guardados
//!     imgs            ← [(primer token, huella)] de las imágenes dentro de lo guardado
//!     t-000000.gkv    ← K/V de un tramo de tokens [desde, hasta)
//!     e-007400.gkv    ← (modelos híbridos) conv + estado recurrente EN un token
//! ```
//!
//! Al leer se restaura **el prefijo más largo disponible**: en un denso, hasta donde coinciden los
//! tokens; en un híbrido, hasta el último estado guardado dentro de lo que coincide (el estado
//! recurrente no se puede recortar).

use crate::gkv;
use crate::modelo::{Cache, CacheCapa};
use anyhow::{anyhow, bail, Context, Result};
use gotv_engine::{anyhow_tr, bail_tr};
use mlx_rs::{ops, ops::indexing::TryIndexOp, Array, Dtype};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Un tramo nuevo se escribe sólo si añade al menos esto (o si hubo que cortar).
pub const TRAMO_MIN: usize = 256;
/// Tope de tokens por archivo de tramo…
const TRAMO_MAX: usize = 8192;
/// …y de bytes (un denso de 32B guarda 256 KB por token: 1 024 tokens por archivo).
const TRAMO_BYTES: usize = 256 << 20;
/// Estados recurrentes que se conservan (cada uno ~50 MB en el 9B).
const ESTADOS_MAX: usize = 4;
/// Menos que esto no compensa leerlo del disco.
pub const LEER_MIN: usize = 1024;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Tramo {
    pub desde: usize,
    pub hasta: usize,
    pub archivo: String,
    pub bytes: u64,
    pub crudos: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Estado {
    pub en: usize,
    pub archivo: String,
    pub bytes: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Indice {
    pub version: u32,
    pub capas: usize,
    pub recurrente: bool,
    pub tramos: Vec<Tramo>,
    pub estados: Vec<Estado>,
    /// La conversación siguió y lo nuevo NO cupo en el tope: lo guardado es sólo un prefijo.
    #[serde(default)]
    pub recortado: bool,
}

impl Indice {
    /// Hasta qué token llega lo guardado.
    pub fn hasta(&self) -> usize {
        self.tramos.last().map(|t| t.hasta).unwrap_or(0)
    }
    /// Bytes en disco (tramos + estados).
    pub fn bytes(&self) -> u64 {
        self.tramos.iter().map(|t| t.bytes).sum::<u64>() + self.estados.iter().map(|e| e.bytes).sum::<u64>()
    }
    /// Razón disco/crudo de los tramos ya escritos (para estimar lo que ocupará el siguiente).
    pub fn razon(&self) -> Option<f64> {
        let c: u64 = self.tramos.iter().map(|t| t.crudos).sum();
        (c > 0).then(|| self.tramos.iter().map(|t| t.bytes).sum::<u64>() as f64 / c as f64)
    }
}

/// Lo guardado de una conversación con un modelo, tal como está en disco.
#[derive(Clone, Debug, Default)]
pub struct Disco {
    pub dir: PathBuf,
    pub indice: Indice,
    pub ids: Vec<u32>,
    pub imgs: Vec<(usize, u64)>,
}

fn leer_ids(p: &Path) -> Option<Vec<u32>> {
    Some(std::fs::read(p).ok()?.as_chunks::<4>().0.iter().map(|c| u32::from_le_bytes(*c)).collect())
}

fn escribir_atomico(p: &Path, datos: &[u8]) -> Result<()> {
    let tmp = p.with_extension(format!("{}.tmp", std::process::id()));
    std::fs::write(&tmp, datos).with_context(|| format!("escribir {}", tmp.display()))?;
    std::fs::rename(&tmp, p).with_context(|| format!("renombrar {}", p.display()))?;
    Ok(())
}

impl Disco {
    /// Abre lo guardado en `dir` (o un `Disco` vacío si no hay nada). Borra los archivos que no estén
    /// en el índice: son restos de un guardado que se cortó antes de escribirlo.
    pub fn abrir(dir: &Path) -> Disco {
        let vacio = Disco { dir: dir.to_path_buf(), ..Default::default() };
        let Some(indice) = leer_valor::<Indice>(&dir.join("indice.gotv"), b"MRIX") else {
            return vacio;
        };
        let ids = leer_ids(&dir.join("ids")).unwrap_or_default();
        if ids.len() < indice.hasta() {
            return vacio; // el índice promete más tokens de los que hay: no se usa
        }
        let imgs = leer_valor(&dir.join("imgs"), b"MRIM").unwrap_or_default();
        let d = Disco { dir: dir.to_path_buf(), indice, ids, imgs };
        d.limpiar();
        d
    }

    /// Borra lo que no está en el índice (salvo los tres archivos fijos y temporales recientes).
    fn limpiar(&self) {
        let vivos: std::collections::HashSet<&str> = self.indice.tramos.iter().map(|t| t.archivo.as_str()).chain(self.indice.estados.iter().map(|e| e.archivo.as_str())).collect();
        for e in std::fs::read_dir(&self.dir).into_iter().flatten().flatten() {
            let n = e.file_name().to_string_lossy().to_string();
            if matches!(n.as_str(), "indice.gotv" | "ids" | "imgs") || vivos.contains(n.as_str()) {
                continue;
            }
            // un temporal de hace menos de una hora puede ser de otro guardado en curso
            let viejo = e.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_none_or(|d| d.as_secs() > 3600);
            if !n.contains(".tmp") || viejo {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
}

/// Hasta dónde coinciden lo guardado y el prompt, contando las imágenes: dos imágenes del mismo
/// tamaño dan los mismos tokens, la huella las distingue.
pub fn prefijo_comun(d: &Disco, ids: &[u32], lugares: &[(usize, u64)]) -> usize {
    let mut p = d.ids.iter().zip(ids).take_while(|(a, b)| a == b).count();
    for &(i, h) in &d.imgs {
        if i < p && !lugares.contains(&(i, h)) {
            p = p.min(i);
        }
    }
    for &(i, h) in lugares {
        if i < p && !d.imgs.contains(&(i, h)) {
            p = p.min(i);
        }
    }
    p
}

/// Hasta dónde se puede restaurar de `d` para un prompt que coincide en `p` tokens.
pub fn restaurable(d: &Disco, p: usize) -> usize {
    if d.indice.recurrente {
        d.indice.estados.iter().map(|e| e.en).filter(|&e| e <= p && e <= d.indice.hasta()).max().unwrap_or(0)
    } else {
        p.min(d.indice.hasta())
    }
}

fn dtype_de(n: &str) -> Result<Dtype> {
    Ok(match n {
        "BF16" => Dtype::Bfloat16,
        "F16" => Dtype::Float16,
        "F32" => Dtype::Float32,
        "I32" => Dtype::Int32,
        "U32" => Dtype::Uint32,
        otro => bail!("dtype no previsto: {otro}"),
    })
}

/// Un tensor leído → `Array` de MLX (copia los bytes; se valida el tamaño ANTES del `unsafe`).
fn a_array(l: gkv::Leido) -> Result<Array> {
    let dt = dtype_de(&l.dtype)?;
    let forma: Vec<i32> = l.forma.iter().map(|&x| i32::try_from(x)).collect::<std::result::Result<_, _>>().map_err(|_| anyhow!("forma enorme"))?;
    let n: usize = forma.iter().map(|&x| x as usize).product();
    if n * gkv::bytes_dtype(&l.dtype).unwrap_or(0) != l.datos.len() {
        bail_tr!("{}: la forma {:?} no cuadra con {} bytes", "{}: shape {:?} does not match {} bytes", l.nombre, l.forma, l.datos.len());
    }
    // SAFETY: el puntero vale mientras viva `l.datos` (MLX copia los datos al crear el array) y el
    // tamaño se comprobó arriba contra forma × dtype
    Ok(unsafe { Array::from_raw_data(l.datos.as_ptr() as *const std::ffi::c_void, &forma, dt) })
}

/// Guarda arrays de MLX como `GKV2`: MLX los escribe a un `.safetensors` temporal (él sabe hacerlos
/// contiguos) y el códec lo comprime. Devuelve (bytes en disco, crudos).
fn guardar_gkv(destino: &Path, partes: Vec<(String, Array)>) -> Result<(u64, u64)> {
    let tmp = destino.with_extension(format!("{}.tmp.safetensors", std::process::id()));
    Array::save_safetensors(partes, &HashMap::new(), &tmp).map_err(|e| anyhow!("guardar {}: {e:?}", tmp.display()))?;
    let r = gkv::desde_safetensors(&tmp, destino);
    let _ = std::fs::remove_file(&tmp);
    r
}

/// Lo que dejó un guardado.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Guardado {
    /// tokens añadidos (0 = no hizo falta)
    pub nuevos: usize,
    /// tokens que se cortaron por no coincidir ya (compactar, rebobinar)
    pub cortados: usize,
    pub bytes: u64,
    /// no se añadió porque no cabe en el tope (queda el prefijo ya guardado)
    pub no_cabe: bool,
}

/// Bytes por token de los K/V de esta caché (para partir en archivos y estimar el tope).
pub fn bytes_por_token(c: &Cache) -> usize {
    c.capas.iter().filter_map(|cc| cc.k.as_ref().zip(cc.v.as_ref())).map(|(k, v)| (k.dim(1) * k.dim(3)) as usize * k.item_size() + (v.dim(1) * v.dim(3)) as usize * v.item_size()).sum()
}

/// Guarda la caché `c` (que va EXACTAMENTE por `ids`) añadiendo sólo lo nuevo a lo que hay en `d`.
/// `cabe(bytes)` decide si cabe en el tope (F3); si no cabe, no se añade nada (queda el prefijo).
pub fn guardar(d: &mut Disco, c: &Cache, ids: &[u32], lugares: &[(usize, u64)], cabe: &mut dyn FnMut(u64, &Disco) -> bool) -> Result<Guardado> {
    if c.largo as usize != ids.len() {
        bail_tr!("la caché va por {} tokens y los ids son {}", "the cache is at {} tokens and the ids are {}", c.largo, ids.len());
    }
    let recurrente = c.capas.iter().any(|cc| cc.estado.is_some() || cc.conv.is_some());
    let mut g = Guardado::default();
    // 1) lo que ya no coincide se corta (por tramos enteros: un tramo no se reescribe a medias)
    let p = prefijo_comun(d, ids, lugares);
    if d.indice.capas != c.capas.len() || d.indice.recurrente != recurrente {
        d.indice = Indice { version: 2, capas: c.capas.len(), recurrente, ..Default::default() };
    }
    let antes = d.indice.hasta();
    d.indice.tramos.retain(|t| t.hasta <= p);
    let b = d.indice.hasta();
    d.indice.estados.retain(|e| e.en <= b);
    g.cortados = antes.saturating_sub(b);
    let n = ids.len();
    let delta = n.saturating_sub(b);
    // menos de TRAMO_MIN nuevos y nada cortado: se acumula para el siguiente guardado (un híbrido
    // escribiría además un estado de ~50 MB por un puñado de tokens)
    if delta < TRAMO_MIN && g.cortados == 0 {
        return Ok(g);
    }
    // 2) el tope (F3): lo nuevo, estimado con la razón de lo ya escrito
    let bpt = bytes_por_token(c).max(1);
    let razon = d.indice.razon().unwrap_or(0.72);
    let estimado = |m: usize| ((m * bpt) as f64 * razon + if recurrente { 60e6 } else { 0.0 }) as u64;
    let mut n = n;
    if delta > 0 && !cabe(estimado(delta), d) {
        // en un denso cabe un PREFIJO: los K/V se cortan donde sea. Búsqueda binaria en pasos de
        // TRAMO_MIN: `cabe` desaloja lo viejo cuando hace falta, así que el mayor prefijo que cabe
        // gana sobre las memorias de otras conversaciones (medido: probando mitades se quedaba en el
        // primer tamaño que cabía SIN desalojar). En un híbrido no se puede: haría falta el estado
        // recurrente en ese punto y sólo está el de `n`.
        let (mut lo, mut hi) = (0usize, if recurrente { 0 } else { (delta - 1) / TRAMO_MIN });
        while lo < hi {
            let mid = (lo + hi).div_ceil(2);
            if cabe(estimado(mid * TRAMO_MIN), d) {
                lo = mid;
            } else {
                hi = mid - 1;
            }
        }
        let m = lo * TRAMO_MIN;
        g.no_cabe = true;
        if m < TRAMO_MIN {
            // no cabe nada: lo cortado se refleja igual (el índice debe decir la verdad)
            if g.cortados > 0 || !d.indice.recortado {
                d.indice.recortado = true;
                escribir_indice(d, &ids[..b], lugares)?;
            }
            return Ok(g);
        }
        n = b + m;
    }
    d.indice.recortado = g.no_cabe;
    let ids = &ids[..n];
    std::fs::create_dir_all(&d.dir).with_context(|| format!("crear {}", d.dir.display()))?;
    // 3) los tramos nuevos [b, n), en archivos de hasta TRAMO_MAX tokens y TRAMO_BYTES
    let por_archivo = (TRAMO_BYTES / bpt).clamp(TRAMO_MIN, TRAMO_MAX);
    let mut a = b;
    while a < n {
        let z = (a + por_archivo).min(n);
        let mut partes = Vec::new();
        for (i, cc) in c.capas.iter().enumerate() {
            if let (Some(k), Some(v)) = (&cc.k, &cc.v) {
                partes.push((format!("{i}.k"), k.try_index((.., .., a as i32..z as i32, ..)).map_err(|e| anyhow!("{e:?}"))?));
                partes.push((format!("{i}.v"), v.try_index((.., .., a as i32..z as i32, ..)).map_err(|e| anyhow!("{e:?}"))?));
            }
        }
        let archivo = format!("t-{a:07}.gkv");
        let (bytes, crudos) = if partes.is_empty() { (0, 0) } else { guardar_gkv(&d.dir.join(&archivo), partes)? };
        d.indice.tramos.push(Tramo { desde: a, hasta: z, archivo, bytes, crudos });
        g.bytes += bytes;
        a = z;
    }
    // 4) híbridos: el estado recurrente en `n` (desde él se restaura)
    if recurrente && delta > 0 {
        let mut partes = Vec::new();
        for (i, cc) in c.capas.iter().enumerate() {
            if let Some(x) = &cc.conv {
                partes.push((format!("{i}.conv"), x.clone()));
            }
            if let Some(x) = &cc.estado {
                partes.push((format!("{i}.estado"), x.clone()));
            }
        }
        let archivo = format!("e-{n:07}.gkv");
        let (bytes, _) = guardar_gkv(&d.dir.join(&archivo), partes)?;
        d.indice.estados.push(Estado { en: n, archivo, bytes });
        g.bytes += bytes;
        // sólo los más recientes
        let sobran = d.indice.estados.len().saturating_sub(ESTADOS_MAX);
        d.indice.estados.drain(..sobran);
    }
    g.nuevos = n - b;
    escribir_indice(d, ids, lugares)?;
    Ok(g)
}

/// imgs, ids y, el ÚLTIMO, el índice; después se borra lo que ya no está en él.
fn escribir_indice(d: &mut Disco, ids: &[u32], lugares: &[(usize, u64)]) -> Result<()> {
    std::fs::create_dir_all(&d.dir)?;
    let hasta = d.indice.hasta();
    d.ids = ids[..hasta.min(ids.len())].to_vec();
    d.imgs = lugares.iter().copied().filter(|(i, _)| *i < hasta).collect();
    escribir_atomico(&d.dir.join("imgs"), &bytes_valor(&d.imgs, b"MRIM")?)?;
    escribir_atomico(&d.dir.join("ids"), &d.ids.iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<u8>>())?;
    escribir_atomico(&d.dir.join("indice.gotv"), &bytes_valor(&d.indice, b"MRIX")?)?;
    d.limpiar();
    Ok(())
}

/// Lo propio de GOTV en su formato (códec): el valor serializado con la codificación binaria del códec.
fn bytes_valor<T: Serialize>(v: &T, tipo: &[u8; 4]) -> Result<Vec<u8>> {
    let mut e = gotv_engine::codec::Escritor::nuevo(tipo, 1);
    e.valor(&serde_json::to_value(v)?);
    Ok(e.bytes())
}

fn leer_valor<T: serde::de::DeserializeOwned>(p: &Path, tipo: &[u8; 4]) -> Option<T> {
    gotv_engine::codec::leer_serde(p, tipo)
}

/// Reconstruye la caché de los primeros `r` tokens (ver [`restaurable`]). `capas` = las del modelo.
pub fn leer(d: &Disco, r: usize, capas: usize) -> Result<Cache> {
    if d.indice.capas != capas {
        bail_tr!("la memoria guardada es de otro modelo ({} capas, éste tiene {capas})", "the saved memory is from another model ({} layers, this one has {capas})", d.indice.capas);
    }
    if r == 0 || r > d.indice.hasta() {
        bail!("no hay {r} tokens guardados");
    }
    // piezas de cada tensor K/V, en orden de tokens
    let mut piezas: HashMap<String, Vec<Array>> = HashMap::new();
    for t in d.indice.tramos.iter().filter(|t| t.desde < r) {
        if t.bytes == 0 {
            continue;
        }
        for l in gkv::leer(&d.dir.join(&t.archivo))? {
            let nombre = l.nombre.clone();
            let mut a = a_array(l)?;
            if t.hasta > r {
                // el último tramo se corta en r (sólo en densos: en un híbrido r siempre es un estado)
                a = a.try_index((.., .., ..(r - t.desde) as i32, ..)).map_err(|e| anyhow!("{e:?}"))?;
            }
            piezas.entry(nombre).or_default().push(a);
        }
    }
    let mut cache = Cache { capas: (0..capas).map(|_| CacheCapa::default()).collect(), largo: r as i32, instantaneas: Vec::new() };
    for (nombre, p) in piezas {
        let (i, que) = nombre.split_once('.').ok_or_else(|| anyhow_tr!("tensor sin capa: {nombre}", "tensor without a layer: {nombre}"))?;
        let i: usize = i.parse().map_err(|_| anyhow!("capa ilegible: {nombre}"))?;
        let unido = if p.len() == 1 { p.into_iter().next().unwrap() } else { ops::concatenate(&p, 2).map_err(|e| anyhow!("{e:?}"))? };
        let cc = cache.capas.get_mut(i).ok_or_else(|| anyhow_tr!("capa {i} fuera del modelo", "layer {i} outside the model"))?;
        match que {
            "k" => cc.k = Some(unido),
            "v" => cc.v = Some(unido),
            otro => bail!("tensor inesperado en un tramo: {otro}"),
        }
    }
    if d.indice.recurrente {
        let e = d.indice.estados.iter().find(|e| e.en == r).ok_or_else(|| anyhow!("no hay estado guardado en {r}"))?;
        for l in gkv::leer(&d.dir.join(&e.archivo))? {
            let (i, que) = l.nombre.split_once('.').map(|(a, b)| (a.to_string(), b.to_string())).ok_or_else(|| anyhow_tr!("tensor sin capa", "tensor without a layer"))?;
            let i: usize = i.parse().map_err(|_| anyhow!("capa ilegible"))?;
            let a = a_array(l)?;
            let cc = cache.capas.get_mut(i).ok_or_else(|| anyhow_tr!("capa {i} fuera del modelo", "layer {i} outside the model"))?;
            match que.as_str() {
                "conv" => cc.conv = Some(a),
                "estado" => cc.estado = Some(a),
                otro => bail!("tensor inesperado en un estado: {otro}"),
            }
        }
    }
    // a memoria YA (si no, MLX difiere la lectura al primer uso y el tiempo se esconde)
    for cc in &cache.capas {
        for a in [&cc.k, &cc.v, &cc.conv, &cc.estado].into_iter().flatten() {
            a.eval().map_err(|e| anyhow!("{e:?}"))?;
        }
    }
    Ok(cache)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// (guardia de la raíz, carpeta de la clave): la guardia borra todo al soltarse.
    fn dir(tag: &str) -> (gotv_engine::prueba::CarpetaPrueba, PathBuf) {
        let g = gotv_engine::prueba::CarpetaPrueba::sin_crear(&format!("gotv-memoria-{tag}"));
        let d = g.join("s1").join("clave");
        (g, d)
    }

    /// Una caché falsa de `n` tokens: dos capas de atención (K/V bf16 [1,2,n,4]) y, si `hibrida`, una
    /// lineal con conv/estado. Los valores dependen de la posición: un corte mal hecho se nota.
    fn cache(n: usize, hibrida: bool, semilla: f32) -> Cache {
        let kv = |capa: f32| {
            let datos: Vec<f32> = (0..2 * n * 4).map(|i| ((i / 4) % n) as f32 * 0.01 + capa + semilla).collect();
            Array::from_slice(&datos, &[1, 2, n as i32, 4]).as_dtype(Dtype::Bfloat16).unwrap()
        };
        let mut capas = vec![
            CacheCapa { k: Some(kv(1.0)), v: Some(kv(2.0)), conv: None, estado: None },
            CacheCapa { k: Some(kv(3.0)), v: Some(kv(4.0)), conv: None, estado: None },
        ];
        if hibrida {
            let e: Vec<f32> = (0..16).map(|i| i as f32 + n as f32 + semilla).collect();
            capas.push(CacheCapa { k: None, v: None, conv: Some(Array::from_slice(&e[..6], &[1, 2, 3]).as_dtype(Dtype::Bfloat16).unwrap()), estado: Some(Array::from_slice(&e, &[1, 2, 2, 4])) });
        }
        Cache { capas, largo: n as i32, instantaneas: vec![] }
    }

    fn valores(a: &Array) -> Vec<f32> {
        let a = a.as_dtype(Dtype::Float32).unwrap();
        a.eval().unwrap();
        a.as_slice::<f32>().to_vec()
    }

    fn igual_hasta(c: &Cache, original: &Cache, r: usize) {
        assert_eq!(c.largo as usize, r);
        for (x, o) in c.capas.iter().zip(&original.capas) {
            if let (Some(k), Some(ko)) = (&x.k, &o.k) {
                let ko = ko.try_index((.., .., ..r as i32, ..)).unwrap();
                assert_eq!(valores(k), valores(&ko), "K distinto");
            }
        }
    }

    fn ids(n: usize, desde_cambia: Option<usize>) -> Vec<u32> {
        (0..n as u32).map(|i| if desde_cambia.is_some_and(|d| i as usize >= d) { i + 1_000_000 } else { i }).collect()
    }

    #[test]
    fn crece_por_tramos_corta_y_lee_el_prefijo_mas_largo() {
        for hibrida in [false, true] {
            let (_raiz, d0) = dir(if hibrida { "hibrida" } else { "densa" });
            let mut d = Disco::abrir(&d0);
            // turno 1: 1 000 tokens · turno 2: 2 500 (sólo se añaden 1 500)
            let c1 = cache(1000, hibrida, 0.0);
            let g1 = guardar(&mut d, &c1, &ids(1000, None), &[], &mut |_, _| true).unwrap();
            assert_eq!((g1.nuevos, g1.cortados), (1000, 0));
            let c2 = cache(2500, hibrida, 0.0);
            let g2 = guardar(&mut d, &c2, &ids(2500, None), &[], &mut |_, _| true).unwrap();
            assert_eq!((g2.nuevos, g2.cortados), (1500, 0), "sólo lo nuevo");
            assert_eq!(d.indice.tramos.iter().map(|t| (t.desde, t.hasta)).collect::<Vec<_>>(), vec![(0, 1000), (1000, 2500)]);
            // menos de TRAMO_MIN nuevos: no se escribe nada
            let g = guardar(&mut d, &cache(2600, hibrida, 0.0), &ids(2600, None), &[], &mut |_, _| true).unwrap();
            assert_eq!(g, Guardado::default());
            // reabrir: un Disco nuevo lee lo mismo; prompt de 3 000 que coincide entero
            let d2 = Disco::abrir(&d0);
            assert_eq!(d2.indice, d.indice);
            let p = prefijo_comun(&d2, &ids(3000, None), &[]);
            assert_eq!(p, 2500);
            let r = restaurable(&d2, p);
            assert_eq!(r, 2500);
            igual_hasta(&leer(&d2, r, c2.capas.len()).unwrap(), &c2, r);
            // rebobinar: el prompt diverge en 1 800 → denso: 1 800 exactos; híbrido: el estado en 1 000
            let p = prefijo_comun(&d2, &ids(3000, Some(1800)), &[]);
            let r = restaurable(&d2, p);
            assert_eq!(r, if hibrida { 1000 } else { 1800 });
            let c = leer(&d2, r, c2.capas.len()).unwrap();
            igual_hasta(&c, &c2, r);
            if hibrida {
                assert_eq!(valores(c.capas[2].estado.as_ref().unwrap()), valores(c1.capas[2].estado.as_ref().unwrap()), "el estado del token 1 000");
            }
            // compactar: cambia desde el 1 500 → se corta en 1 000 (tramo entero) y se sigue
            let c3 = cache(2200, hibrida, 0.0);
            let g3 = guardar(&mut d, &c3, &ids(2200, Some(1500)), &[], &mut |_, _| true).unwrap();
            assert_eq!((g3.cortados, g3.nuevos), (1500, 1200));
            assert_eq!(d.indice.hasta(), 2200);
            assert!(d.indice.estados.iter().all(|e| e.en <= 1000 || e.en == 2200));
        }
    }

    #[test]
    fn el_indice_manda_y_la_basura_se_borra() {
        let (_raiz, d0) = dir("basura");
        let mut d = Disco::abrir(&d0);
        guardar(&mut d, &cache(600, false, 0.0), &ids(600, None), &[], &mut |_, _| true).unwrap();
        // un guardado que se cortó antes del índice: tramo nuevo sin índice + un temporal viejo
        std::fs::write(d0.join("t-0000600.gkv"), b"a medias").unwrap();
        let d2 = Disco::abrir(&d0);
        assert_eq!(d2.indice.hasta(), 600, "vale el índice de antes");
        assert!(!d0.join("t-0000600.gkv").exists(), "lo que no está en el índice se borra");
        // un índice que promete más tokens de los que hay no se usa
        std::fs::write(d0.join("ids"), [0u8; 8]).unwrap();
        assert_eq!(Disco::abrir(&d0).indice.hasta(), 0);
    }

    #[test]
    fn si_no_cabe_no_se_anade_y_lo_guardado_sigue() {
        let (_raiz, d0) = dir("tope");
        let mut d = Disco::abrir(&d0);
        guardar(&mut d, &cache(600, false, 0.0), &ids(600, None), &[], &mut |_, _| true).unwrap();
        let g = guardar(&mut d, &cache(1500, false, 0.0), &ids(1500, None), &[], &mut |_, _| false).unwrap();
        assert!(g.no_cabe && g.nuevos == 0);
        assert_eq!(Disco::abrir(&d0).indice.hasta(), 600, "queda el prefijo que cupo");
    }

    #[test]
    fn las_imagenes_distintas_cortan_el_prefijo() {
        let (_raiz, d0) = dir("imgs");
        let mut d = Disco::abrir(&d0);
        guardar(&mut d, &cache(800, false, 0.0), &ids(800, None), &[(100, 7)], &mut |_, _| true).unwrap();
        assert_eq!(prefijo_comun(&d, &ids(900, None), &[(100, 7)]), 800);
        assert_eq!(prefijo_comun(&d, &ids(900, None), &[(100, 8)]), 100, "otra imagen del mismo tamaño");
        assert_eq!(prefijo_comun(&d, &ids(900, None), &[]), 100);
    }
}

#[cfg(test)]
mod tests_tope {
    use super::*;

    #[test]
    fn si_no_cabe_entera_un_denso_guarda_el_prefijo_que_cabe() {
        let raiz = gotv_engine::prueba::CarpetaPrueba::sin_crear("gotv-memoria-prefijo");
        let d0 = raiz.join("s1").join("clave");
        let datos: Vec<f32> = (0..2 * 4000 * 4).map(|i| i as f32 * 0.001).collect();
        let kv = || Array::from_slice(&datos, &[1, 2, 4000, 4]).as_dtype(Dtype::Bfloat16).unwrap();
        let c = Cache { capas: vec![CacheCapa { k: Some(kv()), v: Some(kv()), conv: None, estado: None }], largo: 4000, instantaneas: vec![] };
        let ids: Vec<u32> = (0..4000).collect();
        // cabe hasta 30 000 bytes estimados (32 B/token × 0,72 → ~1 300 tokens)
        let mut d = Disco::abrir(&d0);
        let g = guardar(&mut d, &c, &ids, &[], &mut |b, _| b <= 30_000).unwrap();
        assert!(g.no_cabe);
        let h = d.indice.hasta();
        assert!(h >= TRAMO_MIN && h.is_multiple_of(TRAMO_MIN) && h < 4000, "un prefijo: {h}");
        assert_eq!(Disco::abrir(&d0).ids.len(), h, "los ids guardados son los del prefijo");
        // un híbrido no puede (le faltaría el estado en el corte): no guarda nada
        let mut ch = Cache { capas: c.capas.clone(), largo: 4000, instantaneas: vec![] };
        ch.capas.push(CacheCapa { k: None, v: None, conv: Some(Array::from_slice(&[1.0f32], &[1])), estado: Some(Array::from_slice(&[2.0f32], &[1])) });
        let d1 = d0.with_file_name("clave-h");
        let mut dh = Disco::abrir(&d1);
        let g = guardar(&mut dh, &ch, &ids, &[], &mut |b, _| b <= 30_000).unwrap();
        assert!(g.no_cabe && g.nuevos == 0 && dh.indice.hasta() == 0);
    }
}
