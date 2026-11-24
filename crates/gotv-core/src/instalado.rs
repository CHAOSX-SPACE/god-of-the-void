//! D8e · La documentación de la versión INSTALADA.
//!
//! Un modelo recuerda la API de «alguna» versión de una librería; el proyecto tiene UNA, y es la
//! que manda. Aquí se lee del disco, sin red: la versión exacta (de `node_modules`, del `.venv`,
//! de `Cargo.lock`, de `go.mod`) y la API real de ESA versión, de sus propias fuentes:
//! - npm: los `.d.ts` del paquete (o los de `@types/…`);
//! - Python: el paquete dentro de `site-packages` (firmas y docstrings);
//! - Rust: `~/.cargo/registry/src/*/<crate>-<versión>` (firmas y comentarios `///`);
//! - Go: la caché de módulos.
//!
//! Sin `symbol` da la versión y lo que el paquete expone arriba del todo; con `symbol`, la firma
//! y la documentación de ese nombre tal como está escrita en la versión instalada.
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq)]
pub struct Hallado {
    /// `npm` · `python` · `cargo` · `go` · `composer` · `swift`
    pub eco: &'static str,
    pub version: String,
    /// la carpeta con las fuentes de esa versión (si están en disco)
    pub fuentes: Option<PathBuf>,
    /// el archivo por el que se entra (tipos, `lib.rs`, `__init__.py`)
    pub entrada: Option<PathBuf>,
    /// de dónde salen los tipos si no son del propio paquete (`@types/react 18.3.12`)
    pub nota: String,
}

fn json(p: &Path) -> Option<serde_json::Value> {
    serde_json::from_str(&std::fs::read_to_string(p).ok()?).ok()
}

/// El `.d.ts` por el que entra un paquete de npm, según su `package.json`.
fn tipos_npm(dir: &Path, pj: &serde_json::Value) -> Option<PathBuf> {
    let de_exports = || {
        let e = &pj["exports"];
        let raiz = if e["."].is_object() { &e["."] } else { e };
        raiz["types"].as_str().or(raiz["import"]["types"].as_str()).or(raiz["require"]["types"].as_str()).or(raiz["default"]["types"].as_str())
    };
    let dicho = pj["types"].as_str().or(pj["typings"].as_str()).or_else(de_exports).map(|t| dir.join(t.trim_start_matches("./")));
    let junto_a_main = || pj["main"].as_str().map(|m| dir.join(m.trim_start_matches("./")).with_extension("d.ts"));
    dicho.filter(|p| p.is_file()).or_else(|| Some(dir.join("index.d.ts")).filter(|p| p.is_file())).or_else(|| junto_a_main().filter(|p| p.is_file()))
}

fn npm(root: &Path, nombre: &str) -> Vec<Hallado> {
    // la raíz y, en un monorepo, sus paquetes (apps/x, packages/x)
    let mut bases = vec![root.to_path_buf()];
    for grupo in ["apps", "packages", "services", "libs"] {
        if let Ok(rd) = std::fs::read_dir(root.join(grupo)) {
            let mut hijos: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
            hijos.sort();
            bases.extend(hijos);
        }
    }
    let mut out: Vec<Hallado> = Vec::new();
    for b in bases {
        let nm = b.join("node_modules");
        let dir = nm.join(nombre);
        let Some(pj) = json(&dir.join("package.json")) else { continue };
        let version = pj["version"].as_str().unwrap_or("?").to_string();
        if out.iter().any(|h| h.version == version) {
            continue;
        }
        let mut h = Hallado { eco: "npm", version, fuentes: Some(dir.clone()), entrada: tipos_npm(&dir, &pj), nota: String::new() };
        if h.entrada.is_none() {
            // sin tipos propios: los de DefinitelyTyped, si están (`@scope/x` → `@types/scope__x`)
            let t = nm.join("@types").join(nombre.trim_start_matches('@').replace('/', "__"));
            if let Some(tj) = json(&t.join("package.json")) {
                h.nota = format!("@types {}", tj["version"].as_str().unwrap_or("?"));
                h.entrada = tipos_npm(&t, &tj);
                h.fuentes = Some(t);
            }
        }
        out.push(h);
        if out.len() >= 3 {
            break;
        }
    }
    out
}

fn normal_py(n: &str) -> String {
    n.to_lowercase().replace(['-', '.'], "_")
}

