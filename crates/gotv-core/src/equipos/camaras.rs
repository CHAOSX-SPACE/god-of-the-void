#![allow(unexpected_cfgs)] // las macros de objc 0.2 preguntan por cfg(feature = "cargo-clippy")
//! F22 paso 9 · Cámaras desde Rust, preguntándole a AVFoundation (`objc`): LISTAR (sin encender ninguna) y
//! una FOTO (pregunta siempre: es la intimidad del usuario). Nada de `system_profiler`, `ffmpeg` ni
//! `imagesnap`: el Portador pidió esta herramienta en Rust y en proceso, sin programas intermedios.
//!
//! La foto: sesión de captura con una salida de vídeo cuyo DELEGADO es una clase Objective-C declarada desde
//! Rust; se queda el fotograma 30 (medido 2026-11-17: la cámara arranca oscura el primer segundo mientras
//! ajusta la luz) y Core Image lo escribe en JPEG. Antes se mira el permiso de cámara: denegado o
//! restringido → error claro que dice dónde se da; sin decidir → macOS pregunta al encender y el tope corta.
use anyhow::{bail, Result};
use objc::declare::ClassDecl;
use objc::runtime::{Class, Object, Sel, BOOL, YES};
use objc::{class, msg_send, sel, sel_impl};
use std::ffi::{c_char, c_void, CStr, CString};
use std::sync::{Mutex, OnceLock};

#[link(name = "AVFoundation", kind = "framework")]
extern "C" {
    static AVMediaTypeVideo: *mut Object;
}
#[link(name = "CoreMedia", kind = "framework")]
extern "C" {
    fn CMSampleBufferGetImageBuffer(sb: *mut c_void) -> *mut c_void;
}
#[link(name = "CoreVideo", kind = "framework")]
extern "C" {
    fn CVBufferRetain(b: *mut c_void) -> *mut c_void;
    fn CVBufferRelease(b: *mut c_void);
}
#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGColorSpaceCreateDeviceRGB() -> *mut c_void;
    fn CGColorSpaceRelease(cs: *mut c_void);
}
#[link(name = "CoreImage", kind = "framework")]
extern "C" {}
extern "C" {
    fn dispatch_queue_create(label: *const c_char, attr: *mut c_void) -> *mut c_void;
    fn dispatch_release(o: *mut c_void);
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Camara {
    pub nombre: String,
    pub modelo: String,
}

unsafe fn cadena(o: *mut Object) -> String {
    if o.is_null() {
        return String::new();
    }
    let p: *const c_char = msg_send![o, UTF8String];
    if p.is_null() {
        String::new()
    } else {
        CStr::from_ptr(p).to_string_lossy().to_string()
    }
}

unsafe fn ns(s: &str) -> *mut Object {
    let c = CString::new(s).unwrap_or_default();
    msg_send![class!(NSString), stringWithUTF8String: c.as_ptr()]
}

unsafe fn dispositivos() -> Result<Vec<*mut Object>> {
    let arr: *mut Object = msg_send![class!(AVCaptureDevice), devicesWithMediaType: AVMediaTypeVideo];
    if arr.is_null() {
        bail!("{}", crate::i18n::t("AVFoundation no respondió", "AVFoundation did not answer"));
    }
    let n: usize = msg_send![arr, count];
    Ok((0..n).map(|i| msg_send![arr, objectAtIndex: i]).collect())
}

/// Las cámaras de vídeo que ve el sistema (nombre y modelo; el identificador único no se muestra).
pub fn listar() -> Result<Vec<Camara>> {
    unsafe {
        Ok(dispositivos()?
            .into_iter()
            .map(|d| {
                let nombre: *mut Object = msg_send![d, localizedName];
                let modelo: *mut Object = msg_send![d, modelID];
                Camara { nombre: cadena(nombre), modelo: cadena(modelo) }
            })
            .collect())
    }
}

/// Lo que comparte el delegado (hilo de AVFoundation) con quien espera la foto: fotogramas vistos y el
/// búfer de imagen retenido (como dirección). Una foto a la vez (`UNA`).
static ESTADO: Mutex<(u32, usize)> = Mutex::new((0, 0));
static UNA: Mutex<()> = Mutex::new(());
/// El fotograma que se guarda (≈1 s a 30 fps).
const FOTOGRAMA: u32 = 30;

extern "C" fn al_llegar(_this: &Object, _cmd: Sel, _salida: *mut Object, muestra: *mut Object, _conexion: *mut Object) {
    let Ok(mut e) = ESTADO.lock() else { return };
    e.0 += 1;
    if e.0 >= FOTOGRAMA && e.1 == 0 {
        unsafe {
            let ib = CMSampleBufferGetImageBuffer(muestra as *mut c_void);
            if !ib.is_null() {
                CVBufferRetain(ib);
                e.1 = ib as usize;
            }
        }
    }
}

fn delegado() -> &'static Class {
    static C: OnceLock<usize> = OnceLock::new();
    let p = *C.get_or_init(|| {
        let mut d = ClassDecl::new("GotvFotoDelegado", class!(NSObject)).expect("clase GotvFotoDelegado");
        unsafe {
            d.add_method(sel!(captureOutput:didOutputSampleBuffer:fromConnection:), al_llegar as extern "C" fn(&Object, Sel, *mut Object, *mut Object, *mut Object));
        }
        d.register() as *const Class as usize
    });
    unsafe { &*(p as *const Class) }
}

