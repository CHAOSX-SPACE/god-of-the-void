//! F22 pasos 2 y 8 · Discos y memorias externos: listar (nombre, tamaño, formato, dónde están montados),
//! expulsar, formatear (FAT en Rust), y grabar una imagen y VERIFICARLA releyéndola.
//!
//! Todo en Rust y en proceso (pedido del Portador): lo que dice el sistema de cada disco viene de
//! DiskArbitration e IOKit (`mac.rs`), sin `diskutil`; desmontar, expulsar y montar van por DiskArbitration;
//! formatear escribe la tabla MBR y el sistema FAT con `fatfs` (Rust) sobre el disco crudo. exFAT, APFS y
//! HFS+ no tienen implementación en Rust: no se ofrecen.
//!
//! Frenos en el CÓDIGO, no en una regla:
//! - **El disco interno y el de arranque no existen** para esta herramienta: se filtran antes de listar y
//!   `comprobar` rechaza cualquier orden sobre ellos (también si el Mac arranca desde un disco externo).
//!   Tampoco existen los contenedores APFS sintetizados (se actúa sobre su disco físico) ni las imágenes de
//!   sólo lectura (medido en esta Mac: Xcode monta 5 imágenes de simuladores).
//! - **Se identifica por huella, no por posición** (freno 3): `listar` da una huella por disco (nombre,
//!   tamaño y la huella de la serie del USB, o la ruta de la imagen) y cada acción la exige y la recalcula.
//! - Grabar en el disco crudo pide administrador: si `/dev/rdiskN` no se puede abrir, se le pide el
//!   descriptor a macOS con su cuadro de autorización (`mac::abrir_con_autorizacion`).
use super::mac::{self, Medio, Orden};
use anyhow::{anyhow, bail, Result};
use sha2::{Digest, Sha256};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Lo que se sabe de los discos (del sistema, o armado a mano en las pruebas).
#[derive(Default, Clone)]
pub struct Mesa {
    pub medios: Vec<Medio>,
    /// los discos enteros de la cadena de arranque (el de `/` y su disco físico)
    pub arranque: Vec<String>,
}

/// `disk4s2` → `disk4`.
pub fn entero(id: &str) -> String {
    let id = id.trim_start_matches("/dev/").trim_start_matches('r');
    let id = if id.starts_with("disk") { id.to_string() } else { format!("disk{id}") };
    match id[4..].find('s') {
        Some(i) => id[..4 + i].to_string(),
        None => id,
    }
}

fn valido(id: &str) -> bool {
    id.starts_with("disk") && id.len() > 4 && id[4..].chars().all(|c| c.is_ascii_digit() || c == 's')
}

/// ¿Es una imagen de disco? Medido 2026-11-17: para el sistema una imagen montada es «Virtual Interface»
/// (IOKit `Physical Interconnect`); el «Disk Image» que muestra `diskutil` lo pone diskutil, no el sistema.
pub fn es_imagen(m: &Medio) -> bool {
    matches!(m.protocolo.as_str(), "Virtual Interface" | "Disk Image") || m.marca.starts_with("imagen:")
}

impl Mesa {
    /// La de esta Mac, ahora.
    pub fn real() -> Result<Mesa> {
        let medios = mac::medios();
        if medios.is_empty() {
            bail!("{}", crate::i18n::t("el sistema no describió ningún disco (DiskArbitration)", "the system described no disks (DiskArbitration)"));
        }
        let mut arranque = Vec::new();
        if let Some(r) = mac::dispositivo_raiz() {
            let e = entero(&r);
            if let Some(f) = medios.iter().find(|m| m.id == e).and_then(|m| m.fisico.clone()) {
                arranque.push(f);
            }
            arranque.push(e);
        }
        arranque.sort();
        arranque.dedup();
        Ok(Mesa { medios, arranque })
    }

    fn medio(&self, id: &str) -> Option<&Medio> {
        self.medios.iter().find(|m| m.id == id)
    }

    pub fn enteros(&self) -> Vec<String> {
        self.medios.iter().filter(|m| m.entero).map(|m| m.id.clone()).collect()
    }

    /// Por qué un disco NO existe para esta herramienta (`None` = se puede mostrar).
    /// `imagenes`: mostrar también las imágenes de disco que el usuario montó con escritura.
    pub fn motivo(&self, id: &str, imagenes: bool) -> Option<String> {
        let id = entero(id);
        let Some(m) = self.medio(&id) else {
            return Some(crate::tr!("«{id}» no existe (¿se desconectó? vuelve a listar)", "«{id}» does not exist (was it unplugged? list again)"));
        };
        if self.arranque.contains(&id) {
            return Some(crate::tr!("«{id}» es el disco de arranque de este Mac", "«{id}» is this Mac's startup disk"));
        }
        if m.interno {
            return Some(crate::tr!("«{id}» es un disco interno del Mac", "«{id}» is an internal Mac disk"));
        }
        if m.clase == "AppleAPFSMedia" || m.fisico.is_some() {
            return Some(crate::tr!("«{id}» es un contenedor APFS sintetizado: se actúa sobre su disco físico", "«{id}» is a synthesized APFS container: act on its physical disk"));
        }
        if !m.escribible {
            return Some(crate::tr!(
                "«{id}» es de sólo lectura (una imagen del sistema, p. ej. un simulador de Xcode, o una tarjeta con el seguro puesto)",
                "«{id}» is read-only (a system image, e.g. an Xcode simulator, or a card with its lock switch on)"
            ));
        }
        if es_imagen(m) && !imagenes {
            return Some(crate::tr!("«{id}» es una imagen de disco (imagenes=true para verla)", "«{id}» is a disk image (imagenes=true to see it)"));
        }
        None
    }

