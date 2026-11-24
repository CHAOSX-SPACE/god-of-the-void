//! M3 · hablar Y escuchar a la vez, SIN oírse: el AudioUnit `VoiceProcessingIO` de macOS (el de
//! FaceTime) con cancelación de eco. La voz de GOTV sale por la MISMA unidad por la que entra el
//! micrófono, así que el sistema sabe qué está sonando y lo resta de lo que oye.
//!
//! Medido sin esto (`vad::tests::eco_del_altavoz`): con el altavoz del Mac, el VAD veía «voz» en el
//! 82 % de las ventanas mientras GOTV hablaba (rachas de 3 s): se habría interrumpido a sí mismo.
//!
//! La unidad exige la MISMA frecuencia a la entrada y a la salida (medido: 24k/16k → OSStatus −10875;
//! 16/16, 24/24 y 48/48 sí): corre a `tasa_sal` (24 kHz, la de Qwen3-TTS, sin degradar la voz) y el
//! micrófono se baja a 16 kHz (VAD y Whisper) con [`crate::remuestreo::De24a16`].

use crate::{ok, Compartido, Formato, Pendiente};
use anyhow::Result;
use std::ffi::c_void;
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

#[repr(C)]
struct DescComponente {
    tipo: u32,
    subtipo: u32,
    fabricante: u32,
    flags: u32,
    mascara: u32,
}

#[repr(C)]
struct BufferAudio {
    canales: u32,
    bytes: u32,
    datos: *mut c_void,
}

#[repr(C)]
struct ListaBuffers {
    n: u32,
    b: [BufferAudio; 1],
}

type Unidad = *mut c_void;
type CbRender = extern "C" fn(*mut c_void, *mut u32, *const c_void, u32, u32, *mut ListaBuffers) -> i32;

#[repr(C)]
struct Callback {
    proc_: CbRender,
    ref_: *mut c_void,
}

#[link(name = "AudioToolbox", kind = "framework")]
extern "C" {
    fn AudioComponentFindNext(c: *mut c_void, d: *const DescComponente) -> *mut c_void;
    fn AudioComponentInstanceNew(c: *mut c_void, sal: *mut Unidad) -> i32;
    fn AudioComponentInstanceDispose(u: Unidad) -> i32;
    fn AudioUnitSetProperty(u: Unidad, id: u32, alcance: u32, elemento: u32, dato: *const c_void, tam: u32) -> i32;
    fn AudioUnitInitialize(u: Unidad) -> i32;
    fn AudioUnitUninitialize(u: Unidad) -> i32;
    fn AudioOutputUnitStart(u: Unidad) -> i32;
    fn AudioOutputUnitStop(u: Unidad) -> i32;
    fn AudioUnitRender(u: Unidad, flags: *mut u32, ts: *const c_void, bus: u32, n: u32, datos: *mut ListaBuffers) -> i32;
}

const fn cc(s: &[u8; 4]) -> u32 {
    u32::from_be_bytes(*s)
}
const K_TIPO_SALIDA: u32 = cc(b"auou");
const K_VPIO: u32 = cc(b"vpio");
const K_APPLE: u32 = cc(b"appl");
const P_FORMATO: u32 = 8; // kAudioUnitProperty_StreamFormat
const P_RENDER: u32 = 23; // kAudioUnitProperty_SetRenderCallback
const P_ACTIVAR_IO: u32 = 2003; // kAudioOutputUnitProperty_EnableIO
const P_CB_ENTRADA: u32 = 2005; // kAudioOutputUnitProperty_SetInputCallback
const GLOBAL: u32 = 0;
const ENTRADA: u32 = 1; // alcance de entrada
const SALIDA: u32 = 2; // alcance de salida
const BUS_ALTAVOZ: u32 = 0;
const BUS_MIC: u32 = 1;

struct Estado {
    unidad: Unidad,
    sal: Compartido,
    mic: Mutex<Option<std::sync::mpsc::Sender<Vec<f32>>>>,
    /// lo que el micrófono va dejando hasta completar un bloque
    resto: Mutex<Vec<f32>>,
    bloque: usize,
    lectura: Mutex<Vec<f32>>,
    bajar: Mutex<Option<crate::remuestreo::De24a16>>,
}

extern "C" fn render(r: *mut c_void, _f: *mut u32, _ts: *const c_void, _bus: u32, n: u32, datos: *mut ListaBuffers) -> i32 {
    // SAFETY: `r` es el `Arc<Estado>` que el Duplex mantiene vivo mientras corra la unidad
    let e = unsafe { &*(r as *const Estado) };
    let lista = unsafe { &mut *datos };
    let n = n as usize;
    let destino = unsafe { std::slice::from_raw_parts_mut(lista.b[0].datos as *mut f32, n) };
    let mut p = e.sal.p.lock().unwrap_or_else(|x| x.into_inner());
    let reales = n.min(p.muestras.len());
    for (d, s) in destino.iter_mut().zip(p.muestras.drain(..reales)) {
        *d = s;
    }
    destino[reales..].fill(0.0);
    if reales < n && p.empujadas > 0 && !p.completa {
        p.vacios += 1;
    }
    p.sonadas += reales as u64;
    drop(p);
    e.sal.cv.notify_all();
    0
}

