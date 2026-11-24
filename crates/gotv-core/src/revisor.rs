//! F5 · Motor de lenguaje NATIVO: el revisor de GOTV, en Rust, sin servidores externos.
//!
//! Medido (banco aislado «biblioteca», Qwen3.6-35B-A3B, 2026-11-12): en Python el agente
//! dejaba imports sin poner (`Almacen`, `ErrorPrestamo`) y sólo lo descubría al correr las
//! pruebas, porque el servidor de lenguaje (pyright/pylsp) no estaba instalado y el proyecto
//! no tenía pyproject.toml: 0 diagnósticos en toda la corrida. Este revisor va SIEMPRE:
//! tras cada edición de un .py mira, con el árbol de tree-sitter,
//!   1. la sintaxis (nodos de error y piezas que faltan),
//!   2. los nombres que no se definen ni se importan en el archivo,
//!   3. los imports del PROYECTO que no existen (módulo o nombre), y
//!   4. los archivos del proyecto que importaban de éste algo que ya no está.
//! Si hay un servidor completo instalado, el agente usa ése (tipos y navegación); éste es el
//! piso que nunca falta. Prefiere callar a inventar: ante la duda, no se señala.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use tree_sitter::{Node, Parser, Tree};

/// Un problema con su línea (1-based).
#[derive(Debug, Clone, PartialEq)]
pub struct Problema {
    pub ruta: PathBuf,
    pub linea: usize,
    pub mensaje: String,
}

/// ¿Este archivo lo revisa el motor nativo?
pub fn soporta(p: &Path) -> bool {
    matches!(p.extension().and_then(|e| e.to_str()), Some("py" | "pyi"))
}

/// Lenguajes que el motor nativo cubre hoy (para el bloque de entorno).
pub const NATIVOS: &[&str] = &["Python"];

/// Nombres que Python da sin importar (builtins 3.13 + los dunder de un módulo).
const BUILTINS: &[&str] = &[
    "abs", "aiter", "all", "anext", "any", "ascii", "bin", "bool", "breakpoint", "bytearray",
    "bytes", "callable", "chr", "classmethod", "compile", "complex", "copyright", "credits",
    "delattr", "dict", "dir", "divmod", "enumerate", "eval", "exec", "exit", "filter", "float",
    "format", "frozenset", "getattr", "globals", "hasattr", "hash", "help", "hex", "id", "input",
    "int", "isinstance", "issubclass", "iter", "len", "license", "list", "locals", "map", "max",
    "memoryview", "min", "next", "object", "oct", "open", "ord", "pow", "print", "property",
    "quit", "range", "repr", "reversed", "round", "set", "setattr", "slice", "sorted",
    "staticmethod", "str", "sum", "super", "tuple", "type", "vars", "zip", "__import__",
    "__build_class__", "__debug__", "__name__", "__file__", "__doc__", "__package__",
    "__spec__", "__loader__", "__path__", "__builtins__", "__dict__", "__module__",
    "__qualname__", "__class__", "__annotations__", "__all__", "__cached__", "reveal_type",
    "reveal_locals", "NotImplemented", "Ellipsis", "None", "True", "False", "_",
    // de una plataforma o internos de CPython (existen sin importar)
    "WindowsError", "_IncompleteInputError", "__peg_parser__", "__warningregistry__",
    // excepciones y avisos
    "BaseException", "BaseExceptionGroup", "Exception", "ExceptionGroup", "ArithmeticError",
    "AssertionError", "AttributeError", "BlockingIOError", "BrokenPipeError", "BufferError",
    "ChildProcessError", "ConnectionAbortedError", "ConnectionError", "ConnectionRefusedError",
    "ConnectionResetError", "EOFError", "EnvironmentError", "FileExistsError",
    "FileNotFoundError", "FloatingPointError", "GeneratorExit", "IOError", "ImportError",
    "IndentationError", "IndexError", "InterruptedError", "IsADirectoryError", "KeyError",
    "KeyboardInterrupt", "LookupError", "MemoryError", "ModuleNotFoundError", "NameError",
    "NotADirectoryError", "NotImplementedError", "OSError", "OverflowError",
    "PermissionError", "ProcessLookupError", "PythonFinalizationError", "RecursionError",
    "ReferenceError", "RuntimeError", "StopAsyncIteration", "StopIteration", "SyntaxError",
    "SystemError", "SystemExit", "TabError", "TimeoutError", "TypeError", "UnboundLocalError",
    "UnicodeDecodeError", "UnicodeEncodeError", "UnicodeError", "UnicodeTranslateError",
    "ValueError", "ZeroDivisionError", "Warning", "BytesWarning", "DeprecationWarning",
    "EncodingWarning", "FutureWarning", "ImportWarning", "PendingDeprecationWarning",
    "ResourceWarning", "RuntimeWarning", "SyntaxWarning", "UnicodeWarning", "UserWarning",
];

fn analizar(fuente: &str) -> Option<Tree> {
    let mut p = Parser::new();
    p.set_language(&tree_sitter_python::LANGUAGE.into()).ok()?;
    p.parse(fuente, None)
}

fn texto<'a>(n: Node, f: &'a str) -> &'a str {
    n.utf8_text(f.as_bytes()).unwrap_or("")
}

fn hijos(n: Node) -> Vec<Node> {
    let mut c = n.walk();
    n.children(&mut c).collect()
}

/// ¿`n` es (uno de) los hijos del campo `campo` de su padre?
fn es_campo(n: Node, campo: &str) -> bool {
    let Some(p) = n.parent() else { return false };
    let mut c = p.walk();
    let r = p.children_by_field_name(campo, &mut c).any(|h| h.id() == n.id());
    r
}

fn linea(n: Node) -> usize {
    n.start_position().row + 1
}

/// 1. Sintaxis: los nodos de error y las piezas que el analizador tuvo que inventar.
fn sintaxis(raiz: Node, f: &str) -> Vec<(usize, String)> {
    let mut v = Vec::new();
    let mut pila = vec![raiz];
    while let Some(n) = pila.pop() {
        if v.len() >= 5 {
            break;
        }
        if n.is_missing() {
            v.push((
                linea(n),
                crate::tr!(
                    "sintaxis: falta «{}»",
                    "syntax: missing «{}»",
                    n.kind()
                ),
            ));
            continue;
        }
        if n.is_error() {
            let t: String = texto(n, f).lines().next().unwrap_or("").trim().chars().take(50).collect();
            v.push((
                linea(n),
                crate::tr!(
                    "sintaxis inválida cerca de «{t}»",
                    "invalid syntax near «{t}»"
                ),
            ));
            continue;
        }
        if n.has_error() {
            let mut hs = hijos(n);
            hs.reverse();
            pila.extend(hs);
        }
    }
    v.sort();
    v.dedup();
    v
}

