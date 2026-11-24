//! M7c · efectos de sonido locales: MOSS-SoundEffect v2.0 (OpenMOSS, Apache-2.0) sobre MLX, escrito a
//! mano desde su `WanAudioPipeline` oficial. Todo en f32 (el oficial en un Mac sin CUDA cae a la CPU
//! con autocast bf16):
//! 1. **Texto**: «{descripción} duration: {s:.1f}s» limpio de espacios → Qwen3-1.7B entero (último
//!    estado oculto) en 512 posiciones: las reales y, detrás, CEROS (`encode_prompt` pone a cero la
//!    cola). El negativo es "" → todo ceros.
//! 2. **DiT** tipo Wan 2.1 sobre audio (30 bloques de 1536, 12 cabezas, FFN 8960): AdaLN con 6
//!    modulaciones, autoatención con RMSNorm de q/k sobre la dimensión entera y RoPE 1D intercalada
//!    (θ = 10 000, cos/sin en f64), atención cruzada a las 512 posiciones de texto (K/V una vez),
//!    FFN con GELU tanh; cabeza con 2 modulaciones del embebido de tiempo.
//! 3. **Flujo**: 100 pasos, sigmas `linspace(1, 0, 101)[:-1]` con shift 5, CFG 4 (positivo y negativo
//!    en UN lote de 2), paso de Euler hasta σ = 0.
//! 4. **DAC** 48 kHz (hop 960, latente de 128 a 50 Hz): `post_quant_conv`, decodificador con pesos
//!    normalizados y snake, `tanh`. El oficial denoisea SIEMPRE 30 s y recorta a lo pedido.

use crate::modelo::{Modelo, M};
use crate::musica::{quitar, Audio, Conv};
use anyhow::{anyhow, bail, Context, Result};
use gotv_engine::{anyhow_tr, bail_tr, tr};
use mlx_rs::fast;
use mlx_rs::ops::indexing::TryIndexOp;
use mlx_rs::ops;
use mlx_rs::{Array, Dtype};
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

const F32: Dtype = Dtype::Float32;
pub const FRECUENCIA: u32 = 48_000;
/// muestras por cuadro latente (2·3·4·5·8)
pub const SALTO: usize = 960;
/// posiciones de texto (`WanPrompter(text_len=512)`)
const TEXTO: i32 = 512;

/// Lo que se pide.
#[derive(Clone, Debug)]
pub struct Pedido {
    /// el sonido, en inglés (como lo entrenaron)
    pub descripcion: String,
    /// 0,1–30 s (el oficial redondea a un decimal)
    pub segundos: f32,
    pub pasos: u32,
    pub cfg: f32,
    pub shift: f32,
    pub semilla: u64,
    /// lo que se denoisea (el oficial: 30 s siempre); el juez lo acorta
    pub segundos_latente: f32,
}

impl Default for Pedido {
    fn default() -> Self {
        Pedido { descripcion: String::new(), segundos: 5.0, pasos: 100, cfg: 4.0, shift: 5.0, semilla: 0, segundos_latente: 30.0 }
    }
}

/// `html.unescape` de lo común (entidades con nombre básicas y numéricas).
pub(crate) fn desescapar(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut resto = s;
    while let Some(i) = resto.find('&') {
        out.push_str(&resto[..i]);
        let tras = &resto[i..];
        let fin = tras.find(';').filter(|&f| f <= 10);
        let ent = fin.map(|f| &tras[1..f]);
        let c = match ent {
            Some("amp") => Some('&'),
            Some("lt") => Some('<'),
            Some("gt") => Some('>'),
            Some("quot") => Some('"'),
            Some("apos") => Some('\''),
            Some(e) if e.starts_with("#x") || e.starts_with("#X") => u32::from_str_radix(&e[2..], 16).ok().and_then(char::from_u32),
            Some(e) if e.starts_with('#') => e[1..].parse::<u32>().ok().and_then(char::from_u32),
            _ => None,
        };
        match (c, fin) {
            (Some(c), Some(f)) => {
                out.push(c);
                resto = &tras[f + 1..];
            }
            _ => {
                out.push('&');
                resto = &tras[1..];
            }
        }
    }
    out.push_str(resto);
    out
}

