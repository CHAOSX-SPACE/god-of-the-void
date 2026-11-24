//! C6 — Leer de todo, como el `Read` de Claude Code: PDF por páginas (PDFKit de macOS por
//! JXA: sin instalar nada), notebooks de Jupyter (celdas con sus salidas) e imágenes
//! (se derivan a `view_image`).

use anyhow::{anyhow, Result};
use std::path::Path;
use std::time::Duration;

/// Páginas por llamada (como Claude Code: más que eso, por rangos).
pub const PDF_MAX_PAGES: usize = 20;
const MAX_CHARS: usize = 24_000;

/// El script recibe la ruta y el rango por `argv` (nunca pegados al código).
const PDF_JXA: &str = r#"
ObjC.import('PDFKit');
function run(argv) {
  var d = $.PDFDocument.alloc.initWithURL($.NSURL.fileURLWithPath(argv[0]));
  if (!d || d.isNil()) { return "ERROR: no es un PDF legible"; }
  var n = d.pageCount, a = parseInt(argv[1]), b = Math.min(parseInt(argv[2]), n);
  var out = ["PAGES " + n];
  for (var i = a - 1; i < b; i++) {
    var p = d.pageAtIndex(i), s = p.string;
    out.push("--- " + (i + 1) + " ---");
    out.push(s.isNil() ? "" : s.js);
  }
  return out.join("\n");
}
"#;

/// «3», «1-5», «10-» → (desde, hasta) 1-based.
pub fn parse_pages(s: &str) -> Option<(usize, usize)> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    match s.split_once('-') {
        Some((a, b)) => {
            let a: usize = a.trim().parse().ok()?;
            let b: usize = if b.trim().is_empty() {
                usize::MAX
            } else {
                b.trim().parse().ok()?
            };
            (a >= 1 && b >= a).then_some((a, b))
        }
        None => s.parse().ok().filter(|n| *n >= 1).map(|n| (n, n)),
    }
}

pub async fn pdf(path: &Path, pages: Option<&str>) -> Result<String> {
    let (a, b) = match pages {
        Some(p) => parse_pages(p).ok_or_else(|| {
            anyhow!(
                "{}",
                crate::tr!(
                    "páginas inválidas «{p}» (p. ej. 3 o 1-5)",
                    "invalid pages «{p}» (e.g. 3 or 1-5)"
                )
            )
        })?,
        None => (1, PDF_MAX_PAGES),
    };
    let b = b.min(a + PDF_MAX_PAGES - 1);
    let o = tokio::time::timeout(
        Duration::from_secs(60),
        tokio::process::Command::new("osascript")
            .args([
                "-l",
                "JavaScript",
                "-e",
                PDF_JXA,
                &path.to_string_lossy(),
                &a.to_string(),
                &b.to_string(),
            ])
            .output(),
    )
    .await
    .map_err(|_| anyhow!("PDF: tiempo"))??;
    let text = String::from_utf8_lossy(&o.stdout).to_string();
    if !o.status.success() || text.starts_with("ERROR") {
        return Err(anyhow!(
            "{}",
            crate::tr!(
                "no pude leer el PDF {}: {}",
                "could not read the PDF {}: {}",
                path.display(),
                String::from_utf8_lossy(&o.stderr).trim()
            )
        ));
    }
    let total: usize = text
        .lines()
        .next()
        .and_then(|l| l.strip_prefix("PAGES "))
        .and_then(|n| n.trim().parse().ok())
        .unwrap_or(0);
    let body: String = text.lines().skip(1).collect::<Vec<_>>().join("\n");
    let shown_to = b.min(total);
    let mut head = crate::tr!(
        "[PDF {} · {total} páginas · mostrando {a}–{shown_to}]",
        "[PDF {} · {total} pages · showing {a}–{shown_to}]",
        path.display()
    );
    if shown_to < total {
        head.push_str(&crate::tr!(
            " — sigue con pages=\"{}-{}\"",
            " — continue with pages=\"{}-{}\"",
            shown_to + 1,
            (shown_to + PDF_MAX_PAGES).min(total)
        ));
    }
    if body.trim().is_empty() {
        head.push_str(crate::i18n::t(
            "\n(sin texto: puede ser un PDF escaneado; si tu modelo ve, conviértelo a imagen y usa view_image)",
            "\n(no text: it may be a scanned PDF; if your model can see, turn it into an image and use view_image)",
        ));
    }
    Ok(cap(format!("{head}\n{body}")))
}

fn cap(s: String) -> String {
    if s.chars().count() <= MAX_CHARS {
        return s;
    }
    let cut: String = s.chars().take(MAX_CHARS).collect();
    format!(
        "{cut}\n…[{}]",
        crate::i18n::t(
            "recortado: pide menos páginas",
            "truncated: ask for fewer pages"
        )
    )
}

