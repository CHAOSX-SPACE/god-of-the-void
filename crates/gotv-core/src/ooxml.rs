//! O2 · La base común de Word y PowerPoint como ARCHIVOS (lo que Claude Code reparte entre
//! `office/helpers` y `office/validators`, Apache-2.0; ver `ESTUDIO-DOCUMENTOS.md` §2.2):
//!   - abrir un paquete con SEGURIDAD: sin enlaces simbólicos ni rutas que salgan del paquete;
//!   - guardarlo como Office lo quiere: `[Content_Types].xml` PRIMERO y SIN comprimir, el resto
//!     comprimido, sin entradas de carpeta, a un temporal y reemplazo atómico;
//!   - un árbol XML que conserva atributos y texto TAL CUAL (sin reformatear: «do NOT reformat or
//!     pretty-print»): lo que no se toca sale byte a byte igual;
//!   - el validador del paquete: cada parte con su tipo, cada relación con su destino, XML bien formado.

use anyhow::{anyhow, bail, Result};
use gotv_engine::{anyhow_tr, bail_tr};
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

/// Un paquete OOXML en memoria: nombre de la parte → bytes (en el orden del zip original).
pub struct Paquete {
    pub partes: BTreeMap<String, Vec<u8>>,
}

impl Paquete {
    pub fn abrir(path: &Path) -> Result<Paquete> {
        let f = std::fs::File::open(path).map_err(|e| anyhow!("no pude abrir {}: {e}", path.display()))?;
        let mut z = zip::ZipArchive::new(f).map_err(|e| anyhow_tr!("{} no es un documento de Office válido (no es un zip): {e}", "{} is not a valid Office document (not a zip): {e}", path.display()))?;
        let mut partes = BTreeMap::new();
        for i in 0..z.len() {
            let mut e = z.by_index(i)?;
            if e.is_dir() {
                continue;
            }
            let nombre = e.name()?.to_string();
            if e.is_symlink() {
                bail_tr!("el documento trae un enlace simbólico ({nombre}): no lo abro (puede apuntar fuera)", "the document has a symbolic link ({nombre}): I won't open it (it can point outside)");
            }
            if nombre.starts_with('/') || nombre.split('/').any(|s| s == "..") || nombre.contains('\\') {
                bail_tr!("el documento trae una ruta insegura ({nombre}): no lo abro", "the document has an unsafe path ({nombre}): I won't open it");
            }
            let mut b = Vec::new();
            e.read_to_end(&mut b)?;
            partes.insert(nombre, b);
        }
        if !partes.contains_key("[Content_Types].xml") {
            bail!("{} no tiene [Content_Types].xml: no es un documento de Office", path.display());
        }
        Ok(Paquete { partes })
    }

    pub fn texto(&self, parte: &str) -> Option<String> {
        self.partes.get(parte).map(|b| String::from_utf8_lossy(b).into_owned())
    }

    pub fn poner(&mut self, parte: &str, contenido: impl Into<Vec<u8>>) {
        self.partes.insert(parte.to_string(), contenido.into());
    }

    /// Guarda: `[Content_Types].xml` primero y sin comprimir, sin carpetas, a un temporal y luego
    /// reemplazo atómico (si algo falla a medias, el archivo de antes queda intacto).
    pub fn guardar(&self, path: &Path) -> Result<()> {
        use zip::write::SimpleFileOptions;
        use zip::CompressionMethod;
        let dir = path.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new("."));
        std::fs::create_dir_all(dir)?;
        let tmp = dir.join(format!(".{}.gotv-tmp", path.file_name().unwrap_or_default().to_string_lossy()));
        {
            let f = std::fs::File::create(&tmp)?;
            let mut w = zip::ZipWriter::new(f);
            let guardado = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
            let comprimido = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
            if let Some(ct) = self.partes.get("[Content_Types].xml") {
                w.start_file("[Content_Types].xml", guardado)?;
                w.write_all(ct)?;
            }
            for (n, b) in &self.partes {
                if n == "[Content_Types].xml" {
                    continue;
                }
                w.start_file(n.as_str(), comprimido)?;
                w.write_all(b)?;
            }
            w.finish()?;
        }
        std::fs::rename(&tmp, path).map_err(|e| {
            let _ = std::fs::remove_file(&tmp);
            anyhow!("no pude reemplazar {}: {e}", path.display())
        })
    }
}

