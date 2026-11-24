//! M2 · la VOZ de GOTV en el núcleo: Qwen3-TTS 1.7B Base clonando la voz elegida por el Portador
//! (una voz SINTÉTICA hecha con VoiceDesign; su referencia vive en `voz-gotv.safetensors`). Port de
//! `mlx_audio/tts/models/qwen3_tts` (talker + predictor de códigos; el decodificador está en
//! `voz_decodificador.rs`):
//!
//! - **Prompt de clonación (ICL):** rol + prefijo de códec (pensar, idioma, vector de hablante) +
//!   [texto de la referencia + texto nuevo + fin] sobre PAD de códec + [BOS + códigos de la
//!   referencia] sobre PAD de texto.
//! - **Por fotograma (12,5 Hz):** el talker (Qwen3 de 28 capas que recibe embeddings) elige el código 0;
//!   el predictor (Qwen3 de 5 capas) elige los otros 15; la suma de sus embeddings + PAD de texto es la
//!   entrada siguiente. Para en el EOS del códec.
//! - Muestreo del modelo (`generation_config.json`): temperatura 0,9, top-k 50, penalización de
//!   repetición 1,05 sobre los últimos 64 códigos; los 1023 tokens especiales del códec, suprimidos.
//!   Temperatura 0 = voraz (los jueces).

use crate::modelo::{Fusiones, M};
use crate::voz_decodificador::Decodificador;
use anyhow::{anyhow, bail, Result};
use gotv_engine::{anyhow_tr, bail_tr, tr};
use mlx_rs::error::Exception;
use mlx_rs::fast::{self, ScaledDotProductAttentionMask};
use mlx_rs::ops::indexing::{argmax_axis, take_axis, TryIndexOp};
use mlx_rs::ops;
use mlx_rs::{Array, Dtype};
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;

type Giro = Box<dyn for<'a> FnMut((&'a Array, &'a Array, &'a Array)) -> std::result::Result<Array, Exception>>;

/// `apply_rotary_pos_emb` de mlx-audio (compilada allá): x·cos + rotate_half(x)·sin.
fn giro_compilado() -> RefCell<Giro> {
    let f = |(x, cos, sin): (&Array, &Array, &Array)| {
        let m = ops::split_equal(x, 2, -1)?;
        let rot = ops::concatenate(&[ops::negative(&m[1])?, m[0].clone()], -1)?;
        ops::add(&ops::multiply(x, cos)?, &ops::multiply(&rot, sin)?)
    };
    // SIN compilar: compilado, cada frase después de la primera iba ~4× más lenta (ver
    // `Fusiones::sin_compilar`)
    RefCell::new(Box::new(f))
}

/// El tokenizador de Qwen3-TTS como lo arma HF para Qwen2 (el repo no trae `tokenizer.json`): BPE de
/// bytes con la regex de Qwen2, NFC y los tokens añadidos de `tokenizer_config.json`.
pub fn tokenizador(dir: &Path) -> Result<tokenizers::Tokenizer> {
    let vocab: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dir.join("vocab.json"))?)?;
    let merges: Vec<serde_json::Value> = std::fs::read_to_string(dir.join("merges.txt"))?
        .lines()
        .filter(|l| !l.starts_with("#version") && !l.trim().is_empty())
        .filter_map(|l| l.split_once(' ').map(|(a, b)| serde_json::json!([a, b])))
        .collect();
    let tc: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dir.join("tokenizer_config.json"))?)?;
    let mut anadidos: Vec<(u64, serde_json::Value)> = tc["added_tokens_decoder"]
        .as_object()
        .ok_or_else(|| anyhow_tr!("tokenizer_config sin added_tokens_decoder", "tokenizer_config without added_tokens_decoder"))?
        .iter()
        .filter_map(|(id, v)| {
            Some((id.parse().ok()?, serde_json::json!({"id": id.parse::<u64>().ok()?, "content": v["content"], "single_word": v["single_word"], "lstrip": v["lstrip"], "rstrip": v["rstrip"], "normalized": v["normalized"], "special": v["special"]})))
        })
        .collect();
    anadidos.sort_by_key(|(id, _)| *id);
    let j = serde_json::json!({
        "version": "1.0", "truncation": null, "padding": null,
        "added_tokens": anadidos.into_iter().map(|(_, v)| v).collect::<Vec<_>>(),
        "normalizer": {"type": "NFC"},
        "pre_tokenizer": {"type": "Sequence", "pretokenizers": [
            {"type": "Split", "pattern": {"Regex": "(?i:'s|'t|'re|'ve|'m|'ll|'d)|[^\\r\\n\\p{L}\\p{N}]?\\p{L}+|\\p{N}| ?[^\\s\\p{L}\\p{N}]+[\\r\\n]*|\\s*[\\r\\n]+|\\s+(?!\\S)|\\s+"}, "behavior": "Isolated", "invert": false},
            {"type": "ByteLevel", "add_prefix_space": false, "trim_offsets": true, "use_regex": false}]},
        "post_processor": {"type": "TemplateProcessing", "single": [{"Sequence": {"id": "A", "type_id": 0}}], "pair": [{"Sequence": {"id": "A", "type_id": 0}}, {"Sequence": {"id": "B", "type_id": 1}}], "special_tokens": {}},
        "decoder": {"type": "ByteLevel", "add_prefix_space": true, "trim_offsets": true, "use_regex": true},
        "model": {"type": "BPE", "dropout": null, "unk_token": null, "continuing_subword_prefix": "", "end_of_word_suffix": "", "fuse_unk": false, "byte_fallback": false, "ignore_merges": false, "vocab": vocab, "merges": merges}
    });
    j.to_string().parse::<tokenizers::Tokenizer>().map_err(|e| anyhow!("tokenizador de Qwen3-TTS: {e}"))
}

