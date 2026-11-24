//! EXP-7 · los DATOS del afinado del separador (sólo para afinar: no viajan con la app; el Portador los
//! autorizó el 2026-11-14 y al usuario le llega el modelo afinado).
//!
//! **Slakh2100 redux** (CC BY 4.0, Zenodo 4599666): un `.tar.gz` de 104 GB con, por canción,
//! `stems/SXX.flac` (mono, 44,1 kHz, 16 bits) y un `metadata.yaml` (`inst_class`, `is_drum`,
//! `audio_rendered` por stem); particiones `train`, `validation`, `test` y `omitted` (los duplicados).
//! Los archivos de una misma canción vienen DESPERDIGADOS por el tar: no se puede afinar al vuelo, hay
//! que extraer. Archivo + extraído no caben juntos en el disco: `extraer_slakh` libera (`F_PUNCHHOLE` de
//! APFS) lo ya leído del archivo mientras extrae. Sólo se guardan los stems y el YAML de
//! `train/validation/test` (fuera `mix.flac`, el MIDI y `omitted`).
//!
//! El ejemplo de entrenamiento es el de `demucs/wav.py` + `solver.augment` del afinado oficial: un trozo
//! al azar de una canción, normalizado por la media y la desviación de la mezcla mono de la canción
//! ENTERA, con el signo de cada fuente al azar (`FlipSign`). Como Slakh es MONO, cada fuente se abre
//! en estéreo con un paneo de igual potencia al azar (el modelo es estéreo; sin esto aprendería que
//! izquierda = derecha).
use crate::musica_fuente::leer;
use anyhow::{Context, Result};
use gotv_engine::{anyhow_tr, bail_tr};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{sync_channel, Receiver};

pub const PARTICIONES: [&str; 3] = ["train", "validation", "test"];

/// Un generador pseudoaleatorio pequeño y reproducible (xorshift64*), para no depender de otro crate.
#[derive(Clone)]
pub struct Azar(u64);

impl Azar {
    pub fn new(semilla: u64) -> Azar {
        Azar(semilla.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }
    pub fn u64(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    /// uniforme en [0, 1)
    pub fn f32(&mut self) -> f32 {
        (self.u64() >> 40) as f32 / (1u64 << 24) as f32
    }
    pub fn hasta(&mut self, n: usize) -> usize {
        (self.u64() % n.max(1) as u64) as usize
    }
}

/// Un hash estable de un texto (para sembrar el azar de cada canción del juez).
pub fn hash(s: &str) -> u64 {
    s.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ b as u64).wrapping_mul(0x100_0000_01b3))
}

// ─────────────────────────────── extraer, liberando el archivo ───────────────────────────────

/// Lee el archivo y, cada GiB, LIBERA (`F_PUNCHHOLE`) lo ya consumido menos un margen de 64 MiB.
struct Liberador {
    f: std::fs::File,
    pos: u64,
    liberado: u64,
    liberar: bool,
    contador: std::sync::Arc<std::sync::atomic::AtomicU64>,
}

impl Read for Liberador {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.f.read(buf)?;
        self.pos += n as u64;
        self.contador.store(self.pos, std::sync::atomic::Ordering::Relaxed);
        if self.liberar && self.pos > self.liberado + (1 << 30) + (64 << 20) {
            let hasta = (self.pos - (64 << 20)) & !((1u64 << 20) - 1);
            agujero(&self.f, self.liberado, hasta - self.liberado)?;
            self.liberado = hasta;
        }
        Ok(n)
    }
}

