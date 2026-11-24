//! F5 ML9 · PUENTES entre lenguajes: los contratos que ningún servidor de lenguaje ve, porque cada lado
//! vive en otro lenguaje. Cada puente saca con tree-sitter (`gotv-sintaxis`) los dos lados y dice dónde
//! dejaron de casar:
//! - [`tauri`]: `invoke("cmd", {args})` en JS/TS ↔ `#[tauri::command] fn cmd(args)` en Rust (y su registro
//!   en `generate_handler!`);
//! - [`http`]: `fetch`/`axios` a `/api/x` ↔ las rutas de axum, Express, FastAPI, Flask, Django, Laravel o
//!   `net/http`;
//! - [`entorno`]: las variables de entorno que lee el código ↔ `.env.example`;
//! - [`datos`]: las consultas SQL del código ↔ la foto del esquema real (F1);
//! - [`plantillas`]: las variables de una plantilla Jinja/Twig/Blade ↔ las que le pasa el controlador.
//!
//! Es heurístico: se dice «posible desajuste», nunca «error». Y sólo lo NUEVO: el mapa de desajustes se
//! guarda en `.gotv/contratos.gotv` (formato propio) antes de la primera edición ([`asegurar`]) y tras cada edición se
//! compara ([`tras_editar`]); lo que ya no casaba antes no se repite.

pub mod datos;
pub mod entorno;
pub mod http;
pub mod plantillas;
pub mod tauri;

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use tree_sitter::Node;

/// Un posible desajuste: `clave` estable (sin número de línea: una línea que se corre no lo vuelve
/// «nuevo») y el texto para el modelo (con `archivo:línea`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Desajuste {
    pub puente: String,
    pub clave: String,
    pub texto: String,
}

/// Un archivo de código del proyecto, leído.
pub(crate) struct Fuente {
    pub rel: String,
    pub texto: String,
    pub lenguaje: gotv_sintaxis::Lenguaje,
}

impl Fuente {
    pub(crate) fn arbol(&self) -> Option<gotv_sintaxis::Arbol> {
        gotv_sintaxis::analizar(self.lenguaje, &self.texto)
    }
}

const EXCLUIDAS: &[&str] = &["target", "node_modules", "dist", "build", "vendor", "__pycache__", "venv", "env", "out", "coverage", "Pods", "DerivedData", "terceros", "third_party"];
const MAX_ARCHIVOS: usize = 3_000;
const MAX_BYTES: u64 = 300_000;

fn recorrer(dir: &Path, fondo: usize, out: &mut Vec<PathBuf>) {
    if fondo > 10 || out.len() >= MAX_ARCHIVOS {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut es: Vec<_> = rd.flatten().collect();
    es.sort_by_key(|e| e.file_name());
    for e in es {
        let n = e.file_name().to_string_lossy().to_string();
        let Ok(ft) = e.file_type() else { continue };
        if ft.is_dir() {
            if !n.starts_with('.') && !EXCLUIDAS.contains(&n.as_str()) {
                recorrer(&e.path(), fondo + 1, out);
            }
        } else if ft.is_file() && !n.contains(".min.") && e.metadata().is_ok_and(|m| m.len() <= MAX_BYTES) && gotv_sintaxis::Lenguaje::de_ruta(&e.path()).is_some_and(|l| !matches!(l, gotv_sintaxis::Lenguaje::Html | gotv_sintaxis::Lenguaje::Css | gotv_sintaxis::Lenguaje::Json | gotv_sintaxis::Lenguaje::C | gotv_sintaxis::Lenguaje::Swift)) {
            out.push(e.path());
            if out.len() >= MAX_ARCHIVOS {
                return;
            }
        }
    }
}

/// El código del proyecto que los puentes miran (Rust, Python, TS/JS, Go, PHP).
pub(crate) fn fuentes(root: &Path) -> Vec<Fuente> {
    // la raíz puede ser `~` u otra carpeta cualquiera: sin señas de proyecto no se recorre
    const SENAS: &[&str] = &[".git", "Cargo.toml", "package.json", "pyproject.toml", "requirements.txt", "setup.py", "manage.py", "go.mod", "composer.json", "Gemfile", "deno.json", ".env.example"];
    if !SENAS.iter().any(|s| root.join(s).exists()) {
        return vec![];
    }
    let mut rutas = Vec::new();
    recorrer(root, 0, &mut rutas);
    rutas
        .into_iter()
        .filter_map(|p| {
            let lenguaje = gotv_sintaxis::Lenguaje::de_ruta(&p)?;
            let texto = std::fs::read_to_string(&p).ok()?;
            Some(Fuente { rel: p.strip_prefix(root).unwrap_or(&p).display().to_string(), texto, lenguaje })
        })
        .collect()
}

/// Recorre todos los nodos (sin recursión: un archivo hondo no agota la pila).
pub(crate) fn cada_nodo<'a>(raiz: Node<'a>, mut f: impl FnMut(Node<'a>)) {
    let mut c = raiz.walk();
    loop {
        f(c.node());
        if c.goto_first_child() {
            continue;
        }
        loop {
            if c.goto_next_sibling() {
                break;
            }
            if !c.goto_parent() {
                return;
            }
        }
    }
}

pub(crate) fn texto<'a>(n: Node, t: &'a str) -> &'a str {
    t.get(n.byte_range()).unwrap_or("")
}

