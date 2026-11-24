//! F5 ML9 · el puente HTTP: las llamadas del frontend (`fetch`, `axios`) a una ruta PROPIA (`/api/x`) ↔ las
//! rutas que declara el backend del mismo proyecto: axum (`.route("/x", get(h))`), Express
//! (`app.get("/x", …)`), FastAPI y Flask (`@app.get("/x")`, `@app.route("/x", methods=[…])`), Django
//! (`path("x/", …)`), Laravel (`Route::get('/x', …)`) y Go (`http.HandleFunc("/x", …)`, `"GET /x"`,
//! `r.GET("/x", …)`). Los segmentos variables (`:id`, `{id}`, `<int:id>`, `${id}`) casan con cualquiera.
//! Sólo se juzga si el proyecto declara alguna ruta; una URL a otro sitio no es de este contrato.
use super::{cada_nodo, linea, literal, texto, Desajuste, Fuente};
use gotv_sintaxis::Lenguaje;
use tree_sitter::Node;

struct Ruta {
    /// None = cualquier método
    metodo: Option<String>,
    segs: Vec<String>,
}

struct Llamada {
    metodo: String,
    ruta: String,
    segs: Vec<String>,
    lugar: String,
}

const METODOS: &[&str] = &["get", "post", "put", "delete", "patch"];

/// `/api/users/:id?x=1` → [api, users, *]; `None` si no es una ruta propia (otro sitio, vacía).
fn segmentos(ruta: &str) -> Option<Vec<String>> {
    let mut r = ruta.trim();
    for pre in ["http://localhost", "http://127.0.0.1", "https://localhost"] {
        if let Some(x) = r.strip_prefix(pre) {
            // el puerto: hasta la primera barra
            r = x.find('/').map(|i| &x[i..]).unwrap_or("/");
        }
    }
    if !r.starts_with('/') {
        return None;
    }
    let r = r.split(['?', '#']).next().unwrap_or("");
    Some(
        r.split('/')
            .filter(|s| !s.is_empty())
            .map(|s| if s.starts_with(':') || s.starts_with('{') || s.starts_with('<') || s.contains('*') { "*".to_string() } else { s.to_string() })
            .collect(),
    )
}

fn casa(ruta: &[String], llamada: &[String]) -> bool {
    // comodín final (`/static/*`, `{*resto}`) se traga el resto
    if ruta.last().is_some_and(|s| s == "*") && llamada.len() >= ruta.len() && ruta.len() > 1 && ruta[..ruta.len() - 1].iter().zip(llamada).all(|(a, b)| a == "*" || b == "*" || a == b) {
        return true;
    }
    ruta.len() == llamada.len() && ruta.iter().zip(llamada).all(|(a, b)| a == "*" || b == "*" || a == b)
}

/// Los métodos de la segunda parte de una ruta de axum: `get(h).post(h2)`.
fn metodos_en(t: &str) -> Vec<String> {
    METODOS.iter().filter(|m| t.contains(&format!("{m}(")) || t.contains(&format!("{m}_service("))).map(|m| m.to_string()).collect()
}

fn primer_arg(args: Node) -> Option<Node> {
    let mut c = args.walk();
    let x = args.named_children(&mut c).find(|n| n.kind() != "comment");
    x
}

