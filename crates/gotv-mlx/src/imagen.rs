//! M4 · imagen local: FLUX.2 klein 4B (Black Forest Labs, Apache-2.0) sobre MLX, escrito a mano.
//!
//! Mismo cálculo que `diffusers.Flux2KleinPipeline` (0.43) en bf16, redondeo a redondeo donde se
//! puede:
//! 1. **Texto**: la plantilla de chat de Qwen3 (`enable_thinking=False`), relleno hasta 512 tokens,
//!    y los estados de las capas 9, 18 y 27 del Qwen3 4B pegados → [1, 512, 7680].
//! 2. **Transformador** (`Flux2Transformer2DModel`): 5 bloques dobles (imagen y texto con pesos
//!    propios, atención conjunta) + 20 simples (una corriente, atención y MLP en paralelo), RoPE de
//!    4 ejes (t, alto, ancho, l), modulación por el paso de tiempo, sin sesgos.
//! 3. **Planificador**: Euler de flujo con corrimiento dinámico exponencial (`mu` empírico de
//!    klein); destilado → sin guía libre de clasificador, ~4 pasos.
//! 4. **VAE** (`AutoencoderKLFlux2`): des-normalizar con la BatchNorm del VAE, des-parchear 2×2 y
//!    decodificar (medio con atención de 1 cabeza, 4 bloques de subida) → píxeles.
//!
//! El juez es la referencia oficial (PyTorch) fuera del proyecto: con `volcado` se guardan el
//! ruido, los embeddings y cada paso para compararlos pieza a pieza.

use crate::modelo::{Modelo, M};
use crate::plantilla::Plantilla;
use anyhow::{anyhow, bail, Context, Result};
use gotv_engine::{anyhow_tr, bail_tr, tr};
use mlx_rs::fast;
use mlx_rs::ops::indexing::TryIndexOp;
use mlx_rs::ops;
use mlx_rs::{Array, Dtype};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

const BF: Dtype = Dtype::Bfloat16;
/// largo fijo del texto (`max_sequence_length` del pipeline)
pub const LARGO_TEXTO: usize = 512;
/// capas del Qwen3 cuyas salidas forman el embedding de texto
const CAPAS_TEXTO: [usize; 3] = [9, 18, 27];

/// Lo que se pide.
#[derive(Clone, Debug)]
pub struct Pedido {
    pub prompt: String,
    pub ancho: u32,
    pub alto: u32,
    pub pasos: u32,
    pub semilla: u64,
    /// imágenes de referencia (M5; Ola PRO: hasta 4): klein las ve como tokens aparte (t = 10, 20, 30,
    /// 40) y las sigue
    pub guias: Vec<Guia>,
    /// Ola PRO · klein BASE: otra carpeta de transformador (el sin destilar); el texto y el VAE son
    /// los mismos archivos que los de klein y se leen de `dir`
    pub transformador: Option<std::path::PathBuf>,
    /// CFG (sólo el base: el destilado la ignora): `neg + guía·(cond − neg)` como el pipeline oficial;
    /// 1 = apagada
    pub guia_cfg: f32,
    /// el prompt NEGATIVO del CFG (el pipeline oficial usa «»)
    pub negativo: String,
}

impl Default for Pedido {
    fn default() -> Self {
        Pedido { prompt: String::new(), ancho: 1024, alto: 1024, pasos: 4, semilla: 0, guias: Vec::new(), transformador: None, guia_cfg: 1.0, negativo: String::new() }
    }
}

/// La imagen lista: RGB de 8 bits, fila a fila.
pub struct Imagen {
    pub ancho: u32,
    pub alto: u32,
    pub rgb: Vec<u8>,
}

impl Imagen {
    pub fn png(&self, ruta: &Path) -> Result<()> {
        let img = image::RgbImage::from_raw(self.ancho, self.alto, self.rgb.clone()).ok_or_else(|| anyhow_tr!("imagen de tamaño incoherente", "image with an inconsistent size"))?;
        img.save_with_format(ruta, image::ImageFormat::Png).with_context(|| format!("guardar {}", ruta.display()))
    }
}

/// Redondeo a bf16 (al par más cercano), como hace torch al guardar en bf16.
pub(crate) fn bf16(x: f32) -> f32 {
    let b = x.to_bits();
    let r = 0x7FFF + ((b >> 16) & 1);
    f32::from_bits(b.wrapping_add(r) & 0xFFFF_0000)
}

/// Escalar del tipo pedido (en MLX un escalar f32 sube un bf16 a f32: hay que darle su tipo).
fn esc(v: f32, d: Dtype) -> Result<Array> {
    Array::from_f32(v).as_dtype(d).m()
}

/// SiLU de torch en bf16: la cuenta en f32 y un solo redondeo.
fn silu(x: &Array) -> Result<Array> {
    let f = x.as_dtype(Dtype::Float32).m()?;
    ops::multiply(&f, &ops::sigmoid(&f).m()?).m()?.as_dtype(x.dtype()).m()
}

/// SwiGLU de FLUX.2: silu(primera mitad) × segunda mitad.
fn swiglu(x: &Array) -> Result<Array> {
    let p = ops::split_equal(x, 2, -1).m()?;
    ops::multiply(&silu(&p[0])?, &p[1]).m()
}

/// RMSNorm de diffusers: varianza en f32, x·rsqrt en f32, → bf16, × peso.
fn rms(x: &Array, w: &Array, eps: f32) -> Result<Array> {
    let xf = x.as_dtype(Dtype::Float32).m()?;
    let var = ops::mean_axes(&ops::square(&xf).m()?, &[-1], true).m()?;
    let n = ops::multiply(&xf, &ops::rsqrt(&ops::add(&var, Array::from_f32(eps)).m()?).m()?).m()?;
    ops::multiply(&n.as_dtype(w.dtype()).m()?, w).m()
}

/// `(1 + escala)·x + corrimiento` en bf16, en el orden de diffusers.
fn modular(x: &Array, escala: &Array, corr: &Array) -> Result<Array> {
    let uno = esc(1.0, x.dtype())?;
    ops::add(&ops::multiply(&ops::add(&uno, escala).m()?, x).m()?, corr).m()
}

/// Lineal sin sesgo (peso ya transpuesto [entrada, salida]).
struct Lin(Array);

impl Lin {
    fn de(p: &mut HashMap<String, Array>, nombre: &str) -> Result<Lin> {
        let w = p.remove(&format!("{nombre}.weight")).ok_or_else(|| anyhow!("falta {nombre}.weight"))?;
        Ok(Lin(ops::transpose_axes(&w, &[1, 0]).m()?))
    }
    fn ap(&self, x: &Array) -> Result<Array> {
        ops::matmul(x, &self.0).m()
    }
}

fn quitar(p: &mut HashMap<String, Array>, n: &str) -> Result<Array> {
    p.remove(n).ok_or_else(|| anyhow!("falta {n}"))
}

struct Doble {
    q: Lin,
    k: Lin,
    v: Lin,
    o: Lin,
    aq: Lin,
    ak: Lin,
    av: Lin,
    ao: Lin,
    nq: Array,
    nk: Array,
    naq: Array,
    nak: Array,
    ff_in: Lin,
    ff_out: Lin,
    ffc_in: Lin,
    ffc_out: Lin,
}

struct Simple {
    qkv_mlp: Lin,
    salida: Lin,
    nq: Array,
    nk: Array,
}

/// `Flux2Transformer2DModel` de klein 4B.
pub struct Transformador {
    t1: Lin,
    t2: Lin,
    mod_img: Lin,
    mod_txt: Lin,
    mod_uno: Lin,
    x_emb: Lin,
    ctx_emb: Lin,
    dobles: Vec<Doble>,
    simples: Vec<Simple>,
    norma_fin: Lin,
    proy_fin: Lin,
    cabezas: i32,
    dim_cabeza: i32,
    eps: f32,
    ejes: Vec<i32>,
    theta: f64,
}

/// La RoPE ya armada para una secuencia: cos y sin [1, S, 1, 128] en f32.
pub struct Giro {
    cos: Array,
    sin: Array,
}

