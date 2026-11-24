//! M7 · música local: ACE-Step 1.5 turbo (ACE Studio y StepFun, MIT) sobre MLX, escrito a mano.
//!
//! El mismo cálculo que el manejador oficial (`acestep`, PyTorch) en un Mac, que corre en f32:
//! 1. **Texto**: la plantilla SFT (instrucción, descripción, metas) → Qwen3-Embedding-0.6B entero
//!    (`last_hidden_state`). La **letra** sólo pasa por su tabla de embeddings.
//! 2. **Condición**: proyector del texto, codificador de letra (8 capas bidireccionales con ventana
//!    deslizante alterna), codificador de timbre (4 capas; con el silencio de referencia da un token)
//!    → `[letra, timbre, texto]`.
//! 3. **DiT** de 24 capas (autoatención con RoPE y ventana, atención cruzada, AdaLN por el tiempo
//!    «meanflow» t y r) sobre `[silencio, máscara, ruido]` en parches de 2.
//! 4. **ODE turbo** de 8 pasos con la tabla fija por `shift` y la corrección DCW («double», Haar).
//! 5. **VAE Oobleck**: 25 cuadros/s × 1920 → estéreo a 48 kHz.
//!
//! El juez es el manejador oficial instrumentado (fuera del proyecto): `volcado` deja cada pieza.

use crate::modelo::{Modelo, M};
use anyhow::{anyhow, bail, Context, Result};
use gotv_engine::{anyhow_tr, bail_tr, tr};
use mlx_rs::fast::{self, ScaledDotProductAttentionMask};
use mlx_rs::ops::indexing::TryIndexOp;
use mlx_rs::ops;
use mlx_rs::{Array, Dtype};
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

const F32: Dtype = Dtype::Float32;
/// muestras de audio por cuadro latente (48 kHz / 25 Hz)
pub const MUESTRAS_POR_CUADRO: usize = 1920;
pub const FRECUENCIA: u32 = 48_000;
/// la instrucción por defecto de texto→música (`DEFAULT_DIT_INSTRUCTION`)
const INSTRUCCION: &str = "Fill the audio semantic mask based on the given conditions:";
/// con códigos del LM la tarea pasa a «cover» y el DiT lee la instrucción del LM
const INSTRUCCION_COVER: &str = "Generate audio semantic tokens based on the given conditions:";

/// Lo que se pide.
#[derive(Clone, Debug)]
pub struct Pedido {
    /// estilo, instrumentos, ambiente (en inglés, como la entrenaron)
    pub descripcion: String,
    /// letra; vacía = instrumental (`[Instrumental]`, como pide la documentación oficial)
    pub letra: String,
    /// código de idioma de la voz (`en`, `es`…); `unknown` sin voz
    pub idioma: String,
    pub segundos: f32,
    pub bpm: Option<u32>,
    /// p. ej. «C major», «A minor»
    pub tonalidad: String,
    /// «2», «3», «4» o «6» (2/4, 3/4, 4/4, 6/8)
    pub compas: String,
    pub semilla: u64,
    /// tabla de pasos del turbo: 1, 2 o 3 (el manejador oficial usa 1)
    pub shift: u32,
    /// el LM planificador (`thinking=True`, el defecto oficial): metadatos que falten + códigos de audio
    pub pensar: bool,
    /// códigos de audio ya hechos (5 Hz, 0..63 999): guían al DiT y saltan el LM
    pub codigos: Option<Vec<u32>>,
    /// M7d: un audio propio para versionar (cover) o repintar una parte (repaint); salta el LM
    pub fuente: Option<Fuente>,
    /// Ola PRO · el muestreo del LM de los códigos (temperatura, cfg, top-p, top-k, negativo)
    pub muestreo: crate::musica_lm::Muestreo,
    /// Ola PRO · audio de REFERENCIA de timbre y mezcla (`reference_audio`): global, no la melodía
    pub timbre: Option<std::path::PathBuf>,
    /// Ola PRO · el DiT BASE (su carpeta): CFG con APG y las tareas que sólo él sabe; `None` = turbo
    pub base: Option<std::path::PathBuf>,
    /// Ola PRO · PRO-4: el LM planificador (la carpeta); `None` = el de 1,7B del repo de ACE-Step 1.5
    pub lm: Option<std::path::PathBuf>,
    /// la tarea del base (separar, añadir o completar pistas sobre `fuente`)
    pub tarea: Option<TareaBase>,
    /// pasos del base (0 = 50) y su guía del CFG (0 = 7, el oficial)
    pub pasos_base: u32,
    pub guia_dit: f32,
}

/// El audio propio de un cover o un repaint (los valores por defecto del oficial).
#[derive(Clone, Debug)]
pub struct Fuente {
    pub ruta: std::path::PathBuf,
    /// `None` = cover; `Some((desde, hasta))` en segundos = repaint (`hasta` < 0: hasta el final;
    /// fuera del audio se rellena con silencio, como el oficial)
    pub repintar: Option<(f32, f32)>,
    /// `audio_cover_strength` (1 = toda la pieza guiada por el audio; ~0,2 para cambiar de estilo)
    pub fuerza_cover: f32,
    /// `cover_noise_strength` (0 = desde ruido puro; cerca de 1 = muy pegado al audio)
    pub ruido_cover: f32,
    /// `repaint_strength` en modo «balanced» (0 conserva al máximo, 1 repinta libre)
    pub fuerza_repintar: f32,
    /// para el juez: los latentes del audio ya codificados (`z` [1, T, 64]), sin pasar por el VAE
    pub latentes: Option<std::path::PathBuf>,
}

impl Fuente {
    pub fn cover(ruta: std::path::PathBuf) -> Fuente {
        Fuente { ruta, repintar: None, fuerza_cover: 1.0, ruido_cover: 0.0, fuerza_repintar: 0.5, latentes: None }
    }
}

impl Default for Pedido {
    fn default() -> Self {
        Pedido { descripcion: String::new(), letra: String::new(), idioma: "unknown".into(), segundos: 30.0, bpm: None, tonalidad: String::new(), compas: String::new(), semilla: 0, shift: 1, pensar: true, codigos: None, fuente: None, muestreo: Default::default(), timbre: None, base: None, lm: None, tarea: None, pasos_base: 0, guia_dit: 0.0 }
    }
}

/// Ola PRO · `apg_forward` del oficial (`apg_guidance.py`) para el CFG del DiT base: la diferencia
/// cond − incond pasa por el búfer de momento (−0,75), se acota a norma 2,5 sobre el eje del TIEMPO
/// (`dims=[1]`) y se queda sólo su parte ORTOGONAL a la predicción condicionada (eta 0):
/// `cond + (guía − 1)·ortogonal`. El oficial proyecta en f64 en la CPU; aquí, en f32.
pub fn apg(cond: &Array, incond: &Array, guia: f32, momento: &mut Option<Array>) -> Result<Array> {
    let mut d = ops::subtract(cond, incond).m()?;
    d = match momento.take() {
        Some(m) => ops::add(&d, &ops::multiply(&m, Array::from_f32(-0.75)).m()?).m()?,
        None => d,
    };
    *momento = Some(d.clone());
    let norma = |x: &Array| -> Result<Array> { ops::sqrt(&ops::sum_axes(&ops::square(x).m()?, &[1], true).m()?).m() };
    let escala = ops::minimum(Array::from_f32(1.0), &ops::divide(Array::from_f32(2.5), &norma(&d)?).m()?).m()?;
    let d = ops::multiply(&d, &escala).m()?;
    // `F.normalize(v1, dim)`: v / max(‖v‖, 1e-12)
    let v1 = ops::divide(cond, &ops::maximum(&norma(cond)?, Array::from_f32(1e-12)).m()?).m()?;
    let paralela = ops::multiply(&ops::sum_axes(&ops::multiply(&d, &v1).m()?, &[1], true).m()?, &v1).m()?;
    let ortogonal = ops::subtract(&d, &paralela).m()?;
    ops::add(cond, &ops::multiply(&ortogonal, Array::from_f32(guia - 1.0)).m()?).m()
}

/// Los pasos del base: `linspace(1, 0, n+1)` y, si shift ≠ 1, `t' = s·t / (1 + (s−1)·t)`, SIN el 0 final
/// (como la tabla del turbo): el bucle hace `n` pasos y el último va a x0 = xt − v·t, que es el Euler
/// del oficial hasta 0. Medido 2026-11-13: con el 0 dentro se hacía una pasada de más (CFG: dos) en t = 0.
pub fn horario_base(pasos: u32, shift: f32) -> Vec<f64> {
    let n = pasos.max(1) as usize;
    (0..n)
        .map(|i| {
            let t = 1.0 - i as f64 / n as f64;
            if (shift - 1.0).abs() > f32::EPSILON {
                shift as f64 * t / (1.0 + (shift as f64 - 1.0) * t)
            } else {
                t
            }
        })
        .collect()
}

/// Las tareas que sólo sabe el DiT base (`TASK_INSTRUCTIONS` del oficial). Las pistas, en inglés y
/// como las nombra el oficial (`vocals`, `drums`, `bass`…).
#[derive(Clone, Debug, PartialEq)]
pub enum TareaBase {
    /// separar una pista de la mezcla
    Extraer(String),
    /// crear una pista nueva sobre el audio
    Lego(String),
    /// completar el audio con esas pistas
    Completar(Vec<String>),
}

/// Las 12 pistas que entrenó ACE-Step (`TRACK_NAMES`).
pub const PISTAS: &[&str] = &["woodwinds", "brass", "fx", "synth", "strings", "percussion", "keyboard", "guitar", "bass", "drums", "backing_vocals", "vocals"];

impl TareaBase {
    pub fn instruccion(&self) -> String {
        match self {
            TareaBase::Extraer(p) => format!("Extract the {} track from the audio:", p.to_uppercase()),
            TareaBase::Lego(p) => format!("Generate the {} track based on the audio context:", p.to_uppercase()),
            TareaBase::Completar(ps) if ps.is_empty() => "Complete the input track:".into(),
            // el oficial las junta con « | » (`" | ".join(track_classes_upper)`)
            TareaBase::Completar(ps) => format!("Complete the input track with {}:", ps.iter().map(|p| p.to_uppercase()).collect::<Vec<_>>().join(" | ")),
        }
    }
}

/// La tarea del base desde las opciones (`{"tipo": "extraer"|"lego"|"completar", "pistas": [...]}`): sólo
/// las 12 pistas del oficial (en minúsculas, «backing vocals» → `backing_vocals`); extraer y lego llevan
/// UNA pista. `None` si no hay tarea o no se entiende.
pub fn tarea_de(opciones: &serde_json::Value) -> Option<TareaBase> {
    let t = opciones.get("tarea")?;
    let pistas: Vec<String> = t
        .get("pistas")
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|x| x.as_str()).map(|s| s.trim().to_lowercase().replace([' ', '-'], "_")).filter(|s| PISTAS.contains(&s.as_str())).collect())
        .unwrap_or_default();
    match t.get("tipo").and_then(|v| v.as_str())? {
        "extraer" | "extract" => Some(TareaBase::Extraer(pistas.into_iter().next()?)),
        "lego" | "anadir" | "add" => Some(TareaBase::Lego(pistas.into_iter().next()?)),
        "completar" | "complete" => Some(TareaBase::Completar(pistas)),
        _ => None,
    }
}

/// Ola PRO · el audio de REFERENCIA de timbre y mezcla como lo prepara el oficial
/// (`process_reference_audio`): si dura menos de 30 s se repite hasta pasarlos; se parte en tercios y de
/// cada tercio se toma un trozo de 10 s en un punto al azar (delante, en medio, detrás); los tres
/// juntos son 30 s. El azar sale de `semilla` (el oficial usa `random`: aquí se puede repetir). `None`
/// si es silencio (el oficial usa entonces el latente de silencio).
pub fn segmentos_referencia(onda: &[Vec<f32>; 2], semilla: u64) -> Option<[Vec<f32>; 2]> {
    let n0 = onda[0].len();
    if n0 == 0 || onda.iter().all(|c| c.iter().all(|x| x.abs() < 1e-6)) {
        return None;
    }
    let (meta, trozo) = (30 * FRECUENCIA as usize, 10 * FRECUENCIA as usize);
    let veces = meta.div_ceil(n0).max(1);
    let largo: [Vec<f32>; 2] = [onda[0].repeat(veces), onda[1].repeat(veces)];
    let total = largo[0].len();
    let tercio = total / 3;
    // un generador sencillo y repetible (splitmix64) para los tres puntos de corte
    let mut s = semilla ^ 0x7E3B_A11D;
    let mut azar = |hasta: usize| -> usize {
        s = s.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = s;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        (z % (hasta as u64 + 1)) as usize
    };
    let inicios = [azar(tercio.saturating_sub(trozo)), tercio + azar(tercio.saturating_sub(trozo)), 2 * tercio + azar((total - 2 * tercio).saturating_sub(trozo))];
    let corta = |c: &Vec<f32>| -> Vec<f32> { inicios.iter().flat_map(|&a| c[a..(a + trozo).min(c.len())].iter().copied()).collect() };
    Some([corta(&largo[0]), corta(&largo[1])])
}

