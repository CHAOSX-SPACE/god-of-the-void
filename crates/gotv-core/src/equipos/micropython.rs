//! F22 paso 4 · MicroPython sin Python: cliente propio del modo «raw REPL» sobre el puerto serie. Ejecutar
//! código, listar, leer, escribir y borrar archivos de la placa, y reiniciarla. Sustituye a `mpremote`.
//!
//! El protocolo (el mismo que usa `mpremote`): Ctrl-C dos veces interrumpe lo que corra; Ctrl-A entra al
//! modo crudo («raw REPL; CTRL-B to exit\r\n>»); se manda el código y Ctrl-D; la placa contesta «OK», la
//! salida, Ctrl-D, los errores, Ctrl-D y «>». Las variables viven entre ejecuciones: así se escribe un
//! archivo grande por tramos sin mandar un código enorme de una vez.
//!
//! Ojo (Banderillas, la CrowPanel ESP32-S3 del Portador): entrar al modo crudo DETIENE el programa que la
//! placa esté corriendo (el tablero de banderas se para hasta reiniciar). Se dice en el resultado.
use anyhow::{anyhow, bail, Result};
use std::io::{Read, Write};
use std::time::{Duration, Instant};

const RAW: &[u8] = b"raw REPL; CTRL-B to exit\r\n>";

pub struct Repl<P: Read + Write> {
    p: P,
    pendiente: Vec<u8>,
}

/// La salida de una ejecución: lo impreso, los errores (Traceback) y si se cortó por tiempo.
#[derive(Debug, Default)]
pub struct Salida {
    pub stdout: String,
    pub stderr: String,
    pub cortado: bool,
}

fn literal(s: &str) -> Result<String> {
    if s.contains(['\n', '\r', '\0']) {
        bail!("{}", crate::i18n::t("la ruta no puede llevar saltos de línea", "the path cannot contain line breaks"));
    }
    Ok(format!("'{}'", s.replace('\\', "\\\\").replace('\'', "\\'")))
}

impl<P: Read + Write> Repl<P> {
    pub fn nuevo(p: P) -> Self {
        Repl { p, pendiente: Vec::new() }
    }

    fn leer_hasta(&mut self, fin: &[u8], tope: Duration) -> Result<(Vec<u8>, bool)> {
        let t0 = Instant::now();
        let mut buf = [0u8; 1024];
        loop {
            if let Some(i) = self.pendiente.windows(fin.len()).position(|w| w == fin) {
                let resto = self.pendiente.split_off(i + fin.len());
                let mut hecho = std::mem::replace(&mut self.pendiente, resto);
                hecho.truncate(i);
                return Ok((hecho, true));
            }
            if t0.elapsed() >= tope {
                return Ok((std::mem::take(&mut self.pendiente), false));
            }
            match self.p.read(&mut buf) {
                Ok(0) => std::thread::sleep(Duration::from_millis(10)),
                Ok(n) => self.pendiente.extend_from_slice(&buf[..n]),
                Err(e) if matches!(e.kind(), std::io::ErrorKind::Interrupted | std::io::ErrorKind::WouldBlock) => std::thread::sleep(Duration::from_millis(10)),
                Err(e) => return Err(anyhow!("{}", crate::tr!("la placa dejó de responder ({e}): ¿se desconectó o se reinició?", "the board stopped answering ({e}): was it unplugged or did it reset?"))),
            }
        }
    }

    fn mandar(&mut self, b: &[u8]) -> Result<()> {
        self.p.write_all(b)?;
        self.p.flush()?;
        Ok(())
    }

    /// Interrumpe lo que corra y entra al modo crudo.
    pub fn entrar(&mut self) -> Result<()> {
        self.mandar(b"\r\x03\x03")?;
        std::thread::sleep(Duration::from_millis(100));
        let _ = self.leer_hasta(b"\x00\x00\x00never", Duration::from_millis(200))?; // vacía lo que hubiera
        self.pendiente.clear();
        self.mandar(b"\r\x01")?;
        let (_, ok) = self.leer_hasta(RAW, Duration::from_secs(3))?;
        if !ok {
            bail!("{}", crate::i18n::t(
                "la placa no entró al modo crudo de MicroPython: ¿lleva MicroPython? ¿hay otro programa usándola o un programa que no se deja interrumpir?",
                "the board did not enter MicroPython raw mode: does it run MicroPython? is another program using it, or a program that cannot be interrupted?"
            ));
        }
        Ok(())
    }

