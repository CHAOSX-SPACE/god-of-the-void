//! F5 ML9 · el puente de Tauri: `invoke("cmd", {args})` en JS/TS ↔ `#[tauri::command] fn cmd(args)` en
//! Rust, y su registro en `generate_handler![…]`. Por omisión Tauri pasa los argumentos de Rust a camelCase
//! (`user_id` ↔ `userId`); con `rename_all = "snake_case"`, tal cual. Los que inyecta Tauri (`State`,
//! `AppHandle`, `Window`, `Webview`, `Request`) no los manda JS.
use super::{camel, cada_nodo, linea, literal, texto, Desajuste, Fuente};
use gotv_sintaxis::Lenguaje;
use std::collections::{BTreeMap, BTreeSet};
use tree_sitter::Node;

struct Comando {
    /// (clave en JS, ¿opcional?)
    params: Vec<(String, bool)>,
    lugar: String,
}

struct Uso {
    nombre: String,
    /// las claves del objeto de argumentos; `None` si no se pueden saber (`...spread`, una variable)
    claves: Option<Vec<String>>,
    lugar: String,
}

const INYECTADOS: &[&str] = &["State<", "AppHandle", "Window", "Webview", "ipc::Request", "Request<", "tauri::State"];

/// ¿Lleva este `function_item` el atributo de comando? Devuelve el texto del atributo.
fn atributo_de_comando<'a>(f: Node, t: &'a str) -> Option<&'a str> {
    let mut a = f.prev_sibling();
    while let Some(n) = a {
        if n.kind() != "attribute_item" {
            break;
        }
        let x = texto(n, t);
        if x.contains("tauri::command") || x.starts_with("#[command") {
            return Some(x);
        }
        a = n.prev_sibling();
    }
    None
}

fn comandos(f: &Fuente, t: &str, raiz: Node, out: &mut BTreeMap<String, Comando>, registrados: &mut Option<BTreeSet<String>>) {
    cada_nodo(raiz, |n| match n.kind() {
        "function_item" => {
            let Some(attr) = atributo_de_comando(n, t) else { return };
            let Some(nombre) = n.child_by_field_name("name").map(|x| texto(x, t).to_string()) else { return };
            let snake = attr.contains("snake_case");
            let mut params = Vec::new();
            if let Some(ps) = n.child_by_field_name("parameters") {
                let mut c = ps.walk();
                for p in ps.named_children(&mut c).filter(|p| p.kind() == "parameter") {
                    let tipo = p.child_by_field_name("type").map(|x| texto(x, t)).unwrap_or("");
                    if INYECTADOS.iter().any(|i| tipo.contains(i)) {
                        continue;
                    }
                    let pat = p.child_by_field_name("pattern").map(|x| texto(x, t)).unwrap_or("");
                    let pat = pat.trim_start_matches("mut ").trim_start_matches('_');
                    if pat.is_empty() {
                        continue;
                    }
                    params.push((if snake { pat.to_string() } else { camel(pat) }, tipo.trim_start().starts_with("Option<")));
                }
            }
            out.insert(nombre, Comando { params, lugar: format!("{}:{}", f.rel, linea(n)) });
        }
        "macro_invocation" => {
            let m = n.child_by_field_name("macro").map(|x| texto(x, t)).unwrap_or("");
            if m.ends_with("generate_handler") {
                let cuerpo = n.named_children(&mut n.walk()).find(|c| c.kind() == "token_tree").map(|c| texto(c, t)).unwrap_or("");
                let set = registrados.get_or_insert_with(BTreeSet::new);
                for nombre in cuerpo.trim_matches(|c| c == '[' || c == ']' || c == '(' || c == ')').split(',') {
                    let nombre = nombre.trim().rsplit("::").next().unwrap_or("").trim();
                    if !nombre.is_empty() {
                        set.insert(nombre.to_string());
                    }
                }
            }
        }
        _ => {}
    });
}

fn usos(f: &Fuente, t: &str, raiz: Node, out: &mut Vec<Uso>) {
    cada_nodo(raiz, |n| {
        if n.kind() != "call_expression" {
            return;
        }
        let Some(fun) = n.child_by_field_name("function") else { return };
        let es_invoke = match fun.kind() {
            "identifier" => texto(fun, t) == "invoke",
            "member_expression" => fun.child_by_field_name("property").is_some_and(|p| texto(p, t) == "invoke"),
            _ => false,
        };
        if !es_invoke {
            return;
        }
        let Some(args) = n.child_by_field_name("arguments") else { return };
        let mut c = args.walk();
        let mut hijos = args.named_children(&mut c);
        let Some(nombre) = hijos.next().and_then(|a| literal(a, t, false)) else { return };
        let claves = match hijos.next() {
            None => Some(vec![]),
            Some(o) if o.kind() == "object" => {
                let mut v = Vec::new();
                let mut conocidas = true;
                let mut c2 = o.walk();
                for h in o.named_children(&mut c2) {
                    match h.kind() {
                        "pair" => match h.child_by_field_name("key") {
                            Some(k) => v.push(literal(k, t, false).unwrap_or_else(|| texto(k, t).to_string())),
                            None => conocidas = false,
                        },
                        "shorthand_property_identifier" => v.push(texto(h, t).to_string()),
                        "comment" => {}
                        _ => conocidas = false,
                    }
                }
                conocidas.then_some(v)
            }
            Some(_) => None,
        };
        out.push(Uso { nombre, claves, lugar: format!("{}:{}", f.rel, linea(n)) });
    });
}

