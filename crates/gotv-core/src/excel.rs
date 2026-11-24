//! O2 · Excel como ARCHIVO (método de Claude Code, skill `xlsx`, Apache-2.0; ver
//! `ESTUDIO-DOCUMENTOS.md` §2.4), en Rust:
//!   - leer = sus «dos cargas» de openpyxl en una: fórmula Y valor guardado de cada celda (calamine);
//!   - crear = openpyxl→ rust_xlsxwriter: fuente Arial, fórmulas EN INGLÉS (las de 2010+ con su
//!     prefijo `_xlfn.` las pone la crate; las de matriz dinámica se escriben como tales);
//!   - editar = umya-spreadsheet: sólo las celdas pedidas, el resto (formato, otras fórmulas) intacto,
//!     y un respaldo antes de escribir;
//!   - recalcular = su `recalc.py`, pero con el EXCEL del usuario (LibreOffice aquí es un resto roto):
//!     una librería escribe las fórmulas SIN valor guardado;
//!   - revisar = su barrido de errores: `#VALUE! #DIV/0! #REF! #NAME? #NULL! #NUM! #N/A`, fórmulas sin
//!     valor y funciones en español. «Recalcular sin errores» prueba que EVALÚAN, no que estén bien.

use anyhow::{anyhow, Result};
use gotv_engine::{anyhow_tr, bail_tr};
use calamine::{open_workbook_auto, Data, Reader};
use serde_json::Value;
use std::path::{Path, PathBuf};

/// Funciones de Excel en español → inglés: el archivo guarda los nombres en INGLÉS (medido
/// 2026-11-11: «=SUMA(B2:B4)» quedó en `#¿NOMBRE?`).
const FUNCIONES: &[(&str, &str)] = &[
    ("SUMA", "SUM"), ("PROMEDIO", "AVERAGE"), ("CONTAR", "COUNT"), ("CONTARA", "COUNTA"), ("SI", "IF"),
    ("SI.ERROR", "IFERROR"), ("REDONDEAR", "ROUND"), ("BUSCARV", "VLOOKUP"), ("BUSCARH", "HLOOKUP"),
    ("BUSCARX", "XLOOKUP"), ("CONCATENAR", "CONCATENATE"), ("HOY", "TODAY"), ("AHORA", "NOW"),
    ("SUMAR.SI", "SUMIF"), ("SUMAR.SI.CONJUNTO", "SUMIFS"), ("CONTAR.SI", "COUNTIF"),
    ("CONTAR.SI.CONJUNTO", "COUNTIFS"), ("PROMEDIO.SI", "AVERAGEIF"), ("Y", "AND"), ("O", "OR"),
    ("NO", "NOT"), ("ENTERO", "INT"), ("TEXTO", "TEXT"), ("IZQUIERDA", "LEFT"), ("DERECHA", "RIGHT"),
    ("EXTRAE", "MID"), ("LARGO", "LEN"), ("MAYUSC", "UPPER"), ("MINUSC", "LOWER"), ("ESPACIOS", "TRIM"),
    ("SUMAPRODUCTO", "SUMPRODUCT"), ("INDICE", "INDEX"), ("COINCIDIR", "MATCH"), ("FECHA", "DATE"),
    ("AÑO", "YEAR"), ("MES", "MONTH"), ("DIA", "DAY"), ("DIAS", "DAYS"), ("REDONDEAR.MAS", "ROUNDUP"), // juez-idioma: datos (nombres de funciones de Excel en español)
    ("REDONDEAR.MENOS", "ROUNDDOWN"), ("RESIDUO", "MOD"), ("POTENCIA", "POWER"), ("RAIZ", "SQRT"),
    ("ABS", "ABS"), ("MAX", "MAX"), ("MIN", "MIN"), ("FILTRAR", "FILTER"), ("ORDENAR", "SORT"),
    ("UNICOS", "UNIQUE"), ("SECUENCIA", "SEQUENCE"), ("UNIRCADENAS", "TEXTJOIN"), ("CONCAT", "CONCAT"),
];

/// Las que DERRAMAN (matriz dinámica): se escriben como fórmula dinámica, o sólo la celda de arriba
/// a la izquierda recibe valor (Claude Code las prohíbe por eso; con el Excel real funcionan así).
const DERRAMAN: &[&str] = &["FILTER", "SORT", "SORTBY", "UNIQUE", "SEQUENCE", "RANDARRAY", "XLOOKUP", "XMATCH"];