/// `fcntl(F_PUNCHHOLE)`: devuelve al disco los bloques de `[desde, desde+largo)` (APFS).
fn agujero(f: &std::fs::File, desde: u64, largo: u64) -> std::io::Result<()> {
    use std::os::fd::AsRawFd;
    let p = libc::fpunchhole_t { fp_flags: 0, reserved: 0, fp_offset: desde as libc::off_t, fp_length: largo as libc::off_t };
    // SAFETY: `p` vive durante la llamada y el descriptor es de un archivo abierto para escribir
    let r = unsafe { libc::fcntl(f.as_raw_fd(), libc::F_PUNCHHOLE, &p) };
    if r == -1 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

/// Lo mínimo libre que se respeta en el disco al extraer o bajar datos: por debajo, se detiene.
pub const PISO_DISCO: u64 = 5_000_000_000;

/// Bytes libres en el volumen de `ruta` (`statvfs`); 0 si no se puede medir.
pub fn libre(ruta: &Path) -> u64 {
    use std::os::unix::ffi::OsStrExt;
    let mut r = ruta.to_path_buf();
    while !r.exists() && r.pop() {}
    let Ok(c) = std::ffi::CString::new(r.as_os_str().as_bytes()) else { return 0 };
    let mut s: libc::statvfs = unsafe { std::mem::zeroed() };
    // SAFETY: `c` es una ruta terminada en cero y `s` vive durante la llamada
    if unsafe { libc::statvfs(c.as_ptr(), &mut s) } != 0 {
        return 0;
    }
    s.f_bavail as u64 * s.f_frsize as u64
}

/// Lo que dejó la extracción.
#[derive(Debug, Default, Clone, Copy)]
pub struct Extraccion {
    pub archivos: usize,
    pub bytes: u64,
    pub saltados: usize,
}

/// Extrae de Slakh2100 redux los stems y el YAML de `train/validation/test` a `destino/<partición>/<pista>/`.
/// Con `liberar`, el archivo se va vaciando mientras se lee (ya NO se puede volver a leer: si algo falla a
/// medias, lo extraído queda y el resto hay que volver a bajarlo).
pub fn extraer_slakh(archivo: &Path, destino: &Path, liberar: bool, avance: &mut dyn FnMut(u64, u64, &Extraccion)) -> Result<Extraccion> {
    let mut f = std::fs::OpenOptions::new().read(true).write(liberar).open(archivo).with_context(|| archivo.display().to_string())?;
    let total = f.metadata()?.len();
    let mut cabeza = [0u8; 2];
    f.read_exact(&mut cabeza)?;
    if cabeza != [0x1f, 0x8b] {
        bail_tr!("{}: no empieza como gzip (¿ya se consumió al extraer?)", "{}: does not start as gzip (already consumed by an extraction?)", archivo.display());
    }
    use std::io::Seek;
    f.seek(std::io::SeekFrom::Start(0))?;
    let contador = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let lector = Liberador { f, pos: 0, liberado: 0, liberar, contador: contador.clone() };
    let gz = flate2::read::GzDecoder::new(std::io::BufReader::with_capacity(4 << 20, lector));
    let mut tar = tar::Archive::new(gz);
    let mut hecho = Extraccion::default();
    let mut ultimo = std::time::Instant::now();
    for e in tar.entries()? {
        let mut e = e?;
        let ruta = e.path()?.into_owned();
        let partes: Vec<String> = ruta.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
        // slakh2100_flac_redux/<partición>/<pista>/(metadata.yaml | stems/SXX.flac)
        let sirve = partes.len() >= 4
            && PARTICIONES.contains(&partes[1].as_str())
            && ((partes.len() == 4 && partes[3] == "metadata.yaml") || (partes.len() == 5 && partes[3] == "stems" && partes[4].ends_with(".flac")));
        if !sirve || !e.header().entry_type().is_file() {
            hecho.saltados += 1;
            continue;
        }
        let rel: PathBuf = partes[1..].iter().collect();
        let fin = destino.join(&rel);
        let largo = e.header().size()?;
        if fin.metadata().map(|m| m.len() == largo).unwrap_or(false) {
            continue;
        }
        std::fs::create_dir_all(fin.parent().unwrap())?;
        let tmp = fin.with_extension("part");
        {
            let mut out = std::io::BufWriter::new(std::fs::File::create(&tmp)?);
            std::io::copy(&mut e, &mut out)?;
            out.flush()?;
        }
        std::fs::rename(&tmp, &fin)?;
        hecho.archivos += 1;
        hecho.bytes += largo;
        if ultimo.elapsed().as_secs() >= 30 {
            if libre(destino) < PISO_DISCO {
                bail_tr!("el disco bajó de {} GB libres: se detiene la extracción (lo extraído queda)", "free disk fell below {} GB: extraction stops (what was extracted remains)", PISO_DISCO / 1_000_000_000);
            }
            ultimo = std::time::Instant::now();
            let leido = contador.load(std::sync::atomic::Ordering::Relaxed);
            avance(leido, total, &hecho);
        }
    }
    let leido = contador.load(std::sync::atomic::Ordering::Relaxed);
    avance(leido, total, &hecho);
    Ok(hecho)
}

// ─────────────────────────────── el índice de canciones ───────────────────────────────

/// Un stem de una canción.
#[derive(Clone, Debug)]
pub struct Stem {
    pub ruta: PathBuf,
    pub clase: String,
    pub bateria: bool,
}

/// Una canción de Slakh lista para usar (todos sus stems renderizados están en el disco).
#[derive(Clone, Debug)]
pub struct Cancion {
    pub nombre: String,
    pub stems: Vec<Stem>,
}

/// Lee el `metadata.yaml` de Slakh: por stem, `audio_rendered`, `inst_class` e `is_drum`.
fn leer_yaml(texto: &str) -> Vec<(String, bool, String, bool)> {
    let mut out: Vec<(String, bool, String, bool)> = Vec::new();
    let mut dentro = false;
    for l in texto.lines() {
        if l.starts_with("stems:") {
            dentro = true;
            continue;
        }
        if !dentro {
            continue;
        }
        if !l.starts_with(' ') && !l.trim().is_empty() {
            dentro = false;
            continue;
        }
        let t = l.trim();
        if l.starts_with("  ") && !l.starts_with("    ") && t.ends_with(':') {
            out.push((t.trim_end_matches(':').to_string(), false, String::new(), false));
        } else if let Some(s) = out.last_mut() {
            if let Some(v) = t.strip_prefix("audio_rendered:") {
                s.1 = v.trim() == "true";
            } else if let Some(v) = t.strip_prefix("inst_class:") {
                s.2 = v.trim().to_string();
            } else if let Some(v) = t.strip_prefix("is_drum:") {
                s.3 = v.trim() == "true";
            }
        }
    }
    out
}

/// Las canciones COMPLETAS de una partición (las que tienen todos sus stems renderizados en el disco).
pub fn canciones(datos: &Path, particion: &str) -> Result<(Vec<Cancion>, usize)> {
    let dir = datos.join(particion);
    let mut out = Vec::new();
    let mut incompletas = 0;
    let mut pistas: Vec<PathBuf> = std::fs::read_dir(&dir).with_context(|| dir.display().to_string())?.filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.is_dir()).collect();
    pistas.sort();
    for p in pistas {
        let Ok(y) = std::fs::read_to_string(p.join("metadata.yaml")) else {
            incompletas += 1;
            continue;
        };
        let mut stems = Vec::new();
        let mut falta = false;
        for (s, renderizado, clase, bateria) in leer_yaml(&y) {
            if !renderizado {
                continue;
            }
            let flac = p.join("stems").join(format!("{s}.flac"));
            let ruta = if flac.exists() { flac } else { p.join("stems").join(format!("{s}.wav")) };
            if !ruta.exists() {
                falta = true;
                break;
            }
            stems.push(Stem { ruta, clase, bateria });
        }
        if falta || stems.is_empty() {
            incompletas += 1;
            continue;
        }
        out.push(Cancion { nombre: p.file_name().unwrap().to_string_lossy().into_owned(), stems });
    }
    Ok((out, incompletas))
}

