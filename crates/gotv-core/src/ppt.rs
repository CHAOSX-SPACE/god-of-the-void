//! O2 · PowerPoint (.pptx) como ARCHIVO, con el método de la skill `pptx` de Claude Code
//! (Apache-2.0; `ESTUDIO-DOCUMENTOS.md` §2.5), en Rust:
//!   - crear = su «deck estructurado»: TEMA con los 12 colores y las dos fuentes, un patrón con
//!     diseños (portada, título y contenido, sólo título, en blanco) y marcadores; lienzo panorámico
//!     13,33″×7,5″; colores en hex de 6 dígitos; gráficos NATIVOS (con su hoja incrustada); un
//!     elemento visual en cada diapositiva; nada de rayas de acento ni barras en los bordes;
//!     títulos 36–40 pt, cuerpo ≥ 14 pt; márgenes ≥ 0,5″;
//!   - leer = su `slide_text.py`: por diapositiva, título, textos, tablas y datos de gráficos;
//!   - editar = unzip → XML → zip: reemplazar texto (uniendo fragmentos `<a:r>`), borrar o duplicar
//!     una diapositiva (sus 5 registros) y agregar diapositivas; respaldo;
//!   - revisar = su validador (paquete, colores, posiciones, cuerpos de texto vacíos, ids).

use crate::ooxml::{escapar, Doc, Elem, Nodo, Paquete};
use anyhow::{anyhow, bail, Result};
use gotv_engine::{anyhow_tr, bail_tr};
use serde_json::Value;
use std::path::Path;

const P: &str = "http://schemas.openxmlformats.org/presentationml/2006/main";
const A: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";
const RN: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const RT: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
/// 13,333″ × 7,5″ (panorámico, el de PowerPoint por omisión)
const ANCHO: i64 = 12_192_000;
const ALTO: i64 = 6_858_000;
const PULG: i64 = 914_400;

/// Las paletas de la guía de Claude Code (primario, secundario, acento) + texto oscuro/claro.
pub(crate) const PALETAS: &[(&str, [&str; 3])] = &[
    ("midnight", ["1E2761", "CADCFC", "FFFFFF"]),
    ("bosque", ["2C5F2D", "97BC62", "F5F5F5"]),
    ("coral", ["F96167", "F9E795", "2F3C7E"]),
    ("terracota", ["B85042", "E7E8D1", "A7BEAE"]),
    ("oceano", ["065A82", "1C7293", "21295C"]),
    ("carbon", ["36454F", "F2F2F2", "212121"]),
    ("teal", ["028090", "00A896", "02C39A"]),
    ("baya", ["6D2E46", "A26769", "ECE2D0"]),
    ("salvia", ["84B59F", "69A297", "50808E"]),
    ("cereza", ["990011", "FCF6F5", "2F3C7E"]),
];

/// El tema: colores (hex de 6) y fuentes (título y cuerpo, de la lista SEGURA de la guía).
struct Tema {
    oscuro: String,
    primario: String,
    secundario: String,
    acento: String,
    claro: String,
    fuente_titulo: String,
    fuente_cuerpo: String,
}

pub(crate) fn hex(s: &str) -> Option<String> {
    let h = s.trim().trim_start_matches('#').to_uppercase();
    (h.len() == 6 && h.chars().all(|c| c.is_ascii_hexdigit())).then_some(h)
}

fn tema(t: &Value) -> Tema {
    let nombre = t["paleta"].as_str().unwrap_or("midnight").to_lowercase();
    let base = PALETAS.iter().find(|(n, _)| nombre.contains(n)).map(|(_, c)| *c).unwrap_or(PALETAS[0].1);
    let col = |k: &str, d: &str| t[k].as_str().and_then(hex).unwrap_or_else(|| d.to_string());
    Tema {
        oscuro: col("oscuro", "1F2937"),
        primario: col("primario", base[0]),
        secundario: col("secundario", base[1]),
        acento: col("acento", base[2]),
        claro: col("claro", "FFFFFF"),
        fuente_titulo: t["fuente_titulo"].as_str().unwrap_or("Cambria").to_string(),
        fuente_cuerpo: t["fuente_cuerpo"].as_str().unwrap_or("Calibri").to_string(),
    }
}

/// El tema que YA tiene la presentación (su `theme1.xml`): lo que se agrega al editar sigue
/// la paleta y las fuentes del deck, no la de por defecto.
fn tema_del_paquete(p: &Paquete) -> Option<Tema> {
    let d = Doc::leer(&p.texto("ppt/theme/theme1.xml")?).ok()?;
    let esquema = d.raiz.hijo("themeElements")?.hijo("clrScheme")?;
    let color = |n: &str| esquema.hijo(n).and_then(|c| c.elems().next()).and_then(|c| c.attr("val").or_else(|| c.attr("lastClr"))).and_then(|v| hex(&v));
    let fuentes = d.raiz.hijo("themeElements")?.hijo("fontScheme");
    let fuente = |k: &str| fuentes.and_then(|f| f.hijo(k)).and_then(|f| f.hijo("latin")).and_then(|l| l.attr("typeface")).filter(|s| !s.is_empty() && !s.starts_with('+'));
    let base = tema(&Value::Null);
    Some(Tema {
        oscuro: color("dk1").filter(|c| c != "000000").unwrap_or(base.oscuro),
        primario: color("accent1")?,
        secundario: color("accent3").unwrap_or(base.secundario),
        acento: color("accent2").unwrap_or(base.acento),
        claro: base.claro,
        fuente_titulo: fuente("majorFont").unwrap_or(base.fuente_titulo),
        fuente_cuerpo: fuente("minorFont").unwrap_or(base.fuente_cuerpo),
    })
}

// ── piezas de dibujo ─────────────────────────────────────────────────────────────────────────

/// Un cuadro de texto (no marcador): `txBox="1"` (la guía: si no, los lectores de pantalla lo
/// anuncian como «gráfico»), con márgenes internos 0 para alinear con otras formas.
#[allow(clippy::too_many_arguments)]
fn cuadro(id: u32, x: i64, y: i64, w: i64, h: i64, parrafos: &str, anclaje: &str) -> String {
    format!(
        "<p:sp><p:nvSpPr><p:cNvPr id=\"{id}\" name=\"Texto {id}\"/><p:cNvSpPr txBox=\"1\"/><p:nvPr/></p:nvSpPr><p:spPr><a:xfrm><a:off x=\"{x}\" y=\"{y}\"/><a:ext cx=\"{w}\" cy=\"{h}\"/></a:xfrm><a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom><a:noFill/></p:spPr><p:txBody><a:bodyPr wrap=\"square\" lIns=\"0\" tIns=\"0\" rIns=\"0\" bIns=\"0\" anchor=\"{anclaje}\"><a:normAutofit/></a:bodyPr><a:lstStyle/>{parrafos}</p:txBody></p:sp>"
    )
}

/// Un párrafo con un solo estilo de texto.
fn parr(texto: &str, pt: u32, color: &str, negrita: bool, fuente: &str, alin: &str, espacio_despues: u32) -> String {
    let b = if negrita { " b=\"1\"" } else { "" };
    let runs: String = texto
        .split("**")
        .enumerate()
        .filter(|(_, t)| !t.is_empty())
        .map(|(i, t)| {
            let bb = if negrita || i % 2 == 1 { " b=\"1\"" } else { "" };
            format!("<a:r><a:rPr lang=\"es-ES\" sz=\"{}\"{bb} dirty=\"0\"><a:solidFill><a:srgbClr val=\"{color}\"/></a:solidFill><a:latin typeface=\"{}\"/></a:rPr><a:t>{}</a:t></a:r>", pt * 100, escapar(fuente, true), escapar(t, false))
        })
        .collect();
    let _ = b;
    format!("<a:p><a:pPr algn=\"{alin}\"><a:spcAft><a:spcPts val=\"{}\"/></a:spcAft><a:buNone/></a:pPr>{runs}<a:endParaRPr lang=\"es-ES\" sz=\"{}\" dirty=\"0\"/></a:p>", espacio_despues * 100, pt * 100)
}

/// Una forma (círculo o rectángulo redondeado) rellena de color, con texto centrado opcional.
#[allow(clippy::too_many_arguments)]
fn forma(id: u32, geom: &str, x: i64, y: i64, w: i64, h: i64, relleno: &str, texto: Option<(&str, u32, &str, &str)>) -> String {
    let cuerpo = match texto {
        Some((t, pt, color, fuente)) => format!("<p:txBody><a:bodyPr wrap=\"none\" anchor=\"ctr\" lIns=\"0\" tIns=\"0\" rIns=\"0\" bIns=\"0\"/><a:lstStyle/>{}</p:txBody>", parr(t, pt, color, true, fuente, "ctr", 0)),
        None => "<p:txBody><a:bodyPr/><a:lstStyle/><a:p><a:endParaRPr lang=\"es-ES\" dirty=\"0\"/></a:p></p:txBody>".into(),
    };
    format!(
        "<p:sp><p:nvSpPr><p:cNvPr id=\"{id}\" name=\"Forma {id}\"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr><p:spPr><a:xfrm><a:off x=\"{x}\" y=\"{y}\"/><a:ext cx=\"{w}\" cy=\"{h}\"/></a:xfrm><a:prstGeom prst=\"{geom}\"><a:avLst/></a:prstGeom><a:solidFill><a:srgbClr val=\"{relleno}\"/></a:solidFill><a:ln><a:noFill/></a:ln></p:spPr>{cuerpo}</p:sp>"
    )
}

/// Marcador del diseño (título, subtítulo, cuerpo) con su texto: hereda posición y estilo.
fn marcador(id: u32, tipo: &str, idx: Option<u32>, parrafos: &str) -> String {
    let i = idx.map(|i| format!(" idx=\"{i}\"")).unwrap_or_default();
    let t = if tipo == "body" { String::new() } else { format!(" type=\"{tipo}\"") };
    format!("<p:sp><p:nvSpPr><p:cNvPr id=\"{id}\" name=\"{tipo} {id}\"/><p:cNvSpPr><a:spLocks noGrp=\"1\"/></p:cNvSpPr><p:nvPr><p:ph{t}{i}/></p:nvPr></p:nvSpPr><p:spPr/><p:txBody><a:bodyPr><a:normAutofit/></a:bodyPr><a:lstStyle/>{parrafos}</p:txBody></p:sp>")
}

fn texto_simple(t: &str) -> String {
    format!("<a:p><a:r><a:rPr lang=\"es-ES\" dirty=\"0\"/><a:t>{}</a:t></a:r></a:p>", escapar(t, false))
}

/// Bordes de celda finos y suaves (sin ellos PowerPoint pinta líneas negras gruesas: QA visual).
const BORDES_CELDA: &str = "<a:lnL w=\"9525\"><a:solidFill><a:srgbClr val=\"D1D5DB\"/></a:solidFill></a:lnL><a:lnR w=\"9525\"><a:solidFill><a:srgbClr val=\"D1D5DB\"/></a:solidFill></a:lnR><a:lnT w=\"9525\"><a:solidFill><a:srgbClr val=\"D1D5DB\"/></a:solidFill></a:lnT><a:lnB w=\"9525\"><a:solidFill><a:srgbClr val=\"D1D5DB\"/></a:solidFill></a:lnB>";

/// Contraste WCAG entre dos colores hex (1–21).
pub(crate) fn contraste(a: &str, b: &str) -> f64 {
    let lum = |h: &str| {
        let c = |i: usize| {
            let v = u8::from_str_radix(&h[i..i + 2], 16).unwrap_or(0) as f64 / 255.0;
            if v <= 0.03928 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
        };
        0.2126 * c(0) + 0.7152 * c(2) + 0.0722 * c(4)
    };
    let (x, y) = (lum(a), lum(b));
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}

/// `color` si se lee sobre `fondo` (≥ 4,5:1); si no, blanco o casi negro, el que más contraste dé
/// (QA visual 2026-11-11: el subtítulo verde sobre verde azulado no se leía).
pub(crate) fn legible(color: &str, fondo: &str) -> String {
    if contraste(color, fondo) >= 4.5 {
        return color.to_string();
    }
    if contraste("F9FAFB", fondo) >= contraste("111827", fondo) { "F9FAFB".into() } else { "111827".into() }
}

