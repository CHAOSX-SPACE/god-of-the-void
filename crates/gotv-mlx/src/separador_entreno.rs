//! EXP-7 · ENTRENAR el separador: HTDemucs con sus parámetros en un MAPA entrenable.
//!
//! `separador.rs` es la inferencia (pesos convertidos una vez a la disposición de MLX, lote de 1). Aquí
//! el mismo cálculo es una FUNCIÓN de un mapa `nombre → Array` con los nombres y la disposición del
//! `state_dict` de torch, así:
//!
//! - `transforms::keyed_value_and_grad` deriva respecto a TODOS los parámetros a la vez;
//! - los gradientes se comparan nombre a nombre con los de torch (el juez de gradientes);
//! - guardar es escribir el mapa tal cual, con los MISMOS metadatos (`klass`, `kwargs`): el resultado lo
//!   lee `separador::Htdemucs::cargar` sin cambiar nada.
//!
//! Va por lotes (`[B, 2, L]`), y la STFT de la mezcla se hace en la GPU con un índice de reflejo
//! precalculado (un `take`). El transformador lleva el `dropout` del entrenamiento oficial (`t_dropout`,
//! 0,02 en los pesos publicados) en sus cuatro sitios: las probabilidades de la atención, la salida de la
//! atención, dentro del FFN y su salida. Con caída, la atención es la explícita (la fusionada no deja
//! tirar probabilidades).
//!
//! La pérdida es la de `demucs/solver.py`: L1 en la onda, media por fuente y por lote, y la media
//! ponderada por fuente (`args.weights`).
use crate::modelo::M;
use crate::separador::{embebido_1d, embebido_2d, gelu, glu, hann, inverso, media_desv, validar, F32, FRECUENCIA, NFFT, SALTO};
use anyhow::{anyhow, Context, Result};
use gotv_engine::{anyhow_tr, bail_tr};
use mlx_rs::error::Exception;
use mlx_rs::ops::indexing::TryIndexOp;
use mlx_rs::{fast, ops, Array, Dtype};
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;
use std::rc::Rc;

/// Los parámetros por su nombre de torch (`encoder.0.conv.weight`…), en f32.
pub type Parametros = HashMap<Rc<str>, Array>;

/// Lo que fija la FORMA del modelo (lo demás lo valida `separador::validar`).
#[derive(Clone, Copy, Debug)]
pub struct Forma {
    pub fuentes: usize,
    pub cabezas: i32,
    /// `bottom_channels` > 0: hay `channel_upsampler`/`downsampler` alrededor del transformador
    pub fondo: bool,
}

/// Un HTDemucs ENTRENABLE: sus parámetros, su forma y los metadatos con que se volverá a guardar.
pub struct Modelo {
    pub parametros: Parametros,
    pub forma: Forma,
    pub fuentes: Vec<String>,
    /// largo del trozo de entrenamiento en muestras
    pub segmento: usize,
    /// `t_dropout` del modelo (la caída del transformador al entrenar)
    pub caida: f32,
    meta: HashMap<String, String>,
}

impl Modelo {
    /// Lee un `.safetensors` de HTDemucs (el mismo que usa la inferencia) y sube todo a f32.
    pub fn cargar(ruta: &Path) -> Result<Modelo> {
        let (p, meta) = Array::load_safetensors_with_metadata(ruta).map_err(|e| anyhow!("{}: {e:?}", ruta.display()))?;
        if meta.get("klass").map(String::as_str) != Some("demucs.htdemucs.HTDemucs") {
            bail_tr!("{}: no es un HTDemucs ({:?})", "{}: not an HTDemucs ({:?})", ruta.display(), meta.get("klass"));
        }
        let k: serde_json::Value = serde_json::from_str(meta.get("kwargs").ok_or_else(|| anyhow_tr!("HTDemucs sin argumentos", "HTDemucs without arguments"))?).context("kwargs")?;
        validar(&k)?;
        let fuentes: Vec<String> = serde_json::from_value(k["sources"].clone()).context("sources")?;
        let seg = &k["segment"];
        let segmento = match (seg.get("numerator").and_then(|v| v.as_u64()), seg.get("denominator").and_then(|v| v.as_u64())) {
            (Some(n), Some(d)) => (n as usize * FRECUENCIA as usize) / d as usize,
            _ => (seg.as_f64().ok_or_else(|| anyhow!("segment"))? * FRECUENCIA as f64) as usize,
        };
        let forma = Forma { fuentes: fuentes.len(), cabezas: k["t_heads"].as_i64().unwrap_or(8) as i32, fondo: k["bottom_channels"].as_i64().unwrap_or(0) > 0 };
        let caida = k["t_dropout"].as_f64().unwrap_or(0.0) as f32;
        let parametros = p.into_iter().map(|(n, a)| a.as_dtype(F32).map(|a| (Rc::from(n.as_str()), a))).collect::<std::result::Result<_, _>>().m()?;
        Ok(Modelo { parametros, forma, fuentes, segmento, caida, meta })
    }

    /// Escribe los parámetros con los metadatos originales, en `tipo` (F16 como los publicados, o F32).
    pub fn guardar(&self, ruta: &Path, tipo: Dtype) -> Result<()> {
        let mut v: Vec<(String, Array)> = Vec::with_capacity(self.parametros.len());
        for (n, a) in &self.parametros {
            v.push((n.to_string(), a.as_dtype(tipo).m()?));
        }
        v.sort_by(|a, b| a.0.cmp(&b.0));
        Array::save_safetensors(v.iter().map(|(n, a)| (n.as_str(), a)), &self.meta, ruta).map_err(|e| anyhow!("{}: {e:?}", ruta.display()))
    }

    /// Da otro NOMBRE a las fuentes (mismas salidas): p. ej. el especialista en batería de `HTDemucs-ft`
    /// re-etiquetado como bombo, caja, platos y toms antes de afinarlo con StemGMD. Reescribe `kwargs`.
    pub fn renombrar_fuentes(&mut self, nuevas: &[&str]) -> Result<()> {
        if nuevas.len() != self.fuentes.len() {
            bail_tr!("el modelo tiene {} fuentes, no {}", "the model has {} sources, not {}", self.fuentes.len(), nuevas.len());
        }
        let mut k: serde_json::Value = serde_json::from_str(self.meta.get("kwargs").ok_or_else(|| anyhow!("kwargs"))?)?;
        k["sources"] = serde_json::json!(nuevas);
        self.meta.insert("kwargs".into(), serde_json::to_string(&k)?);
        self.fuentes = nuevas.iter().map(|s| s.to_string()).collect();
        Ok(())
    }

    /// Cuántos números entrenables hay.
    pub fn tamano(&self) -> usize {
        self.parametros.values().map(|a| a.size()).sum()
    }
}

// ───────────────────────────── piezas, en la disposición de torch ─────────────────────────────

fn g<'a>(p: &'a Parametros, n: &str) -> Result<&'a Array> {
    p.get(n).ok_or_else(|| anyhow_tr!("falta el parámetro {n}", "missing parameter {n}"))
}