impl Transformador {
    pub fn cargar(dir: &Path) -> Result<Transformador> {
        let cfg: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dir.join("config.json")).context("transformer/config.json")?)?;
        let n = |k: &str| cfg.get(k).and_then(|v| v.as_i64()).ok_or_else(|| anyhow_tr!("config del transformador sin {k}", "transformer config without {k}"));
        let (dobles_n, simples_n, cabezas, dim_cabeza) = (n("num_layers")?, n("num_single_layers")?, n("num_attention_heads")? as i32, n("attention_head_dim")? as i32);
        if cfg.get("guidance_embeds").and_then(|v| v.as_bool()).unwrap_or(false) {
            bail_tr!("este FLUX.2 lleva guía embebida (dev): sólo klein por ahora", "this FLUX.2 has embedded guidance (dev): only klein for now");
        }
        let ejes: Vec<i32> = serde_json::from_value(cfg.get("axes_dims_rope").cloned().unwrap_or(serde_json::json!([32, 32, 32, 32])))?;
        if ejes.iter().sum::<i32>() != dim_cabeza {
            bail_tr!("ejes de RoPE {ejes:?} no suman la cabeza {dim_cabeza}", "RoPE axes {ejes:?} do not add up to head {dim_cabeza}");
        }
        let theta = cfg.get("rope_theta").and_then(|v| v.as_f64()).unwrap_or(2000.0);
        let eps = cfg.get("eps").and_then(|v| v.as_f64()).unwrap_or(1e-6) as f32;
        let mut archivos: Vec<PathBuf> = std::fs::read_dir(dir)?.filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.extension().is_some_and(|x| x == "safetensors")).collect();
        archivos.sort();
        let mut p = HashMap::new();
        for a in &archivos {
            p.extend(Array::load_safetensors(a).map_err(|e| anyhow!("{}: {e:?}", a.display()))?);
        }
        let mut dobles = Vec::new();
        for i in 0..dobles_n {
            let b = format!("transformer_blocks.{i}");
            dobles.push(Doble {
                q: Lin::de(&mut p, &format!("{b}.attn.to_q"))?,
                k: Lin::de(&mut p, &format!("{b}.attn.to_k"))?,
                v: Lin::de(&mut p, &format!("{b}.attn.to_v"))?,
                o: Lin::de(&mut p, &format!("{b}.attn.to_out.0"))?,
                aq: Lin::de(&mut p, &format!("{b}.attn.add_q_proj"))?,
                ak: Lin::de(&mut p, &format!("{b}.attn.add_k_proj"))?,
                av: Lin::de(&mut p, &format!("{b}.attn.add_v_proj"))?,
                ao: Lin::de(&mut p, &format!("{b}.attn.to_add_out"))?,
                nq: quitar(&mut p, &format!("{b}.attn.norm_q.weight"))?,
                nk: quitar(&mut p, &format!("{b}.attn.norm_k.weight"))?,
                naq: quitar(&mut p, &format!("{b}.attn.norm_added_q.weight"))?,
                nak: quitar(&mut p, &format!("{b}.attn.norm_added_k.weight"))?,
                ff_in: Lin::de(&mut p, &format!("{b}.ff.linear_in"))?,
                ff_out: Lin::de(&mut p, &format!("{b}.ff.linear_out"))?,
                ffc_in: Lin::de(&mut p, &format!("{b}.ff_context.linear_in"))?,
                ffc_out: Lin::de(&mut p, &format!("{b}.ff_context.linear_out"))?,
            });
        }
        let mut simples = Vec::new();
        for i in 0..simples_n {
            let b = format!("single_transformer_blocks.{i}");
            simples.push(Simple {
                qkv_mlp: Lin::de(&mut p, &format!("{b}.attn.to_qkv_mlp_proj"))?,
                salida: Lin::de(&mut p, &format!("{b}.attn.to_out"))?,
                nq: quitar(&mut p, &format!("{b}.attn.norm_q.weight"))?,
                nk: quitar(&mut p, &format!("{b}.attn.norm_k.weight"))?,
            });
        }
        let t = Transformador {
            t1: Lin::de(&mut p, "time_guidance_embed.timestep_embedder.linear_1")?,
            t2: Lin::de(&mut p, "time_guidance_embed.timestep_embedder.linear_2")?,
            mod_img: Lin::de(&mut p, "double_stream_modulation_img.linear")?,
            mod_txt: Lin::de(&mut p, "double_stream_modulation_txt.linear")?,
            mod_uno: Lin::de(&mut p, "single_stream_modulation.linear")?,
            x_emb: Lin::de(&mut p, "x_embedder")?,
            ctx_emb: Lin::de(&mut p, "context_embedder")?,
            norma_fin: Lin::de(&mut p, "norm_out.linear")?,
            proy_fin: Lin::de(&mut p, "proj_out")?,
            dobles,
            simples,
            cabezas,
            dim_cabeza,
            eps,
            ejes,
            theta,
        };
        if !p.is_empty() {
            let mut sobran: Vec<_> = p.keys().cloned().collect();
            sobran.sort();
            bail_tr!("pesos del transformador sin usar ({}): {:?}…", "unused transformer weights ({}): {:?}…", sobran.len(), &sobran[..sobran.len().min(5)]);
        }
        Ok(t)
    }

    /// RoPE de `Flux2PosEmbed` para estas posiciones (t, alto, ancho, l): por eje
    /// `1/θ^(2i/d)` y ángulos en f64, cos/sin repetidos de a 2 (`repeat_interleave_real`), → f32.
    pub fn giro(&self, ids: &[[i32; 4]]) -> Result<Giro> {
        let d: i32 = self.ejes.iter().sum();
        let mut cos = Vec::with_capacity(ids.len() * d as usize);
        let mut sin = Vec::with_capacity(ids.len() * d as usize);
        for id in ids {
            for (eje, &dim) in self.ejes.iter().enumerate() {
                for i in 0..dim / 2 {
                    let f = 1.0 / self.theta.powf((2 * i) as f64 / dim as f64);
                    let a = id[eje] as f64 * f;
                    let (c, s) = (a.cos() as f32, a.sin() as f32);
                    cos.extend([c, c]);
                    sin.extend([s, s]);
                }
            }
        }
        let s = ids.len() as i32;
        Ok(Giro { cos: Array::from_slice(&cos, &[1, s, 1, d]), sin: Array::from_slice(&sin, &[1, s, 1, d]) })
    }

    /// `apply_rotary_emb` (pares reales entrelazados): x [1, S, H, D] → (x·cos + rot(x)·sin) en f32 → bf16.
    fn girar(x: &Array, g: &Giro) -> Result<Array> {
        let sh = x.shape().to_vec();
        let xf = x.as_dtype(Dtype::Float32).m()?;
        let pares = ops::reshape(&xf, &[sh[0], sh[1], sh[2], sh[3] / 2, 2]).m()?;
        let re = pares.try_index((.., .., .., .., 0)).m()?;
        let im = pares.try_index((.., .., .., .., 1)).m()?;
        let rot = ops::reshape(&ops::stack(&[ops::negative(&im).m()?, re], -1).m()?, &sh).m()?;
        ops::add(&ops::multiply(&xf, &g.cos).m()?, &ops::multiply(&rot, &g.sin).m()?).m()?.as_dtype(x.dtype()).m()
    }

    fn atender(&self, q: &Array, k: &Array, v: &Array) -> Result<Array> {
        let (s, h, d) = (q.dim(1), self.cabezas, self.dim_cabeza);
        let t = |a: &Array| ops::transpose_axes(a, &[0, 2, 1, 3]).m();
        let a = fast::scaled_dot_product_attention(&t(q)?, &t(k)?, &t(v)?, 1.0 / (d as f32).sqrt(), None, None::<&Array>).m()?;
        ops::reshape(&t(&a)?, &[1, s, h * d]).m()
    }

    fn cabezas(&self, x: &Array) -> Result<Array> {
        ops::reshape(x, &[1, x.dim(1), self.cabezas, self.dim_cabeza]).m()
    }

    /// El embedding del paso de tiempo `temb` [1, 3072] para el valor t YA en la cadena de bf16
    /// del pipeline (`Timesteps(256, flip_sin_to_cos)` en f32 → bf16 → lineal → SiLU → lineal).
    fn temb(&self, t: f32) -> Result<Array> {
        let medio = 128;
        let c = -(10000f64.ln()) as f32;
        let mut v = vec![0f32; 2 * medio];
        for j in 0..medio {
            let w = ((c * j as f32) / medio as f32).exp();
            let a = t * w;
            v[j] = a.cos();
            v[medio + j] = a.sin();
        }
        let e = Array::from_slice(&v, &[1, 2 * medio as i32]).as_dtype(BF).m()?;
        self.t2.ap(&silu(&self.t1.ap(&e)?)?)
    }

    /// Una pasada: latentes empaquetados [1, N, 128] (bf16), texto YA proyectado por
    /// `context_embedder` [1, T, 3072], t (cadena bf16 del pipeline) → velocidad [1, N, 128].
    pub fn adelante(&self, x: &Array, ctx: &Array, t: f32, giro: &Giro, cancelar: Option<&AtomicBool>) -> Result<Array> {
        let temb = self.temb(t)?;
        let st = silu(&temb)?;
        let partir = |m: Array, n: i32| -> Result<Vec<Array>> { ops::split_equal(&ops::expand_dims(&m, 1).m()?, n, -1).m() };
        let mi = partir(self.mod_img.ap(&st)?, 6)?;
        let mt = partir(self.mod_txt.ap(&st)?, 6)?;
        let mu = partir(self.mod_uno.ap(&st)?, 3)?;
        let mut img = self.x_emb.ap(x)?;
        let mut txt = ctx.clone();
        let nt = txt.dim(1);
        let eps = self.eps;
        let cortar = || cancelar.is_some_and(|c| c.load(Ordering::Relaxed));
        for b in &self.dobles {
            if cortar() {
                bail!("cancelado");
            }
            let nh = modular(&fast::layer_norm(&img, None, None, eps).m()?, &mi[1], &mi[0])?;
            let nc = modular(&fast::layer_norm(&txt, None, None, eps).m()?, &mt[1], &mt[0])?;
            let q = rms(&self.cabezas(&b.q.ap(&nh)?)?, &b.nq, eps)?;
            let k = rms(&self.cabezas(&b.k.ap(&nh)?)?, &b.nk, eps)?;
            let v = self.cabezas(&b.v.ap(&nh)?)?;
            let eq = rms(&self.cabezas(&b.aq.ap(&nc)?)?, &b.naq, eps)?;
            let ek = rms(&self.cabezas(&b.ak.ap(&nc)?)?, &b.nak, eps)?;
            let ev = self.cabezas(&b.av.ap(&nc)?)?;
            let q = Self::girar(&ops::concatenate(&[eq, q], 1).m()?, giro)?;
            let k = Self::girar(&ops::concatenate(&[ek, k], 1).m()?, giro)?;
            let v = ops::concatenate(&[ev, v], 1).m()?;
            let a = self.atender(&q, &k, &v)?;
            let ta = b.ao.ap(&a.try_index((.., ..nt, ..)).m()?)?;
            let ia = b.o.ap(&a.try_index((.., nt.., ..)).m()?)?;
            img = ops::add(&img, &ops::multiply(&mi[2], &ia).m()?).m()?;
            let nh = fast::layer_norm(&img, None, None, eps).m()?;
            let nh = ops::add(&ops::multiply(&nh, &ops::add(&esc(1.0, BF)?, &mi[4]).m()?).m()?, &mi[3]).m()?;
            img = ops::add(&img, &ops::multiply(&mi[5], &b.ff_out.ap(&swiglu(&b.ff_in.ap(&nh)?)?)?).m()?).m()?;
            txt = ops::add(&txt, &ops::multiply(&mt[2], &ta).m()?).m()?;
            let nc = fast::layer_norm(&txt, None, None, eps).m()?;
            let nc = ops::add(&ops::multiply(&nc, &ops::add(&esc(1.0, BF)?, &mt[4]).m()?).m()?, &mt[3]).m()?;
            txt = ops::add(&txt, &ops::multiply(&mt[5], &b.ffc_out.ap(&swiglu(&b.ffc_in.ap(&nc)?)?)?).m()?).m()?;
            // MLX es perezoso: sin evaluar aquí, el ⏹ sólo se notaba al final del paso (medido: 5,4 s a
            // 1024 px); evaluando bloque a bloque corta en el siguiente bloque
            if cancelar.is_some() {
                mlx_rs::transforms::eval([&img, &txt]).m()?;
            }
        }
        let mut h = ops::concatenate(&[txt, img], 1).m()?;
        let dq = 3 * self.cabezas * self.dim_cabeza;
        for b in &self.simples {
            if cortar() {
                bail!("cancelado");
            }
            let n = modular(&fast::layer_norm(&h, None, None, eps).m()?, &mu[1], &mu[0])?;
            let p = b.qkv_mlp.ap(&n)?;
            let partes = ops::split_at_indices(&p, &[dq], -1).m()?;
            let qkv = ops::split_equal(&partes[0], 3, -1).m()?;
            let q = Self::girar(&rms(&self.cabezas(&qkv[0])?, &b.nq, eps)?, giro)?;
            let k = Self::girar(&rms(&self.cabezas(&qkv[1])?, &b.nk, eps)?, giro)?;
            let v = self.cabezas(&qkv[2])?;
            let a = self.atender(&q, &k, &v)?;
            let o = b.salida.ap(&ops::concatenate(&[a, swiglu(&partes[1])?], -1).m()?)?;
            h = ops::add(&h, &ops::multiply(&mu[2], &o).m()?).m()?;
            if cancelar.is_some() {
                h.eval().m()?;
            }
        }
        let h = h.try_index((.., nt.., ..)).m()?;
        // AdaLayerNormContinuous: lineal(silu(temb)) → (escala, corrimiento) — la ESCALA primero
        let e = ops::split_equal(&self.norma_fin.ap(&st)?, 2, -1).m()?;
        let (escala, corr) = (ops::expand_dims(&e[0], 1).m()?, ops::expand_dims(&e[1], 1).m()?);
        let n = fast::layer_norm(&h, None, None, eps).m()?;
        let n = ops::add(&ops::multiply(&n, &ops::add(&esc(1.0, BF)?, &escala).m()?).m()?, &corr).m()?;
        self.proy_fin.ap(&n)
    }

    /// El texto proyectado una vez (es igual en todos los pasos).
    pub fn contexto(&self, prompt_embeds: &Array) -> Result<Array> {
        self.ctx_emb.ap(prompt_embeds)
    }
}

