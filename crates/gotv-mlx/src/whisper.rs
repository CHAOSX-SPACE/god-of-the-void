//! M1 · Whisper en el núcleo (Rust + MLX, sin Python): el respaldo de oído para los modelos que no
//! oyen. Port de `mlx_audio/stt/models/whisper` (el algoritmo de OpenAI):
//!
//! - **Mel** con las MISMAS operaciones de MLX: relleno de 30 s de silencio, reflejo de 200, cuadros
//!   de 400 con salto 160 × Hann periódica → rFFT → |·|² (sin el último cuadro) → banco mel Slaney
//!   (normalizado) → log10(max(·, 1e-10)) → max(·, máx − 8) → (· + 4) / 4.
//! - **Codificador:** 2 conv1d (k=3; la 2.ª con paso 2) + GELU → posición sinusoidal → N bloques
//!   (LayerNorm → atención con escala d^−¼ en q y k → LayerNorm → MLP GELU) → LayerNorm. En f16.
//! - **Decodificador:** embedding + posición aprendida → bloques con atención propia (caché K/V) y
//!   cruzada (K/V del audio, una vez por ventana) → LayerNorm → logits = x · embeddingᵀ.
//! - **Decodificación:** idioma por el token tras `<|startoftranscript|>`, voraz con supresiones
//!   (blancos al inicio, no-habla, especiales), REGLAS DE MARCAS DE TIEMPO, respaldo por temperatura
//!   (0 → 1 si el texto se repite —razón de compresión > 2,4— o la confianza media < −1), ventana
//!   deslizante de 30 s guiada por las marcas y el texto previo como contexto.
//!
//! Diferencia deliberada con mlx-audio: la regla «las marcas de tiempo no retroceden» usa el VALOR
//! de la última marca (como OpenAI); mlx-audio usa su índice y la regla no actúa.

use crate::modelo::{Fusiones, M};
use anyhow::{anyhow, bail, Result};
use gotv_engine::{anyhow_tr, bail_tr};
use mlx_rs::fast;
use mlx_rs::ops::indexing::{take_axis, TryIndexOp};
use mlx_rs::ops;
use mlx_rs::{Array, Dtype};
use std::collections::HashMap;
use std::path::Path;

pub const MUESTREO: usize = 16_000;
const N_FFT: usize = 400;
const SALTO: usize = 160;
/// 30 s de muestras y de cuadros mel por ventana
const N_MUESTRAS: usize = 30 * MUESTREO;
const N_CUADROS: usize = N_MUESTRAS / SALTO;
const TEMPERATURAS: [f32; 6] = [0.0, 0.2, 0.4, 0.6, 0.8, 1.0];
const TOPE_COMPRESION: f64 = 2.4;
const TOPE_LOGPROB: f64 = -1.0;
const TOPE_NO_HABLA: f64 = 0.6;

#[derive(Clone, Debug)]
pub struct Dims {
    pub n_mels: i32,
    pub n_audio_ctx: i32,
    pub n_estado: i32,
    pub n_cab_audio: i32,
    pub n_capas_audio: i32,
    pub n_vocab: i32,
    pub n_text_ctx: i32,
    pub n_cab_texto: i32,
    pub n_capas_texto: i32,
}

impl Dims {
    fn leer(v: &serde_json::Value) -> Option<Dims> {
        let n = |a: &str, b: &str| v[a].as_i64().or_else(|| v[b].as_i64()).map(|x| x as i32);
        Some(Dims {
            n_mels: n("num_mel_bins", "n_mels")?,
            n_audio_ctx: n("max_source_positions", "n_audio_ctx")?,
            n_estado: n("d_model", "n_audio_state")?,
            n_cab_audio: n("encoder_attention_heads", "n_audio_head")?,
            n_capas_audio: n("encoder_layers", "n_audio_layer")?,
            n_vocab: n("vocab_size", "n_vocab")?,
            n_text_ctx: n("max_target_positions", "n_text_ctx")?,
            n_cab_texto: n("decoder_attention_heads", "n_text_head")?,
            n_capas_texto: n("decoder_layers", "n_text_layer")?,
        })
    }
}

/// `nn.Linear`: con sesgo es `addmm` (una operación), sin sesgo `x · Wᵀ`.
struct Lin {
    w_t: Array,
    b: Option<Array>,
}

impl Lin {
    fn aplicar(&self, x: &Array) -> Result<Array> {
        match &self.b {
            Some(b) => ops::addmm(b, x, &self.w_t, None, None).m(),
            None => ops::matmul(x, &self.w_t).m(),
        }
    }
}

struct Atn {
    q: Lin,
    k: Lin,
    v: Lin,
    o: Lin,
}

struct Bloque {
    atn: Atn,
    atn_ln: (Array, Array),
    cruz: Option<(Atn, (Array, Array))>,
    mlp1: Lin,
    mlp2: Lin,
    mlp_ln: (Array, Array),
}

/// K/V de una capa de atención (`[1, largo, n_estado]` cada uno).
type Kv = Option<(Array, Array)>;

/// Caché del decodificador de una ventana: K/V propios (crecen) y cruzados (fijos).
struct Estado {
    kv: Vec<Kv>,
    cruz: Vec<Kv>,
}

/// Lo que devuelve decodificar una ventana.
struct Resultado {
    tokens: Vec<u32>,
    logprob_media: f64,
    no_habla: f64,
    temperatura: f32,
    compresion: f64,
}

/// Un tramo con sus tiempos (segundos) y su texto.
#[derive(Clone, Debug)]
pub struct Segmento {
    pub inicio: f64,
    pub fin: f64,
    pub texto: String,
    pub tokens: Vec<u32>,
    pub temperatura: f32,
}

#[derive(Clone, Debug)]
pub struct Transcripcion {
    pub texto: String,
    pub idioma: String,
    pub segmentos: Vec<Segmento>,
}

/// Los tokens especiales del tokenizador de Whisper.
struct Especiales {
    eot: u32,
    sot: u32,
    sot_prev: u32,
    sot_lm: u32,
    transcribir: u32,
    traducir: u32,
    no_habla: Option<u32>,
    sin_marcas: u32,
    marca0: u32,
    idiomas: Vec<u32>,
}

