//! Gemma 4 sobre MLX (G4 fase 4), el mismo cálculo que `mlx_lm/models/gemma4_text.py`:
//!
//! - embedding × √hidden, y además un **embedding por capa** (cada capa recibe su propia señal del
//!   token: tabla `embed_tokens_per_layer` + proyección del estado inicial, normalizadas y sumadas);
//! - capas de atención con **ventana deslizante** (512 tokens) y, cada pocas, una **global** con
//!   cabezas más anchas y **RoPE proporcional** (sólo gira una parte, con frecuencias propias);
//! - q/k con RMSNorm por cabeza, v con RMSNorm SIN peso, escala de atención 1;
//! - las últimas `num_kv_shared_layers` capas **no calculan K/V**: usan los de la última capa
//!   anterior de su mismo tipo (ahorra caché);
//! - MLP **GeGLU** (gelu aproximada) entre normas propias, compuerta por capa con la señal del token,
//!   un escalar por capa, y los logits finales recortados con tanh (softcap).
//!
//! Los pesos son los de mlx-community (afines cuantizados); la visión y el audio no se cargan.
//! Sin estado recurrente: la caché se recorta en O(1) como en los densos.

use crate::modelo::{cargar_pesos, escribir_kv, quitar, Cache, CacheCapa, Config, Embedding, Extra, Fusiones, Lineal, Red, M};
use anyhow::Result;
use gotv_engine::{anyhow_tr, bail_tr};
use mlx_rs::fast::{self, ScaledDotProductAttentionMask};
use mlx_rs::ops::indexing::{take_axis, TryIndexOp};
use mlx_rs::ops;
use mlx_rs::Array;
use std::path::Path;

#[derive(Clone, Debug, serde::Deserialize)]
struct GConfig {
    hidden_size: i32,
    num_hidden_layers: i32,
    num_attention_heads: i32,
    head_dim: i32,
    #[serde(default)]
    global_head_dim: Option<i32>,
    num_key_value_heads: i32,
    #[serde(default)]
    num_global_key_value_heads: Option<i32>,
    #[serde(default)]
    num_kv_shared_layers: i32,
    #[serde(default)]
    hidden_size_per_layer_input: i32,
    #[serde(default = "eps")]
    rms_norm_eps: f32,
    #[serde(default = "ventana")]
    sliding_window: i32,
    #[serde(default = "patron")]
    sliding_window_pattern: i32,
    #[serde(default)]
    layer_types: Option<Vec<String>>,
    #[serde(default)]
    rope_parameters: Option<serde_json::Value>,
    #[serde(default)]
    final_logit_softcapping: Option<f32>,
}

fn eps() -> f32 {
    1e-6
}
fn ventana() -> i32 {
    512
}
fn patron() -> i32 {
    5
}

/// Cómo gira una capa: RoPE normal (θ, todas las dimensiones) o proporcional (frecuencias propias).
enum Giro {
    Normal { dims: i32, theta: f32 },
    Proporcional { dims: i32, frecuencias: Array },
}

impl Giro {
    fn de(params: Option<&serde_json::Value>, dims: i32) -> Result<Giro> {
        let p = params.cloned().unwrap_or(serde_json::json!({}));
        let theta = p["rope_theta"].as_f64().unwrap_or(10000.0) as f32;
        match p.get("rope_type").or(p.get("type")).and_then(|t| t.as_str()).unwrap_or("default") {
            "default" => Ok(Giro::Normal { dims, theta }),
            // `ProportionalRoPE`: exponentes sobre las dims COMPLETAS; las no giradas, frecuencia ∞
            "proportional" => {
                let factor = p["factor"].as_f64().unwrap_or(1.0) as f32;
                let girar = ((dims as f64) * p["partial_rotary_factor"].as_f64().unwrap_or(1.0)) as i32;
                // en la GPU como Python: factor · base^(arange(0, girar, 2)/dims) (el pow de Metal ≠ powf)
                let ex = ops::divide(&ops::arange::<_, f32>(0.0, girar as f32, 2.0).m()?, Array::from_f32(dims as f32)).m()?;
                let f = ops::multiply(Array::from_f32(factor), &ops::power(Array::from_f32(theta), &ex).m()?).m()?;
                let resto = Array::from_slice(&vec![f32::INFINITY; ((dims - girar) / 2) as usize], &[(dims - girar) / 2]);
                Ok(Giro::Proporcional { dims, frecuencias: ops::concatenate(&[f, resto], 0).m()? })
            }
            t => bail_tr!("RoPE «{t}» de Gemma aún no está en el núcleo", "Gemma RoPE «{t}» is not in the core yet"),
        }
    }