/// `compute_empirical_mu` de klein.
pub fn mu_empirico(largo: usize, pasos: u32) -> f64 {
    let (a1, b1, a2, b2) = (8.73809524e-05, 1.89833333, 0.00016927, 0.45666666);
    let s = largo as f64;
    if largo > 4300 {
        return a2 * s + b2;
    }
    let m200 = a2 * s + b2;
    let m10 = a1 * s + b1;
    let a = (m200 - m10) / 190.0;
    let b = m200 - 200.0 * a;
    a * pasos as f64 + b
}

/// Las sigmas (con el 0 final) de `FlowMatchEulerDiscreteScheduler` con corrimiento dinámico
/// exponencial, en f32 como numpy/torch: `linspace(1, 1/N, N)` → f32 → `e^mu/(e^mu + (1/t − 1))`.
pub fn sigmas(pasos: u32, largo: usize) -> Vec<f32> {
    let n = pasos.max(1) as usize;
    let mu = mu_empirico(largo, pasos);
    let e = mu.exp() as f32;
    let mut s: Vec<f32> = (0..n)
        .map(|i| {
            let t = if n == 1 { 1.0 } else { 1.0 + (1.0 / n as f64 - 1.0) * i as f64 / (n - 1) as f64 };
            let t = t as f32;
            e / (e + (1.0 / t - 1.0))
        })
        .collect();
    s.push(0.0);
    s
}

/// El t que ve el transformador: `σ·1000` (f32) → bf16 → /1000 en bf16 → ×1000 en bf16.
pub fn t_del_paso(sigma: f32) -> f32 {
    let t = bf16(sigma * 1000.0);
    bf16(bf16(t / 1000.0) * 1000.0)
}

// ───────────────────────────── VAE ─────────────────────────────

struct Conv {
    w: Array,
    b: Array,
    pad: i32,
}

