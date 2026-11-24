//! Modelos de lenguaje sobre MLX, escritos a mano (G4).
//!
//! Fase 1 — familia densa (`qwen3`, `llama`, `mistral`), mismo cálculo que `mlx_lm/models/qwen3.py`:
//! embedding → N × [RMSNorm → atención (q/k con RMSNorm por cabeza, RoPE, GQA) → residuo →
//! RMSNorm → MLP SwiGLU → residuo] → RMSNorm → cabeza (atada al embedding si el modelo lo dice).
//!
//! Fase 2 — el HÍBRIDO de Qwen3.5 (`qwen3_5`; también los Qwen3.6/3.8 modificados, que por dentro
//! son esta familia), como `mlx_lm/models/qwen3_5.py` + `qwen3_next.py` + `gated_delta.py`: 3 de
//! cada 4 capas son atención LINEAL Gated DeltaNet (conv causal corta con estado + recurrencia
//! delta con compuerta, en un kernel Metal propio) y la 4.ª es atención completa con compuerta de
//! salida y RoPE PARCIAL (sólo el 25 % de la cabeza gira).
//!
//! Pesos cuantizados afines (los de mlx-community: U32 empaquetado + escalas + sesgos) o en coma
//! flotante. La caché se puede RECORTAR: el agente reenvía todo el historial en cada paso y sólo
//! se recalcula lo que cambió. La caché KV se recorta en O(1); el estado recurrente NO se puede
//! recortar (resume todo lo leído), así que se guardan INSTANTÁNEAS en puntos útiles (fin del
//! historial, fin del prompt, fin de la respuesta) y se vuelve a la mejor que quepa.

use anyhow::{anyhow, bail, Context, Result};
use gotv_engine::{anyhow_tr, bail_tr, tr};
use mlx_rs::fast::{self, MetalKernel, MetalKernelApplyOptions, MetalKernelOptions, MetalKernelOutput, ScaledDotProductAttentionMask, TemplateArg};
use mlx_rs::ops::indexing::{take_axis, TryIndexMutOp, TryIndexOp};
use mlx_rs::ops::{self, DequantizeOptions, GatherQmmOptions, QuantizedMatmulOptions};
use mlx_rs::error::Exception;
use mlx_rs::transforms::compile::compile;
use mlx_rs::{Array, Dtype};
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;

/// F9.0 · un paso de generación para varias conversaciones a la vez sobre los mismos pesos (y su medición)
pub mod lote;

/// Errores de mlx-rs (no son `Send`) → anyhow con su texto.
pub(crate) trait M<T> {
    fn m(self) -> Result<T>;
}
impl<T, E: std::fmt::Debug> M<T> for std::result::Result<T, E> {
    fn m(self) -> Result<T> {
        self.map_err(|e| anyhow!("mlx: {e:?}"))
    }
}

#[derive(Clone, Debug, serde::Deserialize)]
pub struct Config {
    pub model_type: String,
    pub hidden_size: i32,
    pub num_hidden_layers: i32,
    pub num_attention_heads: i32,
    pub num_key_value_heads: i32,
    #[serde(default)]
    pub head_dim: Option<i32>,
    #[serde(default = "theta")]
    pub rope_theta: f32,
    #[serde(default = "eps")]
    pub rms_norm_eps: f32,
    #[serde(default)]
    pub tie_word_embeddings: bool,
    #[serde(default)]
    pub max_position_embeddings: Option<u32>,
    /// global `{group_size, bits}` y, en los cuantizados mixtos, una entrada POR CAPA
    #[serde(default)]
    pub quantization: Option<serde_json::Value>,
    #[serde(default)]
    pub rope_scaling: Option<serde_json::Value>,
    #[serde(default)]
    pub eos_token_id: Option<serde_json::Value>,
    // ── híbrido (qwen3_5) ──
    /// cada cuántas capas va una de atención completa (las demás son lineales)
    #[serde(default)]
    pub full_attention_interval: Option<i32>,
    #[serde(default)]
    pub linear_num_key_heads: Option<i32>,
    #[serde(default)]
    pub linear_num_value_heads: Option<i32>,
    #[serde(default)]
    pub linear_key_head_dim: Option<i32>,
    #[serde(default)]
    pub linear_value_head_dim: Option<i32>,
    #[serde(default)]
    pub linear_conv_kernel_dim: Option<i32>,
    /// fracción de la cabeza que gira con RoPE (Qwen3.5: 0,25)
    #[serde(default)]
    pub partial_rotary_factor: Option<f32>,
    #[serde(default)]
    pub rope_parameters: Option<serde_json::Value>,
    #[serde(default)]
    pub num_experts: Option<i32>,
    // ── expertos (MoE: qwen3_5_moe) ──
    #[serde(default)]
    pub num_experts_per_tok: Option<i32>,
    /// renormalizar los pesos de los k elegidos (Qwen3.5-MoE: sí, por omisión)
    #[serde(default)]
    pub norm_topk_prob: Option<bool>,
    #[serde(default)]
    pub decoder_sparse_step: Option<i32>,
    #[serde(default)]
    pub mlp_only_layers: Vec<i32>,
    // ── Gemma 4: variantes que aún no se validan ──
    #[serde(default)]
    pub enable_moe_block: Option<bool>,
    #[serde(default)]
    pub attention_k_eq_v: Option<bool>,
    /// prefijo de los nombres en el config de cuantización (`language_model.` en los híbridos)
    #[serde(skip)]
    pub prefijo: String,
}

#[derive(Clone, Copy, Debug, serde::Deserialize, PartialEq)]
pub struct Cuantizacion {
    pub group_size: i32,
    pub bits: i32,
}

/// `config.json` aplanado: los multimodales/híbridos guardan el modelo de texto en `text_config` y
/// la cuantización arriba; `rope_parameters` (formato nuevo) se lleva a los campos de siempre.
/// Devuelve el JSON plano y el prefijo de los nombres en la cuantización (`language_model.`).
pub(crate) fn aplanar(t: &str) -> Result<(serde_json::Value, String)> {
    let v: serde_json::Value = serde_json::from_str(t).context("config.json")?;
    let mut plano = v.clone();
    let mut prefijo = String::new();
    if let Some(tc) = v.get("text_config").and_then(|t| t.as_object()) {
        let mut o = tc.clone();
        for k in ["quantization", "tie_word_embeddings", "eos_token_id"] {
            if let Some(x) = v.get(k) {
                o.entry(k.to_string()).or_insert_with(|| x.clone());
            }
        }
        // el tipo que decide es el de arriba (qwen3_5), no el del texto (qwen3_5_text)
        if let Some(mt) = v.get("model_type") {
            o.insert("model_type".into(), mt.clone());
        }
        plano = serde_json::Value::Object(o);
        prefijo = "language_model.".into();
    }
    // rope_parameters (formato nuevo de transformers) → los campos de siempre
    if let Some(rp) = plano.get("rope_parameters").cloned().filter(|r| r.is_object()) {
        let o = plano.as_object_mut().unwrap();
        if let Some(t) = rp.get("rope_theta") {
            o.entry("rope_theta").or_insert_with(|| t.clone());
        }
        if let Some(f) = rp.get("partial_rotary_factor") {
            o.entry("partial_rotary_factor").or_insert_with(|| f.clone());
        }
    }
    // los valores por omisión del híbrido en `mlx_lm` (qwen3_5.py), NO los del denso: sin esto un
    // config sin rope_parameters giraría la cabeza entera con otra θ y saldría basura sin error
    if matches!(plano.get("model_type").and_then(|m| m.as_str()), Some("qwen3_5" | "qwen3_5_text" | "qwen3_5_moe" | "qwen3_5_moe_text")) {
        let o = plano.as_object_mut().unwrap();
        o.entry("partial_rotary_factor").or_insert(serde_json::json!(0.25));
        o.entry("rope_theta").or_insert(serde_json::json!(100000.0));
    }
    Ok((plano, prefijo))
}

impl Config {
    /// Lee `config.json`. Los híbridos (y los multimodales) guardan el modelo de texto en
    /// `text_config` y la cuantización arriba: se aplanan aquí.
    pub fn leer(dir: &Path) -> Result<Config> {
        let t = std::fs::read_to_string(dir.join("config.json")).context("config.json")?;
        Config::de_json(&t)
    }

    pub fn de_json(t: &str) -> Result<Config> {
        let (plano, prefijo) = aplanar(t)?;
        let mut c: Config = serde_json::from_value(plano).context("config.json")?;
        c.prefijo = prefijo;
        Ok(c)
    }


    /// La cuantización de UNA capa: su entrada propia (cuantizados mixtos: DWQ, oQ…) o la global.
    /// Medido 2026-11-10: el 27B oQ4e trae 4, 5 y 6 bits según la capa; leer sólo la global lo
    /// habría corrompido sin ningún error.
    pub fn cuantizacion(&self, nombre: &str) -> Option<Cuantizacion> {
        let q = self.quantization.as_ref()?;
        let de = |v: &serde_json::Value| serde_json::from_value::<Cuantizacion>(v.clone()).ok();
        q.get(format!("{}{nombre}", self.prefijo).as_str()).and_then(de).or_else(|| q.get(nombre).and_then(de)).or_else(|| de(q))
    }

    pub fn gemma(&self) -> bool {
        matches!(self.model_type.as_str(), "gemma4" | "gemma4_text")
    }

    pub fn hibrido(&self) -> bool {
        matches!(self.model_type.as_str(), "qwen3_5" | "qwen3_5_text" | "qwen3_5_moe" | "qwen3_5_moe_text")
    }

    /// ¿La capa `i` lleva expertos? (como `Qwen3NextDecoderLayer`)
    fn capa_moe(&self, i: i32) -> bool {
        self.num_experts.unwrap_or(0) > 0 && !self.mlp_only_layers.contains(&i) && (i + 1) % self.decoder_sparse_step.unwrap_or(1).max(1) == 0
    }

    /// El tipo de RoPE declarado (rope_scaling o rope_parameters); `None` = el normal.
    fn tipo_rope(&self) -> Option<String> {
        self.parametros_rope().and_then(|r| r.get("type").or(r.get("rope_type")).and_then(|t| t.as_str()).map(str::to_string))
    }

    fn parametros_rope(&self) -> Option<&serde_json::Value> {
        [&self.rope_scaling, &self.rope_parameters].into_iter().flatten().find(|r| !r.is_null() && (r.get("type").is_some() || r.get("rope_type").is_some()))
    }

    /// RoPE «llama3» (Llama 3.1+, Hermes 4): los PERIODOS de cada par (lo que `fast::rope` recibe como
    /// `freqs`), como `Llama3RoPE` de mlx_lm y `_compute_llama3_parameters` de transformers: las
    /// longitudes de onda largas (> contexto original / low_freq_factor) se estiran por `factor`, las
    /// cortas quedan, y las intermedias se suavizan. En f64 y luego f32. `None` si no es llama3.
    pub(crate) fn periodos_llama3(&self, dims: i32) -> Option<Vec<f32>> {
        let r = self.parametros_rope().filter(|_| self.tipo_rope().as_deref() == Some("llama3"))?;
        let num = |k: &str, d: f64| r.get(k).and_then(|v| v.as_f64()).unwrap_or(d);
        let (factor, baja, alta, original) = (num("factor", 1.0), num("low_freq_factor", 1.0), num("high_freq_factor", 4.0), num("original_max_position_embeddings", 8192.0));
        let (onda_baja, onda_alta) = (original / baja, original / alta);
        let base = self.rope_theta as f64;
        Some(
            (0..dims / 2)
                .map(|i| {
                    let periodo = base.powf((2 * i) as f64 / dims as f64);
                    let onda = 2.0 * std::f64::consts::PI * periodo;
                    let p = if onda > onda_baja {
                        periodo * factor
                    } else if onda > onda_alta && onda < onda_baja {
                        let s = (original / onda - baja) / (alta - baja);
                        periodo / ((1.0 - s) / factor + s)
                    } else {
                        periodo
                    };
                    p as f32
                })
                .collect(),
        )
    }
}

fn theta() -> f32 {
    1_000_000.0
}
fn eps() -> f32 {
    1e-6
}

/// Arquitecturas que este núcleo sabe correr (las demás se rechazan con un error claro).
pub fn soportada(model_type: &str) -> bool {
    matches!(model_type, "qwen3" | "qwen2" | "llama" | "mistral" | "qwen3_5" | "qwen3_5_text" | "qwen3_5_moe" | "qwen3_5_moe_text" | "gemma4" | "gemma4_text")
}

