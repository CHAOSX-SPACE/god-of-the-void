//! F22 pasos 7 y 8 · Raspberry Pi de cualquier versión, y que diga cuál es.
//!
//! - **Encendida y en la red** (por las conexiones SSH de `conexiones.rs`, con su huella fijada): el modelo
//!   exacto de `/proc/device-tree/model`; el código de revisión de `/proc/cpuinfo` se descifra AQUÍ, en Rust,
//!   con la tabla oficial (raspberrypi.com/documentation/computers/raspberry-pi.html, «revision codes»,
//!   verificada el 2026-11-16: formato nuevo `NOQuuuWuFMMMCCCCPPPPTTTTTTTTRRRR` y la tabla vieja de 0002 a
//!   0015); el sistema de `/etc/os-release`; temperatura y voltaje con `vcgencmd`. Configurar va con las
//!   órdenes no interactivas de `raspi-config` (siempre con permiso, valores validados aquí).
//! - **Una tarjeta ya grabada:** se lee su partición de arranque (`config.txt`, `cmdline.txt`, `issue.txt`).
//! - **Preparar una tarjeta:** catálogo oficial de Raspberry Pi Imager (`os_list_imagingutility_v4.json`) →
//!   descarga → se descomprime en Rust → se graba y se VERIFICA contra su `extract_sha256` → primera
//!   configuración. El mecanismo de la primera configuración cambia entre versiones (verificado en la
//!   documentación de rpi-imager, `os_customisation_formats.md`): las imágenes actuales (Trixie) declaran
//!   `init_format: cloudinit-rpi` y llevan `user-data`, `meta-data` y `network-config` en la partición de
//!   arranque, con ` ds=nocloud;i=<id>` añadido a `cmdline.txt`, como hace Imager 2.x. Las de Bookworm
//!   (`systemd`, un `firstrun.sh`) quedan FUERA: GOTV lo dice en vez de escribir un guion a medias.
//!   Sólo con llave SSH (sin contraseña): GOTV no maneja contraseñas de nadie.
use anyhow::{anyhow, bail, Result};
use serde_json::Value;
use std::path::{Path, PathBuf};

/// Lo que dice un código de revisión.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Revision {
    pub modelo: String,
    pub revision: String,
    pub memoria: String,
    pub fabricante: String,
    pub procesador: String,
}

fn tipo(t: u32) -> Option<&'static str> {
    Some(match t {
        0x00 => "A",
        0x01 => "B",
        0x02 => "A+",
        0x03 => "B+",
        0x04 => "2B",
        0x05 => "Alpha",
        0x06 => "CM1",
        0x08 => "3B",
        0x09 => "Zero",
        0x0a => "CM3",
        0x0c => "Zero W",
        0x0d => "3B+",
        0x0e => "3A+",
        0x10 => "CM3+",
        0x11 => "4B",
        0x12 => "Zero 2 W",
        0x13 => "400",
        0x14 => "CM4",
        0x15 => "CM4S",
        0x17 => "5",
        0x18 => "CM5",
        0x19 => "500/500+",
        0x1a => "CM5 Lite",
        0x1b => "CM0",
        _ => return None,
    })
}