/// Los identificadores de un DESTINO de asignación (`a`, `a, b`, `[a, *b]`), sin entrar en
/// atributos ni subíndices (`self.x`, `d[k]` no definen nombres).
fn destinos(n: Node, f: &str, out: &mut HashSet<String>) {
    match n.kind() {
        "identifier" => {
            out.insert(texto(n, f).to_string());
        }
        "pattern_list" | "tuple_pattern" | "list_pattern" | "list_splat_pattern"
        | "parenthesized_expression" | "tuple" | "list" | "expression_list"
        | "as_pattern_target" | "list_splat" => {
            for h in hijos(n) {
                destinos(h, f, out);
            }
        }
        _ => {}
    }
}

/// Nombres que el archivo DEFINE (generoso: cualquier ámbito; más definiciones = menos falsos
/// avisos). Con `solo_modulo`, sólo lo de nivel de módulo (lo que otro archivo puede importar).
fn definidos(raiz: Node, f: &str, solo_modulo: bool) -> HashSet<String> {
    let mut d = HashSet::new();
    if solo_modulo {
        // `global x` dentro de una función crea `x` en el MÓDULO (medido: `mimetypes.types_map`)
        let mut pila = vec![raiz];
        while let Some(n) = pila.pop() {
            if n.kind() == "global_statement" {
                for h in hijos(n) {
                    if h.kind() == "identifier" {
                        d.insert(texto(h, f).to_string());
                    }
                }
            }
            pila.extend(hijos(n));
        }
    }
    let mut pila = vec![raiz];
    while let Some(n) = pila.pop() {
        let k = n.kind();
        match k {
            "function_definition" | "class_definition" => {
                if let Some(nom) = n.child_by_field_name("name") {
                    d.insert(texto(nom, f).to_string());
                }
                if solo_modulo {
                    continue; // su cuerpo no es del módulo
                }
            }
            "assignment" | "augmented_assignment" | "for_statement" | "for_in_clause" => {
                if let Some(l) = n.child_by_field_name("left") {
                    destinos(l, f, &mut d);
                }
            }
            "named_expression" => {
                if let Some(nom) = n.child_by_field_name("name") {
                    d.insert(texto(nom, f).to_string());
                }
            }
            "type_alias_statement" => {
                if let Some(l) = n.child_by_field_name("left") {
                    let t = texto(l, f);
                    d.insert(t.split('[').next().unwrap_or(t).trim().to_string());
                }
            }
            "import_statement" => {
                let mut c = n.walk();
                for h in n.children_by_field_name("name", &mut c) {
                    match h.kind() {
                        "aliased_import" => {
                            if let Some(a) = h.child_by_field_name("alias") {
                                d.insert(texto(a, f).to_string());
                            }
                        }
                        _ => {
                            // `import a.b` define `a`
                            let t = texto(h, f);
                            d.insert(t.split('.').next().unwrap_or(t).trim().to_string());
                        }
                    }
                }
                continue;
            }
            "import_from_statement" | "future_import_statement" => {
                let mut c = n.walk();
                for h in n.children_by_field_name("name", &mut c) {
                    match h.kind() {
                        "aliased_import" => {
                            if let Some(a) = h.child_by_field_name("alias") {
                                d.insert(texto(a, f).to_string());
                            }
                        }
                        _ => {
                            d.insert(texto(h, f).trim().to_string());
                        }
                    }
                }
                continue;
            }
            "as_pattern_target" => {
                destinos(n, f, &mut d);
            }
            "global_statement" | "nonlocal_statement" | "case_pattern" | "type_parameter" => {
                let mut sub = vec![n];
                while let Some(s) = sub.pop() {
                    if s.kind() == "identifier" {
                        d.insert(texto(s, f).to_string());
                    }
                    sub.extend(hijos(s));
                }
                continue;
            }
            "identifier" => {
                let padre = n.parent().map(|p| p.kind()).unwrap_or("");
                let enlaza = matches!(
                    padre,
                    "parameters" | "lambda_parameters" | "typed_parameter" | "list_splat_pattern"
                        | "dictionary_splat_pattern" | "as_pattern_target"
                ) || (matches!(padre, "default_parameter" | "typed_default_parameter")
                    && es_campo(n, "name"))
                    // `except E as e`, `with x as y`, `case … as z`: lo que sigue a «as»
                    || n.prev_sibling().map(|s| s.kind() == "as").unwrap_or(false);
                if enlaza && !(solo_modulo && dentro_de_funcion(n)) {
                    d.insert(texto(n, f).to_string());
                }
            }
            _ => {}
        }
        if solo_modulo && matches!(k, "lambda") {
            continue;
        }
        pila.extend(hijos(n));
    }
    d
}

fn dentro_de_funcion(n: Node) -> bool {
    let mut p = n.parent();
    while let Some(x) = p {
        if matches!(x.kind(), "function_definition" | "class_definition" | "lambda") {
            return true;
        }
        p = x.parent();
    }
    false
}

/// ¿Este identificador se USA como nombre (no es atributo, palabra clave de un argumento ni
/// parte de un import)?
fn es_uso(n: Node) -> bool {
    let Some(p) = n.parent() else { return false };
    match p.kind() {
        "attribute" => !es_campo(n, "attribute"),
        "keyword_argument" => !es_campo(n, "name"),
        "dotted_name" | "aliased_import" | "import_prefix" | "relative_import"
        | "global_statement" | "nonlocal_statement" | "keyword_pattern" => false,
        _ => {
            let mut a = Some(p);
            while let Some(x) = a {
                if matches!(
                    x.kind(),
                    "import_statement" | "import_from_statement" | "future_import_statement"
                        | "case_pattern"
                ) {
                    return false;
                }
                a = x.parent();
            }
            true
        }
    }
}

