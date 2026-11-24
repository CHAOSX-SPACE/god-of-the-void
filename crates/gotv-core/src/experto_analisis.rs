//! Ola EXPERTO II · X2-3 — el ANÁLISIS del estudio, en Rust y sin GPU: lo que un profesional mira antes
//! de tocar nada, y lo que la IA necesita saber para generar algo que encaje.
//!
//! - **Pico real** (dBTP): el pico ENTRE muestras, con sobremuestreo ×4 (ITU-R BS.1770-4, anexo 2). Un
//!   seno a un cuarto de la frecuencia de muestreo desfasado 45° marca −3,01 dB de pico de muestra y
//!   0 dB de pico real: es el caso de prueba.
//! - **Sonoridad**: integrada (la de `experto_dsp::lufs`), de corto plazo (3 s) y momentánea (400 ms)
//!   máximas, y el **rango** (LRA de EBU Tech 3342: percentiles 10 y 95 del corto plazo, con puertas de
//!   −70 LUFS y −20 LU).
//! - **Correlación** de fase entre canales (+1 mono, 0 sin relación, −1 en contrafase).
//! - **Tempo**: flujo espectral (lo que SUBE entre cuadros) → autocorrelación con preferencia por el
//!   entorno de 120 bpm (Ellis 2007) → afinado con un peine sobre los múltiplos del periodo.
//! - **Tonalidad**: croma (energía por nota, afinación La = 440 Hz) correlacionado con los perfiles de
//!   Krumhansl y Kessler para las 24 tonalidades.
use crate::experto_dsp::{self as dsp, Estereo};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct Analisis {
    pub segundos: f64,
    /// `None` si no hay pulso claro
    pub bpm: Option<f64>,
    /// 0…1: cuánto destaca ese tempo
    pub bpm_confianza: f64,
    /// «A minor», «C major»… (`None` sin notas claras)
    pub tonalidad: Option<String>,
    pub tonalidad_confianza: f64,
    pub lufs: Option<f64>,
    pub lufs_corto_max: Option<f64>,
    pub lufs_momentaneo_max: Option<f64>,
    /// el rango de sonoridad (LU)
    pub rango_lu: Option<f64>,
    pub pico_db: f64,
    pub pico_real_db: f64,
    pub rms_db: f64,
    pub correlacion: f64,
    /// la componente continua (debe ser ~0)
    pub continua: f64,
}

fn db(v: f64) -> f64 {
    20.0 * v.max(1e-12).log10()
}

/// El pico real de un canal: el máximo entre las muestras y tres puntos intermedios por muestra,
/// interpolados con un seno cardinal de 16 muestras a cada lado con ventana de Blackman.
fn pico_real_canal(x: &[f32]) -> f64 {
    const M: isize = 16;
    // los coeficientes de las tres fases (1/4, 2/4, 3/4)
    let fases: Vec<Vec<f64>> = (1..4)
        .map(|p| {
            let frac = p as f64 / 4.0;
            (-M + 1..=M)
                .map(|k| {
                    let t = k as f64 - frac;
                    let sinc = if t == 0.0 { 1.0 } else { (std::f64::consts::PI * t).sin() / (std::f64::consts::PI * t) };
                    let u = (t + M as f64) / (2.0 * M as f64);
                    let w = 0.42 - 0.5 * (2.0 * std::f64::consts::PI * u).cos() + 0.08 * (4.0 * std::f64::consts::PI * u).cos();
                    sinc * w
                })
                .collect()
        })
        .collect();
    let n = x.len() as isize;
    let mut pico = 0f64;
    for i in 0..n {
        pico = pico.max(x[i as usize].abs() as f64);
        // sólo vale la pena interpolar cerca de un pico: si las vecinas son pequeñas, el intermedio también
        let cerca = x[i as usize].abs().max(x[(i + 1).min(n - 1) as usize].abs()) as f64;
        if cerca * 2.5 < pico {
            continue;
        }
        for f in &fases {
            let mut y = 0f64;
            for (j, c) in f.iter().enumerate() {
                let k = i + (j as isize - M + 1);
                if k >= 0 && k < n {
                    y += x[k as usize] as f64 * c;
                }
            }
            pico = pico.max(y.abs());
        }
    }
    pico
}

