//! O2 · Word (.docx) como ARCHIVO, con el método de la skill `docx` de Claude Code (Apache-2.0;
//! `ESTUDIO-DOCUMENTOS.md` §2.3), en Rust:
//!   - crear = sus trampas de docx-js resueltas en el generador: cada nivel de título con fuente,
//!     espacio antes/después y `keepNext`; listas con numeración (nunca `•` en el texto); tablas con
//!     anchos DOBLES en DXA que suman el ancho; sombreado CLEAR (SOLID sale negro); nada de `\n`
//!     (párrafos separados); salto de página DENTRO de un párrafo; línea = borde inferior de párrafo;
//!   - leer = su `pandoc -t markdown`: títulos, listas, tablas, con el número de cada párrafo;
//!   - editar = unzip → «unir fragmentos» (`merge_runs.py`) → editar `word/document.xml` → zip, sin
//!     reformatear, con respaldo;
//!   - revisar = su validador (paquete, `paraId`, `durableId`, `styleId` repetidos, espacios sin
//!     `xml:space="preserve"`, comentarios emparejados).

use crate::ooxml::{escapar, Doc, Elem, Nodo, Paquete};
use crate::tema_doc::TemaDoc;
use anyhow::{anyhow, bail, Result};
use gotv_engine::{anyhow_tr, bail_tr};
use serde_json::Value;
use std::path::Path;

pub(crate) const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
pub(crate) const R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

// ── crear ────────────────────────────────────────────────────────────────────────────────────

/// Un trozo de texto con formato (de `**negrita**` y `*cursiva*` en el texto del modelo).
pub(crate) fn trozos(t: &str) -> Vec<(String, bool, bool)> {
    let mut v = vec![];
    let (mut neg, mut cur) = (false, false);
    let mut buf = String::new();
    let mut it = t.chars().peekable();
    while let Some(c) = it.next() {
        if c == '*' {
            let doble = it.peek() == Some(&'*');
            if doble {
                it.next();
            }
            if !buf.is_empty() {
                v.push((std::mem::take(&mut buf), neg, cur));
            }
            if doble { neg = !neg } else { cur = !cur }
            continue;
        }
        buf.push(c);
    }
    if !buf.is_empty() {
        v.push((buf, neg, cur));
    }
    v
}

fn run(t: &str, neg: bool, cur: bool) -> String {
    let mut rpr = String::new();
    if neg {
        rpr.push_str("<w:b/><w:bCs/>");
    }
    if cur {
        rpr.push_str("<w:i/><w:iCs/>");
    }
    let rpr = if rpr.is_empty() { String::new() } else { format!("<w:rPr>{rpr}</w:rPr>") };
    // tabulador real, no espacios
    let partes: Vec<String> = t
        .split('\t')
        .map(|s| {
            let pres = if s.starts_with(char::is_whitespace) || s.ends_with(char::is_whitespace) { " xml:space=\"preserve\"" } else { "" };
            format!("<w:t{pres}>{}</w:t>", escapar(s, false))
        })
        .collect();
    format!("<w:r>{rpr}{}</w:r>", partes.join("<w:tab/>"))
}

fn parrafo(estilo: Option<&str>, num: Option<(u32, u32)>, alin: Option<&str>, extra_ppr: &str, texto: &str) -> String {
    let mut ppr = String::new();
    if let Some(e) = estilo {
        ppr.push_str(&format!("<w:pStyle w:val=\"{e}\"/>"));
    }
    if let Some((id, nivel)) = num {
        ppr.push_str(&format!("<w:numPr><w:ilvl w:val=\"{nivel}\"/><w:numId w:val=\"{id}\"/></w:numPr>"));
    }
    ppr.push_str(extra_ppr);
    if let Some(a) = alin {
        ppr.push_str(&format!("<w:jc w:val=\"{a}\"/>"));
    }
    let ppr = if ppr.is_empty() { String::new() } else { format!("<w:pPr>{ppr}</w:pPr>") };
    let runs: String = trozos(texto).iter().map(|(t, n, c)| run(t, *n, *c)).collect();
    format!("<w:p>{ppr}{runs}</w:p>")
}

pub(crate) fn alineacion(b: &Value) -> Option<&'static str> {
    match b["alineacion"].as_str()? {
        "centro" | "centrado" | "center" => Some("center"),
        "derecha" | "right" => Some("right"),
        "justificado" | "justify" | "both" => Some("both"),
        _ => None,
    }
}

/// Un bloque en la forma que el modelo escribe de forma natural → la de la ficha. Medido con el 9B:
/// `{"tipo":"parrafo","texto":"…"}` y `{"bloque":"parrafo","texto":"…"}` en vez de `{"parrafo":"…"}`.
/// También: un texto suelto = párrafo; `{"heading":2,…}`; nombres en inglés.
pub fn normalizar_bloque(b: &Value) -> Value {
    if let Some(s) = b.as_str() {
        return serde_json::json!({ "parrafo": s });
    }
    let Some(o) = b.as_object() else { return b.clone() };
    const CLAVES: [&str; 10] = ["titulo", "h1", "h2", "h3", "parrafo", "lista", "numerada", "tabla", "salto", "linea"];
    if o.keys().any(|k| CLAVES.contains(&k.as_str())) {
        return b.clone();
    }
    let tipo = ["tipo", "bloque", "type", "kind", "estilo", "style"].iter().find_map(|k| o.get(*k)).and_then(|v| v.as_str()).unwrap_or("parrafo").to_lowercase();
    let tipo = tipo.replace('á', "a").replace('í', "i").replace('ñ', "n").replace([' ', '-'], "_");
    let nivel = o.get("nivel").or_else(|| o.get("level")).and_then(|v| v.as_u64());
    let clave = match tipo.as_str() {
        "title" | "titulo_principal" | "titulo" => "titulo",
        "h1" | "heading1" | "titulo1" | "encabezado1" => "h1",
        "h2" | "heading2" | "titulo2" | "subtitulo" | "encabezado2" => "h2",
        "h3" | "heading3" | "titulo3" | "encabezado3" => "h3",
        "heading" | "encabezado" | "seccion" => match nivel {
            Some(2) => "h2",
            Some(3..) => "h3",
            _ => "h1",
        },
        "lista" | "list" | "bullet" | "bullets" | "vinetas" | "lista_vinetas" => "lista",
        "numerada" | "numbered" | "lista_numerada" | "ordered" | "enumeracion" => "numerada",
        "tabla" | "table" => "tabla",
        "salto" | "page_break" | "salto_pagina" | "pagebreak" => "salto",
        "linea" | "line" | "separador" | "hr" => "linea",
        _ => "parrafo",
    };
    let contenido = ["texto", "text", "contenido", "content", "valor", "items", "elementos", "filas", "rows", "datos"].iter().find_map(|k| o.get(*k)).cloned().unwrap_or(Value::Bool(true));
    let mut n = serde_json::Map::new();
    n.insert(clave.into(), contenido);
    for k in ["alineacion", "anchos", "encabezado"] {
        if let Some(v) = o.get(k) {
            n.insert(k.into(), v.clone());
        }
    }
    Value::Object(n)
}

/// Las listas numeradas que hacen falta (cada una reinicia en 1) se cuentan al generar.
struct Gen {
    /// O4: el tema del documento (colores de la tabla y del filete); sin él, sobrio
    tema: Option<TemaDoc>,
    ancho: u32,
    numeradas: u32,
}

impl Gen {
    /// Un bloque de la ficha → XML de párrafos/tablas. `\n` en un texto = párrafos separados.
    fn bloque(&mut self, b: &Value) -> Result<String> {
        let b = &normalizar_bloque(b);
        let o = b.as_object().ok_or_else(|| anyhow!("cada bloque es un objeto, p. ej. {{\"parrafo\": \"…\"}}"))?;
        let alin = alineacion(b);
        let mut s = String::new();
        let lineas = |v: &Value| -> Vec<String> { v.as_str().unwrap_or_default().split('\n').map(str::to_string).collect() };
        if let Some(t) = o.get("titulo") {
            for l in lineas(t) {
                s.push_str(&parrafo(Some("Title"), None, alin, "", &l));
            }
        } else if let Some((nivel, t)) = ["h1", "h2", "h3"].iter().enumerate().find_map(|(i, k)| o.get(*k).map(|t| (i + 1, t))) {
            for l in lineas(t) {
                s.push_str(&parrafo(Some(&format!("Heading{nivel}")), None, alin, "", &l));
            }
        } else if let Some(t) = o.get("parrafo") {
            for l in lineas(t) {
                s.push_str(&parrafo(None, None, alin, "", &l));
            }
        } else if let Some(items) = o.get("lista").or_else(|| o.get("numerada")) {
            let id = if o.contains_key("numerada") {
                self.numeradas += 1;
                1 + self.numeradas
            } else {
                1
            };
            for it in items.as_array().ok_or_else(|| anyhow_tr!("«lista» y «numerada» llevan una lista de textos", "«lista» and «numerada» take a list of texts"))? {
                // un ítem puede ser ["sub-ítem", …] = segundo nivel
                match it {
                    Value::Array(sub) => {
                        for x in sub {
                            s.push_str(&parrafo(Some("ListParagraph"), Some((id, 1)), None, "", x.as_str().unwrap_or_default()));
                        }
                    }
                    x => s.push_str(&parrafo(Some("ListParagraph"), Some((id, 0)), None, "", x.as_str().unwrap_or(&x.to_string()))),
                }
            }
        } else if let Some(t) = o.get("tabla") {
            s.push_str(&self.tabla(t, b)?);
        } else if o.contains_key("salto") {
            s.push_str("<w:p><w:r><w:br w:type=\"page\"/></w:r></w:p>");
        } else if o.contains_key("linea") {
            let color = self.tema.as_ref().map(|t| t.primario.as_str()).unwrap_or("A6A6A6");
            s.push_str(&format!("<w:p><w:pPr><w:pBdr><w:bottom w:val=\"single\" w:sz=\"6\" w:space=\"1\" w:color=\"{color}\"/></w:pBdr></w:pPr></w:p>"));
        } else {
            let claves: Vec<&String> = o.keys().collect();
            bail!("bloque desconocido {claves:?}: usa titulo, h1, h2, h3, parrafo, lista, numerada, tabla, salto o linea");
        }
        Ok(s)
    }

