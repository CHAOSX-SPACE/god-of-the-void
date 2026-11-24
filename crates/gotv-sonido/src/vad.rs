//! M3 · el detector de voz (VAD) en Rust puro, en la CPU: **Silero VAD v6** (MIT, pesos de
//! snakers4/silero-vad en el formato de `mlx-community/silero-vad-v6`, embebidos: 1,2 MB). Port de
//! `mlx_audio/vad/models/silero_vad` (rama de 16 kHz):
//!
//! ventana = 64 de contexto + 512 nuevas (32 ms) → relleno reflejado de 64 → STFT como conv
//! (256, paso 128 → 4 tramos × 129 magnitudes) → 4 conv k3 + ReLU (pasos 1, 2, 2, 1 → 1 tramo) →
//! LSTM(128) un paso con su estado → ReLU → conv 1×1 → sigmoide = probabilidad de voz.
//!
//! En la CPU a propósito: son ~0,5 MFLOP cada 32 ms, y así no compite por la GPU con el modelo que
//! piensa y la voz que habla. [`Turnos`] es la máquina de inicio/fin del monitor de voz (histéresis,
//! relleno, mínimo), con la pausa de 700 ms MEDIDA en el habla del Portador.

use anyhow::{anyhow, Result};
use std::collections::{HashMap, VecDeque};

const PESOS: &[u8] = include_bytes!("../recursos/silero-vad-v6.safetensors");
/// Muestras nuevas por ventana a 16 kHz (32 ms).
pub const VENTANA: usize = 512;
const CONTEXTO: usize = 64;
const RELLENO: usize = 64;
const FILTRO: usize = 256;
const SALTO: usize = 128;
const CORTE: usize = 129;
const H: usize = 128;

struct Tensor {
    forma: Vec<usize>,
    datos: Vec<f32>,
}

/// safetensors mínimo: cabecera JSON + f32 little-endian.
fn leer_safetensors(b: &[u8]) -> Result<HashMap<String, Tensor>> {
    let n = u64::from_le_bytes(b.get(..8).ok_or_else(|| anyhow!("safetensors corto"))?.try_into()?) as usize;
    let cab: serde_json::Value = serde_json::from_slice(&b[8..8 + n])?;
    let datos = &b[8 + n..];
    let mut m = HashMap::new();
    for (k, v) in cab.as_object().ok_or_else(|| anyhow!("cabecera rara"))? {
        if k == "__metadata__" {
            continue;
        }
        if v["dtype"] != "F32" {
            gotv_engine::bail_tr!("{k}: sólo f32", "{k}: f32 only");
        }
        let forma: Vec<usize> = v["shape"].as_array().into_iter().flatten().filter_map(|x| x.as_u64().map(|x| x as usize)).collect();
        let (a, z) = (v["data_offsets"][0].as_u64().unwrap_or(0) as usize, v["data_offsets"][1].as_u64().unwrap_or(0) as usize);
        let d: Vec<f32> = datos[a..z].as_chunks::<4>().0.iter().map(|c| f32::from_le_bytes(*c)).collect();
        if d.len() != forma.iter().product::<usize>() {
            gotv_engine::bail_tr!("{k}: tamaño y forma no cuadran", "{k}: size and shape do not match");
        }
        m.insert(k.clone(), Tensor { forma, datos: d });
    }
    Ok(m)
}

/// Conv 1D [sal, k, ent] (formato MLX) con sesgo y relleno 1 a cada lado.
struct Conv {
    w: Vec<f32>,
    b: Vec<f32>,
    sal: usize,
    k: usize,
    ent: usize,
    paso: usize,
}

impl Conv {
    /// x [T][ent] → [T'][sal], con ReLU.
    fn aplicar(&self, x: &[Vec<f32>]) -> Vec<Vec<f32>> {
        let t = x.len();
        let t2 = (t + 2 - self.k) / self.paso + 1;
        let mut y = vec![vec![0f32; self.sal]; t2];
        for (j, fila) in y.iter_mut().enumerate() {
            for (o, v) in fila.iter_mut().enumerate() {
                let mut s = self.b[o];
                for kk in 0..self.k {
                    let pos = (j * self.paso + kk) as isize - 1;
                    if pos < 0 || pos as usize >= t {
                        continue;
                    }
                    let w = &self.w[(o * self.k + kk) * self.ent..(o * self.k + kk + 1) * self.ent];
                    s += w.iter().zip(&x[pos as usize]).map(|(a, b)| a * b).sum::<f32>();
                }
                *v = s.max(0.0);
            }
        }
        y
    }
}