fn python(root: &Path, nombre: &str) -> Vec<Hallado> {
    let busco = normal_py(nombre);
    let mut out = Vec::new();
    for venv in [".venv", "venv", "env"] {
        let Ok(libs) = std::fs::read_dir(root.join(venv).join("lib")) else { continue };
        for lib in libs.flatten() {
            let sp = lib.path().join("site-packages");
            let Ok(rd) = std::fs::read_dir(&sp) else { continue };
            for e in rd.flatten() {
                let n = e.file_name().to_string_lossy().to_string();
                let Some(base) = n.strip_suffix(".dist-info") else { continue };
                let Some((dist, version)) = base.rsplit_once('-') else { continue };
                if normal_py(dist) != busco {
                    continue;
                }
                // el nombre con el que se IMPORTA puede no ser el de la distribución (PyYAML → yaml)
                let importa = std::fs::read_to_string(e.path().join("top_level.txt")).ok().and_then(|t| t.lines().map(str::trim).find(|l| !l.is_empty() && !l.starts_with('_')).map(str::to_string)).unwrap_or_else(|| busco.clone());
                let (dir, suelto) = (sp.join(&importa), sp.join(format!("{importa}.py")));
                let (fuentes, entrada) = if dir.is_dir() {
                    let ini = ["__init__.pyi", "__init__.py"].iter().map(|f| dir.join(f)).find(|p| p.is_file());
                    (Some(dir), ini)
                } else if suelto.is_file() {
                    (Some(sp.clone()), Some(suelto))
                } else {
                    (None, None)
                };
                out.push(Hallado { eco: "python", version: version.to_string(), fuentes, entrada, nota: if importa != busco { format!("import {importa}") } else { String::new() } });
                return out;
            }
        }
    }
    out
}

/// Las versiones de `nombre` en un `Cargo.lock` (puede haber varias a la vez).
fn versiones_cargo(lock: &str, nombre: &str) -> Vec<String> {
    let igual = |a: &str| a.replace('-', "_") == nombre.replace('-', "_");
    let valor = |l: &str, k: &str| l.strip_prefix(k).map(|v| v.trim().trim_matches('"').to_string());
    let mut out = Vec::new();
    for bloque in lock.split("[[package]]").skip(1) {
        let mut n = None;
        let mut v = None;
        for l in bloque.lines().map(str::trim) {
            n = n.or(valor(l, "name ="));
            v = v.or(valor(l, "version ="));
        }
        if let (Some(n), Some(v)) = (n, v) {
            if igual(&n) && !out.contains(&v) {
                out.push(v);
            }
        }
    }
    out
}

fn cargo(root: &Path, nombre: &str, casa: &Path) -> Vec<Hallado> {
    // el Cargo.lock puede estar en la raíz del espacio de trabajo, por encima de la carpeta
    let lock = root.ancestors().take(4).map(|a| a.join("Cargo.lock")).find(|p| p.is_file()).and_then(|p| std::fs::read_to_string(p).ok());
    let Some(lock) = lock else { return vec![] };
    let registros: Vec<PathBuf> = std::fs::read_dir(casa.join("registry/src")).map(|rd| rd.flatten().map(|e| e.path()).collect()).unwrap_or_default();
    versiones_cargo(&lock, nombre)
        .into_iter()
        .take(3)
        .map(|version| {
            let fuentes = registros.iter().flat_map(|r| [nombre.to_string(), nombre.replace('_', "-"), nombre.replace('-', "_")].map(|n| r.join(format!("{n}-{version}")))).find(|p| p.is_dir());
            let entrada = fuentes.as_ref().map(|f| f.join("src/lib.rs")).filter(|p| p.is_file());
            Hallado { eco: "cargo", version, fuentes, entrada, nota: String::new() }
        })
        .collect()
}

fn go(root: &Path, nombre: &str, cache: &Path) -> Vec<Hallado> {
    let Ok(m) = std::fs::read_to_string(root.join("go.mod")) else { return vec![] };
    for l in m.lines().map(|l| l.trim().trim_start_matches("require ").trim()) {
        let mut partes = l.split_whitespace();
        let (Some(modulo), Some(version)) = (partes.next(), partes.next()) else { continue };
        if !version.starts_with('v') || !(modulo == nombre || modulo.ends_with(&format!("/{nombre}"))) {
            continue;
        }
        // en la caché, una mayúscula se escribe `!minúscula`
        let escapado: String = modulo.chars().flat_map(|c| if c.is_uppercase() { vec!['!', c.to_ascii_lowercase()] } else { vec![c] }).collect();
        let fuentes = Some(cache.join(format!("{escapado}@{version}"))).filter(|p| p.is_dir());
        return vec![Hallado { eco: "go", version: version.to_string(), fuentes, entrada: None, nota: if modulo != nombre { modulo.to_string() } else { String::new() } }];
    }
    vec![]
}

/// F20.2 · PHP: la versión de `composer.lock` y las fuentes en `vendor/<proveedor>/<paquete>`. `nombre` vale
/// entero (`guzzlehttp/guzzle`) o sólo el paquete (`guzzle`).
fn composer(root: &Path, nombre: &str) -> Vec<Hallado> {
    let Some(j) = json(&root.join("composer.lock")) else { return vec![] };
    let mut out = Vec::new();
    for p in ["packages", "packages-dev"].iter().flat_map(|k| j[k].as_array().into_iter().flatten()) {
        let Some(n) = p["name"].as_str() else { continue };
        if n != nombre && n.rsplit('/').next() != Some(nombre) {
            continue;
        }
        let version = p["version"].as_str().unwrap_or("?").trim_start_matches('v').to_string();
        let fuentes = Some(root.join("vendor").join(n)).filter(|d| d.is_dir());
        out.push(Hallado { eco: "composer", version, fuentes, entrada: None, nota: if n != nombre { n.to_string() } else { String::new() } });
    }
    out
}

