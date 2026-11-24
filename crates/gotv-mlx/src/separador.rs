//! PRO-6 · el SEPARADOR de pistas: **HTDemucs** (Hybrid Transformer Demucs v4, Meta; código MIT,
//! pesos de su autor en `adefossez/HTDemucs`, `-6s` y `-ft`) portado a MLX, como `demucs` 4.1:
//!
//! - **dos ramas**: la espectral (STFT 4096/1024 normalizada, complejo como canales —CaC—, 4 capas
//!   de convolución sobre la FRECUENCIA, cada una con su rama residual DConv sobre el TIEMPO) y la
//!   temporal (4 capas de convolución sobre la onda);
//! - en el fondo, el **transformador cruzado** (5 capas: propia, cruzada, propia, cruzada, propia;
//!   norma antes, LayerScale y GroupNorm de salida), con embebido sinusoidal 2D en la rama espectral
//!   y 1D en la temporal;
//! - los **decodificadores** espejo, con los saltos; la salida espectral vuelve por ISTFT y se SUMA a
//!   la temporal;
//! - `apply_model`: trozos de 7,8 s con 25 % de solape y peso triangular, el trozo final rellenado
//!   con su contexto real (`TensorChunk.padded`), y la normalización de `Separator` (media y desviación
//!   de la mezcla mono). Sin `shifts` (el oficial los elige al AZAR; con 0 es determinista).
//!
//! Disposición interna «canales al final»: la rama espectral es `[B, F, T, C]` (las convoluciones de
//! frecuencia son conv2d con núcleo `[k, 1]`), la temporal `[B, L, C]`.
use crate::modelo::{quitar, M};
use anyhow::{anyhow, Context, Result};
use gotv_engine::{anyhow_tr, bail_tr};
use mlx_rs::fast;
use mlx_rs::ops::indexing::TryIndexOp;
use mlx_rs::ops;
use mlx_rs::{Array, Dtype};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

pub const FRECUENCIA: u32 = 44_100;
pub(crate) const NFFT: usize = 4096;
pub(crate) const SALTO: usize = NFFT / 4;
pub(crate) const F32: Dtype = Dtype::Float32;

pub(crate) fn gelu(x: &Array) -> Result<Array> {
    let e = ops::erf(&ops::divide(x, Array::from_f32(std::f32::consts::SQRT_2)).m()?).m()?;
    ops::divide(&ops::multiply(x, &ops::add(Array::from_f32(1.0), &e).m()?).m()?, Array::from_f32(2.0)).m()
}

/// `F.glu(x, dim=canales)`: la primera mitad por la sigmoide de la segunda (canales al final).
pub(crate) fn glu(x: &Array) -> Result<Array> {
    let p = x.split_equal(2, -1).m()?;
    ops::multiply(&p[0], &ops::sigmoid(&p[1]).m()?).m()
}

/// `GroupNorm(1, C)` sobre `[N, …, C]`: media y varianza (sesgada) de TODO lo de cada muestra.
struct Grupo {
    w: Array,
    b: Array,
}

impl Grupo {
    fn de(p: &mut HashMap<String, Array>, n: &str) -> Result<Grupo> {
        Ok(Grupo { w: quitar(p, &format!("{n}.weight"))?, b: quitar(p, &format!("{n}.bias"))? })
    }
    fn ap(&self, x: &Array) -> Result<Array> {
        let ejes: Vec<i32> = (1..x.ndim() as i32).collect();
        let media = x.mean_axes(&ejes, true).m()?;
        let c = ops::subtract(x, &media).m()?;
        let var = ops::square(&c).m()?.mean_axes(&ejes, true).m()?;
        let n = ops::multiply(&c, &ops::rsqrt(&ops::add(&var, Array::from_f32(1e-5)).m()?).m()?).m()?;
        ops::add(&ops::multiply(&n, &self.w).m()?, &self.b).m()
    }
}

/// Una convolución con su sesgo, en el formato de MLX (`[O, …núcleo, I]`).
struct Conv {
    w: Array,
    b: Array,
}

#[derive(Clone, Copy)]
enum Tipo {
    /// Conv1d: `[O, I, K]` → `[O, K, I]`
    C1,
    /// Conv2d: `[O, I, KH, KW]` → `[O, KH, KW, I]`
    C2,
    /// ConvTranspose1d: `[I, O, K]` → `[O, K, I]`
    T1,
    /// ConvTranspose2d: `[I, O, KH, KW]` → `[O, KH, KW, I]`
    T2,
}

impl Conv {
    fn de(p: &mut HashMap<String, Array>, n: &str, t: Tipo) -> Result<Conv> {
        let w = quitar(p, &format!("{n}.weight"))?;
        let ejes: &[i32] = match t {
            Tipo::C1 => &[0, 2, 1],
            Tipo::C2 => &[0, 2, 3, 1],
            Tipo::T1 => &[1, 2, 0],
            Tipo::T2 => &[1, 2, 3, 0],
        };
        Ok(Conv { w: ops::transpose_axes(&w, ejes).m()?, b: quitar(p, &format!("{n}.bias"))? })
    }
    fn c1(&self, x: &Array, paso: i32, relleno: i32, dil: i32) -> Result<Array> {
        ops::add(&ops::conv1d(x, &self.w, paso, relleno, dil, 1).m()?, &self.b).m()
    }
    fn c2(&self, x: &Array, paso: (i32, i32), relleno: (i32, i32)) -> Result<Array> {
        ops::add(&ops::conv2d(x, &self.w, paso, relleno, (1, 1), 1).m()?, &self.b).m()
    }
    fn t1(&self, x: &Array, paso: i32) -> Result<Array> {
        ops::add(&ops::conv_transpose1d(x, &self.w, paso, 0, 1, 0, 1).m()?, &self.b).m()
    }
    fn t2(&self, x: &Array, paso: (i32, i32)) -> Result<Array> {
        ops::add(&ops::conv_transpose2d(x, &self.w, paso, (0, 0), (1, 1), (0, 0), 1).m()?, &self.b).m()
    }
    /// Conv1d de núcleo 1 aplicada como lineal sobre el último eje.
    fn lineal(&self, x: &Array) -> Result<Array> {
        let w = ops::reshape(&self.w, &[self.w.dim(0), -1]).m()?;
        ops::add(&ops::matmul(x, &ops::transpose(&w).m()?).m()?, &self.b).m()
    }
}

