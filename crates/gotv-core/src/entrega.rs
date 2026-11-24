//! Compuerta de entrega (Ola D9): una página del perfil Diseño no se da por terminada sin
//! revisión. Dos capas:
//!
//! - **sin navegador** (sobre la ficha y los archivos): metadatos para buscadores y redes,
//!   textos de relleno, enlaces sin destino, página corta, peso total.
//! - **con el navegador propio** (agent-browser, sesión aparte): desborde a 390/768/1440,
//!   contraste por axe Y por el medidor propio (texto contra CADA parada de color de cada
//!   capa: axe se rinde ante degradados, falla #2534) y accesibilidad grave.
//!
//! El informe vuelve al modelo con arreglos CONCRETOS (qué, dónde y cómo), no con «mejóralo».

use std::path::Path;

/// Frases genéricas que delatan texto de relleno (lo que un cliente no escribiría de su negocio).
const RELLENO: &[&str] = &[
    // juez-idioma: bloque — datos (relleno que se detecta, en los dos idiomas)
    "lorem ipsum",
    "dolor sit amet",
    "texto de ejemplo",
    "aquí va",
    "aqui va",
    "placeholder",
    "tu texto",
    "título aquí",
    "titulo aqui",
    "solución innovadora",
    "soluciones innovadoras",
    "líder en el mercado",
    "lider en el mercado",
    "calidad y excelencia",
    "tu socio de confianza",
    "siguiente nivel",
    "llevamos tu negocio",
    "empresa líder",
    "a la vanguardia",
    "innovative solution",
    "next level",
    "world-class",
    "cutting-edge",
    "your trusted partner",
    "lorem",
];

/// JS que se evalúa en la página: desborde horizontal y textos con contraste insuficiente
/// contra cualquier capa (normaliza colores pintándolos en un lienzo de 1 px).
const MEDIR: &str = r#"(() => {
  const cv = document.createElement('canvas'); cv.width = cv.height = 1;
  const cx = cv.getContext('2d', { willReadFrequently: true });
  const rgba = (c) => { cx.clearRect(0, 0, 1, 1); cx.fillStyle = 'rgba(0,0,0,0)'; cx.fillStyle = c; cx.fillRect(0, 0, 1, 1); const d = cx.getImageData(0, 0, 1, 1).data; return [d[0], d[1], d[2], d[3] / 255]; };
  const sobre = (a, b) => [0, 1, 2].map(i => a[i] * a[3] + b[i] * (1 - a[3]));
  const lum = (c) => { const f = (v) => { v /= 255; return v <= 0.04045 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4; }; return 0.2126 * f(c[0]) + 0.7152 * f(c[1]) + 0.0722 * f(c[2]); };
  const ratio = (a, b) => { const x = lum(a), y = lum(b); return (Math.max(x, y) + 0.05) / (Math.min(x, y) + 0.05); };
  const PARADA = /(rgba?\([^)]*\)|oklab\([^)]*\)|oklch\([^)]*\)|lab\([^)]*\)|lch\([^)]*\)|color\([^)]*\)|hsla?\([^)]*\)|#[0-9a-fA-F]{3,8}\b)/g;
  const fondos = (e) => { const cad = []; for (let n = e; n && n.nodeType === 1; n = n.parentElement) cad.unshift(n);
    let cand = [[255, 255, 255]];
    for (const n of cad) { const s = getComputedStyle(n); const bc = rgba(s.backgroundColor); if (bc[3] > 0) cand = cand.map(c => sobre(bc, c));
      if (/gradient/.test(s.backgroundImage)) { const ps = (s.backgroundImage.match(PARADA) || []).map(rgba).filter(p => p[3] > 0);
        if (ps.length) { const nv = []; for (const c of cand) for (const p of ps) nv.push(sobre(p, c)); cand = nv.slice(0, 24); } } }
    return cand; };
  const vis = (e) => { const r = e.getBoundingClientRect(); const s = getComputedStyle(e); return r.width > 0 && r.height > 0 && s.visibility !== 'hidden' && s.display !== 'none'; };
  const malos = [];
  for (const e of document.querySelectorAll('body *')) {
    if (!vis(e) || e.closest('svg,[aria-hidden="true"]')) continue;
    if (![...e.childNodes].some(n => n.nodeType === 3 && n.textContent.trim().length > 1)) continue;
    const s = getComputedStyle(e); const fg = rgba(s.color); if (fg[3] === 0) continue;
    const grande = parseFloat(s.fontSize) >= 24 || (parseFloat(s.fontSize) >= 18.66 && +s.fontWeight >= 700);
    const min = grande ? 3 : 4.5;
    const peor = Math.min(...fondos(e).map(b => ratio(sobre(fg, b), b)));
    if (peor < min) malos.push({ t: e.textContent.trim().slice(0, 40), r: Math.round(peor * 100) / 100, m: min, cls: (e.className || '').toString().split(' ')[0] || e.tagName.toLowerCase() });
  }
  const anchos = [...document.querySelectorAll('body *')].filter(e => e.getBoundingClientRect().right > window.innerWidth + 1).slice(0, 3).map(e => (e.className || '').toString().split(' ')[0] || e.tagName.toLowerCase());
  return JSON.stringify({ desborde: document.documentElement.scrollWidth - window.innerWidth, anchos, malos: malos.slice(0, 6) });
})()"#;