/// F20.2 · Swift: la versión de `Package.resolved` (formatos 1, 2 y 3) y las fuentes en `.build/checkouts/`.
fn swift(root: &Path, nombre: &str) -> Vec<Hallado> {
    let Some(j) = json(&root.join("Package.resolved")) else { return vec![] };
    let pins = j["pins"].as_array().or(j["object"]["pins"].as_array());
    let bajo = nombre.to_lowercase();
    for p in pins.into_iter().flatten() {
        // v2/v3: identity + location; v1: package + repositoryURL
        let id = p["identity"].as_str().or(p["package"].as_str()).unwrap_or("").to_lowercase();
        let url = p["location"].as_str().or(p["repositoryURL"].as_str()).unwrap_or("");
        let repo = url.trim_end_matches('/').trim_end_matches(".git").rsplit('/').next().unwrap_or("").to_string();
        if id != bajo && repo.to_lowercase() != bajo {
            continue;
        }
        let version = p["state"]["version"].as_str().or(p["state"]["revision"].as_str()).unwrap_or("?").to_string();
        let co = root.join(".build/checkouts");
        let fuentes = [repo.clone(), id.clone()].into_iter().map(|n| co.join(n)).find(|d| d.is_dir());
        return vec![Hallado { eco: "swift", version, fuentes, entrada: None, nota: url.to_string() }];
    }
    vec![]
}

/// F20.2 · un EJEMPLO del propio paquete que use `simbolo`: el primer bloque de código de su README que lo
/// nombra, o el primer archivo de `examples/` que lo usa (alrededor de esa línea). Máximo 15 líneas.
fn ejemplo(dir: &Path, simbolo: &str) -> Option<(String, String)> {
    for readme in ["README.md", "readme.md", "README.markdown", "Readme.md"] {
        let Ok(t) = std::fs::read_to_string(dir.join(readme)) else { continue };
        let mut dentro = false;
        let mut bloque: Vec<&str> = Vec::new();
        for l in t.lines() {
            if l.trim_start().starts_with("```") {
                if dentro && bloque.iter().any(|b| nombra(b, simbolo)) {
                    return Some((readme.to_string(), bloque.iter().take(15).copied().collect::<Vec<_>>().join("\n")));
                }
                dentro = !dentro;
                bloque.clear();
            } else if dentro {
                bloque.push(l);
            }
        }
    }
    let mut lista = Vec::new();
    for d in ["examples", "example", "Examples"] {
        archivos_todos(&dir.join(d), 0, &mut lista);
    }
    lista.sort();
    for p in lista {
        let Ok(t) = std::fs::read_to_string(&p) else { continue };
        let lineas: Vec<&str> = t.lines().collect();
        if let Some(i) = lineas.iter().position(|l| nombra(l, simbolo)) {
            let desde = i.saturating_sub(4);
            let rel = p.strip_prefix(dir).unwrap_or(&p).display().to_string();
            return Some((rel, lineas[desde..(desde + 15).min(lineas.len())].join("\n")));
        }
    }
    None
}

/// Los archivos de código de una carpeta de ejemplos (sin filtrar por ecosistema).
fn archivos_todos(dir: &Path, fondo: usize, out: &mut Vec<PathBuf>) {
    if fondo > 3 || out.len() >= 200 {
        return;
    }
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let p = e.path();
        if p.is_dir() {
            archivos_todos(&p, fondo + 1, out);
        } else if p.extension().is_some_and(|x| matches!(x.to_str(), Some("rs" | "py" | "js" | "mjs" | "ts" | "tsx" | "go" | "php" | "swift"))) {
            out.push(p);
        }
    }
}

/// Dónde está `nombre` en este proyecto, en todos los ecosistemas que tenga.
pub fn hallar(root: &Path, nombre: &str) -> Vec<Hallado> {
    let casa = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    let cargo_casa = std::env::var_os("CARGO_HOME").map(PathBuf::from).unwrap_or_else(|| casa.join(".cargo"));
    let go_cache = std::env::var_os("GOMODCACHE").map(PathBuf::from).unwrap_or_else(|| std::env::var_os("GOPATH").map(PathBuf::from).unwrap_or_else(|| casa.join("go")).join("pkg/mod"));
    let mut out = npm(root, nombre);
    out.extend(python(root, nombre));
    out.extend(cargo(root, nombre, &cargo_casa));
    out.extend(go(root, nombre, &go_cache));
    out.extend(composer(root, nombre));
    out.extend(swift(root, nombre));
    out
}

