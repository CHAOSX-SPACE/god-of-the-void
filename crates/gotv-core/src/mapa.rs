//! D8d · El MAPA del código: qué define cada archivo y dónde, sin leer archivos enteros.
//!
//! Tres usos, una sola lectura del árbol:
//! - `bloque`: un resumen compacto que entra al INICIO de la sesión (dentro de un presupuesto de
//!   caracteres): los archivos más referenciados del proyecto y los nombres que definen;
//! - `esquema`: el esquema de UN archivo (respaldo de `lsp symbols` donde no hay servidor);
//! - `buscar`: dónde se define un nombre en todo el proyecto (respaldo de `lsp workspace`).
//!
//! Sin servidor de lenguaje y sin dependencias: se reconocen las DEFINICIONES por la forma de la
//! línea (palabra clave + nombre) en Rust, Python, JS/TS, Go, Java/Kotlin/C#/Swift/PHP/Scala,
//! C/C++, Ruby, Bash y SQL. No es un analizador: no ve los métodos sin palabra clave de Java o C#
//! ni lo que genera una macro. Para eso está el servidor; esto es lo que hay cuando no lo hay, y
//! lo que cabe mirar en frío en TODO el proyecto en milisegundos.
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq)]
pub struct Simbolo {
    /// la palabra del lenguaje: `fn`, `struct`, `class`, `def`, `func`, `type`…
    pub clase: &'static str,
    pub nombre: String,
    /// línea, desde 1
    pub linea: usize,
    /// 0 = arriba del todo; 1 = dentro de un tipo o un `impl`
    pub nivel: usize,
}

const MAX_ARCHIVO: u64 = 300_000;
const MAX_ARCHIVOS: usize = 2_000;
const MAX_ENTRADAS: usize = 40_000;
const MAX_FONDO: usize = 9;
const EXCLUIDAS: &[&str] = &["target", "node_modules", "dist", "build", "vendor", "__pycache__", "venv", "env", "out", "coverage", "Pods", "DerivedData", "terceros", "third_party"];
/// Lo que delata que una carpeta ES un proyecto (sin esto no se recorre: la raíz puede ser `~`).
const SENAS: &[&str] = &[".git", "Cargo.toml", "package.json", "pyproject.toml", "requirements.txt", "setup.py", "go.mod", "pom.xml", "build.gradle", "build.gradle.kts", "composer.json", "Gemfile", "Makefile", "CMakeLists.txt", "Package.swift", "deno.json"];

fn familia(ext: &str) -> Option<&'static str> {
    Some(match ext {
        "rs" => "rs",
        "py" | "pyi" => "py",
        "js" | "jsx" | "mjs" | "cjs" | "ts" | "tsx" | "mts" | "cts" => "js",
        "go" => "go",
        "java" | "kt" | "kts" | "cs" | "swift" | "php" | "scala" | "dart" => "tipos",
        "c" | "h" | "cc" | "cpp" | "cxx" | "hpp" | "hh" | "m" | "mm" => "c",
        "rb" => "rb",
        "sh" | "bash" | "zsh" => "sh",
        "sql" => "sql",
        _ => return None,
    })
}

fn es_ident(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// El identificador con el que empieza `s` (vacío si no empieza por uno).
fn ident(s: &str) -> &str {
    let fin = s.char_indices().find(|(_, c)| !es_ident(*c)).map(|(i, _)| i).unwrap_or(s.len());
    &s[..fin]
}

/// Quita del principio las palabras de `fuera` (modificadores), las veces que haga falta.
fn sin_modificadores<'a>(mut s: &'a str, fuera: &[&str]) -> &'a str {
    loop {
        let w = ident(s);
        // `pub(crate)`, `pub(super)`…
        if w == "pub" && s[w.len()..].starts_with('(') {
            match s.find(')') {
                Some(i) => {
                    s = s[i + 1..].trim_start();
                    continue;
                }
                None => return s,
            }
        }
        if !w.is_empty() && fuera.contains(&w) && s[w.len()..].starts_with(char::is_whitespace) {
            s = s[w.len()..].trim_start();
        } else {
            return s;
        }
    }
}

/// `clave nombre` al principio de `s` → el nombre.
fn tras<'a>(s: &'a str, clave: &str) -> Option<&'a str> {
    let r = s.strip_prefix(clave)?;
    if !r.starts_with(char::is_whitespace) {
        return None;
    }
    let n = ident(r.trim_start());
    (!n.is_empty()).then_some(n)
}

fn linea_rs(s: &str) -> Option<(&'static str, String)> {
    let s = sin_modificadores(s, &["pub", "async", "unsafe", "default", "extern"]);
    let s = if s.starts_with('"') { s.splitn(3, '"').nth(2).unwrap_or("").trim_start() } else { s };
    for k in ["fn", "struct", "enum", "trait", "mod", "type", "union"] {
        if let Some(n) = tras(s, k) {
            return Some((k, n.to_string()));
        }
    }
    for k in ["const", "static"] {
        if let Some(r) = s.strip_prefix(k).filter(|r| r.starts_with(char::is_whitespace)) {
            let r = sin_modificadores(r.trim_start(), &["mut"]);
            // `const fn x()` es una función
            if let Some(n) = tras(r, "fn") {
                return Some(("fn", n.to_string()));
            }
            let n = ident(r);
            if !n.is_empty() && n != "_" {
                return Some((k, n.to_string()));
            }
        }
    }
    if let Some(r) = s.strip_prefix("macro_rules!") {
        let n = ident(r.trim_start());
        return (!n.is_empty()).then(|| ("macro", n.to_string()));
    }
    // `impl<T> Rasgo for Tipo<T> {` → `impl Tipo` (el tipo es por lo que se busca)
    if let Some(r) = s.strip_prefix("impl") {
        if r.starts_with(char::is_whitespace) || r.starts_with('<') {
            let mut r = r.trim_start();
            if r.starts_with('<') {
                let mut fondo = 0i32;
                let fin = r.char_indices().find(|(_, c)| {
                    match c {
                        '<' => fondo += 1,
                        '>' => fondo -= 1,
                        _ => {}
                    }
                    fondo == 0
                });
                r = fin.map(|(i, _)| r[i + 1..].trim_start()).unwrap_or("");
            }
            let cuerpo = r.split('{').next().unwrap_or(r).split(" where ").next().unwrap_or(r);
            let tipo = cuerpo.rsplit(" for ").next().unwrap_or(cuerpo).trim().trim_start_matches('&').trim_start_matches("dyn ");
            let base = tipo.split('<').next().unwrap_or(tipo).trim();
            let n = ident(base.rsplit("::").next().unwrap_or(base));
            return (!n.is_empty()).then(|| ("impl", n.to_string()));
        }
    }
    None
}