    /// Tabla con anchos DOBLES (rejilla de la tabla Y ancho de cada celda, en DXA, que suman el
    /// ancho útil), primera fila de encabezado en negrita con sombreado CLEAR.
    fn tabla(&mut self, t: &Value, b: &Value) -> Result<String> {
        let filas: Vec<Vec<String>> = t
            .as_array()
            .ok_or_else(|| anyhow_tr!("«tabla» es una lista de filas: [[\"Encabezado\",…],[\"dato\",…]]", "«tabla» is a list of rows: [[\"Header\",…],[\"value\",…]]"))?
            .iter()
            .map(|f| f.as_array().cloned().unwrap_or_default().iter().map(|c| c.as_str().map(str::to_string).unwrap_or_else(|| c.to_string())).collect())
            .collect();
        let cols = filas.iter().map(Vec::len).max().unwrap_or(0);
        if cols == 0 {
            bail_tr!("la tabla está vacía", "the table is empty");
        }
        let pesos: Vec<f64> = match b["anchos"].as_array() {
            Some(a) if a.len() == cols => a.iter().map(|x| x.as_f64().unwrap_or(1.0).max(0.1)).collect(),
            _ => vec![1.0; cols],
        };
        let total: f64 = pesos.iter().sum();
        let mut anchos: Vec<u32> = pesos.iter().map(|p| (self.ancho as f64 * p / total).floor() as u32).collect();
        let resto = self.ancho - anchos.iter().sum::<u32>();
        anchos[cols - 1] += resto; // que sumen EXACTO
        let encabezado = b["encabezado"].as_bool().unwrap_or(true);
        let mut s = format!(
            "<w:tbl><w:tblPr><w:tblStyle w:val=\"TableGrid\"/><w:tblW w:w=\"{}\" w:type=\"dxa\"/><w:tblLayout w:type=\"fixed\"/><w:tblLook w:val=\"04A0\" w:firstRow=\"1\" w:lastRow=\"0\" w:firstColumn=\"1\" w:lastColumn=\"0\" w:noHBand=\"0\" w:noVBand=\"1\"/></w:tblPr><w:tblGrid>",
            self.ancho
        );
        for a in &anchos {
            s.push_str(&format!("<w:gridCol w:w=\"{a}\"/>"));
        }
        s.push_str("</w:tblGrid>");
        for (i, f) in filas.iter().enumerate() {
            let cab = encabezado && i == 0;
            s.push_str(if cab { "<w:tr><w:trPr><w:tblHeader/></w:trPr>" } else { "<w:tr>" });
            for (j, a) in anchos.iter().enumerate() {
                let txt = f.get(j).cloned().unwrap_or_default();
                // con tema: cabecera en el color de marca (su texto, el que se lee encima) y filas alternas suaves
                let sombra = match (&self.tema, cab) {
                    (Some(t), true) => format!("<w:shd w:val=\"clear\" w:color=\"auto\" w:fill=\"{}\"/>", t.primario),
                    (Some(t), false) if (i + usize::from(!encabezado)) % 2 == 0 => format!("<w:shd w:val=\"clear\" w:color=\"auto\" w:fill=\"{}\"/>", t.suave),
                    (None, true) => "<w:shd w:val=\"clear\" w:color=\"auto\" w:fill=\"E7E6E6\"/>".to_string(),
                    _ => String::new(),
                };
                let estilo_celda = if cab && self.tema.is_some() { Some("TablaCab") } else { None };
                let txt = if cab && !txt.is_empty() && !txt.starts_with("**") { format!("**{txt}**") } else { txt };
                s.push_str(&format!("<w:tc><w:tcPr><w:tcW w:w=\"{a}\" w:type=\"dxa\"/>{sombra}</w:tcPr>{}</w:tc>", parrafo(estilo_celda, None, None, "<w:spacing w:before=\"40\" w:after=\"40\"/>", &txt)));
            }
            s.push_str("</w:tr>");
        }
        s.push_str("</w:tbl><w:p/>");
        Ok(s)
    }
}

fn hoy_iso() -> String {
    let s = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0) as i64;
    let (dias, resto) = (s.div_euclid(86400), s.rem_euclid(86400));
    // días desde 1970 → fecha civil (algoritmo de Howard Hinnant)
    let z = dias + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", resto / 3600, resto % 3600 / 60, resto % 60)
}

/// Los estilos del documento. Sin tema salen sobrios (títulos en gris oscuro, tabla gris); con tema
/// (O4) los títulos llevan su tipografía y el color de marca, el título principal un filete debajo, el
/// texto y los bordes los colores del tema, y hay dos estilos más: la cabecera de tabla y el pie.
fn estilos(fuente: &str, tema: Option<&TemaDoc>) -> String {
    let f = escapar(tema.map(|t| t.fuente_cuerpo.as_str()).unwrap_or(fuente), true);
    let ft = escapar(tema.map(|t| t.fuente_titulo.as_str()).unwrap_or(fuente), true);
    let fuentes = format!("<w:rFonts w:ascii=\"{f}\" w:hAnsi=\"{f}\" w:eastAsia=\"{f}\" w:cs=\"{f}\"/>");
    let fuentes_titulo = format!("<w:rFonts w:ascii=\"{ft}\" w:hAnsi=\"{ft}\" w:eastAsia=\"{ft}\" w:cs=\"{ft}\"/>");
    let (c_titulo, c_h2, c_h3, borde) = match tema {
        Some(t) => (t.titulo.as_str(), t.texto.as_str(), t.tenue.as_str(), t.borde.as_str()),
        None => ("1F2937", "1F2937", "1F2937", "BFBFBF"),
    };
    let color_texto = tema.map(|t| format!("<w:color w:val=\"{}\"/>", t.texto)).unwrap_or_default();
    let filete = tema.map(|t| format!("<w:pBdr><w:bottom w:val=\"single\" w:sz=\"12\" w:space=\"8\" w:color=\"{}\"/></w:pBdr>", t.primario)).unwrap_or_default();
    let extra = tema
        .map(|t| {
            format!(
                "<w:style w:type=\"paragraph\" w:styleId=\"TablaCab\"><w:name w:val=\"Tabla Cabecera\"/><w:basedOn w:val=\"Normal\"/><w:uiPriority w:val=\"40\"/><w:qFormat/><w:rPr>{fuentes}<w:b/><w:bCs/><w:color w:val=\"{}\"/></w:rPr></w:style>\
<w:style w:type=\"paragraph\" w:styleId=\"Pie\"><w:name w:val=\"Pie\"/><w:basedOn w:val=\"Normal\"/><w:uiPriority w:val=\"41\"/><w:pPr><w:spacing w:after=\"0\"/><w:jc w:val=\"center\"/></w:pPr><w:rPr>{fuentes}<w:color w:val=\"{}\"/><w:sz w:val=\"18\"/><w:szCs w:val=\"18\"/></w:rPr></w:style>",
                t.sobre_primario, t.tenue
            )
        })
        .unwrap_or_default();
    let titulo = |id: &str, nombre: &str, sz: u32, antes: u32, despues: u32, nivel: Option<u32>, color: &str, borde_inferior: &str| {
        let ol = nivel.map(|n| format!("<w:outlineLvl w:val=\"{n}\"/>")).unwrap_or_default();
        format!(
            "<w:style w:type=\"paragraph\" w:styleId=\"{id}\"><w:name w:val=\"{nombre}\"/><w:basedOn w:val=\"Normal\"/><w:next w:val=\"Normal\"/><w:uiPriority w:val=\"9\"/><w:qFormat/><w:pPr><w:keepNext/><w:keepLines/>{borde_inferior}<w:spacing w:before=\"{antes}\" w:after=\"{despues}\"/>{ol}</w:pPr><w:rPr>{fuentes_titulo}<w:b/><w:bCs/><w:color w:val=\"{color}\"/><w:sz w:val=\"{sz}\"/><w:szCs w:val=\"{sz}\"/></w:rPr></w:style>"
        )
    };
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<w:styles xmlns:w=\"{W}\"><w:docDefaults><w:rPrDefault><w:rPr>{fuentes}<w:sz w:val=\"22\"/><w:szCs w:val=\"22\"/><w:lang w:val=\"es-ES\" w:eastAsia=\"en-US\" w:bidi=\"ar-SA\"/></w:rPr></w:rPrDefault><w:pPrDefault><w:pPr><w:spacing w:after=\"160\" w:line=\"276\" w:lineRule=\"auto\"/></w:pPr></w:pPrDefault></w:docDefaults>\
<w:style w:type=\"paragraph\" w:default=\"1\" w:styleId=\"Normal\"><w:name w:val=\"Normal\"/><w:qFormat/><w:rPr>{fuentes}{color_texto}</w:rPr></w:style>\
{}{}{}{}\
<w:style w:type=\"paragraph\" w:styleId=\"ListParagraph\"><w:name w:val=\"List Paragraph\"/><w:basedOn w:val=\"Normal\"/><w:uiPriority w:val=\"34\"/><w:qFormat/><w:pPr><w:spacing w:after=\"80\"/><w:ind w:left=\"720\"/><w:contextualSpacing/></w:pPr></w:style>\
<w:style w:type=\"table\" w:default=\"1\" w:styleId=\"TableNormal\"><w:name w:val=\"Normal Table\"/><w:uiPriority w:val=\"99\"/><w:semiHidden/><w:unhideWhenUsed/><w:tblPr><w:tblInd w:w=\"0\" w:type=\"dxa\"/><w:tblCellMar><w:top w:w=\"0\" w:type=\"dxa\"/><w:left w:w=\"108\" w:type=\"dxa\"/><w:bottom w:w=\"0\" w:type=\"dxa\"/><w:right w:w=\"108\" w:type=\"dxa\"/></w:tblCellMar></w:tblPr></w:style>\
<w:style w:type=\"table\" w:styleId=\"TableGrid\"><w:name w:val=\"Table Grid\"/><w:basedOn w:val=\"TableNormal\"/><w:uiPriority w:val=\"39\"/><w:pPr><w:spacing w:after=\"0\" w:line=\"240\" w:lineRule=\"auto\"/></w:pPr><w:tblPr><w:tblBorders><w:top w:val=\"single\" w:sz=\"4\" w:space=\"0\" w:color=\"{borde}\"/><w:left w:val=\"single\" w:sz=\"4\" w:space=\"0\" w:color=\"{borde}\"/><w:bottom w:val=\"single\" w:sz=\"4\" w:space=\"0\" w:color=\"{borde}\"/><w:right w:val=\"single\" w:sz=\"4\" w:space=\"0\" w:color=\"{borde}\"/><w:insideH w:val=\"single\" w:sz=\"4\" w:space=\"0\" w:color=\"{borde}\"/><w:insideV w:val=\"single\" w:sz=\"4\" w:space=\"0\" w:color=\"{borde}\"/></w:tblBorders></w:tblPr></w:style>\
{extra}</w:styles>",
        titulo("Title", "Title", 52, 0, 240, None, c_titulo, filete.as_str()),
        titulo("Heading1", "heading 1", 32, 360, 120, Some(0), c_titulo, ""),
        titulo("Heading2", "heading 2", 26, 240, 80, Some(1), c_h2, ""),
        titulo("Heading3", "heading 3", 24, 200, 80, Some(2), c_h3, ""),
    )
}

