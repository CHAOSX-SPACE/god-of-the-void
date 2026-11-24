//! Carta con el formato de la **GTC 185** (ICONTEC, 2009), la guía colombiana de documentación
//! organizacional — leída de primera mano (ver la esencia `gtc185-carta` del Abismo). Juicio del
//! Portador (2026-11-11) sobre la carta que hizo el 9B con bloques sueltos: «sin formato de carta,
//! sin fecha arriba, el destinatario sin mayúscula, pasa a dos hojas sin necesidad».
//!
//! Lo que aplica (numerales de la GTC 185):
//! - márgenes 4.2: superior 3–4 cm, inferior 2–3, izquierdo 3–4, derecho 2–3;
//! - fuente 4.4: 10–12 pt, homogénea, SIN negrilla;
//! - estilos 4.3: bloque extremo (todo a la izquierda), bloque (fecha y firma desde el centro),
//!   semibloque (bloque + sangría de primera línea);
//! - lugar y fecha 4.5.3 completos («Medellín, 11 de noviembre de 2026»);
//! - destinatario 5.4.3 a 2–3 interlíneas: tratamiento, NOMBRE en mayúscula fija, cargo,
//!   organización, dirección, ciudad (nunca «La ciudad» ni «Presente»);
//! - asunto 4.5.5 a 2–3 interlíneas, sin negrilla ni subrayado; saludo 5.4.5 a 1–2; texto 5.4.6 a
//!   1–2, sencillo, una interlínea entre párrafos; despedida 5.4.7 a 1–2;
//! - remitente 5.4.8 a 4–5 interlíneas (el espacio de la firma), NOMBRE en mayúscula fija y el
//!   cargo en la línea siguiente;
//! - líneas especiales 4.5.8 (anexos, copias, transcriptor) a 2 interlíneas, en letra menor;
//! - páginas siguientes 4.5.1: encabezado con el destinatario y el número de página.
//!
//! Y lo que pidió el Portador: que quepa en UNA hoja si puede. Se prueban de más holgado a más
//! compacto (interlíneas dentro de lo que permite la norma y luego 10,5 y 10 pt); sólo si ni así
//! cabe va en dos hojas, con su encabezado de página siguiente.

use crate::ooxml::escapar;
use crate::word::{escribir_docx, R, W};
use anyhow::Result;
use serde_json::Value;
use std::path::Path;

const CM: i64 = 567; // twips por centímetro

/// Interlíneas libres entre partes y tamaño de letra de un intento de maquetación.
#[derive(Clone, Copy, Debug)]
struct Holgura {
    margen_sup: i64,
    fecha_dest: u32,
    dest_asunto: u32,
    asunto_saludo: u32,
    saludo_texto: u32,
    texto_desp: u32,
    desp_firma: u32,
    medio_pt: u32, // tamaño en medios puntos (22 = 11 pt)
}

/// De más holgado a más compacto, todos dentro de la GTC 185.
const INTENTOS: [Holgura; 5] = [
    Holgura { margen_sup: 4 * CM, fecha_dest: 3, dest_asunto: 3, asunto_saludo: 2, saludo_texto: 2, texto_desp: 2, desp_firma: 5, medio_pt: 22 },
    Holgura { margen_sup: 4 * CM, fecha_dest: 2, dest_asunto: 2, asunto_saludo: 1, saludo_texto: 1, texto_desp: 1, desp_firma: 4, medio_pt: 22 },
    Holgura { margen_sup: 3 * CM, fecha_dest: 2, dest_asunto: 2, asunto_saludo: 1, saludo_texto: 1, texto_desp: 1, desp_firma: 4, medio_pt: 22 },
    Holgura { margen_sup: 3 * CM, fecha_dest: 2, dest_asunto: 2, asunto_saludo: 1, saludo_texto: 1, texto_desp: 1, desp_firma: 4, medio_pt: 21 },
    Holgura { margen_sup: 3 * CM, fecha_dest: 2, dest_asunto: 2, asunto_saludo: 1, saludo_texto: 1, texto_desp: 1, desp_firma: 4, medio_pt: 20 },
];