/// `x.docx` → `x.respaldo-<segundos>.docx` (al lado), copiando el original.
pub fn respaldar(p: &Path) -> Result<PathBuf> {
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let stem = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let ext = p.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    let r = p.with_file_name(format!("{stem}.respaldo-{t}{ext}"));
    std::fs::copy(p, &r)?;
    Ok(r)
}

// ── un árbol XML sin pérdidas ─────────────────────────────────────────────────────────────────

/// Un nodo: elemento (nombre y atributos CRUDOS, como venían) o cualquier otra cosa tal cual
/// (texto escapado, comentarios, CDATA, la declaración).
#[derive(Clone, Debug, PartialEq)]
pub enum Nodo {
    Elem(Elem),
    Crudo(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Elem {
    pub nombre: String,
    /// (nombre, valor CRUDO: escapado como en el archivo)
    pub attrs: Vec<(String, String)>,
    pub hijos: Vec<Nodo>,
    /// `<a/>` en el original (si sigue sin hijos se escribe igual)
    pub vacio: bool,
}

impl Elem {
    pub fn nuevo(nombre: &str) -> Elem {
        Elem { nombre: nombre.into(), attrs: vec![], hijos: vec![], vacio: true }
    }
    /// Nombre sin prefijo (`w:r` → `r`).
    pub fn local(&self) -> &str {
        self.nombre.rsplit(':').next().unwrap_or(&self.nombre)
    }
    pub fn attr(&self, k: &str) -> Option<String> {
        self.attrs.iter().find(|(n, _)| n == k).map(|(_, v)| desescapar(v))
    }
    pub fn poner_attr(&mut self, k: &str, v: &str) {
        let v = escapar(v, true);
        match self.attrs.iter_mut().find(|(n, _)| n == k) {
            Some(a) => a.1 = v,
            None => self.attrs.push((k.into(), v)),
        }
    }
    pub fn quitar_attr(&mut self, k: &str) {
        self.attrs.retain(|(n, _)| n != k);
    }
    pub fn elems(&self) -> impl Iterator<Item = &Elem> {
        self.hijos.iter().filter_map(|n| if let Nodo::Elem(e) = n { Some(e) } else { None })
    }
    pub fn hijo(&self, local: &str) -> Option<&Elem> {
        self.elems().find(|e| e.local() == local)
    }
    /// El texto (desescapado) de los nodos de texto directos.
    pub fn texto(&self) -> String {
        self.hijos.iter().filter_map(|n| if let Nodo::Crudo(s) = n { (!s.starts_with('<')).then(|| desescapar(s)) } else { None }).collect()
    }
    pub fn poner_texto(&mut self, t: &str) {
        self.hijos.retain(|n| matches!(n, Nodo::Elem(_)) || matches!(n, Nodo::Crudo(s) if s.starts_with("<!--")));
        self.hijos.insert(0, Nodo::Crudo(escapar(t, false)));
        self.vacio = false;
    }
    /// Todos los descendientes (en orden de documento) con ese nombre local.
    pub fn buscar<'a>(&'a self, local: &str, out: &mut Vec<&'a Elem>) {
        for e in self.elems() {
            if e.local() == local {
                out.push(e);
            }
            e.buscar(local, out);
        }
    }
    pub fn escribir(&self, s: &mut String) {
        s.push('<');
        s.push_str(&self.nombre);
        for (k, v) in &self.attrs {
            s.push_str(&format!(" {k}=\"{v}\""));
        }
        if self.hijos.is_empty() && self.vacio {
            s.push_str("/>");
            return;
        }
        s.push('>');
        for h in &self.hijos {
            match h {
                Nodo::Elem(e) => e.escribir(s),
                Nodo::Crudo(c) => s.push_str(c),
            }
        }
        s.push_str(&format!("</{}>", self.nombre));
    }
}

/// Un documento XML: lo de antes de la raíz (declaración, comentarios) y la raíz.
pub struct Doc {
    pub prologo: String,
    pub raiz: Elem,
}

impl Doc {
    pub fn leer(xml: &str) -> Result<Doc> {
        let b = xml.as_bytes();
        let mut i = 0;
        let mut prologo = String::new();
        // prólogo: declaración, comentarios, espacios
        loop {
            while i < b.len() && (b[i] as char).is_whitespace() {
                prologo.push(b[i] as char);
                i += 1;
            }
            if xml[i..].starts_with("<?") || xml[i..].starts_with("<!--") {
                let fin = if xml[i..].starts_with("<?") { "?>" } else { "-->" };
                let j = xml[i..].find(fin).ok_or_else(|| anyhow_tr!("XML roto: «{fin}» sin cerrar", "broken XML: «{fin}» not closed"))? + i + fin.len();
                prologo.push_str(&xml[i..j]);
                i = j;
            } else {
                break;
            }
        }
        let (raiz, j) = elemento(xml, i)?;
        if !xml[j..].trim().is_empty() && !xml[j..].trim_start().starts_with("<!--") {
            bail_tr!("XML roto: hay contenido después de la raíz", "broken XML: there is content after the root");
        }
        Ok(Doc { prologo, raiz })
    }
    pub fn escribir(&self) -> String {
        let mut s = self.prologo.clone();
        self.raiz.escribir(&mut s);
        s
    }
}

/// Lee un elemento que empieza en `i` (en `<`); devuelve el elemento y dónde termina.
fn elemento(x: &str, i: usize) -> Result<(Elem, usize)> {
    let b = x.as_bytes();
    if b.get(i) != Some(&b'<') {
        bail_tr!("XML roto: se esperaba un elemento en el byte {i}", "broken XML: an element was expected at byte {i}");
    }
    let fin_tag = buscar_fin_tag(x, i)?;
    let dentro = &x[i + 1..fin_tag];
    let vacio = dentro.ends_with('/');
    let dentro = dentro.trim_end_matches('/');
    let nombre_fin = dentro.find(|c: char| c.is_whitespace()).unwrap_or(dentro.len());
    let nombre = dentro[..nombre_fin].to_string();
    if nombre.is_empty() {
        bail_tr!("XML roto: etiqueta sin nombre en el byte {i}", "broken XML: tag without a name at byte {i}");
    }
    let attrs = atributos(&dentro[nombre_fin..])?;
    let mut e = Elem { nombre, attrs, hijos: vec![], vacio };
    let mut j = fin_tag + 1;
    if vacio {
        return Ok((e, j));
    }
    loop {
        if j >= b.len() {
            bail_tr!("XML roto: <{}> sin cerrar", "broken XML: <{}> not closed", e.nombre);
        }
        if x[j..].starts_with("</") {
            let k = x[j..].find('>').ok_or_else(|| anyhow_tr!("XML roto: cierre sin «>»", "broken XML: closing tag without «>»"))? + j;
            let cierra = x[j + 2..k].trim();
            if cierra != e.nombre {
                bail_tr!("XML roto: <{}> se cierra con </{cierra}>", "broken XML: <{}> is closed with </{cierra}>", e.nombre);
            }
            return Ok((e, k + 1));
        }
        if x[j..].starts_with("<!--") || x[j..].starts_with("<![CDATA[") || x[j..].starts_with("<?") {
            let fin = if x[j..].starts_with("<!--") { "-->" } else if x[j..].starts_with("<?") { "?>" } else { "]]>" };
            let k = x[j..].find(fin).ok_or_else(|| anyhow_tr!("XML roto: «{fin}» sin cerrar", "broken XML: «{fin}» not closed"))? + j + fin.len();
            e.hijos.push(Nodo::Crudo(x[j..k].to_string()));
            j = k;
            continue;
        }
        if b[j] == b'<' {
            let (h, k) = elemento(x, j)?;
            e.hijos.push(Nodo::Elem(h));
            j = k;
            continue;
        }
        let k = x[j..].find('<').map(|k| k + j).unwrap_or(b.len());
        e.hijos.push(Nodo::Crudo(x[j..k].to_string()));
        j = k;
    }
}

/// El `>` que cierra una etiqueta, saltando los que van dentro de comillas.
fn buscar_fin_tag(x: &str, i: usize) -> Result<usize> {
    let mut comilla: Option<u8> = None;
    for (k, c) in x.as_bytes()[i..].iter().enumerate() {
        match (comilla, *c) {
            (None, b'"') | (None, b'\'') => comilla = Some(*c),
            (Some(q), c) if c == q => comilla = None,
            (None, b'>') => return Ok(i + k),
            _ => {}
        }
    }
    bail_tr!("XML roto: etiqueta sin «>» en el byte {i}", "broken XML: tag without «>» at byte {i}")
}

fn atributos(s: &str) -> Result<Vec<(String, String)>> {
    let mut v = vec![];
    let mut r = s.trim_start();
    while !r.is_empty() {
        let eq = r.find('=').ok_or_else(|| anyhow_tr!("XML roto: atributo sin «=» en «{}»", "broken XML: attribute without «=» in «{}»", &r[..r.len().min(40)]))?;
        let k = r[..eq].trim().to_string();
        let r2 = r[eq + 1..].trim_start();
        let q = r2.chars().next().ok_or_else(|| anyhow_tr!("XML roto: atributo {k} sin valor", "broken XML: attribute {k} without a value"))?;
        if q != '"' && q != '\'' {
            bail_tr!("XML roto: el atributo {k} no va entre comillas", "broken XML: attribute {k} is not quoted");
        }
        let fin = r2[1..].find(q).ok_or_else(|| anyhow_tr!("XML roto: comillas sin cerrar en {k}", "broken XML: unclosed quotes in {k}"))? + 1;
        v.push((k, r2[1..fin].to_string()));
        r = r2[fin + 1..].trim_start();
    }
    Ok(v)
}

pub fn escapar(s: &str, attr: bool) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' if attr => o.push_str("&quot;"),
            c => o.push(c),
        }
    }
    o
}