/// Una fórmula (con o sin `=`) con sus funciones en español → inglés y `;` → `,` fuera de las
/// comillas. Devuelve la fórmula y lo que cambió.
pub fn formula_en_ingles(f: &str) -> (String, Vec<String>) {
    let mut cambios = vec![];
    let mut nueva = String::new();
    let mut palabra = String::new();
    let mut en_texto = false;
    let mut cambio_sep = false;
    for ch in f.chars() {
        if ch == '"' {
            en_texto = !en_texto;
        }
        if !en_texto && (ch.is_alphanumeric() || ch == '.' || ch == '_') {
            palabra.push(ch);
            continue;
        }
        if ch == '(' && !en_texto {
            if let Some((es, en)) = FUNCIONES.iter().find(|(es, _)| es.eq_ignore_ascii_case(&palabra) || es.eq_ignore_ascii_case(&quitar_tildes(&palabra))) {
                if !es.eq_ignore_ascii_case(en) {
                    cambios.push(format!("{}→{en}", palabra.to_uppercase()));
                }
                palabra = en.to_string();
            }
        }
        nueva.push_str(&palabra);
        palabra.clear();
        if ch == ';' && !en_texto {
            nueva.push(',');
            cambio_sep = true;
        } else {
            nueva.push(ch);
        }
    }
    nueva.push_str(&palabra);
    if cambio_sep {
        cambios.push(";→,".into());
    }
    (nueva, cambios)
}

fn quitar_tildes(s: &str) -> String {
    s.chars().map(|c| match c { 'á' | 'Á' => 'A', 'é' | 'É' => 'E', 'í' | 'Í' => 'I', 'ó' | 'Ó' => 'O', 'ú' | 'Ú' => 'U', 'ñ' | 'Ñ' => 'N', o => o }).collect()
}

/// «B5» → (fila 0-based, columna 0-based).
pub fn celda(r: &str) -> Result<(u32, u16)> {
    let r = r.trim().replace('$', "").to_uppercase();
    let letras: String = r.chars().take_while(|c| c.is_ascii_alphabetic()).collect();
    let numero: String = r[letras.len()..].to_string();
    if letras.is_empty() || letras.len() > 3 || numero.is_empty() || !numero.chars().all(|c| c.is_ascii_digit()) {
        bail_tr!("celda inválida «{r}» (se espera p. ej. «B5»)", "invalid cell «{r}» (e.g. «B5» expected)");
    }
    let col = letras.bytes().fold(0u32, |a, b| a * 26 + (b - b'A' + 1) as u32) - 1;
    let fila: u32 = numero.parse::<u32>()?.checked_sub(1).ok_or_else(|| anyhow_tr!("la fila empieza en 1: «{r}»", "rows start at 1: «{r}»"))?;
    Ok((fila, col as u16))
}

pub fn nombre_celda(fila: u32, col: u32) -> String {
    let mut c = col + 1;
    let mut s = String::new();
    while c > 0 {
        let r = (c - 1) % 26;
        s.insert(0, (b'A' + r as u8) as char);
        c = (c - 1) / 26;
    }
    format!("{s}{}", fila + 1)
}

/// Un rango «A1:C3» (o una celda) → todas sus celdas.
fn rango(r: &str) -> Result<Vec<(u32, u16)>> {
    let (a, b) = r.split_once(':').unwrap_or((r, r));
    let ((f0, c0), (f1, c1)) = (celda(a)?, celda(b)?);
    let mut v = vec![];
    for f in f0.min(f1)..=f0.max(f1) {
        for c in c0.min(c1)..=c0.max(c1) {
            v.push((f, c));
        }
    }
    Ok(v)
}

fn mostrar(d: &Data) -> String {
    match d {
        Data::Empty => String::new(),
        Data::String(s) if s.is_empty() => String::new(),
        Data::String(s) => format!("«{s}»"),
        Data::Float(f) => {
            if f.fract() == 0.0 && f.abs() < 1e15 { format!("{}", *f as i64) } else { format!("{f}") }
        }
        Data::Int(i) => i.to_string(),
        Data::Bool(b) => b.to_string(),
        Data::Error(e) => format!("{e}"),
        Data::DateTime(d) => format!("{d}"),
        Data::DateTimeIso(s) | Data::DurationIso(s) => s.clone(),
    }
}