/// `DConv`: 2 capas residuales [conv k3 dilatada → GN → GELU → conv 1×1 → GN → GLU → LayerScale] sobre
/// `[N, T, C]`.
struct DConv {
    capas: Vec<(Conv, Grupo, Conv, Grupo, Array)>,
}

impl DConv {
    fn de(p: &mut HashMap<String, Array>, n: &str) -> Result<DConv> {
        let mut capas = Vec::new();
        for d in 0..2 {
            let m = format!("{n}.layers.{d}");
            capas.push((
                Conv::de(p, &format!("{m}.0"), Tipo::C1)?,
                Grupo::de(p, &format!("{m}.1"))?,
                Conv::de(p, &format!("{m}.3"), Tipo::C1)?,
                Grupo::de(p, &format!("{m}.4"))?,
                quitar(p, &format!("{m}.6.scale"))?,
            ));
        }
        Ok(DConv { capas })
    }
    fn ap(&self, x: &Array) -> Result<Array> {
        let mut x = x.clone();
        for (d, (c1, g1, c2, g2, escala)) in self.capas.iter().enumerate() {
            let dil = 1 << d;
            let y = gelu(&g1.ap(&c1.c1(&x, 1, dil, dil)?)?)?;
            let y = glu(&g2.ap(&c2.c1(&y, 1, 0, 1)?)?)?;
            x = ops::add(&x, &ops::multiply(&y, escala).m()?).m()?;
        }
        Ok(x)
    }
    /// La rama residual de una capa de FRECUENCIA: corre sobre el tiempo de cada frecuencia.
    fn ap_frec(&self, x: &Array) -> Result<Array> {
        let (b, f, t, c) = (x.dim(0), x.dim(1), x.dim(2), x.dim(3));
        let y = self.ap(&ops::reshape(x, &[b * f, t, c]).m()?)?;
        ops::reshape(&y, &[b, f, t, c]).m()
    }
}

/// `HEncLayer` (sin GroupNorm: `norm_starts` = 4 = profundidad; `context_enc` = 0).
struct Codificador {
    conv: Conv,
    dconv: DConv,
    reescribe: Conv,
    frec: bool,
}

impl Codificador {
    fn de(p: &mut HashMap<String, Array>, n: &str, frec: bool) -> Result<Codificador> {
        let t = if frec { Tipo::C2 } else { Tipo::C1 };
        Ok(Codificador { conv: Conv::de(p, &format!("{n}.conv"), t)?, dconv: DConv::de(p, &format!("{n}.dconv"))?, reescribe: Conv::de(p, &format!("{n}.rewrite"), t)?, frec })
    }
    fn ap(&self, x: &Array) -> Result<Array> {
        if self.frec {
            let y = gelu(&self.conv.c2(x, (4, 1), (2, 0))?)?;
            let y = self.dconv.ap_frec(&y)?;
            glu(&self.reescribe.c2(&y, (1, 1), (0, 0))?)
        } else {
            let le = x.dim(1);
            let x = if le % 4 != 0 { ops::pad(x, &[(0, 0), (0, 4 - le % 4), (0, 0)], None, None).m()? } else { x.clone() };
            let y = gelu(&self.conv.c1(&x, 4, 2, 1)?)?;
            let y = self.dconv.ap(&y)?;
            glu(&self.reescribe.c1(&y, 1, 0, 1)?)
        }
    }
}

/// `HDecLayer` (`context` = 1: la reescritura es 3×3 en frecuencia y 3 en tiempo).
struct Decodificador {
    conv_tr: Conv,
    dconv: DConv,
    reescribe: Conv,
    frec: bool,
    ultimo: bool,
}

impl Decodificador {
    fn de(p: &mut HashMap<String, Array>, n: &str, frec: bool, ultimo: bool) -> Result<Decodificador> {
        let (t, tt) = if frec { (Tipo::C2, Tipo::T2) } else { (Tipo::C1, Tipo::T1) };
        Ok(Decodificador {
            conv_tr: Conv::de(p, &format!("{n}.conv_tr"), tt)?,
            dconv: DConv::de(p, &format!("{n}.dconv"))?,
            reescribe: Conv::de(p, &format!("{n}.rewrite"), t)?,
            frec,
            ultimo,
        })
    }
    /// `largo`: el de la rama temporal ANTES de su codificador (para recortar).
    fn ap(&self, x: &Array, salto: &Array, largo: i32) -> Result<Array> {
        let x = ops::add(x, salto).m()?;
        let z = if self.frec {
            let y = glu(&self.reescribe.c2(&x, (1, 1), (1, 1))?)?;
            let y = self.dconv.ap_frec(&y)?;
            let z = self.conv_tr.t2(&y, (4, 1))?;
            let f = z.dim(1);
            z.try_index((.., 2..f - 2)).m()?
        } else {
            let y = glu(&self.reescribe.c1(&x, 1, 1, 1)?)?;
            let y = self.dconv.ap(&y)?;
            let z = self.conv_tr.t1(&y, 4)?;
            z.try_index((.., 2..2 + largo)).m()?
        };
        if self.ultimo {
            Ok(z)
        } else {
            gelu(&z)
        }
    }
}

struct Atencion {
    w_in: Array,
    b_in: Array,
    w_out: Array,
    b_out: Array,
    cabezas: i32,
}

impl Atencion {
    fn de(p: &mut HashMap<String, Array>, n: &str, cabezas: i32) -> Result<Atencion> {
        Ok(Atencion {
            w_in: ops::transpose(&quitar(p, &format!("{n}.in_proj_weight"))?).m()?,
            b_in: quitar(p, &format!("{n}.in_proj_bias"))?,
            w_out: ops::transpose(&quitar(p, &format!("{n}.out_proj.weight"))?).m()?,
            b_out: quitar(p, &format!("{n}.out_proj.bias"))?,
            cabezas,
        })
    }
    /// `nn.MultiheadAttention(q, k, k)` con `batch_first`: `[B, N, D]`.
    fn ap(&self, q: &Array, k: &Array) -> Result<Array> {
        let d = q.dim(2);
        let w = self.w_in.split_equal(3, -1).m()?;
        let b = self.b_in.split_equal(3, -1).m()?;
        let cab = |x: &Array, i: usize| -> Result<Array> {
            let y = ops::add(&ops::matmul(x, &w[i]).m()?, &b[i]).m()?;
            ops::transpose_axes(&ops::reshape(&y, &[x.dim(0), x.dim(1), self.cabezas, d / self.cabezas]).m()?, &[0, 2, 1, 3]).m()
        };
        let (qh, kh, vh) = (cab(q, 0)?, cab(k, 1)?, cab(k, 2)?);
        let a = fast::scaled_dot_product_attention(&qh, &kh, &vh, 1.0 / ((d / self.cabezas) as f32).sqrt(), None, None::<&Array>).m()?;
        let a = ops::reshape(&ops::transpose_axes(&a, &[0, 2, 1, 3]).m()?, &[q.dim(0), q.dim(1), d]).m()?;
        ops::add(&ops::matmul(&a, &self.w_out).m()?, &self.b_out).m()
    }
}