fn rutas_de(f: &Fuente, raiz: Node, out: &mut Vec<Ruta>) {
    let t = f.texto.as_str();
    let mut pon = |metodos: Vec<String>, ruta: &str| {
        let Some(segs) = segmentos(ruta) else { return };
        if metodos.is_empty() {
            out.push(Ruta { metodo: None, segs });
        } else {
            for m in metodos {
                out.push(Ruta { metodo: Some(m), segs: segs.clone() });
            }
        }
    };
    cada_nodo(raiz, |n| match (f.lenguaje, n.kind()) {
        // axum: .route("/x", get(h).post(h2)) · Go: http.HandleFunc("/x") / mux.HandleFunc("GET /x") / r.GET("/x")
        (Lenguaje::Rust, "call_expression") | (Lenguaje::Go, "call_expression") => {
            let Some(fun) = n.child_by_field_name("function") else { return };
            let campo = fun.child_by_field_name("field").map(|x| texto(x, t)).unwrap_or("");
            let Some(args) = n.child_by_field_name("arguments") else { return };
            let Some(a0) = primer_arg(args).and_then(|a| literal(a, t, false)) else { return };
            match (f.lenguaje, campo) {
                (Lenguaje::Rust, "route") => pon(metodos_en(texto(args, t)), &a0),
                (Lenguaje::Go, "HandleFunc" | "Handle") => match a0.split_once(' ') {
                    Some((m, r)) => pon(vec![m.to_lowercase()], r),
                    None => pon(vec![], &a0),
                },
                (Lenguaje::Go, "GET" | "POST" | "PUT" | "DELETE" | "PATCH" | "Any") if a0.starts_with('/') => {
                    pon(if campo == "Any" { vec![] } else { vec![campo.to_lowercase()] }, &a0)
                }
                _ => {}
            }
        }
        // Express: app.get("/x", h) / router.post(...)
        (Lenguaje::TypeScript | Lenguaje::Tsx | Lenguaje::JavaScript, "call_expression") => {
            let Some(fun) = n.child_by_field_name("function").filter(|x| x.kind() == "member_expression") else { return };
            let obj = fun.child_by_field_name("object").map(|x| texto(x, t)).unwrap_or("");
            let prop = fun.child_by_field_name("property").map(|x| texto(x, t)).unwrap_or("");
            // (no `api`: suele ser una instancia de axios del FRONTEND)
            let servidor = matches!(obj, "app" | "router" | "server") || obj.ends_with("Router") || obj.ends_with("router");
            if !servidor || !(METODOS.contains(&prop) || prop == "all") {
                return;
            }
            let Some(args) = n.child_by_field_name("arguments") else { return };
            if args.named_child_count() < 2 {
                return;
            }
            if let Some(a0) = primer_arg(args).and_then(|a| literal(a, t, false)) {
                pon(if prop == "all" { vec![] } else { vec![prop.to_string()] }, &a0);
            }
        }
        // FastAPI / Flask: @app.get("/x") · @bp.route("/x", methods=["POST"]) · Django: path("x/", vista)
        (Lenguaje::Python, "call") => {
            let Some(fun) = n.child_by_field_name("function") else { return };
            let Some(args) = n.child_by_field_name("arguments") else { return };
            let Some(a0) = primer_arg(args).and_then(|a| literal(a, t, false)) else { return };
            let (nombre, decorador) = match fun.kind() {
                "attribute" => (fun.child_by_field_name("attribute").map(|x| texto(x, t)).unwrap_or(""), n.parent().is_some_and(|p| p.kind() == "decorator")),
                "identifier" => (texto(fun, t), false),
                _ => return,
            };
            match nombre {
                m if decorador && METODOS.contains(&m) => pon(vec![m.to_string()], &a0),
                "route" if decorador => {
                    let a = texto(args, t).to_lowercase();
                    let ms: Vec<String> = METODOS.iter().filter(|m| a.contains(&format!("\"{m}\"")) || a.contains(&format!("'{m}'"))).map(|m| m.to_string()).collect();
                    pon(if a.contains("methods") { ms } else { vec!["get".into()] }, &a0)
                }
                "path" | "re_path" if !decorador && f.rel.ends_with("urls.py") => pon(vec![], &format!("/{}", a0.trim_start_matches('^').trim_end_matches('$'))),
                _ => {}
            }
        }
        // Laravel: Route::get('/x', ...)
        (Lenguaje::Php, "scoped_call_expression") => {
            let alcance = n.child_by_field_name("scope").map(|x| texto(x, t)).unwrap_or("");
            let nombre = n.child_by_field_name("name").map(|x| texto(x, t).to_lowercase()).unwrap_or_default();
            if alcance != "Route" {
                return;
            }
            let Some(args) = n.child_by_field_name("arguments") else { return };
            let a0 = primer_arg(args).and_then(|a| a.named_child(0).or(Some(a))).and_then(|a| literal(a, t, false));
            if let Some(a0) = a0 {
                let r = if a0.starts_with('/') { a0 } else { format!("/{a0}") };
                match nombre.as_str() {
                    m if METODOS.contains(&m) => pon(vec![m.to_string()], &r),
                    "any" | "match" | "resource" | "apiresource" => pon(vec![], &r),
                    _ => {}
                }
            }
        }
        _ => {}
    });
}