/// Un módulo que crea nombres en tiempo de ejecución (`globals().update(…)`, `import *`,
/// `exec`) no se puede juzgar sin ejecutarlo: medido en la biblioteca estándar 3.14 (`ssl`, `re`,
/// `inspect`, `multiprocessing`, `hashlib` definen así sus constantes).
fn dinamico(f: &str) -> bool {
    ["import *", "globals()", "locals()", "vars()", "exec(", "sys.modules[__name__]", "_convert_(", "global_enum"]
        .iter()
        .any(|p| f.contains(p))
}

/// 2. Nombres usados que no se definen, no se importan y no son de Python.
fn nombres_sin_definir(raiz: Node, f: &str) -> Vec<(usize, String)> {
    // lo que define nombres sin que se vean: no se puede juzgar
    if dinamico(f) {
        return Vec::new();
    }
    let def = definidos(raiz, f, false);
    let b: HashSet<&str> = BUILTINS.iter().copied().collect();
    let mut vistos = HashSet::new();
    let mut v = Vec::new();
    let mut pila = vec![raiz];
    while let Some(n) = pila.pop() {
        if n.kind() == "identifier" && es_uso(n) {
            let t = texto(n, f);
            if !t.is_empty() && !def.contains(t) && !b.contains(t) && vistos.insert(t.to_string()) {
                v.push((
                    linea(n),
                    crate::tr!(
                        "`{t}` no está definido ni importado en este archivo (¿falta el import?)",
                        "`{t}` is not defined or imported in this file (missing import?)"
                    ),
                ));
            }
        }
        pila.extend(hijos(n));
    }
    v.sort();
    v
}

/// Las raíces desde donde se resuelve un import absoluto del proyecto: la del proyecto, `src/`
/// y la carpeta que contiene el paquete más alto del archivo.
fn raices(root: &Path, archivo: &Path) -> Vec<PathBuf> {
    let mut r = vec![root.to_path_buf(), root.join("src")];
    let mut d = archivo.parent().map(Path::to_path_buf);
    let mut ultimo = None;
    while let Some(x) = d {
        if x.join("__init__.py").is_file() {
            ultimo = x.parent().map(Path::to_path_buf);
            d = x.parent().map(Path::to_path_buf);
        } else {
            break;
        }
    }
    if let Some(u) = ultimo {
        if !r.contains(&u) {
            r.push(u);
        }
    }
    r
}

/// Dónde vive un módulo: `Some(Some(archivo))` si es un .py (o el __init__ de un paquete),
/// `Some(None)` si es una carpeta-paquete sin __init__, `None` si no existe.
fn ubicar(base: &Path, partes: &[&str]) -> Option<Option<PathBuf>> {
    let mut p = base.to_path_buf();
    for x in partes {
        p.push(x);
    }
    let py = p.with_extension("py");
    if !partes.is_empty() && py.is_file() {
        return Some(Some(py));
    }
    if p.join("__init__.py").is_file() {
        return Some(Some(p.join("__init__.py")));
    }
    if p.is_dir() {
        return Some(None);
    }
    None
}

/// ¿Alguna pieza intermedia es un archivo .py y no un paquete? Entonces lo que sigue sólo puede
/// existir en `sys.modules` (como `xml.parsers.expat.errors`): no se juzga.
fn cuelga_de_un_archivo(base: &Path, partes: &[&str]) -> bool {
    (1..partes.len()).any(|i| {
        let mut p = base.to_path_buf();
        for x in &partes[..i] {
            p.push(x);
        }
        p.with_extension("py").is_file() && !p.is_dir()
    })
}

enum Modulo {
    /// No es del proyecto (biblioteca estándar o de terceros): no se juzga.
    Ajeno,
    Falta(String),
    Esta(Option<PathBuf>, PathBuf),
}

/// Resuelve `from <modulo> import …` / `import <modulo>` contra el proyecto.
fn resolver(root: &Path, archivo: &Path, puntos: usize, nombre: &str) -> Modulo {
    let partes: Vec<&str> = nombre.split('.').filter(|s| !s.is_empty()).collect();
    if puntos > 0 {
        let mut base = archivo.parent().map(Path::to_path_buf).unwrap_or_default();
        for _ in 1..puntos {
            base = base.parent().map(Path::to_path_buf).unwrap_or(base);
        }
        let dir = partes.iter().fold(base.clone(), |b, x| b.join(x));
        return match ubicar(&base, &partes) {
            Some(a) => Modulo::Esta(a, dir),
            None if cuelga_de_un_archivo(&base, &partes) => Modulo::Ajeno,
            None => Modulo::Falta(format!("{}{}", ".".repeat(puntos), nombre)),
        };
    }
    // alias que Python pone en `sys.modules` (no son archivos): sólo importan si el proyecto
    // tapa la biblioteca estándar, como ella misma (medido en 3.14)
    if matches!(nombre, "os.path" | "collections.abc") {
        return Modulo::Ajeno;
    }
    let Some(primera) = partes.first() else { return Modulo::Ajeno };
    for r in raices(root, archivo) {
        // del proyecto si su primera pieza existe aquí
        if r.join(primera).is_dir() || r.join(format!("{primera}.py")).is_file() {
            let dir = partes.iter().fold(r.clone(), |b, x| b.join(x));
            return match ubicar(&r, &partes) {
                Some(a) => Modulo::Esta(a, dir),
                None if cuelga_de_un_archivo(&r, &partes) => Modulo::Ajeno,
                None => Modulo::Falta(nombre.to_string()),
            };
        }
    }
    Modulo::Ajeno
}

/// Lo que un módulo del proyecto ofrece para importar (nivel de módulo). `None` = no se puede
/// saber (import *, `__getattr__` de módulo): no se juzga.
fn ofrece(modulo: &Path) -> Option<HashSet<String>> {
    let f = std::fs::read_to_string(modulo).ok()?;
    if dinamico(&f) || f.contains("def __getattr__") {
        return None;
    }
    let t = analizar(&f)?;
    Some(definidos(t.root_node(), &f, true))
}

