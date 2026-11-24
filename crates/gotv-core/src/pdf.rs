//! O2 · PDF como ARCHIVO (método de la skill `pdf` de Claude Code, llevado a Rust):
//! leer el texto por página, ver sus datos, extraer/unir/rotar páginas y convertir páginas a
//! imagen para MIRARLAS (la QA visual de Claude Code: «render → imágenes → mirar»).
//!
//! - texto: `pdf-extract` (MIT), con `lopdf` de respaldo si aquel no puede;
//! - páginas: `lopdf` (MIT) — unir como su ejemplo `merge.rs`, pero en el ORDEN de las páginas;
//! - imagen: una página sola → `sips` (viene con macOS) → PNG.
//!
//! Nunca se sobrescribe un PDF existente: lo que se escribe va a un destino nuevo.

use anyhow::{anyhow, bail, Context, Result};
use gotv_engine::{anyhow_tr, bail_tr};
use lopdf::{Document, Object, ObjectId};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Tope de texto devuelto (el resto se pide por páginas).
const TOPE_TEXTO: usize = 40_000;
/// Tope de páginas convertidas a imagen por llamada (cada una cuesta tokens al mirarla).
const TOPE_IMAGENES: usize = 10;

fn abrir(p: &Path) -> Result<Document> {
    if !p.exists() {
        bail!("{} no existe", p.display());
    }
    let mut d = Document::load(p).with_context(|| format!("no pude abrir {} como PDF", p.display()))?;
    if d.is_encrypted() {
        // muchos PDF van «cifrados» con contraseña de usuario vacía (sólo restringen permisos)
        d.decrypt("").map_err(|_| anyhow_tr!("{} está protegido con contraseña", "{} is password-protected", p.display()))?;
    }
    Ok(d)
}

/// «1-3,5», «todas», «» → números de página (1-based, sin repetir, en el orden dado).
pub fn paginas(spec: &str, total: u32) -> Result<Vec<u32>> {
    let s = spec.trim().to_lowercase();
    if s.is_empty() || s == "todas" || s == "all" || s == "*" {
        return Ok((1..=total).collect());
    }
    let mut v = vec![];
    for parte in s.split(',').map(str::trim).filter(|x| !x.is_empty()) {
        let (a, b) = match parte.split_once('-') {
            Some((a, b)) => (a.trim(), b.trim()),
            None => (parte, parte),
        };
        let a: u32 = a.parse().map_err(|_| anyhow_tr!("página «{parte}» no es un número (usa «1-3,5»)", "page «{parte}» is not a number (use «1-3,5»)"))?;
        let mut b: u32 = if b.is_empty() || b == "fin" || b == "end" { total } else { b.parse().map_err(|_| anyhow_tr!("página «{parte}» no es un número (usa «1-3,5»)", "page «{parte}» is not a number (use «1-3,5»)"))? };
        // un rango que se pasa del final llega hasta el final («1-5» en 4 páginas = 1-4)
        if parte.contains('-') && a <= total && b > total {
            b = total;
        }
        if a == 0 || b == 0 || a > total || b > total {
            bail_tr!("página {parte} fuera del documento (tiene {total})", "page {parte} is outside the document (it has {total})");
        }
        let rango: Vec<u32> = if a <= b { (a..=b).collect() } else { (b..=a).rev().collect() };
        for n in rango {
            if !v.contains(&n) {
                v.push(n);
            }
        }
    }
    if v.is_empty() {
        bail_tr!("ninguna página en «{spec}»", "no page in «{spec}»");
    }
    Ok(v)
}