/// Por qué un modelo NO puede correr aquí (sólo con su config.json), o `None` si puede.
pub fn por_que_no(c: &Config) -> Option<String> {
    if !soportada(&c.model_type) {
        return Some(tr!("arquitectura «{}» aún no está en el núcleo nativo (qwen3, qwen3_5, gemma4, llama, mistral)", "architecture «{}» is not in the native core yet (qwen3, qwen3_5, gemma4, llama, mistral)", c.model_type));
    }
    // variantes de Gemma 4 sin un modelo con el que validarlas todavía: por el servidor
    if c.gemma() && c.enable_moe_block == Some(true) {
        return Some(tr!("Gemma 4 con expertos (26B-A4B) aún no está en el núcleo", "Gemma 4 with experts (26B-A4B) is not in the core yet"));
    }
    if c.gemma() && c.attention_k_eq_v == Some(true) {
        return Some(tr!("Gemma 4 con K = V (31B) aún no está en el núcleo", "Gemma 4 with K = V (31B) is not in the core yet"));
    }
    if c.num_experts.unwrap_or(0) > 0 && !c.hibrido() {
        return Some(tr!("«{}» con expertos (MoE) aún no está en el núcleo (sí el MoE de Qwen3.5)", "«{}» with experts (MoE) is not in the core yet (the Qwen3.5 MoE is)", c.model_type));
    }
    if let Some(t) = c.tipo_rope().filter(|t| t != "default" && t != "llama3") {
        return Some(tr!("RoPE «{t}» aún no está en el núcleo nativo", "RoPE «{t}» is not in the native core yet"));
    }
    if let Some(q) = c.quantization.as_ref().filter(|q| !q.is_null()) {
        if q.get("mode").and_then(|m| m.as_str()).is_some_and(|m| m != "affine") {
            return Some(tr!("sólo cuantización afín (mxfp4 y otras aún no están en el núcleo)", "affine quantization only (mxfp4 and others are not in the core yet)"));
        }
    }
    None
}

pub(crate) enum Lineal {
    Q { w: Array, s: Array, b: Array, q: Cuantizacion },
    F { w_t: Array },
}

impl Lineal {
    pub(crate) fn cargar(p: &mut HashMap<String, Array>, nombre: &str, cfg: &Config) -> Result<Lineal> {
        let q = cfg.cuantizacion(nombre);
        let w = p.remove(&format!("{nombre}.weight")).ok_or_else(|| anyhow!("falta {nombre}.weight"))?;
        match (p.remove(&format!("{nombre}.scales")), q) {
            (Some(s), Some(q)) => {
                let b = p.remove(&format!("{nombre}.biases")).ok_or_else(|| anyhow_tr!("falta {nombre}.biases (sólo cuantización afín)", "{nombre}.biases is missing (affine quantization only)"))?;
                Ok(Lineal::Q { w, s, b, q })
            }
            (Some(_), None) => bail_tr!("{nombre} está cuantizado pero config.json no dice cómo", "{nombre} is quantized but config.json does not say how"),
            (None, _) => Ok(Lineal::F { w_t: ops::transpose_axes(&w, &[1, 0]).m()? }),
        }
    }

    pub(crate) fn aplicar(&self, x: &Array) -> Result<Array> {
        match self {
            Lineal::Q { w, s, b, q } => ops::quantized_matmul(
                x,
                w,
                s,
                Some(b),
                QuantizedMatmulOptions { group_size: Some(q.group_size), bits: Some(q.bits), transpose: true, ..Default::default() },
            )
            .m(),
            Lineal::F { w_t } => ops::matmul(x, w_t).m(),
        }
    }
}

pub(crate) struct Embedding {
    pub(crate) lineal: Lineal,
}

impl Embedding {
    /// Filas de los tokens (en un embedding cuantizado: se desempaquetan SÓLO esas filas).
    pub(crate) fn filas(&self, ids: &Array) -> Result<Array> {
        match &self.lineal {
            Lineal::Q { w, s, b, q } => ops::dequantize(
                take_axis(w, ids, 0).m()?,
                take_axis(s, ids, 0).m()?,
                Some(&take_axis(b, ids, 0).m()?),
                DequantizeOptions { group_size: Some(q.group_size), bits: Some(q.bits), ..Default::default() },
            )
            .m(),
            Lineal::F { w_t } => {
                let w = ops::transpose_axes(w_t, &[1, 0]).m()?;
                take_axis(&w, ids, 0).m()
            }
        }
    }
}

/// Atención completa (softmax). En Qwen3.5 lleva compuerta de salida: `q_proj` da q Y la compuerta.
struct Atencion {
    q: Lineal,
    k: Lineal,
    v: Lineal,
    o: Lineal,
    q_norma: Option<Array>,
    k_norma: Option<Array>,
    compuerta: bool,
}

/// Atención lineal Gated DeltaNet (Qwen3.5): conv causal corta con estado + recurrencia delta.
struct DeltaNet {
    qkv: Lineal,
    z: Lineal,
    b: Lineal,
    a: Lineal,
    salida: Lineal,
    /// [canales, kernel, 1] (conv por canal)
    conv: Array,
    a_log: Array,
    dt_bias: Array,
    /// RMSNorm con compuerta, por cabeza de valor
    norma: Array,
}

enum Mezcla {
    Atencion(Atencion),
    Lineal(DeltaNet),
}

struct Capa {
    norma_in: Array,
    norma_post: Array,
    mezcla: Mezcla,
    mlp: Mlp,
}

/// Un lineal POR EXPERTO, apilado [expertos, salida, entrada] y cuantizado: sólo se multiplican
/// los expertos elegidos (`gather_qmm`), como `QuantizedSwitchLinear` de `mlx_lm`.
struct Conmutado {
    w: Array,
    s: Array,
    b: Array,
    q: Cuantizacion,
}

impl Conmutado {
    fn cargar(p: &mut HashMap<String, Array>, nombre: &str, cfg: &Config) -> Result<Conmutado> {
        let w = quitar(p, &format!("{nombre}.weight"))?;
        let s = p.remove(&format!("{nombre}.scales")).ok_or_else(|| anyhow_tr!("{nombre}: expertos sin cuantizar aún no están en el núcleo", "{nombre}: unquantized experts are not in the core yet"))?;
        let b = quitar(p, &format!("{nombre}.biases"))?;
        let q = cfg.cuantizacion(nombre).ok_or_else(|| anyhow_tr!("{nombre} está cuantizado pero config.json no dice cómo", "{nombre} is quantized but config.json does not say how"))?;
        Ok(Conmutado { w, s, b, q })
    }

    fn aplicar(&self, x: &Array, expertos: &Array, ordenados: bool) -> Result<Array> {
        ops::gather_qmm(
            x,
            &self.w,
            &self.s,
            Some(&self.b),
            GatherQmmOptions {
                group_size: Some(self.q.group_size),
                bits: Some(self.q.bits),
                transpose: true,
                rhs_indices: Some(expertos),
                sorted_indices: ordenados,
                ..Default::default()
            },
        )
        .m()
    }
}

/// Mezcla de expertos de Qwen3.5-MoE (`Qwen3NextSparseMoeBlock`): un router elige k de N expertos
/// por token y pondera sus salidas; un experto COMPARTIDO, con su propia compuerta, siempre suma.
struct Moe {
    router: Lineal,
    puerta: Conmutado,
    arriba: Conmutado,
    abajo: Conmutado,
    compartido: (Lineal, Lineal, Lineal),
    compuerta_compartido: Lineal,
    k: i32,
    normalizar: bool,
}

enum Mlp {
    Densa { puerta: Lineal, arriba: Lineal, abajo: Lineal },
    Moe(Box<Moe>),
}

/// Caché de UNA capa. Atención: K/V [1, kv_heads, capacidad, head_dim], reservada por bloques de
/// `PASO` y escrita EN SU LUGAR (como `mlx_lm.KVCache`; medido: concatenar en cada token copiaba
/// toda la caché y dejaba el núcleo un 15 % por debajo del servidor). Lineal: las últimas
/// `kernel-1` entradas de la conv y el estado recurrente [1, Hv, Dv, Dk] en f32.
#[derive(Default, Clone)]
pub struct CacheCapa {
    pub(crate) k: Option<Array>,
    pub(crate) v: Option<Array>,
    pub(crate) conv: Option<Array>,
    pub(crate) estado: Option<Array>,
}

pub(crate) const PASO: i32 = 256;
/// Instantáneas del estado recurrente que se conservan (cada una: ~48 MB en el 9B). Las arrays de
/// MLX son inmutables: guardar una es guardar una referencia, no copiar.
const INSTANTANEAS: usize = 8;

/// Cuál instantánea soltar cuando sobran (`largos` en orden creciente): nunca la primera ni la última;
/// la que deja el hueco más chico entre sus vecinas. Así se ralean las apiñadas de un bucle de
/// herramientas y SOBREVIVE la del inicio del turno: el Portador escribe, la plantilla del 9B quita el
/// `<think>` de los pasos anteriores y hay que volver justo ahí (medido 2026-11-11: con «soltar la más
/// vieja», 11 pasos borraban esa instantánea y la orden siguiente releía la conversación entera).
pub(crate) fn cual_desalojar(largos: &[i32]) -> usize {
    if largos.len() < 3 {
        return 0;
    }
    (1..largos.len() - 1).rev().min_by_key(|&i| largos[i + 1] - largos[i - 1]).unwrap()
}

/// Lo que `Modelo::recortar(n)` dejaría, sin tocar la caché (misma regla).
pub(crate) fn alcanzable_en(c: &Cache, n: i32, recurrente: bool) -> i32 {
    if n <= 0 {
        return 0;
    }
    if n >= c.largo || !recurrente {
        return n.min(c.largo);
    }
    c.instantaneas.iter().rev().find(|s| s.largo <= n).map(|s| s.largo).unwrap_or(0)
}

/// El estado recurrente de todas las capas lineales en una posición.
#[derive(Clone)]
pub(crate) struct Instantanea {
    largo: i32,
    capas: Vec<(Option<Array>, Option<Array>)>,
}

type F1 = Box<dyn for<'a> FnMut(&'a Array) -> std::result::Result<Array, Exception>>;
type F2 = Box<dyn for<'a> FnMut((&'a Array, &'a Array)) -> std::result::Result<Array, Exception>>;
type F3 = Box<dyn for<'a> FnMut((&'a Array, &'a Array, &'a Array)) -> std::result::Result<Array, Exception>>;

/// Operaciones elementales FUSIONADAS en un solo kernel, compiladas UNA vez (como los
/// `@mx.compile` de `mlx_lm`). Medido 2026-11-10: `mlx_rs::nn::silu` compila y borra un grafo en
/// CADA llamada (≈160 trazados por token en el 27B) y sin fusión cada paso es un kernel aparte:
/// el núcleo generaba 10,4–11,6 tok/s contra 13–14 de `mlx_lm` con el 27B OptiQ.
pub(crate) struct Fusiones {
    /// silu(compuerta) · arriba (el MLP SwiGLU)
    pub(crate) swiglu: RefCell<F2>,
    /// x · sigmoid(x)
    pub(crate) silu: RefCell<F1>,
    /// exp(−exp(A_log) · softplus(a + dt_bias)), en f32
    pub(crate) decaimiento: RefCell<F3>,
    /// silu(z) · y_normalizada en f32, de vuelta al tipo de y
    pub(crate) norma_puerta: RefCell<F2>,
    /// a · sigmoid(compuerta) (salida de la atención completa del híbrido)
    pub(crate) puerta: RefCell<F2>,
    /// gelu_aprox(compuerta) · arriba (el MLP GeGLU de Gemma)
    pub(crate) geglu: RefCell<F2>,
    /// GeLU con tanh (el MLP de la torre de visión)
    pub(crate) gelu_tanh: RefCell<F1>,
    /// GeLU exacta con erf (el fusionador de la torre de visión)
    pub(crate) gelu_exacta: RefCell<F1>,
}

impl Fusiones {
    pub(crate) fn nuevas() -> Fusiones {
        Self::armar(true)
    }

    /// Las MISMAS operaciones sin compilar. Para la voz (M2), que llama al modelo frase tras frase:
    /// medido 2026-11-10, con funciones compiladas la 1.ª frase iba a 30 fotogramas/s y TODAS las
    /// siguientes a ~7 (por debajo de los 12,5 del habla: la voz se cortaba); limpiar la caché de
    /// compilación de MLX devolvía los 30. Sin compilar, cada frase va igual que la primera.
    pub(crate) fn sin_compilar() -> Fusiones {
        Self::armar(false)
    }