/// 3. Imports del proyecto que no existen. Cada problema dice a qué archivo apunta (para el 4).
fn imports(root: &Path, archivo: &Path, raiz: Node, f: &str) -> Vec<(usize, String, Option<PathBuf>)> {
    let mut v = Vec::new();
    let mut pila = vec![raiz];
    while let Some(n) = pila.pop() {
        match n.kind() {
            "import_from_statement" => {
                let Some(m) = n.child_by_field_name("module_name") else { continue };
                let (puntos, nombre) = if m.kind() == "relative_import" {
                    let t = texto(m, f);
                    let p = t.chars().take_while(|c| *c == '.').count();
                    (p, t[p..].trim().to_string())
                } else {
                    (0, texto(m, f).trim().to_string())
                };
                match resolver(root, archivo, puntos, &nombre) {
                    Modulo::Ajeno => {}
                    Modulo::Falta(x) => v.push((
                        linea(n),
                        crate::tr!(
                            "el módulo `{x}` no existe en el proyecto",
                            "module `{x}` does not exist in the project"
                        ),
                        None,
                    )),
                    Modulo::Esta(arch, dir) => {
                        let ofrecidos = arch.as_deref().and_then(ofrece).or_else(|| arch.is_none().then(HashSet::new));
                        let Some(of) = ofrecidos else { continue };
                        let mut c = n.walk();
                        for h in n.children_by_field_name("name", &mut c) {
                            let nom_nodo = if h.kind() == "aliased_import" { h.child_by_field_name("name").unwrap_or(h) } else { h };
                            let nom = texto(nom_nodo, f).trim().to_string();
                            // un submódulo también se importa así; y los dunder (`__file__`, `__doc__`…) los tiene todo módulo
                            let sub = dir.join(format!("{nom}.py")).is_file() || dir.join(&nom).is_dir() || (nom.starts_with("__") && nom.ends_with("__"));
                            if !of.contains(&nom) && !sub {
                                let donde = arch.as_deref().map(|a| a.strip_prefix(root).unwrap_or(a).display().to_string()).unwrap_or_else(|| dir.display().to_string());
                                let mut hay: Vec<&String> = of.iter().filter(|x| !x.starts_with('_')).collect();
                                hay.sort();
                                let hay: Vec<String> = hay.into_iter().take(8).cloned().collect();
                                let pista = if hay.is_empty() { String::new() } else { crate::tr!(" (allí hay: {})", " (it has: {})", hay.join(", ")) };
                                v.push((
                                    linea(h),
                                    crate::tr!(
                                        "importa `{nom}` de `{donde}`, que no lo define{pista}",
                                        "imports `{nom}` from `{donde}`, which does not define it{pista}"
                                    ),
                                    arch.clone(),
                                ));
                            }
                        }
                    }
                }
                continue;
            }
            "import_statement" => {
                let mut c = n.walk();
                for h in n.children_by_field_name("name", &mut c) {
                    let nom_nodo = if h.kind() == "aliased_import" { h.child_by_field_name("name").unwrap_or(h) } else { h };
                    let nom = texto(nom_nodo, f).trim().to_string();
                    if let Modulo::Falta(x) = resolver(root, archivo, 0, &nom) {
                        v.push((
                            linea(h),
                            crate::tr!(
                                "el módulo `{x}` no existe en el proyecto",
                                "module `{x}` does not exist in the project"
                            ),
                            None,
                        ));
                    }
                }
                continue;
            }
            _ => {}
        }
        pila.extend(hijos(n));
    }
    v.sort_by_key(|x| x.0);
    v
}

/// Revisa UN archivo: sintaxis; si está sano, nombres e imports.
pub fn revisar(root: &Path, archivo: &Path) -> Vec<Problema> {
    let Ok(f) = std::fs::read_to_string(archivo) else { return Vec::new() };
    let Some(t) = analizar(&f) else { return Vec::new() };
    let raiz = t.root_node();
    let mut v: Vec<(usize, String)> = sintaxis(raiz, &f);
    // con errores de sintaxis el árbol no es de fiar: sólo eso
    if v.is_empty() {
        v.extend(nombres_sin_definir(raiz, &f));
        v.extend(imports(root, archivo, raiz, &f).into_iter().map(|(l, m, _)| (l, m)));
    }
    v.sort_by_key(|x| x.0);
    v.into_iter()
        .map(|(linea, mensaje)| Problema { ruta: archivo.to_path_buf(), linea, mensaje })
        .collect()
}

/// 4. Los otros .py del proyecto que importan de `archivo` algo que ya no define.
pub fn rotos_por(root: &Path, archivo: &Path) -> Vec<Problema> {
    let modulo = archivo.file_stem().and_then(|s| s.to_str()).unwrap_or("").to_string();
    if modulo.is_empty() {
        return Vec::new();
    }
    let destino = archivo.canonicalize().unwrap_or_else(|_| archivo.to_path_buf());
    let mut v = Vec::new();
    for otro in archivos_py(root, 400) {
        if otro == archivo {
            continue;
        }
        let Ok(f) = std::fs::read_to_string(&otro) else { continue };
        if !f.contains(&modulo) && !(modulo == "__init__" && f.contains("import")) {
            continue;
        }
        let Some(t) = analizar(&f) else { continue };
        for (linea, mensaje, a) in imports(root, &otro, t.root_node(), &f) {
            let apunta = a.map(|a| a.canonicalize().unwrap_or(a) == destino).unwrap_or(false);
            if apunta {
                v.push(Problema { ruta: otro.clone(), linea, mensaje });
            }
        }
    }
    v
}

fn archivos_py(root: &Path, tope: usize) -> Vec<PathBuf> {
    let mut v = Vec::new();
    let mut pila = vec![root.to_path_buf()];
    while let Some(d) = pila.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            let n = e.file_name().to_string_lossy().to_string();
            if n.starts_with('.') || matches!(n.as_str(), "node_modules" | "__pycache__" | "venv" | "env" | "site-packages" | "target" | "build" | "dist") {
                continue;
            }
            if p.is_dir() {
                pila.push(p);
            } else if soporta(&p) {
                v.push(p);
                if v.len() >= tope {
                    return v;
                }
            }
        }
    }
    v
}

/// El texto que ve el modelo tras su edición (siempre una línea al menos).
pub fn informe(root: &Path, archivo: &Path) -> String {
    let mut ps = revisar(root, archivo);
    let propios = ps.len();
    if propios == 0 {
        ps.extend(rotos_por(root, archivo));
    }
    let rel = |p: &Path| p.strip_prefix(root).unwrap_or(p).display().to_string();
    if ps.is_empty() {
        return crate::tr!(
            "[Revisor de GOTV] {}: sin errores de sintaxis, nombres ni imports (no sustituye a las pruebas)",
            "[GOTV checker] {}: no syntax, name or import errors (not a substitute for the tests)",
            rel(archivo)
        );
    }
    let mut s = crate::tr!(
        "[Revisor de GOTV] {} problema(s) — corrígelos antes de seguir:",
        "[GOTV checker] {} problem(s) — fix them before going on:",
        ps.len()
    );
    for p in ps.iter().take(12) {
        s.push_str(&format!("\n  {}:{} {}", rel(&p.ruta), p.linea, p.mensaje));
    }
    s
}