/// `round(float(seconds), 1)` de Python con el número que escribió quien lo pidió: el f32 pasa por su
/// decimal más corto (12,35 y no 12,3500004) y se redondea EXACTO como el formato de Python.
pub fn segundos_1(s: f32) -> String {
    let d: f64 = s.to_string().parse().unwrap_or(s as f64);
    format!("{d:.1}")
}

/// El texto que lee el codificador: «{descripción} duration: {s:.1f}s» y el `clean='whitespace'` del
/// prompter (`html.unescape` dos veces, espacios colapsados). (`ftfy.fix_text` repara texto mal
/// decodificado; en texto bien formado no cambia nada y aquí no se replica.)
pub fn texto(p: &Pedido) -> String {
    let t = format!("{} duration: {}s", p.descripcion.trim(), segundos_1(p.segundos));
    desescapar(&desescapar(&t)).split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Las sigmas del `FlowMatchScheduler` (shift, `sigma_min` 0, `extra_one_step`), en f32 como torch:
/// `linspace(1, 0, n+1)[:-1]` (la fórmula de dos mitades de su CPU) y `shift·σ/(1 + (shift−1)·σ)`.
pub fn sigmas(pasos: u32, shift: f32) -> Vec<f32> {
    let n = pasos as usize + 1;
    let paso = (0.0f32 - 1.0) / (n as f32 - 1.0);
    let lin: Vec<f32> = (0..n).map(|i| if i < n / 2 { 1.0 + paso * i as f32 } else { 0.0 - paso * (n - 1 - i) as f32 }).collect();
    lin[..n - 1].iter().map(|&s| shift * s / (1.0 + (shift - 1.0) * s)).collect()
}

/// Lineal con sesgo (`addmm`).
struct Lin {
    w_t: Array,
    b: Array,
}

impl Lin {
    fn de(p: &mut HashMap<String, Array>, n: &str) -> Result<Lin> {
        Ok(Lin { w_t: ops::transpose_axes(&quitar(p, &format!("{n}.weight"))?, &[1, 0]).m()?, b: quitar(p, &format!("{n}.bias"))? })
    }
    fn ap(&self, x: &Array) -> Result<Array> {
        ops::addmm(&self.b, x, &self.w_t, 1.0, 1.0).m()
    }
}

fn silu(x: &Array) -> Result<Array> {
    ops::multiply(x, &ops::sigmoid(x).m()?).m()
}

/// GELU con tanh (la de torch `approximate='tanh'`).
fn gelu_tanh(x: &Array) -> Result<Array> {
    let c = (2.0f32 / std::f32::consts::PI).sqrt();
    let x3 = ops::multiply(&ops::multiply(x, x).m()?, x).m()?;
    let dentro = ops::multiply(&ops::add(x, &ops::multiply(&x3, Array::from_f32(0.044715)).m()?).m()?, Array::from_f32(c)).m()?;
    ops::multiply(&ops::multiply(x, Array::from_f32(0.5)).m()?, &ops::add(Array::from_f32(1.0), &ops::tanh(&dentro).m()?).m()?).m()
}

/// cos y sin [1, L, 1, 64] de la RoPE de Wan (pares intercalados), calculados en f64.
fn giro(l: i32, d: i32) -> (Array, Array) {
    let mitad = (d / 2) as usize;
    let mut c = Vec::with_capacity(l as usize * mitad);
    let mut s = Vec::with_capacity(l as usize * mitad);
    for p in 0..l {
        for j in 0..mitad {
            let f = 1.0f64 / 10000f64.powf((2 * j) as f64 / d as f64);
            let a = p as f64 * f;
            c.push(a.cos() as f32);
            s.push(a.sin() as f32);
        }
    }
    (Array::from_slice(&c, &[1, l, 1, mitad as i32]), Array::from_slice(&s, &[1, l, 1, mitad as i32]))
}

/// `rope_apply`: x [B, L, H·D] → pares (par, impar) por cabeza girados.
fn girar(x: &Array, cos: &Array, sin: &Array, h: i32) -> Result<Array> {
    let (b, l, dim) = (x.dim(0), x.dim(1), x.dim(2));
    let d = dim / h;
    let p = ops::reshape(x, &[b, l, h, d / 2, 2]).m()?;
    let par = p.try_index((.., .., .., .., 0)).m()?;
    let impar = p.try_index((.., .., .., .., 1)).m()?;
    let a = ops::subtract(&ops::multiply(&par, cos).m()?, &ops::multiply(&impar, sin).m()?).m()?;
    let bb = ops::add(&ops::multiply(&par, sin).m()?, &ops::multiply(&impar, cos).m()?).m()?;
    ops::reshape(&ops::stack(&[a, bb], -1).m()?, &[b, l, dim]).m()
}

struct Atencion {
    q: Lin,
    k: Lin,
    v: Lin,
    o: Lin,
    nq: Array,
    nk: Array,
}

impl Atencion {
    fn de(p: &mut HashMap<String, Array>, n: &str) -> Result<Atencion> {
        Ok(Atencion {
            q: Lin::de(p, &format!("{n}.to_q"))?,
            k: Lin::de(p, &format!("{n}.to_k"))?,
            v: Lin::de(p, &format!("{n}.to_v"))?,
            o: Lin::de(p, &format!("{n}.to_out.0"))?,
            nq: quitar(p, &format!("{n}.norm_q.weight"))?,
            nk: quitar(p, &format!("{n}.norm_k.weight"))?,
        })
    }
}

struct Bloque {
    sst: Array,
    auto: Atencion,
    cruz: Atencion,
    n3_w: Array,
    n3_b: Array,
    ff1: Lin,
    ff2: Lin,
}

/// El DiT de audio de MOSS (`WanAudioModel`, nombres de diffusers).
pub struct Dit {
    entrada: Lin,
    txt1: Lin,
    txt2: Lin,
    t1: Lin,
    t2: Lin,
    tproy: Lin,
    bloques: Vec<Bloque>,
    sst_cabeza: Array,
    cabeza: Lin,
    cabezas: i32,
    eps: f32,
}

/// La K/V de la atención cruzada de cada bloque para un lote de contextos (iguales en todos los pasos).
pub struct Contexto {
    kv: Vec<(Array, Array)>,
}

fn cabezas(x: &Array, h: i32) -> Result<Array> {
    let (b, l, dim) = (x.dim(0), x.dim(1), x.dim(2));
    ops::transpose_axes(&ops::reshape(x, &[b, l, h, dim / h]).m()?, &[0, 2, 1, 3]).m()
}

fn juntar(x: &Array) -> Result<Array> {
    let (b, h, l, d) = (x.dim(0), x.dim(1), x.dim(2), x.dim(3));
    ops::reshape(&ops::transpose_axes(x, &[0, 2, 1, 3]).m()?, &[b, l, h * d]).m()
}

impl Dit {
    pub fn cargar(dir: &Path) -> Result<Dit> {
        let cfg: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dir.join("config.json")).context("transformer/config.json")?)?;
        let n = |k: &str| cfg.get(k).and_then(|v| v.as_i64()).ok_or_else(|| anyhow_tr!("config del DiT sin {k}", "DiT config without {k}"));
        if cfg.get("patch_size").and_then(|v| v.get(0)).and_then(|v| v.as_i64()) != Some(1) || cfg.get("has_image_input").and_then(|v| v.as_bool()) != Some(false) {
            bail_tr!("sólo el DiT de MOSS con parche 1 y sin imagen", "only the MOSS DiT with patch 1 and no image");
        }
        let mut p = Array::load_safetensors(dir.join("diffusion_pytorch_model.safetensors")).map_err(|e| anyhow!("DiT de MOSS: {e:?}"))?;
        let mut bloques = Vec::new();
        for i in 0..n("num_layers")? {
            let b = format!("blocks.{i}");
            bloques.push(Bloque {
                sst: quitar(&mut p, &format!("{b}.scale_shift_table"))?,
                auto: Atencion::de(&mut p, &format!("{b}.attn1"))?,
                cruz: Atencion::de(&mut p, &format!("{b}.attn2"))?,
                n3_w: quitar(&mut p, &format!("{b}.norm2.weight"))?,
                n3_b: quitar(&mut p, &format!("{b}.norm2.bias"))?,
                ff1: Lin::de(&mut p, &format!("{b}.ffn.net.0.proj"))?,
                ff2: Lin::de(&mut p, &format!("{b}.ffn.net.2"))?,
            });
        }
        // la conv del parche (núcleo 1) es un lineal: [O, I, 1] → [O, I]
        let w = quitar(&mut p, "patch_embedding.weight")?;
        let entrada = Lin { w_t: ops::transpose_axes(&ops::reshape(&w, &[w.dim(0), w.dim(1)]).m()?, &[1, 0]).m()?, b: quitar(&mut p, "patch_embedding.bias")? };
        let d = Dit {
            entrada,
            txt1: Lin::de(&mut p, "condition_embedder.text_embedder.linear_1")?,
            txt2: Lin::de(&mut p, "condition_embedder.text_embedder.linear_2")?,
            t1: Lin::de(&mut p, "condition_embedder.time_embedder.linear_1")?,
            t2: Lin::de(&mut p, "condition_embedder.time_embedder.linear_2")?,
            tproy: Lin::de(&mut p, "condition_embedder.time_proj")?,
            bloques,
            sst_cabeza: quitar(&mut p, "scale_shift_table")?,
            cabeza: Lin::de(&mut p, "proj_out")?,
            cabezas: n("num_heads")? as i32,
            eps: cfg.get("eps").and_then(|v| v.as_f64()).unwrap_or(1e-6) as f32,
        };
        if !p.is_empty() {
            let mut sobran: Vec<_> = p.keys().cloned().collect();
            sobran.sort();
            bail_tr!("pesos del DiT de MOSS sin usar: {:?}", "unused MOSS DiT weights: {:?}", &sobran[..sobran.len().min(5)]);
        }
        Ok(d)
    }

    /// `text_embedding` + K/V cruzadas de cada bloque: contextos [B, 512, 2048].
    pub fn contexto(&self, ctx: &Array) -> Result<Contexto> {
        let c = self.txt2.ap(&gelu_tanh(&self.txt1.ap(ctx)?)?)?;
        let kv = self
            .bloques
            .iter()
            .map(|b| -> Result<(Array, Array)> {
                let k = fast::rms_norm(&b.cruz.k.ap(&c)?, Some(&b.cruz.nk), self.eps).m()?;
                Ok((cabezas(&k, self.cabezas)?, cabezas(&b.cruz.v.ap(&c)?, self.cabezas)?))
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Contexto { kv })
    }

    /// `sinusoidal_embedding_1d` (en f64) → `time_embedding` → (t [1, D], t_mod [1, 6, D]).
    fn tiempo(&self, t: f32) -> Result<(Array, Array)> {
        let medio = 128usize;
        let mut v = vec![0f32; 2 * medio];
        for j in 0..medio {
            let a = t as f64 * 10000f64.powf(-(j as f64) / medio as f64);
            v[j] = a.cos() as f32;
            v[medio + j] = a.sin() as f32;
        }
        let e = Array::from_slice(&v, &[1, 2 * medio as i32]);
        let te = self.t2.ap(&silu(&self.t1.ap(&e)?)?)?;
        let tm = self.tproy.ap(&silu(&te)?)?;
        let d = te.dim(-1);
        Ok((te, ops::reshape(&tm, &[1, 6, d]).m()?))
    }

    /// Una pasada: latentes [B, T, 128], el paso t (0–1000) y el contexto de cada fila → velocidad.
    pub fn ap(&self, lat: &Array, t: f32, ctx: &Contexto, giro_t: &(Array, Array), cancelar: Option<&AtomicBool>) -> Result<Array> {
        let (te, tm) = self.tiempo(t)?;
        let h = self.cabezas;
        let mut x = self.entrada.ap(lat)?;
        let escala = 1.0 / ((x.dim(2) / h) as f32).sqrt();
        let uno = Array::from_f32(1.0);
        for (i, b) in self.bloques.iter().enumerate() {
            if cancelar.is_some_and(|c| c.load(Ordering::Relaxed)) {
                bail!("cancelado");
            }
            let m = ops::split_equal(&ops::add(&b.sst, &tm).m()?, 6, 1).m()?;
            let ln = fast::layer_norm(&x, None::<&Array>, None::<&Array>, self.eps).m()?;
            let entrada = ops::add(&ops::multiply(&ln, &ops::add(&uno, &m[1]).m()?).m()?, &m[0]).m()?;
            let q = girar(&fast::rms_norm(&b.auto.q.ap(&entrada)?, Some(&b.auto.nq), self.eps).m()?, &giro_t.0, &giro_t.1, h)?;
            let k = girar(&fast::rms_norm(&b.auto.k.ap(&entrada)?, Some(&b.auto.nk), self.eps).m()?, &giro_t.0, &giro_t.1, h)?;
            let v = b.auto.v.ap(&entrada)?;
            let a = fast::scaled_dot_product_attention(&cabezas(&q, h)?, &cabezas(&k, h)?, &cabezas(&v, h)?, escala, None, None::<&Array>).m()?;
            x = ops::add(&x, &ops::multiply(&m[2], &b.auto.o.ap(&juntar(&a)?)?).m()?).m()?;
            // atención cruzada (sin modulación) con la norma afín
            let n3 = fast::layer_norm(&x, Some(&b.n3_w), Some(&b.n3_b), self.eps).m()?;
            let qc = fast::rms_norm(&b.cruz.q.ap(&n3)?, Some(&b.cruz.nq), self.eps).m()?;
            let (kc, vc) = &ctx.kv[i];
            let c = fast::scaled_dot_product_attention(&cabezas(&qc, h)?, kc, vc, escala, None, None::<&Array>).m()?;
            x = ops::add(&x, &b.cruz.o.ap(&juntar(&c)?)?).m()?;
            let ln = fast::layer_norm(&x, None::<&Array>, None::<&Array>, self.eps).m()?;
            let entrada = ops::add(&ops::multiply(&ln, &ops::add(&uno, &m[4]).m()?).m()?, &m[3]).m()?;
            x = ops::add(&x, &ops::multiply(&m[5], &b.ff2.ap(&gelu_tanh(&b.ff1.ap(&entrada)?)?)?).m()?).m()?;
            if cancelar.is_some() && i % 6 == 5 {
                x.eval().m()?;
            }
        }
        // cabeza: (desplazamiento, escala) = tabla + t (el embebido, no la proyección)
        let so = ops::split_equal(&ops::add(&self.sst_cabeza, &ops::expand_dims(&te, 1).m()?).m()?, 2, 1).m()?;
        let ln = fast::layer_norm(&x, None::<&Array>, None::<&Array>, self.eps).m()?;
        self.cabeza.ap(&ops::add(&ops::multiply(&ln, &ops::add(&uno, &so[1]).m()?).m()?, &so[0]).m()?)
    }
}