/// A qué fuente del modelo va un stem de Slakh: batería, bajo, guitarra y piano (si el modelo las
/// tiene) y lo demás a «other». La voz nunca (Slakh no tiene).
pub fn fuente_de(fuentes: &[String], clase: &str, bateria: bool) -> Option<usize> {
    let i = |n: &str| fuentes.iter().position(|f| f == n);
    // una clase con el MISMO nombre que una fuente (StemGMD agrupado: kick, snare, cymbals, toms) va a ella
    if let Some(k) = i(clase) {
        return Some(k);
    }
    if bateria {
        return i("drums");
    }
    match clase {
        "Bass" => i("bass"),
        "Guitar" => i("guitar").or_else(|| i("other")),
        "Piano" => i("piano").or_else(|| i("other")),
        _ => i("other"),
    }
}

/// Decodifica una canción y suma sus stems por fuente del modelo: `[S][L]` mono a 44,1 kHz.
pub fn decodificar(c: &Cancion, fuentes: &[String]) -> Result<Vec<Vec<f32>>> {
    use rayon::prelude::*;
    let leidos: Vec<(Option<usize>, Vec<f32>)> = c
        .stems
        .par_iter()
        .map(|s| -> Result<(Option<usize>, Vec<f32>)> {
            let (canales, fr) = leer(&s.ruta)?;
            if fr != crate::separador::FRECUENCIA {
                bail_tr!("{}: {fr} Hz (se esperaba 44100)", "{}: {fr} Hz (expected 44100)", s.ruta.display());
            }
            let n = canales.len().max(1) as f32;
            let l = canales.first().map(|v| v.len()).unwrap_or(0);
            let mono: Vec<f32> = (0..l).map(|i| canales.iter().map(|c| c[i]).sum::<f32>() / n).collect();
            Ok((fuente_de(fuentes, &s.clase, s.bateria), mono))
        })
        .collect::<Result<_>>()?;
    let largo = leidos.iter().map(|(_, v)| v.len()).max().unwrap_or(0);
    let mut out = vec![vec![0f32; largo]; fuentes.len()];
    for (k, v) in leidos {
        if let Some(k) = k {
            for (o, x) in out[k].iter_mut().zip(v) {
                *o += x;
            }
        }
    }
    Ok(out)
}

/// El paneo de igual potencia de cada fuente (centro = 1 y 1; la mezcla mono no cambia en el centro).
fn paneo(fuente: &str, az: &mut Azar) -> (f32, f32) {
    let ancho = match fuente {
        "bass" | "kick" | "snare" => 0.15,
        "vocals" => 0.2,
        "toms" => 0.5,
        "drums" => 0.3,
        _ => 0.6,
    };
    let t = std::f32::consts::FRAC_PI_4 + (az.f32() * 2.0 - 1.0) * ancho;
    (std::f32::consts::SQRT_2 * t.cos(), std::f32::consts::SQRT_2 * t.sin())
}

/// Media y desviación (insesgada) de la mezcla mono de la canción entera (`wav.py::_track_metadata`).
pub fn media_desv_cancion(fuentes: &[Vec<f32>]) -> (f32, f32) {
    let l = fuentes.first().map(|v| v.len()).unwrap_or(0);
    if l < 2 {
        return (0.0, 1.0);
    }
    let mezcla = |i: usize| fuentes.iter().map(|f| f[i] as f64).sum::<f64>();
    let media = (0..l).map(mezcla).sum::<f64>() / l as f64;
    let var = (0..l).map(|i| (mezcla(i) - media).powi(2)).sum::<f64>() / (l - 1) as f64;
    (media as f32, (var.sqrt() as f32).max(1e-5))
}

/// Un ejemplo de entrenamiento: `[S, 2, L]` aplanado (la mezcla es su suma).
pub struct Ejemplo {
    pub fuentes: Vec<f32>,
}

/// Corta `n` trozos de `largo` al azar de una canción ya decodificada, normalizados y en estéreo.
/// Se descartan los casi mudos (rms de la mezcla normalizada < 0,05).
pub fn cortar(c: &[Vec<f32>], nombres: &[String], largo: usize, n: usize, az: &mut Azar) -> Vec<Ejemplo> {
    let l = c.first().map(|v| v.len()).unwrap_or(0);
    if l < largo {
        return Vec::new();
    }
    let (media, desv) = media_desv_cancion(c);
    let s = c.len();
    let mut out = Vec::new();
    for _ in 0..n * 3 {
        if out.len() == n {
            break;
        }
        let a = az.hasta(l - largo + 1);
        let mut v = vec![0f32; s * 2 * largo];
        let mut energia = 0f64;
        for k in 0..s {
            let (gl, gr) = paneo(&nombres[k], az);
            let signo = if az.u64() & 1 == 0 { 1.0 } else { -1.0 };
            for i in 0..largo {
                let x = (c[k][a + i] - media) / desv * signo;
                v[(k * 2) * largo + i] = x * gl;
                v[(k * 2 + 1) * largo + i] = x * gr;
            }
        }
        for i in (0..largo).step_by(7) {
            let m: f32 = (0..s).map(|k| v[(k * 2) * largo + i] + v[(k * 2 + 1) * largo + i]).sum::<f32>() / 2.0;
            energia += (m * m) as f64;
        }
        if (energia / (largo / 7) as f64).sqrt() < 0.05 {
            continue;
        }
        out.push(Ejemplo { fuentes: v });
    }
    out
}