// ── DM6.3 · CSS y HTML, nativo (2026-11-13) ─────────────────────────────────────────────────────
// El navegador DESCARTA en silencio el CSS mal escrito (una llave que falta se come el resto de la
// hoja) y repara el HTML roto a su manera: nadie avisa. Sintaxis con tree-sitter (css/html 0.23.2) y,
// en HTML, los `id` repetidos (rompen anclas, etiquetas de formulario y `getElementById`).

/// Lenguajes de marcado que el revisor nativo mira (sólo sintaxis).
pub const NATIVOS_MARCADO: &[&str] = &["CSS", "HTML"];

/// ¿Es CSS o HTML?
pub fn soporta_marcado(p: &Path) -> bool {
    matches!(p.extension().and_then(|e| e.to_str()), Some("css" | "html" | "htm"))
}

/// Los `id` que aparecen más de una vez en un HTML (en orden de aparición).
fn ids_repetidos(html: &str) -> Vec<String> {
    let mut vistos: Vec<String> = Vec::new();
    let mut repetidos: Vec<String> = Vec::new();
    for marca in [" id=\"", " id='"] {
        let cierre = if marca.ends_with('"') { '"' } else { '\'' };
        for (i, _) in html.match_indices(marca) {
            let resto = &html[i + marca.len()..];
            let id = &resto[..resto.find(cierre).unwrap_or(0)];
            if id.is_empty() || id.contains('{') {
                continue;
            }
            if vistos.iter().any(|v| v == id) {
                if !repetidos.iter().any(|r| r == id) {
                    repetidos.push(id.to_string());
                }
            } else {
                vistos.push(id.to_string());
            }
        }
    }
    repetidos
}

/// Tras editar un `.css` o `.html`: su sintaxis (y, en HTML, los `id` repetidos).
pub fn informe_marcado(root: &Path, archivo: &Path) -> Option<String> {
    let f = std::fs::read_to_string(archivo).ok()?;
    let lenguaje: tree_sitter::Language = match archivo.extension().and_then(|e| e.to_str())? {
        "css" => tree_sitter_css::LANGUAGE.into(),
        "html" | "htm" => tree_sitter_html::LANGUAGE.into(),
        _ => return None,
    };
    let mut p = Parser::new();
    p.set_language(&lenguaje).ok()?;
    let arbol = p.parse(&f, None)?;
    let mut ps: Vec<(usize, String)> = sintaxis(arbol.root_node(), &f);
    if lenguaje == tree_sitter_html::LANGUAGE.into() {
        for id in ids_repetidos(&f) {
            // la línea de la REPETICIÓN (la segunda aparición), que es la que hay que cambiar
            let linea = f
                .lines()
                .enumerate()
                .filter(|(_, l)| l.contains(&format!("id=\"{id}\"")) || l.contains(&format!("id='{id}'")))
                .nth(1)
                .map_or(1, |(i, _)| i + 1);
            ps.push((linea, crate::tr!(
                "el id «{id}» está repetido: un id es único en la página (anclas, etiquetas y scripts usan el primero)",
                "the id «{id}» is repeated: an id is unique in the page (anchors, labels and scripts use the first one)"
            )));
        }
    }
    let rel = archivo.strip_prefix(root).unwrap_or(archivo).display().to_string();
    if ps.is_empty() {
        return Some(crate::tr!(
            "[Revisor de GOTV] {rel}: sin errores de sintaxis (el aspecto se mide en el navegador)",
            "[GOTV checker] {rel}: no syntax errors (the look is measured in the browser)"
        ));
    }
    let mut s = crate::tr!(
        "[Revisor de GOTV] {} problema(s) en {rel} — el navegador los DESCARTA en silencio; corrígelos:",
        "[GOTV checker] {} problem(s) in {rel} — the browser DROPS them silently; fix them:",
        ps.len()
    );
    for (linea, m) in ps.iter().take(10) {
        s.push_str(&format!("\n  {rel}:{linea} {m}"));
    }
    Some(s)
}

#[cfg(test)]
mod marcado {
    use super::*;

    #[test]
    fn css_y_html_rotos_se_nombran_con_su_linea() {
        let d = crate::CarpetaPrueba::nueva("gotv-revisor-marcado");
        let css = d.join("estilo.css");
        std::fs::write(&css, ".a { color: red; }\n.b { color: blue;\n.c { margin: 0 }\n").unwrap();
        let r = informe_marcado(&d, &css).unwrap();
        assert!(r.contains("problema(s)") && r.contains("estilo.css:"), "{r}");
        std::fs::write(&css, ".a { color: red; }\n@media (max-width: 600px) { .b { display: grid; } }\n").unwrap();
        assert!(informe_marcado(&d, &css).unwrap().contains("sin errores de sintaxis"));
        let html = d.join("index.html");
        std::fs::write(&html, "<main>\n<a href=\"#x\" id=\"x\">uno</a>\n<p id=\"x\">dos</p>\n</main>\n").unwrap();
        let r = informe_marcado(&d, &html).unwrap();
        assert!(r.contains("«x» está repetido") && r.contains("index.html:3"), "{r}");
        std::fs::write(&html, "<!doctype html>\n<main><p>hola</p></main>\n").unwrap();
        assert!(informe_marcado(&d, &html).unwrap().contains("sin errores de sintaxis"));
    }
}

// ── L8 · sintaxis con el revisor PROPIO de cada lenguaje (prueba Cable, 2026-11-13) ─────────────
// Medido sobre código roto Y sobre código sano pero mal formateado: estos dicen el error con su línea
// y callan ante el formato. Descartados con datos: `node --check` con TypeScript (marca como error
// un TS válido), `javac` (pide el classpath del proyecto), `deno check` (descarga y juzga tipos).