/// El detector: probabilidad de voz por ventana de 32 ms, con su estado (LSTM + contexto).
pub struct Vad {
    stft: Vec<f32>, // [258][256]
    convs: [Conv; 4],
    wx: Vec<f32>, // [512][128]
    wh: Vec<f32>,
    bias: Vec<f32>,
    fin_w: Vec<f32>,
    fin_b: f32,
    h: [f32; H],
    c: [f32; H],
    contexto: [f32; CONTEXTO],
}

fn sig(x: f32) -> f32 {
    1.0 / (1.0 + (-x).exp())
}

impl Vad {
    pub fn nuevo() -> Result<Vad> {
        let mut p = leer_safetensors(PESOS)?;
        let mut t = |k: &str| p.remove(&format!("vad_16k.{k}")).ok_or_else(|| anyhow!("falta vad_16k.{k}"));
        let mut conv = |n: &str, paso: usize| -> Result<Conv> {
            let w = t(&format!("{n}.weight"))?;
            let b = t(&format!("{n}.bias"))?;
            Ok(Conv { sal: w.forma[0], k: w.forma[1], ent: w.forma[2], w: w.datos, b: b.datos, paso })
        };
        let convs = [conv("conv1", 1)?, conv("conv2", 2)?, conv("conv3", 2)?, conv("conv4", 1)?];
        let stft = t("stft_conv.weight")?;
        if stft.forma != [2 * CORTE, FILTRO, 1] {
            gotv_engine::bail_tr!("stft con forma {:?}", "stft with shape {:?}", stft.forma);
        }
        let fin = t("final_conv.weight")?;
        Ok(Vad {
            stft: stft.datos,
            convs,
            wx: t("lstm.Wx")?.datos,
            wh: t("lstm.Wh")?.datos,
            bias: t("lstm.bias")?.datos,
            fin_w: fin.datos,
            fin_b: t("final_conv.bias")?.datos[0],
            h: [0.0; H],
            c: [0.0; H],
            contexto: [0.0; CONTEXTO],
        })
    }

    /// Olvida el estado (una conversación nueva).
    pub fn reiniciar(&mut self) {
        self.h = [0.0; H];
        self.c = [0.0; H];
        self.contexto = [0.0; CONTEXTO];
    }

    /// Probabilidad de voz de las 512 muestras siguientes (16 kHz, mono, en [−1, 1]).
    pub fn prob(&mut self, bloque: &[f32]) -> f32 {
        assert_eq!(bloque.len(), VENTANA, "{}", gotv_engine::tr!("el VAD come ventanas de {VENTANA} muestras", "the VAD takes windows of {VENTANA} samples"));
        // contexto + bloque, y relleno reflejado por la derecha (x[L-2], x[L-3], …)
        let mut x = Vec::with_capacity(CONTEXTO + VENTANA + RELLENO);
        x.extend_from_slice(&self.contexto);
        x.extend_from_slice(bloque);
        let l = x.len();
        for i in 0..RELLENO {
            x.push(x[l - 2 - i]);
        }
        self.contexto.copy_from_slice(&bloque[VENTANA - CONTEXTO..]);
        // STFT como conv sin sesgo → magnitudes [T][129]
        let tramos = (x.len() - FILTRO) / SALTO + 1;
        let mut mag = vec![vec![0f32; CORTE]; tramos];
        for (ti, fila) in mag.iter_mut().enumerate() {
            let seg = &x[ti * SALTO..ti * SALTO + FILTRO];
            for (j, m) in fila.iter_mut().enumerate() {
                let re: f32 = self.stft[j * FILTRO..(j + 1) * FILTRO].iter().zip(seg).map(|(a, b)| a * b).sum();
                let im: f32 = self.stft[(CORTE + j) * FILTRO..(CORTE + j + 1) * FILTRO].iter().zip(seg).map(|(a, b)| a * b).sum();
                *m = (re * re + im * im).sqrt();
            }
        }
        let mut y = mag;
        for c in &self.convs {
            y = c.aplicar(&y);
        }
        // LSTM sobre cada tramo (con estos tamaños queda uno), compuertas i, f, g, o
        let mut salida = 0.0;
        for paso in &y {
            let mut ifgo = self.bias.clone();
            for (r, v) in ifgo.iter_mut().enumerate() {
                let fx: f32 = self.wx[r * H..(r + 1) * H].iter().zip(paso).map(|(a, b)| a * b).sum();
                let fh: f32 = self.wh[r * H..(r + 1) * H].iter().zip(&self.h).map(|(a, b)| a * b).sum();
                *v += fx + fh;
            }
            for u in 0..H {
                let (i, f, g, o) = (sig(ifgo[u]), sig(ifgo[H + u]), ifgo[2 * H + u].tanh(), sig(ifgo[3 * H + u]));
                self.c[u] = f * self.c[u] + i * g;
                self.h[u] = o * self.c[u].tanh();
            }
            let z: f32 = self.fin_b + self.fin_w.iter().zip(&self.h).map(|(w, h)| w * h.max(0.0)).sum::<f32>();
            salida += sig(z);
        }
        salida / y.len().max(1) as f32
    }
}

