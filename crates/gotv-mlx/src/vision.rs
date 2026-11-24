//! M8 · el núcleo VE: la torre de visión de Qwen3.5 (la misma de Qwen3-VL) sobre MLX, como
//! `mlx_vlm/models/qwen3_vl/vision.py`, y el preprocesado de imagen como su procesador sin torch
//! (`processing_qwen3_vl.py`): redimensionar a múltiplos de 32 con el bicúbico de PIL, normalizar
//! y cortar en parches de 16×16 (×2 en el tiempo) en el orden de los bloques de fusión 2×2.
//!
//! Torre: parches → lineal (la conv 3D de núcleo = paso es un producto) + posición interpolada de
//! una rejilla 48×48 → 27 bloques [LayerNorm → atención con RoPE 2D (alto, ancho) → LayerNorm →
//! MLP GeLU-tanh] → fusionador (LayerNorm, 4 parches vecinos juntos, MLP GeLU) → tokens del
//! modelo de lenguaje. Además, `posiciones` calcula el mRoPE 3D de todo el prompt
//! (`get_rope_index`): una imagen de h×w tokens ocupa sólo max(h, w) posiciones.

use crate::modelo::{Fusiones, M};
use anyhow::{anyhow, Context, Result};
use gotv_engine::{anyhow_tr, bail_tr, tr};
use mlx_rs::fast;
use mlx_rs::ops::indexing::{take_axis, TryIndexOp};
use mlx_rs::ops;
use mlx_rs::Array;
use std::collections::HashMap;
use std::path::Path;

#[derive(Clone, Debug, serde::Deserialize)]
pub struct VConfig {
    pub depth: i32,
    pub hidden_size: i32,
    pub num_heads: i32,
    pub out_hidden_size: i32,
    pub patch_size: i32,
    pub spatial_merge_size: i32,
    pub temporal_patch_size: i32,
    pub num_position_embeddings: i32,
    #[serde(default = "tres")]
    pub in_channels: i32,
    #[serde(default)]
    pub deepstack_visual_indexes: Vec<i32>,
}

fn tres() -> i32 {
    3
}

/// Una imagen lista para la torre: parches [N, C·T·P·P] (f32) y su rejilla (t, alto, ancho) en parches.
pub struct Imagen {
    pub pixeles: Vec<f32>,
    pub rejilla: (i32, i32, i32),
}

impl Imagen {
    /// Tokens que ocupa en el modelo de lenguaje (los parches se fusionan de m×m en uno).
    pub fn tokens(&self, fusion: i32) -> i32 {
        self.rejilla.0 * self.rejilla.1 * self.rejilla.2 / (fusion * fusion)
    }

