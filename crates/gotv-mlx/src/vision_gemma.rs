//! M8b · la visión de Gemma 4 en el núcleo, como `mlx_vlm/models/gemma4/vision.py` y su procesador
//! (`processing_gemma4.py`): redimensionar MANTENIENDO la proporción al presupuesto de parches
//! (lados múltiplos de 3·16 = 48, bicúbico de PIL), escalar a [0, 1] → 2·(x − 0,5) → parches de
//! 16×16 → lineal + posición aprendida (una tabla por eje: x e y) → 16 bloques [RMSNorm → atención
//! con q/k/v normalizados (f32) y RoPE 2D partido por eje → RMSNorm; RMSNorm → MLP GeGLU → RMSNorm],
//! todos los lineales con RECORTE de entrada y salida → promedio por bloques de 3×3 parches → ×√768
//! → proyector al modelo de lenguaje (RMSNorm sin peso + lineal cuantizado).
//!
//! En el prompt cada imagen es `<|image>` + N × `<|image|>` + `<image|>`, con posiciones normales.

use crate::modelo::{Config, Fusiones, Lineal, M};
use anyhow::{anyhow, Context, Result};
use gotv_engine::{bail_tr, tr};
use mlx_rs::fast::{self, ScaledDotProductAttentionMask};
use mlx_rs::ops::indexing::take_axis;
use mlx_rs::ops;
use mlx_rs::{Array, Dtype};
use std::collections::HashMap;
use std::path::Path;

#[derive(Clone, Debug, serde::Deserialize)]
pub struct GVConfig {
    pub hidden_size: i32,
    pub num_attention_heads: i32,
    pub head_dim: i32,
    pub num_hidden_layers: i32,
    pub patch_size: i32,
    pub pooling_kernel_size: i32,
    pub position_embedding_size: i32,
    pub default_output_length: i32,
    #[serde(default = "eps")]
    pub rms_norm_eps: f32,
    #[serde(default)]
    pub use_clipped_linears: bool,
    #[serde(default)]
    pub standardize: bool,
    #[serde(default)]
    pub rope_parameters: Option<serde_json::Value>,
}

fn eps() -> f32 {
    1e-6
}

/// Un lineal SIN sesgo con recorte opcional de entrada y salida (`ClippableLinear`).
pub(crate) struct Recortado {
    w_t: Array,
    topes: Option<[Array; 4]>,
}

impl Recortado {
    pub(crate) fn cargar(p: &mut HashMap<String, Array>, n: &str) -> Result<Recortado> {
        let w = p.remove(&format!("{n}.linear.weight")).ok_or_else(|| anyhow!("falta {n}.linear.weight"))?;
        let mut t = |s: &str| p.remove(&format!("{n}.{s}"));
        let topes = match (t("input_min"), t("input_max"), t("output_min"), t("output_max")) {
            (Some(a), Some(b), Some(c), Some(d)) => Some([a, b, c, d]),
            _ => None,
        };
        Ok(Recortado { w_t: ops::transpose_axes(&w, &[1, 0]).m()?, topes })
    }

    pub(crate) fn aplicar(&self, x: &Array) -> Result<Array> {
        match &self.topes {
            Some([imin, imax, omin, omax]) => {
                let x = ops::clip(x, (imin, imax)).m()?;
                ops::clip(ops::matmul(&x, &self.w_t).m()?, (omin, omax)).m()
            }
            None => ops::matmul(x, &self.w_t).m(),
        }
    }
}

/// RMSNorm en f32 como `VisionRMSNorm` (mean(x²) → rsqrt → × peso), de vuelta al tipo de x.
fn norma_f32(x: &Array, peso: Option<&Array>, eps: f32) -> Result<Array> {
    let xf = x.as_dtype(Dtype::Float32).m()?;
    let var = ops::power(&xf, Array::from_f32(2.0)).m()?.mean_axis(-1, true).m()?;
    let n = ops::multiply(&xf, ops::rsqrt(ops::add(&var, Array::from_f32(eps)).m()?).m()?).m()?;
    let n = match peso {
        Some(w) => ops::multiply(&n, w.as_dtype(Dtype::Float32).m()?).m()?,
        None => n,
    };
    n.as_dtype(x.dtype()).m()
}