pub fn desescapar(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut o = String::with_capacity(s.len());
    let mut r = s;
    while let Some(i) = r.find('&') {
        o.push_str(&r[..i]);
        let Some(j) = r[i..].find(';') else {
            o.push_str(&r[i..]);
            return o;
        };
        let ent = &r[i + 1..i + j];
        let c = match ent {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            e if e.starts_with("#x") => u32::from_str_radix(&e[2..], 16).ok().and_then(char::from_u32),
            e if e.starts_with('#') => e[1..].parse().ok().and_then(char::from_u32),
            _ => None,
        };
        match c {
            Some(c) => o.push(c),
            None => o.push_str(&r[i..=i + j]),
        }
        r = &r[i + j + 1..];
    }
    o.push_str(r);
    o
}

/// Un argumento que debía ser lista u objeto y llegó como TEXTO con JSON (medido con el 9B en
/// `word editar`: `cambios` como cadena, con «,,» dobles) → se repara como en D4b. Si ya era
/// estructura, igual.
pub fn tolerante(v: &serde_json::Value) -> serde_json::Value {
    let Some(s) = v.as_str() else { return v.clone() };
    let mut t = s.trim().to_string();
    while t.contains(",,") {
        t = t.replace(",,", ",");
    }
    if let Some(v) = sin_un_cierre_de_mas(&t) {
        return v;
    }
    gotv_diseno::pagina::reparar_json(&t).map(|(v, _)| v).unwrap_or_else(|_| v.clone())
}

