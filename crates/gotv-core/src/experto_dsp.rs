//! Ola EXPERTO · EXP-2 — el DSP del estudio, en Rust y sin GPU.
//!
//! - **WAV** de entrada (PCM 16/24/32 o coma flotante, también `WAVE_FORMAT_EXTENSIBLE`) y de salida
//!   (coma flotante de 32 bits o PCM de 24).
//! - **EQ**: los biquad de la especificación de Web Audio (`BiquadFilterNode`, las fórmulas del «Audio EQ
//!   Cookbook» de RBJ: graves/agudos con S = 1, campana con Q, paso alto/bajo con la Q en dB), así que el
//!   filtro que se oye y el que se exporta son el MISMO.
//! - **Regiones**: el efecto entra y sale con fundidos de `fundido` segundos dentro del tramo: la
//!   ganancia se interpola en dB, el silencio en lineal, la EQ mezcla seco/filtrado (el filtro corre
//!   continuo sobre toda la pista: sin clics) y el reemplazo cruza a potencia constante.
//! - **Mezcla**: ganancia en dB y paneo con el algoritmo de `StereoPannerNode` para entrada estéreo
//!   (lo que hace la ventana en vivo); grupos (bus con su ganancia y su paneo), silencio y solo.
//! - **Sonoridad**: BS.1770-4 / EBU R128 integrada (filtro K, bloques de 400 ms con 75 % de solape,
//!   puertas −70 LUFS y −10 LU), como `pyloudnorm`.
use crate::experto::{Efecto, Estado, Filtro, Pista, Region};
use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};

/// Una señal estéreo `[izquierdo, derecho]`.
pub type Estereo = [Vec<f32>; 2];

// ───────────────────────────── WAV ─────────────────────────────

/// Lee un WAV: (canales, frecuencia).
pub fn leer_wav(ruta: &Path) -> Result<(Vec<Vec<f32>>, u32)> {
    let b = std::fs::read(ruta).with_context(|| ruta.display().to_string())?;
    if b.len() < 12 || &b[..4] != b"RIFF" || &b[8..12] != b"WAVE" {
        bail!("{}", crate::tr!("{}: no es un WAV", "{}: not a WAV", ruta.display()));
    }
    let (mut i, mut fmt, mut datos) = (12usize, None::<(u16, u16, u32, u16)>, None::<&[u8]>);
    while i + 8 <= b.len() {
        let id = &b[i..i + 4];
        let n = u32::from_le_bytes([b[i + 4], b[i + 5], b[i + 6], b[i + 7]]) as usize;
        let cuerpo = &b[i + 8..(i + 8 + n).min(b.len())];
        if id == b"fmt " && cuerpo.len() >= 16 {
            let u16a = |k: usize| u16::from_le_bytes([cuerpo[k], cuerpo[k + 1]]);
            let mut codigo = u16a(0);
            // WAVE_FORMAT_EXTENSIBLE: el formato real va en el subformato (sus 2 primeros bytes)
            if codigo == 0xFFFE && cuerpo.len() >= 26 {
                codigo = u16a(24);
            }
            fmt = Some((codigo, u16a(2), u32::from_le_bytes([cuerpo[4], cuerpo[5], cuerpo[6], cuerpo[7]]), u16a(14)));
        } else if id == b"data" {
            datos = Some(cuerpo);
        }
        i += 8 + n + (n & 1);
    }
    let (Some((codigo, canales, frecuencia, bits)), Some(d)) = (fmt, datos) else {
        bail!("{}", crate::tr!("{}: WAV sin formato o sin datos", "{}: WAV without format or data", ruta.display()));
    };
    let c = canales.max(1) as usize;
    let muestras: Vec<f32> = match (codigo, bits) {
        (3, 32) => d.chunks_exact(4).map(|x| f32::from_le_bytes([x[0], x[1], x[2], x[3]])).collect(),
        (1, 16) => d.chunks_exact(2).map(|x| i16::from_le_bytes([x[0], x[1]]) as f32 / 32768.0).collect(),
        (1, 24) => d.chunks_exact(3).map(|x| (i32::from_le_bytes([0, x[0], x[1], x[2]]) >> 8) as f32 / 8_388_608.0).collect(),
        (1, 32) => d.chunks_exact(4).map(|x| i32::from_le_bytes([x[0], x[1], x[2], x[3]]) as f32 / 2_147_483_648.0).collect(),
        _ => bail!("{}", crate::tr!("{}: WAV no soportado (formato {codigo}, {bits} bits)", "{}: unsupported WAV (format {codigo}, {bits} bits)", ruta.display())),
    };
    let mut v = vec![Vec::with_capacity(muestras.len() / c); c];
    for (k, m) in muestras.into_iter().enumerate() {
        v[k % c].push(m);
    }
    Ok((v, frecuencia))
}

/// La duración de un WAV en segundos leyendo SÓLO su cabecera (no carga el audio).
pub fn duracion_wav(ruta: &Path) -> Option<f64> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = std::fs::File::open(ruta).ok()?;
    let mut c = [0u8; 12];
    f.read_exact(&mut c).ok()?;
    if &c[..4] != b"RIFF" || &c[8..12] != b"WAVE" {
        return None;
    }
    let mut bytes_por_segundo = 0u32;
    loop {
        let mut h = [0u8; 8];
        f.read_exact(&mut h).ok()?;
        let n = u32::from_le_bytes([h[4], h[5], h[6], h[7]]);
        if &h[..4] == b"fmt " {
            let mut b = vec![0u8; n as usize];
            f.read_exact(&mut b).ok()?;
            bytes_por_segundo = u32::from_le_bytes([b[8], b[9], b[10], b[11]]);
            if n & 1 == 1 {
                f.seek(SeekFrom::Current(1)).ok()?;
            }
        } else if &h[..4] == b"data" {
            return (bytes_por_segundo > 0).then(|| n as f64 / bytes_por_segundo as f64);
        } else {
            f.seek(SeekFrom::Current(n as i64 + (n & 1) as i64)).ok()?;
        }
    }
}

/// Un WAV estéreo, en coma flotante de 32 bits o en PCM de 24 (para los DAW que lo prefieren).
pub fn escribir_wav(ruta: &Path, x: &Estereo, frecuencia: u32, bits24: bool) -> Result<()> {
    let n = x[0].len().min(x[1].len());
    let (codigo, bytes) = if bits24 { (1u16, 3usize) } else { (3u16, 4usize) };
    let datos = (n * 2 * bytes) as u32;
    let mut w = Vec::with_capacity(44 + datos as usize);
    w.extend(b"RIFF");
    w.extend((36 + datos).to_le_bytes());
    w.extend(b"WAVEfmt ");
    w.extend(16u32.to_le_bytes());
    w.extend(codigo.to_le_bytes());
    w.extend(2u16.to_le_bytes());
    w.extend(frecuencia.to_le_bytes());
    w.extend((frecuencia * 2 * bytes as u32).to_le_bytes());
    w.extend(((2 * bytes) as u16).to_le_bytes());
    w.extend(((bytes * 8) as u16).to_le_bytes());
    w.extend(b"data");
    w.extend(datos.to_le_bytes());
    for i in 0..n {
        for c in x {
            if bits24 {
                let v = (c[i].clamp(-1.0, 1.0) as f64 * 8_388_607.0).round() as i32;
                w.extend(&v.to_le_bytes()[..3]);
            } else {
                w.extend(c[i].to_le_bytes());
            }
        }
    }
    if let Some(d) = ruta.parent() {
        std::fs::create_dir_all(d)?;
    }
    std::fs::write(ruta, w).with_context(|| ruta.display().to_string())
}

/// Un WAV estéreo en PCM de 16 bits con TRAMADO triangular (TPDF de ±1 LSB): al bajar a 16 bits, el
/// error de redondeo se vuelve un soplo fijo y no una distorsión que sigue a la música. El ruido es
/// determinista (misma entrada, mismo archivo).
pub fn escribir_wav16(ruta: &Path, x: &Estereo, frecuencia: u32) -> Result<()> {
    let n = x[0].len().min(x[1].len());
    let datos = (n * 4) as u32;
    let mut w = Vec::with_capacity(44 + datos as usize);
    w.extend(b"RIFF");
    w.extend((36 + datos).to_le_bytes());
    w.extend(b"WAVEfmt ");
    w.extend(16u32.to_le_bytes());
    w.extend(1u16.to_le_bytes());
    w.extend(2u16.to_le_bytes());
    w.extend(frecuencia.to_le_bytes());
    w.extend((frecuencia * 4).to_le_bytes());
    w.extend(4u16.to_le_bytes());
    w.extend(16u16.to_le_bytes());
    w.extend(b"data");
    w.extend(datos.to_le_bytes());
    let mut s = 0x9E3779B97F4A7C15u64;
    let mut azar = || {
        s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (s >> 40) as f64 / (1u64 << 24) as f64
    };
    for i in 0..n {
        for c in x {
            let trama = azar() - azar();
            let v = (c[i].clamp(-1.0, 1.0) as f64 * 32_767.0 + trama).round().clamp(-32_768.0, 32_767.0) as i16;
            w.extend(v.to_le_bytes());
        }
    }
    if let Some(d) = ruta.parent() {
        std::fs::create_dir_all(d)?;
    }
    std::fs::write(ruta, w).with_context(|| ruta.display().to_string())
}

/// Un audio del proyecto como estéreo a `frecuencia` (mono → duplicado; más de 2 canales → los 2 primeros).
pub fn leer_estereo(ruta: &Path, frecuencia: u32) -> Result<Estereo> {
    let (mut c, f) = leer_wav(ruta)?;
    if f != frecuencia {
        bail!("{}", crate::tr!("{}: está a {f} Hz y la sesión a {frecuencia} Hz (impórtalo al Experto para convertirlo)", "{}: it is at {f} Hz and the session at {frecuencia} Hz (import it into Expert to convert it)", ruta.display()));
    }
    if c.len() == 1 {
        let m = c[0].clone();
        c.push(m);
    }
    c.truncate(2);
    let d = c.pop().unwrap_or_default();
    let i = c.pop().unwrap_or_default();
    Ok([i, d])
}

// ───────────────────────────── EQ ─────────────────────────────

/// Los coeficientes normalizados (`a0` = 1) de un `BiquadFilterNode`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Biquad {
    pub b0: f64,
    pub b1: f64,
    pub b2: f64,
    pub a1: f64,
    pub a2: f64,
}

impl Biquad {
    /// Las fórmulas de la especificación de Web Audio (§ BiquadFilterNode). `q`: lineal para la campana;
    /// en paso alto/bajo se pasa a dB como espera Web Audio (`Q_dB = 20·log10(q)`); los estantes no la usan.
    pub fn de(filtro: Filtro, frecuencia: f64, db: f64, q: f64, fs: f64) -> Biquad {
        let a = 10f64.powf(db / 40.0);
        let w0 = 2.0 * std::f64::consts::PI * (frecuencia / fs).clamp(0.0, 0.5);
        let (s, c) = (w0.sin(), w0.cos());
        let alfa_q = s / (2.0 * q);
        let q_db = 20.0 * q.log10();
        let alfa_q_db = s / (2.0 * 10f64.powf(q_db / 20.0));
        let alfa_s = s / 2.0 * ((a + 1.0 / a) * (1.0 / 1.0 - 1.0) + 2.0).sqrt();
        let ra = a.sqrt();
        let (b0, b1, b2, a0, a1, a2) = match filtro {
            Filtro::PasoBajo => ((1.0 - c) / 2.0, 1.0 - c, (1.0 - c) / 2.0, 1.0 + alfa_q_db, -2.0 * c, 1.0 - alfa_q_db),
            Filtro::PasoAlto => ((1.0 + c) / 2.0, -(1.0 + c), (1.0 + c) / 2.0, 1.0 + alfa_q_db, -2.0 * c, 1.0 - alfa_q_db),
            Filtro::Campana => (1.0 + alfa_q * a, -2.0 * c, 1.0 - alfa_q * a, 1.0 + alfa_q / a, -2.0 * c, 1.0 - alfa_q / a),
            // las dos de la especificación de Web Audio que faltaban (X2-2)
            Filtro::Muesca => (1.0, -2.0 * c, 1.0, 1.0 + alfa_q, -2.0 * c, 1.0 - alfa_q),
            Filtro::PasoBanda => (alfa_q, 0.0, -alfa_q, 1.0 + alfa_q, -2.0 * c, 1.0 - alfa_q),
            Filtro::Graves => (
                a * ((a + 1.0) - (a - 1.0) * c + 2.0 * alfa_s * ra),
                2.0 * a * ((a - 1.0) - (a + 1.0) * c),
                a * ((a + 1.0) - (a - 1.0) * c - 2.0 * alfa_s * ra),
                (a + 1.0) + (a - 1.0) * c + 2.0 * alfa_s * ra,
                -2.0 * ((a - 1.0) + (a + 1.0) * c),
                (a + 1.0) + (a - 1.0) * c - 2.0 * alfa_s * ra,
            ),
            Filtro::Agudos => (
                a * ((a + 1.0) + (a - 1.0) * c + 2.0 * alfa_s * ra),
                -2.0 * a * ((a - 1.0) + (a + 1.0) * c),
                a * ((a + 1.0) + (a - 1.0) * c - 2.0 * alfa_s * ra),
                (a + 1.0) - (a - 1.0) * c + 2.0 * alfa_s * ra,
                2.0 * ((a - 1.0) - (a + 1.0) * c),
                (a + 1.0) - (a - 1.0) * c - 2.0 * alfa_s * ra,
            ),
        };
        Biquad { b0: b0 / a0, b1: b1 / a0, b2: b2 / a0, a1: a1 / a0, a2: a2 / a0 }
    }