    /// Las particiones de un disco entero, y los volúmenes de los contenedores APFS que salen de él.
    pub fn particiones(&self, id: &str) -> Vec<Volumen> {
        let pref = format!("{id}s");
        let sinteticos: Vec<&str> = self.medios.iter().filter(|m| m.entero && m.fisico.as_deref() == Some(id)).map(|m| m.id.as_str()).collect();
        self.medios
            .iter()
            .filter(|m| !m.entero && (m.id.starts_with(&pref) || sinteticos.iter().any(|s| m.id.starts_with(&format!("{s}s")))))
            .chain(self.medios.iter().filter(|m| m.id == id && !m.volumen.is_empty()))
            .map(|m| Volumen {
                id: m.id.clone(),
                nombre: m.volumen.clone(),
                formato: m.formato.clone(),
                archivos: if m.montado.is_empty() || m.montado == "/" { None } else { archivos_de(Path::new(&m.montado)) },
                montado: m.montado.clone(),
                tam: m.tam,
                usado: m.libre.map(|l| m.tam.saturating_sub(l)),
            })
            .collect()
    }

    /// La huella de un disco entero: nombre, tamaño y lo que lo distingue (serie USB o imagen).
    pub fn huella(&self, id: &str) -> String {
        let id = entero(id);
        let base = match self.medio(&id) {
            Some(m) => format!("{}|{}|{}|{}", m.nombre_medio, m.tam, m.protocolo, m.marca),
            None => String::new(),
        };
        Sha256::digest(base.as_bytes()).iter().take(4).map(|b| format!("{b:02x}")).collect()
    }

    pub fn disco(&self, id: &str) -> Option<Disco> {
        let id = entero(id);
        let m = self.medio(&id)?;
        Some(Disco { nombre: m.nombre_medio.clone(), tam: m.tam, bus: m.protocolo.clone(), volumenes: self.particiones(&id), huella: self.huella(&id), id })
    }

    /// Los discos que esta herramienta puede ver.
    pub fn listar(&self, imagenes: bool) -> Vec<Disco> {
        self.enteros().iter().filter(|d| self.motivo(d, imagenes).is_none()).filter_map(|d| self.disco(d)).collect()
    }