impl Conv {
    fn de(p: &mut HashMap<String, Array>, n: &str) -> Result<Conv> {
        let w = quitar(p, &format!("{n}.weight"))?;
        let pad = w.dim(2) / 2;
        // PyTorch [O, I, kh, kw] → MLX [O, kh, kw, I]
        Ok(Conv { w: ops::transpose_axes(&w, &[0, 2, 3, 1]).m()?, b: quitar(p, &format!("{n}.bias"))?, pad })
    }
    fn ap(&self, x: &Array) -> Result<Array> {
        ops::add(&ops::conv2d(x, &self.w, (1, 1), (self.pad, self.pad), (1, 1), 1).m()?, &self.b).m()
    }
}

struct Norma {
    w: Array,
    b: Array,
}

impl Norma {
    fn de(p: &mut HashMap<String, Array>, n: &str) -> Result<Norma> {
        Ok(Norma { w: quitar(p, &format!("{n}.weight"))?, b: quitar(p, &format!("{n}.bias"))? })
    }
    /// GroupNorm(32, eps 1e-6) sobre NHWC: media y varianza en f32, un solo redondeo al final.
    fn ap(&self, x: &Array) -> Result<Array> {
        let s = x.shape().to_vec();
        let (n, h, w, c) = (s[0], s[1], s[2], s[3]);
        let g = 32;
        let xf = ops::reshape(&x.as_dtype(Dtype::Float32).m()?, &[n, h * w, g, c / g]).m()?;
        let media = ops::mean_axes(&xf, &[1, 3], true).m()?;
        let dif = ops::subtract(&xf, &media).m()?;
        let var = ops::mean_axes(&ops::square(&dif).m()?, &[1, 3], true).m()?;
        let y = ops::multiply(&dif, &ops::rsqrt(&ops::add(&var, Array::from_f32(1e-6)).m()?).m()?).m()?;
        let y = ops::reshape(&y, &[n, h, w, c]).m()?;
        let y = ops::add(&ops::multiply(&y, &self.w.as_dtype(Dtype::Float32).m()?).m()?, &self.b.as_dtype(Dtype::Float32).m()?).m()?;
        y.as_dtype(x.dtype()).m()
    }
}

struct Resnet {
    n1: Norma,
    c1: Conv,
    n2: Norma,
    c2: Conv,
    atajo: Option<Conv>,
}

impl Resnet {
    fn de(p: &mut HashMap<String, Array>, n: &str) -> Result<Resnet> {
        let atajo = if p.contains_key(&format!("{n}.conv_shortcut.weight")) { Some(Conv::de(p, &format!("{n}.conv_shortcut"))?) } else { None };
        Ok(Resnet { n1: Norma::de(p, &format!("{n}.norm1"))?, c1: Conv::de(p, &format!("{n}.conv1"))?, n2: Norma::de(p, &format!("{n}.norm2"))?, c2: Conv::de(p, &format!("{n}.conv2"))?, atajo })
    }
    fn ap(&self, x: &Array) -> Result<Array> {
        let h = self.c1.ap(&silu(&self.n1.ap(x)?)?)?;
        let h = self.c2.ap(&silu(&self.n2.ap(&h)?)?)?;
        let x = match &self.atajo {
            Some(c) => c.ap(x)?,
            None => x.clone(),
        };
        ops::add(&x, &h).m()
    }
}

/// Lineal con sesgo (la atención del VAE): `addmm` como torch, un solo redondeo.
struct LinB {
    w_t: Array,
    b: Array,
}

impl LinB {
    fn de(p: &mut HashMap<String, Array>, n: &str) -> Result<LinB> {
        Ok(LinB { w_t: ops::transpose_axes(&quitar(p, &format!("{n}.weight"))?, &[1, 0]).m()?, b: quitar(p, &format!("{n}.bias"))? })
    }
    fn ap(&self, x: &Array) -> Result<Array> {
        ops::addmm(&self.b, x, &self.w_t, 1.0, 1.0).m()
    }
}

struct AtencionVae {
    norma: Norma,
    q: LinB,
    k: LinB,
    v: LinB,
    o: LinB,
}

impl AtencionVae {
    fn ap(&self, x: &Array) -> Result<Array> {
        let s = x.shape().to_vec();
        let (h, w, c) = (s[1], s[2], s[3]);
        let n = ops::reshape(&self.norma.ap(x)?, &[1, h * w, c]).m()?;
        let cab = |a: Array| ops::reshape(&a, &[1, 1, h * w, c]).m();
        let q = cab(self.q.ap(&n)?)?;
        let k = cab(self.k.ap(&n)?)?;
        let v = cab(self.v.ap(&n)?)?;
        let a = fast::scaled_dot_product_attention(&q, &k, &v, 1.0 / (c as f32).sqrt(), None, None::<&Array>).m()?;
        let a = self.o.ap(&ops::reshape(&a, &[1, h * w, c]).m()?)?;
        ops::add(&ops::reshape(&a, &[1, h, w, c]).m()?, x).m()
    }
}

/// El decodificador de `AutoencoderKLFlux2` y la BatchNorm de sus latentes.
pub struct Vae {
    bn_media: Array,
    bn_var: Array,
    bn_eps: f32,
    post: Conv,
    entrada: Conv,
    medio: (Resnet, AtencionVae, Resnet),
    subidas: Vec<(Vec<Resnet>, Option<Conv>)>,
    norma_fin: Norma,
    salida: Conv,
}

impl Vae {
    pub fn cargar(dir: &Path) -> Result<Vae> {
        let cfg: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dir.join("config.json")).context("vae/config.json")?)?;
        let canales: Vec<i32> = serde_json::from_value(cfg.get("block_out_channels").cloned().ok_or_else(|| anyhow_tr!("vae sin block_out_channels", "vae without block_out_channels"))?)?;
        let capas = cfg.get("layers_per_block").and_then(|v| v.as_i64()).unwrap_or(2) as usize;
        if cfg.get("norm_num_groups").and_then(|v| v.as_i64()) != Some(32) || cfg.get("mid_block_add_attention").and_then(|v| v.as_bool()) == Some(false) {
            bail_tr!("VAE con otra forma (grupos ≠ 32 o sin atención en el medio)", "VAE with another shape (groups ≠ 32 or no mid attention)");
        }
        let bn_eps = cfg.get("batch_norm_eps").and_then(|v| v.as_f64()).unwrap_or(1e-4) as f32;
        let mut p = Array::load_safetensors(dir.join("diffusion_pytorch_model.safetensors")).map_err(|e| anyhow!("vae: {e:?}"))?;
        // el codificador y la conv de cuantización son para imagen→latente (M5): fuera
        p.retain(|k, _| !k.starts_with("encoder.") && !k.starts_with("quant_conv.") && k != "bn.num_batches_tracked");
        // el decodificador corre en f32 (su config pide `force_upcast`; medido contra la verdad fp32: en
        // bf16 se alejaba 4× más que la referencia oficial); la BatchNorm de los latentes se queda en bf16
        for (k, v) in p.iter_mut() {
            if k.starts_with("decoder.") || k.starts_with("post_quant_conv.") {
                *v = v.as_dtype(Dtype::Float32).m()?;
            }
        }
        let m = "decoder.mid_block";
        let medio = (
            Resnet::de(&mut p, &format!("{m}.resnets.0"))?,
            AtencionVae {
                norma: Norma::de(&mut p, &format!("{m}.attentions.0.group_norm"))?,
                q: LinB::de(&mut p, &format!("{m}.attentions.0.to_q"))?,
                k: LinB::de(&mut p, &format!("{m}.attentions.0.to_k"))?,
                v: LinB::de(&mut p, &format!("{m}.attentions.0.to_v"))?,
                o: LinB::de(&mut p, &format!("{m}.attentions.0.to_out.0"))?,
            },
            Resnet::de(&mut p, &format!("{m}.resnets.1"))?,
        );
        let mut subidas = Vec::new();
        for i in 0..canales.len() {
            let b = format!("decoder.up_blocks.{i}");
            let resnets = (0..=capas).map(|j| Resnet::de(&mut p, &format!("{b}.resnets.{j}"))).collect::<Result<Vec<_>>>()?;
            let sube = if i + 1 < canales.len() { Some(Conv::de(&mut p, &format!("{b}.upsamplers.0.conv"))?) } else { None };
            subidas.push((resnets, sube));
        }
        let v = Vae {
            bn_media: quitar(&mut p, "bn.running_mean")?,
            bn_var: quitar(&mut p, "bn.running_var")?,
            bn_eps,
            post: Conv::de(&mut p, "post_quant_conv")?,
            entrada: Conv::de(&mut p, "decoder.conv_in")?,
            medio,
            subidas,
            norma_fin: Norma::de(&mut p, "decoder.conv_norm_out")?,
            salida: Conv::de(&mut p, "decoder.conv_out")?,
        };
        if !p.is_empty() {
            let mut sobran: Vec<_> = p.keys().cloned().collect();
            sobran.sort();
            bail_tr!("pesos del VAE sin usar: {:?}", "unused VAE weights: {:?}", &sobran[..sobran.len().min(5)]);
        }
        Ok(v)
    }

    /// Latentes empaquetados [1, h·w, 128] → latentes del VAE [1, 2h, 2w, 32] (NHWC): BatchNorm
    /// inversa en bf16 (`x·sqrt(var + eps) + media`) y des-parcheo 2×2 (canal = c·4 + 2·dy + dx).
    pub fn desempacar(&self, x: &Array, h: i32, w: i32) -> Result<Array> {
        let d = x.dtype();
        let std = ops::sqrt(&ops::add(&self.bn_var, &esc(self.bn_eps, self.bn_var.dtype())?).m()?).m()?.as_dtype(d).m()?;
        let x = ops::add(&ops::multiply(x, &std).m()?, &self.bn_media.as_dtype(d).m()?).m()?;
        // [1, h, w, 32, 2, 2] → [1, h, 2, w, 2, 32]
        let x = ops::reshape(&x, &[1, h, w, 32, 2, 2]).m()?;
        let x = ops::transpose_axes(&x, &[0, 1, 4, 2, 5, 3]).m()?;
        ops::reshape(&x, &[1, 2 * h, 2 * w, 32]).m()
    }

    /// Latentes [1, H/8, W/8, 32] → imagen [1, H, W, 3] en [-1, 1] (bf16).
    pub fn decodificar(&self, z: &Array, cancelar: Option<&AtomicBool>) -> Result<Array> {
        let z = z.as_dtype(Dtype::Float32).m()?;
        let mut x = self.entrada.ap(&self.post.ap(&z)?)?;
        x = self.medio.0.ap(&x)?;
        x = self.medio.1.ap(&x)?;
        x = self.medio.2.ap(&x)?;
        x.eval().m()?;
        for (resnets, sube) in &self.subidas {
            if cancelar.is_some_and(|c| c.load(Ordering::Relaxed)) {
                bail!("cancelado");
            }
            // evaluar resnet a resnet: a 1024 px cada una mueve cientos de MB y así no conviven
            for r in resnets {
                x = r.ap(&x)?;
                x.eval().m()?;
            }
            if let Some(c) = sube {
                // vecino más cercano ×2 = repetir filas y columnas
                x = ops::repeat_axis::<f32>(&ops::repeat_axis::<f32>(&x, 2, 1).m()?, 2, 2).m()?;
                x = c.ap(&x)?;
                x.eval().m()?;
            }
            let _ = mlx_rs::memory::clear_cache();
        }
        self.salida.ap(&silu(&self.norma_fin.ap(&x)?)?)
    }
}

