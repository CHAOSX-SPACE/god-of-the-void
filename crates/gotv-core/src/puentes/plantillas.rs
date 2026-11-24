//! F5 ML9 · el puente de PLANTILLAS: las variables que usa una plantilla (Jinja, Twig o Blade) ↔ las que le
//! pasa el controlador al dibujarla:
//! - Flask `render_template("x.html", a=…)`, Django `render(request, "x.html", {"a": …})`, FastAPI/Starlette
//!   `templates.TemplateResponse(…, "x.html", {"a": …})`;
//! - Laravel `view('carpeta.x', ['a' => …])` (con `->with('b', …)` y `compact('c')`), Symfony
//!   `$this->render('x.html.twig', ['a' => …])`.
//!
//! El controlador se lee con tree-sitter; la plantilla, por sus marcas (`{{ }}`, `{% %}`, `$var`, `@foreach`),
//! porque no hay gramática de plantillas dentro de GOTV. Prefiere callar: si ALGUNA llamada pasa un contexto
//! que no se puede leer (una variable, `**ctx`), esa plantilla no se juzga; las variables locales (bucles,
//! `set`, `@php`) y las globales del marco (`request`, `loop`, `$errors`…) no cuentan; las plantillas que
//! incluye o extiende tampoco (sólo la que se dibuja).
use super::{cada_nodo, linea, literal, texto, Desajuste, Fuente};
use gotv_sintaxis::Lenguaje;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use tree_sitter::Node;

/// Una llamada que dibuja una plantilla: su nombre, las variables que pasa (`None` = no se pueden saber) y dónde.
struct Dibujo {
    plantilla: String,
    vars: Option<BTreeSet<String>>,
    lugar: String,
    blade: bool,
}

fn claves_de_diccionario(n: Node, t: &str) -> Option<BTreeSet<String>> {
    let mut v = BTreeSet::new();
    let mut c = n.walk();
    for h in n.named_children(&mut c) {
        match h.kind() {
            "pair" => v.insert(h.child_by_field_name("key").and_then(|k| literal(k, t, false))?),
            "comment" => true,
            _ => return None,
        };
    }
    Some(v)
}

fn dibujos_python(f: &Fuente, raiz: Node, out: &mut Vec<Dibujo>) {
    let t = f.texto.as_str();
    cada_nodo(raiz, |n| {
        if n.kind() != "call" {
            return;
        }
        let Some(fun) = n.child_by_field_name("function") else { return };
        let nombre = match fun.kind() {
            "identifier" => texto(fun, t),
            "attribute" => fun.child_by_field_name("attribute").map(|x| texto(x, t)).unwrap_or(""),
            _ => return,
        };
        if !matches!(nombre, "render_template" | "render" | "TemplateResponse" | "render_to_string") {
            return;
        }
        let Some(args) = n.child_by_field_name("arguments") else { return };
        let mut c = args.walk();
        let hijos: Vec<Node> = args.named_children(&mut c).filter(|h| h.kind() != "comment").collect();
        // la plantilla: el primer texto literal entre los posicionales
        let Some(plantilla) = hijos.iter().filter(|h| h.kind() != "keyword_argument").find_map(|h| literal(*h, t, false)) else { return };
        if !plantilla.contains('.') {
            return; // `render(…)` de otra cosa
        }
        let mut vars = Some(BTreeSet::new());
        for h in &hijos {
            match h.kind() {
                "keyword_argument" => {
                    let k = h.child_by_field_name("name").map(|x| texto(x, t)).unwrap_or("");
                    let val = h.child_by_field_name("value");
                    if nombre == "render_template" {
                        if let Some(s) = vars.as_mut() {
                            s.insert(k.to_string());
                        }
                    } else if k == "context" {
                        vars = val.filter(|v| v.kind() == "dictionary").and_then(|v| claves_de_diccionario(v, t));
                    }
                }
                "dictionary_splat" | "list_splat" => vars = None,
                "dictionary" if nombre != "render_template" => match (claves_de_diccionario(*h, t), vars.as_mut()) {
                    (Some(k), Some(s)) => s.extend(k),
                    _ => vars = None,
                },
                // un contexto en una variable: no se puede leer
                "identifier" if nombre != "render_template" && hijos.iter().position(|x| x == h) >= Some(if nombre == "render" { 2 } else { 1 }) && !matches!(texto(*h, t), "request") => vars = None,
                _ => {}
            }
        }
        out.push(Dibujo { plantilla, vars, lugar: format!("{}:{}", f.rel, linea(n)), blade: false });
    });
}