fn linea_py(s: &str) -> Option<(&'static str, String)> {
    let s = sin_modificadores(s, &["async"]);
    tras(s, "def").map(|n| ("def", n.to_string())).or_else(|| tras(s, "class").map(|n| ("class", n.to_string())))
}

fn linea_js(s: &str, nivel: usize) -> Option<(&'static str, String)> {
    let exportado = s.starts_with("export ");
    let s = sin_modificadores(s, &["export", "default", "declare", "async", "abstract"]);
    if let Some(r) = s.strip_prefix("function") {
        let n = ident(r.trim_start_matches('*').trim_start());
        if !n.is_empty() && (r.starts_with(char::is_whitespace) || r.starts_with('*')) {
            return Some(("function", n.to_string()));
        }
    }
    for k in ["class", "interface", "enum", "namespace"] {
        if let Some(n) = tras(s, k) {
            return Some((k, n.to_string()));
        }
    }
    if let Some(n) = tras(s, "type") {
        // `type X = …`, no la propiedad `type: …`
        if s[4..].trim_start()[n.len()..].trim_start().starts_with(['=', '<']) {
            return Some(("type", n.to_string()));
        }
    }
    if nivel == 0 {
        for k in ["const", "let", "var"] {
            if let Some(n) = tras(s, k) {
                let resto = &s[k.len()..].trim_start()[n.len()..];
                let funcion = resto.contains("=>") || resto.contains("function");
                if exportado || funcion {
                    return Some((if funcion { "function" } else { "const" }, n.to_string()));
                }
            }
        }
    }
    None
}

fn linea_go(s: &str) -> Option<(&'static str, String)> {
    if let Some(r) = s.strip_prefix("func ") {
        let r = r.trim_start();
        if let Some(rec) = r.strip_prefix('(') {
            // `func (s *Servidor) Nombre(` → `Servidor.Nombre`
            let (receptor, resto) = rec.split_once(')')?;
            let tipo = ident(receptor.split_whitespace().last()?.trim_start_matches('*'));
            let n = ident(resto.trim_start());
            return (!n.is_empty()).then(|| ("func", if tipo.is_empty() { n.to_string() } else { format!("{tipo}.{n}") }));
        }
        let n = ident(r);
        return (!n.is_empty()).then(|| ("func", n.to_string()));
    }
    tras(s, "type").map(|n| ("type", n.to_string()))
}

fn linea_tipos(s: &str) -> Option<(&'static str, String)> {
    let s = sin_modificadores(
        s,
        &["public", "private", "protected", "internal", "static", "final", "abstract", "open", "override", "sealed", "data", "suspend", "async", "virtual", "partial", "readonly", "export", "inline", "fileprivate", "mutating", "case", "implicit", "lazy", "actual", "expect", "external", "tailrec", "operator", "infix", "unsafe", "new", "extern"],
    );
    for k in ["class", "interface", "enum", "record", "struct", "object", "trait", "protocol", "extension", "namespace", "fun", "func", "function", "def", "mixin", "typealias"] {
        if let Some(n) = tras(s, k) {
            // Kotlin: `fun Tipo.extension()` y `fun <T> nombre()` se dejan al servidor
            return Some((k, n.to_string()));
        }
    }
    None
}

fn linea_c(s: &str, crudo: &str) -> Option<(&'static str, String)> {
    if crudo.starts_with(char::is_whitespace) || s.starts_with('#') {
        return None;
    }
    for k in ["struct", "class", "enum", "union", "namespace"] {
        if let Some(n) = tras(sin_modificadores(s, &["typedef", "static", "extern"]), k) {
            // `struct x *f(void)` es una función que devuelve un struct: sigue abajo
            if !s.contains('(') {
                return (!s.trim_end().ends_with(';') || s.contains('{')).then(|| (k, n.to_string()));
            }
        }
    }
    let antes = s.split('(').next()?;
    if antes.len() == s.len() || s.trim_end().ends_with(';') || antes.contains('=') {
        return None;
    }
    let n = ident_final(antes.trim_end());
    let primera = ident(s);
    if n.is_empty() || ["if", "for", "while", "switch", "return", "else", "do", "typedef", "sizeof"].contains(&primera) || n == primera && !antes.contains("::") {
        return None;
    }
    Some(("fn", n.to_string()))
}

/// El identificador con el que TERMINA `s`.
fn ident_final(s: &str) -> &str {
    let ini = s.char_indices().rev().find(|(_, c)| !es_ident(*c)).map(|(i, c)| i + c.len_utf8()).unwrap_or(0);
    &s[ini..]
}

fn linea_rb(s: &str) -> Option<(&'static str, String)> {
    for k in ["def", "class", "module"] {
        if let Some(r) = s.strip_prefix(k).filter(|r| r.starts_with(char::is_whitespace)) {
            let r = r.trim_start().trim_start_matches("self.");
            let n = ident(r);
            return (!n.is_empty()).then(|| (k, n.to_string()));
        }
    }
    None
}

fn linea_sh(s: &str) -> Option<(&'static str, String)> {
    if let Some(n) = tras(s, "function") {
        return Some(("function", n.to_string()));
    }
    let n = ident(s);
    let r = s[n.len()..].trim_start();
    (!n.is_empty() && r.starts_with("()")).then(|| ("function", n.to_string()))
}

fn linea_sql(s: &str) -> Option<(&'static str, String)> {
    let may = s.to_uppercase();
    if may.len() != s.len() {
        return None; // las mayúsculas cambiaron el largo: no se puede calcar la posición
    }
    let r = may.strip_prefix("CREATE ")?;
    let r = r.trim_start_matches("OR REPLACE ").trim_start_matches("UNIQUE ").trim_start_matches("MATERIALIZED ").trim_start_matches("VIRTUAL ").trim_start_matches("TEMPORARY ").trim_start_matches("TEMP ");
    for (k, clase) in [("TABLE", "table"), ("VIEW", "view"), ("FUNCTION", "function"), ("PROCEDURE", "procedure"), ("INDEX", "index"), ("TRIGGER", "trigger")] {
        if let Some(q) = r.strip_prefix(k).filter(|q| q.starts_with(' ')) {
            let q = q.trim_start().trim_start_matches("IF NOT EXISTS ");
            // el nombre se toma del texto ORIGINAL (con sus mayúsculas), en la misma posición
            let ini = s.len() - q.len();
            let n = s[ini..].trim_start_matches(['"', '`', '[']);
            let n: String = n.chars().take_while(|c| es_ident(*c) || *c == '.').collect();
            return (!n.is_empty()).then_some((clase, n));
        }
    }
    None
}