/// El pico real de la señal en dBTP.
pub fn pico_real_db(x: &Estereo) -> f64 {
    db(pico_real_canal(&x[0]).max(pico_real_canal(&x[1])))
}

/// La sonoridad (LUFS) de cada ventana de `ventana` segundos, cada `salto` segundos.
fn sonoridades(k: &Estereo, fs: u32, ventana: f64, salto: f64) -> Vec<f64> {
    let (v, h) = ((ventana * fs as f64) as usize, ((salto * fs as f64) as usize).max(1));
    let n = k[0].len();
    if n < v || v == 0 {
        return Vec::new();
    }
    // sumas acumuladas de los cuadrados: cada ventana, en O(1)
    let acum: Vec<Vec<f64>> = k
        .iter()
        .map(|c| {
            let mut a = Vec::with_capacity(n + 1);
            let mut s = 0f64;
            a.push(0.0);
            for x in c {
                s += (*x as f64).powi(2);
                a.push(s);
            }
            a
        })
        .collect();
    (0..=(n - v) / h).map(|i| -0.691 + 10.0 * ((acum[0][i * h + v] - acum[0][i * h] + acum[1][i * h + v] - acum[1][i * h]) / v as f64).max(1e-12).log10()).collect()
}

/// El rango de sonoridad (EBU Tech 3342) a partir del corto plazo.
fn rango(cortas: &[f64]) -> Option<f64> {
    let vivas: Vec<f64> = cortas.iter().copied().filter(|l| *l > -70.0).collect();
    if vivas.is_empty() {
        return None;
    }
    let media = 10.0 * (vivas.iter().map(|l| 10f64.powf(l / 10.0)).sum::<f64>() / vivas.len() as f64).log10();
    let mut v: Vec<f64> = vivas.into_iter().filter(|l| *l > media - 20.0).collect();
    if v.len() < 2 {
        return None;
    }
    v.sort_by(f64::total_cmp);
    let pct = |p: f64| v[((v.len() - 1) as f64 * p).round() as usize];
    Some(pct(0.95) - pct(0.10))
}

/// La correlación de fase entre los dos canales.
pub fn correlacion(x: &Estereo) -> f64 {
    let (mut lr, mut ll, mut rr) = (0f64, 0f64, 0f64);
    for (a, b) in x[0].iter().zip(&x[1]) {
        let (a, b) = (*a as f64, *b as f64);
        lr += a * b;
        ll += a * a;
        rr += b * b;
    }
    if ll <= 0.0 || rr <= 0.0 {
        return 0.0;
    }
    lr / (ll * rr).sqrt()
}

/// El módulo del espectro de cada cuadro (ventana de Hann de `n`, salto `h`), de 0 a n/2.
fn espectros(x: &[f32], n: usize, h: usize) -> Vec<Vec<f64>> {
    if x.len() < n {
        return Vec::new();
    }
    let ventana: Vec<f64> = (0..n).map(|i| 0.5 - 0.5 * (2.0 * std::f64::consts::PI * i as f64 / n as f64).cos()).collect();
    let (mut re, mut im) = (vec![0f64; n], vec![0f64; n]);
    (0..=(x.len() - n) / h)
        .map(|m| {
            for i in 0..n {
                re[i] = x[m * h + i] as f64 * ventana[i];
                im[i] = 0.0;
            }
            dsp::fft(&mut re, &mut im, false);
            (0..=n / 2).map(|k| re[k].hypot(im[k])).collect()
        })
        .collect()
}

fn mono(x: &Estereo) -> Vec<f32> {
    x[0].iter().zip(&x[1]).map(|(a, b)| (a + b) * 0.5).collect()
}

