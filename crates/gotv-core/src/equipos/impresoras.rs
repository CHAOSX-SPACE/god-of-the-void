//! F22 paso 9 · Impresoras: listar, estado, cola, imprimir un PDF o un texto, cancelar.
//!
//! Desde Rust, llamando a la biblioteca del sistema de impresión del Mac (libcups, la misma que usan `lp` y
//! `lpstat`; L1.3: una biblioteca en C enlazada vale), sin programas intermedios. El estado viene de los
//! atributos IPP del destino (`printer-state`, `printer-is-accepting-jobs`, `marker-levels`…), que no
//! dependen del idioma (medido: `lpstat -p` sale en español aunque se pida `LANG=C`).
//! Imprimir pregunta SIEMPRE mostrando impresora, archivo y número de páginas.
use anyhow::{anyhow, bail, Result};
use std::collections::HashMap;
use std::ffi::{c_char, c_int, CStr, CString};
use std::path::Path;

#[repr(C)]
struct CupsOption {
    name: *mut c_char,
    value: *mut c_char,
}

#[repr(C)]
struct CupsDest {
    name: *mut c_char,
    instance: *mut c_char,
    is_default: c_int,
    num_options: c_int,
    options: *mut CupsOption,
}

#[repr(C)]
struct CupsJob {
    id: c_int,
    dest: *mut c_char,
    title: *mut c_char,
    user: *mut c_char,
    format: *mut c_char,
    state: c_int,
    size: c_int,
    priority: c_int,
    completed_time: libc::time_t,
    creation_time: libc::time_t,
    processing_time: libc::time_t,
}

#[link(name = "cups")]
extern "C" {
    fn cupsGetDests(dests: *mut *mut CupsDest) -> c_int;
    fn cupsFreeDests(n: c_int, dests: *mut CupsDest);
    fn cupsGetJobs(jobs: *mut *mut CupsJob, name: *const c_char, myjobs: c_int, whichjobs: c_int) -> c_int;
    fn cupsFreeJobs(n: c_int, jobs: *mut CupsJob);
    fn cupsPrintFile(name: *const c_char, filename: *const c_char, title: *const c_char, num_options: c_int, options: *mut CupsOption) -> c_int;
    fn cupsAddOption(name: *const c_char, value: *const c_char, num_options: c_int, options: *mut *mut CupsOption) -> c_int;
    fn cupsFreeOptions(n: c_int, options: *mut CupsOption);
    fn cupsCancelJob(name: *const c_char, job_id: c_int) -> c_int;
    fn cupsLastErrorString() -> *const c_char;
}

fn texto(p: *const c_char) -> String {
    if p.is_null() {
        String::new()
    } else {
        unsafe { CStr::from_ptr(p) }.to_string_lossy().to_string()
    }
}