/// Numeración: num 1 = viñetas; num 2.. = una lista numerada cada una (reinicia en 1).
fn numeracion(numeradas: u32) -> String {
    let nivel = |i: u32, fmt: &str, txt: &str| {
        format!("<w:lvl w:ilvl=\"{i}\"><w:start w:val=\"1\"/><w:numFmt w:val=\"{fmt}\"/><w:lvlText w:val=\"{txt}\"/><w:lvlJc w:val=\"left\"/><w:pPr><w:ind w:left=\"{}\" w:hanging=\"360\"/></w:pPr></w:lvl>", 720 * (i + 1))
    };
    let mut s = format!("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<w:numbering xmlns:w=\"{W}\"><w:abstractNum w:abstractNumId=\"0\"><w:multiLevelType w:val=\"hybridMultilevel\"/>{}{}</w:abstractNum><w:abstractNum w:abstractNumId=\"1\"><w:multiLevelType w:val=\"hybridMultilevel\"/>{}{}</w:abstractNum><w:num w:numId=\"1\"><w:abstractNumId w:val=\"0\"/></w:num>",
        nivel(0, "bullet", "\u{2022}"), nivel(1, "bullet", "\u{25E6}"), nivel(0, "decimal", "%1."), nivel(1, "lowerLetter", "%2)"));
    for i in 0..numeradas.max(1) {
        s.push_str(&format!("<w:num w:numId=\"{}\"><w:abstractNumId w:val=\"1\"/><w:lvlOverride w:ilvl=\"0\"><w:startOverride w:val=\"1\"/></w:lvlOverride></w:num>", 2 + i));
    }
    s.push_str("</w:numbering>");
    s
}

/// CREAR un .docx desde una ficha: `{pagina?: "carta"|"a4", fuente?: "Arial", titulo_doc?,
/// bloques: [{titulo}|{h1}|{h2}|{h3}|{parrafo, alineacion?}|{lista:[…]}|{numerada:[…]}|
/// {tabla:[[…]], anchos?, encabezado?}|{salto:true}|{linea:true}]}`. `**negrita**`, `*cursiva*`.
pub fn crear(path: &Path, ficha: &Value) -> Result<usize> {
    let ficha = &crate::ooxml::tolerante_o_error(ficha, "ficha")?;
    // una CARTA va con su formato (GTC 185), no como bloques sueltos
    if ficha["carta"].is_object() {
        // los campos de la carta que quedaron FUERA de `carta` (un cierre mal puesto) se recogen
        let mut c = ficha["carta"].clone();
        if let (Some(dentro), Some(fuera)) = (c.as_object_mut(), ficha.as_object()) {
            for (k, v) in fuera.iter().filter(|(k, _)| *k != "carta") {
                dentro.entry(k.clone()).or_insert_with(|| v.clone());
            }
        }
        return crate::carta::crear(path, &c);
    }
    let bloques = ficha["bloques"].as_array().filter(|b| !b.is_empty()).ok_or_else(|| anyhow_tr!("la ficha necesita «bloques»: [{{\"titulo\":\"…\"}}, {{\"parrafo\":\"…\"}}]", "the spec needs «bloques»: [{{\"titulo\":\"…\"}}, {{\"parrafo\":\"…\"}}]"))?;
    // A4 por omisión (como docx-js); carta = 12240×15840 DXA (1440 = 1 pulgada)
    let (pw, ph) = match ficha["pagina"].as_str().unwrap_or("a4").to_lowercase().as_str() {
        "carta" | "letter" => (12240, 15840),
        "oficio" | "legal" => (12240, 20160),
        _ => (11906, 16838),
    };
    let margen = 1440;
    // O4: con `tema`, el documento va vestido (colores, tipografías, tabla y pie con el número de página)
    let tema = crate::tema_doc::de(&ficha["tema"]);
    let mut g = Gen { ancho: pw - 2 * margen, numeradas: 0, tema: tema.clone() };
    let mut cuerpo = String::new();
    for b in bloques {
        cuerpo.push_str(&g.bloque(b)?);
    }
    let fuente = ficha["fuente"].as_str().unwrap_or("Arial");
    let titulo = ficha["titulo_doc"].as_str().or_else(|| bloques.iter().find_map(|b| b["titulo"].as_str().or(b["h1"].as_str()))).unwrap_or("Documento");
    // el número de página al pie: por omisión, cuando hay tema; `numeros_pagina` lo fuerza o lo quita
    let pie = ficha["numeros_pagina"].as_bool().unwrap_or(tema.is_some()).then(|| {
        let campo = |c: &str| format!("<w:r><w:fldChar w:fldCharType=\"begin\"/></w:r><w:r><w:instrText xml:space=\"preserve\"> {c} </w:instrText></w:r><w:r><w:fldChar w:fldCharType=\"separate\"/></w:r><w:r><w:t>1</w:t></w:r><w:r><w:fldChar w:fldCharType=\"end\"/></w:r>");
        let estilo = if tema.is_some() { "<w:pStyle w:val=\"Pie\"/>" } else { "" };
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<w:ftr xmlns:w=\"{W}\" xmlns:r=\"{R}\"><w:p><w:pPr>{estilo}<w:jc w:val=\"center\"/></w:pPr>{}<w:r><w:t xml:space=\"preserve\"> / </w:t></w:r>{}</w:p></w:ftr>",
            campo("PAGE"),
            campo("NUMPAGES")
        )
    });
    let ref_pie = if pie.is_some() { "<w:footerReference w:type=\"default\" r:id=\"rIdP1\"/>" } else { "" };
    let sect = format!("<w:sectPr>{ref_pie}<w:pgSz w:w=\"{pw}\" w:h=\"{ph}\"/><w:pgMar w:top=\"{margen}\" w:right=\"{margen}\" w:bottom=\"{margen}\" w:left=\"{margen}\" w:header=\"708\" w:footer=\"708\" w:gutter=\"0\"/><w:cols w:space=\"708\"/></w:sectPr>");
    escribir_docx_con(path, &cuerpo, &sect, fuente, titulo, g.numeradas, &[], tema.as_ref(), pie.as_deref())?;
    Ok(bloques.len())
}

/// Escribe un .docx completo: cuerpo + `<w:sectPr>` dados, con estilos, numeración y propiedades.
/// `cabeceras` son encabezados extra (`word/header{n}.xml`, relación `rIdC{n}`, n desde 1) que el
/// `sectPr` referencia con `<w:headerReference r:id="rIdC1" …/>`. Valida el XML y el paquete ANTES
/// de escribir.
pub(crate) fn escribir_docx(path: &Path, cuerpo: &str, sect_pr: &str, fuente: &str, titulo: &str, numeradas: u32, cabeceras: &[String]) -> Result<()> {
    escribir_docx_con(path, cuerpo, sect_pr, fuente, titulo, numeradas, cabeceras, None, None)
}

