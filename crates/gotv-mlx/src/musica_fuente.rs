//! M7d · el AUDIO FUENTE de un cover o un repaint de ACE-Step, como el manejador oficial:
//! - **lectura** de WAV, FLAC, OGG/Vorbis, MP3 y AAC/ALAC (M4A) con `symphonia` (Rust puro);
//! - **estéreo a 48 kHz** (`_normalize_audio_to_stereo_48k`): mono → duplicado, sólo dos canales,
//!   el remuestreo de `torchaudio.transforms.Resample` portado EXACTO (seno enventanado con Hann,
//!   ancho 6, `rolloff` 0,99, el núcleo en f64 con su índice de salida en f32) y recorte a [−1, 1];
//! - **codificador del VAE Oobleck** y su muestreo gaussiano (media + σ·ε, σ = softplus(escala) + 1e-4),
//!   por tramos con solape como `_tiled_encode_offload_cpu` (30 s y 2 s de solape; 15 s si no hay GPU
//!   grande).

use crate::modelo::M;
use crate::musica::{Conv, Snake};
use anyhow::{anyhow, bail, Context, Result};
use gotv_engine::{anyhow_tr, bail_tr};
use mlx_rs::ops::indexing::TryIndexOp;
use mlx_rs::ops;
use mlx_rs::Array;
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

pub const FRECUENCIA: u32 = 48_000;

/// Lee un archivo de audio: (canales, frecuencia), cada canal en f32 (el entero de 16 bits /32 768,
/// como libsndfile).
pub fn leer(ruta: &Path) -> Result<(Vec<Vec<f32>>, u32)> {
    use symphonia::core::audio::SampleBuffer;
    use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_NULL};
    use symphonia::core::errors::Error;
    use symphonia::core::formats::FormatOptions;
    use symphonia::core::io::MediaSourceStream;
    use symphonia::core::meta::MetadataOptions;
    use symphonia::core::probe::Hint;
    let f = std::fs::File::open(ruta).with_context(|| format!("abrir {}", ruta.display()))?;
    let mss = MediaSourceStream::new(Box::new(f), Default::default());
    let mut pista = Hint::new();
    if let Some(e) = ruta.extension().and_then(|e| e.to_str()) {
        pista.with_extension(e);
    }
    let sondeo = symphonia::default::get_probe()
        .format(&pista, mss, &FormatOptions::default(), &MetadataOptions::default())
        .map_err(|e| anyhow!("{}: formato de audio no reconocido ({e})", ruta.display()))?;
    let mut formato = sondeo.format;
    let track = formato.tracks().iter().find(|t| t.codec_params.codec != CODEC_TYPE_NULL).ok_or_else(|| anyhow_tr!("{}: sin pista de audio", "{}: no audio track", ruta.display()))?;
    let id = track.id;
    let mut dec = symphonia::default::get_codecs().make(&track.codec_params, &DecoderOptions::default()).map_err(|e| anyhow_tr!("{}: códec no admitido ({e})", "{}: unsupported codec ({e})", ruta.display()))?;
    let mut frecuencia = track.codec_params.sample_rate.unwrap_or(0);
    let mut canales: Vec<Vec<f32>> = Vec::new();
    loop {
        let paquete = match formato.next_packet() {
            Ok(p) => p,
            Err(Error::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(Error::ResetRequired) => break,
            Err(e) => bail!("{}: {e}", ruta.display()),
        };
        if paquete.track_id() != id {
            continue;
        }
        let buf = match dec.decode(&paquete) {
            Ok(b) => b,
            Err(Error::DecodeError(_)) => continue,
            Err(e) => bail!("{}: {e}", ruta.display()),
        };
        let spec = *buf.spec();
        frecuencia = spec.rate;
        let n = spec.channels.count();
        if canales.is_empty() {
            canales = vec![Vec::new(); n];
        }
        let mut sb = SampleBuffer::<f32>::new(buf.capacity() as u64, spec);
        sb.copy_interleaved_ref(buf);
        for (i, m) in sb.samples().iter().enumerate() {
            canales[i % n].push(*m);
        }
    }
    if canales.is_empty() || canales[0].is_empty() || frecuencia == 0 {
        bail_tr!("{}: el audio está vacío", "{}: the audio is empty", ruta.display());
    }
    Ok((canales, frecuencia))
}

fn mcd(a: u32, b: u32) -> u32 {
    if b == 0 {
        a
    } else {
        mcd(b, a % b)
    }
}

