//! F22 paso 6 · Raspberry Pi Pico: en modo de arranque (BOOTSEL) aparece como una memoria con
//! `INFO_UF2.TXT` (ahí dice el modelo) y grabar es copiar el `.uf2`.
//!
//! Verificado en el código de las ROM oficiales (pico-bootrom-rp2040 y -rp2350): la RP2040 se llama
//! `RPI-RP2` y su `INFO_UF2.TXT` dice «Board-ID: RPI-RP2»; la RP2350 se llama `RP2350` por omisión, pero el
//! nombre, el modelo y el Board-ID se pueden cambiar en su OTP: se busca el archivo, no el nombre.
//! Antes de copiar se comprueba que el `.uf2` es un UF2 (sus números mágicos) y para qué familia es.
//! `probe-rs` (sondas de depuración) queda fuera: sin sonda no hay nada que medir.
use anyhow::{anyhow, bail, Result};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct PicoArranque {
    pub volumen: PathBuf,
    pub modelo: String,
    pub board_id: String,
    pub version: String,
}

/// Lee `INFO_UF2.TXT` («UF2 Bootloader v3.0 / Model: … / Board-ID: …»).
pub fn leer_info(t: &str, volumen: &Path) -> PicoArranque {
    let campo = |k: &str| t.lines().find_map(|l| l.strip_prefix(k).map(|v| v.trim().to_string())).unwrap_or_default();
    PicoArranque { volumen: volumen.to_path_buf(), modelo: campo("Model:"), board_id: campo("Board-ID:"), version: campo("UF2 Bootloader").trim_start_matches('v').to_string() }
}

/// Las placas en modo de arranque montadas bajo `base` (`/Volumes`).
pub fn buscar_en(base: &Path) -> Vec<PicoArranque> {
    let mut v: Vec<PicoArranque> = std::fs::read_dir(base)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let p = e.path();
            let t = std::fs::read_to_string(p.join("INFO_UF2.TXT")).ok()?;
            Some(leer_info(&t, &p))
        })
        .collect();
    v.sort_by(|a, b| a.volumen.cmp(&b.volumen));
    v
}

pub fn buscar() -> Vec<PicoArranque> {
    buscar_en(Path::new("/Volumes"))
}

impl PicoArranque {
    /// El chip, por lo que dice la placa.
    pub fn chip(&self) -> &'static str {
        let t = format!("{} {}", self.modelo, self.board_id).to_ascii_uppercase();
        if t.contains("RP2350") {
            "RP2350"
        } else if t.contains("RPI-RP2") || t.contains("RP2040") || t.contains("RASPBERRY PI RP2") {
            "RP2040"
        } else {
            ""
        }
    }

    pub fn linea(&self) -> String {
        crate::tr!(
            "{} en modo de arranque · {} · {} ({}) · {}",
            "{} in boot mode · {} · {} ({}) · {}",
            if self.chip().is_empty() { "UF2" } else { self.chip() },
            self.volumen.display(),
            self.modelo,
            self.board_id,
            crate::i18n::t("grabar = copiar un .uf2 (action=grabar)", "flashing = copying a .uf2 (action=grabar)")
        )
    }
}

/// Familias UF2 (identificadores de la especificación UF2 y del SDK de la Pico).
pub fn familia(id: u32) -> &'static str {
    match id {
        0xe48bff56 => "RP2040",
        0xe48bff57 => "ABSOLUTE",
        0xe48bff58 => "DATA",
        0xe48bff59 => "RP2350-ARM-S",
        0xe48bff5a => "RP2350-RISCV",
        0xe48bff5b => "RP2350-ARM-NS",
        _ => "",
    }
}

/// Comprueba que el archivo es un UF2 entero (todos sus bloques con sus números mágicos) y da su familia.
pub fn comprobar_uf2(datos: &[u8]) -> Result<(usize, u32)> {
    if datos.is_empty() || datos.len() % 512 != 0 {
        bail!("{}", crate::i18n::t("no es un .uf2: su tamaño no es múltiplo de 512", "not a .uf2: its size is not a multiple of 512"));
    }
    let le = |b: &[u8], o: usize| u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]);
    let mut fam = 0;
    for (i, b) in datos.chunks(512).enumerate() {
        if le(b, 0) != 0x0A324655 || le(b, 4) != 0x9E5D5157 || le(b, 508) != 0x0AB16F30 {
            bail!("{}", crate::tr!("no es un .uf2 válido (el bloque {i} no lleva sus números mágicos)", "not a valid .uf2 (block {i} lacks its magic numbers)"));
        }
        if le(b, 8) & 0x2000 != 0 && fam == 0 {
            fam = le(b, 28);
        }
    }
    Ok((datos.len() / 512, fam))
}