/// `escribir_docx` con el TEMA del documento (O4) y un pie (`word/footer1.xml`, relación `rIdP1`, que el
/// `sectPr` referencia con `<w:footerReference r:id="rIdP1" …/>`).
#[allow(clippy::too_many_arguments)]
pub(crate) fn escribir_docx_con(path: &Path, cuerpo: &str, sect_pr: &str, fuente: &str, titulo: &str, numeradas: u32, cabeceras: &[String], tema: Option<&TemaDoc>, pie: Option<&str>) -> Result<()> {
    let doc = format!("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<w:document xmlns:w=\"{W}\" xmlns:r=\"{R}\"><w:body>{cuerpo}{sect_pr}</w:body></w:document>");
    // que lo generado sea XML válido ANTES de escribir
    Doc::leer(&doc).map_err(|e| anyhow_tr!("el documento generado no es XML válido: {e}", "the generated document is not valid XML: {e}"))?;
    let mut p = Paquete { partes: Default::default() };
    let ct_cab: String = (1..=cabeceras.len()).map(|n| format!("<Override PartName=\"/word/header{n}.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.header+xml\"/>")).collect();
    let rel_cab: String = (1..=cabeceras.len()).map(|n| format!("<Relationship Id=\"rIdC{n}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/header\" Target=\"header{n}.xml\"/>")).collect();
    let (ct_pie, rel_pie) = match pie {
        Some(_) => (
            "<Override PartName=\"/word/footer1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.footer+xml\"/>",
            "<Relationship Id=\"rIdP1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/footer\" Target=\"footer1.xml\"/>",
        ),
        None => ("", ""),
    };
    p.poner("[Content_Types].xml", format!("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"><Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/><Default Extension=\"xml\" ContentType=\"application/xml\"/><Override PartName=\"/word/document.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml\"/><Override PartName=\"/word/styles.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml\"/><Override PartName=\"/word/numbering.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.numbering+xml\"/><Override PartName=\"/word/settings.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.settings+xml\"/><Override PartName=\"/docProps/core.xml\" ContentType=\"application/vnd.openxmlformats-package.core-properties+xml\"/><Override PartName=\"/docProps/app.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.extended-properties+xml\"/>{ct_cab}{ct_pie}</Types>"));
    p.poner("_rels/.rels", "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"word/document.xml\"/><Relationship Id=\"rId2\" Type=\"http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties\" Target=\"docProps/core.xml\"/><Relationship Id=\"rId3\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/extended-properties\" Target=\"docProps/app.xml\"/></Relationships>");
    p.poner("word/_rels/document.xml.rels", format!("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles\" Target=\"styles.xml\"/><Relationship Id=\"rId2\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/numbering\" Target=\"numbering.xml\"/><Relationship Id=\"rId3\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/settings\" Target=\"settings.xml\"/>{rel_cab}{rel_pie}</Relationships>"));
    for (i, c) in cabeceras.iter().enumerate() {
        Doc::leer(c).map_err(|e| anyhow_tr!("el encabezado generado no es XML válido: {e}", "the generated header is not valid XML: {e}"))?;
        p.poner(&format!("word/header{}.xml", i + 1), c.clone());
    }
    if let Some(pie) = pie {
        Doc::leer(pie).map_err(|e| anyhow_tr!("el pie generado no es XML válido: {e}", "the generated footer is not valid XML: {e}"))?;
        p.poner("word/footer1.xml", pie.to_string());
    }
    p.poner("word/document.xml", doc);
    p.poner("word/styles.xml", estilos(fuente, tema));
    p.poner("word/numbering.xml", numeracion(numeradas));
    p.poner("word/settings.xml", format!("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<w:settings xmlns:w=\"{W}\"><w:defaultTabStop w:val=\"708\"/><w:characterSpacingControl w:val=\"doNotCompress\"/><w:compat><w:compatSetting w:name=\"compatibilityMode\" w:uri=\"http://schemas.microsoft.com/office/word\" w:val=\"15\"/></w:compat></w:settings>"));
    p.poner("docProps/core.xml", format!("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<cp:coreProperties xmlns:cp=\"http://schemas.openxmlformats.org/package/2006/metadata/core-properties\" xmlns:dc=\"http://purl.org/dc/elements/1.1/\" xmlns:dcterms=\"http://purl.org/dc/terms/\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\"><dc:title>{}</dc:title><dc:creator>GOD OF THE VOID</dc:creator><dcterms:created xsi:type=\"dcterms:W3CDTF\">{}</dcterms:created><dcterms:modified xsi:type=\"dcterms:W3CDTF\">{}</dcterms:modified></cp:coreProperties>", escapar(titulo, false), hoy_iso(), hoy_iso()));
    p.poner("docProps/app.xml", "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Properties xmlns=\"http://schemas.openxmlformats.org/officeDocument/2006/extended-properties\"><Application>GOD OF THE VOID</Application></Properties>");
    let errores = crate::ooxml::validar_paquete(&p);
    if !errores.is_empty() {
        bail_tr!("el paquete generado no pasa la validación: {}", "the generated package fails validation: {}", errores.join("; "));
    }
    p.guardar(path)?;
    Ok(())
}

// ── leer ─────────────────────────────────────────────────────────────────────────────────────

/// El texto VISIBLE de un párrafo: `w:t` (no `w:delText`, que es lo borrado con control de
/// cambios), tabuladores y saltos.
fn texto_parrafo(p: &Elem) -> String {
    let mut s = String::new();
    fn rec(e: &Elem, s: &mut String) {
        for h in e.elems() {
            match h.local() {
                "t" => s.push_str(&h.texto()),
                "tab" => s.push('\t'),
                "br" | "cr" => s.push('\n'),
                "del" | "delText" | "pPr" | "rPr" | "instrText" => {} // juez-idioma: datos (etiquetas XML de Word)
                _ => rec(h, s),
            }
        }
    }
    rec(p, &mut s);
    s
}

fn estilo_de(p: &Elem) -> String {
    p.hijo("pPr").and_then(|x| x.hijo("pStyle")).and_then(|x| x.attr("w:val")).unwrap_or_default()
}

fn nivel_titulo(estilo: &str) -> Option<usize> {
    let e = estilo.to_lowercase();
    if e == "title" || e == "ttulo" || e == "titulo" {
        return Some(1);
    }
    // Heading1 / Ttulo1 (Word en español) / Titulo1
    let n = e.trim_start_matches("heading").trim_start_matches("ttulo").trim_start_matches("titulo");
    (n.len() < e.len()).then(|| n.parse::<usize>().ok()).flatten().map(|n| n.min(6))
}

/// LEER: el documento como markdown, con el número de cada párrafo (¶) para editar luego.
pub fn leer(path: &Path) -> Result<String> {
    let p = Paquete::abrir(path)?;
    let xml = p.texto("word/document.xml").ok_or_else(|| anyhow!("{} no tiene word/document.xml: no es un .docx", path.display()))?;
    let d = Doc::leer(&xml)?;
    let cuerpo = d.raiz.hijo("body").ok_or_else(|| anyhow_tr!("documento sin <w:body>", "document without <w:body>"))?;
    let mut out = String::new();
    let mut n = 0;
    for b in cuerpo.elems() {
        match b.local() {
            "p" => {
                n += 1;
                let t = texto_parrafo(b);
                if t.trim().is_empty() {
                    continue;
                }
                let est = estilo_de(b);
                let pre = if let Some(h) = nivel_titulo(&est) {
                    format!("{} ", "#".repeat(h))
                } else if b.hijo("pPr").and_then(|x| x.hijo("numPr")).is_some() {
                    "- ".into()
                } else {
                    String::new()
                };
                out.push_str(&format!("¶{n} {pre}{t}\n"));
            }
            "tbl" => {
                out.push('\n');
                for (i, tr) in b.elems().filter(|e| e.local() == "tr").enumerate() {
                    let celdas: Vec<String> = tr.elems().filter(|e| e.local() == "tc").map(|tc| tc.elems().filter(|e| e.local() == "p").map(texto_parrafo).collect::<Vec<_>>().join(" ").replace('|', "\\|")).collect();
                    out.push_str(&format!("| {} |\n", celdas.join(" | ")));
                    if i == 0 {
                        out.push_str(&format!("|{}\n", "---|".repeat(celdas.len())));
                    }
                }
                out.push('\n');
            }
            _ => {}
        }
    }
    let palabras = out.split_whitespace().filter(|w| !w.starts_with('¶') && *w != "|" && *w != "-").count();
    Ok(crate::tr!("{} · {n} párrafos · ~{palabras} palabras\n\n{out}", "{} · {n} paragraphs · ~{palabras} words\n\n{out}", path.file_name().unwrap_or_default().to_string_lossy()))
}

// ── unir fragmentos (merge_runs.py) ──────────────────────────────────────────────────────────

/// Word parte el texto en muchos `<w:r>` (revisiones, ortografía): la frase que ves no está
/// seguida en el XML. Quita `<w:proofErr>`, los `rsid*` de los runs, une runs CONTIGUOS con
/// `<w:rPr>` IDÉNTICO y une sus `<w:t>`. No une runs de `<w:ins>`/`<w:del>` distintos (cada
/// contenedor por separado). Devuelve cuántos runs unió.
pub fn unir_fragmentos(e: &mut Elem) -> usize {
    let mut n = 0;
    e.hijos.retain(|h| !matches!(h, Nodo::Elem(x) if x.local() == "proofErr"));
    for h in e.hijos.iter_mut() {
        if let Nodo::Elem(x) = h {
            if x.local() == "r" {
                x.attrs.retain(|(k, _)| !k.to_lowercase().contains("rsid"));
            }
            n += unir_fragmentos(x);
        }
    }
    // unir runs hermanos contiguos (sólo espacios entre ellos) con el mismo rPr
    let mut i = 0;
    while i < e.hijos.len() {
        let es_run = |h: &Nodo| matches!(h, Nodo::Elem(x) if x.local() == "r");
        if !es_run(&e.hijos[i]) {
            i += 1;
            continue;
        }
        // siguiente elemento (saltando espacios)
        let mut j = i + 1;
        while j < e.hijos.len() && matches!(&e.hijos[j], Nodo::Crudo(s) if s.trim().is_empty() && !s.starts_with('<')) {
            j += 1;
        }
        if j < e.hijos.len() && es_run(&e.hijos[j]) {
            let (a, b) = match (&e.hijos[i], &e.hijos[j]) {
                (Nodo::Elem(a), Nodo::Elem(b)) => (a.clone(), b.clone()),
                _ => unreachable!(),
            };
            let rpr = |x: &Elem| x.hijo("rPr").map(|r| {
                let mut s = String::new();
                r.escribir(&mut s);
                s
            });
            if rpr(&a) == rpr(&b) {
                let mut unido = a;
                unido.vacio = false;
                unido.hijos.extend(b.hijos.into_iter().filter(|h| !matches!(h, Nodo::Elem(x) if x.local() == "rPr")));
                juntar_textos(&mut unido);
                e.hijos[i] = Nodo::Elem(unido);
                e.hijos.drain(i + 1..=j);
                n += 1;
                continue;
            }
        }
        if let Nodo::Elem(x) = &mut e.hijos[i] {
            juntar_textos(x);
        }
        i += 1;
    }
    n
}

/// Dentro de un run: `<w:t>` (o `<w:delText>`) seguidos → uno solo; `xml:space="preserve"` si
/// quedan espacios en los bordes. Sin `preserve`, Word recorta los bordes: se recortan aquí también.
fn juntar_textos(r: &mut Elem) {
    for tag in ["t", "delText"] {
        let mut i = 0;
        while i < r.hijos.len() {
            let es = |h: &Nodo| matches!(h, Nodo::Elem(x) if x.local() == tag);
            if es(&r.hijos[i]) && i + 1 < r.hijos.len() && es(&r.hijos[i + 1]) {
                let (a, b) = match (&r.hijos[i], &r.hijos[i + 1]) {
                    (Nodo::Elem(a), Nodo::Elem(b)) => (a.clone(), b.clone()),
                    _ => unreachable!(),
                };
                let visible = |x: &Elem| if x.attr("xml:space").as_deref() == Some("preserve") { x.texto() } else { x.texto().trim().to_string() };
                let t = visible(&a) + &visible(&b);
                let mut nuevo = a.clone();
                nuevo.quitar_attr("xml:space");
                if t.starts_with(char::is_whitespace) || t.ends_with(char::is_whitespace) {
                    nuevo.poner_attr("xml:space", "preserve");
                }
                nuevo.poner_texto(&t);
                r.hijos[i] = Nodo::Elem(nuevo);
                r.hijos.remove(i + 1);
                continue;
            }
            i += 1;
        }
    }
}

