//! F22 · El Mac por dentro, desde Rust y sin programas intermedios (pedido del Portador, 2026-11-17: «nada
//! que no sea Rust… solo Rust directo»): IOKit y DiskArbitration por FFI para los discos, libproc para saber
//! quién tiene abierto un puerto, y el descriptor de `authopen` recibido por SCM_RIGHTS.
//!
//! Nada de `diskutil`, `ioreg`, `lsof` ni leer sus JSON/plist: se le pregunta al sistema directamente, como
//! lo hacen esos mismos programas.
use core_foundation::base::{CFType, TCFType};
use core_foundation::boolean::CFBoolean;
use core_foundation::dictionary::CFDictionary;
use core_foundation::number::CFNumber;
use core_foundation::string::CFString;
use core_foundation::url::CFURL;
use core_foundation_sys::base::{CFAllocatorRef, CFTypeRef};
use core_foundation_sys::dictionary::CFDictionaryRef;
use core_foundation_sys::runloop::{kCFRunLoopDefaultMode, CFRunLoopGetCurrent, CFRunLoopRef, CFRunLoopRunInMode};
use core_foundation_sys::string::CFStringRef;
use core_foundation_sys::url::CFURLRef;
use std::ffi::{c_char, c_void, CStr, CString};
use std::time::{Duration, Instant};

type IoObject = u32;
type DASessionRef = *const c_void;
type DADiskRef = *const c_void;
type DADissenterRef = *const c_void;
type Hecho = extern "C" fn(DADiskRef, DADissenterRef, *mut c_void);

#[link(name = "IOKit", kind = "framework")]
extern "C" {
    fn IOServiceMatching(name: *const c_char) -> *mut c_void;
    fn IOServiceGetMatchingServices(main: u32, matching: *mut c_void, existing: *mut IoObject) -> i32;
    fn IOIteratorNext(it: IoObject) -> IoObject;
    fn IOObjectRelease(o: IoObject) -> i32;
    fn IORegistryEntryCreateCFProperty(e: IoObject, key: CFStringRef, alloc: CFAllocatorRef, options: u32) -> CFTypeRef;
    fn IORegistryEntrySearchCFProperty(e: IoObject, plane: *const c_char, key: CFStringRef, alloc: CFAllocatorRef, options: u32) -> CFTypeRef;
    fn IORegistryEntryGetParentEntry(e: IoObject, plane: *const c_char, parent: *mut IoObject) -> i32;
    fn IOObjectConformsTo(o: IoObject, class: *const c_char) -> u32;
}

#[link(name = "DiskArbitration", kind = "framework")]
extern "C" {
    fn DASessionCreate(alloc: CFAllocatorRef) -> DASessionRef;
    fn DASessionScheduleWithRunLoop(s: DASessionRef, rl: CFRunLoopRef, mode: CFStringRef);
    fn DASessionUnscheduleFromRunLoop(s: DASessionRef, rl: CFRunLoopRef, mode: CFStringRef);
    fn DADiskCreateFromBSDName(alloc: CFAllocatorRef, s: DASessionRef, name: *const c_char) -> DADiskRef;
    fn DADiskCopyDescription(d: DADiskRef) -> CFDictionaryRef;
    fn DADiskUnmount(d: DADiskRef, options: u32, cb: Option<Hecho>, ctx: *mut c_void);
    fn DADiskEject(d: DADiskRef, options: u32, cb: Option<Hecho>, ctx: *mut c_void);
    fn DADiskMount(d: DADiskRef, path: CFURLRef, options: u32, cb: Option<Hecho>, ctx: *mut c_void);
    fn DADissenterGetStatus(d: DADissenterRef) -> i32;
    fn DADissenterGetStatusString(d: DADissenterRef) -> CFStringRef;
}

extern "C" {
    fn CFRelease(cf: *const c_void);
}

const PLANO: &[u8] = b"IOService\0";
const RECURSIVO_PADRES: u32 = 1 | 2;