fn sesgo(p: &Parametros, n: &str, x: Array) -> Result<Array> {
    ops::add(&x, g(p, &format!("{n}.bias"))?).m()
}

/// Conv1d: peso `[O, I, K]`; `x` `[N, L, I]`.
fn conv1d(p: &Parametros, n: &str, x: &Array, paso: i32, relleno: i32, dil: i32) -> Result<Array> {
    let w = ops::transpose_axes(g(p, &format!("{n}.weight"))?, &[0, 2, 1]).m()?;
    sesgo(p, n, ops::conv1d(x, &w, paso, relleno, dil, 1).m()?)
}

/// Conv2d: peso `[O, I, KH, KW]`; `x` `[N, H, W, I]`.
fn conv2d(p: &Parametros, n: &str, x: &Array, paso: (i32, i32), relleno: (i32, i32)) -> Result<Array> {
    let w = ops::transpose_axes(g(p, &format!("{n}.weight"))?, &[0, 2, 3, 1]).m()?;
    sesgo(p, n, ops::conv2d(x, &w, paso, relleno, (1, 1), 1).m()?)
}

/// ConvTranspose1d: peso `[I, O, K]`.
fn convt1d(p: &Parametros, n: &str, x: &Array, paso: i32) -> Result<Array> {
    let w = ops::transpose_axes(g(p, &format!("{n}.weight"))?, &[1, 2, 0]).m()?;
    sesgo(p, n, ops::conv_transpose1d(x, &w, paso, 0, 1, 0, 1).m()?)
}

/// ConvTranspose2d: peso `[I, O, KH, KW]`.
fn convt2d(p: &Parametros, n: &str, x: &Array, paso: (i32, i32)) -> Result<Array> {
    let w = ops::transpose_axes(g(p, &format!("{n}.weight"))?, &[1, 2, 3, 0]).m()?;
    sesgo(p, n, ops::conv_transpose2d(x, &w, paso, (0, 0), (1, 1), (0, 0), 1).m()?)
}

/// Conv1d de núcleo 1 (`[O, I, 1]`) como lineal sobre el último eje.
fn conv_lineal(p: &Parametros, n: &str, x: &Array) -> Result<Array> {
    let w = g(p, &format!("{n}.weight"))?;
    let w = ops::reshape(w, &[w.dim(0), -1]).m()?;
    sesgo(p, n, ops::matmul(x, &ops::transpose(&w).m()?).m()?)
}

/// `nn.Linear`: peso `[O, I]`.
fn lineal(p: &Parametros, n: &str, x: &Array) -> Result<Array> {
    sesgo(p, n, ops::matmul(x, &ops::transpose(g(p, &format!("{n}.weight"))?).m()?).m()?)
}

fn norma(p: &Parametros, n: &str, x: &Array) -> Result<Array> {
    fast::layer_norm(x, Some(g(p, &format!("{n}.weight"))?), Some(g(p, &format!("{n}.bias"))?), 1e-5).m()
}

/// `GroupNorm(1, C)` sobre `[N, …, C]`.
fn grupo(p: &Parametros, n: &str, x: &Array) -> Result<Array> {
    let ejes: Vec<i32> = (1..x.ndim() as i32).collect();
    let media = x.mean_axes(&ejes, true).m()?;
    let c = ops::subtract(x, &media).m()?;
    let var = ops::square(&c).m()?.mean_axes(&ejes, true).m()?;
    let y = ops::multiply(&c, &ops::rsqrt(&ops::add(&var, Array::from_f32(1e-5)).m()?).m()?).m()?;
    ops::add(&ops::multiply(&y, g(p, &format!("{n}.weight"))?).m()?, g(p, &format!("{n}.bias"))?).m()
}

/// La caída (`nn.Dropout`) del entrenamiento: con `p` = 0 o sin llave, la identidad.
pub struct Caida {
    p: f32,
    llave: RefCell<Option<Array>>,
}

impl Caida {
    pub fn ninguna() -> Caida {
        Caida { p: 0.0, llave: RefCell::new(None) }
    }
    pub fn con(p: f32, llave: Array) -> Caida {
        Caida { p, llave: RefCell::new(Some(llave)) }
    }
    fn activa(&self) -> bool {
        self.p > 0.0 && self.llave.borrow().is_some()
    }
    fn ap(&self, x: &Array) -> Result<Array> {
        if !self.activa() {
            return Ok(x.clone());
        }
        let mut l = self.llave.borrow_mut();
        let (siguiente, esta) = mlx_rs::random::split(l.as_ref().unwrap(), 2).m()?;
        *l = Some(siguiente);
        let queda = mlx_rs::random::bernoulli(&Array::from_f32(1.0 - self.p), x.shape(), &esta).m()?;
        ops::multiply(&ops::multiply(x, &queda).m()?, Array::from_f32(1.0 / (1.0 - self.p))).m()
    }
}

/// `DConv` sobre `[N, T, C]`.
fn dconv(p: &Parametros, n: &str, x: &Array) -> Result<Array> {
    let mut x = x.clone();
    for d in 0..2 {
        let m = format!("{n}.layers.{d}");
        let dil = 1 << d;
        let y = gelu(&grupo(p, &format!("{m}.1"), &conv1d(p, &format!("{m}.0"), &x, 1, dil, dil)?)?)?;
        let y = glu(&grupo(p, &format!("{m}.4"), &conv1d(p, &format!("{m}.3"), &y, 1, 0, 1)?)?)?;
        x = ops::add(&x, &ops::multiply(&y, g(p, &format!("{m}.6.scale"))?).m()?).m()?;
    }
    Ok(x)
}

/// La rama residual de una capa de frecuencia: el `DConv` corre sobre el tiempo de CADA frecuencia.
fn dconv_frec(p: &Parametros, n: &str, x: &Array) -> Result<Array> {
    let (b, f, t, c) = (x.dim(0), x.dim(1), x.dim(2), x.dim(3));
    let y = dconv(p, n, &ops::reshape(x, &[b * f, t, c]).m()?)?;
    ops::reshape(&y, &[b, f, t, c]).m()
}

fn codificador(p: &Parametros, n: &str, x: &Array, frec: bool) -> Result<Array> {
    if frec {
        let y = gelu(&conv2d(p, &format!("{n}.conv"), x, (4, 1), (2, 0))?)?;
        let y = dconv_frec(p, &format!("{n}.dconv"), &y)?;
        glu(&conv2d(p, &format!("{n}.rewrite"), &y, (1, 1), (0, 0))?)
    } else {
        let le = x.dim(1);
        let x = if le % 4 != 0 { ops::pad(x, &[(0, 0), (0, 4 - le % 4), (0, 0)], None, None).m()? } else { x.clone() };
        let y = gelu(&conv1d(p, &format!("{n}.conv"), &x, 4, 2, 1)?)?;
        let y = dconv(p, &format!("{n}.dconv"), &y)?;
        glu(&conv1d(p, &format!("{n}.rewrite"), &y, 1, 0, 1)?)
    }
}