const MARGEN_IZQ: i64 = 3 * CM;
const MARGEN_DER: i64 = 2 * CM;
const MARGEN_INF: i64 = 5 * CM / 2;

/// Una línea de la carta: texto, alineación y sangría.
#[derive(Clone, Debug)]
enum Linea {
    Vacia,
    Texto { t: String, jc: &'static str, ind: i64, primera: i64, menor: bool, subrayar: bool },
}

fn t(s: impl Into<String>) -> Linea {
    Linea::Texto { t: s.into(), jc: "left", ind: 0, primera: 0, menor: false, subrayar: false }
}

/// La carta ya resuelta (textos y estilo), lista para maquetar con una `Holgura`.
struct Partes {
    codigo: Option<String>,
    lugar_fecha: String,
    destinatario: Vec<Linea>,
    asunto: Option<String>,
    saludo: String,
    parrafos: Vec<String>,
    despedida: String,
    remitente: Vec<String>, // [NOMBRE, cargo, …]
    especiales: Vec<String>,
    estilo: &'static str,
    nombre_dest: String,
}

fn txt(v: &Value) -> Option<String> {
    v.as_str().map(|s| s.trim().replace("**", "")).filter(|s| !s.is_empty())
}

fn primero(o: &Value, ks: &[&str]) -> Option<String> {
    ks.iter().find_map(|k| txt(&o[*k]))
}

const MESES: [&str; 12] = ["enero", "febrero", "marzo", "abril", "mayo", "junio", "julio", "agosto", "septiembre", "octubre", "noviembre", "diciembre"];

/// La fecha de HOY en el reloj local («11 de noviembre de 2026»; mes en minúscula, año sin punto).
fn hoy_largo() -> String {
    let s = std::process::Command::new("date").arg("+%d %m %Y").output().ok().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default();
    let v: Vec<u32> = s.split_whitespace().filter_map(|x| x.parse().ok()).collect();
    match v.as_slice() {
        [d, m, y] if (1..=12).contains(m) => format!("{d} de {} de {y}", MESES[*m as usize - 1]),
        _ => String::new(),
    }
}

fn resolver(c: &Value) -> Result<Partes> {
    let estilo = match c["estilo"].as_str().unwrap_or("").to_lowercase().replace([' ', '-'], "_").as_str() {
        "bloque" => "bloque",
        "semibloque" => "semibloque",
        _ => "bloque_extremo",
    };
    let fecha = primero(c, &["fecha"]).unwrap_or_else(hoy_largo);
    let lugar_fecha = match primero(c, &["ciudad", "lugar"]) {
        Some(ciudad) if !fecha.is_empty() => format!("{ciudad}, {fecha}"),
        Some(ciudad) => ciudad,
        None => fecha,
    };
    // destinatario: objeto con sus partes, o un texto (una organización: carta general)
    let d = &c["destinatario"];
    let subrayar = c["subrayar_destinatario"].as_bool().unwrap_or(false);
    let mut destinatario = vec![];
    let mut nombre_dest = String::new();
    let mut tratamiento = None;
    if d.is_object() {
        tratamiento = primero(d, &["tratamiento", "titulo", "denominacion"]);
        if let Some(tr) = &tratamiento {
            destinatario.push(t(tr.clone()));
        }
        if let Some(n) = primero(d, &["nombre", "name"]) {
            nombre_dest = n.to_uppercase();
            destinatario.push(Linea::Texto { t: nombre_dest.clone(), jc: "left", ind: 0, primera: 0, menor: false, subrayar });
        }
        for k in [&["cargo"][..], &["organizacion", "organización", "empresa", "entidad"], &["direccion", "dirección", "correo", "email"], &["ciudad", "lugar"], &["pais", "país"]] { // juez-idioma: datos (claves que acepta la ficha)
            if let Some(v) = primero(d, k) {
                destinatario.push(t(v));
            }
        }
        if nombre_dest.is_empty() {
            nombre_dest = primero(d, &["organizacion", "organización", "empresa", "entidad"]).unwrap_or_default(); // juez-idioma: datos (claves que acepta la ficha)
        }
    } else if let Some(org) = txt(d) {
        // GTC 5.4.3: sin nombre, a la dependencia u organización
        tratamiento = Some("Señores".into()); // juez-idioma: datos (texto de la carta GTC 185, norma colombiana en español)
        destinatario.push(t("Señores")); // juez-idioma: datos (texto de la carta GTC 185, norma colombiana en español)
        destinatario.push(t(org.clone()));
        nombre_dest = org;
    }
    if destinatario.is_empty() {
        return Err(gotv_engine::anyhow_tr!(
            "la carta necesita «destinatario»: {{\"tratamiento\":\"Señor\",\"nombre\":\"Carlos Pérez Gómez\",\"cargo\":\"Gerente\",\"organizacion\":\"Hotel Montaña\",\"direccion\":\"Calle 10 20-30\",\"ciudad\":\"Medellín, Antioquia\"}} (o sólo el nombre de la organización)",
            "the letter needs «destinatario»: {{\"tratamiento\":\"Señor\",\"nombre\":\"Carlos Pérez Gómez\",\"cargo\":\"Gerente\",\"organizacion\":\"Hotel Montaña\",\"direccion\":\"Calle 10 20-30\",\"ciudad\":\"Medellín, Antioquia\"}} (or just the organization's name)"
        ));
    }
    let parrafos: Vec<String> = ["texto", "parrafos", "párrafos", "cuerpo", "contenido"] // juez-idioma: datos (claves que acepta la ficha)
        .iter()
        .find_map(|k| match &c[*k] {
            Value::Array(a) => Some(a.iter().filter_map(txt).collect::<Vec<_>>()),
            Value::String(s) => Some(s.split("\n\n").map(|p| p.trim().replace('\n', " ")).filter(|p| !p.is_empty()).collect()),
            _ => None,
        })
        .filter(|v| !v.is_empty())
        .ok_or_else(|| gotv_engine::anyhow_tr!("la carta necesita «texto»: [\"primer párrafo\", \"segundo párrafo\", …]", "the letter needs «texto»: [\"first paragraph\", \"second paragraph\", …]"))?;
    // saludo (GTC Anexo A): según el tratamiento; a una organización, «Respetados señores:»
    let saludo = primero(c, &["saludo"]).unwrap_or_else(|| match tratamiento.as_deref().map(str::to_lowercase) {
        Some(tr) if tr == "señores" => "Respetados señores:".into(), // juez-idioma: datos (texto de la carta GTC 185, norma colombiana en español)
        Some(tr) => {
            let fem = tr.ends_with('a');
            format!("{} {tr}:", if fem { "Respetada" } else { "Respetado" })
        }
        None => "Cordial saludo:".into(),
    });
    let r = &c["remitente"];
    let mut remitente = vec![];
    if r.is_object() {
        if let Some(n) = primero(r, &["nombre", "name"]) {
            remitente.push(n.to_uppercase());
        }
        for k in [&["cargo"][..], &["organizacion", "organización", "empresa"]] { // juez-idioma: datos (claves que acepta la ficha)
            if let Some(v) = primero(r, k) {
                remitente.push(v);
            }
        }
    } else if let Some(n) = txt(r) {
        remitente.push(n.to_uppercase());
    }
    if remitente.is_empty() {
        return Err(gotv_engine::anyhow_tr!(
            "la carta necesita «remitente»: {{\"nombre\":\"Ana Gómez Ruiz\",\"cargo\":\"Gerente\"}} (quien firma)",
            "the letter needs «remitente»: {{\"nombre\":\"Ana Gómez Ruiz\",\"cargo\":\"Gerente\"}} (who signs)"
        ));
    }
    let mut especiales = vec![];
    if let Some(a) = primero(c, &["anexos", "anexo"]) {
        especiales.push(format!("Anexo: {}", a.trim_start_matches("Anexo:").trim_start_matches("Anexos:").trim()));
    }
    match &c["copias"] {
        Value::Array(a) => especiales.extend(a.iter().filter_map(txt).map(|x| format!("Copia: {x}"))),
        v => especiales.extend(txt(v).map(|x| format!("Copia: {x}"))),
    }
    if let Some(x) = primero(c, &["transcriptor"]) {
        especiales.push(format!("Transcriptor: {x}"));
    }
    if let Some(x) = primero(c, &["redactor"]) {
        especiales.push(format!("Redactor: {x}"));
    }
    Ok(Partes {
        codigo: primero(c, &["codigo", "código", "radicado"]), // juez-idioma: datos (claves que acepta la ficha)
        lugar_fecha,
        destinatario,
        asunto: primero(c, &["asunto", "referencia"]).map(|a| a.trim_start_matches("Asunto:").trim().to_string()),
        saludo,
        parrafos,
        despedida: primero(c, &["despedida"]).unwrap_or_else(|| "Atentamente,".into()),
        remitente,
        especiales,
        estilo,
        nombre_dest,
    })
}

/// Las líneas de la carta con una holgura dada (`ancho` = ancho del texto en twips).
fn maquetar(p: &Partes, h: &Holgura, ancho: i64) -> Vec<Linea> {
    let mitad = ancho / 2;
    // estilo bloque: código, fecha, firma y cargo desde el centro hacia la derecha (4.3.2)
    let al_centro = |s: String| match p.estilo {
        "bloque_extremo" => t(s),
        _ => Linea::Texto { t: s, jc: "left", ind: mitad, primera: 0, menor: false, subrayar: false },
    };
    let vacias = |n: u32, v: &mut Vec<Linea>| (0..n).for_each(|_| v.push(Linea::Vacia));
    let mut v = vec![];
    if let Some(c) = &p.codigo {
        v.push(al_centro(c.clone()));
        vacias(1, &mut v);
    }
    v.push(al_centro(p.lugar_fecha.clone()));
    vacias(h.fecha_dest, &mut v);
    v.extend(p.destinatario.iter().cloned());
    vacias(h.dest_asunto, &mut v);
    let mut antes_saludo = h.dest_asunto;
    if let Some(a) = &p.asunto {
        v.push(t(format!("Asunto: {a}")));
        vacias(h.asunto_saludo, &mut v);
        antes_saludo = h.asunto_saludo;
    }
    let _ = antes_saludo;
    v.push(t(p.saludo.clone()));
    vacias(h.saludo_texto, &mut v);
    let sangria = if p.estilo == "semibloque" { (5 * CM) / 4 } else { 0 };
    for (i, par) in p.parrafos.iter().enumerate() {
        if i > 0 {
            vacias(1, &mut v); // 5.4.6: una interlínea libre entre párrafos
        }
        v.push(Linea::Texto { t: par.clone(), jc: "both", ind: 0, primera: sangria, menor: false, subrayar: false });
    }
    vacias(h.texto_desp, &mut v);
    v.push(t(p.despedida.clone()));
    vacias(h.desp_firma, &mut v); // el espacio de la firma (5.4.8)
    for r in &p.remitente {
        v.push(al_centro(r.clone()));
    }
    if !p.especiales.is_empty() {
        vacias(2, &mut v);
        for e in &p.especiales {
            v.push(Linea::Texto { t: e.clone(), jc: "left", ind: 0, primera: 0, menor: true, subrayar: false });
        }
    }
    v
}

/// Alto estimado en puntos (interlineado sencillo de Arial ≈ 1,15 del tamaño).
fn alto_pt(lineas: &[Linea], medio_pt: u32, ancho: i64) -> f64 {
    let pt = f64::from(medio_pt) / 2.0;
    lineas
        .iter()
        .map(|l| match l {
            Linea::Vacia => pt * 1.15,
            Linea::Texto { t, ind, primera, menor, .. } => {
                let p = if *menor { pt - 1.5 } else { pt };
                let w = (ancho - ind - primera) * 635; // twips → EMU
                f64::from(crate::ppt::lineas(t, p, false, w)) * p * 1.15
            }
        })
        .sum()
}

fn parrafo(l: &Linea, medio_pt: u32, fuente: &str) -> String {
    let f = escapar(fuente, true);
    let base = |sz: u32, extra: &str| format!("<w:rPr><w:rFonts w:ascii=\"{f}\" w:hAnsi=\"{f}\" w:cs=\"{f}\"/>{extra}<w:sz w:val=\"{sz}\"/><w:szCs w:val=\"{sz}\"/></w:rPr>");
    let espaciado = "<w:spacing w:before=\"0\" w:after=\"0\" w:line=\"240\" w:lineRule=\"auto\"/>";
    match l {
        Linea::Vacia => format!("<w:p><w:pPr>{espaciado}{}</w:pPr></w:p>", base(medio_pt, "")),
        Linea::Texto { t, jc, ind, primera, menor, subrayar } => {
            let sz = if *menor { medio_pt.saturating_sub(3) } else { medio_pt };
            let sangria = if *ind > 0 || *primera > 0 { format!("<w:ind w:left=\"{ind}\" w:firstLine=\"{primera}\"/>") } else { String::new() };
            let u = if *subrayar { "<w:u w:val=\"single\"/>" } else { "" };
            format!(
                "<w:p><w:pPr><w:keepLines/>{espaciado}{sangria}<w:jc w:val=\"{jc}\"/>{}</w:pPr><w:r>{}<w:t xml:space=\"preserve\">{}</w:t></w:r></w:p>",
                base(sz, ""),
                base(sz, u),
                escapar(t, false)
            )
        }
    }
}

/// Encabezado: el membrete (primera página) o «destinatario · Página N» (páginas siguientes, 4.5.1).
fn cabecera(lineas: &[(String, u32, &str)], fuente: &str, con_pagina: bool) -> String {
    let f = escapar(fuente, true);
    let mut ps: String = lineas
        .iter()
        .map(|(s, sz, jc)| format!("<w:p><w:pPr><w:spacing w:before=\"0\" w:after=\"0\"/><w:jc w:val=\"{jc}\"/></w:pPr><w:r><w:rPr><w:rFonts w:ascii=\"{f}\" w:hAnsi=\"{f}\"/><w:sz w:val=\"{sz}\"/></w:rPr><w:t xml:space=\"preserve\">{}</w:t></w:r></w:p>", escapar(s, false)))
        .collect();
    if con_pagina {
        ps.push_str(&format!("<w:p><w:pPr><w:spacing w:before=\"0\" w:after=\"0\"/></w:pPr><w:r><w:rPr><w:rFonts w:ascii=\"{f}\" w:hAnsi=\"{f}\"/><w:sz w:val=\"20\"/></w:rPr><w:t xml:space=\"preserve\">Página </w:t></w:r><w:fldSimple w:instr=\" PAGE \"><w:r><w:rPr><w:rFonts w:ascii=\"{f}\" w:hAnsi=\"{f}\"/><w:sz w:val=\"20\"/></w:rPr><w:t>2</w:t></w:r></w:fldSimple></w:p>")); // juez-idioma: datos (texto de la carta GTC 185, norma colombiana en español)
    }
    format!("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<w:hdr xmlns:w=\"{W}\" xmlns:r=\"{R}\">{ps}</w:hdr>")
}

/// Resultado de maquetar: cuántas hojas y con qué holgura quedó (para el informe y las pruebas).
#[derive(Debug)]
pub struct Maqueta {
    pub hojas: u32,
    pub intento: usize,
    pub pt: f64,
}

/// Crea la carta. Devuelve el número de párrafos de texto (como `word::crear`).
pub fn crear(path: &Path, c: &Value) -> Result<usize> {
    let (n, _) = crear_con_maqueta(path, c)?;
    Ok(n)
}

pub fn crear_con_maqueta(path: &Path, c: &Value) -> Result<(usize, Maqueta)> {
    let p = resolver(c)?;
    let (pw, ph) = match c["pagina"].as_str().unwrap_or("carta").to_lowercase().as_str() {
        "a4" => (11906, 16838),
        "oficio" | "legal" => (12240, 20160),
        _ => (12240, 15840),
    };
    let fuente = c["fuente"].as_str().unwrap_or("Arial");
    let ancho = pw - MARGEN_IZQ - MARGEN_DER;
    // el primer intento que cabe en una hoja; si ninguno, dos hojas con el más compacto a 11 pt
    let cabe = |h: &Holgura| alto_pt(&maquetar(&p, h, ancho), h.medio_pt, ancho) <= (ph - h.margen_sup - MARGEN_INF) as f64 / 20.0;
    let (intento, h, hojas) = match INTENTOS.iter().position(cabe) {
        Some(i) => (i, INTENTOS[i], 1),
        None => (2, INTENTOS[2], 2),
    };
    let lineas = maquetar(&p, &h, ancho);
    let cuerpo: String = lineas.iter().map(|l| parrafo(l, h.medio_pt, fuente)).collect();
    // membrete (primera hoja) y encabezado de páginas siguientes
    let membrete: Vec<(String, u32, &str)> = match &c["membrete"] {
        Value::String(s) if !s.trim().is_empty() => vec![(s.trim().to_string(), 28, "center")],
        Value::Object(_) => {
            let m = &c["membrete"];
            let mut v = vec![];
            if let Some(n) = primero(m, &["nombre", "organizacion", "organización", "empresa"]) { // juez-idioma: datos (claves que acepta la ficha)
                v.push((n, 28, "center"));
            }
            for k in ["linea", "línea", "nit", "direccion", "dirección", "telefono", "teléfono", "correo", "web"] { // juez-idioma: datos (claves que acepta la ficha)
                if let Some(x) = txt(&m[k]) {
                    v.push((x, 16, "center"));
                }
            }
            v
        }
        _ => vec![],
    };
    let mut cabeceras = vec![];
    let mut refs = String::new();
    if !membrete.is_empty() {
        cabeceras.push(cabecera(&membrete, fuente, false));
        refs.push_str(&format!("<w:headerReference w:type=\"first\" r:id=\"rIdC{}\"/>", cabeceras.len()));
    }
    if hojas > 1 {
        cabeceras.push(cabecera(&[(p.nombre_dest.clone(), 20, "left")], fuente, true));
        refs.push_str(&format!("<w:headerReference w:type=\"default\" r:id=\"rIdC{}\"/>", cabeceras.len()));
    }
    let primera_distinta = if refs.is_empty() { "" } else { "<w:titlePg/>" };
    let sect = format!(
        "<w:sectPr>{refs}<w:pgSz w:w=\"{pw}\" w:h=\"{ph}\"/><w:pgMar w:top=\"{}\" w:right=\"{MARGEN_DER}\" w:bottom=\"{MARGEN_INF}\" w:left=\"{MARGEN_IZQ}\" w:header=\"{}\" w:footer=\"708\" w:gutter=\"0\"/><w:cols w:space=\"708\"/>{primera_distinta}</w:sectPr>",
        h.margen_sup,
        CM * 5 / 4
    );
    let titulo = p.asunto.clone().unwrap_or_else(|| "Carta".into());
    escribir_docx(path, &cuerpo, &sect, fuente, &titulo, 0, &cabeceras)?;
    Ok((p.parrafos.len(), Maqueta { hojas, intento, pt: f64::from(h.medio_pt) / 2.0 }))
}

#[cfg(test)]
mod pruebas {
    use super::*;
    use serde_json::json;