    fn aplicar(&self, x: &Array, offset: i32) -> Result<Array> {
        match self {
            Giro::Normal { dims, theta } => fast::rope(x, *dims, false, *theta, 1.0, offset, None::<&Array>).m(),
            Giro::Proporcional { dims, frecuencias } => fast::rope(x, *dims, false, None::<f32>, 1.0, offset, frecuencias).m(),
        }
    }
}

struct Capa {
    deslizante: bool,
    head_dim: i32,
    n_kv: i32,
    giro: Giro,
    q: Lineal,
    /// `None` en las capas que comparten K/V
    kv: Option<(Lineal, Lineal, Array)>,
    q_norma: Array,
    o: Lineal,
    /// de qué capa toma K/V (ella misma si los calcula)
    fuente: usize,
    norma_in: Array,
    norma_post_atencion: Array,
    norma_pre_mlp: Array,
    norma_post_mlp: Array,
    puerta: Lineal,
    arriba: Lineal,
    abajo: Lineal,
    /// compuerta por capa con la señal del token (2B/4B)
    por_capa: Option<(Lineal, Lineal, Array)>,
    escalar: Option<Array>,
}

pub struct Gemma {
    cfg: Config,
    g: GConfig,
    embed: Embedding,
    embed_por_capa: Option<Embedding>,
    proyeccion_por_capa: Option<(Lineal, Array)>,
    capas: Vec<Capa>,
    norma: Array,
    f: Fusiones,
}

