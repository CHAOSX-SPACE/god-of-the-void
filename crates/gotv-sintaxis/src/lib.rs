//! F5 ML7 · el plan B NATIVO del motor de lenguaje: tree-sitter con 12 gramáticas compiladas dentro.
//!
//! Sin servidor de lenguaje instalado (o mientras arranca), GOTV igual ve los errores de SINTAXIS de un
//! archivo al instante y su esquema de símbolos (funciones, clases, tipos…), y encuentra definiciones por
//! nombre. No sabe de tipos ni de imports: eso es del servidor (nivel 2) o del revisor de Python.
//!
//! [`analizar`] → [`Arbol`]; [`errores`] → los nodos de error y las piezas que faltan; [`simbolos`] → las
//! definiciones con su línea y su anidamiento. Sólo datos: los mensajes para el modelo los arma quien llama.

use std::path::Path;
use tree_sitter::{Node, Parser, Tree};

/// Los lenguajes con gramática dentro de GOTV.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Lenguaje {
    Rust,
    Python,
    TypeScript,
    Tsx,
    JavaScript,
    Go,
    Php,
    Swift,
    C,
    Html,
    Css,
    Json,
}

impl Lenguaje {
    pub const TODOS: [Lenguaje; 12] = [Lenguaje::Rust, Lenguaje::Python, Lenguaje::TypeScript, Lenguaje::Tsx, Lenguaje::JavaScript, Lenguaje::Go, Lenguaje::Php, Lenguaje::Swift, Lenguaje::C, Lenguaje::Html, Lenguaje::Css, Lenguaje::Json];

    /// Por la extensión del archivo (`None`: sin gramática aquí). `.h` va como C; C++ no tiene gramática
    /// (la de C daría errores falsos).
    pub fn de_ruta(p: &Path) -> Option<Lenguaje> {
        let e = p.extension()?.to_string_lossy().to_lowercase();
        Some(match e.as_str() {
            "rs" => Lenguaje::Rust,
            "py" | "pyi" => Lenguaje::Python,
            "ts" | "mts" | "cts" => Lenguaje::TypeScript,
            "tsx" => Lenguaje::Tsx,
            "js" | "mjs" | "cjs" | "jsx" => Lenguaje::JavaScript,
            "go" => Lenguaje::Go,
            "php" => Lenguaje::Php,
            "swift" => Lenguaje::Swift,
            "c" | "h" => Lenguaje::C,
            "html" | "htm" => Lenguaje::Html,
            "css" => Lenguaje::Css,
            "json" => Lenguaje::Json,
            _ => return None,
        })
    }

    pub fn nombre(self) -> &'static str {
        match self {
            Lenguaje::Rust => "rust",
            Lenguaje::Python => "python",
            Lenguaje::TypeScript => "typescript",
            Lenguaje::Tsx => "tsx",
            Lenguaje::JavaScript => "javascript",
            Lenguaje::Go => "go",
            Lenguaje::Php => "php",
            Lenguaje::Swift => "swift",
            Lenguaje::C => "c",
            Lenguaje::Html => "html",
            Lenguaje::Css => "css",
            Lenguaje::Json => "json",
        }
    }

    fn gramatica(self) -> tree_sitter::Language {
        match self {
            Lenguaje::Rust => tree_sitter_rust::LANGUAGE.into(),
            Lenguaje::Python => tree_sitter_python::LANGUAGE.into(),
            Lenguaje::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            Lenguaje::Tsx => tree_sitter_typescript::LANGUAGE_TSX.into(),
            Lenguaje::JavaScript => tree_sitter_javascript::LANGUAGE.into(),
            Lenguaje::Go => tree_sitter_go::LANGUAGE.into(),
            // con su HTML alrededor (`<?php … ?>` dentro de una página)
            Lenguaje::Php => tree_sitter_php::LANGUAGE_PHP.into(),
            Lenguaje::Swift => tree_sitter_swift::LANGUAGE.into(),
            Lenguaje::C => tree_sitter_c::LANGUAGE.into(),
            Lenguaje::Html => tree_sitter_html::LANGUAGE.into(),
            Lenguaje::Css => tree_sitter_css::LANGUAGE.into(),
            Lenguaje::Json => tree_sitter_json::LANGUAGE.into(),
        }
    }
}

/// Un archivo analizado: el árbol y su texto.
pub struct Arbol {
    pub lenguaje: Lenguaje,
    arbol: Tree,
    texto: String,
}

impl Arbol {
    pub fn texto(&self) -> &str {
        &self.texto
    }

    /// La raíz del árbol de tree-sitter (para quien necesite ir más allá de errores y símbolos).
    pub fn raiz(&self) -> Node<'_> {
        self.arbol.root_node()
    }
}