/// El CODIFICADOR de `AutoencoderKLFlux2` (imagen → latente): la imagen de referencia con la que
/// klein condiciona (M5: la isométrica vectorial como guía de la ilustración). En f32, como el
/// decodificador; la BatchNorm y el empaquetado en bf16, como el pipeline.
pub struct Codificador {
    entrada: Conv,
    /// resnets y, salvo en el último, la conv de bajada (paso 2, relleno asimétrico (0,1,0,1))
    bajadas: Vec<(Vec<Resnet>, Option<Conv>)>,
    medio: (Resnet, AtencionVae, Resnet),
    norma_fin: Norma,
    salida: Conv,
    cuant: Conv,
    bn_media: Array,
    bn_var: Array,
    bn_eps: f32,
}

impl Codificador {
    pub fn cargar(dir: &Path) -> Result<Codificador> {
        let cfg: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dir.join("config.json")).context("vae/config.json")?)?;
        let canales: Vec<i32> = serde_json::from_value(cfg.get("block_out_channels").cloned().ok_or_else(|| anyhow_tr!("vae sin block_out_channels", "vae without block_out_channels"))?)?;
        let capas = cfg.get("layers_per_block").and_then(|v| v.as_i64()).unwrap_or(2) as usize;
        let bn_eps = cfg.get("batch_norm_eps").and_then(|v| v.as_f64()).unwrap_or(1e-4) as f32;
        let mut p = Array::load_safetensors(dir.join("diffusion_pytorch_model.safetensors")).map_err(|e| anyhow!("vae: {e:?}"))?;
        p.retain(|k, _| k.starts_with("encoder.") || k.starts_with("quant_conv.") || k == "bn.running_mean" || k == "bn.running_var");
        for (k, v) in p.iter_mut() {
            if !k.starts_with("bn.") {
                *v = v.as_dtype(Dtype::Float32).m()?;
            }
        }
        let mut bajadas = Vec::new();
        for i in 0..canales.len() {
            let b = format!("encoder.down_blocks.{i}");
            let resnets = (0..capas).map(|j| Resnet::de(&mut p, &format!("{b}.resnets.{j}"))).collect::<Result<Vec<_>>>()?;
            let baja = if i + 1 < canales.len() { Some(Conv::de(&mut p, &format!("{b}.downsamplers.0.conv"))?) } else { None };
            bajadas.push((resnets, baja));
        }
        let m = "encoder.mid_block";
        let c = Codificador {
            entrada: Conv::de(&mut p, "encoder.conv_in")?,
            bajadas,
            medio: (
                Resnet::de(&mut p, &format!("{m}.resnets.0"))?,
                AtencionVae {
                    norma: Norma::de(&mut p, &format!("{m}.attentions.0.group_norm"))?,
                    q: LinB::de(&mut p, &format!("{m}.attentions.0.to_q"))?,
                    k: LinB::de(&mut p, &format!("{m}.attentions.0.to_k"))?,
                    v: LinB::de(&mut p, &format!("{m}.attentions.0.to_v"))?,
                    o: LinB::de(&mut p, &format!("{m}.attentions.0.to_out.0"))?,
                },
                Resnet::de(&mut p, &format!("{m}.resnets.1"))?,
            ),
            norma_fin: Norma::de(&mut p, "encoder.conv_norm_out")?,
            salida: Conv::de(&mut p, "encoder.conv_out")?,
            cuant: Conv::de(&mut p, "quant_conv")?,
            bn_media: quitar(&mut p, "bn.running_mean")?,
            bn_var: quitar(&mut p, "bn.running_var")?,
            bn_eps,
        };
        if !p.is_empty() {
            let mut sobran: Vec<_> = p.keys().cloned().collect();
            sobran.sort();
            bail_tr!("pesos del codificador sin usar: {:?}", "unused encoder weights: {:?}", &sobran[..sobran.len().min(5)]);
        }
        Ok(c)
    }

    /// RGB de 8 bits (alto × ancho × 3, múltiplos de 16) → latentes EMPAQUETADOS y normalizados
    /// [1, (alto/16)·(ancho/16), 128] en bf16, como `_encode_vae_image` + `_pack_latents`:
    /// píxel/255·2 − 1 → codificar → media de la gaussiana («argmax») → parche 2×2 →
    /// `(x − media_bn)/sqrt(var_bn + eps)`.
    pub fn codificar(&self, g: &Guia) -> Result<Array> {
        let (h, w) = (g.alto as i32, g.ancho as i32);
        if h % 16 != 0 || w % 16 != 0 || g.rgb.len() != (h * w * 3) as usize {
            bail_tr!("guía de {}×{} incoherente (debe ser múltiplo de 16)", "inconsistent {}×{} guide (must be a multiple of 16)", g.ancho, g.alto);
        }
        let px = Array::from_slice(&g.rgb, &[1, h, w, 3]).as_dtype(Dtype::Float32).m()?;
        let mut x = ops::subtract(&ops::multiply(&ops::divide(&px, Array::from_f32(255.0)).m()?, Array::from_f32(2.0)).m()?, Array::from_f32(1.0)).m()?;
        x = self.entrada.ap(&x)?;
        for (resnets, baja) in &self.bajadas {
            for r in resnets {
                x = r.ap(&x)?;
            }
            if let Some(c) = baja {
                // Downsample2D de diffusers: relleno (0,1,0,1) con ceros y conv 3×3 de paso 2
                let rel = ops::pad(&x, &[(0, 0), (0, 1), (0, 1), (0, 0)], Array::from_f32(0.0), None).m()?;
                x = ops::add(&ops::conv2d(&rel, &c.w, (2, 2), (0, 0), (1, 1), 1).m()?, &c.b).m()?;
            }
            x.eval().m()?;
        }
        x = self.medio.0.ap(&x)?;
        x = self.medio.1.ap(&x)?;
        x = self.medio.2.ap(&x)?;
        let momentos = self.cuant.ap(&self.salida.ap(&silu(&self.norma_fin.ap(&x)?)?)?)?;
        // [1, H/8, W/8, 64]: media = los primeros 32 canales
        let z = momentos.try_index((.., .., .., ..32)).m()?;
        let (hz, wz) = (z.dim(1), z.dim(2));
        // parche 2×2: [1, hz/2, 2, wz/2, 2, 32] → [1, hz/2, wz/2, 32, 2, 2] (canal = c·4 + 2·dy + dx)
        let z = ops::reshape(&z, &[1, hz / 2, 2, wz / 2, 2, 32]).m()?;
        let z = ops::transpose_axes(&z, &[0, 1, 3, 5, 2, 4]).m()?;
        let z = ops::reshape(&z, &[1, hz / 2, wz / 2, 128]).m()?.as_dtype(BF).m()?;
        let std = ops::sqrt(&ops::add(&self.bn_var, &esc(self.bn_eps, self.bn_var.dtype())?).m()?).m()?.as_dtype(BF).m()?;
        let z = ops::divide(&ops::subtract(&z, &self.bn_media.as_dtype(BF).m()?).m()?, &std).m()?;
        let z = ops::reshape(&z, &[1, (hz / 2) * (wz / 2), 128]).m()?;
        z.eval().m()?;
        Ok(z)
    }
}

