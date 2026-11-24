//! M6 · video local: Wan 2.2 TI2V 5B (Wan-AI, Apache-2.0) sobre MLX, escrito a mano desde el
//! `WanPipeline` de diffusers 0.43 (repo `Wan-AI/Wan2.2-TI2V-5B-Diffusers`, commit b8fff7315c).
//! Se construye por fases (PLAN «M6 · Video en Rust, por fases»):
//! - **V1 · planificador UniPC** (este archivo): `UniPCMultistepScheduler` con la config del repo —
//!   `flow_prediction` + `use_flow_sigmas`, `flow_shift` 5, `solver_type` bh2, orden 2,
//!   `lower_order_final`, `final_sigmas_type` zero, `predict_x0`—. Predictor UniP y corrector UniC.
//! - V2 · UMT5-XXL · V3 · transformador · V4 · VAE · V5 · pipeline y MP4 (pendientes).

use crate::modelo::M;
use anyhow::Result;
use mlx_rs::ops;
use mlx_rs::ops::indexing::TryIndexOp;
use mlx_rs::Array;
use gotv_engine::{anyhow_tr, bail_tr};

// ───────────────────────────── V1 · el planificador UniPC ─────────────────────────────

/// Pasos de entrenamiento (`num_train_timesteps`).
const PASOS_ENTRENO: f64 = 1000.0;
/// `flow_shift` de la config del repo.
pub const DESPLAZAMIENTO: f64 = 5.0;

/// Sigmas (f32, con el 0 final) y pasos de tiempo (enteros) de `set_timesteps`, con la aritmética de
/// numpy: `linspace(1, 1/1000, n+1)[:-1]` en f64 (`i·paso + inicio`), el desplazamiento
/// `s·σ/(1 + (s−1)·σ)` en f64, el σ₀ = 1 corrido 1e-6, el paso de tiempo = `σ·1000` TRUNCADO a entero
/// (`astype(int64)`), y sólo al final las sigmas a f32.
pub fn calendario(pasos: usize, desplazamiento: f64) -> (Vec<f32>, Vec<i64>) {
    let n = pasos + 1;
    let inicio = 1.0f64;
    let fin = 1.0 / PASOS_ENTRENO;
    let paso = (fin - inicio) / (n - 1) as f64;
    let mut s: Vec<f64> = (0..n).map(|i| if i == n - 1 { fin } else { i as f64 * paso + inicio }).collect();
    s.truncate(pasos);
    for v in s.iter_mut() {
        *v = desplazamiento * *v / (1.0 + (desplazamiento - 1.0) * *v);
    }
    if let Some(v) = s.first_mut() {
        if (*v - 1.0).abs() < 1e-6 {
            *v -= 1e-6;
        }
    }
    let t = s.iter().map(|&v| (v * PASOS_ENTRENO) as i64).collect();
    let mut sig: Vec<f32> = s.iter().map(|&v| v as f32).collect();
    sig.push(0.0);
    (sig, t)
}

/// λ = log α − log σ con α = 1 − σ (flujo), todo en f32 como los tensores 0-d de torch.
fn lambda(sigma: f32) -> f32 {
    (1.0 - sigma).ln() - sigma.ln()
}

/// `torch.linalg.solve` de 2×2 (LU con pivote parcial, como LAPACK `sgesv`), en f32.
fn resolver2(r: [[f32; 2]; 2], b: [f32; 2]) -> [f32; 2] {
    let (mut a, mut b) = (r, b);
    if a[1][0].abs() > a[0][0].abs() {
        a.swap(0, 1);
        b.swap(0, 1);
    }
    let l = a[1][0] * (1.0 / a[0][0]);
    let u11 = a[1][1] - l * a[0][1];
    let y1 = b[1] - l * b[0];
    let x1 = y1 / u11;
    let x0 = (b[0] - a[0][1] * x1) / a[0][0];
    [x0, x1]
}

/// Los coeficientes `b` de UniPC (bh2) para un `hh` y un orden: `hφ_k·k!/B(h)`.
fn coef_b(hh: f32, orden: usize) -> (f32, Vec<f32>) {
    let h_phi_1 = hh.exp_m1();
    let mut h_phi_k = h_phi_1 / hh - 1.0;
    let b_h = hh.exp_m1();
    let mut factorial = 1.0f32;
    let mut b = Vec::with_capacity(orden);
    for i in 1..=orden {
        b.push(h_phi_k * factorial / b_h);
        factorial *= (i + 1) as f32;
        h_phi_k = h_phi_k / hh - 1.0 / factorial;
    }
    (h_phi_1, b)
}

fn escalar(v: f32) -> Array {
    Array::from_f32(v)
}

/// El planificador con su estado (las dos últimas predicciones de x₀ y la muestra anterior).
pub struct UniPc {
    pub sigmas: Vec<f32>,
    pub pasos: Vec<i64>,
    /// las predicciones de x₀ convertidas (la más nueva al final), como `model_outputs`
    salidas: Vec<Array>,
    ultima: Option<Array>,
    menores: usize,
    orden: usize,
    i: usize,
}

impl UniPc {
    pub fn nuevo(pasos: usize) -> UniPc {
        let (sigmas, pasos) = calendario(pasos, DESPLAZAMIENTO);
        UniPc { sigmas, pasos, salidas: Vec::new(), ultima: None, menores: 0, orden: 0, i: 0 }
    }

    /// Un paso: `salida` = lo que predice el modelo (velocidad del flujo) en la muestra `x` del paso
    /// actual; devuelve la muestra del paso siguiente. El mismo orden de operaciones (y de redondeos)
    /// que `UniPCMultistepScheduler.step`.
    pub fn paso(&mut self, salida: &Array, x: &Array) -> Result<Array> {
        let i = self.i;
        let sig = &self.sigmas;
        // convert_model_output (con la muestra SIN corregir): x₀ = x − σ·v
        let x0 = ops::subtract(x, &ops::multiply(escalar(sig[i]), salida).m()?).m()?;
        // el corrector UniC (desde el segundo paso), con el orden del paso anterior
        let x = match &self.ultima {
            Some(ultima) if i > 0 => self.corregir(&x0, ultima, x)?,
            _ => x.clone(),
        };
        if self.salidas.len() == 2 {
            self.salidas.remove(0);
        }
        self.salidas.push(x0);
        let tope = 2.min(self.pasos.len() - i); // lower_order_final
        self.orden = tope.min(self.menores + 1);
        self.ultima = Some(x.clone());
        let siguiente = self.predecir(&x)?;
        if self.menores < 2 {
            self.menores += 1;
        }
        self.i += 1;
        Ok(siguiente)
    }

    /// El predictor UniP (bh2): `multistep_uni_p_bh_update`.
    fn predecir(&self, x: &Array) -> Result<Array> {
        let i = self.i;
        let (sigma_t, sigma_s0) = (self.sigmas[i + 1], self.sigmas[i]);
        let alpha_t = 1.0 - sigma_t;
        let lambda_s0 = lambda(sigma_s0);
        let h = lambda(sigma_t) - lambda_s0;
        let m0 = &self.salidas[self.salidas.len() - 1];
        let hh = -h;
        let (h_phi_1, _) = coef_b(hh, self.orden);
        let b_h = hh.exp_m1();
        let base = ops::subtract(&ops::multiply(escalar(sigma_t / sigma_s0), x).m()?, &ops::multiply(escalar(alpha_t * h_phi_1), m0).m()?).m()?;
        if self.orden < 2 {
            return Ok(base);
        }
        // orden 2: D1 = (m₁ − m₀)/r₁ y ρ = ½
        let rk = (lambda(self.sigmas[i - 1]) - lambda_s0) / h;
        let d1 = ops::divide(&ops::subtract(&self.salidas[self.salidas.len() - 2], m0).m()?, escalar(rk)).m()?;
        let res = ops::multiply(escalar(0.5), &d1).m()?;
        ops::subtract(&base, &ops::multiply(escalar(alpha_t * b_h), &res).m()?).m()
    }

    /// El corrector UniC (bh2): `multistep_uni_c_bh_update` con `model_t` = la x₀ de ESTE paso.
    fn corregir(&self, model_t: &Array, ultima: &Array, _x: &Array) -> Result<Array> {
        let i = self.i;
        let orden = self.orden;
        let (sigma_t, sigma_s0) = (self.sigmas[i], self.sigmas[i - 1]);
        let alpha_t = 1.0 - sigma_t;
        let lambda_s0 = lambda(sigma_s0);
        let h = lambda(sigma_t) - lambda_s0;
        let m0 = &self.salidas[self.salidas.len() - 1];
        let hh = -h;
        let (h_phi_1, b) = coef_b(hh, orden);
        let b_h = hh.exp_m1();
        let base = ops::subtract(&ops::multiply(escalar(sigma_t / sigma_s0), ultima).m()?, &ops::multiply(escalar(alpha_t * h_phi_1), m0).m()?).m()?;
        let d1_t = ops::subtract(model_t, m0).m()?;
        let suma = if orden == 1 {
            ops::multiply(escalar(0.5), &d1_t).m()?
        } else {
            let rk = (lambda(self.sigmas[i - 2]) - lambda_s0) / h;
            let rho = resolver2([[1.0, 1.0], [rk, 1.0]], [b[0], b[1]]);
            let d1 = ops::divide(&ops::subtract(&self.salidas[self.salidas.len() - 2], m0).m()?, escalar(rk)).m()?;
            ops::add(&ops::multiply(escalar(rho[0]), &d1).m()?, &ops::multiply(escalar(rho[1]), &d1_t).m()?).m()?
        };
        ops::subtract(&base, &ops::multiply(escalar(alpha_t * b_h), &suma).m()?).m()
    }
}

// ───────────────────────────── V2 · el texto: UMT5-XXL ─────────────────────────────

/// Posiciones de texto (`max_sequence_length` = `text_len` de Wan).
pub const TEXTO: usize = 512;
/// El negativo por defecto del repo oficial (`wan/configs/shared_config.py`, `sample_neg_prompt`).
pub const NEGATIVO: &str = "色调艳丽，过曝，静态，细节模糊不清，字幕，风格，作品，画作，画面，静止，整体发灰，最差质量，低质量，JPEG压缩残留，丑陋的，残缺的，多余的手指，画得不好的手部，画得不好的脸部，畸形的，毁容的，形态畸形的肢体，手指融合，静止不动的画面，杂乱的背景，三条腿，背景人很多，倒着走";

const F32: mlx_rs::Dtype = mlx_rs::Dtype::Float32;
const BF16: mlx_rs::Dtype = mlx_rs::Dtype::Bfloat16;

/// Tope de la caché de búferes de MLX mientras vive; al soltarse devuelve el tope que había y vacía la
/// caché. Sin tope (la CLI y las pruebas no lo ponen; la app lo acota a 2 GB) MLX guarda cada búfer
/// liberado para reusarlo: con las tiras del VAE —de tamaños distintos, casi nunca reutilizables— el
/// proceso llegó a 49 GB con 20 GB vivos, el sistema entró en swap y cada trozo tardaba más que el
/// anterior (3 → 19 → 52 s; 674 s el VAE a 720p, medido 2026-11-13).
struct TopeCache(Option<usize>);

impl TopeCache {
    fn poner(bytes: usize) -> TopeCache {
        TopeCache(mlx_rs::memory::set_cache_limit(bytes).ok())
    }
}

impl Drop for TopeCache {
    fn drop(&mut self) {
        let _ = mlx_rs::memory::clear_cache();
        if let Some(a) = self.0 {
            let _ = mlx_rs::memory::set_cache_limit(a);
        }
    }
}

/// El tope del video: el mismo que pone el motor de la app (`CACHE_MLX`).
const CACHE_VIDEO: usize = 2 << 30;