/// Una capa del transformador: propia (`MyTransformerEncoderLayer`) o cruzada
/// (`CrossTransformerEncoderLayer`), las dos con norma antes, LayerScale y GroupNorm(1) de salida.
struct Capa {
    atencion: Atencion,
    n1: (Array, Array),
    n2: (Array, Array),
    /// sólo la cruzada: la norma del FFN (la `norm2` normaliza la otra rama)
    n3: Option<(Array, Array)>,
    l1: (Array, Array),
    l2: (Array, Array),
    g1: Array,
    g2: Array,
    salida: Grupo,
}

fn ln(p: &mut HashMap<String, Array>, n: &str) -> Result<(Array, Array)> {
    Ok((quitar(p, &format!("{n}.weight"))?, quitar(p, &format!("{n}.bias"))?))
}

fn lin(p: &mut HashMap<String, Array>, n: &str) -> Result<(Array, Array)> {
    Ok((ops::transpose(&quitar(p, &format!("{n}.weight"))?).m()?, quitar(p, &format!("{n}.bias"))?))
}

fn norma(x: &Array, n: &(Array, Array)) -> Result<Array> {
    fast::layer_norm(x, Some(&n.0), Some(&n.1), 1e-5).m()
}

impl Capa {
    fn de(p: &mut HashMap<String, Array>, n: &str, cruzada: bool, cabezas: i32) -> Result<Capa> {
        Ok(Capa {
            atencion: Atencion::de(p, &format!("{n}.{}", if cruzada { "cross_attn" } else { "self_attn" }), cabezas)?,
            n1: ln(p, &format!("{n}.norm1"))?,
            n2: ln(p, &format!("{n}.norm2"))?,
            n3: if cruzada { Some(ln(p, &format!("{n}.norm3"))?) } else { None },
            l1: lin(p, &format!("{n}.linear1"))?,
            l2: lin(p, &format!("{n}.linear2"))?,
            g1: quitar(p, &format!("{n}.gamma_1.scale"))?,
            g2: quitar(p, &format!("{n}.gamma_2.scale"))?,
            salida: Grupo::de(p, &format!("{n}.norm_out"))?,
        })
    }
    fn ffn(&self, x: &Array) -> Result<Array> {
        let h = gelu(&ops::add(&ops::matmul(x, &self.l1.0).m()?, &self.l1.1).m()?)?;
        ops::add(&ops::matmul(&h, &self.l2.0).m()?, &self.l2.1).m()
    }
    /// `k` = la OTRA rama (cruzada) o `None` (propia).
    fn ap(&self, x: &Array, k: Option<&Array>) -> Result<Array> {
        let (a, n_ffn) = match (k, &self.n3) {
            (Some(k), Some(n3)) => (self.atencion.ap(&norma(x, &self.n1)?, &norma(k, &self.n2)?)?, n3),
            _ => {
                let h = norma(x, &self.n1)?;
                (self.atencion.ap(&h, &h)?, &self.n2)
            }
        };
        let x = ops::add(x, &ops::multiply(&a, &self.g1).m()?).m()?;
        let x = ops::add(&x, &ops::multiply(&self.ffn(&norma(&x, n_ffn)?)?, &self.g2).m()?).m()?;
        self.salida.ap(&x)
    }
}

/// `create_2d_sin_embedding(C, Fr, T1)` reordenado como los tokens `(t1 fr)`: `[1, T1·Fr, C]`.
pub(crate) fn embebido_2d(c: usize, fr: usize, t1: usize) -> Array {
    let d = c / 2;
    // como torch: TODO en f32 (el ángulo se redondea a f32 y su seno se toma exacto)
    let div: Vec<f32> = (0..d).step_by(2).map(|i| (i as f32 * -((10000f64.ln() / d as f64) as f32)).exp()).collect();
    let mut v = vec![0f32; t1 * fr * c];
    for t in 0..t1 {
        for f in 0..fr {
            let fila = &mut v[(t * fr + f) * c..(t * fr + f + 1) * c];
            for (j, dv) in div.iter().enumerate() {
                let i = 2 * j;
                let (at, af) = (((t as f32) * dv) as f64, ((f as f32) * dv) as f64);
                fila[i] = at.sin() as f32;
                fila[i + 1] = at.cos() as f32;
                fila[d + i] = af.sin() as f32;
                fila[d + i + 1] = af.cos() as f32;
            }
        }
    }
    Array::from_slice(&v, &[1, (t1 * fr) as i32, c as i32])
}

/// `create_sin_embedding(T, C)`: coseno en la primera mitad, seno en la segunda: `[1, T, C]`.
pub(crate) fn embebido_1d(t: usize, c: usize) -> Array {
    let m = c / 2;
    let mut v = vec![0f32; t * c];
    for p in 0..t {
        for a in 0..m {
            let fase = ((p as f32) / 10000f32.powf(a as f32 / (m as f32 - 1.0))) as f64;
            v[p * c + a] = fase.cos() as f32;
            v[p * c + m + a] = fase.sin() as f32;
        }
    }
    Array::from_slice(&v, &[1, t as i32, c as i32])
}

/// Un HTDemucs: los pesos de UN modelo (`.safetensors` con su clase y sus argumentos en los metadatos).
pub struct Htdemucs {
    pub fuentes: Vec<String>,
    /// largo del trozo de entrenamiento en muestras (`int(segment · 44100)`)
    pub segmento: usize,
    cod: Vec<Codificador>,
    tcod: Vec<Codificador>,
    dec: Vec<Decodificador>,
    tdec: Vec<Decodificador>,
    emb_frec: Array,
    /// `channel_upsampler`, `channel_downsampler`, `_t`… (sólo con `bottom_channels`)
    fondo: Option<[Conv; 4]>,
    norma_in: (Array, Array),
    norma_in_t: (Array, Array),
    capas: Vec<Capa>,
    capas_t: Vec<Capa>,
}