/// Lo que el sistema sabe de un disco o una partición (`DADiskCopyDescription` + IOKit).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Medio {
    pub id: String,
    pub entero: bool,
    pub tam: u64,
    pub nombre_medio: String,
    /// «USB», «Disk Image», «Apple Fabric», «SATA»…
    pub protocolo: String,
    pub interno: bool,
    pub escribible: bool,
    /// clase de IOKit del medio («IOMedia», «AppleAPFSMedia» para un contenedor sintetizado)
    pub clase: String,
    pub volumen: String,
    pub montado: String,
    pub formato: String,
    pub libre: Option<u64>,
    /// el disco entero FÍSICO del que sale (para un contenedor APFS sintetizado)
    pub fisico: Option<String>,
    /// lo que distingue a este disco de otro igual: huella de la serie USB o ruta de la imagen
    pub marca: String,
}

fn cadena(v: &CFType) -> Option<String> {
    if let Some(s) = v.downcast::<CFString>() {
        return Some(s.to_string());
    }
    if let Some(d) = v.downcast::<core_foundation::data::CFData>() {
        return Some(String::from_utf8_lossy(d.bytes()).trim_end_matches('\0').to_string());
    }
    None
}

fn clave(d: &CFDictionary<CFString, CFType>, k: &str) -> Option<CFType> {
    d.find(CFString::new(k)).map(|v| v.clone())
}

fn dic_cadena(d: &CFDictionary<CFString, CFType>, k: &str) -> String {
    clave(d, k).and_then(|v| cadena(&v)).unwrap_or_default()
}

fn dic_bool(d: &CFDictionary<CFString, CFType>, k: &str) -> bool {
    clave(d, k).and_then(|v| v.downcast::<CFBoolean>()).map(bool::from).unwrap_or(false)
}

fn dic_num(d: &CFDictionary<CFString, CFType>, k: &str) -> u64 {
    clave(d, k).and_then(|v| v.downcast::<CFNumber>()).and_then(|n| n.to_i64()).unwrap_or(0) as u64
}

fn propiedad(e: IoObject, k: &str, buscar_en_padres: bool) -> Option<CFType> {
    let key = CFString::new(k);
    let r = unsafe {
        if buscar_en_padres {
            IORegistryEntrySearchCFProperty(e, PLANO.as_ptr() as _, key.as_concrete_TypeRef(), std::ptr::null(), RECURSIVO_PADRES)
        } else {
            IORegistryEntryCreateCFProperty(e, key.as_concrete_TypeRef(), std::ptr::null(), 0)
        }
    };
    (!r.is_null()).then(|| unsafe { CFType::wrap_under_create_rule(r) })
}

/// Recorre todos los `IOMedia` del sistema.
fn medios_iokit() -> Vec<(String, IoObject)> {
    let mut out = Vec::new();
    unsafe {
        let m = IOServiceMatching(b"IOMedia\0".as_ptr() as _);
        let mut it: IoObject = 0;
        if IOServiceGetMatchingServices(0, m, &mut it) != 0 {
            return out;
        }
        loop {
            let o = IOIteratorNext(it);
            if o == 0 {
                break;
            }
            match propiedad(o, "BSD Name", false).and_then(|v| cadena(&v)) {
                Some(n) => out.push((n, o)),
                None => {
                    IOObjectRelease(o);
                }
            }
        }
        IOObjectRelease(it);
    }
    out
}

/// El disco entero físico del que cuelga un medio (sube por el árbol de IOKit hasta otro `IOMedia` entero).
fn fisico_de(e: IoObject, propio: &str) -> Option<String> {
    unsafe {
        let mut actual = e;
        let mut liberar = Vec::new();
        let mut hallado = None;
        for _ in 0..32 {
            let mut padre: IoObject = 0;
            if IORegistryEntryGetParentEntry(actual, PLANO.as_ptr() as _, &mut padre) != 0 || padre == 0 {
                break;
            }
            liberar.push(padre);
            actual = padre;
            if IOObjectConformsTo(padre, b"IOMedia\0".as_ptr() as _) != 0 {
                if let Some(n) = propiedad(padre, "BSD Name", false).and_then(|v| cadena(&v)) {
                    if n != propio {
                        hallado = Some(super::discos::entero(&n));
                        break;
                    }
                }
            }
        }
        for p in liberar {
            IOObjectRelease(p);
        }
        hallado
    }
}

fn libre(ruta: &str) -> Option<u64> {
    let c = CString::new(ruta).ok()?;
    let mut s: libc::statfs = unsafe { std::mem::zeroed() };
    (unsafe { libc::statfs(c.as_ptr(), &mut s) } == 0).then(|| s.f_bavail as u64 * s.f_bsize as u64)
}

