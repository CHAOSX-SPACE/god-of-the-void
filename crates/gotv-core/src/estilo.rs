//! Proyectos existentes (Ola D10): antes de diseñar en un proyecto que YA tiene su estilo, GOTV
//! lo lee (variables CSS, Tailwind, familias tipográficas) y se lo dice al modelo como dato
//! medido: se adopta lo que hay, no se impone una dirección de GOTV encima.

use std::path::{Path, PathBuf};

#[derive(Debug, Default, PartialEq)]
pub struct Estilo {
    /// variables CSS de color: (nombre, valor)
    pub colores: Vec<(String, String)>,
    /// familias tipográficas declaradas (sin las del sistema)
    pub fuentes: Vec<String>,
    pub tailwind: bool,
    /// el proyecto lo generó GOTV (tiene el manifiesto de `pagina`): entonces se rehace con `pagina`
    pub de_gotv: bool,
    /// hojas leídas (para decirle al modelo dónde mirar)
    pub hojas: Vec<PathBuf>,
    /// clases que define la hoja principal (por orden de aparición): lo que el modelo
    /// necesita para escribir una página nueva sin leer la hoja entera
    pub clases: Vec<String>,
    /// cómo enlaza sus hojas una página existente: (archivo, líneas <link>)
    pub cabeza: Option<(String, Vec<String>)>,
}

const SISTEMA: &[&str] = &[
    "sans-serif",
    "serif",
    "monospace",
    "system-ui",
    "ui-sans-serif",
    "ui-serif",
    "ui-monospace",
    "-apple-system",
    "blinkmacsystemfont",
    "segoe ui",
    "roboto",
    "helvetica",
    "helvetica neue",
    "arial",
    "georgia",
    "inherit",
    "initial",
    "cursive",
    "times new roman",
    "menlo",
    "consolas",
    "sf mono",
    "iowan old style",
    "palatino linotype",
    "emoji",
    "apple color emoji",
];

/// Hojas de estilo del proyecto (raíz, src/, styles/, css/, assets/, public/), sin node_modules ni
/// dist; como mucho 20 archivos y 400 KB cada uno.
fn hojas(root: &Path) -> Vec<PathBuf> {
    let mut v = Vec::new();
    let mut pend = vec![(root.to_path_buf(), 0)];
    while let Some((d, prof)) = pend.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in rd.flatten() {
            let p = e.path();
            let n = e.file_name().to_string_lossy().to_string();
            if p.is_dir() {
                let l = n.to_lowercase();
                let respaldo = [
                    "backup",
                    "respaldo",
                    "_old",
                    "-old",
                    "copia",
                    "archivo", // juez-idioma: datos (nombre de carpeta)
                    "archive",
                    "deprecated",
                    "legacy",
                ]
                .iter()
                .any(|x| l.contains(x));
                if prof < 3
                    && !n.starts_with('.')
                    && !respaldo
                    && !matches!(
                        n.as_str(),
                        "node_modules" | "dist" | "build" | "target" | "vendor" | "out" | ".next"
                    )
                {
                    pend.push((p, prof + 1));
                }
            } else if matches!(
                p.extension().and_then(|x| x.to_str()),
                Some("css" | "scss" | "sass" | "less")
            ) && e.metadata().map(|m| m.len() < 400_000).unwrap_or(false)
            {
                v.push(p);
            }
        }
        if v.len() >= 20 {
            break;
        }
    }
    // primero las menos profundas (la hoja principal suele estar arriba), luego por nombre
    v.sort_by_key(|p| {
        (
            p.strip_prefix(root)
                .map(|r| r.components().count())
                .unwrap_or(9),
            p.clone(),
        )
    });
    v.truncate(20);
    v
}

fn parece_color(v: &str) -> bool {
    let v = v.trim().to_lowercase();
    v.starts_with('#')
        || v.starts_with("rgb")
        || v.starts_with("hsl")
        || v.starts_with("oklch")
        || v.starts_with("oklab")
        || v.starts_with("color(")
}