/// Lo que el puerto exige de la configuración (lo demás no se implementó y se rechaza con su nombre).
pub(crate) fn validar(k: &serde_json::Value) -> Result<()> {
    let igual = |c: &str, v: serde_json::Value| -> Result<()> {
        if k.get(c) != Some(&v) {
            bail_tr!("HTDemucs: «{c}» = {:?} no está soportado (se esperaba {v})", "HTDemucs: «{c}» = {:?} is not supported (expected {v})", k.get(c));
        }
        Ok(())
    };
    use serde_json::json;
    igual("audio_channels", json!(2))?;
    igual("samplerate", json!(44100))?;
    igual("channels", json!(48))?;
    igual("growth", json!(2))?;
    igual("nfft", json!(4096))?;
    igual("wiener_iters", json!(0))?;
    igual("cac", json!(true))?;
    igual("depth", json!(4))?;
    igual("rewrite", json!(true))?;
    igual("multi_freqs", json!([]))?;
    igual("freq_emb", json!(0.2))?;
    igual("emb_scale", json!(10))?;
    igual("kernel_size", json!(8))?;
    igual("stride", json!(4))?;
    igual("context", json!(1))?;
    igual("context_enc", json!(0))?;
    igual("norm_starts", json!(4))?;
    igual("dconv_mode", json!(3))?;
    igual("dconv_depth", json!(2))?;
    igual("t_layers", json!(5))?;
    igual("t_emb", json!("sin"))?; // juez-idioma: datos (valor del config oficial)
    igual("t_norm_in", json!(true))?;
    igual("t_norm_first", json!(true))?;
    igual("t_norm_out", json!(true))?;
    igual("t_group_norm", json!(false))?;
    igual("t_layer_scale", json!(true))?;
    igual("t_gelu", json!(true))?;
    igual("t_cross_first", json!(false))?;
    igual("t_sparse_self_attn", json!(false))?;
    igual("t_sparse_cross_attn", json!(false))?;
    igual("t_weight_pos_embed", json!(1.0))?;
    igual("t_sin_random_shift", json!(0))?;
    Ok(())
}

impl Htdemucs {
    pub fn cargar(ruta: &Path) -> Result<Htdemucs> {
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
        let cabezas = k["t_heads"].as_i64().unwrap_or(8) as i32;
        let mut p: HashMap<String, Array> = p.into_iter().map(|(n, a)| a.as_dtype(F32).map(|a| (n, a))).collect::<std::result::Result<_, _>>().m()?;
        let p = &mut p;
        let mut cod = Vec::new();
        let mut tcod = Vec::new();
        let mut dec = Vec::new();
        let mut tdec = Vec::new();
        for i in 0..4 {
            cod.push(Codificador::de(p, &format!("encoder.{i}"), true)?);
            tcod.push(Codificador::de(p, &format!("tencoder.{i}"), false)?);
            dec.push(Decodificador::de(p, &format!("decoder.{i}"), true, i == 3)?);
            tdec.push(Decodificador::de(p, &format!("tdecoder.{i}"), false, i == 3)?);
        }
        let emb = ops::multiply(&quitar(p, "freq_emb.embedding.weight")?, Array::from_f32(10.0 * 0.2)).m()?;
        let emb_frec = ops::expand_dims(&ops::expand_dims(&emb, 0).m()?, 2).m()?;
        let fondo = if k["bottom_channels"].as_i64().unwrap_or(0) > 0 {
            Some([
                Conv::de(p, "channel_upsampler", Tipo::C1)?,
                Conv::de(p, "channel_downsampler", Tipo::C1)?,
                Conv::de(p, "channel_upsampler_t", Tipo::C1)?,
                Conv::de(p, "channel_downsampler_t", Tipo::C1)?,
            ])
        } else {
            None
        };
        let norma_in = ln(p, "crosstransformer.norm_in")?;
        let norma_in_t = ln(p, "crosstransformer.norm_in_t")?;
        let mut capas = Vec::new();
        let mut capas_t = Vec::new();
        for i in 0..5 {
            let cruzada = i % 2 == 1;
            capas.push(Capa::de(p, &format!("crosstransformer.layers.{i}"), cruzada, cabezas)?);
            capas_t.push(Capa::de(p, &format!("crosstransformer.layers_t.{i}"), cruzada, cabezas)?);
        }
        if !p.is_empty() {
            let mut sobran: Vec<&String> = p.keys().collect();
            sobran.sort();
            bail_tr!("HTDemucs: pesos sin usar: {:?}", "HTDemucs: unused weights: {:?}", sobran);
        }
        Ok(Htdemucs { fuentes, segmento, cod, tcod, dec, tdec, emb_frec, fondo, norma_in, norma_in_t, capas, capas_t })
    }