/// La imagen guía (RGB de 8 bits, fila a fila).
#[derive(Clone, Debug)]
pub struct Guia {
    pub ancho: u32,
    pub alto: u32,
    pub rgb: Vec<u8>,
}

impl Guia {
    /// Una guía desde un archivo: SVG (se rasteriza con resvg al tamaño pedido, centrado y sobre
    /// blanco) o PNG/JPEG/WebP (recorte centrado a múltiplos de 16; sin re-muestrear). Tope de 1024²
    /// píxeles de área, como el pipeline.
    pub fn de_archivo(ruta: &Path, ancho: u32, alto: u32) -> Result<Guia> {
        let ext = ruta.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
        if ext == "svg" {
            let svg = std::fs::read(ruta).with_context(|| format!("leer {}", ruta.display()))?;
            return Guia::de_svg(&svg, ancho, alto);
        }
        let img = image::open(ruta).with_context(|| format!("abrir {}", ruta.display()))?.to_rgb8();
        let (w0, h0) = img.dimensions();
        let (w, h) = ((w0 / 16) * 16, (h0 / 16) * 16);
        if w < 256 || h < 256 {
            bail_tr!("la guía es muy pequeña ({w0}×{h0}; mínimo 256×256)", "the guide is too small ({w0}×{h0}; minimum 256×256)");
        }
        if (w as u64) * (h as u64) > 1024 * 1024 {
            bail_tr!("la guía es muy grande ({w0}×{h0}); hasta 1024×1024 píxeles de área", "the guide is too large ({w0}×{h0}); up to 1024×1024 pixels of area");
        }
        let (x0, y0) = ((w0 - w) / 2, (h0 - h) / 2);
        let recorte = image::imageops::crop_imm(&img, x0, y0, w, h).to_image();
        Ok(Guia { ancho: w, alto: h, rgb: recorte.into_raw() })
    }

    pub fn de_svg(svg: &[u8], ancho: u32, alto: u32) -> Result<Guia> {
        let (ancho, alto) = ((ancho / 16) * 16, (alto / 16) * 16);
        if (ancho as u64) * (alto as u64) > 1024 * 1024 || ancho < 256 || alto < 256 {
            bail_tr!("tamaño de guía fuera de rango: {ancho}×{alto} (256 a 1024×1024 de área)", "guide size out of range: {ancho}×{alto} (256 to 1024×1024 of area)");
        }
        let mut op = resvg::usvg::Options::default();
        op.fontdb_mut().load_system_fonts();
        let arbol = resvg::usvg::Tree::from_data(svg, &op).map_err(|e| anyhow!("SVG ilegible: {e}"))?;
        let t = arbol.size();
        let s = (ancho as f32 / t.width()).min(alto as f32 / t.height());
        let (dx, dy) = ((ancho as f32 - t.width() * s) / 2.0, (alto as f32 - t.height() * s) / 2.0);
        let mut lienzo = resvg::tiny_skia::Pixmap::new(ancho, alto).ok_or_else(|| anyhow!("lienzo de {ancho}×{alto}"))?;
        lienzo.fill(resvg::tiny_skia::Color::WHITE);
        resvg::render(&arbol, resvg::tiny_skia::Transform::from_row(s, 0.0, 0.0, s, dx, dy), &mut lienzo.as_mut());
        // fondo opaco: RGBA premultiplicado con alfa 255 = RGB tal cual
        let rgb = lienzo.data().as_chunks::<4>().0.iter().flat_map(|p| [p[0], p[1], p[2]]).collect();
        Ok(Guia { ancho, alto, rgb })
    }
}

/// `(x·0.5 + 0.5).clamp(0, 1)` en bf16, luego `·255` y redondeo en f32 (el `postprocess` de diffusers).
pub fn a_pixeles(x: &Array) -> Result<Vec<u8>> {
    let d = x.dtype();
    let y = ops::add(&ops::multiply(x, &esc(0.5, d)?).m()?, &esc(0.5, d)?).m()?;
    let y = ops::clip(&y, (&esc(0.0, d)?, &esc(1.0, d)?)).m()?;
    let y = ops::round(&ops::multiply(&y.as_dtype(Dtype::Float32).m()?, Array::from_f32(255.0)).m()?, None).m()?;
    let y = y.as_dtype(Dtype::Uint8).m()?;
    y.eval().m()?;
    Ok(y.as_slice::<u8>().to_vec())
}

// ───────────────────────────── el pipeline ─────────────────────────────

/// Los tokens del texto como el pipeline: plantilla de chat del modelo, relleno a la derecha
/// hasta 512 con `<|endoftext|>`, truncado a 512. Devuelve (ids, cuántos son reales).
pub fn tokens(dir: &Path, prompt: &str) -> Result<(Vec<u32>, usize)> {
    let tdir = dir.join("tokenizer");
    let plantilla = Plantilla::cargar(&tdir)?;
    let texto = plantilla.prompt(&[gotv_engine::Message::user(prompt)], None)?;
    let tk = tokenizers::Tokenizer::from_file(tdir.join("tokenizer.json")).map_err(|e| anyhow!("tokenizer: {e}"))?;
    let enc = tk.encode(texto.as_str(), true).map_err(|e| anyhow_tr!("tokenizar: {e}", "tokenizing: {e}"))?;
    let mut ids: Vec<u32> = enc.get_ids().to_vec();
    ids.truncate(LARGO_TEXTO);
    let reales = ids.len();
    if reales == 0 {
        bail_tr!("el texto quedó vacío", "the text ended up empty");
    }
    let tc: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(tdir.join("tokenizer_config.json"))?)?;
    let pad_txt = tc.get("pad_token").and_then(|p| p.as_str().or_else(|| p.get("content").and_then(|c| c.as_str()))).unwrap_or("<|endoftext|>");
    let pad = tk.token_to_id(pad_txt).ok_or_else(|| anyhow!("token de relleno {pad_txt} desconocido"))?;
    ids.resize(LARGO_TEXTO, pad);
    Ok((ids, reales))
}