/// LEER: por hoja, cada celda con su valor guardado y, si la tiene, su fórmula (las «dos cargas»).
pub fn leer(path: &Path, hoja: Option<&str>, max_celdas: usize) -> Result<String> {
    let mut wb = open_workbook_auto(path).map_err(|e| anyhow!("no pude abrir {}: {e}", path.display()))?;
    let nombres = wb.sheet_names();
    let mut out = format!("{} · hojas: {}\n", path.file_name().unwrap_or_default().to_string_lossy(), nombres.join(", "));
    let elegidas: Vec<String> = match hoja {
        Some(h) => vec![nombres.iter().find(|n| n.eq_ignore_ascii_case(h)).cloned().ok_or_else(|| anyhow!("no hay hoja «{h}» (hay: {})", nombres.join(", ")))?],
        None => nombres.clone(),
    };
    let mut n = 0;
    for h in elegidas {
        let valores = wb.worksheet_range(&h).map_err(|e| anyhow!("hoja «{h}»: {e}"))?;
        let formulas = wb.worksheet_formula(&h).unwrap_or_default();
        let (f0, c0) = valores.start().unwrap_or((0, 0));
        out.push_str(&format!("## {h} ({} filas × {} columnas usadas)\n", valores.height(), valores.width()));
        let mut todas: std::collections::BTreeMap<(u32, u32), (String, Option<String>)> = Default::default();
        for (i, fila) in valores.rows().enumerate() {
            for (j, v) in fila.iter().enumerate() {
                if !matches!(v, Data::Empty) {
                    todas.entry((f0 + i as u32, c0 + j as u32)).or_default().0 = mostrar(v);
                }
            }
        }
        if let Some((g0, d0)) = formulas.start() {
            for (i, fila) in formulas.rows().enumerate() {
                for (j, f) in fila.iter().enumerate() {
                    if !f.is_empty() {
                        todas.entry((g0 + i as u32, d0 + j as u32)).or_default().1 = Some(f.clone());
                    }
                }
            }
        }
        for ((f, c), (v, formula)) in todas {
            n += 1;
            if n > max_celdas {
                continue;
            }
            let nom = nombre_celda(f, c);
            match formula {
                Some(fm) if v.is_empty() => out.push_str(&crate::tr!("{nom} = ={fm} → (sin valor guardado: recalcular)\n", "{nom} = ={fm} → (no stored value: recalculate)\n")),
                Some(fm) => out.push_str(&format!("{nom} = ={fm} → {v}\n")),
                None => out.push_str(&format!("{nom} = {v}\n")),
            }
        }
    }
    if n > max_celdas {
        out.push_str(&crate::tr!("… {} celdas más (pide una hoja concreta)\n", "… {} more cells (ask for a specific sheet)\n", n - max_celdas));
    }
    Ok(out)
}

/// REVISAR: errores de fórmula, fórmulas sin valor guardado y funciones en español.
pub struct Revision {
    pub formulas: usize,
    pub errores: Vec<(String, String)>,
    pub sin_valor: Vec<String>,
    pub en_espanol: Vec<String>,
}

impl Revision {
    pub fn limpia(&self) -> bool {
        self.errores.is_empty() && self.sin_valor.is_empty() && self.en_espanol.is_empty()
    }
    pub fn informe(&self) -> String {
        if self.limpia() {
            return crate::tr!("✓ revisión: {} fórmula(s), 0 errores", "✓ review: {} formula(s), 0 errors", self.formulas);
        }
        let mut s = crate::tr!("✗ revisión: {} fórmula(s)", "✗ review: {} formula(s)", self.formulas);
        if !self.errores.is_empty() {
            let l: Vec<String> = self.errores.iter().take(20).map(|(c, e)| format!("{c} {e}")).collect();
            s.push_str(&crate::tr!(" · {} ERROR(ES): {}", " · {} ERROR(S): {}", self.errores.len(), l.join(", ")));
        }
        if !self.sin_valor.is_empty() {
            s.push_str(&crate::tr!(" · {} sin valor guardado (recalcula): {}", " · {} without a stored value (recalculate): {}", self.sin_valor.len(), self.sin_valor.iter().take(10).cloned().collect::<Vec<_>>().join(", ")));
        }
        if !self.en_espanol.is_empty() {
            s.push_str(&crate::tr!(" · funciones en español (el archivo las quiere en inglés): {}", " · functions in Spanish (the file wants them in English): {}", self.en_espanol.join(", ")));
        }
        s.push_str(crate::i18n::t(". NO lo entregues así: corrige lo nombrado y vuelve a revisar.", ". Do NOT deliver it like this: fix what is named and review again."));
        s
    }
}