pub fn leer(root: &Path) -> Estilo {
    let mut e = Estilo {
        tailwind: [
            "tailwind.config.js",
            "tailwind.config.ts",
            "tailwind.config.cjs",
            "tailwind.config.mjs",
        ]
        .iter()
        .any(|f| root.join(f).exists()),
        de_gotv: gotv_diseno::pagina::es_de_gotv(root),
        ..Default::default()
    };
    for h in hojas(root) {
        let Ok(t) = std::fs::read_to_string(&h) else {
            continue;
        };
        if t.contains("@tailwind")
            || t.contains("@import \"tailwindcss\"")
            || t.contains("@import 'tailwindcss'")
        {
            e.tailwind = true;
        }
        // --nombre: valor;
        for linea in t.split([';', '{', '}']) {
            let l = linea.trim();
            if let Some(rest) = l.strip_prefix("--") {
                if let Some((n, v)) = rest.split_once(':') {
                    let (n, v) = (n.trim(), v.trim());
                    if parece_color(v)
                        && e.colores.len() < 48
                        && !e.colores.iter().any(|(x, _)| x == n)
                    {
                        e.colores.push((n.to_string(), v.to_string()));
                    }
                }
            }
            if let Some((_, v)) = l.split_once("font-family:") {
                for f in v.split(',') {
                    let f = f.trim().trim_matches(['"', '\'']).trim();
                    // `var(--x, inherit)` y demás: lo que trae paréntesis no es una familia
                    if f.contains('(') || f.contains(')') || f.contains('!') {
                        continue;
                    }
                    if !f.is_empty()
                        && !SISTEMA.contains(&f.to_lowercase().as_str())
                        && !e.fuentes.iter().any(|x| x == f)
                        && e.fuentes.len() < 4
                    {
                        e.fuentes.push(f.to_string());
                    }
                }
            }
        }
        // clases que define (selectores `.nombre` al empezar una regla)
        if e.clases.len() < 200 {
            for regla in t.split('}') {
                let sel = regla.split('{').next().unwrap_or("");
                for parte in sel.split(',') {
                    let parte = parte.trim();
                    if let Some(r) = parte.strip_prefix('.') {
                        let n: String = r
                            .chars()
                            .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
                            .collect();
                        if n.len() > 1 && !e.clases.contains(&n) && e.clases.len() < 200 {
                            e.clases.push(n);
                        }
                    }
                }
            }
        }
        e.hojas
            .push(h.strip_prefix(root).map(Path::to_path_buf).unwrap_or(h));
    }
    // primero las de ESTRUCTURA (contenedor, cabecera, tarjeta, botón…), que son las que una
    // página nueva necesita; luego el resto en su orden
    let prio = |c: &str| {
        let c = c.to_lowercase();
        [
            "container",
            "contenedor",
            "wrapper",
            "layout",
            "main",
            "page",
            "header",
            "nav",
            "footer",
            "card",
            "panel",
            "section",
            "title",
            "titulo",
            "btn",
            "button",
            "boton",
            "grid",
            "list",
            "badge",
            "text-",
        ]
        .iter()
        .position(|k| c.contains(k))
        .unwrap_or(99)
    };
    e.clases.sort_by_key(|c| prio(c));
    e.clases.truncate(40);
    // una página existente: cómo enlaza sus hojas (lo que una página nueva debe copiar)
    for nombre in [
        "index.html",
        "index.php",
        "index.htm",
        "public/index.html",
        "src/index.html",
    ] {
        if let Ok(t) = std::fs::read_to_string(root.join(nombre)) {
            let links: Vec<String> = t
                .lines()
                .map(str::trim)
                .filter(|l| {
                    l.starts_with("<link") && (l.contains("stylesheet") || l.contains("fonts."))
                })
                .take(6)
                .map(String::from)
                .collect();
            if !links.is_empty() {
                e.cabeza = Some((nombre.to_string(), links));
                break;
            }
        }
    }
    // primero los colores de identidad (marca, acento, fondo, texto), luego el resto
    let peso = |n: &str| {
        let n = n.to_lowercase();
        if [
            "primary", "primario", "brand", "marca", "accent", "acento", "main",
        ]
        .iter()
        .any(|k| n.contains(k))
        {
            0
        } else if ["bg", "background", "fondo", "surface", "superficie"]
            .iter()
            .any(|k| n.contains(k))
        {
            1
        } else if ["text", "texto", "ink", "fg", "foreground"]
            .iter()
            .any(|k| n.contains(k))
        {
            2
        } else {
            3
        }
    };
    e.colores.sort_by_key(|(n, _)| peso(n));
    e.colores.truncate(16);
    e
}