    fn ficha(parrafos: usize) -> Value {
        let p = "Nos complace presentarles nuestra cafetería, un espacio dedicado al café de origen con más de diez años de trayectoria en la ciudad y un equipo comprometido con la calidad.";
        json!({
            "ciudad": "Medellín", "fecha": "11 de noviembre de 2026",
            "membrete": {"nombre": "GRANO ALTO S.A.S.", "linea": "NIT 900.123.456-7 · Carrera 43A 7-50 · Medellín"},
            "destinatario": {"tratamiento": "Señora", "nombre": "Laura Restrepo Mejía", "cargo": "Gerente General", "organizacion": "Hotel Montaña", "direccion": "Calle 10 32-15", "ciudad": "Medellín, Antioquia"},
            "asunto": "Propuesta de suministro",
            "texto": vec![p; parrafos],
            "remitente": {"nombre": "Andrés Gómez Ruiz", "cargo": "Gerente Comercial"},
            "anexos": "1 (portafolio, cuatro folios)"
        })
    }

    #[test]
    fn la_carta_lleva_el_formato_de_la_gtc_185() {
        let d = crate::CarpetaPrueba::nueva("gotv-carta");
        let x = d.join("carta.docx");
        let (_, m) = crear_con_maqueta(&x, &ficha(3)).unwrap();
        assert_eq!(m.hojas, 1, "{m:?}");
        assert_eq!(m.intento, 0, "una carta corta va con la holgura amplia: {m:?}");
        let l = crate::word::leer(&x).unwrap();
        // orden: lugar y fecha → destinatario (NOMBRE en mayúscula fija) → asunto → saludo → texto → despedida → REMITENTE
        let pos = |s: &str| l.find(s).unwrap_or_else(|| panic!("falta «{s}» en {l}"));
        assert!(pos("Medellín, 11 de noviembre de 2026") < pos("Señora"));
        assert!(pos("LAURA RESTREPO MEJÍA") < pos("Gerente General") && pos("Gerente General") < pos("Hotel Montaña"));
        assert!(pos("Hotel Montaña") < pos("Asunto: Propuesta de suministro") && pos("Asunto:") < pos("Respetada señora:"));
        assert!(pos("Respetada señora:") < pos("Nos complace") && pos("Atentamente,") < pos("ANDRÉS GÓMEZ RUIZ"));
        assert!(pos("ANDRÉS GÓMEZ RUIZ") < pos("Gerente Comercial") && pos("Gerente Comercial") < pos("Anexo: 1"));
        // sin negrilla ni saltos de página; membrete en encabezado de la primera hoja; valida
        let pq = crate::ooxml::Paquete::abrir(&x).unwrap();
        let doc = pq.texto("word/document.xml").unwrap();
        assert!(!doc.contains("<w:b/>") && !doc.contains("w:type=\"page\""));
        assert!(pq.texto("word/header1.xml").unwrap().contains("GRANO ALTO S.A.S.") && doc.contains("<w:titlePg/>"));
        assert!(doc.contains("w:left=\"1701\"") && doc.contains("w:right=\"1134\""), "márgenes 3 y 2 cm");
        let (errores, avisos) = crate::word::revisar(&x).unwrap();
        assert!(errores.is_empty() && avisos.is_empty(), "{errores:?} {avisos:?}");
    }

