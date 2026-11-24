//! O4 · PDF de DISEÑO: de una ficha de bloques (la MISMA del Word: título, secciones, párrafos, listas,
//! tablas, línea, salto) a un PDF maquetado con el tema del documento (`tema_doc`), impreso por el
//! navegador propio de GOTV (`agent-browser`, que usa Chrome): página → PDF.
//!
//! - **El HTML** (`html`) es puro y se prueba solo: tamaño de página y márgenes en `@page`, número de
//!   página al pie («n / total»), las tipografías de `gotv-diseno` INCRUSTADAS (un PDF sí puede llevarlas
//!   dentro, al revés que un .docx), cabecera de tabla que se repite en cada página, filas alternas,
//!   viudas y huérfanas controladas, y los títulos pegados a lo que sigue.
//! - **Imprimir** (`crear`) abre ese HTML con `file://` y pide `pdf`; después comprueba que el archivo
//!   exista y sea un PDF. Nunca sobrescribe.
use crate::tema_doc::TemaDoc;
use anyhow::{anyhow, Result};
use gotv_engine::anyhow_tr;
use serde_json::Value;
use std::path::Path;

fn esc(t: &str) -> String {
    t.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// Un texto de la ficha con su `**negrita**` y su `*cursiva*`.
fn rico(t: &str) -> String {
    crate::word::trozos(t)
        .into_iter()
        .map(|(x, neg, cur)| {
            let x = esc(&x);
            match (neg, cur) {
                (true, true) => format!("<strong><em>{x}</em></strong>"),
                (true, false) => format!("<strong>{x}</strong>"),
                (false, true) => format!("<em>{x}</em>"),
                _ => x,
            }
        })
        .collect()
}

/// El tamaño de página en milímetros.
fn pagina(ficha: &Value) -> (f64, f64) {
    match ficha["pagina"].as_str().unwrap_or("carta").to_lowercase().as_str() {
        "a4" => (210.0, 297.0),
        "oficio" | "legal" => (215.9, 355.6),
        _ => (215.9, 279.4),
    }
}

/// Las tipografías del tema: (pila del título, pila del texto, `@font-face` incrustados).
fn tipografias(t: &TemaDoc) -> (String, String, String) {
    use base64::Engine;
    let respaldo = |f: &str| if f == "Georgia" { "Georgia, \"Times New Roman\", serif" } else if f == "Courier New" { "\"Courier New\", monospace" } else { "Arial, Helvetica, sans-serif" };
    let (mut titulo, mut texto, mut caras) = (respaldo(&t.fuente_titulo).to_string(), respaldo(&t.fuente_cuerpo).to_string(), String::new());
    if let Some((a, b)) = (!t.par.is_empty()).then(|| gotv_diseno::recursos::par(&t.par)).flatten() {
        for f in [a, b] {
            let datos = base64::engine::general_purpose::STANDARD.encode(f.woff2);
            let cara = gotv_diseno::recursos::font_face(f, &format!("data:font/woff2;base64,{datos}"));
            if !caras.contains(&format!("\"{}\"", f.familia)) {
                // `block`: al imprimir se espera a la fuente; con `swap` saldría la de respaldo
                caras.push_str(&cara.replace("font-display: swap", "font-display: block"));
            }
        }
        titulo = format!("\"{}\", {titulo}", a.familia);
        texto = format!("\"{}\", {texto}", b.familia);
    }
    (titulo, texto, caras)
}

/// El HTML listo para imprimir y cuántos bloques trae.
pub fn html(ficha: &Value) -> Result<(String, usize)> {
    let ficha = &crate::ooxml::tolerante_o_error(ficha, "ficha")?;
    let bloques = ficha["bloques"].as_array().filter(|b| !b.is_empty()).ok_or_else(|| anyhow_tr!("la ficha necesita «bloques»: [{{\"titulo\":\"…\"}}, {{\"parrafo\":\"…\"}}]", "the spec needs «bloques»: [{{\"titulo\":\"…\"}}, {{\"parrafo\":\"…\"}}]"))?;
    // sin tema pedido, uno sobrio (gris): un PDF de diseño siempre va vestido
    let t = crate::tema_doc::de(&ficha["tema"]).or_else(|| crate::tema_doc::de(&Value::String("gris".into()))).ok_or_else(|| anyhow!("tema"))?;
    let (pw, ph) = pagina(ficha);
    let (f_titulo, f_texto, caras) = tipografias(&t);
    let mut cuerpo = String::new();
    for crudo in bloques {
        let b = &crate::word::normalizar_bloque(crudo);
        // la tolerancia de `word` convierte lo que no entiende en un párrafo SIN texto: aquí eso no se
        // traga en silencio (un bloque que desaparece del PDF es un informe incompleto sin aviso)
        if b["parrafo"].as_bool().is_some() {
            let claves: Vec<&String> = crudo.as_object().map(|o| o.keys().collect()).unwrap_or_default();
            return Err(anyhow_tr!("bloque desconocido {claves:?}: usa titulo, h1, h2, h3, parrafo, lista, numerada, tabla, salto o linea", "unknown block {claves:?}: use titulo, h1, h2, h3, parrafo, lista, numerada, tabla, salto or linea"));
        }
        let o = b.as_object().ok_or_else(|| anyhow_tr!("cada bloque es un objeto, p. ej. {{\"parrafo\": \"…\"}}", "each block is an object, e.g. {{\"parrafo\": \"…\"}}"))?;
        let clase = match crate::word::alineacion(b) {
            Some("center") => " class=\"cen\"",
            Some("right") => " class=\"der\"",
            Some("both") => " class=\"jus\"",
            _ => "",
        };
        let lineas = |v: &Value| -> Vec<String> { v.as_str().unwrap_or_default().split('\n').filter(|l| !l.trim().is_empty()).map(str::to_string).collect() };
        if let Some(v) = o.get("titulo") {
            cuerpo.push_str(&format!("<h1{clase}>{}</h1>\n", lineas(v).iter().map(|l| rico(l)).collect::<Vec<_>>().join("<br>")));
        } else if let Some((n, v)) = ["h1", "h2", "h3"].iter().enumerate().find_map(|(i, k)| o.get(*k).map(|v| (i + 2, v))) {
            for l in lineas(v) {
                cuerpo.push_str(&format!("<h{n}{clase}>{}</h{n}>\n", rico(&l)));
            }
        } else if let Some(v) = o.get("parrafo") {
            for l in lineas(v) {
                cuerpo.push_str(&format!("<p{clase}>{}</p>\n", rico(&l)));
            }
        } else if let Some(items) = o.get("lista").or_else(|| o.get("numerada")) {
            let et = if o.contains_key("numerada") { "ol" } else { "ul" };
            let items = items.as_array().ok_or_else(|| anyhow_tr!("«lista» y «numerada» llevan una lista de textos", "«lista» and «numerada» take a list of texts"))?;
            cuerpo.push_str(&format!("<{et}>"));
            let mut abierto = false;
            for it in items {
                match it {
                    // un ítem que es una lista = segundo nivel, dentro del ítem anterior
                    Value::Array(sub) => {
                        let hijos: String = sub.iter().map(|x| format!("<li>{}</li>", rico(x.as_str().unwrap_or_default()))).collect();
                        if !abierto {
                            cuerpo.push_str("<li>");
                        }
                        cuerpo.push_str(&format!("<{et}>{hijos}</{et}></li>"));
                        abierto = false;
                    }
                    x => {
                        if abierto {
                            cuerpo.push_str("</li>");
                        }
                        cuerpo.push_str(&format!("<li>{}", rico(x.as_str().unwrap_or(&x.to_string()))));
                        abierto = true;
                    }
                }
            }
            if abierto {
                cuerpo.push_str("</li>");
            }
            cuerpo.push_str(&format!("</{et}>\n"));
        } else if let Some(v) = o.get("tabla") {
            let filas: Vec<Vec<String>> = v
                .as_array()
                .ok_or_else(|| anyhow_tr!("«tabla» es una lista de filas: [[\"Encabezado\",…],[\"dato\",…]]", "«tabla» is a list of rows: [[\"Header\",…],[\"value\",…]]"))?
                .iter()
                .map(|f| f.as_array().cloned().unwrap_or_default().iter().map(|c| c.as_str().map(str::to_string).unwrap_or_else(|| c.to_string())).collect())
                .collect();
            let cols = filas.iter().map(Vec::len).max().unwrap_or(0);
            if cols == 0 {
                return Err(anyhow_tr!("la tabla está vacía", "the table is empty"));
            }
            let pesos: Vec<f64> = match b["anchos"].as_array() {
                Some(a) if a.len() == cols => a.iter().map(|x| x.as_f64().unwrap_or(1.0).max(0.1)).collect(),
                _ => vec![1.0; cols],
            };
            let total: f64 = pesos.iter().sum();
            cuerpo.push_str("<table><colgroup>");
            for p in &pesos {
                cuerpo.push_str(&format!("<col style=\"width:{:.3}%\">", 100.0 * p / total));
            }
            cuerpo.push_str("</colgroup>");
            let encabezado = b["encabezado"].as_bool().unwrap_or(true);
            let fila = |f: &Vec<String>, celda: &str| -> String { (0..cols).map(|j| format!("<{celda}>{}</{celda}>", rico(f.get(j).map(String::as_str).unwrap_or("")))).collect() };
            let mut resto = &filas[..];
            if encabezado {
                cuerpo.push_str(&format!("<thead><tr>{}</tr></thead>", fila(&filas[0], "th")));
                resto = &filas[1..];
            }
            cuerpo.push_str("<tbody>");
            for f in resto {
                cuerpo.push_str(&format!("<tr>{}</tr>", fila(f, "td")));
            }
            cuerpo.push_str("</tbody></table>\n");
        } else if o.contains_key("salto") {
            cuerpo.push_str("<div class=\"salto\"></div>\n");
        } else if o.contains_key("linea") {
            cuerpo.push_str("<hr>\n");
        } else {
            let claves: Vec<&String> = o.keys().collect();
            return Err(anyhow_tr!("bloque desconocido {claves:?}: usa titulo, h1, h2, h3, parrafo, lista, numerada, tabla, salto o linea", "unknown block {claves:?}: use titulo, h1, h2, h3, parrafo, lista, numerada, tabla, salto or linea"));
        }
    }
    let titulo = ficha["titulo_doc"].as_str().or_else(|| bloques.iter().find_map(|b| b["titulo"].as_str().or(b["h1"].as_str()))).unwrap_or("Documento");
    let numeros = if ficha["numeros_pagina"].as_bool().unwrap_or(true) { format!("@bottom-center {{ content: counter(page) \" / \" counter(pages); font: 8.5pt {f_texto}; color: #{}; }}", t.tenue) } else { String::new() };
    let css = format!(
        "{caras}@page {{ size: {pw}mm {ph}mm; margin: 22mm 22mm 24mm; {numeros} }}\n\
html {{ -webkit-print-color-adjust: exact; print-color-adjust: exact; }}\n\
body {{ margin: 0; font: 10.5pt/1.5 {f_texto}; color: #{texto}; }}\n\
h1, h2, h3, h4 {{ font-family: {f_titulo}; break-after: avoid; break-inside: avoid; }}\n\
h1 {{ font-size: 26pt; line-height: 1.15; font-weight: 700; color: #{c_titulo}; margin: 0 0 16pt; padding-bottom: 9pt; border-bottom: 1.5pt solid #{primario}; }}\n\
h2 {{ font-size: 15pt; line-height: 1.25; font-weight: 700; color: #{c_titulo}; margin: 20pt 0 6pt; }}\n\
h3 {{ font-size: 12.5pt; font-weight: 700; color: #{texto}; margin: 14pt 0 5pt; }}\n\
h4 {{ font-size: 11pt; font-weight: 700; color: #{tenue}; margin: 12pt 0 4pt; }}\n\
p {{ margin: 0 0 8pt; orphans: 3; widows: 3; }}\n\
.cen {{ text-align: center; }} .der {{ text-align: right; }} .jus {{ text-align: justify; hyphens: auto; }}\n\
ul, ol {{ margin: 0 0 8pt; padding-left: 18pt; }} li {{ margin-bottom: 3pt; }} li > ul, li > ol {{ margin: 3pt 0 0; }}\n\
ul > li::marker {{ color: #{primario}; }}\n\
table {{ width: 100%; border-collapse: collapse; margin: 6pt 0 12pt; font-size: 9.5pt; }}\n\
thead {{ display: table-header-group; }} tr {{ break-inside: avoid; }}\n\
th {{ background: #{primario}; color: #{sobre}; text-align: left; font-weight: 700; padding: 5pt 7pt; }}\n\
td {{ padding: 5pt 7pt; border-bottom: 0.5pt solid #{borde}; vertical-align: top; }}\n\
tbody tr:nth-child(even) td {{ background: #{suave}; }}\n\
hr {{ border: 0; border-top: 1pt solid #{primario}; margin: 12pt 0; }}\n\
.salto {{ break-after: page; }}\n",
        texto = t.texto,
        c_titulo = t.titulo,
        primario = t.primario,
        tenue = t.tenue,
        sobre = t.sobre_primario,
        borde = t.borde,
        suave = t.suave,
    );
    let idioma = crate::i18n::t("es", "en");
    Ok((format!("<!doctype html>\n<html lang=\"{idioma}\"><head><meta charset=\"utf-8\"><title>{}</title>\n<style>\n{css}</style></head>\n<body>\n{cuerpo}</body></html>\n", esc(titulo)), bloques.len()))
}

/// Crea el PDF: HTML temporal dentro del proyecto → navegador propio → `destino` (que no debe existir).
pub async fn crear(owner: &str, root: &Path, ficha: &Value, destino: &Path) -> Result<String> {
    if destino.extension().map(|e| e.to_string_lossy().to_lowercase()) != Some("pdf".into()) {
        return Err(anyhow_tr!("`destino` debe terminar en .pdf", "`destino` must end in .pdf"));
    }
    if destino.exists() {
        return Err(anyhow_tr!("{} ya existe: elige otro nombre (nunca se sobrescribe)", "{} already exists: choose another name (nothing is overwritten)", destino.display()));
    }
    let (pagina, n) = html(ficha)?;
    let tmp = root.join(".gotv").join("tmp");
    std::fs::create_dir_all(&tmp)?;
    let ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
    let fuente = tmp.join(format!("pdf-{ms}.html"));
    std::fs::write(&fuente, pagina)?;
    if let Some(d) = destino.parent() {
        std::fs::create_dir_all(d)?;
    }
    let url = format!("file://{}", fuente.display().to_string().replace(' ', "%20"));
    let r = async {
        crate::browser::run(owner, root, &["open".into(), url], None, 60).await?;
        // que las fuentes incrustadas estén listas antes de imprimir
        let _ = crate::browser::run(owner, root, &["eval".into(), "document.fonts.ready.then(() => document.fonts.size)".into()], None, 30).await;
        crate::browser::run(owner, root, &["pdf".into(), destino.display().to_string()], None, 120).await
    }
    .await;
    let _ = std::fs::remove_file(&fuente);
    r?;
    let bytes = std::fs::read(destino).map_err(|_| anyhow_tr!("el navegador no escribió {}", "the browser did not write {}", destino.display()))?;
    if !bytes.starts_with(b"%PDF") {
        let _ = std::fs::remove_file(destino);
        return Err(anyhow_tr!("lo que escribió el navegador no es un PDF", "what the browser wrote is not a PDF"));
    }
    let paginas = lopdf::Document::load_mem(&bytes).map(|d| d.get_pages().len()).unwrap_or(0);
    Ok(crate::tr!(
        "✓ {} creado: {n} bloque(s), {paginas} página(s), {} KB. Míralo con accion «imagenes» + view_image.",
        "✓ {} created: {n} block(s), {paginas} page(s), {} KB. Look at it with accion «imagenes» + view_image.",
        destino.display(),
        bytes.len() / 1024
    ))
}

#[cfg(test)]
mod pruebas {
    use super::*;
    use serde_json::json;

    fn ficha() -> Value {
        json!({"pagina": "a4", "tema": {"marca": "verde", "fuentes": "editorial"}, "bloques": [
            {"titulo": "Informe <anual> & cierre"},
            {"parrafo": "Bogotá, 14 de noviembre de 2026", "alineacion": "derecha"},
            {"h1": "Resumen"},
            {"parrafo": "Crecimos **un 12 %** y *sin deuda*."},
            {"lista": ["Ventas", ["Tienda", "Web"], "Costos"]},
            {"numerada": ["Diagnóstico", "Plan"]},
            {"tabla": [["Línea", "Total"], ["Tienda", "$10"], ["Web", "$5"]], "anchos": [3, 1]},
            {"linea": true},
            {"salto": true},
            {"h2": "Anexo"}
        ]})
    }

    /// El HTML lleva el tamaño de página, el pie con «n / total», las fuentes del par INCRUSTADAS, la
    /// tabla con cabecera repetible y sus anchos, las listas anidadas bien cerradas, y el texto escapado.
    #[test]
    fn el_html_lleva_todo_lo_del_diseno() {
        let (h, n) = html(&ficha()).unwrap();
        assert_eq!(n, 10);
        let t = crate::tema_doc::de(&ficha()["tema"]).unwrap();
        assert!(h.contains("@page { size: 210mm 297mm;") && h.contains("counter(page) \" / \" counter(pages)"));
        assert!(h.contains("font-family: \"Fraunces\"") && h.contains("font-family: \"Newsreader\"") && h.contains("data:font/woff2;base64,") && h.contains("font-display: block"));
        assert!(h.contains(&format!("th {{ background: #{}; color: #{};", t.primario, t.sobre_primario)));
        assert!(h.contains("<h1>Informe &lt;anual&gt; &amp; cierre</h1>") && h.contains("<title>Informe &lt;anual&gt; &amp; cierre</title>"));
        assert!(h.contains("<p class=\"der\">Bogotá, 14 de noviembre de 2026</p>"));
        assert!(h.contains("<p>Crecimos <strong>un 12 %</strong> y <em>sin deuda</em>.</p>"));
        assert!(h.contains("<ul><li>Ventas<ul><li>Tienda</li><li>Web</li></ul></li><li>Costos</li></ul>"), "lista anidada dentro de su ítem");
        assert!(h.contains("<ol><li>Diagnóstico</li><li>Plan</li></ol>"));
        assert!(h.contains("<col style=\"width:75.000%\"><col style=\"width:25.000%\">") && h.contains("<thead><tr><th>Línea</th><th>Total</th></tr></thead><tbody><tr><td>Tienda</td><td>$10</td></tr>"));
        assert!(h.contains("<hr>") && h.contains("<div class=\"salto\"></div>") && h.contains("<h3>Anexo</h3>"));
        // cada etiqueta abierta se cierra (un HTML roto imprime mal sin avisar)
        for et in ["ul", "ol", "li", "table", "thead", "tbody", "tr", "p", "h1", "h2", "h3"] {
            assert_eq!(h.matches(&format!("<{et}>")).count() + h.matches(&format!("<{et} ")).count(), h.matches(&format!("</{et}>")).count(), "<{et}>");
        }
    }

    /// Sin tema sale sobrio (gris, par técnico); carta por omisión; lo que no vale se dice.
    #[test]
    fn sin_tema_sobrio_y_lo_invalido_se_dice() {
        let (h, _) = html(&json!({"numeros_pagina": false, "bloques": [{"titulo": "Acta"}, {"parrafo": "Texto"}]})).unwrap();
        assert!(h.contains("size: 215.9mm 279.4mm") && !h.contains("counter(page)"));
        // sobrio = gris, con el par técnico incrustado (Space Grotesk + Inter) y Arial de respaldo
        assert!(h.contains("\"Inter\", Arial, Helvetica, sans-serif") && h.contains("font-family: \"Space Grotesk\""));
        assert!(html(&json!({"bloques": []})).is_err());
        assert!(html(&json!({"bloques": [{"tabla": []}]})).is_err());
        assert!(html(&json!({"bloques": [{"volar": 1}]})).is_err());
    }

    /// CON EL NAVEGADOR (a mano: `cargo test -p gotv-core --release -- --ignored --nocapture pdf_de_diseno_real`):
    /// imprime la ficha y comprueba páginas, tamaño y que el texto se lee del PDF.
    #[tokio::test]
    #[ignore]
    async fn pdf_de_diseno_real() {
        let d = crate::CarpetaPrueba::nueva("gotv-pdf-diseno");
        let destino = d.join("informe.pdf");
        let r = crear("prueba-pdf", &d, &ficha(), &destino).await.unwrap();
        println!("{r}");
        let info = crate::pdf::info(&destino).unwrap();
        println!("{info}");
        let texto = crate::pdf::leer(&destino, "").unwrap();
        println!("{texto}");
        assert!(info.contains("2 p") && texto.contains("Resumen") && texto.contains("Tienda") && texto.contains("1 / 2"), "{info}\n{texto}");
        assert!(crear("prueba-pdf", &d, &ficha(), &destino).await.is_err(), "nunca sobrescribe");
        crate::browser::close_session("prueba-pdf");
        if let Some(o) = std::env::var_os("GOTV_PDF_SALIDA") {
            std::fs::copy(&destino, o).unwrap();
        }
    }
}