/// D11 (medido): un rediseño del 9B se veía bien y pasaba la higiene, pero perdió los 5 precios de
/// los servicios. Los DATOS DUROS de la página original (precios, horas, años, teléfonos, correos,
/// números de dirección) se buscan en la nueva; los que faltan se nombran.
/// El texto (entre etiquetas) de la página original donde vive un dato: «Mantenimiento general:
/// $45.000». Con él, el modelo sabe QUÉ poner, no sólo el número suelto.
pub fn linea_de(vieja_html: &str, dato: &str) -> String {
    let digitos = |x: &str| x.chars().filter(|c| c.is_ascii_digit()).collect::<String>();
    let es_tel = dato.contains(' ') && dato.chars().all(|c| c.is_ascii_digit() || c == ' ');
    vieja_html
        .split(['<', '>'])
        .enumerate()
        .filter(|(i, _)| i % 2 == 0) // los trozos de TEXTO (lo de dentro de <…> son etiquetas)
        .map(|(_, t)| t.split_whitespace().collect::<Vec<_>>().join(" "))
        .find(|t| if es_tel { digitos(t).contains(&dato.replace(' ', "")) } else { t.to_lowercase().contains(&dato.to_lowercase()) })
        .map(|t| t.chars().take(90).collect())
        .unwrap_or_else(|| dato.to_string())
}

