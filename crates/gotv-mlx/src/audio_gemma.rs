//! M1b · el OÍDO de Gemma 4 en el núcleo, como `mlx_vlm/models/gemma4/audio.py` y su extractor
//! (`audio_feature_extractor.py`, el de transformers):
//!
//! 1. **Mel** (CPU, con los tipos de numpy): PCM 16 kHz → relleno a múltiplo de 128 → 160 ceros a la
//!    izquierda → cuadros de 321 con salto 160 (se usan 320) × Hann periódica en f32 → |rFFT 512| →
//!    banco mel HTK de transformers (128 filtros, 0–8 kHz, en f64) → log(x + 0,001) → f32.
//! 2. **Conformer** (MLX): dos conv 3×3 con paso 2 (LayerNorm sin sesgo + ReLU) → lineal a 1024 →
//!    12 bloques [FF/2 → atención LOCAL por trozos de 12 con 12 de pasado, posición relativa
//!    sinusoidal y tope tanh(·/50)·50 → conv ligera causal (GLU + depthwise k=5) → FF/2 → RMSNorm],
//!    todos los lineales con RECORTE → lineal de salida con sesgo (1536).
//! 3. **Proyector** al modelo de lenguaje: RMSNorm sin peso + lineal cuantizado.
//!
//! En el prompt cada audio es `<|audio>` + N × `<|audio|>` + `<audio|>` con N = ⌈ms / 40⌉ (≤ 750).

use crate::modelo::{Config, Fusiones, Lineal, M};
use crate::vision_gemma::Recortado;
use anyhow::{anyhow, Result};
use gotv_engine::bail_tr;
use mlx_rs::fast;
use mlx_rs::ops::indexing::{take_axis, TryIndexOp};
use mlx_rs::ops;
use mlx_rs::{Array, Dtype};
use std::collections::HashMap;
use std::path::Path;

#[derive(Clone, Debug, serde::Deserialize)]
pub struct GAConfig {
    pub hidden_size: i32,
    pub num_attention_heads: i32,
    pub num_hidden_layers: i32,
    pub attention_chunk_size: i32,
    pub attention_context_left: i32,
    pub attention_context_right: i32,
    pub attention_invalid_logits_value: f32,
    pub attention_logit_cap: f32,
    pub conv_kernel_size: i32,
    pub residual_weight: f32,
    #[serde(default = "eps")]
    pub rms_norm_eps: f32,
    pub subsampling_conv_channels: Vec<i32>,
    #[serde(default)]
    pub output_proj_dims: Option<i32>,
}

fn eps() -> f32 {
    1e-6
}

pub const MUESTREO: usize = 16_000;
const VENTANA: usize = 320;
const SALTO: usize = 160;
const FFT: usize = 512;
const MELS: usize = 128;
/// 30 s: el extractor trunca ahí y el procesador topa los tokens en 750
const MAX_MUESTRAS: usize = 480_000;
const MAX_TOKENS: usize = 750;

/// Un audio hecho espectrograma: [cuadros × 128] y qué cuadros son relleno.
pub struct Sonido {
    pub mel: Vec<f32>,
    pub invalido: Vec<bool>,
    pub cuadros: usize,
    /// tokens que ocupa en el prompt (⌈ms / 40⌉, ≤ 750)
    pub tokens: usize,
}

/// `_compute_audio_num_tokens` del procesador: ⌈(muestras / 16000 · 1000) / 40⌉, tope 750.
pub fn tokens_de(muestras: usize) -> usize {
    let ms = muestras as f64 / MUESTREO as f64 * 1000.0;
    ((ms / 40.0).ceil() as usize).min(MAX_TOKENS)
}

/// El banco mel de transformers (`mel_filter_bank`, HTK, sin normalizar) en f64: [257][128].
fn banco_mel() -> Vec<f64> {
    let hz_a_mel = |f: f64| 2595.0 * (1.0 + f / 700.0).log10();
    let mel_a_hz = |m: f64| 700.0 * (10f64.powf(m / 2595.0) - 1.0);
    // np.linspace: i·paso + inicio, y el último exacto
    let linspace = |a: f64, b: f64, n: usize| -> Vec<f64> {
        let paso = (b - a) / (n - 1) as f64;
        (0..n).map(|i| if i == n - 1 { b } else { i as f64 * paso + a }).collect()
    };
    let mels = linspace(hz_a_mel(0.0), hz_a_mel((MUESTREO / 2) as f64), MELS + 2);
    let filtros: Vec<f64> = mels.iter().map(|&m| mel_a_hz(m)).collect();
    let bins = FFT / 2 + 1;
    let fft = linspace(0.0, (MUESTREO / 2) as f64, bins);
    let dif: Vec<f64> = filtros.windows(2).map(|w| w[1] - w[0]).collect();
    let mut b = vec![0f64; bins * MELS];
    for k in 0..bins {
        for i in 0..MELS {
            let baja = -(filtros[i] - fft[k]) / dif[i];
            let sube = (filtros[i + 2] - fft[k]) / dif[i + 1];
            b[k * MELS + i] = 0f64.max(baja.min(sube));
        }
    }
    b
}