pub fn revisar(path: &Path) -> Result<Revision> {
    let mut wb = open_workbook_auto(path).map_err(|e| anyhow!("no pude abrir {}: {e}", path.display()))?;
    let mut r = Revision { formulas: 0, errores: vec![], sin_valor: vec![], en_espanol: vec![] };
    for h in wb.sheet_names() {
        let valores = wb.worksheet_range(&h).unwrap_or_default();
        let formulas = wb.worksheet_formula(&h).unwrap_or_default();
        let (f0, c0) = valores.start().unwrap_or((0, 0));
        for (fila, col, v) in valores.used_cells() {
            if let Data::Error(e) = v {
                r.errores.push((format!("{h}!{}", nombre_celda(f0 + fila as u32, c0 + col as u32)), format!("{e}")));
            }
        }
        if let Some((g0, d0)) = formulas.start() {
            for (fila, col, f) in formulas.used_cells() {
                if f.is_empty() {
                    continue;
                }
                r.formulas += 1;
                let (fa, ca) = (g0 + fila as u32, d0 + col as u32);
                let nom = format!("{h}!{}", nombre_celda(fa, ca));
                if matches!(valores.get_value((fa, ca)), None | Some(Data::Empty)) || matches!(valores.get_value((fa, ca)), Some(Data::String(s)) if s.is_empty()) {
                    r.sin_valor.push(nom.clone());
                }
                let (_, cambios) = formula_en_ingles(f);
                if cambios.iter().any(|c| c.contains('→') && !c.starts_with(';')) {
                    r.en_espanol.push(format!("{nom} ({})", cambios.join(", ")));
                }
            }
        }
    }
    Ok(r)
}

/// El valor de una celda pedido por el modelo: número, booleano, fórmula («=…») o texto.
enum Valor {
    Numero(f64),
    Bool(bool),
    Formula(String),
    Texto(String),
    Vacio,
}

fn valor(v: &Value) -> Valor {
    match v {
        Value::Number(n) => Valor::Numero(n.as_f64().unwrap_or(0.0)),
        Value::Bool(b) => Valor::Bool(*b),
        Value::Null => Valor::Vacio,
        Value::String(s) if s.trim_start().starts_with('=') => Valor::Formula(s.trim().to_string()),
        Value::String(s) => Valor::Texto(s.clone()),
        otro => Valor::Texto(otro.to_string()),
    }
}

/// La fórmula con su resultado guardado VACÍO: rust_xlsxwriter guarda 0 por omisión, y leído sin
/// recalcular el total «valía 0» (medido 2026-11-11) — peor que vacío, porque engaña. El libro
/// lleva `fullCalcOnLoad`: Excel la calcula al abrirlo.
fn sin_valor(f: &str) -> rust_xlsxwriter::Formula {
    rust_xlsxwriter::Formula::new(f).set_result("")
}

fn derrama(f: &str) -> bool {
    let u = f.to_uppercase();
    DERRAMAN.iter().any(|d| u.contains(&format!("{d}(")))
}

/// Celdas pedidas: `{"A1": "Producto", "B2": 8000, "B5": "=SUM(B2:B4)"}` y/o `filas` desde A1
/// (o desde `desde`): `[["Producto","Precio"],["Latte",8000]]`.
fn celdas_pedidas(h: &Value) -> Result<Vec<(u32, u16, Value)>> {
    let mut v = vec![];
    if let Some(filas) = h["filas"].as_array() {
        let (f0, c0) = match h["desde"].as_str() {
            Some(d) => celda(d)?,
            None => (0, 0),
        };
        for (i, fila) in filas.iter().enumerate() {
            for (j, x) in fila.as_array().cloned().unwrap_or_default().into_iter().enumerate() {
                v.push((f0 + i as u32, c0 + j as u16, x));
            }
        }
    }
    if let Some(m) = h["celdas"].as_object() {
        for (k, x) in m {
            let (f, c) = celda(k)?;
            v.push((f, c, x.clone()));
        }
    }
    Ok(v)
}