/// `prompt_clean` del pipeline: `html.unescape` dos veces, espacios colapsados y recortados.
/// (`ftfy.fix_text` repara texto MAL decodificado; en texto bien formado no cambia nada.)
pub fn limpiar(t: &str) -> String {
    let d = crate::efectos::desescapar(&crate::efectos::desescapar(t.trim()));
    d.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Los ids del T5: el texto limpio, truncado a 511 piezas y `</s>` al final (como el tokenizador
/// rápido con `truncation=True, max_length=512`).
pub fn tokens(tk: &tokenizers::Tokenizer, texto: &str) -> Result<Vec<u32>> {
    let e = tk.encode(limpiar(texto), false).map_err(|e| anyhow::anyhow!("tokenizer T5: {e}"))?;
    let mut ids = e.get_ids().to_vec();
    ids.truncate(TEXTO - 1);
    ids.push(tk.token_to_id("</s>").ok_or_else(|| anyhow_tr!("tokenizer T5 sin </s>", "T5 tokenizer without </s>"))?);
    Ok(ids)
}

/// El cubo de posición relativa del T5 bidireccional (32 cubos, distancia máx. 128), con la aritmética
/// f32 de torch: `8 + int(log(r/8)/log(16)·8)`, tope 15, +16 si la clave va DESPUÉS de la consulta.
pub fn cubo(rel: i64) -> i32 {
    let (mitad, exactos) = (16i64, 8i64);
    let mut c = if rel > 0 { mitad } else { 0 };
    let r = rel.abs();
    c += if r < exactos {
        r
    } else {
        let l = ((r as f32) / exactos as f32).ln() / (128.0f64 / exactos as f64).ln() as f32 * (mitad - exactos) as f32;
        (exactos + l as i64).min(mitad - 1)
    };
    c as i32
}

/// RMSNorm del T5 (sin media ni sesgo) en f32: `w · x·rsqrt(mean(x²) + ε)`.
fn rms_t5(x: &Array, w: &Array) -> Result<Array> {
    let var = ops::mean_axes(&ops::square(x).m()?, &[-1], true).m()?;
    let n = ops::multiply(x, &ops::rsqrt(&ops::add(&var, escalar(1e-6)).m()?).m()?).m()?;
    ops::multiply(&w.as_dtype(F32).m()?, &n).m()
}

/// `x @ Wᵀ` con el peso guardado en bf16 y subido a f32 al usarlo (el juez carga el modelo en f32).
fn lineal(x: &Array, w: &Array) -> Result<Array> {
    ops::matmul(x, &ops::transpose_axes(&w.as_dtype(F32).m()?, &[1, 0]).m()?).m()
}

/// GELU «new» (tanh) de transformers.
fn gelu_new(x: &Array) -> Result<Array> {
    let c = (2.0f32 / std::f32::consts::PI).sqrt();
    let x3 = ops::multiply(&ops::multiply(x, x).m()?, x).m()?;
    let dentro = ops::multiply(escalar(c), &ops::add(x, &ops::multiply(escalar(0.044715), &x3).m()?).m()?).m()?;
    let t = ops::add(escalar(1.0), &ops::tanh(&dentro).m()?).m()?;
    ops::multiply(&ops::multiply(escalar(0.5), x).m()?, &t).m()
}

struct CapaT5 {
    n1: Array,
    q: Array,
    k: Array,
    v: Array,
    o: Array,
    /// `relative_attention_bias` [32, 64]: en el UMT5 cada capa trae el suyo
    sesgo: Array,
    n2: Array,
    wi0: Array,
    wi1: Array,
    wo: Array,
}

/// El codificador UMT5-XXL (`UMT5EncoderModel`): 24 capas de 4096, 64 cabezas de 64, FFN gated-GELU
/// de 10 240, atención SIN escala (T5) con sesgo de posición relativa por capa.
pub struct Umt5 {
    emb: Array,
    capas: Vec<CapaT5>,
    norma: Array,
}

const CABEZAS_T5: i32 = 64;

impl Umt5 {
    /// Lee `text_encoder/` (safetensors por partes, bf16: 11,4 GB en memoria).
    pub fn cargar(dir: &std::path::Path) -> Result<Umt5> {
        let mut p = std::collections::HashMap::new();
        let mut archivos: Vec<_> = std::fs::read_dir(dir)?.filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.extension().is_some_and(|x| x == "safetensors")).collect();
        archivos.sort();
        if archivos.is_empty() { let d = dir.display(); bail_tr!("no hay pesos del UMT5 en {d}", "no UMT5 weights in {d}"); }
        for a in archivos {
            p.extend(Array::load_safetensors(&a).map_err(|e| anyhow::anyhow!("UMT5 {}: {e:?}", a.display()))?);
        }
        let mut q = |n: &str| p.remove(n).ok_or_else(|| anyhow::anyhow!("UMT5: falta {n}"));
        let mut capas = Vec::with_capacity(24);
        for i in 0..24 {
            let b = format!("encoder.block.{i}.layer");
            capas.push(CapaT5 {
                n1: q(&format!("{b}.0.layer_norm.weight"))?,
                q: q(&format!("{b}.0.SelfAttention.q.weight"))?,
                k: q(&format!("{b}.0.SelfAttention.k.weight"))?,
                v: q(&format!("{b}.0.SelfAttention.v.weight"))?,
                o: q(&format!("{b}.0.SelfAttention.o.weight"))?,
                sesgo: q(&format!("{b}.0.SelfAttention.relative_attention_bias.weight"))?,
                n2: q(&format!("{b}.1.layer_norm.weight"))?,
                wi0: q(&format!("{b}.1.DenseReluDense.wi_0.weight"))?,
                wi1: q(&format!("{b}.1.DenseReluDense.wi_1.weight"))?,
                wo: q(&format!("{b}.1.DenseReluDense.wo.weight"))?,
            });
        }
        Ok(Umt5 { emb: q("shared.weight")?, norma: q("encoder.final_layer_norm.weight")?, capas })
    }

    /// Los estados ocultos [1, L, 4096] (f32) de las L piezas REALES. Con la máscara de relleno del
    /// pipeline las reales no ven el relleno, así que codificarlas solas da lo mismo.
    pub fn codificar(&self, ids: &[u32]) -> Result<Array> {
        let l = ids.len() as i32;
        let cubos: Vec<i32> = (0..l).flat_map(|q| (0..l).map(move |k| cubo((k - q) as i64))).collect();
        let cubos = Array::from_slice(&cubos, &[l, l]);
        let ids = Array::from_slice(&ids.iter().map(|&i| i as i32).collect::<Vec<_>>(), &[1, l]);
        let mut h = ops::indexing::take_axis(&self.emb, &ids, 0).m()?.as_dtype(F32).m()?;
        let cab = |x: &Array| -> Result<Array> { ops::transpose_axes(&ops::reshape(x, &[1, l, CABEZAS_T5, 64]).m()?, &[0, 2, 1, 3]).m() };
        for c in &self.capas {
            // sesgo [L, L, 64] → [1, 64, L, L]
            let s = ops::indexing::take_axis(&c.sesgo.as_dtype(F32).m()?, &cubos, 0).m()?;
            let s = ops::expand_dims(&ops::transpose_axes(&s, &[2, 0, 1]).m()?, 0).m()?;
            let n = rms_t5(&h, &c.n1)?;
            let a = mlx_rs::fast::scaled_dot_product_attention(&cab(&lineal(&n, &c.q)?)?, &cab(&lineal(&n, &c.k)?)?, &cab(&lineal(&n, &c.v)?)?, 1.0, Some(mlx_rs::fast::ScaledDotProductAttentionMask::Array(&s)), None::<&Array>).m()?;
            let a = ops::reshape(&ops::transpose_axes(&a, &[0, 2, 1, 3]).m()?, &[1, l, CABEZAS_T5 * 64]).m()?;
            h = ops::add(&h, &lineal(&a, &c.o)?).m()?;
            let n = rms_t5(&h, &c.n2)?;
            let f = ops::multiply(&gelu_new(&lineal(&n, &c.wi0)?)?, &lineal(&n, &c.wi1)?).m()?;
            h = ops::add(&h, &lineal(&f, &c.wo)?).m()?;
            h.eval().m()?; // capa a capa: el grafo no crece y la memoria pasajera se suelta
        }
        rms_t5(&h, &self.norma)
    }
}

/// [1, 512, 4096]: las posiciones reales y CEROS detrás (lo que hace `_get_t5_prompt_embeds`).
pub fn rellenar(h: &Array) -> Result<Array> {
    let l = h.dim(1);
    if l >= TEXTO as i32 {
        return Ok(h.clone());
    }
    let ceros = ops::zeros::<f32>(&[1, TEXTO as i32 - l, h.dim(2)]).m()?;
    ops::concatenate(&[h.clone(), ceros], 1).m()
}

// ───────────────────────────── V3 · el transformador 5B ─────────────────────────────

/// Lineal con sesgo (`addmm`, como `nn.Linear` en f32).
struct Lin {
    w_t: Array,
    b: Array,
}

impl Lin {
    fn de(p: &mut std::collections::HashMap<String, Array>, n: &str) -> Result<Lin> {
        Self::de_en(p, n, F32)
    }
    /// Con los pesos en `dt` (bf16 en producción). Se evalúa YA: la lectura perezosa del safetensors
    /// convierte peso a peso y los 20 GB en f32 nunca conviven con su copia.
    fn de_en(p: &mut std::collections::HashMap<String, Array>, n: &str, dt: mlx_rs::Dtype) -> Result<Lin> {
        let w = p.remove(&format!("{n}.weight")).ok_or_else(|| anyhow::anyhow!("Wan: falta {n}.weight"))?;
        let b = p.remove(&format!("{n}.bias")).ok_or_else(|| anyhow::anyhow!("Wan: falta {n}.bias"))?;
        let (w, b) = (w.as_dtype(dt).m()?, b.as_dtype(dt).m()?);
        mlx_rs::transforms::eval([&w, &b]).m()?;
        Ok(Lin { w_t: ops::transpose_axes(&w, &[1, 0]).m()?, b })
    }
    /// Entra y sale en f32; multiplica en la precisión de los pesos.
    fn ap(&self, x: &Array) -> Result<Array> {
        let dt = self.w_t.dtype();
        if dt == F32 {
            return ops::addmm(&self.b, x, &self.w_t, 1.0, 1.0).m();
        }
        ops::addmm(&self.b, &x.as_dtype(dt).m()?, &self.w_t, 1.0, 1.0).m()?.as_dtype(F32).m()
    }
}

fn silu(x: &Array) -> Result<Array> {
    ops::multiply(x, &ops::sigmoid(x).m()?).m()
}

/// RoPE 3D de Wan (`WanRotaryPosEmbed`): la cabeza de 128 se reparte en t/h/w = 44/42/42; cada parte
/// con su `get_1d_rotary_pos_embed` (θ = 10 000, f64 como en la CPU del juez) y la posición de su eje.
/// Devuelve cos y sin [1, L, 1, 64] (un valor por par), con L = F·H·W en el orden de los tokens.
pub fn giro_3d(f: i32, h: i32, w: i32) -> (Array, Array) {
    let partes = [(44usize, f), (42, h), (42, w)];
    let pares: usize = partes.iter().map(|(d, _)| d / 2).sum();
    let l = (f * h * w) as usize;
    let (mut c, mut s) = (Vec::with_capacity(l * pares), Vec::with_capacity(l * pares));
    let frec = |d: usize, j: usize| 1.0f64 / 10000f64.powf((2 * j) as f64 / d as f64);
    for pf in 0..f {
        for ph in 0..h {
            for pw in 0..w {
                for (k, &(d, _)) in partes.iter().enumerate() {
                    let pos = [pf, ph, pw][k] as f64;
                    for j in 0..d / 2 {
                        let a = pos * frec(d, j);
                        c.push(a.cos() as f32);
                        s.push(a.sin() as f32);
                    }
                }
            }
        }
    }
    (Array::from_slice(&c, &[1, l as i32, 1, pares as i32]), Array::from_slice(&s, &[1, l as i32, 1, pares as i32]))
}

/// `apply_rotary_emb` de `WanAttnProcessor` sobre [B, L, H·D]: pares (par, impar) girados.
fn girar(x: &Array, giro: &(Array, Array), h: i32) -> Result<Array> {
    let (b, l, dim) = (x.dim(0), x.dim(1), x.dim(2));
    let d = dim / h;
    let p = ops::reshape(x, &[b, l, h, d / 2, 2]).m()?;
    let (par, impar) = (p.try_index((.., .., .., .., 0)).m()?, p.try_index((.., .., .., .., 1)).m()?);
    let a = ops::subtract(&ops::multiply(&par, &giro.0).m()?, &ops::multiply(&impar, &giro.1).m()?).m()?;
    let bb = ops::add(&ops::multiply(&par, &giro.1).m()?, &ops::multiply(&impar, &giro.0).m()?).m()?;
    ops::reshape(&ops::stack(&[a, bb], -1).m()?, &[b, l, dim]).m()
}

fn cabezas(x: &Array, h: i32) -> Result<Array> {
    let (b, l, dim) = (x.dim(0), x.dim(1), x.dim(2));
    ops::transpose_axes(&ops::reshape(x, &[b, l, h, dim / h]).m()?, &[0, 2, 1, 3]).m()
}

fn juntar(x: &Array) -> Result<Array> {
    let (b, h, l, d) = (x.dim(0), x.dim(1), x.dim(2), x.dim(3));
    ops::reshape(&ops::transpose_axes(x, &[0, 2, 1, 3]).m()?, &[b, l, h * d]).m()
}

struct AtencionWan {
    q: Lin,
    k: Lin,
    v: Lin,
    o: Lin,
    nq: Array,
    nk: Array,
}

impl AtencionWan {
    fn de(p: &mut std::collections::HashMap<String, Array>, n: &str, dt: mlx_rs::Dtype) -> Result<AtencionWan> {
        let mut w = |k: &str| p.remove(&format!("{n}.{k}")).ok_or_else(|| anyhow::anyhow!("Wan: falta {n}.{k}")).and_then(|a| a.as_dtype(F32).m());
        let (nq, nk) = (w("norm_q.weight")?, w("norm_k.weight")?);
        let mut l = |k: &str| Lin::de_en(p, &format!("{n}.{k}"), dt);
        Ok(AtencionWan { q: l("to_q")?, k: l("to_k")?, v: l("to_v")?, o: l("to_out.0")?, nq, nk })
    }
}

struct BloqueWan {
    sst: Array,
    auto: AtencionWan,
    cruz: AtencionWan,
    n2_w: Array,
    n2_b: Array,
    ff1: Lin,
    ff2: Lin,
}

/// El paso de tiempo de cada token: uno solo (texto→video) o dos (imagen→video: 0 en los tokens del
/// primer cuadro, t en el resto). `primeros` = cuántos tokens van con 0.
#[derive(Clone, Copy, Debug)]
pub struct Tiempo {
    pub t: f32,
    pub primeros: i32,
}

/// La K/V de la atención cruzada de cada bloque (el texto no cambia entre pasos).
pub struct ContextoWan {
    kv: Vec<(Array, Array)>,
}

/// `WanTransformer3DModel` del 5B: parche 1×2×2 (48 → 3072), 30 bloques de 24 cabezas de 128, FFN
/// 14 336 con GELU tanh, norma cruzada afín, salida 3072 → 48·4 y des-parcheo.
pub struct Transformador {
    parche: Lin,
    txt1: Lin,
    txt2: Lin,
    t1: Lin,
    t2: Lin,
    tproy: Lin,
    bloques: Vec<BloqueWan>,
    sst_cabeza: Array,
    salida: Lin,
    /// La precisión de los bloques (pesos, multiplicaciones y atención): bf16 en producción, f32 en los jueces.
    dt: mlx_rs::Dtype,
}

const CABEZAS_WAN: i32 = 24;
const EPS_WAN: f32 = 1e-6;

impl Transformador {
    /// Lee `transformer/` (5 partes safetensors en f32, 20 GB) con los bloques en bf16.
    pub fn cargar(dir: &std::path::Path) -> Result<Transformador> {
        Self::cargar_en(dir, BF16)
    }

    /// Con los bloques en `dt`. f32 = el juez estricto (20 GB); bf16 = como la referencia oficial
    /// (pesos y multiplicaciones en bf16; residuo, normas, modulación y tiempo en f32): 10 GB.
    pub fn cargar_en(dir: &std::path::Path, dt: mlx_rs::Dtype) -> Result<Transformador> {
        let cfg: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dir.join("config.json"))?)?;
        let n = |k: &str| cfg.get(k).and_then(|v| v.as_i64()).unwrap_or(-1);
        anyhow::ensure!(
            n("num_layers") == 30 && n("num_attention_heads") == CABEZAS_WAN as i64 && n("in_channels") == 48 && cfg.get("patch_size") == Some(&serde_json::json!([1, 2, 2])),
            gotv_engine::tr!("sólo el transformador de Wan 2.2 TI2V 5B (30 capas, 24 cabezas, 48 canales, parche 1×2×2)", "only the Wan 2.2 TI2V 5B transformer (30 layers, 24 heads, 48 channels, 1×2×2 patch)")
        );
        let mut p = std::collections::HashMap::new();
        let mut archivos: Vec<_> = std::fs::read_dir(dir)?.filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.extension().is_some_and(|x| x == "safetensors")).collect();
        archivos.sort();
        for a in archivos {
            p.extend(Array::load_safetensors(&a).map_err(|e| anyhow::anyhow!("Wan {}: {e:?}", a.display()))?);
        }
        let mut bloques = Vec::with_capacity(30);
        for i in 0..30 {
            let b = format!("blocks.{i}");
            let mut w = |k: &str| p.remove(&format!("{b}.{k}")).ok_or_else(|| anyhow::anyhow!("Wan: falta {b}.{k}")).and_then(|a| a.as_dtype(F32).m());
            let (sst, n2_w, n2_b) = (w("scale_shift_table")?, w("norm2.weight")?, w("norm2.bias")?);
            bloques.push(BloqueWan {
                sst,
                n2_w,
                n2_b,
                auto: AtencionWan::de(&mut p, &format!("{b}.attn1"), dt)?,
                cruz: AtencionWan::de(&mut p, &format!("{b}.attn2"), dt)?,
                ff1: Lin::de_en(&mut p, &format!("{b}.ffn.net.0.proj"), dt)?,
                ff2: Lin::de_en(&mut p, &format!("{b}.ffn.net.2"), dt)?,
            });
        }
        // la conv 3D del parche (núcleo = paso = 1×2×2) es un lineal sobre (c, kt, kh, kw)
        let w = p.remove("patch_embedding.weight").ok_or_else(|| anyhow::anyhow!("Wan: falta patch_embedding"))?.as_dtype(F32).m()?;
        let o = w.dim(0);
        let parche = Lin { w_t: ops::transpose_axes(&ops::reshape(&w, &[o, -1]).m()?, &[1, 0]).m()?, b: p.remove("patch_embedding.bias").unwrap().as_dtype(F32).m()? };
        let t = Transformador {
            parche,
            txt1: Lin::de_en(&mut p, "condition_embedder.text_embedder.linear_1", dt)?,
            txt2: Lin::de_en(&mut p, "condition_embedder.text_embedder.linear_2", dt)?,
            t1: Lin::de(&mut p, "condition_embedder.time_embedder.linear_1")?,
            t2: Lin::de(&mut p, "condition_embedder.time_embedder.linear_2")?,
            tproy: Lin::de(&mut p, "condition_embedder.time_proj")?,
            bloques,
            sst_cabeza: p.remove("scale_shift_table").unwrap().as_dtype(F32).m()?,
            salida: Lin::de(&mut p, "proj_out")?,
            dt,
        };
        if !p.is_empty() {
            let mut sobran: Vec<_> = p.keys().cloned().collect();
            sobran.sort();
            let s = &sobran[..sobran.len().min(5)]; bail_tr!("pesos de Wan sin usar: {s:?}", "unused Wan weights: {s:?}");
        }
        Ok(t)
    }

    /// `text_embedder` (lineal → GELU tanh → lineal) y la K/V cruzada de cada bloque: [B, 512, 4096].
    pub fn contexto(&self, texto: &Array) -> Result<ContextoWan> {
        let c = self.txt2.ap(&gelu_new(&self.txt1.ap(texto)?)?)?;
        let kv = self
            .bloques
            .iter()
            .map(|b| -> Result<(Array, Array)> {
                let k = mlx_rs::fast::rms_norm(&b.cruz.k.ap(&c)?, Some(&b.cruz.nk), EPS_WAN).m()?;
                let (k, v) = (cabezas(&k, CABEZAS_WAN)?.as_dtype(self.dt).m()?, cabezas(&b.cruz.v.ap(&c)?, CABEZAS_WAN)?.as_dtype(self.dt).m()?);
                Ok((k, v))
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(ContextoWan { kv })
    }

    /// `Timesteps(256, flip_sin_to_cos)` en f32 como diffusers → `time_embedder` → (temb [1, D],
    /// modulación [1, 6, D]).
    fn tiempo(&self, t: f32) -> Result<(Array, Array)> {
        let medio = 128usize;
        let mut v = vec![0f32; 2 * medio];
        for j in 0..medio {
            let exponente = -(10000f64.ln() as f32) * j as f32 / medio as f32;
            let a = t * exponente.exp();
            v[j] = a.cos();
            v[medio + j] = a.sin();
        }
        let e = Array::from_slice(&v, &[1, 2 * medio as i32]);
        let te = self.t2.ap(&silu(&self.t1.ap(&e)?)?)?;
        let tm = self.tproy.ap(&silu(&te)?)?;
        let d = te.dim(-1);
        Ok((te, ops::reshape(&tm, &[1, 6, d]).m()?))
    }

    /// La modulación por token [1, L, D] (o [1, 1, D] si todos llevan el mismo t).
    fn por_token(t0: Option<&Array>, t: &Array, primeros: i32, l: i32) -> Result<Array> {
        match t0 {
            None => Ok(t.clone()),
            Some(t0) => {
                let idx: Vec<i32> = (0..l).map(|i| i32::from(i < primeros)).collect();
                let es0 = ops::reshape(Array::from_slice(&idx, &[l]), &[1, l, 1]).m()?;
                ops::select(&ops::eq(&es0, Array::from_int(1)).m()?, t0, t).m()
            }
        }
    }

    /// Una pasada: latentes [1, 48, F, H, W] (H y W pares), el tiempo y el contexto → velocidad con la
    /// misma forma. `giro` = `giro_3d(F, H/2, W/2)`.
    pub fn adelante(&self, lat: &Array, tiempo: Tiempo, ctx: &ContextoWan, giro: &(Array, Array), cancelar: Option<&std::sync::atomic::AtomicBool>) -> Result<Array> {
        let (c, f, h, w) = (lat.dim(1), lat.dim(2), lat.dim(3), lat.dim(4));
        let (hp, wp) = (h / 2, w / 2);
        let l = f * hp * wp;
        // parches: [1, C, F, H, W] → [1, F, H/2, W/2, C, 1, 2, 2] → [1, L, C·4]
        let x = ops::reshape(lat, &[1, c, f, hp, 2, wp, 2]).m()?;
        let x = ops::transpose_axes(&x, &[0, 2, 3, 5, 1, 4, 6]).m()?;
        let mut x = self.parche.ap(&ops::reshape(&x, &[1, l, c * 4]).m()?)?;
        let (te, tm) = self.tiempo(tiempo.t)?;
        let cero = if tiempo.primeros > 0 { Some(self.tiempo(0.0)?) } else { None };
        let uno = escalar(1.0);
        for (i, b) in self.bloques.iter().enumerate() {
            if cancelar.is_some_and(|c| c.load(std::sync::atomic::Ordering::Relaxed)) {
                anyhow::bail!("cancelado");
            }
            let m = ops::split_equal(&ops::add(&b.sst, &tm).m()?, 6, 1).m()?;
            let m0 = match &cero {
                Some((_, tm0)) => Some(ops::split_equal(&ops::add(&b.sst, tm0).m()?, 6, 1).m()?),
                None => None,
            };
            let md = |k: usize| Self::por_token(m0.as_ref().map(|v| &v[k]), &m[k], tiempo.primeros, l);
            // 1 · autoatención con RMSNorm de q/k sobre toda la dimensión y RoPE 3D
            let ln = mlx_rs::fast::layer_norm(&x, None::<&Array>, None::<&Array>, EPS_WAN).m()?;
            let entrada = ops::add(&ops::multiply(&ln, &ops::add(&uno, &md(1)?).m()?).m()?, &md(0)?).m()?;
            let q = girar(&mlx_rs::fast::rms_norm(&b.auto.q.ap(&entrada)?, Some(&b.auto.nq), EPS_WAN).m()?, giro, CABEZAS_WAN)?;
            let k = girar(&mlx_rs::fast::rms_norm(&b.auto.k.ap(&entrada)?, Some(&b.auto.nk), EPS_WAN).m()?, giro, CABEZAS_WAN)?;
            let v = b.auto.v.ap(&entrada)?;
            let escala = 1.0 / (128f32).sqrt();
            let en = |x: &Array| -> Result<Array> { cabezas(x, CABEZAS_WAN)?.as_dtype(self.dt).m() };
            let a = mlx_rs::fast::scaled_dot_product_attention(&en(&q)?, &en(&k)?, &en(&v)?, escala, None, None::<&Array>).m()?.as_dtype(F32).m()?;
            x = ops::add(&x, &ops::multiply(&b.auto.o.ap(&juntar(&a)?)?, &md(2)?).m()?).m()?;
            // 2 · atención cruzada (sin modulación) tras la norma afín, a las 512 posiciones de texto
            let n2 = mlx_rs::fast::layer_norm(&x, Some(&b.n2_w), Some(&b.n2_b), EPS_WAN).m()?;
            let qc = mlx_rs::fast::rms_norm(&b.cruz.q.ap(&n2)?, Some(&b.cruz.nq), EPS_WAN).m()?;
            let (kc, vc) = &ctx.kv[i];
            let a = mlx_rs::fast::scaled_dot_product_attention(&en(&qc)?, kc, vc, escala, None, None::<&Array>).m()?.as_dtype(F32).m()?;
            x = ops::add(&x, &b.cruz.o.ap(&juntar(&a)?)?).m()?;
            // 3 · FFN con su modulación
            let ln = mlx_rs::fast::layer_norm(&x, None::<&Array>, None::<&Array>, EPS_WAN).m()?;
            let entrada = ops::add(&ops::multiply(&ln, &ops::add(&uno, &md(4)?).m()?).m()?, &md(3)?).m()?;
            let ff = b.ff2.ap(&gelu_new(&b.ff1.ap(&entrada)?)?)?;
            x = ops::add(&x, &ops::multiply(&ff, &md(5)?).m()?).m()?;
            x.eval().m()?; // bloque a bloque: con miles de tokens el grafo entero no cabe
        }
        // cabeza: (desplazamiento, escala) = tabla + temb
        let so = ops::split_equal(&ops::add(&self.sst_cabeza, &ops::expand_dims(&te, 1).m()?).m()?, 2, 1).m()?;
        let so0 = match &cero {
            Some((te0, _)) => Some(ops::split_equal(&ops::add(&self.sst_cabeza, &ops::expand_dims(te0, 1).m()?).m()?, 2, 1).m()?),
            None => None,
        };
        let sd = |k: usize| Self::por_token(so0.as_ref().map(|v| &v[k]), &so[k], tiempo.primeros, l);
        let ln = mlx_rs::fast::layer_norm(&x, None::<&Array>, None::<&Array>, EPS_WAN).m()?;
        let y = self.salida.ap(&ops::add(&ops::multiply(&ln, &ops::add(&uno, &sd(1)?).m()?).m()?, &sd(0)?).m()?)?;
        // des-parcheo: [1, L, (1·2·2·C)] → [1, F, H/2, W/2, 1, 2, 2, C] → [1, C, F, H, W]
        let y = ops::reshape(&y, &[1, f, hp, wp, 1, 2, 2, c]).m()?;
        let y = ops::transpose_axes(&y, &[0, 7, 1, 4, 2, 5, 3, 6]).m()?;
        ops::reshape(&y, &[1, c, f, h, w]).m()
    }
}

// ───────────────────────────── V4 · el VAE de Wan 2.2 (decodificador) ─────────────────────────────
// Todo en canales al final ([1, T, H, W, C], el formato de MLX). El decodificador va CUADRO LATENTE A
// CUADRO LATENTE con cachés causales, exactamente como `AutoencoderKLWan._decode`: el primero da 1
// cuadro de video y cada uno de los siguientes 4.

/// Convolución causal 3D (`WanCausalConv3d`): relleno temporal de `kt − 1` cuadros DELANTE (ceros o la
/// caché del trozo anterior) y espacial simétrico de `ks/2`. Va como `kt` convoluciones 2D (una por
/// toma temporal, con los cuadros en el lote) sumadas: la misma cuenta que `conv3d`.
struct Conv3 {
    /// una por toma temporal: [O, kh, kw, I]
    tomas: Vec<Array>,
    b: Array,
    ks: i32,
}

/// Presupuesto del DESPLIEGUE de cada `conv2d` (bytes). Las conv de MLX con muchos canales despliegan
/// su entrada (im2col: ×9 con 3×3; ×27 el `conv3d`) y Metal retiene esos temporales hasta cerrar el
/// lote. Medido 2026-11-13 a 720p (4×352×640, 512→512, 3×3×3): `conv3d` entero ≈ 25 GB pasajeros (53 GB
/// de pico en el VAE); `conv3d` por tiras de 1 GB 1,36 s; `conv2d` por toma con los 4 cuadros en lote
/// 0,68 s pero 36 GB; por toma y cuadro a cuadro 0,78 s y 14 GB. Por eso: tomas 2D, tantos cuadros por
/// llamada como quepan en el presupuesto y, si ni uno cabe, bandas de filas con su borde; cada grupo se
/// evalúa al acabar. El resultado es el mismo (`conv_por_tiras_como_entera`).
const CONV_DESPLIEGUE: i64 = 2 << 30;

impl Conv3 {
    /// `w` en [O, kt, kh, kw, I].
    fn nueva(w: &Array, b: Array) -> Result<Conv3> {
        let (kt, ks) = (w.dim(1), w.dim(2));
        let tomas = (0..kt)
            .map(|j| -> Result<Array> {
                let a = ops::squeeze_axes(&w.try_index((.., j..j + 1)).m()?, &[1]).m()?;
                a.eval().m()?;
                Ok(a)
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Conv3 { tomas, b, ks })
    }
    fn de(p: &mut std::collections::HashMap<String, Array>, n: &str) -> Result<Conv3> {
        let w = p.remove(&format!("{n}.weight")).ok_or_else(|| anyhow::anyhow!("VAE: falta {n}.weight"))?.as_dtype(F32).m()?;
        let w = ops::transpose_axes(&w, &[0, 2, 3, 4, 1]).m()?; // [O, I, kt, kh, kw] → [O, kt, kh, kw, I]
        Conv3::nueva(&w, p.remove(&format!("{n}.bias")).ok_or_else(|| anyhow::anyhow!("VAE: falta {n}.bias"))?.as_dtype(F32).m()?)
    }
    fn kt(&self) -> i32 {
        self.tomas.len() as i32
    }
    fn ap(&self, x: &Array, cache: Option<&Array>) -> Result<Array> {
        self.ap_con(x, cache, CONV_DESPLIEGUE)
    }
    fn ap_con(&self, x: &Array, cache: Option<&Array>, presupuesto: i64) -> Result<Array> {
        let kt = self.kt();
        let mut x = x.clone();
        if kt > 1 {
            let mut delante = Vec::new();
            let hay = cache.map_or(0, |c| c.dim(1));
            if kt - 1 > hay {
                delante.push(ops::zeros::<f32>(&[1, kt - 1 - hay, x.dim(2), x.dim(3), x.dim(4)]).m()?);
            }
            if let Some(c) = cache {
                delante.push(c.clone());
            }
            delante.push(x);
            x = ops::concatenate(&delante, 1).m()?;
        }
        let pad = self.ks / 2;
        let (t, h, w, c) = (x.dim(1) - (kt - 1), x.dim(2), x.dim(3), x.dim(4));
        let xp = if pad > 0 { ops::pad(&x, &[(0, 0), (0, 0), (pad, pad), (pad, pad), (0, 0)][..], None, None).m()? } else { x };
        // bytes desplegados por fila de salida de un cuadro, en UNA toma
        let por_fila = w as i64 * (self.ks * self.ks * c) as i64 * 4;
        let por_cuadro = por_fila * h as i64;
        let (grupo, filas) = if por_cuadro <= presupuesto { ((presupuesto / por_cuadro).clamp(1, t as i64) as i32, h) } else { (1, (presupuesto / por_fila).clamp(1, h as i64) as i32) };
        let trocea = grupo < t || filas < h;
        let mut grupos = Vec::new();
        let mut o0 = 0;
        while o0 < t {
            let o1 = (o0 + grupo).min(t);
            let mut bandas = Vec::new();
            let mut r0 = 0;
            while r0 < h {
                let r1 = (r0 + filas).min(h);
                let mut y: Option<Array> = None;
                for (j, wj) in self.tomas.iter().enumerate() {
                    let j = j as i32;
                    let pieza = xp.try_index((.., o0 + j..o1 + j, r0..r1 + 2 * pad)).m()?;
                    let pieza = ops::reshape(&pieza, &[o1 - o0, r1 - r0 + 2 * pad, w + 2 * pad, c]).m()?;
                    let yj = ops::conv2d(&pieza, wj, (1, 1), (0, 0), (1, 1), 1).m()?;
                    y = Some(match y {
                        Some(a) => ops::add(&a, &yj).m()?,
                        None => yj,
                    });
                }
                let y = ops::add(y.expect("kt ≥ 1"), &self.b).m()?;
                if trocea {
                    y.eval().m()?;
                }
                bandas.push(y);
                r0 = r1;
            }
            grupos.push(if bandas.len() == 1 { bandas.pop().unwrap() } else { ops::concatenate(&bandas, 1).m()? });
            o0 = o1;
        }
        let y = if grupos.len() == 1 { grupos.pop().unwrap() } else { ops::concatenate(&grupos, 0).m()? };
        ops::expand_dims(&y, 0).m()
    }
}

/// `WanRMS_norm` sobre los canales: `x/max(‖x‖, 1e-12)·√C·γ`.
fn rms_canal(x: &Array, gamma: &Array) -> Result<Array> {
    let c = x.dim(-1);
    let n = ops::sqrt(&ops::sum_axes(&ops::square(x).m()?, &[-1], true).m()?).m()?;
    let y = ops::divide(x, &ops::maximum(&n, escalar(1e-12)).m()?).m()?;
    ops::multiply(&ops::multiply(&y, escalar((c as f32).sqrt())).m()?, gamma).m()
}

/// Las cachés causales de un trozo a otro: una ranura por convolución, en orden de recorrido.
#[derive(Clone)]
enum Cache {
    Nada,
    /// la marca de `upsample3d` en el primer trozo: aún no hubo convolución temporal
    Rep,
    Hay(Array),
}

struct Cachés {
    v: Vec<Cache>,
    i: usize,
}

impl Cachés {
    fn ranura(&mut self) -> usize {
        if self.i == self.v.len() {
            self.v.push(Cache::Nada);
        }
        self.i += 1;
        self.i - 1
    }
}

/// Los últimos ≤ 2 cuadros de `x`; si hay menos de 2 y la ranura ya guardaba algo, delante su último.
fn guardar(x: &Array, previo: &Cache) -> Result<Array> {
    let t = x.dim(1);
    let ult = x.try_index((.., (t - 2).max(0)..)).m()?;
    match previo {
        Cache::Hay(p) if ult.dim(1) < 2 => {
            let pt = p.dim(1);
            ops::concatenate(&[p.try_index((.., pt - 1..)).m()?, ult], 1).m()
        }
        // una COPIA: la vista de los 2 últimos cuadros mantendría vivo el búfer entero del trozo
        _ => ops::add(&ult, escalar(0.0)).m(),
    }
}

/// Una convolución causal con su ranura de caché (la regla de `WanResidualBlock` y del decodificador).
fn causal(c: &Conv3, x: &Array, k: &mut Cachés) -> Result<Array> {
    let r = k.ranura();
    let nueva = guardar(x, &k.v[r])?;
    let y = c.ap(x, if let Cache::Hay(a) = &k.v[r] { Some(a) } else { None })?;
    k.v[r] = Cache::Hay(nueva);
    Ok(y)
}

struct ResVae {
    n1: Array,
    c1: Conv3,
    n2: Array,
    c2: Conv3,
    atajo: Option<Conv3>,
}

impl ResVae {
    fn de(p: &mut std::collections::HashMap<String, Array>, n: &str) -> Result<ResVae> {
        let mut g = |k: &str| p.remove(&format!("{n}.{k}")).ok_or_else(|| anyhow::anyhow!("VAE: falta {n}.{k}")).and_then(|a| ops::reshape(&a.as_dtype(F32).m()?, &[-1]).m());
        let (n1, n2) = (g("norm1.gamma")?, g("norm2.gamma")?);
        let atajo = if p.contains_key(&format!("{n}.conv_shortcut.weight")) { Some(Conv3::de(p, &format!("{n}.conv_shortcut"))?) } else { None };
        Ok(ResVae { n1, n2, c1: Conv3::de(p, &format!("{n}.conv1"))?, c2: Conv3::de(p, &format!("{n}.conv2"))?, atajo })
    }
    fn ap(&self, x: &Array, k: &mut Cachés) -> Result<Array> {
        let h = match &self.atajo {
            Some(a) => a.ap(x, None)?,
            None => x.clone(),
        };
        let y = silu(&rms_canal(x, &self.n1)?)?;
        let y = causal(&self.c1, &y, k)?;
        let y = silu(&rms_canal(&y, &self.n2)?)?;
        let y = causal(&self.c2, &y, k)?;
        ops::add(&y, &h).m()
    }
}

/// La atención espacial del bloque medio (una cabeza de 1024, por cuadro).
struct AtVae {
    n: Array,
    qkv_w: Array,
    qkv_b: Array,
    pw: Array,
    pb: Array,
}

impl AtVae {
    fn ap(&self, x: &Array) -> Result<Array> {
        let (t, h, w, c) = (x.dim(1), x.dim(2), x.dim(3), x.dim(4));
        let y = ops::reshape(&rms_canal(x, &self.n)?, &[t, h * w, c]).m()?;
        let qkv = ops::addmm(&self.qkv_b, &y, &self.qkv_w, 1.0, 1.0).m()?;
        let partes = ops::split_equal(&qkv, 3, -1).m()?;
        let u = |a: &Array| ops::expand_dims(a, 1).m();
        let a = mlx_rs::fast::scaled_dot_product_attention(&u(&partes[0])?, &u(&partes[1])?, &u(&partes[2])?, 1.0 / (c as f32).sqrt(), None, None::<&Array>).m()?;
        let a = ops::reshape(&a, &[t, h * w, c]).m()?;
        let a = ops::addmm(&self.pb, &a, &self.pw, 1.0, 1.0).m()?;
        ops::add(&ops::reshape(&a, &[1, t, h, w, c]).m()?, x).m()
    }
}

/// Un bloque de subida residual (`WanResidualUpBlock`).
struct SubeVae {
    res: Vec<ResVae>,
    /// `time_conv` de `upsample3d` (dim → 2·dim)
    tiempo: Option<Conv3>,
    /// la conv 2D 3×3 tras el vecino más cercano ×2, como `Conv3` de kt = 1 (va por tiras igual)
    espacial: Option<Conv3>,
    /// `DupUp3D` del atajo: (canales de salida, factor temporal)
    dup: Option<(i32, i32)>,
}

impl SubeVae {
    fn ap(&self, x: &Array, k: &mut Cachés, primero: bool) -> Result<Array> {
        let copia = x.clone();
        let mut y = x.clone();
        for r in &self.res {
            y = r.ap(&y, k)?;
        }
        if let Some(e) = &self.espacial {
            if let Some(tc) = &self.tiempo {
                // upsample3d: el primer trozo NO duplica en el tiempo (sólo deja la marca «Rep»)
                let r = k.ranura();
                match k.v[r].clone() {
                    Cache::Nada => k.v[r] = Cache::Rep,
                    previo => {
                        let t = y.dim(1);
                        let mut nueva = ops::add(&y.try_index((.., (t - 2).max(0)..)).m()?, escalar(0.0)).m()?;
                        if nueva.dim(1) < 2 {
                            nueva = match &previo {
                                Cache::Hay(p) => ops::concatenate(&[p.try_index((.., p.dim(1) - 1..)).m()?, nueva], 1).m()?,
                                _ => ops::concatenate(&[ops::zeros_like(&nueva).m()?, nueva], 1).m()?,
                            };
                        }
                        let z = tc.ap(&y, if let Cache::Hay(a) = &previo { Some(a) } else { None })?;
                        k.v[r] = Cache::Hay(nueva);
                        // [1, T, H, W, 2C] → cuadros (t, 0), (t, 1)
                        let (h, w, c) = (z.dim(2), z.dim(3), z.dim(4) / 2);
                        let z = ops::reshape(&z, &[1, t, h, w, 2, c]).m()?;
                        y = ops::reshape(&ops::transpose_axes(&z, &[0, 1, 4, 2, 3, 5]).m()?, &[1, 2 * t, h, w, c]).m()?;
                    }
                }
            }
            // vecino más cercano ×2 y la conv 3×3 por cuadro
            let z = ops::repeat_axis::<f32>(ops::repeat_axis::<f32>(y, 2, 2).m()?, 2, 3).m()?;
            y = e.ap(&z, None)?;
        }
        if let Some((salida, ft)) = self.dup {
            // DupUp3D: canales repetidos → (o, ft, fs, fs) repartidos en (tiempo, alto, ancho)
            let (t, h, w, cin) = (copia.dim(1), copia.dim(2), copia.dim(3), copia.dim(4));
            let rep = salida * ft * 4 / cin;
            let z = ops::repeat_axis::<f32>(copia, rep, 4).m()?;
            let z = ops::reshape(&z, &[1, t, h, w, salida, ft, 2, 2]).m()?;
            let z = ops::transpose_axes(&z, &[0, 1, 5, 2, 6, 3, 7, 4]).m()?;
            let mut z = ops::reshape(&z, &[1, t * ft, h * 2, w * 2, salida]).m()?;
            if primero && ft > 1 {
                z = z.try_index((.., ft - 1..)).m()?;
            }
            y = ops::add(&y, &z).m()?;
        }
        Ok(y)
    }
}

/// El decodificador del VAE de Wan 2.2 (z 48, ×16 espacial con el parche 2, ×4 temporal).
pub struct VaeWan {
    pq: Conv3,
    cin: Conv3,
    medio: [ResVae; 2],
    at: AtVae,
    sube: Vec<SubeVae>,
    nout: Array,
    cout: Conv3,
    media: Array,
    inv_std: Array,
}

impl VaeWan {
    /// Lee `vae/` (sólo el decodificador y `post_quant_conv`; el codificador, para imagen→video, en V5).
    pub fn cargar(dir: &std::path::Path) -> Result<VaeWan> {
        let cfg: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dir.join("config.json"))?)?;
        anyhow::ensure!(cfg["z_dim"] == 48 && cfg["is_residual"] == true && cfg["decoder_base_dim"] == 256 && cfg["patch_size"] == 2, gotv_engine::tr!("sólo el VAE de Wan 2.2 (z 48, residual, base 256, parche 2)", "only the Wan 2.2 VAE (z 48, residual, base 256, patch 2)"));
        let vec_f32 = |k: &str| -> Result<Vec<f32>> { cfg[k].as_array().ok_or_else(|| anyhow_tr!("VAE sin {k}", "VAE without {k}"))?.iter().map(|v| v.as_f64().map(|f| f as f32).ok_or_else(|| anyhow::anyhow!("{k}"))).collect() };
        let media = Array::from_slice(&vec_f32("latents_mean")?, &[48]);
        let std = vec_f32("latents_std")?;
        let inv_std = Array::from_slice(&std.iter().map(|s| 1.0f32 / s).collect::<Vec<_>>(), &[48]);
        let mut p = Array::load_safetensors(dir.join("diffusion_pytorch_model.safetensors")).map_err(|e| anyhow::anyhow!("VAE: {e:?}"))?;
        p.retain(|k, _| k.starts_with("decoder.") || k.starts_with("post_quant_conv."));
        let mut g = |k: &str| p.remove(k).ok_or_else(|| anyhow::anyhow!("VAE: falta {k}")).and_then(|a| a.as_dtype(F32).m());
        let lin1x1 = |a: Array| -> Result<Array> { ops::transpose_axes(&ops::reshape(&a, &[a.dim(0), a.dim(1)]).m()?, &[1, 0]).m() };
        let at = AtVae {
            n: ops::reshape(&g("decoder.mid_block.attentions.0.norm.gamma")?, &[-1]).m()?,
            qkv_w: lin1x1(g("decoder.mid_block.attentions.0.to_qkv.weight")?)?,
            qkv_b: g("decoder.mid_block.attentions.0.to_qkv.bias")?,
            pw: lin1x1(g("decoder.mid_block.attentions.0.proj.weight")?)?,
            pb: g("decoder.mid_block.attentions.0.proj.bias")?,
        };
        let nout = ops::reshape(&g("decoder.norm_out.gamma")?, &[-1]).m()?;
        // (entrada, salida, ¿sube?, ¿en el tiempo?) de los 4 bloques: [1024, 1024, 1024, 512, 256]
        let planes = [(1024, 1024, true, true), (1024, 1024, true, true), (1024, 512, true, false), (512, 256, false, false)];
        let mut sube = Vec::new();
        for (i, &(_entrada, salida, arriba, tiempo)) in planes.iter().enumerate() {
            let b = format!("decoder.up_blocks.{i}");
            let res = (0..3).map(|j| ResVae::de(&mut p, &format!("{b}.resnets.{j}"))).collect::<Result<Vec<_>>>()?;
            let tiempo_c = if tiempo { Some(Conv3::de(&mut p, &format!("{b}.upsampler.time_conv"))?) } else { None };
            let espacial = if arriba {
                let w = p.remove(&format!("{b}.upsampler.resample.1.weight")).ok_or_else(|| anyhow_tr!("VAE: falta la conv de subida {i}", "VAE: the upsampling conv {i} is missing"))?.as_dtype(F32).m()?;
                let bb = p.remove(&format!("{b}.upsampler.resample.1.bias")).unwrap().as_dtype(F32).m()?;
                Some(Conv3::nueva(&ops::expand_dims(&ops::transpose_axes(&w, &[0, 2, 3, 1]).m()?, 1).m()?, bb)?)
            } else {
                None
            };
            sube.push(SubeVae { res, tiempo: tiempo_c, espacial, dup: if arriba { Some((salida, if tiempo { 2 } else { 1 })) } else { None } });
        }
        let v = VaeWan {
            pq: Conv3::de(&mut p, "post_quant_conv")?,
            cin: Conv3::de(&mut p, "decoder.conv_in")?,
            medio: [ResVae::de(&mut p, "decoder.mid_block.resnets.0")?, ResVae::de(&mut p, "decoder.mid_block.resnets.1")?],
            at,
            sube,
            nout,
            cout: Conv3::de(&mut p, "decoder.conv_out")?,
            media,
            inv_std,
        };
        if !p.is_empty() {
            let mut sobran: Vec<_> = p.keys().cloned().collect();
            sobran.sort();
            let s = &sobran[..sobran.len().min(5)]; bail_tr!("pesos del VAE sin usar: {s:?}", "unused VAE weights: {s:?}");
        }
        Ok(v)
    }

    /// Latentes normalizados del transformador [1, 48, F, h, w] → video [1, 1 + 4(F−1), 16h, 16w, 3] en
    /// [−1, 1].
    pub fn decodificar(&self, z: &Array, cancelar: Option<&std::sync::atomic::AtomicBool>) -> Result<Array> {
        self.decodificar_con(z, cancelar, &|_, _| {})
    }

    /// Igual, avisando `avance(hechos, total)` al terminar cada cuadro latente (a 720p cada uno son
    /// ~11 s).
    pub fn decodificar_con(&self, z: &Array, cancelar: Option<&std::sync::atomic::AtomicBool>, avance: &dyn Fn(u32, u32)) -> Result<Array> {
        let _tope = TopeCache::poner(CACHE_VIDEO);
        // desnormalizar: z / (1/σ) + μ, en canales al final
        let z = ops::transpose_axes(z, &[0, 2, 3, 4, 1]).m()?;
        let z = ops::add(&ops::divide(&z, &self.inv_std).m()?, &self.media).m()?;
        let x = self.pq.ap(&z, None)?;
        let mut k = Cachés { v: Vec::new(), i: 0 };
        let mut trozos = Vec::new();
        let n = x.dim(1);
        for i in 0..n {
            if cancelar.is_some_and(|c| c.load(std::sync::atomic::Ordering::Relaxed)) {
                anyhow::bail!("cancelado");
            }
            k.i = 0;
            let mut y = causal(&self.cin, &x.try_index((.., i..i + 1)).m()?, &mut k)?;
            y = self.medio[0].ap(&y, &mut k)?;
            y = self.at.ap(&y)?;
            y = self.medio[1].ap(&y, &mut k)?;
            for s in &self.sube {
                y = s.ap(&y, &mut k, i == 0)?;
            }
            let y = silu(&rms_canal(&y, &self.nout)?)?;
            let y = causal(&self.cout, &y, &mut k)?;
            y.eval().m()?;
            trozos.push(y);
            avance(i as u32 + 1, n as u32);
        }
        let y = ops::concatenate(&trozos, 1).m()?;
        // des-parcheo 2×2: canal = (c, i → ancho, j → alto)
        let (t, h, w) = (y.dim(1), y.dim(2), y.dim(3));
        let y = ops::reshape(&y, &[1, t, h, w, 3, 2, 2]).m()?;
        let y = ops::transpose_axes(&y, &[0, 1, 2, 6, 3, 5, 4]).m()?;
        let y = ops::reshape(&y, &[1, t, 2 * h, 2 * w, 3]).m()?;
        ops::clip(&y, (&escalar(-1.0), &escalar(1.0))).m()
    }
}