    /// El freno ANTES de cualquier acción: existe, no es interno ni de arranque, y es el MISMO (huella).
    pub fn comprobar(&self, id: &str, huella: Option<&str>, imagenes: bool) -> Result<Disco> {
        let id_ent = entero(id);
        if !valido(&id_ent) {
            bail!("{}", crate::tr!("«{id}» no es un identificador de disco (diskN)", "«{id}» is not a disk identifier (diskN)"));
        }
        if let Some(m) = self.motivo(&id_ent, imagenes) {
            bail!("{}", crate::tr!("{m}: GOTV no actúa sobre él", "{m}: GOTV will not act on it"));
        }
        let d = self.disco(&id_ent).ok_or_else(|| anyhow!("{}", crate::tr!("«{id}» desapareció", "«{id}» disappeared")))?;
        match huella {
            Some(h) if h == d.huella => Ok(d),
            Some(_) => bail!("{}", crate::tr!(
                "«{id_ent}» ya no es el mismo disco que listaste (otro nombre, tamaño o serie): vuelve a listar y usa su huella nueva",
                "«{id_ent}» is no longer the disk you listed (different name, size or serial): list again and use its new fingerprint"
            )),
            None => bail!("{}", crate::tr!("falta `huella` (la da action=listar): se identifica por huella, no por posición", "missing `huella` (action=listar gives it): disks are identified by fingerprint, not position")),
        }
    }
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct Volumen {
    pub id: String,
    pub nombre: String,
    pub formato: String,
    pub montado: String,
    pub tam: u64,
    pub usado: Option<u64>,
    /// archivos del usuario en la raíz del volumen (sin los ocultos que crea macOS); `None` = no se pudo mirar
    pub archivos: Option<usize>,
}

/// Lo que macOS (o Windows) crea solo en una memoria: no son datos del usuario.
fn del_sistema(n: &str) -> bool {
    n.starts_with("._") || matches!(n, ".Spotlight-V100" | ".fseventsd" | ".Trashes" | ".TemporaryItems" | ".DS_Store" | ".VolumeIcon.icns" | "System Volume Information")
}

/// Cuántos archivos del usuario hay en la raíz de un volumen montado.
pub fn archivos_de(raiz: &Path) -> Option<usize> {
    Some(std::fs::read_dir(raiz).ok()?.flatten().filter(|e| !del_sistema(&e.file_name().to_string_lossy())).count())
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct Disco {
    pub id: String,
    pub nombre: String,
    pub tam: u64,
    pub bus: String,
    pub volumenes: Vec<Volumen>,
    pub huella: String,
}

pub fn gb(b: u64) -> String {
    if b >= 1_000_000_000 {
        format!("{:.1} GB", b as f64 / 1e9)
    } else {
        format!("{:.1} MB", b as f64 / 1e6)
    }
}

impl Disco {
    /// ¿Tiene datos que se perderían? Un solo archivo del usuario ya son datos (medido 2026-11-17: una tarjeta
    /// con `config.txt` y `cmdline.txt` ocupaba pocos KB y «más de 2 MB usados» la daba por vacía). Una
    /// partición que no se puede mirar (sin montar, o la ext4 de una Raspberry) se da por llena.
    pub fn con_datos(&self) -> bool {
        self.volumenes.iter().any(|v| match (v.archivos, v.usado) {
            (Some(n), _) if n > 0 => true,
            (Some(_), Some(u)) => u > 2_000_000,
            (Some(_), None) => false,
            (None, Some(u)) => u > 2_000_000,
            (None, None) => v.tam > 2_000_000,
        })
    }

    /// Freno 2: un disco de más de 64 GB o con datos exige teclear su nombre.
    pub fn pide_nombre(&self) -> bool {
        self.tam > 64_000_000_000 || self.con_datos()
    }

    /// El nombre que hay que teclear: el del primer volumen con nombre, o el del aparato.
    pub fn nombre_a_teclear(&self) -> String {
        self.volumenes.iter().map(|v| v.nombre.clone()).find(|n| !n.is_empty()).unwrap_or_else(|| self.nombre.clone())
    }

    /// Nombre, tamaño y lo que se perdería.
    pub fn resumen(&self) -> String {
        let vols: Vec<String> = self
            .volumenes
            .iter()
            .map(|v| {
                let uso = v.usado.map(|u| crate::tr!(", {} usados", ", {} used", gb(u))).unwrap_or_default();
                let donde = if v.montado.is_empty() { String::new() } else { format!(" @ {}", v.montado) };
                format!("{} «{}» {} {}{uso}{donde}", v.id, v.nombre, v.formato, gb(v.tam))
            })
            .collect();
        let vols = if vols.is_empty() { crate::i18n::t("sin volúmenes", "no volumes").to_string() } else { vols.join("; ") };
        format!("{} · {} · {} · {} · {} [{}]", self.id, self.nombre, gb(self.tam), self.bus, vols, self.huella)
    }
}

const ESPERA: Duration = Duration::from_secs(60);

/// Desmonta todos los volúmenes del disco (si algo los tiene abiertos, se dice).
pub fn desmontar(id: &str) -> Result<()> {
    mac::ordenar(&entero(id), Orden::DesmontarTodo, ESPERA).map_err(|e| {
        anyhow!("{}", crate::tr!("no se pudo desmontar {} ({e}): ¿algún programa tiene archivos abiertos en él?", "could not unmount {} ({e}): does some program have files open on it?", entero(id)))
    })
}

pub fn expulsar(id: &str) -> Result<()> {
    desmontar(id)?;
    mac::ordenar(&entero(id), Orden::Expulsar, ESPERA).map_err(|e| anyhow!("{}", crate::tr!("no se pudo expulsar ({e})", "could not eject ({e})")))
}

/// Tras escribir en crudo: macOS relee la tabla de particiones UN MOMENTO después de cerrar el disco
/// (medido 2026-11-17: montar enseguida no veía nada). Se espera hasta 10 s a que aparezcan y se monta.
pub fn releer(id: &str) {
    let id = entero(id);
    let pref = format!("{id}s");
    for _ in 0..20 {
        if mac::medios().iter().any(|m| m.id.starts_with(&pref)) {
            let _ = montar(&id);
            return;
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    let _ = montar(&id);
}

pub fn montar(id: &str) -> Result<()> {
    mac::ordenar(&entero(id), Orden::MontarTodo, ESPERA).map_err(|e| anyhow!("{}", crate::tr!("no se pudo montar {} ({e})", "could not mount {} ({e})", entero(id))))
}

pub fn crudo(id: &str) -> PathBuf {
    PathBuf::from(format!("/dev/r{}", entero(id)))
}

/// Abre el disco crudo: directo si se puede, si no con el cuadro de administrador de macOS.
pub fn abrir_crudo(dev: &Path, escribir: bool) -> Result<std::fs::File> {
    match std::fs::OpenOptions::new().read(true).write(escribir).open(dev) {
        Ok(f) => Ok(f),
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied && dev.starts_with("/dev") => mac::abrir_con_autorizacion(&dev.display().to_string(), escribir).map_err(|e| {
            anyhow!("{}", crate::tr!("macOS no dio permiso para {} ({e}): ¿se canceló el cuadro de administrador?", "macOS did not grant access to {} ({e}): was the administrator dialog cancelled?", dev.display()))
        }),
        Err(e) => Err(anyhow!("{}: {e}", dev.display())),
    }
}

/// Termina la escritura. En macOS `sync_all` es F_FULLFSYNC y un disco crudo lo rechaza con ENOTTY (medido
/// 2026-11-17): lo escrito en crudo no pasa por la caché de bloques, basta un fsync simple.
pub fn cerrar(f: std::fs::File) -> Result<()> {
    match f.sync_all() {
        Err(e) if matches!(e.raw_os_error(), Some(libc::ENOTTY) | Some(libc::ENOTSUP)) => {
            use std::os::fd::AsRawFd;
            unsafe { libc::fsync(f.as_raw_fd()) };
            Ok(())
        }
        r => Ok(r?),
    }
}

// ───────── formatear: MBR + FAT, en Rust ─────────

const BLOQUE: usize = 512;
const PAGINA: u64 = 4096;
/// La partición empieza en 1 MiB (alineada para cualquier tarjeta)
const INICIO: u64 = 1 << 20;

/// Lectura y escritura en bloques alineados de 4 KiB (lo que exige un disco crudo) sobre una ventana del
/// disco [`base`, `base + largo`). Guarda un bloque y lo escribe al cambiar de bloque o al vaciar.
pub struct Ventana<F: Read + Write + Seek> {
    f: F,
    base: u64,
    largo: u64,
    pos: u64,
    bloque: Option<(u64, Vec<u8>, bool)>,
}

impl<F: Read + Write + Seek> Ventana<F> {
    pub fn nueva(f: F, base: u64, largo: u64) -> Self {
        Ventana { f, base, largo, pos: 0, bloque: None }
    }

    fn vaciar(&mut self) -> std::io::Result<()> {
        if let Some((n, b, sucio)) = &mut self.bloque {
            if *sucio {
                self.f.seek(SeekFrom::Start(self.base + *n * PAGINA))?;
                self.f.write_all(b)?;
                *sucio = false;
            }
        }
        Ok(())
    }

    fn cargar(&mut self, n: u64) -> std::io::Result<()> {
        if self.bloque.as_ref().is_some_and(|b| b.0 == n) {
            return Ok(());
        }
        self.vaciar()?;
        let mut b = vec![0u8; PAGINA as usize];
        self.f.seek(SeekFrom::Start(self.base + n * PAGINA))?;
        let mut k = 0;
        while k < b.len() {
            match self.f.read(&mut b[k..])? {
                0 => break,
                r => k += r,
            }
        }
        self.bloque = Some((n, b, false));
        Ok(())
    }

    pub fn terminar(mut self) -> std::io::Result<F> {
        self.vaciar()?;
        self.f.flush()?;
        Ok(self.f)
    }
}

impl<F: Read + Write + Seek> Read for Ventana<F> {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        if self.pos >= self.largo || out.is_empty() {
            return Ok(0);
        }
        let (n, off) = (self.pos / PAGINA, (self.pos % PAGINA) as usize);
        self.cargar(n)?;
        let b = &self.bloque.as_ref().unwrap().1;
        let k = out.len().min(PAGINA as usize - off).min((self.largo - self.pos) as usize);
        out[..k].copy_from_slice(&b[off..off + k]);
        self.pos += k as u64;
        Ok(k)
    }
}

impl<F: Read + Write + Seek> Write for Ventana<F> {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        if self.pos >= self.largo {
            return Err(std::io::Error::other(crate::i18n::t("se llegó al final del disco", "reached the end of the disk")));
        }
        let (n, off) = (self.pos / PAGINA, (self.pos % PAGINA) as usize);
        self.cargar(n)?;
        let k = data.len().min(PAGINA as usize - off).min((self.largo - self.pos) as usize);
        let b = self.bloque.as_mut().unwrap();
        b.1[off..off + k].copy_from_slice(&data[..k]);
        b.2 = true;
        self.pos += k as u64;
        Ok(k)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.vaciar()?;
        self.f.flush()
    }
}

impl<F: Read + Write + Seek> Seek for Ventana<F> {
    fn seek(&mut self, s: SeekFrom) -> std::io::Result<u64> {
        let p = match s {
            SeekFrom::Start(p) => p as i64,
            SeekFrom::End(d) => self.largo as i64 + d,
            SeekFrom::Current(d) => self.pos as i64 + d,
        };
        if p < 0 {
            return Err(std::io::Error::other("seek"));
        }
        self.pos = p as u64;
        Ok(self.pos)
    }
}

/// Nombre válido para un volumen FAT: 11 letras en mayúsculas, sin símbolos.
pub fn nombre_volumen(nombre: &str) -> String {
    let limpio: String = nombre.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-').collect();
    let limpio = if limpio.is_empty() { "GOTV".to_string() } else { limpio };
    limpio.to_ascii_uppercase().chars().take(11).collect()
}

/// El tipo de FAT según el tamaño (FAT32 desde 64 MiB, como hace macOS con las memorias) y su código MBR.
pub fn tipo_fat(bytes: u64) -> (fatfs::FatType, u8, &'static str) {
    if bytes >= 64 << 20 {
        (fatfs::FatType::Fat32, 0x0C, "FAT32")
    } else if bytes >= 16 << 20 {
        (fatfs::FatType::Fat16, 0x0E, "FAT16")
    } else {
        (fatfs::FatType::Fat12, 0x01, "FAT12")
    }
}

/// Una tabla MBR con UNA partición desde 1 MiB hasta el final.
pub fn mbr(total: u64, tipo: u8) -> [u8; 512] {
    let mut s = [0u8; 512];
    let inicio = (INICIO / BLOQUE as u64) as u32;
    let sectores = (total / BLOQUE as u64).saturating_sub(inicio as u64).min(u32::MAX as u64) as u32;
    let e = &mut s[446..462];
    e[0] = 0x00; // no arrancable
    e[1..4].copy_from_slice(&[0xFE, 0xFF, 0xFF]); // CHS: «usar LBA»
    e[4] = tipo;
    e[5..8].copy_from_slice(&[0xFE, 0xFF, 0xFF]);
    e[8..12].copy_from_slice(&inicio.to_le_bytes());
    e[12..16].copy_from_slice(&sectores.to_le_bytes());
    // firma de disco (para que el sistema distinga este de otro)
    let id = (std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0) as u32).to_le_bytes();
    s[440..444].copy_from_slice(&id);
    s[510] = 0x55;
    s[511] = 0xAA;
    s
}

/// Formatea en `f` (el disco entero crudo, o un archivo que hace de disco) de `total` bytes: borra las
/// tablas viejas (también la copia GPT del final), escribe el MBR y crea el sistema FAT. Devuelve el tipo.
pub fn formatear_en<F: Read + Write + Seek>(f: F, total: u64, nombre: &str) -> Result<(&'static str, F)> {
    if total < INICIO + (2 << 20) {
        bail!("{}", crate::i18n::t("el disco es demasiado chico para formatearlo", "the disk is too small to format"));
    }
    let largo = (total - INICIO) / PAGINA * PAGINA;
    let (tipo, codigo, texto) = tipo_fat(largo);
    let mut v = Ventana::nueva(f, 0, total / PAGINA * PAGINA);
    // fuera las tablas viejas: el primer MiB (MBR y GPT) y el último (la copia de GPT)
    let ceros = vec![0u8; 1 << 20];
    v.seek(SeekFrom::Start(0))?;
    v.write_all(&ceros)?;
    let fin = (total / PAGINA * PAGINA).saturating_sub(1 << 20).max(INICIO);
    v.seek(SeekFrom::Start(fin))?;
    let resto = (total / PAGINA * PAGINA - fin) as usize;
    v.write_all(&ceros[..resto.min(ceros.len())])?;
    v.seek(SeekFrom::Start(0))?;
    v.write_all(&mbr(total, codigo))?;
    let f = v.terminar()?;
    let mut etiqueta = [b' '; 11];
    let n = nombre_volumen(nombre);
    etiqueta[..n.len()].copy_from_slice(n.as_bytes());
    let part = Ventana::nueva(f, INICIO, largo);
    let opciones = fatfs::FormatVolumeOptions::new().fat_type(tipo).volume_label(etiqueta).total_sectors((largo / BLOQUE as u64) as u32).bytes_per_sector(BLOQUE as u16);
    let mut part = part;
    fatfs::format_volume(&mut part, opciones).map_err(|e| anyhow!("{}", crate::tr!("no se pudo crear el sistema {texto}: {e}", "could not create the {texto} filesystem: {e}")))?;
    Ok((texto, part.terminar()?))
}

/// Formatea el disco ENTERO (tras `comprobar`): desmonta, escribe en crudo y vuelve a montar.
pub fn formatear(id: &str, tam: u64, nombre: &str) -> Result<String> {
    desmontar(id)?;
    let f = abrir_crudo(&crudo(id), true)?;
    let (texto, f) = formatear_en(f, tam, nombre)?;
    cerrar(f)?;
    let _ = montar(id);
    Ok(crate::tr!("✓ formateado {} como {texto} «{}» (MBR, una partición)", "✓ formatted {} as {texto} «{}» (MBR, one partition)", entero(id), nombre_volumen(nombre)))
}

/// Los formatos que NO tienen implementación en Rust (exFAT, APFS, HFS+) → (nombre de diskutil, esquema).
/// `None` para FAT, que se formatea en Rust.
pub fn formato_de_apple(f: &str) -> Result<Option<(&'static str, &'static str)>> {
    Ok(match f.to_ascii_lowercase().as_str() {
        "fat32" | "fat" | "msdos" => None,
        "exfat" => Some(("ExFAT", "MBRFormat")),
        "apfs" => Some(("APFS", "GPTFormat")),
        "hfs" | "hfs+" | "jhfs+" | "macos" => Some(("JHFS+", "GPTFormat")),
        otro => bail!("{}", crate::tr!("formato «{otro}» desconocido: fat32 · exfat · apfs · hfs", "unknown format «{otro}»: fat32 · exfat · apfs · hfs")),
    })
}

/// Nombre para un volumen exFAT, APFS o HFS+ (letras, números, espacio, guion; hasta 64).
pub fn nombre_volumen_apple(nombre: &str) -> String {
    let l: String = nombre.chars().filter(|c| c.is_ascii_alphanumeric() || " _-".contains(*c)).take(64).collect();
    if l.trim().is_empty() { "GOTV".into() } else { l.trim().to_string() }
}

/// EXCEPCIÓN DECLARADA a «todo en Rust» (decisión del Portador, 2026-11-17): exFAT, APFS y HFS+ no tienen
/// implementación en Rust, así que esos tres formatos van con `diskutil eraseDisk`, la herramienta de Apple.
/// FAT sigue en Rust (`formatear`). Los frenos son los mismos: se llama tras `comprobar` y el nombre tecleado.
pub fn formatear_con_diskutil(id: &str, fs: &str, esquema: &str, nombre: &str) -> Result<String> {
    let nom = nombre_volumen_apple(nombre);
    let o = std::process::Command::new("/usr/sbin/diskutil").args(["eraseDisk", fs, &nom, esquema, &entero(id)]).output()?;
    if !o.status.success() {
        bail!("diskutil eraseDisk: {}{}", String::from_utf8_lossy(&o.stdout).trim(), String::from_utf8_lossy(&o.stderr).trim());
    }
    Ok(crate::tr!("✓ formateado {} como {fs} «{nom}» (con diskutil: {fs} no tiene implementación en Rust)", "✓ formatted {} as {fs} «{nom}» (with diskutil: {fs} has no Rust implementation)", entero(id)))
}

// ───────── grabar una imagen y verificarla ─────────

const TAMPON: usize = 4 << 20;

/// Escritor que manda bloques de 512 (lo que exige un disco crudo), resume lo escrito y avisa del avance.
pub struct Alineado<'a, W: Write> {
    w: W,
    buf: Vec<u8>,
    hash: Sha256,
    pub n: u64,
    avance: &'a mut dyn FnMut(u64),
    ultimo: u64,
}