extern "C" fn entrada(r: *mut c_void, flags: *mut u32, ts: *const c_void, bus: u32, n: u32, _d: *mut ListaBuffers) -> i32 {
    // SAFETY: como en `render`
    let e = unsafe { &*(r as *const Estado) };
    let mut buf = e.lectura.lock().unwrap_or_else(|x| x.into_inner());
    buf.resize(n as usize, 0.0);
    let mut lista = ListaBuffers { n: 1, b: [BufferAudio { canales: 1, bytes: n * 4, datos: buf.as_mut_ptr() as *mut c_void }] };
    let st = unsafe { AudioUnitRender(e.unidad, flags, ts, bus, n, &mut lista) };
    if st != 0 {
        return st;
    }
    let mut resto = e.resto.lock().unwrap_or_else(|x| x.into_inner());
    match e.bajar.lock().unwrap_or_else(|x| x.into_inner()).as_mut() {
        Some(r) => resto.extend(r.procesar(&buf[..n as usize])),
        None => resto.extend_from_slice(&buf[..n as usize]),
    }
    let g = e.mic.lock().unwrap_or_else(|x| x.into_inner());
    while resto.len() >= e.bloque {
        let b: Vec<f32> = resto.drain(..e.bloque).collect();
        if let Some(tx) = g.as_ref() {
            let _ = tx.send(b);
        }
    }
    0
}

/// Altavoz + micrófono con cancelación de eco.
pub struct Duplex {
    e: Arc<Estado>,
    rx: Mutex<std::sync::mpsc::Receiver<Vec<f32>>>,
}

// SAFETY: el único campo sin `Mutex` es el puntero a la unidad de Core Audio, que admite llamadas
// desde cualquier hilo (sus callbacks ya corren en un hilo propio de Core Audio). Con esto, `Duplex`
// (Arc<Estado> + receptor tras Mutex) es Send + Sync por sí solo.
unsafe impl Send for Estado {}
unsafe impl Sync for Estado {}

impl Duplex {
    /// `tasa_sal` para lo que suena, `tasa_ent` y `bloque` (muestras) para lo que se oye.
    pub fn abrir(tasa_sal: u32, tasa_ent: u32, bloque: usize) -> Result<Duplex> {
        let bajar = match (tasa_sal, tasa_ent) {
            (s, e) if s == e => None,
            (24000, 16000) => Some(crate::remuestreo::De24a16::nuevo()),
            (s, e) => gotv_engine::bail_tr!("Duplex: {s} → {e} Hz no está soportado (sólo iguales o 24000 → 16000)", "Duplex: {s} → {e} Hz is not supported (only equal rates or 24000 → 16000)"),
        };
        let d = DescComponente { tipo: K_TIPO_SALIDA, subtipo: K_VPIO, fabricante: K_APPLE, flags: 0, mascara: 0 };
        let comp = unsafe { AudioComponentFindNext(std::ptr::null_mut(), &d) };
        if comp.is_null() {
            anyhow::bail!("macOS no ofrece VoiceProcessingIO");
        }
        let mut unidad: Unidad = std::ptr::null_mut();
        ok(unsafe { AudioComponentInstanceNew(comp, &mut unidad) }, "crear VoiceProcessingIO")?;
        let (tx, rx) = std::sync::mpsc::channel();
        let e = Arc::new(Estado { unidad, sal: Compartido { p: Mutex::new(Pendiente::default()), cv: Condvar::new() }, mic: Mutex::new(Some(tx)), resto: Mutex::new(Vec::new()), bloque, lectura: Mutex::new(Vec::new()), bajar: Mutex::new(bajar) });
        let poner = |id: u32, alcance: u32, el: u32, dato: *const c_void, tam: usize, que: &str| ok(unsafe { AudioUnitSetProperty(unidad, id, alcance, el, dato, tam as u32) }, que);
        let si: u32 = 1;
        let r = (|| -> Result<()> {
            poner(P_ACTIVAR_IO, ENTRADA, BUS_MIC, &si as *const u32 as *const c_void, 4, gotv_engine::i18n::t("activar el micrófono", "enabling the microphone"))?;
            poner(P_ACTIVAR_IO, SALIDA, BUS_ALTAVOZ, &si as *const u32 as *const c_void, 4, gotv_engine::i18n::t("activar el altavoz", "enabling the speaker"))?;
            let fs: Formato = crate::formato(tasa_sal);
            let fe: Formato = crate::formato(tasa_sal); // la unidad: misma tasa a ambos lados
            poner(P_FORMATO, ENTRADA, BUS_ALTAVOZ, &fs as *const Formato as *const c_void, std::mem::size_of::<Formato>(), gotv_engine::i18n::t("formato del altavoz", "speaker format"))?;
            poner(P_FORMATO, SALIDA, BUS_MIC, &fe as *const Formato as *const c_void, std::mem::size_of::<Formato>(), gotv_engine::i18n::t("formato del micrófono", "microphone format"))?;
            let cr = Callback { proc_: render, ref_: Arc::as_ptr(&e) as *mut c_void };
            poner(P_RENDER, ENTRADA, BUS_ALTAVOZ, &cr as *const Callback as *const c_void, std::mem::size_of::<Callback>(), gotv_engine::i18n::t("callback del altavoz", "speaker callback"))?;
            let ce = Callback { proc_: entrada, ref_: Arc::as_ptr(&e) as *mut c_void };
            poner(P_CB_ENTRADA, GLOBAL, BUS_MIC, &ce as *const Callback as *const c_void, std::mem::size_of::<Callback>(), gotv_engine::i18n::t("callback del micrófono", "microphone callback"))?;
            ok(unsafe { AudioUnitInitialize(unidad) }, "inicializar VoiceProcessingIO")?;
            ok(unsafe { AudioOutputUnitStart(unidad) }, "arrancar VoiceProcessingIO")
        })();
        if let Err(x) = r {
            unsafe { AudioComponentInstanceDispose(unidad) };
            return Err(x);
        }
        Ok(Duplex { e, rx: Mutex::new(rx) })
    }