/// DM6.5: las líneas que la edición AÑADE y que tapan un síntoma en vez de arreglarlo:
/// `overflow(-x): hidden` dentro de una regla de `html`, `body` o `:root` (esconde el desborde de
/// toda la página) y cualquier `!important` nuevo. En una tarjeta, `overflow: hidden` es legítimo.
/// `previo`: el archivo ANTES del punto editado (en un `edit`, el selector de la regla suele estar
/// arriba, fuera del fragmento).
pub fn sintomas_tapados(viejo: &str, nuevo: &str, previo: &str) -> Vec<String> {
    let antes: std::collections::HashSet<&str> = viejo.lines().map(str::trim).collect();
    let lineas: Vec<&str> = nuevo.lines().collect();
    let mut v = Vec::new();
    for (i, l) in lineas.iter().enumerate() {
        let t = l.trim();
        if t.is_empty() || antes.contains(t) {
            continue;
        }
        let compacto: String = t.to_lowercase().chars().filter(|c| !c.is_whitespace()).collect();
        let esconde = compacto.contains("overflow:hidden") || compacto.contains("overflow-x:hidden") || compacto.contains("overflow:clip") || compacto.contains("overflow-x:clip");
        if esconde {
            // el selector de la regla en la que cae: la última línea con «{» hasta aquí (o la misma)
            let selector = lineas[..=i]
                .iter()
                .rev()
                .copied()
                .chain(previo.lines().rev())
                .find(|x| x.contains('{'))
                .map(|x| x.split('{').next().unwrap_or("").to_lowercase())
                .unwrap_or_default();
            let global = selector
                .split(|c: char| c == ',' || c.is_whitespace())
                .any(|s| matches!(s.trim(), "html" | "body" | ":root"));
            if global {
                v.push(t.to_string());
                continue;
            }
        }
        if compacto.contains("!important") {
            v.push(t.to_string());
        }
    }
    v
}

/// Clases que usa un HTML y que NO define ninguna hoja del proyecto (medido, D10: el 9B inventó
/// `.step`, `.step-num`… y los números de paso salieron como texto suelto).
pub fn clases_inexistentes(root: &Path, html: &str) -> Vec<String> {
    let mut definidas = std::collections::HashSet::new();
    for h in hojas(root) {
        if let Ok(t) = std::fs::read_to_string(&h) {
            for (i, _) in t.match_indices('.') {
                let n: String = t[i + 1..]
                    .chars()
                    .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
                    .collect();
                if !n.is_empty() {
                    definidas.insert(n);
                }
            }
        }
    }
    // el <style> de la propia página también define
    for (i, _) in html.match_indices('.') {
        let n: String = html[i + 1..]
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
            .collect();
        if !n.is_empty() {
            definidas.insert(n);
        }
    }
    let mut faltan = Vec::new();
    // `class="…"`, `class='…'` y el `className="…"` de JSX/TSX (DM6.2); `className={…}` es código: no
    for (marca, cierre) in [("class=\"", '"'), ("class='", '\''), ("className=\"", '"'), ("className='", '\'')] {
        for (i, _) in html.match_indices(marca) {
            // «class="» también aparece dentro de «className="»: ése lo cuenta su propia marca
            if marca.starts_with("class=") && html[..i].ends_with("Name") {
                continue;
            }
            let resto = &html[i + marca.len()..];
            let valor = &resto[..resto.find(cierre).unwrap_or(0)];
            for c in valor.split_whitespace() {
                if !definidas.contains(c) && !faltan.iter().any(|x| x == c) && !c.contains('{') {
                    faltan.push(c.to_string());
                }
            }
        }
    }
    faltan
}