/// Las claves de un `[ 'a' => …, 'b' => … ]` de PHP (`compact('c', 'd')` dentro también vale).
fn claves_php(n: Node, t: &str) -> Option<BTreeSet<String>> {
    match n.kind() {
        "array_creation_expression" => {
            let mut v = BTreeSet::new();
            let mut c = n.walk();
            for e in n.named_children(&mut c).filter(|e| e.kind() == "array_element_initializer") {
                let mut c2 = e.walk();
                let partes: Vec<Node> = e.named_children(&mut c2).collect();
                if partes.len() < 2 {
                    return None; // un elemento sin clave
                }
                v.insert(literal(partes[0], t, false)?);
            }
            Some(v)
        }
        "function_call_expression" if n.child_by_field_name("function").is_some_and(|f| texto(f, t) == "compact") => {
            let args = n.child_by_field_name("arguments")?;
            let mut c = args.walk();
            let v: Option<BTreeSet<String>> = args.named_children(&mut c).filter(|a| a.kind() == "argument").map(|a| a.named_child(0).and_then(|x| literal(x, t, false))).collect();
            v
        }
        _ => None,
    }
}

fn dibujos_php(f: &Fuente, raiz: Node, out: &mut Vec<Dibujo>) {
    let t = f.texto.as_str();
    // `->with('x', …)` encadenados a un `view(…)`: se suman a ese dibujo (por su posición en el texto)
    let mut extra: BTreeMap<usize, BTreeSet<String>> = BTreeMap::new();
    cada_nodo(raiz, |n| {
        if n.kind() == "member_call_expression" && n.child_by_field_name("name").is_some_and(|x| texto(x, t) == "with") {
            let objeto = n.child_by_field_name("object").map(|o| o.start_byte()).unwrap_or(0);
            if let Some(k) = n.child_by_field_name("arguments").and_then(|a| a.named_child(0)).and_then(|a| a.named_child(0)).and_then(|x| literal(x, t, false)) {
                extra.entry(objeto).or_default().insert(k);
            }
        }
    });
    cada_nodo(raiz, |n| {
        let (plantilla_arg, blade) = match n.kind() {
            "function_call_expression" if n.child_by_field_name("function").is_some_and(|f| texto(f, t) == "view") => (true, true),
            "member_call_expression" if n.child_by_field_name("name").is_some_and(|x| texto(x, t) == "render") => (true, false),
            _ => (false, false),
        };
        if !plantilla_arg {
            return;
        }
        let Some(args) = n.child_by_field_name("arguments") else { return };
        let mut c = args.walk();
        let partes: Vec<Node> = args.named_children(&mut c).filter(|a| a.kind() == "argument").filter_map(|a| a.named_child(0)).collect();
        let Some(plantilla) = partes.first().and_then(|p| literal(*p, t, false)) else { return };
        if !blade && !plantilla.ends_with(".twig") {
            return;
        }
        let mut vars = match partes.get(1) {
            None => Some(BTreeSet::new()),
            Some(p) => claves_php(*p, t),
        };
        if let (Some(v), Some(e)) = (vars.as_mut(), extra.get(&n.start_byte())) {
            v.extend(e.iter().cloned());
        }
        out.push(Dibujo { plantilla, vars, lugar: format!("{}:{}", f.rel, linea(n)), blade });
    });
}

const GLOBALES_JINJA: &[&str] = &[
    "loop", "request", "session", "g", "config", "self", "super", "range", "dict", "lipsum", "cycler", "joiner", "namespace", "true", "false", "none", "True", "False", "None", "and", "or", "not", "in", "is", "if", "else", "app", "_self", "csrf_token", "url_for", "get_flashed_messages", "user", "perms", "messages", "static", "block", "varargs", "kwargs", "caller",
];