    /// `HTDemucs.forward` sobre UN trozo ya normalizado: `[2][largo]` → `[fuentes][2][largo]`.
    pub fn adelante(&self, mezcla: &[Vec<f32>; 2]) -> Result<Vec<[Vec<f32>; 2]>> {
        let largo = mezcla[0].len();
        // ── la rama espectral: STFT (`_spec`) y complejo como canales ──
        let (re, im, cuadros) = espectro(mezcla)?; // [2, T, 2048] cada uno
        // [B=1, F, T, 4] con los canales (c0 re, c0 im, c1 re, c1 im)
        let z = ops::stack(&[re.try_index(0).m()?, im.try_index(0).m()?, re.try_index(1).m()?, im.try_index(1).m()?], -1).m()?;
        let z = ops::expand_dims(&ops::transpose_axes(&z, &[1, 0, 2]).m()?, 0).m()?;
        let (media, desv) = media_desv(&z)?;
        let mut x = ops::divide(&ops::subtract(&z, &media).m()?, &ops::add(&desv, Array::from_f32(1e-5)).m()?).m()?;
        // ── la rama temporal: [1, L, 2] ──
        let mut onda = Vec::with_capacity(largo * 2);
        for i in 0..largo {
            onda.push(mezcla[0][i]);
            onda.push(mezcla[1][i]);
        }
        let xt0 = Array::from_slice(&onda, &[1, largo as i32, 2]);
        let (media_t, desv_t) = media_desv(&xt0)?;
        let mut xt = ops::divide(&ops::subtract(&xt0, &media_t).m()?, &ops::add(&desv_t, Array::from_f32(1e-5)).m()?).m()?;
        // ── codificadores ──
        let mut saltos = Vec::new();
        let mut saltos_t = Vec::new();
        let mut largos_t = Vec::new();
        for i in 0..4 {
            largos_t.push(xt.dim(1));
            xt = self.tcod[i].ap(&xt)?;
            saltos_t.push(xt.clone());
            x = self.cod[i].ap(&x)?;
            if i == 0 {
                x = ops::add(&x, &self.emb_frec).m()?;
            }
            saltos.push(x.clone());
        }
        // ── el transformador cruzado ──
        let (fr, t1) = (x.dim(1), x.dim(2));
        if let Some(f) = &self.fondo {
            x = f[0].lineal(&x)?;
            xt = f[2].lineal(&xt)?;
        }
        let c = x.dim(3);
        let mut tok = ops::reshape(&ops::transpose_axes(&x, &[0, 2, 1, 3]).m()?, &[1, t1 * fr, c]).m()?;
        tok = ops::add(&norma(&tok, &self.norma_in)?, &embebido_2d(c as usize, fr as usize, t1 as usize)).m()?;
        let t2 = xt.dim(1);
        let mut tokt = ops::add(&norma(&xt, &self.norma_in_t)?, &embebido_1d(t2 as usize, c as usize)).m()?;
        for i in 0..5 {
            if i % 2 == 0 {
                tok = self.capas[i].ap(&tok, None)?;
                tokt = self.capas_t[i].ap(&tokt, None)?;
            } else {
                let viejo = tok.clone();
                tok = self.capas[i].ap(&tok, Some(&tokt))?;
                tokt = self.capas_t[i].ap(&tokt, Some(&viejo))?;
            }
        }
        x = ops::transpose_axes(&ops::reshape(&tok, &[1, t1, fr, c]).m()?, &[0, 2, 1, 3]).m()?;
        xt = tokt;
        if let Some(f) = &self.fondo {
            x = f[1].lineal(&x)?;
            xt = f[3].lineal(&xt)?;
        }
        // ── decodificadores ──
        for i in 0..4 {
            x = self.dec[i].ap(&x, &saltos.pop().unwrap(), 0)?;
            xt = self.tdec[i].ap(&xt, &saltos_t.pop().unwrap(), largos_t.pop().unwrap())?;
        }
        // ── de vuelta: el espectro por ISTFT + la onda ──
        let s = self.fuentes.len();
        let x = ops::add(&ops::multiply(&x, &desv).m()?, &media).m()?; // [1, F, T, S·4]
        // canal = s·4 + c·2 + (re|im) → [S·2, T, F] parte real e imaginaria
        let x = ops::reshape(&x, &[x.dim(1), x.dim(2), (s * 2) as i32, 2]).m()?;
        let x = ops::transpose_axes(&x, &[2, 1, 0, 3]).m()?;
        let p = x.split_equal(2, -1).m()?;
        let re = ops::squeeze_axes(&p[0], &[-1]).m()?;
        let im = ops::squeeze_axes(&p[1], &[-1]).m()?;
        let ondas = inverso(&re, &im, cuadros, largo)?; // [S·2, largo]
        let xt = ops::add(&ops::multiply(&xt, &desv_t).m()?, &media_t).m()?; // [1, L, S·2]
        let xt = ops::transpose(&ops::squeeze_axes(&xt, &[0]).m()?).m()?;
        let total = ops::add(&ondas, &xt).m()?;
        total.eval().m()?;
        let v: Vec<f32> = total.as_slice::<f32>().to_vec();
        Ok((0..s).map(|f| [v[(2 * f) * largo..(2 * f + 1) * largo].to_vec(), v[(2 * f + 1) * largo..(2 * f + 2) * largo].to_vec()]).collect())
    }
}

/// Media y desviación (INSESGADA, como `torch.std`) de todo lo que no es el lote.
pub(crate) fn media_desv(x: &Array) -> Result<(Array, Array)> {
    let ejes: Vec<i32> = (1..x.ndim() as i32).collect();
    let n: i32 = ejes.iter().map(|&e| x.dim(e)).product();
    let media = x.mean_axes(&ejes, true).m()?;
    let var = ops::sum_axes(&ops::square(&ops::subtract(x, &media).m()?).m()?, &ejes, true).m()?;
    let desv = ops::sqrt(&ops::divide(&var, Array::from_f32((n - 1) as f32)).m()?).m()?;
    Ok((media, desv))
}

pub(crate) fn hann() -> Vec<f32> {
    (0..NFFT).map(|n| (0.5 - 0.5 * (2.0 * std::f64::consts::PI * n as f64 / NFFT as f64).cos()) as f32).collect()
}

/// Reflejo como `F.pad(mode="reflect")`: `[izq, x, der]` sin repetir el borde.
fn reflejar(x: &[f32], izq: usize, der: usize) -> Vec<f32> {
    let n = x.len();
    let mut y = Vec::with_capacity(n + izq + der);
    y.extend((1..=izq).rev().map(|i| x[i.min(n - 1)]));
    y.extend_from_slice(x);
    y.extend((0..der).map(|i| x[(n - 2 - i.min(n - 2)).min(n - 1)]));
    y
}

/// `HTDemucs._spec`: [2][L] → (real, imaginaria) `[2, le, 2048]` y `le` (= ⌈L/1024⌉).
pub(crate) fn espectro(x: &[Vec<f32>; 2]) -> Result<(Array, Array, usize)> {
    let l = x[0].len();
    let le = l.div_ceil(SALTO);
    let pad = SALTO / 2 * 3;
    let ventana = Array::from_slice(&hann(), &[1, 1, NFFT as i32]);
    let mut cuadros = Vec::new();
    let n_cuadros = le + 4;
    for c in x {
        // `_spec` refleja 1536 (+ lo que falte para un múltiplo); `stft(center=True)` refleja 2048 más
        let a = reflejar(c, pad, pad + le * SALTO - l);
        let b = reflejar(&a, NFFT / 2, NFFT / 2);
        debug_assert_eq!(1 + (b.len() - NFFT) / SALTO, n_cuadros);
        for i in 0..n_cuadros {
            cuadros.extend_from_slice(&b[i * SALTO..i * SALTO + NFFT]);
        }
    }
    let c = ops::multiply(&Array::from_slice(&cuadros, &[2, n_cuadros as i32, NFFT as i32]), &ventana).m()?;
    let z = mlx_rs::fft::rfft(&c, NFFT as i32, -1).m()?;
    // `normalized=True` (÷√n_fft), sin la última frecuencia, cuadros [2, 2+le)
    let z = z.try_index((.., 2..2 + le as i32, 0..(NFFT / 2) as i32)).m()?;
    let esc = Array::from_f32(1.0 / (NFFT as f32).sqrt());
    Ok((ops::multiply(&z.real().m()?, &esc).m()?, ops::multiply(&z.imag().m()?, &esc).m()?, le))
}