    fn armar(compilar: bool) -> Fusiones {
        let swiglu = |(g, u): (&Array, &Array)| ops::multiply(&ops::multiply(g, &ops::sigmoid(g)?)?, u);
        let silu = |x: &Array| ops::multiply(x, &ops::sigmoid(x)?);
        let decaimiento = |(a_log, a, dt): (&Array, &Array, &Array)| {
            let sp = ops::logaddexp(&ops::add(a, dt)?, &Array::from_f32(0.0).as_dtype(a.dtype())?)?;
            ops::exp(&ops::negative(&ops::multiply(&ops::exp(&a_log.as_dtype(Dtype::Float32)?)?, &sp)?)?)
        };
        let norma_puerta = |(z, x): (&Array, &Array)| {
            let z = z.as_dtype(Dtype::Float32)?;
            ops::multiply(&ops::multiply(&z, &ops::sigmoid(&z)?)?, &x.as_dtype(Dtype::Float32)?)?.as_dtype(x.dtype())
        };
        let puerta = |(a, g): (&Array, &Array)| ops::multiply(a, &ops::sigmoid(g)?);
        // 0,5·x·(1 + tanh(√(2/π)·(x + 0,044715·x³))), como `nn.gelu_approx` (escalares con el dtype de x)
        let geglu = |(g, u): (&Array, &Array)| {
            let t = |v: f32| Array::from_f32(v).as_dtype(g.dtype());
            let x3 = ops::power(g, &t(3.0)?)?;
            let dentro = ops::multiply(&t((2.0 / std::f32::consts::PI).sqrt())?, &ops::add(g, &ops::multiply(&t(0.044715)?, &x3)?)?)?;
            let gelu = ops::multiply(&ops::multiply(&t(0.5)?, g)?, &ops::add(&t(1.0)?, &ops::tanh(&dentro)?)?)?;
            ops::multiply(&gelu, u)
        };
        // `nn.gelu_approx`: 0,5·x·(1 + tanh(√(2/π)·(x + 0,044715·x³)))
        let gelu_tanh = |x: &Array| {
            let t = |v: f32| Array::from_f32(v).as_dtype(x.dtype());
            let x3 = ops::power(x, &t(3.0)?)?;
            let dentro = ops::multiply(&t((2.0 / std::f32::consts::PI).sqrt())?, &ops::add(x, &ops::multiply(&t(0.044715)?, &x3)?)?)?;
            ops::multiply(&ops::multiply(&t(0.5)?, x)?, &ops::add(&t(1.0)?, &ops::tanh(&dentro)?)?)
        };
        // `nn.gelu`: x·(1 + erf(x/√2))/2
        let gelu_exacta = |x: &Array| {
            let t = |v: f32| Array::from_f32(v).as_dtype(x.dtype());
            let e = ops::erf(&ops::divide(x, &t(std::f32::consts::SQRT_2)?)?)?;
            ops::divide(&ops::multiply(x, &ops::add(&t(1.0)?, &e)?)?, &t(2.0)?)
        };
        if !compilar {
            return Fusiones {
                swiglu: RefCell::new(Box::new(swiglu)),
                silu: RefCell::new(Box::new(silu)),
                decaimiento: RefCell::new(Box::new(decaimiento)),
                norma_puerta: RefCell::new(Box::new(norma_puerta)),
                puerta: RefCell::new(Box::new(puerta)),
                geglu: RefCell::new(Box::new(geglu)),
                gelu_tanh: RefCell::new(Box::new(gelu_tanh)),
                gelu_exacta: RefCell::new(Box::new(gelu_exacta)),
            };
        }
        Fusiones {
            swiglu: RefCell::new(Box::new(compile(swiglu, true))),
            silu: RefCell::new(Box::new(compile(silu, true))),
            decaimiento: RefCell::new(Box::new(compile(decaimiento, true))),
            norma_puerta: RefCell::new(Box::new(compile(norma_puerta, true))),
            puerta: RefCell::new(Box::new(compile(puerta, true))),
            geglu: RefCell::new(Box::new(compile(geglu, true))),
            gelu_tanh: RefCell::new(Box::new(compile(gelu_tanh, true))),
            gelu_exacta: RefCell::new(Box::new(compile(gelu_exacta, true))),
        }
    }
}

pub struct Modelo {
    pub cfg: Config,
    embed: Embedding,
    capas: Vec<Capa>,
    norma: Array,
    cabeza: Option<Lineal>,
    head_dim: i32,
    /// dimensiones que giran con RoPE (= head_dim salvo RoPE parcial)
    rope_dims: i32,
    /// RoPE «llama3»: los periodos de cada par (`None` = la RoPE normal con θ)
    rope_periodos: Option<Array>,
    /// hay capas con estado recurrente (no recortable)
    pub recurrente: bool,
    kernel_delta: Option<MetalKernel>,
    f: Fusiones,
    /// mRoPE entrelazado: cuántas frecuencias van a (t, alto, ancho)
    mrope: [i32; 3],
    kernel_mrope: Option<MetalKernel>,
}

pub struct Cache {
    pub capas: Vec<CacheCapa>,
    /// cuántos tokens hay en la caché (todas las capas igual)
    pub largo: i32,
    pub(crate) instantaneas: Vec<Instantanea>,
}

impl Cache {
    pub fn capas_len(&self) -> usize {
        self.capas.len()
    }

    /// Guarda la caché tal como está (K/V recortados a `largo`, conv y estado recurrente) en un
    /// .safetensors; escribe a un temporal y lo renombra (nunca queda un archivo a medias).
    pub fn guardar(&self, ruta: &Path) -> Result<()> {
        let mut partes: Vec<(String, Array)> = Vec::new();
        for (i, cc) in self.capas.iter().enumerate() {
            if let (Some(k), Some(v)) = (&cc.k, &cc.v) {
                partes.push((format!("{i}.k"), k.try_index((.., .., ..self.largo, ..)).m()?));
                partes.push((format!("{i}.v"), v.try_index((.., .., ..self.largo, ..)).m()?));
            }
            if let Some(c) = &cc.conv {
                partes.push((format!("{i}.conv"), c.clone()));
            }
            if let Some(e) = &cc.estado {
                partes.push((format!("{i}.estado"), e.clone()));
            }
        }
        let meta = HashMap::from([("largo".to_string(), self.largo.to_string()), ("capas".to_string(), self.capas.len().to_string())]);
        let tmp = ruta.with_extension(format!("{}.tmp.safetensors", std::process::id()));
        Array::save_safetensors(partes, &meta, &tmp).map_err(|e| anyhow_tr!("guardar caché: {e:?}", "saving the cache: {e:?}"))?;
        std::fs::rename(&tmp, ruta).context(tr!("guardar caché", "saving the cache"))?;
        Ok(())
    }

    /// Lee una caché guardada con `guardar` (las capas deben coincidir con el modelo).
    pub fn leer(ruta: &Path, capas: usize) -> Result<Cache> {
        let (mut a, meta) = Array::load_safetensors_with_metadata(ruta).map_err(|e| anyhow_tr!("leer caché: {e:?}", "reading the cache: {e:?}"))?;
        let n: usize = meta.get("capas").and_then(|v| v.parse().ok()).unwrap_or(0);
        if n != capas {
            bail_tr!("la caché guardada es de otro modelo ({n} capas, este tiene {capas})", "the saved cache is from another model ({n} layers, this one has {capas})");
        }
        let largo: i32 = meta.get("largo").and_then(|v| v.parse().ok()).ok_or_else(|| anyhow_tr!("caché sin largo", "cache without a length"))?;
        let capas = (0..capas)
            .map(|i| CacheCapa { k: a.remove(&format!("{i}.k")), v: a.remove(&format!("{i}.v")), conv: a.remove(&format!("{i}.conv")), estado: a.remove(&format!("{i}.estado")) })
            .collect();
        Ok(Cache { capas, largo, instantaneas: Vec::new() })
    }
}

/// Una familia de modelos del núcleo: el motor la usa sin saber cuál es.
pub trait Red {
    fn cfg(&self) -> &Config;
    fn cache_nueva(&self) -> Cache;
    /// Recorta para reutilizar un prefijo de `n` tokens; devuelve cuántos quedaron DE VERDAD.
    fn recortar(&self, c: &mut Cache, n: i32) -> Result<i32>;
    /// Lo que `recortar(n)` dejaría, SIN tocar la caché. En un híbrido puede ser mucho menos que `n`
    /// (la última instantánea ≤ n): comparar la memoria del disco contra `n` la descartaba aunque la
    /// RAM no pudiera volver a ningún punto (medido 2026-11-11: releyó 56k tokens con el estado en disco).
    fn alcanzable(&self, c: &Cache, n: i32) -> i32 {
        n.clamp(0, c.largo)
    }
    /// Guarda el estado en la posición actual (sólo las familias con estado recurrente).
    fn marcar(&self, c: &mut Cache);
    /// Pasa los tokens nuevos y devuelve los logits del último.
    fn adelante(&self, ids: &[u32], c: &mut Cache) -> Result<Array>;
    /// Igual, con tokens visuales y/o posiciones mRoPE explícitas (M8). Por omisión, sin visión.
    fn adelante_ext(&self, ids: &[u32], extra: &Extra, c: &mut Cache) -> Result<Array> {
        if extra.rasgos.is_some() || extra.pos.is_some() {
            bail_tr!("esta familia aún no ve imágenes en el núcleo", "this family does not see images in the core yet");
        }
        self.adelante(ids, c)
    }
}

/// Lo que necesita el kernel del mRoPE para un trozo.
pub(crate) struct GiroM {
    pos: Array,
    inv: Array,
    sel: Array,
}

/// Lo que acompaña a un trozo de tokens cuando el prompt trae imágenes.
#[derive(Default)]
pub struct Extra<'a> {
    /// los tokens visuales de todas las imágenes [n, D] y, por token del trozo, cuál le toca (−1 = texto)
    pub rasgos: Option<(&'a Array, Vec<i32>)>,
    /// posiciones (t, alto, ancho) de cada token del trozo
    pub pos: Option<[Vec<i32>; 3]>,
}

impl Red for Modelo {
    fn cfg(&self) -> &Config {
        &self.cfg
    }
    fn cache_nueva(&self) -> Cache {
        Modelo::cache_nueva(self)
    }
    fn recortar(&self, c: &mut Cache, n: i32) -> Result<i32> {
        Modelo::recortar(self, c, n)
    }
    fn alcanzable(&self, c: &Cache, n: i32) -> i32 {
        alcanzable_en(c, n, self.recurrente)
    }
    fn marcar(&self, c: &mut Cache) {
        Modelo::marcar(self, c)
    }
    fn adelante(&self, ids: &[u32], c: &mut Cache) -> Result<Array> {
        Modelo::adelante_ext(self, ids, &Extra::default(), c)
    }
    fn adelante_ext(&self, ids: &[u32], extra: &Extra, c: &mut Cache) -> Result<Array> {
        Modelo::adelante_ext(self, ids, extra, c)
    }
}

/// Carga la familia que diga el config.json del modelo.
pub fn cargar_red(dir: &Path) -> Result<Box<dyn Red>> {
    let cfg = Config::leer(dir)?;
    if let Some(e) = por_que_no(&cfg) {
        bail!("{e}");
    }
    if cfg.gemma() {
        return Ok(Box::new(crate::gemma::Gemma::cargar(dir, cfg)?));
    }
    Ok(Box::new(Modelo::cargar(dir)?))
}

pub(crate) fn quitar(p: &mut HashMap<String, Array>, n: &str) -> Result<Array> {
    p.remove(n).ok_or_else(|| anyhow!("falta {n}"))
}

/// Los pesos del modelo de TEXTO: un archivo o varios fragmentos. En los multimodales (Qwen3.5 con
/// visión, Gemma 4 con visión y audio) sólo se quedan los de `language_model.` y se les quita el
/// prefijo; las torres de visión/audio no se cargan.
pub(crate) fn cargar_pesos(dir: &Path) -> Result<HashMap<String, Array>> {
    let mut archivos: Vec<_> = std::fs::read_dir(dir)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "safetensors"))
        .collect();
    archivos.sort();
    if archivos.is_empty() {
        bail!("no hay .safetensors en {}", dir.display());
    }
    let mut todos: Vec<(String, Array)> = Vec::new();
    for a in &archivos {
        todos.extend(Array::load_safetensors(a).map_err(|e| anyhow!("{}: {e:?}", a.display()))?);
    }
    let multimodal = todos.iter().any(|(k, _)| k.starts_with("language_model.") || k.starts_with("model.language_model."));
    let mut pesos = HashMap::new();
    for (k, v) in todos {
        let k = if let Some(r) = k.strip_prefix("model.language_model.") {
            format!("model.{r}")
        } else if let Some(r) = k.strip_prefix("language_model.") {
            r.to_string()
        } else if multimodal || k.starts_with("vision_tower") || k.starts_with("model.visual") {
            continue;
        } else {
            k
        };
        pesos.insert(k, v);
    }
    // un `Qwen3Model` a secas (sin `model.`, p. ej. el Qwen3-Embedding de ACE-Step): se le pone el
    // prefijo para que lo lea el mismo cargador
    if !pesos.keys().any(|k| k.starts_with("model.")) && pesos.contains_key("embed_tokens.weight") {
        pesos = pesos.into_iter().map(|(k, v)| if k.starts_with("lm_head.") { (k, v) } else { (format!("model.{k}"), v) }).collect();
    }
    Ok(pesos)
}

/// `1 / base^(arange(0, dim, 2)/dim)` en la GPU, EXACTAMENTE como Python (`compute_inv_freq`):
/// el `pow` de Metal y el `powf` de la CPU difieren en el último bit y, en bf16, eso basta para
/// separar la torre de visión de mlx_vlm (medido 2026-11-10: el bloque 1 se movía un ulp).
pub(crate) fn inv_frec(dim: i32, base: f32) -> Result<Array> {
    let ex = ops::divide(&ops::arange::<_, f32>(0.0, dim as f32, 2.0).m()?, Array::from_f32(dim as f32)).m()?;
    ops::divide(Array::from_f32(1.0), &ops::power(Array::from_f32(base), &ex).m()?).m()
}

/// Sólo los pesos que empiezan por `prefijo` (sin él): p. ej. la torre de visión.
pub(crate) fn cargar_pesos_de(dir: &Path, prefijo: &str) -> Result<HashMap<String, Array>> {
    let mut pesos = HashMap::new();
    for e in std::fs::read_dir(dir)?.filter_map(|e| e.ok()) {
        let a = e.path();
        if a.extension().is_some_and(|x| x == "safetensors") {
            for (k, v) in Array::load_safetensors(&a).map_err(|e| anyhow!("{}: {e:?}", a.display()))? {
                if let Some(r) = k.strip_prefix(prefijo) {
                    pesos.insert(r.to_string(), v);
                }
            }
        }
    }
    Ok(pesos)
}