pub fn datos_perdidos(vieja_html: &str, nueva_html: &str) -> Vec<String> {
    let texto = |h: &str| {
        let mut o = String::with_capacity(h.len());
        let mut dentro = false;
        for c in h.chars() {
            match c {
                '<' => dentro = true,
                '>' => {
                    dentro = false;
                    o.push(' ');
                }
                c if !dentro => o.push(c),
                _ => {}
            }
        }
        o.replace("&nbsp;", " ").replace("&amp;", "&").replace("&#36;", "$")
    };
    let vieja = texto(vieja_html);
    let nueva = texto(nueva_html).to_lowercase();
    let nueva_plana = nueva.split_whitespace().collect::<Vec<_>>().join(" ");
    let nueva_digitos: String = nueva.chars().filter(|c| c.is_ascii_digit()).collect();
    let limpia = |t: &str| t.trim_matches(|c: char| ",;:()\"'«»!?¿¡*".contains(c) || (c == '.' && !t.contains('@'))).to_string(); // juez-idioma: datos (signos)
    let palabras: Vec<String> = vieja.split_whitespace().map(limpia).filter(|t| !t.is_empty()).collect();
    let mut datos: Vec<String> = Vec::new();
    let mut i = 0;
    while i < palabras.len() {
        let t = &palabras[i];
        let digitos = |x: &str| !x.is_empty() && x.chars().all(|c| c.is_ascii_digit());
        // teléfono: 3+ grupos de dígitos seguidos con 7–12 dígitos en total
        let mut j = i;
        while j < palabras.len() && digitos(&palabras[j]) {
            j += 1;
        }
        let total: usize = palabras[i..j].iter().map(|x| x.len()).sum();
        if j - i >= 3 && (7..=12).contains(&total) {
            datos.push(palabras[i..j].join(" "));
            i = j;
            continue;
        }
        let precio = t.strip_prefix('$').map(|x| x.to_string()).or_else(|| (i > 0 && palabras[i - 1] == "$").then(|| t.clone()));
        if let Some(p) = precio.filter(|p| p.chars().any(|c| c.is_ascii_digit())) {
            datos.push(p);
        } else if t.contains('@') && t.contains('.') {
            datos.push(t.to_lowercase());
        } else {
            let hora = t.len() <= 5 && t.contains(':') && t.split(':').all(digitos);
            let anio = t.len() == 4 && digitos(t) && (t.starts_with("19") || t.starts_with("20")) && !(i > 0 && matches!(palabras[i - 1].as_str(), "©" | "(c)" | "copyright"));
            let direccion = t.contains('-') && t.split('-').count() == 2 && t.split('-').all(digitos);
            if hora || anio || direccion {
                datos.push(t.clone());
            }
        }
        i += 1;
    }
    let mut vistos = std::collections::HashSet::new();
    datos
        .into_iter()
        .filter(|d| vistos.insert(d.clone()))
        .filter(|d| {
            let es_tel = d.contains(' ') && d.chars().all(|c| c.is_ascii_digit() || c == ' ');
            if es_tel {
                !nueva_digitos.contains(&d.replace(' ', ""))
            } else {
                !nueva_plana.contains(&d.to_lowercase())
            }
        })
        .collect()
}


#[derive(Debug, Default)]
pub struct Revision {
    /// fallos que bloquean la entrega (desborde, contraste, a11y grave)
    pub fallos: Vec<String>,
    /// avisos de oficio (metadatos, relleno, enlaces…): se dicen, no bloquean
    pub avisos: Vec<String>,
    /// el navegador no estaba disponible: sólo se hizo la revisión estática
    pub sin_navegador: bool,
}

impl Revision {
    pub fn ok(&self) -> bool {
        self.fallos.is_empty()
    }

    pub fn informe(&self) -> String {
        let mut s = if self.ok() {
            crate::i18n::t("REVISIÓN DE ENTREGA: ✅ pasa (desborde 390/768/1440, contraste axe + propio, accesibilidad grave).", "DELIVERY REVIEW: ✅ passes (overflow 390/768/1440, contrast axe + own, serious accessibility).").to_string()
        } else {
            crate::tr!(
                "REVISIÓN DE ENTREGA: ❌ {} fallo(s) que debes arreglar antes de terminar:",
                "DELIVERY REVIEW: ❌ {} problem(s) you must fix before finishing:",
                self.fallos.len()
            )
        };
        for f in &self.fallos {
            s.push_str(&format!("\n- ❌ {f}"));
        }
        for a in &self.avisos {
            s.push_str(&format!("\n- ⚠ {a}"));
        }
        if self.sin_navegador {
            s.push_str(crate::i18n::t("\n- (sin navegador: sólo revisión de la ficha; ábrela con `browser` y corre [\"a11y\"])", "\n- (no browser: spec review only; open it with `browser` and run [\"a11y\"])"));
        }
        s
    }
}