    /// Ejecuta código y devuelve su salida. Si pasa de `tope`, se interrumpe (Ctrl-C) y se dice.
    pub fn ejecutar(&mut self, codigo: &str, tope: Duration) -> Result<Salida> {
        for tramo in codigo.as_bytes().chunks(256) {
            self.mandar(tramo)?;
            std::thread::sleep(Duration::from_millis(10));
        }
        self.mandar(b"\x04")?;
        let (ok, _) = self.leer_hasta(b"OK", Duration::from_secs(3))?;
        if !ok.is_empty() && ok.iter().any(|b| !b.is_ascii_whitespace() && *b != b'>') {
            bail!("{}", crate::tr!("la placa no aceptó el código: {}", "the board did not accept the code: {}", String::from_utf8_lossy(&ok).trim()));
        }
        let (out, fin) = self.leer_hasta(b"\x04", tope)?;
        if !fin {
            // pasó del tope: se interrumpe y se recoge lo que haya
            self.mandar(b"\x03")?;
            let (resto, _) = self.leer_hasta(b"\x04", Duration::from_secs(2))?;
            let (err, _) = self.leer_hasta(b"\x04", Duration::from_secs(2))?;
            let _ = self.leer_hasta(b">", Duration::from_secs(1))?;
            let mut o = out;
            o.extend_from_slice(&resto);
            return Ok(Salida { stdout: texto(&o), stderr: texto(&err), cortado: true });
        }
        let (err, _) = self.leer_hasta(b"\x04", Duration::from_secs(5))?;
        let _ = self.leer_hasta(b">", Duration::from_secs(2))?;
        Ok(Salida { stdout: texto(&out), stderr: texto(&err), cortado: false })
    }

    fn correr(&mut self, codigo: &str) -> Result<String> {
        let s = self.ejecutar(codigo, Duration::from_secs(20))?;
        if !s.stderr.trim().is_empty() {
            bail!("{}", s.stderr.trim());
        }
        Ok(s.stdout)
    }

    /// (nombre, es carpeta, tamaño)
    pub fn listar(&mut self, ruta: &str) -> Result<Vec<(String, bool, i64)>> {
        let r = literal(if ruta.is_empty() { "/" } else { ruta })?;
        let t = self.correr(&format!("import os\nfor e in os.ilistdir({r}):\n print('%s\\t%d\\t%d' % (e[0], e[1], e[3] if len(e) > 3 else -1))\n"))?;
        Ok(t.lines()
            .filter_map(|l| {
                let mut c = l.split('\t');
                let n = c.next()?.to_string();
                let tipo: u32 = c.next()?.trim().parse().ok()?;
                let tam: i64 = c.next().and_then(|x| x.trim().parse().ok()).unwrap_or(-1);
                Some((n, tipo & 0x4000 != 0, tam))
            })
            .collect())
    }

    pub fn leer(&mut self, ruta: &str) -> Result<Vec<u8>> {
        let r = literal(ruta)?;
        let t = self.correr(&format!("import ubinascii\nf=open({r},'rb')\nwhile 1:\n b=f.read(256)\n if not b:break\n print(ubinascii.hexlify(b).decode())\nf.close()\n"))?;
        let mut out = Vec::new();
        for l in t.lines().map(str::trim).filter(|l| !l.is_empty()) {
            out.extend(desde_hex(l).ok_or_else(|| anyhow!("{}", crate::i18n::t("la placa devolvió algo que no es el archivo", "the board returned something that is not the file")))?);
        }
        Ok(out)
    }

    /// Escribe el archivo por tramos (cada ejecución lleva hasta 2 KB del archivo).
    pub fn escribir(&mut self, ruta: &str, datos: &[u8]) -> Result<()> {
        let r = literal(ruta)?;
        self.correr(&format!("import ubinascii\nf=open({r},'wb')\nw=f.write\nu=ubinascii.unhexlify\n"))?;
        for bloque in datos.chunks(2048) {
            let mut c = String::new();
            for tramo in bloque.chunks(256) {
                c.push_str(&format!("w(u('{}'))\n", a_hex(tramo)));
            }
            self.correr(&c)?;
        }
        self.correr("f.close()\n")?;
        Ok(())
    }