// ─────────────── EXP-7 · VOCES REALES: que el afinado con Slakh (sin voz) no olvide la voz ───────────────
//
// Medido el 2026-11-14: afinar HTDemucs-6s con Slakh y la voz con peso 0 sube la media de prueba de 5,29 a
// 8,65 dB pero hunde la voz de 15,58 a 7,60 dB (y mezclar pesos base/afinado no la salva: con alfa 0,3 ya
// pierde 0,8 dB). Remedio: a la mitad de los trozos se le SUMA una voz real aislada, y la voz se aprende
// con peso 1 (sin voz, su verdad es el silencio: Slakh no tiene voz). Voces: a capelas CC BY de ccMixter y
// VocalSet (CC BY 4.0, 20 cantantes).

/// Las voces de una partición: VocalSet por CANTANTE (female9/male11 validan, female8/male10 prueban) y el
/// resto por el hash del nombre del archivo (0 → test, 1 → validation, el resto → train). Fijo y estable.
pub fn voces(dirs: &[PathBuf], particion: &str) -> Vec<PathBuf> {
    fn recorrer(d: &Path, out: &mut Vec<PathBuf>) {
        let Ok(r) = std::fs::read_dir(d) else { return };
        for e in r.filter_map(|e| e.ok()) {
            let p = e.path();
            if p.is_dir() {
                recorrer(&p, out);
            } else if matches!(p.extension().and_then(|x| x.to_str()).map(|x| x.to_ascii_lowercase()).as_deref(), Some("wav" | "mp3" | "flac" | "ogg")) {
                out.push(p);
            }
        }
    }
    let mut todas = Vec::new();
    for d in dirs {
        recorrer(d, &mut todas);
    }
    todas.sort();
    todas
        .into_iter()
        .filter(|p| {
            let cantante = p.components().filter_map(|c| c.as_os_str().to_str()).find(|c| (c.starts_with("female") || c.starts_with("male")) && c.trim_start_matches("fe").trim_start_matches("male").parse::<u32>().is_ok());
            let cual = match cantante {
                Some("female9" | "male11") => "validation",
                Some("female8" | "male10") => "test",
                Some(_) => "train",
                None => match hash(&p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()) % 10 {
                    0 => "test",
                    1 => "validation",
                    _ => "train",
                },
            };
            cual == particion
        })
        .collect()
}

/// Un trozo de `largo` muestras (a 44,1 kHz, estéreo) de una voz: de varios intentos al azar se queda con
/// el de más energía (las a capelas tienen silencios largos); si el archivo es más corto, cae en un lugar
/// al azar de un trozo mudo. Se corta ANTES de remuestrear (barato aunque la canción dure minutos).
pub fn trozo_de_voz(archivo: &Path, largo: usize, az: &mut Azar) -> Result<[Vec<f32>; 2]> {
    let (canales, f0) = leer(archivo)?;
    let fr = crate::separador::FRECUENCIA;
    let n = canales.first().map(|c| c.len()).unwrap_or(0);
    if n == 0 {
        bail_tr!("{} no tiene audio", "{} has no audio", archivo.display());
    }
    let largo0 = ((largo as u64 * f0 as u64).div_ceil(fr as u64) as usize) + 64;
    let energia = |a: usize, l: usize| -> f64 { canales.iter().map(|c| c[a..a + l].iter().step_by(11).map(|s| (*s as f64).powi(2)).sum::<f64>()).sum() };
    let (a, l) = if n > largo0 {
        let mut mejor = (0usize, -1f64);
        for _ in 0..6 {
            let a = az.hasta(n - largo0 + 1);
            let e = energia(a, largo0);
            if e > mejor.1 {
                mejor = (a, e);
            }
        }
        (mejor.0, largo0)
    } else {
        (0, n)
    };
    let mut out: [Vec<f32>; 2] = std::array::from_fn(|k| {
        let c = &canales[k.min(canales.len() - 1)];
        crate::musica_fuente::remuestrear(&c[a..a + l], f0, fr)
    });
    let hecho = out[0].len().min(largo);
    let desde = if hecho < largo { az.hasta(largo - hecho + 1) } else { 0 };
    for c in out.iter_mut() {
        let mut v = vec![0f32; largo];
        v[desde..desde + hecho].copy_from_slice(&c[..hecho]);
        *c = v;
    }
    Ok(out)
}