fn decodificador(p: &Parametros, n: &str, x: &Array, salto: &Array, largo: i32, frec: bool, ultimo: bool) -> Result<Array> {
    let x = ops::add(x, salto).m()?;
    let z = if frec {
        let y = glu(&conv2d(p, &format!("{n}.rewrite"), &x, (1, 1), (1, 1))?)?;
        let y = dconv_frec(p, &format!("{n}.dconv"), &y)?;
        let z = convt2d(p, &format!("{n}.conv_tr"), &y, (4, 1))?;
        let f = z.dim(1);
        z.try_index((.., 2..f - 2)).m()?
    } else {
        let y = glu(&conv1d(p, &format!("{n}.rewrite"), &x, 1, 1, 1)?)?;
        let y = dconv(p, &format!("{n}.dconv"), &y)?;
        let z = convt1d(p, &format!("{n}.conv_tr"), &y, 4)?;
        z.try_index((.., 2..2 + largo)).m()?
    };
    if ultimo {
        Ok(z)
    } else {
        gelu(&z)
    }
}

/// Cómo se calcula la atención.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Atencion {
    /// `fast::scaled_dot_product_attention` (menos memoria; con caída, sólo pierde la de las probabilidades)
    Fusionada,
    /// `softmax(q·kᵀ/√d)·v` a mano (admite la caída de las probabilidades)
    Explicita,
}

/// `nn.MultiheadAttention(q, k, k)` con `batch_first`.
fn atencion(p: &Parametros, n: &str, q: &Array, k: &Array, cabezas: i32, caida: &Caida, modo: Atencion) -> Result<Array> {
    let d = q.dim(2);
    let dh = d / cabezas;
    let w = g(p, &format!("{n}.in_proj_weight"))?.split_equal(3, 0).m()?;
    let b = g(p, &format!("{n}.in_proj_bias"))?.split_equal(3, 0).m()?;
    let cab = |x: &Array, i: usize| -> Result<Array> {
        let y = ops::add(&ops::matmul(x, &ops::transpose(&w[i]).m()?).m()?, &b[i]).m()?;
        ops::transpose_axes(&ops::reshape(&y, &[x.dim(0), x.dim(1), cabezas, dh]).m()?, &[0, 2, 1, 3]).m()
    };
    let (qh, kh, vh) = (cab(q, 0)?, cab(k, 1)?, cab(k, 2)?);
    let escala = 1.0 / (dh as f32).sqrt();
    // fusionada: sin la caída de las probabilidades (las otras tres caídas siguen activas)
    let a = if modo == Atencion::Fusionada {
        fast::scaled_dot_product_attention(&qh, &kh, &vh, escala, None, None::<&Array>).m()?
    } else {
        let s = ops::multiply(&ops::matmul(&qh, &ops::transpose_axes(&kh, &[0, 1, 3, 2]).m()?).m()?, Array::from_f32(escala)).m()?;
        let s = caida.ap(&ops::softmax_axis(&s, -1, true).m()?)?;
        ops::matmul(&s, &vh).m()?
    };
    let a = ops::reshape(&ops::transpose_axes(&a, &[0, 2, 1, 3]).m()?, &[q.dim(0), q.dim(1), d]).m()?;
    lineal(p, &format!("{n}.out_proj"), &a)
}

/// Una capa del transformador (propia o cruzada), norma antes, LayerScale, GroupNorm de salida.
fn capa(p: &Parametros, n: &str, x: &Array, otra: Option<&Array>, cabezas: i32, caida: &Caida, modo: Atencion) -> Result<Array> {
    let (a, norma_ffn) = match otra {
        Some(k) => (atencion(p, &format!("{n}.cross_attn"), &norma(p, &format!("{n}.norm1"), x)?, &norma(p, &format!("{n}.norm2"), k)?, cabezas, caida, modo)?, "norm3"),
        None => {
            let h = norma(p, &format!("{n}.norm1"), x)?;
            (atencion(p, &format!("{n}.self_attn"), &h, &h, cabezas, caida, modo)?, "norm2")
        }
    };
    let x = ops::add(x, &ops::multiply(&caida.ap(&a)?, g(p, &format!("{n}.gamma_1.scale"))?).m()?).m()?;
    let h = norma(p, &format!("{n}.{norma_ffn}"), &x)?;
    let h = caida.ap(&gelu(&lineal(p, &format!("{n}.linear1"), &h)?)?)?;
    let f = caida.ap(&lineal(p, &format!("{n}.linear2"), &h)?)?;
    let x = ops::add(&x, &ops::multiply(&f, g(p, &format!("{n}.gamma_2.scale"))?).m()?).m()?;
    grupo(p, &format!("{n}.norm_out"), &x)
}

/// La posición en la señal ORIGINAL de cada muestra de los cuadros que usa `_spec` (los `le` del centro):
/// el reflejo de `_spec` (1536 + lo que falte) y el de `stft(center=True)` (2048), compuestos.
fn indice_cuadros(largo: usize) -> (Vec<i32>, usize) {
    let le = largo.div_ceil(SALTO);
    let refleja = |i: i64, n: i64| -> i64 {
        let mut i = i;
        // el reflejo de torch (sin repetir el borde); con relleno < n basta una vuelta
        if i < 0 {
            i = -i;
        }
        if i >= n {
            i = 2 * (n - 1) - i;
        }
        i.clamp(0, n - 1)
    };
    let pad = (SALTO / 2 * 3) as i64;
    let la = (le * SALTO + 2 * SALTO / 2 * 3) as i64;
    let mut idx = Vec::with_capacity(le * NFFT);
    for j in 0..le {
        for t in 0..NFFT {
            let q = ((j + 2) * SALTO + t) as i64 - (NFFT / 2) as i64;
            idx.push(refleja(refleja(q, la) - pad, largo as i64) as i32);
        }
    }
    (idx, le)
}

/// `_spec` por lotes en la GPU: `[N, L]` → (real, imaginaria) `[N, le, 2048]`.
pub fn espectro_lote(x: &Array) -> Result<(Array, Array, usize)> {
    let (n, l) = (x.dim(0), x.dim(1) as usize);
    let (idx, le) = indice_cuadros(l);
    let idx = Array::from_slice(&idx, &[(le * NFFT) as i32]);
    let c = ops::reshape(&x.take_axis(&idx, 1).m()?, &[n, le as i32, NFFT as i32]).m()?;
    let c = ops::multiply(&c, &Array::from_slice(&hann(), &[1, 1, NFFT as i32])).m()?;
    let z = mlx_rs::fft::rfft(&c, NFFT as i32, -1).m()?;
    let z = z.try_index((.., .., 0..(NFFT / 2) as i32)).m()?;
    let esc = Array::from_f32(1.0 / (NFFT as f32).sqrt());
    Ok((ops::multiply(&z.real().m()?, &esc).m()?, ops::multiply(&z.imag().m()?, &esc).m()?, le))
}