// ───────────────────────────── DAC ─────────────────────────────

/// Snake del DAC: `x + 1/(α + 1e-9) · sin(α·x)²` (α directa, sin exponencial).
struct Snake {
    a: Array,
    inv: Array,
}

impl Snake {
    fn de(p: &mut HashMap<String, Array>, n: &str) -> Result<Snake> {
        let a = ops::reshape(&quitar(p, &format!("{n}.alpha"))?, &[-1]).m()?;
        let inv = ops::divide(Array::from_f32(1.0), &ops::add(&a, Array::from_f32(1e-9)).m()?).m()?;
        Ok(Snake { a, inv })
    }
    fn ap(&self, x: &Array) -> Result<Array> {
        let s = ops::sin(&ops::multiply(&self.a, x).m()?).m()?;
        ops::add(x, &ops::multiply(&self.inv, &ops::square(&s).m()?).m()?).m()
    }
}

struct Residual {
    s1: Snake,
    c1: Conv,
    s2: Snake,
    c2: Conv,
    dil: i32,
}

struct BloqueDac {
    snake: Snake,
    sube: Conv,
    paso: i32,
    res: Vec<Residual>,
}

/// El decodificador del DAC continuo (`vae_128d_48k.pth`).
pub struct Dac {
    post: Conv,
    entrada: Conv,
    bloques: Vec<BloqueDac>,
    snake: Snake,
    salida: Conv,
}