pub(crate) fn linea(n: Node) -> usize {
    n.start_position().row + 1
}

/// El contenido de un literal de texto de cualquier lenguaje. Con `comodin`, las partes interpoladas
/// (`${id}`, `{id}` de un f-string) se vuelven `*`; sin él, un literal interpolado no cuenta (`None`).
pub(crate) fn literal(n: Node, t: &str, comodin: bool) -> Option<String> {
    let k = n.kind();
    if !matches!(k, "string" | "string_literal" | "template_string" | "interpreted_string_literal" | "raw_string_literal" | "encapsed_string" | "concatenated_string") {
        return None;
    }
    let mut s = String::new();
    let mut c = n.walk();
    let mut interpolado = false;
    let hijos: Vec<Node> = n.named_children(&mut c).collect();
    if hijos.is_empty() {
        // sin hijos con nombre: el texto entre las comillas
        let crudo = texto(n, t);
        return Some(crudo.trim_matches(|ch| ch == '"' || ch == '\'' || ch == '`').to_string());
    }
    let mut reconocidos = 0;
    for h in hijos {
        match h.kind() {
            "string_fragment" | "string_content" | "escape_sequence" | "interpreted_string_literal_content" | "raw_string_literal_content" => {
                reconocidos += 1;
                s.push_str(texto(h, t));
            }
            "template_substitution" | "interpolation" | "string_interpolation" | "variable_name" | "dynamic_variable_name" => {
                reconocidos += 1;
                interpolado = true;
                s.push('*');
            }
            "string_start" | "string_end" => {}
            _ => {}
        }
    }
    if interpolado && !comodin {
        return None;
    }
    if reconocidos == 0 {
        // hijos de un tipo que esta gramática nombra distinto: el texto entre las comillas
        return Some(texto(n, t).trim_matches(|ch| ch == '"' || ch == '\'' || ch == '`').to_string());
    }
    Some(s)
}

/// `user_id` → `userId` (lo que hace Tauri con los argumentos por omisión).
pub(crate) fn camel(s: &str) -> String {
    let mut out = String::new();
    let mut mayus = false;
    for ch in s.chars() {
        if ch == '_' && !out.is_empty() {
            mayus = true;
        } else if mayus {
            out.extend(ch.to_uppercase());
            mayus = false;
        } else {
            out.push(ch);
        }
    }
    out
}

/// Todos los posibles desajustes del proyecto ahora.
pub fn desajustes(root: &Path) -> Vec<Desajuste> {
    let f = fuentes(root);
    let mut v = tauri::revisar(&f);
    v.extend(http::revisar(&f));
    v.extend(entorno::revisar(root, &f));
    v.extend(datos::revisar(root, &f));
    v.extend(plantillas::revisar(root, &f));
    v
}

/// Formato propio (códec, CONT); hasta 2026-11-17 era `contratos.json` (se migra solo).
fn archivo(root: &Path) -> PathBuf {
    root.join(".gotv").join("contratos.gotv")
}

fn leer(root: &Path) -> Option<Vec<Desajuste>> {
    let f = archivo(root);
    crate::programar::codec::leer_serde_migrado(&f, &f.with_extension("json"), b"CONT")
}

fn guardar(root: &Path, v: &[Desajuste]) {
    let _ = crate::programar::codec::guardar_serde(&archivo(root), b"CONT", &v);
}

/// ¿Puede este archivo mover un contrato? (código de los lenguajes de los puentes, o un `.env*`)
pub fn relevante(p: &Path) -> bool {
    p.file_name().is_some_and(|n| n.to_string_lossy().starts_with(".env"))
        // una plantilla (Jinja, Twig; Blade es `.php`)
        || p.extension().is_some_and(|e| matches!(e.to_string_lossy().to_lowercase().as_str(), "html" | "htm" | "twig" | "jinja" | "jinja2" | "j2"))
        || gotv_sintaxis::Lenguaje::de_ruta(p).is_some_and(|l| matches!(l, gotv_sintaxis::Lenguaje::Rust | gotv_sintaxis::Lenguaje::Python | gotv_sintaxis::Lenguaje::TypeScript | gotv_sintaxis::Lenguaje::Tsx | gotv_sintaxis::Lenguaje::JavaScript | gotv_sintaxis::Lenguaje::Go | gotv_sintaxis::Lenguaje::Php))
}

/// La base pasa a ser lo de AHORA (tras una foto nueva de un esquema que se leyó, no que se cambió: lo
/// que ya no casaba no es culpa de la próxima edición).
pub fn rebasar(root: &Path) {
    guardar(root, &desajustes(root));
}

/// Antes de la primera edición: si aún no hay mapa, se toma (la base contra la que se dirá qué es nuevo).
pub fn asegurar(root: &Path) {
    if !archivo(root).is_file() {
        guardar(root, &desajustes(root));
    }
}