/// `HTDemucs.forward` (modo entrenamiento, sin relleno al segmento): mezcla `[B, 2, L]` → `[B, S, 2, L]`.
pub fn adelante(p: &Parametros, forma: Forma, mezcla: &Array, caida: &Caida, modo: Atencion) -> Result<Array> {
    let (bt, largo) = (mezcla.dim(0), mezcla.dim(2));
    let s = forma.fuentes as i32;
    // ── rama espectral: [B, F, T, 4] con (c0 re, c0 im, c1 re, c1 im) ──
    let (re, im, le) = espectro_lote(&ops::reshape(mezcla, &[bt * 2, largo]).m()?)?;
    let re = ops::reshape(&re, &[bt, 2, le as i32, (NFFT / 2) as i32]).m()?;
    let im = ops::reshape(&im, &[bt, 2, le as i32, (NFFT / 2) as i32]).m()?;
    let z = ops::stack(&[re.try_index((.., 0)).m()?, im.try_index((.., 0)).m()?, re.try_index((.., 1)).m()?, im.try_index((.., 1)).m()?], -1).m()?;
    let z = ops::transpose_axes(&z, &[0, 2, 1, 3]).m()?;
    let (media, desv) = media_desv(&z)?;
    let mut x = ops::divide(&ops::subtract(&z, &media).m()?, &ops::add(&desv, Array::from_f32(1e-5)).m()?).m()?;
    // ── rama temporal: [B, L, 2] ──
    let xt0 = ops::transpose_axes(mezcla, &[0, 2, 1]).m()?;
    let (media_t, desv_t) = media_desv(&xt0)?;
    let mut xt = ops::divide(&ops::subtract(&xt0, &media_t).m()?, &ops::add(&desv_t, Array::from_f32(1e-5)).m()?).m()?;
    // ── codificadores ──
    let mut saltos = Vec::new();
    let mut saltos_t = Vec::new();
    let mut largos_t = Vec::new();
    for i in 0..4 {
        largos_t.push(xt.dim(1));
        xt = codificador(p, &format!("tencoder.{i}"), &xt, false)?;
        saltos_t.push(xt.clone());
        x = codificador(p, &format!("encoder.{i}"), &x, true)?;
        if i == 0 {
            // `ScaledEmbedding` (×10) por `freq_emb` (0,2)
            let emb = ops::multiply(g(p, "freq_emb.embedding.weight")?, Array::from_f32(10.0 * 0.2)).m()?;
            x = ops::add(&x, &ops::expand_dims(&ops::expand_dims(&emb, 0).m()?, 2).m()?).m()?;
        }
        saltos.push(x.clone());
    }
    // ── transformador cruzado ──
    let (fr, t1) = (x.dim(1), x.dim(2));
    if forma.fondo {
        x = conv_lineal(p, "channel_upsampler", &x)?;
        xt = conv_lineal(p, "channel_upsampler_t", &xt)?;
    }
    let c = x.dim(3);
    let mut tok = ops::reshape(&ops::transpose_axes(&x, &[0, 2, 1, 3]).m()?, &[bt, t1 * fr, c]).m()?;
    tok = ops::add(&norma(p, "crosstransformer.norm_in", &tok)?, &embebido_2d(c as usize, fr as usize, t1 as usize)).m()?;
    let t2 = xt.dim(1);
    let mut tokt = ops::add(&norma(p, "crosstransformer.norm_in_t", &xt)?, &embebido_1d(t2 as usize, c as usize)).m()?;
    for i in 0..5 {
        let (n, nt) = (format!("crosstransformer.layers.{i}"), format!("crosstransformer.layers_t.{i}"));
        if i % 2 == 0 {
            tok = capa(p, &n, &tok, None, forma.cabezas, caida, modo)?;
            tokt = capa(p, &nt, &tokt, None, forma.cabezas, caida, modo)?;
        } else {
            let viejo = tok.clone();
            tok = capa(p, &n, &tok, Some(&tokt), forma.cabezas, caida, modo)?;
            tokt = capa(p, &nt, &tokt, Some(&viejo), forma.cabezas, caida, modo)?;
        }
    }
    x = ops::transpose_axes(&ops::reshape(&tok, &[bt, t1, fr, c]).m()?, &[0, 2, 1, 3]).m()?;
    xt = tokt;
    if forma.fondo {
        x = conv_lineal(p, "channel_downsampler", &x)?;
        xt = conv_lineal(p, "channel_downsampler_t", &xt)?;
    }
    // ── decodificadores ──
    for i in 0..4 {
        x = decodificador(p, &format!("decoder.{i}"), &x, &saltos.pop().unwrap(), 0, true, i == 3)?;
        xt = decodificador(p, &format!("tdecoder.{i}"), &xt, &saltos_t.pop().unwrap(), largos_t.pop().unwrap(), false, i == 3)?;
    }
    // ── de vuelta a la onda ──
    let x = ops::add(&ops::multiply(&x, &desv).m()?, &media).m()?; // [B, F, T, S·4]
    let x = ops::reshape(&x, &[bt, x.dim(1), x.dim(2), s * 2, 2]).m()?;
    let x = ops::transpose_axes(&x, &[0, 3, 2, 1, 4]).m()?; // [B, S·2, T, F, 2]
    let x = ops::reshape(&x, &[bt * s * 2, x.dim(2), x.dim(3), 2]).m()?;
    let partes = x.split_equal(2, -1).m()?;
    let re = ops::squeeze_axes(&partes[0], &[-1]).m()?;
    let im = ops::squeeze_axes(&partes[1], &[-1]).m()?;
    let ondas = ops::reshape(&inverso(&re, &im, le, largo as usize)?, &[bt, s, 2, largo]).m()?;
    let xt = ops::add(&ops::multiply(&xt, &desv_t).m()?, &media_t).m()?; // [B, L, S·2]
    let xt = ops::reshape(&ops::transpose_axes(&xt, &[0, 2, 1]).m()?, &[bt, s, 2, largo]).m()?;
    ops::add(&ondas, &xt).m()
}

/// La pérdida de `solver.py` con `optim.loss = l1`: `|est − fuentes|` medio en canales y tiempo, medio en
/// el lote, y la media ponderada por fuente. Devuelve también la de cada fuente (`[S]`).
pub fn perdida_l1(est: &Array, fuentes: &Array, pesos: &[f32]) -> Result<(Array, Array)> {
    let l = ops::abs(&ops::subtract(est, fuentes).m()?).m()?.mean_axes(&[2, 3], false).m()?.mean_axes(&[0], false).m()?;
    if pesos.len() as i32 != l.dim(0) {
        bail_tr!("hay {} pesos para {} fuentes", "there are {} weights for {} sources", pesos.len(), l.dim(0));
    }
    let w = Array::from_slice(pesos, &[pesos.len() as i32]);
    let total = ops::divide(&ops::sum(&ops::multiply(&l, &w).m()?, None).m()?, Array::from_f32(pesos.iter().sum())).m()?;
    Ok((total, l))
}