/// `(extensiones, programa, argumentos antes del archivo, lenguaje)`
const EXTERNOS: &[(&[&str], &str, &[&str], &str)] = &[
    (&["js", "mjs", "cjs"], "node", &["--check"], "JavaScript"),
    (&["php"], "php", &["-l"], "PHP"),
    (&["rb"], "ruby", &["-c"], "Ruby"),
    (&["go"], "gofmt", &["-e", "-l"], "Go"),
    (&["swift"], "swiftc", &["-parse"], "Swift"),
    (&["rs"], "rustfmt", &["--emit", "stdout"], "Rust"),
];

fn externo_para(p: &Path) -> Option<&'static (&'static [&'static str], &'static str, &'static [&'static str], &'static str)> {
    let ext = p.extension()?.to_str()?;
    EXTERNOS.iter().find(|(exts, prog, _, _)| exts.contains(&ext) && crate::lsp::on_path(prog))
}

/// Los revisores de sintaxis que hay en ESTA Mac, para el bloque de entorno: «Go (gofmt)».
pub fn externos_disponibles() -> Vec<String> {
    EXTERNOS
        .iter()
        .filter(|(_, prog, _, _)| crate::lsp::on_path(prog))
        .map(|(_, prog, args, l)| format!("{l} ({prog} {})", args.first().unwrap_or(&"")))
        .collect()
}

/// La edición de Rust del `Cargo.toml` más cercano (rustfmt usa 2015 por omisión y entonces
/// `async fn` es un error).
fn edicion_rust(archivo: &Path) -> String {
    let mut d = archivo.parent();
    while let Some(dir) = d {
        if let Ok(t) = std::fs::read_to_string(dir.join("Cargo.toml")) {
            if let Some(e) = t.lines().map(str::trim).find_map(|l| l.strip_prefix("edition")) {
                let e = e.trim_start_matches([' ', '=']).trim().trim_matches('"');
                if !e.is_empty() {
                    return e.to_string();
                }
            }
        }
        d = dir.parent();
    }
    "2021".into()
}

/// Tras editar un archivo de un lenguaje sin servidor ni revisor nativo: su revisor de sintaxis
/// propio, si está instalado. `None` si no hay ninguno para esa extensión.
/// F5 ML7 · la sintaxis por el árbol de `gotv-sintaxis` (Rust, TypeScript, JavaScript, Go, PHP, Swift, C,
/// JSON…), al instante y sin programas externos. `Some` sólo si HAY errores: si el árbol está limpio, el
/// revisor externo (más estricto: el compilador o el intérprete) sigue diciendo la última palabra.
pub fn informe_nativo(root: &Path, archivo: &Path) -> Option<String> {
    let l = gotv_sintaxis::Lenguaje::de_ruta(archivo)?;
    let t = std::fs::read_to_string(archivo).ok()?;
    let a = gotv_sintaxis::analizar(l, &t)?;
    let errores = gotv_sintaxis::errores(&a, 6);
    if errores.is_empty() {
        return None;
    }
    let rel = archivo.strip_prefix(root).unwrap_or(archivo).display().to_string();
    let lineas: Vec<String> = errores
        .iter()
        .map(|e| match &e.falta {
            Some(f) => crate::tr!("  {rel}:{}:{}: falta «{f}»", "  {rel}:{}:{}: missing «{f}»", e.linea, e.columna),
            None => crate::tr!("  {rel}:{}:{}: sintaxis inválida cerca de «{}»", "  {rel}:{}:{}: invalid syntax near «{}»", e.linea, e.columna, e.cerca),
        })
        .collect();
    Some(crate::tr!(
        "[Revisor de GOTV] {rel}: ERROR DE SINTAXIS ({}, árbol nativo) — corrígelo antes de seguir:\n{}",
        "[GOTV checker] {rel}: SYNTAX ERROR ({}, native tree) — fix it before going on:\n{}",
        l.nombre(),
        lineas.join("\n")
    ))
}

/// ¿La salida de un revisor señala ESTE archivo o una línea? (`a.rs:3:5`, `line 2`, `--> src/lib.rs`)
fn senala_el_archivo(texto: &str, archivo: &Path) -> bool {
    let nombre = archivo.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let con_linea = texto.split(|c: char| c.is_whitespace() || c == ',').any(|w| {
        let mut partes = w.split(':');
        partes.next().is_some_and(|_| partes.next().is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit())))
    });
    (!nombre.is_empty() && texto.contains(&nombre)) || con_linea || texto.to_lowercase().split("line ").skip(1).any(|r| r.starts_with(|c: char| c.is_ascii_digit()))
}

pub async fn informe_externo(root: &Path, archivo: &Path) -> Option<String> {
    let (_, prog, args, _) = externo_para(archivo)?;
    let mut c = crate::entorno::comando_tokio(prog);
    c.args(*args);
    if *prog == "rustfmt" {
        c.args(["--edition", &edicion_rust(archivo)]);
    }
    c.arg(archivo).current_dir(root).stdin(std::process::Stdio::null()).kill_on_drop(true);
    let rel = archivo.strip_prefix(root).unwrap_or(archivo).display().to_string();
    let salida = tokio::time::timeout(std::time::Duration::from_secs(20), c.output()).await.ok()?.ok()?;
    let nombre = format!("{prog} {}", args.first().unwrap_or(&""));
    if salida.status.success() {
        return Some(crate::tr!(
            "[Revisor de GOTV] {rel}: sin errores de sintaxis ({nombre}) — nombres y tipos los dirán la compilación y las pruebas",
            "[GOTV checker] {rel}: no syntax errors ({nombre}) — names and types are for the build and the tests to say"
        ));
    }
    let err = String::from_utf8_lossy(&salida.stderr);
    let out = String::from_utf8_lossy(&salida.stdout);
    let texto = if err.trim().is_empty() { out } else { err };
    // medido 2026-11-18 (banco F8 con Huihui): «rustup could not choose a version of rustfmt» salió como ERROR DE
    // SINTAXIS de un archivo sano, y el modelo intentó instalar rustup con `curl | sh`. Un error de sintaxis
    // SEÑALA el archivo o una línea; si no señala nada, lo que falló fue la herramienta, y se dice así.
    if !senala_el_archivo(&texto, archivo) {
        let primera = texto.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("").chars().take(200).collect::<String>();
        return Some(crate::tr!(
            "[Revisor de GOTV] {rel}: no pude revisarlo ({nombre} no corrió: {primera}) — no es un error de tu código; la compilación y las pruebas lo dirán",
            "[GOTV checker] {rel}: could not check it ({nombre} did not run: {primera}) — this is not an error in your code; the build and the tests will tell"
        ));
    }
    let abs = archivo.display().to_string();
    let lineas: Vec<String> = texto
        .lines()
        .filter(|l| !l.trim().is_empty() && !l.contains("node:internal") && !l.starts_with("Errors parsing"))
        .take(10)
        .map(|l| format!("  {}", l.replace(&abs, &rel)))
        .collect();
    Some(crate::tr!(
        "[Revisor de GOTV] {rel}: ERROR DE SINTAXIS ({nombre}) — corrígelo antes de seguir:\n{}",
        "[GOTV checker] {rel}: SYNTAX ERROR ({nombre}) — fix it before going on:\n{}",
        lineas.join("\n")
    ))
}