/// Las definiciones de un archivo, por la forma de sus líneas. `ext` sin punto.
pub fn simbolos(ext: &str, texto: &str) -> Vec<Simbolo> {
    let Some(fam) = familia(&ext.to_lowercase()) else { return vec![] };
    let mut out = Vec::new();
    for (i, crudo) in texto.lines().enumerate() {
        if crudo.len() > 400 {
            continue; // minificado o datos: no es código que se lea
        }
        let s = crudo.trim_start();
        if s.is_empty() || s.starts_with("//") || s.starts_with('*') || s.starts_with("/*") || (s.starts_with('#') && fam != "c") || s.starts_with("--") {
            continue;
        }
        let sangria: usize = crudo.chars().take_while(|c| c.is_whitespace()).map(|c| if c == '\t' { 4 } else { 1 }).sum();
        let nivel = sangria / if fam == "rb" || (fam == "js" && sangria % 4 != 0) { 2 } else { 4 };
        if nivel > 1 {
            continue;
        }
        let r = match fam {
            "rs" => linea_rs(s),
            "py" => linea_py(s),
            "js" => linea_js(s, nivel),
            "go" => linea_go(s),
            "tipos" => linea_tipos(s),
            "c" => linea_c(s, crudo),
            "rb" => linea_rb(s),
            "sh" => linea_sh(s),
            "sql" => linea_sql(s),
            _ => None,
        };
        if let Some((clase, nombre)) = r {
            out.push(Simbolo { clase, nombre, linea: i + 1, nivel });
        }
    }
    out
}

/// F5 ML7 · la palabra con que cada lenguaje escribe esa definición (como en el mapa: `def`, `func`…).
fn clase_de(l: gotv_sintaxis::Lenguaje, tipo: &'static str) -> &'static str {
    use gotv_sintaxis::Lenguaje as L;
    match (l, tipo) {
        (L::Python, "fn" | "method") => "def",
        (L::Go | L::Swift, "fn" | "method") => "func",
        (L::Go, "struct" | "interface") => "type",
        (L::TypeScript | L::Tsx | L::JavaScript | L::Php, "fn" | "method") => "function",
        (L::TypeScript | L::Tsx | L::JavaScript, "mod") => "namespace",
        (_, "method") => "fn",
        (_, t) => t,
    }
}

/// F5 ML7 · las definiciones por el ÁRBOL de tree-sitter (todas, a cualquier profundidad), si el lenguaje
/// tiene gramática dentro de GOTV; `None` si no.
pub fn simbolos_por_arbol(ruta: &Path, texto: &str) -> Option<Vec<Simbolo>> {
    let l = gotv_sintaxis::Lenguaje::de_ruta(ruta)?;
    let a = gotv_sintaxis::analizar(l, texto)?;
    Some(gotv_sintaxis::simbolos(&a, 2_000).into_iter().map(|s| Simbolo { clase: clase_de(l, s.tipo), nombre: s.nombre, linea: s.linea, nivel: s.nivel }).collect())
}

/// El esquema de un archivo: una definición por línea, con su número, sangrada por nivel. Con gramática
/// (ML7), del árbol; si no, por la forma de sus líneas.
pub fn esquema(ruta: &Path, texto: &str) -> String {
    let ext = ruta.extension().and_then(|e| e.to_str()).unwrap_or("");
    let s = simbolos_por_arbol(ruta, texto).unwrap_or_else(|| simbolos(ext, texto));
    if s.is_empty() {
        return crate::i18n::t("(sin definiciones reconocibles)", "(no recognizable definitions)").into();
    }
    let mut out: Vec<String> = s.iter().take(300).map(|x| format!("{}: {}{} {}", x.linea, "  ".repeat(x.nivel), x.clase, x.nombre)).collect();
    if s.len() > 300 {
        out.push(crate::tr!("… y {} más", "… and {} more", s.len() - 300));
    }
    out.join("\n")
}

pub struct Archivo {
    pub rel: String,
    pub simbolos: Vec<Simbolo>,
    /// F21: cambiado en las últimas 48 horas (pesa más en el mapa)
    pub reciente: bool,
}

/// F21 · las definiciones de un archivo para el mapa: del ÁRBOL si su lenguaje tiene gramática (ve los
/// métodos sin palabra clave), si no por la forma de las líneas; sólo los dos primeros niveles.
fn simbolos_para_el_mapa(p: &Path, texto: &str) -> Vec<Simbolo> {
    match simbolos_por_arbol(p, texto) {
        Some(v) => v.into_iter().filter(|s| s.nivel <= 1).collect(),
        None => simbolos(p.extension().and_then(|x| x.to_str()).unwrap_or(""), texto),
    }
}

/// La clase de una definición guardada en la caché, de vuelta a `&'static str` (las conocidas sin
/// gastar memoria; una desconocida se guarda una vez por proceso).
fn clase_estatica(c: &str) -> &'static str {
    const CONOCIDAS: &[&str] = &[
        "fn", "function", "func", "fun", "def", "method", "class", "struct", "enum", "trait", "interface", "type", "typealias", "typedef", "impl", "mod", "module", "namespace", "const", "static", "macro", "protocol", "extension", "object", "record", "union", "table", "view", "index", "trigger", "procedure", "selector", "key",
    ];
    if let Some(k) = CONOCIDAS.iter().find(|k| **k == c) {
        return k;
    }
    static OTRAS: std::sync::OnceLock<std::sync::Mutex<HashSet<&'static str>>> = std::sync::OnceLock::new();
    let mut m = OTRAS.get_or_init(Default::default).lock().unwrap();
    if let Some(k) = m.get(c) {
        return k;
    }
    let k: &'static str = Box::leak(c.to_string().into_boxed_str());
    m.insert(k);
    k
}

/// F21 · la caché del mapa: por archivo, su tamaño y fecha (lo que lo identifica sin leerlo) y sus
/// definiciones. Sólo se vuelve a analizar lo que cambió.
#[derive(serde::Serialize, serde::Deserialize, Default)]
struct CacheMapa {
    version: u32,
    archivos: std::collections::BTreeMap<String, EntradaCache>,
}

#[derive(serde::Serialize, serde::Deserialize, Clone)]
struct EntradaCache {
    tam: u64,
    fecha: u128,
    /// (clase, nombre, línea, nivel)
    simbolos: Vec<(String, String, usize, usize)>,
}

/// Sube cuando cambia CÓMO se sacan las definiciones (la caché vieja no sirve).
const VERSION_CACHE: u32 = 1;

fn archivo_cache(root: &Path) -> PathBuf {
    // formato propio (códec, MAPA); la `mapa.json` de antes era caché: se recalcula, no se migra
    root.join(".gotv").join("mapa.gotv")
}