fn texto_pdf(o: &Object) -> Option<String> {
    match o {
        Object::String(b, _) => {
            // UTF-16BE con BOM, o PDFDocEncoding (≈ latin-1)
            if b.starts_with(&[0xFE, 0xFF]) {
                let u: Vec<u16> = b[2..].chunks(2).filter(|c| c.len() == 2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect();
                Some(String::from_utf16_lossy(&u))
            } else {
                Some(b.iter().map(|&c| c as char).collect())
            }
        }
        _ => None,
    }
}

/// Los datos del PDF: páginas, tamaño, metadatos, formulario, cifrado.
pub fn info(p: &Path) -> Result<String> {
    let d = abrir(p)?;
    let pags = d.get_pages();
    let kb = std::fs::metadata(p).map(|m| m.len() / 1024).unwrap_or(0);
    let mut out = crate::tr!("{} · {} páginas · {kb} KB · PDF {}\n", "{} · {} pages · {kb} KB · PDF {}\n", p.file_name().unwrap_or_default().to_string_lossy(), pags.len(), d.version);
    if let Some((_, id)) = pags.iter().next() {
        // el MediaBox puede venir HEREDADO de un nodo padre del árbol de páginas
        let mut nodo = d.get_dictionary(*id).ok();
        let mut caja = None;
        for _ in 0..16 {
            let Some(n) = nodo else { break };
            if let Ok(m) = n.get(b"MediaBox").and_then(|m| d.dereference(m).map(|(_, o)| o)).and_then(Object::as_array) {
                caja = Some(m);
                break;
            }
            nodo = n.get(b"Parent").and_then(Object::as_reference).and_then(|r| d.get_dictionary(r)).ok();
        }
        if let Some(caja) = caja {
            let n: Vec<f32> = caja.iter().filter_map(|x| x.as_float().ok().or_else(|| x.as_i64().ok().map(|i| i as f32))).collect();
            if n.len() == 4 {
                let (w, h) = ((n[2] - n[0]).abs(), (n[3] - n[1]).abs());
                let mm = |pt: f32| (pt * 25.4 / 72.0).round();
                let nombre = match (mm(w.min(h)) as i32, mm(w.max(h)) as i32) {
                    (209..=211, 296..=298) => " (A4)",
                    (215..=217, 279..=280) => " (carta)",
                    (215..=217, 330..=356) => " (oficio)",
                    _ => "",
                };
                out.push_str(&crate::tr!("página 1: {} × {} mm{nombre}{}\n", "page 1: {} × {} mm{nombre}{}\n", mm(w), mm(h), if w > h { crate::i18n::t(", horizontal", ", landscape") } else { "" }));
            }
        }
    }
    if let Ok(Object::Reference(id)) = d.trailer.get(b"Info") {
        if let Ok(info) = d.get_dictionary(*id) {
            let t = crate::i18n::t;
            for (k, nombre) in [(&b"Title"[..], t("título", "title")), (b"Author", t("autor", "author")), (b"Subject", t("asunto", "subject")), (b"Creator", t("creado con", "created with")), (b"Producer", t("productor", "producer"))] {
                if let Some(v) = info.get(k).ok().and_then(texto_pdf).filter(|v| !v.trim().is_empty()) {
                    out.push_str(&format!("{nombre}: {}\n", v.trim()));
                }
            }
        }
    }
    let catalogo = d.catalog().ok();
    if let Some(form) = catalogo.and_then(|c| c.get(b"AcroForm").ok()) {
        let campos = match form {
            Object::Reference(id) => d.get_dictionary(*id).ok().and_then(|f| f.get(b"Fields").ok()).and_then(|f| f.as_array().ok()).map(Vec::len),
            Object::Dictionary(f) => f.get(b"Fields").ok().and_then(|f| f.as_array().ok()).map(Vec::len),
            _ => None,
        };
        out.push_str(&crate::tr!("formulario: sí ({} campos)\n", "form: yes ({} fields)\n", campos.unwrap_or(0)));
    }
    if d.encryption_state.is_some() || d.trailer.get(b"Encrypt").is_ok() {
        out.push_str(crate::i18n::t("cifrado: sí (sin contraseña de apertura)\n", "encrypted: yes (no open password)\n"));
    }
    Ok(out)
}

/// El texto de cada página (pdf-extract; si falla o entra en pánico, lopdf).
fn textos_por_pagina(p: &Path, total: u32) -> Vec<String> {
    let bytes = std::fs::read(p).unwrap_or_default();
    let r = std::panic::catch_unwind(|| pdf_extract::extract_text_from_mem_by_pages(&bytes));
    if let Ok(Ok(v)) = r {
        if v.len() == total as usize {
            return v;
        }
    }
    match abrir(p) {
        Ok(d) => (1..=total).map(|n| d.extract_text(&[n]).unwrap_or_default()).collect(),
        Err(_) => vec![String::new(); total as usize],
    }
}

fn limpiar(t: &str) -> String {
    // líneas con espacios de sobra y saltos múltiples (lo típico de un PDF)
    let mut out = String::new();
    let mut vacias = 0;
    for l in t.lines().map(|l| l.split_whitespace().collect::<Vec<_>>().join(" ")) {
        if l.is_empty() {
            vacias += 1;
            if vacias == 1 && !out.is_empty() {
                out.push('\n');
            }
            continue;
        }
        vacias = 0;
        out.push_str(&l);
        out.push('\n');
    }
    out.trim_end().to_string()
}

/// El texto, página por página (`paginas` = «1-3,5» o vacío para todas).
pub fn leer(p: &Path, spec: &str) -> Result<String> {
    let total = abrir(p)?.get_pages().len() as u32;
    let elegidas = paginas(spec, total)?;
    let textos = textos_por_pagina(p, total);
    let mut out = crate::tr!("{} · {total} páginas\n", "{} · {total} pages\n", p.file_name().unwrap_or_default().to_string_lossy());
    let mut sin_texto = vec![];
    for n in &elegidas {
        let t = limpiar(textos.get(*n as usize - 1).map(String::as_str).unwrap_or(""));
        if t.is_empty() {
            sin_texto.push(n.to_string());
        }
        out.push_str(&crate::tr!("\n<!-- Página {n} -->\n{t}\n", "\n<!-- Page {n} -->\n{t}\n"));
        if out.len() > TOPE_TEXTO {
            let corte = out.char_indices().map(|(i, _)| i).take_while(|i| *i <= TOPE_TEXTO).last().unwrap_or(0);
            out.truncate(corte);
            out.push_str(&crate::tr!("\n… (cortado en la página {n}: pide las siguientes con «paginas»)\n", "\n… (cut at page {n}: ask for the next ones with «paginas»)\n"));
            break;
        }
    }
    if !sin_texto.is_empty() {
        out.push_str(&crate::tr!(
            "\n⚠ sin texto en la(s) página(s) {}: puede ser un escaneo o una imagen — conviértela con accion «imagenes» y MÍRALA (view_image)\n",
            "\n⚠ no text on page(s) {}: it may be a scan or an image — convert it with accion «imagenes» and LOOK at it (view_image)\n",
            sin_texto.join(", ")
        ));
    }
    Ok(out)
}

/// Escribe el documento en `destino` (nuevo) de forma atómica.
fn guardar(d: &mut Document, destino: &Path) -> Result<()> {
    if destino.exists() {
        bail!("{} ya existe: elige otro nombre (no sobrescribo PDFs)", destino.display());
    }
    if destino.extension().map(|e| e.to_string_lossy().to_lowercase()) != Some("pdf".into()) {
        bail_tr!("el destino debe terminar en .pdf", "the destination must end in .pdf");
    }
    if let Some(dir) = destino.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = destino.with_extension(format!("pdf.tmp-{}", std::process::id()));
    let r = d.save(&tmp).map(|_| ()).map_err(|e| anyhow_tr!("no pude escribir el PDF: {e}", "could not write the PDF: {e}"));
    if let Err(e) = r {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    std::fs::rename(&tmp, destino)?;
    Ok(())
}

/// Un PDF nuevo con SÓLO esas páginas (en su orden original).
fn solo_paginas(mut d: Document, quedan: &[u32]) -> Document {
    let total = d.get_pages().len() as u32;
    let fuera: Vec<u32> = (1..=total).filter(|n| !quedan.contains(n)).collect();
    if !fuera.is_empty() {
        d.delete_pages(&fuera);
        d.prune_objects();
    }
    d
}

/// Extrae páginas a un PDF nuevo (dividir = extraer por partes).
pub fn extraer(p: &Path, spec: &str, destino: &Path) -> Result<String> {
    let d = abrir(p)?;
    let total = d.get_pages().len() as u32;
    let mut el = paginas(spec, total)?;
    let ordenadas = {
        let mut o = el.clone();
        o.sort_unstable();
        o
    };
    let aviso = if el != ordenadas { crate::i18n::t(" (van en su orden original; para reordenar, extrae por partes y únelas en el orden que quieras)", " (they keep their original order; to reorder, extract in parts and join them in the order you want)") } else { "" };
    el.sort_unstable();
    let mut nuevo = solo_paginas(d, &el);
    guardar(&mut nuevo, destino)?;
    Ok(crate::tr!("✓ {} páginas → {}{aviso}", "✓ {} pages → {}{aviso}", el.len(), destino.display()))
}

/// Une varios PDF, en el orden dado, en uno nuevo (el ejemplo `merge.rs` de lopdf, con las páginas
/// en SU orden: el ejemplo las ordenaba por número de objeto).
pub fn unir(archivos: &[PathBuf], destino: &Path) -> Result<String> {
    if archivos.len() < 2 {
        bail_tr!("para unir hacen falta al menos 2 PDF en «archivos»", "joining needs at least 2 PDFs in «archivos»");
    }
    let mut max_id = 1;
    let mut paginas_en_orden: Vec<(ObjectId, Object)> = vec![];
    let mut objetos: BTreeMap<ObjectId, Object> = BTreeMap::new();
    for a in archivos {
        let mut d = abrir(a)?;
        d.renumber_objects_with(max_id);
        max_id = d.max_id + 1;
        for (_, id) in d.get_pages() {
            paginas_en_orden.push((id, d.get_object(id)?.to_owned()));
        }
        objetos.extend(d.objects);
    }
    let mut doc = Document::with_version("1.5");
    let mut catalogo: Option<(ObjectId, Object)> = None;
    let mut raiz: Option<(ObjectId, Object)> = None;
    for (id, o) in objetos {
        match o.type_name().unwrap_or(b"") {
            b"Catalog" => {
                if catalogo.is_none() {
                    catalogo = Some((id, o));
                }
            }
            b"Pages" => {
                if let Ok(dic) = o.as_dict() {
                    let mut dic = dic.clone();
                    if let Some((_, ref viejo)) = raiz {
                        if let Ok(v) = viejo.as_dict() {
                            dic.extend(v);
                        }
                    }
                    raiz = Some((raiz.as_ref().map(|r| r.0).unwrap_or(id), Object::Dictionary(dic)));
                }
            }
            b"Page" | b"Outlines" | b"Outline" => {}
            _ => {
                doc.objects.insert(id, o);
            }
        }
    }
    let (raiz_id, raiz_o) = raiz.ok_or_else(|| anyhow_tr!("no encontré el árbol de páginas", "could not find the page tree"))?;
    let (cat_id, cat_o) = catalogo.ok_or_else(|| anyhow_tr!("no encontré el catálogo", "could not find the catalog"))?;
    for (id, o) in &paginas_en_orden {
        if let Ok(dic) = o.as_dict() {
            let mut dic = dic.clone();
            dic.set("Parent", raiz_id);
            doc.objects.insert(*id, Object::Dictionary(dic));
        }
    }
    let mut dic = raiz_o.as_dict()?.clone();
    dic.set("Count", paginas_en_orden.len() as u32);
    dic.set("Kids", paginas_en_orden.iter().map(|(id, _)| Object::Reference(*id)).collect::<Vec<_>>());
    doc.objects.insert(raiz_id, Object::Dictionary(dic));
    let mut cat = cat_o.as_dict()?.clone();
    cat.set("Pages", raiz_id);
    cat.remove(b"Outlines");
    // un formulario a medias de un solo archivo rompería el resto: se quita
    cat.remove(b"AcroForm");
    doc.objects.insert(cat_id, Object::Dictionary(cat));
    doc.trailer.set("Root", cat_id);
    doc.max_id = doc.objects.keys().map(|k| k.0).max().unwrap_or(0);
    doc.renumber_objects();
    doc.compress();
    let n = paginas_en_orden.len();
    guardar(&mut doc, destino)?;
    Ok(crate::tr!("✓ {} PDF unidos ({n} páginas) → {}", "✓ {} PDFs joined ({n} pages) → {}", archivos.len(), destino.display()))
}

/// Gira páginas (múltiplos de 90°) en un PDF nuevo.
pub fn rotar(p: &Path, spec: &str, grados: i64, destino: &Path) -> Result<String> {
    if grados % 90 != 0 {
        bail_tr!("el giro va en múltiplos de 90 (90, 180, 270, -90)", "rotation goes in multiples of 90 (90, 180, 270, -90)");
    }
    let mut d = abrir(p)?;
    let pags = d.get_pages();
    let el = paginas(spec, pags.len() as u32)?;
    for n in &el {
        let id = pags[n];
        let pg = d.get_object_mut(id).and_then(Object::as_dict_mut).map_err(|e| anyhow_tr!("página {n}: {e}", "page {n}: {e}"))?;
        let actual = pg.get(b"Rotate").and_then(Object::as_i64).unwrap_or(0);
        pg.set("Rotate", (actual + grados).rem_euclid(360));
    }
    guardar(&mut d, destino)?;
    Ok(crate::tr!("✓ {} página(s) giradas {grados}° → {}", "✓ {} page(s) rotated {grados}° → {}", el.len(), destino.display()))
}

/// Carpeta temporal propia que se borra al soltarse (sólo la que creó ella misma).
struct Temporal(PathBuf);

impl Temporal {
    fn nueva() -> Result<Self> {
        let n = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        let d = std::env::temp_dir().join(format!("gotv-pdf-{}-{n}", std::process::id()));
        std::fs::create_dir(&d)?;
        Ok(Self(d))
    }
}

impl Drop for Temporal {
    fn drop(&mut self) {
        if self.0.file_name().is_some_and(|n| n.to_string_lossy().starts_with("gotv-pdf-")) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}

/// Convierte páginas a PNG (≤ `lado` px) en `carpeta`, para MIRARLAS con view_image.
pub fn imagenes(p: &Path, spec: &str, carpeta: &Path, lado: u32) -> Result<Vec<PathBuf>> {
    let d = abrir(p)?;
    let total = d.get_pages().len() as u32;
    let el = paginas(spec, total)?;
    if el.len() > TOPE_IMAGENES {
        bail_tr!("{} páginas son muchas para mirar de una vez: pide hasta {TOPE_IMAGENES} (p. ej. «1-{TOPE_IMAGENES}»)", "{} pages are too many to look at once: ask for up to {TOPE_IMAGENES} (e.g. «1-{TOPE_IMAGENES}»)", el.len());
    }
    std::fs::create_dir_all(carpeta)?;
    let tmp = Temporal::nueva()?;
    let base = p.file_stem().unwrap_or_default().to_string_lossy().to_string();
    let mut hechas = vec![];
    for n in el {
        // sips sólo dibuja la PRIMERA página: cada una va sola a un PDF de una página
        let una = tmp.0.join(format!("p{n}.pdf"));
        let mut sola = solo_paginas(d.clone(), &[n]);
        sola.save(&una).map_err(|e| anyhow_tr!("página {n}: {e}", "page {n}: {e}"))?;
        let png = carpeta.join(format!("{base}-p{n}.png"));
        let out = std::process::Command::new("sips")
            .args(["-s", "format", "png", "-Z", &lado.to_string()])
            .arg(&una)
            .arg("--out")
            .arg(&png)
            .output()
            .map_err(|e| anyhow!("sips: {e}"))?;
        if !out.status.success() || !png.exists() {
            bail_tr!("no pude dibujar la página {n}: {}", "could not draw page {n}: {}", String::from_utf8_lossy(&out.stderr).trim());
        }
        hechas.push(png);
    }
    Ok(hechas)
}

/// Un PDF sencillo de texto (A4, Helvetica 12 pt, una línea por elemento; tildes en WinAnsi).
/// Lo usa el banco de oficina para sembrar documentos; no es para documentos con diseño (para
/// eso: Word o PowerPoint → accion «pdf»).
pub fn de_lineas(p: &Path, paginas: &[Vec<String>]) -> Result<()> {
    use lopdf::content::{Content, Operation};
    use lopdf::{dictionary, Stream};
    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();
    let font_id = doc.add_object(dictionary! {"Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica", "Encoding" => "WinAnsiEncoding"});
    let res = doc.add_object(dictionary! {"Font" => dictionary! {"F1" => font_id}});
    let mut kids = vec![];
    for lineas in paginas {
        let mut ops = vec![Operation::new("BT", vec![]), Operation::new("Tf", vec!["F1".into(), 12.into()]), Operation::new("TL", vec![18.into()]), Operation::new("Td", vec![72.into(), 770.into()])];
        for l in lineas {
            // WinAnsi ≈ latin-1 para el español (á é í ó ú ñ ü ¿ ¡)
            let bytes: Vec<u8> = l.chars().map(|c| if (c as u32) < 256 { c as u8 } else { b'?' }).collect();
            ops.push(Operation::new("Tj", vec![Object::String(bytes, lopdf::StringFormat::Literal)]));
            ops.push(Operation::new("T*", vec![]));
        }
        ops.push(Operation::new("ET", vec![]));
        let cid = doc.add_object(Stream::new(dictionary! {}, Content { operations: ops }.encode()?));
        let pid = doc.add_object(dictionary! {"Type" => "Page", "Parent" => pages_id, "Contents" => cid, "Resources" => res, "MediaBox" => vec![0.into(), 0.into(), 595.into(), 842.into()]});
        kids.push(Object::Reference(pid));
    }
    let n = kids.len() as u32;
    doc.objects.insert(pages_id, Object::Dictionary(dictionary! {"Type" => "Pages", "Kids" => kids, "Count" => n}));
    let cat = doc.add_object(dictionary! {"Type" => "Catalog", "Pages" => pages_id});
    doc.trailer.set("Root", cat);
    doc.save(p).map_err(|e| anyhow_tr!("no pude escribir el PDF: {e}", "could not write the PDF: {e}"))?;
    Ok(())
}

#[cfg(test)]
pub(crate) mod pruebas {
    use super::*;
    use lopdf::content::{Content, Operation};
    use lopdf::{dictionary, Stream};

    /// Un PDF de `n` páginas con «Hola página k» en cada una (A4).
    pub(crate) fn pdf_de_prueba(p: &Path, n: u32, marca: &str) {
        let mut doc = Document::with_version("1.5");
        let pages_id = doc.new_object_id();
        let font_id = doc.add_object(dictionary! {"Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica"});
        let res = doc.add_object(dictionary! {"Font" => dictionary! {"F1" => font_id}});
        let mut kids = vec![];
        for k in 1..=n {
            let c = Content {
                operations: vec![
                    Operation::new("BT", vec![]),
                    Operation::new("Tf", vec!["F1".into(), 24.into()]),
                    Operation::new("Td", vec![72.into(), 700.into()]),
                    Operation::new("Tj", vec![Object::string_literal(format!("{marca} pagina {k}"))]),
                    Operation::new("ET", vec![]),
                ],
            };
            let cid = doc.add_object(Stream::new(dictionary! {}, c.encode().unwrap()));
            let pid = doc.add_object(dictionary! {"Type" => "Page", "Parent" => pages_id, "Contents" => cid, "Resources" => res, "MediaBox" => vec![0.into(), 0.into(), 595.into(), 842.into()]});
            kids.push(Object::Reference(pid));
        }
        doc.objects.insert(pages_id, Object::Dictionary(dictionary! {"Type" => "Pages", "Kids" => kids, "Count" => n}));
        let cat = doc.add_object(dictionary! {"Type" => "Catalog", "Pages" => pages_id});
        doc.trailer.set("Root", cat);
        let info = doc.add_object(dictionary! {"Title" => Object::string_literal("Prueba GOTV")});
        doc.trailer.set("Info", info);
        doc.save(p).unwrap();
    }

    #[test]
    fn rangos_de_paginas() {
        assert_eq!(paginas("", 4).unwrap(), vec![1, 2, 3, 4]);
        assert_eq!(paginas("todas", 2).unwrap(), vec![1, 2]);
        assert_eq!(paginas("1-3, 5", 6).unwrap(), vec![1, 2, 3, 5]);
        assert_eq!(paginas("3-1", 6).unwrap(), vec![3, 2, 1]);
        assert_eq!(paginas("2-", 4).unwrap(), vec![2, 3, 4]);
        assert_eq!(paginas("2,2,1", 4).unwrap(), vec![2, 1]);
        assert_eq!(paginas("1-5", 4).unwrap(), vec![1, 2, 3, 4], "el rango se recorta al final");
        assert!(paginas("0", 3).is_err() && paginas("9", 3).is_err() && paginas("a", 3).is_err());
    }

    #[test]
    fn info_leer_extraer_unir_rotar() {
        let d = crate::CarpetaPrueba::nueva("gotv-pdf");
        let a = d.join("a.pdf");
        let b = d.join("b.pdf");
        pdf_de_prueba(&a, 3, "Alfa");
        pdf_de_prueba(&b, 2, "Beta");

        let i = info(&a).unwrap();
        assert!(i.contains("3 páginas") && i.contains("(A4)") && i.contains("título: Prueba GOTV"), "{i}");

        let t = leer(&a, "").unwrap();
        assert!(t.contains("<!-- Página 2 -->\nAlfa pagina 2"), "{t}");
        let t = leer(&a, "3").unwrap();
        assert!(t.contains("Alfa pagina 3") && !t.contains("Alfa pagina 1"), "{t}");

        let e = d.join("extraida.pdf");
        extraer(&a, "1,3", &e).unwrap();
        let t = leer(&e, "").unwrap();
        assert!(t.contains("2 páginas") && t.contains("Alfa pagina 1") && t.contains("Alfa pagina 3") && !t.contains("Alfa pagina 2"), "{t}");
        assert!(extraer(&a, "1", &e).is_err(), "no sobrescribe");

        let u = d.join("unido.pdf");
        unir(&[b.clone(), a.clone()], &u).unwrap();
        let t = leer(&u, "").unwrap();
        assert!(t.contains("5 páginas"), "{t}");
        let pos = |s: &str| t.find(s).unwrap();
        assert!(pos("Beta pagina 1") < pos("Beta pagina 2") && pos("Beta pagina 2") < pos("Alfa pagina 1") && pos("Alfa pagina 1") < pos("Alfa pagina 3"), "orden: {t}");

        let r = d.join("rotado.pdf");
        rotar(&a, "2", 90, &r).unwrap();
        let doc = abrir(&r).unwrap();
        let rot: Vec<i64> = doc.get_pages().values().map(|id| doc.get_dictionary(*id).unwrap().get(b"Rotate").and_then(Object::as_i64).unwrap_or(0)).collect();
        assert_eq!(rot, vec![0, 90, 0]);
        assert!(rotar(&a, "1", 45, &d.join("x.pdf")).is_err());
    }

    #[test]
    fn de_lineas_con_tildes_se_lee_igual() {
        let d = crate::CarpetaPrueba::nueva("gotv-pdf-lineas");
        let a = d.join("pedidos.pdf");
        de_lineas(&a, &[vec!["Pedidos de noviembre".into(), "Café de Nariño   12   8.000".into()], vec!["Página dos: año, señor".into()]]).unwrap();
        let t = leer(&a, "").unwrap();
        assert!(t.contains("2 páginas") && t.contains("Café de Nariño") && t.contains("año, señor"), "{t}");
    }

    #[test]
    fn paginas_a_imagen() {
        let d = crate::CarpetaPrueba::nueva("gotv-pdf-img");
        let a = d.join("a.pdf");
        pdf_de_prueba(&a, 3, "Alfa");
        let v = imagenes(&a, "2-3", &d.join("img"), 600).unwrap();
        assert_eq!(v.len(), 2);
        assert!(v[0].ends_with("a-p2.png") && v[0].exists() && v[1].exists());
        assert!(imagenes(&a, "", &d.join("img"), 600).is_ok());
    }
}

#[cfg(test)]
mod a_mano {
    /// Lee e informa un PDF cualquiera: `GOTV_PDF=/ruta/x.pdf cargo test -p gotv-core -- --ignored leer_pdf --nocapture`
    #[test]
    #[ignore]
    fn leer_pdf() {
        let Ok(p) = std::env::var("GOTV_PDF") else { return };
        let p = std::path::Path::new(&p);
        let pags = std::env::var("GOTV_PAGINAS").unwrap_or_default();
        println!("{}\n{}", super::info(p).unwrap(), super::leer(p, &pags).unwrap());
    }
}