/// El permiso de cámara de la app que corre GOTV: 3 autorizado · 2 denegado · 1 restringido · 0 sin decidir.
pub fn permiso() -> isize {
    unsafe { msg_send![class!(AVCaptureDevice), authorizationStatusForMediaType: AVMediaTypeVideo] }
}

/// Toma la foto (síncrono: se llama desde un hilo bloqueante). Devuelve el nombre de la cámara.
pub fn foto_bloqueante(quiere: Option<&str>, destino: &std::path::Path, tope: std::time::Duration) -> Result<String> {
    let _una = UNA.lock().map_err(|_| anyhow::anyhow!("cámara ocupada"))?; // juez-idioma: datos (no se ve: candado envenenado)
    unsafe {
        // 3 = autorizado · 2 = denegado · 1 = restringido · 0 = sin decidir (macOS pregunta al encender)
        let mut permiso: isize = msg_send![class!(AVCaptureDevice), authorizationStatusForMediaType: AVMediaTypeVideo];
        if permiso == 0 {
            // sin decidir: se PIDE (medido 2026-11-17: sin pedirlo, macOS no muestra el cuadro y la cámara no
            // da imagen); macOS lo muestra si la app que corre GOTV declara NSCameraUsageDescription
            let respuesta = std::sync::Arc::new(std::sync::atomic::AtomicU8::new(0));
            let r = respuesta.clone();
            let bloque = block2::RcBlock::new(move |ok: objc2::runtime::Bool| {
                r.store(if ok.as_bool() { 3 } else { 2 }, std::sync::atomic::Ordering::SeqCst);
            });
            let b: *const block2::Block<dyn Fn(objc2::runtime::Bool)> = &*bloque;
            let _: () = msg_send![class!(AVCaptureDevice), requestAccessForMediaType: AVMediaTypeVideo completionHandler: b as *mut c_void];
            let t0 = std::time::Instant::now();
            while respuesta.load(std::sync::atomic::Ordering::SeqCst) == 0 && t0.elapsed() < tope {
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            permiso = msg_send![class!(AVCaptureDevice), authorizationStatusForMediaType: AVMediaTypeVideo];
            if permiso == 0 {
                bail!("{}", crate::i18n::t(
                    "macOS no preguntó por la cámara: la app que corre GOTV debe declarar para qué la usa (NSCameraUsageDescription en su Info.plist)",
                    "macOS did not ask for the camera: the app running GOTV must declare what it uses it for (NSCameraUsageDescription in its Info.plist)"
                ));
            }
        }
        if permiso == 1 || permiso == 2 {
            bail!("{}", crate::i18n::t(
                "macOS no da permiso de cámara a la app que corre GOTV: Configuración del Sistema → Privacidad y seguridad → Cámara",
                "macOS does not grant camera access to the app running GOTV: System Settings → Privacy & Security → Camera"
            ));
        }
        let todos = dispositivos()?;
        let elegido = todos.into_iter().find(|d| {
            let n: *mut Object = msg_send![*d, localizedName];
            quiere.is_none_or(|q| cadena(n).to_lowercase().contains(&q.to_lowercase()))
        });
        let Some(dev) = elegido else { bail!("{}", crate::tr!("no hay cámara «{}»", "there is no camera «{}»", quiere.unwrap_or("?"))) };
        let nombre_ns: *mut Object = msg_send![dev, localizedName];
        let nombre = cadena(nombre_ns);
        let mut err: *mut Object = std::ptr::null_mut();
        let entrada: *mut Object = msg_send![class!(AVCaptureDeviceInput), deviceInputWithDevice: dev error: &mut err];
        if entrada.is_null() {
            let d: *mut Object = if err.is_null() { std::ptr::null_mut() } else { msg_send![err, localizedDescription] };
            bail!("{}", crate::tr!("no pude abrir la cámara «{nombre}»: {}", "could not open camera «{nombre}»: {}", cadena(d)));
        }
        let sesion: *mut Object = msg_send![class!(AVCaptureSession), new];
        let salida: *mut Object = msg_send![class!(AVCaptureVideoDataOutput), new];
        let del: *mut Object = msg_send![delegado(), new];
        let cola = dispatch_queue_create(c"gotv.foto".as_ptr(), std::ptr::null_mut());
        let _: () = msg_send![sesion, beginConfiguration];
        let puede_e: BOOL = msg_send![sesion, canAddInput: entrada];
        let puede_s: BOOL = msg_send![sesion, canAddOutput: salida];
        if puede_e != YES || puede_s != YES {
            let _: () = msg_send![sesion, commitConfiguration];
            let _: () = msg_send![sesion, release];
            let _: () = msg_send![salida, release];
            let _: () = msg_send![del, release];
            dispatch_release(cola);
            bail!("{}", crate::tr!("la cámara «{nombre}» no admite la captura (¿la usa otra app?)", "camera «{nombre}» does not accept capture (is another app using it?)"));
        }
        let _: () = msg_send![sesion, addInput: entrada];
        let _: () = msg_send![salida, setSampleBufferDelegate: del queue: cola];
        let _: () = msg_send![sesion, addOutput: salida];
        let _: () = msg_send![sesion, commitConfiguration];
        *ESTADO.lock().unwrap_or_else(|e| e.into_inner()) = (0, 0);
        let _: () = msg_send![sesion, startRunning];
        let t0 = std::time::Instant::now();
        let buf = loop {
            let b = ESTADO.lock().map(|e| e.1).unwrap_or(0);
            if b != 0 || t0.elapsed() > tope {
                break b;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        };
        let _: () = msg_send![sesion, stopRunning];
        let nada: *mut Object = std::ptr::null_mut();
        let _: () = msg_send![salida, setSampleBufferDelegate: nada queue: nada];
        // lo que llegó tarde, fuera (el delegado ya no escribe tras soltarse)
        let tarde = std::mem::take(&mut ESTADO.lock().unwrap_or_else(|e| e.into_inner()).1);
        if tarde != 0 && tarde != buf {
            CVBufferRelease(tarde as *mut c_void);
        }
        let _: () = msg_send![sesion, release];
        let _: () = msg_send![salida, release];
        let _: () = msg_send![del, release];
        dispatch_release(cola);
        if buf == 0 {
            bail!("{}", crate::tr!(
                "la cámara «{nombre}» no dio imagen en {} s (si macOS preguntó por el permiso de cámara, acéptalo y repite)",
                "camera «{nombre}» gave no image in {} s (if macOS asked for camera access, accept it and retry)",
                tope.as_secs()
            ));
        }
        if let Some(dir) = destino.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let img: *mut Object = msg_send![class!(CIImage), imageWithCVPixelBuffer: buf as *mut c_void];
        let ctx: *mut Object = msg_send![class!(CIContext), context];
        let url: *mut Object = msg_send![class!(NSURL), fileURLWithPath: ns(&destino.display().to_string())];
        let opciones: *mut Object = msg_send![class!(NSDictionary), dictionary];
        let cs = CGColorSpaceCreateDeviceRGB();
        let mut err2: *mut Object = std::ptr::null_mut();
        let ok: BOOL = msg_send![ctx, writeJPEGRepresentationOfImage: img toURL: url colorSpace: cs options: opciones error: &mut err2];
        CGColorSpaceRelease(cs);
        CVBufferRelease(buf as *mut c_void);
        if ok != YES {
            let d: *mut Object = if err2.is_null() { std::ptr::null_mut() } else { msg_send![err2, localizedDescription] };
            bail!("{}", crate::tr!("no pude guardar la foto: {}", "could not save the photo: {}", cadena(d)));
        }
        Ok(nombre)
    }
}

/// La foto, sin bloquear el bucle de tokio (40 s de tope: el permiso de macOS la primera vez).
pub async fn foto(quiere: Option<String>, destino: std::path::PathBuf) -> Result<String> {
    tokio::task::spawn_blocking(move || objc::rc::autoreleasepool(|| foto_bloqueante(quiere.as_deref(), &destino, std::time::Duration::from_secs(40)))).await?
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Esta Mac, de verdad: listar NO enciende ninguna cámara (medido el 2026-11-16 con system_profiler:
    /// la del Mac y la del iPhone por Continuidad; aquí sólo se exige que lo que salga tenga nombre).
    #[test]
    fn listar_en_esta_mac() {
        let v = listar().unwrap();
        assert!(v.iter().all(|c| !c.nombre.is_empty()), "{v:?}");
    }

    /// De verdad: una foto con la cámara de esta Mac (la primera vez macOS pide el permiso de cámara a la
    /// app que corre la prueba). `cargo test -p gotv-core foto_de_verdad -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn foto_de_verdad() {
        let d = crate::CarpetaPrueba::nueva("gotv-foto-real");
        eprintln!("permiso de cámara: {}", permiso());
        let p = d.join("foto.jpg");
        let t = std::time::Instant::now();
        let c = objc::rc::autoreleasepool(|| foto_bloqueante(None, &p, std::time::Duration::from_secs(60))).unwrap();
        let bytes = std::fs::read(&p).unwrap();
        eprintln!("{c}: {} bytes en {:.1} s", bytes.len(), t.elapsed().as_secs_f32());
        assert!(bytes.len() > 10_000 && bytes.starts_with(&[0xFF, 0xD8]), "un JPEG de verdad");
        // y una segunda seguida (el delegado, la cola y el estado se limpian bien)
        let p2 = d.join("foto2.jpg");
        objc::rc::autoreleasepool(|| foto_bloqueante(None, &p2, std::time::Duration::from_secs(60))).unwrap();
        assert!(std::fs::metadata(&p2).unwrap().len() > 10_000);
        // una que no existe
        assert!(foto_bloqueante(Some("no-existe-ninguna"), &d.join("x.jpg"), std::time::Duration::from_secs(5)).is_err());
    }
}