/// El tempo en bpm y su confianza (0…1), o `None` si no hay pulso.
pub fn tempo(x: &Estereo, fs: u32) -> Option<(f64, f64)> {
    const N: usize = 1024;
    const H: usize = 512;
    let m = mono(x);
    // hasta 90 s del centro: de sobra para el tempo, y acota el coste
    let tope = 90 * fs as usize;
    let m = if m.len() > tope { &m[(m.len() - tope) / 2..(m.len() + tope) / 2] } else { &m[..] };
    let esp = espectros(m, N, H);
    if esp.len() < 64 {
        return None;
    }
    let cps = fs as f64 / H as f64; // cuadros por segundo
    // el flujo espectral con compresión logarítmica: sólo lo que sube
    let mut flujo: Vec<f64> = vec![0.0];
    for t in 1..esp.len() {
        flujo.push(esp[t].iter().zip(&esp[t - 1]).map(|(a, b)| ((1.0 + 100.0 * a).ln() - (1.0 + 100.0 * b).ln()).max(0.0)).sum());
    }
    // sin su media local (≈ 1 s): quedan los golpes
    let v = (cps as usize).max(1);
    let acum: Vec<f64> = std::iter::once(0.0).chain(flujo.iter().scan(0.0, |s, x| { *s += x; Some(*s) })).collect();
    let golpes: Vec<f64> = (0..flujo.len())
        .map(|i| {
            let (a, b) = (i.saturating_sub(v / 2), (i + v / 2 + 1).min(flujo.len()));
            (flujo[i] - (acum[b] - acum[a]) / (b - a) as f64).max(0.0)
        })
        .collect();
    let energia: f64 = golpes.iter().map(|g| g * g).sum();
    if energia <= 1e-9 {
        return None;
    }
    // autocorrelación entre 30 y 300 bpm
    let (lmin, lmax) = ((60.0 / 300.0 * cps).floor() as usize, ((60.0 / 30.0 * cps).ceil() as usize).min(golpes.len() / 2));
    if lmax <= lmin + 2 {
        return None;
    }
    let ac: Vec<f64> = (0..=lmax * 4).map(|l| if l < golpes.len() { golpes[..golpes.len() - l].iter().zip(&golpes[l..]).map(|(a, b)| a * b).sum::<f64>() / energia } else { 0.0 }).collect();
    let en = |l: f64| -> f64 {
        let (i, f) = (l.floor() as usize, l - l.floor());
        if i + 1 >= ac.len() {
            0.0
        } else {
            ac[i] * (1.0 - f) + ac[i + 1] * f
        }
    };
    // el peine: un tempo verdadero también correlaciona a 2, 3 y 4 periodos
    let peine = |l: f64| (1..=4).map(|k| en(l * k as f64)).sum::<f64>() / 4.0;
    // la preferencia por el entorno de 120 bpm (log-normal de una octava de ancho)
    let peso = |l: f64| {
        let bpm = 60.0 * cps / l;
        (-0.5 * ((bpm / 120.0).log2() / 1.0).powi(2)).exp()
    };
    let mut mejor = (0f64, 0f64);
    let mut l = lmin as f64;
    while l <= lmax as f64 {
        let v = peine(l) * peso(l);
        if v > mejor.1 {
            mejor = (l, v);
        }
        l += 0.25;
    }
    if mejor.1 <= 0.0 {
        return None;
    }
    // afinado: el máximo del peine alrededor, a centésimas de cuadro
    let (mut fino, mut vmax) = (mejor.0, peine(mejor.0));
    let mut l = mejor.0 - 0.5;
    while l <= mejor.0 + 0.5 {
        let v = (1..=8).map(|k| en(l * k as f64)).sum::<f64>();
        if v > vmax || l == mejor.0 - 0.5 {
            (fino, vmax) = (l, v);
        }
        l += 0.01;
    }
    let confianza = peine(fino).clamp(0.0, 1.0);
    if confianza < 0.05 {
        return None;
    }
    Some((60.0 * cps / fino, confianza))
}

const NOTAS: [&str; 12] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
// los perfiles de Krumhansl y Kessler (1982): cuánto «pesa» cada grado en una tonalidad mayor y en una menor
const MAYOR: [f64; 12] = [6.35, 2.23, 3.48, 2.33, 4.38, 4.09, 2.52, 5.19, 2.39, 3.66, 2.29, 2.88];
const MENOR: [f64; 12] = [6.33, 2.68, 3.52, 5.38, 2.60, 3.53, 2.54, 4.75, 3.98, 2.69, 3.34, 3.17];

fn pearson(a: &[f64; 12], b: &[f64; 12]) -> f64 {
    let (ma, mb) = (a.iter().sum::<f64>() / 12.0, b.iter().sum::<f64>() / 12.0);
    let (mut n, mut da, mut dbb) = (0f64, 0f64, 0f64);
    for i in 0..12 {
        n += (a[i] - ma) * (b[i] - mb);
        da += (a[i] - ma).powi(2);
        dbb += (b[i] - mb).powi(2);
    }
    if da <= 0.0 || dbb <= 0.0 {
        0.0
    } else {
        n / (da * dbb).sqrt()
    }
}

