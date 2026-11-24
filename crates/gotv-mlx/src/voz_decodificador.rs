//! M2 · el decodificador de audio de Qwen3-TTS (12 Hz) en el núcleo: 16 códigos por fotograma →
//! onda de 24 kHz. Port de `mlx_audio/tts/models/qwen3_tts/speech_tokenizer.py` (sólo la parte que
//! DECODIFICA), todo en f32 como allá:
//!
//! descuantizar (suma de 1 + 15 tablas → proyecciones 1×1) → conv causal k3 → transformador de 8
//! capas (RMSNorm en f32, RoPE con `rotate_half`, «layer scale») → 2 × [conv transpuesta ×2 +
//! ConvNeXt] → conv k7 → 4 bloques [SnakeBeta → conv transpuesta ×r → 3 residuales (dilatación
//! 1, 3, 9)] → SnakeBeta → conv k7 → recorte a [−1, 1].
//!
//! **Por trozos** (`flujo` + `paso`, el `streaming_step` de mlx-audio): cada conv causal guarda su
//! cola, las transpuestas de los bloques acarrean el solape (sin sesgo) y el transformador su caché
//! K/V. Decodificar entero ES un solo paso desde un flujo nuevo: un camino, un juez.
//!
//! Los pesos vienen en formato PyTorch; se transponen con LA MISMA regla que `sanitize` de mlx-audio
//! (`check_array_shape_qwen3`), y los libros de códigos salen de `embedding_sum / max(uso, 1e-5)`.

use crate::modelo::{Fusiones, M};
use anyhow::{anyhow, Result};
use mlx_rs::fast::{self, ScaledDotProductAttentionMask};
use mlx_rs::ops::indexing::{take_axis, TryIndexOp};
use mlx_rs::ops;
use mlx_rs::Array;
use std::collections::HashMap;
use std::path::Path;

/// `check_array_shape_qwen3`: ¿el peso de conv ya está en formato MLX [sal, k, ent]?
fn ya_mlx(forma: &[i32]) -> bool {
    if forma.len() != 3 {
        return false;
    }
    let (d2, d3) = (forma[1], forma[2]);
    if d2 == 1 {
        d3 > 64
    } else if d3 == 1 {
        d2 <= 64
    } else {
        d2 < d3
    }
}

#[derive(Clone, Debug, serde::Deserialize)]
pub struct DecConfig {
    pub latent_dim: i32,
    pub codebook_dim: i32,
    pub decoder_dim: i32,
    pub hidden_size: i32,
    pub head_dim: i32,
    pub num_attention_heads: i32,
    pub num_key_value_heads: i32,
    pub num_hidden_layers: i32,
    pub num_quantizers: i32,
    pub num_semantic_quantizers: i32,
    pub rms_norm_eps: f32,
    pub rope_theta: f32,
    pub upsample_rates: Vec<i32>,
    pub upsampling_ratios: Vec<i32>,
}

/// Conv 1D (causal o no) con sesgo: `mx.conv1d` + sesgo, como `nn.Conv1d`.
struct Conv {
    w: Array,
    b: Option<Array>,
    relleno: i32,
    dilatacion: i32,
    grupos: i32,
}

/// Estado de una decodificación por trozos: colas de las convs (en el orden en que se aplican),
/// solapes de las transpuestas y la caché del transformador.
pub struct Flujo {
    colas: Vec<Option<Array>>,
    solapes: Vec<Option<Array>>,
    kv: Vec<crate::modelo::CacheCapa>,
    largo: i32,
    i_cola: usize,
    i_solape: usize,
}

impl Flujo {
    fn cola(&mut self) -> &mut Option<Array> {
        if self.i_cola == self.colas.len() {
            self.colas.push(None);
        }
        self.i_cola += 1;
        &mut self.colas[self.i_cola - 1]
    }
    fn solape(&mut self) -> &mut Option<Array> {
        if self.i_solape == self.solapes.len() {
            self.solapes.push(None);
        }
        self.i_solape += 1;
        &mut self.solapes[self.i_solape - 1]
    }
}