/// Escribe K/V nuevos [1, kv, l, hd] en la caché reservada de una capa (crece por bloques de
/// `PASO`, en su lugar) y devuelve TODO lo guardado hasta `antes + l`.
pub(crate) fn escribir_kv(cc: &mut CacheCapa, k: &Array, v: &Array, antes: i32, l: i32) -> Result<(Array, Array)> {
    let despues = antes + l;
    let capacidad = cc.k.as_ref().map(|a| a.dim(2)).unwrap_or(0);
    if despues > capacidad {
        // (F8 de la memoria rápida, DESCARTADO con medición 2026-11-11: crecer por proporción —1/8 de
        // lo que ya hay— en vez de de a PASO no mostró ganancia medible con 25 000 tokens del 0.6B,
        // 20,9–22,7 s contra 21,0–21,6 s, mismo texto, y costaba hasta un 12,5 % más de memoria)
        let nueva = ((l + PASO - 1) / PASO) * PASO;
        let bk = ops::zeros::<f32>(&[1, k.dim(1), nueva, k.dim(3)]).m()?.as_dtype(k.dtype()).m()?;
        let bv = ops::zeros::<f32>(&[1, v.dim(1), nueva, v.dim(3)]).m()?.as_dtype(v.dtype()).m()?;
        match (cc.k.take(), cc.v.take()) {
            (Some(pk), Some(pv)) => {
                let pk = pk.try_index((.., .., ..antes, ..)).m()?;
                let pv = pv.try_index((.., .., ..antes, ..)).m()?;
                cc.k = Some(ops::concatenate(&[pk, bk], 2).m()?);
                cc.v = Some(ops::concatenate(&[pv, bv], 2).m()?);
            }
            _ => {
                cc.k = Some(bk);
                cc.v = Some(bv);
            }
        }
    }
    cc.k.as_mut().unwrap().try_index_mut((.., .., antes..despues, ..), k).m()?;
    cc.v.as_mut().unwrap().try_index_mut((.., .., antes..despues, ..), v).m()?;
    Ok((cc.k.as_ref().unwrap().try_index((.., .., ..despues, ..)).m()?, cc.v.as_ref().unwrap().try_index((.., .., ..despues, ..)).m()?))
}

impl Modelo {
    pub fn cargar(dir: &Path) -> Result<Modelo> {
        Self::cargar_con(dir, false)
    }

    /// M7 · con los pesos en coma flotante pasados a f32: TODO el cálculo va en f32, como el
    /// `LLMHandler` de ACE-Step en Mac (medido 2026-11-12: en bf16 los logits del CFG se alejan
    /// 4,5e-3 de la verdad f64 y la distribución de los códigos cambia un 10 % en variación total).
    pub fn cargar_f32(dir: &Path) -> Result<Modelo> {
        Self::cargar_con(dir, true)
    }

    fn cargar_con(dir: &Path, f32: bool) -> Result<Modelo> {
        let cfg = Config::leer(dir)?;
        if let Some(e) = por_que_no(&cfg) {
            bail!("{e}");
        }
        let mut pesos = cargar_pesos(dir)?;
        if f32 {
            for v in pesos.values_mut() {
                if matches!(v.dtype(), Dtype::Bfloat16 | Dtype::Float16) {
                    *v = v.as_dtype(Dtype::Float32).m()?;
                }
            }
        }
        if cfg.hibrido() {
            sanear_hibrido(&mut pesos)?;
        }
        let embed = Embedding { lineal: Lineal::cargar(&mut pesos, "model.embed_tokens", &cfg)? };
        let intervalo = cfg.full_attention_interval.unwrap_or(4).max(1);
        let mut capas = Vec::with_capacity(cfg.num_hidden_layers as usize);
        for i in 0..cfg.num_hidden_layers {
            let p = format!("model.layers.{i}");
            let lineal = cfg.hibrido() && (i + 1) % intervalo != 0;
            let mezcla = if lineal {
                let n = format!("{p}.linear_attn");
                Mezcla::Lineal(DeltaNet {
                    qkv: Lineal::cargar(&mut pesos, &format!("{n}.in_proj_qkv"), &cfg)?,
                    z: Lineal::cargar(&mut pesos, &format!("{n}.in_proj_z"), &cfg)?,
                    b: Lineal::cargar(&mut pesos, &format!("{n}.in_proj_b"), &cfg)?,
                    a: Lineal::cargar(&mut pesos, &format!("{n}.in_proj_a"), &cfg)?,
                    salida: Lineal::cargar(&mut pesos, &format!("{n}.out_proj"), &cfg)?,
                    conv: quitar(&mut pesos, &format!("{n}.conv1d.weight"))?,
                    a_log: quitar(&mut pesos, &format!("{n}.A_log"))?,
                    dt_bias: quitar(&mut pesos, &format!("{n}.dt_bias"))?,
                    norma: quitar(&mut pesos, &format!("{n}.norm.weight"))?,
                })
            } else {
                Mezcla::Atencion(Atencion {
                    q: Lineal::cargar(&mut pesos, &format!("{p}.self_attn.q_proj"), &cfg)?,
                    k: Lineal::cargar(&mut pesos, &format!("{p}.self_attn.k_proj"), &cfg)?,
                    v: Lineal::cargar(&mut pesos, &format!("{p}.self_attn.v_proj"), &cfg)?,
                    o: Lineal::cargar(&mut pesos, &format!("{p}.self_attn.o_proj"), &cfg)?,
                    q_norma: pesos.remove(&format!("{p}.self_attn.q_norm.weight")),
                    k_norma: pesos.remove(&format!("{p}.self_attn.k_norm.weight")),
                    compuerta: cfg.hibrido(),
                })
            };
            capas.push(Capa {
                norma_in: quitar(&mut pesos, &format!("{p}.input_layernorm.weight"))?,
                norma_post: quitar(&mut pesos, &format!("{p}.post_attention_layernorm.weight"))?,
                mezcla,
                mlp: if cfg.capa_moe(i) {
                    let m = format!("{p}.mlp");
                    Mlp::Moe(Box::new(Moe {
                        router: Lineal::cargar(&mut pesos, &format!("{m}.gate"), &cfg)?,
                        puerta: Conmutado::cargar(&mut pesos, &format!("{m}.switch_mlp.gate_proj"), &cfg)?,
                        arriba: Conmutado::cargar(&mut pesos, &format!("{m}.switch_mlp.up_proj"), &cfg)?,
                        abajo: Conmutado::cargar(&mut pesos, &format!("{m}.switch_mlp.down_proj"), &cfg)?,
                        compartido: (
                            Lineal::cargar(&mut pesos, &format!("{m}.shared_expert.gate_proj"), &cfg)?,
                            Lineal::cargar(&mut pesos, &format!("{m}.shared_expert.up_proj"), &cfg)?,
                            Lineal::cargar(&mut pesos, &format!("{m}.shared_expert.down_proj"), &cfg)?,
                        ),
                        compuerta_compartido: Lineal::cargar(&mut pesos, &format!("{m}.shared_expert_gate"), &cfg)?,
                        k: cfg.num_experts_per_tok.unwrap_or(8),
                        normalizar: cfg.norm_topk_prob.unwrap_or(true),
                    }))
                } else {
                    Mlp::Densa {
                        puerta: Lineal::cargar(&mut pesos, &format!("{p}.mlp.gate_proj"), &cfg)?,
                        arriba: Lineal::cargar(&mut pesos, &format!("{p}.mlp.up_proj"), &cfg)?,
                        abajo: Lineal::cargar(&mut pesos, &format!("{p}.mlp.down_proj"), &cfg)?,
                    }
                },
            });
        }
        let norma = quitar(&mut pesos, "model.norm.weight")?;
        let cabeza = if cfg.tie_word_embeddings || !pesos.contains_key("lm_head.weight") {
            None
        } else {
            Some(Lineal::cargar(&mut pesos, "lm_head", &cfg)?)
        };
        // qwen2/llama: q/k/v con sesgo propio (`.bias`) — aún no
        if pesos.keys().any(|k| k.ends_with("_proj.bias")) {
            bail_tr!("este modelo tiene sesgos en las proyecciones (qwen2): aún no en el núcleo nativo", "this model has projection biases (qwen2): not in the native core yet");
        }
        let head_dim = cfg.head_dim.unwrap_or(cfg.hidden_size / cfg.num_attention_heads);
        let rope_dims = ((head_dim as f32) * cfg.partial_rotary_factor.unwrap_or(1.0)) as i32;
        let recurrente = capas.iter().any(|c| matches!(c.mezcla, Mezcla::Lineal(_)));
        let kernel_delta = if recurrente { Some(kernel_delta()?) } else { None };
        let mrope = cfg
            .rope_parameters
            .as_ref()
            .and_then(|r| r.get("mrope_section"))
            .and_then(|m| serde_json::from_value::<Vec<i32>>(m.clone()).ok())
            .filter(|m| m.len() == 3)
            .map(|m| [m[0], m[1], m[2]])
            .unwrap_or([11, 11, 10]);
        let kernel_mrope = if cfg.hibrido() { Some(kernel_mrope(rope_dims)?) } else { None };
        let rope_periodos = cfg.periodos_llama3(rope_dims).map(|p| Array::from_slice(&p, &[p.len() as i32]));
        Ok(Modelo { cfg, embed, capas, norma, cabeza, head_dim, rope_dims, rope_periodos, recurrente, kernel_delta, f: Fusiones::nuevas(), mrope, kernel_mrope })
    }

    pub fn cache_nueva(&self) -> Cache {
        Cache { capas: (0..self.capas.len()).map(|_| CacheCapa::default()).collect(), largo: 0, instantaneas: Vec::new() }
    }

    /// Recorta la caché para reutilizar un prefijo común de `n` tokens y devuelve cuántos quedaron
    /// DE VERDAD: con capas recurrentes se vuelve a la última instantánea ≤ `n` (lo que falte hasta
    /// `n` hay que volver a pasarlo). La caché KV se recorta en O(1): lo de detrás se sobrescribe.
    pub fn recortar(&self, c: &mut Cache, n: i32) -> Result<i32> {
        if n <= 0 {
            *c = self.cache_nueva();
            return Ok(0);
        }
        if n >= c.largo {
            return Ok(c.largo);
        }
        if !self.recurrente {
            c.largo = n;
            return Ok(n);
        }
        let Some(i) = c.instantaneas.iter().rposition(|s| s.largo <= n) else {
            *c = self.cache_nueva();
            return Ok(0);
        };
        c.instantaneas.truncate(i + 1);
        let s = c.instantaneas[i].clone();
        for (cc, (conv, estado)) in c.capas.iter_mut().zip(s.capas) {
            cc.conv = conv;
            cc.estado = estado;
        }
        c.largo = s.largo;
        Ok(s.largo)
    }

    /// Guarda el estado recurrente en la posición actual (no hace nada en los modelos densos).
    pub fn marcar(&self, c: &mut Cache) {
        if !self.recurrente || c.largo == 0 || c.instantaneas.last().is_some_and(|s| s.largo == c.largo) {
            return;
        }
        let capas = c.capas.iter().map(|cc| (cc.conv.clone(), cc.estado.clone())).collect();
        c.instantaneas.push(Instantanea { largo: c.largo, capas });
        if c.instantaneas.len() > INSTANTANEAS {
            c.instantaneas.remove(cual_desalojar(&c.instantaneas.iter().map(|s| s.largo).collect::<Vec<_>>()));
        }
    }

    fn atencion(&self, at: &Atencion, h: &Array, l: i32, offset: i32, cc: &mut CacheCapa, giro: Option<&GiroM>) -> Result<Array> {
        let (nh, nkv, hd) = (self.cfg.num_attention_heads, self.cfg.num_key_value_heads, self.head_dim);
        let eps = self.cfg.rms_norm_eps;
        let escala = 1.0 / (hd as f32).sqrt();
        let qp = at.q.aplicar(h)?;
        let (mut q, compuerta) = if at.compuerta {
            // q_proj da [q | compuerta] POR CABEZA: [1, l, nh, 2·hd]
            let partes = ops::split_at_indices(ops::reshape(&qp, &[1, l, nh, 2 * hd]).m()?, &[hd], -1).m()?;
            (partes[0].clone(), Some(ops::reshape(&partes[1], &[1, l, nh * hd]).m()?))
        } else {
            (ops::reshape(&qp, &[1, l, nh, hd]).m()?, None)
        };
        let mut k = ops::reshape(at.k.aplicar(h)?, &[1, l, nkv, hd]).m()?;
        let vv = ops::reshape(at.v.aplicar(h)?, &[1, l, nkv, hd]).m()?;
        if let Some(qn) = &at.q_norma {
            q = fast::rms_norm(&q, Some(qn), eps).m()?;
        }
        if let Some(kn) = &at.k_norma {
            k = fast::rms_norm(&k, Some(kn), eps).m()?;
        }
        let q = ops::transpose_axes(&q, &[0, 2, 1, 3]).m()?;
        let k = ops::transpose_axes(&k, &[0, 2, 1, 3]).m()?;
        let vv = ops::transpose_axes(&vv, &[0, 2, 1, 3]).m()?;
        // RoPE parcial: con dims < head_dim, MLX gira sólo las primeras y deja el resto
        let (q, k) = match giro {
            // posiciones 3D explícitas (hay imágenes): x·cos + rotar_mitad(x)·sin sobre las primeras
            // `rope_dims`, en f32 y de vuelta al tipo de x (como el kernel fusionado de mlx_vlm)
            // posiciones 3D explícitas (hay imágenes): el MISMO kernel Metal de mlx_vlm (metal::cos/sin
            // dentro del kernel; con los cos/sin de MLX el último bit cambiaba y la descripción se separaba)
            Some(g) => (self.girar_mrope(&q, g)?, self.girar_mrope(&k, g)?),
            // RoPE «llama3»: los periodos ya ajustados (sin base, como `Llama3RoPE` de mlx_lm)
            None if self.rope_periodos.is_some() => (
                fast::rope(&q, self.rope_dims, false, None, 1.0, offset, self.rope_periodos.as_ref()).m()?,
                fast::rope(&k, self.rope_dims, false, None, 1.0, offset, self.rope_periodos.as_ref()).m()?,
            ),
            None => (
                fast::rope(&q, self.rope_dims, false, self.cfg.rope_theta, 1.0, offset, None::<&Array>).m()?,
                fast::rope(&k, self.rope_dims, false, self.cfg.rope_theta, 1.0, offset, None::<&Array>).m()?,
            ),
        };
        let (k, vv) = escribir_kv(cc, &k, &vv, offset, l)?;
        let mascara = if l > 1 { Some(ScaledDotProductAttentionMask::Causal) } else { None };
        let a = fast::scaled_dot_product_attention(&q, &k, &vv, escala, mascara, None::<&Array>).m()?;
        let mut a = ops::reshape(ops::transpose_axes(&a, &[0, 2, 1, 3]).m()?, &[1, l, nh * hd]).m()?;
        if let Some(g) = compuerta {
            a = (self.f.puerta.borrow_mut())((&a, &g)).m()?;
        }
        at.o.aplicar(&a)
    }