impl Estilo {
    /// ¿Hay un diseño propio que respetar? (GOTV no impone el suyo encima)
    pub fn propio(&self) -> bool {
        !self.de_gotv && (self.tailwind || self.colores.len() >= 2 || !self.fuentes.is_empty())
    }

    /// D11: la orden pide REDISEÑAR (no añadir algo con el estilo de la casa): el diseño actual NO
    /// se conserva, el contenido SÍ. Medido antes del examen: con la nota de «adopta su diseño», un
    /// rediseño explícito se habría hecho con el estilo viejo y sin `pagina`.
    pub fn nota_rediseno(&self) -> Option<String> {
        if !self.propio() {
            return None;
        }
        Some(crate::i18n::t(
            "[El usuario pide REDISEÑAR: el diseño actual NO se conserva; su CONTENIDO sí, TODO]\n- Lee la página entera (read_file) y pasa CADA dato a una ficha: textos, servicios, precios, horarios, dirección, teléfono, testimonios, enlaces. No inventes datos que no estén.\n- Crea la página nueva con `pagina` en la MISMA carpeta: GOTV guarda la original como index-anterior.html y la nueva queda en index.html (no muevas ni copies nada).\n- La revisión de entrega va sola; si pasa, termina.",
            "[The user asks to REDESIGN: the current design is NOT kept; its CONTENT is, ALL of it]\n- Read the whole page (read_file) and move EVERY datum into a spec: texts, services, prices, hours, address, phone, testimonials, links. Do not invent data that is not there.\n- Create the new page with `pagina` in the SAME folder: GOTV keeps the original as index-anterior.html and the new one becomes index.html (do not move or copy anything).\n- The delivery review runs by itself; if it passes, finish.",
        ).to_string())
    }