struct Bloque {
    norma_in: Array,
    norma_post_atencion: Array,
    norma_pre_mlp: Array,
    norma_post_mlp: Array,
    q: Recortado,
    k: Recortado,
    v: Recortado,
    o: Recortado,
    q_norma: Array,
    k_norma: Array,
    puerta: Recortado,
    arriba: Recortado,
    abajo: Recortado,
}

pub struct VisionGemma {
    pub c: GVConfig,
    entrada: Array,
    tabla_pos: Array,
    bloques: Vec<Bloque>,
    proyeccion: Lineal,
    eps_proy: f32,
    theta: f32,
    f: Fusiones,
}

/// Una imagen lista para la torre de Gemma: parches [N, P·P·3] en [0, 1] y su rejilla en parches.
pub struct ImagenG {
    pub parches: Vec<f32>,
    pub alto: i32,
    pub ancho: i32,
}

impl ImagenG {
    /// Tokens que ocupa en el modelo de lenguaje (un token por bloque de k×k parches).
    pub fn tokens(&self, k: i32) -> i32 {
        (self.alto * self.ancho) / (k * k)
    }
}

/// `get_aspect_ratio_preserving_size` de mlx_vlm.
fn tamano(h: usize, w: usize, parche: usize, max_parches: usize, k: usize) -> Result<(usize, usize)> {
    let objetivo = (max_parches * parche * parche) as f64;
    let factor = (objetivo / (h * w) as f64).sqrt();
    let lado = k * parche;
    let mut th = ((factor * h as f64 / lado as f64).floor() as usize) * lado;
    let mut tw = ((factor * w as f64 / lado as f64).floor() as usize) * lado;
    if th == 0 && tw == 0 {
        bail_tr!("imagen demasiado pequeña para Gemma", "image too small for Gemma");
    }
    let max_lado = (max_parches / (k * k)) * lado;
    if th == 0 {
        th = lado;
        tw = ((w as f64 / h as f64).floor() as usize * lado).min(max_lado);
    } else if tw == 0 {
        tw = lado;
        th = ((h as f64 / w as f64).floor() as usize * lado).min(max_lado);
    }
    Ok((th, tw))
}