#[derive(Clone, Debug, serde::Deserialize)]
struct TalkerCfg {
    hidden_size: i32,
    num_hidden_layers: i32,
    num_attention_heads: i32,
    num_key_value_heads: i32,
    head_dim: i32,
    rms_norm_eps: f32,
    rope_theta: f32,
    vocab_size: i32,
    num_code_groups: i32,
    codec_bos_id: i32,
    codec_eos_token_id: i32,
    codec_pad_id: i32,
    codec_think_id: i32,
    codec_think_bos_id: i32,
    codec_think_eos_id: i32,
    codec_nothink_id: i32,
    codec_language_id: HashMap<String, i32>,
    code_predictor_config: PredCfg,
}

#[derive(Clone, Debug, serde::Deserialize)]
struct PredCfg {
    num_hidden_layers: i32,
    num_attention_heads: i32,
    num_key_value_heads: i32,
    head_dim: i32,
    rms_norm_eps: f32,
    rope_theta: f32,
}

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

struct Capa {
    norma_in: Array,
    norma_post: Array,
    q: Lin,
    k: Lin,
    v: Lin,
    o: Lin,
    qn: Array,
    kn: Array,
    puerta: Lin,
    arriba: Lin,
    abajo: Lin,
}

/// Un transformador Qwen3 (talker o predictor) con su caché K/V propia.
struct Pila {
    capas: Vec<Capa>,
    norma: Array,
    nh: i32,
    nkv: i32,
    hd: i32,
    eps: f32,
    /// RoPE precalculada: cos/sin [POS_MAX, hd] en el tipo de los pesos (una vez, no en cada pasada)
    cos: Array,
    sin: Array,
}

/// Fotogramas de la referencia con que se ceba el decodificador (mlx-audio: `streaming_context_size`).
const CONTEXTO: i32 = 25;
/// Tamaño de cada trozo de onda en fotogramas (12,5 por segundo de habla): el primero corto para
/// que la voz arranque pronto, y luego CRECIENDO para que cada trozo esté listo antes de que acabe de
/// sonar lo anterior (la generación va a ~2× el tiempo real: un trozo de n tarda ~n·40 ms y suena
/// n·80 ms). El último tamaño se repite.
const TROZOS: [usize; 5] = [4, 6, 10, 16, 24];
/// Tope de una frase: 2000 fotogramas = 160 s (y la tabla de RoPE llega a 4096 posiciones).
pub const MAX_FOTOGRAMAS: usize = 2000;

/// Posiciones que cubre la tabla de RoPE (≈ 3 min de habla + el prompt).
const POS_MAX: i32 = 4096;

/// La forma de una pila Qwen3 (del `config.json` del talker o del predictor).
#[derive(Clone, Copy)]
struct Forma {
    n: i32,
    nh: i32,
    nkv: i32,
    hd: i32,
    eps: f32,
    theta: f32,
}

/// Caché K/V de una pila: por capa, reservada por bloques y escrita en su lugar (la de `modelo.rs`;
/// concatenar copiaba toda la caché del talker en cada fotograma).
struct CacheKv {
    capas: Vec<crate::modelo::CacheCapa>,
    largo: i32,
}

impl Pila {
    fn cargar(p: &mut HashMap<String, Array>, raiz: &str, f: Forma) -> Result<Pila> {
        let Forma { n, nh, nkv, hd, eps, theta } = f;
        let mut w = |k: &str| p.remove(k).ok_or_else(|| anyhow!("falta {k}"));
        let t = |a: Array| -> Result<Array> { ops::transpose_axes(&a, &[1, 0]).m() };
        let mut capas = Vec::new();
        for i in 0..n {
            let b = format!("{raiz}.layers.{i}");
            let lin = |w: &mut dyn FnMut(&str) -> Result<Array>, s: &str| -> Result<Lin> { Ok(Lin { w_t: t(w(&format!("{b}.{s}.weight"))?)?, b: None }) };
            // (q/k/v o gate/up en UNA matmul cambia el redondeo de MLX: medido, 8/120 fotogramas)
            capas.push(Capa {
                norma_in: w(&format!("{b}.input_layernorm.weight"))?,
                norma_post: w(&format!("{b}.post_attention_layernorm.weight"))?,
                q: lin(&mut w, "self_attn.q_proj")?,
                k: lin(&mut w, "self_attn.k_proj")?,
                v: lin(&mut w, "self_attn.v_proj")?,
                o: lin(&mut w, "self_attn.o_proj")?,
                qn: w(&format!("{b}.self_attn.q_norm.weight"))?,
                kn: w(&format!("{b}.self_attn.k_norm.weight"))?,
                puerta: lin(&mut w, "mlp.gate_proj")?,
                arriba: lin(&mut w, "mlp.up_proj")?,
                abajo: lin(&mut w, "mlp.down_proj")?,
            });
        }
        // 1 / base^(arange(0, hd, 2) / hd), en f32
        let ex = ops::divide(ops::arange::<_, f32>(0.0, hd as f32, 2.0).m()?, Array::from_f32(hd as f32)).m()?;
        let inv_frec = ops::divide(Array::from_f32(1.0), ops::power(Array::from_f32(theta), &ex).m()?).m()?;
        let norma = w(&format!("{raiz}.norm.weight"))?;
        let pos = ops::arange::<_, f32>(0.0, POS_MAX as f32, 1.0).m()?;
        let frec = ops::multiply(ops::expand_dims(&pos, 1).m()?, ops::expand_dims(&inv_frec, 0).m()?).m()?;
        let emb = ops::concatenate(&[frec.clone(), frec], -1).m()?;
        let cos = ops::cos(&emb).m()?.as_dtype(norma.dtype()).m()?;
        let sin = ops::sin(&emb).m()?.as_dtype(norma.dtype()).m()?;
        mlx_rs::transforms::eval([&cos, &sin]).m()?;
        Ok(Pila { capas, norma, nh, nkv, hd, eps, cos, sin })
    }

