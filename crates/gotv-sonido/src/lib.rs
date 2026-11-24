//! M2/M3 · el ALTAVOZ y el MICRÓFONO de GOTV, directo sobre AudioQueue (AudioToolbox de macOS), sin
//! crates de terceros. AudioQueue acepta CUALQUIER frecuencia y convierte por dentro: la voz sale a
//! 24 kHz (lo que da Qwen3-TTS) y el micrófono entra a 16 kHz (lo que piden Whisper y el VAD), sin
//! remuestrear a mano ni tocar la frecuencia del dispositivo (que es de todo el sistema).
//!
//! - [`Altavoz`]: se le empujan trozos de onda mientras se generan y suenan en orden; `callar()` corta
//!   al instante (la interrupción de M3); `esperar()` vuelve cuando terminó de sonar.
//! - [`Microfono`]: entrega bloques de N muestras (512 a 16 kHz = 32 ms, la ventana del VAD Silero).
//!
//! Los callbacks corren en el hilo propio de AudioQueue (run loop nulo): jamás bloquean ni asignan
//! más de lo necesario; el intercambio va por un `Mutex` corto.

pub mod duplex;
pub mod remuestreo;
pub mod vad;

pub use duplex::Duplex;

use anyhow::Result;
use std::collections::VecDeque;
use std::ffi::c_void;
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

#[repr(C)]
pub(crate) struct Formato {
    sample_rate: f64,
    format_id: u32,
    format_flags: u32,
    bytes_per_packet: u32,
    frames_per_packet: u32,
    bytes_per_frame: u32,
    channels_per_frame: u32,
    bits_per_channel: u32,
    reserved: u32,
}

#[repr(C)]
struct Buffer {
    capacidad: u32,
    datos: *mut c_void,
    tamano: u32,
    usuario: *mut c_void,
    pd_capacidad: u32,
    pd: *mut c_void,
    pd_cuenta: u32,
}

type Cola = *mut c_void;
type CbSalida = extern "C" fn(*mut c_void, Cola, *mut Buffer);
type CbEntrada = extern "C" fn(*mut c_void, Cola, *mut Buffer, *const c_void, u32, *const c_void);

#[link(name = "AudioToolbox", kind = "framework")]
extern "C" {
    fn AudioQueueNewOutput(f: *const Formato, cb: CbSalida, u: *mut c_void, rl: *const c_void, modo: *const c_void, flags: u32, sal: *mut Cola) -> i32;
    fn AudioQueueNewInput(f: *const Formato, cb: CbEntrada, u: *mut c_void, rl: *const c_void, modo: *const c_void, flags: u32, sal: *mut Cola) -> i32;
    fn AudioQueueAllocateBuffer(c: Cola, bytes: u32, sal: *mut *mut Buffer) -> i32;
    fn AudioQueueEnqueueBuffer(c: Cola, b: *mut Buffer, n: u32, pd: *const c_void) -> i32;
    fn AudioQueueStart(c: Cola, t: *const c_void) -> i32;
    fn AudioQueueStop(c: Cola, inmediato: u8) -> i32;
    fn AudioQueueDispose(c: Cola, inmediato: u8) -> i32;
}

const LPCM: u32 = u32::from_be_bytes(*b"lpcm");
/// kAudioFormatFlagIsFloat | kAudioFormatFlagIsPacked
const FLOAT_EMPAQUETADO: u32 = 1 | 8;

pub(crate) fn formato(tasa: u32) -> Formato {
    Formato { sample_rate: tasa as f64, format_id: LPCM, format_flags: FLOAT_EMPAQUETADO, bytes_per_packet: 4, frames_per_packet: 1, bytes_per_frame: 4, channels_per_frame: 1, bits_per_channel: 32, reserved: 0 }
}

pub(crate) fn ok(e: i32, que: &str) -> Result<()> {
    if e != 0 {
        gotv_engine::bail_tr!("AudioQueue: {que} falló (OSStatus {e})", "AudioQueue: {que} failed (OSStatus {e})");
    }
    Ok(())
}