/// El embedding de texto [1, 512, 7680] (bf16): carga el Qwen3, lo pasa y lo suelta.
pub fn embeber_texto(dir: &Path, prompt: &str) -> Result<Array> {
    let (ids, reales) = tokens(dir, prompt)?;
    let te = Modelo::cargar(&dir.join("text_encoder"))?;
    // en f32 y redondeado a bf16 UNA vez al final (como `out.to(dtype)` del pipeline). Medido contra la
    // verdad fp32 en 4 descripciones: en bf16 el texto quedaba 1,1–1,3× más lejos que el oficial y la
    // imagen final lo arrastraba (PSNR a la verdad 25,9 dB de media contra 28,7 del oficial)
    let capas = te.ocultos(&ids, reales, &CAPAS_TEXTO, true)?;
    // [1, 512, 2560] × 3 → [1, 512, 3, 2560] → [1, 512, 7680] (por token: capa 9 | 18 | 27)
    let e = ops::reshape(&ops::stack(&capas, 2).m()?, &[1, LARGO_TEXTO as i32, -1]).m()?.as_dtype(BF).m()?;
    e.eval().m()?;
    // los 8 GB del codificador se van ANTES de cargar el transformador (y su caché de MLX con ellos)
    drop(capas);
    drop(te);
    let _ = mlx_rs::memory::clear_cache();
    Ok(e)
}

/// Qué va pasando (para la barra de progreso).
#[derive(Clone, Debug, PartialEq)]
pub enum Etapa {
    Texto,
    Paso(u32, u32),
    Vae,
}

/// Genera la imagen. `volcado`: carpeta donde dejar ruido, embeddings y cada paso (el juez).
pub fn generar(dir: &Path, pedido: &Pedido, progreso: &dyn Fn(Etapa), cancelar: &AtomicBool, volcado: Option<&Path>) -> Result<Imagen> {
    // el VAE baja ×8 y el parche junta 2×2 → múltiplos de 16
    let (ancho, alto) = ((pedido.ancho / 16) * 16, (pedido.alto / 16) * 16);
    if !(256..=2048).contains(&ancho) || !(256..=2048).contains(&alto) {
        bail_tr!("tamaño fuera de rango: {ancho}×{alto} (256–2048, múltiplos de 16)", "size out of range: {ancho}×{alto} (256–2048, multiples of 16)");
    }
    if !(1..=50).contains(&pedido.pasos) {
        bail!("pasos fuera de rango (1–50): {}", pedido.pasos);
    }
    let (h, w) = ((alto / 16) as i32, (ancho / 16) as i32);
    let n = (h * w) as usize;
    let guardar = |nombre: &str, a: &Array| -> Result<()> {
        if let Some(d) = volcado {
            std::fs::create_dir_all(d)?;
            Array::save_safetensors([(nombre, a)], None, d.join(format!("{nombre}.safetensors"))).map_err(|e| anyhow!("volcado {nombre}: {e:?}"))?;
        }
        Ok(())
    };

    progreso(Etapa::Texto);
    let embeds = embeber_texto(dir, &pedido.prompt)?;
    // Ola PRO · klein base: el negativo con el MISMO codificador de texto (sólo si hay CFG)
    let cfg = pedido.guia_cfg > 1.0;
    let embeds_neg = if cfg { Some(embeber_texto(dir, &pedido.negativo)?) } else { None };
    guardar("prompt_embeds", &embeds)?;
    if cancelar.load(Ordering::Relaxed) {
        bail!("cancelado");
    }
    // M5: la guía se codifica ANTES de cargar el transformador (el codificador pesa poco y se suelta)
    // Ola PRO: hasta 4 referencias (`input_image` … `input_image_4` de BFL), cada una con su t
    anyhow::ensure!(pedido.guias.len() <= 4, gotv_engine::tr!("hasta 4 imágenes de referencia ({} dadas)", "up to 4 reference images ({} given)", pedido.guias.len()));
    let mut referencias: Vec<(Array, i32, i32)> = Vec::new();
    if !pedido.guias.is_empty() {
        let cod = Codificador::cargar(&dir.join("vae"))?;
        for (i, g) in pedido.guias.iter().enumerate() {
            let z = cod.codificar(g)?;
            z.eval().m()?;
            // el volcado del juez conserva el nombre de siempre para la primera
            let nombre = if i == 0 { "guia".to_string() } else { format!("guia_{i}") };
            guardar(&nombre, &z)?;
            if let Some(d) = volcado {
                Imagen { ancho: g.ancho, alto: g.alto, rgb: g.rgb.clone() }.png(&d.join(format!("{nombre}.png")))?;
            }
            referencias.push((z, (g.alto / 16) as i32, (g.ancho / 16) as i32));
        }
        drop(cod);
        let _ = mlx_rs::memory::clear_cache();
    }
    // [guía 1, guía 2, …] juntas en la secuencia (`torch.cat(image_latents, dim=1)`)
    let referencia = if referencias.is_empty() {
        None
    } else {
        let zs: Vec<Array> = referencias.iter().map(|(z, _, _)| z.clone()).collect();
        Some(ops::concatenate(&zs, 1).m()?)
    };

    let tr = Transformador::cargar(&pedido.transformador.clone().unwrap_or_else(|| dir.join("transformer")))?;
    let ctx = tr.contexto(&embeds)?;
    let ctx_neg = match &embeds_neg {
        Some(e) => Some(tr.contexto(e)?),
        None => None,
    };
    // posiciones: texto (0, 0, 0, l), imagen (0, fila, columna, 0); el texto va PRIMERO
    let mut ids: Vec<[i32; 4]> = (0..LARGO_TEXTO as i32).map(|l| [0, 0, 0, l]).collect();
    for f in 0..h {
        for c in 0..w {
            ids.push([0, f, c, 0]);
        }
    }
    // las guías van DESPUÉS de la imagen, la i-ésima con t = 10 + 10·i (`_prepare_image_ids`, scale 10)
    for (i, (_, hr, wr)) in referencias.iter().enumerate() {
        let t = 10 + 10 * i as i32;
        for f in 0..*hr {
            for c in 0..*wr {
                ids.push([t, f, c, 0]);
            }
        }
    }
    let giro = tr.giro(&ids)?;
    // ruido [1, 128, h, w] (como prepare_latents) → empaquetado [1, h·w, 128]
    let ruido = mlx_rs::random::normal::<f32>(&[1, 128, h, w], None, None, &mlx_rs::random::key(pedido.semilla).m()?).m()?.as_dtype(BF).m()?;
    guardar("ruido", &ruido)?;
    let mut lat = ops::transpose_axes(&ops::reshape(&ruido, &[1, 128, h * w]).m()?, &[0, 2, 1]).m()?;
    let sig = sigmas(pedido.pasos, n);
    for i in 0..pedido.pasos as usize {
        progreso(Etapa::Paso(i as u32 + 1, pedido.pasos));
        // con guía: [latentes, guía] y de la salida sólo los latentes (`noise_pred[:, :N]`)
        let pasada = |c: &Array| -> Result<Array> {
            match &referencia {
                Some(z) => tr.adelante(&ops::concatenate(&[lat.clone(), z.clone()], 1).m()?, c, t_del_paso(sig[i]), &giro, Some(cancelar))?.try_index((.., ..n as i32, ..)).m(),
                None => tr.adelante(&lat, c, t_del_paso(sig[i]), &giro, Some(cancelar)),
            }
        };
        let mut v = pasada(&ctx)?;
        // Ola PRO · klein base: la pasada negativa (con las MISMAS referencias) y
        // `neg + guía·(cond − neg)` en bf16, como el pipeline
        if let Some(cn) = &ctx_neg {
            let vn = pasada(cn)?;
            let g = Array::from_f32(pedido.guia_cfg).as_dtype(BF).m()?;
            v = ops::add(&vn, &ops::multiply(&g, &ops::subtract(&v, &vn).m()?).m()?).m()?;
        }
        // Euler: x + (σ' − σ)·v como torch (medido en MPS, 100 % de bits iguales): v (bf16) × dt (escalar
        // f32, SIN redondear) en f32 → bf16; luego la suma en f32 → bf16
        let dt = Array::from_f32(sig[i + 1] - sig[i]);
        let paso = ops::multiply(&v.as_dtype(Dtype::Float32).m()?, &dt).m()?.as_dtype(BF).m()?;
        lat = ops::add(&lat.as_dtype(Dtype::Float32).m()?, &paso.as_dtype(Dtype::Float32).m()?).m()?.as_dtype(BF).m()?;
        lat.eval().m()?;
        guardar(&format!("v_{i}"), &v)?;
        guardar(&format!("lat_{i}"), &lat)?;
    }
    drop(tr);
    let _ = mlx_rs::memory::clear_cache();
    if cancelar.load(Ordering::Relaxed) {
        bail!("cancelado");
    }

    progreso(Etapa::Vae);
    let vae = Vae::cargar(&dir.join("vae"))?;
    let z = vae.desempacar(&ops::reshape(&lat, &[1, h, w, 128]).m()?, h, w)?;
    guardar("z", &ops::transpose_axes(&z, &[0, 3, 1, 2]).m()?)?;
    let img = vae.decodificar(&z, Some(cancelar))?;
    guardar("imagen", &ops::transpose_axes(&img, &[0, 3, 1, 2]).m()?)?;
    let rgb = a_pixeles(&img)?;
    Ok(Imagen { ancho, alto, rgb })
}