/// Medido (banco of-carta, 9B): `{"carta": {… "texto": […]}, "remitente": {…}}}` — un `}` de más
/// cerró el objeto antes de tiempo («trailing characters»). Se quita UN cierre (`}` o `]`), del más
/// cercano al error hacia atrás, hasta que el JSON se lea entero.
fn sin_un_cierre_de_mas(t: &str) -> Option<serde_json::Value> {
    let e = match serde_json::from_str::<serde_json::Value>(t) {
        Ok(_) => return None, // ya era válido: nada que reparar aquí
        Err(e) => e,
    };
    if !e.to_string().contains("trailing characters") || e.line() != 1 {
        return None;
    }
    let hasta = t.char_indices().nth(e.column().saturating_sub(1)).map(|(i, _)| i).unwrap_or(t.len());
    let cierres: Vec<usize> = t[..hasta].char_indices().filter(|(_, c)| matches!(c, '}' | ']')).map(|(i, _)| i).collect();
    for &i in cierres.iter().rev().take(40) {
        let mut u = String::with_capacity(t.len());
        u.push_str(&t[..i]);
        u.push_str(&t[i + 1..]);
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&u) {
            return Some(v);
        }
    }
    None
}

/// El texto sin las MARCAS que ponen las lecturas de GOTV (`¶3 `, `# `, `- `, `1. `, `**…**`).
/// Medido (banco of-plantilla, 9B): copió «# Contrato de suministro de café» de `word leer` y el
/// reemplazo nunca lo halló (el `#` no está en el documento). `None` si no tenía marcas.
pub fn sin_marcas_de_leer(s: &str) -> Option<String> {
    let mut t = s.trim_start();
    if let Some(r) = t.strip_prefix('¶') {
        t = r.trim_start_matches(|c: char| c.is_ascii_digit()).trim_start();
    }
    let t = t.trim_start_matches('#').trim_start();
    let t = t.strip_prefix("- ").or_else(|| t.strip_prefix("• ")).unwrap_or(t);
    let dig = t.chars().take_while(|c| c.is_ascii_digit()).count();
    let t = if dig > 0 && dig <= 2 && t[dig..].starts_with(". ") { &t[dig + 2..] } else { t };
    let t = t.replace("**", "");
    (t != s && !t.trim().is_empty()).then_some(t)
}