/// FFT radix-2 en f64 (in situ).
fn fft(re: &mut [f64], im: &mut [f64]) {
    let n = re.len();
    let mut j = 0;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut largo = 2;
    while largo <= n {
        let ang = -2.0 * std::f64::consts::PI / largo as f64;
        for inicio in (0..n).step_by(largo) {
            for k in 0..largo / 2 {
                let (s, c) = (ang * k as f64).sin_cos();
                let (a, b) = (inicio + k, inicio + k + largo / 2);
                let (tr, ti) = (re[b] * c - im[b] * s, re[b] * s + im[b] * c);
                re[b] = re[a] - tr;
                im[b] = im[a] - ti;
                re[a] += tr;
                im[a] += ti;
            }
        }
        largo <<= 1;
    }
}

/// El espectrograma log-mel de Gemma 4 para un PCM mono de 16 kHz en [−1, 1].
pub fn sonido(pcm: &[f32]) -> Result<Sonido> {
    let tokens = tokens_de(pcm.len());
    let pcm = &pcm[..pcm.len().min(MAX_MUESTRAS)];
    let objetivo = pcm.len().div_ceil(128) * 128;
    let largo = SALTO + objetivo;
    if pcm.is_empty() || largo < VENTANA + 1 {
        bail_tr!("el audio es demasiado corto", "the audio is too short");
    }
    let cuadros = (largo - (VENTANA + 1)) / SALTO + 1;
    let muestra = |i: usize| -> f32 { if i < SALTO { 0.0 } else { pcm.get(i - SALTO).copied().unwrap_or(0.0) } };
    // Hann periódica como numpy en f32: 0,5 − 0,5·cos(f32(2π)·i / 320)
    let ventana: Vec<f32> = (0..VENTANA)
        .map(|i| {
            let x = std::f32::consts::TAU * i as f32 / VENTANA as f32;
            0.5f32 - 0.5f32 * ((x as f64).cos() as f32)
        })
        .collect();
    let banco = banco_mel();
    let bins = FFT / 2 + 1;
    let mut mel = vec![0f32; cuadros * MELS];
    let mut invalido = vec![false; cuadros];
    let (mut re, mut im) = (vec![0f64; FFT], vec![0f64; FFT]);
    let mut mag = vec![0f64; bins];
    for c in 0..cuadros {
        re.iter_mut().for_each(|v| *v = 0.0);
        im.iter_mut().for_each(|v| *v = 0.0);
        for j in 0..VENTANA {
            re[j] = (muestra(c * SALTO + j) * ventana[j]) as f64;
        }
        fft(&mut re, &mut im);
        for k in 0..bins {
            // |rfft| de complex64 → f32 y de vuelta a f64 para el matmul con el banco
            mag[k] = (re[k].hypot(im[k]) as f32) as f64;
        }
        // la máscara del cuadro es la de la muestra donde termina (i·160 + 320 con el relleno)
        let valido = c * SALTO + VENTANA - SALTO < pcm.len();
        invalido[c] = !valido;
        for i in 0..MELS {
            let s: f64 = (0..bins).map(|k| mag[k] * banco[k * MELS + i]).sum();
            mel[c * MELS + i] = if valido { (s + 1e-3).ln() as f32 } else { 0.0 };
        }
    }
    Ok(Sonido { mel, invalido, cuadros, tokens })
}