impl Gemma {
    pub fn cargar(dir: &Path, cfg: Config) -> Result<Gemma> {
        let g: GConfig = serde_json::from_value(crate::modelo::aplanar(&std::fs::read_to_string(dir.join("config.json"))?)?.0)?;
        let mut pesos = cargar_pesos(dir)?;
        pesos.retain(|k, _| !["self_attn.rotary_emb", "input_max", "input_min", "output_max", "output_min"].iter().any(|s| k.contains(s)));
        let n = g.num_hidden_layers as usize;
        let tipos: Vec<String> = match &g.layer_types {
            Some(t) if t.len() == n => t.clone(),
            _ => {
                let mut p = vec!["sliding_attention".to_string(); (g.sliding_window_pattern - 1).max(0) as usize];
                p.push("full_attention".into());
                p.iter().cycle().take(n).cloned().collect()
            }
        };
        // las capas compartidas toman K/V de la ÚLTIMA capa anterior de su tipo
        let primera_compartida = n - g.num_kv_shared_layers.max(0) as usize;
        let mut ultima_de_tipo = std::collections::HashMap::new();
        for (i, t) in tipos.iter().enumerate().take(primera_compartida) {
            ultima_de_tipo.insert(t.clone(), i);
        }
        let rp = g.rope_parameters.clone();
        let mut capas = Vec::with_capacity(n);
        for (i, tipo) in tipos.iter().enumerate() {
            let p = format!("model.layers.{i}");
            let deslizante = tipo == "sliding_attention";
            let head_dim = if deslizante { g.head_dim } else { g.global_head_dim.unwrap_or(g.head_dim) };
            let n_kv = if deslizante { g.num_key_value_heads } else { g.num_global_key_value_heads.unwrap_or(g.num_key_value_heads) };
            let propia = i < primera_compartida;
            let fuente = if propia { i } else { *ultima_de_tipo.get(tipo).ok_or_else(|| anyhow_tr!("capa {i}: no hay capa de tipo {tipo} de la que compartir K/V", "layer {i}: no layer of type {tipo} to share K/V from"))? };
            let kv = if propia {
                Some((
                    Lineal::cargar(&mut pesos, &format!("{p}.self_attn.k_proj"), &cfg)?,
                    Lineal::cargar(&mut pesos, &format!("{p}.self_attn.v_proj"), &cfg)?,
                    quitar(&mut pesos, &format!("{p}.self_attn.k_norm.weight"))?,
                ))
            } else {
                None
            };
            let por_capa = if g.hidden_size_per_layer_input > 0 {
                Some((
                    Lineal::cargar(&mut pesos, &format!("{p}.per_layer_input_gate"), &cfg)?,
                    Lineal::cargar(&mut pesos, &format!("{p}.per_layer_projection"), &cfg)?,
                    quitar(&mut pesos, &format!("{p}.post_per_layer_input_norm.weight"))?,
                ))
            } else {
                None
            };
            capas.push(Capa {
                deslizante,
                head_dim,
                n_kv,
                giro: Giro::de(rp.as_ref().and_then(|r| r.get(tipo.as_str())), head_dim)?,
                q: Lineal::cargar(&mut pesos, &format!("{p}.self_attn.q_proj"), &cfg)?,
                kv,
                q_norma: quitar(&mut pesos, &format!("{p}.self_attn.q_norm.weight"))?,
                o: Lineal::cargar(&mut pesos, &format!("{p}.self_attn.o_proj"), &cfg)?,
                fuente,
                norma_in: quitar(&mut pesos, &format!("{p}.input_layernorm.weight"))?,
                norma_post_atencion: quitar(&mut pesos, &format!("{p}.post_attention_layernorm.weight"))?,
                norma_pre_mlp: quitar(&mut pesos, &format!("{p}.pre_feedforward_layernorm.weight"))?,
                norma_post_mlp: quitar(&mut pesos, &format!("{p}.post_feedforward_layernorm.weight"))?,
                puerta: Lineal::cargar(&mut pesos, &format!("{p}.mlp.gate_proj"), &cfg)?,
                arriba: Lineal::cargar(&mut pesos, &format!("{p}.mlp.up_proj"), &cfg)?,
                abajo: Lineal::cargar(&mut pesos, &format!("{p}.mlp.down_proj"), &cfg)?,
                por_capa,
                escalar: pesos.remove(&format!("{p}.layer_scalar")),
            });
        }
        let embed = Embedding { lineal: Lineal::cargar(&mut pesos, "model.embed_tokens", &cfg)? };
        let (embed_por_capa, proyeccion_por_capa) = if g.hidden_size_per_layer_input > 0 {
            (
                Some(Embedding { lineal: Lineal::cargar(&mut pesos, "model.embed_tokens_per_layer", &cfg)? }),
                Some((Lineal::cargar(&mut pesos, "model.per_layer_model_projection", &cfg)?, quitar(&mut pesos, "model.per_layer_projection_norm.weight")?)),
            )
        } else {
            (None, None)
        };
        let norma = quitar(&mut pesos, "model.norm.weight")?;
        if pesos.contains_key("lm_head.weight") && !cfg.tie_word_embeddings {
            bail_tr!("Gemma con cabeza propia (sin atar al embedding) aún no está en el núcleo", "Gemma with its own head (not tied to the embedding) is not in the core yet");
        }
        Ok(Gemma { cfg, g, embed, embed_por_capa, proyeccion_por_capa, capas, norma, f: Fusiones::nuevas() })
    }

    fn escalar(x: &Array, v: f32) -> Result<Array> {
        ops::multiply(x, Array::from_f32(v).as_dtype(x.dtype()).m()?).m()
    }
}

impl Red for Gemma {
    fn cfg(&self) -> &Config {
        &self.cfg
    }

    fn cache_nueva(&self) -> Cache {
        Cache { capas: (0..self.capas.len()).map(|_| CacheCapa::default()).collect(), largo: 0, instantaneas: Vec::new() }
    }

    fn recortar(&self, c: &mut Cache, n: i32) -> Result<i32> {
        if n <= 0 {
            *c = self.cache_nueva();
            return Ok(0);
        }
        c.largo = c.largo.min(n);
        Ok(c.largo)
    }

    fn marcar(&self, _c: &mut Cache) {}

    fn adelante(&self, ids: &[u32], c: &mut Cache) -> Result<Array> {
        self.adelante_ext(ids, &Extra::default(), c)
    }