/// Suma una voz en la fuente `kv` de un ejemplo aplanado `[S, 2, L]`: su nivel cae al azar entre −8 y +2
/// dB del de la mezcla instrumental (la voz suele ir encima), con paneo leve y signo al azar.
pub fn sumar_voz(fuentes: &mut [f32], s: usize, kv: usize, largo: usize, voz: &[Vec<f32>; 2], az: &mut Azar) {
    let rms_mezcla = {
        let mut e = 0f64;
        for i in (0..largo).step_by(7) {
            for c in 0..2 {
                let m: f32 = (0..s).map(|k| fuentes[(k * 2 + c) * largo + i]).sum();
                e += (m * m) as f64;
            }
        }
        (e / (2 * largo.div_ceil(7)) as f64).sqrt()
    };
    let rms_voz = (voz.iter().flat_map(|c| c.iter().step_by(7)).map(|s| (*s as f64).powi(2)).sum::<f64>() / (2 * largo.div_ceil(7)) as f64).sqrt();
    if rms_voz < 1e-6 || rms_mezcla < 1e-6 {
        return;
    }
    let db = -8.0 + 10.0 * az.f32() as f64;
    let g = (rms_mezcla / rms_voz * 10f64.powf(db / 20.0)) as f32 * if az.u64() & 1 == 0 { 1.0 } else { -1.0 };
    let (gl, gr) = paneo("vocals", az);
    for (c, gc) in [(0usize, gl), (1, gr)] {
        for i in 0..largo {
            fuentes[(kv * 2 + c) * largo + i] = voz[c][i] * g * gc;
        }
    }
}

/// Hilos que decodifican canciones al azar y cortan trozos sin parar (hasta que se suelta el receptor).
pub fn cargador(canciones: Vec<Cancion>, nombres: Vec<String>, largo: usize, por_cancion: usize, hilos: usize, semilla: u64) -> Receiver<Ejemplo> {
    cargador_con_voz(canciones, nombres, largo, por_cancion, hilos, semilla, Vec::new(), 0.0)
}

/// Como [`cargador`], y además a cada trozo le suma, con probabilidad `prob`, una voz de `voces` (si el
/// modelo tiene la fuente `vocals`).
#[allow(clippy::too_many_arguments)]
pub fn cargador_con_voz(canciones: Vec<Cancion>, nombres: Vec<String>, largo: usize, por_cancion: usize, hilos: usize, semilla: u64, voces: Vec<PathBuf>, prob: f32) -> Receiver<Ejemplo> {
    let (tx, rx) = sync_channel::<Ejemplo>(16);
    let canciones = std::sync::Arc::new(canciones);
    let voces = std::sync::Arc::new(voces);
    let kv = nombres.iter().position(|n| n == "vocals");
    for h in 0..hilos {
        let tx = tx.clone();
        let canciones = canciones.clone();
        let nombres = nombres.clone();
        let voces = voces.clone();
        std::thread::spawn(move || {
            let mut az = Azar::new(semilla ^ (h as u64 + 1).wrapping_mul(0xA24B_AED4_963E_E407));
            loop {
                let c = &canciones[az.hasta(canciones.len())];
                let Ok(d) = decodificar(c, &nombres) else { continue };
                for mut e in cortar(&d, &nombres, largo, por_cancion, &mut az) {
                    if let Some(kv) = kv.filter(|_| !voces.is_empty() && az.f32() < prob) {
                        if let Ok(v) = trozo_de_voz(&voces[az.hasta(voces.len())], largo, &mut az) {
                            sumar_voz(&mut e.fuentes, nombres.len(), kv, largo, &v, &mut az);
                        }
                    }
                    if tx.send(e).is_err() {
                        return;
                    }
                }
            }
        });
    }
    rx
}

/// Una reserva que BARAJA lo que llega de los hilos (los trozos de una misma canción salen juntos).
pub struct Barajador {
    rx: Receiver<Ejemplo>,
    reserva: Vec<Ejemplo>,
    az: Azar,
    tam: usize,
}

impl Barajador {
    pub fn new(rx: Receiver<Ejemplo>, tam: usize, semilla: u64) -> Barajador {
        Barajador { rx, reserva: Vec::new(), az: Azar::new(semilla), tam }
    }
    pub fn siguiente(&mut self) -> Result<Ejemplo> {
        while self.reserva.len() < self.tam {
            let e = self.rx.recv().map_err(|_| anyhow_tr!("el cargador de datos se detuvo", "the data loader stopped"))?;
            self.reserva.push(e);
        }
        let i = self.az.hasta(self.reserva.len());
        Ok(self.reserva.swap_remove(i))
    }
}

// ─────────────────────────────── StemGMD: un zip PARTIDO leído por rangos ───────────────────────────────
//
// StemGMD (CC BY 4.0, Zenodo 7860223 + 7882857) es un zip partido en 7 trozos (`.z01`…`.z06` + `.zip`,
// ≈ 211 GB; 103 220 archivos, 814 GB sin comprimir). Su directorio central (al final del último trozo)
// dice en qué trozo y en qué posición empieza cada archivo: con eso se baja por RANGOS sólo lo que se usa.

/// Una entrada del directorio central.
#[derive(Clone, Debug)]
pub struct EntradaZip {
    pub nombre: String,
    pub metodo: u16,
    pub crc: u32,
    pub comprimido: u64,
    pub tam: u64,
    /// trozo donde empieza (0 = el primero)
    pub disco: u32,
    /// posición de su cabecera local dentro de ese trozo
    pub desplazamiento: u64,
}

fn u16_(b: &[u8], i: usize) -> u16 {
    u16::from_le_bytes([b[i], b[i + 1]])
}
fn u32_(b: &[u8], i: usize) -> u32 {
    u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]])
}
fn u64_(b: &[u8], i: usize) -> u64 {
    u64::from_le_bytes(b[i..i + 8].try_into().unwrap())
}