/// El método de un `fetch(url, { method: "POST" })` (GET si no lo dice).
fn metodo_de_fetch(opts: Option<Node>, t: &str) -> String {
    let Some(o) = opts.filter(|o| o.kind() == "object") else { return "get".into() };
    let mut c = o.walk();
    for p in o.named_children(&mut c).filter(|p| p.kind() == "pair") {
        if p.child_by_field_name("key").is_some_and(|k| texto(k, t).trim_matches(|c| c == '"' || c == '\'') == "method") {
            if let Some(v) = p.child_by_field_name("value").and_then(|v| literal(v, t, false)) {
                return v.to_lowercase();
            }
            return "?".into();
        }
    }
    "get".into()
}

fn llamadas_de(f: &Fuente, raiz: Node, out: &mut Vec<Llamada>) {
    let t = f.texto.as_str();
    cada_nodo(raiz, |n| {
        if n.kind() != "call_expression" {
            return;
        }
        let Some(fun) = n.child_by_field_name("function") else { return };
        let Some(args) = n.child_by_field_name("arguments") else { return };
        let mut c = args.walk();
        let hijos: Vec<Node> = args.named_children(&mut c).filter(|x| x.kind() != "comment").collect();
        let Some(url) = hijos.first().and_then(|a| literal(*a, t, true)) else { return };
        let metodo = match fun.kind() {
            "identifier" if texto(fun, t) == "fetch" => metodo_de_fetch(hijos.get(1).copied(), t),
            "member_expression" => {
                let obj = fun.child_by_field_name("object").map(|x| texto(x, t)).unwrap_or("");
                let prop = fun.child_by_field_name("property").map(|x| texto(x, t)).unwrap_or("");
                if obj == "axios" && METODOS.contains(&prop) {
                    prop.to_string()
                } else {
                    return;
                }
            }
            _ => return,
        };
        let Some(segs) = segmentos(&url) else { return };
        out.push(Llamada { metodo, ruta: url, segs, lugar: format!("{}:{}", f.rel, linea(n)) });
    });
}