/// La pérdida y su gradiente respecto a TODOS los parámetros (un paso de entrenamiento sin el optimizador).
pub fn perdida_y_gradiente(m: &Modelo, mezcla: &Array, fuentes: &Array, pesos: &[f32], llave: Option<Array>, modo: Atencion) -> Result<(Array, Parametros)> {
    let forma = m.forma;
    let p_caida = m.caida;
    let f = move |p: HashMap<Rc<str>, Array>, (mezcla, fuentes, llave): (Array, Array, Option<Array>)| -> std::result::Result<Vec<Array>, Exception> {
        let caida = match llave {
            Some(k) => Caida::con(p_caida, k),
            None => Caida::ninguna(),
        };
        let r = adelante(&p, forma, &mezcla, &caida, modo).and_then(|est| perdida_l1(&est, &fuentes, pesos));
        r.map(|(t, _)| vec![t]).map_err(|e| Exception::custom(format!("{e:#}")))
    };
    let mut vg = mlx_rs::transforms::keyed_value_and_grad(f);
    let (v, gr) = vg(m.parametros.clone(), (mezcla.clone(), fuentes.clone(), llave)).map_err(|e| anyhow!("{e}"))?;
    Ok((v.into_iter().next().ok_or_else(|| anyhow!("sin pérdida"))?, gr)) // juez-idioma: datos (no ocurre: la función devuelve un valor)
}

/// `torch.nn.utils.clip_grad_norm_`: la norma GLOBAL de todos los gradientes; si pasa de `tope`, todos se
/// escalan por `tope / (norma + 1e-6)`. Devuelve la norma de antes.
pub fn recortar(gr: &mut Parametros, tope: f32) -> Result<f32> {
    let mut total: Option<Array> = None;
    for a in gr.values() {
        let s = ops::sum(&ops::square(a).m()?, None).m()?;
        total = Some(match total {
            None => s,
            Some(t) => ops::add(&t, &s).m()?,
        });
    }
    let norma = ops::sqrt(&total.ok_or_else(|| anyhow!("sin gradientes"))?).m()?.item_exact::<f32>(); // juez-idioma: datos (no ocurre: el modelo tiene parámetros)
    if tope > 0.0 {
        let coef = (tope / (norma + 1e-6)).min(1.0);
        if coef < 1.0 {
            let c = Array::from_f32(coef);
            for a in gr.values_mut() {
                *a = ops::multiply(&*a, &c).m()?;
            }
        }
    }
    Ok(norma)
}

/// `torch.optim.AdamW` EXACTO (el de mlx-rs no corrige el sesgo de los momentos, y en los primeros pasos
/// salta ~3× más): `p ← p·(1 − lr·wd)`; `m, v` con `betas`; `p ← p − (lr/bc₁)·m / (√v/√bc₂ + eps)`.
/// El decaimiento va POR GRUPO, como `demucs/train.py::get_optimizer`: el transformador lleva el suyo
/// (`t_weight_decay`) y lo demás el general.
pub struct AdamW {
    pub lr: f32,
    pub betas: (f32, f32),
    pub eps: f32,
    /// decaimiento de `crosstransformer.*` y del resto
    pub decaimiento: (f32, f32),
    pub paso: i32,
    m: Parametros,
    v: Parametros,
}

/// La clave del paso en los metadatos del estado del optimizador.
const CLAVE_PASO: &str = "paso"; // juez-idioma: datos (clave de los metadatos del punto de control)

impl AdamW {
    /// La receta del afinado oficial (`grids/mmi_ft.py`): lr 1e-4, betas (0,9; 0,999), decaimiento 0,05
    /// en el transformador y 0 en lo demás.
    pub fn afinado(lr: f32) -> AdamW {
        AdamW { lr, betas: (0.9, 0.999), eps: 1e-8, decaimiento: (0.05, 0.0), paso: 0, m: HashMap::new(), v: HashMap::new() }
    }

    pub fn aplicar(&mut self, p: &mut Parametros, gr: &Parametros) -> Result<()> {
        self.paso += 1;
        let (b1, b2) = self.betas;
        let bc1 = 1.0 - b1.powi(self.paso);
        let bc2 = 1.0 - b2.powi(self.paso);
        let (a_b1, a_1b1, a_b2, a_1b2) = (Array::from_f32(b1), Array::from_f32(1.0 - b1), Array::from_f32(b2), Array::from_f32(1.0 - b2));
        let raiz_bc2 = Array::from_f32(bc2.sqrt());
        let eps = Array::from_f32(self.eps);
        let salto = Array::from_f32(self.lr / bc1);
        for (n, g) in gr {
            let par = p.get_mut(n).ok_or_else(|| anyhow_tr!("gradiente sin parámetro: {n}", "gradient without parameter: {n}"))?;
            let wd = if n.starts_with("crosstransformer.") { self.decaimiento.0 } else { self.decaimiento.1 };
            let mut x = par.clone();
            if wd != 0.0 {
                x = ops::multiply(&x, Array::from_f32(1.0 - self.lr * wd)).m()?;
            }
            let m = match self.m.get(n) {
                Some(m0) => ops::add(&ops::multiply(m0, &a_b1).m()?, &ops::multiply(g, &a_1b1).m()?).m()?,
                None => ops::multiply(g, &a_1b1).m()?,
            };
            let g2 = ops::square(g).m()?;
            let v = match self.v.get(n) {
                Some(v0) => ops::add(&ops::multiply(v0, &a_b2).m()?, &ops::multiply(&g2, &a_1b2).m()?).m()?,
                None => ops::multiply(&g2, &a_1b2).m()?,
            };
            let den = ops::add(&ops::divide(&ops::sqrt(&v).m()?, &raiz_bc2).m()?, &eps).m()?;
            *par = ops::subtract(&x, &ops::multiply(&ops::divide(&m, &den).m()?, &salto).m()?).m()?;
            self.m.insert(n.clone(), m);
            self.v.insert(n.clone(), v);
        }
        Ok(())
    }

    /// Los momentos, para evaluarlos junto con los parámetros (y no acumular el grafo).
    pub fn estado(&self) -> impl Iterator<Item = &Array> {
        self.m.values().chain(self.v.values())
    }
}

/// Lo que deja un paso de entrenamiento.
#[derive(Clone, Copy, Debug)]
pub struct Paso {
    pub perdida: f32,
    /// la norma global del gradiente ANTES del recorte
    pub norma: f32,
}