/// El directorio central a partir de la COLA del último trozo (`cola` = sus últimos bytes, que empiezan en
/// `inicio` dentro de ese trozo). Entiende zip64 (desplazamientos de más de 4 GB dentro de un trozo).
pub fn directorio_zip(cola: &[u8], inicio: u64) -> Result<Vec<EntradaZip>> {
    let eocd = (0..cola.len().saturating_sub(21)).rev().find(|&i| u32_(cola, i) == 0x0605_4b50).ok_or_else(|| anyhow_tr!("sin el fin del directorio central del zip", "zip end of central directory not found"))?;
    let (mut cd_off, mut cd_n) = (u32_(cola, eocd + 16) as u64, u16_(cola, eocd + 10) as u64);
    // zip64: el localizador va justo antes del EOCD y apunta al EOCD64
    if eocd >= 20 && u32_(cola, eocd - 20) == 0x0706_4b50 {
        let e64 = u64_(cola, eocd - 20 + 8);
        let i = e64.checked_sub(inicio).ok_or_else(|| anyhow_tr!("el EOCD64 cae antes de la cola bajada", "the EOCD64 falls before the downloaded tail"))? as usize;
        if u32_(cola, i) != 0x0606_4b50 {
            bail_tr!("EOCD64 inválido", "invalid EOCD64");
        }
        cd_n = u64_(cola, i + 32);
        cd_off = u64_(cola, i + 48);
    }
    let mut p = cd_off.checked_sub(inicio).ok_or_else(|| anyhow_tr!("el directorio central empieza antes de la cola bajada: baja más cola", "the central directory starts before the downloaded tail: download more"))? as usize;
    let mut out = Vec::with_capacity(cd_n as usize);
    for _ in 0..cd_n {
        if p + 46 > cola.len() || u32_(cola, p) != 0x0201_4b50 {
            bail_tr!("entrada del directorio central rota en {p}", "broken central directory entry at {p}");
        }
        let (nl, xl, cl) = (u16_(cola, p + 28) as usize, u16_(cola, p + 30) as usize, u16_(cola, p + 32) as usize);
        let mut e = EntradaZip {
            nombre: String::from_utf8_lossy(&cola[p + 46..p + 46 + nl]).into_owned(),
            metodo: u16_(cola, p + 10),
            crc: u32_(cola, p + 16),
            comprimido: u32_(cola, p + 20) as u64,
            tam: u32_(cola, p + 24) as u64,
            disco: u16_(cola, p + 34) as u32,
            desplazamiento: u32_(cola, p + 42) as u64,
        };
        // el extra zip64 (0x0001): tam, comprimido, desplazamiento y disco, sólo los que valen 0xFFFF…
        let mut x = p + 46 + nl;
        let fin_x = x + xl;
        while x + 4 <= fin_x {
            let (id, sz) = (u16_(cola, x), u16_(cola, x + 2) as usize);
            if id == 0x0001 {
                let mut q = x + 4;
                if e.tam == 0xFFFF_FFFF {
                    e.tam = u64_(cola, q);
                    q += 8;
                }
                if e.comprimido == 0xFFFF_FFFF {
                    e.comprimido = u64_(cola, q);
                    q += 8;
                }
                if e.desplazamiento == 0xFFFF_FFFF {
                    e.desplazamiento = u64_(cola, q);
                    q += 8;
                }
                if e.disco == 0xFFFF {
                    e.disco = u32_(cola, q);
                }
            }
            x += 4 + sz;
        }
        out.push(e);
        p = fin_x + cl;
    }
    Ok(out)
}

/// Cuántos bytes hay que bajar desde la cabecera local para tener la entrada entera (con holgura para un
/// extra local distinto del central).
pub fn bytes_de_entrada(e: &EntradaZip) -> u64 {
    30 + e.nombre.len() as u64 + 1024 + e.comprimido
}

/// Descomprime una entrada a partir de sus bytes (desde la cabecera local) y comprueba su CRC-32.
pub fn leer_entrada(b: &[u8], e: &EntradaZip) -> Result<Vec<u8>> {
    if b.len() < 30 || u32_(b, 0) != 0x0403_4b50 {
        bail_tr!("{}: sin cabecera local", "{}: no local header", e.nombre);
    }
    let datos = 30 + u16_(b, 26) as usize + u16_(b, 28) as usize;
    let fin = datos + e.comprimido as usize;
    if b.len() < fin {
        bail_tr!("{}: faltan bytes ({} de {fin})", "{}: missing bytes ({} of {fin})", e.nombre, b.len());
    }
    let crudo = &b[datos..fin];
    let out = match e.metodo {
        0 => crudo.to_vec(),
        8 => {
            let mut v = Vec::with_capacity(e.tam as usize);
            flate2::read::DeflateDecoder::new(crudo).read_to_end(&mut v)?;
            v
        }
        m => bail_tr!("{}: método de compresión {m} no soportado", "{}: compression method {m} not supported", e.nombre),
    };
    let mut crc = flate2::Crc::new();
    crc.update(&out);
    if out.len() as u64 != e.tam || crc.sum() != e.crc {
        bail_tr!("{}: CRC o tamaño no cuadran", "{}: CRC or size mismatch", e.nombre);
    }
    Ok(out)
}

/// Los grupos de la batería que pidió el Portador (bombo, caja, platos y toms) y a cuál va cada clase
/// de StemGMD.
pub const GRUPOS_BATERIA: [&str; 4] = ["kick", "snare", "cymbals", "toms"];

pub fn grupo_de(clase: &str) -> Option<usize> {
    match clase {
        "kick" => Some(0),
        "snare" => Some(1),
        "hihat_closed" | "hihat_open" | "crash_left" | "ride" => Some(2),
        "hi_tom" | "mid_tom" | "low_tom" => Some(3),
        _ => None,
    }
}

// ─────────────────────────────── el juez: SDR en canciones apartadas ───────────────────────────────