    /// Nota para el modelo (sólo si hay un estilo propio).
    pub fn nota(&self) -> Option<String> {
        if !self.propio() {
            return None;
        }
        let mut s = crate::i18n::t(
            "[Este proyecto YA tiene su diseño — adóptalo, no impongas otro]",
            "[This project ALREADY has its design — adopt it, do not impose another]",
        )
        .to_string();
        if !self.colores.is_empty() {
            s.push_str(&crate::tr!(
                "\n- Colores (variables CSS): {}",
                "\n- Colours (CSS variables): {}",
                self.colores
                    .iter()
                    .take(8)
                    .map(|(n, v)| format!("--{n}: {v}"))
                    .collect::<Vec<_>>()
                    .join(" · ")
            ));
        }
        if !self.fuentes.is_empty() {
            s.push_str(&crate::tr!(
                "\n- Tipografía: {}",
                "\n- Typography: {}",
                self.fuentes.join(", ")
            ));
        }
        if self.tailwind {
            s.push_str(crate::i18n::t(
                "\n- Usa Tailwind: estiliza con sus clases y su configuración, no con CSS suelto.",
                "\n- Uses Tailwind: style with its classes and config, not loose CSS.",
            ));
        }
        if !self.hojas.is_empty() {
            s.push_str(&crate::tr!(
                "\n- Hojas: {}",
                "\n- Stylesheets: {}",
                self.hojas
                    .iter()
                    .take(5)
                    .map(|h| h.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        if !self.clases.is_empty() {
            s.push_str(&crate::tr!(
                "\n- Clases que define su hoja (úsalas tal cual): {}",
                "\n- Classes its stylesheet defines (use them as is): {}",
                self.clases
                    .iter()
                    .map(|c| format!(".{c}"))
                    .collect::<Vec<_>>()
                    .join(" ")
            ));
        }
        if let Some((f, links)) = &self.cabeza {
            s.push_str(&crate::tr!(
                "\n- Así enlaza sus estilos {f} (copia estas líneas en el <head> de la página nueva):\n  {}",
                "\n- This is how {f} links its styles (copy these lines into the new page's <head>):\n  {}",
                links.join("\n  ")
            ));
        }
        s.push_str(crate::i18n::t(
            "\n- NO leas la hoja de estilos entera: lo esencial (colores y fuentes) ya está arriba, medido. Para la estructura, lee sólo el <head> y la cabecera de UNA página existente (read_file con limit).\n- Aquí NO uses `pagina` (impone el estilo de GOTV). Escribe tú el HTML con `write_file`: enlaza SU hoja (<link rel=\"stylesheet\" href=\"…\">), copia la estructura de una página existente (cabecera, contenedor) y usa SUS clases y variables. No reescribas su CSS: si falta algo, agrégalo al final de su hoja con sus variables.\n- No toques los archivos que no hacen falta; al terminar, abre la página nueva con `browser` y compárala con una existente.",
            "\n- Do NOT read the whole stylesheet: the essentials (colours and fonts) are above, measured. For structure, read only the <head> and header of ONE existing page (read_file with limit).\n- Do NOT use `pagina` here (it imposes GOTV's style). Write the HTML yourself with `write_file`: link THEIR stylesheet (<link rel=\"stylesheet\" href=\"…\">), copy the structure of an existing page (header, container) and use THEIR classes and variables. Do not rewrite their CSS: if something is missing, add it at the end of their sheet using their variables.\n- Do not touch files that do not need it; when done, open the new page with `browser` and compare it with an existing one.",
        ));
        Some(s)
    }
}

/// ¿La orden pide rediseñar lo que hay (y no añadir algo con su estilo)?
pub fn pide_rediseno(orden: &str) -> bool {
    // sin tildes: «Rediséñala» (imperativo con tilde) no casaba con «rediseñ» — medido en D11
    let o: String = orden
        .to_lowercase()
        .chars()
        .map(|c| match c {
            'á' | 'à' | 'ä' => 'a',
            'é' | 'è' | 'ë' => 'e',
            'í' | 'ì' | 'ï' => 'i',
            'ó' | 'ò' | 'ö' => 'o',
            'ú' | 'ù' | 'ü' => 'u',
            'ñ' => 'n',
            c => c,
        })
        .collect();
    ["redisen", "redesign", "nuevo diseno", "cambia el diseno", "cambiar el diseno", "moderniz", "renueva el diseno", "new design"] // juez-idioma: datos (palabras de una orden)
        .iter()
        .any(|k| o.contains(k))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nombra_las_clases_que_no_existen() {
        let d = crate::CarpetaPrueba::sin_crear("gotv-clases");
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(
            d.join("style.css"),
            ".glass-card{padding:1rem} .btn-primary, .glass-header h1{color:red}",
        )
        .unwrap();
        let html = "<style>.propia{x:1}</style><div class=\"glass-card step\"><span class=\"step-num propia\"></span><h1 class=\"glass-header\"></h1></div>";
        assert_eq!(
            clases_inexistentes(&d, html),
            vec!["step".to_string(), "step-num".to_string()]
        );
        // DM6.2: JSX/TSX (`className`) también; `className={…}` es código y no cuenta
        let jsx = "<div className=\"glass-card tarjeta-nueva\"><p className={estilo}>x</p></div>";
        assert_eq!(clases_inexistentes(&d, jsx), vec!["tarjeta-nueva".to_string()]);
        let _ = std::fs::remove_dir_all(&d);
    }

    /// DM6.5: esconder el desborde de la página o `!important` nuevos; en una tarjeta, no.
    #[test]
    fn sintomas_tapados_y_legitimos() {
        let viejo = "body {\n  margin: 0;\n}\n";
        let nuevo = "body {\n  margin: 0;\n  overflow-x: hidden;\n}\n";
        assert_eq!(sintomas_tapados(viejo, nuevo, ""), vec!["overflow-x: hidden;".to_string()]);
        // un `edit` que sólo trae la línea: el selector está ARRIBA en el archivo
        assert_eq!(sintomas_tapados("  margin: 0;", "  margin: 0;\n  overflow: hidden;", "html, body {\n").len(), 1);
        assert!(sintomas_tapados(".tarjeta {\n}", ".tarjeta {\n  overflow: hidden;\n}", "").is_empty(), "una tarjeta puede recortar");
        assert_eq!(sintomas_tapados(".a { color: red; }", ".a { color: red !important; }", "").len(), 1);
        // lo que ya estaba no cuenta
        assert!(sintomas_tapados("body { overflow-x: hidden; }", "body { overflow-x: hidden; }\n.b {}", "").is_empty());
    }

    #[test]
    fn un_rediseno_no_adopta_el_diseno_viejo() {
        assert!(pide_rediseno("Rediseña esta página para que se vea moderna"));
        assert!(pide_rediseno("Esta carpeta tiene la página de El Pedal. Rediséñala para que se vea profesional"));
        assert!(pide_rediseno("REDISEÑO completo de la web"));
        assert!(pide_rediseno("haz un nuevo diseño de la web"));
        assert!(pide_rediseno("Redesign the landing"));
        assert!(!pide_rediseno("agrega una página de ayuda con el mismo estilo"));
    }

    #[test]
    fn lee_el_diseno_de_un_proyecto_ajeno() {
        let d = crate::CarpetaPrueba::sin_crear("gotv-estilo");
        std::fs::create_dir_all(d.join("src/styles")).unwrap();
        std::fs::create_dir_all(d.join("node_modules/x")).unwrap();
        std::fs::write(d.join("src/styles/base.css"), ":root { --code-bg: #111; --primary: #0f766e; --bg: #fafaf9; --radius: 8px; --text: rgb(20 20 20); }\nbody { font-family: 'Manrope', system-ui, sans-serif; }\ncode { font-family: var(--mono, ui-monospace), monospace; }").unwrap();
        std::fs::create_dir_all(d.join("_backup_ui_2026")).unwrap();
        std::fs::write(
            d.join("_backup_ui_2026/old.css"),
            ":root{--primary:#ff0000}",
        )
        .unwrap();
        std::fs::write(d.join("node_modules/x/x.css"), ":root{--no:#000}").unwrap();
        let e = leer(&d);
        assert_eq!(
            e.colores,
            vec![
                ("primary".into(), "#0f766e".into()),
                ("code-bg".into(), "#111".into()),
                ("bg".into(), "#fafaf9".into()),
                ("text".into(), "rgb(20 20 20)".into())
            ],
            "marca primero; el respaldo no se lee"
        );
        assert_eq!(e.fuentes, vec!["Manrope".to_string()]);
        assert!(e.propio());
        let n = e.nota().unwrap();
        assert!(
            n.contains("--primary: #0f766e")
                && n.contains("Manrope")
                && n.contains("NO uses `pagina`"),
            "{n}"
        );
        assert!(e.clases.is_empty(), "esa hoja no define clases");
        // un proyecto hecho por GOTV no se trata como ajeno (se rehace con `pagina`)
        std::fs::create_dir_all(d.join(".gotv")).unwrap();
        std::fs::write(d.join(".gotv/pagina.json"), "{}").unwrap();
        assert!(leer(&d).nota().is_none());
        let _ = std::fs::remove_dir_all(&d);
        // sin estilo propio: nada
        let v = crate::CarpetaPrueba::sin_crear("gotv-estilo-vacio");
        std::fs::create_dir_all(&v).unwrap();
        assert!(leer(&v).nota().is_none());
        let _ = std::fs::remove_dir_all(&v);
    }
}