/// Una tabla nativa (graphicFrame): encabezado en el color primario, filas alternas suaves.
fn tabla(id: u32, x: i64, y: i64, w: i64, filas: &[Vec<String>], t: &Tema) -> String {
    let cols = filas.iter().map(Vec::len).max().unwrap_or(1).max(1);
    let cw = w / cols as i64;
    let alto_fila = (PULG * 45) / 100;
    let pt = if filas.len() > 8 { 12 } else { 14 };
    let mut s = format!(
        "<p:graphicFrame><p:nvGraphicFramePr><p:cNvPr id=\"{id}\" name=\"Tabla {id}\"/><p:cNvGraphicFramePr><a:graphicFrameLocks noGrp=\"1\"/></p:cNvGraphicFramePr><p:nvPr/></p:nvGraphicFramePr><p:xfrm><a:off x=\"{x}\" y=\"{y}\"/><a:ext cx=\"{}\" cy=\"{}\"/></p:xfrm><a:graphic><a:graphicData uri=\"http://schemas.openxmlformats.org/drawingml/2006/table\"><a:tbl><a:tblPr firstRow=\"1\" bandRow=\"1\"/><a:tblGrid>",
        cw * cols as i64,
        alto_fila * filas.len() as i64
    );
    for _ in 0..cols {
        s.push_str(&format!("<a:gridCol w=\"{cw}\"/>"));
    }
    s.push_str("</a:tblGrid>");
    for (i, f) in filas.iter().enumerate() {
        s.push_str(&format!("<a:tr h=\"{alto_fila}\">"));
        for j in 0..cols {
            let txt = f.get(j).cloned().unwrap_or_default();
            let cab = legible(&t.claro, &t.primario);
            let (fondo, color, neg) = if i == 0 { (t.primario.as_str(), cab.as_str(), true) } else if i % 2 == 0 { ("F3F4F6", t.oscuro.as_str(), false) } else { ("FFFFFF", t.oscuro.as_str(), false) };
            let b = if neg { " b=\"1\"" } else { "" };
            s.push_str(&format!(
                "<a:tc><a:txBody><a:bodyPr/><a:lstStyle/><a:p><a:r><a:rPr lang=\"es-ES\" sz=\"{}\"{b} dirty=\"0\"><a:solidFill><a:srgbClr val=\"{color}\"/></a:solidFill><a:latin typeface=\"{}\"/></a:rPr><a:t>{}</a:t></a:r></a:p></a:txBody><a:tcPr marL=\"91440\" marR=\"91440\" marT=\"45720\" marB=\"45720\" anchor=\"ctr\">{BORDES}<a:solidFill><a:srgbClr val=\"{fondo}\"/></a:solidFill></a:tcPr></a:tc>",
                pt * 100,
                escapar(&t.fuente_cuerpo, true),
                escapar(&txt, false),
                BORDES = BORDES_CELDA
            ));
        }
        s.push_str("</a:tr>");
    }
    s.push_str("</a:tbl></a:graphicData></a:graphic></p:graphicFrame>");
    s
}

// ── gráficos nativos ─────────────────────────────────────────────────────────────────────────

/// Un gráfico: tipo (columnas, barras, lineas, torta), categorías, series (nombre, valores).
struct Grafico {
    tipo: String,
    titulo: Option<String>,
    categorias: Vec<String>,
    series: Vec<(String, Vec<f64>)>,
}

fn grafico_de(v: &Value) -> Result<Grafico> {
    let categorias: Vec<String> = v["categorias"].as_array().ok_or_else(|| anyhow_tr!("el gráfico necesita «categorias»: [\"Ene\",\"Feb\"]", "the chart needs «categorias»: [\"Jan\",\"Feb\"]"))?.iter().map(|c| c.as_str().map(str::to_string).unwrap_or_else(|| c.to_string())).collect();
    let series: Vec<(String, Vec<f64>)> = match &v["series"] {
        Value::Array(a) => a
            .iter()
            .enumerate()
            .map(|(i, s)| (s["nombre"].as_str().map(str::to_string).unwrap_or_else(|| format!("Serie {}", i + 1)), s["valores"].as_array().cloned().unwrap_or_default().iter().map(|x| x.as_f64().unwrap_or(0.0)).collect()))
            .collect(),
        _ => match v["valores"].as_array() {
            Some(a) => vec![(v["nombre"].as_str().unwrap_or("Serie 1").to_string(), a.iter().map(|x| x.as_f64().unwrap_or(0.0)).collect())],
            None => bail_tr!(
                "el gráfico necesita «series»: [{{\"nombre\":\"Ventas\",\"valores\":[1,2]}}] (o «valores»)",
                "the chart needs «series»: [{{\"nombre\":\"Sales\",\"valores\":[1,2]}}] (or «valores»)"
            ),
        },
    };
    if series.iter().any(|(_, v)| v.len() != categorias.len()) {
        bail_tr!("cada serie necesita un valor por categoría ({} categorías)", "each series needs one value per category ({} categories)", categorias.len());
    }
    Ok(Grafico { tipo: v["tipo"].as_str().unwrap_or("columnas").to_lowercase(), titulo: v["titulo"].as_str().map(str::to_string), categorias, series })
}

fn col_letra(i: usize) -> String {
    crate::excel::nombre_celda(0, i as u32).trim_end_matches('1').to_string()
}

/// El XML del gráfico (c:chartSpace) con caché de datos y referencia a su hoja incrustada.
/// Reglas de la guía: ≥ 2 ejes declarados (salvo torta), `c:order`/`c:idx` únicos 0,1,2…, título,
/// etiquetas de datos, paleta del tema, rejilla suave.
fn chart_xml(g: &Grafico, t: &Tema) -> String {
    let n = g.categorias.len();
    let colores = [t.primario.as_str(), t.acento.as_str(), t.secundario.as_str(), "6B7280", "D97706", "059669"];
    let cat = |_s: usize| {
        let pts: String = g.categorias.iter().enumerate().map(|(i, c)| format!("<c:pt idx=\"{i}\"><c:v>{}</c:v></c:pt>", escapar(c, false))).collect();
        format!("<c:cat><c:strRef><c:f>Sheet1!$A$2:$A${}</c:f><c:strCache><c:ptCount val=\"{n}\"/>{pts}</c:strCache></c:strRef></c:cat>", n + 1)
    };
    let torta = g.tipo.starts_with("torta") || g.tipo.starts_with("pie") || g.tipo.starts_with("dona");
    let mut series = String::new();
    for (i, (nombre, vals)) in g.series.iter().enumerate() {
        let l = col_letra(i + 1);
        let pts: String = vals.iter().enumerate().map(|(k, v)| format!("<c:pt idx=\"{k}\"><c:v>{v}</c:v></c:pt>")).collect();
        let relleno = if torta {
            // cada porción con su color
            (0..n).map(|k| format!("<c:dPt><c:idx val=\"{k}\"/><c:bubble3D val=\"0\"/><c:spPr><a:solidFill><a:srgbClr val=\"{}\"/></a:solidFill><a:ln w=\"12700\"><a:solidFill><a:srgbClr val=\"FFFFFF\"/></a:solidFill></a:ln></c:spPr></c:dPt>", colores[k % colores.len()])).collect::<String>()
        } else {
            String::new()
        };
        let sp = if g.tipo.starts_with("linea") || g.tipo.starts_with("line") {
            format!("<c:spPr><a:ln w=\"28575\" cap=\"rnd\"><a:solidFill><a:srgbClr val=\"{}\"/></a:solidFill></a:ln></c:spPr><c:marker><c:symbol val=\"circle\"/><c:size val=\"6\"/></c:marker>", colores[i % colores.len()])
        } else if torta {
            String::new()
        } else {
            format!("<c:spPr><a:solidFill><a:srgbClr val=\"{}\"/></a:solidFill></c:spPr><c:invertIfNegative val=\"0\"/>", colores[i % colores.len()])
        };
        let etiquetas = format!("<c:dLbls><c:spPr><a:noFill/><a:ln><a:noFill/></a:ln></c:spPr><c:txPr><a:bodyPr/><a:lstStyle/><a:p><a:pPr><a:defRPr sz=\"1200\"><a:solidFill><a:srgbClr val=\"{}\"/></a:solidFill></a:defRPr></a:pPr><a:endParaRPr lang=\"es-ES\"/></a:p></c:txPr><c:showLegendKey val=\"0\"/><c:showVal val=\"1\"/><c:showCatName val=\"0\"/><c:showSerName val=\"0\"/><c:showPercent val=\"0\"/><c:showBubbleSize val=\"0\"/></c:dLbls>", t.oscuro);
        series.push_str(&format!(
            "<c:ser><c:idx val=\"{i}\"/><c:order val=\"{i}\"/><c:tx><c:strRef><c:f>Sheet1!${l}$1</c:f><c:strCache><c:ptCount val=\"1\"/><c:pt idx=\"0\"><c:v>{}</c:v></c:pt></c:strCache></c:strRef></c:tx>{sp}{relleno}{etiquetas}{}<c:val><c:numRef><c:f>Sheet1!${l}$2:${l}${}</c:f><c:numCache><c:formatCode>General</c:formatCode><c:ptCount val=\"{n}\"/>{pts}</c:numCache></c:numRef></c:val>{}</c:ser>",
            escapar(nombre, false),
            cat(i),
            n + 1,
            if g.tipo.starts_with("linea") || g.tipo.starts_with("line") { "<c:smooth val=\"0\"/>" } else { "" }
        ));
    }
    let ejes_txt = |id: u32, cruce: u32, tipo: &str, pos: &str| {
        let grid = if tipo == "valAx" { "<c:majorGridlines><c:spPr><a:ln w=\"6350\"><a:solidFill><a:srgbClr val=\"E5E7EB\"/></a:solidFill></a:ln></c:spPr></c:majorGridlines>" } else { "" };
        let extra = if tipo == "catAx" { "<c:auto val=\"1\"/><c:lblAlgn val=\"ctr\"/><c:lblOffset val=\"100\"/><c:noMultiLvlLbl val=\"0\"/>" } else { "<c:crossBetween val=\"between\"/>" };
        format!("<c:{tipo}><c:axId val=\"{id}\"/><c:scaling><c:orientation val=\"minMax\"/></c:scaling><c:delete val=\"0\"/><c:axPos val=\"{pos}\"/>{grid}<c:numFmt formatCode=\"General\" sourceLinked=\"1\"/><c:majorTickMark val=\"none\"/><c:minorTickMark val=\"none\"/><c:tickLblPos val=\"nextTo\"/><c:spPr><a:ln><a:solidFill><a:srgbClr val=\"D1D5DB\"/></a:solidFill></a:ln></c:spPr><c:txPr><a:bodyPr/><a:lstStyle/><a:p><a:pPr><a:defRPr sz=\"1200\"><a:solidFill><a:srgbClr val=\"4B5563\"/></a:solidFill></a:defRPr></a:pPr><a:endParaRPr lang=\"es-ES\"/></a:p></c:txPr><c:crossAx val=\"{cruce}\"/><c:crosses val=\"autoZero\"/>{extra}</c:{tipo}>")
    };
    let cuerpo = if torta {
        let hueco = if g.tipo.starts_with("dona") { "<c:holeSize val=\"55\"/>" } else { "" };
        let tag = if g.tipo.starts_with("dona") { "doughnutChart" } else { "pieChart" };
        format!("<c:{tag}><c:varyColors val=\"1\"/>{series}<c:firstSliceAng val=\"0\"/>{hueco}</c:{tag}>")
    } else if g.tipo.starts_with("linea") || g.tipo.starts_with("line") {
        format!("<c:lineChart><c:grouping val=\"standard\"/><c:varyColors val=\"0\"/>{series}<c:marker val=\"1\"/><c:axId val=\"500000001\"/><c:axId val=\"500000002\"/></c:lineChart>{}{}", ejes_txt(500000001, 500000002, "catAx", "b"), ejes_txt(500000002, 500000001, "valAx", "l"))
    } else {
        let dir = if g.tipo.starts_with("barra") || g.tipo == "bar" { "bar" } else { "col" };
        let (pos_cat, pos_val) = if dir == "bar" { ("l", "b") } else { ("b", "l") };
        format!("<c:barChart><c:barDir val=\"{dir}\"/><c:grouping val=\"clustered\"/><c:varyColors val=\"0\"/>{series}<c:gapWidth val=\"80\"/><c:axId val=\"500000001\"/><c:axId val=\"500000002\"/></c:barChart>{}{}", ejes_txt(500000001, 500000002, "catAx", pos_cat), ejes_txt(500000002, 500000001, "valAx", pos_val))
    };
    let titulo = match &g.titulo {
        Some(tt) => format!("<c:title><c:tx><c:rich><a:bodyPr/><a:lstStyle/><a:p><a:pPr><a:defRPr sz=\"1600\" b=\"1\"/></a:pPr><a:r><a:rPr lang=\"es-ES\" sz=\"1600\" b=\"1\"><a:solidFill><a:srgbClr val=\"{}\"/></a:solidFill></a:rPr><a:t>{}</a:t></a:r></a:p></c:rich></c:tx><c:overlay val=\"0\"/></c:title><c:autoTitleDeleted val=\"0\"/>", t.oscuro, escapar(tt, false)),
        None => "<c:autoTitleDeleted val=\"1\"/>".into(),
    };
    let leyenda = if g.series.len() > 1 || torta { "<c:legend><c:legendPos val=\"b\"/><c:overlay val=\"0\"/></c:legend>" } else { "" };
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<c:chartSpace xmlns:c=\"http://schemas.openxmlformats.org/drawingml/2006/chart\" xmlns:a=\"{A}\" xmlns:r=\"{RN}\"><c:date1904 val=\"0\"/><c:lang val=\"es-ES\"/><c:roundedCorners val=\"0\"/><c:chart>{titulo}<c:plotArea><c:layout/>{cuerpo}</c:plotArea>{leyenda}<c:plotVisOnly val=\"1\"/><c:dispBlanksAs val=\"gap\"/></c:chart><c:txPr><a:bodyPr/><a:lstStyle/><a:p><a:pPr><a:defRPr><a:latin typeface=\"{}\"/></a:defRPr></a:pPr><a:endParaRPr lang=\"es-ES\"/></a:p></c:txPr><c:externalData r:id=\"rId1\"><c:autoUpdate val=\"0\"/></c:externalData></c:chartSpace>",
        escapar(&t.fuente_cuerpo, true)
    )
}

