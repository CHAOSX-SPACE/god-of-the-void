//! F22 paso 1 · Los equipos USB: lista con fabricante, producto, identificadores y clase, qué ES cada uno
//! (tabla propia de identificadores) y el aviso al conectar o desconectar.
//!
//! En Rust y en proceso: `nusb` (USB en Rust puro) da los equipos; `serialport` (Rust) dice qué puerto
//! `/dev/cu.*` tiene cada uno; IOKit (`mac.rs`) dice qué disco. Nada de `ioreg` ni `system_profiler`.
//!
//! Privacidad: el número de serie NUNCA sale; sólo su huella corta (8 hex de SHA-256), que basta para
//! reconocer el mismo equipo tras desconectarlo.
use anyhow::{anyhow, Result};
use serde::Serialize;
use std::time::{Duration, Instant};

/// Un equipo USB conectado.
#[derive(Clone, Debug, Default, Serialize, PartialEq)]
pub struct Usb {
    pub vid: u16,
    pub pid: u16,
    pub fabricante: String,
    pub producto: String,
    /// huella del número de serie (nunca el número)
    #[serde(skip_serializing_if = "String::is_empty")]
    pub serie: String,
    /// clase del equipo (0 = la dicen sus interfaces)
    pub clase: u8,
    /// `locationID` en hex: el puerto físico (cambia si se enchufa en otro sitio)
    pub ubicacion: String,
    /// puertos serie que cuelgan de él (`/dev/cu.*`)
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub puertos: Vec<String>,
    /// discos enteros que cuelgan de él (`diskN`)
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub discos: Vec<String>,
}