/// CREAR un libro nuevo. `hojas`: `[{nombre, filas?, desde?, celdas?, negrita?: ["A1:B1"],
/// formato?: {"B2:B5": "#,##0"}, anchos?: {"A": 18}, fijar?: "A2"}]`. Fuente Arial en todo.
/// Devuelve lo que cambió en las fórmulas (traducciones), para decírselo al modelo.
/// Un `formato` → código de número de Excel (`None` = era «negrita»). Medido (banco of-pdf-excel,
/// 9B): pasó `"formato": {"B1:C1": "negrita"}`; se escribió como código de número y Excel abrió el
/// diálogo «Encontramos un problema con contenido…»: el recálculo se colgó 120 s. Las palabras
/// comunes se traducen; lo que no es un código se rechaza ANTES de escribir.
fn formato_numerico(f: &str) -> Result<Option<String>> {
    let t = f.trim();
    let l = t.to_lowercase();
    let palabra = match l.as_str() {
        "negrita" | "bold" | "negritas" => return Ok(None),
        "moneda" | "pesos" | "dinero" | "currency" => Some("$#,##0"),
        "porcentaje" | "percent" | "%" => Some("0.0%"),
        "miles" | "numero" | "número" | "number" | "entero" => Some("#,##0"), // juez-idioma: datos (palabras que acepta la ficha)
        "decimal" | "decimales" => Some("#,##0.00"),
        "fecha" | "date" => Some("dd/mm/yyyy"),
        "texto" | "text" => Some("@"),
        "general" | "" => Some("General"),
        _ => None,
    };
    if let Some(p) = palabra {
        return Ok(Some(p.to_string()));
    }
    // un código de verdad lleva sus marcas (0 # ? @ % y fechas d m y h s) y nada de letras sueltas
    // fuera de comillas o corchetes
    let mut fuera = String::new();
    let (mut en_comillas, mut en_corchete) = (false, false);
    for ch in t.chars() {
        match ch {
            '"' => en_comillas = !en_comillas,
            '[' if !en_comillas => en_corchete = true,
            ']' if !en_comillas => en_corchete = false,
            _ if !en_comillas && !en_corchete => fuera.push(ch),
            _ => {}
        }
    }
    let marcas = fuera.chars().any(|c| "0#?@%".contains(c)) || fuera.chars().any(|c| "dmyhs".contains(c.to_ascii_lowercase()));
    let letras_raras = fuera.chars().any(|c| c.is_alphabetic() && !"dmyhsaep".contains(c.to_ascii_lowercase())); // a, p: AM/PM · e: 0.00E+00
    if !marcas || letras_raras {
        bail_tr!(
            "«{f}» no es un formato de número de Excel: usa «$#,##0», «#,##0.00», «0.0%», «dd/mm/yyyy» (o «moneda», «porcentaje», «miles», «fecha»); la negrita va en «negrita»: [\"A1:D1\"]",
            "«{f}» is not an Excel number format: use «$#,##0», «#,##0.00», «0.0%», «dd/mm/yyyy» (or «moneda», «porcentaje», «miles», «fecha»); bold goes in «negrita»: [\"A1:D1\"]"
        );
    }
    Ok(Some(t.to_string()))
}

pub fn crear(path: &Path, hojas: &Value) -> Result<(usize, Vec<String>)> {
    let hojas = &crate::ooxml::tolerante_o_error(hojas, "hojas")?;
    use rust_xlsxwriter::{Format, Workbook};
    let hojas = hojas.as_array().filter(|a| !a.is_empty()).ok_or_else(|| anyhow_tr!("`hojas` debe ser una lista con al menos una hoja", "`hojas` must be a list with at least one sheet"))?;
    let mut wb = Workbook::new();
    let mut traducidas = vec![];
    let mut n = 0;
    for h in hojas {
        let ws = wb.add_worksheet();
        // sin esto rust_xlsxwriter guarda 0 como resultado de cada fórmula (ver `sin_valor`)
        ws.set_formula_result_default("");
        if let Some(nom) = h["nombre"].as_str() {
            ws.set_name(nom).map_err(|e| anyhow!("nombre de hoja «{nom}»: {e}"))?;
        }
        let mut negrita: Vec<(u32, u16)> = h["negrita"].as_array().cloned().unwrap_or_default().iter().filter_map(|r| r.as_str()).map(rango).collect::<Result<Vec<_>>>()?.into_iter().flatten().collect();
        let mut formatos: Vec<((u32, u16), String)> = vec![];
        if let Some(m) = h["formato"].as_object() {
            for (r, f) in m {
                let f = f.as_str().unwrap_or("General");
                for c in rango(r)? {
                    match formato_numerico(f)? {
                        Some(nf) => formatos.push((c, nf)),
                        None => negrita.push(c),
                    }
                }
            }
        }
        for (f, c, x) in celdas_pedidas(h)? {
            let mut fmt = Format::new().set_font_name("Arial");
            if negrita.contains(&(f, c)) {
                fmt = fmt.set_bold();
            }
            if let Some((_, nf)) = formatos.iter().find(|(k, _)| *k == (f, c)) {
                fmt = fmt.set_num_format(nf);
            }
            match valor(&x) {
                Valor::Numero(v) => ws.write_number_with_format(f, c, v, &fmt).map(|_| ()),
                Valor::Bool(b) => ws.write_boolean_with_format(f, c, b, &fmt).map(|_| ()),
                Valor::Texto(s) => ws.write_string_with_format(f, c, s, &fmt).map(|_| ()),
                Valor::Vacio => Ok(()),
                Valor::Formula(s) => {
                    let (en, cambios) = formula_en_ingles(&s);
                    if !cambios.is_empty() {
                        traducidas.push(format!("{} {}", nombre_celda(f, c as u32), cambios.join(", ")));
                    }
                    if derrama(&en) {
                        ws.write_dynamic_formula_with_format(f, c, sin_valor(&en), &fmt).map(|_| ())
                    } else {
                        ws.write_formula_with_format(f, c, sin_valor(&en), &fmt).map(|_| ())
                    }
                }
            }
            .map_err(|e| anyhow!("celda {}: {e}", nombre_celda(f, c as u32)))?;
            n += 1;
        }
        if let Some(m) = h["anchos"].as_object() {
            for (col, w) in m {
                let (_, c) = celda(&format!("{col}1"))?;
                ws.set_column_width(c, w.as_f64().unwrap_or(12.0)).map_err(|e| anyhow!("ancho {col}: {e}"))?;
            }
        }
        if let Some(fz) = h["fijar"].as_str() {
            let (f, c) = celda(fz)?;
            ws.set_freeze_panes(f, c).map_err(|e| anyhow!("fijar {fz}: {e}"))?;
        }
    }
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    wb.save(path).map_err(|e| anyhow!("no pude guardar {}: {e}", path.display()))?;
    Ok((n, traducidas))
}