// ─────────────── V5b · el codificador del VAE (imagen→video: sólo el PRIMER cuadro) ───────────────
// `WanEncoder3d` + `quant_conv` sobre UN cuadro: es el primer trozo de `AutoencoderKLWan._encode`, con
// todas las cachés vacías (relleno causal de ceros). En ese trozo las bajadas 3D guardan su caché y NO
// aplican su `time_conv` (el cuadro sigue siendo uno), y el atajo `AvgDown3D` con factor temporal 2
// rellena UN cuadro de ceros delante y promedia con él.

/// Un bloque de bajada (`WanResidualDownBlock`): residuales, bajada espacial y atajo promediado.
struct BajaVae {
    res: Vec<ResVae>,
    /// `ZeroPad2d((0, 1, 0, 1))` + conv 3×3 de paso 2: [O, 3, 3, I] y sesgo
    baja: Option<(Array, Array)>,
    /// `AvgDown3D`: (canales de salida, factor temporal, factor espacial)
    atajo: (i32, i32, i32),
}

impl BajaVae {
    fn ap(&self, x: &Array, k: &mut Cachés) -> Result<Array> {
        let copia = x.clone();
        let mut y = x.clone();
        for r in &self.res {
            y = r.ap(&y, k)?;
        }
        if let Some((w, b)) = &self.baja {
            let (t, h, ww, c) = (y.dim(1), y.dim(2), y.dim(3), y.dim(4));
            let z = ops::pad(&ops::reshape(&y, &[t, h, ww, c]).m()?, &[(0, 0), (0, 1), (0, 1), (0, 0)][..], None, None).m()?;
            let z = ops::add(&ops::conv2d(&z, w, (2, 2), (0, 0), (1, 1), 1).m()?, b).m()?;
            y = ops::reshape(&z, &[1, t, z.dim(1), z.dim(2), z.dim(3)]).m()?;
        }
        ops::add(&y, &promedio_abajo(&copia, self.atajo)?).m()
    }
}