/// Un evento de [`Turnos`].
#[derive(Debug, Clone, PartialEq)]
pub enum Evento {
    /// empezó a hablar (muestra absoluta, ya con el relleno previo)
    Inicio(usize),
    /// terminó una frase: su audio (16 kHz) y dónde empezó y terminó
    Fin { audio: Vec<f32>, desde: usize, hasta: usize },
    /// fue demasiado corto (un golpe, una tos): su duración en segundos
    Corto(f32),
}

/// Inicio y fin de voz con histéresis (el `Detector` de `monitor_voz.py`, que sigue a `VADIterator`
/// de Silero): empieza al pasar `umbral`, termina tras `silencio_ms` por debajo de `umbral − 0,15`,
/// antepone `relleno_ms` de audio previo y descarta lo más corto que `min_voz_ms`.
pub struct Turnos {
    pub umbral: f32,
    umbral_fin: f32,
    silencio: usize,
    relleno: usize,
    min_voz: usize,
    previo: VecDeque<Vec<f32>>,
    previo_max: usize,
    hablando: bool,
    audio: Vec<f32>,
    callado: usize,
    inicio: usize,
    muestra: usize,
}

/// Silencio que cierra un turno: MEDIDO en el habla del Portador (pausa más larga dentro de un
/// turno 608 ms, más corta entre turnos 3,48 s).
pub const SILENCIO_MS: usize = 700;

impl Turnos {
    pub fn nuevos(umbral: f32, silencio_ms: usize, relleno_ms: usize, min_voz_ms: usize) -> Turnos {
        let ms = |x: usize| x * 16;
        let relleno = ms(relleno_ms);
        Turnos { umbral, umbral_fin: (umbral - 0.15).max(0.01), silencio: ms(silencio_ms), relleno, min_voz: ms(min_voz_ms), previo: VecDeque::new(), previo_max: (relleno / VENTANA + 1).max(1), hablando: false, audio: Vec::new(), callado: 0, inicio: 0, muestra: 0 }
    }

    /// Los valores del monitor de voz: 0,5 · 700 ms · 200 ms · 250 ms.
    pub fn por_defecto() -> Turnos {
        Turnos::nuevos(0.5, SILENCIO_MS, 200, 250)
    }

    pub fn hablando(&self) -> bool {
        self.hablando
    }

    /// Silencio acumulado dentro de la frase en curso, en ms (0 si se está hablando o no hay frase).
    pub fn silencio_ms(&self) -> usize {
        if self.hablando {
            self.callado / 16
        } else {
            0
        }
    }

    /// El audio de la frase en curso (para transcribirlo por adelantado durante una pausa).
    pub fn audio_en_curso(&self) -> &[f32] {
        &self.audio
    }