/// Analiza `texto` con la gramática de `lenguaje`. `None` sólo si tree-sitter no pudo (texto enorme
/// cortado por su tope, gramática incompatible).
pub fn analizar(lenguaje: Lenguaje, texto: &str) -> Option<Arbol> {
    let mut p = Parser::new();
    p.set_language(&lenguaje.gramatica()).ok()?;
    let arbol = p.parse(texto, None)?;
    Some(Arbol { lenguaje, arbol, texto: texto.to_string() })
}

/// Un error de sintaxis: dónde (1-based) y qué.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ErrorSintaxis {
    pub linea: usize,
    pub columna: usize,
    /// la pieza que FALTA (`;`, `)`, `identifier`…), si es eso
    pub falta: Option<String>,
    /// si no, el comienzo de lo que no se entiende (una línea, ≤ 50 caracteres)
    pub cerca: String,
}

fn texto_de<'a>(n: Node, t: &'a str) -> &'a str {
    t.get(n.byte_range()).unwrap_or("")
}

/// Los errores de sintaxis, en orden de aparición, como mucho `tope`. Un error no se repite en sus hijos.
pub fn errores(a: &Arbol, tope: usize) -> Vec<ErrorSintaxis> {
    let mut v = Vec::new();
    if !a.raiz().has_error() {
        return v;
    }
    let mut pila = vec![a.raiz()];
    while let Some(n) = pila.pop() {
        if v.len() >= tope {
            break;
        }
        let pos = n.start_position();
        if n.is_missing() {
            v.push(ErrorSintaxis { linea: pos.row + 1, columna: pos.column + 1, falta: Some(n.kind().to_string()), cerca: String::new() });
            continue;
        }
        if n.is_error() {
            let cerca: String = texto_de(n, &a.texto).lines().next().unwrap_or("").trim().chars().take(50).collect();
            v.push(ErrorSintaxis { linea: pos.row + 1, columna: pos.column + 1, falta: None, cerca });
            continue;
        }
        if n.has_error() {
            // en orden: la pila saca el último, así que se meten al revés
            let mut c = n.walk();
            let hijos: Vec<Node> = n.children(&mut c).collect();
            pila.extend(hijos.into_iter().rev());
        }
    }
    v.sort_by_key(|e| (e.linea, e.columna));
    v
}

/// Una definición del archivo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Simbolo {
    pub nombre: String,
    /// `fn`, `method`, `class`, `struct`, `enum`, `trait`, `interface`, `type`, `impl`, `mod`, `const`,
    /// `protocol`, `extension`, `selector`, `key`
    pub tipo: &'static str,
    /// 1-based
    pub linea: usize,
    /// cuántas definiciones la contienen (un método de una clase: 1)
    pub nivel: usize,
}

/// El nombre de una declaración de C: baja por los `declarator` hasta el identificador.
fn nombre_c<'a>(n: Node, t: &'a str) -> Option<&'a str> {
    let mut d = n.child_by_field_name("declarator")?;
    for _ in 0..8 {
        if matches!(d.kind(), "identifier" | "field_identifier" | "type_identifier") {
            return Some(texto_de(d, t));
        }
        d = d.child_by_field_name("declarator")?;
    }
    None
}

fn nombre_campo<'a>(n: Node, t: &'a str) -> Option<&'a str> {
    n.child_by_field_name("name").map(|x| texto_de(x, t)).filter(|s| !s.is_empty())
}