    /// Desde los bytes de la imagen (o un `data:` URL), como `Qwen3VLImageProcessor._process_one`.
    pub fn desde_bytes(bytes: &[u8], c: &VConfig, min_px: i64, max_px: i64) -> Result<Imagen> {
        let img = image::load_from_memory(bytes).context(tr!("no pude leer la imagen", "could not read the image"))?;
        // PIL `convert("RGB")`: el canal alfa se descarta (no se compone sobre un fondo)
        let rgb = img.to_rgb8();
        let (w, h) = (rgb.width() as usize, rgb.height() as usize);
        let factor = (c.patch_size * c.spatial_merge_size) as usize;
        let (nh, nw) = tamano_inteligente(h, w, factor, min_px, max_px)?;
        let datos = if (nh, nw) != (h, w) { bicubico_pil(rgb.as_raw(), w, h, nw, nh) } else { rgb.into_raw() };
        // (x/255 − media)/desv con media = desv = 0,5, en f32 como numpy
        let norm = |v: u8| ((v as f32) * (1.0 / 255.0) - 0.5) / 0.5;
        let (ps, ms, tps, ch) = (c.patch_size as usize, c.spatial_merge_size as usize, c.temporal_patch_size as usize, 3usize);
        let (gh, gw) = (nh / ps, nw / ps);
        // orden de salida: (bloque_fila, bloque_col, fila_en_bloque, col_en_bloque) × (C, T, P, P)
        let mut out = Vec::with_capacity(gh * gw * ch * tps * ps * ps);
        for bh in 0..gh / ms {
            for bw in 0..gw / ms {
                for mh in 0..ms {
                    for mw in 0..ms {
                        let (py, px) = ((bh * ms + mh) * ps, (bw * ms + mw) * ps);
                        for k in 0..ch {
                            for _t in 0..tps {
                                for y in 0..ps {
                                    for x in 0..ps {
                                        out.push(norm(datos[((py + y) * nw + (px + x)) * 3 + k]));
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        Ok(Imagen { pixeles: out, rejilla: (1, gh as i32, gw as i32) })
    }

    /// Desde un `data:image/…;base64,…`.
    pub fn desde_url(url: &str, c: &VConfig, min_px: i64, max_px: i64) -> Result<Imagen> {
        use base64::Engine as _;
        let b64 = url.split_once(',').map(|x| x.1).ok_or_else(|| anyhow_tr!("imagen sin datos", "image without data"))?;
        let bytes = base64::engine::general_purpose::STANDARD.decode(b64.trim()).context(tr!("imagen en base64 inválida", "invalid base64 image"))?;
        Imagen::desde_bytes(&bytes, c, min_px, max_px)
    }
}

/// `_smart_resize_image` de mlx_vlm (el `smart_resize` de Qwen2-VL).
fn tamano_inteligente(h: usize, w: usize, factor: usize, min_px: i64, max_px: i64) -> Result<(usize, usize)> {
    if h < factor || w < factor {
        bail_tr!("la imagen es muy pequeña ({w}×{h}); el mínimo es {factor} px por lado", "the image is too small ({w}×{h}); the minimum is {factor} px per side");
    }
    if h.max(w) as f64 / h.min(w) as f64 > 200.0 {
        bail_tr!("proporción de la imagen mayor que 200", "image aspect ratio above 200");
    }
    // `round` de Python: al par en los empates
    let red = |v: f64| v.round_ties_even();
    let (f, hf, wf) = (factor as f64, h as f64, w as f64);
    let mut hb = red(hf / f) * f;
    let mut wb = red(wf / f) * f;
    if (hb * wb) as i64 > max_px {
        let beta = (hf * wf / max_px as f64).sqrt();
        hb = f.max((hf / beta / f).floor() * f);
        wb = f.max((wf / beta / f).floor() * f);
    } else if ((hb * wb) as i64) < min_px {
        let beta = (min_px as f64 / (hf * wf)).sqrt();
        hb = (hf * beta / f).ceil() * f;
        wb = (wf * beta / f).ceil() * f;
    }
    Ok((hb as usize, wb as usize))
}

/// El bicúbico de PIL (`Image.resize(…, BICUBIC)`, a = −0,5) con su antialias y su aritmética de
/// punto fijo de 22 bits: primero en horizontal, luego en vertical, recortando a 0..255 tras cada
/// pasada — así salen los MISMOS bytes que en Python.
pub(crate) fn bicubico_pil(src: &[u8], w: usize, h: usize, nw: usize, nh: usize) -> Vec<u8> {
    const BITS: u32 = 22;
    fn cubica(x: f64) -> f64 {
        let a = -0.5;
        let x = x.abs();
        if x < 1.0 {
            ((a + 2.0) * x - (a + 3.0)) * x * x + 1.0
        } else if x < 2.0 {
            (((x - 5.0) * x + 8.0) * x - 4.0) * a
        } else {
            0.0
        }
    }
    // coeficientes de una dimensión: (primer índice, pesos en punto fijo)
    fn coefs(ent: usize, sal: usize) -> Vec<(usize, Vec<i64>)> {
        let escala = ent as f64 / sal as f64;
        let fs = escala.max(1.0);
        let soporte = 2.0 * fs;
        (0..sal)
            .map(|xx| {
                let centro = (xx as f64 + 0.5) * escala;
                let xmin = ((centro - soporte + 0.5) as i64).max(0) as usize;
                let xmax = (((centro + soporte + 0.5) as i64).min(ent as i64) as usize).saturating_sub(xmin);
                let mut k: Vec<f64> = (0..xmax).map(|x| cubica((x as f64 + xmin as f64 - centro + 0.5) / fs)).collect();
                let suma: f64 = k.iter().sum();
                if suma != 0.0 {
                    k.iter_mut().for_each(|v| *v /= suma);
                }
                let fijo = k.iter().map(|&v| if v < 0.0 { (-0.5 + v * (1u64 << BITS) as f64) as i64 } else { (0.5 + v * (1u64 << BITS) as f64) as i64 }).collect();
                (xmin, fijo)
            })
            .collect()
    }
    let recorta = |s: i64| -> u8 { (s >> BITS).clamp(0, 255) as u8 };
    let medio = 1i64 << (BITS - 1);
    // horizontal
    let mut tmp = vec![0u8; nw * h * 3];
    if nw != w {
        let ch = coefs(w, nw);
        for y in 0..h {
            for (xx, (x0, k)) in ch.iter().enumerate() {
                for c in 0..3 {
                    let mut s = medio;
                    for (i, kk) in k.iter().enumerate() {
                        s += src[(y * w + x0 + i) * 3 + c] as i64 * kk;
                    }
                    tmp[(y * nw + xx) * 3 + c] = recorta(s);
                }
            }
        }
    } else {
        tmp.copy_from_slice(src);
    }
    if nh == h {
        return tmp;
    }
    // vertical
    let cv = coefs(h, nh);
    let mut out = vec![0u8; nw * nh * 3];
    for (yy, (y0, k)) in cv.iter().enumerate() {
        for x in 0..nw {
            for c in 0..3 {
                let mut s = medio;
                for (i, kk) in k.iter().enumerate() {
                    s += tmp[((y0 + i) * nw + x) * 3 + c] as i64 * kk;
                }
                out[(yy * nw + x) * 3 + c] = recorta(s);
            }
        }
    }
    out
}

struct Lin {
    w_t: Array,
    b: Array,
}

impl Lin {
    fn cargar(p: &mut HashMap<String, Array>, n: &str) -> Result<Lin> {
        let w = p.remove(&format!("{n}.weight")).ok_or_else(|| anyhow!("falta {n}.weight"))?;
        let b = p.remove(&format!("{n}.bias")).ok_or_else(|| anyhow!("falta {n}.bias"))?;
        Ok(Lin { w_t: ops::transpose_axes(&w, &[1, 0]).m()?, b })
    }
    /// `nn.Linear` con sesgo es `addmm` (UNA operación, un redondeo): matmul + add por separado
    /// redondea dos veces en bf16 y la torre se separaba ya en el bloque 0 (coseno 0,99992).
    fn aplicar(&self, x: &Array) -> Result<Array> {
        ops::addmm(&self.b, x, &self.w_t, None, None).m()
    }
}

struct Norma {
    w: Array,
    b: Array,
}

impl Norma {
    fn cargar(p: &mut HashMap<String, Array>, n: &str) -> Result<Norma> {
        Ok(Norma {
            w: p.remove(&format!("{n}.weight")).ok_or_else(|| anyhow!("falta {n}.weight"))?,
            b: p.remove(&format!("{n}.bias")).ok_or_else(|| anyhow!("falta {n}.bias"))?,
        })
    }
    fn aplicar(&self, x: &Array) -> Result<Array> {
        fast::layer_norm(x, &self.w, &self.b, 1e-6).m()
    }
}

struct Bloque {
    norma1: Norma,
    norma2: Norma,
    qkv: Lin,
    proy: Lin,
    fc1: Lin,
    fc2: Lin,
}

pub struct Vision {
    pub c: VConfig,
    parche: Lin,
    pos: Array,
    bloques: Vec<Bloque>,
    fusion_norma: Norma,
    fusion1: Lin,
    fusion2: Lin,
    f: Fusiones,
}

impl Vision {
    /// ¿Este modelo trae una torre de visión que el núcleo sabe correr? (sólo el config)
    pub fn config(dir: &Path) -> Option<VConfig> {
        let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dir.join("config.json")).ok()?).ok()?;
        let vc = v.get("vision_config")?;
        let tipo = v["model_type"].as_str().unwrap_or("");
        if !matches!(tipo, "qwen3_5" | "qwen3_5_moe") || vc["model_type"].as_str().is_some_and(|t| !t.starts_with("qwen3_5")) {
            return None;
        }
        let c: VConfig = serde_json::from_value(vc.clone()).ok()?;
        c.deepstack_visual_indexes.is_empty().then_some(c)
    }

    pub fn cargar(dir: &Path, c: VConfig) -> Result<Vision> {
        let mut p = crate::modelo::cargar_pesos_de(dir, "vision_tower.")?;
        if p.is_empty() {
            bail_tr!("el modelo no trae la torre de visión (vision_tower.*)", "the model has no vision tower (vision_tower.*)");
        }
        // conv 3D de núcleo = paso → producto: [oculto, T, P, P, C] → [oculto, T·P·P·C]
        let w = p.remove("patch_embed.proj.weight").ok_or_else(|| anyhow!("falta patch_embed.proj.weight"))?;
        let forma = w.shape().to_vec();
        if forma.len() != 5 || forma[4] != c.in_channels {
            bail_tr!("patch_embed con forma {forma:?} (se esperaba [oculto, T, P, P, C])", "patch_embed with shape {forma:?} ([hidden, T, P, P, C] expected)");
        }
        let w = ops::reshape(&w, &[c.hidden_size, -1]).m()?;
        let b = p.remove("patch_embed.proj.bias").ok_or_else(|| anyhow!("falta patch_embed.proj.bias"))?;
        let parche = Lin { w_t: ops::transpose_axes(&w, &[1, 0]).m()?, b };
        let pos = p.remove("pos_embed.weight").ok_or_else(|| anyhow!("falta pos_embed.weight"))?;
        let mut bloques = Vec::new();
        for i in 0..c.depth {
            let n = format!("blocks.{i}");
            bloques.push(Bloque {
                norma1: Norma::cargar(&mut p, &format!("{n}.norm1"))?,
                norma2: Norma::cargar(&mut p, &format!("{n}.norm2"))?,
                qkv: Lin::cargar(&mut p, &format!("{n}.attn.qkv"))?,
                proy: Lin::cargar(&mut p, &format!("{n}.attn.proj"))?,
                fc1: Lin::cargar(&mut p, &format!("{n}.mlp.linear_fc1"))?,
                fc2: Lin::cargar(&mut p, &format!("{n}.mlp.linear_fc2"))?,
            });
        }
        Ok(Vision {
            fusion_norma: Norma::cargar(&mut p, "merger.norm")?,
            fusion1: Lin::cargar(&mut p, "merger.linear_fc1")?,
            fusion2: Lin::cargar(&mut p, "merger.linear_fc2")?,
            c,
            parche,
            pos,
            bloques,
            f: Fusiones::nuevas(),
        })
    }

    /// La posición de cada parche: la rejilla 48×48 aprendida, interpolada bilinealmente al tamaño
    /// de la imagen y reordenada por bloques de fusión (`fast_pos_embed_interpolate`).
    fn posicion(&self, h: i32, w: i32) -> Result<Array> {
        let n = (self.c.num_position_embeddings as f64).sqrt() as i32;
        // mx.linspace(0, n−1, k) en f32 EXACTO: MLX calcula (n−1)·(i/(k−1)), no i·paso (medido:
        // i·paso difiere en el último bit en 6–14 de cada 32–44 valores y la posición se movía)
        let lin = |k: i32| -> Vec<f32> {
            if k == 1 {
                return vec![0.0];
            }
            (0..k).map(|i| (n - 1) as f32 * (i as f32 / (k - 1) as f32)).collect()
        };
        let (hs, ws) = (lin(h), lin(w));
        let mut idx: [Vec<i32>; 4] = Default::default();
        let mut pes: [Vec<f32>; 4] = Default::default();
        for &y in &hs {
            let (y0, dy) = (y as i32, y - (y as i32) as f32);
            let y1 = (y0 + 1).min(n - 1);
            for &x in &ws {
                let (x0, dx) = (x as i32, x - (x as i32) as f32);
                let x1 = (x0 + 1).min(n - 1);
                for (k, (i, p)) in [(y0 * n + x0, (1.0 - dy) * (1.0 - dx)), (y0 * n + x1, (1.0 - dy) * dx), (y1 * n + x0, dy * (1.0 - dx)), (y1 * n + x1, dy * dx)].into_iter().enumerate() {
                    idx[k].push(i);
                    pes[k].push(p);
                }
            }
        }
        let tipo = self.pos.dtype();
        let mut suma: Option<Array> = None;
        for k in 0..4 {
            let filas = take_axis(&self.pos, Array::from_slice(&idx[k], &[idx[k].len() as i32]), 0).m()?;
            // los pesos van en el tipo de la tabla (bf16), como en mlx_vlm
            let p = Array::from_slice(&pes[k], &[pes[k].len() as i32, 1]).as_dtype(tipo).m()?;
            let t = ops::multiply(&filas, &p).m()?;
            suma = Some(match suma {
                None => t,
                Some(s) => ops::add(&s, &t).m()?,
            });
        }
        let d = self.c.hidden_size;
        let m = self.c.spatial_merge_size;
        let s = ops::reshape(suma.unwrap(), &[h / m, m, w / m, m, d]).m()?;
        ops::reshape(ops::transpose_axes(&s, &[0, 2, 1, 3, 4]).m()?, &[-1, d]).m()
    }

    /// cos y sin del RoPE 2D de los parches [N, cabeza] (`rot_pos_emb`): mitad alto, mitad ancho.
    fn giro(&self, h: i32, w: i32) -> Result<(Array, Array)> {
        let cabeza = self.c.hidden_size / self.c.num_heads;
        let dim = cabeza / 2;
        // frecuencias en la GPU como Python (`VisionRotaryEmbedding`): el pow de Metal ≠ powf
        let inv = crate::modelo::inv_frec(dim, 10000.0)?;
        let m = self.c.spatial_merge_size;
        let (mut filas, mut cols) = (Vec::with_capacity((h * w) as usize), Vec::with_capacity((h * w) as usize));
        for bh in 0..h / m {
            for bw in 0..w / m {
                for ih in 0..m {
                    for iw in 0..m {
                        filas.push((bh * m + ih) as f32);
                        cols.push((bw * m + iw) as f32);
                    }
                }
            }
        }
        let n = h * w;
        let porf = |v: &Vec<f32>| ops::multiply(Array::from_slice(v, &[n, 1]), &inv);
        let a = ops::concatenate(&[porf(&filas).m()?, porf(&cols).m()?], 1).m()?;
        // [N, dim] → [1, N, 1, cabeza] (repetido dos veces, `tile(…, 2)`)
        let cos = ops::cos(&a).m()?;
        let sin = ops::sin(&a).m()?;
        let doble = |x: Array| -> Result<Array> { ops::reshape(ops::concatenate(&[x.clone(), x], 1).m()?, &[1, h * w, 1, cabeza]).m() };
        Ok((doble(cos)?, doble(sin)?))
    }

    /// Los tokens visuales de UNA imagen: [N/m², salida].
    pub fn rasgos(&self, img: &Imagen) -> Result<Array> {
        self.correr(img, &mut None)
    }

    /// La torre, guardando sus etapas si se piden (para el juez: dónde se separa de mlx_vlm).
    fn correr(&self, img: &Imagen, etapas: &mut Option<Vec<(String, Array)>>) -> Result<Array> {
        let depurar = etapas.is_some();
        let mut guarda = |n: &str, a: &Array| {
            if let Some(e) = etapas.as_mut() {
                e.push((n.to_string(), a.clone()));
            }
        };
        let c = &self.c;
        let (_, h, w) = img.rejilla;
        let n = h * w;
        let fila = c.in_channels * c.temporal_patch_size * c.patch_size * c.patch_size;
        let tipo = self.pos.dtype();
        // [N, C, T, P, P] → [N, T, P, P, C] (el orden de la conv de MLX)
        let x = Array::from_slice(&img.pixeles, &[n, fila]).as_dtype(tipo).m()?;
        let x = ops::reshape(&x, &[n, c.in_channels, c.temporal_patch_size, c.patch_size, c.patch_size]).m()?;
        let x = ops::reshape(ops::transpose_axes(&x, &[0, 2, 3, 4, 1]).m()?, &[n, fila]).m()?;
        // la conv 3D de Python suma el sesgo DESPUÉS (no es addmm): matmul + add da los mismos bytes
        let parche = ops::add(&ops::matmul(&x, &self.parche.w_t).m()?, &self.parche.b).m()?;
        let posicion = self.posicion(h, w)?;
        guarda("parche", &parche);
        guarda("posicion", &posicion);
        let mut hs = ops::add(&parche, &posicion).m()?;
        let (cos, sin) = self.giro(h, w)?;
        let cabezas = c.num_heads;
        let cabeza = c.hidden_size / cabezas;
        let mitad = cabeza / 2;
        let escala = (cabeza as f32).powf(-0.5);
        // x·cos + rotar_mitad(x)·sin, de vuelta al tipo de x (`apply_rotary_pos_emb_vision`)
        let girar = |t: &Array| -> Result<Array> {
            let partes = ops::split_at_indices(t, &[mitad], -1).m()?;
            let rot = ops::concatenate(&[ops::negative(&partes[1]).m()?, partes[0].clone()], -1).m()?;
            ops::add(&ops::multiply(t, &cos).m()?, &ops::multiply(&rot, &sin).m()?).m()?.as_dtype(t.dtype()).m()
        };
        for (i, b) in self.bloques.iter().enumerate() {
            let y = b.norma1.aplicar(&hs)?;
            let qkv = ops::reshape(b.qkv.aplicar(&y)?, &[n, 3, cabezas, cabeza]).m()?;
            let qkv = ops::transpose_axes(&qkv, &[1, 0, 2, 3]).m()?; // [3, N, H, d]
            let q = girar(&qkv.try_index(0..1).m()?)?;
            let k = girar(&qkv.try_index(1..2).m()?)?;
            let v = qkv.try_index(2..3).m()?;
            let t = |a: &Array| ops::transpose_axes(a, &[0, 2, 1, 3]);
            let a = sdpa_fusionada(&t(&q).m()?, &t(&k).m()?, &t(&v).m()?, escala)?;
            let a = ops::reshape(ops::transpose_axes(&a, &[0, 2, 1, 3]).m()?, &[n, -1]).m()?;
            hs = ops::add(&hs, &b.proy.aplicar(&a)?).m()?;
            let y = b.norma2.aplicar(&hs)?;
            let y = b.fc2.aplicar(&(self.f.gelu_tanh.borrow_mut())(&b.fc1.aplicar(&y)?).m()?)?;
            hs = ops::add(&hs, &y).m()?;
            if depurar {
                guarda(&format!("bloque{i}"), &hs);
            }
        }
        // fusionador: norma por parche, 4 vecinos juntos, MLP con GeLU exacta
        let m2 = c.spatial_merge_size * c.spatial_merge_size;
        let y = ops::reshape(self.fusion_norma.aplicar(&hs)?, &[-1, c.hidden_size * m2]).m()?;
        let y = (self.f.gelu_exacta.borrow_mut())(&self.fusion1.aplicar(&y)?).m()?;
        self.fusion2.aplicar(&y)
    }
}

/// `ensure_fused_sdpa` de mlx_vlm: el kernel fusionado de MLX sólo admite cabezas de 64, 80 o 128;
/// la de 72 se rellena con ceros hasta 80 y se recorta después. Sin esto la atención va por otro
/// camino y redondea distinto (medido: la torre se separaba de mlx_vlm desde el bloque 0).
fn sdpa_fusionada(q: &Array, k: &Array, v: &Array, escala: f32) -> Result<Array> {
    let d = q.dim(-1);
    let meta = [64, 80, 128].into_iter().find(|&t| d <= t).unwrap_or(d);
    if meta == d {
        return fast::scaled_dot_product_attention(q, k, v, escala, None, None::<&Array>).m();
    }
    let rellena = |a: &Array| -> Result<Array> {
        let mut forma = a.shape().to_vec();
        *forma.last_mut().unwrap() = meta - d;
        ops::concatenate(&[a.clone(), ops::zeros::<f32>(&forma).m()?.as_dtype(a.dtype()).m()?], -1).m()
    };
    let o = fast::scaled_dot_product_attention(&rellena(q)?, &rellena(k)?, &rellena(v)?, escala, None, None::<&Array>).m()?;
    o.try_index((.., .., .., ..d)).m()
}

/// Las posiciones mRoPE (t, alto, ancho) de TODO el prompt (`get_rope_index`): el texto avanza de a
/// uno; una imagen ocupa una rejilla (t, h/m, w/m) que empieza donde iba el texto, y el texto
/// siguiente sigue desde la mayor posición + 1.
pub fn posiciones(ids: &[u32], token_imagen: u32, rejillas: &[(i32, i32, i32)], fusion: i32) -> Result<[Vec<i32>; 3]> {
    let mut p: [Vec<i32>; 3] = [Vec::with_capacity(ids.len()), Vec::with_capacity(ids.len()), Vec::with_capacity(ids.len())];
    let mut siguiente = 0i32;
    let mut i = 0usize;
    let mut r = rejillas.iter();
    while i < ids.len() {
        if ids[i] != token_imagen {
            for d in &mut p {
                d.push(siguiente);
            }
            siguiente += 1;
            i += 1;
            continue;
        }
        let &(t, h, w) = r.next().ok_or_else(|| anyhow_tr!("hay más imágenes en el prompt que rejillas", "there are more images in the prompt than grids"))?;
        let (gh, gw) = (h / fusion, w / fusion);
        let n = (t * gh * gw) as usize;
        if i + n > ids.len() || ids[i..i + n].iter().any(|&x| x != token_imagen) {
            bail_tr!("la imagen no ocupa sus {n} tokens seguidos", "the image does not take its {n} tokens in a row");
        }
        let base = siguiente;
        let mut maximo = base;
        for tt in 0..t {
            for hh in 0..gh {
                for ww in 0..gw {
                    let (a, b, c) = (base + tt, base + hh, base + ww);
                    p[0].push(a);
                    p[1].push(b);
                    p[2].push(c);
                    maximo = maximo.max(a).max(b).max(c);
                }
            }
        }
        siguiente = maximo + 1;
        i += n;
    }
    if r.next().is_some() {
        bail_tr!("sobran rejillas: el prompt tiene menos imágenes", "too many grids: the prompt has fewer images");
    }
    Ok(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn maxdif(a: &[f32], b: &[f32]) -> f32 {
        a.iter().zip(b).map(|(x, y)| (x - y).abs()).fold(0.0, f32::max)
    }

    /// Juez contra mlx_vlm en tres niveles (`ref_vision.py`):
    /// `GOTV_MLX_DIR=<9B> GOTV_REF_VISION=<dir con ref.json y ref.safetensors> GOTV_IMG=<png>
    /// cargo test -p gotv-mlx --release -- --ignored --nocapture ve_como_mlx_vlm`
    #[test]
    #[ignore]
    fn ve_como_mlx_vlm() {
        use crate::modelo::{cargar_red, Extra};
        let (Ok(dir), Ok(rd), Ok(imagen)) = (std::env::var("GOTV_MLX_DIR"), std::env::var("GOTV_REF_VISION"), std::env::var("GOTV_IMG")) else { return };
        let dir = Path::new(&dir);
        let rj: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(Path::new(&rd).join("ref.json")).unwrap()).unwrap();
        let rs = Array::load_safetensors(Path::new(&rd).join("ref.safetensors")).unwrap();
        let (ref_px, ref_rasgos) = (&rs["pixeles"], &rs["rasgos"]);
        let c = Vision::config(dir).expect("el modelo trae visión que el núcleo sabe correr");
        // 1 · preprocesado
        let img = Imagen::desde_bytes(&std::fs::read(&imagen).unwrap(), &c, 65536, 16_777_216).unwrap();
        let g = &rj["grid"][0];
        assert_eq!(img.rejilla, (g[0].as_i64().unwrap() as i32, g[1].as_i64().unwrap() as i32, g[2].as_i64().unwrap() as i32));
        let d1 = maxdif(&img.pixeles, ref_px.as_slice::<f32>());
        println!("1 · píxeles: {} valores, diferencia máxima {d1}", img.pixeles.len());
        assert!(d1 < 0.02, "el preprocesado difiere: {d1}");
        // 2 · torre (con los píxeles de la referencia: se juzga sólo la torre), etapa por etapa
        let v = Vision::cargar(dir, c.clone()).unwrap();
        if let Ok(et) = Array::load_safetensors(Path::new(&rd).join("etapas.safetensors")) {
            let im_ref = Imagen { pixeles: ref_px.as_slice::<f32>().to_vec(), rejilla: img.rejilla };
            let mut mias = Some(vec![]);
            v.correr(&im_ref, &mut mias).unwrap();
            for (n, a) in mias.unwrap() {
                let Some(r) = et.get(&n) else { continue };
                let a = a.as_dtype(mlx_rs::Dtype::Float32).unwrap();
                a.eval().unwrap();
                let (x, y) = (a.as_slice::<f32>(), r.as_slice::<f32>());
                let nx = x.iter().map(|v| v * v).sum::<f32>().sqrt();
                let ny = y.iter().map(|v| v * v).sum::<f32>().sqrt();
                let cs = x.iter().zip(y).map(|(p, q)| p * q).sum::<f32>() / (nx * ny);
                println!("   etapa {n:9} coseno {cs:.6} · dif. máx {}", maxdif(x, y));
            }
        }
        let t0 = std::time::Instant::now();
        let im_ref = Imagen { pixeles: ref_px.as_slice::<f32>().to_vec(), rejilla: img.rejilla };
        let f = v.rasgos(&im_ref).unwrap().as_dtype(mlx_rs::Dtype::Float32).unwrap();
        f.eval().unwrap();
        println!("2 · torre: {:?} en {:?}", f.shape(), t0.elapsed());
        let (a, b) = (f.as_slice::<f32>(), ref_rasgos.as_slice::<f32>());
        let d2 = maxdif(a, b);
        let norma = |x: &[f32]| x.iter().map(|v| v * v).sum::<f32>().sqrt();
        let coseno = a.iter().zip(b).map(|(x, y)| x * y).sum::<f32>() / (norma(a) * norma(b));
        println!("   diferencia máxima {d2} · coseno {coseno:.6}");
        assert!(coseno > 0.999, "la torre difiere: coseno {coseno}");
        // 3 · modelo completo con la imagen
        let ids: Vec<u32> = rj["ids"].as_array().unwrap().iter().map(|x| x.as_u64().unwrap() as u32).collect();
        let fusion = c.spatial_merge_size;
        let pos = posiciones(&ids, 248056, &[img.rejilla], fusion).unwrap();
        let rp = &rj["posiciones"];
        for d in 0..3 {
            let r: Vec<i32> = rp[d].as_array().unwrap().iter().map(|x| x.as_i64().unwrap() as i32).collect();
            assert_eq!(pos[d], r, "posiciones del eje {d}");
        }
        println!("3 · posiciones 3D idénticas ({} tokens)", ids.len());
        let m = cargar_red(dir).unwrap();
        let mut cache = m.cache_nueva();
        let mut k = 0;
        let cual: Vec<i32> = ids.iter().map(|&t| if t == 248056 { k += 1; k - 1 } else { -1 }).collect();
        let rasgos = v.rasgos(&img).unwrap();
        let l = m.adelante_ext(&ids, &Extra { rasgos: Some((&rasgos, cual)), pos: Some(pos.clone()) }, &mut cache).unwrap().as_dtype(mlx_rs::Dtype::Float32).unwrap();
        let orden = ops::argsort_axis(ops::negative(&l).unwrap(), -1).unwrap();
        let top: Vec<u32> = (0..5).map(|i| orden.try_index(i).unwrap().try_item_cast::<u32>().unwrap()).collect();
        let vals: Vec<f32> = top.iter().map(|&i| l.try_index(i as i32).unwrap().try_item_cast::<f32>().unwrap()).collect();
        println!("   top5 {top:?} {vals:?}\n   ref  {} {}", rj["top5"], rj["top5_val"]);
        let sig = pos.iter().map(|p| *p.iter().max().unwrap()).max().unwrap() + 1;
        let mut gen = vec![];
        let mut nxt = top[0];
        for paso in 0..30 {
            gen.push(nxt);
            let p = [vec![sig + paso], vec![sig + paso], vec![sig + paso]];
            let l = m.adelante_ext(&[nxt], &Extra { rasgos: None, pos: Some(p) }, &mut cache).unwrap();
            nxt = mlx_rs::ops::indexing::argmax_axis(&l, -1, false).unwrap().try_item_cast::<u32>().unwrap();
        }
        let refv: Vec<u32> = rj["voraz"].as_array().unwrap().iter().map(|x| x.as_u64().unwrap() as u32).collect();
        let iguales = gen.iter().zip(&refv).take_while(|(a, b)| a == b).count();
        println!("   voraz iguales {iguales}/30 · ref: {}", rj["texto"]);
        let hasta = refv.iter().position(|&t| t == 248046).map(|p| p + 1).unwrap_or(refv.len());
        assert_eq!(gen[..hasta], refv[..hasta], "la descripción difiere de mlx_vlm");
    }

    #[test]
    fn el_tamano_inteligente_es_el_de_mlx_vlm() {
        // múltiplos de 32 se quedan; el resto se redondea al par en los empates
        assert_eq!(tamano_inteligente(512, 512, 32, 65536, 16777216).unwrap(), (512, 512));
        assert_eq!(tamano_inteligente(768, 1024, 32, 65536, 16777216).unwrap(), (768, 1024));
        assert_eq!(tamano_inteligente(700, 1000, 32, 65536, 16777216).unwrap(), (704, 992));
        // por debajo del mínimo sube; por encima del máximo baja
        assert_eq!(tamano_inteligente(100, 100, 32, 65536, 16777216).unwrap(), (256, 256));
        assert!(tamano_inteligente(10, 100, 32, 65536, 16777216).is_err());
    }

    #[test]
    fn las_posiciones_de_una_imagen_ocupan_su_lado_mayor() {
        // 2 tokens de texto, una imagen 1×(4×6 parches → 2×3 tokens), 2 de texto
        let im = 9u32;
        let ids = [1, 2, im, im, im, im, im, im, 3, 4];
        let p = posiciones(&ids, im, &[(1, 4, 6)], 2).unwrap();
        assert_eq!(p[0], vec![0, 1, 2, 2, 2, 2, 2, 2, 5, 6]);
        assert_eq!(p[1], vec![0, 1, 2, 2, 2, 3, 3, 3, 5, 6]);
        assert_eq!(p[2], vec![0, 1, 2, 3, 4, 2, 3, 4, 5, 6]);
        assert!(posiciones(&ids, im, &[], 2).is_err());
    }
}