/// La hoja de Excel incrustada (para «Editar datos» en PowerPoint): A = categorías, B.. = series.
fn hoja_del_grafico(g: &Grafico) -> Result<Vec<u8>> {
    let mut wb = rust_xlsxwriter::Workbook::new();
    let ws = wb.add_worksheet();
    ws.set_name("Sheet1").map_err(|e| anyhow!("{e}"))?;
    for (i, c) in g.categorias.iter().enumerate() {
        ws.write_string(i as u32 + 1, 0, c).map_err(|e| anyhow!("{e}"))?;
    }
    for (j, (n, vals)) in g.series.iter().enumerate() {
        ws.write_string(0, j as u16 + 1, n).map_err(|e| anyhow!("{e}"))?;
        for (i, v) in vals.iter().enumerate() {
            ws.write_number(i as u32 + 1, j as u16 + 1, *v).map_err(|e| anyhow!("{e}"))?;
        }
    }
    wb.save_to_buffer().map_err(|e| anyhow!("{e}"))
}

fn marco_grafico(id: u32, rid: &str, x: i64, y: i64, w: i64, h: i64) -> String {
    format!("<p:graphicFrame><p:nvGraphicFramePr><p:cNvPr id=\"{id}\" name=\"Gráfico {id}\"/><p:cNvGraphicFramePr/><p:nvPr/></p:nvGraphicFramePr><p:xfrm><a:off x=\"{x}\" y=\"{y}\"/><a:ext cx=\"{w}\" cy=\"{h}\"/></p:xfrm><a:graphic><a:graphicData uri=\"http://schemas.openxmlformats.org/drawingml/2006/chart\"><c:chart xmlns:c=\"http://schemas.openxmlformats.org/drawingml/2006/chart\" r:id=\"{rid}\"/></a:graphicData></a:graphic></p:graphicFrame>")
}

// ── diapositivas ─────────────────────────────────────────────────────────────────────────────

/// Lo que produce una diapositiva: su XML, el diseño que usa (1 portada, 2 título y contenido,
/// 3 sólo título, 4 en blanco) y sus gráficos.
struct Diapo {
    xml: String,
    diseno: u32,
    graficos: Vec<Grafico>,
}

fn lista_textos(v: &Value) -> Vec<String> {
    match v {
        Value::Array(a) => a.iter().map(|x| x.as_str().map(str::to_string).unwrap_or_else(|| x.to_string())).collect(),
        Value::String(s) => s.lines().map(str::to_string).filter(|l| !l.trim().is_empty()).collect(),
        _ => vec![],
    }
}

/// Una diapositiva en la forma que el modelo escribe de forma natural → la de la ficha (como en
/// Word): `{"type":"bullets","title":…,"items":[…]}`, `{"tipo":"contenido","contenido":[…]}`…
fn normalizar(d: &Value) -> Value {
    let mut o = d.as_object().cloned().unwrap_or_default();
    // medido (banco of-deck, 9B): la diapositiva ANIDADA bajo su tipo — `{"cifra": {"cifra":"80%",
    // "texto":…}}`, `{"dos_columnas": {"izquierda":…}}` — imprimía el objeto como texto. Se aplana.
    // (`grafico` no: su objeto ES la ficha del gráfico)
    for k in ["portada", "seccion", "cierre", "puntos", "dos_columnas", "cifra", "tabla"] {
        if let Some(Value::Object(dentro)) = o.get(k).cloned() {
            o.remove(k);
            for (ck, cv) in dentro {
                o.entry(ck).or_insert(cv);
            }
            o.entry("tipo").or_insert_with(|| Value::String(k.into()));
        }
    }
    let tipo =["tipo", "type", "layout", "diseno"].iter().find_map(|k| o.get(*k)).and_then(|v| v.as_str()).unwrap_or("").to_lowercase();
    let alias = |k: &str, de: &[&str], o: &mut serde_json::Map<String, Value>| {
        if !o.contains_key(k) {
            if let Some(v) = de.iter().find_map(|a| o.get(*a)).cloned() {
                o.insert(k.into(), v);
            }
        }
    };
    alias("titulo", &["title", "encabezado", "heading"], &mut o);
    alias("subtitulo", &["subtitle", "bajada"], &mut o);
    alias("puntos", &["bullets", "items", "contenido", "content", "lista", "puntos_clave"], &mut o);
    alias("texto", &["text", "body", "cuerpo", "descripcion"], &mut o);
    alias("tabla", &["table", "filas", "rows"], &mut o);
    alias("grafico", &["chart", "grafica"], &mut o);
    alias("cifra", &["stat", "numero", "dato", "kpi"], &mut o);
    let t = match tipo.as_str() {
        "portada" | "title" | "titulo" | "cover" | "inicio" => "portada",
        "seccion" | "section" | "divisor" => "seccion",
        "cierre" | "closing" | "final" | "conclusion" | "gracias" => "cierre",
        "dos_columnas" | "two_columns" | "comparacion" | "columnas" => "dos_columnas",
        "cifra" | "stat" | "numero" | "kpi" => "cifra",
        "tabla" | "table" => "tabla",
        "grafico" | "chart" | "grafica" => "grafico",
        "puntos" | "bullets" | "contenido" | "content" | "lista" => "puntos",
        _ if o.contains_key("grafico") => "grafico",
        _ if o.contains_key("tabla") => "tabla",
        _ if o.contains_key("cifra") => "cifra",
        _ if o.contains_key("izquierda") => "dos_columnas",
        _ if o.contains_key("puntos") => "puntos",
        _ => "portada",
    };
    o.insert("tipo".into(), Value::String(t.into()));
    Value::Object(o)
}

fn diapositiva(d: &Value, t: &Tema, n: usize) -> Result<Diapo> {
    let d = normalizar(d);
    let titulo = d["titulo"].as_str().unwrap_or("").to_string();
    let m = PULG / 2 + PULG / 10; // margen 0,6″
    let mut id = 2;
    let mut sig = || {
        id += 1;
        id
    };
    let mut cuerpo = String::new();
    let mut graficos = vec![];
    let fondo_oscuro = |c: &str| format!("<p:bg><p:bgPr><a:solidFill><a:srgbClr val=\"{c}\"/></a:solidFill><a:effectLst/></p:bgPr></p:bg>");
    let mut fondo = String::new();
    let tit = |pt: u32, color: &str, alin: &str| parr(&titulo, pt, color, true, &t.fuente_titulo, alin, 0);
    let diseno = match d["tipo"].as_str().unwrap_or("puntos") {
        "portada" | "cierre" | "seccion" => {
            // «sándwich»: portada, secciones y cierre en fondo oscuro (la guía)
            fondo = fondo_oscuro(&t.primario);
            let sub = d["subtitulo"].as_str().or(d["texto"].as_str()).unwrap_or("");
            let pt = if d["tipo"] == "seccion" { 36 } else { 44 };
            cuerpo.push_str(&cuadro(sig(), m * 2, ALTO * 30 / 100, ANCHO - m * 4, PULG * 3 / 2, &tit(pt_que_cabe(&titulo, pt, ANCHO - m * 4, PULG * 3 / 2), &legible(&t.claro, &t.primario), "l"), "b"));
            // motivo: un círculo del acento (no una raya)
            cuerpo.push_str(&forma(sig(), "ellipse", m * 2, ALTO * 30 / 100 - PULG * 7 / 10, PULG / 2, PULG / 2, &t.acento, None));
            if !sub.is_empty() {
                cuerpo.push_str(&cuadro(sig(), m * 2, ALTO * 30 / 100 + PULG * 17 / 10, ANCHO - m * 4, PULG, &parr(sub, 20, &legible(&t.secundario, &t.primario), false, &t.fuente_cuerpo, "l", 0), "t"));
            }
            4
        }
        "cifra" => {
            let cifra = d["cifra"].as_str().map(str::to_string).unwrap_or_else(|| d["cifra"].to_string());
            let texto = d["texto"].as_str().unwrap_or("");
            cuerpo.push_str(&cuadro(sig(), m, m, ANCHO - 2 * m, PULG, &tit(pt_que_cabe(&titulo, 36, ANCHO - 2 * m, PULG), &t.oscuro, "l"), "t"));
            cuerpo.push_str(&cuadro(sig(), m, PULG * 2, ANCHO / 2 - m, PULG * 3, &parr(&cifra, 72, &legible(&t.primario, "FFFFFF"), true, &t.fuente_titulo, "l", 0), "ctr"));
            cuerpo.push_str(&cuadro(sig(), ANCHO / 2, PULG * 2, ANCHO / 2 - m, PULG * 3, &parr(texto, 20, &t.oscuro, false, &t.fuente_cuerpo, "l", 0), "ctr"));
            4
        }
        "dos_columnas" => {
            cuerpo.push_str(&cuadro(sig(), m, m, ANCHO - 2 * m, PULG, &tit(pt_que_cabe(&titulo, 36, ANCHO - 2 * m, PULG), &t.oscuro, "l"), "t"));
            let col_w = (ANCHO - 2 * m - PULG / 2) / 2;
            // las dos tarjetas del alto de la más llena (QA visual: tarjetas a toda altura con dos
            // líneas dentro dejaban un hueco enorme)
            let lineas = ["izquierda", "derecha"].iter().map(|l| lista_textos(&d[*l]).len() as i64 + if d[format!("titulo_{l}")].as_str().is_some() { 2 } else { 0 }).max().unwrap_or(1);
            let alto_tarjeta = (PULG / 2 + lineas * PULG * 45 / 100).clamp(PULG * 2, ALTO - PULG * 18 / 10 - m);
            for (k, lado) in ["izquierda", "derecha"].iter().enumerate() {
                let x = m + k as i64 * (col_w + PULG / 2);
                // tarjeta con un tinte suave (no una raya de borde)
                cuerpo.push_str(&forma(sig(), "roundRect", x, PULG * 18 / 10, col_w, alto_tarjeta, if k == 0 { "F3F4F6" } else { "EEF2FF" }, None));
                let cab = d[format!("titulo_{lado}")].as_str().unwrap_or("");
                let mut ps = String::new();
                if !cab.is_empty() {
                    ps.push_str(&parr(cab, 22, &legible(&t.primario, if k == 0 { "F3F4F6" } else { "EEF2FF" }), true, &t.fuente_titulo, "l", 10));
                }
                for p in lista_textos(&d[*lado]) {
                    ps.push_str(&parr(&p, 18, &t.oscuro, false, &t.fuente_cuerpo, "l", 8));
                }
                cuerpo.push_str(&cuadro(sig(), x + PULG / 4, PULG * 2, col_w - PULG / 2, alto_tarjeta - PULG * 4 / 10, &ps, "t"));
            }
            3
        }
        "tabla" => {
            cuerpo.push_str(&cuadro(sig(), m, m, ANCHO - 2 * m, PULG, &tit(pt_que_cabe(&titulo, 36, ANCHO - 2 * m, PULG), &t.oscuro, "l"), "t"));
            let filas: Vec<Vec<String>> = d["tabla"].as_array().ok_or_else(|| anyhow_tr!("diapositiva {n}: «tabla» es una lista de filas", "slide {n}: «tabla» is a list of rows"))?.iter().map(lista_textos).collect();
            cuerpo.push_str(&tabla(sig(), m, PULG * 18 / 10, ANCHO - 2 * m, &filas, t));
            3
        }
        "grafico" => {
            cuerpo.push_str(&cuadro(sig(), m, m, ANCHO - 2 * m, PULG, &tit(pt_que_cabe(&titulo, 36, ANCHO - 2 * m, PULG), &t.oscuro, "l"), "t"));
            graficos.push(grafico_de(&d["grafico"]).map_err(|e| anyhow!("diapositiva {n}: {e}"))?);
            let texto = d["texto"].as_str().unwrap_or("");
            let (gw, tx) = if texto.is_empty() { (ANCHO - 2 * m, None) } else { (ANCHO * 62 / 100 - m, Some(ANCHO * 62 / 100 + PULG / 4)) };
            cuerpo.push_str(&marco_grafico(sig(), "rIdG1", m, PULG * 17 / 10, gw, ALTO - PULG * 17 / 10 - m));
            if let Some(x) = tx {
                cuerpo.push_str(&cuadro(sig(), x, PULG * 2, ANCHO - x - m, ALTO - PULG * 2 - m, &parr(texto, 18, &t.oscuro, false, &t.fuente_cuerpo, "l", 0), "ctr"));
            }
            3
        }
        _ => {
            // puntos: cada uno en una fila con su número en un círculo del color primario
            // (el «icono en círculo» de la guía), nada de viñetas literales
            cuerpo.push_str(&cuadro(sig(), m, m, ANCHO - 2 * m, PULG, &tit(pt_que_cabe(&titulo, 36, ANCHO - 2 * m, PULG), &t.oscuro, "l"), "t"));
            let puntos = lista_textos(&d["puntos"]);
            let k = puntos.len().max(1) as i64;
            let arriba = PULG * 18 / 10;
            let alto_fila = ((ALTO - arriba - m) / k).min(PULG * 11 / 10);
            let pt = if k > 5 { 16 } else { 18 };
            let circ = (alto_fila * 6 / 10).min(PULG * 6 / 10);
            for (i, p) in puntos.iter().enumerate() {
                let y = arriba + i as i64 * alto_fila;
                // el número CABE en su círculo (medido: «10» a 14 pt en un círculo de 0,29″ se partía
                // en «1 / 0»): sin ajuste de línea y a la medida del círculo
                let num = (i + 1).to_string();
                let circ_pt = (circ / 12700) as u32;
                let num_pt = (circ_pt * if num.len() > 1 { 45 } else { 60 } / 100).clamp(8, 14);
                cuerpo.push_str(&forma(sig(), "ellipse", m, y + (alto_fila - circ) / 2, circ, circ, &t.primario, Some((&num, num_pt, &legible(&t.claro, &t.primario), &t.fuente_titulo))));
                cuerpo.push_str(&cuadro(sig(), m + circ + PULG / 4, y, ANCHO - 2 * m - circ - PULG / 4, alto_fila, &parr(p, pt, &t.oscuro, false, &t.fuente_cuerpo, "l", 0), "ctr"));
            }
            3
        }
    };
    let xml = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<p:sld xmlns:a=\"{A}\" xmlns:r=\"{RN}\" xmlns:p=\"{P}\"><p:cSld>{fondo}<p:spTree><p:nvGrpSpPr><p:cNvPr id=\"1\" name=\"\"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"0\" cy=\"0\"/><a:chOff x=\"0\" y=\"0\"/><a:chExt cx=\"0\" cy=\"0\"/></a:xfrm></p:grpSpPr>{cuerpo}</p:spTree></p:cSld><p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:sld>"
    );
    let _ = marcador;
    let _ = texto_simple;
    Ok(Diapo { xml, diseno, graficos })
}