    #[test]
    fn se_ajusta_a_una_hoja_y_si_no_cabe_va_en_dos_con_encabezado() {
        let d = crate::CarpetaPrueba::nueva("gotv-carta-larga");
        // más texto → más compacta, pero en UNA hoja
        let (_, m) = crear_con_maqueta(&d.join("media.docx"), &ficha(7)).unwrap();
        assert!(m.hojas == 1 && m.intento > 0, "{m:?}");
        // mucho texto → dos hojas, con «destinatario · Página N» en las siguientes (4.5.1)
        let (_, m) = crear_con_maqueta(&d.join("larga.docx"), &ficha(16)).unwrap();
        assert_eq!(m.hojas, 2, "{m:?}");
        assert_eq!(m.pt, 11.0, "a dos hojas no se encoge la letra");
        let pq = crate::ooxml::Paquete::abrir(&d.join("larga.docx")).unwrap();
        let h2 = pq.texto("word/header2.xml").unwrap();
        assert!(h2.contains("LAURA RESTREPO MEJÍA") && h2.contains("PAGE"), "{h2}");
    }

    /// La forma real del 9B (banco of-carta): la ficha como TEXTO con un `}` de más tras `texto`,
    /// que dejaba `remitente` y `anexos` fuera de `carta`. Se repara y la carta sale entera.
    #[test]
    fn la_ficha_con_un_cierre_de_mas_se_repara() {
        let d = crate::CarpetaPrueba::nueva("gotv-carta-cierre");
        let x = d.join("carta.docx");
        let del_9b = json!("{\"carta\": {\"ciudad\": \"Medellín\", \"destinatario\": \"Hotel Montaña\", \"asunto\": \"Presentación\", \"texto\": [\"Somos Grano Alto.\", \"Agradecemos su tiempo.\"]}, \"remitente\": {\"nombre\": \"Cafetería Grano Alto\", \"cargo\": \"Gerente General\"}, \"anexos\": \"Portafolio\"}}");
        crate::word::crear(&x, &del_9b).unwrap();
        let l = crate::word::leer(&x).unwrap();
        assert!(l.contains("CAFETERÍA GRANO ALTO") && l.contains("Gerente General") && l.contains("Anexo: Portafolio") && l.contains("Agradecemos su tiempo."), "{l}");
        // un JSON roto de verdad sigue diciendo dónde se rompe
        assert!(crate::word::crear(&d.join("y.docx"), &json!("{\"carta\": {\"texto\": [\"a\" \"b\"]}}")).is_err());
    }