/// De dónde se montó `/`: «disk3s1s1».
pub fn dispositivo_raiz() -> Option<String> {
    let mut s: libc::statfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statfs(c"/".as_ptr(), &mut s) } != 0 {
        return None;
    }
    let d = unsafe { CStr::from_ptr(s.f_mntfromname.as_ptr()) }.to_string_lossy().to_string();
    Some(d.trim_start_matches("/dev/").to_string())
}

/// Todos los discos y particiones del Mac, con lo que dice DiskArbitration y lo que añade IOKit.
pub fn medios() -> Vec<Medio> {
    let mut out = Vec::new();
    unsafe {
        let sesion = DASessionCreate(std::ptr::null());
        if sesion.is_null() {
            return out;
        }
        for (bsd, o) in medios_iokit() {
            let c = CString::new(bsd.clone()).unwrap_or_default();
            let disco = DADiskCreateFromBSDName(std::ptr::null(), sesion, c.as_ptr());
            if disco.is_null() {
                IOObjectRelease(o);
                continue;
            }
            let dref = DADiskCopyDescription(disco);
            CFRelease(disco);
            if dref.is_null() {
                IOObjectRelease(o);
                continue;
            }
            let d: CFDictionary<CFString, CFType> = CFDictionary::wrap_under_create_rule(dref);
            let montado = clave(&d, "DAVolumePath").and_then(|v| v.downcast::<CFURL>()).and_then(|u| u.to_path()).map(|p| p.display().to_string()).unwrap_or_default();
            let mut m = Medio {
                id: bsd.clone(),
                entero: dic_bool(&d, "DAMediaWhole"),
                tam: dic_num(&d, "DAMediaSize"),
                nombre_medio: dic_cadena(&d, "DAMediaName"),
                protocolo: dic_cadena(&d, "DADeviceProtocol"),
                interno: dic_bool(&d, "DADeviceInternal"),
                escribible: dic_bool(&d, "DAMediaWritable"),
                clase: dic_cadena(&d, "DAMediaKind"),
                volumen: dic_cadena(&d, "DAVolumeName"),
                formato: dic_cadena(&d, "DAVolumeKind"),
                libre: if montado.is_empty() { None } else { libre(&montado) },
                montado,
                ..Default::default()
            };
            // medido 2026-11-17: DiskArbitration da DAMediaKind = «IOMedia» (la clase base) también para un
            // contenedor APFS sintetizado; se le pregunta a IOKit si el objeto ES un AppleAPFSMedia
            if IOObjectConformsTo(o, b"AppleAPFSMedia\0".as_ptr() as _) != 0 {
                m.clase = "AppleAPFSMedia".into();
            }
            if m.entero {
                if m.clase == "AppleAPFSMedia" {
                    m.fisico = fisico_de(o, &bsd);
                }
                // la serie del USB (sólo su huella) o la ruta de la imagen: lo que distingue al disco
                if let Some(s) = propiedad(o, "USB Serial Number", true).or_else(|| propiedad(o, "kUSBSerialNumberString", true)).and_then(|v| cadena(&v)) {
                    m.marca = format!("usb:{}", super::usb::huella_serie(&s));
                } else if let Some(i) = propiedad(o, "image-path", true).and_then(|v| cadena(&v)) {
                    m.marca = format!("imagen:{i}");
                }
            }
            IOObjectRelease(o);
            out.push(m);
        }
        CFRelease(sesion);
    }
    out.sort_by(|a, b| a.id.cmp(&b.id));
    out
}

// ───────── desmontar, expulsar, montar (asíncronos en DiskArbitration: se espera su respuesta) ─────────

struct Respuesta {
    hecho: bool,
    error: Option<String>,
}

extern "C" fn al_terminar(_d: DADiskRef, disidente: DADissenterRef, ctx: *mut c_void) {
    let r = unsafe { &mut *(ctx as *mut Respuesta) };
    if !disidente.is_null() {
        let st = unsafe { DADissenterGetStatus(disidente) };
        let s = unsafe { DADissenterGetStatusString(disidente) };
        let texto = if s.is_null() { String::new() } else { unsafe { CFString::wrap_under_get_rule(s) }.to_string() };
        r.error = Some(format!("0x{st:x} {texto}").trim().to_string());
    }
    r.hecho = true;
}