fn es_proyecto(root: &Path) -> bool {
    SENAS.iter().any(|s| root.join(s).exists())
}

fn recorrer(dir: &Path, fondo: usize, vistas: &mut usize, out: &mut Vec<PathBuf>) {
    if fondo > MAX_FONDO || out.len() >= MAX_ARCHIVOS || *vistas >= MAX_ENTRADAS {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut entradas: Vec<_> = rd.flatten().collect();
    // orden fijo: el mismo árbol da el mismo mapa (el sistema de archivos no lo garantiza)
    entradas.sort_by_key(|e| e.file_name());
    for e in entradas {
        *vistas += 1;
        if out.len() >= MAX_ARCHIVOS || *vistas >= MAX_ENTRADAS {
            return;
        }
        let nombre = e.file_name().to_string_lossy().to_string();
        let Ok(ft) = e.file_type() else { continue };
        if ft.is_dir() {
            if !nombre.starts_with('.') && !EXCLUIDAS.contains(&nombre.as_str()) {
                recorrer(&e.path(), fondo + 1, vistas, out);
            }
        } else if ft.is_file() {
            let p = e.path();
            let ext = p.extension().and_then(|x| x.to_str()).unwrap_or("").to_lowercase();
            let pasa = familia(&ext).is_some() && !nombre.contains(".min.") && !nombre.ends_with(".d.ts") && e.metadata().map(|m| m.len() <= MAX_ARCHIVO).unwrap_or(false);
            if pasa {
                out.push(p);
            }
        }
    }
}

/// Lee el proyecto UNA vez: las definiciones de cada archivo y en cuántos archivos aparece cada
/// identificador (eso dice qué nombres —y qué archivos— usa el resto del código).
pub fn indexar(root: &Path) -> (Vec<Archivo>, HashMap<String, u32>) {
    let mut rutas = Vec::new();
    recorrer(root, 0, &mut 0, &mut rutas);
    // F21: la caché (sólo en un proyecto: nunca se escribe `.gotv` en una carpeta cualquiera)
    let con_cache = es_proyecto(root);
    let mut cache: CacheMapa = if con_cache {
        crate::programar::codec::leer_serde(&archivo_cache(root), b"MAPA").filter(|c: &CacheMapa| c.version == VERSION_CACHE).unwrap_or_default()
    } else {
        CacheMapa::default()
    };
    let mut nueva = CacheMapa { version: VERSION_CACHE, archivos: Default::default() };
    let mut cambio = false;
    let ahora = std::time::SystemTime::now();
    let mut archivos = Vec::new();
    let mut menciones: HashMap<String, u32> = HashMap::new();
    for p in rutas {
        let Ok(texto) = std::fs::read_to_string(&p) else { continue };
        let mut vistos: HashSet<&str> = HashSet::new();
        for w in texto.split(|c: char| !es_ident(c)) {
            if w.len() >= 3 && !w.starts_with(|c: char| c.is_ascii_digit()) && vistos.insert(w) {
                *menciones.entry(w.to_string()).or_default() += 1;
            }
        }
        let rel = p.strip_prefix(root).unwrap_or(&p).to_string_lossy().to_string();
        let meta = std::fs::metadata(&p).ok();
        let tam = meta.as_ref().map(|m| m.len()).unwrap_or(0);
        let modificado = meta.as_ref().and_then(|m| m.modified().ok());
        let fecha = modificado.and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_nanos()).unwrap_or(0);
        let reciente = modificado.and_then(|t| ahora.duration_since(t).ok()).is_some_and(|d| d.as_secs() < 48 * 3600);
        let entrada = match cache.archivos.remove(&rel).filter(|e| e.tam == tam && e.fecha == fecha) {
            Some(e) => e,
            None => {
                cambio = true;
                let s = simbolos_para_el_mapa(&p, &texto);
                EntradaCache { tam, fecha, simbolos: s.into_iter().map(|x| (x.clase.to_string(), x.nombre, x.linea, x.nivel)).collect() }
            }
        };
        let simbolos: Vec<Simbolo> = entrada.simbolos.iter().map(|(c, n, l, v)| Simbolo { clase: clase_estatica(c), nombre: n.clone(), linea: *l, nivel: *v }).collect();
        nueva.archivos.insert(rel.clone(), entrada);
        if !simbolos.is_empty() {
            archivos.push(Archivo { rel, simbolos, reciente });
        }
    }
    // lo que quedó en la vieja es de archivos que ya no están
    if con_cache && (cambio || !cache.archivos.is_empty()) {
        let _ = crate::programar::codec::guardar_serde(&archivo_cache(root), b"MAPA", &nueva);
    }
    (archivos, menciones)
}

/// El nombre por el que el resto del código menciona un símbolo (`Tipo.Metodo` → `Metodo`).
fn corto(n: &str) -> &str {
    n.rsplit('.').next().unwrap_or(n)
}

/// El mapa compacto para el inicio de la sesión, en no más de `presupuesto` caracteres: los
/// archivos cuyas definiciones más usa el resto del proyecto, cada uno con sus nombres más
/// usados. Determinista: el mismo árbol da el mismo texto. Vacío si la carpeta no es un proyecto
/// o casi no tiene código.
pub fn bloque(root: &Path, presupuesto: usize) -> String {
    bloque_con(root, None, presupuesto, None)
}