/// El motor que registran la CLI y la app en `gotv_core::generadores` (firma sin tipos de MLX).
#[allow(clippy::too_many_arguments)]
pub fn generar_png(dir: &Path, prompt: &str, ancho: u32, alto: u32, pasos: u32, semilla: u64, guias: &[std::path::PathBuf], opciones: &serde_json::Value, salida: &Path, progreso: &(dyn Fn(String) + Sync), cancelar: &AtomicBool) -> Result<(u32, u32)> {
    // un SVG se rasteriza al tamaño pedido (o 1024² si se pide «el de la guía»)
    let (gw, gh) = if ancho == 0 || alto == 0 { (1024, 1024) } else { (ancho.min(1024), alto.min(1024)) };
    let guias = guias.iter().map(|r| Guia::de_archivo(r, gw, gh)).collect::<Result<Vec<_>>>()?;
    // ancho y alto 0 = el de la PRIMERA guía (como el pipeline sin alto/ancho: `height = height or
    // image_height` la primera vez); con tamaño pedido, la imagen sale de ese tamaño aunque haya guías
    let (ancho, alto) = match guias.first() {
        Some(g) if ancho == 0 || alto == 0 => (g.ancho, g.alto),
        _ => (ancho, alto),
    };
    // Ola PRO · klein base: su transformador, la guía del CFG y el negativo
    let p = Pedido {
        prompt: prompt.to_string(),
        ancho,
        alto,
        pasos,
        semilla,
        guias,
        transformador: opciones.get("transformador").and_then(|v| v.as_str()).map(std::path::PathBuf::from),
        guia_cfg: opciones.get("guia").and_then(|v| v.as_f64()).map_or(1.0, |g| g.clamp(1.0, 15.0) as f32),
        negativo: opciones.get("negativo").and_then(|v| v.as_str()).unwrap_or("").to_string(),
    };
    let img = generar(
        dir,
        &p,
        &|e| {
            progreso(match e {
                Etapa::Texto => "texto".into(),
                Etapa::Paso(i, n) => tr!("paso {i}/{n}", "step {i}/{n}"),
                Etapa::Vae => "vae".into(),
            })
        },
        cancelar,
        None,
    )?;
    img.png(salida)?;
    Ok((img.ancho, img.alto))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bf16_redondea_al_par() {
        assert_eq!(bf16(1.0), 1.0);
        // 1 + 2^-8 está justo en medio de 1 y 1 + 2^-7: va al par (1.0)
        assert_eq!(bf16(1.0 + 1.0 / 256.0), 1.0);
        assert_eq!(bf16(1.0 + 3.0 / 256.0), 1.0 + 4.0 / 256.0);
        assert_eq!(bf16(1000.0), 1000.0);
        assert_eq!(bf16(999.7), 1000.0);
    }

    #[test]
    fn mu_y_sigmas_como_klein() {
        // 1024×1024 → 64×64 = 4096 tokens, 4 pasos (valores de diffusers, medidos con su código)
        let s = sigmas(4, 4096);
        assert_eq!(s.len(), 5);
        assert_eq!(s[0], 1.0);
        assert_eq!(s[4], 0.0);
        assert!(s.windows(2).all(|p| p[0] > p[1]));
        let mu = mu_empirico(4096, 4);
        let (a1, b1, a2, b2) = (8.73809524e-05f64, 1.89833333, 0.00016927, 0.45666666);
        let (m200, m10) = (a2 * 4096.0 + b2, a1 * 4096.0 + b1);
        let a = (m200 - m10) / 190.0;
        assert!((mu - (a * 4.0 + m200 - 200.0 * a)).abs() < 1e-12);
        // por encima de 4300 tokens, mu es lineal en el largo
        assert!((mu_empirico(5000, 4) - (a2 * 5000.0 + b2)).abs() < 1e-12);
        // 512×512 (1024 tokens), 4 pasos: las sigmas que imprimió diffusers 0.43 (juez, 2026-11-12)
        for (a, b) in sigmas(4, 1024).iter().zip([1.0f32, 0.9580854, 0.8839819, 0.7174966, 0.0]) {
            assert!((a - b).abs() < 1e-7, "{a} ≠ {b}");
        }
    }

    #[test]
    fn t_sigue_la_cadena_bf16() {
        assert_eq!(t_del_paso(1.0), 1000.0);
        let t = t_del_paso(0.7);
        assert_eq!(t, bf16(t));
    }

    /// Diagnóstico para el juez: las capas 1..=12 del codificador de texto, a un safetensors.
    /// `GOTV_IMAGEN_DIR=… GOTV_VOLCADO_CAPAS=archivo cargo test -p gotv-mlx --release volcar_capas -- --ignored`
    #[test]
    #[ignore]
    fn volcar_capas_del_texto() {
        let dir = std::path::PathBuf::from(std::env::var("GOTV_IMAGEN_DIR").unwrap());
        let salida = std::env::var("GOTV_VOLCADO_CAPAS").unwrap();
        let prompt = std::env::var("GOTV_PROMPT").unwrap_or("A red fox sitting in a snowy forest at sunrise, photorealistic".into());
        let (ids, reales) = tokens(&dir, &prompt).unwrap();
        let te = Modelo::cargar(&dir.join("text_encoder")).unwrap();
        let capas: Vec<usize> = (1..=12).collect();
        let o = te.ocultos(&ids, reales, &capas, true).unwrap();
        let nombres: Vec<String> = capas.iter().map(|c| format!("h{c}")).collect();
        Array::save_safetensors(nombres.iter().zip(o.iter()).map(|(n, a)| (n.as_str(), a)), None, &salida).unwrap();
    }

    /// El ⏹ corta una generación REAL a mitad de los pasos en menos de 2 s (se mira entre bloques).
    /// `GOTV_IMAGEN_DIR=… cargo test -p gotv-mlx --release cancelar_corta -- --ignored`
    #[test]
    #[ignore]
    fn cancelar_corta_la_generacion() {
        let dir = std::path::PathBuf::from(std::env::var("GOTV_IMAGEN_DIR").unwrap());
        let cancelar = std::sync::Arc::new(AtomicBool::new(false));
        let pasos = std::sync::Arc::new(std::sync::Mutex::new(Vec::<Etapa>::new()));
        let (c2, p2) = (cancelar.clone(), pasos.clone());
        let hilo = std::thread::spawn(move || {
            let p = Pedido { prompt: "a red fox".into(), ancho: 1024, alto: 1024, pasos: 8, semilla: 1, ..Default::default() };
            generar(&dir, &p, &|e| p2.lock().unwrap().push(e), &c2, None)
        });
        // se espera a que vaya por el paso 2 (texto y transformador ya cargados)
        let t0 = std::time::Instant::now();
        while !pasos.lock().unwrap().contains(&Etapa::Paso(2, 8)) {
            assert!(t0.elapsed().as_secs() < 120, "no llegó al paso 2");
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        let corte = std::time::Instant::now();
        cancelar.store(true, Ordering::Relaxed);
        let r = hilo.join().unwrap();
        let tardo = corte.elapsed();
        assert!(r.as_ref().err().is_some_and(|e| e.to_string().contains("cancelado")), "no se canceló");
        assert!(tardo.as_secs_f32() < 2.0, "tardó {tardo:?} en cortar");
        assert!(!pasos.lock().unwrap().contains(&Etapa::Vae), "llegó al VAE");
        eprintln!("cortó en {:.2} s", tardo.as_secs_f32());
    }
}