#[derive(Clone, Copy)]
pub enum Orden {
    DesmontarTodo,
    Expulsar,
    MontarTodo,
}

/// Desmonta, expulsa o monta un disco entero y espera la respuesta del sistema (con tope).
pub fn ordenar(id: &str, orden: Orden, tope: Duration) -> Result<(), String> {
    let c = CString::new(id).map_err(|e| e.to_string())?;
    let mut r = Respuesta { hecho: false, error: None };
    unsafe {
        let sesion = DASessionCreate(std::ptr::null());
        if sesion.is_null() {
            return Err("DASessionCreate".into());
        }
        let rl = CFRunLoopGetCurrent();
        DASessionScheduleWithRunLoop(sesion, rl, kCFRunLoopDefaultMode);
        let disco = DADiskCreateFromBSDName(std::ptr::null(), sesion, c.as_ptr());
        if disco.is_null() {
            DASessionUnscheduleFromRunLoop(sesion, rl, kCFRunLoopDefaultMode);
            CFRelease(sesion);
            return Err(id.to_string());
        }
        let ctx = &mut r as *mut Respuesta as *mut c_void;
        match orden {
            Orden::DesmontarTodo => DADiskUnmount(disco, 1, Some(al_terminar), ctx),
            Orden::Expulsar => DADiskEject(disco, 0, Some(al_terminar), ctx),
            Orden::MontarTodo => DADiskMount(disco, std::ptr::null(), 1, Some(al_terminar), ctx),
        }
        let t0 = Instant::now();
        while !r.hecho && t0.elapsed() < tope {
            CFRunLoopRunInMode(kCFRunLoopDefaultMode, 0.1, 1);
        }
        CFRelease(disco);
        DASessionUnscheduleFromRunLoop(sesion, rl, kCFRunLoopDefaultMode);
        CFRelease(sesion);
    }
    if !r.hecho {
        return Err(format!("{} s", tope.as_secs()));
    }
    r.error.map_or(Ok(()), Err)
}

// ───────── quién tiene abierto un archivo (libproc, sin `lsof`) ─────────

/// Procesos que tienen abierto `ruta` («Thonny (pid 4242)»).
pub fn quien_tiene(ruta: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut pids = vec![0i32; 8192];
    let n = unsafe { libc::proc_listallpids(pids.as_mut_ptr() as *mut c_void, (pids.len() * 4) as i32) };
    if n <= 0 {
        return out;
    }
    let tam_fd = std::mem::size_of::<libc::proc_fdinfo>();
    for &pid in pids.iter().take(n as usize).filter(|p| **p > 0) {
        let mut fds = vec![0u8; 4096 * tam_fd];
        let b = unsafe { libc::proc_pidinfo(pid, libc::PROC_PIDLISTFDS, 0, fds.as_mut_ptr() as *mut c_void, fds.len() as i32) };
        if b <= 0 {
            continue;
        }
        for i in 0..(b as usize / tam_fd) {
            let fd: libc::proc_fdinfo = unsafe { std::ptr::read_unaligned(fds.as_ptr().add(i * tam_fd) as *const _) };
            if fd.proc_fdtype as i32 != libc::PROX_FDTYPE_VNODE {
                continue;
            }
            // struct vnode_fdinfowithpath: proc_fileinfo (24) + vnode_info (152) + ruta (MAXPATHLEN)
            let mut v = vec![0u8; 24 + 152 + 1024];
            let r = unsafe { libc::proc_pidfdinfo(pid, fd.proc_fd, 2 /* PROC_PIDFDVNODEPATHINFO */, v.as_mut_ptr() as *mut c_void, v.len() as i32) };
            if r <= 0 {
                continue;
            }
            let p = CStr::from_bytes_until_nul(&v[176..]).map(|c| c.to_string_lossy().to_string()).unwrap_or_default();
            if p == ruta {
                let mut nombre = [0u8; 256];
                unsafe { libc::proc_name(pid, nombre.as_mut_ptr() as *mut c_void, 256) };
                let nombre = CStr::from_bytes_until_nul(&nombre).map(|c| c.to_string_lossy().to_string()).unwrap_or_default();
                out.push(format!("{nombre} (pid {pid})"));
                break;
            }
        }
    }
    out
}