fn es_id(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Los nombres RAÍZ de una expresión de Jinja/Twig: ni atributos (`.x`), ni filtros (`|x`), ni llamadas
/// (`f(`), ni lo que hay entre comillas.
fn raices(expr: &str) -> Vec<String> {
    let mut v = Vec::new();
    let b: Vec<char> = expr.chars().collect();
    let mut i = 0;
    let mut previo = ' ';
    while i < b.len() {
        let c = b[i];
        if c == '"' || c == '\'' {
            let q = c;
            i += 1;
            while i < b.len() && b[i] != q {
                i += 1;
            }
            i += 1;
            previo = q;
            continue;
        }
        if es_id(c) && !c.is_ascii_digit() {
            let ini = i;
            while i < b.len() && es_id(b[i]) {
                i += 1;
            }
            let palabra: String = b[ini..i].iter().collect();
            let mut j = i;
            while j < b.len() && b[j] == ' ' {
                j += 1;
            }
            let llamada = j < b.len() && b[j] == '(';
            let clave = j < b.len() && b[j] == '=' && b.get(j + 1) != Some(&'=');
            if previo != '.' && previo != '|' && !llamada && !clave {
                v.push(palabra);
            }
            previo = 'a';
            continue;
        }
        if !c.is_whitespace() {
            previo = c;
        }
        i += 1;
    }
    v
}

/// (variable, línea) que usa una plantilla Jinja/Twig, sin las locales ni las globales.
fn usadas_jinja(t: &str) -> BTreeMap<String, usize> {
    let mut locales: BTreeSet<String> = BTreeSet::new();
    let mut usos: Vec<(String, usize)> = Vec::new();
    for (i, l) in t.lines().enumerate() {
        let mut resto = l;
        while let Some(a) = resto.find(['{']) {
            let r = &resto[a..];
            let (cierre, es_bloque) = if r.starts_with("{{") {
                ("}}", false)
            } else if r.starts_with("{%") {
                ("%}", true)
            } else {
                resto = &r[1..];
                continue;
            };
            let Some(z) = r.find(cierre) else { break };
            let dentro = r[2..z].trim().trim_matches('-').trim();
            resto = &r[z + 2..];
            if !es_bloque {
                usos.extend(raices(dentro).into_iter().map(|x| (x, i + 1)));
                continue;
            }
            let mut p = dentro.split_whitespace();
            match p.next().unwrap_or("") {
                "for" => {
                    let (vars, expr) = dentro[3..].split_once(" in ").unwrap_or(("", ""));
                    locales.extend(vars.split(',').map(|x| x.trim().to_string()));
                    usos.extend(raices(expr.split(" if ").next().unwrap_or("")).into_iter().map(|x| (x, i + 1)));
                }
                "set" | "with" => {
                    let (vars, expr) = dentro.split_once('=').unwrap_or((dentro, ""));
                    locales.extend(vars.split_whitespace().skip(1).flat_map(|x| x.split(',')).map(|x| x.trim().to_string()));
                    usos.extend(raices(expr).into_iter().map(|x| (x, i + 1)));
                }
                "macro" | "call" => {
                    if let Some((_, a)) = dentro.split_once('(') {
                        locales.extend(a.trim_end_matches(')').split(',').map(|x| x.split('=').next().unwrap_or("").trim().to_string()));
                    }
                }
                "if" | "elif" => usos.extend(raices(&dentro[dentro.find(' ').unwrap_or(dentro.len())..]).into_iter().map(|x| (x, i + 1))),
                _ => {}
            }
        }
    }
    let mut m = BTreeMap::new();
    for (x, l) in usos {
        if !locales.contains(&x) && !GLOBALES_JINJA.contains(&x.as_str()) {
            m.entry(x).or_insert(l);
        }
    }
    m
}

const GLOBALES_BLADE: &[&str] = &["loop", "errors", "slot", "attributes", "component", "__env", "app", "message", "this", "__data", "__path"];

/// (variable, línea) que usa una plantilla Blade, sin las locales (`@foreach … as $x`, `@php $x =`) ni las globales.
fn usadas_blade(t: &str) -> BTreeMap<String, usize> {
    if t.contains("@props") {
        return BTreeMap::new(); // un componente: sus variables son sus props
    }
    let mut locales = BTreeSet::new();
    for l in t.lines() {
        for parte in l.split(" as ").skip(1) {
            for v in parte.split("=>") {
                if let Some(n) = v.trim().strip_prefix('$') {
                    locales.insert(n.chars().take_while(|c| es_id(*c)).collect::<String>());
                }
            }
        }
        for (j, _) in l.match_indices('$') {
            let n: String = l[j + 1..].chars().take_while(|c| es_id(*c)).collect();
            let tras = l[j + 1 + n.len()..].trim_start();
            if tras.starts_with('=') && !tras.starts_with("==") && !tras.starts_with("=>") {
                locales.insert(n);
            }
        }
    }
    let mut m = BTreeMap::new();
    for (i, l) in t.lines().enumerate() {
        for (j, _) in l.match_indices('$') {
            let n: String = l[j + 1..].chars().take_while(|c| es_id(*c)).collect();
            if !n.is_empty() && !n.starts_with(|c: char| c.is_ascii_digit()) && !locales.contains(&n) && !GLOBALES_BLADE.contains(&n.as_str()) {
                m.entry(n).or_insert(i + 1);
            }
        }
    }
    m
}

/// Las carpetas `templates` del proyecto (hasta 5 niveles; sin `node_modules` ni ocultas).
fn carpetas_de_plantillas(dir: &Path, fondo: usize, out: &mut Vec<PathBuf>) {
    if fondo > 5 {
        return;
    }
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let n = e.file_name().to_string_lossy().to_string();
        if e.file_type().is_ok_and(|t| t.is_dir()) && !n.starts_with('.') && !matches!(n.as_str(), "node_modules" | "target" | "vendor" | "venv" | ".venv" | "dist" | "build") {
            if n == "templates" {
                out.push(e.path());
            }
            carpetas_de_plantillas(&e.path(), fondo + 1, out);
        }
    }
}