/// Descifra el código de `/proc/cpuinfo` («Revision : c03111»).
pub fn descifrar(codigo: u32) -> Option<Revision> {
    if codigo & (1 << 23) != 0 {
        let memoria = match (codigo >> 20) & 7 {
            0 => "256 MB",
            1 => "512 MB",
            2 => "1 GB",
            3 => "2 GB",
            4 => "4 GB",
            5 => "8 GB",
            6 => "16 GB",
            _ => "?",
        };
        let fabricante = match (codigo >> 16) & 0xF {
            0 => "Sony UK",
            1 => "Egoman",
            2 | 4 => "Embest",
            3 => "Sony Japan",
            5 => "Stadium",
            _ => "?",
        };
        let procesador = match (codigo >> 12) & 0xF {
            0 => "BCM2835",
            1 => "BCM2836",
            2 => "BCM2837",
            3 => "BCM2711",
            4 => "BCM2712",
            _ => "?",
        };
        let t = tipo((codigo >> 4) & 0xFF)?;
        return Some(Revision {
            modelo: format!("Raspberry Pi {t}"),
            revision: format!("1.{}", codigo & 0xF),
            memoria: memoria.into(),
            fabricante: fabricante.into(),
            procesador: procesador.into(),
        });
    }
    // formato viejo (el bit 24 es «garantía anulada» por sobretensión: no cambia el modelo)
    let (m, r, mem, fab) = match codigo & 0xFFFF {
        0x0002 | 0x0003 => ("B", "1.0", "256 MB", "Egoman"),
        0x0004 => ("B", "2.0", "256 MB", "Sony UK"),
        0x0005 => ("B", "2.0", "256 MB", "Qisda"),
        0x0006 => ("B", "2.0", "256 MB", "Egoman"),
        0x0007 => ("A", "2.0", "256 MB", "Egoman"),
        0x0008 => ("A", "2.0", "256 MB", "Sony UK"),
        0x0009 => ("A", "2.0", "256 MB", "Qisda"),
        0x000d | 0x000f => ("B", "2.0", "512 MB", "Egoman"),
        0x000e => ("B", "2.0", "512 MB", "Sony UK"),
        0x0010 => ("B+", "1.2", "512 MB", "Sony UK"),
        0x0011 => ("CM1", "1.0", "512 MB", "Sony UK"),
        0x0012 => ("A+", "1.1", "256 MB", "Sony UK"),
        0x0013 => ("B+", "1.2", "512 MB", "Embest"),
        0x0014 => ("CM1", "1.0", "512 MB", "Embest"),
        0x0015 => ("A+", "1.1", "256/512 MB", "Embest"),
        _ => return None,
    };
    Some(Revision { modelo: format!("Raspberry Pi {m}"), revision: r.into(), memoria: mem.into(), fabricante: fab.into(), procesador: "BCM2835".into() })
}

/// Lo que sale de la orden de `info` (tramos separados por «---»).
#[derive(Debug, Default, PartialEq, serde::Serialize)]
pub struct Info {
    pub modelo: String,
    pub revision: Option<Revision>,
    pub codigo: String,
    pub sistema: String,
    pub nombre: String,
    pub nucleo: String,
    pub memoria_total: String,
    pub temperatura: String,
    pub voltaje: String,
    pub limitado: String,
}

/// Una sola orden remota (una conexión SSH) que junta todo. No lee el número de serie.
pub const ORDEN_INFO: &str = "tr -d '\\0' < /proc/device-tree/model 2>/dev/null; echo; echo ---; grep -m1 '^Revision' /proc/cpuinfo; echo ---; grep -E '^(PRETTY_NAME|VERSION_CODENAME)=' /etc/os-release; echo ---; hostname; uname -r; grep -m1 MemTotal /proc/meminfo; echo ---; vcgencmd measure_temp 2>/dev/null; vcgencmd measure_volts core 2>/dev/null; vcgencmd get_throttled 2>/dev/null";

pub fn leer_info(t: &str) -> Info {
    let partes: Vec<&str> = t.split("---").map(str::trim).collect();
    let p = |i: usize| partes.get(i).copied().unwrap_or("");
    let codigo = p(1).split(':').nth(1).unwrap_or("").trim().to_string();
    let mut i = Info {
        modelo: p(0).lines().next().unwrap_or("").trim().to_string(),
        revision: u32::from_str_radix(&codigo, 16).ok().and_then(descifrar),
        codigo,
        ..Default::default()
    };
    for l in p(2).lines() {
        if let Some(v) = l.strip_prefix("PRETTY_NAME=") {
            i.sistema = v.trim_matches('"').to_string();
        }
    }
    let mut lineas = p(3).lines();
    i.nombre = lineas.next().unwrap_or("").trim().to_string();
    i.nucleo = lineas.next().unwrap_or("").trim().to_string();
    if let Some(kb) = lineas.next().and_then(|l| l.split_whitespace().nth(1)).and_then(|k| k.parse::<u64>().ok()) {
        i.memoria_total = format!("{:.1} GB", kb as f64 / 1_048_576.0);
    }
    for l in p(4).lines() {
        if let Some(v) = l.strip_prefix("temp=") {
            i.temperatura = v.replace("'C", " °C");
        } else if let Some(v) = l.strip_prefix("volt=") {
            i.voltaje = v.to_string();
        } else if let Some(v) = l.strip_prefix("throttled=") {
            i.limitado = v.to_string();
        }
    }
    i
}