    /// Filtra un canal (forma directa I, estado en f64).
    pub fn filtrar(&self, x: &[f32]) -> Vec<f32> {
        let (mut x1, mut x2, mut y1, mut y2) = (0f64, 0f64, 0f64, 0f64);
        x.iter()
            .map(|&v| {
                let v = v as f64;
                let y = self.b0 * v + self.b1 * x1 + self.b2 * x2 - self.a1 * y1 - self.a2 * y2;
                (x2, x1, y2, y1) = (x1, v, y1, y);
                y as f32
            })
            .collect()
    }
}

// ───────────────────────────── regiones ─────────────────────────────

/// El peso del efecto en la muestra `n` (0 fuera del tramo, 1 dentro, con rampas de `fundido` s).
fn peso(n: usize, fs: f64, desde: f64, hasta: f64, fundido: f64) -> f64 {
    let t = n as f64 / fs;
    if t < desde || t >= hasta {
        return 0.0;
    }
    let f = fundido.max(0.0).min((hasta - desde) / 2.0);
    if f <= 0.0 {
        return 1.0;
    }
    ((t - desde) / f).min((hasta - t) / f).clamp(0.0, 1.0)
}

/// Aplica UNA región a la pista (en su sitio). `root` resuelve el archivo de un reemplazo.
pub fn aplicar_region(x: &mut Estereo, r: &Region, fs: u32, root: &Path) -> Result<()> {
    let fsf = fs as f64;
    let ini = ((r.desde * fsf) as usize).min(x[0].len());
    let fin = ((r.hasta * fsf).ceil() as usize).min(x[0].len());
    if ini >= fin {
        return Ok(());
    }
    match &r.efecto {
        Efecto::Ganancia { db, fundido } => {
            for c in x.iter_mut() {
                for n in ini..fin {
                    let w = peso(n, fsf, r.desde, r.hasta, *fundido as f64);
                    c[n] = (c[n] as f64 * 10f64.powf(*db as f64 * w / 20.0)) as f32;
                }
            }
        }
        Efecto::Silencio { fundido } => {
            for c in x.iter_mut() {
                for n in ini..fin {
                    c[n] = (c[n] as f64 * (1.0 - peso(n, fsf, r.desde, r.hasta, *fundido as f64))) as f32;
                }
            }
        }
        Efecto::Eq { filtro, frecuencia, db, q, fundido } => {
            let b = Biquad::de(*filtro, *frecuencia as f64, *db as f64, *q as f64, fsf);
            for c in x.iter_mut() {
                let y = b.filtrar(c);
                for n in ini..fin {
                    let w = peso(n, fsf, r.desde, r.hasta, *fundido as f64);
                    c[n] = (c[n] as f64 + w * (y[n] as f64 - c[n] as f64)) as f32;
                }
            }
        }
        Efecto::Tono { semitonos, fundido } => {
            // sólo el tramo con 1 s de margen a cada lado (el vocoder necesita contexto; fuera no se toca)
            let margen = fs as usize;
            let (a, b) = (ini.saturating_sub(margen), (fin + margen).min(x[0].len()));
            for c in x.iter_mut() {
                let y = transponer(&c[a..b], *semitonos as f64, fs);
                for n in ini..fin {
                    let w = peso(n, fsf, r.desde, r.hasta, *fundido as f64);
                    c[n] = (c[n] as f64 + w * (y[n - a] as f64 - c[n] as f64)) as f32;
                }
            }
        }
        Efecto::Reemplazo { archivo, fundido, .. } => {
            let otro = leer_estereo(&root.join(archivo), fs)?;
            for (c, o) in x.iter_mut().zip(&otro) {
                for n in ini..fin {
                    let w = peso(n, fsf, r.desde, r.hasta, *fundido as f64) * std::f64::consts::FRAC_PI_2;
                    let v = o.get(n - ini).copied().unwrap_or(0.0) as f64;
                    c[n] = (c[n] as f64 * w.cos() + v * w.sin()) as f32;
                }
            }
        }
    }
    Ok(())
}

/// El trozo de un clip con su tono y su ritmo cambiados. El vocoder es lo más caro del estudio (medido
/// 2026-11-14: 60 s estéreo tardaban 24 s), así que los dos canales van en paralelo y el resultado se guarda
/// en la caché por lo que lo define (archivo, trozo, tono, ritmo, invertido): mover el clip, o tocar
/// cualquier otra cosa de la pista, ya no lo recalcula.
fn clip_transformado(root: &Path, c: &Clip, a: usize, b: usize, trozo: Estereo, fs: u32) -> Estereo {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    huella_archivos(root, std::iter::once(c.archivo.as_str()), &mut h);
    (a, b, c.tono.to_bits(), c.ritmo.to_bits(), c.invertido, fs).hash(&mut h);
    let ruta = root.join(".gotv/experto/cache").join(format!("clip-{:016x}.wav", h.finish()));
    if let Ok(x) = leer_estereo(&ruta, fs) {
        return x;
    }
    let (tono, ritmo) = (c.tono as f64, c.ritmo as f64);
    let [l, r] = &trozo;
    let (yl, yr) = std::thread::scope(|s| {
        let der = s.spawn(|| cambiar_clip(r, tono, ritmo, fs));
        (cambiar_clip(l, tono, ritmo, fs), der.join().unwrap_or_default())
    });
    let x: Estereo = [yl, yr];
    let _ = escribir_wav(&ruta, &x, fs, false);
    x
}

/// X2-1 · La pista MONTADA: sus clips colocados en la línea de tiempo, en orden de capa. Un clip
/// escribe encima de lo que hubiera (tapa); en sus fundidos cruza a potencia constante con lo de
/// debajo (o con el silencio). Sin fundidos, partir un audio y dejar las dos mitades juntas devuelve
/// el audio original muestra a muestra.
pub fn montar_clips(estado: &Estado, pista: &Pista, fs: u32, root: &Path) -> Result<Estereo> {
    let fsf = fs as f64;
    let mut fuentes: std::collections::HashMap<String, Estereo> = std::collections::HashMap::new();
    let mut out: Estereo = [Vec::new(), Vec::new()];
    for c in estado.clips_de(pista.id) {
        if c.silencio {
            continue;
        }
        if !fuentes.contains_key(&c.archivo) {
            fuentes.insert(c.archivo.clone(), leer_estereo(&root.join(&c.archivo), fs)?);
        }
        let f = &fuentes[&c.archivo];
        let total = f[0].len();
        let a = ((c.desde * fsf).round() as usize).min(total);
        // el trozo de archivo que usa: su duración en la línea de tiempo × su ritmo
        let b = match c.duracion {
            Some(d) => (a + (d * c.ritmo as f64 * fsf).round() as usize).min(total),
            None => total,
        };
        if b <= a {
            continue;
        }
        let mut trozo: Estereo = [f[0][a..b].to_vec(), f[1][a..b].to_vec()];
        if c.invertido {
            trozo.iter_mut().for_each(|ch| ch.reverse());
        }
        if c.tono != 0.0 || c.ritmo != 1.0 {
            trozo = clip_transformado(root, c, a, b, trozo, fs);
        }
        let n = trozo[0].len();
        let ini = (c.inicio * fsf).round() as usize;
        if out[0].len() < ini + n {
            out.iter_mut().for_each(|ch| ch.resize(ini + n, 0.0));
        }
        let g = db_a_lineal(c.ganancia_db);
        let fe = ((c.fundido_entrada as f64 * fsf) as usize).min(n);
        let fsal = ((c.fundido_salida as f64 * fsf) as usize).min(n - fe);
        for ch in 0..2 {
            for k in 0..n {
                let v = trozo[ch][k] as f64 * g;
                let w = if k < fe {
                    (k as f64 + 0.5) / fe as f64
                } else if k >= n - fsal {
                    ((n - k) as f64 - 0.5) / fsal as f64
                } else {
                    1.0
                };
                let o = &mut out[ch][ini + k];
                *o = if w >= 1.0 {
                    v as f32
                } else {
                    let t = w * std::f64::consts::FRAC_PI_2;
                    (*o as f64 * t.cos() + v * t.sin()) as f32
                };
            }
        }
    }
    Ok(out)
}

/// La pista montada con sus regiones encendidas (en el orden en que se hicieron), SIN sus insertos.
pub fn pista_con_regiones(estado: &Estado, pista: &Pista, fs: u32, root: &Path) -> Result<Estereo> {
    let mut x = if estado.pista_intacta(pista) { leer_estereo(&root.join(&pista.archivo), fs)? } else { montar_clips(estado, pista, fs, root)? };
    for r in estado.regiones.iter().filter(|r| r.pista == pista.id && !r.apagada) {
        aplicar_region(&mut x, r, fs, root)?;
    }
    Ok(x)
}

/// La pista con sus clips, sus regiones y luego sus insertos. Todo ANTES del fader y del paneo.
pub fn render_pista(estado: &Estado, pista: &Pista, fs: u32, root: &Path) -> Result<Estereo> {
    let mut x = pista_con_regiones(estado, pista, fs, root)?;
    aplicar_insertos(&mut x, &pista.insertos, fs);
    Ok(x)
}

// ───────────────────────────── insertos ─────────────────────────────

use crate::experto::{Automatizacion, Clip, Compresor, Delay, Insertos, Parametro, Puerta, Reverb};

/// El coeficiente de un suavizado exponencial de `ms` milisegundos.
fn coef(ms: f32, fs: u32) -> f64 {
    (-1.0 / (ms.max(0.01) as f64 * 1e-3 * fs as f64)).exp()
}

/// La cadena del canal, en su orden fijo: puerta → EQ → compresor → de-esser → saturación → chorus →
/// delay → reverb → ancho → limitador.
pub fn aplicar_insertos(x: &mut Estereo, i: &Insertos, fs: u32) {
    use crate::experto_fx as fx;
    if let Some(p) = i.puerta {
        puerta(x, &p, fs);
    }
    fx::ecualizar(x, &i.eq, fs);
    if let Some(c) = i.compresor {
        comprimir(x, &c, fs);
    }
    if let Some(d) = i.deesser {
        fx::desesear(x, &d, fs);
    }
    if let Some(s) = i.saturacion {
        fx::saturar(x, &s);
    }
    if let Some(c) = i.chorus {
        fx::corear(x, &c, fs);
    }
    if let Some(d) = i.delay {
        retardar(x, &d, fs);
    }
    if let Some(r) = i.reverb {
        reverberar(x, &r, fs);
    }
    if let Some(a) = i.ancho {
        fx::ensanchar(x, a);
    }
    if let Some(l) = i.limitador {
        fx::limitar(x, &l, fs);
    }
}

/// Compresor de rodilla dura (Giannoulis, Massberg y Reiss, «Digital Dynamic Range Compressor Design»,
/// JAES 2012): la atenuación pedida por el computador de ganancia sobre el pico enlazado de los dos
/// canales pasa por el detector «suave desacoplado» (retén con liberación y luego ataque); después,
/// la compensación.
pub fn comprimir(x: &mut Estereo, c: &Compresor, fs: u32) {
    let (at, li) = (coef(c.ataque_ms, fs), coef(c.liberacion_ms, fs));
    let (umbral, razon, compensa) = (c.umbral_db as f64, c.razon.max(1.0) as f64, c.ganancia_db as f64);
    let (mut y1, mut yl) = (0f64, 0f64);
    let [l, r] = x;
    for (a, b) in l.iter_mut().zip(r.iter_mut()) {
        let pico = (a.abs().max(b.abs()) as f64).max(1e-12);
        let nivel = 20.0 * pico.log10();
        let atenuar = if nivel > umbral { nivel - (umbral + (nivel - umbral) / razon) } else { 0.0 };
        y1 = atenuar.max(li * y1 + (1.0 - li) * atenuar);
        yl = at * yl + (1.0 - at) * y1;
        let g = 10f64.powf((compensa - yl) / 20.0);
        *a = (*a as f64 * g) as f32;
        *b = (*b as f64 * g) as f32;
    }
}

/// Puerta: abierta (1) si el pico supera el umbral, cerrada (0) si no; abre con el ataque y cierra con
/// la liberación (rampas exponenciales, sin clics).
pub fn puerta(x: &mut Estereo, p: &Puerta, fs: u32) {
    let (at, li) = (coef(p.ataque_ms, fs), coef(p.liberacion_ms, fs));
    let umbral = 10f64.powf(p.umbral_db as f64 / 20.0);
    let mut g = 0f64;
    let [l, r] = x;
    for (a, b) in l.iter_mut().zip(r.iter_mut()) {
        let abierta = (a.abs().max(b.abs()) as f64) >= umbral;
        let (objetivo, k) = if abierta { (1.0, at) } else { (0.0, li) };
        g = k * g + (1.0 - k) * objetivo;
        *a = (*a as f64 * g) as f32;
        *b = (*b as f64 * g) as f32;
    }
}