const FUERA: &[&str] = &["node_modules", "tests", "test", "__tests__", "benches", "examples", "docs", "__pycache__", "target", "testdata"];

fn archivos(dir: &Path, exts: &[&str], fondo: usize, out: &mut Vec<PathBuf>) {
    if fondo > 7 || out.len() >= 4_000 {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut entradas: Vec<_> = rd.flatten().collect();
    entradas.sort_by_key(|e| e.file_name());
    for e in entradas {
        let n = e.file_name().to_string_lossy().to_string();
        let Ok(ft) = e.file_type() else { continue };
        if ft.is_dir() {
            if !n.starts_with('.') && !FUERA.contains(&n.as_str()) {
                archivos(&e.path(), exts, fondo + 1, out);
            }
        } else if ft.is_file() && exts.iter().any(|x| n.ends_with(x)) && e.metadata().map(|m| m.len() <= 600_000).unwrap_or(false) {
            out.push(e.path());
        }
    }
}

fn extensiones(eco: &str) -> &'static [&'static str] {
    match eco {
        "npm" => &[".d.ts", ".d.mts", ".d.cts"],
        "python" => &[".pyi", ".py"],
        "cargo" => &[".rs"],
        "composer" => &[".php"],
        "swift" => &[".swift"],
        _ => &[".go"],
    }
}

/// La documentación y la firma de la definición de la línea `i` (desde 0): los comentarios de
/// encima, la firma hasta su `{`, `;` o `:`, el cuerpo si es un tipo, y el docstring en Python.
fn extracto(lineas: &[&str], i: usize, clase: &str, python: bool) -> String {
    let sangria = |l: &str| l.len() - l.trim_start().len();
    let mut desde = i;
    while desde > 0 && i - desde < 200 {
        let a = lineas[desde - 1].trim_start();
        if a.starts_with("///") || a.starts_with("//") || a.starts_with('*') || a.starts_with("/**") || a.starts_with("#[") || a.starts_with('@') || (python && a.starts_with('#')) {
            desde -= 1;
        } else {
            break;
        }
    }
    let mut hasta = i;
    while hasta + 1 < lineas.len() && hasta - i < 12 {
        let l = lineas[hasta].trim_end();
        if l.contains('{') || l.ends_with(';') || (python && l.ends_with(':')) || (!python && l.ends_with('}')) {
            break;
        }
        hasta += 1;
    }
    let tipo = matches!(clase, "struct" | "enum" | "trait" | "interface" | "class" | "type" | "union" | "record" | "protocol");
    let abre = lineas[hasta].contains('{') && !lineas[hasta].trim_end().ends_with('}');
    if tipo && !python && abre {
        // el cuerpo de un tipo ES su API: hasta su llave de cierre (o 40 líneas)
        let base = sangria(lineas[i]);
        while hasta + 1 < lineas.len() && hasta - i < 40 {
            hasta += 1;
            if lineas[hasta].trim_start().starts_with('}') && sangria(lineas[hasta]) == base {
                break;
            }
        }
    }
    if python && hasta + 1 < lineas.len() {
        let d = lineas[hasta + 1].trim_start();
        if let Some(comillas) = ["\"\"\"", "'''"].into_iter().find(|c| d.starts_with(c)) {
            hasta += 1;
            let una_linea = d.len() > 3 && d[3..].contains(comillas);
            while !una_linea && hasta + 1 < lineas.len() && hasta - i < 30 {
                hasta += 1;
                if lineas[hasta].contains(comillas) {
                    break;
                }
            }
        }
    }
    // una doc larga (ejemplos, secciones) conserva su COMIENZO —la frase que dice qué hace— y se
    // dice cuánto se quitó: lo que se pierde por abajo son ejemplos; por arriba, el sentido
    const DOC: usize = 16;
    let fin = hasta.min(lineas.len() - 1);
    let texto = if i - desde > DOC + 4 {
        // los atributos pegados a la firma (`#[inline]`, `@decorador`) se quedan con ella
        let mut firma = i;
        while firma > desde + DOC && (lineas[firma - 1].trim_start().starts_with("#[") || lineas[firma - 1].trim_start().starts_with('@')) {
            firma -= 1;
        }
        let quitadas = firma - (desde + DOC);
        let elision = crate::tr!("… (+{quitadas} líneas de documentación: ejemplos y detalles)", "… (+{quitadas} more documentation lines: examples and details)");
        format!("{}\n{elision}\n{}", lineas[desde..desde + DOC].join("\n"), lineas[firma..=fin].join("\n"))
    } else {
        lineas[desde..=fin].join("\n")
    };
    if texto.chars().count() > 2_400 {
        format!("{}\n…", texto.chars().take(2_400).collect::<String>())
    } else {
        texto
    }
}