/// Cuadros latentes para una duración: `max(128, ⌊s·48000⌋ // 1920)` (como el manejador oficial).
pub fn cuadros(segundos: f32) -> usize {
    (((segundos as f64) * FRECUENCIA as f64).floor() as usize / MUESTRAS_POR_CUADRO).max(128)
}

/// La plantilla SFT que lee el Qwen3-Embedding (`SFT_GEN_PROMPT` con metas de `_build_metadata_dict`).
pub fn texto_sft(p: &Pedido, cover: bool) -> String {
    // Ola PRO: las tareas del base llevan su propia instrucción (`generate_instruction` del oficial)
    let instruccion = match &p.tarea {
        Some(t) => t.instruccion(),
        None if cover => INSTRUCCION_COVER.to_string(),
        None => INSTRUCCION.to_string(),
    };
    let bpm = p.bpm.map(|b| b.to_string()).unwrap_or_else(|| "N/A".into());
    let tono = if p.tonalidad.trim().is_empty() { "N/A".to_string() } else { p.tonalidad.clone() };
    let compas = if p.compas.trim().is_empty() || p.compas == "N/A" { "N/A".to_string() } else { p.compas.clone() };
    let metas = format!("- bpm: {bpm}\n- timesignature: {compas}\n- keyscale: {tono}\n- duration: {} seconds\n", p.segundos as i64);
    format!("# Instruction\n{instruccion}\n\n# Caption\n{}\n\n# Metas\n{metas}<|endoftext|>\n", p.descripcion)
}

/// La letra con su encabezado de idioma (`_format_lyrics`).
pub fn texto_letra(p: &Pedido) -> String {
    let (letra, idioma) = if p.letra.trim().is_empty() { ("[Instrumental]".to_string(), "unknown".to_string()) } else { (etiquetas_oficiales(&p.letra), p.idioma.clone()) };
    format!("# Languages\n{idioma}\n\n# Lyric\n{letra}<|endoftext|>")
}