    fn delta(&self, d: &DeltaNet, h: &Array, l: i32, cc: &mut CacheCapa) -> Result<Array> {
        let c = &self.cfg;
        let (hk, hv) = (c.linear_num_key_heads.unwrap_or(16), c.linear_num_value_heads.unwrap_or(32));
        let (dk, dv) = (c.linear_key_head_dim.unwrap_or(128), c.linear_value_head_dim.unwrap_or(128));
        let kc = c.linear_conv_kernel_dim.unwrap_or(4);
        let (key_dim, value_dim) = (hk * dk, hv * dv);
        let conv_dim = 2 * key_dim + value_dim;
        let qkv = d.qkv.aplicar(h)?;
        let z = ops::reshape(d.z.aplicar(h)?, &[1, l, hv, dv]).m()?;
        let b = d.b.aplicar(h)?;
        let a = d.a.aplicar(h)?;
        // conv causal corta: lo que guardó el paso anterior + lo nuevo
        let previo = match cc.conv.take() {
            Some(p) => p,
            None => ops::zeros::<f32>(&[1, kc - 1, conv_dim]).m()?.as_dtype(qkv.dtype()).m()?,
        };
        let entrada = ops::concatenate(&[previo, qkv], 1).m()?;
        cc.conv = Some(entrada.try_index((.., -(kc - 1).., ..)).m()?.contiguous().m()?);
        let conv = (self.f.silu.borrow_mut())(&ops::conv1d(&entrada, &d.conv, 1, 0, 1, conv_dim).m()?).m()?;
        let partes = ops::split_at_indices(&conv, &[key_dim, 2 * key_dim], -1).m()?;
        let q = ops::reshape(&partes[0], &[1, l, hk, dk]).m()?;
        let k = ops::reshape(&partes[1], &[1, l, hk, dk]).m()?;
        let v = ops::reshape(&partes[2], &[1, l, hv, dv]).m()?;
        // q y k normalizados (sin peso) y escalados; el escalar con el dtype de q (un f32 suelto
        // promovería todo a f32, a diferencia del float de Python)
        let inv = (dk as f32).powf(-0.5);
        let esc = |x: f32, t: Dtype| Array::from_f32(x).as_dtype(t);
        let q = ops::multiply(fast::rms_norm(&q, None, 1e-6).m()?, esc(inv * inv, q.dtype()).m()?).m()?;
        let k = ops::multiply(fast::rms_norm(&k, None, 1e-6).m()?, esc(inv, k.dtype()).m()?).m()?;
        let beta = ops::sigmoid(&b).m()?;
        let g = (self.f.decaimiento.borrow_mut())((&d.a_log, &a, &d.dt_bias)).m()?;
        let estado = match cc.estado.take() {
            Some(e) => e,
            None => ops::zeros::<f32>(&[1, hv, dv, dk]).m()?,
        };
        let kernel = self.kernel_delta.as_ref().ok_or_else(|| anyhow_tr!("sin kernel delta", "no delta kernel"))?;
        let opciones = MetalKernelApplyOptions {
            template_args: vec![
                ("InT".into(), TemplateArg::Dtype(q.dtype())),
                ("StT".into(), TemplateArg::Dtype(Dtype::Float32)),
                ("Dk".into(), TemplateArg::Int(dk)),
                ("Dv".into(), TemplateArg::Int(dv)),
                ("Hk".into(), TemplateArg::Int(hk)),
                ("Hv".into(), TemplateArg::Int(hv)),
            ],
            ..Default::default()
        };
        let salidas = [
            MetalKernelOutput { shape: vec![1, l, hv, dv], dtype: q.dtype() },
            MetalKernelOutput { shape: vec![1, hv, dv, dk], dtype: Dtype::Float32 },
        ];
        let t = Array::from_int(l);
        // SAFETY: el mismo kernel que `mlx_lm/models/gated_delta.py`. Rejilla (32, Dv, Hv) con grupos
        // (32, 4, 1): cada hilo lee Dk/32 posiciones de su fila de estado y de q/k [1,T,Hk,Dk],
        // v [1,T,Hv,Dv], g y beta [1,T,Hv] (todas contiguas: `ensure_row_contiguous`), y escribe
        // y[t, hv, dv] (un hilo por simdgroup) y su trozo de state_out, sin solaparse.
        let mut r = unsafe { kernel.apply(&[&q, &k, &v, &g, &beta, &estado, &t], &salidas, (32, dv, hv), (32, 4, 1), opciones).m()? };
        let nuevo = r.pop().ok_or_else(|| anyhow_tr!("kernel sin estado", "kernel without state"))?;
        let y = r.pop().ok_or_else(|| anyhow_tr!("kernel sin salida", "kernel without output"))?;
        cc.estado = Some(nuevo);
        // RMSNorm con compuerta: silu(z) · norma(y), en f32 y de vuelta al dtype de y
        let x = fast::rms_norm(&y, Some(&d.norma), c.rms_norm_eps).m()?;
        let o = (self.f.norma_puerta.borrow_mut())((&z, &x)).m()?;
        d.salida.aplicar(&ops::reshape(&o, &[1, l, value_dim]).m()?)
    }

    /// La mezcla de expertos sobre `x` [1, l, D] (como `SwitchGLU` + `Qwen3NextSparseMoeBlock`).
    fn expertos(&self, moe: &Moe, x: &Array, l: i32) -> Result<Array> {
        let k = moe.k;
        let probs = ops::softmax_axis(moe.router.aplicar(x)?, -1, true).m()?;
        let elegidos = ops::argpartition_axis(&probs, -k, -1).m()?.try_index((.., .., -k..)).m()?;
        let mut pesos = probs.take_along_axis(&elegidos, -1).m()?;
        if moe.normalizar {
            pesos = ops::divide(&pesos, pesos.sum_axis(-1, true).m()?).m()?;
        }
        let xe = x.expand_dims_axes(&[-2, -3]).m()?;
        let d = self.cfg.hidden_size;
        // muchos tokens (prellenado): se ORDENAN por experto para leer cada matriz una sola vez
        let y = if l * k >= 64 {
            let plano = elegidos.flatten(None, None).m()?;
            let orden = ops::argsort_axis(&plano, 0).m()?;
            let inverso = ops::argsort_axis(&orden, 0).m()?;
            let fila = ops::floor_divide(&orden, Array::from_int(k).as_dtype(orden.dtype()).m()?).m()?;
            let xs = take_axis(&xe.flatten(0, -3).m()?, &fila, 0).m()?;
            let idx = take_axis(&plano, &orden, 0).m()?;
            let up = moe.arriba.aplicar(&xs, &idx, true)?;
            let g = moe.puerta.aplicar(&xs, &idx, true)?;
            let h = (self.f.swiglu.borrow_mut())((&g, &up)).m()?;
            let y = moe.abajo.aplicar(&h, &idx, true)?;
            ops::reshape(take_axis(&y, &inverso, 0).m()?, &[1, l, k, d]).m()?
        } else {
            let up = moe.arriba.aplicar(&xe, &elegidos, false)?;
            let g = moe.puerta.aplicar(&xe, &elegidos, false)?;
            let h = (self.f.swiglu.borrow_mut())((&g, &up)).m()?;
            ops::reshape(moe.abajo.aplicar(&h, &elegidos, false)?, &[1, l, k, d]).m()?
        };
        let y = ops::multiply(&y, pesos.expand_dims(-1).m()?.as_dtype(y.dtype()).m()?).m()?.sum_axis(-2, false).m()?;
        let (cp, ca, cb) = &moe.compartido;
        let compartido = cb.aplicar(&(self.f.swiglu.borrow_mut())((&cp.aplicar(x)?, &ca.aplicar(x)?)).m()?)?;
        let compartido = (self.f.puerta.borrow_mut())((&compartido, &moe.compuerta_compartido.aplicar(x)?)).m()?;
        ops::add(&y, &compartido).m()
    }

    /// Pasa `ids` (tokens nuevos) por el modelo con la caché y devuelve los logits del ÚLTIMO.
    pub fn adelante(&self, ids: &[u32], c: &mut Cache) -> Result<Array> {
        self.adelante_ext(ids, &Extra::default(), c)
    }

    /// Las entradas del kernel del mRoPE ENTRELAZADO para estas posiciones: posiciones [3, 1, l]
    /// (t, alto, ancho), frecuencias y el selector (la frecuencia j toma alto si j ≡ 1 mód 3, ancho
    /// si j ≡ 2, tiempo si no; `_interleaved_position_selector` de mlx_vlm).
    fn mrope(&self, pos: &[Vec<i32>; 3]) -> Result<GiroM> {
        let rd = self.rope_dims;
        let mitad = rd / 2;
        let sel: Vec<i32> = (0..mitad)
            .map(|j| {
                if j % 3 == 1 && j < self.mrope[1] * 3 {
                    1
                } else if j % 3 == 2 && j < self.mrope[2] * 3 {
                    2
                } else {
                    0
                }
            })
            .collect();
        let l = pos[0].len() as i32;
        let todas: Vec<i32> = pos.iter().flatten().copied().collect();
        Ok(GiroM { pos: Array::from_slice(&todas, &[3, 1, l]), inv: inv_frec(rd, self.cfg.rope_theta)?, sel: Array::from_slice(&sel, &[mitad]) })
    }

    /// Gira q o k [1, cabezas, l, d] con el kernel del mRoPE entrelazado.
    fn girar_mrope(&self, x: &Array, g: &GiroM) -> Result<Array> {
        let kernel = self.kernel_mrope.as_ref().ok_or_else(|| anyhow_tr!("este modelo no tiene mRoPE", "this model has no mRoPE"))?;
        let forma = x.shape().to_vec();
        let ranuras = self.rope_dims / 2 + forma[3] - self.rope_dims;
        let trabajo = forma[0] * forma[1] * forma[2] * ranuras;
        let opciones = MetalKernelApplyOptions { template_args: vec![("T".into(), TemplateArg::Dtype(x.dtype()))], ..Default::default() };
        // SAFETY: el kernel de `mlx_vlm/models/rope_utils.py`: un hilo por (lote, cabeza, token,
        // ranura); cada uno lee su par de x (contiguo por `ensure_row_contiguous`) y la posición y
        // frecuencia de su ranura, y escribe SÓLO su par (o su dimensión sin girar) en x_out.
        let mut r = unsafe {
            kernel
                .apply(&[x, &g.pos, &g.inv, &g.sel], &[MetalKernelOutput { shape: forma.clone(), dtype: x.dtype() }], (trabajo, 1, 1), (256, 1, 1), opciones)
                .m()?
        };
        r.pop().ok_or_else(|| anyhow_tr!("kernel mRoPE sin salida", "mRoPE kernel without output"))
    }

    /// Con tokens visuales y/o posiciones mRoPE explícitas (M8); sin ellos, como siempre.
    pub fn adelante_ext(&self, ids: &[u32], extra: &Extra, c: &mut Cache) -> Result<Array> {
        let l = ids.len() as i32;
        let x = self.tronco(ids, extra, c)?;
        // sólo el último token importa para generar
        let ultimo = take_axis(&x, Array::from_slice(&[l - 1], &[1]), 1).m()?;
        let ultimo = fast::rms_norm(&ultimo, Some(&self.norma), self.cfg.rms_norm_eps).m()?;
        let logits = match &self.cabeza {
            Some(c) => c.aplicar(&ultimo)?,
            None => self.embed.lineal.aplicar(&ultimo)?,
        };
        ops::reshape(&logits, &[-1]).m()
    }