/// `AvgDown3D` en canales al final: relleno temporal de ceros DELANTE hasta múltiplo de `ft`, bloques
/// (ft, fs, fs) a canales en el orden (c, t, h, w) de diffusers y media por grupos consecutivos.
fn promedio_abajo(x: &Array, (salida, ft, fs): (i32, i32, i32)) -> Result<Array> {
    let (t, h, w, c) = (x.dim(1), x.dim(2), x.dim(3), x.dim(4));
    let relleno = (ft - t % ft) % ft;
    let x = if relleno > 0 { ops::concatenate(&[ops::zeros::<f32>(&[1, relleno, h, w, c]).m()?, x.clone()], 1).m()? } else { x.clone() };
    let tt = (t + relleno) / ft;
    let (hh, ww) = (h / fs, w / fs);
    let x = ops::reshape(&x, &[1, tt, ft, hh, fs, ww, fs, c]).m()?;
    let x = ops::transpose_axes(&x, &[0, 1, 3, 5, 7, 2, 4, 6]).m()?; // [1, T', H', W', C, ft, fs, fs]
    let grupo = c * ft * fs * fs / salida;
    let x = ops::reshape(&x, &[1, tt, hh, ww, salida, grupo]).m()?;
    ops::mean_axes(&x, &[-1], false).m()
}