/// Eco: una línea de `ms` por canal con realimentación; sale seco·(1 − mezcla) + eco·mezcla.
pub fn retardar(x: &mut Estereo, d: &Delay, fs: u32) {
    let n = ((d.ms as f64 * 1e-3 * fs as f64).round() as usize).max(1);
    let (fb, m) = (d.realimentacion.clamp(0.0, 0.95) as f64, d.mezcla.clamp(0.0, 1.0) as f64);
    for c in x.iter_mut() {
        let mut linea = vec![0f64; n];
        let mut k = 0;
        for v in c.iter_mut() {
            let eco = linea[k];
            linea[k] = *v as f64 + eco * fb;
            k = (k + 1) % n;
            *v = (*v as f64 * (1.0 - m) + eco * m) as f32;
        }
    }
}

/// Freeverb (Jezar at Dreampoint, dominio público): 8 peines con amortiguación y 4 pasatodos por canal,
/// con las afinaciones originales (a 44,1 kHz, escaladas a `fs`) y el ensanche estéreo de 23 muestras.
pub fn reverberar(x: &mut Estereo, r: &Reverb, fs: u32) {
    const PEINES: [usize; 8] = [1116, 1188, 1277, 1356, 1422, 1491, 1557, 1617];
    const PASATODOS: [usize; 4] = [556, 441, 341, 225];
    const ENSANCHE: usize = 23;
    let esc = fs as f64 / 44_100.0;
    let realim = r.tamano.clamp(0.0, 1.0) as f64 * 0.28 + 0.7;
    let amort = r.amortiguacion.clamp(0.0, 1.0) as f64 * 0.4;
    let m = r.mezcla.clamp(0.0, 1.0) as f64;
    let ganancia_entrada = 0.015;
    for (canal, c) in x.iter_mut().enumerate() {
        let extra = if canal == 1 { ENSANCHE } else { 0 };
        let mut peines: Vec<(Vec<f64>, usize, f64)> = PEINES.iter().map(|&n| (vec![0f64; (((n + extra) as f64 * esc) as usize).max(1)], 0usize, 0f64)).collect();
        let mut pasa: Vec<(Vec<f64>, usize)> = PASATODOS.iter().map(|&n| (vec![0f64; (((n + extra) as f64 * esc) as usize).max(1)], 0usize)).collect();
        for v in c.iter_mut() {
            let entrada = *v as f64 * ganancia_entrada;
            let mut salida = 0f64;
            for (buf, k, filtro) in peines.iter_mut() {
                let y = buf[*k];
                *filtro = y * (1.0 - amort) + *filtro * amort;
                buf[*k] = entrada + *filtro * realim;
                *k = (*k + 1) % buf.len();
                salida += y;
            }
            for (buf, k) in pasa.iter_mut() {
                let b = buf[*k];
                let y = -salida + b;
                buf[*k] = salida + b * 0.5;
                *k = (*k + 1) % buf.len();
                salida = y;
            }
            // `wet` de Freeverb = mezcla · escala 3
            *v = (*v as f64 * (1.0 - m) + salida * m * 3.0) as f32;
        }
    }
}

// ───────────────────────────── transponer ─────────────────────────────

/// FFT compleja radix-2 en su sitio (`inversa` sin normalizar).
pub(crate) fn fft(re: &mut [f64], im: &mut [f64], inversa: bool) {
    let n = re.len();
    let mut j = 0;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut largo = 2;
    while largo <= n {
        let ang = 2.0 * std::f64::consts::PI / largo as f64 * if inversa { 1.0 } else { -1.0 };
        let (wr, wi) = (ang.cos(), ang.sin());
        for i in (0..n).step_by(largo) {
            let (mut cr, mut ci) = (1.0f64, 0.0f64);
            for k in 0..largo / 2 {
                let (a, b) = (i + k, i + k + largo / 2);
                let (tr, ti) = (re[b] * cr - im[b] * ci, re[b] * ci + im[b] * cr);
                re[b] = re[a] - tr;
                im[b] = im[a] - ti;
                re[a] += tr;
                im[a] += ti;
                (cr, ci) = (cr * wr - ci * wi, cr * wi + ci * wr);
            }
        }
        largo <<= 1;
    }
}

/// Suavizado cepstral de un espectro LOGARÍTMICO `log[0..=N/2]`: se quedan los primeros `l` coeficientes.
fn suavizar_cepstro(log: &[f64], n: usize, l: usize) -> Vec<f64> {
    let (mut re, mut im) = (vec![0f64; n], vec![0f64; n]);
    for k in 0..n {
        re[k] = log[if k <= n / 2 { k } else { n - k }];
    }
    fft(&mut re, &mut im, true);
    for (q, (a, b)) in re.iter_mut().zip(im.iter_mut()).enumerate() {
        let guarda = q < l || q > n - l;
        *a = if guarda { *a / n as f64 } else { 0.0 };
        *b = if guarda { *b / n as f64 } else { 0.0 };
    }
    fft(&mut re, &mut im, false);
    re[..=n / 2].to_vec()
}

/// El orden del cepstro para la envolvente de un cuadro, `fs/(2·F0)` como recomiendan Röbel y Rodet, con la
/// F0 por autocorrelación (la inversa del espectro de potencia; entre 70 y 500 Hz). `None` si el cuadro no
/// tiene un tono claro (ruido, percusión, acordes densos): entonces se usa el orden por omisión.
fn orden_cepstro(mag: &[f64], n: usize, fs: u32) -> Option<usize> {
    let (mut re, mut im) = (vec![0f64; n], vec![0f64; n]);
    for k in 0..n {
        let m = mag[if k <= n / 2 { k } else { n - k }];
        re[k] = m * m;
    }
    fft(&mut re, &mut im, true);
    let r0 = re[0];
    if r0 <= 1e-12 {
        return None;
    }
    let (min, max) = ((fs as usize / 500).max(2), (fs as usize / 70).min(n / 2));
    let lag = (min..max).max_by(|a, b| re[*a].total_cmp(&re[*b]))?;
    // la ventana de Hann atenúa la autocorrelación con el retardo: se compensa antes de decidir
    let atenua = 1.0 - lag as f64 / n as f64;
    if re[lag] / r0 / atenua < 0.3 {
        return None;
    }
    Some(((0.5 * lag as f64).round() as usize).clamp(16, n / 4))
}

/// La envolvente espectral (formantes) de `mag[0..=N/2]` por la «envolvente verdadera» (Röbel y Rodet,
/// DAFx 2005): el suavizado cepstral se itera quedándose con el máximo entre el espectro y la envolvente,
/// hasta que pasa por los PICOS armónicos. El cepstro simple sigue el promedio del espectro logarítmico y
/// los valles entre armónicos lo hunden (medido: una resonancia en 700 Hz salía en 422 Hz).
fn envolvente(mag: &[f64], n: usize, l: usize) -> Vec<f64> {
    let original: Vec<f64> = mag.iter().map(|m| (m + 1e-9).ln()).collect();
    let mut a = original.clone();
    let mut env = suavizar_cepstro(&a, n, l);
    for _ in 0..12 {
        for k in 0..=n / 2 {
            a[k] = original[k].max(env[k]);
        }
        env = suavizar_cepstro(&a, n, l);
    }
    env.iter().map(|v| v.exp()).collect()
}

/// Transpone `semitonos` SIN cambiar la duración y SIN volver «aguada» ni robótica la voz:
/// - vocoder de fase (Hann de 2048, 87,5 % de solape) que estira el tiempo por `r = 2^(s/12)`, con BLOQUEO
///   DE FASE por identidad (Laroche y Dolson, 1999): sólo los picos espectrales avanzan su fase por su
///   frecuencia verdadera, y cada bin conserva su relación de fase con su pico (sin bloqueo, los armónicos
///   pierden coherencia y suena metálico, «por un tubo»: lo oyó el Portador el 2026-11-14);
/// - la fase avanza con el salto de síntesis REAL de cada cuadro (el redondeo de un salto no entero
///   añadía aspereza);
/// - PRESERVACIÓN DE FORMANTES: cada bin se corrige por `env(k·r)/env(k)` (envolvente cepstral), así que
///   tras el remuestreo las resonancias de la garganta quedan donde estaban (si no, la voz sube «de
///   ardilla»);
/// - remuestreo cúbico (Hermite) por `1/r`.
pub fn transponer(x: &[f32], semitonos: f64, fs: u32) -> Vec<f32> {
    transponer_con(x, semitonos, fs, true)
}

/// El vocoder de fase: `x` ESTIRADO por `estira` (su duración × `estira`) sin cambiar el tono. Con
/// `formantes` = la razón de tono que vendrá después al remuestrear, la envolvente se precorrige para
/// que las resonancias queden donde estaban.
fn vocoder(x: &[f32], estira: f64, fs: u32, formantes: Option<f64>) -> Vec<f64> {
    const N: usize = 2048;
    // el salto de análisis: 256 mientras el de síntesis no pase de N/4 (hasta ×2); más allá, se encoge
    let ha: usize = ((N as f64 / 4.0 / estira).floor() as usize).clamp(32, 256);
    let hs = ha as f64 * estira;
    let rf = formantes.unwrap_or(1.0);
    let formantes = formantes.is_some();
    if x.len() < N {
        return x.iter().map(|v| *v as f64).collect();
    }
    // el cepstro se corta a ~1,4 ms: guarda la envolvente y deja fuera los armónicos (F0 < ~700 Hz)
    // el orden del cepstro por omisión (sin tono claro): ~1,7 ms, suave pero sin borrar los formantes
    let lifter = ((fs as f64 / 600.0).round() as usize).clamp(8, N / 4);
    let ventana: Vec<f64> = (0..N).map(|n| 0.5 - 0.5 * (2.0 * std::f64::consts::PI * n as f64 / N as f64).cos()).collect();
    let cuadros = (x.len() - N) / ha + 1;
    let largo_y = ((cuadros - 1) as f64 * hs).round() as usize + N + 2;
    let (mut y, mut norma) = (vec![0f64; largo_y], vec![0f64; largo_y]);
    let mut fase_ant = vec![0f64; N / 2 + 1];
    let mut fase_sint = vec![0f64; N / 2 + 1];
    let (mut re, mut im) = (vec![0f64; N], vec![0f64; N]);
    let (mut mag, mut fase, mut verdadera, mut nueva) = (vec![0f64; N / 2 + 1], vec![0f64; N / 2 + 1], vec![0f64; N / 2 + 1], vec![0f64; N / 2 + 1]);
    let mut inicio_ant = 0usize;
    for m in 0..cuadros {
        for n in 0..N {
            re[n] = x[m * ha + n] as f64 * ventana[n];
            im[n] = 0.0;
        }
        fft(&mut re, &mut im, false);
        let inicio = (m as f64 * hs).round() as usize;
        let hs_real = (inicio - inicio_ant) as f64;
        for k in 0..=N / 2 {
            mag[k] = re[k].hypot(im[k]);
            fase[k] = im[k].atan2(re[k]);
            let omega = 2.0 * std::f64::consts::PI * k as f64 / N as f64;
            let mut d = fase[k] - fase_ant[k] - omega * ha as f64;
            d -= 2.0 * std::f64::consts::PI * (d / (2.0 * std::f64::consts::PI)).round();
            verdadera[k] = omega + d / ha as f64;
        }
        // los picos (máximos locales en ±2 bins, por encima del ruido) y la región de cada uno
        let tope = mag.iter().cloned().fold(0.0, f64::max);
        let picos: Vec<usize> = (2..N / 2 - 1).filter(|&k| mag[k] > tope * 1e-5 && mag[k] > mag[k - 1] && mag[k] >= mag[k + 1] && mag[k] > mag[k - 2] && mag[k] >= mag[k + 2]).collect();
        if m == 0 {
            nueva.copy_from_slice(&fase);
        } else if picos.is_empty() {
            for k in 0..=N / 2 {
                nueva[k] = fase_sint[k] + verdadera[k] * hs_real;
            }
        } else {
            let mut j = 0;
            for k in 0..=N / 2 {
                // el pico más cercano (frontera a mitad de camino entre picos)
                while j + 1 < picos.len() && (picos[j + 1] as isize - k as isize).abs() < (picos[j] as isize - k as isize).abs() {
                    j += 1;
                }
                let p = picos[j];
                let fase_pico = fase_sint[p] + verdadera[p] * hs_real;
                nueva[k] = fase_pico + (fase[k] - fase[p]);
            }
        }
        // formantes: la envolvente de este cuadro, para que tras el remuestreo quede en su sitio
        let env = if formantes { envolvente(&mag, N, orden_cepstro(&mag, N, fs).unwrap_or(lifter)) } else { vec![1.0; N / 2 + 1] };
        for k in 0..=N / 2 {
            let kr = k as f64 * rf;
            let destino = if kr >= (N / 2) as f64 {
                env[N / 2]
            } else {
                let (i, f) = (kr.floor() as usize, kr - kr.floor());
                env[i] * (1.0 - f) + env[(i + 1).min(N / 2)] * f
            };
            let g = if formantes { (destino / env[k]).clamp(1.0 / 16.0, 16.0) } else { 1.0 };
            re[k] = mag[k] * g * nueva[k].cos();
            im[k] = mag[k] * g * nueva[k].sin();
            if k > 0 && k < N / 2 {
                re[N - k] = re[k];
                im[N - k] = -im[k];
            }
        }
        fase_ant.copy_from_slice(&fase);
        fase_sint.copy_from_slice(&nueva);
        inicio_ant = inicio;
        fft(&mut re, &mut im, true);
        for n in 0..N {
            y[inicio + n] += re[n] / N as f64 * ventana[n];
            norma[inicio + n] += ventana[n] * ventana[n];
        }
    }
    for (v, w) in y.iter_mut().zip(&norma) {
        if *w > 1e-6 {
            *v /= w;
        }
    }
    y
}