impl Info {
    pub fn texto(&self) -> String {
        let mut v = vec![if self.modelo.is_empty() { crate::i18n::t("(no dice su modelo: ¿es una Raspberry Pi?)", "(it does not report a model: is it a Raspberry Pi?)").to_string() } else { self.modelo.clone() }];
        if let Some(r) = &self.revision {
            v.push(crate::tr!(
                "{} rev {} · {} de memoria · {} · fabricada por {} (código {})",
                "{} rev {} · {} of memory · {} · made by {} (code {})",
                r.modelo, r.revision, r.memoria, r.procesador, r.fabricante, self.codigo
            ));
        }
        if !self.sistema.is_empty() {
            v.push(crate::tr!("sistema: {} · núcleo {}", "system: {} · kernel {}", self.sistema, self.nucleo));
        }
        if !self.nombre.is_empty() {
            v.push(crate::tr!("nombre: {} · memoria total {}", "hostname: {} · total memory {}", self.nombre, self.memoria_total));
        }
        if !self.temperatura.is_empty() {
            let lim = match self.limitado.as_str() {
                "0x0" => crate::i18n::t("sin limitaciones", "not throttled").to_string(),
                "" => String::new(),
                x => crate::tr!("⚠ limitada o con poca alimentación ({x}): revisa la fuente", "⚠ throttled or under-powered ({x}): check the power supply"),
            };
            v.push(format!("{} · {} · {lim}", self.temperatura, self.voltaje));
        }
        v.join("\n")
    }
}

fn valida(v: &str, extra: &str, max: usize) -> Result<String> {
    if v.is_empty() || v.len() > max || !v.chars().all(|c| c.is_ascii_alphanumeric() || extra.contains(c)) {
        bail!("{}", crate::tr!("valor «{v}» no válido aquí", "value «{v}» is not valid here"));
    }
    Ok(v.to_string())
}

/// La orden remota de `configurar` (validada aquí: nada de texto libre del modelo en la terminal remota).
pub fn orden_configurar(args: &Value) -> Result<(String, String)> {
    let s = |k: &str| args.get(k).and_then(|v| v.as_str()).map(str::trim).filter(|v| !v.is_empty());
    let rc = "sudo -n raspi-config nonint";
    let si_no = |k: &str| args.get(k).and_then(|v| v.as_bool()).map(|b| if b { "0" } else { "1" }); // raspi-config: 0 = encender
    let mut ordenes = Vec::new();
    let mut que = Vec::new();
    if let Some(n) = s("nombre") {
        let n = valida(&n.to_ascii_lowercase(), "-", 63)?;
        ordenes.push(format!("{rc} do_hostname {n}"));
        que.push(crate::tr!("nombre → {n}", "hostname → {n}"));
    }
    for (k, f) in [("ssh", "do_ssh"), ("i2c", "do_i2c"), ("spi", "do_spi"), ("vnc", "do_vnc")] {
        if let Some(v) = si_no(k) {
            ordenes.push(format!("{rc} {f} {v}"));
            que.push(format!("{k} {}", if v == "0" { "on" } else { "off" }));
        }
    }
    if let Some(z) = s("zona") {
        let z = valida(z, "/_+-", 64)?;
        ordenes.push(format!("{rc} do_change_timezone {z}"));
        que.push(crate::tr!("zona horaria → {z}", "timezone → {z}"));
    }
    if let Some(p) = s("wifi_pais") {
        let p = valida(&p.to_ascii_uppercase(), "", 2)?;
        ordenes.push(format!("{rc} do_wifi_country {p}"));
        que.push(crate::tr!("país del Wi-Fi → {p}", "Wi-Fi country → {p}"));
    }
    if let Some(l) = s("config_txt") {
        let l = valida(l, "_=,.:-", 120)?;
        ordenes.push(format!("C=/boot/firmware/config.txt; [ -f $C ] || C=/boot/config.txt; sudo -n cp $C $C.gotv-$(date +%s) && echo {l} | sudo -n tee -a $C >/dev/null"));
        que.push(crate::tr!("config.txt += {l} (con copia de respaldo; se aplica al reiniciar)", "config.txt += {l} (with a backup copy; applies on reboot)"));
    }
    if ordenes.is_empty() {
        bail!("{}", crate::i18n::t("nada que configurar: nombre, ssh, i2c, spi, vnc, zona, wifi_pais o config_txt", "nothing to configure: nombre, ssh, i2c, spi, vnc, zona, wifi_pais or config_txt"));
    }
    Ok((ordenes.join(" && "), que.join(", ")))
}