/// PCM mono en f32 desde un WAV (RIFF): PCM de 16 bits o flotante de 32, 16 kHz.
pub fn pcm_de_wav(b: &[u8]) -> Result<Vec<f32>> {
    if b.len() < 12 || &b[0..4] != b"RIFF" || &b[8..12] != b"WAVE" {
        bail_tr!("el audio no es un WAV", "the audio is not a WAV");
    }
    let (mut fmt, mut datos) = (None, None);
    let mut i = 12;
    while i + 8 <= b.len() {
        let n = u32::from_le_bytes([b[i + 4], b[i + 5], b[i + 6], b[i + 7]]) as usize;
        let cuerpo = &b[i + 8..(i + 8 + n).min(b.len())];
        match &b[i..i + 4] {
            b"fmt " => fmt = Some(cuerpo),
            b"data" => datos = Some(cuerpo),
            _ => {}
        }
        i += 8 + n + (n & 1);
    }
    let (Some(f), Some(d)) = (fmt, datos) else { bail_tr!("WAV sin «fmt» o sin «data»", "WAV without «fmt» or «data»") };
    if f.len() < 16 {
        bail_tr!("WAV con «fmt» corto", "WAV with a short «fmt»");
    }
    let u16_ = |o: usize| u16::from_le_bytes([f[o], f[o + 1]]);
    let mut formato = u16_(0);
    let canales = u16_(2).max(1) as usize;
    let tasa = u32::from_le_bytes([f[4], f[5], f[6], f[7]]) as usize;
    let bits = u16_(14);
    if formato == 0xFFFE && f.len() >= 26 {
        formato = u16_(24); // WAVE_FORMAT_EXTENSIBLE: el subformato
    }
    if tasa != MUESTREO {
        bail_tr!("el audio va a {tasa} Hz y el oído de Gemma pide {MUESTREO} Hz", "the audio runs at {tasa} Hz and Gemma's hearing needs {MUESTREO} Hz");
    }
    let muestras: Vec<f32> = match (formato, bits) {
        (1, 16) => d.as_chunks::<2>().0.iter().map(|c| i16::from_le_bytes(*c) as f32 / 32768.0).collect(),
        (3, 32) => d.as_chunks::<4>().0.iter().map(|c| f32::from_le_bytes(*c)).collect(),
        _ => bail_tr!("WAV con formato {formato} de {bits} bits: el núcleo lee PCM de 16 bits o flotante de 32", "WAV with format {formato} at {bits} bits: the core reads 16-bit PCM or 32-bit float"),
    };
    if canales == 1 {
        return Ok(muestras);
    }
    Ok(muestras.chunks_exact(canales).map(|c| c.iter().sum::<f32>() / canales as f32).collect())
}

/// Un escalar del tipo de `como` (los escalares de Python se adaptan al tipo del arreglo).
fn esc(v: f32, como: &Array) -> Result<Array> {
    Array::from_f32(v).as_dtype(como.dtype()).m()
}

fn tomar(p: &mut HashMap<String, Array>, n: &str) -> Result<Array> {
    p.remove(n).ok_or_else(|| anyhow!("falta audio_tower.{n}"))
}

struct Avance {
    pre: Array,
    l1: Recortado,
    l2: Recortado,
    post: Array,
}

struct BloqueA {
    ff1: Avance,
    ff2: Avance,
    q: Recortado,
    k: Recortado,
    v: Recortado,
    post: Recortado,
    rel_k_t: Array,
    escala_dim: Array,
    norma_pre_at: Array,
    norma_post_at: Array,
    norma_out: Array,
    lc_pre: Array,
    lc_ini: Recortado,
    lc_conv: Array,
    lc_norma: Array,
    lc_fin: Recortado,
}

pub struct OidoGemma {
    pub c: GAConfig,
    convs: Vec<(Array, Array)>,
    entrada_t: Array,
    bloques: Vec<BloqueA>,
    salida: Option<(Array, Array)>,
    proyeccion: Lineal,
    inv_tiempos: Array,
    f: Fusiones,
}