/// `y` en la posición fraccionaria `t` (Hermite cúbico; fuera, cero).
fn hermite(y: &[f64], t: f64) -> f64 {
    let i = t.floor() as isize;
    let f = t - i as f64;
    let p = |k: isize| if i + k < 0 { y.first().copied().unwrap_or(0.0) } else { y.get((i + k) as usize).copied().unwrap_or(0.0) };
    let (a, b, c, d) = (p(-1), p(0), p(1), p(2));
    let c1 = 0.5 * (c - a);
    let c2 = a - 2.5 * b + 2.0 * c - 0.5 * d;
    let c3 = 0.5 * (d - a) + 1.5 * (b - c);
    ((c3 * f + c2) * f + c1) * f + b
}

/// `transponer` con la corrección de formantes a elección (las pruebas la comparan).
pub fn transponer_con(x: &[f32], semitonos: f64, fs: u32, formantes: bool) -> Vec<f32> {
    let r = 2f64.powf(semitonos / 12.0);
    if x.len() < 2048 || semitonos == 0.0 {
        return x.to_vec();
    }
    let y = vocoder(x, r, fs, formantes.then_some(r));
    // remuestreo: la muestra n de la salida es y[n·r]
    (0..x.len()).map(|n| hermite(&y, n as f64 * r) as f32).collect()
}

/// X2-1 · Un clip con otro TONO (`semitonos`) y otro RITMO (`ritmo`: 2 = el doble de rápido y la mitad
/// de largo), cada uno sin tocar al otro: el vocoder estira por `2^(st/12) / ritmo` y el remuestreo por
/// `2^(st/12)` deja el tono pedido y `largo / ritmo` muestras. Con ritmo = `2^(st/12)` no hace falta
/// vocoder: es la cinta que corre más rápido.
pub fn cambiar_clip(x: &[f32], semitonos: f64, ritmo: f64, fs: u32) -> Vec<f32> {
    if (semitonos == 0.0 && ritmo == 1.0) || x.is_empty() || ritmo <= 0.0 {
        return x.to_vec();
    }
    let r = 2f64.powf(semitonos / 12.0);
    let estira = r / ritmo;
    let largo = (x.len() as f64 / ritmo).round() as usize;
    let y: Vec<f64> = if (estira - 1.0).abs() < 1e-9 || x.len() < 2048 { x.iter().map(|v| *v as f64).collect() } else { vocoder(x, estira, fs, (semitonos != 0.0).then_some(r)) };
    // sin vocoder (clip muy corto) el remuestreo lleva todo el cambio
    let paso = if x.len() < 2048 && (estira - 1.0).abs() >= 1e-9 { ritmo } else { r };
    (0..largo).map(|n| hermite(&y, n as f64 * paso) as f32).collect()
}

// ───────────────────────────── mezcla ─────────────────────────────

pub fn db_a_lineal(db: f32) -> f64 {
    10f64.powf(db as f64 / 20.0)
}

/// `StereoPannerNode` con entrada estéreo (especificación de Web Audio).
pub fn panear(x: &mut Estereo, pan: f32) {
    let p = pan.clamp(-1.0, 1.0) as f64;
    let xx = if p <= 0.0 { p + 1.0 } else { p };
    let (gl, gr) = ((xx * std::f64::consts::FRAC_PI_2).cos(), (xx * std::f64::consts::FRAC_PI_2).sin());
    let [l, r] = x;
    for (a, b) in l.iter_mut().zip(r.iter_mut()) {
        let (il, ir) = (*a as f64, *b as f64);
        let (ol, or) = if p <= 0.0 { (il + ir * gl, ir * gr) } else { (il * gl, ir + il * gr) };
        *a = ol as f32;
        *b = or as f32;
    }
}

fn escalar(x: &mut Estereo, g: f64) {
    for c in x.iter_mut() {
        for v in c.iter_mut() {
            *v = (*v as f64 * g) as f32;
        }
    }
}

/// ¿Suena esta pista? (silencio de pista o de su grupo; con algún solo, sólo lo que está en solo)
pub fn suena(estado: &Estado, p: &Pista) -> bool {
    let g = p.grupo.and_then(|id| estado.grupos.iter().find(|g| g.id == id));
    if p.silencio || g.is_some_and(|g| g.silencio) {
        return false;
    }
    let hay_solo = estado.pistas.iter().any(|p| p.solo) || estado.grupos.iter().any(|g| g.solo);
    !hay_solo || p.solo || g.is_some_and(|g| g.solo)
}

/// La curva de una automatización, muestra a muestra (`n` muestras a `fs`).
fn curva(a: &Automatizacion, n: usize, fs: u32) -> Vec<f32> {
    let p = &a.puntos;
    let mut out = Vec::with_capacity(n);
    let mut i = 0; // el primer punto con t > ahora
    for k in 0..n {
        let t = k as f64 / fs as f64;
        while i < p.len() && p[i].t <= t {
            i += 1;
        }
        out.push(if i == 0 {
            p[0].v
        } else if i == p.len() {
            p[i - 1].v
        } else {
            let (a, b) = (p[i - 1], p[i]);
            a.v + (b.v - a.v) * ((t - a.t) / (b.t - a.t)) as f32
        });
    }
    out
}

/// `panear` con un paneo distinto en cada muestra.
fn panear_curva(x: &mut Estereo, pan: &[f32]) {
    let [l, r] = x;
    for ((a, b), p) in l.iter_mut().zip(r.iter_mut()).zip(pan) {
        let p = p.clamp(-1.0, 1.0) as f64;
        let xx = if p <= 0.0 { p + 1.0 } else { p };
        let (gl, gr) = ((xx * std::f64::consts::FRAC_PI_2).cos(), (xx * std::f64::consts::FRAC_PI_2).sin());
        let (il, ir) = (*a as f64, *b as f64);
        let (ol, or) = if p <= 0.0 { (il + ir * gl, ir * gr) } else { (il * gl, ir + il * gr) };
        *a = ol as f32;
        *b = or as f32;
    }
}

/// ¿El grupo de la pista es un BUS con efectos? (entonces sus pistas se suman ANTES de sus insertos)
fn bus_con_efectos<'a>(estado: &'a Estado, p: &Pista) -> Option<&'a crate::experto::Grupo> {
    p.grupo.and_then(|id| estado.grupos.iter().find(|g| g.id == id)).filter(|g| !g.insertos.vacio())
}

/// Lo que aporta una pista: su fader (más su curva de volumen, en dB) y su paneo (o su curva). Si su
/// grupo NO tiene efectos, también el fader y el paneo del grupo: como todo eso es lineal, la suma de
/// estas «pistas finales» ES la mezcla (lo que se exporta como stems). Si el grupo es un bus con
/// efectos, aquí queda lo que ENTRA al bus.
pub fn pista_final(estado: &Estado, p: &Pista, x: Estereo) -> Estereo {
    pista_final_a(estado, p, x, 48_000)
}

pub fn pista_final_a(estado: &Estado, p: &Pista, mut x: Estereo, fs: u32) -> Estereo {
    if !suena(estado, p) {
        for c in x.iter_mut() {
            c.iter_mut().for_each(|v| *v = 0.0);
        }
        return x;
    }
    escalar(&mut x, db_a_lineal(p.ganancia_db));
    if let Some(a) = estado.automatizacion(p.id, Parametro::Volumen) {
        let db = curva(a, x[0].len(), fs);
        for c in x.iter_mut() {
            for (v, d) in c.iter_mut().zip(&db) {
                *v = (*v as f64 * 10f64.powf(*d as f64 / 20.0)) as f32;
            }
        }
    }
    match estado.automatizacion(p.id, Parametro::Pan) {
        Some(a) => {
            let c = curva(a, x[0].len(), fs);
            panear_curva(&mut x, &c)
        }
        None => panear(&mut x, p.pan),
    }
    if bus_con_efectos(estado, p).is_none() {
        if let Some(g) = p.grupo.and_then(|id| estado.grupos.iter().find(|g| g.id == id)) {
            escalar(&mut x, db_a_lineal(g.ganancia_db));
            panear(&mut x, g.pan);
        }
    }
    x
}

/// Suma señales estéreo (del largo de la más larga).
pub fn sumar(partes: &[Estereo]) -> Estereo {
    let n = partes.iter().map(|x| x[0].len()).max().unwrap_or(0);
    let mut s: Estereo = [vec![0f32; n], vec![0f32; n]];
    for x in partes {
        for c in 0..2 {
            for (a, b) in s[c].iter_mut().zip(&x[c]) {
                *a += *b;
            }
        }
    }
    s
}

/// Lo que sale de mezclar la sesión.
pub struct Mezcla {
    /// la mezcla final, con buses y maestro
    pub mezcla: Estereo,
    /// la «pista final» de cada pista, en el orden de `estado.pistas`
    pub finales: Vec<Estereo>,
    /// los buses CON efectos, ya procesados (su número y su señal): sustituyen a sus pistas en la suma
    pub buses: Vec<(u32, Estereo)>,
}

impl Mezcla {
    /// Las partes que SUMAN la mezcla antes del maestro: cada pista libre (o de un grupo sin efectos)
    /// y cada bus con efectos. Con su nombre, para exportarlas.
    pub fn partes<'a>(&'a self, estado: &'a Estado) -> Vec<(String, &'a Estereo)> {
        let mut v: Vec<(String, &Estereo)> = estado.pistas.iter().zip(&self.finales).filter(|(p, _)| bus_con_efectos(estado, p).is_none()).map(|(p, x)| (p.nombre.clone(), x)).collect();
        for (id, x) in &self.buses {
            let nombre = estado.grupos.iter().find(|g| g.id == *id).map(|g| g.nombre.clone()).unwrap_or_default();
            v.push((nombre, x));
        }
        v
    }
}

/// La mezcla entera: pistas → buses (suma, insertos, fader y paneo del bus) → maestro (insertos y
/// ganancia). Sin buses con efectos ni maestro, es la suma de las pistas finales, como siempre.
pub fn mezclar_todo(estado: &Estado, fs: u32, root: &Path) -> Result<Mezcla> {
    let finales: Vec<Estereo> = estado.pistas.iter().map(|p| Ok(pista_final_a(estado, p, render_pista(estado, p, fs, root)?, fs))).collect::<Result<_>>()?;
    let mut partes: Vec<Estereo> = Vec::new();
    let mut buses = Vec::new();
    for g in estado.grupos.iter().filter(|g| !g.insertos.vacio()) {
        let suyas: Vec<Estereo> = estado.pistas.iter().zip(&finales).filter(|(p, _)| p.grupo == Some(g.id)).map(|(_, x)| x.clone()).collect();
        if suyas.is_empty() {
            continue;
        }
        let mut bus = sumar(&suyas);
        aplicar_insertos(&mut bus, &g.insertos, fs);
        escalar(&mut bus, db_a_lineal(g.ganancia_db));
        panear(&mut bus, g.pan);
        buses.push((g.id, bus));
    }
    for (p, x) in estado.pistas.iter().zip(&finales) {
        if bus_con_efectos(estado, p).is_none() {
            partes.push(x.clone());
        }
    }
    partes.extend(buses.iter().map(|(_, x)| x.clone()));
    let mut mezcla = sumar(&partes);
    aplicar_insertos(&mut mezcla, &estado.maestro.insertos, fs);
    if estado.maestro.ganancia_db != 0.0 {
        escalar(&mut mezcla, db_a_lineal(estado.maestro.ganancia_db));
    }
    Ok(Mezcla { mezcla, finales, buses })
}

/// La mezcla y la «pista final» de cada pista (en el orden de `estado.pistas`).
pub fn mezclar(estado: &Estado, fs: u32, root: &Path) -> Result<(Estereo, Vec<Estereo>)> {
    let m = mezclar_todo(estado, fs, root)?;
    Ok((m.mezcla, m.finales))
}

// ───────────────────────────── sonoridad ─────────────────────────────

/// El filtro K de ITU-R BS.1770-4 a cualquier frecuencia: el diseño de Brecht De Man (bilineal con
/// `tan`), que reproduce EXACTA la tabla del ITU a 48 kHz. NO el «K-weighting» por omisión de
/// pyloudnorm, que es una aproximación RBJ (1500 Hz, +4 dB): se juzga contra su clase «DeMan».
pub fn filtro_k(fs: f64) -> (Biquad, Biquad) {
    let estante = {
        let (g, q, fc) = (3.999_843_853_97f64, 0.707_175_236_955_419_3f64, 1_681.974_450_955_531_9f64);
        let k = (std::f64::consts::PI * fc / fs).tan();
        let vh = 10f64.powf(g / 20.0);
        let vb = vh.powf(0.499_666_774_155);
        let a0 = 1.0 + k / q + k * k;
        Biquad { b0: (vh + vb * k / q + k * k) / a0, b1: 2.0 * (k * k - vh) / a0, b2: (vh - vb * k / q + k * k) / a0, a1: 2.0 * (k * k - 1.0) / a0, a2: (1.0 - k / q + k * k) / a0 }
    };
    let rlb = {
        let (q, fc) = (0.500_327_037_325_395_3f64, 38.135_470_876_139_82f64);
        let k = (std::f64::consts::PI * fc / fs).tan();
        let d = 1.0 + k / q + k * k;
        Biquad { b0: 1.0, b1: -2.0, b2: 1.0, a1: 2.0 * (k * k - 1.0) / d, a2: (1.0 - k / q + k * k) / d }
    };
    (estante, rlb)
}