/// `HTDemucs._ispec` + `istft(normalized=True, center=True)`: `[N, le, 2048]` (real e imaginaria) →
/// `[N, largo]`.
pub(crate) fn inverso(re: &Array, im: &Array, le: usize, largo: usize) -> Result<Array> {
    let n = re.dim(0);
    // la frecuencia que faltaba (cero) y 2 cuadros de cero a cada lado
    let completar = |a: &Array| ops::pad(a, &[(0, 0), (2, 2), (0, 1)], None, None).m();
    let j = Array::from_complex(mlx_rs::complex64::new(0.0, 1.0));
    let z = ops::add(&completar(re)?, &ops::multiply(&completar(im)?, &j).m()?).m()?;
    let cuadros = le + 4;
    let w = hann();
    let marcos = mlx_rs::fft::irfft(&z, NFFT as i32, -1).m()?; // [N, cuadros, 4096]
    let marcos = ops::multiply(&marcos, &Array::from_slice(&w.iter().map(|v| v * (NFFT as f32).sqrt()).collect::<Vec<f32>>(), &[1, 1, NFFT as i32])).m()?;
    // solapar y sumar con salto 1024: cada cuadro son 4 bloques; el bloque k del cuadro i cae en i+k
    let bloques = ops::reshape(&marcos, &[n, cuadros as i32, 4, SALTO as i32]).m()?;
    let mut suma: Option<Array> = None;
    for k in 0..4 {
        let b = bloques.try_index((.., .., k)).m()?;
        let b = ops::pad(&b, &[(0, 0), (k, 3 - k), (0, 0)], None, None).m()?;
        suma = Some(match suma {
            None => b,
            Some(s) => ops::add(&s, &b).m()?,
        });
    }
    let y = ops::reshape(&suma.unwrap(), &[n, -1]).m()?; // [N, (cuadros+3)·1024]
    // la envolvente Σ ventana² (constante, en la CPU)
    let total = (cuadros + 3) * SALTO;
    let mut env = vec![0f32; total];
    for i in 0..cuadros {
        for (t, v) in w.iter().enumerate() {
            env[i * SALTO + t] += v * v;
        }
    }
    // `istft` recorta n_fft/2 al principio y deja `le·1024 + 2·1536`; `_ispec` recorta 1536 más
    let inicio = NFFT / 2 + SALTO / 2 * 3;
    let env = Array::from_slice(&env[inicio..inicio + largo], &[1, largo as i32]);
    let y = y.try_index((.., inicio as i32..(inicio + largo) as i32)).m()?;
    ops::divide(&y, &env).m()
}

/// Un modelo o una «bolsa» (`-ft`: 4 modelos, cada uno a cargo de UNA fuente).
pub struct Separador {
    pub fuentes: Vec<String>,
    modelos: Vec<Htdemucs>,
    /// `pesos[m][s]`: cuánto aporta el modelo m a la fuente s (el YAML; sin YAML, todo 1)
    pesos: Vec<Vec<f32>>,
}

/// El YAML de la bolsa: `models: ['a', 'b']` y, si está, `weights: [[1., 0.], …]`.
fn leer_bolsa(texto: &str) -> Result<(Vec<String>, Option<Vec<Vec<f32>>>)> {
    let i = texto.find("models:").ok_or_else(|| anyhow_tr!("el YAML no tiene «models»", "the YAML has no «models»"))?;
    let resto = &texto[i + 7..];
    let fin = resto.find(']').ok_or_else(|| anyhow_tr!("«models» sin cerrar en el YAML", "unclosed «models» in the YAML"))?;
    let modelos: Vec<String> = resto[..fin].trim().trim_start_matches('[').split(',').map(|s| s.trim().trim_matches('\'').trim_matches('"').to_string()).filter(|s| !s.is_empty()).collect();
    let pesos = match texto.find("weights:") {
        None => None,
        Some(j) => {
            let r = &texto[j + 8..];
            let mut filas = Vec::new();
            let mut cuerpo = r.trim_start();
            cuerpo = cuerpo.strip_prefix('[').unwrap_or(cuerpo);
            while let Some(a) = cuerpo.find('[') {
                let b = cuerpo[a..].find(']').ok_or_else(|| anyhow_tr!("«weights» sin cerrar en el YAML", "unclosed «weights» in the YAML"))? + a;
                let fila: Vec<f32> = cuerpo[a + 1..b].split(',').map(|v| v.trim()).filter(|v| !v.is_empty()).map(|v| v.parse::<f32>()).collect::<std::result::Result<_, _>>().context("weights")?;
                filas.push(fila);
                cuerpo = &cuerpo[b + 1..];
            }
            Some(filas)
        }
    };
    Ok((modelos, pesos))
}

impl Separador {
    /// La carpeta del repo (`adefossez/HTDemucs`, `-6s`, `-ft`): su YAML y sus `.safetensors`.
    pub fn cargar(dir: &Path) -> Result<Separador> {
        let yaml = std::fs::read_dir(dir)
            .with_context(|| format!("{}", dir.display()))?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .find(|p| p.extension().and_then(|e| e.to_str()) == Some("yaml"))
            .ok_or_else(|| anyhow_tr!("{}: falta el YAML del modelo", "{}: the model YAML is missing", dir.display()))?;
        let (nombres, pesos) = leer_bolsa(&std::fs::read_to_string(&yaml)?)?;
        let modelos: Vec<Htdemucs> = nombres.iter().map(|n| Htdemucs::cargar(&dir.join(format!("{n}.safetensors")))).collect::<Result<_>>()?;
        let fuentes = modelos.first().ok_or_else(|| anyhow!("bolsa vacía"))?.fuentes.clone();
        if modelos.iter().any(|m| m.fuentes != fuentes) {
            bail_tr!("los modelos de la bolsa no separan las mismas fuentes", "the bag models do not split the same sources");
        }
        let pesos = pesos.unwrap_or_else(|| vec![vec![1.0; fuentes.len()]; modelos.len()]);
        if pesos.len() != modelos.len() || pesos.iter().any(|f| f.len() != fuentes.len()) {
            bail_tr!("los pesos de la bolsa no cuadran con sus modelos", "the bag weights do not match its models");
        }
        Ok(Separador { fuentes, modelos, pesos })
    }