/// Lee la partición de arranque de una tarjeta ya grabada.
pub fn leer_tarjeta(boot: &Path) -> Result<String> {
    if !boot.join("config.txt").is_file() && !boot.join("cmdline.txt").is_file() {
        bail!("{}", crate::tr!("{} no parece la partición de arranque de una Raspberry Pi (sin config.txt ni cmdline.txt)", "{} does not look like a Raspberry Pi boot partition (no config.txt nor cmdline.txt)", boot.display()));
    }
    let leer = |n: &str| std::fs::read_to_string(boot.join(n)).unwrap_or_default();
    let issue = leer("issue.txt");
    let mut v = vec![match issue.lines().next() {
        Some(l) if !l.trim().is_empty() => crate::tr!("sistema: {}", "system: {}", l.trim()),
        _ => crate::i18n::t("sistema: (sin issue.txt: no es Raspberry Pi OS o es muy viejo)", "system: (no issue.txt: not Raspberry Pi OS or very old)").to_string(),
    }];
    let cfg: Vec<String> = leer("config.txt").lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')).map(String::from).collect();
    v.push(format!("config.txt: {}", cfg.join(" · ")));
    v.push(format!("cmdline.txt: {}", leer("cmdline.txt").trim()));
    let primer: Vec<&str> = ["user-data", "network-config", "firstrun.sh", "userconf.txt", "ssh", "rpi-preseed.toml"].into_iter().filter(|n| boot.join(n).exists()).collect();
    v.push(if primer.is_empty() {
        crate::i18n::t("primera configuración: ninguna (arrancará pidiendo usuario en pantalla)", "first-boot setup: none (it will ask for a user on screen)").to_string()
    } else {
        crate::tr!("primera configuración: {}", "first-boot setup: {}", primer.join(", "))
    });
    Ok(v.join("\n"))
}

// ───────────── preparar una tarjeta ─────────────

pub const CATALOGO: &str = "https://downloads.raspberrypi.com/os_list_imagingutility_v4.json";

/// Una imagen del catálogo oficial.
#[derive(Debug, Clone, PartialEq)]
pub struct Imagen {
    pub nombre: String,
    pub url: String,
    pub extract_size: u64,
    pub extract_sha256: String,
    pub descarga: u64,
    pub init_format: String,
    pub fecha: String,
}

/// Aplana el catálogo (con sus `subitems`) y deja las imágenes descargables.
pub fn imagenes(cat: &Value) -> Vec<Imagen> {
    fn caminar(v: &Value, out: &mut Vec<Imagen>) {
        for e in v.as_array().into_iter().flatten() {
            if let Some(url) = e["url"].as_str().filter(|u| u.starts_with("https://")) {
                out.push(Imagen {
                    nombre: e["name"].as_str().unwrap_or("").into(),
                    url: url.into(),
                    extract_size: e["extract_size"].as_u64().unwrap_or(0),
                    extract_sha256: e["extract_sha256"].as_str().unwrap_or("").to_ascii_lowercase(),
                    descarga: e["image_download_size"].as_u64().unwrap_or(0),
                    init_format: e["init_format"].as_str().unwrap_or("none").into(),
                    fecha: e["release_date"].as_str().unwrap_or("").into(),
                });
            }
            caminar(&e["subitems"], out);
        }
    }
    let mut out = Vec::new();
    caminar(&cat["os_list"], &mut out);
    out
}

/// La imagen pedida por su nombre (exacto primero, luego el que la contenga si es UNO solo).
pub fn elegir<'a>(v: &'a [Imagen], nombre: &str) -> Result<&'a Imagen> {
    let n = nombre.to_lowercase();
    if let Some(i) = v.iter().find(|i| i.nombre.to_lowercase() == n) {
        return Ok(i);
    }
    let c: Vec<&Imagen> = v.iter().filter(|i| i.nombre.to_lowercase().contains(&n)).collect();
    match c.len() {
        1 => Ok(c[0]),
        0 => bail!("{}", crate::tr!("no hay «{nombre}» en el catálogo oficial", "no «{nombre}» in the official catalogue")),
        _ => bail!("{}", crate::tr!("«{nombre}» casa con varias: {}", "«{nombre}» matches several: {}", c.iter().map(|i| i.nombre.as_str()).collect::<Vec<_>>().join(" · "))),
    }
}