/// Como `tolerante`, pero si llegó TEXTO que ni reparado es JSON, dice DÓNDE se rompe. Medido
/// (banco of-deck, 9B): mandó la ficha como texto con `["derecha": […]]` y el aviso «la ficha
/// necesita diapositivas» no le decía el error real: tardó 3 intentos.
pub fn tolerante_o_error(v: &serde_json::Value, campo: &str) -> anyhow::Result<serde_json::Value> {
    let r = tolerante(v);
    if let (Some(s), true) = (v.as_str(), r.is_string()) {
        if let Err(e) = serde_json::from_str::<serde_json::Value>(s.trim()) {
            let col = e.column().saturating_sub(1);
            let linea = s.lines().nth(e.line().saturating_sub(1)).unwrap_or("");
            let desde = linea.char_indices().map(|(i, _)| i).find(|i| *i + 60 >= col).unwrap_or(0);
            let trozo: String = linea[desde..].chars().take(90).collect();
            bail_tr!(
                "«{campo}» llegó como TEXTO y no es JSON válido ({e}), cerca de: …{trozo}… — mándalo como OBJETO JSON (sin comillas alrededor) y revisa esa parte: un arreglo [ ] no lleva «clave: valor», un objeto {{ }} sí",
                "«{campo}» arrived as TEXT and is not valid JSON ({e}), near: …{trozo}… — send it as a JSON OBJECT (no quotes around it) and check that part: an array [ ] has no «key: value», an object {{ }} does"
            );
        }
    }
    Ok(r)
}

// ── el validador del paquete ──────────────────────────────────────────────────────────────────