/// UN paso de entrenamiento: pérdida y gradiente, recorte de norma (`tope`; 5 en el afinado oficial),
/// AdamW, y se evalúan parámetros y momentos (para que el grafo no crezca de un paso al siguiente).
#[allow(clippy::too_many_arguments)]
pub fn paso(m: &mut Modelo, opt: &mut AdamW, mezcla: &Array, fuentes: &Array, pesos: &[f32], llave: Option<Array>, modo: Atencion, tope: f32) -> Result<Paso> {
    let (v, mut gr) = perdida_y_gradiente(m, mezcla, fuentes, pesos, llave, modo)?;
    let norma = recortar(&mut gr, tope)?;
    let perdida = v.item_exact::<f32>();
    opt.aplicar(&mut m.parametros, &gr)?;
    drop(gr);
    mlx_rs::transforms::eval(m.parametros.values().chain(opt.estado())).m()?;
    Ok(Paso { perdida, norma })
}

/// UN paso del optimizador con el gradiente ACUMULADO de varios sublotes (el lote efectivo del oficial sin
/// su memoria): la media de los gradientes de cada `(mezcla, fuentes)`, recorte y AdamW. La pérdida es la
/// media de los sublotes.
#[allow(clippy::too_many_arguments)]
pub fn paso_acumulado(m: &mut Modelo, opt: &mut AdamW, sublotes: &[(Array, Array)], pesos: &[f32], llaves: Vec<Option<Array>>, modo: Atencion, tope: f32) -> Result<Paso> {
    if sublotes.is_empty() || llaves.len() != sublotes.len() {
        bail_tr!("sublotes y llaves no cuadran", "sub-batches and keys do not match");
    }
    let n = sublotes.len() as f32;
    let mut suma: Option<Parametros> = None;
    let mut perdida = 0f32;
    for ((mezcla, fuentes), llave) in sublotes.iter().zip(llaves) {
        let (v, gr) = perdida_y_gradiente(m, mezcla, fuentes, pesos, llave, modo)?;
        let s = match suma.take() {
            None => gr,
            Some(mut s) => {
                for (k, g) in gr {
                    let a = s.get_mut(&k).ok_or_else(|| anyhow!("{k}"))?;
                    *a = ops::add(&*a, &g).m()?;
                }
                s
            }
        };
        // se evalúa cada vez: así sólo vive el grafo de UN sublote
        mlx_rs::transforms::eval(s.values().chain(std::iter::once(&v))).m()?;
        perdida += v.item_exact::<f32>();
        suma = Some(s);
    }
    let mut gr = suma.unwrap();
    if n > 1.0 {
        let c = Array::from_f32(1.0 / n);
        for a in gr.values_mut() {
            *a = ops::multiply(&*a, &c).m()?;
        }
    }
    let norma = recortar(&mut gr, tope)?;
    opt.aplicar(&mut m.parametros, &gr)?;
    drop(gr);
    mlx_rs::transforms::eval(m.parametros.values().chain(opt.estado())).m()?;
    Ok(Paso { perdida: perdida / n, norma })
}

/// La media móvil exponencial de los pesos (`ema.batch` del oficial): `e ← d·e + (1−d)·p`.
pub struct Media {
    pub decaimiento: f32,
    pub p: Parametros,
}

impl Media {
    pub fn de(m: &Modelo, decaimiento: f32) -> Media {
        Media { decaimiento, p: m.parametros.clone() }
    }
    pub fn actualizar(&mut self, m: &Modelo) -> Result<()> {
        let (d, u) = (Array::from_f32(self.decaimiento), Array::from_f32(1.0 - self.decaimiento));
        for (k, e) in self.p.iter_mut() {
            let p = m.parametros.get(k).ok_or_else(|| anyhow!("{k}"))?;
            *e = ops::add(&ops::multiply(&*e, &d).m()?, &ops::multiply(p, &u).m()?).m()?;
        }
        mlx_rs::transforms::eval(self.p.values()).m()
    }
}

impl Modelo {
    /// Escribe OTROS parámetros (p. ej. la media móvil) con los metadatos de este modelo.
    pub fn guardar_con(&self, p: &Parametros, ruta: &Path, tipo: Dtype) -> Result<()> {
        let mut v: Vec<(String, Array)> = Vec::with_capacity(p.len());
        for (n, a) in p {
            v.push((n.to_string(), a.as_dtype(tipo).m()?));
        }
        v.sort_by(|a, b| a.0.cmp(&b.0));
        Array::save_safetensors(v.iter().map(|(n, a)| (n.as_str(), a)), &self.meta, ruta).map_err(|e| anyhow!("{}: {e:?}", ruta.display()))
    }

    /// Una carpeta que lee `separador::Separador::cargar`: `<nombre>.safetensors` + el YAML de la bolsa.
    pub fn escribir_bolsa(&self, p: &Parametros, dir: &Path, nombre: &str, tipo: Dtype) -> Result<()> {
        std::fs::create_dir_all(dir)?;
        self.guardar_con(p, &dir.join(format!("{nombre}.safetensors")), tipo)?;
        std::fs::write(dir.join("afinado.yaml"), format!("models: ['{nombre}']\n"))?;
        Ok(())
    }

    /// Carga parámetros guardados (un punto de control) encima de los de este modelo: mismos nombres y formas.
    pub fn cargar_parametros(&mut self, ruta: &Path) -> Result<()> {
        let (p, _) = Array::load_safetensors_with_metadata(ruta).map_err(|e| anyhow!("{}: {e:?}", ruta.display()))?;
        for (n, a) in p {
            let viejo = self.parametros.get_mut(n.as_str()).ok_or_else(|| anyhow_tr!("parámetro ajeno al modelo: {n}", "parameter foreign to the model: {n}"))?;
            if viejo.shape() != a.shape() {
                bail_tr!("{n}: forma {:?} en vez de {:?}", "{n}: shape {:?} instead of {:?}", a.shape(), viejo.shape());
            }
            *viejo = a.as_dtype(F32).m()?;
        }
        Ok(())
    }
}

impl AdamW {
    /// Guarda los momentos y el paso (para reanudar un afinado largo).
    pub fn guardar(&self, ruta: &Path) -> Result<()> {
        let mut v: Vec<(String, &Array)> = Vec::new();
        for (n, a) in &self.m {
            v.push((format!("m.{n}"), a));
        }
        for (n, a) in &self.v {
            v.push((format!("v.{n}"), a));
        }
        let meta: HashMap<String, String> = [(CLAVE_PASO.to_string(), self.paso.to_string())].into_iter().collect();
        Array::save_safetensors(v.iter().map(|(n, a)| (n.as_str(), *a)), &meta, ruta).map_err(|e| anyhow!("{}: {e:?}", ruta.display()))
    }

    pub fn cargar(&mut self, ruta: &Path) -> Result<()> {
        let (p, meta) = Array::load_safetensors_with_metadata(ruta).map_err(|e| anyhow!("{}: {e:?}", ruta.display()))?;
        self.paso = meta.get(CLAVE_PASO).and_then(|s| s.parse().ok()).ok_or_else(|| anyhow!("{CLAVE_PASO}"))?;
        for (n, a) in p {
            if let Some(k) = n.strip_prefix("m.") {
                self.m.insert(Rc::from(k), a);
            } else if let Some(k) = n.strip_prefix("v.") {
                self.v.insert(Rc::from(k), a);
            }
        }
        Ok(())
    }
}