// ── el paquete ───────────────────────────────────────────────────────────────────────────────

fn tema_xml(t: &Tema) -> String {
    let c = |n: &str, v: &str| format!("<a:{n}><a:srgbClr val=\"{v}\"/></a:{n}>");
    let ln = |w: u32| format!("<a:ln w=\"{w}\" cap=\"flat\" cmpd=\"sng\" algn=\"ctr\"><a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill><a:prstDash val=\"solid\"/><a:miter lim=\"800000\"/></a:ln>");
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<a:theme xmlns:a=\"{A}\" name=\"GOTV\"><a:themeElements><a:clrScheme name=\"GOTV\">{}{}{}{}{}{}{}{}{}{}{}{}</a:clrScheme><a:fontScheme name=\"GOTV\"><a:majorFont><a:latin typeface=\"{}\"/><a:ea typeface=\"\"/><a:cs typeface=\"\"/></a:majorFont><a:minorFont><a:latin typeface=\"{}\"/><a:ea typeface=\"\"/><a:cs typeface=\"\"/></a:minorFont></a:fontScheme><a:fmtScheme name=\"GOTV\"><a:fillStyleLst><a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill><a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill><a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill></a:fillStyleLst><a:lnStyleLst>{}{}{}</a:lnStyleLst><a:effectStyleLst><a:effectStyle><a:effectLst/></a:effectStyle><a:effectStyle><a:effectLst/></a:effectStyle><a:effectStyle><a:effectLst/></a:effectStyle></a:effectStyleLst><a:bgFillStyleLst><a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill><a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill><a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill></a:bgFillStyleLst></a:fmtScheme></a:themeElements><a:objectDefaults/><a:extraClrSchemeLst/></a:theme>",
        c("dk1", &t.oscuro), c("lt1", "FFFFFF"), c("dk2", &t.primario), c("lt2", "F3F4F6"),
        c("accent1", &t.primario), c("accent2", &t.acento), c("accent3", &t.secundario), c("accent4", "6B7280"), c("accent5", "D97706"), c("accent6", "059669"),
        c("hlink", &t.primario), c("folHlink", "6B7280"),
        escapar(&t.fuente_titulo, true), escapar(&t.fuente_cuerpo, true), ln(6350), ln(12700), ln(19050)
    )
}

fn arbol_vacio() -> &'static str {
    "<p:nvGrpSpPr><p:cNvPr id=\"1\" name=\"\"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"0\" cy=\"0\"/><a:chOff x=\"0\" y=\"0\"/><a:chExt cx=\"0\" cy=\"0\"/></a:xfrm></p:grpSpPr>"
}

#[allow(clippy::too_many_arguments)]
fn ph(id: u32, nombre: &str, tipo: &str, idx: Option<u32>, x: i64, y: i64, w: i64, h: i64) -> String {
    let i = idx.map(|i| format!(" idx=\"{i}\"")).unwrap_or_default();
    // «obj» = sin tipo (el de contenido de un DISEÑO); en el PATRÓN el de texto es type="body" (medido
    // 2026-11-11: sin él PowerPoint pedía «reparar» TODA presentación, aunque el XSD la acepta)
    let t = if tipo == "obj" { String::new() } else { format!(" type=\"{tipo}\"") };
    format!("<p:sp><p:nvSpPr><p:cNvPr id=\"{id}\" name=\"{nombre}\"/><p:cNvSpPr><a:spLocks noGrp=\"1\"/></p:cNvSpPr><p:nvPr><p:ph{t}{i}/></p:nvPr></p:nvSpPr><p:spPr><a:xfrm><a:off x=\"{x}\" y=\"{y}\"/><a:ext cx=\"{w}\" cy=\"{h}\"/></a:xfrm></p:spPr><p:txBody><a:bodyPr/><a:lstStyle/><a:p><a:endParaRPr lang=\"es-ES\"/></a:p></p:txBody></p:sp>")
}

fn patron_xml(t: &Tema) -> String {
    let m = PULG / 2 + PULG / 10;
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<p:sldMaster xmlns:a=\"{A}\" xmlns:r=\"{RN}\" xmlns:p=\"{P}\"><p:cSld><p:bg><p:bgRef idx=\"1001\"><a:schemeClr val=\"bg1\"/></p:bgRef></p:bg><p:spTree>{}{}{}</p:spTree></p:cSld><p:clrMap bg1=\"lt1\" tx1=\"dk1\" bg2=\"lt2\" tx2=\"dk2\" accent1=\"accent1\" accent2=\"accent2\" accent3=\"accent3\" accent4=\"accent4\" accent5=\"accent5\" accent6=\"accent6\" hlink=\"hlink\" folHlink=\"folHlink\"/><p:sldLayoutIdLst><p:sldLayoutId id=\"2147483649\" r:id=\"rId1\"/><p:sldLayoutId id=\"2147483650\" r:id=\"rId2\"/><p:sldLayoutId id=\"2147483651\" r:id=\"rId3\"/><p:sldLayoutId id=\"2147483652\" r:id=\"rId4\"/></p:sldLayoutIdLst><p:txStyles><p:titleStyle><a:lvl1pPr algn=\"l\"><a:defRPr sz=\"3600\" b=\"1\"><a:solidFill><a:schemeClr val=\"tx1\"/></a:solidFill><a:latin typeface=\"+mj-lt\"/></a:defRPr></a:lvl1pPr></p:titleStyle><p:bodyStyle><a:lvl1pPr marL=\"285750\" indent=\"-285750\"><a:spcAft><a:spcPts val=\"800\"/></a:spcAft><a:buFont typeface=\"Arial\"/><a:buChar char=\"&#8226;\"/><a:defRPr sz=\"1800\"><a:solidFill><a:schemeClr val=\"tx1\"/></a:solidFill><a:latin typeface=\"+mn-lt\"/></a:defRPr></a:lvl1pPr><a:lvl2pPr marL=\"742950\" indent=\"-285750\"><a:buFont typeface=\"Arial\"/><a:buChar char=\"&#8211;\"/><a:defRPr sz=\"1600\"><a:solidFill><a:schemeClr val=\"tx1\"/></a:solidFill><a:latin typeface=\"+mn-lt\"/></a:defRPr></a:lvl2pPr></p:bodyStyle><p:otherStyle><a:lvl1pPr><a:defRPr sz=\"1800\"><a:solidFill><a:schemeClr val=\"tx1\"/></a:solidFill><a:latin typeface=\"+mn-lt\"/></a:defRPr></a:lvl1pPr></p:otherStyle></p:txStyles></p:sldMaster>",
        arbol_vacio(),
        ph(2, crate::i18n::t("Título", "Title"), "title", None, m, m, ANCHO - 2 * m, PULG),
        ph(3, crate::i18n::t("Texto", "Text"), "body", Some(1), m, PULG * 18 / 10, ANCHO - 2 * m, ALTO - PULG * 18 / 10 - m),
    )
    .replace("{tema}", &t.primario)
}

fn diseno_xml(n: u32) -> String {
    let m = PULG / 2 + PULG / 10;
    let (tipo, nombre, formas) = match n {
        1 => ("title", crate::i18n::t("Portada", "Title slide"), format!("{}{}", ph(2, crate::i18n::t("Título", "Title"), "ctrTitle", None, m * 2, ALTO * 30 / 100, ANCHO - m * 4, PULG * 3 / 2), ph(3, crate::i18n::t("Subtítulo", "Subtitle"), "subTitle", Some(1), m * 2, ALTO * 30 / 100 + PULG * 17 / 10, ANCHO - m * 4, PULG))),
        2 => ("obj", crate::i18n::t("Título y contenido", "Title and content"), format!("{}{}", ph(2, crate::i18n::t("Título", "Title"), "title", None, m, m, ANCHO - 2 * m, PULG), ph(3, crate::i18n::t("Contenido", "Content"), "obj", Some(1), m, PULG * 18 / 10, ANCHO - 2 * m, ALTO - PULG * 18 / 10 - m))),
        3 => ("titleOnly", crate::i18n::t("Sólo título", "Title only"), ph(2, crate::i18n::t("Título", "Title"), "title", None, m, m, ANCHO - 2 * m, PULG)),
        _ => ("blank", crate::i18n::t("En blanco", "Blank"), String::new()),
    };
    format!("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<p:sldLayout xmlns:a=\"{A}\" xmlns:r=\"{RN}\" xmlns:p=\"{P}\" type=\"{tipo}\" preserve=\"1\"><p:cSld name=\"{nombre}\"><p:spTree>{}{formas}</p:spTree></p:cSld><p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:sldLayout>", arbol_vacio())
}