/// Lo que comparten el altavoz y su callback.
#[derive(Default)]
pub(crate) struct Pendiente {
    pub(crate) muestras: VecDeque<f32>,
    /// muestras REALES (no silencio) ya entregadas al dispositivo
    pub(crate) sonadas: u64,
    pub(crate) empujadas: u64,
    /// veces que el dispositivo pidió y no había nada (la generación iba detrás)
    pub(crate) vacios: u64,
    /// la frase en curso ya llegó entera (`esperar`): quedarse sin onda ya no es un corte
    pub(crate) completa: bool,
    pub(crate) cerrado: bool,
}

pub(crate) struct Compartido {
    pub(crate) p: Mutex<Pendiente>,
    pub(crate) cv: Condvar,
}

/// Bloque que pide el dispositivo: 40 ms; 3 en vuelo = ~120 ms de latencia de salida.
const BLOQUE_MS: u32 = 40;
const BLOQUES: usize = 3;

extern "C" fn llenar(u: *mut c_void, cola: Cola, b: *mut Buffer) {
    // SAFETY: `u` es el `Arc<Compartido>` que el Altavoz mantiene vivo mientras exista la cola
    let c = unsafe { &*(u as *const Compartido) };
    let buf = unsafe { &mut *b };
    let n = (buf.capacidad / 4) as usize;
    let destino = unsafe { std::slice::from_raw_parts_mut(buf.datos as *mut f32, n) };
    let mut p = c.p.lock().unwrap_or_else(|e| e.into_inner());
    if p.cerrado {
        return; // la cola se está desmontando: no se re-encola
    }
    let reales = n.min(p.muestras.len());
    for (d, s) in destino.iter_mut().zip(p.muestras.drain(..reales)) {
        *d = s;
    }
    destino[reales..].fill(0.0);
    // un corte: el dispositivo pidió más, la frase empezó y todavía no llegó entera
    if reales < n && p.empujadas > 0 && !p.completa {
        p.vacios += 1;
    }
    p.sonadas += reales as u64;
    drop(p);
    c.cv.notify_all();
    buf.tamano = (n * 4) as u32;
    unsafe { AudioQueueEnqueueBuffer(cola, b, 0, std::ptr::null()) };
}

/// La salida de audio: trozos de onda mono f32 a `tasa` Hz, en orden.
pub struct Altavoz {
    cola: Cola,
    c: Arc<Compartido>,
    tasa: u32,
}

// SAFETY: la cola de AudioQueue se puede usar desde cualquier hilo (API de Core Audio con lock propio)
unsafe impl Send for Altavoz {}
// SAFETY: lo compartido va tras `Mutex`/`Condvar`; la cola de AudioQueue admite llamadas desde
// cualquier hilo (el locutor empuja, el oído pregunta si suena, el turno la calla).
unsafe impl Sync for Altavoz {}

impl Altavoz {
    pub fn abrir(tasa: u32) -> Result<Altavoz> {
        let c = Arc::new(Compartido { p: Mutex::new(Pendiente::default()), cv: Condvar::new() });
        let f = formato(tasa);
        let mut cola: Cola = std::ptr::null_mut();
        ok(unsafe { AudioQueueNewOutput(&f, llenar, Arc::as_ptr(&c) as *mut c_void, std::ptr::null(), std::ptr::null(), 0, &mut cola) }, gotv_engine::i18n::t("crear la salida", "creating the output"))?;
        let bytes = tasa * BLOQUE_MS / 1000 * 4;
        for _ in 0..BLOQUES {
            let mut b: *mut Buffer = std::ptr::null_mut();
            ok(unsafe { AudioQueueAllocateBuffer(cola, bytes, &mut b) }, "reservar un bloque")?;
            // en silencio hasta que lleguen muestras (el callback los rellena desde ahí)
            unsafe {
                std::ptr::write_bytes((*b).datos as *mut u8, 0, bytes as usize);
                (*b).tamano = bytes;
            }
            ok(unsafe { AudioQueueEnqueueBuffer(cola, b, 0, std::ptr::null()) }, "encolar un bloque")?;
        }
        ok(unsafe { AudioQueueStart(cola, std::ptr::null()) }, gotv_engine::i18n::t("arrancar la salida", "starting the output"))?;
        Ok(Altavoz { cola, c, tasa })
    }