/// Un extracto de una canción APARTADA: sus fuentes en estéreo (paneo fijo, sembrado por el nombre) y su
/// mezcla, SIN normalizar (el `Separador` normaliza como en la app).
pub struct Extracto {
    pub nombre: String,
    pub fuentes: Vec<[Vec<f32>; 2]>,
    pub mezcla: [Vec<f32>; 2],
}

/// `segundos` desde el primer tercio de la canción (o lo que haya).
pub fn extracto(c: &Cancion, nombres: &[String], segundos: f32) -> Result<Extracto> {
    let d = decodificar(c, nombres)?;
    let l = d.first().map(|v| v.len()).unwrap_or(0);
    let largo = ((segundos * crate::separador::FRECUENCIA as f32) as usize).min(l);
    let a = (l / 3).min(l - largo);
    let mut az = Azar::new(hash(&c.nombre));
    let mut fuentes = Vec::new();
    let mut mezcla = [vec![0f32; largo], vec![0f32; largo]];
    for (k, f) in d.iter().enumerate() {
        let (gl, gr) = paneo(&nombres[k], &mut az);
        let l_ = f[a..a + largo].iter().map(|x| x * gl).collect::<Vec<f32>>();
        let r_ = f[a..a + largo].iter().map(|x| x * gr).collect::<Vec<f32>>();
        for i in 0..largo {
            mezcla[0][i] += l_[i];
            mezcla[1][i] += r_[i];
        }
        fuentes.push([l_, r_]);
    }
    Ok(Extracto { nombre: c.nombre.clone(), fuentes, mezcla })
}

/// `new_sdr` de demucs sobre todo el extracto (estéreo): 10·log10((Σref² + ε) / (Σ(ref−est)² + ε)).
pub fn sdr(referencia: &[Vec<f32>; 2], estimada: &[Vec<f32>; 2]) -> f64 {
    let (mut s, mut r) = (1e-8f64, 1e-8f64);
    for c in 0..2 {
        for (a, b) in referencia[c].iter().zip(&estimada[c]) {
            s += (*a as f64).powi(2);
            r += (*a as f64 - *b as f64).powi(2);
        }
    }
    10.0 * (s / r).log10()
}

/// El veredicto del juez: SDR medio por fuente (sólo donde la fuente SUENA: ≥ 1 % de la energía de la
/// mezcla) y el promedio de las fuentes juzgadas.
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct Informe {
    pub por_fuente: Vec<(String, f64, usize)>,
    pub media: f64,
    /// el SDR de no separar nada (la mezcla como estimación): el piso que todo modelo debe superar
    #[serde(default)]
    pub sin_separar: Vec<(String, f64)>,
}