impl Dac {
    pub fn cargar(pth: &Path) -> Result<Dac> {
        let (mut p, raiz) = crate::pt::cargar(pth)?;
        let kw = raiz.get("metadata").and_then(|m| m.get("kwargs")).map(|k| k.json()).ok_or_else(|| anyhow_tr!("{}: sin metadata.kwargs", "{}: no metadata.kwargs", pth.display()))?;
        if kw.get("continuous").and_then(|v| v.as_bool()) != Some(true) {
            bail_tr!("sólo el DAC continuo (el de MOSS)", "only the continuous DAC (the MOSS one)");
        }
        let pasos: Vec<i32> = serde_json::from_value(kw.get("decoder_rates").cloned().ok_or_else(|| anyhow_tr!("DAC sin decoder_rates", "DAC without decoder_rates"))?)?;
        p.retain(|k, _| k.starts_with("decoder.") || k.starts_with("post_quant_conv."));
        let mut bloques = Vec::new();
        for (i, &paso) in pasos.iter().enumerate() {
            let b = format!("decoder.model.{}.block", i + 1);
            let mut res = Vec::new();
            for (j, dil) in [1, 3, 9].into_iter().enumerate() {
                let r = format!("{b}.{}.block", j + 2);
                res.push(Residual { s1: Snake::de(&mut p, &format!("{r}.0"))?, c1: Conv::de(&mut p, &format!("{r}.1"), false)?, s2: Snake::de(&mut p, &format!("{r}.2"))?, c2: Conv::de(&mut p, &format!("{r}.3"), false)?, dil });
            }
            bloques.push(BloqueDac { snake: Snake::de(&mut p, &format!("{b}.0"))?, sube: Conv::de(&mut p, &format!("{b}.1"), true)?, paso, res });
        }
        let k = pasos.len() + 1;
        let d = Dac {
            post: Conv::plana(&mut p, "post_quant_conv")?,
            entrada: Conv::de(&mut p, "decoder.model.0", false)?,
            bloques,
            snake: Snake::de(&mut p, &format!("decoder.model.{k}"))?,
            salida: Conv::de(&mut p, &format!("decoder.model.{}", k + 1), false)?,
        };
        if !p.is_empty() {
            let mut sobran: Vec<_> = p.keys().cloned().collect();
            sobran.sort();
            bail_tr!("pesos del DAC sin usar: {:?}", "unused DAC weights: {:?}", &sobran[..sobran.len().min(5)]);
        }
        Ok(d)
    }