/// Lo que Office exige del PAQUETE (base de `validators/base.py`): XML bien formado en cada
/// parte, un tipo de contenido para cada parte, y cada relación interna apuntando a algo que existe.
pub fn validar_paquete(p: &Paquete) -> Vec<String> {
    let mut errores = vec![];
    let tipos = p.texto("[Content_Types].xml").unwrap_or_default();
    let Ok(ct) = Doc::leer(&tipos) else {
        return vec![crate::tr!("[Content_Types].xml no es XML válido", "[Content_Types].xml is not valid XML")];
    };
    let mut por_omision = vec![];
    let mut sobrescritos = vec![];
    for e in ct.raiz.elems() {
        match e.local() {
            "Default" => por_omision.push(e.attr("Extension").unwrap_or_default().to_lowercase()),
            "Override" => sobrescritos.push(e.attr("PartName").unwrap_or_default().trim_start_matches('/').to_string()),
            _ => {}
        }
    }
    for (n, b) in &p.partes {
        if n == "[Content_Types].xml" {
            continue;
        }
        let ext = n.rsplit('.').next().unwrap_or("").to_lowercase();
        if !sobrescritos.contains(n) && !por_omision.contains(&ext) {
            errores.push(crate::tr!("{n}: sin tipo de contenido en [Content_Types].xml (añade un <Override PartName=\"/{n}\" …/>)", "{n}: no content type in [Content_Types].xml (add an <Override PartName=\"/{n}\" …/>)"));
        }
        if n.ends_with(".xml") || n.ends_with(".rels") {
            if let Err(e) = Doc::leer(&String::from_utf8_lossy(b)) {
                errores.push(format!("{n}: {e}"));
            }
        }
    }
    for o in &sobrescritos {
        if !p.partes.contains_key(o) {
            errores.push(crate::tr!("[Content_Types].xml declara /{o}, que no existe", "[Content_Types].xml declares /{o}, which does not exist"));
        }
    }
    for (n, b) in &p.partes {
        if !n.ends_with(".rels") {
            continue;
        }
        let Ok(d) = Doc::leer(&String::from_utf8_lossy(b)) else { continue };
        let base = n.trim_end_matches(".rels").replace("_rels/", "");
        let carpeta = base.rsplit_once('/').map(|(c, _)| c.to_string()).unwrap_or_default();
        for r in d.raiz.elems().filter(|e| e.local() == "Relationship") {
            if r.attr("TargetMode").as_deref() == Some("External") {
                continue;
            }
            let t = r.attr("Target").unwrap_or_default();
            let destino = resolver(&carpeta, &t);
            if !p.partes.contains_key(&destino) {
                errores.push(crate::tr!("{n}: la relación {} apunta a «{t}», que no existe", "{n}: relationship {} points at «{t}», which does not exist", r.attr("Id").unwrap_or_default()));
            }
        }
    }
    errores
}

/// Destino de una relación, relativo a la carpeta de la parte de origen (`opc_target`).
pub fn resolver(carpeta: &str, t: &str) -> String {
    let unido = if let Some(abs) = t.strip_prefix('/') { abs.to_string() } else if carpeta.is_empty() { t.to_string() } else { format!("{carpeta}/{t}") };
    let mut partes: Vec<&str> = vec![];
    for s in unido.split('/') {
        match s {
            "" | "." => {}
            ".." => {
                partes.pop();
            }
            s => partes.push(s),
        }
    }
    partes.join("/")
}

#[cfg(test)]
mod pruebas {
    use super::*;

    #[test]
    fn el_xml_vuelve_byte_a_byte() {
        let x = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\r\n<w:document xmlns:w=\"http://x\" w:a='1'><w:body><w:p><w:r><w:t xml:space=\"preserve\"> Hola &amp; adiós </w:t></w:r><!-- c --><w:br/></w:p><![CDATA[<x>]]></w:body></w:document>";
        let d = Doc::leer(x).unwrap();
        assert_eq!(d.escribir().replace("w:a=\"1\"", "w:a='1'"), x, "sin tocar, sale igual (las comillas simples pasan a dobles)");
        let mut ts = vec![];
        d.raiz.buscar("t", &mut ts);
        assert_eq!(ts[0].texto(), " Hola & adiós ");
        assert!(Doc::leer("<a><b></a>").is_err() && Doc::leer("<a x=1/>").is_err());
    }

    #[test]
    fn rutas_de_relaciones() {
        assert_eq!(resolver("word", "media/a.png"), "word/media/a.png");
        assert_eq!(resolver("ppt/slides", "../slideLayouts/slideLayout1.xml"), "ppt/slideLayouts/slideLayout1.xml");
        assert_eq!(resolver("", "word/document.xml"), "word/document.xml");
        assert_eq!(resolver("word", "/customXml/item1.xml"), "customXml/item1.xml");
    }
}