    pub fn tasa(&self) -> u32 {
        self.tasa
    }

    /// Añade onda al final de lo que está sonando.
    pub fn empujar(&self, onda: &[f32]) {
        let mut p = self.c.p.lock().unwrap_or_else(|e| e.into_inner());
        p.muestras.extend(onda.iter().copied());
        p.empujadas += onda.len() as u64;
        p.completa = false;
    }

    /// Corta en seco lo pendiente (interrupción): lo que ya está en el dispositivo (< 120 ms) termina.
    pub fn callar(&self) {
        let mut p = self.c.p.lock().unwrap_or_else(|e| e.into_inner());
        let quedan = p.muestras.len() as u64;
        p.muestras.clear();
        p.empujadas -= quedan;
        p.completa = true;
        drop(p);
        self.c.cv.notify_all();
    }

    /// ¿Queda algo por sonar?
    pub fn sonando(&self) -> bool {
        let p = self.c.p.lock().unwrap_or_else(|e| e.into_inner());
        p.sonadas < p.empujadas
    }

    /// Veces que el dispositivo se quedó sin onda a mitad de frase (la generación iba detrás).
    pub fn cortes(&self) -> u64 {
        self.c.p.lock().unwrap_or_else(|e| e.into_inner()).vacios
    }

    /// Como [`Altavoz::esperar`], pero vuelve antes si `cortar()` dice que sí (una interrupción).
    pub fn esperar_o(&self, cortar: &dyn Fn() -> bool) {
        let mut p = self.c.p.lock().unwrap_or_else(|e| e.into_inner());
        p.completa = true;
        while p.sonadas < p.empujadas && !cortar() {
            p = self.c.cv.wait_timeout(p, Duration::from_millis(30)).map(|(g, _)| g).unwrap_or_else(|e| e.into_inner().0);
        }
    }

    /// Vuelve cuando todo lo empujado salió al dispositivo (+ el último bloque en vuelo). Marca la
    /// frase como completa: lo que falte desde aquí es su final, no un corte.
    pub fn esperar(&self) {
        let mut p = self.c.p.lock().unwrap_or_else(|e| e.into_inner());
        p.completa = true;
        while p.sonadas < p.empujadas {
            p = self.c.cv.wait_timeout(p, Duration::from_millis(100)).map(|(g, _)| g).unwrap_or_else(|e| e.into_inner().0);
        }
        drop(p);
        std::thread::sleep(Duration::from_millis((BLOQUE_MS * BLOQUES as u32) as u64));
    }
}

impl Drop for Altavoz {
    fn drop(&mut self) {
        self.c.p.lock().unwrap_or_else(|e| e.into_inner()).cerrado = true;
        unsafe {
            AudioQueueStop(self.cola, 1);
            AudioQueueDispose(self.cola, 1);
        }
    }
}

struct Escucha {
    enviar: Mutex<Option<std::sync::mpsc::Sender<Vec<f32>>>>,
}

extern "C" fn recibir(u: *mut c_void, cola: Cola, b: *mut Buffer, _t: *const c_void, _n: u32, _pd: *const c_void) {
    // SAFETY: `u` es el `Arc<Escucha>` que el Microfono mantiene vivo mientras exista la cola
    let e = unsafe { &*(u as *const Escucha) };
    let buf = unsafe { &mut *b };
    let n = (buf.tamano / 4) as usize;
    let datos = unsafe { std::slice::from_raw_parts(buf.datos as *const f32, n) }.to_vec();
    let g = e.enviar.lock().unwrap_or_else(|x| x.into_inner());
    if let Some(tx) = g.as_ref() {
        if tx.send(datos).is_ok() {
            unsafe { AudioQueueEnqueueBuffer(cola, b, 0, std::ptr::null()) };
        }
    }
}