/// Las etiquetas de estructura van en inglés, como las entrenó ACE-Step ([verse], [chorus]…).
/// Medido 2026-11-12: el 9B escribió la letra en español y TRADUJO las etiquetas ([verso], [coro],
/// [ponte]). Sólo se tocan las líneas que son únicamente una etiqueta; el número se conserva.
pub fn etiquetas_oficiales(letra: &str) -> String {
    let mapa: &[(&[&str], &str)] = &[
        (&["verso", "estrofa", "verse", "couplet"], "verse"),
        (&["coro", "estribillo", "chorus", "refrain"], "chorus"),
        (&["precoro", "pre-coro", "pre coro", "pre-chorus", "prechorus"], "pre-chorus"),
        (&["puente", "ponte", "bridge"], "bridge"),
        (&["intro", "introducción", "introduccion", "entrada"], "intro"), // juez-idioma: datos (alias en español de la etiqueta)
        (&["outro", "final", "cierre", "salida", "coda"], "outro"),
        (&["solo"], "solo"),
        (&["gancho", "hook"], "hook"),
    ];
    letra
        .lines()
        .map(|l| {
            let t = l.trim();
            let Some(dentro) = t.strip_prefix('[').and_then(|r| r.strip_suffix(']')) else { return l.to_string() };
            let d = dentro.trim().to_lowercase();
            let (base, num) = match d.rsplit_once(' ') {
                Some((b, n)) if n.chars().all(|c| c.is_ascii_digit()) => (b.trim().to_string(), format!(" {n}")),
                _ => (d.clone(), String::new()),
            };
            match mapa.iter().find(|(v, _)| v.contains(&base.as_str())) {
                // ya en su forma oficial (aunque lleve mayúscula): tal cual, como la recibiría el oficial
                Some((_, oficial)) if base == *oficial => l.to_string(),
                Some((_, oficial)) => format!("[{oficial}{num}]"),
                None => l.to_string(),
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Los horarios del turbo (`SHIFT_TIMESTEPS` del modelo oficial).
pub fn horario(shift: u32) -> Result<Vec<f64>> {
    Ok(match shift {
        1 => vec![1.0, 0.875, 0.75, 0.625, 0.5, 0.375, 0.25, 0.125],
        2 => vec![1.0, 0.9333333333333333, 0.8571428571428571, 0.7692307692307693, 0.6666666666666666, 0.5454545454545454, 0.4, 0.2222222222222222],
        3 => vec![1.0, 0.9545454545454546, 0.9, 0.8333333333333334, 0.75, 0.6428571428571429, 0.5, 0.3],
        _ => bail_tr!("shift {shift} no existe en el turbo (1, 2 o 3)", "shift {shift} does not exist in the turbo (1, 2 or 3)"),
    })
}

// ───────────────────────────── piezas comunes (todo en f32) ─────────────────────────────

pub(crate) fn quitar(p: &mut HashMap<String, Array>, n: &str) -> Result<Array> {
    p.remove(n).ok_or_else(|| anyhow!("falta {n}"))?.as_dtype(F32).m()
}

/// Lineal (peso transpuesto [entrada, salida]) con sesgo opcional; con sesgo, `addmm` (un redondeo).
struct Lin {
    w_t: Array,
    b: Option<Array>,
}

impl Lin {
    fn de(p: &mut HashMap<String, Array>, n: &str) -> Result<Lin> {
        let w = quitar(p, &format!("{n}.weight"))?;
        let b = match p.remove(&format!("{n}.bias")) {
            Some(b) => Some(b.as_dtype(F32).m()?),
            None => None,
        };
        Ok(Lin { w_t: ops::transpose_axes(&w, &[1, 0]).m()?, b })
    }
    fn ap(&self, x: &Array) -> Result<Array> {
        match &self.b {
            Some(b) => ops::addmm(b, x, &self.w_t, 1.0, 1.0).m(),
            None => ops::matmul(x, &self.w_t).m(),
        }
    }
}

/// RMSNorm de Qwen3 en f32.
fn rms(x: &Array, w: &Array, eps: f32) -> Result<Array> {
    let var = ops::mean_axes(&ops::square(x).m()?, &[-1], true).m()?;
    ops::multiply(&ops::multiply(x, &ops::rsqrt(&ops::add(&var, Array::from_f32(eps)).m()?).m()?).m()?, w).m()
}

fn silu(x: &Array) -> Result<Array> {
    ops::multiply(x, &ops::sigmoid(x).m()?).m()
}

/// cos y sin [1, 1, L, d] de la RoPE de Qwen3 (`inv_freq = 1/θ^(2i/d)`, `emb = [ángulos, ángulos]`).
fn giro(l: i32, d: i32, theta: f32) -> (Array, Array) {
    let inv: Vec<f32> = (0..d / 2).map(|i| 1.0 / theta.powf((2 * i) as f32 / d as f32)).collect();
    let (mut c, mut s) = (Vec::with_capacity((l * d) as usize), Vec::with_capacity((l * d) as usize));
    for p in 0..l {
        for _ in 0..2 {
            for f in &inv {
                let a = p as f32 * f;
                c.push(a.cos());
                s.push(a.sin());
            }
        }
    }
    (Array::from_slice(&c, &[1, 1, l, d]), Array::from_slice(&s, &[1, 1, l, d]))
}

fn girar(x: &Array, cos: &Array, sin: &Array) -> Result<Array> {
    let p = ops::split_equal(x, 2, -1).m()?;
    let rot = ops::concatenate(&[ops::negative(&p[1]).m()?, p[0].clone()], -1).m()?;
    ops::add(&ops::multiply(x, cos).m()?, &ops::multiply(&rot, sin).m()?).m()
}

/// Máscara aditiva bidireccional con ventana: 0 si `|i − j| ≤ ventana`, el mínimo de f32 si no.
fn mascara_ventana(l: i32, ventana: i32) -> Array {
    let mut m = vec![0f32; (l * l) as usize];
    for i in 0..l {
        for j in 0..l {
            if (i - j).abs() > ventana {
                m[(i * l + j) as usize] = f32::MIN;
            }
        }
    }
    Array::from_slice(&m, &[1, 1, l, l])
}

/// Atención de ACE-Step (q/k con RMSNorm por cabeza, GQA). Sin `contexto` es autoatención con RoPE.
struct Atencion {
    q: Lin,
    k: Lin,
    v: Lin,
    o: Lin,
    qn: Array,
    kn: Array,
}

struct Cfg {
    cabezas: i32,
    kv: i32,
    dim: i32,
    eps: f32,
    theta: f32,
    ventana: i32,
    /// `true` = capa con ventana deslizante
    tipos: Vec<bool>,
}

impl Atencion {
    fn de(p: &mut HashMap<String, Array>, n: &str) -> Result<Atencion> {
        Ok(Atencion {
            q: Lin::de(p, &format!("{n}.q_proj"))?,
            k: Lin::de(p, &format!("{n}.k_proj"))?,
            v: Lin::de(p, &format!("{n}.v_proj"))?,
            o: Lin::de(p, &format!("{n}.o_proj"))?,
            qn: quitar(p, &format!("{n}.q_norm.weight"))?,
            kn: quitar(p, &format!("{n}.k_norm.weight"))?,
        })
    }

    fn cabezas(x: &Array, h: i32, d: i32) -> Result<Array> {
        ops::reshape(x, &[x.dim(0), x.dim(1), h, d]).m()
    }

    /// K y V de un contexto (la atención cruzada los calcula una vez para todos los pasos).
    fn kv(&self, c: &Cfg, ctx: &Array) -> Result<(Array, Array)> {
        let k = rms(&Self::cabezas(&self.k.ap(ctx)?, c.kv, c.dim)?, &self.kn, c.eps)?;
        let v = Self::cabezas(&self.v.ap(ctx)?, c.kv, c.dim)?;
        Ok((ops::transpose_axes(&k, &[0, 2, 1, 3]).m()?, ops::transpose_axes(&v, &[0, 2, 1, 3]).m()?))
    }

    fn ap(&self, c: &Cfg, x: &Array, rope: Option<(&Array, &Array)>, mascara: Option<&Array>, kv: Option<&(Array, Array)>) -> Result<Array> {
        let l = x.dim(1);
        let q = rms(&Self::cabezas(&self.q.ap(x)?, c.cabezas, c.dim)?, &self.qn, c.eps)?;
        let mut q = ops::transpose_axes(&q, &[0, 2, 1, 3]).m()?;
        let (k, v) = match kv {
            Some((k, v)) => (k.clone(), v.clone()),
            None => {
                let (mut k, v) = self.kv(c, x)?;
                if let Some((cos, sin)) = rope {
                    q = girar(&q, cos, sin)?;
                    k = girar(&k, cos, sin)?;
                }
                (k, v)
            }
        };
        let m = mascara.map(ScaledDotProductAttentionMask::Array);
        let a = fast::scaled_dot_product_attention(&q, &k, &v, 1.0 / (c.dim as f32).sqrt(), m, None::<&Array>).m()?;
        self.o.ap(&ops::reshape(&ops::transpose_axes(&a, &[0, 2, 1, 3]).m()?, &[x.dim(0), l, c.cabezas * c.dim]).m()?)
    }
}

struct Mlp {
    gate: Lin,
    up: Lin,
    down: Lin,
}

impl Mlp {
    fn de(p: &mut HashMap<String, Array>, n: &str) -> Result<Mlp> {
        Ok(Mlp { gate: Lin::de(p, &format!("{n}.gate_proj"))?, up: Lin::de(p, &format!("{n}.up_proj"))?, down: Lin::de(p, &format!("{n}.down_proj"))? })
    }
    fn ap(&self, x: &Array) -> Result<Array> {
        self.down.ap(&ops::multiply(&silu(&self.gate.ap(x)?)?, &self.up.ap(x)?).m()?)
    }
}

/// `AceStepEncoderLayer`: una capa de Qwen3 bidireccional.
struct CapaCod {
    ln_in: Array,
    att: Atencion,
    ln_post: Array,
    mlp: Mlp,
}

/// Los codificadores de letra y de timbre: entrada lineal, capas, norma.
struct Codificador {
    entrada: Lin,
    capas: Vec<CapaCod>,
    norma: Array,
}

impl Codificador {
    fn de(p: &mut HashMap<String, Array>, n: &str, capas: usize) -> Result<Codificador> {
        let mut v = Vec::new();
        for i in 0..capas {
            let c = format!("{n}.layers.{i}");
            v.push(CapaCod {
                ln_in: quitar(p, &format!("{c}.input_layernorm.weight"))?,
                att: Atencion::de(p, &format!("{c}.self_attn"))?,
                ln_post: quitar(p, &format!("{c}.post_attention_layernorm.weight"))?,
                mlp: Mlp::de(p, &format!("{c}.mlp"))?,
            });
        }
        Ok(Codificador { entrada: Lin::de(p, &format!("{n}.embed_tokens"))?, capas: v, norma: quitar(p, &format!("{n}.norm.weight"))? })
    }

    fn ap(&self, c: &Cfg, x: &Array) -> Result<Array> {
        self.capas_y_norma(c, self.entrada.ap(x)?)
    }

    /// Las capas y la norma sobre `h` [B, L, D] (cada fila del lote es su propia secuencia).
    fn capas_y_norma(&self, c: &Cfg, mut h: Array) -> Result<Array> {
        let l = h.dim(1);
        let (cos, sin) = giro(l, c.dim, c.theta);
        let ventana = mascara_ventana(l, c.ventana);
        for (i, capa) in self.capas.iter().enumerate() {
            let m = if c.tipos[i] { Some(&ventana) } else { None };
            let a = capa.att.ap(c, &rms(&h, &capa.ln_in, c.eps)?, Some((&cos, &sin)), m, None)?;
            h = ops::add(&h, &a).m()?;
            h = ops::add(&h, &capa.mlp.ap(&rms(&h, &capa.ln_post, c.eps)?)?).m()?;
        }
        rms(&h, &self.norma, c.eps)
    }
}

/// Códigos del LM → pistas a 25 Hz: el FSQ residual (un cuantizador, niveles [8,8,8,5,5,5],
/// `preserve_symmetry`: dígito d → d·2/(L−1) − 1) con su `project_out` (6 → 2048), y el
/// `AudioTokenDetokenizer`: cada token se repite en 5 parches + los tokens especiales, cada grupo
/// de 5 pasa SOLO por 2 capas bidireccionales, norma y `proj_out` (2048 → 64).
struct Destokenizador {
    niveles: Vec<u32>,
    fsq_out: Lin,
    cod: Codificador,
    especiales: Array,
    salida: Lin,
}

impl Destokenizador {
    fn de(p: &mut HashMap<String, Array>, niveles: Vec<u32>, capas: usize) -> Result<Destokenizador> {
        Ok(Destokenizador {
            fsq_out: Lin::de(p, "tokenizer.quantizer.project_out")?,
            cod: Codificador::de(p, "detokenizer", capas)?,
            especiales: quitar(p, "detokenizer.special_tokens")?,
            salida: Lin::de(p, "detokenizer.proj_out")?,
            niveles,
        })
    }

    /// Los valores del FSQ de cada código [N, 6] (`indices_to_level_indices` + escala simétrica).
    fn fsq(&self, codigos: &[u32]) -> Vec<f32> {
        let mut v = Vec::with_capacity(codigos.len() * self.niveles.len());
        for &c in codigos {
            let mut base = 1u32;
            for &l in &self.niveles {
                let d = (c / base) % l;
                v.push(d as f32 * (2.0 / (l - 1) as f32) - 1.0);
                base *= l;
            }
        }
        v
    }

    fn ap(&self, c: &Cfg, codigos: &[u32]) -> Result<Array> {
        let n = codigos.len() as i32;
        let q = Array::from_slice(&self.fsq(codigos), &[1, n, self.niveles.len() as i32]);
        let q = self.fsq_out.ap(&q)?;
        let h = self.cod.entrada.ap(&q)?; // [1, N, D]
        let d = h.dim(2);
        let h = ops::add(&ops::reshape(&h, &[n, 1, d]).m()?, &self.especiales).m()?; // [N, 5, D]
        let h = self.cod.capas_y_norma(c, h)?;
        let h = self.salida.ap(&h)?; // [N, 5, 64]
        ops::reshape(&h, &[1, n * h.dim(1), h.dim(2)]).m()
    }
}

/// El tokenizador de audio (cover con audio propio): latentes a 25 Hz → códigos a 5 Hz. Cada grupo de
/// 5 cuadros pasa por `audio_acoustic_proj`, el `AttentionPooler` (un token especial delante, 2 capas
/// bidireccionales, norma; sale el token especial) y el FSQ residual con su acotado DURO
/// (`project_in`, tanh(x/c)·c con c = 1 + 1/(L−1), recorte a [−1, 1] y ⌊(L−1)(x+1)/2 + ½⌋).
struct TokAudio {
    proj: Lin,
    pooler: Codificador,
    especial: Array,
    fsq_in: Lin,
}

impl TokAudio {
    fn de(p: &mut HashMap<String, Array>, capas: usize) -> Result<TokAudio> {
        Ok(TokAudio {
            proj: Lin::de(p, "tokenizer.audio_acoustic_proj")?,
            especial: quitar(p, "tokenizer.attention_pooler.special_token")?,
            pooler: Codificador::de(p, "tokenizer.attention_pooler", capas)?,
            fsq_in: Lin::de(p, "tokenizer.quantizer.project_in")?,
        })
    }

    /// Latentes [1, 5N, 64] → los códigos (0..63 999) de cada grupo.
    fn codigos(&self, c: &Cfg, x: &Array, niveles: &[u32]) -> Result<Vec<u32>> {
        let n = x.dim(1) / 5;
        let h = self.proj.ap(&ops::reshape(x, &[n, 5, x.dim(2)]).m()?)?;
        let h = self.pooler.entrada.ap(&h)?;
        let d = h.dim(2);
        let esp = ops::broadcast_to(&self.especial, &[n, 1, d]).m()?;
        let h = self.pooler.capas_y_norma(c, ops::concatenate(&[esp, h], 1).m()?)?;
        let z = self.fsq_in.ap(&h.try_index((.., 0, ..)).m()?)?; // [N, 6]
        z.eval().m()?;
        let v = z.as_slice::<f32>();
        let k = niveles.len();
        let mut out = Vec::with_capacity(n as usize);
        for g in 0..n as usize {
            let (mut idx, mut base) = (0u32, 1u32);
            for (j, &l) in niveles.iter().enumerate() {
                let cota = 1.0 + 1.0 / (l - 1) as f32;
                let s = ((v[g * k + j] / cota).tanh() * cota).clamp(-1.0, 1.0);
                let digito = (((l - 1) as f32 * (s + 1.0) / 2.0) + 0.5).floor() as u32;
                idx += digito.min(l - 1) * base;
                base *= l;
            }
            out.push(idx);
        }
        Ok(out)
    }
}

/// `TimestepEmbedding`: senoidal de 256 (t·1000, [cos, sin]) → lineal → SiLU → lineal (= temb) y
/// `time_proj(SiLU(temb))` → [1, 6, D].
struct Tiempo {
    l1: Lin,
    l2: Lin,
    proj: Lin,
}

impl Tiempo {
    fn de(p: &mut HashMap<String, Array>, n: &str) -> Result<Tiempo> {
        Ok(Tiempo { l1: Lin::de(p, &format!("{n}.linear_1"))?, l2: Lin::de(p, &format!("{n}.linear_2"))?, proj: Lin::de(p, &format!("{n}.time_proj"))? })
    }
    fn ap(&self, t: f32) -> Result<(Array, Array)> {
        let medio = 128usize;
        let t = t * 1000.0;
        let c = -(10000f64.ln()) as f32;
        let mut v = vec![0f32; 2 * medio];
        for j in 0..medio {
            let a = t * ((c * j as f32) / medio as f32).exp();
            v[j] = a.cos();
            v[medio + j] = a.sin();
        }
        let e = Array::from_slice(&v, &[1, 2 * medio as i32]);
        let temb = self.l2.ap(&silu(&self.l1.ap(&e)?)?)?;
        let p = self.proj.ap(&silu(&temb)?)?;
        let d = temb.dim(-1);
        Ok((temb, ops::reshape(&p, &[1, 6, d]).m()?))
    }
}

struct CapaDit {
    sst: Array,
    n_auto: Array,
    auto: Atencion,
    n_cruz: Array,
    cruz: Atencion,
    n_mlp: Array,
    mlp: Mlp,
}

/// El modelo ACE-Step turbo: condición + DiT.
pub struct AceStep {
    cfg: Cfg,
    texto: Lin,
    letra: Codificador,
    timbre: Codificador,
    t: Tiempo,
    t_r: Tiempo,
    proj_in_w: Array,
    proj_in_b: Array,
    cond: Lin,
    capas: Vec<CapaDit>,
    n_out: Array,
    sst_out: Array,
    proj_out_w: Array,
    proj_out_b: Array,
    parche: i32,
    destok: Destokenizador,
    tok: TokAudio,
    /// Ola PRO · el «sin condición» del CFG del base (`null_condition_emb`, [1, 1, 2048])
    nulo: Array,
    /// el turbo (8 pasos destilados) o el base (CFG + APG)
    pub turbo: bool,
}

/// La condición ya preparada para el DiT: el texto proyectado por `condition_embedder` y la K/V
/// de la atención cruzada de cada capa (iguales en todos los pasos).
pub struct Condicion {
    kv: Vec<(Array, Array)>,
}

impl AceStep {
    pub fn cargar(dir: &Path) -> Result<AceStep> {
        let cfg: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dir.join("config.json")).context("acestep-v15-turbo/config.json")?)?;
        let n = |k: &str| cfg.get(k).and_then(|v| v.as_i64()).ok_or_else(|| anyhow_tr!("config de ACE-Step sin {k}", "ACE-Step config without {k}"));
        // Ola PRO: el turbo (8 pasos, sin guía) o el BASE (misma arquitectura; CFG con APG, 50 pasos)
        let turbo = cfg.get("is_turbo").and_then(|v| v.as_bool()).unwrap_or(false);
        let tipos: Vec<bool> = cfg.get("layer_types").and_then(|v| v.as_array()).ok_or_else(|| anyhow_tr!("config sin layer_types", "config without layer_types"))?.iter().map(|t| t.as_str() == Some("sliding_attention")).collect();
        let c = Cfg {
            cabezas: n("num_attention_heads")? as i32,
            kv: n("num_key_value_heads")? as i32,
            dim: n("head_dim")? as i32,
            eps: cfg.get("rms_norm_eps").and_then(|v| v.as_f64()).unwrap_or(1e-6) as f32,
            theta: cfg.get("rope_theta").and_then(|v| v.as_f64()).unwrap_or(1e6) as f32,
            ventana: n("sliding_window")? as i32,
            tipos,
        };
        let mut p = Array::load_safetensors(dir.join("model.safetensors")).map_err(|e| anyhow!("acestep: {e:?}"))?;
        // el tokenizador de audio entero: su salida FSQ (códigos del LM → pistas) y su entrada (audio
        // propio de un cover → códigos)
        p.retain(|k, _| k.starts_with("encoder.") || k.starts_with("decoder.") || k.starts_with("detokenizer.") || k.starts_with("tokenizer.") || k == "null_condition_emb");
        let niveles: Vec<u32> = cfg.get("fsq_input_levels").and_then(|v| v.as_array()).ok_or_else(|| anyhow_tr!("config sin fsq_input_levels", "config without fsq_input_levels"))?.iter().filter_map(|v| v.as_u64().map(|x| x as u32)).collect();
        p.remove("encoder.timbre_encoder.special_token");
        let mut capas = Vec::new();
        for i in 0..n("num_hidden_layers")? {
            let b = format!("decoder.layers.{i}");
            capas.push(CapaDit {
                sst: quitar(&mut p, &format!("{b}.scale_shift_table"))?,
                n_auto: quitar(&mut p, &format!("{b}.self_attn_norm.weight"))?,
                auto: Atencion::de(&mut p, &format!("{b}.self_attn"))?,
                n_cruz: quitar(&mut p, &format!("{b}.cross_attn_norm.weight"))?,
                cruz: Atencion::de(&mut p, &format!("{b}.cross_attn"))?,
                n_mlp: quitar(&mut p, &format!("{b}.mlp_norm.weight"))?,
                mlp: Mlp::de(&mut p, &format!("{b}.mlp"))?,
            });
        }
        // conv del parche: PyTorch [O, I, K] → MLX [O, K, I]; la traspuesta [I, O, K] → [O, K, I]
        let proj_in_w = ops::transpose_axes(&quitar(&mut p, "decoder.proj_in.1.weight")?, &[0, 2, 1]).m()?;
        let proj_out_w = ops::transpose_axes(&quitar(&mut p, "decoder.proj_out.1.weight")?, &[1, 2, 0]).m()?;
        let a = AceStep {
            texto: Lin::de(&mut p, "encoder.text_projector")?,
            letra: Codificador::de(&mut p, "encoder.lyric_encoder", n("num_lyric_encoder_hidden_layers")? as usize)?,
            timbre: Codificador::de(&mut p, "encoder.timbre_encoder", n("num_timbre_encoder_hidden_layers")? as usize)?,
            t: Tiempo::de(&mut p, "decoder.time_embed")?,
            t_r: Tiempo::de(&mut p, "decoder.time_embed_r")?,
            proj_in_w,
            proj_in_b: quitar(&mut p, "decoder.proj_in.1.bias")?,
            cond: Lin::de(&mut p, "decoder.condition_embedder")?,
            capas,
            n_out: quitar(&mut p, "decoder.norm_out.weight")?,
            sst_out: quitar(&mut p, "decoder.scale_shift_table")?,
            proj_out_w,
            proj_out_b: quitar(&mut p, "decoder.proj_out.1.bias")?,
            parche: n("patch_size")? as i32,
            tok: TokAudio::de(&mut p, n("num_attention_pooler_hidden_layers")? as usize)?,
            destok: Destokenizador::de(&mut p, niveles, n("num_attention_pooler_hidden_layers")? as usize)?,
            nulo: quitar(&mut p, "null_condition_emb")?,
            turbo,
            cfg: c,
        };
        if !p.is_empty() {
            let mut sobran: Vec<_> = p.keys().cloned().collect();
            sobran.sort();
            bail_tr!("pesos de ACE-Step sin usar: {:?}", "unused ACE-Step weights: {:?}", &sobran[..sobran.len().min(5)]);
        }
        Ok(a)
    }

    /// `AceStepConditionEncoder` + `condition_embedder` + K/V cruzadas: texto [1, Lt, 1024],
    /// letra [1, Ll, 1024], referencia de timbre [1, 750, 64] → la condición del DiT.
    pub fn condicion(&self, texto: &Array, letra: &Array, timbre: &Array) -> Result<(Array, Condicion)> {
        let t = self.texto.ap(texto)?;
        let l = self.letra.ap(&self.cfg, letra)?;
        let tb = self.timbre.ap(&self.cfg, timbre)?.try_index((.., 0..1, ..)).m()?;
        // `pack_sequences` con máscaras completas (un pedido, sin relleno) = concatenar
        let enc = ops::concatenate(&[l, tb, t], 1).m()?;
        let e = self.cond.ap(&enc)?;
        let kv = self.capas.iter().map(|c| c.cruz.kv(&self.cfg, &e)).collect::<Result<Vec<_>>>()?;
        Ok((enc, Condicion { kv }))
    }

    /// Ola PRO · la condición NULA del CFG del base: `null_condition_emb` con la forma de la salida
    /// del codificador (`expand_as(encoder_hidden_states)`), luego `condition_embedder` y K/V.
    pub fn condicion_nula(&self, enc: &Array) -> Result<Condicion> {
        let nulo = ops::broadcast_to(&self.nulo.as_dtype(enc.dtype()).m()?, enc.shape()).m()?;
        let e = self.cond.ap(&nulo)?;
        let kv = self.capas.iter().map(|c| c.cruz.kv(&self.cfg, &e)).collect::<Result<Vec<_>>>()?;
        Ok(Condicion { kv })
    }

    /// Códigos del LM (5 Hz) → pistas [1, 5N, 64] a 25 Hz (`_decode_audio_codes_to_latents`).
    pub fn pistas(&self, codigos: &[u32]) -> Result<Array> {
        if codigos.is_empty() {
            bail_tr!("sin códigos de audio", "no audio codes");
        }
        self.destok.ap(&self.cfg, codigos)
    }

    /// Audio propio → pistas (`prepare_condition` con `is_covers` y sin códigos): `tokenize` (relleno
    /// con el silencio hasta un múltiplo de 5) → códigos → `detokenize` → recortado a los T cuadros.
    /// Devuelve también los códigos (el juez los compara).
    pub fn pistas_de_latentes(&self, x: &Array, silencio: &Array) -> Result<(Array, Vec<u32>)> {
        let t = x.dim(1);
        let x = if t % 5 != 0 { ops::concatenate(&[x.clone(), silencio.try_index((.., ..5 - t % 5, ..)).m()?], 1).m()? } else { x.clone() };
        let codigos = self.tok.codigos(&self.cfg, &x, &self.destok.niveles)?;
        Ok((self.destok.ap(&self.cfg, &codigos)?.try_index((.., ..t, ..)).m()?, codigos))
    }

    /// Una pasada del DiT: xt [1, T, 64], contexto [1, T, 128] → velocidad [1, T, 64].
    pub fn dit(&self, xt: &Array, ctx: &Array, t: f32, cond: &Condicion, cancelar: Option<&AtomicBool>) -> Result<Array> {
        let c = &self.cfg;
        let (temb_t, proj_t) = self.t.ap(t)?;
        let (temb_r, proj_r) = self.t_r.ap(0.0)?; // t − r = 0 (el turbo usa r = t)
        let temb = ops::add(&temb_t, &temb_r).m()?;
        let proj = ops::add(&proj_t, &proj_r).m()?;
        let mut h = ops::concatenate(&[ctx.clone(), xt.clone()], -1).m()?;
        let largo = h.dim(1);
        if largo % self.parche != 0 {
            let relleno = ops::zeros::<f32>(&[1, self.parche - largo % self.parche, h.dim(2)]).m()?;
            h = ops::concatenate(&[h, relleno], 1).m()?;
        }
        h = ops::add(&ops::conv1d(&h, &self.proj_in_w, self.parche, 0, 1, 1).m()?, &self.proj_in_b).m()?;
        let l = h.dim(1);
        let (cos, sin) = giro(l, c.dim, c.theta);
        let ventana = mascara_ventana(l, c.ventana);
        let uno = Array::from_f32(1.0);
        for (i, capa) in self.capas.iter().enumerate() {
            if cancelar.is_some_and(|x| x.load(Ordering::Relaxed)) {
                bail!("cancelado");
            }
            let m = ops::split_equal(&ops::add(&capa.sst, &proj).m()?, 6, 1).m()?;
            let n = ops::add(&ops::multiply(&rms(&h, &capa.n_auto, c.eps)?, &ops::add(&uno, &m[1]).m()?).m()?, &m[0]).m()?;
            let masc = if c.tipos[i] { Some(&ventana) } else { None };
            let a = capa.auto.ap(c, &n, Some((&cos, &sin)), masc, None)?;
            h = ops::add(&h, &ops::multiply(&a, &m[2]).m()?).m()?;
            let x = capa.cruz.ap(c, &rms(&h, &capa.n_cruz, c.eps)?, None, None, Some(&cond.kv[i]))?;
            h = ops::add(&h, &x).m()?;
            let n = ops::add(&ops::multiply(&rms(&h, &capa.n_mlp, c.eps)?, &ops::add(&uno, &m[4]).m()?).m()?, &m[3]).m()?;
            h = ops::add(&h, &ops::multiply(&capa.mlp.ap(&n)?, &m[5]).m()?).m()?;
            if cancelar.is_some() {
                h.eval().m()?;
            }
        }
        // AdaLN de salida: (desplazamiento, escala) = tabla + temb — el DESPLAZAMIENTO primero
        let so = ops::split_equal(&ops::add(&self.sst_out, &ops::expand_dims(&temb, 1).m()?).m()?, 2, 1).m()?;
        let h = ops::add(&ops::multiply(&rms(&h, &self.n_out, c.eps)?, &ops::add(&uno, &so[1]).m()?).m()?, &so[0]).m()?;
        let h = ops::add(&ops::conv_transpose1d(&h, &self.proj_out_w, self.parche, 0, 1, 0, 1).m()?, &self.proj_out_b).m()?;
        h.try_index((.., ..largo, ..)).m()
    }
}

/// DCW «double» con Haar (cero de relleno), en forma cerrada: `x + a·P_L(x−y) + b·P_H(x−y)`, donde
/// P_L promedia cada par (x0,x1),(x2,x3)… y P_H deja ± la media diferencia; con T impar la última
/// muestra lleva d/2 en las dos bandas. Medido contra `pytorch_wavelets` (DWT1D J=1).
pub fn dcw_doble(x: &Array, y: &Array, a: f32, b: f32) -> Result<Array> {
    let t = x.dim(1);
    let c = x.dim(2);
    let d = ops::subtract(x, y).m()?;
    let par = t - t % 2;
    let dp = ops::reshape(&d.try_index((.., ..par, ..)).m()?, &[1, par / 2, 2, c]).m()?;
    let s = ops::split_equal(&dp, 2, 2).m()?;
    let media = ops::multiply(&ops::add(&s[0], &s[1]).m()?, Array::from_f32(0.5)).m()?;
    let dif = ops::multiply(&ops::subtract(&s[0], &s[1]).m()?, Array::from_f32(0.5)).m()?;
    let pl = ops::reshape(&ops::concatenate(&[media.clone(), media], 2).m()?, &[1, par, c]).m()?;
    let ph = ops::reshape(&ops::concatenate(&[dif.clone(), ops::negative(&dif).m()?], 2).m()?, &[1, par, c]).m()?;
    let (pl, ph) = if t % 2 == 1 {
        let ult = ops::multiply(&d.try_index((.., par.., ..)).m()?, Array::from_f32(0.5)).m()?;
        (ops::concatenate(&[pl, ult.clone()], 1).m()?, ops::concatenate(&[ph, ult], 1).m()?)
    } else {
        (pl, ph)
    };
    ops::add(&ops::add(x, &ops::multiply(&pl, Array::from_f32(a)).m()?).m()?, &ops::multiply(&ph, Array::from_f32(b)).m()?).m()
}

// ───────────────────────────── VAE Oobleck ─────────────────────────────

/// Conv1d con normalización de pesos ya fundida (`w = g·v/‖v‖`, norma por la dimensión 0).
pub(crate) struct Conv {
    w: Array,
    b: Option<Array>,
}

pub(crate) fn peso_normalizado(p: &mut HashMap<String, Array>, n: &str) -> Result<Array> {
    let g = quitar(p, &format!("{n}.weight_g"))?;
    let v = quitar(p, &format!("{n}.weight_v"))?;
    let norma = ops::sqrt(&ops::sum_axes(&ops::square(&v).m()?, &[1, 2], true).m()?).m()?;
    ops::multiply(&g, &ops::divide(&v, &norma).m()?).m()
}

impl Conv {
    /// `traspuesta`: ConvTranspose1d ([I, O, K] → [O, K, I]); si no, Conv1d ([O, I, K] → [O, K, I]).
    pub(crate) fn de(p: &mut HashMap<String, Array>, n: &str, traspuesta: bool) -> Result<Conv> {
        let w = peso_normalizado(p, n)?;
        let w = if traspuesta { ops::transpose_axes(&w, &[1, 2, 0]).m()? } else { ops::transpose_axes(&w, &[0, 2, 1]).m()? };
        let b = match p.remove(&format!("{n}.bias")) {
            Some(b) => Some(b.as_dtype(F32).m()?),
            None => None,
        };
        Ok(Conv { w, b })
    }
    fn sesgo(&self, x: Array) -> Result<Array> {
        match &self.b {
            Some(b) => ops::add(&x, b).m(),
            None => Ok(x),
        }
    }
    pub(crate) fn ap(&self, x: &Array, relleno: i32, dilatacion: i32) -> Result<Array> {
        self.sesgo(ops::conv1d(x, &self.w, 1, relleno, dilatacion, 1).m()?)
    }
    fn ap_t(&self, x: &Array, paso: i32, relleno: i32) -> Result<Array> {
        self.ap_t_extra(x, paso, relleno, 0)
    }
    /// Conv1d con paso (el codificador del VAE baja la resolución así).
    pub(crate) fn ap_paso(&self, x: &Array, paso: i32, relleno: i32) -> Result<Array> {
        self.sesgo(ops::conv1d(x, &self.w, paso, relleno, 1, 1).m()?)
    }
    /// ConvTranspose1d con `output_padding` (el DAC lo usa en los pasos impares).
    pub(crate) fn ap_t_extra(&self, x: &Array, paso: i32, relleno: i32, extra: i32) -> Result<Array> {
        self.sesgo(ops::conv_transpose1d(x, &self.w, paso, relleno, 1, extra, 1).m()?)
    }
    /// Una Conv1d SIN normalización de pesos (`weight` [O, I, K] → [O, K, I]) con su sesgo.
    pub(crate) fn plana(p: &mut HashMap<String, Array>, n: &str) -> Result<Conv> {
        let w = ops::transpose_axes(&quitar(p, &format!("{n}.weight"))?, &[0, 2, 1]).m()?;
        Ok(Conv { w, b: Some(quitar(p, &format!("{n}.bias"))?) })
    }
}

/// Snake: `x + 1/(β + 1e-9) · sin(α·x)²` con α = e^alpha, β = e^beta.
pub(crate) struct Snake {
    alfa: Array,
    inv_beta: Array,
}

impl Snake {
    pub(crate) fn de(p: &mut HashMap<String, Array>, n: &str) -> Result<Snake> {
        let a = ops::exp(&ops::reshape(&quitar(p, &format!("{n}.alpha"))?, &[-1]).m()?).m()?;
        let b = ops::exp(&ops::reshape(&quitar(p, &format!("{n}.beta"))?, &[-1]).m()?).m()?;
        let inv_beta = ops::divide(Array::from_f32(1.0), &ops::add(&b, Array::from_f32(1e-9)).m()?).m()?;
        Ok(Snake { alfa: a, inv_beta })
    }
    pub(crate) fn ap(&self, x: &Array) -> Result<Array> {
        let s = ops::sin(&ops::multiply(&self.alfa, x).m()?).m()?;
        ops::add(x, &ops::multiply(&self.inv_beta, &ops::square(&s).m()?).m()?).m()
    }
}

struct Residual {
    s1: Snake,
    c1: Conv,
    s2: Snake,
    c2: Conv,
    dil: i32,
}

impl Residual {
    fn ap(&self, x: &Array) -> Result<Array> {
        let h = self.c1.ap(&self.s1.ap(x)?, 3 * self.dil, self.dil)?;
        let h = self.c2.ap(&self.s2.ap(&h)?, 0, 1)?;
        ops::add(x, &h).m()
    }
}

struct Bloque {
    snake: Snake,
    sube: Conv,
    paso: i32,
    res: Vec<Residual>,
}

/// El decodificador de `AutoencoderOobleck`, en f32.
pub struct Vae {
    entrada: Conv,
    bloques: Vec<Bloque>,
    snake: Snake,
    salida: Conv,
}

impl Vae {
    pub fn cargar(dir: &Path) -> Result<Vae> {
        let cfg: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dir.join("config.json")).context("vae/config.json")?)?;
        let mut bajadas: Vec<i32> = serde_json::from_value(cfg.get("downsampling_ratios").cloned().ok_or_else(|| anyhow_tr!("vae sin downsampling_ratios", "vae without downsampling_ratios"))?)?;
        bajadas.reverse();
        let mut p = Array::load_safetensors(dir.join("diffusion_pytorch_model.safetensors")).map_err(|e| anyhow!("vae: {e:?}"))?;
        p.retain(|k, _| k.starts_with("decoder."));
        let mut bloques = Vec::new();
        for (i, &paso) in bajadas.iter().enumerate() {
            let b = format!("decoder.block.{i}");
            let mut res = Vec::new();
            for (j, dil) in [1, 3, 9].into_iter().enumerate() {
                let r = format!("{b}.res_unit{}", j + 1);
                res.push(Residual { s1: Snake::de(&mut p, &format!("{r}.snake1"))?, c1: Conv::de(&mut p, &format!("{r}.conv1"), false)?, s2: Snake::de(&mut p, &format!("{r}.snake2"))?, c2: Conv::de(&mut p, &format!("{r}.conv2"), false)?, dil });
            }
            bloques.push(Bloque { snake: Snake::de(&mut p, &format!("{b}.snake1"))?, sube: Conv::de(&mut p, &format!("{b}.conv_t1"), true)?, paso, res });
        }
        let v = Vae { entrada: Conv::de(&mut p, "decoder.conv1", false)?, bloques, snake: Snake::de(&mut p, "decoder.snake1")?, salida: Conv::de(&mut p, "decoder.conv2", false)? };
        if !p.is_empty() {
            let mut sobran: Vec<_> = p.keys().cloned().collect();
            sobran.sort();
            bail_tr!("pesos del VAE de audio sin usar: {:?}", "unused audio VAE weights: {:?}", &sobran[..sobran.len().min(5)]);
        }
        Ok(v)
    }

    /// Latentes [1, T, 64] → audio [1, T·1920, 2], POR TRAMOS de 128 cuadros con 32 de margen a
    /// cada lado (el decodificador sólo mira unos pocos cuadros alrededor: el centro de cada tramo
    /// sale como en la decodificación entera). Medido 2026-11-12: entero, 120 s pedían 51,5 GB.
    pub fn decodificar(&self, z: &Array, cancelar: Option<&AtomicBool>) -> Result<Array> {
        const TRAMO: i32 = 128;
        const MARGEN: i32 = 32;
        let t = z.dim(1);
        if t <= TRAMO + 2 * MARGEN {
            return self.decodificar_entero(z, cancelar);
        }
        let mut partes = Vec::new();
        let mut s = 0;
        while s < t {
            let e = (s + TRAMO).min(t);
            let (a, b) = ((s - MARGEN).max(0), (e + MARGEN).min(t));
            let x = self.decodificar_entero(&z.try_index((.., a..b, ..)).m()?, cancelar)?;
            let ini = (s - a) * MUESTRAS_POR_CUADRO as i32;
            let p = x.try_index((.., ini..ini + (e - s) * MUESTRAS_POR_CUADRO as i32, ..)).m()?;
            p.eval().m()?;
            partes.push(p);
            drop(x);
            let _ = mlx_rs::memory::clear_cache();
            s = e;
        }
        ops::concatenate(&partes, 1).m()
    }

    /// La decodificación de un trozo entero (sin tramos).
    pub fn decodificar_entero(&self, z: &Array, cancelar: Option<&AtomicBool>) -> Result<Array> {
        let mut x = self.entrada.ap(z, 3, 1)?;
        for b in &self.bloques {
            if cancelar.is_some_and(|c| c.load(Ordering::Relaxed)) {
                bail!("cancelado");
            }
            x = b.sube.ap_t(&b.snake.ap(&x)?, b.paso, (b.paso + 1) / 2)?;
            for r in &b.res {
                x = r.ap(&x)?;
            }
            x.eval().m()?;
        }
        self.salida.ap(&self.snake.ap(&x)?, 3, 1)
    }
}