    /// El juez de verdad: Word dibuja las cartas y sus hojas coinciden con lo estimado.
    /// `cargo test -p gotv-core --release -- --ignored --nocapture carta_con_word`
    #[tokio::test]
    #[ignore]
    async fn carta_con_word() {
        let d = std::env::var("GOTV_CARTAS").map(std::path::PathBuf::from).unwrap_or_else(|_| std::env::temp_dir().join("gotv-cartas"));
        std::fs::create_dir_all(&d).unwrap();
        for (nombre, n) in [("corta", 3), ("media", 7), ("larga", 16)] {
            let x = d.join(format!("{nombre}.docx"));
            let _ = std::fs::remove_file(&x);
            let (_, m) = crear_con_maqueta(&x, &ficha(n)).unwrap();
            let pdf = d.join(format!("{nombre}.pdf"));
            crate::apps::exportar_pdf(&x, &pdf, 120).await.unwrap();
            let info = crate::pdf::info(&pdf).unwrap();
            println!("{nombre}: estimado {} hoja(s) (intento {}, {} pt) · Word: {}", m.hojas, m.intento, m.pt, info.lines().next().unwrap_or(""));
            assert!(info.contains(&format!("· {} páginas", m.hojas)), "{nombre}: {info}");
        }
    }

    #[test]
    fn estilos_y_formas_sueltas() {
        let d = crate::CarpetaPrueba::nueva("gotv-carta-formas");
        // destinatario como texto (carta general a una organización) y remitente como texto
        crear(&d.join("g.docx"), &json!({"destinatario": "Hotel Montaña", "texto": "Primer párrafo.\n\nSegundo párrafo.", "remitente": "Ana Gómez", "estilo": "bloque"})).unwrap();
        let l = crate::word::leer(&d.join("g.docx")).unwrap();
        assert!(l.contains("Señores") && l.contains("Respetados señores:") && l.contains("ANA GÓMEZ") && l.contains("Segundo párrafo."), "{l}");
        let doc = crate::ooxml::Paquete::abrir(&d.join("g.docx")).unwrap().texto("word/document.xml").unwrap();
        assert!(doc.contains("<w:ind w:left="), "estilo bloque: fecha y firma desde el centro");
        // la opción del Portador: subrayar el nombre del destinatario
        let mut f = ficha(2);
        f["subrayar_destinatario"] = json!(true);
        crear(&d.join("s.docx"), &f).unwrap();
        let doc = crate::ooxml::Paquete::abrir(&d.join("s.docx")).unwrap().texto("word/document.xml").unwrap();
        assert!(doc.contains("<w:u w:val=\"single\"/>"));
        // faltan partes → dice qué y cómo
        let e = crear(&d.join("x.docx"), &json!({"texto": ["Hola"]})).unwrap_err().to_string();
        assert!(e.contains("destinatario"), "{e}");
        // la fecha de hoy, en español y sin punto en el año
        let hoy = hoy_largo();
        assert!(MESES.iter().any(|m| hoy.contains(&format!(" de {m} de "))), "{hoy}");
    }
}