pub fn juzgar(sep: &crate::separador::Separador, extractos: &[Extracto], omitir: &[&str]) -> Result<Informe> {
    let nombres = &sep.fuentes;
    let mut suma: HashMap<usize, (f64, usize)> = HashMap::new();
    // el piso: el SDR de NO separar (la mezcla entera como estimación de cada fuente)
    let mut piso: HashMap<usize, (f64, usize)> = HashMap::new();
    let nada = std::sync::atomic::AtomicBool::new(false);
    for e in extractos {
        if e.fuentes.len() != nombres.len() {
            bail_tr!("el extracto {} no tiene las fuentes del modelo", "excerpt {} does not have the model sources", e.nombre);
        }
        let est = sep.separar(&e.mezcla, &|_, _| {}, &nada)?;
        let em: f64 = e.mezcla.iter().flat_map(|c| c.iter()).map(|x| (*x as f64).powi(2)).sum();
        for (k, n) in nombres.iter().enumerate() {
            if omitir.contains(&n.as_str()) {
                continue;
            }
            let ef: f64 = e.fuentes[k].iter().flat_map(|c| c.iter()).map(|x| (*x as f64).powi(2)).sum();
            if ef < 0.01 * em {
                continue;
            }
            let v = suma.entry(k).or_insert((0.0, 0));
            v.0 += sdr(&e.fuentes[k], &est[k]);
            let p = piso.entry(k).or_insert((0.0, 0));
            p.0 += sdr(&e.fuentes[k], &e.mezcla);
            p.1 += 1;
            v.1 += 1;
        }
    }
    let mut por_fuente: Vec<(String, f64, usize)> = suma.iter().map(|(k, (s, n))| (nombres[*k].clone(), s / *n as f64, *n)).collect();
    por_fuente.sort_by_key(|(n, _, _)| nombres.iter().position(|x| x == n));
    let media = if por_fuente.is_empty() { f64::NAN } else { por_fuente.iter().map(|x| x.1).sum::<f64>() / por_fuente.len() as f64 };
    let mut sin_separar: Vec<(String, f64)> = piso.iter().map(|(k, (s, n))| (nombres[*k].clone(), s / *n as f64)).collect();
    sin_separar.sort_by_key(|(n, _)| nombres.iter().position(|x| x == n));
    Ok(Informe { por_fuente, media, sin_separar })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn el_yaml_de_slakh_se_lee() {
        let y = "UUID: x\nstems:\n  S00:\n    audio_rendered: true\n    inst_class: Piano\n    is_drum: false\n  S01:\n    audio_rendered: false\n    inst_class: Drums\n    is_drum: true\n  S02:\n    inst_class: Bass\n    is_drum: false\n    audio_rendered: true\ntarget_peak: -1\n";
        let s = leer_yaml(y);
        assert_eq!(s.len(), 3);
        assert_eq!(s[0], ("S00".into(), true, "Piano".into(), false));
        assert_eq!(s[1], ("S01".into(), false, "Drums".into(), true));
        assert_eq!(s[2], ("S02".into(), true, "Bass".into(), false));
    }

    #[test]
    fn cada_clase_va_a_su_fuente() {
        let seis: Vec<String> = ["drums", "bass", "other", "vocals", "guitar", "piano"].iter().map(|s| s.to_string()).collect();
        let cuatro: Vec<String> = ["drums", "bass", "other", "vocals"].iter().map(|s| s.to_string()).collect();
        assert_eq!(fuente_de(&seis, "Drums", true), Some(0));
        assert_eq!(fuente_de(&seis, "Guitar", false), Some(4));
        assert_eq!(fuente_de(&cuatro, "Guitar", false), Some(2));
        assert_eq!(fuente_de(&seis, "Piano", false), Some(5));
        assert_eq!(fuente_de(&seis, "Organ", false), Some(2));
        assert_eq!(fuente_de(&seis, "Bass", false), Some(1));
    }

    #[test]
    fn el_corte_normaliza_como_wav_py() {
        let mut az = Azar::new(3);
        let c: Vec<Vec<f32>> = (0..2).map(|k| (0..100_000).map(|i| ((i as f32) * 0.01 * (k + 1) as f32).sin() * 0.3).collect()).collect();
        let nombres: Vec<String> = vec!["drums".into(), "other".into()];
        let ej = cortar(&c, &nombres, 20_000, 4, &mut az);
        assert_eq!(ej.len(), 4);
        // la mezcla mono de un trozo, re-escalada por la desviación de la canción, ≈ la original
        let (_, desv) = media_desv_cancion(&c);
        let e = &ej[0].fuentes;
        let rms: f64 = (0..20_000).map(|i| ((0..2).map(|k| e[(k * 2) * 20_000 + i] + e[(k * 2 + 1) * 20_000 + i]).sum::<f32>() / 2.0) as f64).map(|m| m * m).sum::<f64>() / 20_000.0;
        assert!(rms.sqrt() > 0.3 && rms.sqrt() < 2.0, "rms normalizado {} (desv {desv})", rms.sqrt());
    }

    #[test]
    fn el_disco_libre_se_mide() {
        let l = libre(&std::env::temp_dir().join("no-existe-aun").join("x"));
        assert!(l > 1_000_000, "libre = {l}");
    }

    #[test]
    fn el_agujero_libera_bloques() {
        let d = std::env::temp_dir().join(format!("gotv-agujero-{}", std::process::id()));
        let f = std::fs::OpenOptions::new().create(true).truncate(true).read(true).write(true).open(&d).unwrap();
        (&f).write_all(&vec![7u8; 8 << 20]).unwrap();
        f.sync_all().unwrap();
        agujero(&f, 0, 4 << 20).unwrap();
        let mut v = Vec::new();
        std::fs::File::open(&d).unwrap().read_to_end(&mut v).unwrap();
        assert_eq!(v.len(), 8 << 20, "el largo no cambia");
        assert!(v[..4 << 20].iter().all(|b| *b == 0) && v[4 << 20..].iter().all(|b| *b == 7));
        std::fs::remove_file(&d).unwrap();
    }

    #[test]
    fn las_voces_se_reparten_fijas_y_sin_cruzarse() {
        let d = std::env::temp_dir().join(format!("gotv-voces-{}", std::process::id()));
        for f in ["vs/female9/a.wav", "vs/male10/b.wav", "vs/female1/c.wav", "vs/female1/notas.txt"] {
            std::fs::create_dir_all(d.join(f).parent().unwrap()).unwrap();
            std::fs::write(d.join(f), b"x").unwrap();
        }
        for i in 0..40 {
            std::fs::write(d.join(format!("cc{i}.mp3")), b"x").unwrap();
        }
        let dirs = [d.clone()];
        let (t, v, p) = (voces(&dirs, "train"), voces(&dirs, "validation"), voces(&dirs, "test"));
        assert!(v.iter().any(|x| x.ends_with("female9/a.wav")) && p.iter().any(|x| x.ends_with("male10/b.wav")));
        assert!(t.iter().any(|x| x.ends_with("female1/c.wav")) && !t.iter().any(|x| x.extension().unwrap() == "txt"));
        assert_eq!(t.len() + v.len() + p.len(), 43, "cada voz cae en UNA partición");
        assert!(t.iter().all(|x| !v.contains(x) && !p.contains(x)));
        assert_eq!(voces(&dirs, "train"), t, "el reparto es estable");
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn la_voz_sumada_cae_en_su_fuente_y_a_su_nivel() {
        let (s, largo) = (2usize, 700usize);
        let mut f = vec![0f32; s * 2 * largo];
        for i in 0..largo {
            let x = (i as f32 * 0.05).sin();
            f[i] = x;
            f[largo + i] = x;
        }
        let antes = f[..2 * largo].to_vec();
        let voz: [Vec<f32>; 2] = std::array::from_fn(|_| (0..largo).map(|i| 3.0 * (i as f32 * 0.31).sin()).collect());
        let rms = |v: &[f32]| (v.iter().map(|x| (*x as f64).powi(2)).sum::<f64>() / v.len() as f64).sqrt();
        for semilla in 0..20 {
            let mut g = f.clone();
            sumar_voz(&mut g, s, 1, largo, &voz, &mut Azar::new(semilla));
            assert_eq!(&g[..2 * largo], &antes[..], "la música no se toca");
            let db = 20.0 * (rms(&g[2 * largo..]) / rms(&antes)).log10();
            assert!((-8.5..=2.5).contains(&db), "la voz a {db:.2} dB de la mezcla");
        }
    }
}