/// Descarga a `destino` (si ya está entero, no se vuelve a bajar).
pub async fn descargar(url: &str, destino: &Path, tam: u64, avance: &mut (dyn FnMut(u64) + Send)) -> Result<PathBuf> {
    use futures_util::StreamExt;
    use std::io::Write;
    if tam > 0 && std::fs::metadata(destino).map(|m| m.len()).ok() == Some(tam) {
        return Ok(destino.to_path_buf());
    }
    if let Some(p) = destino.parent() {
        std::fs::create_dir_all(p)?;
    }
    let parcial = destino.with_extension("parcial");
    let r = reqwest::Client::builder().build()?.get(url).send().await?.error_for_status()?;
    let mut f = std::fs::File::create(&parcial)?;
    let mut n = 0u64;
    let mut ultimo = 0u64;
    let mut flujo = r.bytes_stream();
    while let Some(trozo) = flujo.next().await {
        let t = trozo?;
        f.write_all(&t)?;
        n += t.len() as u64;
        if n - ultimo >= 64 << 20 {
            ultimo = n;
            avance(n);
        }
    }
    f.sync_all()?;
    if tam > 0 && n != tam {
        let _ = std::fs::remove_file(&parcial);
        bail!("{}", crate::tr!("la descarga quedó incompleta ({n} de {tam} bytes)", "the download is incomplete ({n} of {tam} bytes)"));
    }
    std::fs::rename(&parcial, destino)?;
    Ok(destino.to_path_buf())
}

/// La primera configuración que se deja en la tarjeta.
#[derive(Debug, Default, Clone)]
pub struct PrimerArranque {
    pub nombre: String,
    pub usuario: String,
    /// la llave PÚBLICA (una línea `ssh-ed25519 …`)
    pub llave: String,
    pub wifi_ssid: Option<String>,
    /// la clave del Wi-Fi la teclea el usuario en un cuadro tapado (no pasa por el modelo)
    pub wifi_clave: Option<String>,
    pub pais: String,
    pub zona: String,
    pub teclado: String,
}