/// F21 · la herramienta `mapa`: el de una `carpeta` (o del proyecto), en `tokens` (1 500 por omisión), con
/// los archivos de la `tarea` primero (los que la nombran en su ruta o en sus definiciones).
pub fn herramienta(root: &Path, args: &serde_json::Value) -> String {
    let carpeta = args["carpeta"].as_str().map(str::trim).filter(|c| !c.is_empty() && *c != ".");
    let tokens = args["tokens"].as_u64().unwrap_or(1_500).clamp(200, 8_000) as usize;
    let tarea = args["tarea"].as_str().filter(|t| !t.trim().is_empty());
    // F8.4 paso 5: `@nombre` (o `@nombre/sub`) es el mapa de una carpeta EXTRA del proyecto, con sus rutas como
    // `@nombre/…` (así las resuelven read_file y edit)
    if let Some(resto) = carpeta.and_then(|c| c.strip_prefix('@')) {
        let (nombre, sub) = resto.split_once('/').map(|(n, s)| (n, Some(s).filter(|s| !s.is_empty()))).unwrap_or((resto, None));
        let Some((_, base)) = crate::raices::extras_de(root).into_iter().find(|(n, _)| n == nombre) else {
            let hay: Vec<String> = crate::raices::extras_de(root).into_iter().map(|(n, _)| format!("@{n}")).collect();
            return crate::tr!("(no hay una carpeta extra «@{nombre}» en este proyecto; las que hay: {})", "(there is no extra folder «@{nombre}» in this project; the ones there are: {})", if hay.is_empty() { "—".into() } else { hay.join(", ") });
        };
        if !es_proyecto(&base) {
            return crate::tr!("(la carpeta «@{nombre}» no es un proyecto: usa list_dir y glob en «@{nombre}/»)", "(folder «@{nombre}» is not a project: use list_dir and glob on «@{nombre}/»)");
        }
        let t = bloque_con(&base, Some(sub.unwrap_or("")), tokens * 4, tarea);
        return if t.trim().is_empty() {
            crate::tr!("(sin definiciones reconocibles en «@{nombre}»)", "(no recognizable definitions in «@{nombre}»)")
        } else {
            crate::tr!("(rutas de la carpeta extra: antepón «@{nombre}/»)\n{}", "(paths of the extra folder: prefix «@{nombre}/»)\n{}", t.trim_start())
        };
    }
    if !es_proyecto(root) {
        return crate::i18n::t("(esta carpeta no es un proyecto: no hay mapa; usa list_dir y glob)", "(this folder is not a project: no map; use list_dir and glob)").into();
    }
    let t = bloque_con(root, carpeta, tokens * 4, tarea);
    if t.trim().is_empty() {
        return match carpeta {
            Some(c) => crate::tr!("(sin definiciones reconocibles en «{c}»)", "(no recognizable definitions in «{c}»)"),
            None => crate::i18n::t("(sin definiciones reconocibles en el proyecto)", "(no recognizable definitions in the project)").into(),
        };
    }
    t.trim_start().to_string()
}

fn bloque_con(root: &Path, carpeta: Option<&str>, presupuesto: usize, tarea: Option<&str>) -> String {
    if !es_proyecto(root) {
        return String::new();
    }
    let (mut archivos, menciones) = indexar(root);
    // (`Some("")` = toda la carpeta, PEDIDA: la de una extra `@nombre`)
    if let Some(c) = carpeta.map(|c| c.trim_start_matches("./").trim_end_matches('/')).filter(|c| !c.is_empty()) {
        archivos.retain(|a| a.rel == c || a.rel.starts_with(&format!("{c}/")));
    }
    // el bloque del inicio pide un proyecto con algo de código; la herramienta, aunque sea un archivo
    if archivos.len() < if carpeta.is_some() || tarea.is_some() { 1 } else { 3 } {
        return String::new();
    }
    // F21: las palabras de la tarea (4 letras o más) suben los archivos que las nombran
    let palabras: Vec<String> = tarea.map(|t| t.to_lowercase().split(|c: char| !c.is_alphanumeric() && c != '_').filter(|w| w.chars().count() >= 4).map(String::from).collect()).unwrap_or_default();
    let de_la_tarea = |a: &Archivo| palabras.iter().any(|w| a.rel.to_lowercase().contains(w.as_str()) || a.simbolos.iter().any(|s| s.nombre.to_lowercase().contains(w.as_str())));
    // «reciente» sólo distingue si también hay archivos viejos (en un proyecto recién hecho, todos lo son)
    let hay_viejos = archivos.iter().any(|a| !a.reciente);
    // Cuánto usa el resto del proyecto un nombre: los archivos que lo mencionan SIN definirlo,
    // repartidos entre los que lo definen. Medido sobre el propio GOTV (2026-11-14): sin ese
    // reparto ganaban los archivos grandes llenos de `new`, `push`, `esc` —nombres que medio
    // proyecto define por su cuenta— y las copias de respaldo de un mismo archivo.
    let mut definen: HashMap<&str, u32> = HashMap::new();
    for a in &archivos {
        let unicos: HashSet<&str> = a.simbolos.iter().filter(|s| s.clase != "impl" && s.clase != "mod").map(|s| corto(&s.nombre)).collect();
        for n in unicos {
            *definen.entry(n).or_default() += 1;
        }
    }
    // Y sólo cuentan los nombres DISTINTIVOS: que medio proyecto escriba `push`, `name` o `get`
    // no dice que use ESA función (son palabras de cualquier código). Distintivo = compuesto
    // (`validar_pedido`, `TemaDoc`, `enCache`), muy largo, o un TIPO (su nombre propio sí es suyo).
    let tipo = |clase: &str| matches!(clase, "struct" | "enum" | "trait" | "class" | "interface" | "type" | "union" | "record" | "protocol" | "object" | "table");
    let distintivo = |clase: &str, n: &str| {
        let mixto = n.chars().skip(1).any(|c| c.is_uppercase()) && n.chars().any(|c| c.is_lowercase());
        n.contains('_') || mixto || n.chars().count() >= 12 || (tipo(clase) && n.chars().count() >= 4)
    };
    let peso = |clase: &str, n: &str| -> u32 {
        let n = corto(n);
        if !distintivo(clase, n) {
            return 0;
        }
        let d = definen.get(n).copied().unwrap_or(1).max(1);
        let m = menciones.get(n).copied().unwrap_or(0);
        (m.saturating_sub(d) * 100 / d).min(2_000)
    };
    let mut orden: Vec<(u32, &Archivo)> = archivos
        .iter()
        .map(|a| {
            let unicos: HashSet<(&str, &str)> = a.simbolos.iter().filter(|s| s.clase != "impl" && s.clase != "mod").map(|s| (s.clase, s.nombre.as_str())).collect();
            let base: u32 = unicos.iter().map(|(c, n)| peso(c, n)).sum();
            // F21: lo tocado hace poco y lo que nombra la tarea, delante
            let reciente = if a.reciente && hay_viejos { (base / 2).max(50) } else { 0 };
            let tarea = if de_la_tarea(a) { 1_000_000 } else { 0 };
            (base + reciente + tarea, a)
        })
        .collect();
    orden.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.rel.cmp(&b.1.rel)));
    let cabeza = crate::i18n::t(
        "\n\nMapa del código (los archivos que más usa el resto y sus nombres más usados; el esquema de un archivo o dónde se define un nombre: herramienta `lsp` —búscala con tool_search—, «symbols» o «workspace»; el de una carpeta o para una tarea: `mapa`):\n",
        "\n\nCode map (the files the rest uses most and their most used names; a file's outline or where a name is defined: the `lsp` tool —find it with tool_search—, «symbols» or «workspace»; one folder's or for a task: `mapa`):\n",
    );
    let mut lineas: Vec<String> = Vec::new();
    let mut usado = cabeza.chars().count() + 60;
    for (_, a) in &orden {
        let mut vistos = HashSet::new();
        let mut nombres: Vec<&Simbolo> = a.simbolos.iter().filter(|s| s.clase != "impl" && s.clase != "mod" && vistos.insert(s.nombre.as_str())).collect();
        let total = nombres.len();
        if total == 0 {
            continue;
        }
        nombres.sort_by(|x, y| peso(y.clase, &y.nombre).cmp(&peso(x.clase, &x.nombre)).then(x.linea.cmp(&y.linea)));
        nombres.truncate(6);
        let l = format!("- {}: {}{}", a.rel, nombres.iter().map(|s| format!("{} {}", s.clase, s.nombre)).collect::<Vec<_>>().join(", "), if total > 6 { format!(" (+{})", total - 6) } else { String::new() });
        let n = l.chars().count() + 1;
        if usado + n > presupuesto {
            break;
        }
        usado += n;
        lineas.push(l);
    }
    if lineas.is_empty() {
        return String::new();
    }
    let fuera = archivos.len() - lineas.len();
    lineas.sort();
    let mut s = format!("{cabeza}{}", lineas.join("\n"));
    if fuera > 0 {
        s.push_str(&crate::tr!("\n(+{fuera} archivos de código más)", "\n(+{fuera} more code files)"));
    }
    s
}

