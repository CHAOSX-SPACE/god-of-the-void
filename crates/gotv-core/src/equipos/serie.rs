//! F22 paso 3 · Puertos serie: cuáles hay y qué equipo USB tienen detrás, quién los tiene abiertos, y el
//! monitor serie. El puerto se abre con `termios` (libc) en modo crudo, desde Rust.
//!
//! Trampa de la ficha: otro programa con el puerto abierto (Thonny, un monitor serie, el IDE de Arduino)
//! bloquea la placa: se le pregunta al núcleo quién lo tiene (libproc, `mac::quien_tiene`) y se dice.
use anyhow::{anyhow, bail, Result};
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, FromRawFd};
use std::path::Path;
use std::time::{Duration, Instant};

/// Un puerto serie con el equipo USB que tiene detrás (si lo hay).
#[derive(Clone, Debug, serde::Serialize)]
pub struct Puerto {
    pub ruta: String,
    pub usb: Option<super::usb::Usb>,
}

/// Los puertos de llamada (`/dev/cu.*`), sin los del propio Mac que no son de ningún equipo.
pub fn listar() -> Vec<Puerto> {
    let usb = super::usb::listar().unwrap_or_default();
    let mut v: Vec<Puerto> = std::fs::read_dir("/dev")
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| e.file_name().to_str().map(String::from))
        .filter(|n| n.starts_with("cu.") && !matches!(n.as_str(), "cu.Bluetooth-Incoming-Port" | "cu.debug-console"))
        .map(|n| {
            let ruta = format!("/dev/{n}");
            Puerto { usb: usb.iter().find(|u| u.puertos.contains(&ruta)).cloned(), ruta }
        })
        .collect();
    v.sort_by(|a, b| a.ruta.cmp(&b.ruta));
    v
}

pub fn linea(p: &Puerto) -> String {
    match &p.usb {
        Some(u) => format!("{} ← {}", p.ruta, super::usb::linea(u)),
        None => p.ruta.clone(),
    }
}

/// Sólo rutas de puertos serie (el modelo no abre otra cosa con esto).
pub fn validar(ruta: &str) -> Result<()> {
    let ok = (ruta.starts_with("/dev/cu.") || ruta.starts_with("/dev/tty.")) && !ruta.contains("..") && ruta.len() < 128;
    if !ok {
        bail!("{}", crate::tr!("«{ruta}» no es un puerto serie (/dev/cu.…): mira action=listar que=puertos", "«{ruta}» is not a serial port (/dev/cu.…): see action=listar que=puertos"));
    }
    Ok(())
}

/// ¿Quién tiene abierto el puerto? (proceso y pid, preguntado al núcleo con libproc, sin `lsof`)
pub fn quien_lo_tiene(ruta: &str) -> Option<String> {
    let v: Vec<String> = super::mac::quien_tiene(ruta).into_iter().filter(|q| !q.ends_with(&format!("(pid {})", std::process::id()))).collect();
    (!v.is_empty()).then(|| v.join(", "))
}

fn velocidad(baudios: u32) -> Option<libc::speed_t> {
    Some(match baudios {
        9600 => libc::B9600,
        19200 => libc::B19200,
        38400 => libc::B38400,
        57600 => libc::B57600,
        115200 => libc::B115200,
        230400 => libc::B230400,
        // macOS acepta velocidades arbitrarias en cfsetspeed (460800, 921600…)
        b if b >= 300 => b as libc::speed_t,
        _ => return None,
    })
}

/// Pone en modo crudo (8N1, sin eco ni control de flujo) un descriptor de terminal.
pub fn crudo(fd: i32, baudios: u32) -> Result<()> {
    unsafe {
        let mut t: libc::termios = std::mem::zeroed();
        if libc::tcgetattr(fd, &mut t) != 0 {
            bail!("tcgetattr: {}", std::io::Error::last_os_error());
        }
        libc::cfmakeraw(&mut t);
        t.c_cflag |= libc::CLOCAL | libc::CREAD;
        t.c_cflag &= !(libc::CRTSCTS | libc::CSTOPB | libc::PARENB);
        t.c_cc[libc::VMIN] = 0;
        t.c_cc[libc::VTIME] = 1; // lecturas con tope de 100 ms
        let v = velocidad(baudios).ok_or_else(|| anyhow!("{}", crate::tr!("velocidad {baudios} no válida", "invalid speed {baudios}")))?;
        libc::cfsetspeed(&mut t, v);
        if libc::tcsetattr(fd, libc::TCSANOW, &t) != 0 {
            bail!("tcsetattr: {}", std::io::Error::last_os_error());
        }
    }
    Ok(())
}

/// Un puerto serie abierto en modo crudo y en exclusiva.
pub struct Serie {
    pub f: std::fs::File,
}

impl Serie {
    pub fn abrir(ruta: &str, baudios: u32) -> Result<Serie> {
        validar(ruta)?;
        let c = std::ffi::CString::new(ruta)?;
        // O_CLOEXEC: sin él, cada proceso hijo de GOTV (un `bash`, otra prueba) HEREDA el puerto de la placa y
        // aparece como «el otro programa que lo tiene» (medido 2026-11-17 en las pruebas en paralelo)
        let fd = unsafe { libc::open(c.as_ptr(), libc::O_RDWR | libc::O_NOCTTY | libc::O_NONBLOCK | libc::O_CLOEXEC) };
        if fd < 0 {
            let e = std::io::Error::last_os_error();
            let quien = quien_lo_tiene(ruta);
            return Err(match (e.raw_os_error(), quien) {
                (_, Some(q)) => anyhow!("{}", crate::tr!("{ruta} está ocupado por {q}: ciérralo (o su monitor serie) y repite", "{ruta} is busy with {q}: close it (or its serial monitor) and retry")),
                (Some(libc::ENOENT), _) => anyhow!("{}", crate::tr!("{ruta} no existe (¿se desconectó la placa?)", "{ruta} does not exist (was the board unplugged?)")),
                _ => anyhow!("{ruta}: {e}"),
            });
        }
        let f = unsafe { std::fs::File::from_raw_fd(fd) };
        unsafe {
            // en exclusiva: nadie más lo abre mientras GOTV habla con la placa
            libc::ioctl(fd, libc::TIOCEXCL as _);
            let fl = libc::fcntl(fd, libc::F_GETFL);
            libc::fcntl(fd, libc::F_SETFL, fl & !libc::O_NONBLOCK);
        }
        crudo(fd, baudios)?;
        Ok(Serie { f })
    }