// ───────────────────────────── el pipeline ─────────────────────────────

/// El audio listo: estéreo intercalado (L, R, L, R…) a 48 kHz, en [-1, 1].
pub struct Audio {
    pub muestras: Vec<f32>,
    pub canales: u16,
    pub frecuencia: u32,
    /// lo que decidió el LM (bpm, tono, compás), si planeó
    pub plan: Option<String>,
}

impl Audio {
    pub fn segundos(&self) -> f32 {
        self.muestras.len() as f32 / self.canales as f32 / self.frecuencia as f32
    }

    /// WAV en coma flotante de 32 bits (como guarda el oficial sus WAV).
    pub fn wav(&self) -> Vec<u8> {
        let datos = (self.muestras.len() * 4) as u32;
        let mut w = Vec::with_capacity(44 + datos as usize);
        w.extend(b"RIFF");
        w.extend((36 + datos).to_le_bytes());
        w.extend(b"WAVEfmt ");
        w.extend(16u32.to_le_bytes());
        w.extend(3u16.to_le_bytes()); // IEEE float
        w.extend(self.canales.to_le_bytes());
        w.extend(self.frecuencia.to_le_bytes());
        w.extend((self.frecuencia * self.canales as u32 * 4).to_le_bytes());
        w.extend((self.canales * 4).to_le_bytes());
        w.extend(32u16.to_le_bytes());
        w.extend(b"data");
        w.extend(datos.to_le_bytes());
        for m in &self.muestras {
            w.extend(m.to_le_bytes());
        }
        w
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Etapa {
    /// el LM escribe los metadatos que faltan (fase 1)
    Plan,
    /// se lee y se codifica el audio propio (cover/repaint)
    Fuente,
    /// el LM escribe los códigos de audio (fase 2): hechos, total
    Codigos(u32, u32),
    Texto,
    Paso(u32, u32),
    Audio,
}

/// Los tokens como el tokenizador oficial (con su `<|endoftext|>` final), truncados a `tope`.
fn tokens(tk: &tokenizers::Tokenizer, texto: &str, tope: usize) -> Result<Vec<u32>> {
    let mut ids = tk.encode(texto, true).map_err(|e| anyhow_tr!("tokenizar: {e}", "tokenizing: {e}"))?.get_ids().to_vec();
    ids.truncate(tope);
    Ok(ids)
}

/// Genera la música. `ruido`: el del juez (si no, el de MLX con la semilla); `volcado`: dónde dejar cada pieza.
pub fn generar(dir: &Path, p: &Pedido, progreso: &dyn Fn(Etapa), cancelar: &AtomicBool, ruido: Option<&Path>, volcado: Option<&Path>) -> Result<Audio> {
    if p.tarea.is_some() && p.base.is_none() {
        bail_tr!("separar, añadir o completar pistas es del DiT BASE de ACE-Step (el turbo no lo sabe)", "splitting, adding or completing tracks needs ACE-Step's BASE DiT (the turbo cannot)");
    }
    if p.tarea.is_some() && p.fuente.is_none() {
        bail_tr!("esa tarea trabaja sobre un audio: falta el audio", "that task works on an audio: the audio is missing");
    }
    if p.fuente.is_none() && !(5.0..=600.0).contains(&p.segundos) {
        bail_tr!("duración fuera de rango (5–600 s): {}", "duration out of range (5–600 s): {}", p.segundos);
    }
    let corte = || -> Result<()> {
        if cancelar.load(Ordering::Relaxed) {
            bail!("cancelado");
        }
        Ok(())
    };
    let guardar = |n: &str, a: &Array| -> Result<()> {
        if let Some(d) = volcado {
            std::fs::create_dir_all(d)?;
            Array::save_safetensors([(n, a)], None, d.join(format!("{n}.safetensors"))).map_err(|e| anyhow!("volcado {n}: {e:?}"))?;
        }
        Ok(())
    };
    // 0. el LM planificador (`thinking=True`): fase 1 si faltan metadatos, fase 2 (códigos) siempre
    let mut p = p.clone();
    let mut plan_lm: Option<String> = None;
    // (cover y repaint son tareas de condicionamiento directo: el oficial no llama al LM)
    // (las tareas del base tampoco: se condicionan con el audio y GOTV no les pide plan al LM)
    if p.codigos.is_none() && p.pensar && p.fuente.is_none() && p.tarea.is_none() {
        let lm = crate::musica_lm::Lm::cargar(&p.lm.clone().unwrap_or_else(|| dir.join("acestep-5Hz-lm-1.7B")))?;
        let letra_lm = if p.letra.trim().is_empty() { "[Instrumental]".to_string() } else { etiquetas_oficiales(&p.letra) };
        let original = p.descripcion.clone();
        let mut metas = crate::musica_lm::Metas {
            bpm: p.bpm,
            duracion: Some(p.segundos as u32),
            tonalidad: Some(p.tonalidad.trim().to_string()).filter(|s| !s.is_empty() && !s.eq_ignore_ascii_case("n/a")),
            compas: Some(p.compas.trim().to_string()).filter(|s| !s.is_empty() && !s.eq_ignore_ascii_case("n/a")),
            ..Default::default()
        };
        if metas.bpm.is_none() || metas.tonalidad.is_none() || metas.compas.is_none() {
            progreso(Etapa::Plan);
            let plan = lm.plan(&p.descripcion, &letra_lm, &metas, p.semilla, cancelar)?;
            // como `_update_metadata_from_lm`: lo del usuario manda y el LM llena lo que falta; su
            // descripción pasa al DiT (`use_cot_caption`). Su idioma NO: el oficial lo busca como
            // `vocal_language` y el parser lo guarda como `language` (medido en el código, 2026-11-12)
            p.bpm = p.bpm.or(plan.bpm);
            if p.tonalidad.trim().is_empty() {
                p.tonalidad = plan.tonalidad.clone().unwrap_or_default();
            }
            if p.compas.trim().is_empty() {
                p.compas = plan.compas.clone().unwrap_or_default();
            }
            if let Some(c) = &plan.caption {
                p.descripcion = c.clone();
            }
            // la CoT de la fase 2 es lo que se leyó de la fase 1, tal cual
            metas = plan;
        }
        corte()?;
        let codigos = lm.codigos(
            &original, // el LM lee la descripción del usuario; la suya va en la CoT
            &letra_lm,
            &crate::musica_lm::cot(&metas),
            p.segundos,
            &p.muestreo,
            p.semilla,
            cancelar,
            &|i, n| progreso(Etapa::Codigos(i as u32, n as u32)),
        )?;
        let compas = match p.compas.trim() { "6" => "6/8".to_string(), "" => "?".to_string(), c if c.contains('/') => c.to_string(), c => format!("{c}/4") };
        plan_lm = Some(format!("{} bpm · {} · {compas}", p.bpm.map(|b| b.to_string()).unwrap_or_else(|| "?".into()), if p.tonalidad.is_empty() { "?" } else { &p.tonalidad }));
        drop(lm);
        let _ = mlx_rs::memory::clear_cache();
        p.codigos = Some(codigos);
    }
    // M7d: el audio propio (cover/repaint): estéreo a 48 kHz → (relleno del repaint) → VAE por
    // tramos con su muestreo → latentes objetivo
    let mut fuente_onda: Option<[Vec<f32>; 2]> = None;
    let mut objetivo: Option<Array> = None;
    let mut region: Option<(f64, f64)> = None;
    if let Some(f) = p.fuente.clone() {
        progreso(Etapa::Fuente);
        let mut onda = crate::musica_fuente::fuente(&f.ruta)?;
        let dur = onda[0].len() as f64 / FRECUENCIA as f64;
        if !(1.0..=600.0).contains(&dur) {
            bail_tr!("el audio fuente dura {dur:.1} s (se admite de 1 s a 10 min)", "the source audio lasts {dur:.1} s (1 s to 10 min is accepted)");
        }
        // los metadatos dicen la duración del audio tal cual (`int(segundos)`)
        p.segundos = dur as f32;
        if let Some((desde, hasta)) = f.repintar {
            let (desde, fin) = (desde as f64, if hasta < 0.0 { dur } else { hasta as f64 });
            if fin <= desde {
                bail_tr!("repintar: el final ({fin:.1} s) debe ir después del inicio ({desde:.1} s)", "repaint: the end ({fin:.1} s) must come after the start ({desde:.1} s)");
            }
            // `prepare_padding_info`: silencio delante si empieza antes de 0, detrás si acaba después
            let (izq, der) = ((-desde).max(0.0), (fin - dur).max(0.0));
            let (ni, nd) = ((izq * FRECUENCIA as f64) as usize, (der * FRECUENCIA as f64) as usize);
            for c in onda.iter_mut() {
                let mut v = vec![0f32; ni];
                v.extend_from_slice(c);
                v.resize(v.len() + nd, 0.0);
                *c = v;
            }
            region = Some((desde + izq, fin + izq));
        }
        let z = match &f.latentes {
            Some(r) => Array::load_safetensors(r).map_err(|e| anyhow_tr!("latentes: {e:?}", "latents: {e:?}"))?.remove("z").ok_or_else(|| anyhow_tr!("el archivo no trae «z»", "the file has no «z»"))?.as_dtype(F32).m()?,
            None => {
                let cod = crate::musica_fuente::CodificadorVae::cargar(&dir.join("vae"))?;
                let semilla = p.semilla;
                let z = cod.codificar(
                    &onda,
                    30 * FRECUENCIA as usize,
                    2 * FRECUENCIA as usize,
                    &mut |i, forma| mlx_rs::random::normal::<f32>(forma, None, None, &mlx_rs::random::key(semilla.wrapping_add(0x5EED + i as u64)).m()?).m(),
                    Some(cancelar),
                )?;
                drop(cod);
                let _ = mlx_rs::memory::clear_cache();
                z
            }
        };
        guardar("fuente_z", &z)?;
        objetivo = Some(z);
        fuente_onda = Some(onda);
        corte()?;
    }
    let p = &p;
    let cover = p.codigos.is_some();
    // una tarea del base usa el audio TAL CUAL de contexto (no lo tokeniza como un cover)
    let cover_fuente = objetivo.is_some() && region.is_none() && p.tarea.is_none();
    let t_lat = match (&objetivo, &p.codigos) {
        (Some(z), _) => z.dim(1).max(128),
        (None, Some(c)) => (c.len() * 5).max(128) as i32,
        _ => cuadros(p.segundos) as i32,
    };

    // 1. texto y letra
    progreso(Etapa::Texto);
    let edir = dir.join("Qwen3-Embedding-0.6B");
    let tk = tokenizers::Tokenizer::from_file(edir.join("tokenizer.json")).map_err(|e| anyhow!("tokenizer: {e}"))?;
    let ids_t = tokens(&tk, &texto_sft(p, cover), 256)?;
    let ids_l = tokens(&tk, &texto_letra(p), 2048)?;
    let emb = Modelo::cargar(&edir)?;
    let texto = emb.estado_final(&ids_t)?;
    let letra = emb.filas_embedding(&ids_l)?;
    mlx_rs::transforms::eval([&texto, &letra]).m()?;
    drop(emb);
    let _ = mlx_rs::memory::clear_cache();
    guardar("texto_ids", &Array::from_slice(&ids_t.iter().map(|&i| i as i32).collect::<Vec<_>>(), &[1, ids_t.len() as i32]))?;
    guardar("letra_ids", &Array::from_slice(&ids_l.iter().map(|&i| i as i32).collect::<Vec<_>>(), &[1, ids_l.len() as i32]))?;
    guardar("texto_emb", &texto)?;
    guardar("letra_emb", &letra)?;
    corte()?;

    // 2. condición: silencio de referencia (timbre: 750 cuadros; fuente: T cuadros) y máscara de unos
    // Ola PRO: el DiT base si se pidió (misma arquitectura; su `silence_latent` al lado)
    let tdir = p.base.clone().unwrap_or_else(|| dir.join("acestep-v15-turbo"));
    // Ola PRO · la referencia de timbre y mezcla (`reference_audio`): 30 s en 3 trozos → el MISMO
    // codificador del VAE que el cover → 750 cuadros. Se codifica ANTES de cargar el DiT (un modelo a la vez)
    let timbre_ref = match &p.timbre {
        Some(r) => {
            progreso(Etapa::Fuente);
            let onda = crate::musica_fuente::fuente(r)?;
            match segmentos_referencia(&onda, p.semilla) {
                Some(trozos) => {
                    let cod = crate::musica_fuente::CodificadorVae::cargar(&dir.join("vae"))?;
                    let semilla = p.semilla;
                    let z = cod.codificar(
                        &trozos,
                        30 * FRECUENCIA as usize,
                        2 * FRECUENCIA as usize,
                        &mut |i, forma| mlx_rs::random::normal::<f32>(forma, None, None, &mlx_rs::random::key(semilla.wrapping_add(0x71B2E + i as u64)).m()?).m(),
                        Some(cancelar),
                    )?;
                    drop(cod);
                    let _ = mlx_rs::memory::clear_cache();
                    guardar("timbre_z", &z)?;
                    Some(z)
                }
                // silencio: como el oficial, el latente de silencio
                None => None,
            }
        }
        None => None,
    };
    corte()?;
    let silencio = silencio(&tdir.join("silence_latent.safetensors"), &tdir.join("silence_latent.pt"))?;
    let modelo = AceStep::cargar(&tdir)?;
    let timbre = match &timbre_ref {
        Some(z) if z.dim(1) >= 750 => z.try_index((.., ..750, ..)).m()?,
        // por si el codificador diera menos cuadros: se completa con silencio
        Some(z) => ops::concatenate(&[z.clone(), silencio.try_index((.., ..750 - z.dim(1), ..)).m()?], 1).m()?,
        None => silencio.try_index((.., ..750, ..)).m()?,
    };
    let (enc, cond) = modelo.condicion(&texto, &letra, &timbre)?;
    // Ola PRO · base: la condición NULA para el CFG (APG), si la guía pasa de 1
    let guia_dit = if p.guia_dit > 0.0 { p.guia_dit } else { 7.0 };
    let cond_nula = if !modelo.turbo && guia_dit > 1.0 { Some(modelo.condicion_nula(&enc)?) } else { None };
    let mut momento: Option<Array> = None;
    guardar("enc", &enc)?;
    // con códigos: la fuente son las pistas del LM, rellenas con el silencio o recortadas a T
    // (`_prepare_precomputed_lm_hints` + `is_covers`)
    // M7d: los latentes del audio propio, rellenos con el silencio hasta T
    let objetivo_t = match &objetivo {
        Some(z) if z.dim(1) < t_lat => Some(ops::concatenate(&[z.clone(), silencio.try_index((.., ..t_lat - z.dim(1), ..)).m()?], 1).m()?),
        Some(z) => Some(z.clone()),
        None => None,
    };
    // repaint: la región en cuadros (`int(s·48000 // 1920)`, acotada como el oficial)
    let cuadros_rep = region.map(|(a, b)| {
        let s0 = ((a * FRECUENCIA as f64 / 1920.0).floor() as i32).clamp(0, t_lat - 1);
        let s1 = ((b * FRECUENCIA as f64 / 1920.0).floor() as i32).min(t_lat).max(s0 + 1);
        (s0, s1)
    });
    let fuente = match (&objetivo_t, &p.codigos) {
        // Ola PRO · tarea del base: el audio codificado, entero, de contexto (máscara completa)
        (Some(zt), _) if p.tarea.is_some() => zt.clone(),
        // cover: las pistas que salen de TOKENIZAR el propio audio (`is_covers`)
        (Some(zt), _) if cuadros_rep.is_none() => {
            let (pistas, codigos) = modelo.pistas_de_latentes(zt, &silencio)?;
            guardar("fuente_codigos", &Array::from_slice(&codigos.iter().map(|&c| c as i32).collect::<Vec<_>>(), &[codigos.len() as i32]))?;
            guardar("pistas", &pistas)?;
            pistas
        }
        // repaint: el audio con la región a repintar cambiada por el silencio
        (Some(zt), _) => {
            let (s0, s1) = cuadros_rep.unwrap_or((0, t_lat));
            ops::concatenate(&[zt.try_index((.., ..s0, ..)).m()?, silencio.try_index((.., s0..s1, ..)).m()?, zt.try_index((.., s1.., ..)).m()?], 1).m()?
        }
        (None, Some(c)) => {
            let pistas = modelo.pistas(c)?;
            guardar("pistas", &pistas)?;
            let n = pistas.dim(1);
            if n < t_lat {
                ops::concatenate(&[pistas, silencio.try_index((.., ..t_lat - n, ..)).m()?], 1).m()?
            } else {
                pistas.try_index((.., ..t_lat, ..)).m()?
            }
        }
        _ => silencio.try_index((.., ..t_lat, ..)).m()?,
    };
    // la máscara de trozos: 1 = «genera esto»; en las tareas del base, 2 = el modo «auto» del oficial
    // (`chunk_mask_mode = "auto"`: «all 2.0, model decides»). Medido 2026-11-13: con 1 y la canción de
    // contexto, «extraer la voz» y «extraer la batería» devolvían la MISMA canción (correlación 1,000)
    let valor_mascara = if p.tarea.is_some() { 2.0f32 } else { 1.0 };
    let ctx = ops::concatenate(&[fuente, ops::multiply(&ops::ones::<f32>(&[1, t_lat, 64]).m()?, Array::from_f32(valor_mascara)).m()?], -1).m()?;
    // cover con `audio_cover_strength` < 1: los últimos pasos van SIN cover (fuente de silencio; el
    // texto es el mismo: también lleva la instrucción por defecto)
    let ctx_sin_cover = ops::concatenate(&[silencio.try_index((.., ..t_lat, ..)).m()?, ops::ones::<f32>(&[1, t_lat, 64]).m()?], -1).m()?;

    // 3. ODE turbo + DCW
    let mut xt = match ruido {
        Some(r) => Array::load_safetensors(r).map_err(|e| anyhow_tr!("ruido: {e:?}", "noise: {e:?}"))?.remove("ruido").ok_or_else(|| anyhow_tr!("el archivo no trae «ruido»", "the file has no «ruido»"))?.as_dtype(F32).m()?,
        // en una tarea del base el ruido NO sale de la misma semilla que la pieza: medido 2026-11-13, la
        // canción hecha con la semilla 7 y la extracción con la 7 arrancaban de un ruido 47 % correlado
        // con el audio y el resultado se quedaba pegado a él (0,997; el oficial, con su propio azar, no)
        None => {
            let s = if p.tarea.is_some() { p.semilla ^ 0x7A3E_51D0_C0DE_0001 } else { p.semilla };
            mlx_rs::random::normal::<f32>(&[1, t_lat, 64], None, None, &mlx_rs::random::key(s).m()?).m()?
        }
    };
    if xt.shape() != [1, t_lat, 64] {
        bail!("ruido de forma {:?} (se esperaba [1, {t_lat}, 64])", xt.shape());
    }
    guardar("ruido", &xt)?;
    let ruido0 = xt.clone();
    // el turbo: su tabla destilada; el base: `linspace` + shift, 50 pasos por omisión (el oficial)
    let mut ts = if modelo.turbo { horario(p.shift)? } else { horario_base(if p.pasos_base == 0 { 50 } else { p.pasos_base.min(200) }, p.shift as f32) };
    let f = p.fuente.clone();
    // cover con `cover_noise_strength` > 0: se arranca del audio re-ruidoso en el paso más cercano
    // a 1 − fuerza (`renoise`) y se saltan los pasos anteriores
    if let (true, Some(f), Some(zt)) = (cover_fuente, &f, &objetivo_t) {
        if f.ruido_cover > 0.0 {
            let nivel = 1.0 - f.ruido_cover as f64;
            let k = (0..ts.len()).fold(0, |m, i| if (ts[i] - nivel).abs() < (ts[m] - nivel).abs() { i } else { m });
            let tk = Array::from_f32(ts[k] as f32);
            xt = ops::add(&ops::multiply(&tk, &ruido0).m()?, &ops::multiply(&ops::subtract(Array::from_f32(1.0), &tk).m()?, zt).m()?).m()?;
            ts = ts[k..].to_vec();
        }
    }
    let n = ts.len();
    let pasos_cover = match (&f, cover_fuente) {
        (Some(f), true) => (n as f64 * f.fuerza_cover as f64) as usize,
        _ => n,
    };
    // repaint en modo «balanced»: inyección del audio re-ruidoso fuera de la región en los primeros
    // round(r·n) pasos y mezcla de borde de round(25·r) cuadros, r = 1 − fuerza
    let r_rep = f.as_ref().map(|f| 1.0 - f.fuerza_repintar.clamp(0.0, 1.0) as f64).unwrap_or(0.0);
    let corte_inyeccion = crate::musica_fuente::redondeo_par(r_rep * n as f64) as usize;
    let mascara_rep = match cuadros_rep {
        Some((s0, s1)) => {
            let v: Vec<f32> = (0..t_lat).map(|i| if i >= s0 && i < s1 { 1.0 } else { 0.0 }).collect();
            Some(Array::from_slice(&v, &[1, t_lat, 1]))
        }
        None => None,
    };
    for i in 0..n {
        progreso(Etapa::Paso(i as u32 + 1, n as u32));
        let t = ts[i];
        let ctx_i = if i >= pasos_cover { &ctx_sin_cover } else { &ctx };
        let mut v = modelo.dit(&xt, ctx_i, t as f32, &cond, Some(cancelar))?;
        if let Some(cn) = &cond_nula {
            let vn = modelo.dit(&xt, ctx_i, t as f32, cn, Some(cancelar))?;
            v = apg(&v, &vn, guia_dit, &mut momento)?;
        }
        guardar(&format!("v_{i}"), &v)?;
        let tt = Array::from_f32(t as f32);
        if i == n - 1 {
            // x0 = xt − v·t (aquí DCW no cambia nada: x0 y la predicción limpia son lo mismo)
            xt = ops::subtract(&xt, &ops::multiply(&v, &tt).m()?).m()?;
        } else {
            let limpio = ops::subtract(&xt, &ops::multiply(&v, &tt).m()?).m()?;
            let dt = Array::from_f32((t - ts[i + 1]) as f32);
            let x = ops::subtract(&xt, &ops::multiply(&v, &dt).m()?).m()?;
            // DCW: por omisión sólo en el turbo (`dcw_enabled`, docs/en/DCW.md); el base, Euler tal cual
            xt = if modelo.turbo { dcw_doble(&x, &limpio, (t * 0.05) as f32, ((1.0 - t) * 0.02) as f32)? } else { x };
            // `_repaint_step_injection`: fuera de la región, el audio re-ruidoso al nivel siguiente
            if let (Some(m), Some(zt)) = (&mascara_rep, &objetivo_t) {
                if i < corte_inyeccion {
                    let tn = Array::from_f32(ts[i + 1] as f32);
                    let zr = ops::add(&ops::multiply(&tn, &ruido0).m()?, &ops::multiply(&ops::subtract(Array::from_f32(1.0), &tn).m()?, zt).m()?).m()?;
                    xt = ops::add(&ops::multiply(m, &xt).m()?, &ops::multiply(&ops::subtract(Array::from_f32(1.0), m).m()?, &zr).m()?).m()?;
                }
            }
        }
        xt.eval().m()?;
        guardar(&format!("x_{i}"), &xt)?;
        corte()?;
    }
    // `_repaint_boundary_blend`: mezcla suave con el audio en los bordes de la región
    let cf = crate::musica_fuente::redondeo_par(25.0 * r_rep) as i32;
    if let (Some((s0, s1)), Some(zt), true) = (cuadros_rep, &objetivo_t, cf > 0) {
        let mut suave: Vec<f32> = (0..t_lat).map(|i| if i >= s0 && i < s1 { 1.0 } else { 0.0 }).collect();
        if !(s0 == 0 && s1 == t_lat) {
            let fs = (s0 - cf).max(0);
            if s0 - fs > 0 {
                let l = linspace_torch(0.0, 1.0, (s0 - fs + 2) as usize);
                suave[fs as usize..s0 as usize].copy_from_slice(&l[1..l.len() - 1]);
            }
            let fe = (s1 + cf).min(t_lat);
            if fe - s1 > 0 {
                let l = linspace_torch(1.0, 0.0, (fe - s1 + 2) as usize);
                suave[s1 as usize..fe as usize].copy_from_slice(&l[1..l.len() - 1]);
            }
        }
        let m = Array::from_slice(&suave, &[1, t_lat, 1]);
        xt = ops::add(&ops::multiply(&m, &xt).m()?, &ops::multiply(&ops::subtract(Array::from_f32(1.0), &m).m()?, zt).m()?).m()?;
        xt.eval().m()?;
        guardar("x_mezcla", &xt)?;
    }
    drop(cond);
    drop(modelo);
    let _ = mlx_rs::memory::clear_cache();

    // 4. audio
    progreso(Etapa::Audio);
    let vae = Vae::cargar(&dir.join("vae"))?;
    let onda = vae.decodificar(&xt, Some(cancelar))?;
    onda.eval().m()?;
    guardar("audio_crudo", &onda)?;
    // como el oficial: si el pico pasa de 1 se divide por él; luego, a −1 dBFS (`normalize_audio`)
    let pico = ops::max(&ops::abs(&onda).m()?, None).m()?.item_exact::<f32>();
    let mut onda = if pico > 1.0 { ops::divide(&onda, Array::from_f32(pico)).m()? } else { onda };
    // repaint: fuera de la región vuelve el audio ORIGINAL (sin el error del VAE), con un fundido de
    // 0,05·r s en los bordes (`apply_repaint_waveform_splice`)
    if let (Some((a, b)), Some(src)) = (region, &fuente_onda) {
        onda.eval().m()?;
        let mut v = onda.as_slice::<f32>().to_vec();
        let (n_pred, n_src) = (v.len() / 2, src[0].len());
        let minimo = n_pred.min(n_src);
        let ini = ((a * FRECUENCIA as f64) as usize).min(minimo);
        let fin = ((b * FRECUENCIA as f64) as usize).min(minimo).max(ini);
        if !(ini == 0 && fin >= minimo) {
            let cf = ((0.05 * r_rep) * FRECUENCIA as f64) as usize;
            let mut m = vec![0f32; minimo];
            m[ini..fin].iter_mut().for_each(|x| *x = 1.0);
            if cf > 0 {
                let fs = ini.saturating_sub(cf);
                if ini > fs {
                    let l = linspace_torch(0.0, 1.0, ini - fs + 2);
                    m[fs..ini].copy_from_slice(&l[1..l.len() - 1]);
                }
                let fe = (fin + cf).min(minimo);
                if fe > fin {
                    let l = linspace_torch(1.0, 0.0, fe - fin + 2);
                    m[fin..fe].copy_from_slice(&l[1..l.len() - 1]);
                }
            }
            for (i, &k) in m.iter().enumerate() {
                for c in 0..2 {
                    v[2 * i + c] = k * v[2 * i + c] + (1.0 - k) * src[c][i];
                }
            }
        }
        onda = Array::from_slice(&v, &[1, n_pred as i32, 2]);
    }
    let pico2 = ops::max(&ops::abs(&onda).m()?, None).m()?.item_exact::<f32>();
    if pico2 >= 1e-6 {
        onda = ops::multiply(&onda, Array::from_f32(10f32.powf(-1.0 / 20.0) / pico2)).m()?;
    }
    onda.eval().m()?;
    let muestras = onda.as_slice::<f32>().to_vec();
    Ok(Audio { muestras, canales: 2, frecuencia: FRECUENCIA, plan: plan_lm })
}

/// El audio propio tal como llega del núcleo: (ruta, repintar desde–hasta, fuerza del cover).
pub type FuenteMotor<'a> = Option<(&'a Path, Option<(f32, f32)>, f32)>;

/// `torch.linspace` en f32 (la fórmula de dos mitades de su CPU).
pub(crate) fn linspace_torch(a: f32, b: f32, n: usize) -> Vec<f32> {
    if n == 1 {
        return vec![a];
    }
    let paso = (b - a) / (n as f32 - 1.0);
    (0..n).map(|i| if i < n / 2 { a + paso * i as f32 } else { b - paso * (n - 1 - i) as f32 }).collect()
}

/// El motor que registran la CLI y la app en `gotv_core::generadores` (firma sin tipos de MLX):
/// escribe el WAV en `salida` y devuelve los segundos de audio.
#[allow(clippy::too_many_arguments)]
pub fn generar_wav(
    dir: &Path,
    descripcion: &str,
    letra: &str,
    idioma: &str,
    segundos: f32,
    bpm: Option<u32>,
    tonalidad: &str,
    compas: &str,
    semilla: u64,
    fuente: FuenteMotor,
    opciones: &serde_json::Value,
    salida: &Path,
    progreso: &(dyn Fn(String) + Sync),
    cancelar: &AtomicBool,
) -> Result<(f32, Option<String>)> {
    let instrumental = letra.trim().is_empty();
    // Ola PRO: lo que los pesos permiten tocar (todo opcional; lo que falta, el valor oficial)
    let num = |k: &str| opciones.get(k).and_then(|v| v.as_f64());
    let defecto = crate::musica_lm::Muestreo::default();
    let muestreo = crate::musica_lm::Muestreo {
        temperatura: num("temperatura").map_or(defecto.temperatura, |v| v.clamp(0.0, 2.0) as f32),
        cfg: num("cfg_lm").map_or(defecto.cfg, |v| v.clamp(1.0, 5.0) as f32),
        top_p: num("top_p").map_or(defecto.top_p, |v| v.clamp(0.1, 1.0) as f32),
        top_k: num("top_k").map_or(0, |v| v.clamp(0.0, 1000.0) as usize),
        negativo: opciones.get("excluir").and_then(|v| v.as_str()).map(str::trim).filter(|s| !s.is_empty()).map_or(defecto.negativo.clone(), str::to_string),
    };
    let shift = num("shift").map_or(1, |v| v.clamp(1.0, 3.0).round() as u32);
    let pensar = opciones.get("pensar").and_then(|v| v.as_bool()).unwrap_or(true);
    let fuente = fuente.map(|(ruta, repintar, fuerza)| Fuente {
        repintar,
        fuerza_cover: fuerza.clamp(0.0, 1.0),
        ruido_cover: num("ruido_cover").map_or(0.0, |v| v.clamp(0.0, 0.99) as f32),
        fuerza_repintar: num("fuerza_repintar").map_or(0.5, |v| v.clamp(0.0, 1.0) as f32),
        ..Fuente::cover(ruta.to_path_buf())
    });
    let p = Pedido {
        descripcion: descripcion.to_string(),
        letra: letra.to_string(),
        idioma: if instrumental { "unknown".into() } else if idioma.trim().is_empty() || idioma == "unknown" { "en".into() } else { idioma.to_string() },
        segundos,
        bpm,
        tonalidad: tonalidad.to_string(),
        compas: compas.to_string(),
        semilla,
        shift,
        pensar,
        codigos: None,
        muestreo,
        timbre: opciones.get("timbre").and_then(|v| v.as_str()).map(str::trim).filter(|s| !s.is_empty()).map(std::path::PathBuf::from),
        // Ola PRO · el DiT base y su tarea: {"base": carpeta, "tarea": {"tipo": extraer|lego|completar, "pistas": [...]}}
        base: opciones.get("base").and_then(|v| v.as_str()).map(std::path::PathBuf::from),
        lm: opciones.get("lm").and_then(|v| v.as_str()).map(std::path::PathBuf::from),
        tarea: tarea_de(opciones),
        pasos_base: num("pasos_base").map_or(0, |v| v.clamp(1.0, 200.0) as u32),
        guia_dit: num("guia_dit").map_or(0.0, |v| v.clamp(1.0, 15.0) as f32),
        fuente,
    };
    let audio = generar(
        dir,
        &p,
        &|e| {
            progreso(match e {
                Etapa::Plan => "plan".into(),
                Etapa::Fuente => "fuente".into(),
                Etapa::Codigos(i, n) => tr!("códigos {i}/{n}", "codes {i}/{n}"),
                Etapa::Texto => "texto".into(),
                Etapa::Paso(i, n) => tr!("paso {i}/{n}", "step {i}/{n}"),
                Etapa::Audio => "audio".into(),
            })
        },
        cancelar,
        None,
        None,
    )?;
    std::fs::write(salida, audio.wav()).with_context(|| format!("guardar {}", salida.display()))?;
    Ok((audio.segundos(), audio.plan))
}

/// El latente de silencio [1, 15000, 64]: el `.pt` oficial es un zip de PyTorch; GOTV lee su
/// copia en safetensors si existe y, si no, los datos crudos del zip (f32, [1, 64, 15000]).
fn silencio(st: &Path, pt: &Path) -> Result<Array> {
    if st.is_file() {
        let mut m = Array::load_safetensors(st).map_err(|e| anyhow!("silencio: {e:?}"))?;
        return m.remove("silencio").ok_or_else(|| anyhow_tr!("silence_latent.safetensors sin «silencio»", "silence_latent.safetensors without «silencio»"));
    }
    let datos = leer_pt_f32(pt)?;
    if datos.len() != 64 * 15000 {
        bail_tr!("silence_latent.pt con {} valores (se esperaban 64·15000)", "silence_latent.pt with {} values (64·15000 expected)", datos.len());
    }
    let a = Array::from_slice(&datos, &[1, 64, 15000]);
    ops::transpose_axes(&a, &[0, 2, 1]).m()
}

/// El único tensor de un `.pt` de PyTorch (un zip con `*/data/0` en crudo, little-endian).
fn leer_pt_f32(pt: &Path) -> Result<Vec<f32>> {
    let b = std::fs::read(pt).with_context(|| format!("leer {}", pt.display()))?;
    // torch escribe las entradas SIN compresión pero con «descriptor de datos» (bandera 0x08): en el
    // encabezado local el tamaño vale 0 y el real está en el DIRECTORIO CENTRAL (medido con unzip -lv)
    let le16 = |o: usize| -> Result<usize> { b.get(o..o + 2).map(|s| u16::from_le_bytes([s[0], s[1]]) as usize).ok_or_else(|| anyhow!("zip corto")) };
    let le32 = |o: usize| -> Result<usize> { b.get(o..o + 4).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]) as usize).ok_or_else(|| anyhow!("zip corto")) };
    let fin = (0..b.len().saturating_sub(21)).rev().find(|&i| b[i..i + 4] == [0x50, 0x4b, 0x05, 0x06]).ok_or_else(|| anyhow!("{}: no es un zip", pt.display()))?;
    let (entradas, mut c) = (le16(fin + 10)?, le32(fin + 16)?);
    for _ in 0..entradas {
        if b.get(c..c + 4) != Some(&[0x50, 0x4b, 0x01, 0x02]) {
            bail_tr!("{}: directorio central dañado", "{}: damaged central directory", pt.display());
        }
        let (metodo, tam, nl, el, cl, local) = (le16(c + 10)?, le32(c + 20)?, le16(c + 28)?, le16(c + 30)?, le16(c + 32)?, le32(c + 42)?);
        let nombre = String::from_utf8_lossy(&b[c + 46..c + 46 + nl]).to_string();
        if nombre.ends_with("/data/0") {
            if metodo != 0 {
                bail_tr!("silence_latent.pt comprimido (método {metodo})", "silence_latent.pt compressed (method {metodo})");
            }
            let ini = local + 30 + le16(local + 26)? + le16(local + 28)?;
            let d = b.get(ini..ini + tam).ok_or_else(|| anyhow!("zip corto"))?;
            return Ok(d.as_chunks::<4>().0.iter().map(|x| f32::from_le_bytes(*x)).collect());
        }
        c += 46 + nl + el + cl;
    }
    bail_tr!("no encontré los datos del tensor en {}", "could not find the tensor data in {}", pt.display())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Diagnóstico: ¿el ruido del DiT (`key(semilla)`) se parece al del codificador del audio
    /// (`key(semilla + 0x5EED)`)? Medido 2026-11-13: el ruido volcado salía 47 % correlado con la canción.
    #[test]
    #[ignore]
    fn ruidos_de_semillas_distintas() {
        let a = mlx_rs::random::normal::<f32>(&[1, 500, 64], None, None, &mlx_rs::random::key(7).unwrap()).unwrap();
        let b = mlx_rs::random::normal::<f32>(&[1, 500, 64], None, None, &mlx_rs::random::key(7 + 0x5EED).unwrap()).unwrap();
        let (a, b) = (a.as_slice::<f32>().to_vec(), b.as_slice::<f32>().to_vec());
        let n = a.len() as f64;
        let (ma, mb) = (a.iter().map(|x| *x as f64).sum::<f64>() / n, b.iter().map(|x| *x as f64).sum::<f64>() / n);
        let cov: f64 = a.iter().zip(&b).map(|(x, y)| (*x as f64 - ma) * (*y as f64 - mb)).sum::<f64>() / n;
        let sa = (a.iter().map(|x| (*x as f64 - ma).powi(2)).sum::<f64>() / n).sqrt();
        let sb = (b.iter().map(|y| (*y as f64 - mb).powi(2)).sum::<f64>() / n).sqrt();
        println!("correlación key(7) vs key(7+0x5EED): {:.4}; primeros {:?} {:?}", cov / (sa * sb), &a[..3], &b[..3]);
    }

    /// Ola PRO · el DiT base: las instrucciones de sus tareas (`TASK_INSTRUCTIONS`, « | » en complete),
    /// sus pasos (`linspace` + shift) y APG (sin guía = la condicionada; ortogonal a ella; momento).
    #[test]
    fn base_instrucciones_horario_y_apg() {
        let t = |j: serde_json::Value| tarea_de(&serde_json::json!({ "tarea": j }));
        assert_eq!(t(serde_json::json!({"tipo": "extraer", "pistas": ["Vocals"]})).unwrap().instruccion(), "Extract the VOCALS track from the audio:");
        assert_eq!(t(serde_json::json!({"tipo": "lego", "pistas": ["guitar"]})).unwrap().instruccion(), "Generate the GUITAR track based on the audio context:");
        assert_eq!(t(serde_json::json!({"tipo": "completar", "pistas": ["drums", "backing vocals", "kazoo"]})).unwrap().instruccion(), "Complete the input track with DRUMS | BACKING_VOCALS:");
        assert_eq!(t(serde_json::json!({"tipo": "completar", "pistas": []})).unwrap().instruccion(), "Complete the input track:");
        assert!(t(serde_json::json!({"tipo": "extraer", "pistas": ["kazoo"]})).is_none(), "una pista que el modelo no conoce");
        assert!(tarea_de(&serde_json::json!({})).is_none());
        // linspace(1, 0, 4) y con shift 3: t' = 3t / (1 + 2t)
        assert_eq!(horario_base(4, 1.0), vec![1.0, 0.75, 0.5, 0.25], "4 pasos, sin el 0 final");
        let s = horario_base(2, 3.0);
        assert!((s[0] - 1.0).abs() < 1e-12 && (s[1] - 0.75).abs() < 1e-12 && s.len() == 2, "{s:?}");
        // APG: con guía 1 queda la condicionada; la corrección es ORTOGONAL a ella (por el eje del tiempo)
        let c = Array::from_slice(&[1.0f32, 0.0, 0.0, 1.0], &[1, 2, 2]);
        let u = Array::from_slice(&[0.5f32, 0.5, 0.0, 0.0], &[1, 2, 2]);
        let mut m = None;
        let g1 = apg(&c, &u, 1.0, &mut m).unwrap();
        assert_eq!(g1.as_slice::<f32>(), c.as_slice::<f32>());
        let mut m = None;
        let g = apg(&c, &u, 3.0, &mut m).unwrap();
        let corr: Vec<f32> = g.as_slice::<f32>().iter().zip(c.as_slice::<f32>()).map(|(a, b)| a - b).collect();
        // por canal, la corrección (eje 1) es perpendicular a la condicionada: Σ corr·c = 0
        for canal in 0..2 {
            let punto: f32 = (0..2).map(|t| corr[t * 2 + canal] * c.as_slice::<f32>()[t * 2 + canal]).sum();
            assert!(punto.abs() < 1e-6, "canal {canal}: {punto}");
        }
        assert!(m.is_some(), "el momento queda para el paso siguiente");
    }

    /// Ola PRO: la referencia de timbre son 30 s (3 trozos de 10 s, uno por tercio); una corta se
    /// repite; el silencio no cuenta; la misma semilla da los mismos cortes.
    #[test]
    fn la_referencia_son_tres_trozos_de_diez_segundos() {
        let f = FRECUENCIA as usize;
        // 12 s: cada muestra lleva su segundo (para saber de qué tercio sale cada trozo)
        let onda: [Vec<f32>; 2] = [(0..12 * f).map(|i| (i / f) as f32 + 1.0).collect(), (0..12 * f).map(|i| -((i / f) as f32 + 1.0)).collect()];
        let r = segmentos_referencia(&onda, 3).unwrap();
        assert_eq!((r[0].len(), r[1].len()), (30 * f, 30 * f));
        assert_eq!(r[1][5], -r[0][5], "los dos canales se cortan igual");
        // repetida a 36 s: tercios de 12 s; el primer trozo cae en el primer tercio, etc.
        assert!(r[0][0] >= 1.0 && r[0][0] <= 3.0, "el primero empieza en los 2 s primeros del tercio: {}", r[0][0]);
        assert_eq!(segmentos_referencia(&onda, 3).unwrap()[0], r[0], "repetible");
        assert_ne!(segmentos_referencia(&onda, 4).unwrap()[0], r[0], "otra semilla, otros cortes");
        assert!(segmentos_referencia(&[vec![0.0; f], vec![0.0; f]], 1).is_none(), "el silencio no es referencia");
        // una larga (2 min): 30 s igual
        let larga: [Vec<f32>; 2] = [vec![0.1; 120 * f], vec![0.1; 120 * f]];
        assert_eq!(segmentos_referencia(&larga, 9).unwrap()[0].len(), 30 * f);
    }

    #[test]
    fn las_cadenas_son_las_oficiales() {
        // las que capturó el juez del manejador oficial (2026-11-12)
        let p = Pedido { descripcion: "calm lo-fi hip hop beat with warm piano and soft drums".into(), segundos: 10.0, ..Default::default() };
        assert_eq!(
            texto_sft(&p, false),
            "# Instruction\nFill the audio semantic mask based on the given conditions:\n\n# Caption\ncalm lo-fi hip hop beat with warm piano and soft drums\n\n# Metas\n- bpm: N/A\n- timesignature: N/A\n- keyscale: N/A\n- duration: 10 seconds\n<|endoftext|>\n"
        );
        assert_eq!(texto_letra(&p), "# Languages\nunknown\n\n# Lyric\n[Instrumental]<|endoftext|>");
        let q = Pedido { letra: "hola mundo".into(), idioma: "es".into(), bpm: Some(90), tonalidad: "A minor".into(), compas: "4".into(), segundos: 30.5, ..p };
        assert!(texto_sft(&q, false).contains("- bpm: 90\n- timesignature: 4\n- keyscale: A minor\n- duration: 30 seconds\n"));
        assert_eq!(texto_letra(&q), "# Languages\nes\n\n# Lyric\nhola mundo<|endoftext|>");
    }

    #[test]
    fn las_etiquetas_traducidas_vuelven_a_las_oficiales() {
        // las que escribió el 9B (2026-11-12): [verso], [coro], [ponte]
        let l = "[verso]\n¡Feliz cumpleaños, Ana!\n\n[coro]\nHaz un deseo\n[ponte]\nQue tu año\n[Estribillo 2]\nOtra vez\n[outro]\nFin";
        assert_eq!(etiquetas_oficiales(l), "[verse]\n¡Feliz cumpleaños, Ana!\n\n[chorus]\nHaz un deseo\n[bridge]\nQue tu año\n[chorus 2]\nOtra vez\n[outro]\nFin");
        // lo que ya es oficial no se toca (ni la mayúscula), ni [Instrumental], ni corchetes de texto
        let o = "[Verse]\nhola\n[Instrumental]\n[a mi manera] es una canción";
        assert_eq!(etiquetas_oficiales(o), o);
        let p = Pedido { letra: "[coro]\nla la".into(), idioma: "es".into(), ..Default::default() };
        assert_eq!(texto_letra(&p), "# Languages\nes\n\n# Lyric\n[chorus]\nla la<|endoftext|>");
    }

    #[test]
    fn cuadros_como_el_manejador() {
        assert_eq!(cuadros(10.0), 250);
        assert_eq!(cuadros(30.0), 750);
        assert_eq!(cuadros(3.0), 128); // mínimo de 128 cuadros
        assert_eq!(cuadros(10.03), 250);
    }

    #[test]
    fn dcw_en_forma_cerrada() {
        // contra pytorch_wavelets (DWT1D J=1 haar, cero): x = 1..6 y 1..7, y = 0
        for n in [6, 7] {
            let v: Vec<f32> = (1..=n).map(|i| i as f32).collect();
            let x = Array::from_slice(&v, &[1, n, 1]);
            let y = ops::zeros::<f32>(&[1, n, 1]).unwrap();
            // a = 1, b = 0 → x + P_L(x); a = 0, b = 1 → x + P_H(x)
            let l = dcw_doble(&x, &y, 1.0, 0.0).unwrap();
            let h = dcw_doble(&x, &y, 0.0, 1.0).unwrap();
            let pl: Vec<f32> = l.as_slice::<f32>().iter().zip(&v).map(|(a, b)| a - b).collect();
            let ph: Vec<f32> = h.as_slice::<f32>().iter().zip(&v).map(|(a, b)| a - b).collect();
            let esp_l = [1.5, 1.5, 3.5, 3.5, 5.5, 5.5, 3.5];
            let esp_h = [-0.5, 0.5, -0.5, 0.5, -0.5, 0.5, 3.5];
            assert_eq!(pl, esp_l[..n as usize].to_vec(), "P_L n={n}");
            assert_eq!(ph, esp_h[..n as usize].to_vec(), "P_H n={n}");
        }
    }

    #[test]
    fn el_wav_es_valido() {
        let a = Audio { muestras: vec![0.0, 0.5, -0.5, 1.0], canales: 2, frecuencia: 48000, plan: None };
        let w = a.wav();
        assert_eq!(&w[..4], b"RIFF");
        assert_eq!(w.len(), 44 + 16);
        assert_eq!(u16::from_le_bytes([w[20], w[21]]), 3);
        assert_eq!(a.segundos(), 2.0 / 48000.0);
    }

    /// Juez del VAE AISLADO: decodifica los latentes del oficial (`latentes` en su volcado).
    /// `GOTV_MUSICA_DIR=… GOTV_VOLCADO_OFICIAL=v.safetensors GOTV_SALIDA_VAE=a.safetensors cargo test -p gotv-mlx --release vae_aislado -- --ignored`
    #[test]
    #[ignore]
    fn vae_aislado() {
        let dir = std::path::PathBuf::from(std::env::var("GOTV_MUSICA_DIR").unwrap());
        let lat = Array::load_safetensors(std::env::var("GOTV_VOLCADO_OFICIAL").unwrap()).unwrap().remove("latentes").unwrap();
        let vae = Vae::cargar(&dir.join("vae")).unwrap();
        let a = vae.decodificar(&lat, None).unwrap();
        Array::save_safetensors([("audio", &a)], None, std::env::var("GOTV_SALIDA_VAE").unwrap()).unwrap();
    }

    /// El ⏹ corta una pieza REAL de 120 s a mitad de los pasos en menos de 1 s.
    /// `GOTV_MUSICA_DIR=… cargo test -p gotv-mlx --release cancelar_corta_la_musica -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn cancelar_corta_la_musica() {
        let dir = std::path::PathBuf::from(std::env::var("GOTV_MUSICA_DIR").unwrap());
        let cancelar = std::sync::Arc::new(AtomicBool::new(false));
        let etapas = std::sync::Arc::new(std::sync::Mutex::new(Vec::<Etapa>::new()));
        let (c2, e2) = (cancelar.clone(), etapas.clone());
        let hilo = std::thread::spawn(move || {
            let p = Pedido { descripcion: "epic orchestral".into(), segundos: 120.0, semilla: 1, ..Default::default() };
            generar(&dir, &p, &|e| e2.lock().unwrap().push(e), &c2, None, None).map(|_| ())
        });
        let t0 = std::time::Instant::now();
        while !etapas.lock().unwrap().contains(&Etapa::Paso(3, 8)) {
            assert!(t0.elapsed().as_secs() < 120, "no llegó al paso 3");
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let corte = std::time::Instant::now();
        cancelar.store(true, Ordering::Relaxed);
        let r = hilo.join().unwrap();
        let tardo = corte.elapsed();
        assert!(r.err().is_some_and(|e| e.to_string().contains("cancelado")), "no se canceló");
        assert!(tardo.as_secs_f32() < 1.0, "tardó {tardo:?}");
        assert!(!etapas.lock().unwrap().contains(&Etapa::Audio), "llegó al audio");
        eprintln!("cortó en {:.2} s", tardo.as_secs_f32());
    }
}