    pub fn borrar(&mut self, ruta: &str) -> Result<()> {
        let r = literal(ruta)?;
        self.correr(&format!("import os\nos.remove({r})\n")).map(|_| ())
    }

    /// Reinicio suave (Ctrl-D en el modo crudo: vuelve a correr `main.py`) y salida del modo crudo.
    pub fn reiniciar(&mut self) -> Result<()> {
        self.mandar(b"\x04")?;
        let _ = self.leer_hasta(RAW, Duration::from_secs(5))?;
        self.salir()
    }

    pub fn salir(&mut self) -> Result<()> {
        self.mandar(b"\r\x02")
    }
}

fn texto(b: &[u8]) -> String {
    String::from_utf8_lossy(b).replace("\r\n", "\n")
}

pub fn a_hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

pub fn desde_hex(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 {
        return None;
    }
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok()).collect()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::collections::BTreeMap;

    /// Una placa MicroPython FALSA al otro lado de un pseudoterminal: habla el modo crudo y entiende las
    /// formas de código que manda el cliente (listar, leer, escribir por tramos, borrar, print, error).
    /// Prueba el protocolo y el reparto de la salida; el código de MicroPython en sí queda sin medir con la
    /// placa real.
    pub fn placa_falsa(mut p: std::fs::File) -> std::thread::JoinHandle<BTreeMap<String, Vec<u8>>> {
        std::thread::spawn(move || {
            let mut fs: BTreeMap<String, Vec<u8>> = BTreeMap::new();
            fs.insert("boot.py".into(), b"# boot\n".to_vec());
            let (mut abierto, mut codigo, mut crudo) = (None::<String>, Vec::<u8>::new(), false);
            let mut b = [0u8; 4096];
            let t0 = Instant::now();
            while t0.elapsed() < Duration::from_secs(20) {
                let n = match p.read(&mut b) {
                    Ok(0) | Err(_) => {
                        std::thread::sleep(Duration::from_millis(5));
                        continue;
                    }
                    Ok(n) => n,
                };
                for &c in &b[..n] {
                    match c {
                        0x03 => {
                            codigo.clear();
                            if !crudo {
                                let _ = p.write_all(b"\r\nKeyboardInterrupt\r\n>>> ");
                            }
                        }
                        0x01 => {
                            crudo = true;
                            codigo.clear();
                            let _ = p.write_all(RAW);
                        }
                        0x02 => {
                            crudo = false;
                            let _ = p.write_all(b"\r\n>>> ");
                        }
                        0x04 if crudo && codigo.iter().all(|x| x.is_ascii_whitespace()) => {
                            let _ = p.write_all(b"OK\x04\x04>");
                            let _ = p.write_all(b"MPY: soft reboot\r\n");
                            let _ = p.write_all(RAW);
                            codigo.clear();
                        }
                        0x04 if crudo => {
                            let t = String::from_utf8_lossy(&codigo).to_string();
                            codigo.clear();
                            let (mut out, mut err) = (String::new(), String::new());
                            let ruta = |t: &str| t.split('\'').nth(1).unwrap_or("").to_string();
                            if t.contains("os.ilistdir(") {
                                for (k, v) in &fs {
                                    out.push_str(&format!("{k}\t32768\t{}\r\n", v.len()));
                                }
                            } else if t.contains(",'wb')") {
                                abierto = Some(ruta(&t));
                                fs.insert(ruta(&t), Vec::new());
                            } else if t.starts_with("w(u('") {
                                for l in t.lines() {
                                    let h = l.trim_start_matches("w(u('").trim_end_matches("'))");
                                    fs.get_mut(abierto.as_ref().unwrap()).unwrap().extend(desde_hex(h).unwrap());
                                }
                            } else if t.starts_with("f.close()") {
                                abierto = None;
                            } else if t.contains(",'rb')") {
                                match fs.get(&ruta(&t)) {
                                    Some(v) => {
                                        for tramo in v.chunks(256) {
                                            out.push_str(&format!("{}\r\n", a_hex(tramo)));
                                        }
                                    }
                                    None => err = "Traceback (most recent call last):\r\n  File \"<stdin>\", line 2\r\nOSError: [Errno 2] ENOENT\r\n".into(),
                                }
                            } else if t.contains("os.remove(") {
                                fs.remove(&ruta(&t));
                            } else if t.contains("1/0") {
                                err = "Traceback (most recent call last):\r\nZeroDivisionError: divide by zero\r\n".into();
                            } else if t.contains("while True") {
                                // un bucle sin fin: imprime hasta que llegue Ctrl-C
                                let _ = p.write_all(b"OKvuelta\r\n");
                                loop {
                                    let mut x = [0u8; 8];
                                    if let Ok(k) = p.read(&mut x) {
                                        if x[..k].contains(&0x03) {
                                            break;
                                        }
                                    }
                                }
                                let _ = p.write_all(b"\x04Traceback (most recent call last):\r\nKeyboardInterrupt: \r\n\x04>");
                                continue;
                            } else if let Some(i) = t.find("print(") {
                                out = t[i + 6..].split(')').next().unwrap_or("").trim_matches('\'').to_string() + "\r\n";
                            }
                            let _ = p.write_all(format!("OK{out}\x04{err}\x04>").as_bytes());
                        }
                        _ if crudo => codigo.push(c),
                        _ => {}
                    }
                }
                if !crudo && t0.elapsed() > Duration::from_millis(500) && codigo.is_empty() && fs.contains_key("__fin__") {
                    break;
                }
            }
            fs
        })
    }

    fn conectar() -> (Repl<crate::equipos::serie::Serie>, std::thread::JoinHandle<BTreeMap<String, Vec<u8>>>) {
        let (gotv, placa) = crate::equipos::serie::par_pty().unwrap();
        let h = placa_falsa(placa);
        (Repl::nuevo(crate::equipos::serie::Serie::de_archivo(gotv).unwrap()), h)
    }

    /// El flujo de Banderillas sin Python: entrar, subir un main.py, listarlo, leerlo igual, correr código,
    /// ver el Traceback, borrar y reiniciar.
    #[test]
    fn el_cliente_raw_repl_contra_una_placa_falsa() {
        let (mut r, _h) = conectar();
        r.entrar().unwrap();
        let s = r.ejecutar("print('hola')\n", Duration::from_secs(3)).unwrap();
        assert_eq!(s.stdout, "hola\n");
        assert!(s.stderr.is_empty() && !s.cortado);
        // un main.py de 5 KB (varios tramos de 2 KB) con bytes de todo tipo
        let main: Vec<u8> = (0..5000u32).map(|i| (i * 7 % 256) as u8).collect();
        r.escribir("main.py", &main).unwrap();
        let l = r.listar("/").unwrap();
        assert!(l.contains(&("main.py".to_string(), false, 5000)), "{l:?}");
        assert_eq!(r.leer("main.py").unwrap(), main);
        let e = r.ejecutar("1/0\n", Duration::from_secs(3)).unwrap();
        assert!(e.stderr.contains("ZeroDivisionError"), "{e:?}");
        assert!(r.leer("no-existe.py").unwrap_err().to_string().contains("ENOENT"));
        r.borrar("main.py").unwrap();
        assert!(!r.listar("/").unwrap().iter().any(|x| x.0 == "main.py"));
        r.reiniciar().unwrap();
    }

    /// Un programa sin fin se corta al tope (Ctrl-C) y se dice, sin colgar a GOTV.
    #[test]
    fn un_bucle_sin_fin_se_corta_al_tope() {
        let (mut r, _h) = conectar();
        r.entrar().unwrap();
        let s = r.ejecutar("while True:\n print('vuelta')\n", Duration::from_millis(400)).unwrap();
        assert!(s.cortado);
        assert!(s.stdout.contains("vuelta"));
    }

    #[test]
    fn rutas_y_hex() {
        assert_eq!(literal("a'b").unwrap(), "'a\\'b'");
        assert!(literal("a\nimport os").is_err(), "no se cuela código por la ruta");
        assert_eq!(desde_hex(&a_hex(&[0, 255, 16])).unwrap(), vec![0, 255, 16]);
        assert!(desde_hex("abc").is_none());
    }

    /// Una «placa» que no habla MicroPython (no contesta al modo crudo): se dice, no se cuelga.
    #[test]
    fn una_placa_sin_micropython_se_dice() {
        let (gotv, _placa) = crate::equipos::serie::par_pty().unwrap();
        let mut r = Repl::nuevo(crate::equipos::serie::Serie::de_archivo(gotv).unwrap());
        let e = r.entrar().unwrap_err().to_string();
        assert!(e.contains("MicroPython"), "{e}");
    }
}