    /// Para las pruebas: un descriptor ya abierto (un pseudoterminal).
    pub fn de_archivo(f: std::fs::File) -> Result<Serie> {
        crudo(f.as_raw_fd(), 115_200)?;
        Ok(Serie { f })
    }
}

impl Read for Serie {
    fn read(&mut self, b: &mut [u8]) -> std::io::Result<usize> {
        self.f.read(b)
    }
}

impl Write for Serie {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.f.write(b)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.f.flush()
    }
}

/// Lee lo que llegue durante `dura` (o hasta `tope` bytes).
pub fn leer_durante<R: Read>(r: &mut R, dura: Duration, tope: usize) -> Vec<u8> {
    let t0 = Instant::now();
    let mut out = Vec::new();
    let mut buf = [0u8; 4096];
    while t0.elapsed() < dura && out.len() < tope {
        match r.read(&mut buf) {
            Ok(0) => std::thread::sleep(Duration::from_millis(20)),
            Ok(n) => out.extend_from_slice(&buf[..n]),
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted || e.kind() == std::io::ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(20)),
            Err(_) => break,
        }
    }
    out
}

/// El monitor serie: manda `enviar` (si hay) y devuelve lo que la placa diga durante `segundos`.
pub fn monitor<P: Read + Write>(p: &mut P, enviar: Option<&str>, segundos: u64) -> Result<String> {
    if let Some(t) = enviar {
        p.write_all(t.replace("\\n", "\n").replace("\\r", "\r").as_bytes())?;
        p.flush()?;
    }
    let b = leer_durante(p, Duration::from_secs(segundos.clamp(1, 120)), 1 << 20);
    Ok(String::from_utf8_lossy(&b).replace("\r\n", "\n"))
}

/// Un par de pseudoterminales: (lado de GOTV, lado de la placa falsa). Para las pruebas de MicroPython.
pub fn par_pty() -> Result<(std::fs::File, std::fs::File)> {
    let (mut m, mut s) = (0, 0);
    let r = unsafe { libc::openpty(&mut m, &mut s, std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut()) };
    if r != 0 {
        bail!("openpty: {}", std::io::Error::last_os_error());
    }
    unsafe {
        // openpty no los marca: que ningún proceso hijo los herede
        libc::fcntl(m, libc::F_SETFD, libc::FD_CLOEXEC);
        libc::fcntl(s, libc::F_SETFD, libc::FD_CLOEXEC);
    }
    let (m, s) = unsafe { (std::fs::File::from_raw_fd(m), std::fs::File::from_raw_fd(s)) };
    crudo(m.as_raw_fd(), 115_200)?;
    crudo(s.as_raw_fd(), 115_200)?;
    Ok((s, m))
}

pub fn existe(ruta: &str) -> bool {
    Path::new(ruta).exists()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn solo_rutas_de_puertos_serie() {
        assert!(validar("/dev/cu.usbmodem1101").is_ok());
        assert!(validar("/dev/disk0").is_err());
        assert!(validar("/dev/cu.../../disk0").is_err());
        assert!(validar("/etc/passwd").is_err());
    }

    /// GOTV mismo no cuenta como «el otro programa» que bloquea la placa.
    #[test]
    fn quien_tiene_el_puerto_sin_contarse_a_si_mismo() {
        let (a, _b) = par_pty().unwrap();
        let ruta = unsafe { std::ffi::CStr::from_ptr(libc::ttyname(a.as_raw_fd())) }.to_string_lossy().to_string();
        assert!(!super::super::mac::quien_tiene(&ruta).is_empty(), "el núcleo lo ve abierto");
        assert_eq!(quien_lo_tiene(&ruta), None, "pero es GOTV mismo");
    }

    /// El monitor sobre un par de pseudoterminales: lo enviado llega a la «placa» y lo que dice vuelve.
    #[test]
    fn el_monitor_serie_con_una_placa_falsa() {
        let (gotv, mut placa) = par_pty().unwrap();
        let mut p = Serie::de_archivo(gotv).unwrap();
        let h = std::thread::spawn(move || {
            let mut b = [0u8; 64];
            let mut recibido = Vec::new();
            let t0 = Instant::now();
            while !recibido.ends_with(b"\n") && t0.elapsed() < Duration::from_secs(3) {
                if let Ok(n) = placa.read(&mut b) {
                    recibido.extend_from_slice(&b[..n]);
                }
            }
            placa.write_all(b"temperatura=21\r\nok\r\n").unwrap();
            std::thread::sleep(Duration::from_millis(1500));
            recibido
        });
        let out = monitor(&mut p, Some("estado\\n"), 1).unwrap();
        assert_eq!(h.join().unwrap(), b"estado\n");
        assert!(out.contains("temperatura=21\nok"), "{out:?}");
    }

    #[test]
    fn listar_en_esta_mac_no_falla() {
        let v = listar();
        assert!(v.iter().all(|p| p.ruta.starts_with("/dev/cu.") && !p.ruta.contains("Bluetooth")));
    }
}