fn rels(entradas: &[(String, String, String)]) -> String {
    let r: String = entradas.iter().map(|(id, tipo, t)| format!("<Relationship Id=\"{id}\" Type=\"{tipo}\" Target=\"{t}\"/>")).collect();
    format!("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">{r}</Relationships>")
}

/// CREAR un .pptx: `{tema?: {paleta, primario?, acento?, fuente_titulo?, fuente_cuerpo?},
/// diapositivas: [{tipo: portada|seccion|puntos|dos_columnas|cifra|tabla|grafico|cierre, titulo,
/// subtitulo?, puntos?, izquierda?, derecha?, titulo_izquierda?, titulo_derecha?, cifra?, texto?,
/// tabla?, grafico?: {tipo: columnas|barras|lineas|torta|dona, titulo?, categorias, series}}]}`.
pub fn crear(path: &Path, ficha: &Value) -> Result<usize> {
    let ficha = &crate::ooxml::tolerante_o_error(ficha, "ficha")?;
    let ds = ficha["diapositivas"].as_array().or_else(|| ficha["slides"].as_array()).filter(|d| !d.is_empty()).ok_or_else(|| anyhow_tr!("la ficha necesita «diapositivas»: [{{\"tipo\":\"portada\",\"titulo\":\"…\"}}, …]", "the spec needs «diapositivas»: [{{\"tipo\":\"portada\",\"titulo\":\"…\"}}, …]"))?;
    let t = tema(&ficha["tema"]);
    let mut p = Paquete { partes: Default::default() };
    let mut tipos = String::from("<Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/><Default Extension=\"xml\" ContentType=\"application/xml\"/><Default Extension=\"xlsx\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.sheet\"/><Override PartName=\"/ppt/presentation.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml\"/><Override PartName=\"/ppt/slideMasters/slideMaster1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.slideMaster+xml\"/><Override PartName=\"/ppt/theme/theme1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.theme+xml\"/><Override PartName=\"/ppt/presProps.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.presProps+xml\"/><Override PartName=\"/ppt/viewProps.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.viewProps+xml\"/><Override PartName=\"/ppt/tableStyles.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.tableStyles+xml\"/><Override PartName=\"/docProps/core.xml\" ContentType=\"application/vnd.openxmlformats-package.core-properties+xml\"/><Override PartName=\"/docProps/app.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.extended-properties+xml\"/>");
    for n in 1..=4 {
        tipos.push_str(&format!("<Override PartName=\"/ppt/slideLayouts/slideLayout{n}.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.slideLayout+xml\"/>"));
        p.poner(&format!("ppt/slideLayouts/slideLayout{n}.xml"), diseno_xml(n));
        p.poner(&format!("ppt/slideLayouts/_rels/slideLayout{n}.xml.rels"), rels(&[("rId1".into(), format!("{RT}/slideMaster"), "../slideMasters/slideMaster1.xml".into())]));
    }
    p.poner("ppt/slideMasters/slideMaster1.xml", patron_xml(&t));
    let mut mrels: Vec<(String, String, String)> = (1..=4).map(|n| (format!("rId{n}"), format!("{RT}/slideLayout"), format!("../slideLayouts/slideLayout{n}.xml"))).collect();
    mrels.push(("rId5".into(), format!("{RT}/theme"), "../theme/theme1.xml".into()));
    p.poner("ppt/slideMasters/_rels/slideMaster1.xml.rels", rels(&mrels));
    p.poner("ppt/theme/theme1.xml", tema_xml(&t));
    let mut prels = vec![("rId1".to_string(), format!("{RT}/slideMaster"), "slideMasters/slideMaster1.xml".to_string()), ("rId2".into(), format!("{RT}/theme"), "theme/theme1.xml".into()), ("rId3".into(), format!("{RT}/presProps"), "presProps.xml".into()), ("rId4".into(), format!("{RT}/viewProps"), "viewProps.xml".into()), ("rId5".into(), format!("{RT}/tableStyles"), "tableStyles.xml".into())];
    let mut ids = String::new();
    let mut n_graf = 0;
    for (i, d) in ds.iter().enumerate() {
        let n = i + 1;
        let dp = diapositiva(d, &t, n)?;
        let mut srels = vec![("rId1".to_string(), format!("{RT}/slideLayout"), format!("../slideLayouts/slideLayout{}.xml", dp.diseno))];
        let mut xml = dp.xml;
        for (k, g) in dp.graficos.iter().enumerate() {
            n_graf += 1;
            let rid = format!("rIdG{}", k + 1);
            srels.push((rid, format!("{RT}/chart"), format!("../charts/chart{n_graf}.xml")));
            p.poner(&format!("ppt/charts/chart{n_graf}.xml"), chart_xml(g, &t));
            p.poner(&format!("ppt/charts/_rels/chart{n_graf}.xml.rels"), rels(&[("rId1".into(), format!("{RT}/package"), format!("../embeddings/Microsoft_Excel_Worksheet{n_graf}.xlsx"))]));
            p.poner(&format!("ppt/embeddings/Microsoft_Excel_Worksheet{n_graf}.xlsx"), hoja_del_grafico(g)?);
            tipos.push_str(&format!("<Override PartName=\"/ppt/charts/chart{n_graf}.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.drawingml.chart+xml\"/>"));
        }
        if dp.graficos.is_empty() {
            xml = xml.replace("rIdG1", "rId1");
        }
        p.poner(&format!("ppt/slides/slide{n}.xml"), xml);
        p.poner(&format!("ppt/slides/_rels/slide{n}.xml.rels"), rels(&srels));
        tipos.push_str(&format!("<Override PartName=\"/ppt/slides/slide{n}.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.slide+xml\"/>"));
        let rid = format!("rId{}", 100 + n);
        prels.push((rid.clone(), format!("{RT}/slide"), format!("slides/slide{n}.xml")));
        ids.push_str(&format!("<p:sldId id=\"{}\" r:id=\"{rid}\"/>", 255 + n));
    }
    p.poner("ppt/presentation.xml", format!("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<p:presentation xmlns:a=\"{A}\" xmlns:r=\"{RN}\" xmlns:p=\"{P}\" saveSubsetFonts=\"1\"><p:sldMasterIdLst><p:sldMasterId id=\"2147483648\" r:id=\"rId1\"/></p:sldMasterIdLst><p:sldIdLst>{ids}</p:sldIdLst><p:sldSz cx=\"{ANCHO}\" cy=\"{ALTO}\"/><p:notesSz cx=\"6858000\" cy=\"9144000\"/><p:defaultTextStyle><a:lvl1pPr><a:defRPr lang=\"es-ES\"/></a:lvl1pPr></p:defaultTextStyle></p:presentation>"));
    p.poner("ppt/_rels/presentation.xml.rels", rels(&prels));
    p.poner("ppt/presProps.xml", format!("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<p:presentationPr xmlns:a=\"{A}\" xmlns:r=\"{RN}\" xmlns:p=\"{P}\"/>"));
    p.poner("ppt/viewProps.xml", format!("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<p:viewPr xmlns:a=\"{A}\" xmlns:r=\"{RN}\" xmlns:p=\"{P}\"><p:normalViewPr><p:restoredLeft sz=\"15620\"/><p:restoredTop sz=\"94660\"/></p:normalViewPr><p:gridSpacing cx=\"76200\" cy=\"76200\"/></p:viewPr>"));
    p.poner("ppt/tableStyles.xml", format!("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<a:tblStyleLst xmlns:a=\"{A}\" def=\"{{5C22544A-7EE6-4342-B048-85BDC9FD1C3A}}\"/>"));
    let titulo = ds.first().and_then(|d| normalizar(d)["titulo"].as_str().map(str::to_string)).unwrap_or_else(|| crate::tr!("Presentación", "Presentation"));
    p.poner("docProps/core.xml", format!("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<cp:coreProperties xmlns:cp=\"http://schemas.openxmlformats.org/package/2006/metadata/core-properties\" xmlns:dc=\"http://purl.org/dc/elements/1.1/\" xmlns:dcterms=\"http://purl.org/dc/terms/\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\"><dc:title>{}</dc:title><dc:creator>GOD OF THE VOID</dc:creator></cp:coreProperties>", escapar(&titulo, false)));
    p.poner("docProps/app.xml", format!("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Properties xmlns=\"http://schemas.openxmlformats.org/officeDocument/2006/extended-properties\"><Application>GOD OF THE VOID</Application><Slides>{}</Slides></Properties>", ds.len()));
    p.poner("_rels/.rels", rels(&[("rId1".into(), format!("{RT}/officeDocument"), "ppt/presentation.xml".into()), ("rId2".into(), "http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties".into(), "docProps/core.xml".into()), ("rId3".into(), format!("{RT}/extended-properties"), "docProps/app.xml".into())]));
    p.poner("[Content_Types].xml", format!("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">{tipos}</Types>"));
    let errores = revisar_paquete(&p);
    if !errores.is_empty() {
        bail_tr!("la presentación generada no pasa la validación: {}", "the generated presentation fails validation: {}", errores.join("; "));
    }
    p.guardar(path)?;
    Ok(ds.len())
}

// ── leer ─────────────────────────────────────────────────────────────────────────────────────

/// Las diapositivas en el orden de `<p:sldIdLst>`: (número, parte).
fn orden_diapositivas(p: &Paquete) -> Result<Vec<String>> {
    let pres = Doc::leer(&p.texto("ppt/presentation.xml").ok_or_else(|| anyhow!("no es un .pptx (falta ppt/presentation.xml)"))?)?;
    let rels = Doc::leer(&p.texto("ppt/_rels/presentation.xml.rels").unwrap_or_default())?;
    let mut v = vec![];
    if let Some(lista) = pres.raiz.hijo("sldIdLst") {
        for s in lista.elems() {
            let rid = s.attr("r:id").unwrap_or_default();
            if let Some(r) = rels.raiz.elems().find(|r| r.attr("Id").as_deref() == Some(rid.as_str())) {
                v.push(crate::ooxml::resolver("ppt", &r.attr("Target").unwrap_or_default()));
            }
        }
    }
    Ok(v)
}

fn textos_de(e: &Elem) -> Vec<String> {
    let mut ps = vec![];
    e.buscar("p", &mut ps);
    ps.iter()
        .map(|p| {
            let mut ts = vec![];
            p.buscar("t", &mut ts);
            ts.iter().map(|t| t.texto()).collect::<String>()
        })
        .filter(|s| !s.trim().is_empty())
        .collect()
}

/// LEER: cada diapositiva con sus textos (de arriba abajo), tablas y datos de gráficos.
pub fn leer(path: &Path) -> Result<String> {
    let p = Paquete::abrir(path)?;
    let orden = orden_diapositivas(&p)?;
    let mut out = format!("{} · {} diapositivas\n", path.file_name().unwrap_or_default().to_string_lossy(), orden.len());
    for (i, parte) in orden.iter().enumerate() {
        out.push_str(&format!("\n<!-- Diapositiva {} -->\n", i + 1));
        let Some(x) = p.texto(parte) else { continue };
        let d = Doc::leer(&x)?;
        let Some(arbol) = d.raiz.hijo("cSld").and_then(|c| c.hijo("spTree")) else { continue };
        // formas ordenadas por posición (arriba → abajo, izquierda → derecha)
        let mut formas: Vec<(i64, i64, &Elem)> = arbol
            .elems()
            .filter(|e| matches!(e.local(), "sp" | "graphicFrame" | "grpSp" | "pic"))
            .map(|e| {
                let mut off = vec![];
                e.buscar("off", &mut off);
                let (x, y) = off.first().map(|o| (o.attr("x").and_then(|v| v.parse::<i64>().ok()).unwrap_or(0), o.attr("y").and_then(|v| v.parse::<i64>().ok()).unwrap_or(0))).unwrap_or((0, 0));
                // por el CENTRO vertical (redondeado a 0,1″): el número en su círculo va antes que su texto
                let mut ext = vec![];
                e.buscar("ext", &mut ext);
                let h = ext.first().and_then(|x| x.attr("cy")).and_then(|v| v.parse::<i64>().ok()).unwrap_or(0);
                let y = (y + h / 2) / (PULG / 10);
                (y, x, e)
            })
            .collect();
        formas.sort_by_key(|(y, x, _)| (*y, *x));
        let mut numero: Option<String> = None;
        for (_, _, e) in formas {
            let mut tbl = vec![];
            e.buscar("tbl", &mut tbl);
            if let Some(t) = tbl.first() {
                for (k, tr) in t.elems().filter(|x| x.local() == "tr").enumerate() {
                    let celdas: Vec<String> = tr.elems().filter(|x| x.local() == "tc").map(|tc| textos_de(tc).join(" ")).collect();
                    out.push_str(&format!("| {} |\n", celdas.join(" | ")));
                    if k == 0 {
                        out.push_str(&format!("|{}\n", "---|".repeat(celdas.len())));
                    }
                }
                continue;
            }
            let mut ch = vec![];
            e.buscar("chart", &mut ch);
            if let Some(c) = ch.first() {
                // el gráfico: su relación → su parte → sus series
                let rid = c.attr("r:id").unwrap_or_default();
                let rels_parte = format!("{}/_rels/{}.rels", parte.rsplit_once('/').map(|x| x.0).unwrap_or(""), parte.rsplit('/').next().unwrap_or(""));
                if let Some(rx) = p.texto(&rels_parte).and_then(|r| Doc::leer(&r).ok()) {
                    if let Some(rel) = rx.raiz.elems().find(|r| r.attr("Id").as_deref() == Some(rid.as_str())) {
                        let cp = crate::ooxml::resolver(parte.rsplit_once('/').map(|x| x.0).unwrap_or(""), &rel.attr("Target").unwrap_or_default());
                        if let Some(cd) = p.texto(&cp).and_then(|x| Doc::leer(&x).ok()) {
                            let mut sers = vec![];
                            cd.raiz.buscar("ser", &mut sers);
                            for s in sers {
                                let nombre = s.hijo("tx").map(textos_de_cache).unwrap_or_default().join("");
                                let cats = s.hijo("cat").map(textos_de_cache).unwrap_or_default();
                                let vals = s.hijo("val").map(textos_de_cache).unwrap_or_default();
                                let pares: Vec<String> = cats.iter().zip(&vals).map(|(c, v)| format!("{c}={v}")).collect();
                                out.push_str(&crate::tr!("[gráfico] {nombre}: {}\n", "[chart] {nombre}: {}\n", pares.join(", ")));
                            }
                        }
                    }
                }
                continue;
            }
            let ts = textos_de(e);
            // el número de un círculo («1», «2»…) se pega a su texto: «1. Definición…»
            if let [n] = ts.as_slice() {
                if (1..=2).contains(&n.len()) && n.chars().all(|c| c.is_ascii_digit()) {
                    numero = Some(n.clone());
                    continue;
                }
            }
            for (k, t) in ts.iter().enumerate() {
                match numero.take().filter(|_| k == 0) {
                    Some(n) => out.push_str(&format!("{n}. {t}\n")),
                    None => out.push_str(&format!("{t}\n")),
                }
            }
        }
        if let Some(n) = numero {
            out.push_str(&format!("{n}\n"));
        }
    }
    Ok(out)
}

fn textos_de_cache(e: &Elem) -> Vec<String> {
    let mut vs = vec![];
    e.buscar("v", &mut vs);
    vs.iter().map(|v| v.texto()).collect()
}

// ── editar ───────────────────────────────────────────────────────────────────────────────────

/// Une `<a:r>` contiguos con `<a:rPr>` idéntico (sin los atributos de corrección `dirty`/`err`):
/// la versión de PowerPoint del `merge_runs`.
fn unir_runs(e: &mut Elem) {
    for h in e.hijos.iter_mut() {
        if let Nodo::Elem(x) = h {
            if x.local() == "rPr" {
                x.quitar_attr("dirty");
                x.quitar_attr("err");
            }
            unir_runs(x);
        }
    }
    let mut i = 0;
    while i + 1 < e.hijos.len() {
        let par = match (&e.hijos[i], &e.hijos[i + 1]) {
            (Nodo::Elem(a), Nodo::Elem(b)) if a.local() == "r" && b.local() == "r" => Some((a.clone(), b.clone())),
            _ => None,
        };
        if let Some((a, b)) = par {
            let rpr = |x: &Elem| x.hijo("rPr").map(|r| {
                let mut s = String::new();
                r.escribir(&mut s);
                s
            });
            if rpr(&a) == rpr(&b) {
                let t = a.hijo("t").map(|t| t.texto()).unwrap_or_default() + &b.hijo("t").map(|t| t.texto()).unwrap_or_default();
                let mut nuevo = a;
                for h in nuevo.hijos.iter_mut() {
                    if let Nodo::Elem(x) = h {
                        if x.local() == "t" {
                            x.poner_texto(&t);
                        }
                    }
                }
                e.hijos[i] = Nodo::Elem(nuevo);
                e.hijos.remove(i + 1);
                continue;
            }
        }
        i += 1;
    }
}

fn reemplazar_en(e: &mut Elem, viejo: &str, nuevo: &str) -> usize {
    let mut n = 0;
    for h in e.hijos.iter_mut() {
        if let Nodo::Elem(x) = h {
            if x.local() == "t" {
                let t = x.texto();
                if t.contains(viejo) {
                    n += t.matches(viejo).count();
                    x.poner_texto(&t.replace(viejo, nuevo));
                }
            } else {
                n += reemplazar_en(x, viejo, nuevo);
            }
        }
    }
    n
}

/// EDITAR un .pptx: `[{"reemplazar":"texto","por":"nuevo"}, {"borrar_diapositiva": n},
/// {"duplicar_diapositiva": n}, {"agregar": [diapositivas como en crear], "tema"?}]`. Todo lo
/// estructural (borrar, duplicar, agregar) va ANTES que el contenido, como pide la guía.
pub fn editar(path: &Path, cambios: &Value) -> Result<String> {
    let cambios = crate::ooxml::tolerante_o_error(cambios, "cambios")?;
    let cambios = cambios.as_array().filter(|c| !c.is_empty()).ok_or_else(|| anyhow_tr!(
            "`cambios` es una lista: [{{\"reemplazar\":\"…\",\"por\":\"…\"}}, {{\"duplicar_diapositiva\":2}}, {{\"borrar_diapositiva\":3}}, {{\"agregar\":[…]}}]",
            "`cambios` is a list: [{{\"reemplazar\":\"…\",\"por\":\"…\"}}, {{\"duplicar_diapositiva\":2}}, {{\"borrar_diapositiva\":3}}, {{\"agregar\":[…]}}]"
        ))?;
    let mut p = Paquete::abrir(path)?;
    let mut informe = vec![];
    // estructura primero
    let mut ordenados: Vec<&Value> = cambios.iter().filter(|c| c.get("reemplazar").is_none()).collect();
    // medido (banco of-deck, 9B): repitió un reemplazo al final de la lista y agregó otros «X por X»;
    // el repetido ya no hallaba su texto y el lote entero fallaba. Lo idéntico y lo repetido se omite.
    let mut vistos: Vec<&Value> = vec![];
    for c in cambios.iter().filter(|c| c.get("reemplazar").is_some()) {
        if c["reemplazar"] != c["por"] && !vistos.contains(&c) {
            vistos.push(c);
        }
    }
    ordenados.extend(vistos);
    for c in ordenados {
        if let Some(v) = c["reemplazar"].as_str() {
            let nuevo = c["por"].as_str().unwrap_or_default();
            let mut total = 0;
            // tal cual y, si no aparece, sin las marcas de `leer` («1. » de los círculos, «**»)
            let limpio = crate::ooxml::sin_marcas_de_leer(v);
            let nuevo_limpio = crate::ooxml::sin_marcas_de_leer(nuevo).unwrap_or_else(|| nuevo.to_string());
            let intentos: Vec<(&str, &str)> = std::iter::once((v, nuevo)).chain(limpio.as_deref().map(|l| (l, nuevo_limpio.as_str()))).collect();
            for (buscar, poner) in intentos {
                for parte in orden_diapositivas(&p)? {
                    let mut d = Doc::leer(&p.texto(&parte).unwrap_or_default())?;
                    unir_runs(&mut d.raiz);
                    let n = reemplazar_en(&mut d.raiz, buscar, poner);
                    if n > 0 {
                        total += n;
                        p.poner(&parte, d.escribir());
                    }
                }
                if total > 0 {
                    break;
                }
            }
            if total == 0 {
                bail_tr!("no encontré «{v}» en la presentación (lee primero y copia el texto exacto)", "could not find «{v}» in the presentation (read first and copy the exact text)");
            }
            informe.push(format!("«{v}» → «{nuevo}» ({total})"));
        } else if let Some(n) = c["borrar_diapositiva"].as_u64() {
            borrar(&mut p, n as usize)?;
            informe.push(format!("diapositiva {n} borrada"));
        } else if let Some(n) = c["duplicar_diapositiva"].as_u64() {
            let nueva = duplicar(&mut p, n as usize)?;
            informe.push(crate::tr!("diapositiva {n} duplicada (la copia es la {nueva})", "slide {n} duplicated (the copy is {nueva})"));
        } else if let Some(ds) = c["agregar"].as_array() {
            // la paleta del deck, salvo que el cambio pida otra
            let t = if c["tema"].is_object() { tema(&c["tema"]) } else { tema_del_paquete(&p).unwrap_or_else(|| tema(&Value::Null)) };
            for d in ds {
                agregar(&mut p, d, &t)?;
            }
            informe.push(format!("{} diapositiva(s) agregadas al final", ds.len()));
        } else {
            bail!("cambio desconocido {c}: usa reemplazar/por, borrar_diapositiva, duplicar_diapositiva o agregar");
        }
    }
    let errores = revisar_paquete(&p);
    if !errores.is_empty() {
        bail_tr!("el resultado no pasa la validación (no lo guardé): {}", "the result fails validation (not saved): {}", errores.join("; "));
    }
    let r = crate::ooxml::respaldar(path)?;
    p.guardar(path)?;
    informe.push(format!("respaldo: {}", r.file_name().unwrap_or_default().to_string_lossy()));
    Ok(informe.join(" · "))
}

fn siguiente_libre(p: &Paquete, carpeta: &str, base: &str) -> usize {
    (1..).find(|i| !p.partes.contains_key(&format!("{carpeta}/{base}{i}.xml"))).unwrap()
}

/// Registra una diapositiva nueva (los 5 registros de `add_slide.py`): tipo de contenido, relación
/// con id nuevo en `presentation.xml.rels`, `<p:sldId>` con id nuevo (al final o tras `despues`),
/// y la cuenta en `docProps/app.xml`. Devuelve su número en el orden.
fn registrar(p: &mut Paquete, parte: &str, despues: Option<usize>) -> Result<usize> {
    let ct = p.texto("[Content_Types].xml").unwrap_or_default();
    p.poner("[Content_Types].xml", ct.replace("</Types>", &format!("<Override PartName=\"/{parte}\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.slide+xml\"/></Types>")));
    let pr = p.texto("ppt/_rels/presentation.xml.rels").unwrap_or_default();
    let rid = (1..).map(|i| format!("rIdS{i}")).find(|r| !pr.contains(&format!("Id=\"{r}\""))).unwrap();
    let target = parte.trim_start_matches("ppt/");
    p.poner("ppt/_rels/presentation.xml.rels", pr.replace("</Relationships>", &format!("<Relationship Id=\"{rid}\" Type=\"{RT}/slide\" Target=\"{target}\"/></Relationships>")));
    let mut pres = Doc::leer(&p.texto("ppt/presentation.xml").unwrap_or_default())?;
    let lista = pres
        .raiz
        .hijos
        .iter_mut()
        .find_map(|h| if let Nodo::Elem(x) = h { (x.local() == "sldIdLst").then_some(x) } else { None })
        .ok_or_else(|| anyhow_tr!("presentation.xml sin <p:sldIdLst>", "presentation.xml without <p:sldIdLst>"))?;
    let max_id = lista.elems().filter_map(|e| e.attr("id")?.parse::<u32>().ok()).max().unwrap_or(255);
    let mut nuevo = Elem::nuevo(&format!("{}sldId", if lista.nombre.contains(':') { "p:" } else { "" }));
    nuevo.attrs.push(("id".into(), (max_id + 1).to_string()));
    nuevo.attrs.push(("r:id".into(), rid));
    let pos_elems: Vec<usize> = lista.hijos.iter().enumerate().filter(|(_, h)| matches!(h, Nodo::Elem(_))).map(|(i, _)| i).collect();
    let (pos, numero) = match despues {
        Some(n) if n >= 1 && n <= pos_elems.len() => (pos_elems[n - 1] + 1, n + 1),
        _ => (lista.hijos.len(), pos_elems.len() + 1),
    };
    lista.hijos.insert(pos, Nodo::Elem(nuevo));
    lista.vacio = false;
    p.poner("ppt/presentation.xml", pres.escribir());
    contar_en_app(p);
    Ok(numero)
}

fn contar_en_app(p: &mut Paquete) {
    let n = orden_diapositivas(p).map(|v| v.len()).unwrap_or(0);
    if let Some(app) = p.texto("docProps/app.xml") {
        if let (Some(i), Some(j)) = (app.find("<Slides>"), app.find("</Slides>")) {
            p.poner("docProps/app.xml", format!("{}<Slides>{n}{}", &app[..i], &app[j..]));
        }
    }
}

fn borrar(p: &mut Paquete, n: usize) -> Result<()> {
    let orden = orden_diapositivas(p)?;
    if n == 0 || n > orden.len() {
        bail!("no hay diapositiva {n} (hay {})", orden.len());
    }
    if orden.len() == 1 {
        bail_tr!("no borro la única diapositiva", "I won't delete the only slide");
    }
    let parte = &orden[n - 1];
    let target = parte.trim_start_matches("ppt/");
    let mut pres = Doc::leer(&p.texto("ppt/presentation.xml").unwrap_or_default())?;
    let rels = p.texto("ppt/_rels/presentation.xml.rels").unwrap_or_default();
    let rd = Doc::leer(&rels)?;
    let rid = rd.raiz.elems().find(|r| r.attr("Target").as_deref() == Some(target)).and_then(|r| r.attr("Id")).ok_or_else(|| anyhow_tr!("no encuentro la relación de {parte}", "cannot find the relationship of {parte}"))?;
    if let Some(Nodo::Elem(lista)) = pres.raiz.hijos.iter_mut().find(|h| matches!(h, Nodo::Elem(x) if x.local() == "sldIdLst")) {
        lista.hijos.retain(|h| !matches!(h, Nodo::Elem(x) if x.attr("r:id").as_deref() == Some(rid.as_str())));
    }
    p.poner("ppt/presentation.xml", pres.escribir());
    // como `clean.py`: fuera la parte, su relación, sus .rels y su tipo de contenido
    let mut rd2 = rd;
    rd2.raiz.hijos.retain(|h| !matches!(h, Nodo::Elem(x) if x.attr("Id").as_deref() == Some(rid.as_str())));
    p.poner("ppt/_rels/presentation.xml.rels", rd2.escribir());
    p.partes.remove(parte);
    let (carpeta, archivo) = parte.rsplit_once('/').unwrap_or(("", parte));
    p.partes.remove(&format!("{carpeta}/_rels/{nombre_parte}.rels", nombre_parte = archivo));
    let ct = p.texto("[Content_Types].xml").unwrap_or_default();
    let pat = format!("<Override PartName=\"/{parte}\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.slide+xml\"/>");
    p.poner("[Content_Types].xml", ct.replace(&pat, ""));
    contar_en_app(p);
    Ok(())
}

fn duplicar(p: &mut Paquete, n: usize) -> Result<usize> {
    let orden = orden_diapositivas(p)?;
    let parte = orden.get(n.wrapping_sub(1)).cloned().ok_or_else(|| anyhow!("no hay diapositiva {n} (hay {})", orden.len()))?;
    let k = siguiente_libre(p, "ppt/slides", "slide");
    let nueva = format!("ppt/slides/slide{k}.xml");
    let xml = p.partes.get(&parte).cloned().unwrap_or_default();
    p.poner(&nueva, xml);
    let (carpeta, archivo) = parte.rsplit_once('/').unwrap();
    if let Some(r) = p.texto(&format!("{carpeta}/_rels/{nombre_parte}.rels", nombre_parte = archivo)) {
        // sin la relación de notas: la copia no comparte las notas de la original
        let mut d = Doc::leer(&r)?;
        d.raiz.hijos.retain(|h| !matches!(h, Nodo::Elem(x) if x.attr("Type").unwrap_or_default().ends_with("/notesSlide")));
        p.poner(&format!("ppt/slides/_rels/slide{k}.xml.rels"), d.escribir());
    }
    registrar(p, &nueva, Some(n))
}

fn agregar(p: &mut Paquete, d: &Value, t: &Tema) -> Result<usize> {
    let k = siguiente_libre(p, "ppt/slides", "slide");
    let dp = diapositiva(d, t, k)?;
    // el diseño del patrón que más se parezca (por su `type`); si no, el primero
    let disenos: Vec<String> = p.partes.keys().filter(|n| n.starts_with("ppt/slideLayouts/") && n.ends_with(".xml") && !n.contains("_rels")).cloned().collect();
    let quiere = match dp.diseno {
        1 => "title",
        2 => "obj",
        3 => "titleOnly",
        _ => "blank",
    };
    let diseno = disenos.iter().find(|n| p.texto(n).is_some_and(|x| x.contains(&format!("type=\"{quiere}\"")))).or_else(|| disenos.iter().find(|n| p.texto(n).is_some_and(|x| x.contains("type=\"blank\"")))).or(disenos.first()).cloned().ok_or_else(|| anyhow_tr!("la presentación no tiene diseños de diapositiva", "the presentation has no slide layouts"))?;
    let mut srels = vec![("rId1".to_string(), format!("{RT}/slideLayout"), format!("../slideLayouts/{}", diseno.rsplit('/').next().unwrap()))];
    let mut xml = dp.xml;
    for (j, g) in dp.graficos.iter().enumerate() {
        let c = siguiente_libre(p, "ppt/charts", "chart");
        srels.push((format!("rIdG{}", j + 1), format!("{RT}/chart"), format!("../charts/chart{c}.xml")));
        p.poner(&format!("ppt/charts/chart{c}.xml"), chart_xml(g, t));
        p.poner(&format!("ppt/charts/_rels/chart{c}.xml.rels"), rels(&[("rId1".into(), format!("{RT}/package"), format!("../embeddings/Microsoft_Excel_Worksheet_gotv{c}.xlsx"))]));
        p.poner(&format!("ppt/embeddings/Microsoft_Excel_Worksheet_gotv{c}.xlsx"), hoja_del_grafico(g)?);
        let ct = p.texto("[Content_Types].xml").unwrap_or_default();
        let mut ct2 = ct.replace("</Types>", &format!("<Override PartName=\"/ppt/charts/chart{c}.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.drawingml.chart+xml\"/></Types>"));
        if !ct2.contains("Extension=\"xlsx\"") {
            ct2 = ct2.replace("</Types>", "<Default Extension=\"xlsx\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.sheet\"/></Types>");
        }
        p.poner("[Content_Types].xml", ct2);
    }
    if dp.graficos.is_empty() {
        xml = xml.replace("rIdG1", "rId1");
    }
    let nueva = format!("ppt/slides/slide{k}.xml");
    p.poner(&nueva, xml);
    p.poner(&format!("ppt/slides/_rels/slide{k}.xml.rels"), rels(&srels));
    registrar(p, &nueva, None)
}

// ── revisar ──────────────────────────────────────────────────────────────────────────────────

/// Lo de `validators/pptx.py` + `helpers/pptx_slide.py`: paquete, colores de 6 hex, posiciones y
/// tamaños numéricos, `<p:txBody>` no vacío, ids de diapositiva únicos y ≥ 256, ids de forma únicos
/// por diapositiva, animaciones que apuntan a formas que ya no están, series con `c:order` único.
pub fn revisar_paquete(p: &Paquete) -> Vec<String> {
    let mut e = crate::ooxml::validar_paquete(p);
    for (n, b) in &p.partes {
        if !n.ends_with(".xml") || !(n.starts_with("ppt/slides/") || n.starts_with("ppt/slideLayouts/") || n.starts_with("ppt/slideMasters/") || n.starts_with("ppt/charts/")) {
            continue;
        }
        let x = String::from_utf8_lossy(b);
        let Ok(d) = Doc::leer(&x) else { continue };
        let mut c = vec![];
        d.raiz.buscar("srgbClr", &mut c);
        for s in c {
            let v = s.attr("val").unwrap_or_default();
            if hex(&v).filter(|h| *h == v.to_uppercase()).is_none() {
                e.push(crate::tr!("{n}: color «{v}» no es de 6 dígitos hex", "{n}: color «{v}» is not 6 hex digits"));
            }
        }
        for tag in ["off", "ext", "chOff", "chExt"] {
            let mut v = vec![];
            d.raiz.buscar(tag, &mut v);
            for o in v {
                for (k, val) in &o.attrs {
                    if matches!(k.as_str(), "x" | "y" | "cx" | "cy") && val.parse::<i64>().is_err() {
                        e.push(crate::tr!("{n}: <a:{tag} {k}=\"{val}\"> no es un número", "{n}: <a:{tag} {k}=\"{val}\"> is not a number"));
                    }
                }
            }
        }
        let mut tb = vec![];
        d.raiz.buscar("txBody", &mut tb);
        if tb.iter().any(|t| t.elems().next().is_none()) {
            e.push(crate::tr!("{n}: un <p:txBody> sin hijos", "{n}: a <p:txBody> without children"));
        }
        if n.starts_with("ppt/slides/") {
            let mut ids = vec![];
            d.raiz.buscar("cNvPr", &mut ids);
            let mut vistos = std::collections::HashSet::new();
            for i in &ids {
                if let Some(v) = i.attr("id") {
                    if !vistos.insert(v.clone()) {
                        e.push(format!("{n}: id de forma {v} repetido"));
                    }
                }
            }
            let mut anim = vec![];
            d.raiz.buscar("spTgt", &mut anim);
            for a in anim {
                if let Some(spid) = a.attr("spid") {
                    if !vistos.contains(&spid) {
                        e.push(crate::tr!("{n}: una animación apunta a la forma {spid}, que ya no está (borra su <p:timing>)", "{n}: an animation points at shape {spid}, which is gone (delete its <p:timing>)"));
                    }
                }
            }
        }
        if n.starts_with("ppt/charts/") {
            let mut ords = vec![];
            d.raiz.buscar("order", &mut ords);
            let vals: Vec<String> = ords.iter().filter_map(|o| o.attr("val")).collect();
            let mut u = vals.clone();
            u.sort();
            u.dedup();
            if u.len() != vals.len() {
                e.push(format!("{n}: dos series comparten c:order"));
            }
        }
    }
    if let Some(pr) = p.texto("ppt/presentation.xml").and_then(|x| Doc::leer(&x).ok()) {
        if let Some(l) = pr.raiz.hijo("sldIdLst") {
            let ids: Vec<u32> = l.elems().filter_map(|s| s.attr("id")?.parse().ok()).collect();
            let mut u = ids.clone();
            u.sort();
            u.dedup();
            if u.len() != ids.len() || ids.iter().any(|i| *i < 256) {
                e.push("presentation.xml: ids de diapositiva repetidos o < 256".into());
            }
        }
    }
    e
}

pub fn revisar(path: &Path) -> Result<Vec<String>> {
    Ok(revisar_paquete(&Paquete::abrir(path)?))
}

// ── ¿cabe el texto? (O3: la regla «no entregues texto que se sale de su caja», MEDIDA) ─────────

/// Ancho medio de un carácter en «em» (fracción del tamaño de letra). Calibrado con PowerPoint:
/// un título de Cambria negrita a 36 pt dibujó líneas de 48-51 caracteres en 12,1″ (873 pt) → ≤ 0,475 em.
fn em_por_caracter(negrita: bool) -> f64 {
    if negrita {
        0.47
    } else {
        0.44
    }
}

/// Líneas que ocupa `texto` a `pt` en un ancho de `ancho` EMU (ajuste por palabras).
pub(crate) fn lineas(texto: &str, pt: f64, negrita: bool, ancho: i64) -> u32 {
    let ancho_pt = ancho as f64 / 12700.0;
    let por_car = pt * em_por_caracter(negrita);
    if ancho_pt <= 0.0 || por_car <= 0.0 {
        return 1;
    }
    let (mut n, mut linea) = (1u32, 0.0f64);
    for palabra in texto.split_whitespace() {
        let w = palabra.chars().count() as f64 * por_car;
        let sep = if linea > 0.0 { por_car } else { 0.0 };
        if linea > 0.0 && linea + sep + w > ancho_pt {
            n += 1;
            linea = w;
        } else {
            linea += sep + w;
        }
        // una palabra más ancha que la caja se parte
        while linea > ancho_pt {
            n += 1;
            linea -= ancho_pt;
        }
    }
    n
}

/// El tamaño de título que CABE en su caja (de `pt` hacia abajo, hasta 28 pt: la guía pide
/// títulos grandes; si ni así cabe, se queda en 28 y `avisos` lo dice).
fn pt_que_cabe(texto: &str, pt: u32, ancho: i64, alto: i64) -> u32 {
    let mut p = pt;
    while p > 28 && f64::from(lineas(texto, f64::from(p), true, ancho)) * f64::from(p) * 1.2 > alto as f64 / 12700.0 {
        p -= 2;
    }
    p
}

/// Textos que NO caben en su cuadro, diapositiva por diapositiva (estimación; tablas y gráficos
/// aparte). Atrapa lo que la QA visual vería: un título largo encima de los puntos.
pub fn desbordes(path: &Path) -> Result<Vec<String>> {
    let p = Paquete::abrir(path)?;
    let mut out = vec![];
    for (i, parte) in orden_diapositivas(&p)?.iter().enumerate() {
        let Some(x) = p.texto(parte) else { continue };
        let d = Doc::leer(&x)?;
        let mut formas = vec![];
        d.raiz.buscar("sp", &mut formas);
        for sp in formas {
            let Some(tx) = sp.hijo("txBody") else { continue };
            let cuerpo = tx.hijo("bodyPr");
            if cuerpo.and_then(|b| b.attr("wrap")).as_deref() == Some("none") {
                continue;
            }
            let mut ext = vec![];
            sp.buscar("ext", &mut ext);
            let Some(e) = ext.first() else { continue };
            let (Some(cx), Some(cy)) = (e.attr("cx").and_then(|v| v.parse::<i64>().ok()), e.attr("cy").and_then(|v| v.parse::<i64>().ok())) else { continue };
            // márgenes internos (por defecto 0,1″ a los lados y 0,05″ arriba y abajo)
            let ins = |k: &str, def: i64| cuerpo.and_then(|b| b.attr(k)).and_then(|v| v.parse::<i64>().ok()).unwrap_or(def);
            let ancho = cx - ins("lIns", 91440) - ins("rIns", 91440);
            let alto_pt = (cy - ins("tIns", 45720) - ins("bIns", 45720)) as f64 / 12700.0;
            let mut necesita = 0.0;
            let mut muestra = String::new();
            for par in tx.elems().filter(|e| e.local() == "p") {
                let mut runs = vec![];
                par.buscar("r", &mut runs);
                let texto: String = runs.iter().filter_map(|r| r.hijo("t")).map(|t| t.texto()).collect();
                let pt = runs
                    .iter()
                    .filter_map(|r| r.hijo("rPr").and_then(|p| p.attr("sz")).and_then(|v| v.parse::<f64>().ok()))
                    .fold(0.0, f64::max)
                    / 100.0;
                let pt = if pt > 0.0 { pt } else { 18.0 };
                let negrita = runs.iter().any(|r| r.hijo("rPr").and_then(|p| p.attr("b")).as_deref() == Some("1"));
                let n = if texto.trim().is_empty() { 1 } else { lineas(&texto, pt, negrita, ancho) };
                let despues = par.hijo("pPr").and_then(|p| p.hijo("spcAft")).and_then(|s| s.hijo("spcPts")).and_then(|s| s.attr("val")).and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0) / 100.0;
                necesita += f64::from(n) * pt * 1.2 + despues;
                if muestra.is_empty() && !texto.trim().is_empty() {
                    muestra = texto.chars().take(40).collect();
                }
            }
            // 10 % de holgura: la estimación es aproximada y lo justo no se marca
            if necesita > alto_pt * 1.1 && !muestra.is_empty() {
                out.push(crate::tr!(
                    "diapositiva {}: «{muestra}…» no cabe en su cuadro (necesita ~{:.1}″ de alto, tiene {:.1}″): acórtalo o pártelo en dos diapositivas",
                    "slide {}: «{muestra}…» does not fit its box (needs ~{:.1}″ of height, has {:.1}″): shorten it or split it into two slides",
                    i + 1,
                    necesita / 72.0,
                    alto_pt / 72.0
                ));
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod pruebas {
    use super::*;
    use serde_json::json;

    pub fn ficha() -> Value {
        json!({"tema": {"paleta": "teal"}, "diapositivas": [
            {"tipo": "portada", "titulo": "Inteligencia artificial", "subtitulo": "Qué es y cómo usarla"},
            {"tipo": "puntos", "titulo": "Por qué importa", "puntos": ["Automatiza tareas repetitivas", "Ayuda a decidir con datos", "Crea contenido"]},
            {"tipo": "dos_columnas", "titulo": "Antes y después", "titulo_izquierda": "Antes", "izquierda": ["Manual", "Lento"], "titulo_derecha": "Después", "derecha": ["Automático", "Rápido"]},
            {"tipo": "cifra", "titulo": "Adopción", "cifra": "72 %", "texto": "de las empresas ya usa IA en algún proceso"},
            {"tipo": "tabla", "titulo": "Herramientas", "tabla": [["Uso", "Ejemplo"], ["Texto", "Asistentes"], ["Imagen", "Generadores"]]},
            {"tipo": "grafico", "titulo": "Inversión anual", "grafico": {"tipo": "columnas", "titulo": "Millones USD", "categorias": ["2023", "2024", "2025"], "series": [{"nombre": "Inversión", "valores": [120, 180, 260]}]}, "texto": "Crece más del 40 % al año"},
            {"tipo": "cierre", "titulo": "Gracias", "subtitulo": "Preguntas"}
        ]})
    }

    #[test]
    fn crear_leer_y_revisar() {
        let d = crate::CarpetaPrueba::nueva("gotv-ppt");
        let p = d.join("ia.pptx");
        assert_eq!(crear(&p, &ficha()).unwrap(), 7);
        let errores = revisar(&p).unwrap();
        assert!(errores.is_empty(), "{errores:?}");
        let pq = Paquete::abrir(&p).unwrap();
        assert!(pq.partes.contains_key("ppt/embeddings/Microsoft_Excel_Worksheet1.xlsx"), "el gráfico lleva su hoja");
        let s2 = pq.texto("ppt/slides/slide2.xml").unwrap();
        assert!(!s2.contains('\u{2022}'), "puntos sin viñeta literal");
        assert!(s2.contains("txBox=\"1\""), "cuadros de texto marcados como texto");
        let l = leer(&p).unwrap();
        println!("{l}");
        assert!(l.contains("7 diapositivas") && l.contains("Inteligencia artificial") && l.contains("Automatiza tareas repetitivas"));
        assert!(l.contains("| Uso | Ejemplo |") && l.contains("[gráfico] Inversión: 2023=120, 2024=180, 2025=260"), "{l}");
    }

    #[test]
    fn editar_estructura_y_texto() {
        let d = crate::CarpetaPrueba::nueva("gotv-ppt-editar");
        let p = d.join("ia.pptx");
        crear(&p, &ficha()).unwrap();
        let inf = editar(&p, &json!([
            {"reemplazar": "Por qué importa", "por": "Por qué nos importa"},
            {"duplicar_diapositiva": 2},
            {"borrar_diapositiva": 5},
            {"agregar": [{"tipo": "puntos", "titulo": "Próximos pasos", "puntos": ["Piloto", "Medir"]}, {"tipo": "grafico", "titulo": "Uso", "grafico": {"tipo": "torta", "categorias": ["A", "B"], "valores": [60, 40]}}]}
        ])).unwrap();
        println!("{inf}");
        let l = leer(&p).unwrap();
        println!("{l}");
        assert!(l.contains("9 diapositivas"), "7 + 1 copia − 1 borrada + 2 agregadas: {l}");
        assert_eq!(l.matches("Por qué nos importa").count(), 2, "el reemplazo alcanzó la copia también");
        assert!(l.contains("Próximos pasos") && l.contains("[gráfico] Serie 1: A=60, B=40"));
        assert!(revisar(&p).unwrap().is_empty(), "{:?}", revisar(&p).unwrap());
        let app = Paquete::abrir(&p).unwrap().texto("docProps/app.xml").unwrap();
        assert!(app.contains("<Slides>9</Slides>"), "la cuenta en app.xml: {app}");
    }

    #[test]
    fn las_formas_del_banco_se_entienden() {
        // diapositivas anidadas bajo su tipo (lo que mandó el 9B en of-deck)
        let d = crate::CarpetaPrueba::nueva("gotv-ppt-formas");
        let p = d.join("cafe.pptx");
        crear(&p, &json!({"diapositivas": [
            {"tipo": "portada", "titulo": "El Café de Colombia"},
            {"titulo": "Liderazgo", "cifra": {"cifra": "80%", "texto": "de los granos suaves", "tipo": "cifra"}},
            {"titulo": "Cosecha", "dos_columnas": {"izquierda": ["A mano"], "derecha": ["Al sol"], "titulo_izquierda": "Cosecha", "titulo_derecha": "Secado"}}
        ]})).unwrap();
        let l = leer(&p).unwrap();
        assert!(l.contains("80%") && !l.contains("{\"cifra\"") && l.contains("A mano") && l.contains("Secado"), "{l}");
        assert!(desbordes(&p).unwrap().is_empty(), "{:?}", desbordes(&p).unwrap());
        // un lote con «X por X» y un reemplazo repetido ya no tumba los cambios buenos
        let inf = editar(&p, &json!([
            {"reemplazar": "Liderazgo", "por": "Liderazgo mundial"},
            {"reemplazar": "Cosecha", "por": "Cosecha"},
            {"reemplazar": "Liderazgo", "por": "Liderazgo mundial"}
        ])).unwrap();
        assert!(inf.contains("Liderazgo mundial") && leer(&p).unwrap().contains("Liderazgo mundial"), "{inf}");
    }

    #[test]
    fn la_ficha_como_texto_roto_dice_donde_se_rompe() {
        // la forma real del 9B en el banco (of-deck): la ficha como TEXTO y un arreglo con «clave: valor»
        let rota = json!("{\"diapositivas\": [{\"tipo\": \"portada\", \"titulo\": \"El Café de Colombia\"}, {\"dos_columnas\": [\"derecha\": [\"Cereza roja\"], \"izquierda\": [\"Cosecha\"]], \"titulo\": \"Proceso\"}]}");
        let d = crate::CarpetaPrueba::nueva("gotv-ppt-rota");
        let e = crear(&d.join("x.pptx"), &rota).unwrap_err().to_string();
        assert!(e.contains("llegó como TEXTO") && e.contains("\"derecha\": [") && e.contains("OBJETO"), "{e}");
        // la misma ficha como texto BIEN formado sí se acepta
        let bien = json!("{\"diapositivas\": [{\"tipo\": \"portada\", \"titulo\": \"El Café de Colombia\"}]}");
        assert_eq!(crear(&d.join("y.pptx"), &bien).unwrap(), 1);
    }

    #[test]
    fn el_texto_que_no_cabe_se_avisa_y_el_titulo_se_ajusta() {
        // la medida: 3 líneas de ~50 caracteres a 36 pt en 12,1″ (lo que dibujó PowerPoint)
        let largo = "Las ventajas más importantes de instalar paneles solares en una vivienda familiar de la ciudad según los expertos consultados en el estudio de este año";
        assert_eq!(lineas(largo, 36.0, true, ANCHO - 2 * (PULG / 2 + PULG / 10)), 3);
        assert_eq!(lineas("Ventajas", 36.0, true, ANCHO), 1);
        // un título de dos líneas baja de 36 a un tamaño que cabe; uno corto queda en 36
        let dos = "Las ventajas de instalar paneles solares en una vivienda de la ciudad";
        let ancho = ANCHO - 2 * (PULG / 2 + PULG / 10);
        assert_eq!(pt_que_cabe("Ventajas", 36, ancho, PULG), 36);
        let p = pt_que_cabe(dos, 36, ancho, PULG);
        assert!(p < 36 && p >= 28 && f64::from(lineas(dos, f64::from(p), true, ancho)) * f64::from(p) * 1.2 <= 72.0, "{p}");

        let d = crate::CarpetaPrueba::nueva("gotv-ppt-cabe");
        let x = d.join("x.pptx");
        crear(&x, &ficha()).unwrap();
        assert!(desbordes(&x).unwrap().is_empty(), "el deck de prueba cabe: {:?}", desbordes(&x).unwrap());
        // sembrado: reemplazar un título por uno larguísimo (lo que haría un «editar» descuidado)
        editar(&x, &json!([{"reemplazar": "Por qué importa", "por": largo}])).unwrap();
        let v = desbordes(&x).unwrap();
        assert_eq!(v.len(), 1, "{v:?}");
        assert!(v[0].starts_with("diapositiva 2:") && v[0].contains("no cabe"), "{v:?}");
    }

    #[test]
    fn lo_agregado_sigue_la_paleta_del_deck() {
        let d = crate::CarpetaPrueba::nueva("gotv-ppt-paleta");
        let p = d.join("ia.pptx");
        crear(&p, &ficha()).unwrap(); // paleta teal
        let teal = PALETAS.iter().find(|(n, _)| *n == "teal").unwrap().1[0];
        let t = tema_del_paquete(&Paquete::abrir(&p).unwrap()).unwrap();
        assert_eq!(t.primario, teal);
        editar(&p, &json!([{"agregar": [{"tipo": "cierre", "titulo": "Fin"}]}])).unwrap();
        let pq = Paquete::abrir(&p).unwrap();
        let ultima = orden_diapositivas(&pq).unwrap().pop().unwrap();
        let x = pq.texto(&ultima).unwrap();
        assert!(x.contains(teal), "la diapositiva agregada usa el teal del deck");
        assert!(!x.contains(PALETAS[0].1[0]), "y no el midnight por defecto");
    }
}

#[cfg(test)]
mod con_powerpoint {
    use super::*;
    use serde_json::json;

    /// El juez de verdad: el PowerPoint del usuario abre lo creado y lo editado (si estuviera roto
    /// pediría «reparar» y la conversión fallaría) y lo convierte a PDF.
    /// `cargo test -p gotv-core --release -- --ignored --nocapture powerpoint_abre_lo_creado`
    #[tokio::test]
    #[ignore]
    async fn powerpoint_abre_lo_creado() {
        let d = crate::CarpetaPrueba::nueva("gotv-ppt-real");
        let p = d.join("ia.pptx");
        crear(&p, &pruebas::ficha()).unwrap();
        crate::apps::exportar_pdf(&p, &d.join("ia.pdf"), 180).await.unwrap();
        println!("creado → PDF {} bytes", std::fs::metadata(d.join("ia.pdf")).unwrap().len());
        editar(&p, &json!([{"duplicar_diapositiva": 2}, {"borrar_diapositiva": 3}, {"agregar": [{"tipo": "grafico", "titulo": "Uso", "grafico": {"tipo": "lineas", "categorias": ["A", "B", "C"], "series": [{"nombre": "X", "valores": [1, 3, 2]}, {"nombre": "Y", "valores": [2, 1, 3]}]}}]}])).unwrap();
        crate::apps::exportar_pdf(&p, &d.join("editada.pdf"), 180).await.unwrap();
        println!("editado → PDF {} bytes", std::fs::metadata(d.join("editada.pdf")).unwrap().len());
        // para mirarlo a mano si hace falta
        let _ = std::fs::copy(d.join("ia.pdf"), "/tmp/gotv-ppt-prueba.pdf");
    }
}

#[cfg(test)]
mod aislar {
    use super::*;

    /// Una presentación por TIPO de diapositiva → ¿la abre PowerPoint? (para aislar un defecto)
    /// `cargo test -p gotv-core --release -- --ignored --nocapture aislar_tipos`
    #[tokio::test]
    #[ignore]
    async fn aislar_tipos() {
        let d = crate::CarpetaPrueba::nueva("gotv-ppt-aislar");
        let f = pruebas::ficha();
        for (i, dp) in f["diapositivas"].as_array().unwrap().iter().enumerate() {
            let p = d.join(format!("t{i}.pptx"));
            crear(&p, &serde_json::json!({"tema": f["tema"], "diapositivas": [dp]})).unwrap();
            let r = crate::apps::exportar_pdf(&p, &d.join(format!("t{i}.pdf")), 40).await;
            println!("{:14} {}", dp["tipo"].as_str().unwrap(), if r.is_ok() { "✓ abre".to_string() } else { format!("✗ {}", r.unwrap_err().to_string().chars().take(90).collect::<String>()) });
        }
    }
}

#[cfg(test)]
mod volcar {
    /// Escribe la presentación de prueba en `GOTV_VOLCAR` (para validarla parte por parte con los
    /// XSD oficiales). `GOTV_VOLCAR=/ruta/x.pptx cargo test -p gotv-core -- --ignored volcar_ppt`
    #[test]
    #[ignore]
    fn volcar_ppt() {
        let Ok(p) = std::env::var("GOTV_VOLCAR") else { return };
        super::crear(std::path::Path::new(&p), &super::pruebas::ficha()).unwrap();
    }

    /// Crea un .pptx desde una ficha JSON: `GOTV_FICHA=f.json GOTV_SALIDA=x.pptx cargo test -p gotv-core -- --ignored crear_ppt`
    #[test]
    #[ignore]
    fn crear_ppt() {
        let (Ok(f), Ok(s)) = (std::env::var("GOTV_FICHA"), std::env::var("GOTV_SALIDA")) else { return };
        let ficha: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(f).unwrap()).unwrap();
        super::crear(std::path::Path::new(&s), &ficha).unwrap();
    }

    /// Lee y revisa un .pptx cualquiera: `GOTV_LEER=/ruta/x.pptx cargo test -p gotv-core -- --ignored leer_ppt --nocapture`
    #[test]
    #[ignore]
    fn leer_ppt() {
        let Ok(p) = std::env::var("GOTV_LEER") else { return };
        let p = std::path::Path::new(&p);
        println!("{}\n— revisión: {:?}\n— desbordes: {:?}", super::leer(p).unwrap(), super::revisar(p).unwrap(), super::desbordes(p).unwrap());
    }
}