impl OidoGemma {
    /// ¿Gemma 4 con un oído que el núcleo sabe correr? (sólo el config)
    pub fn config(dir: &Path) -> Option<GAConfig> {
        let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dir.join("config.json")).ok()?).ok()?;
        if !matches!(v["model_type"].as_str(), Some("gemma4")) || v["audio_config"]["model_type"].as_str() != Some("gemma4_audio") {
            return None;
        }
        let c: GAConfig = serde_json::from_value(v["audio_config"].clone()).ok()?;
        (c.subsampling_conv_channels.len() == 2).then_some(c)
    }

    pub fn cargar(dir: &Path, c: GAConfig, cfg_texto: &Config) -> Result<OidoGemma> {
        let mut p = crate::modelo::cargar_pesos_de(dir, "audio_tower.")?;
        if p.is_empty() {
            bail_tr!("el modelo no trae el oído (audio_tower.*)", "the model has no hearing (audio_tower.*)");
        }
        let mut convs = Vec::new();
        for (i, entra) in [1, c.subsampling_conv_channels[0]].into_iter().enumerate() {
            let n = format!("subsample_conv_projection.layer{i}");
            let mut w = tomar(&mut p, &format!("{n}.conv.weight"))?;
            if w.dim(-1) != entra {
                w = ops::transpose_axes(&w, &[0, 2, 3, 1]).m()?; // de PyTorch [sal, ent, kH, kW]
            }
            convs.push((w, tomar(&mut p, &format!("{n}.norm.weight"))?));
        }
        let entrada = tomar(&mut p, "subsample_conv_projection.input_proj_linear.weight")?;
        let avance = |p: &mut HashMap<String, Array>, n: &str| -> Result<Avance> {
            Ok(Avance {
                pre: tomar(p, &format!("{n}.pre_layer_norm.weight"))?,
                l1: Recortado::cargar(p, &format!("{n}.ffw_layer_1"))?,
                l2: Recortado::cargar(p, &format!("{n}.ffw_layer_2"))?,
                post: tomar(p, &format!("{n}.post_layer_norm.weight"))?,
            })
        };
        let mut bloques = Vec::new();
        for i in 0..c.num_hidden_layers {
            let n = format!("layers.{i}");
            let mut conv = tomar(&mut p, &format!("{n}.lconv1d.depthwise_conv1d.weight"))?;
            if conv.dim(-1) != 1 {
                conv = ops::transpose_axes(&conv, &[0, 2, 1]).m()?;
            }
            bloques.push(BloqueA {
                ff1: avance(&mut p, &format!("{n}.feed_forward1"))?,
                ff2: avance(&mut p, &format!("{n}.feed_forward2"))?,
                q: Recortado::cargar(&mut p, &format!("{n}.self_attn.q_proj"))?,
                k: Recortado::cargar(&mut p, &format!("{n}.self_attn.k_proj"))?,
                v: Recortado::cargar(&mut p, &format!("{n}.self_attn.v_proj"))?,
                post: Recortado::cargar(&mut p, &format!("{n}.self_attn.post"))?,
                rel_k_t: ops::transpose_axes(&tomar(&mut p, &format!("{n}.self_attn.relative_k_proj.weight"))?, &[1, 0]).m()?,
                escala_dim: tomar(&mut p, &format!("{n}.self_attn.per_dim_scale"))?,
                norma_pre_at: tomar(&mut p, &format!("{n}.norm_pre_attn.weight"))?,
                norma_post_at: tomar(&mut p, &format!("{n}.norm_post_attn.weight"))?,
                norma_out: tomar(&mut p, &format!("{n}.norm_out.weight"))?,
                lc_pre: tomar(&mut p, &format!("{n}.lconv1d.pre_layer_norm.weight"))?,
                lc_ini: Recortado::cargar(&mut p, &format!("{n}.lconv1d.linear_start"))?,
                lc_conv: conv,
                lc_norma: tomar(&mut p, &format!("{n}.lconv1d.conv_norm.weight"))?,
                lc_fin: Recortado::cargar(&mut p, &format!("{n}.lconv1d.linear_end"))?,
            });
        }
        let salida = match (p.remove("output_proj.weight"), p.remove("output_proj.bias")) {
            (Some(w), Some(b)) => Some((ops::transpose_axes(&w, &[1, 0]).m()?, b)),
            (None, None) => None,
            _ => bail_tr!("output_proj del oído a medias", "hearing output_proj half present"),
        };
        // el proyector vive fuera de la torre (`embed_audio.*`), cuantizado como el texto
        let mut pp = crate::modelo::cargar_pesos_de(dir, "embed_audio.")?;
        let mut sin_prefijo = cfg_texto.clone();
        sin_prefijo.prefijo = String::new();
        let proyeccion = Lineal::cargar(&mut pp, "embedding_projection", &sin_prefijo)?;
        // 1 · exp(arange(D/2) · −ln(10⁴)/(D/2 − 1)), en f32 como mlx
        let mitad = c.hidden_size / 2;
        let inc = (10000f64).ln() / ((mitad - 1).max(1)) as f64;
        let inv = ops::exp(ops::multiply(ops::arange::<_, f32>(0.0, mitad as f32, 1.0).m()?, Array::from_f32(-inc as f32)).m()?).m()?;
        let inv_tiempos = ops::reshape(&inv, &[1, 1, mitad]).m()?;
        Ok(OidoGemma { entrada_t: ops::transpose_axes(&entrada, &[1, 0]).m()?, convs, bloques, salida, proyeccion, inv_tiempos, c, f: Fusiones::nuevas() })
    }

    fn rms(&self, x: &Array, w: &Array) -> Result<Array> {
        fast::rms_norm(x, Some(w), 1e-6).m()
    }

    /// `ConformerFeedForward`: x + 0,5 · RMS(l2(silu(l1(RMS(x))))).
    fn avance(&self, a: &Avance, x: &Array) -> Result<Array> {
        let y = a.l1.aplicar(&self.rms(x, &a.pre)?)?;
        let y = (self.f.silu.borrow_mut())(&y).m()?;
        let y = self.rms(&a.l2.aplicar(&y)?, &a.post)?;
        ops::add(x, ops::multiply(&y, esc(self.c.residual_weight, &y)?).m()?).m()
    }

    /// `AudioAttention`: local por trozos con posición relativa y tope.
    fn atencion(&self, b: &BloqueA, x: &Array, tt: i32, u: i32, idx_ctx: &Array, cond: &Array) -> Result<Array> {
        let c = &self.c;
        let (n, hd) = (c.num_attention_heads, c.hidden_size / c.num_attention_heads);
        let (w, pasado, futuro) = (c.attention_chunk_size, (c.attention_context_left - 1).max(0), c.attention_context_right);
        let ctx = w + pasado + futuro;
        let forma = [1, tt, n, hd];
        let f32_ = |a: Array| -> Result<Array> { ops::reshape(a.as_dtype(Dtype::Float32).m()?, &forma).m() };
        let q = f32_(b.q.aplicar(x)?)?;
        let k = f32_(b.k.aplicar(x)?)?;
        let v = f32_(b.v.aplicar(x)?)?;
        // softplus(per_dim_scale) = logaddexp(x, 0), y la escala de q en su tipo (bf16)
        let pds = ops::logaddexp(&b.escala_dim, esc(0.0, &b.escala_dim)?).m()?;
        let q_escala = (hd as f64).powf(-0.5) / std::f64::consts::LN_2;
        let k_escala = (1.0 + std::f64::consts::E).ln() / std::f64::consts::LN_2;
        let q = ops::multiply(&q, ops::multiply(esc(q_escala as f32, &pds)?, &pds).m()?).m()?;
        let k = ops::multiply(&k, esc(k_escala as f32, &k)?).m()?;
        // q en bloques [1, U, W, N, H]; k y v con su contexto [1, U, C, N, H]
        let qb = if u * w > tt { ops::pad(&q, &[(0, 0), (0, u * w - tt), (0, 0), (0, 0)], None, None).m()? } else { q };
        let qb = ops::reshape(&qb, &[1, u, w, n, hd]).m()?;
        let contexto = |a: &Array| -> Result<Array> {
            let a = ops::pad(a, &[(0, 0), (pasado, futuro + w - 1), (0, 0), (0, 0)], None, None).m()?;
            take_axis(&a, idx_ctx, 1).m()
        };
        let (kb, vb) = (contexto(&k)?, contexto(&v)?);
        // posición relativa: señal sinusoidal de [pasado … −futuro] → relative_k_proj
        let pos: Vec<f32> = (0..=pasado + futuro).map(|i| (pasado - i) as f32).collect();
        let span = pos.len() as i32;
        let t = ops::multiply(Array::from_slice(&pos, &[1, span, 1]), &self.inv_tiempos).m()?;
        let senal = ops::concatenate(&[ops::sin(&t).m()?, ops::cos(&t).m()?], -1).m()?.as_dtype(b.rel_k_t.dtype()).m()?;
        let se = ops::reshape(ops::matmul(&senal, &b.rel_k_t).m()?, &[span, n, hd]).m()?.as_dtype(Dtype::Float32).m()?;
        let qp = ops::transpose_axes(&qb, &[0, 3, 1, 2, 4]).m()?;
        let kp = ops::transpose_axes(&kb, &[0, 3, 1, 4, 2]).m()?;
        let ac = ops::matmul(&qp, &kp).m()?;
        let qr = ops::reshape(&qp, &[1, n, u * w, hd]).m()?;
        let bd = ops::reshape(ops::matmul(&qr, ops::transpose_axes(&se, &[1, 2, 0]).m()?).m()?, &[1, n, u, w, span]).m()?;
        // `_relative_shift`
        let bd = ops::pad(&bd, &[(0, 0), (0, 0), (0, 0), (0, 0), (0, ctx + 1 - span)], None, None).m()?;
        let bd = ops::reshape(&bd, &[1, n, u, w * (ctx + 1)]).m()?;
        let bd = bd.try_index((.., .., .., ..w * ctx)).m()?;
        let bd = ops::reshape(&bd, &[1, n, u, w, ctx]).m()?;
        let l = ops::add(&ac, &bd).m()?;
        let cap = c.attention_logit_cap;
        let l = ops::multiply(ops::tanh(ops::divide(&l, esc(cap, &l)?).m()?).m()?, esc(cap, &l)?).m()?;
        let l = ops::select(cond, &l, esc(c.attention_invalid_logits_value, &l)?).m()?;
        let p = ops::softmax_axis(&l, -1, None).m()?;
        let o = ops::einsum("bnuwc,bucnh->buwnh", [&p, &vb]).m()?;
        let o = ops::reshape(&o, &[1, u * w, n, hd]).m()?;
        let o = o.try_index((.., ..tt)).m()?;
        b.post.aplicar(&ops::reshape(&o, &[1, tt, n * hd]).m()?)
    }

    /// `ConformerLightConv1d`.
    fn conv_ligera(&self, b: &BloqueA, x: &Array) -> Result<Array> {
        let y = b.lc_ini.aplicar(&self.rms(x, &b.lc_pre)?)?;
        let m = ops::split_equal(&y, 2, -1).m()?;
        let y = ops::multiply(&m[0], ops::sigmoid(&m[1]).m()?).m()?;
        let y = ops::pad(&y, &[(0, 0), (self.c.conv_kernel_size - 1, 0), (0, 0)], None, None).m()?;
        let grupos = y.dim(-1);
        let y = ops::conv_general(&y, &b.lc_conv, &[1][..], &[0][..], &[1][..], &[1][..], grupos, false).m()?;
        let y = self.rms(&y, &b.lc_norma)?;
        let y = (self.f.silu.borrow_mut())(&y).m()?;
        ops::add(b.lc_fin.aplicar(&y)?, x).m()
    }

    /// Los tokens de audio PROYECTADOS al modelo de lenguaje: [tokens, oculto del texto].
    pub fn rasgos(&self, s: &Sonido) -> Result<Array> {
        let (_, proy) = self.correr(s)?;
        let filas = proy.dim(0);
        // `masked_scatter` toma las primeras filas (y daría la vuelta si faltaran)
        let idx: Vec<i32> = (0..s.tokens as i32).map(|i| i % filas).collect();
        take_axis(&proy, Array::from_slice(&idx, &[idx.len() as i32]), 0).m()
    }

    /// (salida de la torre [T, 1536], proyectados [T, oculto]) — la primera sirve al juez.
    pub fn correr(&self, s: &Sonido) -> Result<(Array, Array)> {
        let c = &self.c;
        let eps = c.rms_norm_eps;
        // SSCP: [1, T, F, 1] → 2 × (anular relleno → bordes de 1 → conv 3×3 paso 2 → LayerNorm → ReLU)
        let mut x = Array::from_slice(&s.mel, &[1, s.cuadros as i32, MELS as i32, 1]);
        let mut invalido = s.invalido.clone();
        for (w, ln) in &self.convs {
            let m = Array::from_slice(&invalido, &[1, invalido.len() as i32, 1, 1]);
            x = ops::select(&m, esc(0.0, &x)?, &x).m()?;
            x = ops::pad(&x, &[(0, 0), (1, 1), (1, 1), (0, 0)], None, None).m()?;
            x = ops::conv2d(&x, w, (2, 2), (0, 0), (1, 1), 1).m()?;
            let to = x.dim(1) as usize;
            invalido = invalido.iter().step_by(2).take(to).copied().collect();
            x = fast::layer_norm(&x, Some(ln), None, eps).m()?;
            x = ops::maximum(&x, esc(0.0, &x)?).m()?;
        }
        let (tt, fr, ch) = (x.dim(1), x.dim(2), x.dim(3));
        let mut h = ops::matmul(ops::reshape(&x, &[1, tt, fr * ch]).m()?, &self.entrada_t).m()?;
        // la atención local: índices del contexto de cada trozo y la condición [1, 1, U, W, C]
        let (w, pasado, futuro) = (c.attention_chunk_size, (c.attention_context_left - 1).max(0), c.attention_context_right);
        let ctx = w + pasado + futuro;
        let u = (tt + w - 1) / w;
        let mut idx = Vec::with_capacity((u * ctx) as usize);
        let mut cond = Vec::with_capacity((u * w * ctx) as usize);
        for ui in 0..u {
            for ci in 0..ctx {
                idx.push(ui * w + ci);
            }
            for wi in 0..w {
                for ci in 0..ctx {
                    let pos = ui * w + ci - pasado;
                    let valido = pos >= 0 && pos < tt && !invalido[pos as usize];
                    let dist = wi + pasado - ci;
                    let causal = (dist >= 0 && dist < pasado) || (dist < 0 && -dist < futuro);
                    cond.push(valido && causal);
                }
            }
        }
        let idx_ctx = Array::from_slice(&idx, &[u, ctx]);
        let cond = Array::from_slice(&cond, &[1, 1, u, w, ctx]);
        let validez: Vec<f32> = invalido.iter().map(|&i| if i { 0.0 } else { 1.0 }).collect();
        let validez = Array::from_slice(&validez, &[1, tt, 1]);
        for b in &self.bloques {
            h = self.avance(&b.ff1, &h)?;
            let y = self.atencion(b, &self.rms(&h, &b.norma_pre_at)?, tt, u, &idx_ctx, &cond)?;
            h = ops::add(&h, self.rms(&y, &b.norma_post_at)?).m()?;
            h = ops::multiply(&h, validez.as_dtype(h.dtype()).m()?).m()?;
            h = self.conv_ligera(b, &h)?;
            h = self.avance(&b.ff2, &h)?;
            h = self.rms(&h, &b.norma_out)?;
        }
        if let Some((w_t, sesgo)) = &self.salida {
            h = ops::addmm(sesgo, &h, w_t, None, None).m()?;
        }
        let m = Array::from_slice(&invalido, &[1, tt, 1]);
        let h = ops::select(&m, esc(0.0, &h)?, &h).m()?;
        let proy = self.proyeccion.aplicar(&fast::rms_norm(&h, None, eps).m()?)?;
        Ok((ops::reshape(&h, &[tt, -1]).m()?, ops::reshape(&proy, &[tt, -1]).m()?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn comparar(a: &[f32], b: &[f32]) -> (f32, f32) {
        let d = a.iter().zip(b).map(|(x, y)| (x - y).abs()).fold(0.0, f32::max);
        let n = |x: &[f32]| x.iter().map(|v| v * v).sum::<f32>().sqrt();
        (d, a.iter().zip(b).map(|(x, y)| x * y).sum::<f32>() / (n(a) * n(b)))
    }

    #[test]
    fn los_tokens_son_los_del_procesador() {
        assert_eq!(tokens_de(44_480), 70); // 2,78 s → 69,5 → 70
        assert_eq!(tokens_de(16_000), 25);
        assert_eq!(tokens_de(16_000 * 60), 750, "tope de 30 s");
    }

    #[test]
    fn el_banco_mel_es_el_de_transformers() {
        let b = banco_mel();
        // medido en transformers 5.22: sólo el filtro 0 queda en cero (más estrecho que un bin de
        // 31,25 Hz) y la suma total es 252,78513714066457
        let ceros: Vec<usize> = (0..MELS).filter(|&i| (0..FFT / 2 + 1).all(|k| b[k * MELS + i] == 0.0)).collect();
        assert_eq!(ceros, vec![0]);
        assert!(b.iter().all(|&v| (0.0..=1.0).contains(&v)));
        assert!((b.iter().sum::<f64>() - 252.785_137_140_664_57).abs() < 1e-9, "{}", b.iter().sum::<f64>());
    }

    #[test]
    fn lee_wav_pcm16() {
        let mut w = Vec::new();
        let datos: Vec<u8> = [0i16, 16384, -32768].iter().flat_map(|v| v.to_le_bytes()).collect();
        w.extend(b"RIFF");
        w.extend(((36 + datos.len()) as u32).to_le_bytes());
        w.extend(b"WAVEfmt ");
        w.extend(16u32.to_le_bytes());
        w.extend(1u16.to_le_bytes());
        w.extend(1u16.to_le_bytes());
        w.extend(16000u32.to_le_bytes());
        w.extend(32000u32.to_le_bytes());
        w.extend(2u16.to_le_bytes());
        w.extend(16u16.to_le_bytes());
        w.extend(b"FLLR");
        w.extend(3u32.to_le_bytes());
        w.extend([0, 0, 0, 0]); // trozo impar + su relleno
        w.extend(b"data");
        w.extend((datos.len() as u32).to_le_bytes());
        w.extend(&datos);
        assert_eq!(pcm_de_wav(&w).unwrap(), vec![0.0, 0.5, -1.0]);
        assert!(pcm_de_wav(b"no es un wav").is_err());
    }

    /// Juez contra mlx_vlm (`ref_audio_gemma.py`): mel, torre, proyección y la respuesta.
    /// `GOTV_MLX_DIR=<gemma> GOTV_REF_AUDIO=<dir> GOTV_WAV=<wav> cargo test -p gotv-mlx --release
    /// -- --ignored --nocapture oye_como_mlx_vlm_gemma`
    #[test]
    #[ignore]
    fn oye_como_mlx_vlm_gemma() {
        use crate::modelo::{cargar_red, Extra};
        let (Ok(dir), Ok(rd), Ok(wav)) = (std::env::var("GOTV_MLX_DIR"), std::env::var("GOTV_REF_AUDIO"), std::env::var("GOTV_WAV")) else { return };
        let dir = Path::new(&dir);
        let rj: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(Path::new(&rd).join("ref.json")).unwrap()).unwrap();
        let rs = Array::load_safetensors(Path::new(&rd).join("ref.safetensors")).unwrap();
        // 1 · PCM y mel
        let pcm = pcm_de_wav(&std::fs::read(&wav).unwrap()).unwrap();
        let (d0, _) = comparar(&pcm, rs["pcm"].as_slice::<f32>());
        assert_eq!(pcm.len(), rs["pcm"].size(), "mismas muestras");
        assert_eq!(d0, 0.0, "el PCM difiere");
        let s = sonido(&pcm).unwrap();
        let rmel = &rs["mel"];
        assert_eq!(s.cuadros as i32, rmel.dim(1), "mismos cuadros");
        let (d1, c1) = comparar(&s.mel, rmel.as_slice::<f32>());
        let masc: Vec<bool> = rs["mascara"].as_slice::<f32>().iter().map(|&v| v == 0.0).collect();
        println!("1 · mel {}×128 → dif. máx {d1:e} · coseno {c1:.9} · máscara igual {}", s.cuadros, masc == s.invalido);
        assert_eq!(masc, s.invalido, "la máscara de cuadros difiere");
        assert!(d1 < 1e-3, "el mel difiere");
        // 2 · torre y proyección — con el mel de la REFERENCIA, para juzgar la torre sola
        let m = cargar_red(dir).unwrap();
        let c = OidoGemma::config(dir).expect("Gemma 4 con oído");
        let o = OidoGemma::cargar(dir, c, m.cfg()).unwrap();
        let s_ref = Sonido { mel: rmel.as_slice::<f32>().to_vec(), invalido: masc, cuadros: s.cuadros, tokens: s.tokens };
        let (torre, proy) = o.correr(&s_ref).unwrap();
        let (torre, proy) = (torre.as_dtype(Dtype::Float32).unwrap(), proy.as_dtype(Dtype::Float32).unwrap());
        let (d2, c2) = comparar(torre.as_slice::<f32>(), rs["torre"].as_slice::<f32>());
        let (d3, c3) = comparar(proy.as_slice::<f32>(), rs["rasgos"].as_slice::<f32>());
        println!("2 · torre: dif. máx {d2} · coseno {c2:.6}\n   proyección: dif. máx {d3} · coseno {c3:.6}");
        assert!(c3 > 0.999, "la torre difiere");
        // y con NUESTRO mel
        let (_, p2) = o.correr(&s).unwrap();
        let (d4, c4) = comparar(p2.as_dtype(Dtype::Float32).unwrap().as_slice::<f32>(), rs["rasgos"].as_slice::<f32>());
        println!("   con el mel propio: dif. máx {d4} · coseno {c4:.6}");
        // 3 · el modelo completo con el audio
        let ids: Vec<u32> = rj["ids"].as_array().unwrap().iter().map(|x| x.as_u64().unwrap() as u32).collect();
        assert_eq!(ids.iter().filter(|&&t| t == 258881).count(), s.tokens, "tokens de audio = ⌈ms/40⌉");
        let mut k = 0;
        let cual: Vec<i32> = ids.iter().map(|&t| if t == 258881 { k += 1; k - 1 } else { -1 }).collect();
        let mut cache = m.cache_nueva();
        let rasgos = o.rasgos(&s).unwrap();
        let l = m.adelante_ext(&ids, &Extra { rasgos: Some((&rasgos, cual)), pos: None }, &mut cache).unwrap().as_dtype(Dtype::Float32).unwrap();
        let orden = ops::argsort_axis(ops::negative(&l).unwrap(), -1).unwrap();
        let top: Vec<u32> = (0..5).map(|i| orden.try_index(i).unwrap().try_item_cast::<u32>().unwrap()).collect();
        println!("3 · top5 {top:?} · ref {}", rj["top5"]);
        let mut gen = vec![];
        let mut nxt = top[0];
        for _ in 0..12 {
            gen.push(nxt);
            nxt = mlx_rs::ops::indexing::argmax_axis(m.adelante(&[nxt], &mut cache).unwrap(), -1, false).unwrap().try_item_cast::<u32>().unwrap();
        }
        let refv: Vec<u32> = rj["voraz"].as_array().unwrap().iter().map(|x| x.as_u64().unwrap() as u32).collect();
        println!("   voraz iguales {}/12 · ref: {}", gen.iter().zip(&refv).take_while(|(a, b)| a == b).count(), rj["texto"]);
        let hasta = refv.iter().position(|&t| t == 106).map(|p| p + 1).unwrap_or(refv.len());
        assert_eq!(gen[..hasta], refv[..hasta], "la respuesta difiere de mlx_vlm");
    }
}