/// La entrada de audio: bloques mono f32 de `bloque` muestras a `tasa` Hz.
pub struct Microfono {
    cola: Cola,
    e: Arc<Escucha>,
    rx: std::sync::mpsc::Receiver<Vec<f32>>,
}

unsafe impl Send for Microfono {}

impl Microfono {
    /// Abre el micrófono por defecto. macOS pide permiso la primera vez (a la app o a la terminal).
    pub fn abrir(tasa: u32, bloque: usize) -> Result<Microfono> {
        let (tx, rx) = std::sync::mpsc::channel();
        let e = Arc::new(Escucha { enviar: Mutex::new(Some(tx)) });
        let f = formato(tasa);
        let mut cola: Cola = std::ptr::null_mut();
        ok(unsafe { AudioQueueNewInput(&f, recibir, Arc::as_ptr(&e) as *mut c_void, std::ptr::null(), std::ptr::null(), 0, &mut cola) }, gotv_engine::i18n::t("abrir el micrófono", "opening the microphone"))?;
        for _ in 0..4 {
            let mut b: *mut Buffer = std::ptr::null_mut();
            ok(unsafe { AudioQueueAllocateBuffer(cola, (bloque * 4) as u32, &mut b) }, "reservar un bloque")?;
            ok(unsafe { AudioQueueEnqueueBuffer(cola, b, 0, std::ptr::null()) }, "encolar un bloque")?;
        }
        ok(unsafe { AudioQueueStart(cola, std::ptr::null()) }, gotv_engine::i18n::t("arrancar el micrófono", "starting the microphone"))?;
        Ok(Microfono { cola, e, rx })
    }

    /// El siguiente bloque (espera hasta `t`); `None` si no llegó nada.
    pub fn leer(&self, t: Duration) -> Option<Vec<f32>> {
        self.rx.recv_timeout(t).ok()
    }
}

impl Drop for Microfono {
    fn drop(&mut self) {
        *self.e.enviar.lock().unwrap_or_else(|x| x.into_inner()) = None;
        unsafe {
            AudioQueueStop(self.cola, 1);
            AudioQueueDispose(self.cola, 1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn el_formato_es_lpcm_float_mono() {
        let f = formato(24000);
        assert_eq!((f.format_id, f.format_flags, f.bytes_per_frame, f.channels_per_frame), (0x6C70_636D, 9, 4, 1));
        assert_eq!(std::mem::size_of::<Formato>(), 40, "AudioStreamBasicDescription mide 40 bytes");
    }

    /// Suena 0,6 s de un La (440 Hz) a 24 kHz, en 3 trozos, y mide que todo salió.
    /// `GOTV_SONAR=1 cargo test -p gotv-sonido -- --ignored --nocapture suena_un_la`
    #[test]
    #[ignore]
    fn suena_un_la() {
        if std::env::var_os("GOTV_SONAR").is_none() {
            return;
        }
        let a = Altavoz::abrir(24000).unwrap();
        let t = std::time::Instant::now();
        for k in 0..3 {
            let onda: Vec<f32> = (0..4800).map(|i| 0.2 * (2.0 * std::f32::consts::PI * 440.0 * (k * 4800 + i) as f32 / 24000.0).sin()).collect();
            a.empujar(&onda);
        }
        assert!(a.sonando());
        a.esperar();
        println!("0,6 s de La en {:?} · cortes {}", t.elapsed(), a.cortes());
        assert!(!a.sonando());
        assert!(t.elapsed() >= Duration::from_millis(600));
        assert_eq!(a.cortes(), 0, "todo empujado de una vez: sin cortes");
        // un hueco a propósito: 0,1 s, pausa de 0,3 s, 0,1 s → el contador lo ve
        let corto: Vec<f32> = (0..2400).map(|i| 0.2 * (2.0 * std::f32::consts::PI * 660.0 * i as f32 / 24000.0).sin()).collect();
        a.empujar(&corto);
        std::thread::sleep(Duration::from_millis(400));
        a.empujar(&corto);
        a.esperar();
        println!("con un hueco de 0,3 s: cortes {}", a.cortes());
        assert!(a.cortes() > 0, "el hueco se cuenta");
    }
}