// ── editar ───────────────────────────────────────────────────────────────────────────────────

/// Reemplaza `viejo` por `nuevo` en los párrafos de `e` (aunque cruce varios runs: el texto nuevo
/// queda en el formato del run donde EMPIEZA). Devuelve cuántas veces.
fn reemplazar_en(e: &mut Elem, viejo: &str, nuevo: &str, todas: bool) -> usize {
    let mut n = 0;
    for h in e.hijos.iter_mut() {
        let Nodo::Elem(x) = h else { continue };
        if x.local() == "p" {
            while texto_parrafo(x).contains(viejo) {
                if !reemplazar_en_parrafo(x, viejo, nuevo) {
                    break;
                }
                n += 1;
                if !todas {
                    return n;
                }
            }
        } else {
            n += reemplazar_en(x, viejo, nuevo, todas);
            if n > 0 && !todas {
                return n;
            }
        }
    }
    n
}

/// Los `<w:t>` del párrafo en orden (camino de índices desde el párrafo).
fn caminos_t(e: &Elem, base: &mut Vec<usize>, out: &mut Vec<Vec<usize>>) {
    for (i, h) in e.hijos.iter().enumerate() {
        if let Nodo::Elem(x) = h {
            if matches!(x.local(), "del" | "delText" | "pPr" | "rPr") { // juez-idioma: datos (etiquetas XML de Word)
                continue;
            }
            base.push(i);
            if x.local() == "t" {
                out.push(base.clone());
            } else {
                caminos_t(x, base, out);
            }
            base.pop();
        }
    }
}

fn en_camino<'a>(e: &'a mut Elem, c: &[usize]) -> &'a mut Elem {
    let mut x = e;
    for &i in c {
        x = match &mut x.hijos[i] {
            Nodo::Elem(h) => h,
            _ => unreachable!(),
        };
    }
    x
}

fn reemplazar_en_parrafo(p: &mut Elem, viejo: &str, nuevo: &str) -> bool {
    let mut caminos = vec![];
    caminos_t(p, &mut vec![], &mut caminos);
    let textos: Vec<String> = caminos.iter().map(|c| en_camino(p, c).texto()).collect();
    let todo: String = textos.concat();
    let Some(ini) = todo.find(viejo) else { return false };
    let fin = ini + viejo.len();
    let mut pos = 0;
    let mut puesto = false;
    for (k, c) in caminos.iter().enumerate() {
        let (a, b) = (pos, pos + textos[k].len());
        pos = b;
        if b <= ini || a >= fin {
            continue;
        }
        let t = &textos[k];
        let corte_ini = ini.saturating_sub(a).min(t.len());
        let corte_fin = (fin - a).min(t.len());
        let mut nuevo_t = t[..corte_ini].to_string();
        if !puesto {
            nuevo_t.push_str(nuevo);
            puesto = true;
        }
        nuevo_t.push_str(&t[corte_fin..]);
        let el = en_camino(p, c);
        el.poner_texto(&nuevo_t);
        el.quitar_attr("xml:space");
        if nuevo_t.starts_with(char::is_whitespace) || nuevo_t.ends_with(char::is_whitespace) {
            el.poner_attr("xml:space", "preserve");
        }
    }
    true
}

/// EDITAR un .docx existente: unir fragmentos y luego, en orden, cada cambio de la lista:
/// `{"reemplazar": "texto viejo", "por": "texto nuevo", "todas"?: false}` (por omisión TODAS las veces,
/// como en PowerPoint: medido en el banco, cambiar sólo la primera dejaba un `{{cliente}}` sin llenar),
/// `{"agregar": [bloques como en crear], "despues_de"?: "texto de un párrafo"}` (sin `despues_de`:
/// al final), `{"borrar": "texto de un párrafo"}`. Respaldo antes de escribir. Devuelve el informe.
pub fn editar(path: &Path, cambios: &Value) -> Result<String> {
    let cambios = &crate::ooxml::tolerante_o_error(cambios, "cambios")?;
    let cambios = cambios.as_array().filter(|c| !c.is_empty()).ok_or_else(|| anyhow_tr!(
            "`cambios` es una lista: [{{\"reemplazar\":\"…\",\"por\":\"…\"}}, {{\"agregar\":[…]}}, {{\"borrar\":\"…\"}}]",
            "`cambios` is a list: [{{\"reemplazar\":\"…\",\"por\":\"…\"}}, {{\"agregar\":[…]}}, {{\"borrar\":\"…\"}}]"
        ))?;
    let mut p = Paquete::abrir(path)?;
    let xml = p.texto("word/document.xml").ok_or_else(|| anyhow!("no es un .docx (falta word/document.xml)"))?;
    let mut d = Doc::leer(&xml)?;
    let unidos = unir_fragmentos(&mut d.raiz);
    let mut informe = vec![format!("{unidos} fragmentos unidos")];
    let ancho = ancho_util(&d.raiz);
    let estilos_doc = p.texto("word/styles.xml").unwrap_or_default();
    let mut necesita_numeracion = false;
    for c in cambios {
        if let Some(v) = c["reemplazar"].as_str() {
            let nuevo = c["por"].as_str().unwrap_or_default();
            let todas = c["todas"].as_bool().unwrap_or(true);
            let mut n = reemplazar_en(&mut d.raiz, v, nuevo, todas);
            if n == 0 {
                // copiado de `leer` con sus marcas («# Título», «¶3 …», «- punto»): sin ellas
                if let Some(limpio) = crate::ooxml::sin_marcas_de_leer(v) {
                    let nuevo2 = crate::ooxml::sin_marcas_de_leer(nuevo).unwrap_or_else(|| nuevo.to_string());
                    n = reemplazar_en(&mut d.raiz, &limpio, &nuevo2, todas);
                }
            }
            if n == 0 {
                bail_tr!("no encontré «{v}» en el documento (lee el documento y copia el texto exacto)", "could not find «{v}» in the document (read the document and copy the exact text)");
            }
            informe.push(format!("«{v}» → «{nuevo}» ({n} vez/veces)"));
        } else if let Some(bloques) = c["agregar"].as_array() {
            let mut g = Gen { ancho, numeradas: 100, tema: None };
            let mut xml_nuevo = String::new();
            for b in bloques {
                xml_nuevo.push_str(&g.bloque(b)?);
                necesita_numeracion |= b.get("lista").is_some() || b.get("numerada").is_some();
            }
            // los estilos de título que el documento no tenga → formato directo
            let xml_nuevo = adaptar_estilos(&xml_nuevo, &estilos_doc);
            let envoltura = Doc::leer(&format!("<w:body xmlns:w=\"{W}\">{xml_nuevo}</w:body>"))?;
            let nuevos: Vec<Nodo> = envoltura.raiz.hijos;
            let cuerpo = cuerpo_mut(&mut d.raiz)?;
            let pos = match c["despues_de"].as_str() {
                Some(t) => {
                    cuerpo.hijos.iter().position(|h| matches!(h, Nodo::Elem(x) if x.local() == "p" && texto_parrafo(x).contains(t))).ok_or_else(|| anyhow_tr!("no hay un párrafo con «{t}» para agregar después", "there is no paragraph with «{t}» to add after"))? + 1
                }
                // al final, ANTES de <w:sectPr> (que debe ser el último)
                None => cuerpo.hijos.iter().rposition(|h| matches!(h, Nodo::Elem(x) if x.local() == "sectPr")).unwrap_or(cuerpo.hijos.len()),
            };
            let n = nuevos.len();
            for (k, nodo) in nuevos.into_iter().enumerate() {
                cuerpo.hijos.insert(pos + k, nodo);
            }
            informe.push(format!("{} bloque(s) agregados ({n} elementos)", bloques.len()));
        } else if let Some(t) = c["borrar"].as_str() {
            let cuerpo = cuerpo_mut(&mut d.raiz)?;
            let antes = cuerpo.hijos.len();
            cuerpo.hijos.retain(|h| !matches!(h, Nodo::Elem(x) if x.local() == "p" && texto_parrafo(x).contains(t)));
            let n = antes - cuerpo.hijos.len();
            if n == 0 {
                bail_tr!("no hay un párrafo con «{t}» para borrar", "there is no paragraph with «{t}» to delete");
            }
            informe.push(crate::tr!("{n} párrafo(s) borrados", "{n} paragraph(s) deleted"));
        } else {
            bail!("cambio desconocido {c}: usa reemplazar/por, agregar (+despues_de) o borrar");
        }
    }
    p.poner("word/document.xml", d.escribir());
    if necesita_numeracion {
        asegurar_numeracion(&mut p)?;
    }
    let errores = crate::ooxml::validar_paquete(&p);
    if !errores.is_empty() {
        bail_tr!("el resultado no pasa la validación (no lo guardé): {}", "the result fails validation (not saved): {}", errores.join("; "));
    }
    let r = crate::ooxml::respaldar(path)?;
    p.guardar(path)?;
    informe.push(format!("respaldo: {}", r.file_name().unwrap_or_default().to_string_lossy()));
    Ok(informe.join(" · "))
}

fn cuerpo_mut(raiz: &mut Elem) -> Result<&mut Elem> {
    raiz.hijos
        .iter_mut()
        .find_map(|h| if let Nodo::Elem(x) = h { (x.local() == "body").then_some(x) } else { None })
        .ok_or_else(|| anyhow_tr!("documento sin <w:body>", "document without <w:body>"))
}