impl VisionGemma {
    /// ¿Gemma 4 con una torre que el núcleo sabe correr? (sólo el config)
    pub fn config(dir: &Path) -> Option<GVConfig> {
        let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dir.join("config.json")).ok()?).ok()?;
        if !matches!(v["model_type"].as_str(), Some("gemma4")) || v["vision_config"]["model_type"].as_str() != Some("gemma4_vision") {
            return None;
        }
        // la atención bidireccional sobre la imagen (otros tamaños de Gemma 4) aún no está
        if v["text_config"]["use_bidirectional_attention"].as_str() == Some("vision") {
            return None;
        }
        let c: GVConfig = serde_json::from_value(v["vision_config"].clone()).ok()?;
        (!c.standardize).then_some(c)
    }

    pub fn cargar(dir: &Path, c: GVConfig, cfg_texto: &Config) -> Result<VisionGemma> {
        let mut p = crate::modelo::cargar_pesos_de(dir, "vision_tower.")?;
        if p.is_empty() {
            bail_tr!("el modelo no trae la torre de visión (vision_tower.*)", "the model has no vision tower (vision_tower.*)");
        }
        let entrada = p.remove("patch_embedder.input_proj.weight").ok_or_else(|| anyhow!("falta patch_embedder.input_proj"))?;
        let tabla_pos = p.remove("patch_embedder.position_embedding_table").ok_or_else(|| anyhow!("falta position_embedding_table"))?;
        let mut bloques = Vec::new();
        for i in 0..c.num_hidden_layers {
            let n = format!("encoder.layers.{i}");
            let w = |p: &mut HashMap<String, Array>, s: &str| p.remove(&format!("{n}.{s}")).ok_or_else(|| anyhow!("falta {n}.{s}"));
            bloques.push(Bloque {
                norma_in: w(&mut p, "input_layernorm.weight")?,
                norma_post_atencion: w(&mut p, "post_attention_layernorm.weight")?,
                norma_pre_mlp: w(&mut p, "pre_feedforward_layernorm.weight")?,
                norma_post_mlp: w(&mut p, "post_feedforward_layernorm.weight")?,
                q: Recortado::cargar(&mut p, &format!("{n}.self_attn.q_proj"))?,
                k: Recortado::cargar(&mut p, &format!("{n}.self_attn.k_proj"))?,
                v: Recortado::cargar(&mut p, &format!("{n}.self_attn.v_proj"))?,
                o: Recortado::cargar(&mut p, &format!("{n}.self_attn.o_proj"))?,
                q_norma: w(&mut p, "self_attn.q_norm.weight")?,
                k_norma: w(&mut p, "self_attn.k_norm.weight")?,
                puerta: Recortado::cargar(&mut p, &format!("{n}.mlp.gate_proj"))?,
                arriba: Recortado::cargar(&mut p, &format!("{n}.mlp.up_proj"))?,
                abajo: Recortado::cargar(&mut p, &format!("{n}.mlp.down_proj"))?,
            });
        }
        // el proyector vive fuera de la torre (`embed_vision.*`), cuantizado como el texto
        let mut pp = crate::modelo::cargar_pesos_de(dir, "embed_vision.")?;
        let mut sin_prefijo = cfg_texto.clone();
        sin_prefijo.prefijo = String::new();
        let proyeccion = Lineal::cargar(&mut pp, "embedding_projection", &sin_prefijo)?;
        let theta = c.rope_parameters.as_ref().and_then(|r| r["rope_theta"].as_f64()).unwrap_or(100.0) as f32;
        let eps_proy = c.rms_norm_eps;
        Ok(VisionGemma { entrada: ops::transpose_axes(&entrada, &[1, 0]).m()?, tabla_pos, bloques, proyeccion, eps_proy, theta, c, f: Fusiones::nuevas() })
    }

    /// La imagen como la deja el procesador de Gemma 4: proporción, bicúbico de PIL, [0, 1].
    pub fn preparar(&self, bytes: &[u8]) -> Result<ImagenG> {
        let c = &self.c;
        let img = image::load_from_memory(bytes).context(tr!("no pude leer la imagen", "could not read the image"))?.to_rgb8();
        let (w, h) = (img.width() as usize, img.height() as usize);
        let (ps, k) = (c.patch_size as usize, c.pooling_kernel_size as usize);
        let max_parches = (c.default_output_length as usize) * k * k;
        let (nh, nw) = tamano(h, w, ps, max_parches, k)?;
        let datos = if (nh, nw) != (h, w) { crate::vision::bicubico_pil(img.as_raw(), w, h, nw, nh) } else { img.into_raw() };
        // numpy: uint8 → f32 · (1/255) en f32
        let esc = 1.0f32 / 255.0;
        let (gh, gw) = (nh / ps, nw / ps);
        let mut out = Vec::with_capacity(gh * gw * ps * ps * 3);
        for by in 0..gh {
            for bx in 0..gw {
                for y in 0..ps {
                    for x in 0..ps {
                        let base = ((by * ps + y) * nw + (bx * ps + x)) * 3;
                        out.extend((0..3).map(|ch| datos[base + ch] as f32 * esc));
                    }
                }
            }
        }
        Ok(ImagenG { parches: out, alto: gh as i32, ancho: gw as i32 })
    }

    /// RoPE 2D de Gemma: la cabeza se parte en dos (x, y) y cada mitad gira con su posición.
    fn girar(&self, t: &Array, pos: &[Array; 2]) -> Result<Array> {
        let d = t.dim(-1);
        let canales = 2 * (d / 4);
        let mitad = canales / 2;
        let ex = ops::multiply(Array::from_f32(2.0 / canales as f32), ops::arange::<_, f32>(0.0, mitad as f32, 1.0).m()?).m()?;
        let escala = ops::power(Array::from_f32(self.theta), &ex).m()?;
        let mut partes = Vec::new();
        for (i, p) in pos.iter().enumerate() {
            let x = t.try_index_ext(i as i32 * canales, (i as i32 + 1) * canales)?;
            let ang = ops::divide(p, &escala).m()?; // [L, mitad]
            let doble = |a: Array| -> Result<Array> {
                let a = ops::concatenate(&[a.clone(), a], -1).m()?.as_dtype(t.dtype()).m()?;
                ops::expand_dims(&a, 1).m() // [L, 1, canales]
            };
            let (cos, sin) = (doble(ops::cos(&ang).m()?)?, doble(ops::sin(&ang).m()?)?);
            let m = ops::split_at_indices(&x, &[mitad], -1).m()?;
            let rot = ops::concatenate(&[ops::negative(&m[1]).m()?, m[0].clone()], -1).m()?;
            partes.push(ops::add(&ops::multiply(&x, &cos).m()?, &ops::multiply(&rot, &sin).m()?).m()?);
        }
        ops::concatenate(&partes, -1).m()
    }

    /// Los tokens visuales PROYECTADOS al modelo de lenguaje: [N/k², oculto del texto].
    pub fn rasgos(&self, img: &ImagenG) -> Result<Array> {
        self.correr(img).map(|(_, p)| p)
    }

    /// (salida de la torre tras el promedio, proyectados) — la primera sirve al juez.
    pub fn correr(&self, img: &ImagenG) -> Result<(Array, Array)> {
        let c = &self.c;
        let n = img.alto * img.ancho;
        let fila = c.patch_size * c.patch_size * 3;
        let tipo = self.tabla_pos.dtype();
        // 2·(x − 0,5) en f32 y al tipo de la entrada
        let x = Array::from_slice(&img.parches, &[n, fila]);
        let x = ops::multiply(Array::from_f32(2.0), ops::subtract(&x, Array::from_f32(0.5)).m()?).m()?.as_dtype(self.entrada.dtype()).m()?;
        let x = ops::matmul(&x, &self.entrada).m()?;
        // posición: tabla[0][columna] + tabla[1][fila]
        let (cols, filas): (Vec<i32>, Vec<i32>) = (0..n).map(|i| (i % img.ancho, i / img.ancho)).unzip();
        let tx = take_axis(&take_axis(&self.tabla_pos, Array::from_int(0), 0).m()?, Array::from_slice(&cols, &[n]), 0).m()?;
        let ty = take_axis(&take_axis(&self.tabla_pos, Array::from_int(1), 0).m()?, Array::from_slice(&filas, &[n]), 0).m()?;
        let mut h = ops::add(&x, &ops::add(&tx, &ty).m()?.as_dtype(tipo).m()?).m()?;
        let pos = [
            ops::reshape(Array::from_slice(&cols.iter().map(|&v| v as f32).collect::<Vec<_>>(), &[n]), &[n, 1]).m()?,
            ops::reshape(Array::from_slice(&filas.iter().map(|&v| v as f32).collect::<Vec<_>>(), &[n]), &[n, 1]).m()?,
        ];
        let (cab, hd) = (c.num_attention_heads, c.head_dim);
        let eps = c.rms_norm_eps;
        // máscara aditiva de ceros (sin relleno, todo válido), como la que arma mlx_vlm
        let mascara = ops::zeros::<f32>(&[1, 1, n, n]).m()?.as_dtype(h.dtype()).m()?;
        for b in &self.bloques {
            let y = fast::rms_norm(&h, Some(&b.norma_in), eps).m()?;
            let q = norma_f32(&ops::reshape(b.q.aplicar(&y)?, &[n, cab, hd]).m()?, Some(&b.q_norma), 1e-6)?;
            let k = norma_f32(&ops::reshape(b.k.aplicar(&y)?, &[n, cab, hd]).m()?, Some(&b.k_norma), 1e-6)?;
            let v = norma_f32(&ops::reshape(b.v.aplicar(&y)?, &[n, cab, hd]).m()?, None, 1e-6)?;
            let q = self.girar(&q, &pos)?;
            let k = self.girar(&k, &pos)?;
            let t = |a: &Array| -> Result<Array> { ops::expand_dims(ops::transpose_axes(a, &[1, 0, 2]).m()?, 0).m() };
            let a = fast::scaled_dot_product_attention(&t(&q)?, &t(&k)?, &t(&v)?, 1.0, Some(ScaledDotProductAttentionMask::Array(&mascara)), None::<&Array>).m()?;
            let a = ops::reshape(ops::transpose_axes(&a, &[0, 2, 1, 3]).m()?, &[n, -1]).m()?;
            let a = fast::rms_norm(b.o.aplicar(&a)?, Some(&b.norma_post_atencion), eps).m()?;
            h = ops::add(&h, &a).m()?;
            let y = fast::rms_norm(&h, Some(&b.norma_pre_mlp), eps).m()?;
            let g = (self.f.gelu_tanh.borrow_mut())(&b.puerta.aplicar(&y)?).m()?;
            let y = b.abajo.aplicar(&ops::multiply(&g, b.arriba.aplicar(&y)?).m()?)?;
            let y = fast::rms_norm(&y, Some(&b.norma_post_mlp), eps).m()?;
            h = ops::add(&h, &y).m()?;
        }
        // promedio por bloques de k×k parches (`_avg_pool_by_positions`): pesos 1/k² en f32
        let kk = c.pooling_kernel_size;
        let largo = n / (kk * kk);
        let ancho_b = img.ancho / kk;
        let mut pesos = vec![0f32; (largo * n) as usize];
        for i in 0..n {
            let idx = (cols[i as usize] / kk) + ancho_b * (filas[i as usize] / kk);
            pesos[(idx * n + i) as usize] = 1.0 / (kk * kk) as f32;
        }
        let pesos = Array::from_slice(&pesos, &[largo, n]);
        let pool = ops::matmul(&pesos, &h.as_dtype(Dtype::Float32).m()?).m()?.as_dtype(h.dtype()).m()?;
        let pool = ops::multiply(&pool, Array::from_f32((c.hidden_size as f32).sqrt()).as_dtype(pool.dtype()).m()?).m()?;
        let proy = self.proyeccion.aplicar(&fast::rms_norm(&pool, None, self.eps_proy).m()?)?;
        Ok((pool, proy))
    }
}