/// Un lote `[B, S, 2, L]` (y su mezcla `[B, 2, L]`) a partir de ejemplos aplanados `[S, 2, L]`.
pub fn lote(ejemplos: &[Vec<f32>], s: usize, largo: usize) -> Result<(Array, Array)> {
    let b = ejemplos.len();
    let mut v = Vec::with_capacity(b * s * 2 * largo);
    for e in ejemplos {
        if e.len() != s * 2 * largo {
            bail_tr!("un ejemplo de {} valores en vez de {}", "an example of {} values instead of {}", e.len(), s * 2 * largo);
        }
        v.extend_from_slice(e);
    }
    let fuentes = Array::from_slice(&v, &[b as i32, s as i32, 2, largo as i32]);
    let mezcla = ops::sum_axes(&fuentes, &[1], false).m()?;
    Ok((mezcla, fuentes))
}

/// Una mezcla de `[2][L]` en un lote de 1 (para las pruebas y la inferencia del afinado).
pub fn lote_de(x: &[Vec<f32>; 2]) -> Array {
    let l = x[0].len();
    let mut v = Vec::with_capacity(2 * l);
    v.extend_from_slice(&x[0]);
    v.extend_from_slice(&x[1]);
    Array::from_slice(&v, &[1, 2, l as i32])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::separador::espectro;
    use std::path::PathBuf;

    /// `espectro` (el de la inferencia) contra `espectro_lote`: la mayor diferencia.
    fn _comprobar_espectro(x: &[Vec<f32>; 2]) -> Result<f32> {
        let (re, im, _) = espectro(x)?;
        let (re2, im2, _) = espectro_lote(&ops::squeeze_axes(&lote_de(x), &[0]).m()?)?;
        let d = ops::maximum(&ops::abs(&ops::subtract(&re, &re2).m()?).m()?.max(None).m()?, &ops::abs(&ops::subtract(&im, &im2).m()?).m()?.max(None).m()?).m()?;
        Ok(d.item_exact::<f32>())
    }

    fn senal(l: usize) -> [Vec<f32>; 2] {
        std::array::from_fn(|c| (0..l).map(|i| (i as f32 * 0.013 * (c + 1) as f32).sin() * 0.5 + ((i * 7919 % 1000) as f32 / 1000.0 - 0.5) * 0.1).collect())
    }

    #[test]
    fn la_stft_por_lotes_es_la_de_la_inferencia() {
        for l in [30_000usize, 44_100, 2048 * 5] {
            let d = _comprobar_espectro(&senal(l)).unwrap();
            assert!(d < 1e-5, "L={l}: la STFT en la GPU se aparta {d}");
        }
    }

    /// El mapa entrenable da la MISMA separación que la inferencia (`separador::Htdemucs`) con los
    /// mismos pesos (sin caída, atención fusionada y explícita).
    #[test]
    fn el_mapa_separa_igual_que_la_inferencia() {
        let ruta = crate::separador::carpeta("adefossez/HTDemucs").join("955717e8.safetensors");
        if !ruta.exists() {
            eprintln!("sin HTDemucs instalado: {}", ruta.display());
            return;
        }
        let x = senal(44_100);
        let inf = crate::separador::Htdemucs::cargar(&ruta).unwrap().adelante(&x).unwrap();
        let m = Modelo::cargar(&ruta).unwrap();
        for modo in [Atencion::Fusionada, Atencion::Explicita] {
            let y = adelante(&m.parametros, m.forma, &lote_de(&x), &Caida::ninguna(), modo).unwrap();
            y.eval().unwrap();
            let v = y.as_slice::<f32>();
            let l = x[0].len();
            let (mut senal, mut ruido) = (0f64, 0f64);
            for (s, pista) in inf.iter().enumerate() {
                for c in 0..2 {
                    for i in 0..l {
                        let a = v[(s * 2 + c) * l + i] as f64;
                        senal += (pista[c][i] as f64).powi(2);
                        ruido += (a - pista[c][i] as f64).powi(2);
                    }
                }
            }
            let snr = 10.0 * (senal / ruido.max(1e-30)).log10();
            eprintln!("{modo:?}: {snr:.1} dB contra la inferencia");
            assert!(snr > 100.0, "{modo:?}: {snr:.1} dB");
        }
    }

    /// JUEZ DE GRADIENTES contra torch (`juez_gradientes.py` con `demucs` 4.1): la misma pérdida L1 y el
    /// gradiente de CADA parámetro, sobre el lote que escribió el juez (`GOTV_DEMUCS_GRAD`).
    #[test]
    #[ignore]
    fn juez_de_gradientes() {
        let Some(arch) = std::env::var_os("GOTV_DEMUCS_GRAD").map(PathBuf::from) else { return };
        let repo = std::env::var("GOTV_DEMUCS_REPO").unwrap_or_else(|_| "adefossez/HTDemucs".into());
        let dir = crate::separador::carpeta(&repo);
        let yaml = std::fs::read_to_string(std::fs::read_dir(&dir).unwrap().filter_map(|e| e.ok().map(|e| e.path())).find(|p| p.extension().and_then(|e| e.to_str()) == Some("yaml")).unwrap()).unwrap();
        let nombre = yaml.split('\'').nth(1).unwrap().to_string();
        let m = Modelo::cargar(&dir.join(format!("{nombre}.safetensors"))).unwrap();
        let (j, _) = Array::load_safetensors_with_metadata(&arch).unwrap();
        let mezcla = j["mezcla"].clone();
        let fuentes = j["fuentes"].clone();
        let pesos: Vec<f32> = (0..j["pesos"].dim(0)).map(|i| j["pesos"].try_index(i).unwrap().item_exact::<f32>()).collect();
        let perdida_t = j["perdida"].item_exact::<f32>();
        for modo in [Atencion::Explicita, Atencion::Fusionada] {
            let t0 = std::time::Instant::now();
            let (v, gr) = perdida_y_gradiente(&m, &mezcla, &fuentes, &pesos, None, modo).unwrap();
            let gs: Vec<&Array> = gr.values().collect();
            mlx_rs::transforms::eval(gs).unwrap();
            let perdida = v.item_exact::<f32>();
            eprintln!("{modo:?}: pérdida {perdida:.7} (torch {perdida_t:.7}) en {:.2} s", t0.elapsed().as_secs_f64());
            assert!(((perdida - perdida_t) / perdida_t).abs() < 1e-4, "la pérdida difiere");
            let mut peor = (0f64, String::new());
            let mut faltan = 0;
            let (mut num, mut den) = (0f64, 0f64);
            for (n, a) in &gr {
                let Some(b) = j.get(&format!("grad.{n}")) else {
                    faltan += 1;
                    continue;
                };
                assert_eq!(a.shape(), b.shape(), "{n}");
                // con operaciones de MLX: los gradientes pueden ser vistas no contiguas
                let d2 = ops::sum(&ops::square(&ops::subtract(a, b).unwrap()).unwrap(), None).unwrap().item_exact::<f32>() as f64;
                let b2 = ops::sum(&ops::square(b).unwrap(), None).unwrap().item_exact::<f32>() as f64;
                num += d2;
                den += b2;
                let rel = (d2 / b2.max(1e-30)).sqrt();
                if rel > peor.0 && b2 > 1e-24 {
                    peor = (rel, n.to_string());
                }
            }
            let global = (num / den).sqrt();
            eprintln!("{modo:?}: {} parámetros; error relativo global {global:.2e}; peor tensor {:.2e} ({})", gr.len(), peor.0, peor.1);
            assert_eq!(faltan, 0, "parámetros sin gradiente de torch");
            assert!(global < 1e-4, "{modo:?}: el gradiente global se aparta {global:e}");
            assert!(peor.0 < 1e-2, "{modo:?}: {} se aparta {:e}", peor.1, peor.0);
        }
    }

    /// JUEZ DEL OPTIMIZADOR: con los gradientes de TORCH, el recorte de norma y un paso del AdamW propio
    /// dan los mismos parámetros nuevos que `clip_grad_norm_` + `torch.optim.AdamW` con los dos grupos.
    #[test]
    #[ignore]
    fn juez_del_optimizador() {
        let Some(arch) = std::env::var_os("GOTV_DEMUCS_GRAD").map(PathBuf::from) else { return };
        let repo = std::env::var("GOTV_DEMUCS_REPO").unwrap_or_else(|_| "adefossez/HTDemucs".into());
        let dir = crate::separador::carpeta(&repo);
        let yaml = std::fs::read_to_string(std::fs::read_dir(&dir).unwrap().filter_map(|e| e.ok().map(|e| e.path())).find(|p| p.extension().and_then(|e| e.to_str()) == Some("yaml")).unwrap()).unwrap();
        let nombre = yaml.split('\'').nth(1).unwrap().to_string();
        let mut m = Modelo::cargar(&dir.join(format!("{nombre}.safetensors"))).unwrap();
        let (j, _) = Array::load_safetensors_with_metadata(&arch).unwrap();
        let mut gr: Parametros = m.parametros.keys().map(|n| (n.clone(), j[&format!("grad.{n}")].clone())).collect();
        let norma = recortar(&mut gr, j["tope"].item_exact::<f32>()).unwrap();
        let norma_t = j["norma"].item_exact::<f32>();
        eprintln!("norma {norma:.6e} (torch {norma_t:.6e})");
        assert!(((norma - norma_t) / norma_t).abs() < 1e-5);
        let viejos = m.parametros.clone();
        let mut opt = AdamW::afinado(1e-4);
        opt.aplicar(&mut m.parametros, &gr).unwrap();
        let (mut num, mut den, mut peor) = (0f64, 0f64, (0f64, String::new()));
        for (n, a) in &m.parametros {
            let b = &j[&format!("nuevo.{n}")];
            // se compara el SALTO (nuevo − viejo), no el parámetro: el salto es ~1e-4 del valor
            let da = ops::subtract(a, &viejos[n]).unwrap();
            let db = ops::subtract(b, &viejos[n]).unwrap();
            let d2 = ops::sum(&ops::square(&ops::subtract(&da, &db).unwrap()).unwrap(), None).unwrap().item_exact::<f32>() as f64;
            let b2 = ops::sum(&ops::square(&db).unwrap(), None).unwrap().item_exact::<f32>() as f64;
            num += d2;
            den += b2;
            let rel = (d2 / b2.max(1e-30)).sqrt();
            if rel > peor.0 {
                peor = (rel, n.to_string());
            }
        }
        let global = (num / den).sqrt();
        eprintln!("salto del AdamW: error relativo global {global:.2e}; peor tensor {:.2e} ({})", peor.0, peor.1);
        assert!(global < 1e-3 && peor.0 < 1e-2, "el AdamW se aparta del de torch");
    }

    /// MEDIR antes de prometer: tiempo y pico de memoria de un paso COMPLETO (gradiente + recorte +
    /// AdamW) con trozos del largo del afinado oficial. `GOTV_MEDIR_PASO=segundos:lote,…` (por omisión
    /// `10:1,10:2,10:4`); cada caso: 1 paso de calentamiento + 3 medidos, con caída y explícita (la del
    /// oficial) y sin caída y fusionada.
    #[test]
    #[ignore]
    fn medir_paso() {
        let Ok(casos) = std::env::var("GOTV_MEDIR_PASO") else { return };
        let ruta = crate::separador::carpeta("adefossez/HTDemucs").join("955717e8.safetensors");
        let gb = |b: usize| b as f64 / (1u64 << 30) as f64;
        for caso in casos.split(',') {
            let (seg, lote) = caso.split_once(':').unwrap();
            let (seg, lote): (f32, i32) = (seg.parse().unwrap(), lote.parse().unwrap());
            let l = (seg * FRECUENCIA as f32) as i32;
            for (modo, con_caida) in [(Atencion::Explicita, true), (Atencion::Fusionada, false)] {
                let mut m = Modelo::cargar(&ruta).unwrap();
                let mut opt = AdamW::afinado(1e-4);
                let fuentes = ops::multiply(&mlx_rs::random::normal::<f32>(&[lote, 4, 2, l], None, None, &mlx_rs::random::key(1).unwrap()).unwrap(), Array::from_f32(0.1)).unwrap();
                let mezcla = ops::sum_axes(&fuentes, &[1], false).unwrap();
                mlx_rs::transforms::eval([&fuentes, &mezcla]).unwrap();
                mlx_rs::memory::clear_cache().unwrap();
                mlx_rs::memory::reset_peak_memory().unwrap();
                let mut tiempos = Vec::new();
                for i in 0..4 {
                    let llave = con_caida.then(|| mlx_rs::random::key(100 + i as u64).unwrap());
                    let t0 = std::time::Instant::now();
                    let r = paso(&mut m, &mut opt, &mezcla, &fuentes, &[1.0; 4], llave, modo, 5.0).unwrap();
                    let t = t0.elapsed().as_secs_f64();
                    if i > 0 {
                        tiempos.push(t);
                    }
                    eprintln!("  {seg} s × {lote}, {modo:?}{}: paso {i}: {t:.2} s, pérdida {:.4}, norma {:.3}", if con_caida { " con caída" } else { "" }, r.perdida, r.norma);
                }
                let media = tiempos.iter().sum::<f64>() / tiempos.len() as f64;
                eprintln!("MEDIDA {seg} s × lote {lote} · {modo:?}{}: {media:.2} s/paso ({:.1} s de audio por s) · pico {:.1} GB", if con_caida { " con caída" } else { "" }, seg as f64 * lote as f64 / media, gb(mlx_rs::memory::peak_memory().unwrap()));
            }
        }
    }
}