/// EDITAR un libro existente: SÓLO las celdas pedidas (el resto, con su formato y sus fórmulas,
/// queda intacto). `cambios`: `[{hoja?, celdas: {"B3": 9000, "B6": "=SUM(B2:B5)"}}]`. Devuelve
/// (celdas cambiadas, traducciones, ruta del respaldo).
pub fn editar(path: &Path, cambios: &Value) -> Result<(usize, Vec<String>, PathBuf)> {
    let cambios = &crate::ooxml::tolerante_o_error(cambios, "cambios")?;
    let cambios = cambios.as_array().filter(|a| !a.is_empty()).ok_or_else(|| anyhow_tr!("`cambios` debe ser una lista: [{{\"hoja\": \"Hoja1\", \"celdas\": {{\"B3\": 9000}}}}]", "`cambios` must be a list: [{{\"hoja\": \"Sheet1\", \"celdas\": {{\"B3\": 9000}}}}]"))?;
    let mut libro = umya_spreadsheet::reader::xlsx::read(path).map_err(|e| anyhow!("no pude abrir {}: {e}", path.display()))?;
    let primera = libro.sheet_collection().first().map(|s| s.name().to_string()).ok_or_else(|| anyhow_tr!("el libro no tiene hojas", "the workbook has no sheets"))?;
    let mut n = 0;
    let mut traducidas = vec![];
    for cb in cambios {
        let hoja = cb["hoja"].as_str().unwrap_or(&primera).to_string();
        let ws = libro.sheet_by_name_mut(&hoja).map_err(|_| anyhow!("no hay hoja «{hoja}»"))?;
        for (f, c, x) in celdas_pedidas(cb)? {
            let nom = nombre_celda(f, c as u32);
            let cel = ws.cell_mut(nom.as_str());
            match valor(&x) {
                Valor::Numero(v) => {
                    cel.set_value_number(v);
                }
                Valor::Bool(b) => {
                    cel.set_value_bool(b);
                }
                Valor::Texto(s) => {
                    cel.set_value_string(s);
                }
                Valor::Vacio => {
                    cel.set_value_string("");
                }
                Valor::Formula(s) => {
                    let (en, t) = formula_en_ingles(&s);
                    if !t.is_empty() {
                        traducidas.push(format!("{nom} {}", t.join(", ")));
                    }
                    cel.set_formula(en.trim_start_matches('='));
                }
            }
            n += 1;
        }
    }
    let r = respaldo(path);
    std::fs::copy(path, &r)?;
    umya_spreadsheet::writer::xlsx::write(&libro, path).map_err(|e| anyhow!("no pude guardar {}: {e}", path.display()))?;
    Ok((n, traducidas, r))
}

/// `x.xlsx` → `x.respaldo-<segundos>.xlsx` (al lado).
fn respaldo(p: &Path) -> PathBuf {
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let stem = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    p.with_file_name(format!("{stem}.respaldo-{t}.xlsx"))
}

#[cfg(test)]
mod pruebas {
    use super::*;
    use serde_json::json;

    #[test]
    fn celdas_y_nombres() {
        assert_eq!(celda("B5").unwrap(), (4, 1));
        assert_eq!(celda("$AA$10").unwrap(), (9, 26));
        assert_eq!(nombre_celda(4, 1), "B5");
        assert_eq!(nombre_celda(9, 26), "AA10");
        assert!(celda("5B").is_err() && celda("B0").is_err());
        assert_eq!(rango("A1:B2").unwrap().len(), 4);
    }