pub struct Whisper {
    pub dims: Dims,
    conv1: (Array, Array),
    conv2: (Array, Array),
    pos_audio: Array,
    enc: Vec<Bloque>,
    ln_post: (Array, Array),
    emb: Array,
    emb_t: Array,
    pos_texto: Array,
    dec: Vec<Bloque>,
    ln: (Array, Array),
    mascara: Array,
    filtros: Array,
    ventana: Array,
    tok: tokenizers::Tokenizer,
    esp: Especiales,
    suprimir: Vec<u32>,
    blanco: Vec<u32>,
    limpiar_espacios: bool,
    f: Fusiones,
}

/// ¿Es una carpeta de Whisper? (config `model_type = whisper` y su tokenizador)
pub fn es_whisper(dir: &Path) -> bool {
    std::fs::read_to_string(dir.join("config.json"))
        .ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .is_some_and(|v| v["model_type"].as_str() == Some("whisper"))
        && dir.join("tokenizer.json").exists()
}

/// Las claves de un Whisper en formato de Hugging Face, al formato de MLX (`sanitize` de mlx-audio).
fn clave_mlx(k: &str) -> Option<String> {
    let mut k = k.strip_prefix("model.").unwrap_or(k).to_string();
    const MAPA: [(&str, Option<&str>); 19] = [
        ("encoder.embed_positions.weight", None),
        ("decoder.embed_positions.weight", Some("decoder.positional_embedding")),
        ("encoder.layer_norm.", Some("encoder.ln_post.")),
        ("decoder.layer_norm.", Some("decoder.ln.")),
        ("encoder.layers.", Some("encoder.blocks.")),
        ("decoder.layers.", Some("decoder.blocks.")),
        (".self_attn_layer_norm.", Some(".attn_ln.")),
        (".final_layer_norm.", Some(".mlp_ln.")),
        (".encoder_attn_layer_norm.", Some(".cross_attn_ln.")),
        (".fc1.", Some(".mlp1.")),
        (".fc2.", Some(".mlp2.")),
        (".self_attn.q_proj.", Some(".attn.query.")),
        (".self_attn.k_proj.", Some(".attn.key.")),
        (".self_attn.v_proj.", Some(".attn.value.")),
        (".self_attn.out_proj.", Some(".attn.out.")),
        (".encoder_attn.q_proj.", Some(".cross_attn.query.")),
        (".encoder_attn.k_proj.", Some(".cross_attn.key.")),
        (".encoder_attn.v_proj.", Some(".cross_attn.value.")),
        (".encoder_attn.out_proj.", Some(".cross_attn.out.")),
    ];
    for (viejo, nuevo) in MAPA {
        if k.contains(viejo) {
            k = k.replace(viejo, nuevo?);
        }
    }
    Some(k.replace("decoder.embed_tokens.", "decoder.token_embedding."))
}

/// El banco mel de `mlx_audio.dsp.mel_filters(16000, 400, n, norm="slaney", mel_scale=None)` con
/// las MISMAS operaciones de MLX en f32. Devuelve [201, n] (listo para `magnitudes · banco`).
fn banco_mel(n_mels: i32) -> Result<Array> {
    let f_sp = 200.0f64 / 3.0;
    let min_log_hz = 1000.0f64;
    let min_log_mel = min_log_hz / f_sp;
    let logstep = 6.4f64.ln() / 27.0;
    let hz_a_mel = |f: f64| if f >= min_log_hz { min_log_mel + (f / min_log_hz).ln() / logstep } else { f / f_sp };
    let lin = |a: f64, b: f64, n: i32| ops::linspace::<f64, f32>(a, b, ops::LinspaceOptions { count: n, endpoint: true }).m();
    let bins = (N_FFT / 2 + 1) as i32;
    let todas = lin(0.0, (MUESTREO / 2) as f64, bins)?;
    let m_pts = lin(hz_a_mel(0.0), hz_a_mel((MUESTREO / 2) as f64), n_mels + 2)?;
    let e = |v: f64| Array::from_f32(v as f32);
    let lineal = ops::add(e(0.0), ops::multiply(e(f_sp), &m_pts).m()?).m()?;
    let logar = ops::multiply(e(min_log_hz), ops::exp(ops::multiply(e(logstep), ops::subtract(&m_pts, e(min_log_mel)).m()?).m()?).m()?).m()?;
    let f_pts = ops::select(ops::ge(&m_pts, e(min_log_mel)).m()?, &logar, &lineal).m()?;
    let n = n_mels + 2;
    let tramo = |a: &Array, i: i32, j: i32| a.try_index(i..j).m();
    let f_dif = ops::subtract(tramo(&f_pts, 1, n)?, tramo(&f_pts, 0, n - 1)?).m()?;
    let pendientes = ops::subtract(ops::expand_dims(&f_pts, 0).m()?, ops::expand_dims(&todas, 1).m()?).m()?;
    let baja = ops::divide(ops::negative(pendientes.try_index((.., 0..n - 2)).m()?).m()?, tramo(&f_dif, 0, n - 2)?).m()?;
    let sube = ops::divide(pendientes.try_index((.., 2..n)).m()?, tramo(&f_dif, 1, n - 1)?).m()?;
    let fb = ops::maximum(ops::zeros_like(&baja).m()?, ops::minimum(&baja, &sube).m()?).m()?;
    let enorm = ops::divide(e(2.0), ops::subtract(tramo(&f_pts, 2, n_mels + 2)?, tramo(&f_pts, 0, n_mels)?).m()?).m()?;
    ops::multiply(&fb, ops::expand_dims(&enorm, 0).m()?).m()
}

/// `HFTokenizer.decode(..., clean_up_tokenization_spaces=True)`: los espacios ante la puntuación.
fn limpiar(s: &str) -> String {
    s.replace(" .", ".")
        .replace(" ?", "?")
        .replace(" !", "!")
        .replace(" ,", ",")
        .replace(" ' ", "'")
        .replace(" n't", "n't")
        .replace(" 'm", "'m")
        .replace(" 's", "'s")
        .replace(" 've", "'ve")
        .replace(" 're", "'re")
}

/// `zlib.compress` (nivel 6): la razón de compresión que delata un texto repetido.
fn razon_compresion(t: &str) -> f64 {
    use std::io::Write;
    let b = t.as_bytes();
    let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::new(6));
    let _ = z.write_all(b);
    let c = z.finish().map(|v| v.len()).unwrap_or(1).max(1);
    b.len() as f64 / c as f64
}