/// Ancho útil de la página (para las tablas que se agregan).
fn ancho_util(raiz: &Elem) -> u32 {
    let mut s = vec![];
    raiz.buscar("sectPr", &mut s);
    let Some(sp) = s.last() else { return 9026 };
    let w: u32 = sp.hijo("pgSz").and_then(|x| x.attr("w:w")).and_then(|v| v.parse().ok()).unwrap_or(11906);
    let m = |k: &str| sp.hijo("pgMar").and_then(|x| x.attr(k)).and_then(|v| v.parse::<u32>().ok()).unwrap_or(1440);
    w.saturating_sub(m("w:left") + m("w:right")).max(2000)
}

/// Un documento ajeno puede no tener `Heading1`/`Title`/`ListParagraph`/`TableGrid` (Word en
/// español usa `Ttulo1`): si el estilo no existe, se quita la referencia y se pone formato directo.
fn adaptar_estilos(xml: &str, estilos: &str) -> String {
    let mut s = xml.to_string();
    for (id, directo) in [("Title", "<w:b/><w:sz w:val=\"52\"/>"), ("Heading1", "<w:b/><w:sz w:val=\"32\"/>"), ("Heading2", "<w:b/><w:sz w:val=\"26\"/>"), ("Heading3", "<w:b/><w:sz w:val=\"24\"/>")] {
        if !estilos.contains(&format!("w:styleId=\"{id}\"")) {
            s = s.replace(&format!("<w:pStyle w:val=\"{id}\"/>"), "<w:keepNext/>");
            // formato directo en sus runs: se marca y se inyecta en cada <w:r>
            s = s.replace("<w:pPr><w:keepNext/></w:pPr><w:r>", &format!("<w:pPr><w:keepNext/></w:pPr><w:r><w:rPr>{directo}</w:rPr>"));
        }
    }
    for id in ["ListParagraph", "TableGrid"] {
        if !estilos.contains(&format!("w:styleId=\"{id}\"")) {
            s = s.replace(&format!("<w:pStyle w:val=\"{id}\"/>"), "").replace(&format!("<w:tblStyle w:val=\"{id}\"/>"), "<w:tblBorders><w:top w:val=\"single\" w:sz=\"4\" w:space=\"0\" w:color=\"BFBFBF\"/><w:left w:val=\"single\" w:sz=\"4\" w:space=\"0\" w:color=\"BFBFBF\"/><w:bottom w:val=\"single\" w:sz=\"4\" w:space=\"0\" w:color=\"BFBFBF\"/><w:right w:val=\"single\" w:sz=\"4\" w:space=\"0\" w:color=\"BFBFBF\"/><w:insideH w:val=\"single\" w:sz=\"4\" w:space=\"0\" w:color=\"BFBFBF\"/><w:insideV w:val=\"single\" w:sz=\"4\" w:space=\"0\" w:color=\"BFBFBF\"/></w:tblBorders>");
        }
    }
    s
}

/// Las listas agregadas usan numId 1 (viñetas) y 101.. (numeradas): se añaden al numbering.xml del
/// documento (o se crea, con su relación y su tipo) con ids que no choquen.
fn asegurar_numeracion(p: &mut Paquete) -> Result<()> {
    let doc = p.texto("word/document.xml").unwrap_or_default();
    let usados: Vec<u32> = (101..200).filter(|i| doc.contains(&format!("<w:numId w:val=\"{i}\"/>"))).collect();
    let necesita_vinetas = doc.contains("<w:numId w:val=\"1\"/>");
    match p.texto("word/numbering.xml") {
        None => {
            // uno nuevo: viñetas en num 1 y las numeradas en sus ids
            let mut n = numeracion(0);
            let extra: String = usados.iter().map(|i| format!("<w:num w:numId=\"{i}\"><w:abstractNumId w:val=\"1\"/><w:lvlOverride w:ilvl=\"0\"><w:startOverride w:val=\"1\"/></w:lvlOverride></w:num>")).collect();
            n = n.replace("</w:numbering>", &format!("{extra}</w:numbering>"));
            p.poner("word/numbering.xml", n);
            let rels = p.texto("word/_rels/document.xml.rels").unwrap_or_default();
            let id = (1..999).map(|i| format!("rIdGotv{i}")).find(|i| !rels.contains(i.as_str())).unwrap();
            p.poner("word/_rels/document.xml.rels", rels.replace("</Relationships>", &format!("<Relationship Id=\"{id}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/numbering\" Target=\"numbering.xml\"/></Relationships>")));
            let ct = p.texto("[Content_Types].xml").unwrap_or_default();
            if !ct.contains("/word/numbering.xml") {
                p.poner("[Content_Types].xml", ct.replace("</Types>", "<Override PartName=\"/word/numbering.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.numbering+xml\"/></Types>"));
            }
        }
        Some(n) => {
            // uno existente: abstractNums propios con ids libres, y los num que faltan
            let libre = |pref: &str, desde: u32| (desde..9999).find(|i| !n.contains(&format!("{pref}=\"{i}\""))).unwrap();
            let a_vin = libre("w:abstractNumId", 900);
            let a_num = libre("w:abstractNumId", a_vin + 1);
            let nivel = |i: u32, fmt: &str, txt: &str| format!("<w:lvl w:ilvl=\"{i}\"><w:start w:val=\"1\"/><w:numFmt w:val=\"{fmt}\"/><w:lvlText w:val=\"{txt}\"/><w:lvlJc w:val=\"left\"/><w:pPr><w:ind w:left=\"{}\" w:hanging=\"360\"/></w:pPr></w:lvl>", 720 * (i + 1));
            let abstractos = format!(
                "<w:abstractNum w:abstractNumId=\"{a_vin}\"><w:multiLevelType w:val=\"hybridMultilevel\"/>{}{}</w:abstractNum><w:abstractNum w:abstractNumId=\"{a_num}\"><w:multiLevelType w:val=\"hybridMultilevel\"/>{}{}</w:abstractNum>",
                nivel(0, "bullet", "\u{2022}"), nivel(1, "bullet", "\u{25E6}"), nivel(0, "decimal", "%1."), nivel(1, "lowerLetter", "%2)")
            );
            // los abstractNum van ANTES de los num (orden del esquema)
            let mut n2 = match n.find("<w:num ") {
                Some(i) => format!("{}{abstractos}{}", &n[..i], &n[i..]),
                None => n.replace("</w:numbering>", &format!("{abstractos}</w:numbering>")),
            };
            // num 1 del documento puede existir ya (de otra lista): las viñetas nuevas usan uno libre
            let mut doc2 = doc.clone();
            let mut nums = String::new();
            if necesita_vinetas {
                let id = libre("w:numId", 900);
                doc2 = doc2.replace("<w:numId w:val=\"1\"/>", &format!("<w:numId w:val=\"{id}\"/>"));
                nums.push_str(&format!("<w:num w:numId=\"{id}\"><w:abstractNumId w:val=\"{a_vin}\"/></w:num>"));
            }
            for u in &usados {
                let id = (u + 900..99999).find(|i| !n2.contains(&format!("w:numId=\"{i}\"")) && !nums.contains(&format!("w:numId=\"{i}\""))).unwrap();
                doc2 = doc2.replace(&format!("<w:numId w:val=\"{u}\"/>"), &format!("<w:numId w:val=\"{id}\"/>"));
                nums.push_str(&format!("<w:num w:numId=\"{id}\"><w:abstractNumId w:val=\"{a_num}\"/><w:lvlOverride w:ilvl=\"0\"><w:startOverride w:val=\"1\"/></w:lvlOverride></w:num>"));
            }
            n2 = n2.replace("</w:numbering>", &format!("{nums}</w:numbering>"));
            p.poner("word/numbering.xml", n2);
            p.poner("word/document.xml", doc2);
        }
    }
    Ok(())
}

// ── revisar ──────────────────────────────────────────────────────────────────────────────────

/// REVISAR (su `validators/docx.py`): paquete (tipos, relaciones, XML), `paraId` hex < 0x80000000,
/// `durableId` < 0x7FFFFFFF, `styleId` sin repetir, espacios en los bordes sin `preserve`,
/// marcas de comentario emparejadas. Devuelve (errores, avisos).
pub fn revisar(path: &Path) -> Result<(Vec<String>, Vec<String>)> {
    let p = Paquete::abrir(path)?;
    let mut errores = crate::ooxml::validar_paquete(&p);
    let mut avisos = vec![];
    for (n, b) in &p.partes {
        if !n.starts_with("word/") || !n.ends_with(".xml") {
            continue;
        }
        let x = String::from_utf8_lossy(b);
        let Ok(d) = Doc::leer(&x) else { continue };
        let mut todos = vec![];
        d.raiz.buscar("p", &mut todos);
        for e in &todos {
            if let Some(v) = e.attr("w14:paraId") {
                match u32::from_str_radix(&v, 16) {
                    Ok(i) if i >= 0x8000_0000 => errores.push(format!("{n}: paraId={v} ≥ 0x80000000")),
                    Err(_) => errores.push(format!("{n}: paraId={v} no es hex")),
                    _ => {}
                }
            }
        }
        if n == "word/styles.xml" {
            let mut st = vec![];
            d.raiz.buscar("style", &mut st);
            let mut vistos = std::collections::HashSet::new();
            for s in st {
                if let Some(id) = s.attr("w:styleId") {
                    if !vistos.insert(id.clone()) {
                        errores.push(format!("styles.xml: styleId «{id}» repetido"));
                    }
                }
            }
        }
        let mut ts = vec![];
        d.raiz.buscar("t", &mut ts);
        let sin_preserve = ts.iter().filter(|t| {
            let s = t.texto();
            (s.starts_with(char::is_whitespace) || s.ends_with(char::is_whitespace)) && t.attr("xml:space").as_deref() != Some("preserve")
        }).count();
        if sin_preserve > 0 {
            avisos.push(crate::tr!("{n}: {sin_preserve} texto(s) con espacios en los bordes sin xml:space=\"preserve\" (Word los recorta)", "{n}: {sin_preserve} text(s) with edge spaces without xml:space=\"preserve\" (Word trims them)"));
        }
        if n == "word/document.xml" {
            let (mut a, mut c) = (vec![], vec![]);
            d.raiz.buscar("commentRangeStart", &mut a);
            d.raiz.buscar("commentRangeEnd", &mut c);
            if a.len() != c.len() {
                errores.push(format!("document.xml: {} inicios de comentario y {} finales", a.len(), c.len()));
            }
            // juicio del Portador (2026-11-11): una carta de una hoja salió en DOS por un salto de
            // página que metió el modelo. Un salto en un documento que cabe en una hoja se avisa.
            let mut brs = vec![];
            d.raiz.buscar("br", &mut brs);
            let saltos = brs.iter().filter(|b| b.attr("w:type").as_deref() == Some("page")).count();
            if saltos > 0 {
                // ~90 caracteres por línea de 11 pt en una hoja carta; un párrafo vacío es 1 línea
                let lineas: usize = todos.iter().map(|p| texto_parrafo(p).chars().count().div_ceil(90).max(1) + 1).sum();
                if lineas < 45 {
                    avisos.push(crate::tr!("document.xml: {saltos} salto(s) de página en un documento de ~{lineas} líneas, que cabe en UNA hoja: quítalo (`borrar` el párrafo del salto) o, si es una carta, créala con la ficha «carta»", "document.xml: {saltos} page break(s) in a ~{lineas}-line document that fits on ONE page: remove it (`borrar` the break paragraph) or, if it is a letter, create it with the «carta» spec"));
                }
            }
        }
    }
    Ok((errores, avisos))
}