    /// M11 · el vector de un texto como `qwen3_embedding` de mlx_vlm: el modelo entero (máscara
    /// causal, sin caché previa) → norma final → el ÚLTIMO token → L2 (`normalize_embeddings`).
    pub fn incrustar(&self, ids: &[u32]) -> Result<Array> {
        let l = ids.len() as i32;
        let mut c = self.cache_nueva();
        let x = self.tronco(ids, &Extra::default(), &mut c)?;
        let ultimo = take_axis(&x, Array::from_slice(&[l - 1], &[1]), 1).m()?;
        let e = ops::reshape(fast::rms_norm(&ultimo, Some(&self.norma), self.cfg.rms_norm_eps).m()?, &[1, -1]).m()?;
        let n = mlx_rs::linalg::norm(&e, 2.0, mlx_rs::linalg::NormOptions { axes: (-1).into(), keep_dims: true }).m()?;
        let n = ops::maximum(&n, Array::from_f32(1e-9).as_dtype(n.dtype()).m()?).m()?;
        ops::reshape(ops::divide(&e, &n).m()?, &[-1]).m()
    }

    /// Las capas sobre los tokens nuevos: el estado de TODOS [1, l, D] antes de la norma final.
    fn tronco(&self, ids: &[u32], extra: &Extra, c: &mut Cache) -> Result<Array> {
        let l = ids.len() as i32;
        let v: Vec<u32> = ids.to_vec();
        let ids_a = Array::from_slice(&v, &[1, l]);
        let mut x = self.embed.filas(&ids_a)?;
        // los tokens de imagen reciben su rasgo visual en lugar del embedding del token
        if let Some((rasgos, cual)) = &extra.rasgos {
            if cual.len() != l as usize {
                bail_tr!("rasgos visuales para {} tokens y el trozo tiene {l}", "visual features for {} tokens and the chunk has {l}", cual.len());
            }
            if cual.iter().any(|&i| i >= 0) {
                let idx: Vec<i32> = cual.iter().map(|&i| i.max(0)).collect();
                let mascara: Vec<bool> = cual.iter().map(|&i| i >= 0).collect();
                let filas = take_axis(rasgos, Array::from_slice(&idx, &[l]), 0).m()?.as_dtype(x.dtype()).m()?;
                let filas = ops::reshape(&filas, &[1, l, -1]).m()?;
                let m = Array::from_slice(&mascara, &[1, l, 1]);
                x = ops::select(&m, &filas, &x).m()?;
            }
        }
        let giro = match &extra.pos {
            Some(p) if p[0].len() == l as usize => Some(self.mrope(p)?),
            Some(_) => bail!("posiciones mRoPE de otro largo"),
            None => None,
        };
        let eps = self.cfg.rms_norm_eps;
        let offset = c.largo;
        for (capa, cc) in self.capas.iter().zip(c.capas.iter_mut()) {
            let h = fast::rms_norm(&x, Some(&capa.norma_in), eps).m()?;
            let r = match &capa.mezcla {
                Mezcla::Atencion(at) => self.atencion(at, &h, l, offset, cc, giro.as_ref())?,
                Mezcla::Lineal(d) => self.delta(d, &h, l, cc)?,
            };
            x = ops::add(&x, &r).m()?;
            let h = fast::rms_norm(&x, Some(&capa.norma_post), eps).m()?;
            let m = match &capa.mlp {
                Mlp::Densa { puerta, arriba, abajo } => abajo.aplicar(&(self.f.swiglu.borrow_mut())((&puerta.aplicar(&h)?, &arriba.aplicar(&h)?)).m()?)?,
                Mlp::Moe(moe) => self.expertos(moe, &h, l)?,
            };
            x = ops::add(&x, &m).m()?;
        }
        c.largo += l;
        Ok(x)
    }

    /// M7 · `last_hidden_state` de transformers: todas las capas (causal, sin relleno) y la norma
    /// final, en f32 (el Qwen3-Embedding con el que ACE-Step lee su descripción).
    pub fn estado_final(&self, ids: &[u32]) -> Result<Array> {
        let n = self.capas.len();
        let x = self.ocultos(ids, ids.len(), &[n], true)?.pop().ok_or_else(|| anyhow_tr!("estado_final: sin salida", "estado_final: no output"))?;
        let xf = x.as_dtype(Dtype::Float32).m()?;
        let var = ops::mean_axes(&ops::square(&xf).m()?, &[-1], true).m()?;
        let y = ops::multiply(&xf, &ops::rsqrt(&ops::add(&var, Array::from_f32(self.cfg.rms_norm_eps)).m()?).m()?).m()?;
        ops::multiply(&y, &self.norma.as_dtype(Dtype::Float32).m()?).m()
    }

    /// M7 · las filas de la tabla de embeddings (ACE-Step lee así la letra: sin capas), en f32.
    pub fn filas_embedding(&self, ids: &[u32]) -> Result<Array> {
        self.embed.filas(&Array::from_slice(ids, &[1, ids.len() as i32]))?.as_dtype(Dtype::Float32).m()
    }

    /// M4 · el modelo como CODIFICADOR de texto (FLUX.2 klein lee así su Qwen3): todo el texto de
    /// una vez, sin caché, con las claves de relleno tapadas (`attention_mask` de transformers:
    /// causal Y clave real) y devuelve el estado DESPUÉS de cada capa pedida (`hidden_states[k]` =
    /// tras k capas, sin la norma final). Las cuentas van como transformers en bf16, redondeo a
    /// redondeo: RMSNorm en f32 → tipo de x → × peso; RoPE con cos/sin de f32 pasados a bf16 y
    /// `q·cos + rotar_mitad(q)·sin` en bf16. Con `f32` TODO va en f32 (los pesos bf16 se promueven
    /// operación a operación, sin copiar los 8 GB): más cerca de la verdad que transformers en bf16.
    /// Sólo modelos densos de atención completa.
    pub fn ocultos(&self, ids: &[u32], reales: usize, capas: &[usize], f32: bool) -> Result<Vec<Array>> {
        let l = ids.len() as i32;
        let hasta = capas.iter().copied().max().unwrap_or(0);
        if hasta == 0 || hasta > self.capas.len() || reales == 0 || reales > ids.len() {
            bail!("ocultos: capas {capas:?} de {} y {reales} tokens reales de {l}", self.capas.len());
        }
        let (nh, nkv, hd) = (self.cfg.num_attention_heads, self.cfg.num_key_value_heads, self.head_dim);
        if self.rope_dims != hd {
            bail!("ocultos: RoPE parcial no soportado");
        }
        let eps = self.cfg.rms_norm_eps;
        let tipo = if f32 { Dtype::Float32 } else { self.norma.dtype() };
        // RoPE de transformers: inv_freq = 1/θ^(2i/d) en f32, ángulo = pos·inv_freq en f32,
        // emb = [ángulos, ángulos], cos/sin en f32 → tipo del modelo
        let mitad = (hd / 2) as usize;
        let inv: Vec<f32> = match self.cfg.periodos_llama3(hd) {
            Some(p) => p.iter().map(|x| 1.0 / x).collect(),
            None => (0..mitad).map(|i| 1.0 / self.cfg.rope_theta.powf((2 * i) as f32 / hd as f32)).collect(),
        };
        let (mut cos, mut sin) = (Vec::with_capacity(l as usize * hd as usize), Vec::with_capacity(l as usize * hd as usize));
        for p in 0..l {
            for _ in 0..2 {
                for f in &inv {
                    let a = p as f32 * f;
                    cos.push(a.cos());
                    sin.push(a.sin());
                }
            }
        }
        let cos = Array::from_slice(&cos, &[1, 1, l, hd]).as_dtype(tipo).m()?;
        let sin = Array::from_slice(&sin, &[1, 1, l, hd]).as_dtype(tipo).m()?;
        let girar = |x: &Array| -> Result<Array> {
            let partes = ops::split_equal(x, 2, -1).m()?;
            let rot = ops::concatenate(&[ops::negative(&partes[1]).m()?, partes[0].clone()], -1).m()?;
            ops::add(&ops::multiply(x, &cos).m()?, &ops::multiply(&rot, &sin).m()?).m()
        };
        // la norma de transformers: varianza en f32, x·rsqrt en f32, → tipo de x, × peso
        let rms = |x: &Array, w: &Array| -> Result<Array> {
            let xf = x.as_dtype(Dtype::Float32).m()?;
            let var = ops::mean_axes(&ops::square(&xf).m()?, &[-1], true).m()?;
            let n = ops::multiply(&xf, &ops::rsqrt(&ops::add(&var, Array::from_f32(eps)).m()?).m()?).m()?;
            ops::multiply(&n.as_dtype(x.dtype()).m()?, w).m()
        };
        // máscara: la consulta i mira la clave j si j ≤ i y j es un token real
        let mut m = vec![false; (l * l) as usize];
        for i in 0..l as usize {
            for j in 0..=i.min(reales - 1) {
                m[i * l as usize + j] = true;
            }
        }
        let mascara = Array::from_slice(&m, &[1, 1, 1, l, l]);
        let escala = 1.0 / (hd as f32).sqrt();
        let mut x = self.embed.filas(&Array::from_slice(ids, &[1, l]))?.as_dtype(tipo).m()?;
        let mut salida = Vec::with_capacity(capas.len());
        for (i, capa) in self.capas.iter().take(hasta).enumerate() {
            let Mezcla::Atencion(at) = &capa.mezcla else { bail_tr!("ocultos: capa lineal (híbrido) no soportada", "ocultos: linear (hybrid) layer not supported") };
            if at.compuerta {
                bail_tr!("ocultos: atención con compuerta no soportada", "ocultos: gated attention not supported");
            }
            let h = rms(&x, &capa.norma_in)?;
            let mut q = ops::reshape(at.q.aplicar(&h)?, &[1, l, nh, hd]).m()?;
            let mut k = ops::reshape(at.k.aplicar(&h)?, &[1, l, nkv, hd]).m()?;
            let v = ops::reshape(at.v.aplicar(&h)?, &[1, l, nkv, hd]).m()?;
            if let Some(qn) = &at.q_norma {
                q = rms(&q, qn)?;
            }
            if let Some(kn) = &at.k_norma {
                k = rms(&k, kn)?;
            }
            let q = girar(&ops::transpose_axes(&q, &[0, 2, 1, 3]).m()?)?;
            let k = girar(&ops::transpose_axes(&k, &[0, 2, 1, 3]).m()?)?;
            let v = ops::transpose_axes(&v, &[0, 2, 1, 3]).m()?;
            // atención en f32 (512 tokens: barato). Medido contra la verdad fp32: con el SDPA de bf16 el
            // texto se alejaba 1,2× más que la referencia oficial
            let g = nh / nkv;
            let qf = ops::reshape(&q.as_dtype(Dtype::Float32).m()?, &[1, nkv, g, l, hd]).m()?;
            let kf = ops::expand_dims(&k.as_dtype(Dtype::Float32).m()?, 2).m()?;
            let vf = ops::expand_dims(&v.as_dtype(Dtype::Float32).m()?, 2).m()?;
            let s = ops::multiply(&ops::matmul(&qf, &ops::transpose_axes(&kf, &[0, 1, 2, 4, 3]).m()?).m()?, Array::from_f32(escala)).m()?;
            let s = ops::select(&mascara, &s, Array::from_f32(f32::NEG_INFINITY)).m()?;
            let a = ops::matmul(&ops::softmax_axis(&s, -1, true).m()?, &vf).m()?;
            let a = ops::reshape(&a, &[1, nh, l, hd]).m()?.as_dtype(q.dtype()).m()?;
            let a = ops::reshape(ops::transpose_axes(&a, &[0, 2, 1, 3]).m()?, &[1, l, nh * hd]).m()?;
            x = ops::add(&x, &at.o.aplicar(&a)?).m()?;
            let h = rms(&x, &capa.norma_post)?;
            let r = match &capa.mlp {
                Mlp::Densa { puerta, arriba, abajo } => {
                    // SiLU de torch: en f32 y un solo redondeo; luego × arriba en bf16
                    let g = puerta.aplicar(&h)?;
                    let gf = g.as_dtype(Dtype::Float32).m()?;
                    let s = ops::multiply(&gf, &ops::sigmoid(&gf).m()?).m()?.as_dtype(g.dtype()).m()?;
                    abajo.aplicar(&ops::multiply(&s, &arriba.aplicar(&h)?).m()?)?
                }
                Mlp::Moe(moe) => self.expertos(moe, &h, l)?,
            };
            x = ops::add(&x, &r).m()?;
            if capas.contains(&(i + 1)) {
                salida.push(x.clone());
            }
        }
        Ok(salida)
    }
}