fn logsumexp(v: &[f32]) -> f64 {
    let m = v.iter().copied().fold(f32::NEG_INFINITY, f32::max) as f64;
    if m.is_infinite() {
        return m;
    }
    m + v.iter().map(|&x| ((x as f64) - m).exp()).sum::<f64>().ln()
}

/// Un generador pequeño (xorshift64*) para el muestreo con temperatura.
struct Azar(u64);

impl Azar {
    fn nuevo() -> Azar {
        let s = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(0x9E37_79B9);
        Azar(s | 1)
    }
    fn uniforme(&mut self) -> f64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        (self.0.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 11) as f64 / (1u64 << 53) as f64
    }
}

impl Whisper {
    pub fn cargar(dir: &Path) -> Result<Whisper> {
        let cfg: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dir.join("config.json"))?)?;
        let dims = Dims::leer(&cfg).ok_or_else(|| anyhow!("config.json de Whisper incompleto"))?;
        let mut p: HashMap<String, Array> = HashMap::new();
        for (k, v) in crate::modelo::cargar_pesos_de(dir, "")? {
            if v.dtype() == Dtype::Uint32 {
                bail_tr!("este Whisper está cuantizado; el núcleo corre los de f16/f32 (p. ej. whisper-large-v3-turbo-asr-fp16)", "this Whisper is quantized; the core runs f16/f32 ones (e.g. whisper-large-v3-turbo-asr-fp16)");
            }
            if let Some(k) = clave_mlx(&k) {
                let mut v = if v.dtype() != Dtype::Float16 { v.as_dtype(Dtype::Float16).m()? } else { v };
                // conv1d de PyTorch [sal, ent, k] → MLX [sal, k, ent]
                if (k.ends_with("conv1.weight") || k.ends_with("conv2.weight")) && v.ndim() == 3 && v.dim(1) != 3 {
                    v = ops::transpose_axes(&v, &[0, 2, 1]).m()?;
                }
                p.insert(k, v);
            }
        }
        let mut w = |n: &str| p.remove(n).ok_or_else(|| anyhow!("falta {n} en Whisper"));
        let t = |a: Array| -> Result<Array> { ops::transpose_axes(&a, &[1, 0]).m() };
        let lin = |w: &mut dyn FnMut(&str) -> Result<Array>, n: &str, sesgo: bool| -> Result<Lin> {
            Ok(Lin { w_t: t(w(&format!("{n}.weight"))?)?, b: if sesgo { Some(w(&format!("{n}.bias"))?) } else { None } })
        };
        let atn = |w: &mut dyn FnMut(&str) -> Result<Array>, n: &str| -> Result<Atn> {
            Ok(Atn { q: lin(w, &format!("{n}.query"), true)?, k: lin(w, &format!("{n}.key"), false)?, v: lin(w, &format!("{n}.value"), true)?, o: lin(w, &format!("{n}.out"), true)? })
        };
        let bloques = |w: &mut dyn FnMut(&str) -> Result<Array>, raiz: &str, n: i32, cruz: bool| -> Result<Vec<Bloque>> {
            (0..n)
                .map(|i| {
                    let b = format!("{raiz}.blocks.{i}");
                    let ln = |w: &mut dyn FnMut(&str) -> Result<Array>, s: &str| -> Result<(Array, Array)> { Ok((w(&format!("{b}.{s}.weight"))?, w(&format!("{b}.{s}.bias"))?)) };
                    Ok(Bloque {
                        atn: atn(w, &format!("{b}.attn"))?,
                        atn_ln: ln(w, "attn_ln")?,
                        cruz: if cruz { Some((atn(w, &format!("{b}.cross_attn"))?, ln(w, "cross_attn_ln")?)) } else { None },
                        mlp1: Lin { w_t: t(w(&format!("{b}.mlp1.weight"))?)?, b: Some(w(&format!("{b}.mlp1.bias"))?) },
                        mlp2: Lin { w_t: t(w(&format!("{b}.mlp2.weight"))?)?, b: Some(w(&format!("{b}.mlp2.bias"))?) },
                        mlp_ln: ln(w, "mlp_ln")?,
                    })
                })
                .collect()
        };
        let enc = bloques(&mut w, "encoder", dims.n_capas_audio, false)?;
        let dec = bloques(&mut w, "decoder", dims.n_capas_texto, true)?;
        let conv1 = (w("encoder.conv1.weight")?, w("encoder.conv1.bias")?);
        let conv2 = (w("encoder.conv2.weight")?, w("encoder.conv2.bias")?);
        let ln_post = (w("encoder.ln_post.weight")?, w("encoder.ln_post.bias")?);
        let ln = (w("decoder.ln.weight")?, w("decoder.ln.bias")?);
        let emb = w("decoder.token_embedding.weight")?;
        let pos_texto = w("decoder.positional_embedding")?;
        // posición sinusoidal del audio (`sinusoids`), en f32 y a f16
        let mitad = dims.n_estado / 2;
        let inc = 10000f64.ln() / (mitad - 1) as f64;
        let inv = ops::exp(ops::multiply(Array::from_f32(-inc as f32), ops::arange::<_, f32>(0.0, mitad as f32, 1.0).m()?).m()?).m()?;
        let tiempo = ops::multiply(ops::expand_dims(ops::arange::<_, f32>(0.0, dims.n_audio_ctx as f32, 1.0).m()?, 1).m()?, ops::expand_dims(&inv, 0).m()?).m()?;
        let pos_audio = ops::concatenate(&[ops::sin(&tiempo).m()?, ops::cos(&tiempo).m()?], 1).m()?.as_dtype(Dtype::Float16).m()?;
        // máscara causal aditiva (`create_additive_causal_mask`): 0 / mínimo de f32, a f16 (−∞)
        let n = dims.n_text_ctx as usize;
        let m: Vec<f32> = (0..n * n).map(|i| if i % n > i / n { f32::MIN } else { 0.0 }).collect();
        let mascara = Array::from_slice(&m, &[n as i32, n as i32]).as_dtype(Dtype::Float16).m()?;
        // Hann periódica como Python (f64 → f32)
        let ventana: Vec<f32> = (0..N_FFT).map(|i| (0.5 * (1.0 - (2.0 * std::f64::consts::PI * i as f64 / N_FFT as f64).cos())) as f32).collect();
        let ventana = Array::from_slice(&ventana, &[N_FFT as i32]);
        let filtros = banco_mel(dims.n_mels)?;
        // tokenizador y especiales
        let tok = tokenizers::Tokenizer::from_file(dir.join("tokenizer.json")).map_err(|e| anyhow!("tokenizer.json de Whisper: {e}"))?;
        let id = |s: &str| tok.token_to_id(s).ok_or_else(|| anyhow_tr!("Whisper sin el token {s}", "Whisper without the {s} token"));
        let sot = id("<|startoftranscript|>")?;
        let multilingue = dims.n_vocab >= 51865;
        let n_idiomas = (dims.n_vocab - 51765 - multilingue as i32).max(0) as u32;
        let esp = Especiales {
            eot: id("<|endoftext|>")?,
            sot,
            sot_prev: id("<|startofprev|>")?,
            sot_lm: id("<|startoflm|>")?,
            transcribir: id("<|transcribe|>")?,
            traducir: id("<|translate|>")?,
            no_habla: tok.token_to_id("<|nospeech|>").or_else(|| tok.token_to_id("<|nocaptions|>")),
            sin_marcas: id("<|notimestamps|>")?,
            marca0: id("<|0.00|>")?,
            idiomas: if multilingue { (sot + 1..sot + 1 + n_idiomas).collect() } else { Vec::new() },
        };
        let tc: serde_json::Value = std::fs::read_to_string(dir.join("tokenizer_config.json")).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        let limpiar_espacios = tc["clean_up_tokenization_spaces"].as_bool().unwrap_or(true);
        let mut wsp = Whisper {
            dims,
            conv1,
            conv2,
            pos_audio,
            enc,
            ln_post,
            emb_t: t(emb.clone())?,
            emb,
            pos_texto,
            dec,
            ln,
            mascara,
            filtros,
            ventana,
            tok,
            esp,
            suprimir: Vec::new(),
            blanco: Vec::new(),
            limpiar_espacios,
            f: Fusiones::nuevas(),
        };
        wsp.suprimir = wsp.tokens_suprimidos();
        wsp.blanco = wsp.codificar(" ").into_iter().chain([wsp.esp.eot]).collect();
        Ok(wsp)
    }

    fn codificar(&self, s: &str) -> Vec<u32> {
        self.tok.encode(s, false).map(|e| e.get_ids().to_vec()).unwrap_or_default()
    }

    /// Texto de unos tokens (sin las marcas de tiempo), como `HFTokenizerWrapper.decode`.
    fn texto(&self, ids: &[u32]) -> String {
        let f: Vec<u32> = ids.iter().copied().filter(|&t| t < self.esp.marca0).collect();
        let s = self.tok.decode(&f, false).unwrap_or_default();
        if self.limpiar_espacios { limpiar(&s) } else { s }
    }

    /// `get_suppress_tokens(tokenizer, "-1")`: símbolos que no son habla + especiales.
    fn tokens_suprimidos(&self) -> Vec<u32> {
        let mut r = std::collections::BTreeSet::new();
        for semilla in [" -", " '"] {
            if let Some(&t) = self.codificar(semilla).first() {
                r.insert(t);
            }
        }
        let mut simbolos: Vec<String> = "\"#()*+/:;<=>@[\\]^_`{|}~「」『』".chars().map(String::from).collect();
        simbolos.extend("<< >> <<< >>> -- --- -( -[ (' (\" (( )) ((( ))) [[ ]] {{ }} ♪♪ ♪♪♪".split(' ').map(String::from));
        let varios = "♩♪♫♬♭♮♯";
        simbolos.extend(varios.chars().map(String::from));
        for s in &simbolos {
            let es_vario = s.chars().count() == 1 && varios.contains(s.as_str());
            for t in [self.codificar(s), self.codificar(&format!(" {s}"))] {
                if !t.is_empty() && (t.len() == 1 || es_vario) {
                    r.insert(t[0]);
                }
            }
        }
        let e = &self.esp;
        r.extend([e.transcribir, e.traducir, e.sot, e.sot_prev, e.sot_lm]);
        r.extend(e.no_habla);
        r.into_iter().collect()
    }

    fn ln(x: &Array, p: &(Array, Array)) -> Result<Array> {
        fast::layer_norm(x, Some(&p.0), Some(&p.1), 1e-5).m()
    }

    fn gelu(&self, x: &Array) -> Result<Array> {
        (self.f.gelu_exacta.borrow_mut())(x).m()
    }

    /// `MultiHeadAttention.qkv_attention`.
    fn qkv(q: &Array, k: &Array, v: &Array, cab: i32, mascara: Option<&Array>) -> Result<Array> {
        let (b, n, d) = (q.dim(0), q.dim(1), q.dim(2));
        let kl = k.dim(1);
        let esc = Array::from_f32(((d / cab) as f64).powf(-0.25) as f32).as_dtype(q.dtype()).m()?;
        let q = ops::multiply(ops::transpose_axes(ops::reshape(q, &[b, n, cab, -1]).m()?, &[0, 2, 1, 3]).m()?, &esc).m()?;
        let k = ops::multiply(ops::transpose_axes(ops::reshape(k, &[b, kl, cab, -1]).m()?, &[0, 2, 3, 1]).m()?, &esc).m()?;
        let v = ops::transpose_axes(ops::reshape(v, &[b, kl, cab, -1]).m()?, &[0, 2, 1, 3]).m()?;
        let mut qk = ops::matmul(&q, &k).m()?;
        if let Some(m) = mascara {
            qk = ops::add(&qk, m.try_index((0..n, 0..n)).m()?).m()?;
        }
        let w = ops::softmax_axis(&qk, -1, true).m()?;
        ops::reshape(ops::transpose_axes(ops::matmul(&w, &v).m()?, &[0, 2, 1, 3]).m()?, &[b, n, d]).m()
    }

    /// Un bloque residual; `cache` = (K/V propios, K/V cruzados) del decodificador (None en el
    /// codificador).
    fn bloque(&self, b: &Bloque, x: &Array, xa: Option<&Array>, cab: i32, mascara: Option<&Array>, cache: Option<(&mut Kv, &mut Kv)>) -> Result<Array> {
        let (kv, cruz) = match cache {
            Some((a, b)) => (Some(a), Some(b)),
            None => (None, None),
        };
        let y = Self::ln(x, &b.atn_ln)?;
        let q = b.atn.q.aplicar(&y)?;
        let (mut k, mut v) = (b.atn.k.aplicar(&y)?, b.atn.v.aplicar(&y)?);
        if let Some(c) = kv {
            if let Some((pk, pv)) = c.as_ref() {
                k = ops::concatenate(&[pk.clone(), k], 1).m()?;
                v = ops::concatenate(&[pv.clone(), v], 1).m()?;
            }
            *c = Some((k.clone(), v.clone()));
        }
        let mut x = ops::add(x, b.atn.o.aplicar(&Self::qkv(&q, &k, &v, cab, mascara)?)?).m()?;
        if let (Some((a, ln)), Some(xa)) = (&b.cruz, xa) {
            let y = Self::ln(&x, ln)?;
            let q = a.q.aplicar(&y)?;
            let (k, v) = match cruz {
                Some(c) => {
                    if c.is_none() {
                        *c = Some((a.k.aplicar(xa)?, a.v.aplicar(xa)?));
                    }
                    c.clone().ok_or_else(|| anyhow_tr!("caché cruzada", "cross cache"))?
                }
                None => (a.k.aplicar(xa)?, a.v.aplicar(xa)?),
            };
            x = ops::add(&x, a.o.aplicar(&Self::qkv(&q, &k, &v, cab, None)?)?).m()?;
        }
        let y = b.mlp2.aplicar(&self.gelu(&b.mlp1.aplicar(&Self::ln(&x, &b.mlp_ln)?)?)?)?;
        ops::add(&x, &y).m()
    }

    /// El espectrograma log-mel de TODO el audio + 30 s de silencio: [cuadros, n_mels] en f32.
    pub fn mel(&self, pcm: &[f32]) -> Result<Array> {
        let mel = ops::matmul(&self.magnitudes(pcm)?, &self.filtros).m()?;
        let log = ops::log10(ops::maximum(&mel, Array::from_f32(1e-10)).m()?).m()?;
        let tope = ops::subtract(log.max(None).m()?, Array::from_f32(8.0)).m()?;
        let log = ops::maximum(&log, &tope).m()?;
        ops::divide(ops::add(&log, Array::from_f32(4.0)).m()?, Array::from_f32(4.0)).m()
    }

    /// |STFT|² del audio + 30 s de silencio, sin el último cuadro: [cuadros, 201].
    fn magnitudes(&self, pcm: &[f32]) -> Result<Array> {
        let mut x: Vec<f32> = Vec::with_capacity(pcm.len() + N_MUESTRAS + N_FFT);
        let r = N_FFT / 2;
        let largo = pcm.len() + N_MUESTRAS;
        let muestra = |i: usize| if i < pcm.len() { pcm[i] } else { 0.0 };
        // reflejo: x[1..=r] invertido + x + x[−r−1..−1] invertido
        x.extend((1..=r).rev().map(muestra));
        x.extend((0..largo).map(muestra));
        x.extend((largo - 1 - r..largo - 1).rev().map(muestra));
        let n = 1 + (x.len() - N_FFT) / SALTO;
        let mut cuadros = Vec::with_capacity(n * N_FFT);
        for i in 0..n {
            cuadros.extend_from_slice(&x[i * SALTO..i * SALTO + N_FFT]);
        }
        let c = ops::multiply(Array::from_slice(&cuadros, &[n as i32, N_FFT as i32]), &self.ventana).m()?;
        let espectro = mlx_rs::fft::rfft(&c, N_FFT as i32, -1).m()?;
        let espectro = espectro.try_index(0..n as i32 - 1).m()?;
        ops::square(ops::abs(&espectro).m()?).m()
    }

    /// Una ventana de 3000 cuadros (recortada o rellena con ceros), en f16: [1, 3000, n_mels].
    fn ventana_mel(mel: &Array, desde: i32, cuantos: i32) -> Result<Array> {
        let s = mel.try_index(desde..desde + cuantos).m()?;
        let s = if cuantos < N_CUADROS as i32 { ops::pad(&s, &[(0, N_CUADROS as i32 - cuantos), (0, 0)], None, None).m()? } else { s };
        ops::expand_dims(s.as_dtype(Dtype::Float16).m()?, 0).m()
    }

    /// El codificador: [1, 3000, n_mels] → [1, 1500, n_estado].
    pub fn codificador(&self, mel: &Array) -> Result<Array> {
        let x = ops::add(ops::conv1d(mel, &self.conv1.0, 1, 1, 1, 1).m()?, &self.conv1.1).m()?;
        let x = self.gelu(&x)?;
        let x = ops::add(ops::conv1d(&x, &self.conv2.0, 2, 1, 1, 1).m()?, &self.conv2.1).m()?;
        let x = self.gelu(&x)?;
        if x.dim(1) != self.pos_audio.dim(0) {
            bail!("forma de audio inesperada en Whisper");
        }
        let mut x = ops::add(&x, &self.pos_audio).m()?;
        for b in &self.enc {
            x = self.bloque(b, &x, None, self.dims.n_cab_audio, None, None)?;
        }
        Self::ln(&x, &self.ln_post)
    }

    /// El decodificador sobre unos tokens nuevos: logits [n, vocab] en f32.
    fn decodificador(&self, ids: &[u32], xa: &Array, e: &mut Estado) -> Result<Array> {
        let desde = e.kv[0].as_ref().map(|(k, _)| k.dim(1)).unwrap_or(0);
        let n = ids.len() as i32;
        let x = take_axis(&self.emb, Array::from_slice(ids, &[1, n]), 0).m()?;
        let mut x = ops::add(&x, self.pos_texto.try_index(desde..desde + n).m()?).m()?;
        for (i, b) in self.dec.iter().enumerate() {
            x = self.bloque(b, &x, Some(xa), self.dims.n_cab_texto, Some(&self.mascara), Some((&mut e.kv[i], &mut e.cruz[i])))?;
        }
        let l = ops::matmul(Self::ln(&x, &self.ln)?, &self.emb_t).m()?;
        ops::reshape(l.as_dtype(Dtype::Float32).m()?, &[n, -1]).m()
    }

    fn estado(&self) -> Estado {
        let n = self.dec.len();
        Estado { kv: vec![None; n], cruz: vec![None; n] }
    }

    /// El idioma del audio: el token de idioma más probable tras `<|startoftranscript|>`.
    pub fn idioma(&self, enc: &Array) -> Result<u32> {
        if self.esp.idiomas.is_empty() {
            return self.tok.token_to_id("<|en|>").ok_or_else(|| anyhow_tr!("Whisper sin <|en|>", "Whisper without <|en|>"));
        }
        let mut e = self.estado();
        let l = self.decodificador(&[self.esp.sot], enc, &mut e)?;
        let v: Vec<f32> = l.as_dtype(Dtype::Float32).m()?.as_slice::<f32>().to_vec();
        let mut mejor = (self.esp.idiomas[0], f32::NEG_INFINITY);
        for &t in &self.esp.idiomas {
            if v[t as usize] > mejor.1 {
                mejor = (t, v[t as usize]);
            }
        }
        Ok(mejor.0)
    }

    /// Las supresiones y las reglas de marcas de tiempo sobre los logits del paso.
    fn filtrar(&self, l: &mut [f32], tokens: &[u32], inicio: usize) {
        let e = &self.esp;
        let menos = f32::NEG_INFINITY;
        let tb = e.marca0 as usize;
        if tokens.len() == inicio {
            for &t in &self.blanco {
                l[t as usize] = menos;
            }
        }
        for &t in &self.suprimir {
            l[t as usize] = menos;
        }
        // reglas de marcas de tiempo
        l[e.sin_marcas as usize] = menos;
        let seq = &tokens[inicio..];
        let ultima = seq.last().is_some_and(|&t| t >= e.marca0);
        let penultima = seq.len() < 2 || seq[seq.len() - 2] >= e.marca0;
        if ultima {
            if penultima {
                l[tb..].iter_mut().for_each(|x| *x = menos);
            } else {
                l[..e.eot as usize].iter_mut().for_each(|x| *x = menos);
            }
        }
        if let Some(&ult) = seq.iter().rev().find(|&&t| t >= e.marca0) {
            // las marcas no retroceden (y cada tramo dura algo): el VALOR, como OpenAI
            let tope = if ultima && !penultima { ult } else { ult + 1 } as usize;
            let tope = tope.min(l.len());
            l[tb..tope].iter_mut().for_each(|x| *x = menos);
        }
        if tokens.len() == inicio {
            l[..tb].iter_mut().for_each(|x| *x = menos);
            // `max_initial_timestamp` = 1 s → índice 50
            let ultimo = tb + 50;
            l[ultimo + 1..].iter_mut().for_each(|x| *x = menos);
        }
        // si la suma de probabilidad de las marcas supera a cualquier texto, toca marca
        let lse = logsumexp(l);
        let marcas = logsumexp(&l[tb..]) - lse;
        let texto = l[..tb].iter().copied().fold(f32::NEG_INFINITY, f32::max) as f64 - lse;
        if marcas > texto {
            l[..tb].iter_mut().for_each(|x| *x = menos);
        }
    }

    /// Decodifica una ventana (ya codificada) con una temperatura (`DecodingTask.run`).
    fn decodificar(&self, enc: &Array, iniciales: &[u32], temp: f32, azar: &mut Azar) -> Result<Resultado> {
        let e = &self.esp;
        let inicio = iniciales.len();
        let i_sot = iniciales.iter().position(|&t| t == e.sot).unwrap_or(0);
        let max_muestras = (self.dims.n_text_ctx / 2) as usize;
        let mut st = self.estado();
        let mut tokens = iniciales.to_vec();
        let primeros = self.decodificador(iniciales, enc, &mut st)?;
        let no_habla = match e.no_habla {
            Some(nh) => {
                let fila: Vec<f32> = primeros.try_index(i_sot as i32).m()?.as_slice::<f32>().to_vec();
                ((fila[nh as usize] as f64) - logsumexp(&fila)).exp()
            }
            None => f64::NAN,
        };
        let mut l: Vec<f32> = primeros.try_index(inicio as i32 - 1).m()?.as_slice::<f32>().to_vec();
        let mut suma = 0f32;
        let mut muestras = 0;
        loop {
            self.filtrar(&mut l, &tokens, inicio);
            let sig = if temp == 0.0 {
                let mut m = 0;
                for (i, &x) in l.iter().enumerate() {
                    if x > l[m] {
                        m = i;
                    }
                }
                m as u32
            } else {
                let lse = logsumexp(&l.iter().map(|&x| x / temp).collect::<Vec<_>>());
                let u = azar.uniforme();
                let mut acum = 0f64;
                let mut elegido = e.eot;
                for (i, &x) in l.iter().enumerate() {
                    acum += ((x / temp) as f64 - lse).exp();
                    if acum > u {
                        elegido = i as u32;
                        break;
                    }
                }
                elegido
            };
            if tokens.last() != Some(&e.eot) {
                suma += ((l[sig as usize] as f64) - logsumexp(&l)) as f32;
            }
            tokens.push(sig);
            muestras += 1;
            if sig == e.eot || muestras >= max_muestras || tokens.len() > self.dims.n_text_ctx as usize {
                break;
            }
            l = self.decodificador(&[sig], enc, &mut st)?.as_slice::<f32>().to_vec();
        }
        let mut salida: Vec<u32> = tokens[inicio..].to_vec();
        if let Some(p) = salida.iter().position(|&t| t == e.eot) {
            salida.truncate(p);
        }
        let texto = self.texto(&salida).trim().to_string();
        Ok(Resultado {
            logprob_media: suma as f64 / (salida.len() + 1) as f64,
            compresion: razon_compresion(&texto),
            tokens: salida,
            no_habla,
            temperatura: temp,
        })
    }

    /// `decode_with_fallback`: sube la temperatura si el texto se repite o la confianza es baja.
    fn con_respaldo(&self, enc: &Array, iniciales: &[u32], azar: &mut Azar) -> Result<Resultado> {
        let mut r = None;
        for &t in &TEMPERATURAS {
            let res = self.decodificar(enc, iniciales, t, azar)?;
            let mut otra = res.compresion > TOPE_COMPRESION || res.logprob_media < TOPE_LOGPROB;
            if res.no_habla > TOPE_NO_HABLA && res.logprob_media < TOPE_LOGPROB {
                otra = false; // silencio
            }
            r = Some(res);
            if !otra {
                break;
            }
        }
        r.ok_or_else(|| anyhow_tr!("sin decodificación", "no decoding"))
    }

    /// Transcribe un PCM mono de 16 kHz en [−1, 1] (`Model.generate` con sus valores por defecto).
    pub fn transcribir(&self, pcm: &[f32]) -> Result<Transcripcion> {
        let e = &self.esp;
        let mel = self.mel(pcm)?;
        let cuadros = mel.dim(0) as usize - N_CUADROS;
        // idioma: con la primera ventana COMPLETA del mel (incluido el silencio de relleno)
        let enc0 = self.codificador(&Self::ventana_mel(&mel, 0, N_CUADROS as i32)?)?;
        let t_idioma = self.idioma(&enc0)?;
        let idioma = self.tok.id_to_token(t_idioma).unwrap_or_default().trim_matches(|c| c == '<' || c == '>' || c == '|').to_string();
        let sot_seq: Vec<u32> = if e.idiomas.is_empty() { vec![e.sot] } else { vec![e.sot, t_idioma, e.transcribir] };
        let mut azar = Azar::nuevo();
        let (mut seek, mut todos, mut desde_prompt) = (0usize, Vec::<u32>::new(), 0usize);
        let mut segmentos = Vec::new();
        let paso_entrada = N_CUADROS / self.dims.n_audio_ctx as usize; // 2 cuadros por posición
        let precision = (paso_entrada * SALTO) as f64 / MUESTREO as f64; // 0,02 s
        while seek < cuadros {
            let t0 = (seek * SALTO) as f64 / MUESTREO as f64;
            let tam = N_CUADROS.min(cuadros - seek);
            let dur = (tam * SALTO) as f64 / MUESTREO as f64;
            let enc = self.codificador(&Self::ventana_mel(&mel, seek as i32, tam as i32)?)?;
            let previo = &todos[desde_prompt..];
            let mut iniciales = Vec::new();
            if !previo.is_empty() {
                let max = (self.dims.n_text_ctx / 2 - 1) as usize;
                iniciales.push(e.sot_prev);
                iniciales.extend_from_slice(&previo[previo.len().saturating_sub(max)..]);
            }
            iniciales.extend_from_slice(&sot_seq);
            let r = self.con_respaldo(&enc, &iniciales, &mut azar)?;
            if r.no_habla > TOPE_NO_HABLA && r.logprob_media <= TOPE_LOGPROB {
                seek += tam; // silencio: a la ventana siguiente
                continue;
            }
            let tk = &r.tokens;
            let es_marca: Vec<bool> = tk.iter().map(|&t| t >= e.marca0).collect();
            let unica_al_final = tk.len() >= 2 && !es_marca[tk.len() - 2] && es_marca[tk.len() - 1];
            let consecutivas: Vec<usize> = (0..tk.len().saturating_sub(1)).filter(|&i| es_marca[i] && es_marca[i + 1]).map(|i| i + 1).collect();
            let mut actuales: Vec<Segmento> = Vec::new();
            let nuevo = |a: f64, b: f64, t: &[u32]| Segmento { inicio: a, fin: b, texto: self.texto(&t.iter().copied().filter(|&x| x < e.eot).collect::<Vec<_>>()), tokens: t.to_vec(), temperatura: r.temperatura };
            if !consecutivas.is_empty() {
                let mut cortes = consecutivas.clone();
                if unica_al_final {
                    cortes.push(tk.len());
                }
                let mut ultimo = 0;
                for &c in &cortes {
                    let s = &tk[ultimo..c];
                    let a = (s[0] - e.marca0) as f64;
                    let b = (s[s.len() - 1] - e.marca0) as f64;
                    actuales.push(nuevo(t0 + a * precision, t0 + b * precision, s));
                    ultimo = c;
                }
                if unica_al_final {
                    seek += tam;
                } else {
                    seek += (tk[ultimo - 1] - e.marca0) as usize * paso_entrada;
                }
            } else {
                let mut d = dur;
                if let Some(&ult) = tk.iter().rev().find(|&&t| t >= e.marca0) {
                    if ult != e.marca0 {
                        d = (ult - e.marca0) as f64 * precision;
                    }
                }
                actuales.push(nuevo(t0, t0 + d, tk));
                seek += tam;
            }
            for s in actuales.iter_mut() {
                if s.inicio == s.fin || s.texto.trim().is_empty() {
                    s.texto.clear();
                    s.tokens.clear();
                }
            }
            todos.extend(actuales.iter().flat_map(|s| s.tokens.iter().copied()));
            segmentos.extend(actuales);
            if r.temperatura > 0.5 {
                desde_prompt = todos.len(); // con temperatura alta no se arrastra el contexto
            }
        }
        Ok(Transcripcion { texto: self.texto(&todos), idioma, segmentos })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limpia_espacios_como_transformers() {
        assert_eq!(limpiar(" 7 , 3 . ¿ok ?"), " 7, 3. ¿ok?");
        assert_eq!(limpiar("it 's I 'm"), "it's I'm");
    }

    #[test]
    fn la_compresion_delata_repeticiones() {
        assert!(razon_compresion(&"hola ".repeat(50)) > TOPE_COMPRESION);
        assert!(razon_compresion("El laboratorio abre a las ocho de la mañana.") < TOPE_COMPRESION);
    }

    #[test]
    fn mapea_claves_de_hugging_face() {
        assert_eq!(clave_mlx("model.encoder.layers.3.self_attn.q_proj.weight").as_deref(), Some("encoder.blocks.3.attn.query.weight"));
        assert_eq!(clave_mlx("model.decoder.layers.0.encoder_attn_layer_norm.bias").as_deref(), Some("decoder.blocks.0.cross_attn_ln.bias"));
        assert_eq!(clave_mlx("model.encoder.embed_positions.weight"), None);
        assert_eq!(clave_mlx("decoder.blocks.0.mlp1.weight").as_deref(), Some("decoder.blocks.0.mlp1.weight"));
    }

    fn comparar(a: &[f32], b: &[f32]) -> (f32, f32) {
        let d = a.iter().zip(b).map(|(x, y)| (x - y).abs()).fold(0.0, f32::max);
        let n = |x: &[f32]| x.iter().map(|v| v * v).sum::<f32>().sqrt();
        (d, a.iter().zip(b).map(|(x, y)| x * y).sum::<f32>() / (n(a) * n(b)))
    }

    /// El mel por etapas contra mlx-audio (`ref_mel_etapas.py`): ventana, banco, |STFT|², mel lineal.
    /// Medido 2026-11-10: la rFFT en GPU de MLX CAMBIÓ entre 0.32.2 (la de mlx-sys) y 0.32.4 (la del
    /// Python): contra 0.32.2 el |STFT|² es idéntico (0 diferencias); contra 0.32.4, 3 % en el último bit.
    #[test]
    #[ignore]
    fn mel_por_etapas() {
        let (Ok(dir), Ok(et), Ok(wav)) = (std::env::var("GOTV_WHISPER"), std::env::var("GOTV_MEL_ETAPAS"), std::env::var("GOTV_WAV")) else { return };
        let w = Whisper::cargar(Path::new(&dir)).unwrap();
        let r = Array::load_safetensors(&et).unwrap();
        let pcm = crate::audio_gemma::pcm_de_wav(&std::fs::read(&wav).unwrap()).unwrap();
        let mag = w.magnitudes(&pcm).unwrap();
        let lineal = ops::matmul(&mag, &w.filtros).unwrap();
        // la misma |rFFT|² forzada a CPU y a GPU
        let por = |dev: mlx_rs::Device| -> Array {
            mlx_rs::with_device(dev, || {
                let m = w.magnitudes(&pcm).unwrap();
                m.eval().unwrap();
                m
            })
        };
        let (cpu, gpu) = (por(mlx_rs::Device::cpu()), por(mlx_rs::Device::gpu()));
        for (n, a) in [("ventana", &w.ventana), ("filtros", &w.filtros), ("mag", &mag), ("mel_lineal", &lineal), ("mag_cpu", &cpu), ("mag_gpu", &gpu)] {
            // la referencia guarda una sola |STFT|²: la del dispositivo por defecto (GPU)
            let b = &r[if n.starts_with("mag") { "mag" } else { n }];
            let distintos = a.as_slice::<f32>().iter().zip(b.as_slice::<f32>()).filter(|(x, y)| x != y).count();
            let (d, _) = comparar(a.as_slice::<f32>(), b.as_slice::<f32>());
            println!("{n}: forma {:?}/{:?} · distintos {distintos} · dif máx {d:e}", a.shape(), b.shape());
        }
    }

    /// Juez contra mlx-audio (`ref_whisper.py`): mel, codificador, supresiones, idioma, texto y
    /// tramos, para cada audio de `GOTV_REF_WHISPER/<nombre>` con su `<nombre>.wav` en `GOTV_AUDIOS`.
    /// `GOTV_WHISPER=<dir> GOTV_REF_WHISPER=<dir> GOTV_AUDIOS=<dir> cargo test -p gotv-mlx --release
    /// -- --ignored --nocapture transcribe_como_mlx_audio`
    #[test]
    #[ignore]
    fn transcribe_como_mlx_audio() {
        let (Ok(dir), Ok(rd), Ok(ad)) = (std::env::var("GOTV_WHISPER"), std::env::var("GOTV_REF_WHISPER"), std::env::var("GOTV_AUDIOS")) else { return };
        let t = std::time::Instant::now();
        let w = Whisper::cargar(Path::new(&dir)).unwrap();
        println!("carga {:?}", t.elapsed());
        let sup: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(Path::new(&rd).join("suprimir.json")).unwrap()).unwrap();
        let num = |v: &serde_json::Value| -> Vec<u32> { v.as_array().unwrap().iter().map(|x| x.as_u64().unwrap() as u32).collect() };
        assert_eq!(w.suprimir, num(&sup["suprimir"]), "tokens suprimidos");
        assert_eq!(w.blanco, num(&sup["blanco"]), "blancos");
        assert_eq!(w.esp.idiomas, num(&sup["idiomas"]), "tokens de idioma");
        let mut nombres: Vec<String> = std::fs::read_dir(&rd).unwrap().filter_map(|e| e.ok()).filter(|e| e.path().is_dir()).map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        nombres.sort();
        for n in nombres {
            let r = Path::new(&rd).join(&n);
            let rj: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(r.join("ref.json")).unwrap()).unwrap();
            let rs = Array::load_safetensors(r.join("ref.safetensors")).unwrap();
            let pcm = crate::audio_gemma::pcm_de_wav(&std::fs::read(Path::new(&ad).join(format!("{n}.wav"))).unwrap()).unwrap();
            assert_eq!(comparar(&pcm, rs["pcm"].as_slice::<f32>()).0, 0.0, "{n}: PCM");
            let mel = w.mel(&pcm).unwrap();
            assert_eq!(mel.shape(), rs["mel"].shape(), "{n}: forma del mel");
            let (d1, _) = comparar(mel.as_slice::<f32>(), rs["mel"].as_slice::<f32>());
            let enc = w.codificador(&Whisper::ventana_mel(&mel, 0, N_CUADROS as i32).unwrap()).unwrap().as_dtype(Dtype::Float32).unwrap();
            let (d2, c2) = comparar(enc.as_slice::<f32>(), rs["enc"].as_slice::<f32>());
            // el codificador solo, con el mel de la REFERENCIA
            let enc_r = w.codificador(&Whisper::ventana_mel(&rs["mel"], 0, N_CUADROS as i32).unwrap()).unwrap().as_dtype(Dtype::Float32).unwrap();
            let (d2r, c2r) = comparar(enc_r.as_slice::<f32>(), rs["enc"].as_slice::<f32>());
            let mm: Vec<usize> = mel.as_slice::<f32>().iter().zip(rs["mel"].as_slice::<f32>()).enumerate().filter(|(_, (a, b))| a != b).map(|(i, _)| i).take(5).collect();
            println!("   codificador con el mel de la referencia: dif {d2r} cos {c2r:.6} · mel distinto en {:?}", mm);
            let t = std::time::Instant::now();
            let tr = w.transcribir(&pcm).unwrap();
            let seg = t.elapsed();
            let refseg = rj["segmentos"].as_array().unwrap();
            let iguales = tr.segmentos.iter().zip(refseg).filter(|(a, b)| a.tokens == num(&b["tokens"])).count();
            println!("{n}: mel dif {d1:e} · codificador dif {d2} cos {c2:.6} · idioma {} (ref {}) · {:?}\n   tramos iguales {iguales}/{} (propios {})\n   {:?}\n   ref {:?}", tr.idioma, rj["idioma"], seg, refseg.len(), tr.segmentos.len(), tr.texto, rj["texto"].as_str().unwrap());
            assert_eq!(tr.idioma, rj["idioma"].as_str().unwrap(), "{n}: idioma");
            assert_eq!(tr.texto, rj["texto"].as_str().unwrap(), "{n}: texto");
        }
    }
}