/// La energía por nota (Do … Si) de la señal, entre 55 Hz y 2 kHz.
pub fn croma(x: &Estereo, fs: u32) -> [f64; 12] {
    const N: usize = 8192;
    let m = mono(x);
    let tope = 120 * fs as usize;
    let m = if m.len() > tope { &m[(m.len() - tope) / 2..(m.len() + tope) / 2] } else { &m[..] };
    let mut c = [0f64; 12];
    for e in espectros(m, N, N / 2) {
        for (k, v) in e.iter().enumerate() {
            let f = k as f64 * fs as f64 / N as f64;
            if !(55.0..=2000.0).contains(&f) {
                continue;
            }
            // la nota MIDI (La 440 = 69); se reparte entre las dos notas vecinas según lo desafinada
            let nota = 69.0 + 12.0 * (f / 440.0).log2();
            let (base, frac) = (nota.floor(), nota - nota.floor());
            let p = base as i64;
            c[p.rem_euclid(12) as usize] += v * v * (1.0 - frac);
            c[(p + 1).rem_euclid(12) as usize] += v * v * frac;
        }
    }
    c
}

/// La tonalidad («A minor») y su confianza (la correlación de la mejor, 0…1).
pub fn tonalidad(x: &Estereo, fs: u32) -> Option<(String, f64)> {
    let c = croma(x, fs);
    if c.iter().sum::<f64>() <= 1e-9 {
        return None;
    }
    // la raíz comprime: sin ella manda la nota más fuerte y no el conjunto
    let c: [f64; 12] = std::array::from_fn(|i| c[i].sqrt());
    let mut mejor = (String::new(), f64::MIN);
    for tonica in 0..12 {
        let rotado: [f64; 12] = std::array::from_fn(|i| c[(i + tonica) % 12]);
        for (perfil, modo) in [(&MAYOR, "major"), (&MENOR, "minor")] {
            let r = pearson(&rotado, perfil);
            if r > mejor.1 {
                mejor = (format!("{} {modo}", NOTAS[tonica]), r);
            }
        }
    }
    (mejor.1 > 0.2).then(|| (mejor.0, mejor.1.clamp(0.0, 1.0)))
}

/// Todo el análisis de una señal.
pub fn analizar(x: &Estereo, fs: u32) -> Analisis {
    let n = x[0].len().min(x[1].len());
    if n == 0 {
        return Analisis { pico_db: db(0.0), pico_real_db: db(0.0), rms_db: db(0.0), ..Default::default() };
    }
    let (k1, k2) = dsp::filtro_k(fs as f64);
    let k: Estereo = [k2.filtrar(&k1.filtrar(&x[0])), k2.filtrar(&k1.filtrar(&x[1]))];
    let cortas = sonoridades(&k, fs, 3.0, 0.5);
    let momentaneas = sonoridades(&k, fs, 0.4, 0.1);
    let maximo = |v: &[f64]| v.iter().copied().fold(f64::MIN, f64::max);
    let pico = x.iter().flat_map(|c| c.iter()).fold(0f32, |m, v| m.max(v.abs())) as f64;
    let cuadrados: f64 = x.iter().flat_map(|c| c.iter()).map(|v| (*v as f64).powi(2)).sum();
    let t = tempo(x, fs);
    let ton = tonalidad(x, fs);
    Analisis {
        segundos: n as f64 / fs as f64,
        bpm: t.map(|(b, _)| (b * 10.0).round() / 10.0),
        bpm_confianza: t.map(|(_, c)| c).unwrap_or(0.0),
        tonalidad: ton.as_ref().map(|(t, _)| t.clone()),
        tonalidad_confianza: ton.map(|(_, c)| c).unwrap_or(0.0),
        lufs: dsp::lufs(x, fs),
        lufs_corto_max: (!cortas.is_empty()).then(|| maximo(&cortas)).filter(|l| *l > -70.0),
        lufs_momentaneo_max: (!momentaneas.is_empty()).then(|| maximo(&momentaneas)).filter(|l| *l > -70.0),
        rango_lu: rango(&cortas),
        pico_db: db(pico),
        pico_real_db: pico_real_db(x),
        rms_db: db((cuadrados / (2 * n) as f64).sqrt()),
        correlacion: correlacion(x),
        continua: x.iter().flat_map(|c| c.iter()).map(|v| *v as f64).sum::<f64>() / (2 * n) as f64,
    }
}