/// Sonoridad integrada en LUFS (BS.1770-4), como `pyloudnorm.Meter(fs, "DeMan").integrated_loudness`.
/// `None` si todo cae bajo la puerta absoluta (silencio).
pub fn lufs(x: &Estereo, fs: u32) -> Option<f64> {
    let fsf = fs as f64;
    let (estante, rlb) = filtro_k(fsf);
    let k: Vec<Vec<f32>> = x.iter().map(|c| rlb.filtrar(&estante.filtrar(c))).collect();
    let n = k[0].len();
    let (tg, paso) = (0.4f64, 0.25f64);
    let t = n as f64 / fsf;
    if t < tg {
        return None;
    }
    let bloques = (((t - tg) / (tg * paso)).round() as usize) + 1;
    let z: Vec<Vec<f64>> = k
        .iter()
        .map(|c| {
            (0..bloques)
                .map(|j| {
                    let l = (tg * (j as f64 * paso) * fsf) as usize;
                    let u = ((tg * (j as f64 * paso + 1.0) * fsf) as usize).min(n);
                    c[l..u].iter().map(|v| (*v as f64).powi(2)).sum::<f64>() / (tg * fsf)
                })
                .collect()
        })
        .collect();
    let suma = |j: usize| z.iter().map(|c| c[j]).sum::<f64>();
    let l: Vec<f64> = (0..bloques).map(|j| -0.691 + 10.0 * suma(j).log10()).collect();
    let abs: Vec<usize> = (0..bloques).filter(|&j| l[j] >= -70.0).collect();
    if abs.is_empty() {
        return None;
    }
    let media = |js: &[usize]| z.iter().map(|c| js.iter().map(|&j| c[j]).sum::<f64>() / js.len() as f64).sum::<f64>();
    let rel = -0.691 + 10.0 * media(&abs).log10() - 10.0;
    let js: Vec<usize> = (0..bloques).filter(|&j| l[j] > rel && l[j] > -70.0).collect();
    if js.is_empty() {
        return None;
    }
    Some(-0.691 + 10.0 * media(&js).log10())
}

/// La ganancia (dB) que lleva la mezcla a `objetivo` LUFS sin pasar el pico de −1 dBFS (si el pico lo
/// impide, se queda corta y lo dice el segundo valor: los dB que faltaron).
pub fn normalizar(x: &Estereo, fs: u32, objetivo: f64) -> Option<(f64, f64)> {
    let l = lufs(x, fs)?;
    let pico = x.iter().flat_map(|c| c.iter()).fold(0f32, |m, v| m.max(v.abs())) as f64;
    let deseado = objetivo - l;
    let tope = if pico > 0.0 { -1.0 - 20.0 * pico.log10() } else { deseado };
    let g = deseado.min(tope);
    Some((g, deseado - g))
}

/// La huella de lo que define el audio de unos archivos: su nombre y cuándo cambiaron.
fn huella_archivos<'a>(root: &Path, archivos: impl Iterator<Item = &'a str>, h: &mut impl std::hash::Hasher) {
    use std::hash::Hash;
    for a in archivos {
        a.hash(h);
        std::fs::metadata(root.join(a)).and_then(|m| m.modified()).ok().map(|t| format!("{t:?}")).hash(h);
    }
}

/// La huella de una pista ANTES de su fader: sus clips, sus regiones (y los archivos de sus
/// reemplazos) y sus insertos.
fn huella_pista(root: &Path, estado: &Estado, p: &Pista, h: &mut impl std::hash::Hasher) {
    use std::hash::Hash;
    let clips: Vec<&Clip> = estado.clips_de(p.id).collect();
    serde_json::to_string(&clips).unwrap_or_default().hash(h);
    huella_archivos(root, clips.iter().map(|c| c.archivo.as_str()), h);
    let regiones: Vec<&Region> = estado.regiones.iter().filter(|r| r.pista == p.id).collect();
    serde_json::to_string(&regiones).unwrap_or_default().hash(h);
    huella_archivos(root, regiones.iter().filter_map(|r| if let Efecto::Reemplazo { archivo, .. } = &r.efecto { Some(archivo.as_str()) } else { None }), h);
    serde_json::to_string(&p.insertos).unwrap_or_default().hash(h);
}

fn en_cache(root: &Path, nombre: String, fs: u32, hacer: impl FnOnce() -> Result<Estereo>) -> Result<PathBuf> {
    let ruta = root.join(".gotv/experto/cache").join(nombre);
    if !ruta.is_file() {
        escribir_wav(&ruta, &hacer()?, fs, false)?;
    }
    Ok(ruta)
}

/// Donde queda la versión renderizada (clips, regiones e insertos aplicados; antes del fader) de una
/// pista, según lo que la define. Si ya existe, se reutiliza.
pub fn pista_en_cache(root: &Path, estado: &Estado, p: &Pista, fs: u32) -> Result<PathBuf> {
    use std::hash::Hasher;
    let mut h = std::collections::hash_map::DefaultHasher::new();
    huella_pista(root, estado, p, &mut h);
    en_cache(root, format!("pista-{}-{:016x}.wav", p.id, h.finish()), fs, || render_pista(estado, p, fs, root))
}

/// El audio de una pista COMO ESTÁ (sus clips y sus regiones, sin sus insertos) en un archivo: el suyo
/// si está intacta y sin regiones; si no, uno en `.gotv/experto/audio/` (no en la caché, que se poda:
/// de él cuelgan las pistas separadas).
pub fn pista_editada(root: &Path, estado: &Estado, p: &Pista, fs: u32) -> Result<PathBuf> {
    use std::hash::Hasher;
    if estado.pista_intacta(p) && !estado.regiones.iter().any(|r| r.pista == p.id && !r.apagada) {
        return Ok(root.join(&p.archivo));
    }
    let mut h = std::collections::hash_map::DefaultHasher::new();
    huella_pista(root, estado, &Pista { insertos: Default::default(), ..p.clone() }, &mut h);
    let ruta = root.join(".gotv/experto/audio").join(format!("editada-{}-{:016x}.wav", p.id, h.finish()));
    if !ruta.is_file() {
        escribir_wav(&ruta, &pista_con_regiones(estado, p, fs, root)?, fs, false)?;
    }
    Ok(ruta)
}

/// La huella de TODA la sesión que suena (cada pista, su mezcla, los buses, las curvas y el maestro).
fn huella_mezcla(root: &Path, estado: &Estado, h: &mut impl std::hash::Hasher) {
    use std::hash::Hash;
    for p in &estado.pistas {
        huella_pista(root, estado, p, h);
        (p.ganancia_db.to_bits(), p.pan.to_bits(), p.silencio, p.solo, p.grupo).hash(h);
    }
    serde_json::to_string(&(&estado.grupos, &estado.automatizaciones, &estado.maestro)).unwrap_or_default().hash(h);
}

/// La MEZCLA FINAL (con buses y maestro) en caché: lo que suena cuando hay efectos que la ventana no
/// puede hacer en vivo, y lo mismo que se exporta.
pub fn mezcla_en_cache(root: &Path, estado: &Estado, fs: u32) -> Result<PathBuf> {
    use std::hash::Hasher;
    let mut h = std::collections::hash_map::DefaultHasher::new();
    huella_mezcla(root, estado, &mut h);
    en_cache(root, format!("mezcla-{:016x}.wav", h.finish()), fs, || Ok(mezclar_todo(estado, fs, root)?.mezcla))
}