/// `torchaudio.functional.resample` con `sinc_interp_hann`, `lowpass_filter_width` 6 y `rolloff` 0,99
/// (los valores de `transforms.Resample`): el núcleo se calcula en f64 (el desplazamiento de cada
/// fase de salida, −j/n, en f32 como en torch) y se guarda en f32.
pub fn remuestrear(x: &[f32], de: u32, a: u32) -> Vec<f32> {
    if de == a || x.is_empty() {
        return x.to_vec();
    }
    let g = mcd(de, a);
    let (o, n) = ((de / g) as usize, (a / g) as usize);
    let ancho_filtro = 6.0f64;
    let base = (o.min(n) as f64) * 0.99;
    let ancho = (ancho_filtro * o as f64 / base).ceil() as usize;
    let largo_nucleo = 2 * ancho + o;
    let escala = base / o as f64;
    let pi = std::f64::consts::PI;
    let mut nucleo = vec![0f32; n * largo_nucleo];
    for j in 0..n {
        let tj = (-(j as f32) / n as f32) as f64;
        for i in 0..largo_nucleo {
            let idx = (i as f64 - ancho as f64) / o as f64;
            let t = ((tj + idx) * base).clamp(-ancho_filtro, ancho_filtro);
            let ventana = (t * pi / ancho_filtro / 2.0).cos().powi(2);
            let tt = t * pi;
            let s = if tt == 0.0 { 1.0 } else { tt.sin() / tt };
            nucleo[j * largo_nucleo + i] = (s * (ventana * escala)) as f32;
        }
    }
    let largo = x.len();
    let mut xp = vec![0f32; largo + 2 * ancho + o];
    xp[ancho..ancho + largo].copy_from_slice(x);
    let objetivo = ((n as f64) * largo as f64 / o as f64).ceil() as usize;
    let mut y = Vec::with_capacity(objetivo + n);
    let mut f = 0usize;
    while y.len() < objetivo && f * o + largo_nucleo <= xp.len() {
        let ventana = &xp[f * o..f * o + largo_nucleo];
        for j in 0..n {
            let k = &nucleo[j * largo_nucleo..(j + 1) * largo_nucleo];
            let s: f64 = ventana.iter().zip(k).map(|(a, b)| *a as f64 * *b as f64).sum();
            y.push(s as f32);
        }
        f += 1;
    }
    y.truncate(objetivo);
    y
}

/// `_normalize_audio_to_stereo_48k`: [izquierdo, derecho] a 48 kHz en [−1, 1].
pub fn estereo_48k(canales: Vec<Vec<f32>>, frecuencia: u32) -> [Vec<f32>; 2] {
    let mut c = canales;
    if c.len() == 1 {
        let m = c[0].clone();
        c.push(m);
    }
    c.truncate(2);
    let mut it = c.into_iter().map(|v| remuestrear(&v, frecuencia, FRECUENCIA).into_iter().map(|s| s.clamp(-1.0, 1.0)).collect::<Vec<f32>>());
    [it.next().unwrap_or_default(), it.next().unwrap_or_default()]
}

/// Mono a `frecuencia` Hz en [−1, 1]: la media de los canales, remuestreada como la música
/// (sinc con ventana de Hann). Para el oído (`gotv escuchar --archivo`: Whisper y el detector
/// de voz piden 16 kHz) desde CUALQUIER audio que lea `leer` (WAV de cualquier frecuencia, MP3,
/// FLAC, OGG, M4A), sin `afconvert`. Sólo CPU.
pub fn mono(canales: &[Vec<f32>], de: u32, frecuencia: u32) -> Vec<f32> {
    let n = canales.iter().map(Vec::len).min().unwrap_or(0);
    let k = canales.len().max(1) as f32;
    let m: Vec<f32> = (0..n).map(|i| canales.iter().map(|c| c[i]).sum::<f32>() / k).collect();
    remuestrear(&m, de, frecuencia).into_iter().map(|s| s.clamp(-1.0, 1.0)).collect()
}

/// Lee y normaliza el audio fuente (`process_src_audio`).
pub fn fuente(ruta: &Path) -> Result<[Vec<f32>; 2]> {
    let (c, f) = leer(ruta)?;
    Ok(estereo_48k(c, f))
}