#[cfg(test)]
mod externos {
    use super::*;

    /// Con los revisores que haya en esta Mac: el roto se nombra con su línea, el sano (aunque mal
    /// formateado) pasa limpio. Los que no estén instalados se saltan.
    /// Banco F8 (Huihui): la herramienta que no corre no es un error de sintaxis del archivo.
    #[test]
    fn un_revisor_que_no_corre_no_es_un_error_de_sintaxis() {
        let p = Path::new("/p/api/src/lib.rs");
        assert!(!senala_el_archivo("error: rustup could not choose a version of rustfmt to run, because one wasn't specified explicitly, and no default is configured.\nhelp: run 'rustup default stable'", p));
        assert!(senala_el_archivo("error: expected one of `,` or `)`, found `{`\n --> /p/api/src/lib.rs:1:6", p));
        assert!(senala_el_archivo("PHP Parse error: syntax error, unexpected '{' in x.php on line 2", p));
        assert!(senala_el_archivo("a.go:3:9: syntax error: unexpected {", Path::new("/p/a.go")));
    }

    #[tokio::test]
    async fn cada_lenguaje_con_su_revisor() {
        let d = crate::CarpetaPrueba::nueva("gotv-revisor-externo");
        let casos: &[(&str, &str, &str, &str)] = &[
            ("a.js", "function f( {\n  return 1;\n}\n", "function f( ) {return 1}\n", "a.js:2"),
            ("a.php", "<?php\nfunction f( {\n  return 1;\n}\n", "<?php\nfunction f( ) {return 1;}\n", "line 2"),
            ("a.rb", "def f(\n  1\n", "def f; 1; end\n", "a.rb:2"),
            ("a.go", "package a\n\nfunc F( {\n}\n", "package a\nfunc F( ) {  }\n", "a.go:3"),
            ("a.swift", "func f( {\n}\n", "func f( ) {  }\n", "a.swift:1"),
            ("a.rs", "fn f( {\n    1\n}\n", "fn f( )->i32{1}\n", "a.rs:3"),
        ];
        for (nombre, roto, sano, señal) in casos {
            let p = d.join(nombre);
            if externo_para(&p).is_none() {
                continue;
            }
            std::fs::write(&p, roto).unwrap();
            let r = informe_externo(&d, &p).await.unwrap();
            assert!(r.contains("ERROR DE SINTAXIS") && r.contains(señal), "{nombre}: {r}");
            assert!(!r.contains(&d.display().to_string()), "rutas relativas: {r}");
            std::fs::write(&p, sano).unwrap();
            let r = informe_externo(&d, &p).await.unwrap();
            assert!(r.contains("sin errores de sintaxis"), "{nombre} sano: {r}");
        }
        assert!(informe_externo(&d, &d.join("a.ts")).await.is_none(), "TypeScript: sin revisor fiable");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn proyecto(archivos: &[(&str, &str)]) -> crate::CarpetaPrueba {
        let d = crate::CarpetaPrueba::nueva("gotv-revisor");
        for (a, t) in archivos {
            let p = d.join(a);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, t).unwrap();
        }
        d
    }

    fn msgs(ps: &[Problema]) -> Vec<String> {
        ps.iter().map(|p| format!("{}:{}", p.linea, p.mensaje)).collect()
    }

    #[test]
    fn el_import_que_falta_se_ve_al_editar() {
        // el caso medido: servicio.py usa Almacen y ErrorPrestamo sin importarlos
        let d = proyecto(&[
            ("biblioteca/__init__.py", "from .servicio import Biblioteca\n"),
            ("biblioteca/almacen.py", "class Almacen:\n    pass\n"),
            ("biblioteca/servicio.py", "import sqlite3\n\nclass Biblioteca:\n    def __init__(self, ruta=':memory:'):\n        self._a = Almacen(sqlite3.connect(ruta))\n\n    def devolver(self, pid):\n        if pid < 0:\n            raise ErrorPrestamo('no')\n        return [x for x in range(pid) if x]\n"),
        ]);
        let ps = revisar(&d, &d.join("biblioteca/servicio.py"));
        let m = msgs(&ps);
        assert_eq!(ps.len(), 2, "{m:?}");
        assert!(m[0].starts_with("5:") && m[0].contains("`Almacen`"), "{m:?}");
        assert!(m[1].starts_with("9:") && m[1].contains("`ErrorPrestamo`"), "{m:?}");
    }

    #[test]
    fn codigo_sano_no_da_falsos_avisos() {
        let f = r#"
from __future__ import annotations
import os.path as op, json
from dataclasses import dataclass, field
from typing import TYPE_CHECKING
if TYPE_CHECKING:
    from collections.abc import Iterator

@dataclass
class Punto:
    x: int = 0
    y: int = field(default=0)

    def norma(self, *args, escala: float = 1.0, **kw) -> float:
        total = 0
        for i, (a, b) in enumerate(zip(args, args)):
            total += a * b + i
        cuadrados = [v * v for v in (self.x, self.y)]
        f = lambda q, r=2: q ** r
        with open(op.join("a", "b"), encoding="utf-8") as fh, open("c") as (g):
            datos = json.load(fh)
        try:
            pass
        except (KeyError, ValueError) as err:
            raise RuntimeError(str(err)) from err
        if (n := len(cuadrados)) > 1:
            total += n
        match datos:
            case {"k": valor, **resto}:
                total += valor
            case [primero, *_]:
                total += primero
            case Punto(x=cx) as p:
                total += cx
        global CONTADOR
        CONTADOR = 1
        return f(total, r=escala) + sum(kw.values())

def gen() -> Iterator[int]:
    yield from range(3)

type Par[T] = tuple[T, T]
print(f"{Punto().norma(1, 2)!r:>10}", __name__, __file__)
"#;
        let d = proyecto(&[("m.py", f)]);
        let ps = revisar(&d, &d.join("m.py"));
        assert!(ps.is_empty(), "{:?}", msgs(&ps));
    }