/// Pesos del híbrido tal como salen de transformers (sin pasar por `mlx_lm.convert`): la conv va
/// [C, 1, K] → [C, K, 1] y las normas guardan `peso − 1` (Qwen3.5 normaliza con 1 + peso). Los de
/// mlx-community ya vienen saneados: no se tocan.
///
/// Las normas se corrigen según lo que VALEN, no según lo que traiga el archivo: `mlx_lm` las corre
/// si ve capas `mtp`, y el Fable 27B oQ4e trae `mtp` CON las normas ya corridas (media 0,97, igual
/// que el 9B) → en `mlx_lm` sale basura (medido 2026-11-10). Crudas valen ≈ 0; corridas, ≈ 1.
fn sanear_hibrido(p: &mut HashMap<String, Array>) -> Result<()> {
    p.retain(|k, _| !k.starts_with("mtp.") && !k.contains(".mtp."));
    // MoE de transformers sin convertir: `experts.gate_up_proj` fusionado → gate/up apilados
    let fusionados: Vec<String> = p.keys().filter(|k| k.ends_with(".mlp.experts.gate_up_proj")).cloned().collect();
    for k in fusionados {
        let gu = p.remove(&k).unwrap();
        let base = k.trim_end_matches(".experts.gate_up_proj");
        let mitad = gu.dim(-2) / 2;
        p.insert(format!("{base}.switch_mlp.gate_proj.weight"), gu.try_index((.., ..mitad, ..)).m()?);
        p.insert(format!("{base}.switch_mlp.up_proj.weight"), gu.try_index((.., mitad.., ..)).m()?);
        if let Some(d) = p.remove(&format!("{base}.experts.down_proj")) {
            p.insert(format!("{base}.switch_mlp.down_proj.weight"), d);
        }
    }
    for v in p.iter_mut().filter(|(k, v)| k.ends_with("conv1d.weight") && v.shape().last() != Some(&1)).map(|(_, v)| v) {
        *v = ops::swap_axes(&*v, 1, 2).m()?;
    }
    let muestra = p.get("model.layers.0.input_layernorm.weight").ok_or_else(|| anyhow!("falta model.layers.0.input_layernorm"))?;
    let media = muestra.as_dtype(Dtype::Float32).m()?.mean(None).m()?.try_item_cast::<f32>().m()?;
    if media.abs() >= 0.5 {
        return Ok(());
    }
    let normas = [".input_layernorm.weight", ".post_attention_layernorm.weight", "model.norm.weight", ".q_norm.weight", ".k_norm.weight"];
    for (k, v) in p.iter_mut() {
        if normas.iter().any(|s| k.ends_with(s)) && v.ndim() == 1 {
            *v = ops::add(&*v, Array::from_f32(1.0).as_dtype(v.dtype()).m()?).m()?;
        }
    }
    Ok(())
}

/// El mRoPE entrelazado en Metal: el MISMO kernel de `mlx_vlm` (`_mrope_apply_kernel`, emparejado
/// por mitades, posiciones 3D; MIT, © 2025 Prince Canuma). Gira las primeras `rd` dimensiones.
fn kernel_mrope(rd: i32) -> Result<MetalKernel> {
    let fuente = format!(
        r#"
        uint elem = thread_position_in_grid.x;
        const int half_dim = {mitad};
        const int q_bsz = x_shape[0];
        const int q_heads = x_shape[1];
        const int q_len = x_shape[2];
        const int q_dim = x_shape[3];
        const int slots = half_dim + q_dim - {rd};
        const int work_size = q_bsz * q_heads * q_len * slots;
        if (elem >= uint(work_size)) {{
            return;
        }}
        int local = int(elem);
        int slot = local % slots;
        int tmp = local / slots;
        int t = tmp % q_len;
        tmp = tmp / q_len;
        int h = tmp % q_heads;
        int b = tmp / q_heads;
        int base = ((b * q_heads + h) * q_len + t) * q_dim;
        if (slot >= half_dim) {{
            int pass_d = {rd} + slot - half_dim;
            int pass_idx = base + pass_d;
            x_out[pass_idx] = x[pass_idx];
            return;
        }}
        int freq_idx = slot;
        int d = freq_idx;
        int pair_d = d + half_dim;
        int axis = int(position_selector[freq_idx]);
        float pos = static_cast<float>(position_ids[(axis * q_bsz + b) * q_len + t]);
        float angle = pos * static_cast<float>(inv_freq[freq_idx]);
        float c = metal::cos(angle);
        float s = metal::sin(angle);
        int idx = base + d;
        float xv = static_cast<float>(x[idx]);
        float xp = static_cast<float>(x[base + pair_d]);
        x_out[idx] = static_cast<T>(xv * c - xp * s);
        x_out[base + pair_d] = static_cast<T>(xp * c + xv * s);
    "#,
        mitad = rd / 2,
        rd = rd
    );
    MetalKernel::new(
        &format!("gotv_mrope_{rd}"),
        &["x", "position_ids", "inv_freq", "position_selector"],
        &["x_out"],
        &fuente,
        MetalKernelOptions { ensure_row_contiguous: true, ..Default::default() },
    )
    .m()
}

/// La recurrencia Gated DeltaNet en Metal: el MISMO kernel que `mlx_lm/models/gated_delta.py`
/// (compuerta escalar por cabeza, sin máscara: un solo lote). Por cada cabeza de valor y fila Dv,
/// 32 hilos se reparten Dk; en cada token: estado·g, memoria = estado·k, delta = (v − memoria)·β,
/// estado += k·delta, y = estado·q.
fn kernel_delta() -> Result<MetalKernel> {
    let fuente = r#"
        auto n = thread_position_in_grid.z;
        auto b_idx = n / Hv;
        auto hv_idx = n % Hv;
        auto hk_idx = hv_idx / (Hv / Hk);
        constexpr int n_per_t = Dk / 32;

        auto q_ = q + b_idx * T * Hk * Dk + hk_idx * Dk;
        auto k_ = k + b_idx * T * Hk * Dk + hk_idx * Dk;
        auto v_ = v + b_idx * T * Hv * Dv + hv_idx * Dv;
        y += b_idx * T * Hv * Dv + hv_idx * Dv;

        auto dk_idx = thread_position_in_threadgroup.x;
        auto dv_idx = thread_position_in_grid.y;

        auto i_state = state_in + (n * Dv + dv_idx) * Dk;
        auto o_state = state_out + (n * Dv + dv_idx) * Dk;

        float state[n_per_t];
        for (int i = 0; i < n_per_t; ++i) {
          auto s_idx = n_per_t * dk_idx + i;
          state[i] = static_cast<float>(i_state[s_idx]);
        }

        auto g_ = g + b_idx * T * Hv;
        auto beta_ = beta + b_idx * T * Hv;

        for (int t = 0; t < T; ++t) {
          float kv_mem = 0.0f;
          for (int i = 0; i < n_per_t; ++i) {
            auto s_idx = n_per_t * dk_idx + i;
            state[i] = state[i] * g_[hv_idx];
            kv_mem += state[i] * k_[s_idx];
          }
          kv_mem = simd_sum(kv_mem);

          auto delta = (v_[dv_idx] - kv_mem) * beta_[hv_idx];

          float out = 0.0f;
          for (int i = 0; i < n_per_t; ++i) {
            auto s_idx = n_per_t * dk_idx + i;
            state[i] = state[i] + k_[s_idx] * delta;
            out += state[i] * q_[s_idx];
          }
          out = simd_sum(out);
          if (thread_index_in_simdgroup == 0) {
            y[dv_idx] = static_cast<InT>(out);
          }
          q_ += Hk * Dk;
          k_ += Hk * Dk;
          v_ += Hv * Dv;
          y += Hv * Dv;
          g_ += Hv;
          beta_ += Hv;
        }
        for (int i = 0; i < n_per_t; ++i) {
          auto s_idx = n_per_t * dk_idx + i;
          o_state[s_idx] = static_cast<StT>(state[i]);
        }
    "#;
    MetalKernel::new(
        "gotv_gated_delta_step",
        &["q", "k", "v", "g", "beta", "state_in", "T"],
        &["y", "state_out"],
        fuente,
        MetalKernelOptions { ensure_row_contiguous: true, ..Default::default() },
    )
    .m()
}

#[cfg(test)]
mod tests {
    /// M11 · dentro de la capa 0 contra mlx_vlm (etapas de `capa0.safetensors`).
    #[test]
    #[ignore]
    fn etapas_capa0_del_embebedor() {
        use super::*;
        let (Ok(d), Ok(r)) = (std::env::var("GOTV_EMBED_DIR"), std::env::var("GOTV_REF_CAPA0")) else { return };
        let dir = Path::new(&d);
        let rs = Array::load_safetensors(&r).unwrap();
        let tok = tokenizers::Tokenizer::from_file(dir.join("tokenizer.json")).unwrap();
        let ids = tok.encode("fn check_credentials(user: &str, pass: &str) -> bool { verify_hash(pass, &load_hash(user)) }", true).unwrap().get_ids().to_vec();
        let m = Modelo::cargar(dir).unwrap();
        let l = ids.len() as i32;
        let x = m.embed.filas(&Array::from_slice(&ids, &[1, l])).unwrap();
        let c0 = &m.capas[0];
        let n = fast::rms_norm(&x, Some(&c0.norma_in), m.cfg.rms_norm_eps).unwrap();
        let Mezcla::Atencion(at) = &c0.mezcla else { panic!() };
        let q = at.q.aplicar(&n).unwrap();
        let mut c = m.cache_nueva();
        let a = m.atencion(at, &n, l, 0, &mut c.capas[0], None).unwrap();
        let h = ops::add(&x, &a).unwrap();
        let hn = fast::rms_norm(&h, Some(&c0.norma_post), m.cfg.rms_norm_eps).unwrap();
        let Mlp::Densa { puerta, arriba, abajo } = &c0.mlp else { panic!() };
        let mlp = abajo.aplicar(&(m.f.swiglu.borrow_mut())((&puerta.aplicar(&hn).unwrap(), &arriba.aplicar(&hn).unwrap())).unwrap()).unwrap();
        for (nombre, v) in [("norma_in", &n), ("q", &q), ("atencion", &a), ("h", &h), ("mlp", &mlp)] {
            let v = v.as_dtype(Dtype::Float32).unwrap();
            let (p, q) = (v.as_slice::<f32>(), rs[nombre].as_slice::<f32>());
            let dif = p.iter().zip(q).map(|(a, b)| (a - b).abs()).fold(0.0, f32::max);
            println!("{nombre}: dif máx {dif} · iguales {}/{}", p.iter().zip(q).filter(|(a, b)| a == b).count(), p.len());
        }
    }

    /// M11 · el estado capa a capa contra mlx_vlm (`ref_embed_capas.py`), recortando el modelo.
    /// `GOTV_EMBED_DIR=<dir> GOTV_REF_CAPAS=<capas.safetensors> cargo test -p gotv-mlx --release --
    /// --ignored --nocapture capas_del_embebedor`
    #[test]
    #[ignore]
    fn capas_del_embebedor() {
        use super::*;
        let (Ok(d), Ok(r)) = (std::env::var("GOTV_EMBED_DIR"), std::env::var("GOTV_REF_CAPAS")) else { return };
        let dir = Path::new(&d);
        let rs = Array::load_safetensors(&r).unwrap();
        let tok = tokenizers::Tokenizer::from_file(dir.join("tokenizer.json")).unwrap();
        let ids = tok.encode("fn check_credentials(user: &str, pass: &str) -> bool { verify_hash(pass, &load_hash(user)) }", true).unwrap().get_ids().to_vec();
        for k in [0usize, 1, 2, 4, 14, 28] {
            let mut m = Modelo::cargar(dir).unwrap();
            m.capas.truncate(k);
            let mut c = m.cache_nueva();
            let x = m.tronco(&ids, &Extra::default(), &mut c).unwrap().as_dtype(Dtype::Float32).unwrap();
            let b = &rs[&if k == 0 { "emb".to_string() } else { format!("capa{}", k - 1) }];
            let (a, b) = (x.as_slice::<f32>(), b.as_slice::<f32>());
            let dif = a.iter().zip(b).map(|(p, q)| (p - q).abs()).fold(0.0, f32::max);
            let iguales = a.iter().zip(b).filter(|(p, q)| p == q).count();
            println!("capa {k}: dif máx {dif} · iguales {iguales}/{}", a.len());
        }
    }

    use super::*;