impl Conv {
    /// `step` de `CausalConv1d`: la cola del trozo anterior (o ceros al empezar) delante.
    fn aplicar(&self, x: &Array, f: &mut Flujo) -> Result<Array> {
        let x = if self.relleno > 0 {
            let cola = f.cola();
            let x = match cola.take() {
                Some(c) => ops::concatenate(&[c, x.clone()], 1).m()?,
                None => ops::pad(x, &[(0, 0), (self.relleno, 0), (0, 0)], None, None).m()?,
            };
            *cola = Some(x.try_index((.., -self.relleno.., ..)).m()?);
            x
        } else {
            x.clone()
        };
        let y = ops::conv1d(&x, &self.w, 1, 0, self.dilatacion, self.grupos).m()?;
        match &self.b {
            Some(b) => ops::add(&y, b).m(),
            None => Ok(y),
        }
    }
}

/// Conv transpuesta con sesgo y recorte por la derecha (causal).
struct ConvT {
    w: Array,
    b: Array,
    paso: i32,
    recorte: i32,
}

impl ConvT {
    /// `step` de `DecoderBlockUpsample`: suma el solape del trozo anterior y guarda la cola (sin su
    /// sesgo: la cabeza del trozo siguiente ya lo trae).
    fn aplicar(&self, x: &Array, f: &mut Flujo) -> Result<Array> {
        let mut y = ops::add(ops::conv_transpose1d(x, &self.w, self.paso, 0, 1, 0, 1).m()?, &self.b).m()?;
        if self.recorte == 0 {
            return Ok(y);
        }
        let solape = f.solape();
        if let Some(o) = solape.take() {
            let n = o.dim(1);
            y = ops::concatenate(&[ops::add(y.try_index((.., ..n, ..)).m()?, &o).m()?, y.try_index((.., n.., ..)).m()?], 1).m()?;
        }
        let t = y.dim(1);
        *solape = Some(ops::subtract(y.try_index((.., t - self.recorte.., ..)).m()?, &self.b).m()?);
        y.try_index((.., ..t - self.recorte, ..)).m()
    }
}

/// `nn.Linear` con sesgo = `addmm`; sin sesgo, `x · Wᵀ`.
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

/// SnakeBeta: x + 1/(e^β + 1e-9) · sin(x · e^α)².
struct Snake {
    alfa: Array,
    inv_beta: Array,
}

impl Snake {
    fn nuevo(alfa: Array, beta: Array) -> Result<Snake> {
        let alfa = ops::exp(&alfa).m()?;
        let inv_beta = ops::divide(Array::from_f32(1.0), ops::add(ops::exp(&beta).m()?, Array::from_f32(1e-9)).m()?).m()?;
        Ok(Snake { alfa, inv_beta })
    }
    fn aplicar(&self, x: &Array) -> Result<Array> {
        let s = ops::power(ops::sin(ops::multiply(x, &self.alfa).m()?).m()?, Array::from_f32(2.0)).m()?;
        ops::add(x, ops::multiply(&self.inv_beta, &s).m()?).m()
    }
}

struct Residual {
    a1: Snake,
    c1: Conv,
    a2: Snake,
    c2: Conv,
}

struct Bloque {
    snake: Snake,
    sube: ConvT,
    residuales: Vec<Residual>,
}

struct ConvNext {
    dw: Conv,
    norma: (Array, Array),
    pw1: Lin,
    pw2: Lin,
    gamma: Array,
}

struct CapaT {
    norma_in: Array,
    norma_post: Array,
    q: Lin,
    k: Lin,
    v: Lin,
    o: Lin,
    escala_at: Array,
    puerta: Lin,
    arriba: Lin,
    abajo: Lin,
    escala_mlp: Array,
}

pub struct Decodificador {
    pub c: DecConfig,
    /// tablas de la descuantización: la primera (semántica) y las 15 acústicas [2048, 256]
    tablas: Vec<Array>,
    proy_primera: Array,
    proy_resto: Array,
    pre_conv: Conv,
    entrada: Lin,
    capas: Vec<CapaT>,
    norma: Array,
    salida: Lin,
    inv_frec: Array,
    sube: Vec<(ConvT, ConvNext)>,
    conv0: Conv,
    bloques: Vec<Bloque>,
    snake_fin: Snake,
    conv_fin: Conv,
    f: Fusiones,
}

