//! F22 paso 5 · ESP32: identificar el chip (ESP32, S2, S3, C3, C6, H2…), tamaño de memoria y MAC; grabar un
//! firmware; borrar la memoria.
//!
//! Con `espflash` 4.7 COMO BIBLIOTECA (la de esp-rs, en Rust), dentro del proceso: el Portador pidió «solo
//! Rust directo» (2026-11-17), sin programas intermedios. `esptool` (Python) no se usa aunque esté instalado.
//! API verificada en su código (v4.7.0): `Connection::new` + `Flasher::connect` → `device_info`,
//! `erase_flash`, `write_bin_to_flash`. Un ELF necesita gestor de arranque y tabla de particiones del
//! proyecto: se graba la imagen `.bin` completa (la que publican MicroPython, Arduino o ESP-IDF).
//! Sin medir con una placa real: no hay ninguna conectada a esta Mac.
use anyhow::{anyhow, bail, Result};
use espflash::connection::{Connection, ResetAfterOperation, ResetBeforeOperation};
use espflash::flasher::Flasher;
use std::path::Path;
use std::time::Duration;

/// Lo que dice la placa.
#[derive(Debug, Default, PartialEq, serde::Serialize)]
pub struct Chip {
    pub chip: String,
    pub revision: String,
    pub memoria: String,
    pub funciones: String,
    pub mac: String,
}

/// «esp32s3» → «ESP32-S3».
pub fn nombre_chip(n: &str) -> String {
    let n = n.to_ascii_lowercase();
    match n.strip_prefix("esp32") {
        Some("") => "ESP32".into(),
        Some(resto) => format!("ESP32-{}", resto.trim_start_matches('-').to_ascii_uppercase()),
        None => n.to_ascii_uppercase(),
    }
}

/// Tamaño de memoria en texto («16 MB»).
pub fn memoria(bytes: u32) -> String {
    if bytes >= 1 << 20 {
        format!("{} MB", bytes >> 20)
    } else {
        format!("{} KB", bytes >> 10)
    }
}

impl Chip {
    pub fn linea(&self) -> String {
        let mut l = self.chip.clone();
        if !self.memoria.is_empty() {
            l.push_str(&format!(", {}", self.memoria));
        }
        if !self.revision.is_empty() {
            l.push_str(&crate::tr!(" (revisión {})", " (revision {})", self.revision));
        }
        if !self.funciones.is_empty() {
            l.push_str(&format!(" · {}", self.funciones));
        }
        if !self.mac.is_empty() {
            l.push_str(&format!(" · MAC {}", self.mac));
        }
        l
    }
}

/// Convierte el error en lo que hay que hacer (la trampa del botón BOOT, el puerto ocupado).
pub fn explicar(error: &str, puerto: &str) -> String {
    let t = error.trim();
    let consejo = if t.contains("Failed to connect") || t.contains("Wrong boot mode") || t.contains("no sync reply") || t.contains("Timeout") {
        crate::i18n::t(
            "la placa no entró en modo de grabación: mantén pulsado el botón BOOT (o IO0), pulsa y suelta RESET (EN), suelta BOOT y repite. Algunas placas lo piden siempre",
            "the board did not enter download mode: hold the BOOT (or IO0) button, press and release RESET (EN), release BOOT and retry. Some boards always need it",
        )
        .to_string()
    } else if t.contains("not found") || t.contains("No such file") {
        crate::tr!("{puerto} no existe: ¿se desconectó la placa? (action=listar que=puertos)", "{puerto} does not exist: was the board unplugged? (action=listar que=puertos)")
    } else if t.contains("busy") || t.contains("Resource busy") || t.contains("exclusive") {
        match super::serie::quien_lo_tiene(puerto) {
            Some(q) => crate::tr!("{puerto} está ocupado por {q}: ciérralo y repite", "{puerto} is busy with {q}: close it and retry"),
            None => crate::tr!("{puerto} está ocupado por otro programa: ciérralo y repite", "{puerto} is busy with another program: close it and retry"),
        }
    } else {
        String::new()
    };
    if consejo.is_empty() {
        format!("espflash: {t}")
    } else {
        format!("{consejo}\n(espflash: {t})")
    }
}

/// Conecta con el cargador de la placa (la reinicia en modo de grabación).
fn conectar(puerto: &str) -> Result<Flasher> {
    super::serie::validar(puerto)?;
    let info = serialport::available_ports()
        .map_err(|e| anyhow!("{e}"))?
        .into_iter()
        .find(|p| p.port_name == puerto)
        .and_then(|p| match p.port_type {
            serialport::SerialPortType::UsbPort(u) => Some(u),
            _ => None,
        })
        .ok_or_else(|| anyhow!("{}", crate::tr!("{puerto} no es un puerto USB conectado (action=listar que=puertos)", "{puerto} is not a connected USB port (action=listar que=puertos)")))?;
    let serie = serialport::new(puerto, 115_200).timeout(Duration::from_secs(3)).open_native().map_err(|e| anyhow!("{}", explicar(&e.to_string(), puerto)))?;
    let c = Connection::new(serie, info, ResetAfterOperation::HardReset, ResetBeforeOperation::DefaultReset, 115_200);
    Flasher::connect(c, true, true, false, None, None).map_err(|e| anyhow!("{}", explicar(&e.to_string(), puerto)))
}

/// Pregunta al chip qué es.
pub fn identificar(puerto: &str) -> Result<Chip> {
    let mut f = conectar(puerto)?;
    let d = f.device_info().map_err(|e| anyhow!("{}", explicar(&e.to_string(), puerto)))?;
    Ok(Chip {
        chip: nombre_chip(&d.chip.to_string()),
        revision: d.revision.map(|(a, b)| format!("v{a}.{b}")).unwrap_or_default(),
        memoria: memoria(d.flash_size.size()),
        funciones: d.features.join(", "),
        mac: d.mac_address.unwrap_or_default(),
    })
}