/// Graba el `.uf2`: lo copia a la placa y espera a que se reinicie (el volumen desaparece).
pub fn grabar(placa: &PicoArranque, uf2: &Path, espera: Duration) -> Result<String> {
    let datos = std::fs::read(uf2).map_err(|e| anyhow!("{}: {e}", uf2.display()))?;
    let (bloques, fam) = comprobar_uf2(&datos)?;
    let f = familia(fam);
    let chip = placa.chip();
    if !chip.is_empty() && !f.is_empty() && !f.starts_with(chip) && !matches!(f, "ABSOLUTE" | "DATA") {
        bail!("{}", crate::tr!("el .uf2 es para {f} y la placa es {chip}: no se graba (no arrancaría)", "the .uf2 is for {f} and the board is {chip}: not flashed (it would not boot)"));
    }
    let nombre = uf2.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "firmware.uf2".into());
    let destino = placa.volumen.join(&nombre);
    // la placa se reinicia en cuanto recibe el último bloque: el cierre puede fallar aunque todo llegó
    let r = std::fs::write(&destino, &datos);
    let t0 = Instant::now();
    while placa.volumen.join("INFO_UF2.TXT").exists() && t0.elapsed() < espera {
        std::thread::sleep(Duration::from_millis(200));
    }
    let reinicio = !placa.volumen.join("INFO_UF2.TXT").exists();
    if let Err(e) = r {
        if !reinicio {
            bail!("{}", crate::tr!("no se pudo copiar a la placa: {e}", "could not copy to the board: {e}"));
        }
    }
    Ok(if reinicio {
        crate::tr!("✓ grabado ({bloques} bloques, {}): la placa se reinició con el programa nuevo", "✓ flashed ({bloques} blocks, {}): the board restarted with the new program", if f.is_empty() { "UF2" } else { f })
    } else {
        crate::tr!(
            "copiado ({bloques} bloques) pero la placa sigue en modo de arranque tras {} s: ¿el .uf2 es para esta placa?",
            "copied ({bloques} blocks) but the board is still in boot mode after {} s: is the .uf2 for this board?",
            espera.as_secs()
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uf2(bloques: u32, fam: u32) -> Vec<u8> {
        let mut v = Vec::new();
        for i in 0..bloques {
            let mut b = vec![0u8; 512];
            b[0..4].copy_from_slice(&0x0A324655u32.to_le_bytes());
            b[4..8].copy_from_slice(&0x9E5D5157u32.to_le_bytes());
            b[8..12].copy_from_slice(&0x2000u32.to_le_bytes());
            b[12..16].copy_from_slice(&(0x10000000 + i * 256).to_le_bytes());
            b[16..20].copy_from_slice(&256u32.to_le_bytes());
            b[20..24].copy_from_slice(&i.to_le_bytes());
            b[24..28].copy_from_slice(&bloques.to_le_bytes());
            b[28..32].copy_from_slice(&fam.to_le_bytes());
            b[508..512].copy_from_slice(&0x0AB16F30u32.to_le_bytes());
            v.extend(b);
        }
        v
    }

    /// Los textos exactos de las ROM oficiales.
    #[test]
    fn identifica_la_pico_por_info_uf2() {
        let d = crate::CarpetaPrueba::nueva("gotv-pico");
        std::fs::create_dir_all(d.join("RPI-RP2")).unwrap();
        std::fs::write(d.join("RPI-RP2/INFO_UF2.TXT"), "UF2 Bootloader v3.0\nModel: Raspberry Pi RP2\nBoard-ID: RPI-RP2\n").unwrap();
        std::fs::create_dir_all(d.join("RP2350")).unwrap();
        std::fs::write(d.join("RP2350/INFO_UF2.TXT"), "UF2 Bootloader v1.1\nModel: Raspberry Pi RP2350\nBoard-ID: RP2350\n").unwrap();
        std::fs::create_dir_all(d.join("MEMORIA")).unwrap();
        let v = buscar_en(&d);
        assert_eq!(v.len(), 2, "la memoria cualquiera no es una Pico");
        let rp2040 = v.iter().find(|p| p.volumen.ends_with("RPI-RP2")).unwrap();
        assert_eq!(rp2040.chip(), "RP2040");
        assert_eq!(rp2040.version, "3.0");
        let rp2350 = v.iter().find(|p| p.volumen.ends_with("RP2350")).unwrap();
        assert_eq!(rp2350.chip(), "RP2350");
        assert!(rp2350.linea().contains("RP2350"));
    }

    #[test]
    fn grabar_es_copiar_el_uf2_y_esperar_el_reinicio() {
        let d = crate::CarpetaPrueba::nueva("gotv-pico-grabar");
        let vol = d.join("RPI-RP2");
        std::fs::create_dir_all(&vol).unwrap();
        std::fs::write(vol.join("INFO_UF2.TXT"), "UF2 Bootloader v3.0\nModel: Raspberry Pi RP2\nBoard-ID: RPI-RP2\n").unwrap();
        let placa = buscar_en(&d).remove(0);
        std::fs::write(d.join("blink.uf2"), uf2(4, 0xe48bff56)).unwrap();
        // la «placa» se reinicia al recibir el archivo: el volumen pierde su INFO_UF2.TXT
        let v2 = vol.clone();
        let h = std::thread::spawn(move || {
            let t0 = Instant::now();
            while !v2.join("blink.uf2").exists() && t0.elapsed() < Duration::from_secs(3) {
                std::thread::sleep(Duration::from_millis(20));
            }
            std::fs::remove_file(v2.join("INFO_UF2.TXT")).unwrap();
        });
        let r = grabar(&placa, &d.join("blink.uf2"), Duration::from_secs(3)).unwrap();
        h.join().unwrap();
        assert!(r.contains("✓") && r.contains("RP2040"), "{r}");
        assert_eq!(std::fs::read(vol.join("blink.uf2")).unwrap(), uf2(4, 0xe48bff56));
        // un .uf2 de RP2350 en una RP2040: no se graba
        std::fs::write(vol.join("INFO_UF2.TXT"), "UF2 Bootloader v3.0\nModel: Raspberry Pi RP2\nBoard-ID: RPI-RP2\n").unwrap();
        std::fs::write(d.join("otra.uf2"), uf2(2, 0xe48bff59)).unwrap();
        assert!(grabar(&placa, &d.join("otra.uf2"), Duration::from_millis(100)).unwrap_err().to_string().contains("RP2350"));
        // un archivo que no es UF2
        std::fs::write(d.join("falso.uf2"), vec![0u8; 1024]).unwrap();
        assert!(grabar(&placa, &d.join("falso.uf2"), Duration::from_millis(100)).is_err());
        assert!(!vol.join("falso.uf2").exists(), "nada se copia si no es un UF2");
    }
}