    pub fn empujar(&self, onda: &[f32]) {
        let mut p = self.e.sal.p.lock().unwrap_or_else(|x| x.into_inner());
        p.muestras.extend(onda.iter().copied());
        p.empujadas += onda.len() as u64;
        p.completa = false;
    }

    pub fn callar(&self) {
        let mut p = self.e.sal.p.lock().unwrap_or_else(|x| x.into_inner());
        let quedan = p.muestras.len() as u64;
        p.muestras.clear();
        p.empujadas -= quedan;
        p.completa = true;
        drop(p);
        self.e.sal.cv.notify_all();
    }

    pub fn sonando(&self) -> bool {
        let p = self.e.sal.p.lock().unwrap_or_else(|x| x.into_inner());
        p.sonadas < p.empujadas
    }

    /// Marca la frase como completa (lo que falte ya no es un corte).
    pub fn completa(&self) {
        self.e.sal.p.lock().unwrap_or_else(|x| x.into_inner()).completa = true;
    }

    pub fn cortes(&self) -> u64 {
        self.e.sal.p.lock().unwrap_or_else(|x| x.into_inner()).vacios
    }

    pub fn esperar(&self) {
        let mut p = self.e.sal.p.lock().unwrap_or_else(|x| x.into_inner());
        p.completa = true;
        while p.sonadas < p.empujadas {
            p = self.e.sal.cv.wait_timeout(p, Duration::from_millis(100)).map(|(g, _)| g).unwrap_or_else(|x| x.into_inner().0);
        }
    }

    /// El siguiente bloque del micrófono (ya sin el eco de lo que suena).
    pub fn leer(&self, t: Duration) -> Option<Vec<f32>> {
        self.rx.lock().unwrap_or_else(|x| x.into_inner()).recv_timeout(t).ok()
    }
}

impl Drop for Duplex {
    fn drop(&mut self) {
        *self.e.mic.lock().unwrap_or_else(|x| x.into_inner()) = None;
        unsafe {
            AudioOutputUnitStop(self.e.unidad);
            AudioUnitUninitialize(self.e.unidad);
            AudioComponentInstanceDispose(self.e.unidad);
        }
    }
}

#[cfg(test)]
mod tests {
    /// Qué formatos acepta VoiceProcessingIO en esta Mac. `GOTV_SONAR=1 cargo test -p gotv-sonido -- --ignored --nocapture formatos_vpio`
    #[test]
    #[ignore]
    fn formatos_vpio() {
        if std::env::var_os("GOTV_SONAR").is_none() {
            return;
        }
        for (s, e) in [(24000, 16000), (24000, 24000), (16000, 16000), (48000, 48000)] {
            let r = super::Duplex::abrir(s, e, 512);
            println!("salida {s} · entrada {e}: {}", r.as_ref().map(|_| "✓".to_string()).unwrap_or_else(|x| x.to_string()));
            assert!(r.is_ok());
            drop(r);
        }
    }
}