fn es_palabra(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// ¿Aparece `simbolo` en `l` como palabra entera?
fn nombra(l: &str, simbolo: &str) -> bool {
    l.match_indices(simbolo).any(|(i, _)| !l[..i].chars().next_back().is_some_and(es_palabra) && !l[i + simbolo.len()..].chars().next().is_some_and(es_palabra))
}

/// Las definiciones de un archivo de las fuentes. F20.2: los `.d.ts`, PHP y Swift por el ÁRBOL nativo (ve los
/// miembros de una interfaz y los métodos sin palabra clave); Rust, Python y Go por la forma de la línea,
/// que ya ordena lo público y sus `impl` como se midió con paquetes reales.
fn simbolos_de(eco: &str, p: &Path, ext: &str, texto: &str) -> Vec<crate::mapa::Simbolo> {
    match eco {
        "npm" | "composer" | "swift" => crate::mapa::simbolos_por_arbol(p, texto).unwrap_or_else(|| crate::mapa::simbolos(ext, texto)),
        _ => crate::mapa::simbolos(ext, texto),
    }
}

/// La firma y la doc de `simbolo` en las fuentes de una versión instalada.
fn del_simbolo(h: &Hallado, simbolo: &str) -> String {
    let Some(dir) = &h.fuentes else {
        return crate::i18n::t("(sus fuentes no están en disco: instala las dependencias del proyecto)", "(its sources are not on disk: install the project's dependencies)").into();
    };
    let mut lista = Vec::new();
    archivos(dir, extensiones(h.eco), 0, &mut lista);
    // primero lo de más arriba (la API pública suele estar cerca de la raíz del paquete)
    lista.sort_by_key(|p| (p.components().count(), p.clone()));
    let python = h.eco == "python";
    // (orden, texto): primero lo público y de nivel superior — medido con serde_json y rusqlite
    // (2026-11-14): sin ordenar, dos métodos `from_str` y veinte `impl Connection` tapaban a la
    // función y al tipo que se buscaban
    let mut definiciones: Vec<((usize, bool, usize), String)> = Vec::new();
    let mut impls: Vec<String> = Vec::new();
    let mut menciones: Vec<String> = Vec::new();
    for (prof, p) in lista.iter().enumerate() {
        let Ok(texto) = std::fs::read_to_string(p) else { continue };
        if !texto.contains(simbolo) {
            continue;
        }
        let rel = p.strip_prefix(dir).unwrap_or(p).display().to_string();
        let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("");
        let lineas: Vec<&str> = texto.lines().collect();
        let mut aqui = false;
        for s in simbolos_de(h.eco, p, ext, &texto) {
            if s.nombre != simbolo && s.nombre.rsplit('.').next() != Some(simbolo) {
                continue;
            }
            aqui = true;
            if s.clase == "impl" || s.clase == "mod" {
                impls.push(format!("{rel}:{}", s.linea));
                continue;
            }
            let l = lineas[s.linea - 1].trim_start();
            let privado = !(l.starts_with("pub ") || l.starts_with("pub(") || l.starts_with("export ")) && matches!(h.eco, "cargo" | "npm");
            if definiciones.len() < 40 {
                definiciones.push(((s.nivel, privado, prof), format!("── {rel}:{}\n{}", s.linea, extracto(&lineas, s.linea - 1, s.clase, python))));
            }
        }
        // lo que el extractor no ve (miembros de una interfaz, métodos sin palabra clave)
        if !aqui && menciones.len() < 8 {
            for (i, l) in lineas.iter().enumerate() {
                let t = l.trim();
                let declara = nombra(t, simbolo) && !t.starts_with("//") && !t.starts_with('*') && !t.starts_with("import ") && !t.starts_with("use ") && !t.starts_with('#');
                if declara && t.len() < 300 {
                    menciones.push(format!("{rel}:{}: {t}", i + 1));
                    if menciones.len() >= 8 {
                        break;
                    }
                }
            }
        }
    }
    if !definiciones.is_empty() || !impls.is_empty() {
        definiciones.sort_by(|a, b| a.0.cmp(&b.0));
        // la misma declaración repetida (los tipos de React para cada versión de TypeScript) cuenta una vez
        let cuerpo = |t: &str| t.split_once('\n').map(|(_, c)| c.to_string()).unwrap_or_default();
        let mut vistos = std::collections::HashSet::new();
        definiciones.retain(|d| vistos.insert(cuerpo(&d.1)));
        let de_mas = definiciones.len().saturating_sub(3);
        let mut s = definiciones.iter().take(3).map(|d| d.1.as_str()).collect::<Vec<_>>().join("\n\n");
        if de_mas > 0 {
            s.push_str(&crate::tr!("\n\n(+{de_mas} definiciones más con ese nombre)", "\n\n(+{de_mas} more definitions with that name)"));
        }
        if !impls.is_empty() {
            // sus métodos están en sus `impl`: se dice dónde, no se vuelcan
            let n = impls.len();
            let donde = impls.into_iter().take(8).collect::<Vec<_>>().join(", ");
            s.push_str(&crate::tr!("\n\nsus métodos (impl), en: {donde}{}", "\n\nits methods (impl), in: {donde}{}", if n > 8 { format!(" (+{})", n - 8) } else { String::new() }));
        }
        // F20.2: cómo lo usa el PROPIO paquete (de su README o de sus ejemplos), en esta versión
        if let Some((de, ej)) = ejemplo(dir, simbolo) {
            s.push_str(&crate::tr!("\n\nejemplo del propio paquete ({de}):\n{ej}", "\n\nexample from the package itself ({de}):\n{ej}"));
        }
        s.trim_start().to_string()
    } else if !menciones.is_empty() {
        crate::tr!("(sin una definición con palabra clave; aparece declarado aquí:)\n{}", "(no keyword definition; it is declared here:)\n{}", menciones.join("\n"))
    } else {
        crate::tr!("«{simbolo}» NO existe en esta versión: no lo uses de memoria. Mira lo que sí expone (la misma consulta sin `symbol`).", "«{simbolo}» does NOT exist in this version: do not use it from memory. See what it does expose (the same query without `symbol`).")
    }
}

/// Lo que el paquete expone arriba del todo, en una lista compacta.
fn expone(h: &Hallado) -> String {
    let Some(entrada) = &h.entrada else { return String::new() };
    let Ok(texto) = std::fs::read_to_string(entrada) else { return String::new() };
    let ext = entrada.extension().and_then(|e| e.to_str()).unwrap_or("");
    let mut vistos = std::collections::HashSet::new();
    let nombres: Vec<String> = simbolos_de(h.eco, entrada, ext, &texto).into_iter().filter(|s| s.nivel == 0 && s.clase != "impl" && !s.nombre.starts_with('_') && vistos.insert(s.nombre.clone())).map(|s| format!("{} {}", s.clase, s.nombre)).collect();
    if nombres.is_empty() {
        return String::new();
    }
    let total = nombres.len();
    let mut s = nombres.into_iter().take(60).collect::<Vec<_>>().join(", ");
    if total > 60 {
        s.push_str(&format!(" (+{})", total - 60));
    }
    s
}

/// La respuesta para el modelo. `simbolo` vacío = sólo versión y lo que expone.
pub fn consultar(root: &Path, nombre: &str, simbolo: &str) -> String {
    let nombre = nombre.trim();
    let hallados = hallar(root, nombre);
    if hallados.is_empty() {
        return crate::tr!(
            "«{nombre}» NO está instalado en este proyecto (miré node_modules, .venv, Cargo.lock, go.mod, composer.lock y Package.resolved). Si hace falta, instálalo; para ver la última versión publicada usa registry npm, pypi o crates.",
            "«{nombre}» is NOT installed in this project (I looked in node_modules, .venv, Cargo.lock, go.mod, composer.lock and Package.resolved). Install it if needed; for the latest published version use registry npm, pypi or crates."
        );
    }
    let rel = |p: &Path| p.strip_prefix(root).map(|r| r.display().to_string()).unwrap_or_else(|_| p.display().to_string());
    let mut out = Vec::new();
    for h in &hallados {
        let mut s = crate::tr!("{} {nombre} {} — la versión INSTALADA en este proyecto", "{} {nombre} {} — the version INSTALLED in this project", h.eco, h.version);
        if !h.nota.is_empty() {
            s.push_str(&format!(" ({})", h.nota));
        }
        match &h.fuentes {
            Some(f) => s.push_str(&crate::tr!("\nfuentes: {}", "\nsources: {}", rel(f))),
            None => s.push_str(crate::i18n::t("\nfuentes: no están en disco (dependencia local o sin descargar)", "\nsources: not on disk (local or not yet downloaded dependency)")),
        }
        if simbolo.trim().is_empty() {
            let e = expone(h);
            if !e.is_empty() {
                // la entrada, relativa a las fuentes (que ya se dijeron arriba)
                let donde = h.entrada.as_deref().map(|p| h.fuentes.as_deref().and_then(|f| p.strip_prefix(f).ok()).map(|r| r.display().to_string()).unwrap_or_else(|| rel(p))).unwrap_or_default();
                s.push_str(&crate::tr!("\nexpone ({donde}): {e}", "\nexposes ({donde}): {e}"));
            }
            s.push_str(crate::i18n::t("\nPara la firma y la doc de un nombre: la misma consulta con `symbol`.", "\nFor a name's signature and docs: the same query with `symbol`."));
        } else {
            s.push('\n');
            s.push_str(&del_simbolo(h, simbolo.trim()));
        }
        out.push(s);
    }
    out.join("\n\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn escribir(p: &Path, t: &str) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, t).unwrap();
    }

    /// F20.2 · PHP (composer.lock + vendor/) y Swift (Package.resolved + .build/checkouts/), por el árbol
    /// nativo, con el ejemplo del propio paquete.
    #[test]
    fn php_y_swift_con_su_version_y_su_ejemplo() {
        let d = crate::CarpetaPrueba::nueva("gotv-instalado-php-swift");
        escribir(&d.join("composer.lock"), r#"{"packages":[{"name":"guzzlehttp/guzzle","version":"7.8.1"}],"packages-dev":[]}"#);
        escribir(&d.join("vendor/guzzlehttp/guzzle/src/Client.php"), "<?php\nnamespace GuzzleHttp;\n\nclass Client\n{\n    /**\n     * Envía una petición HTTP.\n     */\n    public function send(RequestInterface $request, array $options = []): ResponseInterface\n    {\n        return $this->sendAsync($request, $options)->wait();\n    }\n}\n");
        escribir(&d.join("vendor/guzzlehttp/guzzle/README.md"), "# Guzzle\n\n```php\n$client = new GuzzleHttp\\Client();\n$response = $client->send($request);\n```\n");
        let t = consultar(&d, "guzzle", "send");
        assert!(t.contains("composer guzzle 7.8.1") && t.contains("guzzlehttp/guzzle"), "{t}");
        assert!(t.contains("src/Client.php:9") && t.contains("public function send(RequestInterface $request"), "{t}");
        assert!(t.contains("README.md") && t.contains("$client->send($request)"), "el ejemplo del propio paquete: {t}");
        assert!(consultar(&d, "guzzle", "noExiste").contains("NO existe en esta versión") || consultar(&d, "guzzle", "noExiste").contains("does NOT exist"));
        // Swift: Package.resolved v2 y su checkout
        escribir(&d.join("Package.resolved"), r#"{"pins":[{"identity":"swift-argument-parser","kind":"remoteSourceControl","location":"https://github.com/apple/swift-argument-parser.git","state":{"revision":"abc","version":"1.3.0"}}],"version":2}"#);
        escribir(&d.join(".build/checkouts/swift-argument-parser/Sources/ArgumentParser/ParsableCommand.swift"), "/// Un comando de la línea de órdenes.\npublic protocol ParsableCommand {\n    func run() throws\n}\n");
        let t = consultar(&d, "swift-argument-parser", "ParsableCommand");
        assert!(t.contains("swift swift-argument-parser 1.3.0") && t.contains("ParsableCommand.swift:2") && t.contains("public protocol ParsableCommand"), "{t}");
        // v1 (`object.pins`, `package`, `repositoryURL`)
        escribir(&d.join("Package.resolved"), r#"{"object":{"pins":[{"package":"ArgumentParser","repositoryURL":"https://github.com/apple/swift-argument-parser","state":{"version":"1.2.0"}}]},"version":1}"#);
        assert!(consultar(&d, "swift-argument-parser", "").contains("1.2.0"));
    }

    #[test]
    fn npm_lee_la_version_y_los_tipos_del_paquete_o_de_types() {
        let d = crate::CarpetaPrueba::sin_crear("gotv-instalado-npm");
        escribir(&d.join("node_modules/fechas/package.json"), r#"{"name":"fechas","version":"3.2.1","types":"./dist/index.d.ts"}"#);
        escribir(
            &d.join("node_modules/fechas/dist/index.d.ts"),
            "/**\n * Suma días a una fecha.\n * @param dias cuántos\n */\nexport declare function sumarDias(fecha: Date, dias: number): Date;\nexport interface Opciones {\n  zona: string;\n  formatear(fecha: Date): string;\n}\n",
        );
        escribir(&d.join("node_modules/viejo/package.json"), r#"{"name":"viejo","version":"1.0.0","main":"index.js"}"#);
        escribir(&d.join("node_modules/@types/viejo/package.json"), r#"{"name":"@types/viejo","version":"1.0.4","types":"index.d.ts"}"#);
        escribir(&d.join("node_modules/@types/viejo/index.d.ts"), "export function antiguo(): void;\n");
        let r = consultar(&d, "fechas", "");
        assert!(r.starts_with("npm fechas 3.2.1") && r.contains("node_modules/fechas") && r.contains("function sumarDias, interface Opciones"), "{r}");
        let r = consultar(&d, "fechas", "sumarDias");
        assert!(r.contains("── dist/index.d.ts:5") && r.contains("Suma días a una fecha.") && r.contains("sumarDias(fecha: Date, dias: number): Date;"), "{r}");
        // el cuerpo de una interfaz es su API
        let r = consultar(&d, "fechas", "Opciones");
        assert!(r.contains("formatear(fecha: Date): string;") && r.trim_end().ends_with('}'), "{r}");
        // un miembro sin palabra clave: F20.2, el árbol lo ve como DEFINICIÓN (antes, sólo una mención)
        let r = consultar(&d, "fechas", "formatear");
        assert!(r.contains("── dist/index.d.ts:8") && r.contains("formatear(fecha: Date): string;"), "{r}");
        // lo que no existe en ESTA versión se dice sin rodeos
        assert!(consultar(&d, "fechas", "restarMeses").contains("NO existe"));
        let v = consultar(&d, "viejo", "antiguo");
        assert!(v.contains("npm viejo 1.0.0") && v.contains("@types 1.0.4") && v.contains("export function antiguo(): void;"), "{v}");
        assert!(consultar(&d, "fantasma", "").contains("NO está instalado"));
    }

    #[test]
    fn python_cargo_y_go() {
        let d = crate::CarpetaPrueba::sin_crear("gotv-instalado-otros");
        // Python: la distribución PyYAML se importa como `yaml`
        let sp = d.join(".venv/lib/python3.12/site-packages");
        escribir(&sp.join("PyYAML-6.0.2.dist-info/top_level.txt"), "_yaml\nyaml\n");
        escribir(&sp.join("yaml/__init__.py"), "import os\n\ndef safe_load(stream):\n    \"\"\"\n    Parse the first YAML document in a stream.\n    \"\"\"\n    return load(stream)\n\nclass Loader:\n    pass\n");
        let h = hallar(&d, "pyyaml");
        assert_eq!((h[0].eco, h[0].version.as_str(), h[0].nota.as_str()), ("python", "6.0.2", "import yaml"));
        let r = consultar(&d, "PyYAML", "safe_load");
        assert!(r.contains("python PyYAML 6.0.2") && r.contains("── __init__.py:3") && r.contains("Parse the first YAML document") && !r.contains("return load"), "{r}");
        assert!(consultar(&d, "PyYAML", "").contains("def safe_load, class Loader"));

        // Rust: dos versiones a la vez en el lock; las fuentes, en el registro
        escribir(&d.join("Cargo.lock"), "version = 3\n\n[[package]]\nname = \"serde_json\"\nversion = \"1.0.132\"\nsource = \"registry\"\n\n[[package]]\nname = \"otro\"\nversion = \"0.1.0\"\n\n[[package]]\nname = \"serde_json\"\nversion = \"0.9.0\"\n");
        let casa = d.join("cargo-casa");
        escribir(&casa.join("registry/src/index.crates.io-abc/serde_json-1.0.132/src/lib.rs"), "/// Deserialize an instance of type `T` from a string of JSON text.\n#[inline]\npub fn from_str<'a, T>(s: &'a str) -> Result<T>\nwhere\n    T: de::Deserialize<'a>,\n{\n    from_trait(read::StrRead::new(s))\n}\n");
        let c = cargo(&d.join("crates/x"), "serde-json", &casa);
        assert_eq!(c.iter().map(|h| h.version.as_str()).collect::<Vec<_>>(), ["1.0.132", "0.9.0"]);
        assert!(c[0].entrada.is_some() && c[1].fuentes.is_none());
        let f = del_simbolo(&c[0], "from_str");
        assert!(f.starts_with("── src/lib.rs:3\n/// Deserialize an instance") && f.contains("T: de::Deserialize<'a>,") && !f.contains("from_trait"), "{f}");
        assert!(del_simbolo(&c[1], "from_str").starts_with('('));

        // Go: la mayúscula del módulo se escapa en la caché
        escribir(&d.join("go.mod"), "module demo\n\nrequire (\n\tgithub.com/BurntSushi/toml v1.4.0\n)\n");
        let cache = d.join("gomod");
        escribir(&cache.join("github.com/!burnt!sushi/toml@v1.4.0/decode.go"), "package toml\n\n// Decode the TOML data in to the pointer v.\nfunc Decode(data string, v any) (MetaData, error) {\n\treturn MetaData{}, nil\n}\n");
        let g = go(&d, "toml", &cache);
        assert_eq!((g[0].version.as_str(), g[0].nota.as_str()), ("v1.4.0", "github.com/BurntSushi/toml"));
        let f = del_simbolo(&g[0], "Decode");
        assert!(f.contains("── decode.go:4") && f.contains("// Decode the TOML data") && f.contains("func Decode(data string, v any) (MetaData, error) {"), "{f}");
    }
}

#[cfg(test)]
mod reales {
    /// A MANO, sobre un proyecto de verdad:
    /// `GOTV_INST_RAIZ=/ruta GOTV_INST=paquete[:símbolo] cargo test -p gotv-core --release -- --ignored --nocapture instalado_de_verdad`
    #[test]
    #[ignore]
    fn instalado_de_verdad() {
        let raiz = std::path::PathBuf::from(std::env::var("GOTV_INST_RAIZ").expect("GOTV_INST_RAIZ"));
        for pedido in std::env::var("GOTV_INST").expect("GOTV_INST").split(',') {
            let (n, s) = pedido.split_once(':').unwrap_or((pedido, ""));
            let t = std::time::Instant::now();
            let r = super::consultar(&raiz, n, s);
            println!("══ {pedido} ({:?}, {} caracteres)\n{}\n", t.elapsed(), r.chars().count(), r.chars().take(1_500).collect::<String>());
        }
    }
}