/// `round()` de Python (mitades al par).
pub(crate) fn redondeo_par(x: f64) -> i64 {
    let r = x.round();
    if (x - x.trunc()).abs() == 0.5 && (r as i64) % 2 != 0 {
        (r - x.signum()) as i64
    } else {
        r as i64
    }
}

struct Residual {
    s1: Snake,
    c1: Conv,
    s2: Snake,
    c2: Conv,
    dil: i32,
}

struct BloqueCod {
    res: Vec<Residual>,
    snake: Snake,
    baja: Conv,
    paso: i32,
}

/// El codificador de `AutoencoderOobleck` (f32): estéreo [1, N, 2] → parámetros [1, T, 128].
pub struct CodificadorVae {
    entrada: Conv,
    bloques: Vec<BloqueCod>,
    snake: Snake,
    salida: Conv,
}

impl CodificadorVae {
    pub fn cargar(dir: &Path) -> Result<CodificadorVae> {
        let cfg: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dir.join("config.json")).context("vae/config.json")?)?;
        let pasos: Vec<i32> = serde_json::from_value(cfg.get("downsampling_ratios").cloned().ok_or_else(|| anyhow_tr!("vae sin downsampling_ratios", "vae without downsampling_ratios"))?)?;
        let mut p: HashMap<String, Array> = Array::load_safetensors(dir.join("diffusion_pytorch_model.safetensors")).map_err(|e| anyhow!("vae: {e:?}"))?;
        p.retain(|k, _| k.starts_with("encoder."));
        let mut bloques = Vec::new();
        for (i, &paso) in pasos.iter().enumerate() {
            let b = format!("encoder.block.{i}");
            let mut res = Vec::new();
            for (j, dil) in [1, 3, 9].into_iter().enumerate() {
                let r = format!("{b}.res_unit{}", j + 1);
                res.push(Residual { s1: Snake::de(&mut p, &format!("{r}.snake1"))?, c1: Conv::de(&mut p, &format!("{r}.conv1"), false)?, s2: Snake::de(&mut p, &format!("{r}.snake2"))?, c2: Conv::de(&mut p, &format!("{r}.conv2"), false)?, dil });
            }
            bloques.push(BloqueCod { res, snake: Snake::de(&mut p, &format!("{b}.snake1"))?, baja: Conv::de(&mut p, &format!("{b}.conv1"), false)?, paso });
        }
        let c = CodificadorVae { entrada: Conv::de(&mut p, "encoder.conv1", false)?, bloques, snake: Snake::de(&mut p, "encoder.snake1")?, salida: Conv::de(&mut p, "encoder.conv2", false)? };
        if !p.is_empty() {
            let mut sobran: Vec<_> = p.keys().cloned().collect();
            sobran.sort();
            bail_tr!("pesos del codificador del VAE sin usar: {:?}", "unused VAE encoder weights: {:?}", &sobran[..sobran.len().min(5)]);
        }
        Ok(c)
    }

    /// Un trozo de audio [1, N, 2] → (media, σ) [1, T, 64].
    pub fn parametros(&self, x: &Array, cancelar: Option<&AtomicBool>) -> Result<(Array, Array)> {
        let mut h = self.entrada.ap(x, 3, 1)?;
        for b in &self.bloques {
            if cancelar.is_some_and(|c| c.load(Ordering::Relaxed)) {
                bail!("cancelado");
            }
            // cada unidad se evalúa y se suelta la caché: con 30 s de audio a 48 kHz cada activación
            // del primer bloque pesa ~0,7 GB y la conv de núcleo 7 multiplica eso (medido: 27 GB de
            // pico sin esto en un cover de 120 s)
            for r in &b.res {
                let y = r.c1.ap(&r.s1.ap(&h)?, 3 * r.dil, r.dil)?;
                let y = r.c2.ap(&r.s2.ap(&y)?, 0, 1)?;
                h = ops::add(&h, &y).m()?;
                h.eval().m()?;
                let _ = mlx_rs::memory::clear_cache();
            }
            h = b.baja.ap_paso(&b.snake.ap(&h)?, b.paso, (b.paso + 1) / 2)?;
            h.eval().m()?;
            let _ = mlx_rs::memory::clear_cache();
        }
        let o = self.salida.ap(&self.snake.ap(&h)?, 1, 1)?;
        let mitad = o.dim(2) / 2;
        let media = o.try_index((.., .., ..mitad)).m()?;
        let escala = o.try_index((.., .., mitad..)).m()?;
        // softplus de torch (umbral 20) + 1e-4
        let sp = ops::select(&ops::gt(&escala, Array::from_f32(20.0)).m()?, &escala, &ops::log1p(&ops::exp(&escala).m()?).m()?).m()?;
        Ok((media, ops::add(&sp, Array::from_f32(1e-4)).m()?))
    }

    /// `tiled_encode` + `latent_dist.sample()`: audio [2][N] → latentes [1, T, 64]. `ruido(i, forma)`
    /// da el ε de cada tramo (el oficial usa el azar de torch).
    pub fn codificar(&self, audio: &[Vec<f32>; 2], tramo: usize, solape: usize, ruido: &mut dyn FnMut(usize, &[i32]) -> Result<Array>, cancelar: Option<&AtomicBool>) -> Result<Array> {
        let n = audio[0].len();
        let trozo = |a: usize, b: usize| -> Array {
            let mut v = Vec::with_capacity((b - a) * 2);
            for (l, r) in audio[0][a..b].iter().zip(&audio[1][a..b]) {
                v.push(*l);
                v.push(*r);
            }
            Array::from_slice(&v, &[1, (b - a) as i32, 2])
        };
        let mut muestra = |i: usize, a: usize, b: usize| -> Result<Array> {
            let (m, s) = self.parametros(&trozo(a, b), cancelar)?;
            let e = ruido(i, m.shape())?;
            let z = ops::add(&m, &ops::multiply(&s, &e).m()?).m()?;
            z.eval().m()?;
            Ok(z)
        };
        if n <= tramo {
            return muestra(0, 0, n);
        }
        let paso = tramo - 2 * solape;
        let pasos = n.div_ceil(paso);
        let (fin_nucleo, fin_ventana) = (paso.min(n), (paso.min(n) + solape).min(n));
        let z0 = muestra(0, 0, fin_ventana)?;
        let factor = fin_ventana as f64 / z0.dim(1) as f64;
        let recorte = redondeo_par((fin_ventana - fin_nucleo) as f64 / factor) as i32;
        let mut partes = vec![if recorte > 0 { z0.try_index((.., ..z0.dim(1) - recorte, ..)).m()? } else { z0 }];
        for i in 1..pasos {
            let ini = i * paso;
            let fin = (ini + paso).min(n);
            let (va, vb) = (ini.saturating_sub(solape), (fin + solape).min(n));
            let z = muestra(i, va, vb)?;
            let (ra, rb) = (redondeo_par((ini - va) as f64 / factor) as i32, redondeo_par((vb - fin) as f64 / factor) as i32);
            let hasta = if rb > 0 { z.dim(1) - rb } else { z.dim(1) };
            partes.push(z.try_index((.., ra..hasta, ..)).m()?);
        }
        ops::concatenate(&partes, 1).m()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// La voz de GOTV sale a 24 kHz: un WAV así (estéreo, tono de 440 Hz) llega al oído a 16 kHz
    /// mono con la misma duración y el mismo tono (cruces por cero). Sólo CPU, sin modelos.
    #[test]
    fn un_wav_de_24k_llega_al_oido_a_16k() {
        let tono: Vec<f32> = (0..24_000).map(|i| 0.5 * (2.0 * std::f32::consts::PI * 440.0 * i as f32 / 24_000.0).sin()).collect();
        let c = gotv_engine::prueba::CarpetaPrueba::nueva("gotv-mono16");
        let wav = c.ruta().join("voz.wav");
        std::fs::write(&wav, crate::wav_pcm16(&tono, 24_000)).unwrap();
        let (canales, de) = leer(&wav).unwrap();
        assert_eq!(de, 24_000);
        let dos = vec![canales[0].clone(), canales[0].clone()];
        let m = mono(&dos, de, 16_000);
        assert!((m.len() as i64 - 16_000).abs() <= 1, "{}", m.len());
        let cruces = m.windows(2).filter(|w| (w[0] < 0.0) != (w[1] < 0.0)).count();
        assert!((870..=890).contains(&cruces), "440 Hz = 880 cruces/s: {cruces}");
        let pico = m.iter().fold(0f32, |a, &x| a.max(x.abs()));
        assert!((0.45..=0.55).contains(&pico), "amplitud {pico}");
    }

    #[test]
    fn remuestreo_como_torchaudio() {
        // 44,1 → 48 kHz: largo ⌈160·N/147⌉; un seno de 1 kHz sigue siendo un seno de 1 kHz
        let n = 4410;
        let x: Vec<f32> = (0..n).map(|i| (2.0 * std::f32::consts::PI * 1000.0 * i as f32 / 44100.0).sin() * 0.5).collect();
        let y = remuestrear(&x, 44100, 48000);
        assert_eq!(y.len(), (160.0f64 * n as f64 / 147.0).ceil() as usize);
        let err: f32 = (500..y.len() - 500).map(|i| (y[i] - (2.0 * std::f32::consts::PI * 1000.0 * i as f32 / 48000.0).sin() * 0.5).abs()).fold(0.0, f32::max);
        assert!(err < 2e-3, "{err}");
        assert_eq!(remuestrear(&x, 48000, 48000), x);
    }

    /// Juez: `GOTV_JUEZ_FUENTE=<audio>` y `GOTV_JUEZ_SALIDA=<carpeta>` → `fuente_48k` [2, N] y los
    /// parámetros del VAE (`media`, `sigma` [1, T, 64]) del audio entero, para compararlos con el oficial.
    #[test]
    #[ignore]
    fn juez_fuente_y_vae() {
        let ruta = std::path::PathBuf::from(std::env::var("GOTV_JUEZ_FUENTE").expect("GOTV_JUEZ_FUENTE"));
        let sal = std::path::PathBuf::from(std::env::var("GOTV_JUEZ_SALIDA").expect("GOTV_JUEZ_SALIDA"));
        std::fs::create_dir_all(&sal).unwrap();
        let t0 = std::time::Instant::now();
        let a = fuente(&ruta).unwrap();
        eprintln!("leído y remuestreado en {:.2} s", t0.elapsed().as_secs_f32());
        let n = a[0].len() as i32;
        let mut v = a[0].clone();
        v.extend_from_slice(&a[1]);
        Array::save_safetensors([("fuente_48k", &Array::from_slice(&v, &[2, n]))], None, sal.join("fuente_48k.safetensors")).unwrap();
        let home = std::path::PathBuf::from(std::env::var("HOME").unwrap());
        let cod = CodificadorVae::cargar(&home.join(".cache/gotv/models/ACE-Step/Ace-Step1.5/vae")).unwrap();
        let mut x = Vec::with_capacity(2 * n as usize);
        for (l, r) in a[0].iter().zip(&a[1]) {
            x.push(*l);
            x.push(*r);
        }
        let t1 = std::time::Instant::now();
        let (m, s) = cod.parametros(&Array::from_slice(&x, &[1, n, 2]), None).unwrap();
        mlx_rs::transforms::eval([&m, &s]).unwrap();
        eprintln!("VAE codificador en {:.2} s", t1.elapsed().as_secs_f32());
        Array::save_safetensors([("media", &m), ("sigma", &s)], None, sal.join("vae.safetensors")).unwrap();
    }

    /// Juez del ensamblado por tramos: `GOTV_JUEZ_TRAMOS=<audio largo>` y `GOTV_JUEZ_SALIDA` → las
    /// MEDIAS ensambladas (ε = 0) con tramos de 30 s y 2 s de solape.
    #[test]
    #[ignore]
    fn juez_tramos() {
        let ruta = std::path::PathBuf::from(std::env::var("GOTV_JUEZ_TRAMOS").expect("GOTV_JUEZ_TRAMOS"));
        let sal = std::path::PathBuf::from(std::env::var("GOTV_JUEZ_SALIDA").expect("GOTV_JUEZ_SALIDA"));
        std::fs::create_dir_all(&sal).unwrap();
        let a = fuente(&ruta).unwrap();
        let home = std::path::PathBuf::from(std::env::var("HOME").unwrap());
        let cod = CodificadorVae::cargar(&home.join(".cache/gotv/models/ACE-Step/Ace-Step1.5/vae")).unwrap();
        let z = cod.codificar(&a, 30 * 48_000, 2 * 48_000, &mut |_, forma| ops::zeros::<f32>(forma).m(), None).unwrap();
        eprintln!("{} muestras → {} cuadros", a[0].len(), z.dim(1));
        Array::save_safetensors([("z_media", &z)], None, sal.join("tramos.safetensors")).unwrap();
    }

    #[test]
    fn redondeo_como_python() {
        assert_eq!((redondeo_par(12.5), redondeo_par(13.5), redondeo_par(2.4), redondeo_par(-0.5)), (12, 14, 2, 0));
    }
}