    fn pesos<'a>(&'a self, v: &mut Vec<&'a Array>) {
        for c in &self.capas {
            v.extend([&c.norma_in, &c.norma_post, &c.q.w_t, &c.k.w_t, &c.v.w_t, &c.o.w_t, &c.qn, &c.kn, &c.puerta.w_t, &c.arriba.w_t, &c.abajo.w_t]);
        }
        v.push(&self.norma);
    }

    fn cache(&self) -> CacheKv {
        CacheKv { capas: vec![Default::default(); self.capas.len()], largo: 0 }
    }

    /// [1, l, H] (desde la posición de la caché) → estado normalizado [1, l, H].
    fn adelante(&self, x: &Array, cache: &mut CacheKv, f: &Fusiones, giro: &RefCell<Giro>) -> Result<Array> {
        let l = x.dim(1);
        let desde = cache.largo;
        if desde + l > POS_MAX {
            bail_tr!("la voz pasó de {POS_MAX} posiciones (frase demasiado larga: pártela)", "the voice went past {POS_MAX} positions (sentence too long: split it)");
        }
        let tipo = x.dtype();
        let corte = |a: &Array| -> Result<Array> { ops::expand_dims(a.try_index(desde..desde + l).m()?.as_dtype(tipo).m()?, 0).m() };
        let (cos, sin) = (corte(&self.cos)?, corte(&self.sin)?);
        let (nh, nkv, hd) = (self.nh, self.nkv, self.hd);
        let mut h = x.clone();
        for (capa, cc) in self.capas.iter().zip(cache.capas.iter_mut()) {
            let y = fast::rms_norm(&h, Some(&capa.norma_in), self.eps).m()?;
            let forma = |a: Array, n: i32| -> Result<Array> { ops::reshape(&a, &[1, l, n, hd]).m() };
            let q = fast::rms_norm(forma(capa.q.aplicar(&y)?, nh)?, Some(&capa.qn), self.eps).m()?;
            let k = fast::rms_norm(forma(capa.k.aplicar(&y)?, nkv)?, Some(&capa.kn), self.eps).m()?;
            let v = forma(capa.v.aplicar(&y)?, nkv)?;
            let tr = |a: &Array| -> Result<Array> { ops::transpose_axes(a, &[0, 2, 1, 3]).m() };
            let q = (giro.borrow_mut())((&tr(&q)?, &cos, &sin)).m()?;
            let k = (giro.borrow_mut())((&tr(&k)?, &cos, &sin)).m()?;
            let (k, v) = crate::modelo::escribir_kv(cc, &k, &tr(&v)?, desde, l)?;
            let mascara = if l > 1 { Some(ScaledDotProductAttentionMask::Causal) } else { None };
            let a = fast::scaled_dot_product_attention(&q, &k, &v, (hd as f32).powf(-0.5), mascara, None::<&Array>).m()?;
            let a = ops::reshape(ops::transpose_axes(&a, &[0, 2, 1, 3]).m()?, &[1, l, nh * hd]).m()?;
            h = ops::add(&h, capa.o.aplicar(&a)?).m()?;
            let y = fast::rms_norm(&h, Some(&capa.norma_post), self.eps).m()?;
            let y = capa.abajo.aplicar(&(f.swiglu.borrow_mut())((&capa.puerta.aplicar(&y)?, &capa.arriba.aplicar(&y)?)).m()?)?;
            h = ops::add(&h, &y).m()?;
        }
        cache.largo += l;
        fast::rms_norm(&h, Some(&self.norma), self.eps).m()
    }
}

/// La voz de referencia ya codificada (se calcula una vez): códigos [16, T] y vector de hablante.
pub struct Referencia {
    pub codigos: Array,
    pub hablante: Array,
    pub texto: String,
}

impl Referencia {
    pub fn cargar(seguro: &Path, texto: &str) -> Result<Referencia> {
        let a = Array::load_safetensors(seguro).map_err(|e| anyhow!("{}: {e:?}", seguro.display()))?;
        Ok(Referencia { codigos: a.get("ref_codes").cloned().ok_or_else(|| anyhow_tr!("sin ref_codes", "no ref_codes"))?, hablante: a.get("speaker").cloned().ok_or_else(|| anyhow_tr!("sin speaker", "no speaker"))?, texto: texto.into() })
    }
}