    #[test]
    fn la_sintaxis_rota_se_dice_con_su_linea() {
        let d = proyecto(&[("m.py", "def f(x):\n    return x\n\ndef g(y)\n    return y\n")]);
        let ps = revisar(&d, &d.join("m.py"));
        assert!(!ps.is_empty() && ps[0].linea == 4, "{:?}", msgs(&ps));
        assert!(ps[0].mensaje.contains("sintaxis") || ps[0].mensaje.contains("syntax"), "{:?}", msgs(&ps));
    }

    #[test]
    fn import_de_un_nombre_que_el_modulo_no_tiene() {
        let d = proyecto(&[
            ("biblioteca/__init__.py", "from .servicio import Biblioteca\nfrom .errores import ErrorPrestamo\n"),
            ("biblioteca/errores.py", "class ErrorDePrestamo(Exception):\n    pass\n"),
            ("biblioteca/servicio.py", "class Biblioteca:\n    pass\n"),
        ]);
        let ps = revisar(&d, &d.join("biblioteca/__init__.py"));
        let m = msgs(&ps);
        assert_eq!(ps.len(), 1, "{m:?}");
        assert!(m[0].starts_with("2:") && m[0].contains("`ErrorPrestamo`") && m[0].contains("ErrorDePrestamo"), "{m:?}");
    }

    #[test]
    fn modulos_absolutos_y_relativos_del_proyecto() {
        let d = proyecto(&[
            ("pkg/__init__.py", ""),
            ("pkg/a.py", "X = 1\n"),
            ("pkg/sub/__init__.py", ""),
            ("pkg/sub/b.py", "from ..a import X\nfrom pkg.a import X as Y\nfrom pkg import sub\nfrom . import b\nimport pkg.nada\nfrom ..zeta import Q\nimport requests, os\n"),
        ]);
        let ps = revisar(&d, &d.join("pkg/sub/b.py"));
        let m = msgs(&ps);
        assert_eq!(ps.len(), 2, "{m:?}");
        assert!(m[0].starts_with("5:") && m[0].contains("pkg.nada"), "{m:?}");
        assert!(m[1].starts_with("6:") && m[1].contains("..zeta"), "{m:?}");
    }

    #[test]
    fn quitar_un_nombre_avisa_a_quien_lo_importaba() {
        let d = proyecto(&[
            ("biblioteca/__init__.py", ""),
            ("biblioteca/almacen.py", "class Almacen2:\n    pass\n"),
            ("biblioteca/servicio.py", "from .almacen import Almacen\n\nA = Almacen\n"),
        ]);
        let s = informe(&d, &d.join("biblioteca/almacen.py"));
        assert!(s.contains("servicio.py:1") && s.contains("`Almacen`"), "{s}");
    }

    #[test]
    fn sin_problemas_lo_dice_en_una_linea() {
        let d = proyecto(&[("m.py", "import math\nprint(math.pi)\n")]);
        let s = informe(&d, &d.join("m.py"));
        assert_eq!(s.lines().count(), 1, "{s}");
        assert!(s.contains("m.py"), "{s}");
    }

    #[test]
    fn import_estrella_no_se_juzga() {
        let d = proyecto(&[("m.py", "from os.path import *\nprint(join('a'))\n")]);
        assert!(revisar(&d, &d.join("m.py")).is_empty());
    }
}

/// Banco de falsos avisos: `GOTV_REVISOR_DIR=<carpeta> cargo test -p gotv-core --lib revisor::banco -- --ignored --nocapture`
#[cfg(test)]
mod banco {
    #[test]
    #[ignore]
    fn sobre_una_carpeta_real() {
        let Ok(d) = std::env::var("GOTV_REVISOR_DIR") else { return };
        let root = std::path::PathBuf::from(d);
        let archivos = super::archivos_py(&root, 5000);
        let mut con = 0;
        let mut total = 0;
        for a in &archivos {
            let ps = super::revisar(&root, a);
            if !ps.is_empty() {
                con += 1;
            }
            for p in ps {
                total += 1;
                println!("{}:{} {}", p.ruta.strip_prefix(&root).unwrap_or(&p.ruta).display(), p.linea, p.mensaje);
            }
        }
        println!("== {} archivos · {con} con avisos · {total} avisos", archivos.len());
    }
}

#[cfg(test)]
mod pruebas_nativas {
    use super::*;

    /// F5 ML7: sin servidor ni programa externo, el árbol nativo dice el error de sintaxis con su línea;
    /// un archivo limpio, o sin gramática, no dice nada (sigue el revisor externo).
    #[test]
    fn la_sintaxis_por_el_arbol_nativo() {
        let d = crate::CarpetaPrueba::nueva("gotv-revisor-nativo");
        std::fs::write(d.join("a.go"), "package x\n\nfunc f() int {\n    return (1 + \n}\n").unwrap();
        std::fs::write(d.join("b.ts"), "export const x: number = 1;\n").unwrap();
        std::fs::write(d.join("c.json"), "{\"a\": 1,,}\n").unwrap();
        std::fs::write(d.join("d.kt"), "fun x( {\n").unwrap();
        let r = informe_nativo(&d, &d.join("a.go")).unwrap();
        assert!(r.contains("ERROR DE SINTAXIS") || r.contains("SYNTAX ERROR"), "{r}");
        assert!(r.contains("a.go:4:") || r.contains("a.go:5:"), "{r}");
        assert!(informe_nativo(&d, &d.join("b.ts")).is_none(), "limpio: sin informe");
        assert!(informe_nativo(&d, &d.join("c.json")).unwrap().contains("c.json:1:"));
        assert!(informe_nativo(&d, &d.join("d.kt")).is_none(), "sin gramática: lo dirá el externo");
        // el esquema del archivo, del árbol, con la palabra de su lenguaje
        let e = crate::mapa::esquema(Path::new("x.go"), "package x\ntype S struct{}\nfunc (s S) M() {}\n");
        assert_eq!(e, "2: type S\n3: func M");
    }
}