pub(crate) fn revisar(fuentes: &[Fuente]) -> Vec<Desajuste> {
    // ¿hay Tauri en el Rust del proyecto? (si no, un `invoke` es de otra biblioteca)
    if !fuentes.iter().any(|f| f.lenguaje == Lenguaje::Rust && f.texto.contains("tauri")) {
        return vec![];
    }
    let mut cmds = BTreeMap::new();
    let mut registrados = None;
    let mut llamadas = Vec::new();
    for f in fuentes {
        match f.lenguaje {
            Lenguaje::Rust if f.texto.contains("command") || f.texto.contains("generate_handler") => {
                if let Some(a) = f.arbol() {
                    comandos(f, &f.texto, a.raiz(), &mut cmds, &mut registrados);
                }
            }
            Lenguaje::TypeScript | Lenguaje::Tsx | Lenguaje::JavaScript if f.texto.contains("invoke") => {
                if let Some(a) = f.arbol() {
                    usos(f, &f.texto, a.raiz(), &mut llamadas);
                }
            }
            _ => {}
        }
    }
    let mut v = Vec::new();
    let mut d = |clave: String, texto: String| v.push(Desajuste { puente: "tauri".into(), clave, texto });
    for u in &llamadas {
        let archivo = u.lugar.rsplit_once(':').map(|x| x.0).unwrap_or(&u.lugar);
        let Some(c) = cmds.get(&u.nombre) else {
            d(format!("falta|{}|{archivo}", u.nombre), crate::tr!(
                "{}: invoke(«{}») — no hay ningún `#[tauri::command] fn {}` en el Rust del proyecto",
                "{}: invoke(«{}») — there is no `#[tauri::command] fn {}` in the project's Rust",
                u.lugar, u.nombre, u.nombre
            ));
            continue;
        };
        if registrados.as_ref().is_some_and(|r| !r.contains(&u.nombre)) {
            d(format!("sin-registro|{}", u.nombre), crate::tr!(
                "{}: invoke(«{}») — `fn {}` ({}) no está en `generate_handler![…]`: la llamada fallará",
                "{}: invoke(«{}») — `fn {}` ({}) is not in `generate_handler![…]`: the call will fail",
                u.lugar, u.nombre, u.nombre, c.lugar
            ));
        }
        let Some(claves) = &u.claves else { continue };
        for (p, opcional) in &c.params {
            if !opcional && !claves.contains(p) {
                d(format!("arg-falta|{}|{p}|{archivo}", u.nombre), crate::tr!(
                    "{}: invoke(«{}») no manda «{p}», que `fn {}` ({}) pide",
                    "{}: invoke(«{}») does not send «{p}», which `fn {}` ({}) requires",
                    u.lugar, u.nombre, u.nombre, c.lugar
                ));
            }
        }
        for k in claves {
            if !c.params.iter().any(|(p, _)| p == k) {
                d(format!("arg-sobra|{}|{k}|{archivo}", u.nombre), crate::tr!(
                    "{}: invoke(«{}») manda «{k}», que `fn {}` ({}) no tiene (¿snake_case contra camelCase?)",
                    "{}: invoke(«{}») sends «{k}», which `fn {}` ({}) does not have (snake_case vs camelCase?)",
                    u.lugar, u.nombre, u.nombre, c.lugar
                ));
            }
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
    fn invoke_contra_comando() {
        let rs = f("src-tauri/src/lib.rs", Lenguaje::Rust, "#[tauri::command]\nasync fn guardar_nota(app: tauri::AppHandle, user_id: u32, texto: Option<String>) -> Result<(), String> { Ok(()) }\n#[tauri::command(rename_all = \"snake_case\")]\nfn borrar(nota_id: u32) {}\n#[tauri::command]\nfn suelto() {}\npub fn run() { tauri::Builder::default().invoke_handler(tauri::generate_handler![guardar_nota, comandos::borrar]); }\n");
        let ts = f("src/App.tsx", Lenguaje::Tsx, "import { invoke } from '@tauri-apps/api/core';\nexport function App() {\n  invoke<void>('guardar_nota', { userId: 1 });\n  invoke('guardar_nota', { user_id: 1 });\n  invoke('borrar', { nota_id: 2 });\n  invoke('suelto');\n  invoke('no_existe', {});\n  invoke('guardar_nota', datos);\n  return <div/>;\n}\n");
        let v = revisar(&[rs, ts]);
        let claves: Vec<&str> = v.iter().map(|d| d.clave.as_str()).collect();
        assert!(claves.contains(&"falta|no_existe|src/App.tsx"), "{claves:?}");
        assert!(claves.contains(&"sin-registro|suelto"), "{claves:?}");
        assert!(claves.contains(&"arg-falta|guardar_nota|userId|src/App.tsx") && claves.contains(&"arg-sobra|guardar_nota|user_id|src/App.tsx"), "{claves:?}");
        // userId bien (texto es opcional), borrar con snake_case bien, la variable `datos` no se juzga
        assert_eq!(v.len(), 4, "{claves:?}");
        assert!(v.iter().any(|d| d.texto.contains("src/App.tsx:7")), "{v:?}");
    }

    #[test]
    fn sin_tauri_un_invoke_es_de_otra_biblioteca() {
        let js = f("a.js", Lenguaje::JavaScript, "lambda.invoke('x');\ninvoke('y');\n");
        assert!(revisar(&[js]).is_empty());
    }
}