    /// Un bloque de audio y su probabilidad de voz.
    pub fn paso(&mut self, x: &[f32], p: f32) -> Option<Evento> {
        let mut ev = None;
        if !self.hablando {
            if p >= self.umbral {
                self.hablando = true;
                self.callado = 0;
                self.audio = self.previo.iter().flatten().copied().collect();
                self.inicio = self.muestra.saturating_sub(self.audio.len());
                ev = Some(Evento::Inicio(self.inicio));
            }
            if self.previo.len() == self.previo_max {
                self.previo.pop_front();
            }
            self.previo.push_back(x.to_vec());
        }
        if self.hablando {
            self.audio.extend_from_slice(x);
            if p < self.umbral_fin {
                self.callado += x.len();
                if self.callado >= self.silencio {
                    self.hablando = false;
                    let quitar = self.callado.saturating_sub(self.relleno);
                    let mut a = std::mem::take(&mut self.audio);
                    a.truncate(a.len() - quitar.min(a.len()));
                    let hasta = self.muestra + x.len() - quitar;
                    self.previo.clear();
                    ev = Some(if a.len() >= self.min_voz + self.relleno { Evento::Fin { audio: a, desde: self.inicio, hasta } } else { Evento::Corto(a.len() as f32 / 16000.0) });
                }
            } else {
                self.callado = 0;
            }
        }
        self.muestra += x.len();
        ev
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn el_silencio_no_es_voz_y_el_estado_se_reinicia() {
        let mut v = Vad::nuevo().unwrap();
        let p0 = v.prob(&[0.0; VENTANA]);
        assert!(p0 < 0.1, "silencio → {p0}");
        v.reiniciar();
        assert_eq!(v.prob(&[0.0; VENTANA]), p0, "mismo estado inicial, misma salida");
    }

    /// Juez contra mlx-audio (mismos pesos v6): probabilidad por ventana en WAV reales.
    /// `GOTV_VAD_REF=<probs.json de ref_vad.py> cargo test -p gotv-sonido --release -- --ignored --nocapture vad_como`
    #[test]
    #[ignore]
    fn vad_como_mlx_audio() {
        let Ok(r) = std::env::var("GOTV_VAD_REF") else { return };
        let refs: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(r).unwrap()).unwrap();
        for (wav, ps) in refs.as_object().unwrap() {
            let b = std::fs::read(wav).unwrap();
            // PCM 16 bits: el bloque «data»
            let i = b.windows(4).position(|w| w == b"data").unwrap() + 8;
            let a: Vec<f32> = b[i..].as_chunks::<2>().0.iter().map(|c| i16::from_le_bytes(*c) as f32 / 32768.0).collect();
            let ps: Vec<f32> = ps.as_array().unwrap().iter().map(|x| x.as_f64().unwrap() as f32).collect();
            let mut v = Vad::nuevo().unwrap();
            let t = std::time::Instant::now();
            let mias: Vec<f32> = a.as_chunks::<VENTANA>().0.iter().take(ps.len()).map(|c| v.prob(c)).collect();
            let dt = t.elapsed();
            let dif = mias.iter().zip(&ps).map(|(x, y)| (x - y).abs()).fold(0.0, f32::max);
            let distintas = mias.iter().zip(&ps).filter(|(x, y)| (**x >= 0.5) != (**y >= 0.5)).count();
            println!("{} · {} ventanas · dif. máx {dif:e} · decisiones distintas {distintas} · {:.1} µs por ventana (32 ms de audio)", wav.rsplit('/').next().unwrap(), mias.len(), dt.as_secs_f64() * 1e6 / mias.len() as f64);
            assert_eq!(mias.len(), ps.len());
            assert!(dif < 1e-3, "la probabilidad difiere");
            assert_eq!(distintas, 0);
        }
    }