#[cfg(test)]
mod pruebas {
    use super::*;
    use serde_json::json;

    fn ficha() -> Value {
        json!({"pagina": "carta", "bloques": [
            {"titulo": "Carta de tecnología"},
            {"parrafo": "Bogotá, 11 de noviembre de 2026", "alineacion": "derecha"},
            {"h1": "Propuesta"},
            {"parrafo": "Estimado cliente:\nLe presentamos **tres** servicios *nuevos*."},
            {"lista": ["Software a medida", "Consultoría", ["Nube", "Seguridad"]]},
            {"numerada": ["Diagnóstico", "Plan", "Ejecución"]},
            {"tabla": [["Servicio", "Precio"], ["Software", "$10.000.000"], ["Consultoría", "$2.000.000"]], "anchos": [2, 1]},
            {"linea": true},
            {"salto": true},
            {"parrafo": "Atentamente,\nEquipo GOTV"}
        ]})
    }

    /// Crear → el paquete valida, el XML lleva lo que pide la guía, y leer devuelve el contenido.
    #[test]
    fn crear_y_leer() {
        let d = crate::CarpetaPrueba::nueva("gotv-word");
        let p = d.join("carta.docx");
        crear(&p, &ficha()).unwrap();
        let pq = Paquete::abrir(&p).unwrap();
        assert!(crate::ooxml::validar_paquete(&pq).is_empty(), "{:?}", crate::ooxml::validar_paquete(&pq));
        let x = pq.texto("word/document.xml").unwrap();
        assert!(!x.contains('\u{2022}'), "nunca • en el texto (va en la numeración)");
        assert!(x.contains("<w:numId w:val=\"1\"/>") && x.contains("<w:numId w:val=\"2\"/>"), "viñetas y numerada");
        assert!(x.contains("w:val=\"clear\"") && !x.contains("w:val=\"solid\""), "sombreado CLEAR");
        assert!(x.contains("<w:pgSz w:w=\"12240\" w:h=\"15840\"/>"), "carta");
        // anchos dobles que suman el ancho útil (12240 − 2·1440 = 9360)
        assert!(x.contains("<w:tblW w:w=\"9360\" w:type=\"dxa\"/>") && x.contains("<w:gridCol w:w=\"6240\"/><w:gridCol w:w=\"3120\"/>") && x.contains("<w:tcW w:w=\"6240\" w:type=\"dxa\"/>"));
        assert!(x.contains("<w:r><w:br w:type=\"page\"/></w:r></w:p>"), "salto dentro de un párrafo");
        let st = pq.texto("word/styles.xml").unwrap();
        for h in ["Heading1", "Heading2", "Heading3"] {
            let i = st.find(&format!("w:styleId=\"{h}\"")).unwrap();
            let bloque = &st[i..i + 600];
            assert!(bloque.contains("<w:keepNext/>") && bloque.contains("w:before=") && bloque.contains("w:after=") && bloque.contains("w:rFonts"), "{h}: fuente, antes/después y keepNext");
        }
        let l = leer(&p).unwrap();
        println!("{l}");
        assert!(l.contains("# Carta de tecnología") && l.contains("# Propuesta"));
        assert!(l.contains("Le presentamos tres servicios nuevos."), "la negrita no deja asteriscos");
        assert!(l.contains("- Software a medida") && l.contains("| Software | $10.000.000 |"));
        let (e, a) = revisar(&p).unwrap();
        assert!(e.is_empty() && a.is_empty(), "{e:?} {a:?}");
    }

    /// Unir fragmentos: la frase partida por Word (rsid, proofErr, runs iguales) se vuelve buscable
    /// sin cambiar lo que se ve; los runs con formato distinto NO se unen.
    #[test]
    fn unir_fragmentos_como_merge_runs() {
        let x = format!("<w:document xmlns:w=\"{W}\"><w:body><w:p><w:r w:rsidR=\"00A1\"><w:t>Hola </w:t></w:r><w:proofErr w:type=\"spellStart\"/><w:r w:rsidR=\"00B2\"><w:t xml:space=\"preserve\">mun</w:t></w:r><w:r><w:t>do</w:t></w:r><w:r><w:rPr><w:b/></w:rPr><w:t>!</w:t></w:r></w:p></w:body></w:document>");
        let mut d = Doc::leer(&x).unwrap();
        let n = unir_fragmentos(&mut d.raiz);
        let s = d.escribir();
        assert_eq!(n, 2, "{s}");
        assert!(!s.contains("rsid") && !s.contains("proofErr"), "{s}");
        // «Hola » sin preserve → Word lo dibuja «Hola» (recorta): unido queda «Holamundo»
        assert!(s.contains("<w:t>Holamundo</w:t>"), "{s}");
        assert!(s.contains("<w:rPr><w:b/></w:rPr><w:t>!</w:t>"), "el run en negrita queda aparte: {s}");
    }

    /// Lo copiado de `leer` CON sus marcas (banco of-plantilla) se halla igual.
    #[test]
    fn reemplazar_lo_copiado_de_leer_con_sus_marcas() {
        let d = crate::CarpetaPrueba::nueva("gotv-word-marcas");
        let p = d.join("c.docx");
        crear(&p, &json!({"bloques": [{"titulo": "Contrato de suministro de café"}, {"lista": ["Entrega mensual"]}, {"parrafo": "Firma: **{{cliente}}**"}]})).unwrap();
        editar(&p, &json!([
            {"reemplazar": "# Contrato de suministro de café", "por": "# Contrato de suministro para Ana"},
            {"reemplazar": "- Entrega mensual", "por": "- Entrega quincenal"}
        ])).unwrap();
        let l = leer(&p).unwrap();
        assert!(l.contains("Contrato de suministro para Ana") && !l.contains("# # ") && l.contains("Entrega quincenal"), "{l}");
        assert_eq!(crate::ooxml::sin_marcas_de_leer("¶3 Firma: **Ana**").as_deref(), Some("Firma: Ana"));
        assert_eq!(crate::ooxml::sin_marcas_de_leer("2. Avances en medicina").as_deref(), Some("Avances en medicina"));
        assert_eq!(crate::ooxml::sin_marcas_de_leer("Texto normal"), None);
        assert!(editar(&p, &json!([{"reemplazar": "# No existe", "por": "x"}])).is_err());
    }

    /// Las formas que el 9B escribió de verdad (2026-11-11) se aceptan: bloques {tipo,texto} y
    /// {bloque,texto}, y `cambios` como TEXTO con «,,» dobles.
    #[test]
    fn acepta_las_formas_del_9b() {
        assert_eq!(normalizar_bloque(&json!({"texto": "P.D.", "tipo": "parrafo"})), json!({"parrafo": "P.D."}));
        assert_eq!(normalizar_bloque(&json!({"bloque": "parrafo", "texto": "P.D."})), json!({"parrafo": "P.D."}));
        assert_eq!(normalizar_bloque(&json!({"type": "heading", "level": 2, "text": "Plazos"})), json!({"h2": "Plazos"}));
        assert_eq!(normalizar_bloque(&json!("suelto")), json!({"parrafo": "suelto"}));
        let d = crate::CarpetaPrueba::nueva("gotv-word-9b");
        let p = d.join("carta.docx");
        crear(&p, &json!({"bloques": [{"tipo": "titulo", "texto": "Carta"}, {"parrafo": "Estimado/a destinatario/a,"}]})).unwrap();
        let del_9b = json!("[{\"reemplazar\":\"Estimado/a destinatario/a,\",,\"por\":\"Estimada Laura,\",,\"todas\":false},{\"agregar\":[{\"bloque\":\"parrafo\",\"texto\":\"P.D.: vence el 30 de noviembre\"}]}]");
        editar(&p, &del_9b).unwrap();
        let l = leer(&p).unwrap();
        assert!(l.contains("Estimada Laura,") && l.contains("P.D.: vence el 30 de noviembre"), "{l}");
    }

    /// Editar: reemplazar aunque cruce runs (conservando formato del inicio), agregar después de un
    /// párrafo (con lista: numeración añadida sin chocar) y borrar; respaldo; el paquete valida.
    #[test]
    fn editar_conserva_y_valida() {
        let d = crate::CarpetaPrueba::nueva("gotv-word-editar");
        let p = d.join("carta.docx");
        crear(&p, &ficha()).unwrap();
        // una frase partida en dos runs con el MISMO formato (como deja Word)
        let mut pq = Paquete::abrir(&p).unwrap();
        let x = pq.texto("word/document.xml").unwrap().replace("<w:t>Estimado cliente:</w:t>", "<w:t xml:space=\"preserve\">Estimado </w:t></w:r><w:r w:rsidR=\"0011\"><w:t>cliente:</w:t>");
        pq.poner("word/document.xml", x);
        pq.guardar(&p).unwrap();
        let inf = editar(&p, &json!([
            {"reemplazar": "Estimado cliente:", "por": "Estimada Laura:"},
            {"agregar": [{"h2": "Plazos"}, {"numerada": ["Mes 1", "Mes 2"]}], "despues_de": "Propuesta"},
            {"borrar": "Equipo GOTV"}
        ])).unwrap();
        println!("{inf}");
        let l = leer(&p).unwrap();
        assert!(l.contains("Estimada Laura:") && !l.contains("Estimado cliente"), "{l}");
        let i_prop = l.find("# Propuesta").unwrap();
        let i_plaz = l.find("## Plazos").unwrap();
        let i_est = l.find("Estimada Laura").unwrap();
        assert!(i_prop < i_plaz && i_plaz < i_est, "agregado justo después de «Propuesta»: {l}");
        assert!(!l.contains("Equipo GOTV") && l.contains("Atentamente"), "{l}");
        assert!(inf.contains("respaldo"));
        let (e, _) = revisar(&p).unwrap();
        assert!(e.is_empty(), "{e:?}");
        let n = Paquete::abrir(&p).unwrap().texto("word/numbering.xml").unwrap();
        assert!(n.matches("<w:abstractNum ").count() == 4, "dos abstractNum propios añadidos a los dos de antes");
        assert!(editar(&p, &json!([{"reemplazar": "no existe", "por": "x"}])).is_err());
    }