    /// Cuántas pasadas del modelo cuesta separar `muestras` (para el avance).
    pub fn pasadas(&self, muestras: usize) -> usize {
        let seg = self.modelos[0].segmento;
        let paso = (0.75 * seg as f64) as usize;
        muestras.div_ceil(paso).max(1) * self.modelos.iter().zip(&self.pesos).filter(|(_, w)| w.iter().any(|v| *v != 0.0)).count()
    }

    /// `Separator.separate_tensor` (sin `shifts`): estéreo a 44,1 kHz → una onda estéreo por fuente.
    /// `avance(hechas, total)` tras cada trozo.
    pub fn separar(&self, mezcla: &[Vec<f32>; 2], avance: &dyn Fn(usize, usize), cancelar: &AtomicBool) -> Result<Vec<[Vec<f32>; 2]>> {
        let l = mezcla[0].len();
        if l == 0 || mezcla[1].len() != l {
            bail_tr!("audio vacío o con canales de distinto largo", "empty audio or channels of different lengths");
        }
        // la normalización de `Separator`: media y desviación (insesgada) de la mezcla MONO
        let mono: Vec<f64> = (0..l).map(|i| (mezcla[0][i] as f64 + mezcla[1][i] as f64) / 2.0).collect();
        let media = mono.iter().sum::<f64>() / l as f64;
        let desv = if l > 1 { (mono.iter().map(|v| (v - media).powi(2)).sum::<f64>() / (l - 1) as f64).sqrt() } else { 0.0 } + 1e-8;
        let norm: [Vec<f32>; 2] = [mezcla[0].iter().map(|v| ((*v as f64 - media) / desv) as f32).collect(), mezcla[1].iter().map(|v| ((*v as f64 - media) / desv) as f32).collect()];
        let s = self.fuentes.len();
        let mut total = vec![[vec![0f64; l], vec![0f64; l]]; s];
        let mut pesos_fuente = vec![0f64; s];
        let pasadas = self.pasadas(l);
        let mut hechas = 0;
        for (modelo, w) in self.modelos.iter().zip(&self.pesos) {
            if w.iter().all(|v| *v == 0.0) {
                continue;
            }
            let est = self.trozos(modelo, &norm, &mut hechas, pasadas, avance, cancelar)?;
            for k in 0..s {
                if w[k] == 0.0 {
                    continue;
                }
                for c in 0..2 {
                    for (t, v) in total[k][c].iter_mut().zip(&est[k][c]) {
                        *t += w[k] as f64 * *v as f64;
                    }
                }
                pesos_fuente[k] += w[k] as f64;
            }
        }
        Ok(total
            .into_iter()
            .zip(pesos_fuente)
            .map(|(f, p)| {
                let a = |c: &Vec<f64>| c.iter().map(|v| (v / p * desv + media) as f32).collect::<Vec<f32>>();
                [a(&f[0]), a(&f[1])]
            })
            .collect())
    }

    /// `apply_model(split=True, overlap=0.25)` con UN modelo.
    fn trozos(&self, m: &Htdemucs, x: &[Vec<f32>; 2], hechas: &mut usize, pasadas: usize, avance: &dyn Fn(usize, usize), cancelar: &AtomicBool) -> Result<Vec<[Vec<f32>; 2]>> {
        let l = x[0].len();
        let seg = m.segmento;
        let paso = (0.75 * seg as f64) as usize;
        let mitad = seg / 2;
        let mut peso: Vec<f64> = (1..=mitad).map(|v| v as f64).chain((1..=seg - mitad).rev().map(|v| v as f64)).collect();
        let tope = peso.iter().cloned().fold(0.0, f64::max);
        peso.iter_mut().for_each(|v| *v /= tope);
        let s = m.fuentes.len();
        let mut out = vec![[vec![0f64; l], vec![0f64; l]]; s];
        let mut suma = vec![0f64; l];
        let mut desde = 0;
        while desde < l {
            if cancelar.load(Ordering::Relaxed) {
                bail_tr!("cancelado", "cancelled");
            }
            let largo = (l - desde).min(seg);
            // `TensorChunk.padded(seg)`: centrado, con el contexto REAL a los lados y ceros fuera
            let delta = seg - largo;
            let inicio = desde as i64 - (delta / 2) as i64;
            let trozo: [Vec<f32>; 2] = std::array::from_fn(|c| (0..seg as i64).map(|i| {
                let j = inicio + i;
                if j >= 0 && (j as usize) < l { x[c][j as usize] } else { 0.0 }
            }).collect());
            let y = m.adelante(&trozo)?;
            // `center_trim`: se quita delta/2 a la izquierda
            let a = delta / 2;
            for k in 0..s {
                for c in 0..2 {
                    for i in 0..largo {
                        out[k][c][desde + i] += peso[i] * y[k][c][a + i] as f64;
                    }
                }
            }
            for i in 0..largo {
                suma[desde + i] += peso[i];
            }
            *hechas += 1;
            avance(*hechas, pasadas);
            desde += paso;
        }
        Ok(out.into_iter().map(|f| [f[0].iter().zip(&suma).map(|(v, w)| (v / w) as f32).collect(), f[1].iter().zip(&suma).map(|(v, w)| (v / w) as f32).collect()]).collect())
    }
}