    /// ECO: ¿el micrófono oye la voz de GOTV por el altavoz? Suena un WAV (24 kHz, 16 bits) y a la
    /// vez se escucha el micrófono con el VAD. `GOTV_ECO_WAV=<wav> cargo test -p gotv-sonido --release
    /// -- --ignored --nocapture eco_del_altavoz`
    #[test]
    #[ignore]
    fn eco_del_altavoz() {
        let Ok(w) = std::env::var("GOTV_ECO_WAV") else { return };
        let b = std::fs::read(w).unwrap();
        let i = b.windows(4).position(|w| w == b"data").unwrap() + 8;
        let onda: Vec<f32> = b[i..].as_chunks::<2>().0.iter().map(|c| i16::from_le_bytes(*c) as f32 / 32768.0).collect();
        // GOTV_ECO_AEC=1: todo por VoiceProcessingIO · =mixto: micrófono por VoiceProcessingIO y la
        // voz por AudioQueue (la salida con la calidad de `gotv decir`) · sin ella: AudioQueue sueltas
        let modo = std::env::var("GOTV_ECO_AEC").unwrap_or_default();
        let aec = !modo.is_empty();
        let mixto = modo == "mixto";
        let duplex = aec.then(|| crate::Duplex::abrir(24000, 16000, VENTANA).unwrap());
        let mic_suelto = (!aec).then(|| crate::Microfono::abrir(16000, VENTANA).unwrap());
        let leer_bloque = |t| duplex.as_ref().map(|d| d.leer(t)).unwrap_or_else(|| mic_suelto.as_ref().unwrap().leer(t));
        // 1 s de silencio de la sala primero, para comparar
        let mut v = Vad::nuevo().unwrap();
        let leer = |seg: f32, v: &mut Vad| -> Vec<f32> {
            let t = std::time::Instant::now();
            let mut ps = Vec::new();
            while t.elapsed().as_secs_f32() < seg {
                if let Some(x) = leer_bloque(std::time::Duration::from_secs(1)) {
                    if x.len() == VENTANA {
                        ps.push(v.prob(&x));
                    }
                }
            }
            ps
        };
        let sala = leer(1.0, &mut v);
        let altavoz = (!aec || mixto).then(|| crate::Altavoz::abrir(24000).unwrap());
        match (&duplex, &altavoz) {
            (_, Some(a)) => a.empujar(&onda),
            (Some(d), _) => d.empujar(&onda),
            _ => unreachable!(),
        }
        let durante = leer(onda.len() as f32 / 24000.0 + 0.3, &mut v);
        let resumen = |n: &str, ps: &[f32]| {
            let voz = ps.iter().filter(|p| **p >= 0.5).count();
            let max = ps.iter().copied().fold(0.0, f32::max);
            let mut seguidas = 0;
            let mut racha = 0;
            for p in ps {
                racha = if *p >= 0.5 { racha + 1 } else { 0 };
                seguidas = seguidas.max(racha);
            }
            println!("{n}: {} ventanas · ≥0,5 en {voz} ({:.0} %) · máx {max:.2} · racha más larga {} ({} ms)", ps.len(), voz as f32 * 100.0 / ps.len().max(1) as f32, seguidas, seguidas * 32);
        };
        resumen("sala en silencio", &sala);
        resumen("mientras GOTV habla", &durante);
        // dónde caen las rachas (≥ 3 ventanas = 96 ms)
        let mut rachas = Vec::new();
        let mut ini = None;
        for (i, p) in durante.iter().chain([&0.0]).enumerate() {
            match (*p >= 0.5, ini) {
                (true, None) => ini = Some(i),
                (false, Some(a)) => {
                    if i - a >= 3 {
                        rachas.push(format!("{:.2}–{:.2} s", a as f32 * 0.032, i as f32 * 0.032));
                    }
                    ini = None;
                }
                _ => {}
            }
        }
        println!("rachas: {rachas:?}");
    }

    #[test]
    fn turnos_con_histeresis_relleno_y_minimo() {
        let mut t = Turnos::nuevos(0.5, 700, 200, 250);
        let b = vec![0.1f32; VENTANA];
        let mut evs = Vec::new();
        // 10 bloques de silencio, 20 de voz (640 ms), una pausa de 0,45 (entre fin y umbral: sigue), 25 de silencio
        let probs: Vec<f32> = std::iter::repeat_n(0.02, 10).chain(std::iter::repeat_n(0.9, 20)).chain([0.45]).chain(std::iter::repeat_n(0.02, 25)).collect();
        for p in probs {
            if let Some(e) = t.paso(&b, p) {
                evs.push(e);
            }
        }
        assert_eq!(evs.len(), 2, "{evs:?}");
        // el inicio retrocede el relleno: 7 bloques previos (200 ms → 6 + 1)
        assert_eq!(evs[0], Evento::Inicio(10 * VENTANA - 7 * VENTANA));
        let Evento::Fin { audio, desde, hasta } = &evs[1] else { panic!("{:?}", evs[1]) };
        assert_eq!(*desde, 3 * VENTANA);
        assert_eq!(audio.len(), hasta - desde, "el audio va de desde a hasta");
        assert!(!t.hablando());
        // un golpe de 2 bloques es «corto»
        let mut t = Turnos::por_defecto();
        let ev: Vec<_> = std::iter::repeat_n(0.9, 2).chain(std::iter::repeat_n(0.0, 30)).filter_map(|p| t.paso(&b, p)).collect();
        assert!(matches!(ev.last(), Some(Evento::Corto(_))), "{ev:?}");
    }
}