impl Decodificador {
    /// Carga `speech_tokenizer/` (config + pesos) de la carpeta del modelo Qwen3-TTS.
    pub fn cargar(dir: &Path) -> Result<Decodificador> {
        let cfg: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dir.join("speech_tokenizer/config.json"))?)?;
        let c: DecConfig = serde_json::from_value(cfg["decoder_config"].clone())?;
        let mut p: HashMap<String, Array> = HashMap::new();
        for e in std::fs::read_dir(dir.join("speech_tokenizer"))?.filter_map(|e| e.ok()) {
            if e.path().extension().is_some_and(|x| x == "safetensors") {
                for (k, v) in Array::load_safetensors(e.path()).map_err(|e| anyhow!("{e:?}"))? {
                    if let Some(k) = k.strip_prefix("decoder.") {
                        p.insert(k.to_string(), v);
                    }
                }
            }
        }
        let mut w = |n: &str| p.remove(n).ok_or_else(|| anyhow!("falta decoder.{n}"));
        // la regla de `sanitize`: transpuestas (1,2,0), convs y proyecciones (0,2,1), salvo si ya están
        let conv_w = |a: Array| -> Result<Array> { if ya_mlx(a.shape()) { Ok(a) } else { ops::transpose_axes(&a, &[0, 2, 1]).m() } };
        let convt_w = |a: Array| -> Result<Array> { if ya_mlx(a.shape()) { Ok(a) } else { ops::transpose_axes(&a, &[1, 2, 0]).m() } };
        let t2 = |a: Array| -> Result<Array> { ops::transpose_axes(&a, &[1, 0]).m() };
        // libros de códigos: embedding_sum / max(uso, 1e-5)
        let mut tablas = Vec::new();
        for (raiz, n) in [("rvq_first", c.num_semantic_quantizers), ("rvq_rest", c.num_quantizers - c.num_semantic_quantizers)] {
            for i in 0..n {
                let base = format!("quantizer.{raiz}.vq.layers.{i}._codebook");
                let uso = w(&format!("{base}.cluster_usage"))?;
                let suma = w(&format!("{base}.embedding_sum"))?;
                let uso = ops::clip(ops::expand_dims(&uso, 1).m()?, (&Array::from_f32(1e-5), ())).m()?;
                tablas.push(ops::divide(&suma, &uso).m()?);
            }
        }
        let proy_primera = conv_w(w("quantizer.rvq_first.output_proj.weight")?)?;
        let proy_resto = conv_w(w("quantizer.rvq_rest.output_proj.weight")?)?;
        let conv = |w: &mut dyn FnMut(&str) -> Result<Array>, n: &str, relleno: i32, dil: i32, grupos: i32| -> Result<Conv> {
            Ok(Conv { w: conv_w(w(&format!("{n}.weight"))?)?, b: Some(w(&format!("{n}.bias"))?), relleno, dilatacion: dil, grupos })
        };
        let lin = |w: &mut dyn FnMut(&str) -> Result<Array>, n: &str, sesgo: bool| -> Result<Lin> {
            Ok(Lin { w_t: t2(w(&format!("{n}.weight"))?)?, b: if sesgo { Some(w(&format!("{n}.bias"))?) } else { None } })
        };
        let pre_conv = conv(&mut w, "pre_conv.conv", 2, 1, 1)?;
        let entrada = lin(&mut w, "pre_transformer.input_proj", true)?;
        let mut capas = Vec::new();
        for i in 0..c.num_hidden_layers {
            let n = format!("pre_transformer.layers.{i}");
            capas.push(CapaT {
                norma_in: w(&format!("{n}.input_layernorm.weight"))?,
                norma_post: w(&format!("{n}.post_attention_layernorm.weight"))?,
                q: lin(&mut w, &format!("{n}.self_attn.q_proj"), false)?,
                k: lin(&mut w, &format!("{n}.self_attn.k_proj"), false)?,
                v: lin(&mut w, &format!("{n}.self_attn.v_proj"), false)?,
                o: lin(&mut w, &format!("{n}.self_attn.o_proj"), false)?,
                escala_at: w(&format!("{n}.self_attn_layer_scale.scale"))?,
                puerta: lin(&mut w, &format!("{n}.mlp.gate_proj"), false)?,
                arriba: lin(&mut w, &format!("{n}.mlp.up_proj"), false)?,
                abajo: lin(&mut w, &format!("{n}.mlp.down_proj"), false)?,
                escala_mlp: w(&format!("{n}.mlp_layer_scale.scale"))?,
            });
        }
        let norma = w("pre_transformer.norm.weight")?;
        let salida = lin(&mut w, "pre_transformer.output_proj", true)?;
        let mut sube = Vec::new();
        for (i, &f) in c.upsampling_ratios.iter().enumerate() {
            let n = format!("upsample.{i}");
            let ct = ConvT { w: convt_w(w(&format!("{n}.0.conv.weight"))?)?, b: w(&format!("{n}.0.conv.bias"))?, paso: f, recorte: 0 };
            let cn = ConvNext {
                dw: conv(&mut w, &format!("{n}.1.dwconv.conv"), 6, 1, c.latent_dim)?,
                norma: (w(&format!("{n}.1.norm.weight"))?, w(&format!("{n}.1.norm.bias"))?),
                pw1: lin(&mut w, &format!("{n}.1.pwconv1"), true)?,
                pw2: lin(&mut w, &format!("{n}.1.pwconv2"), true)?,
                gamma: w(&format!("{n}.1.gamma"))?,
            };
            sube.push((ct, cn));
        }
        let conv0 = conv(&mut w, "decoder.0.conv", 6, 1, 1)?;
        let mut bloques = Vec::new();
        for (i, &r) in c.upsample_rates.iter().enumerate() {
            let n = format!("decoder.{}.block", i + 1);
            let snake = Snake::nuevo(w(&format!("{n}.0.alpha"))?, w(&format!("{n}.0.beta"))?)?;
            let sube = ConvT { w: convt_w(w(&format!("{n}.1.conv.weight"))?)?, b: w(&format!("{n}.1.conv.bias"))?, paso: r, recorte: r };
            let mut residuales = Vec::new();
            for (j, d) in [(2, 1), (3, 3), (4, 9)] {
                let m = format!("{n}.{j}");
                residuales.push(Residual {
                    a1: Snake::nuevo(w(&format!("{m}.act1.alpha"))?, w(&format!("{m}.act1.beta"))?)?,
                    c1: conv(&mut w, &format!("{m}.conv1.conv"), 6 * d, d, 1)?,
                    a2: Snake::nuevo(w(&format!("{m}.act2.alpha"))?, w(&format!("{m}.act2.beta"))?)?,
                    c2: conv(&mut w, &format!("{m}.conv2.conv"), 0, 1, 1)?,
                });
            }
            bloques.push(Bloque { snake, sube, residuales });
        }
        let ultimo = c.upsample_rates.len() + 1;
        let snake_fin = Snake::nuevo(w(&format!("decoder.{ultimo}.alpha"))?, w(&format!("decoder.{ultimo}.beta"))?)?;
        let conv_fin = conv(&mut w, &format!("decoder.{}.conv", ultimo + 1), 6, 1, 1)?;
        let hd = c.head_dim;
        let ex = ops::divide(ops::arange::<_, f32>(0.0, hd as f32, 2.0).m()?, Array::from_f32(hd as f32)).m()?;
        let inv_frec = ops::divide(Array::from_f32(1.0), ops::power(Array::from_f32(c.rope_theta), &ex).m()?).m()?;
        Ok(Decodificador { c, tablas, proy_primera, proy_resto, pre_conv, entrada, capas, norma, salida, inv_frec, sube, conv0, bloques, snake_fin, conv_fin, f: Fusiones::sin_compilar() })
    }

    /// Lee del disco TODOS los pesos (MLX los carga perezosos).
    pub fn leer(&self) -> Result<()> {
        let mut v: Vec<&Array> = self.tablas.iter().collect();
        v.extend([&self.proy_primera, &self.proy_resto, &self.norma, &self.inv_frec]);
        let conv = |c: &'_ Conv| -> Vec<Array> { std::iter::once(c.w.clone()).chain(c.b.clone()).collect() };
        let mut extra: Vec<Array> = Vec::new();
        extra.extend(conv(&self.pre_conv));
        extra.extend(conv(&self.conv0));
        extra.extend(conv(&self.conv_fin));
        for l in [&self.entrada, &self.salida] {
            extra.push(l.w_t.clone());
            extra.extend(l.b.clone());
        }
        for c in &self.capas {
            extra.extend([&c.norma_in, &c.norma_post, &c.q.w_t, &c.k.w_t, &c.v.w_t, &c.o.w_t, &c.escala_at, &c.puerta.w_t, &c.arriba.w_t, &c.abajo.w_t, &c.escala_mlp].map(|a| a.clone()));
        }
        for (ct, cn) in &self.sube {
            extra.extend([ct.w.clone(), ct.b.clone(), cn.norma.0.clone(), cn.norma.1.clone(), cn.pw1.w_t.clone(), cn.pw2.w_t.clone(), cn.gamma.clone()]);
            extra.extend(conv(&cn.dw));
            extra.extend(cn.pw1.b.clone().into_iter().chain(cn.pw2.b.clone()));
        }
        for b in &self.bloques {
            extra.extend([b.snake.alfa.clone(), b.snake.inv_beta.clone(), b.sube.w.clone(), b.sube.b.clone()]);
            for r in &b.residuales {
                extra.extend([r.a1.alfa.clone(), r.a1.inv_beta.clone(), r.a2.alfa.clone(), r.a2.inv_beta.clone()]);
                extra.extend(conv(&r.c1));
                extra.extend(conv(&r.c2));
            }
        }
        extra.extend([self.snake_fin.alfa.clone(), self.snake_fin.inv_beta.clone()]);
        v.extend(extra.iter());
        mlx_rs::transforms::eval(v).m()
    }

    /// RMSNorm del decodificador: en f32, `peso · x̂` (el orden de mlx-audio).
    fn rms(x: &Array, peso: &Array, eps: f32) -> Result<Array> {
        let var = ops::power(x, Array::from_f32(2.0)).m()?.mean_axis(-1, true).m()?;
        let n = ops::multiply(x, ops::rsqrt(ops::add(&var, Array::from_f32(eps)).m()?).m()?).m()?;
        ops::multiply(peso, &n).m()
    }

    /// `rotate_half` + `q·cos + rot(q)·sin`.
    fn girar(x: &Array, cos: &Array, sin: &Array) -> Result<Array> {
        let m = ops::split_equal(x, 2, -1).m()?;
        let rot = ops::concatenate(&[ops::negative(&m[1]).m()?, m[0].clone()], -1).m()?;
        ops::add(ops::multiply(x, cos).m()?, ops::multiply(&rot, sin).m()?).m()
    }

    /// El transformador previo: [1, T, latente] → [1, T, latente], con la caché K/V del flujo.
    fn transformador(&self, x: &Array, fl: &mut Flujo) -> Result<Array> {
        let c = &self.c;
        let t = x.dim(1);
        let desde = fl.largo;
        let (nh, nkv, hd) = (c.num_attention_heads, c.num_key_value_heads, c.head_dim);
        let mut h = self.entrada.aplicar(x)?;
        let pos = ops::arange::<_, f32>(desde as f32, (desde + t) as f32, 1.0).m()?;
        let frec = ops::multiply(ops::expand_dims(&pos, 1).m()?, ops::expand_dims(&self.inv_frec, 0).m()?).m()?;
        let emb = ops::concatenate(&[frec.clone(), frec], -1).m()?;
        let (cos, sin) = (ops::cos(&emb).m()?, ops::sin(&emb).m()?);
        if fl.kv.is_empty() {
            fl.kv = vec![Default::default(); self.capas.len()];
        }
        for (capa, cc) in self.capas.iter().zip(fl.kv.iter_mut()) {
            let y = Self::rms(&h, &capa.norma_in, c.rms_norm_eps)?;
            let forma = |a: Array, n: i32| -> Result<Array> { ops::transpose_axes(ops::reshape(&a, &[1, t, n, hd]).m()?, &[0, 2, 1, 3]).m() };
            let q = Self::girar(&forma(capa.q.aplicar(&y)?, nh)?, &cos, &sin)?;
            let k = Self::girar(&forma(capa.k.aplicar(&y)?, nkv)?, &cos, &sin)?;
            let v = forma(capa.v.aplicar(&y)?, nkv)?;
            let (k, v) = crate::modelo::escribir_kv(cc, &k, &v, desde, t)?;
            // causal con desplazamiento (abajo a la derecha): cada consulta ve la caché y su pasado
            let a = fast::scaled_dot_product_attention(&q, &k, &v, (hd as f32).powf(-0.5), if t > 1 { Some(ScaledDotProductAttentionMask::Causal) } else { None }, None::<&Array>).m()?;
            let a = ops::reshape(ops::transpose_axes(&a, &[0, 2, 1, 3]).m()?, &[1, t, nh * hd]).m()?;
            h = ops::add(&h, ops::multiply(&capa.escala_at, capa.o.aplicar(&a)?).m()?).m()?;
            let y = Self::rms(&h, &capa.norma_post, c.rms_norm_eps)?;
            let g = (self.f.silu.borrow_mut())(&capa.puerta.aplicar(&y)?).m()?;
            let y = capa.abajo.aplicar(&ops::multiply(&g, capa.arriba.aplicar(&y)?).m()?)?;
            h = ops::add(&h, ops::multiply(&capa.escala_mlp, &y).m()?).m()?;
        }
        fl.largo += t;
        self.salida.aplicar(&Self::rms(&h, &self.norma, c.rms_norm_eps)?)
    }

    /// Descuantiza [T, 16] códigos → [1, T, codebook_dim].
    fn descuantizar(&self, codigos: &Array) -> Result<Array> {
        let t = codigos.dim(0);
        let suma = |desde: usize, hasta: usize| -> Result<Array> {
            let mut acc = ops::zeros::<f32>(&[1, t, self.tablas[0].dim(1)]).m()?;
            for (i, tabla) in self.tablas.iter().enumerate().take(hasta).skip(desde) {
                let col = codigos.try_index((.., i as i32)).m()?;
                acc = ops::add(&acc, ops::expand_dims(take_axis(tabla, &col, 0).m()?, 0).m()?).m()?;
            }
            Ok(acc)
        };
        let ns = self.c.num_semantic_quantizers as usize;
        let a = ops::conv1d(suma(0, ns)?, &self.proy_primera, 1, 0, 1, 1).m()?;
        let b = ops::conv1d(suma(ns, self.tablas.len())?, &self.proy_resto, 1, 0, 1, 1).m()?;
        ops::add(&a, &b).m()
    }

    /// Decodifica de una vez: códigos [T, 16] (i32) → onda [T · 1920] en [−1, 1].
    pub fn decodificar(&self, codigos: &Array) -> Result<Array> {
        self.paso(&mut Self::flujo(), codigos)
    }

    /// Un flujo nuevo (al empezar cada frase).
    pub fn flujo() -> Flujo {
        Flujo { colas: Vec::new(), solapes: Vec::new(), kv: Vec::new(), largo: 0, i_cola: 0, i_solape: 0 }
    }

    /// Decodifica SÓLO los códigos nuevos [n, 16] → onda [n · 1920], continuando el flujo.
    pub fn paso(&self, fl: &mut Flujo, codigos: &Array) -> Result<Array> {
        fl.i_cola = 0;
        fl.i_solape = 0;
        let h = self.descuantizar(codigos)?;
        let h = self.pre_conv.aplicar(&h, fl)?;
        let mut h = self.transformador(&h, fl)?;
        for (ct, cn) in &self.sube {
            h = ct.aplicar(&h, fl)?;
            h = self.convnext(cn, &h, fl)?;
        }
        let mut x = self.conv0.aplicar(&h, fl)?;
        for b in &self.bloques {
            x = b.snake.aplicar(&x)?;
            x = b.sube.aplicar(&x, fl)?;
            for r in &b.residuales {
                let y = r.c1.aplicar(&r.a1.aplicar(&x)?, fl)?;
                let y = r.c2.aplicar(&r.a2.aplicar(&y)?, fl)?;
                x = ops::add(&y, &x).m()?;
            }
        }
        let x = self.conv_fin.aplicar(&self.snake_fin.aplicar(&x)?, fl)?;
        ops::clip(ops::reshape(&x, &[-1]).m()?, (&Array::from_f32(-1.0), &Array::from_f32(1.0))).m()
    }

    fn convnext(&self, cn: &ConvNext, x: &Array, fl: &mut Flujo) -> Result<Array> {
        let y = cn.dw.aplicar(x, fl)?;
        let y = fast::layer_norm(&y, Some(&cn.norma.0), Some(&cn.norma.1), 1e-6).m()?;
        let y = (self.f.gelu_exacta.borrow_mut())(&cn.pw1.aplicar(&y)?).m()?;
        let y = ops::multiply(&cn.gamma, cn.pw2.aplicar(&y)?).m()?;
        ops::add(x, &y).m()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn la_regla_de_formas_es_la_de_mlx_audio() {
        assert!(!ya_mlx(&[1024, 1, 7])); // dwconv PyTorch
        assert!(!ya_mlx(&[768, 768, 1])); // conv k1 PyTorch
        assert!(!ya_mlx(&[1024, 512, 3]));
        assert!(!ya_mlx(&[1536, 768, 16])); // transpuesta PyTorch
        assert!(ya_mlx(&[1024, 3, 512]));
        assert!(ya_mlx(&[1024, 7, 1]));
    }

    /// Juez contra mlx-audio (`ref_qwen3tts.py`): los códigos voraces → la misma onda.
    /// `GOTV_TTS_DIR=<modelo> GOTV_REF_TTS=<dir> cargo test -p gotv-mlx --release -- --ignored --nocapture decodifica_como_mlx_audio`
    #[test]
    #[ignore]
    fn decodifica_como_mlx_audio() {
        let (Ok(d), Ok(r)) = (std::env::var("GOTV_TTS_DIR"), std::env::var("GOTV_REF_TTS")) else { return };
        let rs = Array::load_safetensors(Path::new(&r).join("ref.safetensors")).unwrap();
        let t = std::time::Instant::now();
        let dec = Decodificador::cargar(Path::new(&d)).unwrap();
        let codigos = &rs["codes"];
        let wav = dec.decodificar(codigos).unwrap();
        wav.eval().unwrap();
        let (a, b) = (wav.as_slice::<f32>(), rs["wav"].as_slice::<f32>());
        assert_eq!(a.len(), b.len(), "mismo largo");
        let dif = a.iter().zip(b).map(|(x, y)| (x - y).abs()).fold(0.0, f32::max);
        let iguales = a.iter().zip(b).filter(|(x, y)| x == y).count();
        println!("{} fotogramas → {} muestras en {:?} · dif. máx {dif:e} · iguales {iguales}/{}", codigos.dim(0), a.len(), t.elapsed(), a.len());
        assert!(dif < 1e-3, "la onda difiere");
        // por trozos de 10 (como `ref_qwen3tts.py`) contra `streaming_step` de mlx-audio
        let t = std::time::Instant::now();
        let mut fl = Decodificador::flujo();
        let mut trozos = Vec::new();
        let n = codigos.dim(0);
        for i in (0..n).step_by(10) {
            let w = dec.paso(&mut fl, &codigos.try_index(i..(i + 10).min(n)).unwrap()).unwrap();
            w.eval().unwrap();
            trozos.extend_from_slice(w.as_slice::<f32>());
        }
        let b = rs["wav_trozos"].as_slice::<f32>();
        assert_eq!(trozos.len(), b.len(), "mismo largo por trozos");
        let dif_t = trozos.iter().zip(b).map(|(x, y)| (x - y).abs()).fold(0.0, f32::max);
        let entero = trozos.iter().zip(a).map(|(x, y)| (x - y).abs()).fold(0.0, f32::max);
        println!("por trozos de 10 en {:?} · dif. máx con mlx-audio {dif_t:e} · con la decodificación entera {entero:e}", t.elapsed());
        assert!(dif_t < 1e-3, "la onda por trozos difiere");
    }
}