/// El codificador del VAE de Wan 2.2 para el primer cuadro de imagen→video.
pub struct CodificadorWan {
    cin: Conv3,
    bajas: Vec<BajaVae>,
    medio: [ResVae; 2],
    at: AtVae,
    nout: Array,
    cout: Conv3,
    qc: Conv3,
    media: Array,
    inv_std: Array,
}

impl CodificadorWan {
    /// Lee `vae/` (sólo `encoder.` y `quant_conv.`; los `time_conv` de las bajadas 3D no actúan sobre
    /// un cuadro suelto y se descartan).
    pub fn cargar(dir: &std::path::Path) -> Result<CodificadorWan> {
        let cfg: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dir.join("config.json"))?)?;
        anyhow::ensure!(cfg["z_dim"] == 48 && cfg["is_residual"] == true && cfg["base_dim"] == 160 && cfg["patch_size"] == 2, gotv_engine::tr!("sólo el VAE de Wan 2.2 (z 48, residual, base 160, parche 2)", "only the Wan 2.2 VAE (z 48, residual, base 160, patch 2)"));
        let vec_f32 = |k: &str| -> Result<Vec<f32>> { cfg[k].as_array().ok_or_else(|| anyhow_tr!("VAE sin {k}", "VAE without {k}"))?.iter().map(|v| v.as_f64().map(|f| f as f32).ok_or_else(|| anyhow::anyhow!("{k}"))).collect() };
        let media = Array::from_slice(&vec_f32("latents_mean")?, &[48]);
        let inv_std = Array::from_slice(&vec_f32("latents_std")?.iter().map(|s| 1.0f32 / s).collect::<Vec<_>>(), &[48]);
        let mut p = Array::load_safetensors(dir.join("diffusion_pytorch_model.safetensors")).map_err(|e| anyhow::anyhow!("VAE: {e:?}"))?;
        p.retain(|k, _| (k.starts_with("encoder.") || k.starts_with("quant_conv.")) && !k.contains(".downsampler.time_conv."));
        let mut g = |k: &str| p.remove(k).ok_or_else(|| anyhow::anyhow!("VAE: falta {k}")).and_then(|a| a.as_dtype(F32).m());
        let lin1x1 = |a: Array| -> Result<Array> { ops::transpose_axes(&ops::reshape(&a, &[a.dim(0), a.dim(1)]).m()?, &[1, 0]).m() };
        let at = AtVae {
            n: ops::reshape(&g("encoder.mid_block.attentions.0.norm.gamma")?, &[-1]).m()?,
            qkv_w: lin1x1(g("encoder.mid_block.attentions.0.to_qkv.weight")?)?,
            qkv_b: g("encoder.mid_block.attentions.0.to_qkv.bias")?,
            pw: lin1x1(g("encoder.mid_block.attentions.0.proj.weight")?)?,
            pb: g("encoder.mid_block.attentions.0.proj.bias")?,
        };
        let nout = ops::reshape(&g("encoder.norm_out.gamma")?, &[-1]).m()?;
        // (salida, factor temporal, ¿baja?) de los 4 bloques: dims [160, 160, 320, 640, 640]
        let planes = [(160, 1, true), (320, 2, true), (640, 2, true), (640, 1, false)];
        let mut bajas = Vec::new();
        for (i, &(salida, ft, baja)) in planes.iter().enumerate() {
            let b = format!("encoder.down_blocks.{i}");
            let res = (0..2).map(|j| ResVae::de(&mut p, &format!("{b}.resnets.{j}"))).collect::<Result<Vec<_>>>()?;
            let conv = if baja {
                let w = p.remove(&format!("{b}.downsampler.resample.1.weight")).ok_or_else(|| anyhow_tr!("VAE: falta la conv de bajada {i}", "VAE: the downsampling conv {i} is missing"))?.as_dtype(F32).m()?;
                let bb = p.remove(&format!("{b}.downsampler.resample.1.bias")).unwrap().as_dtype(F32).m()?;
                Some((ops::transpose_axes(&w, &[0, 2, 3, 1]).m()?, bb))
            } else {
                None
            };
            bajas.push(BajaVae { res, baja: conv, atajo: (salida, ft, if baja { 2 } else { 1 }) });
        }
        let c = CodificadorWan {
            cin: Conv3::de(&mut p, "encoder.conv_in")?,
            bajas,
            medio: [ResVae::de(&mut p, "encoder.mid_block.resnets.0")?, ResVae::de(&mut p, "encoder.mid_block.resnets.1")?],
            at,
            nout,
            cout: Conv3::de(&mut p, "encoder.conv_out")?,
            qc: Conv3::de(&mut p, "quant_conv")?,
            media,
            inv_std,
        };
        if !p.is_empty() {
            let mut sobran: Vec<_> = p.keys().cloned().collect();
            sobran.sort();
            let s = &sobran[..sobran.len().min(5)]; bail_tr!("pesos del codificador sin usar: {s:?}", "unused encoder weights: {s:?}");
        }
        Ok(c)
    }

    /// Imagen [H, W, 3] en [−1, 1] (H y W múltiplos de 32) → latente NORMALIZADO del primer cuadro
    /// [1, 48, 1, H/16, W/16]: la media de la gaussiana (`argmax`) y `(z − μ)·(1/σ)` como el pipeline.
    pub fn codificar(&self, img: &Array) -> Result<Array> {
        let _tope = TopeCache::poner(CACHE_VIDEO);
        let (h, w) = (img.dim(0), img.dim(1));
        // parcheo 2×2: canal = (c, j → ancho, i → alto), el orden de `patchify`
        let x = ops::reshape(img, &[1, 1, h / 2, 2, w / 2, 2, 3]).m()?;
        let x = ops::transpose_axes(&x, &[0, 1, 2, 4, 6, 5, 3]).m()?;
        let x = ops::reshape(&x, &[1, 1, h / 2, w / 2, 12]).m()?;
        let mut k = Cachés { v: Vec::new(), i: 0 };
        let mut y = causal(&self.cin, &x, &mut k)?;
        for b in &self.bajas {
            y = b.ap(&y, &mut k)?;
            y.eval().m()?;
        }
        y = self.medio[0].ap(&y, &mut k)?;
        y = self.at.ap(&y)?;
        y = self.medio[1].ap(&y, &mut k)?;
        let y = silu(&rms_canal(&y, &self.nout)?)?;
        let y = causal(&self.cout, &y, &mut k)?;
        let y = self.qc.ap(&y, None)?;
        let mu = y.try_index((.., .., .., .., ..48)).m()?;
        let z = ops::multiply(&ops::subtract(&mu, &self.media).m()?, &self.inv_std).m()?;
        let z = ops::transpose_axes(&z, &[0, 4, 1, 2, 3]).m()?.contiguous().m()?; // [1, 48, 1, h, w]
        z.eval().m()?;
        Ok(z)
    }
}

// ───────────────────────────── V5 · el pipeline texto→video ─────────────────────────────

/// Lo que se pide. Por defecto lo del repo oficial (`wan_ti2v_5B.py`): 1280×704, 24 cps, 50 pasos,
/// guía 5 y el negativo oficial; los cuadros, 1 + 4k (121 = 5 s).
#[derive(Clone, Debug)]
pub struct Pedido {
    pub prompt: String,
    pub negativo: String,
    pub ancho: u32,
    pub alto: u32,
    pub cuadros: u32,
    pub pasos: u32,
    pub guia: f32,
    pub semilla: u64,
    /// imagen→video: el primer cuadro. El tamaño sale entonces de SU proporción con el área
    /// `ancho × alto` como máximo (`best_output_size` del repo oficial), no de `ancho` y `alto`.
    pub imagen: Option<std::path::PathBuf>,
    /// El transformador en f32 (los jueces de punta a punta) en vez de bf16 (producción). Medido
    /// 2026-11-13: con 4 pasos grandes y guía 5, diffusers en bf16 contra diffusers en f32 da OTRO
    /// video (media 21,7/255): el bf16 sólo se juzga por pasada, el pipeline se juzga en f32.
    pub f32_estricto: bool,
}

impl Default for Pedido {
    fn default() -> Self {
        Pedido { prompt: String::new(), negativo: NEGATIVO.into(), ancho: 1280, alto: 704, cuadros: 121, pasos: 50, guia: 5.0, semilla: 0, imagen: None, f32_estricto: false }
    }
}

/// Cuadros por segundo del 5B (`sample_fps`).
pub const CPS: u32 = 24;