/// Tras editar `editado`: los posibles desajustes NUEVOS respecto al mapa guardado (que se actualiza).
/// `None` si el archivo no toca ningún puente, si no había mapa previo o si nada nuevo dejó de casar.
pub fn tras_editar(root: &Path, editado: &Path) -> Option<String> {
    if !relevante(editado) {
        return None;
    }
    novedades(root)
}

/// Tras cambiar el ESQUEMA de una base con `base_datos` (su foto ya está al día): las consultas del código
/// que dejaron de casar (p. ej. una columna quitada). Mismo criterio: sólo lo nuevo.
pub fn tras_cambio_de_esquema(root: &Path) -> Option<String> {
    if !archivo(root).is_file() {
        // sin base previa no se sabe qué es nuevo: se toma ahora (la próxima vez ya compara)
        return None;
    }
    novedades(root)
}

fn novedades(root: &Path) -> Option<String> {
    let antes = leer(root);
    let ahora = desajustes(root);
    guardar(root, &ahora);
    let antes: BTreeSet<(String, String)> = antes?.into_iter().map(|d| (d.puente, d.clave)).collect();
    let nuevos: Vec<&Desajuste> = ahora.iter().filter(|d| !antes.contains(&(d.puente.clone(), d.clave.clone()))).collect();
    if nuevos.is_empty() {
        return None;
    }
    let mut s = crate::i18n::t(
        "[Puentes] ⚠ posible desajuste entre lenguajes tras esta edición (heurístico: compruébalo):",
        "[Bridges] ⚠ possible cross-language mismatch after this edit (heuristic: check it):",
    )
    .to_string();
    for d in nuevos.iter().take(8) {
        s.push_str(&format!("\n  - {}", d.texto));
    }
    if nuevos.len() > 8 {
        s.push_str(&crate::tr!("\n  … y {} más", "\n  … and {} more", nuevos.len() - 8));
    }
    Some(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn camel_como_tauri() {
        assert_eq!(camel("user_id"), "userId");
        assert_eq!(camel("nombre"), "nombre");
        assert_eq!(camel("a_b_c"), "aBC");
    }

    /// El ciclo completo: sin mapa no se inventa «nuevo»; tras tomarlo, renombrar un comando en Rust avisa
    /// del `invoke` que ya no casa; lo que ya estaba roto no se repite; un archivo ajeno a los puentes no
    /// cuesta nada.
    #[test]
    fn solo_lo_nuevo_tras_una_edicion() {
        let d = crate::CarpetaPrueba::nueva("gotv-puentes-ciclo");
        std::fs::write(d.join("package.json"), "{}").unwrap();
        std::fs::create_dir_all(d.join("src-tauri/src")).unwrap();
        std::fs::create_dir_all(d.join("web")).unwrap();
        let rs = "use tauri;\n#[tauri::command]\nfn saludar(nombre: String) -> String { nombre }\n\nfn main() {\n    tauri::Builder::default().invoke_handler(tauri::generate_handler![saludar]);\n}\n";
        std::fs::write(d.join("src-tauri/src/main.rs"), rs).unwrap();
        std::fs::write(d.join("web/app.ts"), "import { invoke } from '@tauri-apps/api/core';\nawait invoke('saludar', { nombre: 'x' });\nawait invoke('viejo_roto');\n").unwrap();
        asegurar(&d);
        assert!(tras_editar(&d, &d.join("README.md")).is_none(), "no es código de un puente");
        assert!(tras_editar(&d, &d.join("web/app.ts")).is_none(), "nada nuevo: «viejo_roto» ya estaba");
        std::fs::write(d.join("src-tauri/src/main.rs"), rs.replace("fn saludar", "fn saludo").replace("![saludar]", "![saludo]")).unwrap();
        let t = tras_editar(&d, &d.join("src-tauri/src/main.rs")).unwrap();
        assert!(t.contains("saludar") && t.contains("web/app.ts:2") && !t.contains("viejo_roto"), "{t}");
        assert!(tras_editar(&d, &d.join("src-tauri/src/main.rs")).is_none(), "ya avisado: no se repite");
    }
}

#[cfg(test)]
mod medida {
    /// Cuánto cuesta revisar los puentes de un proyecto real (corre tras cada edición de código):
    /// `GOTV_PUENTES_RAIZ=/ruta cargo test --release --workspace -- --ignored medida --nocapture`.
    #[test]
    #[ignore = "mide un proyecto real: GOTV_PUENTES_RAIZ"]
    fn medida_en_un_proyecto_real() {
        let Some(r) = std::env::var_os("GOTV_PUENTES_RAIZ") else { return };
        let r = std::path::PathBuf::from(r);
        let t = std::time::Instant::now();
        let n = super::fuentes(&r).len();
        let leer = t.elapsed();
        let d = super::desajustes(&r);
        eprintln!("{n} archivos · leer {leer:?} · todo {:?} · {} desajustes", t.elapsed(), d.len());
        for x in d.iter().take(15) {
            eprintln!("  {} {}", x.puente, x.texto);
        }
    }
}