/// Qué es un equipo según sus identificadores (la tabla de la ficha F22).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Que {
    Espressif,
    PuenteSerie(&'static str),
    PicoArranque(&'static str),
    PicoMicroPython,
    PicoSdk,
    RaspberryArranque,
    Arduino,
    Adafruit,
    StLink,
    SondaPico,
    Otro,
}

/// La tabla de identificación (incluida en el binario; verificada con picotool, los controladores de Linux
/// y la tabla de Espressif el 2026-11-16).
pub fn identificar(vid: u16, pid: u16) -> Que {
    match (vid, pid) {
        (0x303a, _) => Que::Espressif,
        (0x10c4, 0xea60) => Que::PuenteSerie("CP210x"),
        (0x1a86, 0x7523) => Que::PuenteSerie("CH340"),
        (0x1a86, 0x55d4) => Que::PuenteSerie("CH9102"),
        (0x0403, 0x6001) => Que::PuenteSerie("FTDI FT232R"),
        (0x0403, 0x6010) => Que::PuenteSerie("FTDI FT2232"),
        (0x0403, 0x6015) => Que::PuenteSerie("FTDI FT-X"),
        (0x2e8a, 0x0003) => Que::PicoArranque("RP2040"),
        (0x2e8a, 0x000f) => Que::PicoArranque("RP2350"),
        (0x2e8a, 0x0005) => Que::PicoMicroPython,
        (0x2e8a, 0x000a) => Que::PicoSdk,
        (0x2e8a, 0x000c) => Que::SondaPico,
        (0x0a5c, p) if p >> 8 == 0x27 => Que::RaspberryArranque,
        (0x2341, _) => Que::Arduino,
        (0x239a, _) => Que::Adafruit,
        (0x0483, p) if (0x3740..=0x374f).contains(&p) => Que::StLink,
        _ => Que::Otro,
    }
}

impl Que {
    pub fn texto(self) -> String {
        match self {
            Que::Espressif => crate::i18n::t("Espressif con USB propio (ESP32-S2, S3, C3, C6 o H2): action=identificar pregunta al chip", "Espressif with native USB (ESP32-S2, S3, C3, C6 or H2): action=identificar asks the chip").into(),
            Que::PuenteSerie(chip) => crate::tr!(
                "puente serie {chip}: casi siempre una placa ESP32 o Arduino detrás (se confirma preguntando al chip con action=identificar)",
                "{chip} serial bridge: almost always an ESP32 or Arduino board behind it (confirmed by asking the chip with action=identificar)"
            ),
            Que::PicoArranque(chip) => crate::tr!("Raspberry Pi Pico ({chip}) en modo de arranque: grabar es copiar un .uf2 (action=grabar)", "Raspberry Pi Pico ({chip}) in boot mode: flashing is copying a .uf2 (action=grabar)"),
            Que::PicoMicroPython => crate::i18n::t("Raspberry Pi Pico con MicroPython (action=ejecutar, action=archivos)", "Raspberry Pi Pico with MicroPython (action=ejecutar, action=archivos)").into(),
            Que::PicoSdk => crate::i18n::t("Raspberry Pi Pico con el puerto serie del SDK", "Raspberry Pi Pico with the SDK serial port").into(),
            Que::RaspberryArranque => crate::i18n::t(
                "Raspberry Pi (Compute Module) en modo de arranque por USB: GOTV la detecta, pero grabarla así queda fuera por ahora",
                "Raspberry Pi (Compute Module) in USB boot mode: GOTV detects it, but flashing it this way is out of scope for now",
            )
            .into(),
            Que::Arduino => crate::i18n::t("Arduino", "Arduino").into(),
            Que::Adafruit => crate::i18n::t("placa de Adafruit", "Adafruit board").into(),
            Que::StLink => crate::i18n::t("sonda de depuración ST-Link (STM32)", "ST-Link debug probe (STM32)").into(),
            Que::SondaPico => crate::i18n::t("Raspberry Pi Debug Probe (CMSIS-DAP)", "Raspberry Pi Debug Probe (CMSIS-DAP)").into(),
            Que::Otro => String::new(),
        }
    }

    /// ¿Puede ser una placa con la que habla `esp.rs`?
    pub fn quiza_esp(self) -> bool {
        matches!(self, Que::Espressif | Que::PuenteSerie(_))
    }
}

/// Huella corta de un número de serie (8 hex de SHA-256): reconoce el equipo sin publicar el número.
pub fn huella_serie(serie: &str) -> String {
    use sha2::{Digest, Sha256};
    if serie.trim().is_empty() {
        return String::new();
    }
    Sha256::digest(serie.trim().as_bytes()).iter().take(4).map(|b| format!("{b:02x}")).collect()
}

/// Un puerto serie USB tal como lo da `serialport`: (ruta, vid, pid, huella de la serie).
pub type PuertoUsb = (String, u16, u16, String);

/// Une cada equipo con sus puertos (mismos identificadores y serie) y sus discos (misma serie).
pub fn unir(mut v: Vec<Usb>, puertos: &[PuertoUsb], discos: &[(String, String)]) -> Vec<Usb> {
    for u in &mut v {
        u.puertos = puertos.iter().filter(|p| p.1 == u.vid && p.2 == u.pid && (p.3 == u.serie || u.serie.is_empty() || p.3.is_empty())).map(|p| p.0.clone()).collect();
        if !u.serie.is_empty() {
            let marca = format!("usb:{}", u.serie);
            u.discos = discos.iter().filter(|d| d.1 == marca).map(|d| d.0.clone()).collect();
        }
        u.puertos.sort();
        u.puertos.dedup();
    }
    v.sort_by(|a, b| a.ubicacion.cmp(&b.ubicacion));
    v
}

/// Los puertos serie USB, de `serialport` (los `/dev/cu.*`).
pub fn puertos_usb() -> Vec<PuertoUsb> {
    serialport::available_ports()
        .unwrap_or_default()
        .into_iter()
        .filter(|p| p.port_name.starts_with("/dev/cu."))
        .filter_map(|p| match p.port_type {
            serialport::SerialPortType::UsbPort(u) => Some((p.port_name, u.vid, u.pid, huella_serie(u.serial_number.as_deref().unwrap_or("")))),
            _ => None,
        })
        .collect()
}

/// Los equipos USB conectados AHORA (sin puertos ni discos: `listar` los une).
pub fn equipos() -> Result<Vec<Usb>> {
    use nusb::MaybeFuture;
    let it = nusb::list_devices().wait().map_err(|e| anyhow!("USB: {e}"))?;
    Ok(it
        .map(|d| Usb {
            vid: d.vendor_id(),
            pid: d.product_id(),
            fabricante: d.manufacturer_string().unwrap_or("").trim().to_string(),
            producto: d.product_string().unwrap_or("").trim().to_string(),
            serie: huella_serie(d.serial_number().unwrap_or("")),
            clase: d.class(),
            ubicacion: format!("{:#010x}", d.location_id()),
            ..Default::default()
        })
        .collect())
}

/// Los equipos USB con su puerto serie y su disco.
pub fn listar() -> Result<Vec<Usb>> {
    let discos: Vec<(String, String)> = super::mac::medios().into_iter().filter(|m| m.entero && m.marca.starts_with("usb:")).map(|m| (m.id, m.marca)).collect();
    Ok(unir(equipos()?, &puertos_usb(), &discos))
}

pub fn linea(u: &Usb) -> String {
    let es = identificar(u.vid, u.pid).texto();
    let mut l = format!("{:04x}:{:04x} · {} {}", u.vid, u.pid, u.fabricante, u.producto);
    if !es.is_empty() {
        l.push_str(&format!(" — {es}"));
    }
    if !u.puertos.is_empty() {
        l.push_str(&crate::tr!(" · puerto {}", " · port {}", u.puertos.join(", ")));
    }
    if !u.discos.is_empty() {
        l.push_str(&crate::tr!(" · disco {}", " · disk {}", u.discos.join(", ")));
    }
    l
}

/// Un cambio entre dos fotos: lo que se conectó y lo que se desconectó.
#[derive(Debug, Default, PartialEq)]
pub struct Cambio {
    pub conectados: Vec<Usb>,
    pub desconectados: Vec<Usb>,
}

impl Cambio {
    pub fn vacio(&self) -> bool {
        self.conectados.is_empty() && self.desconectados.is_empty()
    }

    /// El aviso de una línea por equipo (para F3 y la notificación de F2.1).
    pub fn aviso(&self) -> String {
        let mut v = Vec::new();
        for u in &self.conectados {
            v.push(crate::tr!("🔌 conectado: {}", "🔌 connected: {}", linea(u)));
        }
        for u in &self.desconectados {
            v.push(crate::tr!("⏏ desconectado: {:04x}:{:04x} {}", "⏏ disconnected: {:04x}:{:04x} {}", u.vid, u.pid, u.producto));
        }
        v.join("\n")
    }
}

fn clave(u: &Usb) -> (u16, u16, &str, &str) {
    (u.vid, u.pid, u.serie.as_str(), u.ubicacion.as_str())
}

pub fn diferencias(antes: &[Usb], ahora: &[Usb]) -> Cambio {
    Cambio {
        conectados: ahora.iter().filter(|u| !antes.iter().any(|a| clave(a) == clave(u))).cloned().collect(),
        desconectados: antes.iter().filter(|a| !ahora.iter().any(|u| clave(a) == clave(u))).cloned().collect(),
    }
}

/// Espera hasta que algo se conecte o se desconecte (o el tope). `foto` da la lista cada vez.
pub async fn esperar_cambio<F: FnMut() -> Result<Vec<Usb>>>(mut foto: F, tope: Duration, paso: Duration) -> Result<Cambio> {
    let antes = foto()?;
    let t0 = Instant::now();
    loop {
        tokio::time::sleep(paso).await;
        let ahora = foto()?;
        let c = diferencias(&antes, &ahora);
        if !c.vacio() || t0.elapsed() >= tope {
            return Ok(c);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn equipo(vid: u16, pid: u16, serie: &str, ubic: &str) -> Usb {
        Usb { vid, pid, producto: "x".into(), serie: huella_serie(serie), ubicacion: ubic.into(), ..Default::default() }
    }

    #[test]
    fn la_tabla_usb_con_identificadores_conocidos() {
        assert_eq!(identificar(0x303a, 0x1001), Que::Espressif);
        assert_eq!(identificar(0x10c4, 0xea60), Que::PuenteSerie("CP210x"));
        assert_eq!(identificar(0x1a86, 0x7523), Que::PuenteSerie("CH340"));
        assert_eq!(identificar(0x1a86, 0x55d4), Que::PuenteSerie("CH9102"));
        assert_eq!(identificar(0x0403, 0x6001), Que::PuenteSerie("FTDI FT232R"));
        assert_eq!(identificar(0x2e8a, 0x0003), Que::PicoArranque("RP2040"));
        assert_eq!(identificar(0x2e8a, 0x000f), Que::PicoArranque("RP2350"));
        assert_eq!(identificar(0x2e8a, 0x0005), Que::PicoMicroPython);
        assert_eq!(identificar(0x2e8a, 0x000a), Que::PicoSdk);
        assert_eq!(identificar(0x0a5c, 0x2711), Que::RaspberryArranque);
        assert_eq!(identificar(0x0a5c, 0x2100), Que::Otro, "otro equipo de Broadcom no es una Raspberry");
        assert_eq!(identificar(0x2341, 0x0043), Que::Arduino);
        assert_eq!(identificar(0x239a, 0x80f4), Que::Adafruit);
        assert_eq!(identificar(0x0483, 0x374b), Que::StLink);
        assert_eq!(identificar(0x05ac, 0x8104), Que::Otro);
        assert!(Que::PuenteSerie("CH340").quiza_esp() && !Que::Arduino.quiza_esp());
    }

    /// Cada equipo con SU puerto y SU disco, también con dos placas iguales (las distingue la serie).
    #[test]
    fn cada_equipo_con_su_puerto_y_su_disco_sin_la_serie() {
        let v = vec![equipo(0x1a86, 0x7523, "SECRETO-A", "0x01100000"), equipo(0x1a86, 0x7523, "SECRETO-B", "0x01200000"), equipo(0x0781, 0x5581, "SECRETO-M", "0x01300000")];
        let puertos = vec![("/dev/cu.usbserial-120".to_string(), 0x1a86, 0x7523, huella_serie("SECRETO-B")), ("/dev/cu.usbserial-110".to_string(), 0x1a86, 0x7523, huella_serie("SECRETO-A"))];
        let discos = vec![("disk7".to_string(), format!("usb:{}", huella_serie("SECRETO-M")))];
        let u = unir(v, &puertos, &discos);
        assert_eq!(u[0].puertos, vec!["/dev/cu.usbserial-110"]);
        assert_eq!(u[1].puertos, vec!["/dev/cu.usbserial-120"]);
        assert_eq!(u[2].discos, vec!["disk7"]);
        assert!(!format!("{u:?}").contains("SECRETO"), "el número de serie no sale");
        assert!(linea(&u[0]).contains("CH340") && linea(&u[0]).contains("/dev/cu.usbserial-110"));
    }

    #[tokio::test]
    async fn el_aviso_de_conexion_y_desconexion() {
        let todos = vec![equipo(0x1a86, 0x7523, "a", "0x01"), equipo(0x05ac, 0x1, "b", "0x02")];
        let sin_placa: Vec<Usb> = todos.iter().filter(|u| u.vid != 0x1a86).cloned().collect();
        let mut n = 0;
        let c = esperar_cambio(
            || {
                n += 1;
                Ok(if n < 3 { sin_placa.clone() } else { todos.clone() })
            },
            Duration::from_secs(5),
            Duration::from_millis(5),
        )
        .await
        .unwrap();
        assert_eq!(c.conectados.len(), 1);
        assert!(c.aviso().contains("CH340"), "{}", c.aviso());
        let d = diferencias(&todos, &sin_placa);
        assert_eq!(d.desconectados.len(), 1);
        let s = esperar_cambio(|| Ok(todos.clone()), Duration::from_millis(20), Duration::from_millis(5)).await.unwrap();
        assert!(s.vacio());
    }

    /// Esta Mac, de verdad (sólo lee), con nusb.
    #[test]
    fn listar_en_esta_mac_no_falla() {
        let v = listar().unwrap();
        assert!(v.iter().all(|u| u.serie.is_empty() || u.serie.len() == 8));
    }
}