fn cache() -> &'static std::sync::Mutex<HashMap<PathBuf, (std::time::Instant, String)>> {
    static C: std::sync::OnceLock<std::sync::Mutex<HashMap<PathBuf, (std::time::Instant, String)>>> = std::sync::OnceLock::new();
    C.get_or_init(Default::default)
}

/// El mapa que YA lleva un mensaje de sistema (el bloque entero, con sus dos saltos de línea
/// de delante), si lo lleva. Una conversación empezada conserva SU mapa hasta el final: cambiarlo
/// a mitad reescribe el principio del prompt y obliga al motor a releerla entera (ley L4 del plan
/// del cuerpo). El bloque no tiene líneas en blanco dentro: acaba en la primera que encuentre.
pub fn bloque_en(sistema: &str) -> Option<&str> {
    // juez-idioma: par — las dos cabeceras de `bloque` (el mapa puede venir de una sesión en el otro idioma)
    let i = ["\n\nMapa del código (", "\n\nCode map ("].iter().filter_map(|c| sistema.find(c)).min()?;
    let fin = sistema[i + 2..].find("\n\n").map(|f| i + 2 + f).unwrap_or(sistema.len());
    Some(&sistema[i..fin])
}

/// Olvida el mapa guardado de `root`: la próxima conversación NUEVA lo calcula otra vez.
pub fn olvidar(root: &Path) {
    if let Ok(mut m) = cache().lock() {
        m.remove(root);
    }
}

/// El mapa de `root` para una conversación NUEVA, calculado una vez y guardado 10 minutos (varias
/// conversaciones que nacen seguidas no vuelven a leer el árbol). Dentro de una conversación no se
/// llama dos veces: `Agent::system_prompt` reutiliza el que ya está en su mensaje de sistema.
pub fn en_cache(root: &Path) -> String {
    let c = cache();
    if let Some((cuando, texto)) = c.lock().ok().and_then(|m| m.get(root).cloned()) {
        if cuando.elapsed() < std::time::Duration::from_secs(600) {
            return texto;
        }
    }
    let texto = if std::env::var("GOTV_SIN_MAPA").is_ok() { String::new() } else { bloque(root, 1_800) };
    if let Ok(mut m) = c.lock() {
        m.insert(root.to_path_buf(), (std::time::Instant::now(), texto.clone()));
    }
    texto
}