/// Revisión estática de la ficha (no necesita navegador).
pub fn estatica(ficha: &serde_json::Value, bytes_totales: usize) -> Vec<String> {
    let mut av = Vec::new();
    let titulo = ficha["titulo"].as_str().unwrap_or("");
    let n = titulo.chars().count();
    if !(10..=65).contains(&n) {
        av.push(crate::tr!(
            "«titulo» tiene {n} caracteres: entre 10 y 65 se lee entero en buscadores y pestañas",
            "«titulo» has {n} characters: 10 to 65 reads whole in search engines and tabs"
        ));
    }
    let d = ficha["descripcion"].as_str().unwrap_or("").chars().count();
    if d == 0 {
        av.push(crate::i18n::t("falta «descripcion» (1 frase, 50–160 caracteres): es lo que muestran Google y las redes", "missing «descripcion» (1 sentence, 50–160 characters): it is what Google and social networks show").into());
    } else if !(50..=160).contains(&d) {
        av.push(crate::tr!(
            "«descripcion» tiene {d} caracteres: lo ideal es 50–160",
            "«descripcion» has {d} characters: ideal is 50–160"
        ));
    }
    // textos de relleno en CUALQUIER texto de la ficha
    let mut textos = Vec::new();
    fn junta(v: &serde_json::Value, out: &mut Vec<String>) {
        match v {
            serde_json::Value::String(s) => out.push(s.to_lowercase()),
            serde_json::Value::Array(a) => a.iter().for_each(|x| junta(x, out)),
            serde_json::Value::Object(o) => o.values().for_each(|x| junta(x, out)),
            _ => {}
        }
    }
    junta(ficha, &mut textos);
    let todo = textos.join(" \u{1}");
    let hallados: Vec<&str> = RELLENO
        .iter()
        .copied()
        .filter(|p| todo.contains(p))
        .collect();
    if !hallados.is_empty() {
        av.push(crate::tr!(
            "texto de relleno: {} — escribe algo concreto de ESTE negocio (qué vende, a quién, dónde, cuánto)",
            "filler text: {} — write something concrete about THIS business (what it sells, to whom, where, how much)",
            hallados.iter().map(|h| format!("«{h}»")).collect::<Vec<_>>().join(", ")
        ));
    }
    // enlaces a ninguna parte
    let mut sin_destino = 0;
    fn cuenta(v: &serde_json::Value, n: &mut usize) {
        match v {
            serde_json::Value::Object(o) => {
                if o.get("href")
                    .and_then(|h| h.as_str())
                    .is_some_and(|h| h.trim() == "#")
                {
                    *n += 1;
                }
                o.values().for_each(|x| cuenta(x, n));
            }
            serde_json::Value::Array(a) => a.iter().for_each(|x| cuenta(x, n)),
            _ => {}
        }
    }
    cuenta(ficha, &mut sin_destino);
    if sin_destino >= 3 {
        av.push(crate::tr!(
            "{sin_destino} enlaces o botones van a «#» (a ninguna parte): apúntalos a una sección (#precios, #contacto) o a una página real",
            "{sin_destino} links or buttons go to «#» (nowhere): point them to a section (#precios, #contacto) or a real page"
        ));
    }
    let contenido = ficha["secciones"]
        .as_array()
        .map(|s| {
            s.iter()
                .filter(|x| !matches!(x["tipo"].as_str(), Some("navegacion" | "pie" | "heroe")))
                .count()
        })
        .unwrap_or(0);
    if contenido < 3 {
        av.push(crate::tr!(
            "la página tiene {contenido} sección(es) de contenido: una landing profesional suele llevar 4–7 (qué ofrece, cómo funciona, pruebas sociales, precios, preguntas, contacto)",
            "the page has {contenido} content section(s): a professional landing usually has 4–7 (offer, how it works, social proof, pricing, FAQ, contact)"
        ));
    }
    if bytes_totales > 1_500_000 {
        av.push(crate::tr!(
            "la página pesa {:.1} MB: por encima de 1,5 MB carga lento en móvil",
            "the page weighs {:.1} MB: above 1.5 MB it loads slowly on mobile",
            bytes_totales as f64 / 1e6
        ));
    }
    av
}