/// El análisis de un WAV de la caché, guardado a su lado (`<archivo>.analisis.gotv`, formato propio):
/// se calcula una sola vez. El `.analisis.json` de antes era caché: se recalcula.
pub fn analizar_archivo(ruta: &std::path::Path, fs: u32) -> anyhow::Result<Analisis> {
    let propio = ruta.with_extension("analisis.gotv");
    if let Some(a) = crate::programar::codec::leer_serde(&propio, b"EXAN") {
        return Ok(a);
    }
    let a = analizar(&dsp::leer_estereo(ruta, fs)?, fs);
    let _ = crate::programar::codec::guardar_serde(&propio, b"EXAN", &a);
    Ok(a)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FS: u32 = 48_000;

    fn seno(f: f64, a: f64, fase: f64, s: f64) -> Vec<f32> {
        (0..(s * FS as f64) as usize).map(|n| (a * (2.0 * std::f64::consts::PI * f * n as f64 / FS as f64 + fase).sin()) as f32).collect()
    }

    /// El caso clásico del pico entre muestras: un seno a fs/4 desfasado 45° toca ±0,7071 en las
    /// muestras, pero la onda real llega a 1.
    #[test]
    fn el_pico_real_ve_lo_que_pasa_entre_muestras() {
        // con entrada y salida suaves: un seno que arranca de golpe SÍ tiene un pico verdadero mayor en el borde
        let mut m = seno(12_000.0, 1.0, std::f64::consts::FRAC_PI_4, 0.5);
        let n = m.len();
        for i in 0..2400 {
            let w = (0.5 - 0.5 * (std::f64::consts::PI * i as f64 / 2400.0).cos()) as f32;
            m[i] *= w;
            m[n - 1 - i] *= w;
        }
        let x: Estereo = [m.clone(), m];
        let de_muestra = db(x[0].iter().fold(0f32, |m, v| m.max(v.abs())) as f64);
        assert!((de_muestra + 3.01).abs() < 0.01, "{de_muestra}");
        let real = pico_real_db(&x);
        assert!(real.abs() < 0.1, "pico real {real} dBTP");
        // un seno lento ya lo dice el pico de muestra: los dos coinciden
        let y: Estereo = [seno(100.0, 0.5, 0.0, 0.5), seno(100.0, 0.5, 0.0, 0.5)];
        assert!((pico_real_db(&y) - db(0.5)).abs() < 0.01);
        // un impulso aislado no se infla
        let mut z: Estereo = [vec![0.0; 4800], vec![0.0; 4800]];
        z[0][2400] = 0.8;
        assert!((pico_real_db(&z) - db(0.8)).abs() < 0.01);
    }

    /// Un clic cada 0,5 s es 120 bpm; cada 60/93 s, 93 bpm; cada 60/174 s, 174. A ±0,5 bpm.
    #[test]
    fn el_tempo_de_un_clic_sale_exacto() {
        for bpm in [93.0, 120.0, 128.0, 140.0, 160.0, 70.0] {
            let n = 30 * FS as usize;
            let periodo = 60.0 / bpm * FS as f64;
            let mut m = vec![0f32; n];
            let mut k = 0;
            loop {
                let i = (k as f64 * periodo).round() as usize;
                if i + 400 >= n {
                    break;
                }
                // un golpe: 8 ms de un seno de 1 kHz que decae (el primero de cada 4, más fuerte)
                for j in 0..400 {
                    m[i + j] += (if k % 4 == 0 { 0.9 } else { 0.6 }) * (-(j as f32) / 80.0).exp() * (2.0 * std::f32::consts::PI * 1000.0 * j as f32 / FS as f32).sin();
                }
                k += 1;
            }
            let (t, c) = tempo(&[m.clone(), m], FS).expect("hay pulso");
            assert!((t - bpm).abs() < 0.5, "{bpm} bpm → {t} (confianza {c})");
        }
        // el silencio y un tono fijo no tienen tempo
        assert!(tempo(&[vec![0.0; 10 * FS as usize], vec![0.0; 10 * FS as usize]], FS).is_none());
    }

    /// Acordes sintéticos (cada nota con 4 armónicos): Do-Fa-Sol-Do es Do mayor; La m-Re m-Mi m-La m es
    /// La menor; transportado 3 semitonos arriba, Mi♭ mayor (D#) y Do menor.
    #[test]
    fn la_tonalidad_de_unos_acordes() {
        let nota = |midi: f64, s: f64| -> Vec<f32> {
            let f = 440.0 * 2f64.powf((midi - 69.0) / 12.0);
            let mut x = vec![0f32; (s * FS as f64) as usize];
            for (h, a) in [(1.0, 0.5), (2.0, 0.25), (3.0, 0.12), (4.0, 0.06)] {
                for (i, v) in x.iter_mut().enumerate() {
                    *v += (a * (2.0 * std::f64::consts::PI * f * h * i as f64 / FS as f64).sin()) as f32;
                }
            }
            x
        };
        let cancion = |acordes: &[[f64; 3]], sube: f64| -> Estereo {
            let mut m: Vec<f32> = Vec::new();
            for a in acordes {
                let mut suma = vec![0f32; FS as usize];
                for n in a {
                    for (s, v) in suma.iter_mut().zip(nota(n + sube, 1.0)) {
                        *s += v / 3.0;
                    }
                }
                m.extend(suma);
            }
            [m.clone(), m]
        };
        // Do (60 64 67), Fa (65 69 72), Sol (67 71 74), Do
        let mayor = [[60.0, 64.0, 67.0], [65.0, 69.0, 72.0], [67.0, 71.0, 74.0], [60.0, 64.0, 67.0]];
        // La m (57 60 64), Re m (62 65 69), Mi m (64 67 71), La m
        let menor = [[57.0, 60.0, 64.0], [62.0, 65.0, 69.0], [64.0, 67.0, 71.0], [57.0, 60.0, 64.0]];
        assert_eq!(tonalidad(&cancion(&mayor, 0.0), FS).unwrap().0, "C major");
        assert_eq!(tonalidad(&cancion(&menor, 0.0), FS).unwrap().0, "A minor");
        assert_eq!(tonalidad(&cancion(&mayor, 3.0), FS).unwrap().0, "D# major");
        assert_eq!(tonalidad(&cancion(&menor, 3.0), FS).unwrap().0, "C minor");
        assert!(tonalidad(&[vec![0.0; 96_000], vec![0.0; 96_000]], FS).is_none());
    }

    /// Sonoridad, rango y correlación de señales conocidas.
    #[test]
    fn sonoridad_rango_y_correlacion() {
        // 10 s a −20 dBFS y 10 s a −30: el rango es ~10 LU y la corta máxima, la del tramo fuerte
        let mut m = seno(1000.0, 0.1, 0.0, 10.0);
        m.extend(seno(1000.0, 0.1 * 10f64.powf(-0.5), 0.0, 10.0));
        let x: Estereo = [m.clone(), m];
        let a = analizar(&x, FS);
        assert!((a.rango_lu.unwrap() - 10.0).abs() < 0.3, "rango {:?}", a.rango_lu);
        // un seno de 1 kHz a −20 dBFS de pico en los dos canales: −20 − 3,01 + 3,01 − 0,691 + … ≈ −20 LUFS
        assert!((a.lufs_corto_max.unwrap() + 20.0).abs() < 0.2, "{:?}", a.lufs_corto_max);
        assert!((a.lufs_momentaneo_max.unwrap() + 20.0).abs() < 0.2);
        assert!((a.pico_db + 20.0).abs() < 0.01 && (a.correlacion - 1.0).abs() < 1e-9 && a.continua.abs() < 1e-4);
        assert!((a.segundos - 20.0).abs() < 1e-9);
        // contrafase: −1; canales sin relación: ~0
        let inv: Estereo = [seno(440.0, 0.3, 0.0, 1.0), seno(440.0, -0.3, 0.0, 1.0)];
        assert!((correlacion(&inv) + 1.0).abs() < 1e-9);
        let sin: Estereo = [seno(440.0, 0.3, 0.0, 1.0), seno(557.0, 0.3, 0.0, 1.0)];
        assert!(correlacion(&sin).abs() < 0.01);
        // vacío: no revienta
        assert_eq!(analizar(&[vec![], vec![]], FS).segundos, 0.0);
    }
}