/// Cómo elegir cada código.
#[derive(Clone, Copy, Debug)]
pub struct Muestreo {
    pub temperatura: f32,
    pub top_k: usize,
    pub penalizacion: f32,
}

impl Default for Muestreo {
    /// `generation_config.json` de Qwen3-TTS
    fn default() -> Self {
        Muestreo { temperatura: 0.9, top_k: 50, penalizacion: 1.05 }
    }
}

pub struct Voz {
    cfg: TalkerCfg,
    tok: tokenizers::Tokenizer,
    texto_emb: Array,
    proy_texto: (Lin, Lin),
    codec_emb: Array,
    cabeza: Lin,
    talker: Pila,
    pred_entrada: Lin,
    pred_emb: Vec<Array>,
    pred_cabezas: Vec<Lin>,
    pred: Pila,
    pub decodificador: Decodificador,
    ids: (i32, i32, i32),
    f: Fusiones,
    giro: RefCell<Giro>,
    clave: RefCell<Array>,
    /// los 1023 especiales del códec (todo [vocab-1024, vocab) salvo el EOS): no se pueden elegir
    suprimidos: Array,
}

impl Voz {
    pub fn cargar(dir: &Path) -> Result<Voz> {
        let cfg: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dir.join("config.json"))?)?;
        if cfg["model_type"].as_str() != Some("qwen3_tts") || cfg["tts_model_type"].as_str() != Some("base") {
            bail!("{} no es un Qwen3-TTS Base", dir.display());
        }
        let c: TalkerCfg = serde_json::from_value(cfg["talker_config"].clone())?;
        let ids = (cfg["tts_bos_token_id"].as_i64().unwrap_or(151672) as i32, cfg["tts_eos_token_id"].as_i64().unwrap_or(151673) as i32, cfg["tts_pad_token_id"].as_i64().unwrap_or(151671) as i32);
        let mut p = crate::modelo::cargar_pesos_de(dir, "talker.")?;
        let t = |a: Array| -> Result<Array> { ops::transpose_axes(&a, &[1, 0]).m() };
        let mut w = |k: &str| p.remove(k).ok_or_else(|| anyhow!("falta talker.{k}"));
        let texto_emb = w("model.text_embedding.weight")?;
        let codec_emb = w("model.codec_embedding.weight")?;
        let proy_texto = (
            Lin { w_t: t(w("text_projection.linear_fc1.weight")?)?, b: Some(w("text_projection.linear_fc1.bias")?) },
            Lin { w_t: t(w("text_projection.linear_fc2.weight")?)?, b: Some(w("text_projection.linear_fc2.bias")?) },
        );
        let cabeza = Lin { w_t: t(w("codec_head.weight")?)?, b: None };
        let pred_entrada = Lin { w_t: t(w("code_predictor.small_to_mtp_projection.weight")?)?, b: Some(w("code_predictor.small_to_mtp_projection.bias")?) };
        let n = c.num_code_groups - 1;
        let mut pred_emb = Vec::new();
        let mut pred_cabezas = Vec::new();
        for i in 0..n {
            pred_emb.push(w(&format!("code_predictor.model.codec_embedding.{i}.weight"))?);
            pred_cabezas.push(Lin { w_t: t(w(&format!("code_predictor.lm_head.{i}.weight"))?)?, b: None });
        }
        let talker = Pila::cargar(&mut p, "model", Forma { n: c.num_hidden_layers, nh: c.num_attention_heads, nkv: c.num_key_value_heads, hd: c.head_dim, eps: c.rms_norm_eps, theta: c.rope_theta })?;
        let pc = &c.code_predictor_config;
        let pred = Pila::cargar(&mut p, "code_predictor.model", Forma { n: pc.num_hidden_layers, nh: pc.num_attention_heads, nkv: pc.num_key_value_heads, hd: pc.head_dim, eps: pc.rms_norm_eps, theta: pc.rope_theta })?;
        let decodificador = Decodificador::cargar(dir)?;
        let semilla = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(7) | 1;
        let clave = mlx_rs::random::key(semilla).m()?;
        let v = c.vocab_size;
        let rango = ops::arange::<_, i32>(0, v, 1).m()?;
        let suprimidos = ops::logical_and(ops::ge(&rango, Array::from_int(v - 1024)).m()?, ops::ne(&rango, Array::from_int(c.codec_eos_token_id)).m()?).m()?;
        suprimidos.eval().m()?;
        // MLX lee los pesos al usarlos: se leen AQUÍ, para que la primera frase no pague el disco
        let mut v: Vec<&Array> = vec![&texto_emb, &codec_emb, &proy_texto.0.w_t, &proy_texto.1.w_t, &cabeza.w_t, &pred_entrada.w_t];
        v.extend(proy_texto.0.b.iter().chain(proy_texto.1.b.iter()).chain(pred_entrada.b.iter()));
        v.extend(pred_emb.iter().chain(pred_cabezas.iter().map(|l| &l.w_t)));
        talker.pesos(&mut v);
        pred.pesos(&mut v);
        mlx_rs::transforms::eval(v).m()?;
        decodificador.leer()?;
        Ok(Voz { clave: RefCell::new(clave), suprimidos, tok: tokenizador(dir)?, cfg: c, texto_emb, proy_texto, codec_emb, cabeza, talker, pred_entrada, pred_emb, pred_cabezas, pred, decodificador, ids, f: Fusiones::sin_compilar(), giro: giro_compilado() })
    }

    fn tokens(&self, s: &str) -> Result<Vec<u32>> {
        Ok(self.tok.encode(s, false).map_err(|e| anyhow_tr!("tokenizar: {e}", "tokenizing: {e}"))?.get_ids().to_vec())
    }

    /// text_projection(text_embedding(ids)): [1, n, H].
    fn texto(&self, ids: &[u32]) -> Result<Array> {
        let e = take_axis(&self.texto_emb, Array::from_slice(ids, &[1, ids.len() as i32]), 0).m()?;
        let h = (self.f.silu.borrow_mut())(&self.proy_texto.0.aplicar(&e)?).m()?;
        self.proy_texto.1.aplicar(&h)
    }

    fn codec(&self, ids: &[i32]) -> Result<Array> {
        take_axis(&self.codec_emb, Array::from_slice(ids, &[1, ids.len() as i32]), 0).m()
    }

    /// `_prepare_icl_generation_inputs`: el prompt de clonación [1, L, H].
    pub fn prompt(&self, texto: &str, r: &Referencia, idioma: &str) -> Result<Array> {
        let c = &self.cfg;
        let ref_ids = self.tokens(&format!("<|im_start|>assistant\n{}<|im_end|>\n", r.texto))?;
        let tgt = self.tokens(&format!("<|im_start|>assistant\n{texto}<|im_end|>\n<|im_start|>assistant\n"))?;
        if ref_ids.len() < 5 || tgt.len() < 8 {
            bail_tr!("texto vacío", "empty text");
        }
        let (ref_txt, txt) = (&ref_ids[3..ref_ids.len() - 2], &tgt[3..tgt.len() - 5]);
        let tts = self.texto(&[self.ids.0 as u32, self.ids.1 as u32, self.ids.2 as u32])?;
        let (bos, eos, pad) = (tts.try_index((.., 0..1, ..)).m()?, tts.try_index((.., 1..2, ..)).m()?, tts.try_index((.., 2..3, ..)).m()?);
        let todos: Vec<u32> = ref_txt.iter().chain(txt).copied().collect();
        let texto_e = ops::concatenate(&[self.texto(&todos)?, eos], 1).m()?;
        let lt = texto_e.dim(1);
        // códigos de la referencia: primer libro (códec del talker) + los 15 del predictor
        let cods = &r.codigos;
        let t = cods.dim(1);
        let fila = |i: i32| -> Result<Array> { ops::reshape(cods.try_index(i).m()?, &[1, t]).m() };
        let mut ref_e = take_axis(&self.codec_emb, fila(0)?, 0).m()?;
        for i in 0..(c.num_code_groups - 1) {
            ref_e = ops::add(&ref_e, take_axis(&self.pred_emb[i as usize], fila(i + 1)?, 0).m()?).m()?;
        }
        let codec_icl = ops::concatenate(&[self.codec(&[c.codec_bos_id])?, ref_e], 1).m()?;
        let lc = codec_icl.dim(1);
        let h = c.hidden_size;
        let codec_pad = self.codec(&[c.codec_pad_id])?;
        let texto_con_pad = ops::add(&texto_e, ops::broadcast_to(&codec_pad, &[1, lt, h]).m()?).m()?;
        let codec_con_pad = ops::add(&codec_icl, ops::broadcast_to(&pad, &[1, lc, h]).m()?).m()?;
        let icl = ops::concatenate(&[texto_con_pad, codec_con_pad], 1).m()?;
        let prefijo_ids: Vec<i32> = match c.codec_language_id.get(&idioma.to_lowercase()) {
            Some(&l) => vec![c.codec_think_id, c.codec_think_bos_id, l, c.codec_think_eos_id],
            None => vec![c.codec_nothink_id, c.codec_think_bos_id, c.codec_think_eos_id],
        };
        let prefijo = self.codec(&prefijo_ids)?;
        let hablante = ops::reshape(r.hablante.as_dtype(prefijo.dtype()).m()?, &[1, 1, -1]).m()?;
        let sufijo = self.codec(&[c.codec_pad_id, c.codec_bos_id])?;
        let codec_pre = ops::concatenate(&[prefijo, hablante, sufijo], 1).m()?;
        let n = codec_pre.dim(1);
        let rol = self.texto(&tgt[..3])?;
        let pads = ops::broadcast_to(&pad, &[1, n - 2, h]).m()?;
        let combinado = ops::add(ops::concatenate(&[pads, bos], 1).m()?, codec_pre.try_index((.., ..n - 1, ..)).m()?).m()?;
        ops::concatenate(&[rol, combinado, icl], 1).m()
    }

    /// El PAD de texto (lo que acompaña a cada fotograma generado en la clonación), proyectado JUNTO
    /// a BOS y EOS como en mlx-audio: la misma multiplicación, los mismos redondeos.
    fn pad_texto(&self) -> Result<Array> {
        let tts = self.texto(&[self.ids.0 as u32, self.ids.1 as u32, self.ids.2 as u32])?;
        tts.try_index((.., 2..3, ..)).m()
    }

    /// El código 0 en la GPU, como `_sample_token` de mlx-audio: suprime los especiales del códec,
    /// penaliza los últimos 64 códigos (en bf16, el tipo de los logits) y elige. Sin sincronizar.
    fn elegir_t0(&self, l: &Array, previos: &[u32], m: Muestreo) -> Result<Array> {
        let tipo = l.dtype();
        let mut l = ops::select(&self.suprimidos, Array::from_f32(f32::NEG_INFINITY).as_dtype(tipo).m()?, l).m()?;
        if m.penalizacion != 1.0 && !previos.is_empty() {
            let mut vistos: Vec<i32> = previos[previos.len().saturating_sub(64)..].iter().map(|&t| t as i32).collect();
            vistos.sort_unstable();
            vistos.dedup();
            let ids = Array::from_slice(&vistos, &[vistos.len() as i32]);
            let sel = take_axis(&l, &ids, -1).m()?;
            let p = Array::from_f32(m.penalizacion).as_dtype(tipo).m()?;
            let pen = ops::select(ops::lt(&sel, Array::from_f32(0.0).as_dtype(tipo).m()?).m()?, ops::multiply(&sel, &p).m()?, ops::divide(&sel, &p).m()?).m()?;
            l = mlx_rs::ops::indexing::put_along_axis(&l, &ids, &pen, -1).m()?;
        }
        self.elegir_gpu(&l, m)
    }

    /// Elige un código en la GPU, SIN sincronizar (el predictor encadena 15 por fotograma): voraz =
    /// `argmax`; si no, top-k + `categorical` con la clave del motor. Devuelve un índice [1, 1].
    fn elegir_gpu(&self, l: &Array, m: Muestreo) -> Result<Array> {
        if m.temperatura <= 0.0 {
            return ops::reshape(argmax_axis(l, -1, false).m()?, &[1, 1]).m();
        }
        let x = ops::divide(l.as_dtype(Dtype::Float32).m()?, Array::from_f32(m.temperatura)).m()?;
        let k = (m.top_k.max(1) as i32).min(x.dim(-1));
        // umbral = el k-ésimo mayor: lo que queda debajo no se puede elegir
        let neg = ops::negative(&x).m()?;
        let idx = mlx_rs::ops::argpartition_axis(&neg, k - 1, -1).m()?;
        let top = take_axis(&x, idx.try_index(..k).m()?, -1).m()?;
        let umbral = top.min(None).m()?;
        let filtrado = ops::select(ops::ge(&x, &umbral).m()?, &x, Array::from_f32(f32::NEG_INFINITY)).m()?;
        let (k1, k2) = mlx_rs::random::split(&*self.clave.borrow(), 2).m()?;
        *self.clave.borrow_mut() = k1;
        ops::reshape(mlx_rs::random::categorical(&filtrado, -1, None, &k2).m()?, &[1, 1]).m()
    }

    /// Genera los códigos [N, 16] de un texto con la voz de referencia. `cada` recibe cada fotograma.
    pub fn codigos(&self, texto: &str, r: &Referencia, idioma: &str, m: Muestreo, max: usize, cada: &mut dyn FnMut(&[u32]) -> Result<()>) -> Result<Vec<[u32; 16]>> {
        let c = self.cfg.clone();
        let mut x = self.prompt(texto, r, idioma)?;
        let pad = self.pad_texto()?;
        let mut cache = self.talker.cache();
        let eos = c.codec_eos_token_id as usize;
        let mut previos: Vec<u32> = Vec::new();
        let mut salida = Vec::new();
        let perfil = std::env::var_os("GOTV_VOZ_PERFIL").is_some();
        let (mut t_talker, mut t_pred) = (std::time::Duration::ZERO, std::time::Duration::ZERO);
        for _ in 0..max {
            let t_ini = std::time::Instant::now();
            let h = self.talker.adelante(&x, &mut cache, &self.f, &self.giro)?;
            let ultimo = h.try_index((.., -1.., ..)).m()?;
            let logits = ops::reshape(self.cabeza.aplicar(&ultimo)?, &[-1]).m()?;
            let t0 = self.elegir_t0(&logits, &previos, m)?;
            let e0 = take_axis(&self.codec_emb, &t0, 0).m()?;
            let t_medio = std::time::Instant::now();
            t_talker += t_medio - t_ini;
            // el predictor: [estado del talker, emb(código 0)] y luego cada código, todo en la GPU; el
            // EOS se mira DESPUÉS, en la única sincronización del fotograma (como mlx-audio)
            let mut pc = self.pred.cache();
            let mut entrada = ops::concatenate(&[ultimo.clone(), e0.clone()], 1).m()?;
            let mut indices: Vec<Array> = Vec::with_capacity(15);
            for i in 0..(c.num_code_groups - 1) as usize {
                let e = self.pred_entrada.aplicar(&entrada)?;
                let hp = self.pred.adelante(&e, &mut pc, &self.f, &self.giro)?;
                let lg = ops::reshape(self.pred_cabezas[i].aplicar(&hp.try_index((.., -1.., ..)).m()?)?, &[-1]).m()?;
                let ci = self.elegir_gpu(&lg, m)?;
                entrada = take_axis(&self.pred_emb[i], &ci, 0).m()?;
                indices.push(ci);
            }
            // la entrada siguiente: PAD de texto + suma de los 16 embeddings (en el orden de mlx-audio)
            let mut ce = e0;
            for (i, ci) in indices.iter().enumerate() {
                ce = ops::add(&ce, take_axis(&self.pred_emb[i], ci, 0).m()?).m()?;
            }
            x = ops::add(&pad, &ce).m()?;
            indices.insert(0, t0);
            let todos = ops::concatenate(&indices, 0).m()?.as_dtype(Dtype::Uint32).m()?;
            mlx_rs::transforms::eval([&x, &todos]).m()?;
            let mut fot = [0u32; 16];
            fot.copy_from_slice(&todos.as_slice::<u32>()[..16]);
            t_pred += t_medio.elapsed();
            if fot[0] as usize == eos {
                break;
            }
            let t0 = fot[0];
            previos.push(t0);
            cada(&fot)?;
            salida.push(fot);
        }
        if perfil {
            let n = salida.len().max(1) as f64;
            eprintln!("{}", tr!("perfil voz: armar el talker {:.1} ms/fotograma · predictor + la sincronización {:.1} ms/fotograma", "voice profile: building the talker {:.1} ms/frame · predictor + sync {:.1} ms/frame", t_talker.as_secs_f64() * 1e3 / n, t_pred.as_secs_f64() * 1e3 / n));
        }
        Ok(salida)
    }

    /// Texto → onda de 24 kHz con la voz de referencia, POR TROZOS mientras se genera: `cada` recibe
    /// cada trozo de onda en cuanto existe (el primero, corto, para que la voz arranque pronto).
    /// El decodificador se ceba con la cola de la referencia (`streaming_context_size` = 25 de
    /// mlx-audio) para que el primer trozo tenga el mismo contexto acústico que el resto.
    /// `seguir` se consulta en CADA fotograma: si devuelve `false`, para ya (la interrupción).
    pub fn decir_en_trozos(&self, texto: &str, r: &Referencia, idioma: &str, m: Muestreo, seguir: &dyn Fn() -> bool, cada: &mut dyn FnMut(&[f32]) -> Result<()>) -> Result<usize> {
        let dec = &self.decodificador;
        let mut fl = Decodificador::flujo();
        let t = r.codigos.dim(1);
        if t > 0 {
            let cola = ops::transpose_axes(r.codigos.try_index((.., (t - CONTEXTO).max(0)..)).m()?, &[1, 0]).m()?;
            dec.paso(&mut fl, &cola.as_dtype(Dtype::Int32).m()?)?.eval().m()?; // es la referencia: se descarta
        }
        let mut pendientes: Vec<u32> = Vec::new();
        let mut nro = 0usize;
        let mut soltar = |pend: &mut Vec<u32>, fl: &mut crate::voz_decodificador::Flujo| -> Result<()> {
            let n = pend.len() / 16;
            let a = Array::from_slice(pend, &[n as i32, 16]).as_dtype(Dtype::Int32).m()?;
            let w = dec.paso(fl, &a)?;
            w.eval().m()?;
            pend.clear();
            cada(w.as_slice::<f32>())
        };
        let cods = self.codigos(texto, r, idioma, m, MAX_FOTOGRAMAS, &mut |f| {
            if !seguir() {
                bail!("callada");
            }
            pendientes.extend_from_slice(f);
            if pendientes.len() / 16 >= TROZOS[nro.min(TROZOS.len() - 1)] {
                nro += 1;
                soltar(&mut pendientes, &mut fl)?;
            }
            Ok(())
        })?;
        if !pendientes.is_empty() {
            soltar(&mut pendientes, &mut fl)?;
        }
        Ok(cods.len())
    }

    /// Texto → onda de 24 kHz entera.
    pub fn decir(&self, texto: &str, r: &Referencia, idioma: &str, m: Muestreo) -> Result<Vec<f32>> {
        let mut onda = Vec::new();
        self.decir_en_trozos(texto, r, idioma, m, &|| true, &mut |w| {
            onda.extend_from_slice(w);
            Ok(())
        })?;
        Ok(onda)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir() -> Option<std::path::PathBuf> {
        std::env::var("GOTV_TTS_DIR").ok().map(Into::into)
    }


    /// El tokenizador armado en Rust da los MISMOS ids que el de HF (`ref_qwen3tts.py` los guarda).
    #[test]
    #[ignore]
    fn tokeniza_como_hf() {
        let (Some(d), Ok(r)) = (dir(), std::env::var("GOTV_REF_TTS")) else { return };
        let rj: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(Path::new(&r).join("ref.json")).unwrap()).unwrap();
        let t = tokenizador(&d).unwrap();
        let ids = |s: &str| t.encode(s, false).unwrap().get_ids().to_vec();
        let num = |v: &serde_json::Value| -> Vec<u32> { v.as_array().unwrap().iter().map(|x| x.as_u64().unwrap() as u32).collect() };
        assert_eq!(ids(&format!("<|im_start|>assistant\n{}<|im_end|>\n", rj["ref_text"].as_str().unwrap())), num(&rj["ids_ref"]));
        assert_eq!(ids(&format!("<|im_start|>assistant\n{}<|im_end|>\n<|im_start|>assistant\n", rj["texto"].as_str().unwrap())), num(&rj["ids_tgt"]));
        for (s, v) in rj["ids_extra"].as_object().unwrap() {
            assert_eq!(ids(s), num(v), "«{s}»");
        }
        println!("tokenizador idéntico a HF en {} textos", 2 + rj["ids_extra"].as_object().unwrap().len());
    }

    /// Juez contra mlx-audio (`ref_qwen3tts.py`): el prompt, los logits del primer paso y los
    /// fotogramas voraces.
    /// ¿Se frena la voz de una frase a la siguiente? `GOTV_TTS_DIR=… cargo test -p gotv-mlx --release -- --ignored --nocapture frase_tras_frase`
    #[test]
    #[ignore]
    fn frase_tras_frase() {
        let Some(d) = dir() else { return };
        let r_dir = std::env::var("GOTV_REF_TTS").map(std::path::PathBuf::from).unwrap();
        let r = Referencia::cargar(&r_dir.join("voz-gotv.safetensors"), "Hola, Portador. Ya terminé la compilación: pasan trescientas cuarenta y siete pruebas y no hay ninguna advertencia.").unwrap();
        let m = Muestreo { temperatura: 0.0, ..Default::default() };
        let v = Voz::cargar(&d).unwrap();
        let frase = "Entiendo, fue un momento difícil. Me alegra mucho que estés bien después del sismo, Portador.";
        for i in 1..=3 {
            let t = std::time::Instant::now();
            let n = mlx_rs::with_device(mlx_rs::Device::gpu(), || v.codigos(frase, &r, "spanish", m, 400, &mut |_| Ok(()))).unwrap().len();
            println!("{i} · sólo códigos: {:.1} fotogramas/s", n as f64 / t.elapsed().as_secs_f64());
        }
        mlx_rs::transforms::compile::clear_cache();
        let t = std::time::Instant::now();
        let n = mlx_rs::with_device(mlx_rs::Device::gpu(), || v.codigos(frase, &r, "spanish", m, 400, &mut |_| Ok(()))).unwrap().len();
        println!("4 · tras limpiar la caché de compilación: {:.1} fotogramas/s", n as f64 / t.elapsed().as_secs_f64());
        let _ = mlx_rs::memory::clear_cache();
        let t = std::time::Instant::now();
        let n = mlx_rs::with_device(mlx_rs::Device::gpu(), || v.codigos(frase, &r, "spanish", m, 400, &mut |_| Ok(()))).unwrap().len();
        println!("5 · tras vaciar la caché de memoria: {:.1} fotogramas/s", n as f64 / t.elapsed().as_secs_f64());
    }

    #[test]
    #[ignore]
    fn habla_como_mlx_audio() {
        let (Some(d), Ok(r)) = (dir(), std::env::var("GOTV_REF_TTS")) else { return };
        let rd = Path::new(&r);
        let rj: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(rd.join("ref.json")).unwrap()).unwrap();
        let rs = Array::load_safetensors(rd.join("ref.safetensors")).unwrap();
        let t = std::time::Instant::now();
        let v = Voz::cargar(&d).unwrap();
        let refe = Referencia::cargar(&rd.join("voz-gotv.safetensors"), rj["ref_text"].as_str().unwrap()).unwrap();
        println!("carga {:?}", t.elapsed());
        let cmp = |a: &Array, b: &Array| -> (f32, usize, usize) {
            let a = a.as_dtype(Dtype::Float32).unwrap();
            let (x, y) = (a.as_slice::<f32>(), b.as_slice::<f32>());
            (x.iter().zip(y).map(|(p, q)| (p - q).abs()).fold(0.0, f32::max), x.iter().zip(y).filter(|(p, q)| p == q).count(), x.len())
        };
        let p = v.prompt(rj["texto"].as_str().unwrap(), &refe, "spanish").unwrap();
        assert_eq!(p.dim(1) as u64, rj["prompt_largo"].as_u64().unwrap(), "largo del prompt");
        let (d1, i1, n1) = cmp(&ops::reshape(&p, &[p.dim(1), -1]).unwrap(), &rs["input_embeds"]);
        println!("1 · prompt {} posiciones · dif. máx {d1} · iguales {i1}/{n1}", p.dim(1));
        let mut cache = v.talker.cache();
        let h = v.talker.adelante(&p, &mut cache, &v.f, &v.giro).unwrap();
        let ult = h.try_index((.., -1.., ..)).unwrap();
        let lg = ops::reshape(v.cabeza.aplicar(&ult).unwrap(), &[-1]).unwrap();
        let (d2, i2, n2) = cmp(&lg, &rs["logits0"]);
        let (d3, _, _) = cmp(&ops::reshape(&ult, &[-1]).unwrap(), &rs["hidden0"]);
        println!("2 · logits del paso 0: dif. máx {d2} · iguales {i2}/{n2} · estado dif. {d3}");
        let refc = &rs["codes"];
        let n = refc.dim(0) as usize;
        let t = std::time::Instant::now();
        let cods = v.codigos(rj["texto"].as_str().unwrap(), &refe, "spanish", Muestreo { temperatura: 0.0, ..Default::default() }, n, &mut |_| Ok(())).unwrap();
        let s = t.elapsed();
        let refv: Vec<i32> = refc.as_slice::<i32>().to_vec();
        let iguales = cods.iter().enumerate().take_while(|(i, f)| f.iter().zip(&refv[i * 16..i * 16 + 16]).all(|(a, b)| *a as i32 == *b)).count();
        if iguales < n {
            println!("   fotograma {iguales}: propio {:?}\n               ref    {:?}", cods.get(iguales), &refv[iguales * 16..iguales * 16 + 16]);
        }
        println!("3 · voraz: {iguales}/{n} fotogramas idénticos en {s:?} ({:.1} fotogramas/s, el habla va a 12,5)", cods.len() as f64 / s.as_secs_f64());
        assert!(iguales >= n.min(20), "los fotogramas voraces se separan de mlx-audio en el {iguales}");
    }
}