// ───────── authopen: el cuadro de administrador de macOS, y el descriptor de vuelta ─────────

/// Abre `ruta` con autorización de administrador (el cuadro de macOS; GOTV no ve la contraseña) y recibe el
/// descriptor por SCM_RIGHTS (`authopen -stdoutpipe`): a partir de ahí lee y escribe GOTV, en Rust.
pub fn abrir_con_autorizacion(ruta: &str, escribir: bool) -> std::io::Result<std::fs::File> {
    use std::os::fd::{FromRawFd, OwnedFd};
    let mut par = [0i32; 2];
    if unsafe { libc::socketpair(libc::AF_UNIX, libc::SOCK_STREAM, 0, par.as_mut_ptr()) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    // nuestro extremo no lo hereda nadie; el suyo sí, como salida de authopen
    unsafe { libc::fcntl(par[0], libc::F_SETFD, libc::FD_CLOEXEC) };
    let (nuestro, suyo) = unsafe { (OwnedFd::from_raw_fd(par[0]), OwnedFd::from_raw_fd(par[1])) };
    let flags = if escribir { libc::O_RDWR } else { libc::O_RDONLY };
    let st = std::process::Command::new("/usr/libexec/authopen")
        .args(["-stdoutpipe", "-o", &flags.to_string(), ruta])
        .stdout(std::process::Stdio::from(suyo))
        .status()?;
    if !st.success() {
        return Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied, "authopen"));
    }
    use std::os::fd::AsRawFd;
    let mut dato = [0u8; 64];
    let mut iov = libc::iovec { iov_base: dato.as_mut_ptr() as *mut c_void, iov_len: dato.len() };
    let mut control = [0u8; 64];
    let mut msg: libc::msghdr = unsafe { std::mem::zeroed() };
    msg.msg_iov = &mut iov;
    msg.msg_iovlen = 1;
    msg.msg_control = control.as_mut_ptr() as *mut c_void;
    msg.msg_controllen = control.len() as _;
    if unsafe { libc::recvmsg(nuestro.as_raw_fd(), &mut msg, 0) } < 0 {
        return Err(std::io::Error::last_os_error());
    }
    unsafe {
        let c = libc::CMSG_FIRSTHDR(&msg);
        if c.is_null() || (*c).cmsg_type != libc::SCM_RIGHTS {
            return Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied, crate::i18n::t("authopen no devolvió el descriptor", "authopen did not return the descriptor")));
        }
        let fd = std::ptr::read_unaligned(libc::CMSG_DATA(c) as *const i32);
        Ok(std::fs::File::from_raw_fd(fd))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// En ESTA Mac, de verdad (sólo lee): el sistema dice que disk0 es interno y entero, y `/` sale de un
    /// contenedor APFS sintetizado cuyo disco físico es disk0.
    #[test]
    fn el_sistema_describe_sus_discos() {
        let v = medios();
        let d0 = v.iter().find(|m| m.id == "disk0").expect("disk0");
        assert!(d0.entero && d0.interno && d0.tam > 100_000_000_000, "{d0:?}");
        let raiz = dispositivo_raiz().unwrap();
        assert!(raiz.starts_with("disk"), "{raiz}");
        let ent = super::super::discos::entero(&raiz);
        let r = v.iter().find(|m| m.id == ent).unwrap();
        assert_eq!(r.clase, "AppleAPFSMedia", "{r:?}");
        assert_eq!(r.fisico.as_deref(), Some("disk0"));
        assert!(v.iter().any(|m| m.montado == "/"), "el volumen de arranque se ve montado en /");
    }

    /// libproc encuentra a este mismo proceso con un pseudoterminal abierto.
    #[test]
    fn quien_tiene_abierto_un_terminal() {
        let (a, _b) = crate::equipos::serie::par_pty().unwrap();
        use std::os::fd::AsRawFd;
        let ruta = unsafe { CStr::from_ptr(libc::ttyname(a.as_raw_fd())) }.to_string_lossy().to_string();
        let q = quien_tiene(&ruta);
        assert!(q.iter().any(|x| x.contains(&format!("pid {}", std::process::id()))), "{ruta}: {q:?}");
        assert!(quien_tiene("/dev/no-existe-gotv").is_empty());
    }
}