    #[test]
    fn la_cuantizacion_mixta_se_lee_por_capa() {
        let c: Config = serde_json::from_str(r#"{"model_type":"qwen3","hidden_size":8,"num_hidden_layers":1,"num_attention_heads":1,"num_key_value_heads":1,
            "quantization":{"group_size":64,"bits":4,"model.layers.0.mlp.down_proj":{"group_size":64,"bits":6}}}"#).unwrap();
        assert_eq!(c.cuantizacion("model.layers.0.mlp.down_proj"), Some(Cuantizacion { group_size: 64, bits: 6 }));
        assert_eq!(c.cuantizacion("model.layers.0.mlp.up_proj"), Some(Cuantizacion { group_size: 64, bits: 4 }));
        let sin: Config = serde_json::from_str(r#"{"model_type":"qwen3","hidden_size":8,"num_hidden_layers":1,"num_attention_heads":1,"num_key_value_heads":1}"#).unwrap();
        assert_eq!(sin.cuantizacion("x"), None);
    }

    fn argmax(l: &Array) -> u32 {
        ops::indexing::argmax_axis(l, -1, false).unwrap().try_item_cast::<u32>().unwrap()
    }

    /// Juez numérico contra `mlx_lm` (Python) con el MISMO modelo y los MISMOS tokens:
    /// `GOTV_MLX_DIR=<modelo> GOTV_REF=<ref.json de ref_qwen35.py> cargo test -p gotv-mlx --release
    /// -- --ignored --nocapture coincide_con_mlx_lm`. Exige el mismo top-5, logits a ±0,5 (bf16) y
    /// los 40 tokens voraces idénticos; luego vuelve a una instantánea y debe repetir lo mismo.
    #[test]
    #[ignore]
    fn coincide_con_mlx_lm() {
        let (Ok(dir), Ok(r)) = (std::env::var("GOTV_MLX_DIR"), std::env::var("GOTV_REF")) else { return };
        let r: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(r).unwrap()).unwrap();
        let ids: Vec<u32> = r["ids"].as_array().unwrap().iter().map(|x| x.as_u64().unwrap() as u32).collect();
        let t0 = std::time::Instant::now();
        let m = cargar_red(Path::new(&dir)).unwrap();
        println!("carga {:?}", t0.elapsed());
        let mut c = m.cache_nueva();
        let t1 = std::time::Instant::now();
        let l = m.adelante(&ids, &mut c).unwrap().as_dtype(Dtype::Float32).unwrap();
        l.eval().unwrap();
        println!("prellenado {} tok: {:?}", ids.len(), t1.elapsed());
        m.marcar(&mut c);
        let orden = ops::argsort_axis(ops::negative(&l).unwrap(), -1).unwrap();
        let top: Vec<u32> = (0..5).map(|i| orden.try_index(i).unwrap().try_item_cast::<u32>().unwrap()).collect();
        let vals: Vec<f32> = top.iter().map(|&i| l.try_index(i as i32).unwrap().try_item_cast::<f32>().unwrap()).collect();
        println!("top5 {top:?} {vals:?}\nref  {} {}", r["top5"], r["top5_val"]);
        let ref_top: Vec<u32> = r["top5"].as_array().unwrap().iter().map(|x| x.as_u64().unwrap() as u32).collect();
        let ref_val: Vec<f32> = r["top5_val"].as_array().unwrap().iter().map(|x| x.as_f64().unwrap() as f32).collect();
        assert_eq!(top[0], ref_top[0], "el primer token difiere");
        for (a, b) in vals.iter().zip(&ref_val) {
            assert!((a - b).abs() < 0.5, "logit {a} vs {b}");
        }
        let muestra: Vec<f32> = (0..10).map(|i| l.try_index(i * 24800).unwrap().try_item_cast::<f32>().unwrap()).collect();
        let ref_m: Vec<f32> = r["muestra"].as_array().unwrap().iter().map(|x| x.as_f64().unwrap() as f32).collect();
        let peor = muestra.iter().zip(&ref_m).map(|(a, b)| (a - b).abs()).fold(0.0f32, f32::max);
        println!("muestra de 10 logits: diferencia máxima {peor}");
        assert!(peor < 0.5);
        let voraz = |c: &mut Cache, primero: u32| -> (Vec<u32>, f64) {
            let mut gen = vec![];
            let mut nxt = primero;
            let t = std::time::Instant::now();
            for _ in 0..40 {
                gen.push(nxt);
                nxt = argmax(&m.adelante(&[nxt], c).unwrap());
            }
            (gen, 40.0 / t.elapsed().as_secs_f64())
        };
        let (gen, tps) = voraz(&mut c, top[0]);
        let ref_gen: Vec<u32> = r["greedy"].as_array().unwrap().iter().map(|x| x.as_u64().unwrap() as u32).collect();
        println!("voraz {tps:.1} tok/s (ref {:.1}) · iguales {}/40", r["tok_s"].as_f64().unwrap(), gen.iter().zip(&ref_gen).take_while(|(a, b)| a == b).count());
        // idénticos hasta el fin de turno INCLUIDO (lo de después es basura con logits casi empatados,
        // donde un redondeo bf16 distinto ya elige otro token: medido 37/40 iguales, el 38 tras el fin)
        let hasta = ref_gen.iter().position(|&t| t == 248046 || t == 248044 || t == 151645).map(|p| p + 1).unwrap_or(ref_gen.len());
        assert_eq!(gen[..hasta], ref_gen[..hasta], "la respuesta voraz difiere de mlx_lm");
        // prellenado en caliente (el primero incluye compilar los kernels)
        let mut c2 = m.cache_nueva();
        let t2 = std::time::Instant::now();
        m.adelante(&ids, &mut c2).unwrap().eval().unwrap();
        println!("prellenado en caliente: {:?} (ref {:.3} s)", t2.elapsed(), r["prellenado_s"].as_f64().unwrap());
        // volver a la instantánea del fin del prompt: debe repetir exactamente lo mismo
        // recurrente: vuelve a la instantánea ≤ n; sin estado recurrente (densos, Gemma): recorta exacto
        let mut quedo = m.recortar(&mut c, ids.len() as i32 + 3).unwrap();
        if quedo != ids.len() as i32 {
            assert_eq!(quedo, ids.len() as i32 + 3, "sin estado recurrente el recorte es exacto");
            quedo = m.recortar(&mut c, ids.len() as i32).unwrap();
        }
        assert_eq!(quedo, ids.len() as i32);
        let (gen2, _) = voraz(&mut c, top[0]);
        assert_eq!(gen2, gen, "tras recortar al fin del prompt, la misma respuesta");
        // antes de la primera instantánea → desde cero (recurrente) o exacto (los demás)
        let r = m.recortar(&mut c, 5).unwrap();
        assert!(r == 0 || r == 5, "{r}");
    }

    /// Banco con ruido acotado (la Mac comparte memoria): el MEJOR de 3 tras un calentamiento,
    /// igual que `bench_py.py`. `GOTV_MLX_DIR=… GOTV_IDS=ids.json cargo test -p gotv-mlx --release
    /// -- --ignored --nocapture banco_nativo`
    #[test]
    #[ignore]
    fn banco_nativo() {
        let (Ok(dir), Ok(f)) = (std::env::var("GOTV_MLX_DIR"), std::env::var("GOTV_IDS")) else { return };
        let ids: Vec<u32> = serde_json::from_str(&std::fs::read_to_string(f).unwrap()).unwrap();
        let m = cargar_red(Path::new(&dir)).unwrap();
        let (mut pre, mut dec) = (vec![], vec![]);
        for _ in 0..4 {
            let mut c = m.cache_nueva();
            let t = std::time::Instant::now();
            let mut nxt = argmax(&m.adelante(&ids, &mut c).unwrap());
            pre.push(t.elapsed().as_secs_f64());
            let t = std::time::Instant::now();
            for _ in 0..64 {
                nxt = argmax(&m.adelante(&[nxt], &mut c).unwrap());
            }
            dec.push(64.0 / t.elapsed().as_secs_f64());
        }
        let min = pre[1..].iter().cloned().fold(f64::MAX, f64::min);
        let max = dec[1..].iter().cloned().fold(0.0, f64::max);
        println!("nativo  prellenado {} tok: {min:.3} s · generación {max:.1} tok/s", ids.len());
    }

    /// El config del 9B real (recortado): text_config aplanado, RoPE parcial y θ de rope_parameters,
    /// cuantización por capa con el prefijo `language_model.`.
    #[test]
    fn el_config_hibrido_se_aplana() {
        let c = Config::de_json(
            r#"{"model_type":"qwen3_5","tie_word_embeddings":false,
            "quantization":{"group_size":64,"bits":4,"mode":"affine","language_model.model.layers.0.mlp.down_proj":{"group_size":64,"bits":6}},
            "text_config":{"model_type":"qwen3_5_text","hidden_size":4096,"num_hidden_layers":32,"num_attention_heads":16,"num_key_value_heads":4,
              "head_dim":256,"full_attention_interval":4,"linear_num_key_heads":16,"linear_num_value_heads":32,"eos_token_id":248044,
              "rope_parameters":{"rope_type":"default","rope_theta":10000000,"partial_rotary_factor":0.25,"mrope_section":[11,11,10]}}}"#,
        )
        .unwrap();
        assert_eq!(c.model_type, "qwen3_5");
        assert!(c.hibrido());
        assert_eq!(c.rope_theta, 1e7);
        assert_eq!(c.partial_rotary_factor, Some(0.25));
        assert_eq!(c.cuantizacion("model.layers.0.mlp.down_proj"), Some(Cuantizacion { group_size: 64, bits: 6 }));
        assert_eq!(c.cuantizacion("model.layers.1.mlp.down_proj"), Some(Cuantizacion { group_size: 64, bits: 4 }));
        assert_eq!(por_que_no(&c), None);
        // fase 3: el MoE de Qwen3.5 entra; otros MoE (qwen3_moe, mixtral…) todavía no
        let moe = Config::de_json(r#"{"model_type":"qwen3_5_moe","text_config":{"model_type":"x","hidden_size":8,"num_hidden_layers":4,"num_attention_heads":1,"num_key_value_heads":1,"num_experts":256,"num_experts_per_tok":8,"mlp_only_layers":[2]}}"#).unwrap();
        assert_eq!(por_que_no(&moe), None);
        assert!(moe.hibrido() && moe.capa_moe(0) && moe.capa_moe(3) && !moe.capa_moe(2));
        assert_eq!(moe.partial_rotary_factor, Some(0.25), "valores por omisión del híbrido");
        let otro = Config::de_json(r#"{"model_type":"qwen3_moe","hidden_size":8,"num_hidden_layers":1,"num_attention_heads":1,"num_key_value_heads":1,"num_experts":64}"#).unwrap();
        assert!(por_que_no(&otro).is_some(), "otros MoE siguen por el servidor");
        let yarn = Config::de_json(r#"{"model_type":"qwen3","hidden_size":8,"num_hidden_layers":1,"num_attention_heads":1,"num_key_value_heads":1,"rope_scaling":{"type":"yarn","factor":4}}"#).unwrap();
        assert!(por_que_no(&yarn).unwrap().contains("yarn"));
    }

    /// La RoPE «llama3» de Hermes-4-70B: los periodos son los de `_compute_llama3_parameters` de
    /// transformers (juez 2026-11-12, inv_freq en f32 → periodo) y el modelo YA no va al servidor.
    #[test]
    fn rope_llama3_como_transformers() {
        let c = Config::de_json(
            r#"{"model_type":"llama","hidden_size":8192,"num_hidden_layers":1,"num_attention_heads":64,"num_key_value_heads":8,"rope_theta":500000.0,
              "rope_scaling":{"factor":8.0,"high_freq_factor":4.0,"low_freq_factor":1.0,"original_max_position_embeddings":8192,"rope_type":"llama3"}}"#,
        )
        .unwrap();
        assert!(por_que_no(&c).is_none(), "llama3 ya corre en el núcleo");
        let p = c.periodos_llama3(128).unwrap();
        let oficial = [(0, 1.0), (10, 7.770769646913223), (20, 60.384865781206436), (30, 728.9194583688355), (40, 29170.65759958298), (50, 226678.49377310765), (63, 3258469.0533297313)];
        for (i, v) in oficial {
            assert!(((p[i] as f64 - v) / v).abs() < 1e-6, "periodo {i}: {} vs {v}", p[i]);
        }
        let normal = Config::de_json(r#"{"model_type":"qwen3","hidden_size":8,"num_hidden_layers":1,"num_attention_heads":1,"num_key_value_heads":1}"#).unwrap();
        assert!(normal.periodos_llama3(8).is_none());
    }
}

#[cfg(test)]
mod instantaneas {
    use super::*;

    fn cache(largos: &[i32], largo: i32) -> Cache {
        Cache { capas: vec![], largo, instantaneas: largos.iter().map(|&l| Instantanea { largo: l, capas: vec![] }).collect() }
    }

    /// El caso del Portador (2026-11-11): un turno largo (marca en 53 592, donde abre su mensaje) y
    /// luego 11 pasos de herramienta (dos marcas por paso). Con «soltar la más vieja» la del inicio
    /// del turno moría; raleando por el hueco más chico sobrevive, y la primera y la última también.
    #[test]
    fn el_inicio_del_turno_sobrevive_a_un_bucle_de_herramientas() {
        let mut v: Vec<i32> = vec![51_930, 52_799, 53_592];
        let mut fin = 53_700;
        for _ in 0..11 {
            v.push(fin); // fin del prompt del paso
            fin += 180;
            v.push(fin); // donde abre el resultado de la herramienta
            fin += 420;
            while v.len() > INSTANTANEAS {
                v.remove(cual_desalojar(&v));
            }
        }
        assert!(v.contains(&53_592), "la del inicio del turno se conserva: {v:?}");
        assert_eq!(v[0], 51_930, "la primera nunca se suelta");
        assert_eq!(v.len(), INSTANTANEAS);
        let c = cache(&v, fin);
        // la orden nueva diverge justo después de abrir el turno anterior (la plantilla quita <think>)
        assert_eq!(alcanzable_en(&c, 53_600, true), 53_592);
    }

    /// `alcanzable` = lo que `recortar` dejaría: en un híbrido, la última instantánea ≤ n (0 si no
    /// hay); en un denso, n. Contra ESTO se decide si la memoria del disco sirve.
    #[test]
    fn alcanzable_es_lo_que_recortar_dejaria() {
        let c = cache(&[60_000, 61_000], 62_000);
        assert_eq!(alcanzable_en(&c, 53_600, true), 0, "híbrido sin instantánea ≤ n: desde cero");
        assert_eq!(alcanzable_en(&c, 53_600, false), 53_600, "denso: el prefijo entero");
        assert_eq!(alcanzable_en(&c, 61_500, true), 61_000);
        assert_eq!(alcanzable_en(&c, 70_000, true), 62_000, "más allá de lo calculado: todo");
        assert_eq!(cual_desalojar(&[1, 2]), 0);
    }
}