    /// Latentes [1, T, 128] → audio [1, T·960, 1] en [−1, 1].
    pub fn decodificar(&self, z: &Array, cancelar: Option<&AtomicBool>) -> Result<Array> {
        let mut x = self.entrada.ap(&self.post.ap(z, 0, 1)?, 3, 1)?;
        for b in &self.bloques {
            if cancelar.is_some_and(|c| c.load(Ordering::Relaxed)) {
                bail!("cancelado");
            }
            // ConvTranspose: núcleo 2·paso, relleno ⌈paso/2⌉, `output_padding` = paso mód 2
            x = b.sube.ap_t_extra(&b.snake.ap(&x)?, b.paso, (b.paso + 1) / 2, b.paso % 2)?;
            for r in &b.res {
                let h = r.c1.ap(&r.s1.ap(&x)?, 3 * r.dil, r.dil)?;
                let h = r.c2.ap(&r.s2.ap(&h)?, 0, 1)?;
                x = ops::add(&x, &h).m()?;
            }
            x.eval().m()?;
        }
        ops::tanh(&self.salida.ap(&self.snake.ap(&x)?, 3, 1)?).m()
    }
}

// ───────────────────────────── el pipeline ─────────────────────────────

#[derive(Clone, Debug, PartialEq)]
pub enum Etapa {
    Texto,
    Paso(u32, u32),
    Audio,
}