pub(crate) fn revisar(fuentes: &[Fuente]) -> Vec<Desajuste> {
    let mut rutas = Vec::new();
    let mut llamadas = Vec::new();
    for f in fuentes {
        let backend = match f.lenguaje {
            Lenguaje::Rust => f.texto.contains(".route("),
            Lenguaje::Go => f.texto.contains("Handle") || f.texto.contains(".GET(") || f.texto.contains(".POST("),
            Lenguaje::Python => f.texto.contains("@") || f.rel.ends_with("urls.py"),
            Lenguaje::Php => f.texto.contains("Route::"),
            Lenguaje::TypeScript | Lenguaje::Tsx | Lenguaje::JavaScript => f.texto.contains("app.") || f.texto.contains("outer."),
            _ => false,
        };
        let frontend = matches!(f.lenguaje, Lenguaje::TypeScript | Lenguaje::Tsx | Lenguaje::JavaScript) && (f.texto.contains("fetch(") || f.texto.contains("axios."));
        if !backend && !frontend {
            continue;
        }
        let Some(a) = f.arbol() else { continue };
        if backend {
            rutas_de(f, a.raiz(), &mut rutas);
        }
        if frontend {
            llamadas_de(f, a.raiz(), &mut llamadas);
        }
    }
    // sin rutas propias declaradas, el backend no es de este proyecto (o no se reconoce): no se juzga
    if rutas.is_empty() {
        return vec![];
    }
    let mut v = Vec::new();
    for l in &llamadas {
        let archivo = l.lugar.rsplit_once(':').map(|x| x.0).unwrap_or(&l.lugar);
        let por_ruta: Vec<&Ruta> = rutas.iter().filter(|r| casa(&r.segs, &l.segs)).collect();
        if por_ruta.is_empty() {
            v.push(Desajuste {
                puente: "http".into(),
                clave: format!("sin-ruta|{}|{archivo}", l.segs.join("/")), // juez-idioma: datos (clave interna estable)
                texto: crate::tr!(
                    "{}: {} «{}» — ninguna ruta del backend del proyecto casa con ella",
                    "{}: {} «{}» — no route in the project's backend matches it",
                    l.lugar, l.metodo.to_uppercase(), l.ruta
                ),
            });
        } else if l.metodo != "?" && !por_ruta.iter().any(|r| r.metodo.as_deref().is_none_or(|m| m == l.metodo)) {
            let tiene: Vec<String> = por_ruta.iter().filter_map(|r| r.metodo.clone()).map(|m| m.to_uppercase()).collect();
            v.push(Desajuste {
                puente: "http".into(),
                clave: format!("metodo|{}|{}|{archivo}", l.metodo, l.segs.join("/")), // juez-idioma: datos (clave interna estable)
                texto: crate::tr!(
                    "{}: {} «{}» — esa ruta existe sólo con {}",
                    "{}: {} «{}» — that route exists only with {}",
                    l.lugar, l.metodo.to_uppercase(), l.ruta, tiene.join(", ")
                ),
            });
        }
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(rel: &str, l: Lenguaje, t: &str) -> Fuente {
        Fuente { rel: rel.into(), texto: t.into(), lenguaje: l }
    }

    #[test]
    fn segmentos_y_comodines() {
        assert_eq!(segmentos("/api/users/:id?x=1").unwrap(), ["api", "users", "*"]);
        assert_eq!(segmentos("http://localhost:3000/api/x").unwrap(), ["api", "x"]);
        assert!(segmentos("https://api.github.com/repos").is_none(), "otro sitio");
        assert!(casa(&segmentos("/api/users/{id}").unwrap(), &segmentos("/api/users/*").unwrap()));
        assert!(casa(&segmentos("/static/*").unwrap(), &segmentos("/static/css/a.css").unwrap()));
        assert!(!casa(&segmentos("/api/users").unwrap(), &segmentos("/api/user").unwrap()));
    }

    #[test]
    fn llamadas_contra_rutas_de_cada_backend() {
        let axum = f("server/src/main.rs", Lenguaje::Rust, "fn app() -> Router {\n    Router::new().route(\"/api/notas\", get(listar).post(crear)).route(\"/api/notas/{id}\", delete(borrar))\n}\n");
        let fastapi = f("api/main.py", Lenguaje::Python, "@app.get(\"/api/salud\")\ndef salud():\n    return {}\n@bp.route(\"/api/login\", methods=[\"POST\"])\ndef login():\n    pass\n");
        let django = f("web/urls.py", Lenguaje::Python, "urlpatterns = [path(\"api/perfil/<int:pk>/\", vista)]\n");
        let express = f("srv/app.js", Lenguaje::JavaScript, "app.get('/api/hora', (req, res) => res.send('x'));\n");
        let laravel = f("routes/api.php", Lenguaje::Php, "<?php\nRoute::put('/api/ajustes', [Ajustes::class, 'guardar']);\n");
        let go = f("cmd/main.go", Lenguaje::Go, "package main\nfunc main() {\n\thttp.HandleFunc(\"GET /api/ping\", ping)\n}\n");
        let front = f("web/src/api.ts", Lenguaje::TypeScript, "export async function todo(id: number) {\n  await fetch('/api/notas');\n  await fetch(`/api/notas/${id}`, { method: 'DELETE' });\n  await fetch('/api/notas', { method: 'PUT' });\n  await axios.get('/api/salud');\n  await axios.get('/api/login');\n  await fetch('/api/perfil/7/');\n  await fetch('/api/hora');\n  await fetch('/api/ajustes', { method: 'PUT' });\n  await fetch('/api/ping');\n  await fetch('/api/nada');\n  await fetch('https://otro.sitio/api/nada');\n}\n");
        let v = revisar(&[axum, fastapi, django, express, laravel, go, front]);
        let claves: Vec<&str> = v.iter().map(|d| d.clave.as_str()).collect();
        assert_eq!(claves, ["metodo|put|api/notas|web/src/api.ts", "metodo|get|api/login|web/src/api.ts", "sin-ruta|api/nada|web/src/api.ts"], "{v:?}");
        assert!(v[2].texto.contains("web/src/api.ts:11"), "{}", v[2].texto);
    }

    #[test]
    fn sin_backend_propio_no_se_juzga() {
        let front = f("a.ts", Lenguaje::TypeScript, "fetch('/api/x');\n");
        assert!(revisar(&[front]).is_empty());
    }
}