fn yaml(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

impl PrimerArranque {
    pub fn validar(&self) -> Result<()> {
        valida(&self.nombre, "-", 63)?;
        valida(&self.usuario, "_-", 32)?;
        if !self.usuario.chars().next().is_some_and(|c| c.is_ascii_lowercase()) {
            bail!("{}", crate::i18n::t("el usuario empieza por una letra minúscula", "the user name starts with a lowercase letter"));
        }
        let l = self.llave.trim();
        if !(l.starts_with("ssh-ed25519 ") || l.starts_with("ssh-rsa ") || l.starts_with("ecdsa-sha2-")) || l.contains('\n') || l.contains("PRIVATE") {
            bail!("{}", crate::i18n::t("hace falta una llave PÚBLICA SSH (una línea «ssh-ed25519 …», el .pub): GOTV no pone contraseñas", "a PUBLIC SSH key is needed (one «ssh-ed25519 …» line, the .pub): GOTV does not set passwords"));
        }
        Ok(())
    }

    /// `user-data` de cloud-init en la forma que escribe Raspberry Pi Imager 2.x (`user:` en singular: con
    /// `users:` sin `- default` el usuario quedaría sin grupos, rpi-imager #1601).
    pub fn user_data(&self) -> String {
        let mut t = String::from("#cloud-config\n");
        t.push_str(&format!("hostname: {}\nmanage_etc_hosts: true\n", yaml(&self.nombre)));
        if !self.zona.is_empty() {
            t.push_str(&format!("timezone: {}\n", yaml(&self.zona)));
        }
        if !self.teclado.is_empty() {
            t.push_str(&format!("keyboard:\n  model: pc105\n  layout: {}\n", yaml(&self.teclado)));
        }
        t.push_str(&format!("user:\n  name: {}\n  shell: /bin/bash\n  lock_passwd: true\n  ssh_authorized_keys:\n    - {}\n  sudo: ALL=(ALL) NOPASSWD:ALL\n", yaml(&self.usuario), yaml(self.llave.trim())));
        t.push_str("ssh_pwauth: false\nruncmd:\n  - [ systemctl, enable, --now, ssh ]\n");
        t
    }

    pub fn network_config(&self) -> Option<String> {
        let ssid = self.wifi_ssid.as_ref()?;
        let ap = match &self.wifi_clave {
            Some(c) => format!("          {}:\n            password: {}\n", yaml(ssid), yaml(c)),
            None => format!("          {}:\n            auth:\n              key-management: none\n", yaml(ssid)),
        };
        Some(format!(
            "network:\n  version: 2\n  ethernets:\n    eth0:\n      dhcp4: true\n      optional: true\n  wifis:\n    wlan0:\n      dhcp4: true\n      regulatory-domain: {}\n      access-points:\n{ap}      optional: true\n",
            yaml(&self.pais)
        ))
    }

    /// Escribe la primera configuración en la partición de arranque según el `init_format` de la imagen.
    pub fn escribir(&self, boot: &Path, init_format: &str, instancia: &str) -> Result<Vec<String>> {
        self.validar()?;
        if !matches!(init_format, "cloudinit-rpi" | "cloudinit") {
            bail!("{}", crate::tr!(
                "esta imagen usa «{init_format}» para la primera configuración (Bookworm o anterior: un firstrun.sh); GOTV sólo prepara las que usan cloud-init (Raspberry Pi OS actual, Trixie). Elige la imagen actual o usa Raspberry Pi Imager para esta",
                "this image uses «{init_format}» for first-boot setup (Bookworm or older: a firstrun.sh); GOTV only prepares the ones using cloud-init (current Raspberry Pi OS, Trixie). Pick the current image or use Raspberry Pi Imager for this one"
            ));
        }
        let cmd = boot.join("cmdline.txt");
        let linea = std::fs::read_to_string(&cmd).map_err(|_| anyhow!("{}", crate::tr!("{} no tiene cmdline.txt: ¿es la partición de arranque?", "{} has no cmdline.txt: is it the boot partition?", boot.display())))?;
        let mut hechos = Vec::new();
        std::fs::write(boot.join("user-data"), self.user_data())?;
        hechos.push("user-data".into());
        std::fs::write(boot.join("meta-data"), format!("instance-id: {instancia}\n"))?;
        hechos.push("meta-data".into());
        if let Some(n) = self.network_config() {
            std::fs::write(boot.join("network-config"), n)?;
            hechos.push("network-config".into());
        }
        let linea = linea.trim_end();
        if !linea.contains("ds=nocloud") {
            std::fs::write(&cmd, format!("{linea} ds=nocloud;i={instancia}\n"))?;
            hechos.push("cmdline.txt (ds=nocloud)".into());
        }
        Ok(hechos)
    }
}

/// La partición de arranque montada de un disco (la primera FAT con config.txt o cmdline.txt).
pub fn boot_montado(m: &super::discos::Mesa, id: &str) -> Option<PathBuf> {
    m.particiones(&super::discos::entero(id))
        .into_iter()
        .filter(|v| !v.montado.is_empty())
        .map(|v| PathBuf::from(v.montado))
        .find(|p| p.join("cmdline.txt").is_file() || p.join("config.txt").is_file())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// La tabla oficial: códigos reales de la documentación.
    #[test]
    fn el_codigo_de_revision_con_la_tabla_oficial() {
        let r = descifrar(0xc03111).unwrap();
        assert_eq!((r.modelo.as_str(), r.memoria.as_str(), r.procesador.as_str(), r.fabricante.as_str(), r.revision.as_str()), ("Raspberry Pi 4B", "4 GB", "BCM2711", "Sony UK", "1.1"));
        let r = descifrar(0xd04170).unwrap();
        assert_eq!((r.modelo.as_str(), r.memoria.as_str(), r.procesador.as_str()), ("Raspberry Pi 5", "8 GB", "BCM2712"));
        assert_eq!(descifrar(0xe04171).unwrap().memoria, "16 GB");
        assert_eq!(descifrar(0x902120).unwrap().modelo, "Raspberry Pi Zero 2 W");
        assert_eq!(descifrar(0xc03130).unwrap().modelo, "Raspberry Pi 400");
        assert_eq!(descifrar(0xd04190).unwrap().modelo, "Raspberry Pi 500/500+");
        assert_eq!(descifrar(0xb041a0).unwrap().modelo, "Raspberry Pi CM5 Lite");
        assert_eq!(descifrar(0xa02082).unwrap().modelo, "Raspberry Pi 3B");
        assert_eq!(descifrar(0xa020d3).unwrap().modelo, "Raspberry Pi 3B+");
        assert_eq!(descifrar(0xa22082).unwrap().fabricante, "Embest");
        // formato viejo, también con el bit de garantía
        let v = descifrar(0x000e).unwrap();
        assert_eq!((v.modelo.as_str(), v.memoria.as_str(), v.fabricante.as_str()), ("Raspberry Pi B", "512 MB", "Sony UK"));
        assert_eq!(descifrar(0x1000010).unwrap().modelo, "Raspberry Pi B+");
        assert_eq!(descifrar(0x0015).unwrap().memoria, "256/512 MB");
        assert!(descifrar(0x000a).is_none(), "códigos que no están en la tabla no se inventan");
        assert!(descifrar(0x8000f0).is_none(), "tipo 0x0f es de uso interno");
    }

    #[test]
    fn la_info_de_una_raspberry_por_ssh() {
        let salida = "Raspberry Pi 5 Model B Rev 1.0\n---\nRevision\t: d04170\n---\nPRETTY_NAME=\"Debian GNU/Linux 13 (trixie)\"\nVERSION_CODENAME=trixie\n---\nframbuesa\n6.12.47+rpt-rpi-2712\nMemTotal:        8245760 kB\n---\ntemp=48.3'C\nvolt=0.7200V\nthrottled=0x0\n";
        let i = leer_info(salida);
        assert_eq!(i.modelo, "Raspberry Pi 5 Model B Rev 1.0");
        assert_eq!(i.revision.as_ref().unwrap().memoria, "8 GB");
        assert_eq!(i.nombre, "frambuesa");
        assert_eq!(i.sistema, "Debian GNU/Linux 13 (trixie)");
        assert_eq!(i.temperatura, "48.3 °C");
        assert_eq!(i.memoria_total, "7.9 GB");
        assert!(i.texto().contains("8 GB") && i.texto().contains("BCM2712"));
        assert!(!ORDEN_INFO.contains("Serial"), "el número de serie no se lee");
    }

    #[test]
    fn configurar_valida_lo_que_manda_a_la_raspberry() {
        let (o, q) = orden_configurar(&json!({"nombre": "Taller-1", "ssh": true, "i2c": false})).unwrap();
        assert!(o.contains("do_hostname taller-1") && o.contains("do_ssh 0") && o.contains("do_i2c 1"), "{o}");
        assert!(q.contains("taller-1"));
        assert!(orden_configurar(&json!({"nombre": "a; rm -rf /"})).is_err());
        assert!(orden_configurar(&json!({"zona": "$(reboot)"})).is_err());
        assert!(orden_configurar(&json!({"config_txt": "dtparam=audio=on"})).unwrap().0.contains("tee -a $C"));
        assert!(orden_configurar(&json!({"config_txt": "x`id`"})).is_err());
        assert!(orden_configurar(&json!({})).is_err());
    }

    /// El catálogo con la entrada REAL de «Raspberry Pi OS Lite (64-bit)» del 2026-11-16.
    #[test]
    fn el_catalogo_oficial() {
        let cat = json!({"imager": {}, "os_list": [
            {"name": "Raspberry Pi OS (other)", "subitems": [
                {"name": "Raspberry Pi OS Lite (64-bit)", "url": "https://downloads.raspberrypi.com/raspios_lite_arm64/images/raspios_lite_arm64-2026-10-06/2026-10-06-raspios-trixie-arm64-lite.img.xz",
                 "extract_size": 3078619136u64, "extract_sha256": "b8a393dc1c3701450b5fffbfe74368a98fb6eab7415ce456cc0cead886ea8780", "image_download_size": 550466056u64,
                 "release_date": "2026-10-06", "init_format": "cloudinit-rpi", "devices": ["pi5-64bit", "pi4-64bit", "pi3-64bit"]},
                {"name": "Raspberry Pi OS (Legacy, 64-bit) Lite", "url": "https://downloads.raspberrypi.com/x.img.xz", "init_format": "systemd"}]},
            {"name": "Use custom", "url": ""}]});
        let v = imagenes(&cat);
        assert_eq!(v.len(), 2, "lo que no se descarga (custom) no cuenta");
        let i = elegir(&v, "raspberry pi os lite (64-bit)").unwrap();
        assert_eq!((i.extract_size, i.init_format.as_str(), i.descarga), (3078619136, "cloudinit-rpi", 550466056));
        assert!(elegir(&v, "Lite").is_err(), "ambiguo: se dice cuáles");
        assert!(elegir(&v, "Ubuntu").is_err());
    }

    fn cfg() -> PrimerArranque {
        PrimerArranque {
            nombre: "taller".into(),
            usuario: "zero".into(),
            llave: "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIExample gotv".into(),
            wifi_ssid: Some("Casa \"5G\"".into()),
            wifi_clave: Some("clave-de-prueba".into()),
            pais: "CO".into(),
            zona: "America/Bogota".into(),
            teclado: "es".into(),
        }
    }

    #[test]
    fn la_primera_configuracion_con_cloud_init() {
        let d = crate::CarpetaPrueba::nueva("gotv-rpi-boot");
        std::fs::write(d.join("cmdline.txt"), "console=serial0,115200 root=PARTUUID=abc rootwait\n").unwrap();
        std::fs::write(d.join("config.txt"), "# x\narm_64bit=1\ndtoverlay=vc4-kms-v3d\n").unwrap();
        std::fs::write(d.join("issue.txt"), "Raspberry Pi reference 2026-10-06\n").unwrap();
        let h = cfg().escribir(&d, "cloudinit-rpi", "gotv-1").unwrap();
        assert_eq!(h.len(), 4, "{h:?}");
        let ud = std::fs::read_to_string(d.join("user-data")).unwrap();
        assert!(ud.starts_with("#cloud-config\n") && ud.contains("user:\n  name: \"zero\"") && ud.contains("ssh-ed25519") && ud.contains("lock_passwd: true"), "{ud}");
        let nc = std::fs::read_to_string(d.join("network-config")).unwrap();
        assert!(nc.contains("\"Casa \\\"5G\\\"\":") && nc.contains("regulatory-domain: \"CO\""), "{nc}");
        assert_eq!(std::fs::read_to_string(d.join("cmdline.txt")).unwrap(), "console=serial0,115200 root=PARTUUID=abc rootwait ds=nocloud;i=gotv-1\n");
        // escribir dos veces no duplica la marca
        cfg().escribir(&d, "cloudinit-rpi", "gotv-1").unwrap();
        assert_eq!(std::fs::read_to_string(d.join("cmdline.txt")).unwrap().matches("ds=nocloud").count(), 1);
        let t = leer_tarjeta(&d).unwrap();
        assert!(t.contains("2026-10-06") && t.contains("dtoverlay=vc4-kms-v3d") && t.contains("user-data"), "{t}");
        // una imagen de Bookworm (firstrun.sh): fuera, dicho
        assert!(cfg().escribir(&d, "systemd", "x").unwrap_err().to_string().contains("systemd"));
        // nada de contraseñas ni llaves privadas
        let mut c = cfg();
        c.llave = "-----BEGIN OPENSSH PRIVATE KEY-----".into();
        assert!(c.validar().is_err());
        c = cfg();
        c.nombre = "mal nombre".into();
        assert!(c.validar().is_err());
    }

    /// La descarga contra un servidor local mínimo (sin red): completa, y el reintento no vuelve a bajar.
    #[tokio::test]
    async fn la_descarga_con_su_tamano() {
        let datos: Vec<u8> = (0..200_000u32).map(|i| i as u8).collect();
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let dir = l.local_addr().unwrap();
        let d2 = datos.clone();
        tokio::spawn(async move {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            while let Ok((mut s, _)) = l.accept().await {
                let mut b = [0u8; 1024];
                let _ = s.read(&mut b).await;
                let _ = s.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", d2.len()).as_bytes()).await;
                let _ = s.write_all(&d2).await;
            }
        });
        let c = crate::CarpetaPrueba::nueva("gotv-rpi-bajar");
        let p = descargar(&format!("http://{dir}/x.img.xz"), &c.join("x.img.xz"), datos.len() as u64, &mut |_| ()).await.unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), datos);
        assert!(descargar(&format!("http://{dir}/x.img.xz"), &c.join("y.img.xz"), 999, &mut |_| ()).await.is_err(), "tamaño distinto: incompleta");
        assert!(!c.join("y.img.parcial").exists() && !c.join("y.img.xz").exists());
    }
}