/// Dónde se define `consulta` en el proyecto: primero el nombre exacto, luego los que empiezan
/// por ella, luego los que la contienen. `ruta:línea: clase nombre`.
pub fn buscar(root: &Path, consulta: &str, max: usize) -> String {
    let q = consulta.trim().to_lowercase();
    if q.is_empty() {
        return crate::i18n::t("ERROR: falta `symbol` (el nombre, o parte, de lo que buscas)", "ERROR: missing `symbol` (the name, or part of it, of what you are looking for)").into();
    }
    let (archivos, _) = indexar(root);
    let mut hallados: Vec<(u8, &str, &Simbolo)> = Vec::new();
    for a in &archivos {
        for s in &a.simbolos {
            let n = corto(&s.nombre).to_lowercase();
            let rango = if n == q {
                0
            } else if n.starts_with(&q) {
                1
            } else if s.nombre.to_lowercase().contains(&q) {
                2
            } else {
                continue;
            };
            hallados.push((rango, &a.rel, s));
        }
    }
    if hallados.is_empty() {
        return crate::tr!("(ninguna definición con «{consulta}»; prueba `grep` para sus usos)", "(no definition matching «{consulta}»; try `grep` for its uses)");
    }
    // a igual acierto, antes la definición que su `impl` o la línea `mod x;` que sólo la nombra
    let apoyo = |s: &Simbolo| matches!(s.clase, "impl" | "mod");
    hallados.sort_by(|a, b| a.0.cmp(&b.0).then(apoyo(a.2).cmp(&apoyo(b.2))).then(a.1.cmp(b.1)).then(a.2.linea.cmp(&b.2.linea)));
    let total = hallados.len();
    let mut out: Vec<String> = hallados.iter().take(max).map(|(_, rel, s)| format!("{rel}:{}: {} {}", s.linea, s.clase, s.nombre)).collect();
    if total > max {
        out.push(crate::tr!("… y {} más: afina el nombre", "… and {} more: narrow the name", total - max));
    }
    out.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nombres(ext: &str, t: &str) -> Vec<String> {
        simbolos(ext, t).iter().map(|s| format!("{}{} {}@{}", "·".repeat(s.nivel), s.clase, s.nombre, s.linea)).collect()
    }

    #[test]
    fn rust_con_sus_modificadores_e_impl() {
        let t = "// fn comentada()\npub(crate) async fn leer(p: &Path) -> Result<()> {\n    let fn_local = 1;\n}\npub struct Mapa<T> { x: T }\nimpl<T: Clone> Iterator for Mapa<T> {\n    pub fn nuevo() -> Self { todo!() }\n    const fn tope() -> usize { 3 }\n}\npub const MAX: usize = 3;\nstatic mut CUENTA: u32 = 0;\nmacro_rules! tr { () => {} }\nenum Modo { A }\npub trait Juez {}\ntype Par = (u8, u8);\nextern \"C\" fn puente() {}\n";
        assert_eq!(
            nombres("rs", t),
            ["fn leer@2", "struct Mapa@5", "impl Mapa@6", "·fn nuevo@7", "·fn tope@8", "const MAX@10", "static CUENTA@11", "macro tr@12", "enum Modo@13", "trait Juez@14", "type Par@15", "fn puente@16"]
        );
    }

    #[test]
    fn python_js_go_y_los_demas() {
        assert_eq!(nombres("py", "import os\nclass Pedido:\n    def total(self):\n        def interna(): pass\n    async def guardar(self): ...\ndef main():\n    pass\n# def no()\n"), ["class Pedido@2", "·def total@3", "·def guardar@5", "def main@6"]);
        let js = "export default function App() {}\nexport const TOPE = 3;\nconst privado = 3;\nconst sumar = (a, b) => a + b;\nexport class Tabla<T> {}\nexport interface Fila { type: string }\nexport type Id = string;\nasync function* leer() {}\n  function anidada() {}\n";
        assert_eq!(nombres("tsx", js), ["function App@1", "const TOPE@2", "function sumar@4", "class Tabla@5", "interface Fila@6", "type Id@7", "function leer@8", "·function anidada@9"]);
        assert_eq!(nombres("go", "package x\nfunc (s *Servidor) Abrir(p string) error {\n}\nfunc main() {}\ntype Servidor struct {\n}\n"), ["func Servidor.Abrir@2", "func main@4", "type Servidor@5"]);
        assert_eq!(nombres("kt", "data class Pedido(val id: Int)\n    suspend fun guardar() {}\ninterface Repo\n"), ["class Pedido@1", "·fun guardar@2", "interface Repo@3"]);
        assert_eq!(nombres("c", "#include <x.h>\nstatic int sumar(int a, int b) {\n    return a + b;\n}\nint declarada(void);\nstruct punto {\n};\nstruct punto *nuevo(void)\n{\n}\nint x = f(3);\n"), ["fn sumar@2", "struct punto@6", "fn nuevo@8"]);
        assert_eq!(nombres("rb", "module Tienda\n  class Pedido\n  def self.crear\n"), ["module Tienda@1", "·class Pedido@2", "·def crear@3"]);
        assert_eq!(nombres("sh", "#!/bin/bash\ndesplegar() {\n}\nfunction limpiar {\n}\n"), ["function desplegar@2", "function limpiar@4"]);
        assert_eq!(nombres("sql", "create table if not exists Pedidos (\n  id int\n);\nCREATE UNIQUE INDEX idx_p ON Pedidos(id);\n"), ["table Pedidos@1", "index idx_p@4"]);
        assert!(simbolos("md", "# fn x()").is_empty());
    }

    fn proyecto(tag: &str) -> crate::CarpetaPrueba {
        let d = crate::CarpetaPrueba::sin_crear(&format!("gotv-mapa-{tag}"));
        std::fs::create_dir_all(d.join("src")).unwrap();
        std::fs::create_dir_all(d.join("node_modules/x")).unwrap();
        std::fs::write(d.join("Cargo.toml"), "[package]\nname = \"x\"\n").unwrap();
        std::fs::write(d.join("src/pedido.rs"), "pub struct Pedido { pub total: u32 }\nimpl Pedido {\n    pub fn nuevo() -> Self { Pedido { total: 0 } }\n}\npub fn validar_pedido(p: &Pedido) -> bool { p.total > 0 }\n").unwrap();
        std::fs::write(d.join("src/main.rs"), "mod pedido;\nfn main() {\n    let p = pedido::Pedido::nuevo();\n    pedido::validar_pedido(&p);\n}\n").unwrap();
        std::fs::write(d.join("src/informe.rs"), "use crate::pedido::Pedido;\npub fn informe(p: &Pedido) -> String { crate::pedido::validar_pedido(p).to_string() }\n").unwrap();
        std::fs::write(d.join("src/suelto.rs"), "fn nadie_me_llama() {}\n").unwrap();
        std::fs::write(d.join("node_modules/x/index.js"), "export function ruido() {}\n").unwrap();
        d
    }

    #[test]
    fn el_mapa_pone_primero_lo_que_mas_se_usa_y_cabe_en_su_presupuesto() {
        let d = proyecto("bloque");
        let m = bloque(&d, 1_800);
        assert!(m.contains("- src/pedido.rs: ") && m.contains("struct Pedido") && m.contains("fn validar_pedido"), "{m}");
        assert!(!m.contains("ruido") && !m.contains("impl Pedido") && !m.contains("mod pedido"), "{m}");
        // lo más usado dentro del archivo va primero
        let l = m.lines().find(|l| l.contains("src/pedido.rs")).unwrap();
        assert!(l.find("struct Pedido").unwrap() < l.find("fn nuevo").unwrap(), "{l}");
        // el mismo árbol, el mismo texto
        assert_eq!(m, bloque(&d, 1_800));
        // con poco presupuesto entra sólo lo más usado, y se dice cuánto queda fuera
        let corto = bloque(&d, m.lines().nth(2).unwrap().chars().count() + 3 + 60 + l.chars().count() + 2);
        assert!(corto.contains("src/pedido.rs") && !corto.contains("suelto.rs") && corto.contains("(+"), "{corto}");
        assert!(corto.chars().count() < m.chars().count());
        // una carpeta que no es un proyecto no se recorre
        std::fs::remove_file(d.join("Cargo.toml")).unwrap();
        assert_eq!(bloque(&d, 1_800), "");
    }

    #[test]
    fn buscar_ordena_exacto_prefijo_contiene_y_el_esquema_numera() {
        let d = proyecto("buscar");
        let r = buscar(&d, "pedido", 10);
        let l: Vec<&str> = r.lines().collect();
        assert!(l[0].starts_with("src/pedido.rs:1: struct Pedido"), "{r}");
        assert!(l.iter().any(|x| x.contains("fn validar_pedido")) && !r.contains("ruido"), "{r}");
        assert!(buscar(&d, "informe", 10).starts_with("src/informe.rs:2: fn informe"));
        assert!(buscar(&d, "zzz", 10).contains("zzz") && buscar(&d, " ", 10).starts_with("ERROR"));
        assert!(buscar(&d, "e", 2).lines().count() == 3, "el corte se dice");
        let e = esquema(Path::new("src/pedido.rs"), &std::fs::read_to_string(d.join("src/pedido.rs")).unwrap());
        assert_eq!(e, "1: struct Pedido\n2: impl Pedido\n3:   fn nuevo\n5: fn validar_pedido");
        assert!(esquema(Path::new("x.txt"), "hola").starts_with('('));
    }
}