impl<'a, W: Write> Alineado<'a, W> {
    pub fn nuevo(w: W, avance: &'a mut dyn FnMut(u64)) -> Self {
        Alineado { w, buf: Vec::with_capacity(TAMPON), hash: Sha256::new(), n: 0, avance, ultimo: 0 }
    }

    /// Escribe lo que queda (rellenando con ceros hasta 512) y devuelve (bytes de la imagen, su SHA-256, el escritor).
    pub fn terminar(mut self) -> std::io::Result<(u64, String, W)> {
        if !self.buf.is_empty() {
            let resto = self.buf.len() % BLOQUE;
            if resto != 0 {
                self.buf.resize(self.buf.len() + BLOQUE - resto, 0);
            }
            self.w.write_all(&self.buf)?;
        }
        self.w.flush()?;
        Ok((self.n, format!("{:x}", self.hash.finalize()), self.w))
    }
}

impl<W: Write> Write for Alineado<'_, W> {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        self.hash.update(data);
        self.n += data.len() as u64;
        self.buf.extend_from_slice(data);
        if self.buf.len() >= TAMPON {
            let entero = self.buf.len() - self.buf.len() % BLOQUE;
            self.w.write_all(&self.buf[..entero])?;
            self.buf.drain(..entero);
        }
        if self.n - self.ultimo >= 64 << 20 {
            self.ultimo = self.n;
            (self.avance)(self.n);
        }
        Ok(data.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// ¿La imagen viene comprimida con xz? (por la firma, no por el nombre)
pub fn es_xz(cabeza: &[u8]) -> bool {
    cabeza.starts_with(&[0xFD, b'7', b'z', b'X', b'Z', 0x00])
}

/// Copia la imagen `origen` (cruda o `.xz`) al escritor, alineada. Devuelve (bytes, SHA-256 de lo escrito).
pub fn volcar<W: Write>(origen: &Path, destino: W, avance: &mut dyn FnMut(u64)) -> Result<(u64, String, W)> {
    let mut f = std::io::BufReader::with_capacity(TAMPON, std::fs::File::open(origen).map_err(|e| anyhow!("{}: {e}", origen.display()))?);
    let mut cabeza = [0u8; 6];
    let leidos = {
        use std::io::BufRead;
        let b = f.fill_buf()?;
        let k = b.len().min(6);
        cabeza[..k].copy_from_slice(&b[..k]);
        k
    };
    let mut al = Alineado::nuevo(destino, avance);
    if leidos == 6 && es_xz(&cabeza) {
        // lzma-rs descomprime cada bloque entero en memoria antes de escribirlo y verifica su CRC32/CRC64.
        // Medido 2026-11-17 en la imagen oficial Raspberry Pi OS Lite (64-bit) por su índice xz (petición de
        // rango de 64 KB): CRC64 y 16 bloques de 192 MiB → ~192 MiB de RAM al grabar, no la imagen entera
        lzma_rs::xz_decompress(&mut f, &mut al).map_err(|e| anyhow!("{}", crate::tr!("la imagen .xz está dañada: {e:?}", "the .xz image is damaged: {e:?}")))?;
    } else {
        std::io::copy(&mut f, &mut al)?;
    }
    Ok(al.terminar()?)
}

/// Relee `n` bytes del lector y compara su SHA-256.
pub fn verificar<R: Read>(mut lector: R, n: u64, sha: &str) -> Result<()> {
    let mut h = Sha256::new();
    let mut quedan = n;
    let mut buf = vec![0u8; TAMPON];
    while quedan > 0 {
        // los discos crudos leen en bloques de 512: se lee redondeado y se resume sólo lo de la imagen
        let pedir = (quedan as usize).min(TAMPON);
        let pedir_al = pedir.div_ceil(BLOQUE) * BLOQUE;
        let mut k = 0;
        while k < pedir_al {
            let r = lector.read(&mut buf[k..pedir_al])?;
            if r == 0 {
                break;
            }
            k += r;
        }
        if k < pedir {
            bail!("{}", crate::tr!("la verificación leyó menos de lo grabado ({} de {n} bytes)", "verification read less than written ({} of {n} bytes)", n - quedan + k as u64));
        }
        h.update(&buf[..pedir]);
        quedan -= pedir as u64;
    }
    let leido = format!("{:x}", h.finalize());
    if leido != sha {
        bail!("{}", crate::tr!("✗ la verificación NO casa: lo releído (sha256 {}) no es lo grabado ({}). No uses este disco", "✗ verification FAILED: what was read back (sha256 {}) is not what was written ({}). Do not use this disk", &leido[..12], &sha[..12]));
    }
    Ok(())
}

/// Graba la imagen en el disco (ya comprobado) y la verifica releyéndola. `dev` es `/dev/rdiskN` (o un
/// archivo que hace de disco en las pruebas).
pub fn grabar_y_verificar(origen: &Path, dev: &Path, avance: &mut dyn FnMut(u64)) -> Result<(u64, String)> {
    let w = abrir_crudo(dev, true)?;
    let (n, sha, w) = volcar(origen, w, avance)?;
    cerrar(w)?;
    let r = abrir_crudo(dev, false)?;
    verificar(r, n, &sha)?;
    Ok((n, sha))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    fn medio(id: &str, f: impl FnOnce(&mut Medio)) -> Medio {
        let mut m = Medio { id: id.into(), entero: !id[4..].contains('s'), escribible: true, clase: "IOMedia".into(), ..Default::default() };
        f(&mut m);
        m
    }

    /// La mesa de ESTA Mac medida el 2026-11-16/17: el interno (disk0), el contenedor sintetizado de
    /// arranque (disk3, de disk0), una imagen de simulador de Xcode (disk4, sólo lectura) y una imagen de
    /// 4 MB montada por el usuario (disk14) con su volumen FAT.
    pub fn mesa() -> Mesa {
        let medios = vec![
            medio("disk0", |m| {
                m.tam = 1_000_555_581_440;
                m.nombre_medio = "APPLE SSD AP1024R".into();
                m.protocolo = "Apple Fabric".into();
                m.interno = true;
            }),
            medio("disk0s2", |m| m.tam = 994_662_584_320),
            medio("disk3", |m| {
                m.tam = 994_662_584_320;
                m.interno = true;
                m.clase = "AppleAPFSMedia".into();
                m.fisico = Some("disk0".into());
            }),
            medio("disk3s1s1", |m| {
                m.volumen = "Macintosh HD".into();
                m.montado = "/".into();
            }),
            medio("disk4", |m| {
                m.tam = 3_298_854_400;
                m.nombre_medio = "Disk Image".into();
                m.protocolo = "Virtual Interface".into();
                m.escribible = false;
            }),
            medio("disk14", |m| {
                m.tam = 4_194_304;
                m.nombre_medio = "Disk Image".into();
                m.protocolo = "Virtual Interface".into();
            }),
            medio("disk14s1", |m| {
                m.tam = 4_193_792;
                m.volumen = "PRUEBAGOTV".into();
                m.formato = "msdos".into();
                m.montado = "/Volumes/PRUEBAGOTV".into();
                m.libre = Some(4_164_608);
            }),
        ];
        Mesa { medios, arranque: vec!["disk0".into(), "disk3".into()] }
    }

    /// La misma imagen de 4 MB como si fuera una memoria USB de verdad.
    pub fn mesa_con_memoria() -> Mesa {
        let mut m = mesa();
        let d = m.medios.iter_mut().find(|x| x.id == "disk14").unwrap();
        d.protocolo = "USB".into();
        d.nombre_medio = "SanDisk Cruzer".into();
        d.marca = "usb:1a2b3c4d".into();
        m
    }

    #[test]
    fn el_disco_interno_desaparece() {
        let m = mesa();
        assert!(m.listar(false).is_empty(), "{:?}", m.listar(false));
        assert!(m.motivo("disk0", true).unwrap().contains("disk0"));
        assert!(m.motivo("disk3", true).is_some(), "el contenedor sintetizado no existe");
        assert!(m.motivo("disk4", true).unwrap().contains("disk4"), "la imagen del simulador (sólo lectura) no existe ni con imagenes=true");
        assert!(m.motivo("disk14", false).is_some(), "una imagen del usuario («Virtual Interface») sólo con imagenes=true");
        let mut ro = mesa_con_memoria();
        ro.medios.iter_mut().find(|x| x.id == "disk14").unwrap().escribible = false;
        assert!(ro.listar(true).is_empty(), "una memoria de sólo lectura (el seguro puesto) no se ofrece");
        let v = m.listar(true);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].id, "disk14");
        assert_eq!(v[0].volumenes.len(), 1);
        assert_eq!(v[0].volumenes[0].nombre, "PRUEBAGOTV");
        assert_eq!(v[0].volumenes[0].montado, "/Volumes/PRUEBAGOTV");
        assert!(!v[0].con_datos(), "recién creada: vacía");
        let m = mesa_con_memoria();
        let v = m.listar(false);
        assert_eq!(v.len(), 1);
        assert!(v[0].resumen().contains("SanDisk Cruzer") && v[0].resumen().contains("4.2 MB"), "{}", v[0].resumen());
    }

    /// ESTA Mac, de verdad: el interno, el de arranque y las imágenes del sistema NO aparecen.
    #[test]
    fn en_esta_mac_el_interno_no_existe() {
        let m = Mesa::real().unwrap();
        assert!(m.arranque.contains(&"disk0".to_string()), "{:?}", m.arranque);
        for d in m.listar(true) {
            assert!(!m.arranque.contains(&d.id), "{d:?}");
            assert!(m.medios.iter().any(|x| x.id == d.id && !x.interno && x.escribible));
        }
        assert!(m.comprobar("disk0", Some(&m.huella("disk0")), true).is_err());
    }

    #[test]
    fn una_orden_sobre_el_interno_o_el_de_arranque_se_rechaza() {
        let m = mesa_con_memoria();
        let h0 = m.huella("disk0");
        assert!(m.comprobar("disk0", Some(&h0), true).is_err());
        assert!(m.comprobar("disk0s2", Some(&h0), true).is_err(), "una partición del interno tampoco");
        assert!(m.comprobar("/dev/rdisk3", None, true).is_err());
        assert!(m.comprobar("disk99", Some("x"), true).is_err());
        assert!(m.comprobar("disk; rm -rf /", None, true).is_err());
        let h = m.huella("disk14");
        assert!(m.comprobar("disk14", Some(&h), false).is_ok());
        let mut m2 = mesa_con_memoria();
        m2.arranque.push("disk14".into());
        assert!(m2.comprobar("disk14", Some(&h), false).unwrap_err().to_string().contains("disk14"));
    }

    #[test]
    fn otra_memoria_en_la_misma_posicion_no_pasa() {
        let m = mesa_con_memoria();
        let h = m.huella("disk14");
        assert_eq!(h.len(), 8);
        assert!(m.comprobar("disk14", None, false).is_err(), "sin huella no se actúa");
        let mut otra = mesa_con_memoria();
        otra.medios.iter_mut().find(|x| x.id == "disk14").unwrap().marca = "usb:99999999".into();
        assert!(otra.comprobar("disk14", Some(&h), false).unwrap_err().to_string().contains("disk14"));
        let mut otra = mesa_con_memoria();
        otra.medios.iter_mut().find(|x| x.id == "disk14").unwrap().tam = 64_000_000_000;
        assert_ne!(otra.huella("disk14"), h);
    }

    #[test]
    fn el_freno_del_nombre() {
        let m = mesa_con_memoria();
        let mut d = m.disco("disk14").unwrap();
        assert!(!d.pide_nombre(), "4 MB vacía: basta el sí");
        assert_eq!(d.nombre_a_teclear(), "PRUEBAGOTV");
        d.volumenes[0].usado = Some(50_000_000);
        assert!(d.pide_nombre());
        d.volumenes[0].usado = Some(0);
        d.tam = 128_000_000_000;
        assert!(d.pide_nombre());
        // un solo archivo del usuario ya son datos; los ocultos que crea macOS no
        let mut d = m.disco("disk14").unwrap();
        d.volumenes[0].archivos = Some(1);
        assert!(d.con_datos());
        d.volumenes[0].archivos = Some(0);
        assert!(!d.con_datos());
        // una partición que no se puede mirar (ext4 de una Raspberry) se da por llena
        d.volumenes[0].archivos = None;
        d.volumenes[0].usado = None;
        assert!(d.con_datos());
        let c = crate::CarpetaPrueba::nueva("gotv-equipos-archivos");
        std::fs::create_dir_all(c.join(".Spotlight-V100")).unwrap();
        std::fs::write(c.join("._x"), b"").unwrap();
        assert_eq!(archivos_de(&c), Some(0));
        std::fs::write(c.join("config.txt"), b"x").unwrap();
        assert_eq!(archivos_de(&c), Some(1));
    }

    /// Formatear en Rust sobre un archivo que hace de disco: el MBR queda bien y `fatfs` vuelve a abrir el
    /// sistema con su nombre; FAT32 desde 64 MiB.
    #[test]
    fn formatear_fat_en_rust_sobre_un_archivo() {
        let d = crate::CarpetaPrueba::nueva("gotv-equipos-fat");
        for (mb, esperado) in [(70u64, "FAT32"), (40, "FAT16")] {
            let p = d.join(format!("disco{mb}.bin"));
            let f = std::fs::OpenOptions::new().create(true).truncate(true).read(true).write(true).open(&p).unwrap();
            f.set_len(mb << 20).unwrap();
            // basura vieja: una cabecera GPT al principio
            let mut f = f;
            f.seek(SeekFrom::Start(512)).unwrap();
            f.write_all(b"EFI PART").unwrap();
            let (tipo, _) = formatear_en(f, mb << 20, "mi memoria").unwrap();
            assert_eq!(tipo, esperado);
            let b = std::fs::read(&p).unwrap();
            assert_eq!(&b[510..512], &[0x55, 0xAA]);
            assert_eq!(b[446 + 4], if esperado == "FAT32" { 0x0C } else { 0x0E });
            assert_eq!(u32::from_le_bytes(b[454..458].try_into().unwrap()), 2048);
            assert!(!b[512..1024].starts_with(b"EFI PART"), "la GPT vieja se borró");
            let part = Ventana::nueva(std::fs::File::open(&p).map(|_| std::fs::OpenOptions::new().read(true).write(true).open(&p).unwrap()).unwrap(), INICIO, (mb << 20) - INICIO);
            let fs = fatfs::FileSystem::new(part, fatfs::FsOptions::new()).unwrap();
            assert_eq!(fs.volume_label(), "MIMEMORIA");
            let mut a = fs.root_dir().create_file("hola.txt").unwrap();
            a.write_all(b"desde GOTV").unwrap();
        }
        assert_eq!(nombre_volumen("un nombre muy largo"), "UNNOMBREMUY");
    }

    /// FAT en Rust; exFAT, APFS y HFS+ con diskutil (la excepción); lo demás no existe.
    #[test]
    fn que_formato_va_por_donde() {
        assert_eq!(formato_de_apple("FAT32").unwrap(), None);
        assert_eq!(formato_de_apple("exfat").unwrap(), Some(("ExFAT", "MBRFormat")));
        assert_eq!(formato_de_apple("apfs").unwrap(), Some(("APFS", "GPTFormat")));
        assert_eq!(formato_de_apple("hfs").unwrap(), Some(("JHFS+", "GPTFormat")));
        assert!(formato_de_apple("ntfs").is_err());
        assert_eq!(nombre_volumen_apple("Fotos; rm -rf /"), "Fotos rm -rf");
        assert_eq!(nombre_volumen_apple("$$$"), "GOTV");
    }

    #[test]
    fn la_ventana_alinea_lecturas_y_escrituras() {
        let mut v = Ventana::nueva(std::io::Cursor::new(vec![0u8; 3 * 4096 + 4096]), 4096, 3 * 4096);
        v.seek(SeekFrom::Start(4090)).unwrap();
        v.write_all(b"cruza el bloque").unwrap();
        v.seek(SeekFrom::Start(4090)).unwrap();
        let mut b = [0u8; 15];
        v.read_exact(&mut b).unwrap();
        assert_eq!(&b, b"cruza el bloque");
        let c = v.terminar().unwrap().into_inner();
        assert_eq!(&c[4096 + 4090..4096 + 4105], b"cruza el bloque", "con su base");
        assert!(c[..4096].iter().all(|x| *x == 0), "nada fuera de la ventana");
    }

    #[test]
    fn grabar_una_imagen_sobre_un_archivo_y_verificarla() {
        let d = crate::CarpetaPrueba::nueva("gotv-equipos-grabar");
        let img: Vec<u8> = (0..3_000_100u32).map(|i| (i.wrapping_mul(2654435761) >> 24) as u8).collect();
        let mut xz = Vec::new();
        lzma_rs::xz_compress(&mut std::io::Cursor::new(&img), &mut xz).unwrap();
        std::fs::write(d.join("sistema.img.xz"), &xz).unwrap();
        std::fs::write(d.join("sistema.img"), &img).unwrap();
        let disco = d.join("disco.bin");
        std::fs::write(&disco, vec![0xAAu8; 4_000_000]).unwrap();
        let (n, sha) = grabar_y_verificar(&d.join("sistema.img.xz"), &disco, &mut |_| ()).unwrap();
        assert_eq!(n, img.len() as u64);
        assert_eq!(sha, format!("{:x}", Sha256::digest(&img)));
        let grabado = std::fs::read(&disco).unwrap();
        assert_eq!(&grabado[..img.len()], &img[..]);
        assert!(grabado[img.len()..img.len().div_ceil(512) * 512].iter().all(|b| *b == 0));
        let (n2, sha2) = grabar_y_verificar(&d.join("sistema.img"), &disco, &mut |_| ()).unwrap();
        assert_eq!((n2, &sha2), (n, &sha));
        let mut malo = grabado.clone();
        malo[1_234_567] ^= 0xFF;
        std::fs::write(&disco, &malo).unwrap();
        assert!(verificar(std::fs::File::open(&disco).unwrap(), n, &sha).unwrap_err().to_string().contains('✗'));
        std::fs::write(&disco, vec![0u8; 1000]).unwrap();
        assert!(verificar(std::fs::File::open(&disco).unwrap(), n, &sha).is_err());
    }

    /// Un .xz cortado (descarga a medias) no se graba en silencio. Un byte cambiado DENTRO de un bloque lo
    /// caza el CRC64 de cada bloque (las imágenes oficiales lo llevan: medido, y lzma-rs lo verifica) y,
    /// del catálogo, `extract_sha256`; el compresor de lzma-rs escribe xz SIN suma, por eso aquí se corta.
    #[test]
    fn un_xz_cortado_se_dice() {
        let d = crate::CarpetaPrueba::nueva("gotv-equipos-xz-malo");
        let mut xz = Vec::new();
        lzma_rs::xz_compress(&mut std::io::Cursor::new((0..100_000u32).map(|i| (i * 31) as u8).collect::<Vec<_>>()), &mut xz).unwrap();
        xz.truncate(xz.len() / 2);
        std::fs::write(d.join("malo.img.xz"), &xz).unwrap();
        std::fs::write(d.join("disco.bin"), vec![0u8; 200_000]).unwrap();
        assert!(grabar_y_verificar(&d.join("malo.img.xz"), &d.join("disco.bin"), &mut |_| ()).is_err());
    }
}