/// Los vectores de texto [1, 512, 2048]: las posiciones reales del Qwen3 y ceros detrás.
fn codificar(te: &Modelo, ids: &[u32]) -> Result<Array> {
    let ceros = |n: i32| ops::zeros::<f32>(&[1, n, 2048]);
    if ids.is_empty() {
        return ceros(TEXTO).m();
    }
    let e = te.estado_final(ids)?;
    let n = e.dim(1);
    if n >= TEXTO {
        return e.try_index((.., ..TEXTO, ..)).m();
    }
    ops::concatenate(&[e, ceros(TEXTO - n).m()?], 1).m()
}

/// Genera el efecto. `ruido`: el del juez (`ruido` [1, 128, T]); `volcado`: dónde dejar cada pieza.
pub fn generar(dir: &Path, p: &Pedido, progreso: &dyn Fn(Etapa), cancelar: &AtomicBool, ruido: Option<&Path>, volcado: Option<&Path>) -> Result<Audio> {
    if !(0.1..=30.0).contains(&p.segundos) || p.segundos > p.segundos_latente {
        bail_tr!("duración fuera de rango (0,1–30 s): {}", "duration out of range (0.1–30 s): {}", p.segundos);
    }
    let guardar = |n: &str, a: &Array| -> Result<()> {
        if let Some(d) = volcado {
            std::fs::create_dir_all(d)?;
            Array::save_safetensors([(n, a)], None, d.join(format!("{n}.safetensors"))).map_err(|e| anyhow!("volcado {n}: {e:?}"))?;
        }
        Ok(())
    };
    let corte = || -> Result<()> {
        if cancelar.load(Ordering::Relaxed) {
            bail!("cancelado");
        }
        Ok(())
    };
    let t_lat = (((p.segundos_latente as f64) * FRECUENCIA as f64).round() as usize / SALTO) as i32;

    // 1. texto: positivo y negativo ("" → ceros)
    progreso(Etapa::Texto);
    let tk = tokenizers::Tokenizer::from_file(dir.join("tokenizer/tokenizer.json")).map_err(|e| anyhow!("tokenizer de MOSS: {e}"))?;
    let mut ids = tk.encode(texto(p).as_str(), true).map_err(|e| anyhow_tr!("tokenizar: {e}", "tokenizing: {e}"))?.get_ids().to_vec();
    ids.truncate(TEXTO as usize);
    let te = Modelo::cargar_f32(&dir.join("text_encoder"))?;
    let pos = codificar(&te, &ids)?;
    let neg = codificar(&te, &[])?;
    mlx_rs::transforms::eval([&pos, &neg]).m()?;
    drop(te);
    let _ = mlx_rs::memory::clear_cache();
    guardar("ids", &Array::from_slice(&ids.iter().map(|&i| i as i32).collect::<Vec<_>>(), &[1, ids.len() as i32]))?;
    guardar("ctx_pos", &pos)?;
    corte()?;

    // 2. DiT con CFG en un lote de 2 (fila 0 positiva, fila 1 negativa)
    let dit = Dit::cargar(&dir.join("transformer"))?;
    let ctx = dit.contexto(&ops::concatenate(&[pos, neg], 0).m()?)?;
    let giro_t = giro(t_lat, 128);
    let mut x = match ruido {
        Some(r) => {
            let n = Array::load_safetensors(r).map_err(|e| anyhow_tr!("ruido: {e:?}", "noise: {e:?}"))?.remove("ruido").ok_or_else(|| anyhow_tr!("el archivo no trae «ruido»", "the file has no «ruido»"))?.as_dtype(F32).m()?;
            ops::transpose_axes(&n, &[0, 2, 1]).m()?
        }
        None => mlx_rs::random::normal::<f32>(&[1, t_lat, 128], None, None, &mlx_rs::random::key(p.semilla).m()?).m()?,
    };
    if x.shape() != [1, t_lat, 128] {
        bail!("ruido de forma {:?} (se esperaba [1, {t_lat}, 128])", x.shape());
    }
    let s = sigmas(p.pasos, p.shift);
    let n = s.len();
    for i in 0..n {
        progreso(Etapa::Paso(i as u32 + 1, n as u32));
        let t = s[i] * 1000.0;
        let lote = ops::concatenate(&[x.clone(), x.clone()], 0).m()?;
        let v = dit.ap(&lote, t, &ctx, &giro_t, Some(cancelar))?;
        let (vp, vn) = (v.try_index((0..1, .., ..)).m()?, v.try_index((1..2, .., ..)).m()?);
        let g = ops::add(&vn, &ops::multiply(&ops::subtract(&vp, &vn).m()?, Array::from_f32(p.cfg)).m()?).m()?;
        guardar(&format!("v_{i}"), &g)?;
        let sig = if i + 1 < n { s[i + 1] } else { 0.0 };
        x = ops::add(&x, &ops::multiply(&g, Array::from_f32(sig - s[i])).m()?).m()?;
        x.eval().m()?;
        guardar(&format!("x_{i}"), &x)?;
        corte()?;
    }
    drop(ctx);
    drop(dit);
    let _ = mlx_rs::memory::clear_cache();

    // 3. audio: sólo los cuadros que hacen falta + un margen (el decodificador mira pocos cuadros
    // alrededor; lo recortado es igual que decodificar los 30 s)
    progreso(Etapa::Audio);
    let muestras = (segundos_1(p.segundos).parse::<f64>().unwrap_or(p.segundos as f64) * FRECUENCIA as f64) as usize;
    let cuadros = ((muestras.div_ceil(SALTO) + 32) as i32).min(t_lat);
    let dac = Dac::cargar(&dir.join("vae/vae_128d_48k.pth"))?;
    let onda = dac.decodificar(&x.try_index((.., ..cuadros, ..)).m()?, Some(cancelar))?;
    onda.eval().m()?;
    guardar("audio", &onda)?;
    let todo = onda.as_slice::<f32>();
    Ok(Audio { muestras: todo[..muestras.min(todo.len())].to_vec(), canales: 1, frecuencia: FRECUENCIA, plan: None })
}