/// Un notebook de Jupyter: cada celda con su tipo, su código y sus salidas de texto.
pub fn notebook(path: &Path) -> Result<String> {
    let raw = std::fs::read_to_string(path)?;
    let v: serde_json::Value = serde_json::from_str(&raw).map_err(|e| {
        anyhow!(
            "{}",
            crate::tr!("notebook inválido: {e}", "invalid notebook: {e}")
        )
    })?;
    let text = |x: &serde_json::Value| -> String {
        match x {
            serde_json::Value::Array(a) => a.iter().filter_map(|l| l.as_str()).collect::<String>(),
            serde_json::Value::String(s) => s.clone(),
            _ => String::new(),
        }
    };
    let cells = v["cells"].as_array().cloned().unwrap_or_default();
    let lang = v["metadata"]["kernelspec"]["language"]
        .as_str()
        .or_else(|| v["metadata"]["language_info"]["name"].as_str())
        .unwrap_or("?");
    let mut out = vec![crate::tr!(
        "[notebook {} · {} celdas · {lang}]",
        "[notebook {} · {} cells · {lang}]",
        path.display(),
        cells.len()
    )];
    for (i, c) in cells.iter().enumerate() {
        let kind = c["cell_type"].as_str().unwrap_or("?");
        let count = c["execution_count"]
            .as_u64()
            .map(|n| format!(" [{n}]"))
            .unwrap_or_default();
        // F4: el id (nbformat 4.5) para nombrarla en `notebook`; los viejos van por número
        let id = c["id"].as_str().map(|s| format!(" · id {s}")).unwrap_or_default();
        out.push(format!(
            "── {} {}{count}{id} ──",
            crate::tr!("celda {}", "cell {}", i + 1),
            kind
        ));
        out.push(text(&c["source"]).trim_end().to_string());
        for o in c["outputs"].as_array().cloned().unwrap_or_default() {
            let t = match o["output_type"].as_str().unwrap_or("") {
                "stream" => text(&o["text"]),
                "execute_result" | "display_data" => {
                    if o["data"]["image/png"].is_string() || o["data"]["image/jpeg"].is_string() {
                        crate::i18n::t("[imagen en la salida]", "[image in the output]").to_string()
                    } else {
                        text(&o["data"]["text/plain"])
                    }
                }
                "error" => format!(
                    "{}: {}",
                    o["ename"].as_str().unwrap_or("Error"),
                    o["evalue"].as_str().unwrap_or("")
                ),
                _ => String::new(),
            };
            if !t.trim().is_empty() {
                out.push(format!(
                    "{} {}",
                    crate::i18n::t("⇒ salida:", "⇒ output:"),
                    t.trim_end().chars().take(2000).collect::<String>()
                ));
            }
        }
    }
    Ok(cap(out.join("\n")))
}

/// ¿Es una imagen que se debe mirar (no leer como texto)?
pub fn is_image(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())
            .as_deref(),
        Some("png" | "jpg" | "jpeg" | "gif" | "webp" | "heic" | "heif" | "tif" | "tiff" | "bmp")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rangos_de_paginas() {
        assert_eq!(parse_pages("3"), Some((3, 3)));
        assert_eq!(parse_pages("1-5"), Some((1, 5)));
        assert_eq!(parse_pages("10-"), Some((10, usize::MAX)));
        assert_eq!(parse_pages("5-2"), None);
        assert_eq!(parse_pages("0"), None);
    }

    #[test]
    fn notebook_con_celdas_y_salidas() {
        let d = crate::CarpetaPrueba::sin_crear("gotv-nb");
        std::fs::create_dir_all(&d).unwrap();
        let p = d.join("a.ipynb");
        std::fs::write(&p, r##"{"cells":[{"cell_type":"markdown","source":["# Título"]},{"cell_type":"code","execution_count":2,"source":["x = 41\n","print(x+1)"],"outputs":[{"output_type":"stream","text":["42\n"]}]},{"cell_type":"code","source":"1/0","outputs":[{"output_type":"error","ename":"ZeroDivisionError","evalue":"division by zero"}]}],"metadata":{"kernelspec":{"language":"python"}}}"##).unwrap();
        let t = notebook(&p).unwrap();
        assert!(t.contains("3 ") && t.contains("python"), "{t}");
        assert!(
            t.contains("print(x+1)")
                && t.contains("42")
                && t.contains("ZeroDivisionError: division by zero"),
            "{t}"
        );
    }

    #[tokio::test]
    async fn pdf_de_verdad_con_pdfkit() {
        // un PDF mínimo hecho aquí mismo (texto real en la página)
        let d = crate::CarpetaPrueba::sin_crear("gotv-pdf");
        std::fs::create_dir_all(&d).unwrap();
        let p = d.join("hoja.pdf");
        let txt = d.join("hoja.txt");
        std::fs::write(&txt, "VACIO 73 hoja de datos").unwrap();
        // cupsfilter viene con macOS: texto → PDF
        let ok = std::process::Command::new("/usr/sbin/cupsfilter")
            .arg(&txt)
            .output()
            .map(|o| std::fs::write(&p, o.stdout).is_ok() && o.status.success());
        if !matches!(ok, Ok(true)) {
            return; // sin cupsfilter no hay PDF de prueba
        }
        let t = pdf(&p, None).await.unwrap();
        assert!(t.contains("1 páginas") || t.contains("1 pages"), "{t}");
        assert!(t.contains("VACIO 73"), "{t}");
    }
}