pub fn borrar(puerto: &str) -> Result<()> {
    let mut f = conectar(puerto)?;
    f.erase_flash().map_err(|e| anyhow!("{}", explicar(&e.to_string(), puerto)))
}

/// El avance de espflash, hacia el canal de avance de GOTV.
struct Avance<'a>(&'a mut dyn FnMut(u64));
impl espflash::target::ProgressCallbacks for Avance<'_> {
    fn init(&mut self, _addr: u32, _total: usize) {}
    fn update(&mut self, actual: usize) {
        (self.0)(actual as u64)
    }
    fn verifying(&mut self) {}
    fn finish(&mut self, _skipped: bool) {}
}

/// Qué se graba y dónde: una imagen completa (empieza por 0xE9) en 0x0, o una parte en su dirección.
pub fn preparar(archivo: &Path, direccion: Option<&str>) -> Result<(u32, Vec<u8>)> {
    let datos = std::fs::read(archivo).map_err(|e| anyhow!("{}: {e}", archivo.display()))?;
    if datos.is_empty() {
        bail!("{}", crate::tr!("{} está vacío", "{} is empty", archivo.display()));
    }
    if datos.starts_with(b"\x7fELF") {
        bail!("{}", crate::tr!(
            "{} es un ELF: necesita el gestor de arranque y la tabla de particiones de su proyecto; graba la imagen .bin completa (la «merged» o «factory» que publican MicroPython, Arduino y ESP-IDF) en 0x0",
            "{} is an ELF: it needs its project's bootloader and partition table; flash the complete .bin image (the «merged» or «factory» one MicroPython, Arduino and ESP-IDF publish) at 0x0",
            archivo.display()
        ));
    }
    let dir = match direccion {
        Some(d) => {
            let v = match d.strip_prefix("0x").or_else(|| d.strip_prefix("0X")) {
                Some(h) => u32::from_str_radix(h, 16).ok(),
                None => d.parse().ok(),
            };
            v.ok_or_else(|| anyhow!("{}", crate::tr!("dirección «{d}» no válida (p. ej. 0x0 o 0x10000)", "invalid address «{d}» (e.g. 0x0 or 0x10000)")))?
        }
        None if datos[0] == 0xE9 => 0,
        None => bail!("{}", crate::tr!(
            "{} no parece una imagen de ESP32 (no empieza por 0xE9): di la dirección con `direccion` si es una parte (p. ej. 0x10000)",
            "{} does not look like an ESP32 image (does not start with 0xE9): give `direccion` if it is a part (e.g. 0x10000)",
            archivo.display()
        )),
    };
    Ok((dir, datos))
}

pub fn grabar(puerto: &str, dir: u32, datos: &[u8], avance: &mut dyn FnMut(u64)) -> Result<()> {
    let mut f = conectar(puerto)?;
    f.write_bin_to_flash(dir, datos, &mut Avance(avance)).map_err(|e| anyhow!("{}", explicar(&e.to_string(), puerto)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nombres_y_memoria() {
        assert_eq!(nombre_chip("esp32s3"), "ESP32-S3");
        assert_eq!(nombre_chip("esp32"), "ESP32");
        assert_eq!(nombre_chip("esp32c6"), "ESP32-C6");
        assert_eq!(nombre_chip(&espflash::target::Chip::Esp32h2.to_string()), "ESP32-H2", "el nombre que da espflash");
        assert_eq!(memoria(espflash::flasher::FlashSize::_16Mb.size()), "16 MB");
        let c = Chip { chip: "ESP32-S3".into(), memoria: "16 MB".into(), ..Default::default() };
        assert!(c.linea().starts_with("ESP32-S3, 16 MB"));
    }

    #[test]
    fn la_trampa_del_boton_boot_se_explica() {
        assert!(explicar("Failed to connect to the device", "/dev/cu.x").contains("BOOT"));
        assert!(explicar("Wrong boot mode detected (0x13)! The chip needs to be in download mode.", "/dev/cu.x").contains("BOOT"));
        assert!(explicar("Serial port not found", "/dev/cu.no").contains("/dev/cu.no"));
    }

    #[test]
    fn que_se_graba_y_donde() {
        let d = crate::CarpetaPrueba::nueva("gotv-esp-grabar");
        std::fs::write(d.join("app.elf"), b"\x7fELF\x02\x01").unwrap();
        std::fs::write(d.join("micropython.bin"), [0xE9, 3, 2, 0x20, 0, 0]).unwrap();
        std::fs::write(d.join("parte.bin"), [1, 2, 3, 4]).unwrap();
        assert!(preparar(&d.join("app.elf"), None).unwrap_err().to_string().contains("ELF"));
        assert_eq!(preparar(&d.join("micropython.bin"), None).unwrap().0, 0);
        assert!(preparar(&d.join("parte.bin"), None).is_err(), "un .bin que no es imagen pide su dirección");
        assert_eq!(preparar(&d.join("parte.bin"), Some("0x10000")).unwrap().0, 0x10000);
        assert!(preparar(&d.join("parte.bin"), Some("0x1g")).is_err());
    }

    /// Sin placa: un puerto que no es USB se dice, sin colgarse.
    #[test]
    fn sin_placa_se_dice() {
        assert!(identificar("/dev/cu.no-existe-gotv").is_err());
        assert!(identificar("/dev/disk0").is_err());
    }
}