/// El motor que registran la CLI y la app (firma sin tipos de MLX): escribe el WAV y devuelve los segundos.
pub fn generar_wav(dir: &Path, descripcion: &str, segundos: f32, semilla: u64, salida: &Path, progreso: &(dyn Fn(String) + Sync), cancelar: &AtomicBool) -> Result<f32> {
    let p = Pedido { descripcion: descripcion.to_string(), segundos, semilla, ..Default::default() };
    let audio = generar(
        dir,
        &p,
        &|e| {
            progreso(match e {
                Etapa::Texto => "texto".into(),
                Etapa::Paso(i, n) => tr!("paso {i}/{n}", "step {i}/{n}"),
                Etapa::Audio => "audio".into(),
            })
        },
        cancelar,
        None,
        None,
    )?;
    std::fs::write(salida, audio.wav()).with_context(|| format!("guardar {}", salida.display()))?;
    Ok(audio.segundos())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn el_texto_es_el_del_oficial() {
        let p = Pedido { descripcion: "  a heavy wooden   door\ncreaks &amp; groans ".into(), segundos: 2.04, ..Default::default() };
        assert_eq!(texto(&p), "a heavy wooden door creaks & groans duration: 2.0s");
        let q = Pedido { descripcion: "rain".into(), segundos: 12.35, ..Default::default() };
        assert_eq!(texto(&q), "rain duration: 12.3s");
        assert_eq!(desescapar("&lt;x&gt; &#39;y&#x27; &foo; & z"), "<x> 'y' &foo; & z");
    }

    #[test]
    fn sigmas_como_el_planificador() {
        // linspace(1, 0, 4)[:-1] = [1, 2/3, 1/3]; shift 5 → 5σ/(1+4σ)
        let s = sigmas(3, 5.0);
        assert_eq!(s.len(), 3);
        assert!((s[0] - 1.0).abs() < 1e-7 && (s[1] - (10.0 / 3.0) / (1.0 + 8.0 / 3.0)).abs() < 1e-6 && (s[2] - (5.0 / 3.0) / (1.0 + 4.0 / 3.0)).abs() < 1e-6);
        assert_eq!(sigmas(100, 5.0).len(), 100);
    }

    #[test]
    fn el_wav_mono_es_valido() {
        let a = Audio { muestras: vec![0.0, 0.5, -0.5], canales: 1, frecuencia: FRECUENCIA, plan: None };
        let w = a.wav();
        assert_eq!(&w[..4], b"RIFF");
        assert_eq!(u16::from_le_bytes([w[22], w[23]]), 1);
    }
}