    /// La fórmula EXACTA del 9B y las comunes en español; nada cambia dentro de comillas.
    #[test]
    fn formulas_en_espanol_pasan_a_ingles() {
        assert_eq!(formula_en_ingles("=SUMA(B2:B4)").0, "=SUM(B2:B4)");
        assert_eq!(formula_en_ingles("=SI(PROMEDIO(A1:A3)>5;REDONDEAR(A1;2);\"no; nada\")").0, "=IF(AVERAGE(A1:A3)>5,ROUND(A1,2),\"no; nada\")");
        assert_eq!(formula_en_ingles("=SI.ERROR(BUSCARV(A2;Tabla;2;FALSO);0)").0, "=IFERROR(VLOOKUP(A2,Tabla,2,FALSO),0)");
        assert_eq!(formula_en_ingles("=AÑO(HOY())").0, "=YEAR(TODAY())");
        let (f, c) = formula_en_ingles("=SUM(B2:B4)*2");
        assert_eq!((f.as_str(), c.len()), ("=SUM(B2:B4)*2", 0), "en inglés: igual");
    }

    /// Crear (Arial, fórmula en español traducida) → leer: la fórmula está y, como la escribe una
    #[test]
    fn los_formatos_de_numero_se_validan_antes_de_escribir() {
        for bueno in ["$#,##0", "$#,##0;($#,##0);-", "#,##0.00", "0.0%", "dd/mm/yyyy", "\"$\"#,##0", "[$-es-CO]dd/mm/yyyy", "0.00E+00", "h:mm AM/PM", "@"] {
            assert_eq!(formato_numerico(bueno).unwrap().as_deref(), Some(bueno), "{bueno}");
        }
        assert_eq!(formato_numerico("moneda").unwrap().as_deref(), Some("$#,##0"));
        assert_eq!(formato_numerico("General").unwrap().as_deref(), Some("General"));
        assert_eq!(formato_numerico("negrita").unwrap(), None, "la negrita no es un formato de número");
        for malo in ["rojo", "bonito", "centrado"] {
            assert!(formato_numerico(malo).is_err(), "{malo}");
        }
        // la forma real del 9B: «negrita» en `formato` acaba en negrita, no en un código roto
        let d = crate::CarpetaPrueba::nueva("gotv-excel-formato");
        let p = d.join("pedidos.xlsx");
        crear(&p, &json!([{"filas": [["Producto", "Precio"], ["Té", 6500]], "formato": {"A1:B1": "negrita"}}])).unwrap();
        let estilos = {
            let mut z = zip::ZipArchive::new(std::fs::File::open(&p).unwrap()).unwrap();
            let mut s = String::new();
            std::io::Read::read_to_string(&mut z.by_name("xl/styles.xml").unwrap(), &mut s).unwrap();
            s
        };
        assert!(!estilos.contains("formatCode=\"negrita\"") && estilos.contains("<b/>"), "{estilos}");
    }

    /// LIBRERÍA, sin valor guardado → `revisar` lo dice (hay que recalcular, como en Claude Code).
    #[test]
    fn crear_leer_y_revisar() {
        let d = crate::CarpetaPrueba::nueva("gotv-excel");
        let p = d.join("precios.xlsx");
        let (n, trad) = crear(&p, &json!([{
            "nombre": "Precios",
            "filas": [["Producto", "Precio"], ["Latte", 8000], ["Capuchino", 7500], ["Americano", 5000]],
            "celdas": {"A5": "Total", "B5": "=SUMA(B2:B4)"},
            "negrita": ["A1:B1", "A5:B5"],
            "formato": {"B2:B5": "#,##0"},
            "anchos": {"A": 16},
            "fijar": "A2"
        }])).unwrap();
        assert_eq!(n, 10);
        assert_eq!(trad, vec!["B5 SUMA→SUM"]);
        let l = leer(&p, None, 100).unwrap();
        assert!(l.contains("hojas: Precios"), "{l}");
        assert!(l.contains("A2 = «Latte»") && l.contains("B2 = 8000"), "{l}");
        assert!(l.contains("B5 = =SUM(B2:B4) → (sin valor guardado"), "{l}");
        let r = revisar(&p).unwrap();
        assert_eq!(r.formulas, 1);
        assert_eq!(r.sin_valor, vec!["Precios!B5"]);
        assert!(!r.limpia() && r.informe().contains("recalcula"), "{}", r.informe());
    }