impl Pedido {
    /// Las reglas del pipeline: `cuadros − 1` múltiplo de 4 (se redondea hacia abajo, mínimo 1) y lados
    /// múltiplos de 32 (VAE ×16 · parche 2). Devuelve (cuadros, ancho, alto) ya ajustados.
    pub fn ajustado(&self) -> (u32, u32, u32) {
        let c = (self.cuadros.max(1) - 1) / 4 * 4 + 1;
        (c, self.ancho / 32 * 32, self.alto / 32 * 32)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Etapa {
    Texto,
    /// imagen→video: codificando el primer cuadro
    Imagen,
    Paso(u32, u32),
    Video(u32, u32),
}

/// El video: cuadros RGB de 8 bits.
pub struct Video {
    pub cuadros: Vec<Vec<u8>>,
    pub ancho: u32,
    pub alto: u32,
    pub cps: u32,
}

impl Video {
    pub fn segundos(&self) -> f32 {
        self.cuadros.len() as f32 / self.cps as f32
    }
}

/// [1, T, H, W, 3] en [−1, 1] → cuadros RGB como `postprocess_video` + `export_to_video`:
/// `clamp(x/2 + ½, 0, 1)·255` truncado a u8.
pub fn a_cuadros(v: &Array) -> Result<Vec<Vec<u8>>> {
    let x = ops::clip(&ops::add(&ops::divide(v, escalar(2.0)).m()?, escalar(0.5)).m()?, (&escalar(0.0), &escalar(1.0))).m()?;
    let x = ops::multiply(&x, escalar(255.0)).m()?;
    x.eval().m()?;
    let (t, h, w) = (v.dim(1) as usize, v.dim(2) as usize, v.dim(3) as usize);
    let d = x.as_slice::<f32>();
    Ok((0..t).map(|i| d[i * h * w * 3..(i + 1) * h * w * 3].iter().map(|&p| p as u8).collect()).collect())
}

/// `best_output_size` del repo oficial (`wan/utils/utils.py`) con `dw = dh = 32` (parche 2 × VAE 16):
/// el tamaño de salida más cercano a la proporción de la imagen con área ≤ `area` y lados múltiplos de
/// 32, probando a fijar primero el ancho y luego el alto. Aritmética de Python en f64 (`//` = suelo).
pub fn tamano_para_imagen(w: u32, h: u32, area: u32) -> (u32, u32) {
    let (d, area) = (32.0f64, area as f64);
    let ratio = w as f64 / h as f64;
    let ow = (area * ratio).sqrt();
    let oh = area / ow;
    let ow1 = ((ow / d).floor() * d).max(d);
    let oh1 = (((area / ow1) / d).floor() * d).max(d);
    let oh2 = ((oh / d).floor() * d).max(d);
    let ow2 = (((area / oh2) / d).floor() * d).max(d);
    let (r1, r2) = (ow1 / oh1, ow2 / oh2);
    if (ratio / r1).max(r1 / ratio) < (ratio / r2).max(r2 / ratio) {
        (ow1 as u32, oh1 as u32)
    } else {
        (ow2 as u32, oh2 as u32)
    }
}

/// El primer cuadro como el repo oficial: escala para CUBRIR `ancho × alto` (lados con la ronda de
/// Python, al par) con Lanczos y recorta al centro; píxeles a [−1, 1] con `2·(p/255) − 1` (f32, como
/// `VaeImageProcessor`). Si ya mide eso no se toca (PIL también devuelve la copia). La Lanczos es la del
/// crate `image`, no la de PIL (aritmética entera): al escalar puede diferir en ±1 nivel.
pub fn primer_cuadro(ruta: &std::path::Path, ancho: u32, alto: u32) -> Result<Array> {
    let img = image::open(ruta).map_err(|e| anyhow_tr!("no pude abrir la imagen {}: {e}", "could not open the image {}: {e}", ruta.display()))?.to_rgb8();
    let (iw, ih) = img.dimensions();
    let img = if (iw, ih) == (ancho, alto) {
        img
    } else {
        let escala = (ancho as f64 / iw as f64).max(alto as f64 / ih as f64);
        let (rw, rh) = (((iw as f64 * escala).round_ties_even() as u32).max(ancho), ((ih as f64 * escala).round_ties_even() as u32).max(alto));
        let r = image::imageops::resize(&img, rw, rh, image::imageops::FilterType::Lanczos3);
        image::imageops::crop_imm(&r, (rw - ancho) / 2, (rh - alto) / 2, ancho, alto).to_image()
    };
    let v: Vec<f32> = img.into_raw().iter().map(|&p| 2.0 * (p as f32 / 255.0) - 1.0).collect();
    Ok(Array::from_slice(&v, &[alto as i32, ancho as i32, 3]))
}

/// Genera el video. `ruido`: el del juez (`ruido` [1, 48, F, h, w]) para comparar con la misma entrada.
pub fn generar(dir: &std::path::Path, p: &Pedido, progreso: &dyn Fn(Etapa), cancelar: &std::sync::atomic::AtomicBool, ruido: Option<&std::path::Path>) -> Result<Video> {
    use std::sync::atomic::Ordering;
    let _tope = TopeCache::poner(CACHE_VIDEO);
    let (cuadros, mut ancho, mut alto) = p.ajustado();
    if let Some(r) = &p.imagen {
        let (iw, ih) = image::image_dimensions(r).map_err(|e| anyhow_tr!("no pude leer la imagen {}: {e}", "could not read the image {}: {e}", r.display()))?;
        (ancho, alto) = tamano_para_imagen(iw, ih, p.ancho * p.alto);
    }
    if ancho < 32 || alto < 32 { let (w, h) = (p.ancho, p.alto); bail_tr!("tamaño demasiado pequeño ({w}×{h})", "size too small ({w}×{h})"); }
    anyhow::ensure!(p.pasos >= 1, "hacen falta pasos");
    let cfg = p.guia > 1.0;
    // 1 · el texto (el UMT5 se suelta antes de cargar el transformador)
    progreso(Etapa::Texto);
    let (pos, neg) = {
        let tk = tokenizers::Tokenizer::from_file(dir.join("tokenizer/tokenizer.json")).map_err(|e| anyhow::anyhow!("tokenizer T5: {e}"))?;
        let te = Umt5::cargar(&dir.join("text_encoder"))?;
        let pos = rellenar(&te.codificar(&tokens(&tk, &p.prompt)?)?)?;
        pos.eval().m()?;
        let neg = if cfg {
            let n = rellenar(&te.codificar(&tokens(&tk, &p.negativo)?)?)?;
            n.eval().m()?;
            Some(n)
        } else {
            None
        };
        (pos, neg)
    };
    let _ = mlx_rs::memory::clear_cache();
    if cancelar.load(Ordering::Relaxed) {
        anyhow::bail!("cancelado");
    }
    // 1b · imagen→video: el latente del primer cuadro (el codificador se suelta enseguida)
    let fijo = match &p.imagen {
        Some(r) => {
            progreso(Etapa::Imagen);
            let c = CodificadorWan::cargar(&dir.join("vae"))?.codificar(&primer_cuadro(r, ancho, alto)?)?;
            let _ = mlx_rs::memory::clear_cache();
            Some(c)
        }
        None => None,
    };
    // 2 · el ruido y la difusión
    let (f, h, w) = ((cuadros as i32 - 1) / 4 + 1, alto as i32 / 16, ancho as i32 / 16);
    let mut lat = match ruido {
        Some(r) => Array::load_safetensors(r).map_err(|e| anyhow_tr!("ruido: {e:?}", "noise: {e:?}"))?.remove("ruido").ok_or_else(|| anyhow_tr!("el archivo no trae «ruido»", "the file has no «ruido»"))?.as_dtype(F32).m()?,
        None => mlx_rs::random::normal::<f32>(&[1, 48, f, h, w], None, None, &mlx_rs::random::key(p.semilla).m()?).m()?,
    };
    if lat.shape() != [1, 48, f, h, w] { bail_tr!("el ruido no tiene la forma [1, 48, {f}, {h}, {w}]", "the noise is not shaped [1, 48, {f}, {h}, {w}]"); }
    // con imagen, el primer cuadro latente entra FIJO en cada paso (`(1 − máscara)·condición +
    // máscara·latentes`) y sus tokens llevan t = 0 (`expand_timesteps`); el planificador sigue con los
    // latentes sin mezclar, como diffusers, y al final se vuelve a poner el fijo
    let con_fijo = |lat: &Array| -> Result<Array> {
        match &fijo {
            None => Ok(lat.clone()),
            Some(c) if f == 1 => Ok(c.clone()),
            Some(c) => ops::concatenate(&[c.clone(), lat.try_index((.., .., 1..)).m()?], 2).m(),
        }
    };
    let primeros = if fijo.is_some() { (h / 2) * (w / 2) } else { 0 };
    {
        let tr = Transformador::cargar_en(&dir.join("transformer"), if p.f32_estricto { F32 } else { BF16 })?;
        let ctx_pos = tr.contexto(&pos)?;
        let ctx_neg = match &neg {
            Some(n) => Some(tr.contexto(n)?),
            None => None,
        };
        let giro = giro_3d(f, h / 2, w / 2);
        let mut u = UniPc::nuevo(p.pasos as usize);
        let pasos = u.pasos.clone();
        for (i, &t) in pasos.iter().enumerate() {
            let tiempo = Tiempo { t: t as f32, primeros };
            let entrada = con_fijo(&lat)?;
            let v = tr.adelante(&entrada, tiempo, &ctx_pos, &giro, Some(cancelar))?;
            let v = match &ctx_neg {
                Some(cn) => {
                    let vu = tr.adelante(&entrada, tiempo, cn, &giro, Some(cancelar))?;
                    ops::add(&vu, &ops::multiply(escalar(p.guia), &ops::subtract(&v, &vu).m()?).m()?).m()?
                }
                None => v,
            };
            lat = u.paso(&v, &lat)?;
            lat.eval().m()?;
            progreso(Etapa::Paso(i as u32 + 1, pasos.len() as u32));
        }
    }
    let lat = con_fijo(&lat)?;
    let _ = mlx_rs::memory::clear_cache();
    // 3 · el VAE
    progreso(Etapa::Video(0, f as u32));
    let vae = VaeWan::cargar(&dir.join("vae"))?;
    let v = vae.decodificar_con(&lat, Some(cancelar), &|i, n| progreso(Etapa::Video(i, n)))?;
    let c = a_cuadros(&v)?;
    let _ = mlx_rs::memory::clear_cache();
    Ok(Video { cuadros: c, ancho, alto, cps: CPS })
}

/// El motor que registran la CLI y la app (firma sin tipos de MLX): escribe el MP4 y devuelve los
/// segundos de video y el tamaño (con imagen sale de su proporción).
#[allow(clippy::too_many_arguments)]
pub fn generar_mp4(
    dir: &std::path::Path,
    prompt: &str,
    negativo: Option<&str>,
    segundos: f32,
    ancho: u32,
    alto: u32,
    pasos: u32,
    guia: f32,
    semilla: u64,
    imagen: Option<&std::path::Path>,
    continuar: Option<&std::path::Path>,
    salida: &std::path::Path,
    progreso: &(dyn Fn(String) + Sync),
    cancelar: &std::sync::atomic::AtomicBool,
) -> Result<(f32, u32, u32)> {
    // Ola PRO · EXTENDER: el último cuadro del clip anterior es el primero del nuevo (imagen→video
    // con el área del anterior) y el MP4 que sale trae los dos tramos unidos
    let mut temporal: Option<std::path::PathBuf> = None;
    let (mut ancho, mut alto, mut imagen) = (ancho, alto, imagen.map(|r| r.to_path_buf()));
    if let Some(previo) = continuar {
        anyhow::ensure!(imagen.is_none(), gotv_engine::tr!("o se anima una imagen o se continúa un video, no las dos", "either animate an image or continue a video, not both"));
        progreso(gotv_engine::tr!("leyendo el clip anterior", "reading the previous clip"));
        let (w, h, rgb, _) = crate::video_mp4::ultimo_cuadro(previo)?;
        let t = std::env::temp_dir().join(format!("gotv-continuar-{}-{semilla}.png", std::process::id()));
        image::RgbImage::from_raw(w as u32, h as u32, rgb).ok_or_else(|| anyhow::anyhow!("cuadro {w}×{h}"))?.save(&t)?;
        (ancho, alto, imagen) = (w as u32, h as u32, Some(t.clone()));
        temporal = Some(t);
    }
    let r = generar_y_escribir(dir, prompt, negativo, segundos, ancho, alto, pasos, guia, semilla, imagen.as_deref(), continuar, salida, progreso, cancelar);
    if let Some(t) = temporal {
        let _ = std::fs::remove_file(t);
    }
    r
}

#[allow(clippy::too_many_arguments)]
fn generar_y_escribir(
    dir: &std::path::Path,
    prompt: &str,
    negativo: Option<&str>,
    segundos: f32,
    ancho: u32,
    alto: u32,
    pasos: u32,
    guia: f32,
    semilla: u64,
    imagen: Option<&std::path::Path>,
    continuar: Option<&std::path::Path>,
    salida: &std::path::Path,
    progreso: &(dyn Fn(String) + Sync),
    cancelar: &std::sync::atomic::AtomicBool,
) -> Result<(f32, u32, u32)> {
    let cuadros = (segundos.max(0.05) * CPS as f32).round() as u32 + 1;
    anyhow::ensure!((1..=100).contains(&pasos), gotv_engine::tr!("pasos fuera de rango (1–100): {pasos}", "steps out of range (1–100): {pasos}"));
    anyhow::ensure!((1.0..=20.0).contains(&guia), gotv_engine::tr!("guía fuera de rango (1–20): {guia}", "guidance out of range (1–20): {guia}"));
    let mut p = Pedido { prompt: prompt.to_string(), ancho, alto, cuadros, pasos, guia, semilla, imagen: imagen.map(|r| r.to_path_buf()), ..Default::default() };
    if let Some(n) = negativo {
        p.negativo = n.to_string();
    }
    let v = generar(
        dir,
        &p,
        &|e| {
            progreso(match e {
                Etapa::Texto => "texto".into(),
                Etapa::Imagen => gotv_engine::tr!("primer cuadro", "first frame"),
                Etapa::Paso(i, n) => gotv_engine::tr!("paso {i}/{n}", "step {i}/{n}"),
                Etapa::Video(i, n) => gotv_engine::tr!("video {i}/{n}", "video {i}/{n}"),
            })
        },
        cancelar,
        None,
    )?;
    if let Some(previo) = continuar {
        progreso(gotv_engine::tr!("uniendo con el clip anterior", "joining with the previous clip"));
        return unir_mp4(previo, &v, salida);
    }
    let mut m = crate::video_mp4::Mp4::nuevo(salida, v.ancho as usize, v.alto as usize, v.cps as i32)?;
    for c in &v.cuadros {
        m.cuadro(c)?;
    }
    m.cerrar()?;
    Ok((v.segundos(), v.ancho, v.alto))
}

/// Ola PRO · el clip anterior entero y DETRÁS el nuevo sin su primer cuadro (que es el último del
/// anterior). Si el nuevo salió de otro tamaño se lleva al del anterior. Se escribe aparte y se
/// renombra al final: `salida` puede ser el mismo archivo que `previo`. → (segundos, ancho, alto).
pub fn unir_mp4(previo: &std::path::Path, nuevo: &Video, salida: &std::path::Path) -> Result<(f32, u32, u32)> {
    let mut l = crate::video_mp4::Lector::abrir(previo)?;
    let primero = l.siguiente()?.ok_or_else(|| anyhow_tr!("{} no tiene cuadros", "{} has no frames", previo.display()))?;
    let (w, h) = (primero.0, primero.1);
    let tmp = salida.with_extension("uniendo.mp4");
    let mut m = crate::video_mp4::Mp4::nuevo(&tmp, w, h, nuevo.cps as i32)?;
    let mut n = 0usize;
    let mut cuadro = Some(primero);
    while let Some((_, _, rgb)) = cuadro {
        m.cuadro(&rgb)?;
        n += 1;
        cuadro = l.siguiente()?;
    }
    for c in nuevo.cuadros.iter().skip(1) {
        if (nuevo.ancho as usize, nuevo.alto as usize) == (w, h) {
            m.cuadro(c)?;
        } else {
            let img = image::RgbImage::from_raw(nuevo.ancho, nuevo.alto, c.clone()).ok_or_else(|| anyhow::anyhow!("cuadro"))?;
            m.cuadro(&image::imageops::resize(&img, w as u32, h as u32, image::imageops::FilterType::Lanczos3).into_raw())?;
        }
        n += 1;
    }
    m.cerrar()?;
    std::fs::rename(&tmp, salida)?;
    Ok((n as f32 / nuevo.cps as f32, w as u32, h as u32))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Ola PRO · extender: el anterior entero + el nuevo sin su primer cuadro, en el MISMO archivo
    /// (se escribe aparte y se renombra) y con el nuevo llevado al tamaño del anterior. Sin modelos.
    #[test]
    fn unir_mp4_encadena_sin_repetir_el_cuadro_de_union() {
        let dir = std::env::temp_dir().join(format!("gotv-unir-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let ruta = dir.join("clip.mp4");
        let (w, h) = (64usize, 32usize);
        let plano = |v: u8, w: usize, h: usize| vec![v; w * h * 3];
        let mut m = crate::video_mp4::Mp4::nuevo(&ruta, w, h, 24).unwrap();
        for f in 0..5u8 {
            m.cuadro(&plano(f * 40, w, h)).unwrap();
        }
        m.cerrar().unwrap();
        // el nuevo sale al doble de tamaño: su cuadro 0 (= el último del anterior) no se repite
        let nuevo = Video { cuadros: (0..4u8).map(|f| plano(160 + f * 20, 2 * w, 2 * h)).collect(), ancho: 2 * w as u32, alto: 2 * h as u32, cps: 24 };
        let (s, uw, uh) = unir_mp4(&ruta, &nuevo, &ruta).unwrap();
        assert_eq!((uw, uh), (w as u32, h as u32));
        assert!((s - 8.0 / 24.0).abs() < 1e-6, "5 + 3 cuadros: {s}");
        let mut l = crate::video_mp4::Lector::abrir(&ruta).unwrap();
        let mut medios = Vec::new();
        while let Some((_, _, rgb)) = l.siguiente().unwrap() {
            medios.push(rgb[(h / 2 * w + w / 2) * 3]);
        }
        assert_eq!(medios.len(), 8);
        for (i, esperado) in [0u8, 40, 80, 120, 160, 180, 200, 220].iter().enumerate() {
            assert!((medios[i] as i32 - *esperado as i32).abs() <= 12, "cuadro {i}: {} vs {esperado}", medios[i]);
        }
        assert!(!ruta.with_extension("uniendo.mp4").exists(), "sin restos");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn ajusta_cuadros_y_lados_como_el_pipeline() {
        let p = Pedido { cuadros: 50, ancho: 1290, alto: 710, ..Default::default() };
        assert_eq!(p.ajustado(), (49, 1280, 704));
        assert_eq!(Pedido { cuadros: 0, ..Default::default() }.ajustado().0, 1);
        assert_eq!(Pedido::default().ajustado(), (121, 1280, 704));
    }

    #[test]
    fn cuadros_como_export_to_video() {
        let v = Array::from_slice(&[-1.0f32, 0.0, 1.0, 0.999, -0.999, 2.0], &[1, 1, 1, 2, 3]);
        assert_eq!(a_cuadros(&v).unwrap(), vec![vec![0u8, 127, 255, 254, 0, 255]]);
    }

    /// La RoPE 3D: el token (f, h, w) lleva sus tres ángulos en su tramo de la cabeza.
    #[test]
    fn giro_3d_por_ejes() {
        let (c, s) = giro_3d(2, 3, 4);
        assert_eq!(c.shape(), &[1, 24, 1, 64]);
        let (c, s) = (c.as_slice::<f32>(), s.as_slice::<f32>());
        // token (1, 2, 3) = 1·12 + 2·4 + 3 = 23; primer par de cada eje: frecuencia 1
        let base = 23 * 64;
        assert!((s[base] - 1f64.sin() as f32).abs() < 1e-7, "t");
        assert!((s[base + 22] - 2f64.sin() as f32).abs() < 1e-7, "h");
        assert!((s[base + 43] - 3f64.sin() as f32).abs() < 1e-7, "w");
        assert_eq!(c[0], 1.0);
    }

    fn diferencia(mio: &[f32], juez: &[f32]) -> (f32, f32) {
        assert_eq!(mio.len(), juez.len());
        mio.iter().zip(juez).fold((0.0f32, 0.0f32), |(p, e), (a, b)| (p.max((a - b).abs()), e.max(b.abs())))
    }

    /// Juez del transformador (`juez_dit.py`: diffusers en f32): una pasada en texto→video y otra en
    /// imagen→video. CARGA EL MODELO: `GOTV_WAN_DIR=<repo> GOTV_JUEZ_DIT=<ref.safetensors> cargo test
    /// -p gotv-mlx --release -- --ignored --nocapture transformador_como_diffusers`
    #[test]
    #[ignore]
    fn transformador_como_diffusers() {
        let (Ok(dir), Ok(r)) = (std::env::var("GOTV_WAN_DIR"), std::env::var("GOTV_JUEZ_DIT")) else { return };
        let d = Array::load_safetensors(r).unwrap();
        let tr = Transformador::cargar_en(&std::path::Path::new(&dir).join("transformer"), F32).unwrap();
        let lat = &d["lat"];
        let (f, h, w) = (lat.dim(2), lat.dim(3), lat.dim(4));
        let ctx = tr.contexto(&d["ctx"]).unwrap();
        let giro = giro_3d(f, h / 2, w / 2);
        for (nombre, primeros) in [("t2v", 0), ("i2v", h / 2 * (w / 2))] {
            let v = tr.adelante(lat, Tiempo { t: 961.0, primeros }, &ctx, &giro, None).unwrap();
            v.eval().unwrap();
            let (peor, escala) = diferencia(v.as_slice::<f32>(), d[&format!("v_{nombre}")].as_slice::<f32>());
            println!("{nombre}: diferencia máx {peor:.3e} (escala {escala:.2})");
            assert!(peor <= 1e-3 * escala.max(1.0), "{nombre}: {peor}");
        }
        // bf16 (producción) contra el f32 de diffusers: no puede perder más que la referencia en bf16
        drop((tr, ctx));
        let tr = Transformador::cargar(&std::path::Path::new(&dir).join("transformer")).unwrap();
        let ctx = tr.contexto(&d["ctx"]).unwrap();
        let v = tr.adelante(lat, Tiempo { t: 961.0, primeros: 0 }, &ctx, &giro, None).unwrap();
        v.eval().unwrap();
        let (mio, _) = diferencia(v.as_slice::<f32>(), d["v_t2v"].as_slice::<f32>());
        let (suelo, _) = diferencia(d["v_t2v_bf16"].as_slice::<f32>(), d["v_t2v"].as_slice::<f32>());
        println!("t2v en bf16: diferencia máx {mio:.3e} · la referencia en bf16 pierde {suelo:.3e}");
        assert!(mio <= 1.5 * suelo, "bf16: {mio} > 1,5 × {suelo}");
    }

    /// Juez del VAE (`juez_vae.py`: diffusers en f32, 3 cuadros latentes → 9 de video). CARGA EL
    /// MODELO: `GOTV_WAN_DIR=<repo> GOTV_JUEZ_VAE=<ref.safetensors> cargo test -p gotv-mlx --release
    /// -- --ignored --nocapture vae_como_diffusers`
    #[test]
    #[ignore]
    fn vae_como_diffusers() {
        let (Ok(dir), Ok(r)) = (std::env::var("GOTV_WAN_DIR"), std::env::var("GOTV_JUEZ_VAE")) else { return };
        let d = Array::load_safetensors(r).unwrap();
        let vae = VaeWan::cargar(&std::path::Path::new(&dir).join("vae")).unwrap();
        let v = vae.decodificar(&d["z"], None).unwrap();
        v.eval().unwrap();
        assert_eq!(v.shape(), d["video"].shape(), "forma del video");
        let (peor, escala) = diferencia(v.as_slice::<f32>(), d["video"].as_slice::<f32>());
        println!("VAE: {:?} · diferencia máx {peor:.3e} (escala {escala:.2})", v.shape());
        assert!(peor <= 2e-3, "VAE: {peor}");
    }

    /// Juez de punta a punta (`juez_e2e.py`: WanPipeline de diffusers en f32, 256×160, 9 cuadros, 4
    /// pasos, guía 5, negativo oficial, MISMO ruido): los cuadros en u8 como `export_to_video`. CARGA
    /// LOS TRES MODELOS: `GOTV_WAN_DIR=<repo> GOTV_JUEZ_E2E=<ref.safetensors> cargo test -p gotv-mlx
    /// --release -- --ignored --nocapture punta_a_punta_como_diffusers`
    #[test]
    #[ignore]
    fn punta_a_punta_como_diffusers() {
        let (Ok(dir), Ok(r)) = (std::env::var("GOTV_WAN_DIR"), std::env::var("GOTV_JUEZ_E2E")) else { return };
        let d = Array::load_safetensors(&r).unwrap();
        let p = Pedido {
            prompt: "Un gato naranja duerme sobre una ventana mientras cae la lluvia; luz cálida de atardecer, cámara lenta.".into(),
            ancho: 256,
            alto: 160,
            cuadros: 9,
            pasos: 4,
            f32_estricto: true,
            ..Default::default()
        };
        let no = std::sync::atomic::AtomicBool::new(false);
        let v = generar(std::path::Path::new(&dir), &p, &|_| {}, &no, Some(std::path::Path::new(&r))).unwrap();
        let juez = d["video"].as_slice::<u8>();
        let mio: Vec<u8> = v.cuadros.concat();
        assert_eq!(mio.len(), juez.len(), "tamaño del video");
        let difs: Vec<u8> = mio.iter().zip(juez).map(|(a, b)| a.abs_diff(*b)).collect();
        let peor = *difs.iter().max().unwrap();
        let media = difs.iter().map(|&x| x as f64).sum::<f64>() / difs.len() as f64;
        let distintos = difs.iter().filter(|&&x| x > 0).count() as f64 / difs.len() as f64;
        println!("punta a punta: {} cuadros {}×{} · diferencia máx {peor} · media {media:.4} · {:.2} % de valores distintos", v.cuadros.len(), v.ancho, v.alto, distintos * 100.0);
        // El suelo es el de la REFERENCIA contra sí misma (medido 2026-11-13, este mismo juez): diffusers
        // en MPS frente a diffusers en CPU da máx 11 · media 0,179 (latente final 5,4e-2; con 4 pasos y
        // guía 5 el ruido de f32 crece ~×10 por paso). Rust contra CPU dio máx 11 · media 0,174 (latente
        // 9e-3, más cerca que MPS). Por tramos (`punta_a_punta_por_tramos`) cada pieza da ≤ 5e-4.
        assert!(peor <= 16 && media < 0.25, "punta a punta: máx {peor}, media {media}");
    }

    /// Juez de punta a punta de IMAGEN→VIDEO (`juez_i2v.py`: `WanImageToVideoPipeline` de diffusers en
    /// f32, PNG de 256×160, mismo ruido, 4 pasos, guía 5), en f32 estricto como el de texto→video y con
    /// el mismo suelo medido. CARGA LOS TRES MODELOS: `GOTV_WAN_DIR=<repo>
    /// GOTV_JUEZ_I2V=<dir del juez> cargo test -p gotv-mlx --release -- --ignored --nocapture
    /// imagen_a_video_como_diffusers`
    #[test]
    #[ignore]
    fn imagen_a_video_como_diffusers() {
        let (Ok(dir), Ok(j)) = (std::env::var("GOTV_WAN_DIR"), std::env::var("GOTV_JUEZ_I2V")) else { return };
        let j = std::path::Path::new(&j);
        let d = Array::load_safetensors(j.join("i2v.safetensors")).unwrap();
        let p = Pedido {
            prompt: "The orange cat slowly opens its eyes and stretches on the windowsill.".into(),
            ancho: 256,
            alto: 160,
            cuadros: 9,
            pasos: 4,
            f32_estricto: true,
            imagen: Some(j.join("primer.png")),
            ..Default::default()
        };
        let no = std::sync::atomic::AtomicBool::new(false);
        let v = generar(std::path::Path::new(&dir), &p, &|_| {}, &no, Some(&j.join("i2v.safetensors"))).unwrap();
        assert_eq!((v.ancho, v.alto), (256, 160));
        let juez = d["video"].as_slice::<u8>();
        let mio: Vec<u8> = v.cuadros.concat();
        assert_eq!(mio.len(), juez.len(), "tamaño del video");
        let difs: Vec<u8> = mio.iter().zip(juez).map(|(a, b)| a.abs_diff(*b)).collect();
        let peor = *difs.iter().max().unwrap();
        let media = difs.iter().map(|&x| x as f64).sum::<f64>() / difs.len() as f64;
        // el primer cuadro es la imagen (codificada y decodificada): debe parecerse a ella
        let primer: Vec<u8> = image::open(j.join("primer.png")).unwrap().to_rgb8().into_raw();
        let fiel = v.cuadros[0].iter().zip(&primer).map(|(a, b)| a.abs_diff(*b) as f64).sum::<f64>() / primer.len() as f64;
        println!("imagen→video: {} cuadros {}×{} · diferencia máx {peor} · media {media:.4} · primer cuadro vs la imagen: media {fiel:.2}", v.cuadros.len(), v.ancho, v.alto);
        assert!(peor <= 16 && media < 0.25, "imagen→video: máx {peor}, media {media}");
    }

    /// El mismo juez, por tramos: texto contra los embeddings del juez, cada paso partiendo del
    /// latente DEL JUEZ (error de un paso, sin arrastre) y del propio (error acumulado), y el VAE sobre
    /// los latentes finales del juez. Dice DÓNDE nace una diferencia de la prueba de punta a punta.
    #[test]
    #[ignore]
    fn punta_a_punta_por_tramos() {
        let (Ok(dir), Ok(r)) = (std::env::var("GOTV_WAN_DIR"), std::env::var("GOTV_JUEZ_E2E")) else { return };
        let dir = std::path::Path::new(&dir);
        let d = Array::load_safetensors(&r).unwrap();
        let prompt = "Un gato naranja duerme sobre una ventana mientras cae la lluvia; luz cálida de atardecer, cámara lenta.";
        let (pe, ne) = {
            let tk = tokenizers::Tokenizer::from_file(dir.join("tokenizer/tokenizer.json")).unwrap();
            let te = Umt5::cargar(&dir.join("text_encoder")).unwrap();
            let pe = rellenar(&te.codificar(&tokens(&tk, prompt).unwrap()).unwrap()).unwrap();
            let ne = rellenar(&te.codificar(&tokens(&tk, NEGATIVO).unwrap()).unwrap()).unwrap();
            (pe, ne)
        };
        for (n, mio, juez) in [("texto +", &pe, "pe"), ("texto −", &ne, "ne")] {
            mio.eval().unwrap();
            let (peor, escala) = diferencia(mio.as_slice::<f32>(), d[juez].as_slice::<f32>());
            println!("{n}: diferencia máx {peor:.3e} (escala {escala:.2})");
        }
        let _ = mlx_rs::memory::clear_cache();
        let tr = Transformador::cargar_en(&dir.join("transformer"), F32).unwrap();
        let dj = |k: &str| ops::expand_dims(&d[k], 0).unwrap();
        let (cp, cn) = (tr.contexto(&dj("pe")).unwrap(), tr.contexto(&dj("ne")).unwrap());
        let ruido = &d["ruido"];
        let (f, h, w) = (ruido.dim(2), ruido.dim(3), ruido.dim(4));
        let giro = giro_3d(f, h / 2, w / 2);
        let paso = |u: &mut UniPc, lat: &Array, t: i64| -> Array {
            let tiempo = Tiempo { t: t as f32, primeros: 0 };
            let v = tr.adelante(lat, tiempo, &cp, &giro, None).unwrap();
            let vu = tr.adelante(lat, tiempo, &cn, &giro, None).unwrap();
            let v = ops::add(&vu, ops::multiply(escalar(5.0), ops::subtract(&v, &vu).unwrap()).unwrap()).unwrap();
            let x = u.paso(&v, lat).unwrap();
            x.eval().unwrap();
            x
        };
        // acumulado: mi cadena entera; de un paso: mi UniPC alimentado con los latentes del juez
        let (mut ua, mut ub) = (UniPc::nuevo(4), UniPc::nuevo(4));
        let pasos = ua.pasos.clone();
        let (mut mio, mut previo) = (ruido.clone(), ruido.clone());
        for (i, &t) in pasos.iter().enumerate() {
            mio = paso(&mut ua, &mio, t);
            let uno = paso(&mut ub, &previo, t);
            let juez = &d[&format!("lat_{i}")];
            let (pa, e) = diferencia(mio.as_slice::<f32>(), juez.as_slice::<f32>());
            let (pu, _) = diferencia(uno.as_slice::<f32>(), juez.as_slice::<f32>());
            println!("paso {} (t={t}): acumulada {pa:.3e} · de un paso {pu:.3e} (escala {e:.2})", i + 1);
            previo = juez.clone();
        }
        drop(tr);
        let _ = mlx_rs::memory::clear_cache();
        let vae = VaeWan::cargar(&dir.join("vae")).unwrap();
        let c: Vec<u8> = a_cuadros(&vae.decodificar(&d["lat_3"], None).unwrap()).unwrap().concat();
        let difs: Vec<u8> = c.iter().zip(d["video"].as_slice::<u8>()).map(|(a, b)| a.abs_diff(*b)).collect();
        println!("VAE sobre los latentes del juez: máx {} · media {:.4}", difs.iter().max().unwrap(), difs.iter().map(|&x| x as f64).sum::<f64>() / difs.len() as f64);
    }

    /// Banco a 720p (1280×704, 2 s = 13 cuadros latentes, 11 440 tokens): tiempo y pico de memoria de
    /// UNA pasada del transformador (`GOTV_BANCO_DT=f32` para comparar) y del VAE entero. CARGA LOS
    /// MODELOS: `GOTV_WAN_DIR=<repo> cargo test -p gotv-mlx --release -- --ignored --nocapture rinde_a_720p`
    #[test]
    #[ignore]
    fn rinde_a_720p() {
        let Ok(dir) = std::env::var("GOTV_WAN_DIR") else { return };
        let dir = std::path::Path::new(&dir);
        let gb = |b: usize| b as f64 / 1e9;
        let _tope = TopeCache::poner(CACHE_VIDEO); // como `generar`
        let dt = if std::env::var("GOTV_BANCO_DT").as_deref() == Ok("f32") { F32 } else { BF16 };
        let (f, h, w) = (13, 44, 80);
        let lat = mlx_rs::random::normal::<f32>(&[1, 48, f, h, w], None, None, &mlx_rs::random::key(1).unwrap()).unwrap();
        let t0 = std::time::Instant::now();
        let tr = Transformador::cargar_en(&dir.join("transformer"), dt).unwrap();
        println!("transformador {dt:?}: cargado en {:.1} s · activa {:.1} GB", t0.elapsed().as_secs_f32(), gb(mlx_rs::memory::active_memory().unwrap()));
        let texto = ops::multiply(mlx_rs::random::normal::<f32>(&[1, 512, 4096], None, None, &mlx_rs::random::key(2).unwrap()).unwrap(), escalar(0.2)).unwrap();
        let ctx = tr.contexto(&texto).unwrap();
        let giro = giro_3d(f, h / 2, w / 2);
        mlx_rs::memory::reset_peak_memory().unwrap();
        for i in 0..2 {
            let t0 = std::time::Instant::now();
            let v = tr.adelante(&lat, Tiempo { t: 900.0, primeros: 0 }, &ctx, &giro, None).unwrap();
            v.eval().unwrap();
            println!("  pasada {}: {:.1} s · pico {:.1} GB", i + 1, t0.elapsed().as_secs_f32(), gb(mlx_rs::memory::peak_memory().unwrap()));
        }
        drop((tr, ctx));
        let _ = mlx_rs::memory::clear_cache();
        mlx_rs::memory::reset_peak_memory().unwrap();
        let vae = VaeWan::cargar(&dir.join("vae")).unwrap();
        let t0 = std::time::Instant::now();
        let v = vae.decodificar(&lat, None).unwrap();
        v.eval().unwrap();
        println!("VAE: {:?} en {:.1} s · pico {:.1} GB", v.shape(), t0.elapsed().as_secs_f32(), gb(mlx_rs::memory::peak_memory().unwrap()));
    }
    /// La conv por tomas 2D (entera, por grupos de cuadros y por bandas de filas con su borde) da lo
    /// mismo que el `conv3d` de MLX con el relleno causal: con caché y sin ella, núcleo 3×3×3 y 1×3×3,
    /// bandas de 1 y de 3 filas (la última corta) y grupos de 2 cuadros.
    #[test]
    fn conv_por_tiras_como_entera() {
        let k = |s: u64| mlx_rs::random::key(s).unwrap();
        let x = mlx_rs::random::normal::<f32>(&[1, 4, 7, 5, 6], None, None, &k(1)).unwrap();
        let cache = mlx_rs::random::normal::<f32>(&[1, 2, 7, 5, 6], None, None, &k(2)).unwrap();
        for kt in [3, 1] {
            let w = mlx_rs::random::normal::<f32>(&[4, kt, 3, 3, 6], None, None, &k(3)).unwrap();
            let b = mlx_rs::random::normal::<f32>(&[4], None, None, &k(4)).unwrap();
            let c = Conv3::nueva(&w, b.clone()).unwrap();
            let por_fila = 5 * 9 * 6 * 4;
            for ca in [None, Some(&cache)] {
                // la referencia: conv3d sobre [relleno causal, x]
                let delante = match (kt, ca) {
                    (1, _) => x.clone(),
                    (_, Some(cc)) => ops::concatenate(&[cc.clone(), x.clone()], 1).unwrap(),
                    (_, None) => ops::concatenate(&[ops::zeros::<f32>(&[1, kt - 1, 7, 5, 6]).unwrap(), x.clone()], 1).unwrap(),
                };
                let juez = ops::add(ops::conv3d(&delante, &w, (1, 1, 1), (0, 1, 1), (1, 1, 1), 1).unwrap(), &b).unwrap();
                for presupuesto in [i64::MAX, por_fila, 3 * por_fila, 2 * 7 * por_fila] {
                    let y = c.ap_con(&x, ca, presupuesto).unwrap();
                    assert_eq!(y.shape(), juez.shape());
                    let (peor, _) = diferencia(y.as_slice::<f32>(), juez.as_slice::<f32>());
                    assert!(peor < 1e-4, "kt {kt}, presupuesto {presupuesto}, caché {}: {peor}", ca.is_some());
                }
            }
        }
    }

    /// Juez del codificador (`juez_vae_cod.py`: `AutoencoderKLWan.encode` de diffusers en f32, una
    /// imagen, `argmax` y la normalización del pipeline i2v). CARGA EL VAE: `GOTV_WAN_DIR=<repo>
    /// GOTV_JUEZ_VAE_COD=<ref.safetensors> cargo test -p gotv-mlx --release -- --ignored --nocapture
    /// codificador_como_diffusers`
    #[test]
    #[ignore]
    fn codificador_como_diffusers() {
        let (Ok(dir), Ok(r)) = (std::env::var("GOTV_WAN_DIR"), std::env::var("GOTV_JUEZ_VAE_COD")) else { return };
        let d = Array::load_safetensors(r).unwrap();
        let c = CodificadorWan::cargar(&std::path::Path::new(&dir).join("vae")).unwrap();
        for n in ["a", "b"] {
            let z = c.codificar(&d[&format!("img_{n}")]).unwrap();
            assert_eq!(z.shape(), d[&format!("z_{n}")].shape(), "{n}: forma");
            let (peor, escala) = diferencia(z.as_slice::<f32>(), d[&format!("z_{n}")].as_slice::<f32>());
            println!("codificador {n}: {:?} · diferencia máx {peor:.3e} (escala {escala:.2})", z.shape());
            assert!(peor <= 1e-4 * escala.max(1.0), "{n}: {peor}");
        }
    }

    /// El tamaño de imagen→video, contra `best_output_size` del repo oficial (valores sacados de la
    /// función original, 2026-11-13; 1024×1024 → 960×928 es lo que da el oficial).
    #[test]
    fn tamano_como_el_oficial() {
        let a = 704 * 1280;
        for ((w, h, area), esperado) in [
            ((1920, 1080, a), (1248, 704)),
            ((1080, 1920, a), (704, 1248)),
            ((1024, 1024, a), (960, 928)),
            ((800, 600, a), (1088, 800)),
            ((4000, 3000, a), (1088, 800)),
            ((1280, 704, a), (1280, 704)),
            ((704, 1280, a), (704, 1280)),
            ((3000, 1000, a), (1632, 544)),
            ((256, 160, 256 * 160), (256, 160)),
        ] {
            assert_eq!(tamano_para_imagen(w, h, area), esperado, "{w}×{h}");
        }
    }

    /// El primer cuadro: a la medida no se toca (2·(p/255) − 1 exacto); si no, cubre y recorta al centro.
    #[test]
    fn primer_cuadro_cubre_y_recorta() {
        let dir = std::env::temp_dir().join(format!("gotv-primer-cuadro-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let ruta = dir.join("a.png");
        let mut img = image::RgbImage::new(64, 32);
        for (x, _, p) in img.enumerate_pixels_mut() {
            *p = image::Rgb([if x < 32 { 0 } else { 255 }, 128, 51]);
        }
        img.save(&ruta).unwrap();
        let a = primer_cuadro(&ruta, 64, 32).unwrap();
        assert_eq!(a.shape(), [32, 64, 3]);
        let v = a.as_slice::<f32>();
        assert_eq!((v[0], v[1], v[2]), (-1.0, 2.0 * (128.0f32 / 255.0) - 1.0, 2.0 * (51.0f32 / 255.0) - 1.0));
        assert_eq!(v[3 * 63], 1.0);
        // 32×32: escala 1 (cubre el alto) y recorta el centro del ancho → mitad negra, mitad blanca
        let b = primer_cuadro(&ruta, 32, 32).unwrap();
        assert_eq!(b.shape(), [32, 32, 3]);
        let v = b.as_slice::<f32>();
        assert!(v[0] < -0.9 && v[3 * 31] > 0.9, "{} {}", v[0], v[3 * 31]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Los cubos de posición relativa del T5 (`_relative_position_bucket`, bidireccional, 32/128),
    /// contra los de transformers.
    #[test]
    fn cubos_como_transformers() {
        let r: Vec<i32> = [-200, -128, -127, -64, -20, -9, -8, -7, -1, 0, 1, 7, 8, 9, 20, 64, 127, 128, 511].iter().map(|&x| cubo(x)).collect();
        assert_eq!(r, vec![15, 15, 15, 14, 10, 8, 8, 7, 1, 0, 17, 23, 24, 24, 26, 30, 31, 31, 31]); // transformers 5.22
    }

    /// Juez del UMT5 (`juez_umt5.py`: transformers en f32): mismos ids y estados ocultos.
    /// CARGA EL MODELO: `GOTV_WAN_DIR=<repo> GOTV_JUEZ_UMT5=<ref.safetensors> cargo test -p gotv-mlx
    /// --release -- --ignored --nocapture umt5_como_transformers`
    #[test]
    #[ignore]
    fn umt5_como_transformers() {
        let (Ok(dir), Ok(r)) = (std::env::var("GOTV_WAN_DIR"), std::env::var("GOTV_JUEZ_UMT5")) else { return };
        let dir = std::path::Path::new(&dir);
        let d = Array::load_safetensors(r).unwrap();
        let tk = tokenizers::Tokenizer::from_file(dir.join("tokenizer/tokenizer.json")).unwrap();
        let pos = "Un gato naranja duerme sobre una ventana mientras cae la lluvia; luz cálida de atardecer, cámara lenta.";
        let te = Umt5::cargar(&dir.join("text_encoder")).unwrap();
        for (nombre, texto) in [("pos", pos), ("neg", NEGATIVO)] {
            let ids = tokens(&tk, texto).unwrap();
            let juez: Vec<u32> = d[&format!("ids_{nombre}")].as_slice::<i32>().iter().map(|&i| i as u32).collect();
            assert_eq!(ids, juez, "{nombre}: ids distintos");
            let h = te.codificar(&ids).unwrap();
            h.eval().unwrap();
            let (mio, juez) = (h.as_slice::<f32>(), d[&format!("h_{nombre}")].as_slice::<f32>());
            assert_eq!(mio.len(), juez.len());
            let (mut peor, mut escala) = (0.0f32, 0.0f32);
            for (a, b) in mio.iter().zip(juez) {
                peor = peor.max((a - b).abs());
                escala = escala.max(b.abs());
            }
            println!("{nombre}: {} piezas · diferencia máx {peor:.3e} (escala {escala:.2})", ids.len());
            assert!(peor <= 1e-3 * escala.max(1.0), "{nombre}: {peor}");
        }
    }

    #[test]
    fn limpia_como_el_pipeline() {
        assert_eq!(limpiar("  un  gato &amp;amp; un\tperro \n"), "un gato & un perro");
    }

    /// El calendario de 6 pasos tal como lo da diffusers (`juez_unipc.py`, CPU).
    #[test]
    fn calendario_como_diffusers() {
        let (s, t) = calendario(6, DESPLAZAMIENTO);
        assert_eq!(t, vec![999, 961, 909, 833, 714, 501]);
        let juez = [0.999_999_f32, 0.961_582_84, 0.909_214_85, 0.833_610_95, 0.714_897_3, 0.501_497_03, 0.0];
        assert_eq!(s, juez.to_vec());
        let (s50, t50) = calendario(50, DESPLAZAMIENTO);
        assert_eq!(&t50[..5], &[999, 995, 991, 987, 982]);
        assert_eq!(&t50[47..], &[244, 175, 96]);
        assert_eq!(&s50[48..], &[0.175_969_2_f32, 0.096_778_36, 0.0]);
    }

    #[test]
    fn resolver2_resuelve() {
        let x = resolver2([[1.0, 1.0], [-1.2, 1.0]], [0.3, -0.7]);
        assert!((x[0] + x[1] - 0.3).abs() < 1e-6 && (-1.2 * x[0] + x[1] + 0.7).abs() < 1e-6);
    }

    /// Juez: los 6 pasos de diffusers con muestras y salidas al azar (sin modelo, en CPU).
    /// `GOTV_JUEZ_UNIPC=~/.cache/gotv/juez-wan/unipc.safetensors cargo test -p gotv-mlx --release
    /// -- --ignored --nocapture unipc_como_diffusers`
    #[test]
    #[ignore]
    fn unipc_como_diffusers() {
        let Ok(r) = std::env::var("GOTV_JUEZ_UNIPC") else { return };
        let d = Array::load_safetensors(r).unwrap();
        let mut u = UniPc::nuevo(6);
        let mut x = d["x0"].clone();
        for i in 0..6 {
            x = u.paso(&d[&format!("m{i}")], &x).unwrap();
            x.eval().unwrap();
            let juez = d[&format!("x{}", i + 1)].as_slice::<f32>();
            let mio = x.as_slice::<f32>();
            let (mut peor, mut escala) = (0.0f32, 0.0f32);
            for (a, b) in mio.iter().zip(juez) {
                peor = peor.max((a - b).abs());
                escala = escala.max(b.abs());
            }
            println!("paso {i}: diferencia máx {peor:.3e} (escala {escala:.2})");
            assert!(peor <= 1e-5 * escala.max(1.0), "paso {i}: {peor}");
        }
    }
}