    /// O4 · Con TEMA el documento va vestido: títulos con su tipografía y el color de marca, filete bajo
    /// el título, cabecera de tabla en el color de marca con su texto legible, filas alternas, y pie con
    /// «página / total». El paquete sigue validando y `leer` devuelve lo mismo que sin tema.
    #[test]
    fn con_tema_va_vestido_y_sigue_valido() {
        let d = crate::CarpetaPrueba::nueva("gotv-word-tema");
        let mut f = ficha();
        f["tema"] = json!({"marca": "azul", "fuentes": "editorial"});
        let (con, sin) = (d.join("con.docx"), d.join("sin.docx"));
        crear(&con, &f).unwrap();
        crear(&sin, &ficha()).unwrap();
        let t = crate::tema_doc::de(&f["tema"]).unwrap();
        let pq = Paquete::abrir(&con).unwrap();
        assert!(crate::ooxml::validar_paquete(&pq).is_empty(), "{:?}", crate::ooxml::validar_paquete(&pq));
        let (e, a) = revisar(&con).unwrap();
        assert!(e.is_empty() && a.is_empty(), "{e:?} {a:?}");
        let st = pq.texto("word/styles.xml").unwrap();
        let estilo = |id: &str| -> String { let i = st.find(&format!("w:styleId=\"{id}\"")).unwrap(); st[i..].split("</w:style>").next().unwrap().to_string() };
        assert!(estilo("Title").contains(&format!("w:color w:val=\"{}\"", t.titulo)) && estilo("Title").contains("w:ascii=\"Georgia\""), "título: color de marca y serif");
        assert!(estilo("Title").contains(&format!("<w:pBdr><w:bottom w:val=\"single\" w:sz=\"12\" w:space=\"8\" w:color=\"{}\"/></w:pBdr>", t.primario)), "filete bajo el título");
        assert!(estilo("Heading1").contains(&t.titulo) && estilo("Heading2").contains(&t.texto) && estilo("Heading3").contains(&t.tenue));
        assert!(estilo("Normal").contains(&format!("w:color w:val=\"{}\"", t.texto)) && estilo("Normal").contains("w:ascii=\"Georgia\""), "el par editorial es serif en título y texto");
        assert!(estilo("TablaCab").contains(&t.sobre_primario) && st.contains("w:styleId=\"Pie\""));
        let x = pq.texto("word/document.xml").unwrap();
        // la tabla: cabecera en el color de marca con su estilo; la 2.ª fila de datos, suave; la 1.ª, sin fondo
        assert!(x.contains(&format!("w:fill=\"{}\"/></w:tcPr><w:p><w:pPr><w:pStyle w:val=\"TablaCab\"/>", t.primario)), "cabecera");
        assert_eq!(x.matches(&format!("w:fill=\"{}\"", t.suave)).count(), 2, "una fila alterna de dos celdas");
        assert!(!x.contains("E7E6E6"), "el gris de la tabla sobria no aparece");
        assert!(x.contains(&format!("w:color=\"{}\"/></w:pBdr></w:pPr></w:p>", t.primario)), "la línea, en el color de marca");
        // el pie: su parte, su relación y su referencia en la sección
        assert!(x.contains("<w:sectPr><w:footerReference w:type=\"default\" r:id=\"rIdP1\"/>"));
        let pie = pq.texto("word/footer1.xml").unwrap();
        assert!(pie.contains(" PAGE ") && pie.contains(" NUMPAGES ") && pie.contains("<w:pStyle w:val=\"Pie\"/>"));
        assert!(pq.texto("word/_rels/document.xml.rels").unwrap().contains("Target=\"footer1.xml\"") && pq.texto("[Content_Types].xml").unwrap().contains("/word/footer1.xml"));
        // sin tema, nada de esto (y `numeros_pagina` lo pone aparte)
        let ps = Paquete::abrir(&sin).unwrap();
        assert!(ps.texto("word/footer1.xml").is_none() && ps.texto("word/document.xml").unwrap().contains("E7E6E6") && !ps.texto("word/styles.xml").unwrap().contains("TablaCab"));
        let mut n = ficha();
        n["numeros_pagina"] = json!(true);
        crear(&d.join("num.docx"), &n).unwrap();
        assert!(Paquete::abrir(&d.join("num.docx")).unwrap().texto("word/footer1.xml").is_some());
        f["numeros_pagina"] = json!(false);
        crear(&d.join("sinpie.docx"), &f).unwrap();
        assert!(Paquete::abrir(&d.join("sinpie.docx")).unwrap().texto("word/footer1.xml").is_none());
        // lo que se LEE no cambia con el vestido
        let cuerpo = |p: &Path| leer(p).unwrap().split_once('\n').map(|(_, c)| c.to_string()).unwrap_or_default();
        assert_eq!(cuerpo(&con), cuerpo(&sin));
        // una paleta de las presentaciones: el informe a juego con el deck
        f["tema"] = json!("oceano");
        crear(&d.join("oceano.docx"), &f).unwrap();
        assert!(Paquete::abrir(&d.join("oceano.docx")).unwrap().texto("word/document.xml").unwrap().contains("w:fill=\"065A82\""));
    }
}

#[cfg(test)]
mod con_word {
    use super::*;
    use serde_json::json;

    /// El juez de verdad: el Word del usuario ABRE lo creado y lo editado (si estuviera roto pediría
    /// «reparar» y la conversión fallaría) y lo convierte a PDF con todas sus páginas.
    /// `cargo test -p gotv-core --release -- --ignored --nocapture word_abre_lo_creado`
    #[tokio::test]
    #[ignore]
    async fn word_abre_lo_creado() {
        let d = crate::CarpetaPrueba::nueva("gotv-word-real");
        let p = d.join("carta.docx");
        crear(&p, &json!({"pagina": "carta", "bloques": [
            {"titulo": "Carta de tecnología"}, {"h1": "Propuesta"}, {"parrafo": "Le presentamos **tres** servicios."},
            {"lista": ["Software", "Consultoría"]}, {"numerada": ["Uno", "Dos"]},
            {"tabla": [["Servicio", "Precio"], ["Software", "$10"]]}, {"salto": true}, {"parrafo": "Página dos."}
        ]})).unwrap();
        crate::apps::exportar_pdf(&p, &d.join("carta.pdf"), 120).await.unwrap();
        let pdf = std::fs::read(d.join("carta.pdf")).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
        let paginas = String::from_utf8_lossy(&pdf).matches("/Type /Page").count() + String::from_utf8_lossy(&pdf).matches("/Type/Page").count();
        println!("creado → PDF de {} bytes, ~{paginas} marcas de página", pdf.len());
        editar(&p, &json!([{"reemplazar": "tres", "por": "cuatro"}, {"agregar": [{"lista": ["Nube"]}], "despues_de": "Propuesta"}])).unwrap();
        crate::apps::exportar_pdf(&p, &d.join("editada.pdf"), 120).await.unwrap();
        assert!(std::fs::read(d.join("editada.pdf")).unwrap().starts_with(b"%PDF"), "Word abre también lo editado");
    }

    /// O4 · El Word del usuario abre el documento CON TEMA (estilos nuevos, tabla vestida y pie con
    /// campos PAGE/NUMPAGES) y lo convierte a PDF. `GOTV_PDF_SALIDA` deja una copia para mirarla.
    /// `cargo test -p gotv-core --release -- --ignored --nocapture word_abre_lo_tematizado`
    #[tokio::test]
    #[ignore]
    async fn word_abre_lo_tematizado() {
        let d = crate::CarpetaPrueba::nueva("gotv-word-tema-real");
        let p = d.join("informe.docx");
        crear(&p, &json!({"pagina": "carta", "tema": {"marca": "verde", "fuentes": "editorial"}, "bloques": [
            {"titulo": "Informe anual"}, {"parrafo": "Bogotá, 14 de noviembre de 2026", "alineacion": "derecha"},
            {"h1": "Resumen"}, {"parrafo": "Crecimos **un 12 %** y *sin deuda*."},
            {"h2": "Líneas"}, {"lista": ["Ventas", ["Tienda", "Web"], "Costos"]}, {"numerada": ["Diagnóstico", "Plan"]},
            {"tabla": [["Línea", "Total"], ["Tienda", "$10"], ["Web", "$5"], ["Mayoristas", "$8"]], "anchos": [3, 1]},
            {"linea": true}, {"salto": true}, {"h3": "Anexo"}, {"parrafo": "Página dos."}
        ]})).unwrap();
        let pdf = d.join("informe.pdf");
        crate::apps::exportar_pdf(&p, &pdf, 120).await.unwrap();
        let bytes = std::fs::read(&pdf).unwrap();
        assert!(bytes.starts_with(b"%PDF"));
        let texto = crate::pdf::leer(&pdf, "").unwrap();
        println!("{texto}");
        assert!(texto.contains("Informe anual") && texto.contains("Mayoristas") && texto.contains("Página dos."));
        if let Some(o) = std::env::var_os("GOTV_PDF_SALIDA") {
            std::fs::copy(&pdf, o).unwrap();
        }
    }
}