/// ¿Qué definición es este nodo, y cómo se llama?
fn definicion<'a>(l: Lenguaje, n: Node, t: &'a str, nivel: usize) -> Option<(&'static str, String)> {
    let k = n.kind();
    let con = |tipo: &'static str| nombre_campo(n, t).map(|s| (tipo, s.to_string()));
    match l {
        Lenguaje::Rust => match k {
            "function_item" | "function_signature_item" => con(if nivel > 0 { "method" } else { "fn" }),
            "struct_item" => con("struct"),
            "enum_item" => con("enum"),
            "trait_item" => con("trait"),
            "mod_item" => con("mod"),
            "type_item" => con("type"),
            "const_item" | "static_item" => con("const"),
            "macro_definition" => con("macro"),
            "impl_item" => {
                let ty = n.child_by_field_name("type").map(|x| texto_de(x, t))?;
                Some(("impl", match n.child_by_field_name("trait") {
                    Some(tr) => format!("{} for {ty}", texto_de(tr, t)),
                    None => ty.to_string(),
                }))
            }
            _ => None,
        },
        Lenguaje::Python => match k {
            "function_definition" => con(if nivel > 0 { "method" } else { "fn" }),
            "class_definition" => con("class"),
            _ => None,
        },
        Lenguaje::TypeScript | Lenguaje::Tsx | Lenguaje::JavaScript => match k {
            // `function_signature`: `declare function f(): void;` de un `.d.ts`
            "function_declaration" | "generator_function_declaration" | "function_signature" => con("fn"),
            "class_declaration" | "abstract_class_declaration" => con("class"),
            "method_definition" | "method_signature" | "abstract_method_signature" => con("method"),
            "interface_declaration" => con("interface"),
            "type_alias_declaration" => con("type"),
            "enum_declaration" => con("enum"),
            "internal_module" | "module" => con("mod"),
            // `const f = () => …` / `= function …` / `= class …`
            "variable_declarator" => {
                let v = n.child_by_field_name("value")?;
                matches!(v.kind(), "arrow_function" | "function_expression" | "function" | "class").then(|| con(if v.kind() == "class" { "class" } else { "fn" }))?
            }
            _ => None,
        },
        Lenguaje::Go => match k {
            "function_declaration" => con("fn"),
            "method_declaration" => con("method"),
            "type_spec" => {
                let ty = n.child_by_field_name("type").map(|x| x.kind()).unwrap_or("");
                con(match ty {
                    "struct_type" => "struct",
                    "interface_type" => "interface",
                    _ => "type",
                })
            }
            _ => None,
        },
        Lenguaje::Php => match k {
            "function_definition" => con("fn"),
            "method_declaration" => con("method"),
            "class_declaration" => con("class"),
            "interface_declaration" => con("interface"),
            "trait_declaration" => con("trait"),
            "enum_declaration" => con("enum"),
            _ => None,
        },
        Lenguaje::Swift => match k {
            "function_declaration" | "protocol_function_declaration" => con(if nivel > 0 { "method" } else { "fn" }),
            "init_declaration" => Some(("method", "init".to_string())),
            "protocol_declaration" => con("protocol"),
            "typealias_declaration" => con("type"),
            "class_declaration" => {
                let tipo = match n.child_by_field_name("declaration_kind").map(|x| texto_de(x, t)) {
                    Some("struct") => "struct",
                    Some("enum") => "enum",
                    Some("extension") => "extension",
                    Some("actor") => "class",
                    _ => "class",
                };
                // una extensión se llama por el tipo que extiende
                n.child_by_field_name("name").map(|x| (tipo, texto_de(x, t).to_string()))
            }
            _ => None,
        },
        Lenguaje::C => match k {
            "function_definition" => nombre_c(n, t).map(|s| ("fn", s.to_string())),
            "struct_specifier" | "union_specifier" if n.child_by_field_name("body").is_some() => con("struct"),
            "enum_specifier" if n.child_by_field_name("body").is_some() => con("enum"),
            "type_definition" => nombre_c(n, t).map(|s| ("type", s.to_string())),
            _ => None,
        },
        Lenguaje::Css => match k {
            "rule_set" if nivel == 0 => n.child(0).filter(|c| c.kind() == "selectors").map(|c| ("selector", texto_de(c, t).trim().to_string())),
            "keyframes_statement" => n.child_by_field_name("name").or_else(|| n.named_child(0)).map(|c| ("selector", format!("@keyframes {}", texto_de(c, t)))),
            _ => None,
        },
        Lenguaje::Json => match k {
            // sólo las claves de primer nivel (un JSON grande no es una lista de símbolos)
            "pair" if nivel == 0 && n.parent().and_then(|p| p.parent()).is_some_and(|g| g.kind() == "document") => {
                n.child_by_field_name("key").map(|c| ("key", texto_de(c, t).trim_matches('"').to_string()))
            }
            _ => None,
        },
        Lenguaje::Html => None,
    }
}