/// Revisión completa: estática + navegador (si está). `owner` separa la sesión del navegador.
pub async fn revisar(
    owner: &str,
    root: &Path,
    index: &Path,
    ficha: &serde_json::Value,
    bytes_totales: usize,
) -> Revision {
    // En las pruebas del agente el navegador real queda FUERA (medido 2026-11-10: con la suite en
    // paralelo, abrirlo y medir tres tamaños tardaba 11–14 s y a veces cambiaba el veredicto → la
    // prueba d6 fallaba 1 de cada 5). La prueba de la compuerta lo pide explícito con `revisar_con`.
    revisar_con(owner, root, index, ficha, bytes_totales, !cfg!(test)).await
}

/// DM6.1 (2026-11-13): una página editada A MANO (sin `pagina`, sin ficha) pasa por la misma compuerta
/// del navegador: desborde a 390/768/1440, contraste (axe + propio) y accesibilidad grave. Antes sólo
/// la medía `pagina`; arreglar a mano el CSS de un sitio existente nunca daba un «rojo» de diseño.
pub async fn revisar_a_mano(owner: &str, root: &Path, index: &Path) -> Revision {
    revisar_con(owner, root, index, &serde_json::Value::Null, 0, !cfg!(test)).await
}

/// DM6.1: las páginas que tocó a mano el turno, para medirlas al cerrar: cada `.html`/`.htm` editado
/// y, por cada `.css` editado, las páginas del proyecto que la enlazan (primero `index.html`). Sólo
/// sitios estáticos que se abren como archivo: JSX/TSX, Vue o PHP necesitan su servidor (aún no).
pub fn paginas_tocadas(root: &Path, rutas: &[String]) -> Vec<std::path::PathBuf> {
    let mut v: Vec<std::path::PathBuf> = Vec::new();
    let mut pon = |p: std::path::PathBuf| {
        if p.is_file() && !v.contains(&p) {
            v.push(p);
        }
    };
    let htmls = || -> Vec<std::path::PathBuf> {
        let mut h = Vec::new();
        let mut pendientes = vec![(root.to_path_buf(), 0u32)];
        while let Some((d, nivel)) = pendientes.pop() {
            let Ok(it) = std::fs::read_dir(&d) else { continue };
            for e in it.flatten() {
                let p = e.path();
                let n = e.file_name().to_string_lossy().to_string();
                if p.is_dir() {
                    if nivel < 2 && !n.starts_with('.') && !matches!(n.as_str(), "node_modules" | "dist" | "build" | "vendor") {
                        pendientes.push((p, nivel + 1));
                    }
                } else if n.ends_with(".html") || n.ends_with(".htm") {
                    h.push(p);
                }
            }
        }
        h.sort_by_key(|p| (p.file_name().is_none_or(|n| n != "index.html"), p.clone()));
        h
    };
    for r in rutas {
        let p = crate::paths::resolve(root, r);
        match p.extension().and_then(|e| e.to_str()) {
            Some("html" | "htm") => pon(p),
            Some("css") => {
                let nombre = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                for h in htmls() {
                    if std::fs::read_to_string(&h).is_ok_and(|t| t.contains(&nombre)) {
                        pon(h);
                        break;
                    }
                }
            }
            _ => {}
        }
    }
    v
}