/// Recorte del último eje [a, b) (azúcar para la RoPE por partes).
trait Tramo {
    fn try_index_ext(&self, a: i32, b: i32) -> Result<Array>;
}

impl Tramo for Array {
    fn try_index_ext(&self, a: i32, b: i32) -> Result<Array> {
        use mlx_rs::ops::indexing::TryIndexOp;
        self.try_index((.., .., a..b)).m()
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

    /// Juez contra mlx_vlm (`ref_vision_gemma.py`): píxeles, torre, proyección y la descripción.
    /// `GOTV_MLX_DIR=<gemma> GOTV_REF_VISION=<dir> GOTV_IMG=<png> cargo test -p gotv-mlx --release
    /// -- --ignored --nocapture ve_como_mlx_vlm_gemma`
    #[test]
    #[ignore]
    fn ve_como_mlx_vlm_gemma() {
        use crate::modelo::{cargar_red, Extra};
        use mlx_rs::ops::indexing::TryIndexOp;
        let (Ok(dir), Ok(rd), Ok(imagen)) = (std::env::var("GOTV_MLX_DIR"), std::env::var("GOTV_REF_VISION"), std::env::var("GOTV_IMG")) else { return };
        let dir = Path::new(&dir);
        let rj: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(Path::new(&rd).join("ref.json")).unwrap()).unwrap();
        let rs = Array::load_safetensors(Path::new(&rd).join("ref.safetensors")).unwrap();
        let m = cargar_red(dir).unwrap();
        let c = VisionGemma::config(dir).expect("Gemma 4 con visión admitida");
        let v = VisionGemma::cargar(dir, c, m.cfg()).unwrap();
        // 1 · preprocesado: la referencia viene [1, C, H, W] → al orden de parches
        let img = v.preparar(&std::fs::read(&imagen).unwrap()).unwrap();
        let px = &rs["pixeles"];
        let (hh, ww) = (px.dim(2) as usize, px.dim(3) as usize);
        let (ps, gh, gw) = (16usize, img.alto as usize, img.ancho as usize);
        assert_eq!((gh * ps, gw * ps), (hh, ww), "mismo tamaño que mlx_vlm");
        let r = px.as_slice::<f32>();
        let mut refp = Vec::with_capacity(r.len());
        for by in 0..gh {
            for bx in 0..gw {
                for y in 0..ps {
                    for x in 0..ps {
                        for ch in 0..3 {
                            refp.push(r[(ch * hh + by * ps + y) * ww + bx * ps + x]);
                        }
                    }
                }
            }
        }
        let (d1, _) = comparar(&img.parches, &refp);
        println!("1 · píxeles {}×{} → diferencia máxima {d1}", ww, hh);
        assert!(d1 < 1e-6, "el preprocesado difiere");
        // 2 · torre y proyección
        let (pool, proy) = v.correr(&img).unwrap();
        let (pool, proy) = (pool.as_dtype(Dtype::Float32).unwrap(), proy.as_dtype(Dtype::Float32).unwrap());
        let (d2, c2) = comparar(pool.as_slice::<f32>(), rs["torre"].as_slice::<f32>());
        let (d3, c3) = comparar(proy.as_slice::<f32>(), rs["rasgos"].as_slice::<f32>());
        println!("2 · torre: dif. máx {d2} · coseno {c2:.6}\n   proyección: dif. máx {d3} · coseno {c3:.6}");
        assert!(c3 > 0.999, "la torre difiere");
        // 3 · el modelo completo con la imagen
        let ids: Vec<u32> = rj["ids"].as_array().unwrap().iter().map(|x| x.as_u64().unwrap() as u32).collect();
        let mut k = 0;
        let cual: Vec<i32> = ids.iter().map(|&t| if t == 258880 { k += 1; k - 1 } else { -1 }).collect();
        let mut cache = m.cache_nueva();
        let rasgos = v.rasgos(&img).unwrap();
        let l = m.adelante_ext(&ids, &Extra { rasgos: Some((&rasgos, cual)), pos: None }, &mut cache).unwrap().as_dtype(Dtype::Float32).unwrap();
        let orden = ops::argsort_axis(ops::negative(&l).unwrap(), -1).unwrap();
        let top: Vec<u32> = (0..5).map(|i| orden.try_index(i).unwrap().try_item_cast::<u32>().unwrap()).collect();
        println!("3 · top5 {top:?} · ref {}", rj["top5"]);
        let mut gen = vec![];
        let mut nxt = top[0];
        for _ in 0..30 {
            gen.push(nxt);
            nxt = mlx_rs::ops::indexing::argmax_axis(m.adelante(&[nxt], &mut cache).unwrap(), -1, false).unwrap().try_item_cast::<u32>().unwrap();
        }
        let refv: Vec<u32> = rj["voraz"].as_array().unwrap().iter().map(|x| x.as_u64().unwrap() as u32).collect();
        println!("   voraz iguales {}/30 · ref: {}", gen.iter().zip(&refv).take_while(|(a, b)| a == b).count(), rj["texto"]);
        let hasta = refv.iter().position(|&t| t == 106).map(|p| p + 1).unwrap_or(refv.len());
        assert_eq!(gen[..hasta], refv[..hasta], "la descripción difiere de mlx_vlm");
    }

    #[test]
    fn el_tamano_es_el_de_gemma4() {
        // 512×512 → 768×768 (48×48 parches = 2304 ≤ 2520; 256 tokens)
        assert_eq!(tamano(512, 512, 16, 2520, 3).unwrap(), (768, 768));
        // 1024×768 conserva la proporción en múltiplos de 48
        let (h, w) = tamano(768, 1024, 16, 2520, 3).unwrap();
        assert!(h % 48 == 0 && w % 48 == 0 && (h / 16) * (w / 16) <= 2520 && w > h, "{h}×{w}");
    }
}