    /// M8b: con tokens visuales (como `get_input_embeddings` de mlx_vlm): reemplazan al embedding
    /// YA escalado, y para la señal por capa los tokens de imagen cuentan como el token 0.
    fn adelante_ext(&self, ids: &[u32], extra: &Extra, c: &mut Cache) -> Result<Array> {
        if extra.pos.is_some() {
            bail_tr!("Gemma usa posiciones normales (sin mRoPE)", "Gemma uses normal positions (no mRoPE)");
        }
        let g = &self.g;
        let l = ids.len() as i32;
        let v: Vec<u32> = ids.to_vec();
        let ids_a = Array::from_slice(&v, &[1, l]);
        let mut h = Self::escalar(&self.embed.filas(&ids_a)?, (g.hidden_size as f32).sqrt())?;
        let mut ids_capa = ids_a.clone();
        if let Some((rasgos, cual)) = &extra.rasgos {
            if cual.len() != l as usize {
                bail_tr!("rasgos visuales para {} tokens y el trozo tiene {l}", "visual features for {} tokens and the chunk has {l}", cual.len());
            }
            if cual.iter().any(|&i| i >= 0) {
                let idx: Vec<i32> = cual.iter().map(|&i| i.max(0)).collect();
                let m = Array::from_slice(&cual.iter().map(|&i| i >= 0).collect::<Vec<_>>(), &[1, l, 1]);
                let filas = ops::reshape(take_axis(rasgos, Array::from_slice(&idx, &[l]), 0).m()?.as_dtype(h.dtype()).m()?, &[1, l, -1]).m()?;
                h = ops::select(&m, &filas, &h).m()?;
                let solo_texto: Vec<u32> = ids.iter().zip(cual).map(|(&t, &i)| if i >= 0 { 0 } else { t }).collect();
                ids_capa = Array::from_slice(&solo_texto, &[1, l]);
            }
        }
        // la señal POR CAPA: tabla propia + proyección del estado inicial
        let por_capa: Option<Array> = match (&self.embed_por_capa, &self.proyeccion_por_capa) {
            (Some(e), Some((proy, norma))) => {
                let d = g.hidden_size_per_layer_input;
                let tabla = Self::escalar(&e.filas(&ids_capa)?, (d as f32).sqrt())?;
                let tabla = ops::reshape(&tabla, &[1, l, g.num_hidden_layers, d]).m()?;
                let p = Self::escalar(&proy.aplicar(&h)?, (g.hidden_size as f32).powf(-0.5))?;
                let p = fast::rms_norm(ops::reshape(&p, &[1, l, g.num_hidden_layers, d]).m()?, Some(norma), g.rms_norm_eps).m()?;
                Some(Self::escalar(&ops::add(&p, &tabla).m()?, 2f32.powf(-0.5))?)
            }
            _ => None,
        };
        let eps = g.rms_norm_eps;
        let offset = c.largo;
        let fin = offset + l;
        // K/V de esta pasada por capa (las compartidas los toman de su fuente)
        let mut kv_de: Vec<Option<(Array, Array)>> = vec![None; self.capas.len()];
        // máscara de ventana para el prellenado que la excede (consulta p ve j ∈ (p−ventana, p])
        let ventana = g.sliding_window;
        let mascara_ventana = if l > 1 && fin > ventana {
            let filas = ops::expand_dims(ops::arange::<_, i32>(offset, fin, None).m()?, 1).m()?;
            let cols = ops::expand_dims(ops::arange::<_, i32>(0, fin, None).m()?, 0).m()?;
            let causal = ops::ge(&filas, &cols).m()?;
            let dentro = ops::lt(&filas, ops::add(&cols, Array::from_int(ventana)).m()?).m()?;
            Some(ops::logical_and(&causal, &dentro).m()?)
        } else {
            None
        };
        for (i, capa) in self.capas.iter().enumerate() {
            let residuo = h.clone();
            let x = fast::rms_norm(&h, Some(&capa.norma_in), eps).m()?;
            // ── atención ──
            let hd = capa.head_dim;
            let q = ops::reshape(capa.q.aplicar(&x)?, &[1, l, g.num_attention_heads, hd]).m()?;
            let q = fast::rms_norm(&q, Some(&capa.q_norma), eps).m()?;
            let q = capa.giro.aplicar(&ops::transpose_axes(&q, &[0, 2, 1, 3]).m()?, offset)?;
            let (k, vv) = match &capa.kv {
                Some((kp, vp, kn)) => {
                    let k = ops::reshape(kp.aplicar(&x)?, &[1, l, capa.n_kv, hd]).m()?;
                    let vv = ops::reshape(vp.aplicar(&x)?, &[1, l, capa.n_kv, hd]).m()?;
                    let k = fast::rms_norm(&k, Some(kn), eps).m()?;
                    let k = capa.giro.aplicar(&ops::transpose_axes(&k, &[0, 2, 1, 3]).m()?, offset)?;
                    let vv = ops::transpose_axes(fast::rms_norm(&vv, None, eps).m()?, &[0, 2, 1, 3]).m()?;
                    let par = escribir_kv(&mut c.capas[i], &k, &vv, offset, l)?;
                    kv_de[i] = Some(par.clone());
                    par
                }
                None => kv_de[capa.fuente].clone().ok_or_else(|| anyhow_tr!("capa {i}: su fuente {} no dejó K/V", "layer {i}: its source {} left no K/V", capa.fuente))?,
            };
            let a = if capa.deslizante && l == 1 && fin > ventana {
                // un token: sólo las últimas `ventana` posiciones (él incluido)
                let k = k.try_index((.., .., fin - ventana..fin, ..)).m()?;
                let vv = vv.try_index((.., .., fin - ventana..fin, ..)).m()?;
                fast::scaled_dot_product_attention(&q, &k, &vv, 1.0, None, None::<&Array>).m()?
            } else if capa.deslizante && mascara_ventana.is_some() {
                let m = mascara_ventana.as_ref().unwrap();
                fast::scaled_dot_product_attention(&q, &k, &vv, 1.0, Some(ScaledDotProductAttentionMask::Array(m)), None::<&Array>).m()?
            } else {
                let m = if l > 1 { Some(ScaledDotProductAttentionMask::Causal) } else { None };
                fast::scaled_dot_product_attention(&q, &k, &vv, 1.0, m, None::<&Array>).m()?
            };
            let a = ops::reshape(ops::transpose_axes(&a, &[0, 2, 1, 3]).m()?, &[1, l, -1]).m()?;
            let a = fast::rms_norm(capa.o.aplicar(&a)?, Some(&capa.norma_post_atencion), eps).m()?;
            h = ops::add(&residuo, &a).m()?;
            // ── MLP GeGLU ──
            let residuo = h.clone();
            let x = fast::rms_norm(&h, Some(&capa.norma_pre_mlp), eps).m()?;
            let x = capa.abajo.aplicar(&(self.f.geglu.borrow_mut())((&capa.puerta.aplicar(&x)?, &capa.arriba.aplicar(&x)?)).m()?)?;
            let x = fast::rms_norm(&x, Some(&capa.norma_post_mlp), eps).m()?;
            h = ops::add(&residuo, &x).m()?;
            // ── compuerta con la señal del token para esta capa ──
            if let (Some((compuerta, proyeccion, norma)), Some(pc)) = (&capa.por_capa, &por_capa) {
                let senal = pc.try_index((.., .., i as i32, ..)).m()?;
                let gte = (self.f.geglu.borrow_mut())((&compuerta.aplicar(&h)?, &senal)).m()?;
                let gte = fast::rms_norm(proyeccion.aplicar(&gte)?, Some(norma), eps).m()?;
                h = ops::add(&h, &gte).m()?;
            }
            if let Some(e) = &capa.escalar {
                h = ops::multiply(&h, e).m()?;
            }
        }
        c.largo = fin;
        let ultimo = take_axis(&h, Array::from_slice(&[l - 1], &[1]), 1).m()?;
        let ultimo = fast::rms_norm(&ultimo, Some(&self.norma), eps).m()?;
        let mut logits = self.embed.lineal.aplicar(&ultimo)?;
        if let Some(tope) = g.final_logit_softcapping {
            // tanh(x / tope) · tope
            let t = Array::from_f32(tope).as_dtype(logits.dtype()).m()?;
            logits = ops::multiply(&ops::tanh(&ops::divide(&logits, &t).m()?).m()?, &t).m()?;
        }
        ops::reshape(&logits, &[-1]).m()
    }
}