    /// Editar: sólo lo pedido; el formato (negrita) y las demás fórmulas quedan; respaldo al lado.
    #[test]
    fn editar_conserva_lo_demas() {
        let d = crate::CarpetaPrueba::nueva("gotv-excel-editar");
        let p = d.join("libro.xlsx");
        crear(&p, &json!([{"nombre": "Hoja1", "filas": [["Producto", "Precio"], ["Latte", 8000]], "celdas": {"C2": "=B2*2", "D2": "=B2+1"}, "negrita": ["A1:B1"]}])).unwrap();
        let (n, trad, resp) = editar(&p, &json!([{"celdas": {"B2": 9000, "E2": "=PROMEDIO(B2:C2)"}}])).unwrap();
        assert_eq!(n, 2);
        assert_eq!(trad, vec!["E2 PROMEDIO→AVERAGE"]);
        assert!(resp.exists(), "respaldo");
        let l = leer(&p, None, 100).unwrap();
        assert!(l.contains("B2 = 9000"), "{l}");
        assert!(l.contains("C2 = =B2*2") && l.contains("D2 = =B2+1") && l.contains("E2 = =AVERAGE(B2:C2)"), "las fórmulas que no se tocaron siguen: {l}");
        let libro = umya_spreadsheet::reader::xlsx::read(&p).unwrap();
        let ws = libro.sheet_by_name("Hoja1").unwrap();
        assert!(ws.cell("A1").unwrap().style().font().unwrap().bold(), "la negrita de A1 sigue");
        assert!(editar(&p, &json!([{"hoja": "NoExiste", "celdas": {"A1": 1}}])).is_err());
    }
}

#[cfg(test)]
mod con_excel {
    use super::*;
    use serde_json::json;

    /// El ciclo de Claude Code con el EXCEL real: crear (fórmula en español) → recalcular → revisar
    /// limpio y leer el total; un error sembrado (`=1/0`) se encuentra; editar → recalcular de nuevo.
    /// `cargo test -p gotv-core --release -- --ignored --nocapture ciclo_con_excel`
    #[tokio::test]
    #[ignore]
    async fn ciclo_con_excel() {
        let d = crate::CarpetaPrueba::nueva("gotv-excel-real");
        let p = d.join("precios.xlsx");
        crear(&p, &json!([{"nombre": "Precios", "filas": [["Producto", "Precio"], ["Latte", 8000], ["Capuchino", 7500], ["Americano", 5000]], "celdas": {"A5": "Total", "B5": "=SUMA(B2:B4)", "C2": "=1/0"}}])).unwrap();
        crate::apps::recalcular_excel(&p, 120).await.unwrap();
        let l = leer(&p, None, 100).unwrap();
        println!("{l}");
        assert!(l.contains("B5 = =SUM(B2:B4) → 20500"), "{l}");
        let r = revisar(&p).unwrap();
        println!("{}", r.informe());
        assert_eq!(r.errores, vec![("Precios!C2".to_string(), "#DIV/0!".to_string())], "el error sembrado se encuentra");
        assert!(r.sin_valor.is_empty());
        // corregir y editar: el total sigue a los datos
        editar(&p, &json!([{"celdas": {"C2": "", "B2": 9000}}])).unwrap();
        crate::apps::recalcular_excel(&p, 120).await.unwrap();
        let l = leer(&p, None, 100).unwrap();
        assert!(l.contains("B5 = =SUM(B2:B4) → 21500"), "{l}");
        assert!(revisar(&p).unwrap().limpia(), "{}", revisar(&p).unwrap().informe());
    }
}

#[cfg(test)]
mod pdf_con_excel {
    use super::*;
    use serde_json::json;

    /// El libro ENTERO a PDF con el Excel del usuario (dos hojas → el PDF las lleva).
    /// `cargo test -p gotv-core --release -- --ignored --nocapture excel_a_pdf`
    #[tokio::test]
    #[ignore]
    async fn excel_a_pdf() {
        let d = crate::CarpetaPrueba::nueva("gotv-excel-pdf");
        let p = d.join("libro.xlsx");
        crear(&p, &json!([{"nombre": "Uno", "filas": [["A", 1]]}, {"nombre": "Dos", "filas": [["B", 2]]}])).unwrap();
        crate::apps::exportar_pdf(&p, &d.join("libro.pdf"), 120).await.unwrap();
        let b = std::fs::read(d.join("libro.pdf")).unwrap();
        let t = String::from_utf8_lossy(&b);
        let paginas = t.matches("/Type /Page\n").count() + t.matches("/Type/Page/").count() + t.matches("/Type /Page/").count() + t.matches("/Type /Page>").count() + t.matches("/Type/Page>").count();
        println!("PDF {} bytes · marcas de página ~{paginas}", b.len());
        assert!(b.starts_with(b"%PDF"));
    }
}

#[cfg(test)]
mod a_mano {
    /// Crea un .xlsx desde `hojas` en JSON (sin recalcular): `GOTV_FICHA=h.json GOTV_SALIDA=x.xlsx cargo test -p gotv-core -- --ignored crear_xlsx`
    #[test]
    #[ignore]
    fn crear_xlsx() {
        let (Ok(f), Ok(s)) = (std::env::var("GOTV_FICHA"), std::env::var("GOTV_SALIDA")) else { return };
        let hojas: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(f).unwrap()).unwrap();
        super::crear(std::path::Path::new(&s), &hojas).unwrap();
    }
}