#[cfg(test)]
mod reales {
    /// A MANO, sobre un proyecto de verdad:
    /// `GOTV_MAPA_RAIZ=/ruta cargo test -p gotv-core --release -- --ignored --nocapture mapa_de_un_proyecto_real`
    #[test]
    #[ignore]
    fn mapa_de_un_proyecto_real() {
        let raiz = std::path::PathBuf::from(std::env::var("GOTV_MAPA_RAIZ").expect("GOTV_MAPA_RAIZ"));
        let t = std::time::Instant::now();
        let (archivos, menciones) = super::indexar(&raiz);
        let leer = t.elapsed();
        let simbolos: usize = archivos.iter().map(|a| a.simbolos.len()).sum();
        let t = std::time::Instant::now();
        let m = super::bloque(&raiz, 1_800);
        println!("{m}\n\n— {} archivos con definiciones · {simbolos} definiciones · {} identificadores · leer {leer:?} · mapa completo {:?} · {} caracteres", archivos.len(), menciones.len(), t.elapsed(), m.chars().count());
        if let Ok(q) = std::env::var("GOTV_MAPA_BUSCAR") {
            let t = std::time::Instant::now();
            println!("\nbuscar «{q}» ({:?}):\n{}", t.elapsed(), super::buscar(&raiz, &q, 12));
        }
        assert!(m.chars().count() <= 1_800);
    }
}

#[cfg(test)]
mod congelado {
    /// El bloque del mapa se recupera entero de un mensaje de sistema, en cualquiera de los dos
    /// idiomas, y acaba donde empieza lo siguiente.
    #[test]
    fn el_bloque_se_recupera_del_mensaje_de_sistema() {
        let sistema = "Eres GOTV.\n\nEntorno: macOS\nInstalado: cargo\n\nMapa del código (los archivos…):\n- src/a.rs: fn uno\n- src/b.rs: struct Dos\n(+3 archivos de código más)\n\n# Memoria del proyecto\nreglas";
        assert_eq!(super::bloque_en(sistema), Some("\n\nMapa del código (los archivos…):\n- src/a.rs: fn uno\n- src/b.rs: struct Dos\n(+3 archivos de código más)"));
        assert_eq!(super::bloque_en("x\n\nCode map (the files…):\n- a.rs: fn one"), Some("\n\nCode map (the files…):\n- a.rs: fn one"));
        assert_eq!(super::bloque_en("Eres GOTV.\n\nEntorno: macOS"), None);
    }
}

#[cfg(test)]
mod f21 {
    use super::*;

    fn proyecto(tag: &str) -> crate::CarpetaPrueba {
        let d = crate::CarpetaPrueba::nueva(&format!("gotv-mapa-f21-{tag}"));
        std::fs::create_dir_all(d.join("web/src")).unwrap();
        std::fs::create_dir_all(d.join("api")).unwrap();
        std::fs::write(d.join("package.json"), "{}").unwrap();
        std::fs::write(d.join("web/src/tabla.ts"), "export class TablaPedidos {\n  pintar(filas: number) { return filas; }\n  ordenar() {}\n}\n").unwrap();
        std::fs::write(d.join("web/src/app.ts"), "import { TablaPedidos } from './tabla';\nnew TablaPedidos().pintar(1);\n").unwrap();
        std::fs::write(d.join("api/pagos.py"), "class CobroTarjeta:\n    def cobrar(self):\n        pass\n\ndef validar_tarjeta(n):\n    return n\n").unwrap();
        std::fs::write(d.join("api/usuarios.py"), "def crear_usuario():\n    pass\n").unwrap();
        d
    }

    /// F21 paso 1: la caché `.gotv/mapa.gotv`; cambiar un archivo recalcula SÓLO ése.
    #[test]
    fn la_cache_recalcula_solo_lo_que_cambio() {
        let d = proyecto("cache");
        let (a, _) = indexar(&d);
        assert!(a.iter().any(|x| x.rel == "web/src/tabla.ts" && x.simbolos.iter().any(|s| s.nombre == "pintar" && s.nivel == 1)), "el método sin palabra clave, del árbol");
        let c = archivo_cache(&d);
        assert!(c.is_file());
        // se «marca» la entrada de un archivo que NO cambia: si se recalculara, la marca se perdería
        let mut cache: CacheMapa = crate::programar::codec::leer_serde(&c, b"MAPA").unwrap();
        cache.archivos.get_mut("api/usuarios.py").unwrap().simbolos.push(("fn".into(), "marca_de_la_cache".into(), 9, 0));
        crate::programar::codec::guardar_serde(&c, b"MAPA", &cache).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(d.join("api/pagos.py"), "def otra_funcion():\n    pass\n").unwrap();
        let (a, _) = indexar(&d);
        let de = |r: &str| a.iter().find(|x| x.rel == r).map(|x| x.simbolos.iter().map(|s| s.nombre.clone()).collect::<Vec<_>>()).unwrap_or_default();
        assert!(de("api/usuarios.py").contains(&"marca_de_la_cache".to_string()), "lo que no cambió sale de la caché");
        assert_eq!(de("api/pagos.py"), ["otra_funcion"], "lo que cambió se recalcula");
        // un archivo borrado sale de la caché
        std::fs::remove_file(d.join("api/usuarios.py")).unwrap();
        indexar(&d);
        let cache: CacheMapa = crate::programar::codec::leer_serde(&c, b"MAPA").unwrap();
        assert!(!cache.archivos.contains_key("api/usuarios.py"));
    }

    /// F21 pasos 3 y 4: la herramienta con carpeta, la tarea delante, y el tope nunca se pasa.
    #[test]
    fn la_herramienta_por_carpeta_tarea_y_tope() {
        let d = proyecto("herramienta");
        let todo = herramienta(&d, &serde_json::json!({}));
        assert!(todo.contains("web/src/tabla.ts") && todo.contains("api/pagos.py"), "{todo}");
        let web = herramienta(&d, &serde_json::json!({"carpeta": "web"}));
        assert!(web.contains("tabla.ts") && !web.contains("api/"), "{web}");
        assert!(herramienta(&d, &serde_json::json!({"carpeta": "nada"})).contains("nada"));
        // con un tope que sólo deja pasar UN archivo, el de la tarea es ése
        // la tarea pone delante lo que la nombra: con el tope más chico, ése entra
        let t = herramienta(&d, &serde_json::json!({"tarea": "validar la tarjeta en el cobro", "tokens": 200}));
        assert!(t.contains("api/pagos.py"), "{t}");
        for tokens in [200, 300, 1500] {
            let t = herramienta(&d, &serde_json::json!({"tokens": tokens}));
            assert!(t.chars().count() <= tokens * 4, "{tokens}: {}", t.chars().count());
        }
        let fuera = crate::CarpetaPrueba::nueva("gotv-mapa-f21-noproyecto");
        assert!(herramienta(&fuera, &serde_json::json!({})).contains("no es un proyecto") || herramienta(&fuera, &serde_json::json!({})).contains("not a project"));
        assert!(!fuera.join(".gotv").exists(), "fuera de un proyecto no se escribe caché");
    }
}