/// El motor que registra el binario (`generadores::MotorSeparar`): lee el audio (WAV, FLAC, MP3, M4A…),
/// lo lleva a estéreo a 44,1 kHz (mono → duplicado, como `convert_audio`; el remuestreo exacto de
/// torchaudio), separa, y escribe cada pista de vuelta a la frecuencia y al largo del original, en
/// coma flotante: `<salida>/<base>-<pista>.wav`.
pub fn separar_archivo(dir: &Path, audio: &Path, salida: &Path, base: &str, progreso: &(dyn Fn(String) + Sync), cancelar: &AtomicBool) -> Result<Vec<(String, PathBuf)>> {
    use crate::musica_fuente::{leer, remuestrear};
    let (mut c, fr) = leer(audio)?;
    if c.len() == 1 {
        let m = c[0].clone();
        c.push(m);
    }
    c.truncate(2);
    let largo = c[0].len();
    progreso(gotv_engine::tr!("preparando el audio", "preparing the audio"));
    let x: [Vec<f32>; 2] = [remuestrear(&c[0], fr, FRECUENCIA), remuestrear(&c[1], fr, FRECUENCIA)];
    let sep = Separador::cargar(dir)?;
    let fuentes = sep.separar(&x, &|h, t| progreso(gotv_engine::tr!("trozo {h} de {t}", "chunk {h} of {t}")), cancelar)?;
    std::fs::create_dir_all(salida)?;
    let mut hechas = Vec::new();
    for (nombre, f) in sep.fuentes.iter().zip(fuentes) {
        let mut l = remuestrear(&f[0], FRECUENCIA, fr);
        let mut r = remuestrear(&f[1], FRECUENCIA, fr);
        l.resize(largo, 0.0);
        r.resize(largo, 0.0);
        let muestras: Vec<f32> = l.iter().zip(&r).flat_map(|(a, b)| [*a, *b]).collect();
        let wav = crate::musica::Audio { muestras, canales: 2, frecuencia: fr, plan: None }.wav();
        let ruta = salida.join(format!("{base}-{nombre}.wav"));
        std::fs::write(&ruta, wav).with_context(|| ruta.display().to_string())?;
        hechas.push((nombre.clone(), ruta));
    }
    Ok(hechas)
}

/// La carpeta de un separador del catálogo (`adefossez/HTDemucs`…) dentro de los modelos de GOTV.
pub fn carpeta(repo: &str) -> PathBuf {
    gotv_engine::cache_dir().join("models").join(repo)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn la_bolsa_se_lee_del_yaml() {
        let (m, w) = leer_bolsa("models: ['955717e8']\n").unwrap();
        assert_eq!(m, vec!["955717e8"]);
        assert!(w.is_none());
        let (m, w) = leer_bolsa("models: ['f7e0c4bc', 'd12395a8', '92cfc3b6', '04573f0d']\nweights: [\n  [1., 0., 0., 0.],\n  [0., 1., 0., 0.],\n  [0., 0., 1., 0.],\n  [0., 0., 0., 1.],\n]").unwrap();
        assert_eq!(m.len(), 4);
        assert_eq!(w.unwrap(), vec![vec![1., 0., 0., 0.], vec![0., 1., 0., 0.], vec![0., 0., 1., 0.], vec![0., 0., 0., 1.]]);
    }

    #[test]
    fn reflejo_como_torch() {
        // F.pad([0,1,2,3,4], (2, 3), "reflect") = [2,1,0,1,2,3,4,3,2,1]
        assert_eq!(reflejar(&[0., 1., 2., 3., 4.], 2, 3), vec![2., 1., 0., 1., 2., 3., 4., 3., 2., 1.]);
    }
    /// STFT → ISTFT sin modelo devuelve la onda en el INTERIOR: `_spec` descarta 2 cuadros a cada lado y
    /// `_ispec` los repone en cero, así que los primeros y últimos ~1536 muestras NO vuelven (el oficial
    /// tampoco: lo compensa la rama temporal y el solape de los trozos).
    #[test]
    fn el_espectro_vuelve_a_la_onda() {
        let l: usize = 30_000;
        let x: [Vec<f32>; 2] = std::array::from_fn(|c| (0..l).map(|i| (i as f32 * 0.013 * (c + 1) as f32).sin() * 0.5 + ((i * 7919 % 1000) as f32 / 1000.0 - 0.5) * 0.1).collect());
        let (re, im, le) = espectro(&x).unwrap();
        assert_eq!(le, l.div_ceil(SALTO));
        let y = inverso(&re, &im, le, l).unwrap();
        y.eval().unwrap();
        let v = y.as_slice::<f32>();
        let err = |a: usize, b: usize| (0..2).flat_map(|c| (a..b).map(move |i| (c, i))).map(|(c, i)| (v[c * l + i] - x[c][i]).abs()).fold(0f32, f32::max);
        let (dentro, borde) = (err(2048, l - 2048), err(0, 2048));
        eprintln!("interior {dentro:e}, borde {borde:e}");
        assert!(dentro < 1e-4, "error máximo en el interior {dentro}");
    }

    /// Juez de punta a punta contra `demucs` 4.1 (los volcados del juez Python en
    /// `GOTV_DEMUCS_JUEZ`): `entrada.wav` y una pista por fuente, con el MISMO troceo sin `shifts`.
    #[test]
    #[ignore]
    fn juez_contra_demucs() {
        let Some(dir) = std::env::var_os("GOTV_DEMUCS_JUEZ").map(PathBuf::from) else { return };
        let repo = std::env::var("GOTV_DEMUCS_REPO").unwrap_or_else(|_| "adefossez/HTDemucs".into());
        let sep = Separador::cargar(&carpeta(&repo)).unwrap();
        let (canales, fr) = crate::musica_fuente::leer(&dir.join("entrada.wav")).unwrap();
        assert_eq!(fr, FRECUENCIA);
        let mezcla = [canales[0].clone(), canales[1].clone()];
        let t0 = std::time::Instant::now();
        let fuentes = sep.separar(&mezcla, &|h, t| eprintln!("trozo {h}/{t}"), &AtomicBool::new(false)).unwrap();
        let seg = t0.elapsed().as_secs_f64();
        let mut peor = f64::INFINITY;
        for (k, nombre) in sep.fuentes.iter().enumerate() {
            let (o, _) = crate::musica_fuente::leer(&dir.join(format!("{nombre}.wav"))).unwrap();
            let (mut senal, mut ruido) = (0f64, 0f64);
            for c in 0..2 {
                for (a, b) in fuentes[k][c].iter().zip(&o[c]) {
                    senal += (*b as f64).powi(2);
                    ruido += (*a as f64 - *b as f64).powi(2);
                }
            }
            let snr = 10.0 * (senal / ruido.max(1e-30)).log10();
            eprintln!("{nombre}: {snr:.1} dB contra el oficial");
            peor = peor.min(snr);
        }
        eprintln!("separado en {seg:.1} s; peor pista {peor:.1} dB");
        assert!(peor > 60.0, "una pista se aparta del oficial: {peor:.1} dB");
    }
}