/// Deja la carpeta de caché en sus `tope` archivos más recientes (cada edición crea uno nuevo).
pub fn podar_cache(root: &Path, tope: usize) {
    let Ok(d) = std::fs::read_dir(root.join(".gotv/experto/cache")) else { return };
    let mut a: Vec<(std::time::SystemTime, PathBuf)> = d.flatten().filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path()))).collect();
    a.sort_by(|x, y| y.0.cmp(&x.0));
    for (_, p) in a.into_iter().skip(tope) {
        let _ = std::fs::remove_file(p);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::experto::{Grupo, Op, Origen, Sesion};

    fn seno(f: f64, a: f32, s: f64, fs: u32) -> Vec<f32> {
        (0..(s * fs as f64) as usize).map(|n| a * (2.0 * std::f64::consts::PI * f * n as f64 / fs as f64).sin() as f32).collect()
    }

    fn rms(x: &[f32]) -> f64 {
        (x.iter().map(|v| (*v as f64).powi(2)).sum::<f64>() / x.len().max(1) as f64).sqrt()
    }

    #[test]
    fn wav_va_y_vuelve() {
        let d = crate::CarpetaPrueba::sin_crear("gotv-dsp-wav");
        let x: Estereo = [vec![0.0, 0.5, -0.25, 1.0], vec![0.1, -1.0, 0.0, 0.75]];
        escribir_wav(&d.join("f.wav"), &x, 48_000, false).unwrap();
        assert_eq!(leer_estereo(&d.join("f.wav"), 48_000).unwrap(), x);
        escribir_wav(&d.join("p.wav"), &x, 48_000, true).unwrap();
        let y = leer_estereo(&d.join("p.wav"), 48_000).unwrap();
        for c in 0..2 {
            for (a, b) in x[c].iter().zip(&y[c]) {
                assert!((a - b).abs() < 2e-7, "{a} {b}");
            }
        }
        assert!(leer_estereo(&d.join("f.wav"), 44_100).is_err(), "otra frecuencia: se pide importar");
    }

    /// Las respuestas que deben dar los filtros: +6 dB en la campana a su frecuencia, el estante de
    /// agudos sube los agudos y no los graves, el paso alto quita los graves.
    #[test]
    fn la_eq_hace_lo_que_dice() {
        let fs = 48_000;
        let g = |b: &Biquad, f: f64| {
            let x = seno(f, 0.5, 1.0, fs);
            let y = b.filtrar(&x);
            20.0 * (rms(&y[fs as usize / 2..]) / rms(&x[fs as usize / 2..])).log10()
        };
        let campana = Biquad::de(Filtro::Campana, 1000.0, 6.0, 1.0, fs as f64);
        assert!((g(&campana, 1000.0) - 6.0).abs() < 0.05);
        assert!(g(&campana, 100.0).abs() < 0.5);
        let agudos = Biquad::de(Filtro::Agudos, 4000.0, 9.0, 0.707, fs as f64);
        assert!((g(&agudos, 16000.0) - 9.0).abs() < 0.5 && g(&agudos, 100.0).abs() < 0.2);
        let alto = Biquad::de(Filtro::PasoAlto, 200.0, 0.0, 0.707, fs as f64);
        assert!(g(&alto, 30.0) < -20.0 && g(&alto, 5000.0).abs() < 0.1);
    }

    /// «La batería del segundo 5 al 10 más fuerte»: dentro sube los dB pedidos, fuera no cambia nada, y
    /// los bordes son rampas (sin escalón).
    #[test]
    fn una_region_solo_toca_su_tramo() {
        let fs = 48_000u32;
        let base = seno(220.0, 0.25, 12.0, fs);
        let mut x: Estereo = [base.clone(), base.clone()];
        let r = Region { apagada: false, id: 1, pista: 1, desde: 5.0, hasta: 10.0, efecto: Efecto::Ganancia { db: 6.0, fundido: 0.05 } };
        aplicar_region(&mut x, &r, fs, Path::new(".")).unwrap();
        let tramo = |a: f64, b: f64| ((a * fs as f64) as usize, (b * fs as f64) as usize);
        let (a, b) = tramo(6.0, 9.0);
        assert!((20.0 * (rms(&x[0][a..b]) / rms(&base[a..b])).log10() - 6.0).abs() < 0.01);
        let (a, b) = tramo(0.0, 4.9);
        assert_eq!(&x[0][a..b], &base[a..b], "antes del tramo, idéntico");
        let (a, b) = tramo(10.0, 11.9);
        assert_eq!(&x[0][a..b], &base[a..b], "después, idéntico");
        // la rampa: el salto entre muestras vecinas no supera al de la señal amplificada
        let max_salto = x[0].windows(2).map(|w| (w[1] - w[0]).abs()).fold(0f32, f32::max);
        let max_base = base.windows(2).map(|w| (w[1] - w[0]).abs()).fold(0f32, f32::max);
        assert!(max_salto <= max_base * 2.0 + 1e-6);
    }

    /// La suma de las «pistas finales» ES la mezcla; con todo a 0 dB y centrado, la mezcla de las
    /// pistas separadas es la suma de sus archivos. Silencio y solo hacen lo que dicen.
    #[test]
    fn la_suma_de_pistas_es_la_mezcla() {
        let fs = 48_000u32;
        let d = crate::CarpetaPrueba::sin_crear("gotv-dsp-mezcla");
        let ondas: Vec<Estereo> = (0..3).map(|k| [seno(110.0 * (k + 1) as f64, 0.2, 2.0, fs), seno(130.0 * (k + 1) as f64, 0.2, 2.0, fs)]).collect();
        let mut s = Sesion::default();
        for (k, o) in ondas.iter().enumerate() {
            escribir_wav(&d.join(format!("p{k}.wav")), o, fs, false).unwrap();
            let p = Pista { color: None, id: k as u32 + 1, nombre: format!("p{k}"), archivo: format!("p{k}.wav"), origen: Origen::Separada, de: None, fuente: None, ganancia_db: 0.0, pan: 0.0, silencio: false, solo: false, grupo: None, insertos: Default::default() };
            s.aplicar(Op::AgregarPista { pista: p }).unwrap();
        }
        let (m, _) = mezclar(&s.estado(), fs, &d).unwrap();
        let suma = sumar(&ondas);
        assert_eq!(m, suma, "a 0 dB y centrado (pan 0 deja el estéreo intacto), la mezcla es la suma");
        // con grupo, fader, paneo y una región: la suma de las finales sigue siendo la mezcla
        s.aplicar(Op::CrearGrupo { grupo: Grupo { insertos: Default::default(), id: 9, nombre: "G".into(), ganancia_db: -3.0, pan: 0.4, silencio: false, solo: false } }).unwrap();
        s.aplicar(Op::Agrupar { pistas: vec![1, 2], grupo: Some(9) }).unwrap();
        s.aplicar(Op::Mezcla { pista: 3, ganancia_db: Some(2.0), pan: Some(-0.7), silencio: None, solo: None }).unwrap();
        s.aplicar(Op::Region { region: Region { apagada: false, id: 10, pista: 1, desde: 0.5, hasta: 1.5, efecto: Efecto::Eq { filtro: Filtro::Agudos, frecuencia: 3000.0, db: 6.0, q: 0.707, fundido: 0.03 } } }).unwrap();
        let (m, finales) = mezclar(&s.estado(), fs, &d).unwrap();
        let s2 = sumar(&finales);
        let err = (0..2).flat_map(|c| m[c].iter().zip(&s2[c]).map(|(a, b)| (a - b).abs())).fold(0f32, f32::max);
        assert!(err < 1e-6, "{err}");
        // solo en la pista 3: sólo suena ella; silencio del grupo: no suenan 1 y 2
        s.aplicar(Op::Mezcla { pista: 3, ganancia_db: None, pan: None, silencio: None, solo: Some(true) }).unwrap();
        let (_, f) = mezclar(&s.estado(), fs, &d).unwrap();
        assert!(rms(&f[0][0]) == 0.0 && rms(&f[1][0]) == 0.0 && rms(&f[2][0]) > 0.0);
        s.deshacer();
        s.aplicar(Op::MezclaGrupo { grupo: 9, ganancia_db: None, pan: None, silencio: Some(true), solo: None }).unwrap();
        let (_, f) = mezclar(&s.estado(), fs, &d).unwrap();
        assert!(rms(&f[0][0]) == 0.0 && rms(&f[1][0]) == 0.0 && rms(&f[2][0]) > 0.0);
    }

    /// JUEZ (a mano): imprime los coeficientes de 5 filtros para compararlos en el navegador con
    /// `BiquadFilterNode.getFrequencyResponse` (ver NOTAS «EXP-2»).
    #[test]
    #[ignore]
    fn juez_coeficientes_para_el_navegador() {
        let casos = [(Filtro::Graves, 120.0, 6.0, 0.707), (Filtro::Agudos, 6000.0, -4.5, 0.707), (Filtro::Campana, 1000.0, 5.0, 2.0), (Filtro::PasoAlto, 80.0, 0.0, 0.707), (Filtro::PasoBajo, 9000.0, 0.0, 1.4)];
        let v: Vec<serde_json::Value> = casos
            .iter()
            .map(|(f, fr, db, q)| {
                let b = Biquad::de(*f, *fr, *db, *q, 48_000.0);
                serde_json::json!({"filtro": f, "frecuencia": fr, "db": db, "q": q, "c": [b.b0, b.b1, b.b2, b.a1, b.a2]})
            })
            .collect();
        println!("COEFICIENTES {}", serde_json::to_string(&v).unwrap());
    }

    /// JUEZ: la sonoridad integrada de un WAV (`GOTV_LUFS_WAV`), para compararla con pyloudnorm.
    #[test]
    #[ignore]
    fn juez_lufs() {
        let Some(r) = std::env::var_os("GOTV_LUFS_WAV") else { return };
        let (c, fs) = leer_wav(Path::new(&r)).unwrap();
        let x: Estereo = [c[0].clone(), c[c.len().min(2) - 1].clone()];
        println!("LUFS {:.6}", lufs(&x, fs).unwrap());
    }

    /// Compresor: un seno estable 12 dB sobre el umbral con razón 4 sale 9 dB más bajo (12 − 12/4);
    /// bajo el umbral no se toca; la compensación suma lo que dice.
    #[test]
    fn el_compresor_comprime_lo_que_dice() {
        let fs = 48_000;
        let c = Compresor { umbral_db: -24.0, razon: 4.0, ataque_ms: 5.0, liberacion_ms: 100.0, ganancia_db: 0.0 };
        let fuerte = seno(440.0, 10f32.powf(-12.0 / 20.0), 2.0, fs);
        let mut x: Estereo = [fuerte.clone(), fuerte.clone()];
        comprimir(&mut x, &c, fs);
        let cola = fs as usize;
        let reduccion = 20.0 * (rms(&x[0][cola..]) / rms(&fuerte[cola..])).log10();
        assert!((reduccion + 9.0).abs() < 0.3, "reducción {reduccion:.2} dB (esperado −9)");
        let suave = seno(440.0, 10f32.powf(-40.0 / 20.0), 1.0, fs);
        let mut y: Estereo = [suave.clone(), suave.clone()];
        comprimir(&mut y, &Compresor { ganancia_db: 6.0, ..c }, fs);
        assert!((20.0 * (rms(&y[0]) / rms(&suave)).log10() - 6.0).abs() < 0.01, "bajo el umbral: sólo la compensación");
    }

    /// Puerta: lo que no llega al umbral se apaga; lo que pasa, pasa entero.
    #[test]
    fn la_puerta_apaga_el_ruido() {
        let fs = 48_000;
        let mut ruido: Vec<f32> = (0..fs as usize).map(|n| ((n * 7919 % 1000) as f32 / 1000.0 - 0.5) * 0.002).collect();
        ruido.extend(seno(440.0, 0.5, 1.0, fs));
        let mut x: Estereo = [ruido.clone(), ruido.clone()];
        puerta(&mut x, &Puerta { umbral_db: -40.0, ataque_ms: 1.0, liberacion_ms: 50.0 }, fs);
        let (a, b) = (fs as usize / 2, fs as usize);
        assert!(rms(&x[0][a..b]) < 1e-6, "el ruido bajo el umbral queda en silencio");
        let s = &x[0][fs as usize + 4800..];
        assert!((rms(s) / rms(&ruido[fs as usize + 4800..]) - 1.0).abs() < 0.05, "la señal pasa (salvo los pasos por cero)");
    }

    /// Delay: un impulso sale seco, y su eco a los `ms` pedidos con la realimentación como decaimiento.
    #[test]
    fn el_delay_repite_a_su_tiempo() {
        let fs = 48_000;
        let mut imp = vec![0f32; fs as usize];
        imp[0] = 1.0;
        let mut x: Estereo = [imp.clone(), imp];
        retardar(&mut x, &Delay { ms: 250.0, realimentacion: 0.5, mezcla: 0.5 }, fs);
        let n = 12_000;
        assert!((x[0][0] - 0.5).abs() < 1e-6 && (x[0][n] - 0.5).abs() < 1e-6 && (x[0][2 * n] - 0.25).abs() < 1e-6);
        assert!(x[0][n - 1].abs() < 1e-9 && x[0][n + 1].abs() < 1e-9);
        let mut y: Estereo = [seno(440.0, 0.3, 0.5, fs), seno(440.0, 0.3, 0.5, fs)];
        let seco = y.clone();
        retardar(&mut y, &Delay { ms: 100.0, realimentacion: 0.3, mezcla: 0.0 }, fs);
        assert_eq!(y, seco, "mezcla 0: intacto");
    }

    /// Reverb: un impulso deja una cola que DECAE (energía del primer medio segundo > del último), estable
    /// y finita; con mezcla 0, intacto.
    #[test]
    fn la_reverb_deja_una_cola_que_decae() {
        let fs = 48_000;
        let mut imp = vec![0f32; 3 * fs as usize];
        imp[0] = 1.0;
        let mut x: Estereo = [imp.clone(), imp.clone()];
        reverberar(&mut x, &Reverb { tamano: 0.7, amortiguacion: 0.5, mezcla: 1.0 }, fs);
        assert!(x.iter().all(|c| c.iter().all(|v| v.is_finite())));
        let medio = fs as usize / 2;
        let (ini, fin) = (rms(&x[0][..medio]), rms(&x[0][x[0].len() - medio..]));
        assert!(ini > 1e-4 && fin < ini / 10.0, "cola {ini:e} → {fin:e}");
        assert_ne!(x[0], x[1], "el ensanche estéreo separa los canales");
        let mut y: Estereo = [imp.clone(), imp];
        let seco = y.clone();
        reverberar(&mut y, &Reverb { tamano: 0.7, amortiguacion: 0.5, mezcla: 0.0 }, fs);
        assert_eq!(y, seco);
    }

    /// La frecuencia dominante (pico del espectro de 8192 puntos con interpolación parabólica).
    fn frecuencia(x: &[f32], fs: f64) -> f64 {
        let n = 8192;
        let (mut re, mut im): (Vec<f64>, Vec<f64>) = (x[..n].iter().enumerate().map(|(i, v)| *v as f64 * (0.5 - 0.5 * (2.0 * std::f64::consts::PI * i as f64 / n as f64).cos())).collect(), vec![0.0; n]);
        fft(&mut re, &mut im, false);
        let m: Vec<f64> = (0..n / 2).map(|k| re[k].hypot(im[k])).collect();
        let k = (1..n / 2 - 1).max_by(|a, b| m[*a].total_cmp(&m[*b])).unwrap();
        let (a, b, c) = (m[k - 1].ln(), m[k].ln(), m[k + 1].ln());
        (k as f64 + 0.5 * (a - c) / (a - 2.0 * b + c)) * fs / n as f64
    }

    /// Transponer: +12 lleva 440 Hz a 880, −12 a 220, +7 a 659,3; la duración NO cambia y una FFT de ida y
    /// vuelta devuelve la señal.
    #[test]
    fn transponer_cambia_el_tono_y_no_la_duracion() {
        let fs = 48_000u32;
        let x = seno(440.0, 0.4, 2.0, fs);
        for (s, esperado) in [(12.0, 880.0), (-12.0, 220.0), (7.0, 440.0 * 2f64.powf(7.0 / 12.0))] {
            let y = transponer(&x, s, fs);
            assert_eq!(y.len(), x.len(), "misma duración");
            let f = frecuencia(&y[24_000..], fs as f64);
            assert!((f / esperado - 1.0).abs() < 0.004, "{s} semitonos: {f:.1} Hz (esperado {esperado:.1})");
        }
        let (mut re, mut im): (Vec<f64>, Vec<f64>) = ((0..64).map(|i| (i as f64 * 0.3).sin()).collect(), vec![0.0; 64]);
        let orig = re.clone();
        fft(&mut re, &mut im, false);
        fft(&mut re, &mut im, true);
        assert!(re.iter().zip(&orig).all(|(a, b)| (a / 64.0 - b).abs() < 1e-12));
    }

    /// La voz no sube «de ardilla»: una vocal sintética (pulsos a 150 Hz por una resonancia en 700 Hz)
    /// transpuesta +4 semitonos sube su fundamental a 189 Hz pero su FORMANTE se queda cerca de 700 Hz (sin
    /// la corrección se iría a ~882).
    #[test]
    fn transponer_conserva_los_formantes() {
        let fs = 48_000u32;
        let n = 3 * fs as usize;
        let mut pulsos = vec![0f32; n];
        for k in (0..n).step_by(320) {
            pulsos[k] = 1.0; // 48000 / 320 = 150 Hz
        }
        // un formante realista: +18 dB, Q 3 (≈ 230 Hz de ancho)
        let resonancia = Biquad::de(Filtro::Campana, 700.0, 18.0, 3.0, fs as f64);
        let vocal: Vec<f32> = resonancia.filtrar(&pulsos).iter().map(|v| v * 0.05).collect();
        let y = transponer(&vocal, 4.0, fs);
        // el instrumento de medida, validado: la entrada y la salida SIN corregir
        let formante = |x: &[f32]| -> f64 {
            let (mut re, mut im): (Vec<f64>, Vec<f64>) = (x[fs as usize..fs as usize + 8192].iter().enumerate().map(|(i, v)| *v as f64 * (0.5 - 0.5 * (2.0 * std::f64::consts::PI * i as f64 / 8192.0).cos())).collect(), vec![0.0; 8192]);
            fft(&mut re, &mut im, false);
            let mag: Vec<f64> = (0..=4096).map(|k| re[k].hypot(im[k])).collect();
            let env = envolvente(&mag, 8192, fs as usize / 700);
            let hz = |k: usize| k as f64 * fs as f64 / 8192.0;
            hz((0..=4096).filter(|k| (400.0..1300.0).contains(&hz(*k))).max_by(|a, b| env[*a].total_cmp(&env[*b])).unwrap())
        };
        eprintln!("formante: entrada {:.0} Hz · sin corregir {:.0} Hz · corregido {:.0} Hz", formante(&vocal), formante(&transponer_con(&vocal, 4.0, fs, false)), formante(&y));
        // la amplitud (dB) de cada armónico: la verdad sin envolventes de por medio
        let armonicos = |x: &[f32], f0: f64| -> Vec<(f64, f64)> {
            let s = &x[fs as usize..fs as usize + 32_768];
            (1..=8)
                .map(|h| {
                    let f = f0 * h as f64;
                    let (mut c, mut si) = (0f64, 0f64);
                    for (i, v) in s.iter().enumerate() {
                        let w = 0.5 - 0.5 * (2.0 * std::f64::consts::PI * i as f64 / s.len() as f64).cos();
                        let a = 2.0 * std::f64::consts::PI * f * i as f64 / fs as f64;
                        c += *v as f64 * w * a.cos();
                        si += *v as f64 * w * a.sin();
                    }
                    (f, 20.0 * c.hypot(si).log10())
                })
                .collect()
        };
        let fmt = |v: Vec<(f64, f64)>| v.iter().map(|(f, d)| format!("{f:.0}:{d:.1}")).collect::<Vec<_>>().join(" ");
        eprintln!("entrada    {}", fmt(armonicos(&vocal, 150.0)));
        eprintln!("sin corr.  {}", fmt(armonicos(&transponer_con(&vocal, 4.0, fs, false), 188.98)));
        eprintln!("corregido  {}", fmt(armonicos(&y, 188.98)));
        // fundamental por autocorrelación (entre 100 y 300 Hz) en el centro
        let seg = &y[fs as usize..fs as usize + 16_384];
        let ac = |lag: usize| seg.iter().zip(&seg[lag..]).map(|(a, b)| (*a as f64) * (*b as f64)).sum::<f64>();
        let lag = (160..480).max_by(|a, b| ac(*a).total_cmp(&ac(*b))).unwrap();
        let f0 = fs as f64 / lag as f64;
        assert!((f0 / 188.98 - 1.0).abs() < 0.03, "fundamental {f0:.1} Hz (esperada 189)");
        // el criterio: cada armónico de la salida (2.º a 7.º) debe sonar como la ENTRADA en esa frecuencia
        // (la envolvente de la entrada, interpolada entre sus armónicos)
        let entrada = armonicos(&vocal, 150.0);
        let objetivo = |f: f64| -> f64 {
            let i = ((f / 150.0) as usize).clamp(1, 7);
            let (a, b) = (entrada[i - 1], entrada[i]);
            a.1 + (b.1 - a.1) * (f - a.0) / (b.0 - a.0)
        };
        let error = |x: &[f32]| -> f64 {
            let h = armonicos(x, 188.98);
            (h[1..7].iter().map(|(f, d)| (d - objetivo(*f)).powi(2)).sum::<f64>() / 6.0).sqrt()
        };
        let (con, sin) = (error(&y), error(&transponer_con(&vocal, 4.0, fs, false)));
        eprintln!("error de envolvente: corregido {con:.2} dB · sin corregir {sin:.2} dB");
        assert!(con < 2.0 && con < sin / 2.0, "la envolvente no se conserva: {con:.2} dB (sin corregir {sin:.2} dB)");
    }

    /// El filtro K a 48 kHz es la TABLA del ITU-R BS.1770-4 (tabla 1 y 2).
    #[test]
    fn el_filtro_k_es_el_del_itu() {
        let (e, h) = filtro_k(48_000.0);
        let itu_e = [1.535_124_859_586_97, -2.691_696_189_406_38, 1.198_392_810_852_85, -1.690_659_293_182_41, 0.732_480_774_215_85];
        let itu_h = [1.0, -2.0, 1.0, -1.990_047_454_833_98, 0.990_072_250_366_21];
        for (a, b) in [e.b0, e.b1, e.b2, e.a1, e.a2].iter().zip(itu_e).chain([h.b0, h.b1, h.b2, h.a1, h.a2].iter().zip(itu_h)) {
            assert!((a - b).abs() < 1e-8, "{a} vs ITU {b}");
        }
    }

    /// El paneo de Web Audio con entrada estéreo: −1 lleva todo a la izquierda, 0 no cambia nada.
    #[test]
    fn el_paneo_es_el_de_web_audio() {
        let mut x: Estereo = [vec![0.5], vec![0.25]];
        panear(&mut x, 0.0);
        assert_eq!(x, [vec![0.5], vec![0.25]]);
        panear(&mut x, -1.0);
        assert!((x[0][0] - 0.75).abs() < 1e-7 && x[1][0].abs() < 1e-7);
    }

    /// Un seno de 1 kHz a −20 dBFS en los dos canales: BS.1770 da ≈ −20,0 + 3,01 − 0,691 + K(1 kHz)…
    /// se compara con pyloudnorm en el juez; aquí, la coherencia: +6 dB de ganancia → +6 LU, y el
    /// silencio no tiene sonoridad.
    #[test]
    fn la_sonoridad_es_coherente() {
        let fs = 48_000;
        let x: Estereo = [seno(1000.0, 0.1, 5.0, fs), seno(1000.0, 0.1, 5.0, fs)];
        let a = lufs(&x, fs).unwrap();
        let mut y = x.clone();
        escalar(&mut y, db_a_lineal(6.0));
        assert!((lufs(&y, fs).unwrap() - a - 6.0).abs() < 1e-6);
        assert!(lufs(&[vec![0.0; 96_000], vec![0.0; 96_000]], fs).is_none());
        let (g, falto) = normalizar(&x, fs, -14.0).unwrap();
        assert!(falto >= 0.0 && g > 0.0);
    }

    // ───────────── Ola EXPERTO II ─────────────

    /// Ruido determinista en −0,5…0,5.
    fn ruido(n: usize, semilla: u64) -> Vec<f32> {
        let mut s = semilla;
        (0..n)
            .map(|_| {
                s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                (((s >> 33) as f64 / (1u64 << 31) as f64 - 1.0) * 0.5) as f32
            })
            .collect()
    }

    fn sesion_con(d: &crate::CarpetaPrueba, nombre: &str, x: &Estereo, con_raiz: bool) -> Sesion {
        escribir_wav(&d.join(format!("{nombre}.wav")), x, 48_000, false).unwrap();
        let mut s = Sesion::default();
        if con_raiz {
            s.raiz = Some(d.to_path_buf());
        }
        let p = Pista { color: None, id: 1, nombre: nombre.into(), archivo: format!("{nombre}.wav"), origen: Origen::Importada, de: None, fuente: None, ganancia_db: 0.0, pan: 0.0, silencio: false, solo: false, grupo: None, insertos: Default::default() };
        s.aplicar(Op::AgregarPista { pista: p }).unwrap();
        s
    }

    fn render(s: &Sesion, d: &crate::CarpetaPrueba) -> Estereo {
        let e = s.estado();
        render_pista(&e, &e.pistas[0], 48_000, d).unwrap()
    }

    /// X2-1 · Partir un audio y dejar los trozos juntos devuelve el audio MUESTRA A MUESTRA (con la
    /// raíz conocida y sin ella); mover desplaza exacto; quitar deja silencio; duplicar copia.
    #[test]
    fn partir_mover_quitar_y_duplicar_clips_es_exacto() {
        let fs = 48_000usize;
        for con_raiz in [true, false] {
            let d = crate::CarpetaPrueba::sin_crear(if con_raiz { "gotv-dsp-clips-a" } else { "gotv-dsp-clips-b" });
            let x: Estereo = [ruido(2 * fs, 7), ruido(2 * fs, 8)];
            let mut s = sesion_con(&d, "a", &x, con_raiz);
            let e = s.estado();
            assert_eq!(e.clips.len(), 1);
            assert_eq!(e.clips[0].duracion, con_raiz.then_some(2.0), "con la raíz, el clip sabe cuánto dura");
            assert!(e.pista_intacta(&e.pistas[0]));
            assert_eq!(render(&s, &d), x);
            // dos cortes: tres clips que juntos son el original
            s.aplicar(Op::Partir { pista: 1, segundo: 0.7, clip: None }).unwrap();
            s.aplicar(Op::Partir { pista: 1, segundo: 1.3, clip: None }).unwrap();
            let e = s.estado();
            assert_eq!(e.clips.iter().map(|c| c.id).collect::<Vec<_>>(), vec![1, 2, 3]);
            assert_eq!(e.clips.iter().map(|c| (c.inicio, c.desde)).collect::<Vec<_>>(), vec![(0.0, 0.0), (0.7, 0.7), (1.3, 1.3)]);
            assert!(!e.pista_intacta(&e.pistas[0]));
            assert_eq!(render(&s, &d), x, "partir no cambia el sonido");
            // un corte que no cruza ningún clip se rechaza y no entra en la lista
            let n = s.ops.len();
            assert!(s.aplicar(Op::Partir { pista: 1, segundo: 0.7, clip: None }).is_err());
            assert!(s.aplicar(Op::Partir { pista: 1, segundo: 5.0, clip: Some(1) }).is_err());
            assert_eq!(s.ops.len(), n);
            // mover el del medio (0,7–1,3) a 3 s: deja un hueco de silencio y aparece igual allá
            s.aplicar(Op::MoverClip { clip: 2, inicio: 3.0, pista: None }).unwrap();
            let y = render(&s, &d);
            let (a, b, c) = ((0.7 * fs as f64) as usize, (1.3 * fs as f64) as usize, 3 * fs);
            assert_eq!(y[0].len(), c + (b - a));
            assert_eq!(&y[0][..a], &x[0][..a]);
            assert!(y[0][a..b].iter().all(|v| *v == 0.0), "el hueco es silencio");
            assert_eq!(&y[0][b..2 * fs], &x[0][b..]);
            assert_eq!(&y[1][c..], &x[1][a..b], "el clip movido suena igual en su sitio nuevo");
            // duplicarlo en el hueco lo rellena: vuelve el original en 0–2 s
            s.aplicar(Op::DuplicarClip { clip: 2, inicio: 0.7, pista: None }).unwrap();
            let y = render(&s, &d);
            assert_eq!(&y[0][..2 * fs], &x[0][..]);
            // quitar el movido: la pista vuelve a durar 2 s
            s.aplicar(Op::QuitarClip { clip: 2 }).unwrap();
            assert_eq!(render(&s, &d), x);
            // todo se deshace: cinco pasos atrás, un solo clip intacto
            for _ in 0..5 {
                assert!(s.deshacer());
            }
            assert!(s.estado().pista_intacta(&s.estado().pistas[0]));
        }
    }

    /// X2-1 · Recortar, ganancia, fundidos, silencio, invertir, tono y ritmo de un clip.
    #[test]
    fn los_ajustes_de_un_clip_hacen_lo_que_dicen() {
        let fs = 48_000usize;
        let d = crate::CarpetaPrueba::sin_crear("gotv-dsp-clip-ajustes");
        let x: Estereo = [seno(220.0, 0.5, 2.0, 48_000), seno(220.0, 0.5, 2.0, 48_000)];
        let mut s = sesion_con(&d, "a", &x, true);
        // recortar: suena de 0,5 a 1,5 del archivo, colocado en 0,25
        s.aplicar(Op::RecortarClip { clip: 1, inicio: 0.25, desde: 0.5, duracion: 1.0 }).unwrap();
        let y = render(&s, &d);
        assert_eq!(y[0].len(), (1.25 * fs as f64) as usize);
        assert!(y[0][..fs / 4].iter().all(|v| *v == 0.0));
        assert_eq!(&y[0][fs / 4..], &x[0][fs / 2..fs / 2 + fs]);
        // ganancia −6 dB y fundido de entrada de 100 ms: empieza en ~0 y sube
        s.aplicar(Op::ClipAjustes { clip: 1, ganancia_db: Some(-6.0), fundido_entrada: Some(0.1), fundido_salida: Some(0.2), tono: None, ritmo: None, invertido: None, silencio: None, nombre: Some(" Coro ".into()) }).unwrap();
        let y = render(&s, &d);
        let trozo = &y[0][fs / 4..];
        let g = 10f64.powf(-6.0 / 20.0);
        assert!((rms(&trozo[fs / 2..fs / 2 + 4800]) / rms(&x[0][fs..fs + 4800]) - g).abs() < 1e-4, "en medio, −6 dB");
        assert!(rms(&trozo[..480]) < rms(&trozo[2400..2880]) && rms(&trozo[2400..2880]) < rms(&trozo[4800..5280]), "la entrada sube");
        assert!(rms(&trozo[fs - 480..]) < 0.1 * rms(&trozo[fs / 2..fs / 2 + 480]), "la salida baja");
        assert_eq!(s.estado().clips[0].nombre, "Coro");
        // en silencio no suena; invertido suena al revés
        s.aplicar(Op::ClipAjustes { clip: 1, ganancia_db: Some(0.0), fundido_entrada: Some(0.0), fundido_salida: Some(0.0), tono: None, ritmo: None, invertido: None, silencio: Some(true), nombre: None }).unwrap();
        assert!(render(&s, &d)[0].iter().all(|v| *v == 0.0));
        s.aplicar(Op::ClipAjustes { clip: 1, ganancia_db: None, fundido_entrada: None, fundido_salida: None, tono: None, ritmo: None, invertido: Some(true), silencio: Some(false), nombre: None }).unwrap();
        let y = render(&s, &d);
        let al_reves: Vec<f32> = x[0][fs / 2..fs / 2 + fs].iter().rev().copied().collect();
        assert_eq!(&y[0][fs / 4..], &al_reves[..]);
        // partir un clip invertido: los dos trozos siguen siendo el mismo sonido
        s.aplicar(Op::Partir { pista: 1, segundo: 0.6, clip: None }).unwrap();
        assert_eq!(render(&s, &d), y);
        s.deshacer();
        s.deshacer();
        // ritmo 2: la mitad de largo y el mismo tono; tono +12: mismo largo, una octava arriba
        s.aplicar(Op::ClipAjustes { clip: 1, ganancia_db: None, fundido_entrada: None, fundido_salida: None, tono: None, ritmo: Some(2.0), invertido: None, silencio: Some(false), nombre: None }).unwrap();
        assert_eq!(s.estado().clips[0].duracion, Some(0.5), "al doble de ritmo, el mismo trozo dura la mitad");
        let y = render(&s, &d);
        assert_eq!(y[0].len(), fs / 4 + fs / 2);
        assert!((frecuencia(&y[0][fs / 4 + 4800..], 48_000.0) - 220.0).abs() < 2.0);
        s.aplicar(Op::ClipAjustes { clip: 1, ganancia_db: None, fundido_entrada: None, fundido_salida: None, tono: Some(12.0), ritmo: Some(1.0), invertido: None, silencio: None, nombre: None }).unwrap();
        let y = render(&s, &d);
        assert_eq!(y[0].len(), fs / 4 + fs);
        assert!((frecuencia(&y[0][fs / 4 + 4800..], 48_000.0) - 440.0).abs() < 3.0);
        // ritmo 0,5 (el doble de largo) con el vocoder: el tono no cambia
        s.aplicar(Op::ClipAjustes { clip: 1, ganancia_db: None, fundido_entrada: None, fundido_salida: None, tono: Some(0.0), ritmo: Some(0.5), invertido: None, silencio: None, nombre: None }).unwrap();
        let y = render(&s, &d);
        assert_eq!(y[0].len(), fs / 4 + 2 * fs);
        assert!((frecuencia(&y[0][fs / 4 + 9600..], 48_000.0) - 220.0).abs() < 2.0);
        // lo que no vale, se rechaza
        assert!(s.aplicar(Op::ClipAjustes { clip: 1, ganancia_db: None, fundido_entrada: None, fundido_salida: None, tono: Some(30.0), ritmo: None, invertido: None, silencio: None, nombre: None }).is_err());
        assert!(s.aplicar(Op::RecortarClip { clip: 1, inicio: -1.0, desde: 0.0, duracion: 1.0 }).is_err());
        assert!(s.aplicar(Op::MoverClip { clip: 99, inicio: 0.0, pista: None }).is_err());
        assert!(s.aplicar(Op::MoverClip { clip: 1, inicio: 0.0, pista: Some(42) }).is_err());
    }

    /// X2-1 · Un clip posterior TAPA al anterior donde se pisan, y con fundido cruza a potencia constante.
    #[test]
    fn un_clip_encima_tapa_al_de_abajo() {
        let fs = 48_000usize;
        let d = crate::CarpetaPrueba::sin_crear("gotv-dsp-clip-capas");
        let x: Estereo = [vec![0.5; 2 * fs], vec![0.5; 2 * fs]];
        let mut s = sesion_con(&d, "a", &x, true);
        escribir_wav(&d.join("b.wav"), &[vec![-0.25; fs], vec![-0.25; fs]], 48_000, false).unwrap();
        let mut encima = crate::experto::Clip::entero(0, 1, "b.wav", Some(1.0));
        encima.inicio = 0.5;
        s.aplicar(Op::AgregarClip { clip: encima }).unwrap();
        let y = render(&s, &d);
        assert!(y[0][..fs / 2].iter().all(|v| *v == 0.5) && y[0][fs / 2..fs / 2 + fs].iter().all(|v| *v == -0.25) && y[0][fs / 2 + fs..].iter().all(|v| *v == 0.5));
        // con 100 ms de fundido de entrada: a mitad del fundido, las dos a −3 dB (potencia constante)
        s.aplicar(Op::ClipAjustes { clip: 2, ganancia_db: None, fundido_entrada: Some(0.1), fundido_salida: None, tono: None, ritmo: None, invertido: None, silencio: None, nombre: None }).unwrap();
        let y = render(&s, &d);
        let medio = y[0][fs / 2 + 2400] as f64;
        let esperado = (0.5 - 0.25) * std::f64::consts::FRAC_1_SQRT_2;
        assert!((medio - esperado).abs() < 1e-3, "{medio} vs {esperado}");
        // mover el de abajo lo pone ENCIMA
        s.aplicar(Op::MoverClip { clip: 1, inicio: 0.0, pista: None }).unwrap();
        assert!(render(&s, &d)[0].iter().all(|v| *v == 0.5));
    }

    /// X2-2 · La automatización interpola exacto, el bus procesa la SUMA de sus pistas y el maestro va al
    /// final; las «partes» siguen sumando la mezcla antes del maestro. Una región apagada no suena.
    #[test]
    fn automatizacion_buses_y_maestro() {
        use crate::experto::{Limitador, Parametro, Punto};
        let fs = 48_000u32;
        let n = fs as usize;
        let d = crate::CarpetaPrueba::sin_crear("gotv-dsp-buses");
        let mut s = Sesion::default();
        for k in 0..3u32 {
            escribir_wav(&d.join(format!("p{k}.wav")), &[vec![0.4; 2 * n], vec![0.4; 2 * n]], fs, false).unwrap();
            let p = Pista { color: None, id: k + 1, nombre: format!("p{k}"), archivo: format!("p{k}.wav"), origen: Origen::Importada, de: None, fuente: None, ganancia_db: 0.0, pan: 0.0, silencio: false, solo: false, grupo: None, insertos: Default::default() };
            s.aplicar(Op::AgregarPista { pista: p }).unwrap();
        }
        // volumen de la pista 1: de 0 dB en 0 s a −12 dB en 1 s, y ahí se queda
        s.aplicar(Op::Automatizar { pista: 1, parametro: Parametro::Volumen, puntos: vec![Punto { t: 1.0, v: -12.0 }, Punto { t: 0.0, v: 0.0 }] }).unwrap();
        let e = s.estado();
        assert_eq!(e.automatizaciones[0].puntos[0].t, 0.0, "los puntos quedan ordenados");
        let f = pista_final_a(&e, &e.pistas[0], render_pista(&e, &e.pistas[0], fs, &d).unwrap(), fs);
        for (t, db) in [(0.0, 0.0), (0.25, -3.0), (0.5, -6.0), (1.0, -12.0), (1.7, -12.0)] {
            let v = f[0][(t * fs as f64) as usize] as f64;
            assert!((v - 0.4 * 10f64.powf(db / 20.0)).abs() < 1e-6, "{t} s: {v}");
        }
        assert_eq!(e.automatizacion(1, Parametro::Volumen).unwrap().valor(0.5), -6.0);
        // paneo automatizado: de centro a todo a la derecha
        s.aplicar(Op::Automatizar { pista: 2, parametro: Parametro::Pan, puntos: vec![Punto { t: 0.0, v: 0.0 }, Punto { t: 2.0, v: 1.0 }] }).unwrap();
        let e = s.estado();
        let f = pista_final_a(&e, &e.pistas[1], render_pista(&e, &e.pistas[1], fs, &d).unwrap(), fs);
        assert!((f[0][0] - 0.4).abs() < 1e-6 && f[0][2 * n - 1].abs() < 1e-4 && (f[1][2 * n - 1] - 0.8).abs() < 1e-4);
        // sin puntos, la curva se quita
        s.aplicar(Op::Automatizar { pista: 2, parametro: Parametro::Pan, puntos: vec![] }).unwrap();
        s.aplicar(Op::Automatizar { pista: 1, parametro: Parametro::Volumen, puntos: vec![] }).unwrap();
        assert!(s.estado().automatizaciones.is_empty());
        // bus con limitador a −6 dB sobre las pistas 1 y 2 (suman 0,8): el bus sale a 0,501 y no 0,8
        s.aplicar(Op::CrearGrupo { grupo: Grupo { insertos: Default::default(), id: 9, nombre: "Bus".into(), ganancia_db: 0.0, pan: 0.0, silencio: false, solo: false } }).unwrap();
        s.aplicar(Op::Agrupar { pistas: vec![1, 2], grupo: Some(9) }).unwrap();
        let limite = crate::experto::Insertos { limitador: Some(Limitador { techo_db: -6.0, empuje_db: 0.0, liberacion_ms: 50.0 }), ..Default::default() };
        s.aplicar(Op::InsertosGrupo { grupo: 9, insertos: limite.clone() }).unwrap();
        let e = s.estado();
        let m = mezclar_todo(&e, fs, &d).unwrap();
        let techo = 10f32.powf(-6.0 / 20.0);
        assert_eq!(m.buses.len(), 1);
        assert!((m.buses[0].1[0][n] - techo).abs() < 1e-4, "el bus limita la SUMA: {}", m.buses[0].1[0][n]);
        assert!((m.mezcla[0][n] - (techo + 0.4)).abs() < 1e-4, "bus + pista libre");
        // las partes (pista 3 + bus) suman la mezcla
        let partes = m.partes(&e);
        assert_eq!(partes.iter().map(|(nombre, _)| nombre.as_str()).collect::<Vec<_>>(), vec!["p2", "Bus"]);
        let suma = sumar(&partes.iter().map(|(_, x)| (*x).clone()).collect::<Vec<_>>());
        assert_eq!(suma, m.mezcla);
        // maestro: −6 dB y su propio limitador a −12 dB → nada pasa de −12 − 6 dB
        s.aplicar(Op::Maestro { ganancia_db: Some(-6.0), insertos: Some(crate::experto::Insertos { limitador: Some(Limitador { techo_db: -12.0, empuje_db: 0.0, liberacion_ms: 50.0 }), ..Default::default() }) }).unwrap();
        let m = mezclar_todo(&s.estado(), fs, &d).unwrap();
        let pico = m.mezcla[0].iter().fold(0f32, |a, v| a.max(v.abs()));
        assert!((pico - 10f32.powf(-18.0 / 20.0)).abs() < 1e-4, "{pico}");
        // una región apagada no suena y encendida sí; moverla cambia su tramo
        s.aplicar(Op::Region { region: Region { apagada: false, id: 20, pista: 3, desde: 0.5, hasta: 1.0, efecto: Efecto::Silencio { fundido: 0.0 } } }).unwrap();
        let e = s.estado();
        assert_eq!(render_pista(&e, &e.pistas[2], fs, &d).unwrap()[0][n * 3 / 4], 0.0);
        s.aplicar(Op::RegionAjustes { id: 20, desde: None, hasta: None, apagada: Some(true) }).unwrap();
        let e = s.estado();
        assert_eq!(render_pista(&e, &e.pistas[2], fs, &d).unwrap()[0][n * 3 / 4], 0.4);
        s.aplicar(Op::RegionAjustes { id: 20, desde: Some(1.0), hasta: Some(1.5), apagada: Some(false) }).unwrap();
        let e = s.estado();
        let y = render_pista(&e, &e.pistas[2], fs, &d).unwrap();
        assert!(y[0][n * 3 / 4] == 0.4 && y[0][n * 5 / 4] == 0.0);
        assert!(s.aplicar(Op::RegionAjustes { id: 20, desde: Some(3.0), hasta: Some(2.0), apagada: None }).is_err());
        // la caché de la mezcla cambia con cualquier cosa que suene
        let a = mezcla_en_cache(&d, &s.estado(), fs).unwrap();
        s.aplicar(Op::Maestro { ganancia_db: Some(-3.0), insertos: None }).unwrap();
        let b = mezcla_en_cache(&d, &s.estado(), fs).unwrap();
        assert!(a != b && a.is_file() && b.is_file());
        assert_eq!(mezcla_en_cache(&d, &s.estado(), fs).unwrap(), b);
        podar_cache(&d, 1);
        assert_eq!(std::fs::read_dir(d.join(".gotv/experto/cache")).unwrap().count(), 1);
    }

    /// X2-1 · Quitar un tramo corta en sus bordes y deja silencio; con «cerrar», lo de después se corre.
    #[test]
    fn quitar_un_tramo_y_cerrar_el_hueco() {
        let fs = 48_000usize;
        let d = crate::CarpetaPrueba::sin_crear("gotv-dsp-quitar-tramo");
        let x: Estereo = [ruido(3 * fs, 11), ruido(3 * fs, 12)];
        let mut s = sesion_con(&d, "a", &x, true);
        s.aplicar(Op::QuitarTramo { pista: 1, desde: 1.0, hasta: 2.0, cerrar: false }).unwrap();
        let y = render(&s, &d);
        assert_eq!(y[0].len(), 3 * fs);
        assert_eq!(&y[0][..fs], &x[0][..fs]);
        assert!(y[0][fs..2 * fs].iter().all(|v| *v == 0.0));
        assert_eq!(&y[1][2 * fs..], &x[1][2 * fs..]);
        assert!(s.deshacer());
        s.aplicar(Op::QuitarTramo { pista: 1, desde: 1.0, hasta: 2.0, cerrar: true }).unwrap();
        let y = render(&s, &d);
        assert_eq!(y[0].len(), 2 * fs, "sin hueco: un segundo menos");
        assert_eq!(&y[0][..fs], &x[0][..fs]);
        assert_eq!(&y[0][fs..], &x[0][2 * fs..]);
        // donde no hay nada, se rechaza
        assert!(s.aplicar(Op::QuitarTramo { pista: 1, desde: 10.0, hasta: 11.0, cerrar: false }).is_err());
        assert!(s.aplicar(Op::QuitarTramo { pista: 1, desde: 2.0, hasta: 1.0, cerrar: false }).is_err());
    }
}