fn ultimo_error() -> String {
    texto(unsafe { cupsLastErrorString() })
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Impresora {
    pub nombre: String,
    pub descripcion: String,
    pub modelo: String,
    pub estado: u8,
    pub acepta: bool,
    pub razones: String,
    pub toner: Option<String>,
    pub por_omision: bool,
}

/// Arma una impresora con sus atributos (los de libcups, o a mano en las pruebas).
pub fn de_atributos(nombre: &str, a: &HashMap<String, String>, por_omision: bool) -> Impresora {
    let toner = a.get("marker-levels").filter(|v| !v.is_empty()).map(|niveles| {
        let nombres: Vec<&str> = a.get("marker-names").map(|n| n.split(',').collect()).unwrap_or_default();
        niveles.split(',').enumerate().map(|(i, n)| format!("{} {n} %", nombres.get(i).copied().unwrap_or("")).trim().to_string()).collect::<Vec<_>>().join(", ")
    });
    Impresora {
        nombre: nombre.into(),
        descripcion: a.get("printer-info").cloned().unwrap_or_default(),
        modelo: a.get("printer-make-and-model").cloned().unwrap_or_default(),
        estado: a.get("printer-state").and_then(|s| s.parse().ok()).unwrap_or(0),
        acepta: a.get("printer-is-accepting-jobs").map(|v| v == "true").unwrap_or(false),
        razones: a.get("printer-state-reasons").cloned().filter(|r| r != "none").unwrap_or_default(),
        toner,
        por_omision,
    }
}

impl Impresora {
    pub fn linea(&self) -> String {
        let estado = match self.estado {
            3 => crate::i18n::t("lista", "idle"),
            4 => crate::i18n::t("imprimiendo", "printing"),
            5 => crate::i18n::t("detenida", "stopped"),
            _ => "?",
        };
        let mut l = format!("{} · {} · {estado}", self.nombre, self.descripcion);
        if self.por_omision {
            l.push_str(crate::i18n::t(" · por omisión", " · default"));
        }
        if !self.acepta {
            l.push_str(crate::i18n::t(" · NO acepta trabajos", " · NOT accepting jobs"));
        }
        if !self.razones.is_empty() {
            l.push_str(&format!(" · {}", self.razones));
        }
        if let Some(t) = &self.toner {
            l.push_str(&crate::tr!(" · tinta/tóner: {t}", " · ink/toner: {t}"));
        }
        l
    }
}

/// Las impresoras del Mac, de libcups.
pub fn listar() -> Result<Vec<Impresora>> {
    let mut out = Vec::new();
    unsafe {
        let mut d: *mut CupsDest = std::ptr::null_mut();
        let n = cupsGetDests(&mut d);
        for i in 0..n.max(0) as usize {
            let dest = &*d.add(i);
            if !dest.instance.is_null() {
                continue; // las instancias son la misma impresora con otras opciones
            }
            let mut a = HashMap::new();
            for k in 0..dest.num_options.max(0) as usize {
                let o = &*dest.options.add(k);
                a.insert(texto(o.name), texto(o.value));
            }
            out.push(de_atributos(&texto(dest.name), &a, dest.is_default != 0));
        }
        if !d.is_null() {
            cupsFreeDests(n, d);
        }
    }
    Ok(out)
}

/// La cola: (trabajo «Impresora-12», tamaño en KB). Sin el usuario que lo mandó (dato personal).
pub fn cola() -> Result<Vec<(String, i32)>> {
    let mut out = Vec::new();
    unsafe {
        let mut j: *mut CupsJob = std::ptr::null_mut();
        let n = cupsGetJobs(&mut j, std::ptr::null(), 0, 0 /* CUPS_WHICHJOBS_ACTIVE */);
        for i in 0..n.max(0) as usize {
            let t = &*j.add(i);
            out.push((format!("{}-{}", texto(t.dest), t.id), t.size));
        }
        if !j.is_null() {
            cupsFreeJobs(n, j);
        }
    }
    Ok(out)
}

/// Cuántas páginas: un PDF se cuenta de verdad; un texto, a 60 líneas por hoja; una imagen, una.
pub fn paginas(archivo: &Path) -> Option<usize> {
    let ext = archivo.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "pdf" => lopdf::Document::load(archivo).ok().map(|d| d.get_pages().len()),
        "txt" | "md" | "log" | "csv" | "" => std::fs::read_to_string(archivo).ok().map(|t| t.lines().count().max(1).div_ceil(60)),
        "png" | "jpg" | "jpeg" | "heic" | "tif" | "tiff" | "gif" => Some(1),
        _ => None,
    }
}

/// Parte un trabajo «Impresora-12» en (impresora, número).
pub fn trabajo(id: &str) -> Option<(String, i32)> {
    let (p, n) = id.rsplit_once('-')?;
    let ok = !p.is_empty() && p.chars().all(|c| c.is_ascii_alphanumeric() || "_-.".contains(c));
    ok.then(|| n.parse().ok().map(|n| (p.to_string(), n))).flatten()
}

pub fn imprimir(impresora: &str, archivo: &Path, copias: u32) -> Result<String> {
    let lista = listar()?;
    let p = lista.iter().find(|p| p.nombre == impresora).ok_or_else(|| anyhow!("{}", crate::tr!("no hay una impresora «{impresora}» (hay: {})", "there is no printer «{impresora}» (there are: {})", lista.iter().map(|p| p.nombre.as_str()).collect::<Vec<_>>().join(", "))))?;
    if !p.acepta {
        bail!("{}", crate::tr!("«{impresora}» no acepta trabajos ahora ({})", "«{impresora}» is not accepting jobs now ({})", p.razones));
    }
    let nombre = CString::new(impresora)?;
    let ruta = CString::new(archivo.display().to_string())?;
    let titulo = CString::new(archivo.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default())?;
    let id = unsafe {
        let mut op: *mut CupsOption = std::ptr::null_mut();
        let k = CString::new("copies")?;
        let v = CString::new(copias.clamp(1, 50).to_string())?;
        let n = cupsAddOption(k.as_ptr(), v.as_ptr(), 0, &mut op);
        let id = cupsPrintFile(nombre.as_ptr(), ruta.as_ptr(), titulo.as_ptr(), n, op);
        cupsFreeOptions(n, op);
        id
    };
    if id <= 0 {
        bail!("{}", crate::tr!("CUPS no aceptó el trabajo: {}", "CUPS did not accept the job: {}", ultimo_error()));
    }
    Ok(crate::tr!("✓ enviado a {impresora}: trabajo {impresora}-{id} (se cancela con action=cancelar trabajo={impresora}-{id})", "✓ sent to {impresora}: job {impresora}-{id} (cancel with action=cancelar trabajo={impresora}-{id})"))
}

