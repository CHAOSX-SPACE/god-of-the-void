//! Juez de idioma (P8, F0.1 de PLAN-CUERPO): un texto con letras del español que vive FUERA de un par
//! `tr!("es", "en")` / `i18n::t("es", "en")` es una falla. Mira el código servido de `crates/` (no las
//! pruebas tras `#[cfg(test)]` ni los comentarios). Nació como puerto exacto de
//! `archivo/herramientas/juez-idioma.py` (misma lista de palabras, mismas excepciones, mismas marcas
//! `juez-idioma:`; medido: las mismas 874 líneas una por una) con dos diferencias DELIBERADAS: ya no
//! se deja engañar por `include_str!(` (que contiene `tr!(`) y no mira `examples/` (bancos del
//! desarrollador que no viajan en la app). Reconoce también `bail_tr!` y `anyhow_tr!`.
//!
//! Lección de la falla #53: dos idiomas mantenidos a mano necesitan un juez, no buena voluntad.
//!
//! Estricto desde el 2026-11-12: las 874 deudas que arrastraba el repo se pagaron (traducidas con su
//! par, declaradas como datos con su razón, o movidas a recursos en los dos idiomas). Un texto
//! suelto nuevo rompe `cargo test`.

use std::path::{Path, PathBuf};

const PALABRAS: &[&str] = &[
    "el", "la", "los", "las", "del", "que", "sin", "con", "para", "una", "uno", "archivo", "modelo", "motor", "paso",
    "contexto", "memoria",
];
/// Texto que no se traduce a propósito: nombres, rutas, lo que va al modelo como dato.
const PERMITIDO: &[&str] = &["Dios del Vacío", "bloque de herramienta vacío", "sudo", "/no_think"];

fn es_espanol(lit: &str) -> bool {
    if lit.chars().any(|c| "áéíóúñ¿¡ÁÉÍÓÚÑ".contains(c)) {
        return true;
    }
    lit.split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .any(|w| PALABRAS.iter().any(|p| w.eq_ignore_ascii_case(p)))
}

/// Los literales `"…"` de una línea, como `re.findall(r'"((?:[^"\\]|\\.)*)"', …)`.
fn literales(s: &str) -> Vec<String> {
    let c: Vec<char> = s.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < c.len() {
        if c[i] != '"' {
            i += 1;
            continue;
        }
        let mut j = i + 1;
        let mut fin = None;
        while j < c.len() {
            match c[j] {
                '"' => {
                    fin = Some(j);
                    break;
                }
                '\\' if j + 1 < c.len() => j += 2,
                '\\' => break,
                _ => j += 1,
            }
        }
        match fin {
            Some(f) => {
                out.push(c[i + 1..f].iter().collect());
                i = f + 1;
            }
            None => i += 1,
        }
    }
    out
}

/// La línea abre un par: `tr!(` (también `bail_tr!(` y `anyhow_tr!(`) sin una letra pegada delante —
/// el guion de Python aceptaba `include_str!(`, que CONTIENE `tr!(`, y dejaba sin juzgar esa línea
/// y las tres siguientes — o `i18n::t`.
fn tiene_par(s: &str) -> bool {
    s.contains("i18n::t") || s.match_indices("tr!(").any(|(i, _)| !s[..i].chars().next_back().is_some_and(char::is_alphanumeric))
}

fn sangria(l: &str) -> usize {
    l.len() - l.trim_start().len()
}

fn juzgar(src: &str, nombre: &str, fallas: &mut Vec<String>) {
    let src = src.split("#[cfg(test)]").next().unwrap_or("");
    let (mut pendiente, mut bloque, mut base) = (0usize, false, 0usize);
    for (n, linea) in src.lines().enumerate() {
        let s = linea.trim();
        if s.contains("juez-idioma:") {
            // excepción DECLARADA: la línea, o el bloque si dice «bloque»
            if s.contains("bloque") {
                bloque = true;
                base = sangria(linea);
            }
            if s.starts_with("//") {
                pendiente = pendiente.max(2);
            }
            continue;
        }
        if bloque {
            let ind = sangria(linea);
            if (!s.is_empty() && ind < base) || (s.starts_with(']') && ind <= base && s.ends_with(',')) {
                bloque = false;
            } else {
                continue;
            }
        }
        if s.starts_with("//") {
            continue;
        }
        if tiene_par(s) {
            pendiente = 4; // el par puede seguir en las líneas siguientes
            continue;
        }
        let codigo = s.split("//").next().unwrap_or("");
        for lit in literales(codigo) {
            if es_espanol(&lit) && !PERMITIDO.iter().any(|p| lit.contains(p)) && pendiente == 0 {
                fallas.push(format!("{nombre}:{}: {}", n + 1, lit.chars().take(90).collect::<String>()));
            }
        }
        pendiente = pendiente.saturating_sub(1);
    }
}

fn rs(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            // `examples/` son bancos y talleres del desarrollador: no viajan en la app (gotv-empaquetar
            // copia sólo gotv, gotv-gui y los ayudantes) y sus órdenes en español SON el dato que miden.
            if p.file_name().is_some_and(|n| n == "examples") {
                continue;
            }
            rs(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

fn fallas_del_repo() -> Vec<String> {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_path_buf();
    let raiz = crates.parent().unwrap();
    let este = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/idioma.rs");
    let mut archivos = Vec::new();
    rs(&crates, &mut archivos);
    archivos.sort();
    let mut fallas = Vec::new();
    for f in archivos.iter().filter(|f| **f != este) {
        let Ok(src) = std::fs::read_to_string(f) else { continue };
        juzgar(&src, &f.strip_prefix(raiz).unwrap_or(f).display().to_string(), &mut fallas);
    }
    fallas
}

#[test]
fn ningun_texto_nuevo_en_un_solo_idioma() {
    let fallas = fallas_del_repo();
    for f in &fallas {
        eprintln!("✗ {f}");
    }
    assert!(
        fallas.is_empty(),
        "juez de idioma: {} texto(s) en español fuera de un par ES/EN: envuélvelos en tr!/i18n::t (o bail_tr!/anyhow_tr!), \
         o si son DATOS (palabras que se reconocen, contenido de un documento en español) márcalos con `// juez-idioma: datos (razón)`",
        fallas.len()
    );
}

#[test]
fn el_juez_ve_un_texto_suelto_y_respeta_los_pares() {
    let mut f = Vec::new();
    juzgar("fn a() {\n    let x = \"sólo español\";\n}\n", "a.rs", &mut f);
    assert_eq!(f, ["a.rs:2: sólo español"]);
    f.clear();
    juzgar("fn a() {\n    let x = tr!(\n        \"sólo español\",\n        \"english only\");\n}\n", "a.rs", &mut f);
    assert!(f.is_empty(), "un par no es falla: {f:?}");
    juzgar("// juez-idioma: dato del modelo\nlet x = \"para el modelo\";\n", "a.rs", &mut f);
    assert!(f.is_empty(), "excepción declarada: {f:?}");
    juzgar("let x = 1; // \"comentario con la palabra\"\n#[cfg(test)]\nlet y = \"prueba sin par\";\n", "a.rs", &mut f);
    assert!(f.is_empty(), "comentarios y pruebas no cuentan: {f:?}");
}