/// La compuerta, diciendo si se mide en el navegador real (`navegador`) o sólo la ficha.
pub async fn revisar_con(
    owner: &str,
    root: &Path,
    index: &Path,
    ficha: &serde_json::Value,
    bytes_totales: usize,
    navegador: bool,
) -> Revision {
    // una página hecha a mano no tiene ficha: sólo cuenta lo que mide el navegador
    let mut r = Revision {
        avisos: if ficha.is_null() { Vec::new() } else { estatica(ficha, bytes_totales) },
        ..Default::default()
    };
    if !navegador || crate::browser::binary().is_none() {
        r.sin_navegador = true;
        return r;
    }
    // `session_for` se queda con lo que va tras el ÚLTIMO «#»: con «{owner}#entrega» TODOS los
    // chats compartían `gotv-entrega` y dos revisiones a la vez se cerraban y reabrían el mismo
    // navegador (medido 2026-11-09: un daemon huérfano 27 min). Con «-entrega» es uno por chat.
    let sesion = format!("{owner}-entrega");
    let url = format!("file://{}", index.display());
    let b = |args: &[&str]| {
        let a: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        let s = sesion.clone();
        async move { crate::browser::run(&s, root, &a, None, 60).await }
    };
    if b(&["open", &url]).await.is_err() {
        let _ = b(&["close"]).await; // el daemon pudo nacer aunque `open` fallara
        r.sin_navegador = true;
        return r;
    }
    let mut contraste: std::collections::BTreeMap<String, String> = Default::default();
    for (w, h) in [(390, 844), (768, 1024), (1440, 900)] {
        let _ = b(&["set", "viewport", &w.to_string(), &h.to_string()]).await;
        let _ = b(&["wait", "300"]).await;
        let Ok(out) = b(&["eval", MEDIR]).await else {
            continue;
        };
        // eval devuelve el JSON como cadena JSON
        let v: serde_json::Value = serde_json::from_str(out.trim())
            .ok()
            .and_then(|x: serde_json::Value| {
                x.as_str()
                    .and_then(|s| serde_json::from_str(s).ok())
                    .or(Some(x))
            })
            .unwrap_or_default();
        let d = v["desborde"].as_i64().unwrap_or(0);
        if d > 1 {
            let quien: Vec<String> = v["anchos"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(|s| format!(".{s}")))
                        .collect()
                })
                .unwrap_or_default();
            r.fallos.push(crate::tr!(
                "a {w} px la página se sale {d} px a lo ancho ({}): quita anchos fijos y textos sin espacios; en un bloque libre usa max-width:100%",
                "at {w} px the page overflows {d} px horizontally ({}): remove fixed widths and unbroken text; in a libre block use max-width:100%",
                quien.join(", ")
            ));
        }
        for m in v["malos"].as_array().into_iter().flatten() {
            let t = m["t"].as_str().unwrap_or("").to_string();
            contraste.entry(t.clone()).or_insert_with(|| {
                crate::tr!(
                    "contraste {} < {} en «{t}» (.{}): sobre un fondo de color usa el texto que el tema garantiza (var(--texto), var(--sobre-acento)), no el acento",
                    "contrast {} < {} on «{t}» (.{}): on a coloured background use the text the theme guarantees (var(--texto), var(--sobre-acento)), not the accent",
                    m["r"],
                    m["m"],
                    m["cls"].as_str().unwrap_or("?")
                )
            });
        }
    }
    r.fallos.extend(contraste.into_values());
    if let Ok(out) = b(&["a11y", "--json"]).await {
        match serde_json::from_str::<serde_json::Value>(
            out.lines()
                .filter(|l| !l.starts_with('⚠') && !l.starts_with("[agent-browser]"))
                .collect::<Vec<_>>()
                .join("\n")
                .trim(),
        ) {
            Ok(v) => {
                for x in v["data"]["violations"].as_array().into_iter().flatten() {
                    if matches!(x["impact"].as_str(), Some("serious" | "critical")) {
                        let nodos = x["nodes"].as_array().map(|n| n.len()).unwrap_or(0);
                        r.fallos.push(crate::tr!(
                            "accesibilidad: {} ({} elemento(s)) — {}",
                            "accessibility: {} ({} element(s)) — {}",
                            x["id"].as_str().unwrap_or("?"),
                            nodos,
                            x["help"].as_str().unwrap_or("")
                        ));
                    }
                }
            }
            Err(_) => r.avisos.push(
                crate::i18n::t(
                    "no pude leer la auditoría axe; córrela con `browser` [\"a11y\"]",
                    "could not read the axe audit; run it with `browser` [\"a11y\"]",
                )
                .into(),
            ),
        }
    }
    let _ = b(&["close"]).await;
    r
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// DM6.1: qué páginas se miden al cerrar: el html editado y, por una hoja editada, la página
    /// que la enlaza (index.html primero); un componente JSX o un .py, ninguna.
    #[test]
    fn paginas_tocadas_a_mano() {
        let d = crate::CarpetaPrueba::nueva("gotv-paginas-tocadas");
        std::fs::create_dir_all(d.join("css")).unwrap();
        std::fs::write(d.join("css/sitio.css"), "body{}").unwrap();
        std::fs::write(d.join("contacto.html"), "<link href=\"css/sitio.css\">").unwrap();
        std::fs::write(d.join("index.html"), "<link href=\"css/sitio.css\">").unwrap();
        std::fs::write(d.join("otra.html"), "<p>sin hoja</p>").unwrap();
        let rutas = |r: &[&str]| paginas_tocadas(&d, &r.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        assert_eq!(rutas(&["css/sitio.css"]), vec![d.join("index.html")]);
        assert_eq!(rutas(&["otra.html", "css/sitio.css"]), vec![d.join("otra.html"), d.join("index.html")]);
        assert!(rutas(&["src/App.tsx", "main.py"]).is_empty());
        // sin ficha (a mano) no salen los avisos de la ficha, y en las pruebas no se abre navegador
        let r = tokio::runtime::Runtime::new().unwrap().block_on(revisar_a_mano("prueba", &d, &d.join("index.html")));
        assert!(r.ok() && r.avisos.is_empty(), "{}", r.informe());
    }

    #[test]
    fn la_revision_estatica_nombra_lo_concreto() {
        let mala = json!({"titulo": "X", "descripcion": "", "secciones": [
            {"tipo": "heroe", "titulo": "Soluciones innovadoras para tu negocio", "cta": {"texto": "Ir", "href": "#"}},
            {"tipo": "llamada", "titulo": "Lorem ipsum", "cta": {"texto": "a", "href": "#"}, "cta2": {"texto": "b", "href": "#"}}]});
        let av = estatica(&mala, 2_000_000).join("\n");
        for esperado in [
            "«titulo» tiene 1 caracteres",
            "falta «descripcion»",
            "«soluciones innovadoras»",
            "«lorem ipsum»",
            "3 enlaces o botones van a «#»",
            "1 sección(es) de contenido",
            "pesa 2.0 MB",
        ] {
            assert!(av.contains(esperado), "falta «{esperado}» en:\n{av}");
        }
        let buena = json!({"titulo": "Grano Alto — café de especialidad en Medellín", "descripcion": "Tostamos nuestro café cada semana y lo llevamos a tu casa por suscripción mensual.",
            "secciones": [{"tipo":"heroe","titulo":"Café de altura"},{"tipo":"caracteristicas"},{"tipo":"precios"},{"tipo":"faq"},{"tipo":"contacto","cta":{"href":"#contacto"}}]});
        assert!(
            estatica(&buena, 300_000).is_empty(),
            "{:?}",
            estatica(&buena, 300_000)
        );
    }

    /// Con el navegador REAL: una página buena pasa; un bloque libre con texto casi invisible
    /// y un ancho fijo que rompe el móvil se nombran como fallos.
    #[tokio::test]
    async fn la_compuerta_atrapa_contraste_y_desborde_reales() {
        if crate::browser::binary().is_none() {
            eprintln!("sin navegador: se salta");
            return;
        }
        let d = crate::CarpetaPrueba::sin_crear("gotv-entrega");
        let pinta = |sub: &str, extra: serde_json::Value| {
            let mut f =
                serde_json::to_value(gotv_diseno::pagina::ficha_muestra("tecnico")).unwrap();
            if !extra.is_null() {
                f["secciones"].as_array_mut().unwrap().insert(2, extra);
            }
            let ficha: gotv_diseno::pagina::Ficha = serde_json::from_value(f.clone()).unwrap();
            let p = gotv_diseno::pagina::Pagina::new(ficha).unwrap();
            gotv_diseno::pagina::escribir_proyecto(&d.join(sub), &p.archivos()).unwrap();
            (d.join(sub).join("index.html"), f)
        };
        let (buena, fb) = pinta("buena", serde_json::Value::Null);
        let r = revisar_con("prueba#entrega-buena", &d, &buena, &fb, 300_000, true).await;
        assert!(r.ok(), "la muestra debe pasar:\n{}", r.informe());
        let (mala, fm) = pinta(
            "mala",
            json!({"tipo": "libre", "html": "<p style=\"color:#e8e8ef\">Texto casi invisible del bloque libre</p><div style=\"width:900px;height:20px\"></div>"}),
        );
        let r = revisar_con("prueba#entrega-mala", &d, &mala, &fm, 300_000, true).await;
        let inf = r.informe();
        assert!(!r.ok(), "{inf}");
        assert!(
            inf.contains("Texto casi invisible"),
            "el contraste malo se nombra:\n{inf}"
        );
        assert!(
            inf.contains("a 390 px la página se sale"),
            "el desborde se nombra:\n{inf}"
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn el_informe_dice_si_pasa() {
        let r = Revision::default();
        assert!(r.ok() && r.informe().contains("✅"));
        let r = Revision {
            fallos: vec!["a 390 px se sale".into()],
            ..Default::default()
        };
        assert!(!r.ok() && r.informe().contains("❌ a 390 px"));
    }

    #[test]
    fn d11_un_rediseno_no_pierde_datos_duros() {
        let vieja = "<ul><li>Mantenimiento: $45.000</li><li>Alquiler por día: $ 30.000</li></ul><p>Desde 1998. Lunes a viernes de 8:00 a.m. a 6:00 p.m.</p>\
            <p>Calle 57 # 13-24 · Tel 310 555 2847 · taller@elpedal.co</p>";
        let completa = "<h1>Taller</h1><p>Mantenimiento <b>$45.000</b> · alquiler 30.000</p><p>Desde 1998, 8:00 a 6:00</p><p>Calle 57 #13-24, 310-555-2847, Taller@ElPedal.co</p>";
        assert!(datos_perdidos(vieja, completa).is_empty(), "{:?}", datos_perdidos(vieja, completa));
        let sin_precios = "<p>Desde 1998, 8:00 a 6:00</p><p>Calle 57 #13-24, 310 555 2847, taller@elpedal.co</p>";
        assert_eq!(datos_perdidos(vieja, sin_precios), vec!["45.000".to_string(), "30.000".to_string()]);
        assert_eq!(linea_de(vieja, "45.000"), "Mantenimiento: $45.000");
        assert_eq!(linea_de(vieja, "310 555 2847"), "Calle 57 # 13-24 · Tel 310 555 2847 · taller@elpedal.co");
        // un dato que sólo vive en un enlace NO se ve: cuenta como perdido; el © viejo no
        let en_enlace = "<p>$45.000 · 30.000 · 1998 · 8:00 · 6:00 · Calle 57 # 13-24</p><a href=\"tel:3105552847\">Teléfono</a><a href=\"mailto:taller@elpedal.co\">Correo</a>";
        assert_eq!(datos_perdidos(&format!("{vieja}<p>© 2009</p>"), en_enlace), vec!["310 555 2847".to_string(), "taller@elpedal.co".to_string()]);
    }

    /// Con archivos reales: `GOTV_VIEJA=… GOTV_NUEVA=… cargo test -p gotv-core datos_reales -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn datos_reales() {
        let (v, n) = (std::env::var("GOTV_VIEJA").unwrap(), std::env::var("GOTV_NUEVA").unwrap());
        println!("perdidos: {:?}", datos_perdidos(&std::fs::read_to_string(v).unwrap(), &std::fs::read_to_string(n).unwrap()));
    }
}