/// El archivo de una plantilla: Blade `a.b` → `resources/views/a/b.blade.php`; si no, en alguna carpeta `templates`.
fn archivo_de(root: &Path, d: &Dibujo, carpetas: &[PathBuf]) -> Option<PathBuf> {
    if d.blade {
        let p = root.join("resources/views").join(format!("{}.blade.php", d.plantilla.replace('.', "/")));
        return p.is_file().then_some(p);
    }
    carpetas.iter().map(|c| c.join(&d.plantilla)).find(|p| p.is_file())
}

pub(crate) fn revisar(root: &Path, fuentes: &[Fuente]) -> Vec<Desajuste> {
    let mut dibujos = Vec::new();
    for f in fuentes {
        let candidato = match f.lenguaje {
            Lenguaje::Python => ["render_template", "render(", "TemplateResponse", "render_to_string"].iter().any(|k| f.texto.contains(k)),
            Lenguaje::Php => f.texto.contains("view(") || f.texto.contains("->render("),
            _ => false,
        };
        if !candidato {
            continue;
        }
        let Some(a) = f.arbol() else { continue };
        match f.lenguaje {
            Lenguaje::Python => dibujos_python(f, a.raiz(), &mut dibujos),
            _ => dibujos_php(f, a.raiz(), &mut dibujos),
        }
    }
    if dibujos.is_empty() {
        return vec![];
    }
    let mut carpetas = Vec::new();
    carpetas_de_plantillas(root, 0, &mut carpetas);
    // por plantilla: todas sus llamadas (si alguna no se puede leer, no se juzga)
    let mut por: BTreeMap<PathBuf, (Option<BTreeSet<String>>, Vec<String>)> = BTreeMap::new();
    for d in &dibujos {
        let Some(p) = archivo_de(root, d, &carpetas) else { continue };
        let e = por.entry(p).or_insert_with(|| (Some(BTreeSet::new()), Vec::new()));
        match (&mut e.0, &d.vars) {
            (Some(s), Some(v)) => s.extend(v.iter().cloned()),
            _ => e.0 = None,
        }
        e.1.push(d.lugar.clone());
    }
    let mut v = Vec::new();
    for (p, (pasadas, lugares)) in por {
        let Some(pasadas) = pasadas else { continue };
        let Ok(t) = std::fs::read_to_string(&p) else { continue };
        let blade = p.to_string_lossy().ends_with(".blade.php");
        let usadas = if blade { usadas_blade(&t) } else { usadas_jinja(&t) };
        let rel = p.strip_prefix(root).unwrap_or(&p).display().to_string();
        for (var, l) in usadas {
            if pasadas.contains(&var) {
                continue;
            }
            v.push(Desajuste {
                puente: "plantillas".into(),
                clave: format!("sin-pasar|{rel}|{var}"), // juez-idioma: datos (clave interna estable)
                texto: crate::tr!(
                    "{rel}:{l}: la plantilla usa «{var}», que no le pasa ninguna llamada que la dibuja ({})",
                    "{rel}:{l}: the template uses «{var}», which no call that renders it passes ({})",
                    lugares.join(", ")
                ),
            });
        }
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn variables_de_jinja_y_blade() {
        let j = "{% extends 'base.html' %}\n<h1>{{ titulo | upper }}</h1>\n{% for p in productos if p.activo %}\n  {{ p.nombre }} {{ moneda }}\n{% endfor %}\n{% set total = suma %}{{ total }} {{ url_for('x') }} {{ loop.index }}\n{% if usuario.es_admin and 'x' in permisos %}ok{% endif %}\n";
        assert_eq!(usadas_jinja(j).into_iter().collect::<Vec<_>>(), [("moneda".into(), 4), ("permisos".into(), 7), ("productos".into(), 3), ("suma".into(), 6), ("titulo".into(), 2), ("usuario".into(), 7)]);
        let b = "<h1>{{ $titulo }}</h1>\n@foreach($productos as $k => $p)\n  {{ $p->nombre }} {{ $loop->index }}\n@endforeach\n@php $x = 1; @endphp {{ $x }}\n@error('a') {{ $message }} @enderror\n";
        assert_eq!(usadas_blade(b).into_iter().collect::<Vec<_>>(), [("productos".into(), 2), ("titulo".into(), 1)]);
    }

    #[test]
    fn controladores_contra_sus_plantillas() {
        let d = crate::CarpetaPrueba::nueva("gotv-puente-plantillas");
        std::fs::create_dir_all(d.join("app/templates")).unwrap();
        std::fs::create_dir_all(d.join("resources/views/tienda")).unwrap();
        std::fs::write(d.join("app/templates/inicio.html"), "{{ titulo }} {{ falta_py }}\n").unwrap();
        std::fs::write(d.join("app/templates/libre.html"), "{{ cualquiera }}\n").unwrap();
        std::fs::write(d.join("resources/views/tienda/lista.blade.php"), "{{ $productos }} {{ $total }} {{ $falta_php }}\n").unwrap();
        let py = Fuente { rel: "app/vistas.py".into(), lenguaje: Lenguaje::Python, texto: "def inicio():\n    return render_template('inicio.html', titulo='x')\n\ndef libre(request, ctx):\n    return render(request, 'libre.html', ctx)\n".into() };
        let php = Fuente { rel: "app/Http/Tienda.php".into(), lenguaje: Lenguaje::Php, texto: "<?php\nclass Tienda {\n  function lista() {\n    return view('tienda.lista', ['productos' => []])->with('total', 3);\n  }\n}\n".into() };
        let v = revisar(&d, &[py, php]);
        let claves: Vec<&str> = v.iter().map(|x| x.clave.as_str()).collect();
        assert_eq!(claves, ["sin-pasar|app/templates/inicio.html|falta_py", "sin-pasar|resources/views/tienda/lista.blade.php|falta_php"], "{v:?}");
        assert!(v[0].texto.contains("app/vistas.py:2"), "{}", v[0].texto);
    }
}