/// Las definiciones del archivo, en orden, como mucho `tope`.
pub fn simbolos(a: &Arbol, tope: usize) -> Vec<Simbolo> {
    let mut v = Vec::new();
    // (nodo, nivel de anidamiento entre definiciones)
    let mut pila = vec![(a.raiz(), 0usize)];
    while let Some((n, nivel)) = pila.pop() {
        if v.len() >= tope {
            break;
        }
        let def = definicion(a.lenguaje, n, &a.texto, nivel);
        if let Some((tipo, nombre)) = &def {
            v.push(Simbolo { nombre: nombre.clone(), tipo, linea: n.start_position().row + 1, nivel });
        }
        let dentro = nivel + usize::from(def.is_some());
        let mut c = n.walk();
        let hijos: Vec<Node> = n.named_children(&mut c).collect();
        pila.extend(hijos.into_iter().rev().map(|h| (h, dentro)));
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nombres(l: Lenguaje, t: &str) -> Vec<String> {
        let a = analizar(l, t).unwrap();
        simbolos(&a, 100).into_iter().map(|s| format!("{}{} {} {}", "  ".repeat(s.nivel), s.linea, s.tipo, s.nombre)).collect()
    }

    /// Cada una de las 12 gramáticas carga (ABI compatible con el tree-sitter enlazado) y un texto vacío
    /// no da errores.
    #[test]
    fn las_doce_gramaticas_cargan() {
        for l in Lenguaje::TODOS {
            let a = analizar(l, "").unwrap_or_else(|| panic!("{l:?} no carga"));
            assert!(errores(&a, 10).is_empty(), "{l:?}");
        }
        assert_eq!(Lenguaje::de_ruta(Path::new("a/b.TSX")), Some(Lenguaje::Tsx));
        assert_eq!(Lenguaje::de_ruta(Path::new("a.cpp")), None, "C++ no: la gramática de C daría errores falsos");
    }

    #[test]
    fn errores_de_sintaxis_en_su_linea() {
        let a = analizar(Lenguaje::Rust, "fn main() {\n    let x = 1\n    println!(\"{x}\");\n}\n").unwrap();
        let e = errores(&a, 10);
        assert_eq!(e.len(), 1, "{e:?}");
        // medido: tree-sitter-rust lo da como nodo de error en la línea 2 (no como «;» que falta)
        assert_eq!(e[0].linea, 2, "{e:?}");
        let a = analizar(Lenguaje::C, "int f(void) {\n    return 1\n}\n").unwrap();
        assert!(errores(&a, 10).iter().any(|e| e.linea == 2 && e.falta.as_deref() == Some(";")), "{:?}", errores(&a, 10));
        let a = analizar(Lenguaje::Python, "def f(:\n    return 1\n").unwrap();
        assert_eq!(errores(&a, 10).first().map(|e| e.linea), Some(1));
        let a = analizar(Lenguaje::Json, "{\"a\": 1,, \"b\": 2}").unwrap();
        assert!(!errores(&a, 10).is_empty());
        let a = analizar(Lenguaje::Go, "package x\nfunc f() int { return 1 }\n").unwrap();
        assert!(errores(&a, 10).is_empty());
    }

    #[test]
    fn simbolos_por_lenguaje() {
        assert_eq!(
            nombres(Lenguaje::Rust, "mod m;\nstruct S { a: i32 }\nimpl Clone for S {\n    fn clone(&self) -> S { todo!() }\n}\ntrait T {}\nfn suelta() {}\n"),
            ["1 mod m", "2 struct S", "3 impl Clone for S", "  4 method clone", "6 trait T", "7 fn suelta"]
        );
        assert_eq!(nombres(Lenguaje::Python, "class A:\n    def m(self):\n        pass\n\ndef f():\n    pass\n"), ["1 class A", "  2 method m", "5 fn f"]);
        assert_eq!(
            nombres(Lenguaje::TypeScript, "export interface P { x: number }\ntype Id = string;\nclass C {\n  hola(): void {}\n}\nconst flecha = () => 1;\nconst n = 3;\nexport function f() {}\n"),
            ["1 interface P", "2 type Id", "3 class C", "  4 method hola", "6 fn flecha", "8 fn f"]
        );
        assert_eq!(nombres(Lenguaje::Tsx, "export function App() {\n  return <div/>;\n}\n"), ["1 fn App"]);
        assert_eq!(nombres(Lenguaje::JavaScript, "function a() {}\nconst b = function () {};\n"), ["1 fn a", "2 fn b"]);
        assert_eq!(
            nombres(Lenguaje::Go, "package x\ntype S struct{}\ntype I interface{}\nfunc (s S) M() {}\nfunc F() {}\n"),
            ["2 struct S", "3 interface I", "4 method M", "5 fn F"]
        );
        assert_eq!(nombres(Lenguaje::Php, "<?php\nclass C {\n  public function m() {}\n}\nfunction f() {}\n"), ["2 class C", "  3 method m", "5 fn f"]);
        assert_eq!(
            nombres(Lenguaje::Swift, "struct S {\n    func m() {}\n}\nprotocol P {}\nfunc f() {}\nextension S {}\n"),
            ["1 struct S", "  2 method m", "4 protocol P", "5 fn f", "6 extension S"]
        );
        assert_eq!(
            nombres(Lenguaje::C, "struct punto { int x; };\ntypedef int entero;\nstatic int *sumar(int a) { return 0; }\n"),
            ["1 struct punto", "2 type entero", "3 fn sumar"]
        );
        assert_eq!(nombres(Lenguaje::Css, ".a, .b { color: red }\n@keyframes giro { from {} }\n"), ["1 selector .a, .b", "2 selector @keyframes giro"]);
        assert_eq!(nombres(Lenguaje::Json, "{\"name\": \"x\", \"scripts\": {\"build\": \"y\"}}"), ["1 key name", "1 key scripts"]);
        assert!(nombres(Lenguaje::Html, "<div id=a></div>").is_empty());
    }
}