pub fn cancelar(id: &str) -> Result<String> {
    let (p, n) = trabajo(id).ok_or_else(|| anyhow!("{}", crate::tr!("«{id}» no es un trabajo de la cola (mira action=cola)", "«{id}» is not a queue job (see action=cola)")))?;
    let c = CString::new(p)?;
    if unsafe { cupsCancelJob(c.as_ptr(), n) } == 0 {
        bail!("{}", crate::tr!("CUPS no lo canceló: {}", "CUPS did not cancel it: {}", ultimo_error()));
    }
    Ok(crate::tr!("✓ cancelado {id}", "✓ cancelled {id}"))
}

/// Una página de prueba en texto (la escribe GOTV en `.gotv/equipos/`).
pub fn pagina_de_prueba(root: &Path) -> Result<std::path::PathBuf> {
    let d = root.join(".gotv/equipos");
    std::fs::create_dir_all(&d)?;
    let p = d.join("pagina-de-prueba.txt");
    std::fs::write(&p, crate::i18n::t("GOTV · página de prueba\n\nSi lees esto, la impresora funciona.\n", "GOTV · test page\n\nIf you can read this, the printer works.\n"))?;
    Ok(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// La impresora REAL de esta Mac, leída con libcups (sólo lee: no imprime).
    #[test]
    fn la_impresora_de_esta_mac_por_libcups() {
        let v = listar().unwrap();
        if v.is_empty() {
            eprintln!("sin impresoras: no aplica");
            return;
        }
        let p = v.iter().find(|p| p.por_omision).unwrap_or(&v[0]);
        assert!(matches!(p.estado, 3..=5), "{p:?}");
        assert!(!p.descripcion.is_empty() || !p.modelo.is_empty(), "{p:?}");
        assert!(cola().is_ok());
    }

    /// Los atributos medidos el 2026-11-16 en la HP del Portador (identificadores tapados).
    #[test]
    fn el_estado_sale_de_los_atributos() {
        let a: HashMap<String, String> = [
            ("printer-info", "HP LaserJet M15w (000000)"),
            ("printer-make-and-model", "HP LaserJet M14-M17-AirPrint"),
            ("printer-state", "3"),
            ("printer-is-accepting-jobs", "true"),
            ("printer-state-reasons", "toner-low-warning"),
            ("marker-levels", "0"),
            ("marker-names", "Black Cartridge HP CF248A"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
        let p = de_atributos("HP_X", &a, true);
        assert_eq!(p.estado, 3);
        assert_eq!(p.toner.as_deref(), Some("Black Cartridge HP CF248A 0 %"));
        assert!(p.linea().contains("toner-low-warning"));
    }

    #[test]
    fn paginas_y_trabajos() {
        let d = crate::CarpetaPrueba::nueva("gotv-imprimir");
        std::fs::write(d.join("nota.txt"), "linea\n".repeat(130)).unwrap();
        assert_eq!(paginas(&d.join("nota.txt")), Some(3));
        let mut doc = lopdf::Document::with_version("1.5");
        let paginas_id = doc.new_object_id();
        let mut hijos = Vec::new();
        for _ in 0..2 {
            let p = doc.add_object(lopdf::dictionary! {"Type" => "Page", "Parent" => paginas_id, "MediaBox" => vec![0.into(), 0.into(), 595.into(), 842.into()]});
            hijos.push(lopdf::Object::Reference(p));
        }
        doc.objects.insert(paginas_id, lopdf::Object::Dictionary(lopdf::dictionary! {"Type" => "Pages", "Kids" => hijos, "Count" => 2}));
        let cat = doc.add_object(lopdf::dictionary! {"Type" => "Catalog", "Pages" => paginas_id});
        doc.trailer.set("Root", cat);
        doc.save(d.join("dos.pdf")).unwrap();
        assert_eq!(paginas(&d.join("dos.pdf")), Some(2));
        assert_eq!(paginas(&d.join("x.docx")), None);
        assert_eq!(trabajo("HP_X-12"), Some(("HP_X".into(), 12)));
        assert!(trabajo("HP_X-12; rm").is_none() && trabajo("-a").is_none());
        assert!(pagina_de_prueba(&d).unwrap().is_file());
    }
}
