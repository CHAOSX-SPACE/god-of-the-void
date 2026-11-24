//! D8f · Esqueletos de BACKEND: la API REST de una entidad en Hono (TypeScript), FastAPI (Python)
//! o Axum (Rust), con validación en el borde, configuración por entorno y pruebas ya en verde.
//!
//! Es el gemelo de servidor de [`crate::app`]: la MISMA `entidad` (nombre + campos tipados) da la
//! app y su API, con los mismos nombres de campo. Los tres marcos cumplen un contrato único:
//! `GET /salud`, `GET|POST /<plural>`, `GET|PUT|DELETE /<plural>/{id}`; 201 al crear, 204 al
//! borrar, 404 `{"error"}` si no existe y 422 `{"errores": {campo: mensaje}}` si no valida.
//!
//! Ley de la ola: el modelo describe los datos; el programa escribe lo que un modelo pequeño
//! falla a mano (la forma exacta de cada marco en SU versión, fijada y probada).
//!
//! Las plantillas viven en `recursos/api/<marco>/` con los dos idiomas en línea (`⟦es¦en⟧`).
use crate::app::{ident, mayus, tipo_norm, Entidad};
use gotv_engine::tr;
use serde_json::{json, Value};

/// Los marcos que hay: (id, lenguaje).
pub const MARCOS: &[(&str, &str)] = &[("hono", "TypeScript"), ("fastapi", "Python"), ("axum", "Rust")];

/// Versión de los tipos de Node que entiende el TypeScript fijado (5.6.3).
const TIPOS_NODE: &str = "22.10.2";

/// Palabras que no pueden ser el nombre de un campo en alguno de los tres lenguajes.
const RESERVADAS: &[&str] = &[
    // juez-idioma: bloque — datos (palabras reservadas de TypeScript, Python y Rust; «del» es de Python)
    "as", "async", "await", "break", "case", "catch", "class", "const", "continue", "crate", "def", "default", "del", "delete", "do", "dyn", "elif", "else", "enum", "except", "export", "extends",
    "extern", "false", "finally", "fn", "for", "from", "function", "global", "if", "impl", "import", "in", "is", "lambda", "let", "loop", "match", "mod", "move", "mut", "new", "none", "nonlocal",
    "not", "null", "or", "and", "pass", "pub", "raise", "ref", "return", "self", "static", "struct", "super", "switch", "this", "throw", "trait", "true", "try", "type", "typeof", "unsafe", "use",
    "var", "void", "where", "while", "with", "yield", "e", "o", "x",
];

struct C {
    id: String,
    etiqueta: String,
    tipo: &'static str,
    opciones: Vec<String>,
}

/// Elige el idioma de cada `⟦español¦inglés⟧` de una plantilla.
fn bilingue(s: &str) -> String {
    let en = gotv_engine::i18n::is_en();
    let mut out = String::with_capacity(s.len());
    let mut resto = s;
    while let Some(i) = resto.find('⟦') {
        out.push_str(&resto[..i]);
        let Some(f) = resto[i..].find('⟧') else { break };
        let dentro = &resto[i + '⟦'.len_utf8()..i + f];
        let (es, ing) = dentro.split_once('¦').unwrap_or((dentro, dentro));
        out.push_str(if en { ing } else { es });
        resto = &resto[i + f + '⟧'.len_utf8()..];
    }
    out.push_str(resto);
    out
}

fn campos(e: &Entidad) -> anyhow::Result<Vec<C>> {
    if e.campos.is_empty() {
        gotv_engine::bail_tr!("entidad.campos: hace falta al menos un campo", "entidad.campos: at least one field is needed");
    }
    let mut vistos = std::collections::HashSet::new();
    let mut cs = Vec::new();
    for c in &e.campos {
        let mut id = ident(&c.nombre);
        while RESERVADAS.contains(&id.as_str()) || !vistos.insert(id.clone()) {
            id.push('_');
        }
        let opciones: Vec<String> = c.opciones.iter().map(|o| o.trim().to_string()).filter(|o| !o.is_empty()).collect();
        let tipo = match (tipo_norm(&c.tipo), opciones.is_empty()) {
            (_, false) => "opciones",
            ("opciones", true) => "texto", // sin lista no hay opciones
            (t, true) => t,
        };
        cs.push(C { id, etiqueta: mayus(&c.nombre), tipo, opciones });
    }
    Ok(cs)
}

/// El mensaje de error de un campo, en el idioma de GOTV.
fn mensaje(c: &C) -> String {
    let e = &c.etiqueta;
    match c.tipo {
        "numero" | "dinero" => tr!("{e} debe ser un número igual o mayor que 0.", "{e} must be a number equal to or greater than 0."),
        "correo" => tr!("{e}: escribe un correo válido, como ana@empresa.com.", "{e}: type a valid email, like ana@company.com."),
        "fecha" => tr!("{e} debe ser una fecha AAAA-MM-DD.", "{e} must be a date YYYY-MM-DD."),
        "opciones" => tr!("{e} debe ser una de: {}.", "{e} must be one of: {}.", c.opciones.join(", ")),
        "si_no" => tr!("{e} debe ser verdadero o falso.", "{e} must be true or false."),
        _ => tr!("Escribe {}.", "Enter {}.", e.to_lowercase()),
    }
}

fn valido(c: &C) -> Value {
    match c.tipo {
        "numero" => json!(4),
        "dinero" => json!(15000),
        "correo" => json!("eva@empresa.co"),
        "fecha" => json!("2026-11-14"),
        "si_no" => json!(true),
        "opciones" => json!(c.opciones[0]),
        _ => json!(format!("{} 1", c.etiqueta)),
    }
}

/// Las tres cargas de las pruebas: válida, rota en UN campo (y cuál) y cambiada (y en qué).
struct Cargas {
    valido: Value,
    roto: Value,
    campo_roto: usize,
    cambiado: Value,
    campo_cambiado: Option<usize>,
}

fn cargas(cs: &[C]) -> Cargas {
    let base: serde_json::Map<String, Value> = cs.iter().map(|c| (c.id.clone(), valido(c))).collect();
    let campo_roto = cs.iter().position(|c| c.tipo != "si_no").unwrap_or(0);
    let mut roto = base.clone();
    roto.insert(
        cs[campo_roto].id.clone(),
        match cs[campo_roto].tipo {
            "numero" | "dinero" => json!(-1),
            "correo" => json!("no-es-correo"), // juez-idioma: datos (un correo mal escrito, para la prueba)
            "fecha" => json!("31/12/2026"),
            "opciones" => json!("zzz"),
            "si_no" => json!("x"),
            _ => json!("  "),
        },
    );
    let campo_cambiado = cs.iter().position(|c| c.tipo == "texto").or_else(|| cs.iter().position(|c| matches!(c.tipo, "numero" | "dinero" | "si_no")));
    let mut cambiado = base.clone();
    if let Some(i) = campo_cambiado {
        cambiado.insert(
            cs[i].id.clone(),
            match cs[i].tipo {
                "texto" => json!(gotv_engine::i18n::t("Cambiado", "Changed")),
                "si_no" => json!(false),
                _ => json!(9),
            },
        );
    }
    Cargas { valido: Value::Object(base), roto: Value::Object(roto), campo_roto, cambiado: Value::Object(cambiado), campo_cambiado }
}

fn js(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| "\"\"".into())
}

fn opc(c: &C) -> String {
    format!("OPCIONES_{}", c.id.to_uppercase())
}

fn hay(cs: &[C], tipos: &[&str]) -> bool {
    cs.iter().any(|c| tipos.contains(&c.tipo))
}

// ── Hono (TypeScript) ───────────────────────────────────────────────────────
fn hono(cs: &[C], k: &Cargas) -> Vec<(&'static str, String)> {
    let opciones: String = cs.iter().filter(|c| c.tipo == "opciones").map(|c| format!("export const {} = [{}] as const;\n", opc(c), c.opciones.iter().map(|o| js(o)).collect::<Vec<_>>().join(", "))).collect();
    let tipos = cs
        .iter()
        .map(|c| {
            let t = match c.tipo {
                "numero" | "dinero" => "number".to_string(),
                "si_no" => "boolean".to_string(),
                "opciones" => format!("(typeof {})[number]", opc(c)),
                _ => "string".to_string(),
            };
            format!("{}: {t}", c.id)
        })
        .collect::<Vec<_>>()
        .join("; ");
    let mut patrones = String::new();
    if hay(cs, &["correo"]) {
        patrones.push_str("const CORREO = /^[^\\s@]+@[^\\s@]+\\.[^\\s@]+$/;\n");
    }
    if hay(cs, &["fecha"]) {
        patrones.push_str("const FECHA = /^\\d{4}-\\d{2}-\\d{2}$/;\n");
    }
    if !patrones.is_empty() {
        patrones.insert(0, '\n');
    }
    let mut ayudas = String::new();
    if hay(cs, &["texto"]) {
        ayudas.push_str("  const _texto = (k: string, msg: string) => {\n    const v = o[k];\n    if (typeof v !== \"string\" || !v.trim()) e[k] = msg;\n    return typeof v === \"string\" ? v.trim() : \"\";\n  };\n");
    }
    if hay(cs, &["correo", "fecha"]) {
        ayudas.push_str("  const _patron = (k: string, forma: RegExp, msg: string) => {\n    const v = typeof o[k] === \"string\" ? (o[k] as string).trim() : \"\";\n    if (!forma.test(v)) e[k] = msg;\n    return v;\n  };\n");
    }
    if hay(cs, &["numero", "dinero"]) {
        ayudas.push_str("  const _numero = (k: string, msg: string) => {\n    const v = o[k];\n    if (typeof v !== \"number\" || !Number.isFinite(v) || v < 0) e[k] = msg;\n    return typeof v === \"number\" ? v : 0;\n  };\n");
    }
    if hay(cs, &["opciones"]) {
        ayudas.push_str("  const _opcion = <T extends string>(k: string, lista: readonly T[], msg: string): T => {\n    const v = o[k];\n    if (typeof v === \"string\" && lista.includes(v as T)) return v as T;\n    e[k] = msg;\n    return lista[0];\n  };\n");
    }
    if hay(cs, &["si_no"]) {
        ayudas.push_str("  const _booleano = (k: string, msg: string) => {\n    const v = o[k];\n    if (typeof v !== \"boolean\") e[k] = msg;\n    return v === true;\n  };\n");
    }
    let lineas: String = cs
        .iter()
        .map(|c| {
            let (id, m) = (&c.id, js(&mensaje(c)));
            match c.tipo {
                "numero" | "dinero" => format!("  const {id} = _numero({}, {m});\n", js(id)),
                "correo" => format!("  const {id} = _patron({}, CORREO, {m});\n", js(id)),
                "fecha" => format!("  const {id} = _patron({}, FECHA, {m});\n", js(id)),
                "opciones" => format!("  const {id} = _opcion({}, {}, {m});\n", js(id), opc(c)),
                "si_no" => format!("  const {id} = _booleano({}, {m});\n", js(id)),
                _ => format!("  const {id} = _texto({}, {m});\n", js(id)),
            }
        })
        .collect();
    vec![
        ("__OPCIONES__", if opciones.is_empty() { opciones } else { format!("{opciones}\n") }),
        ("__TIPOS__", tipos),
        ("__PATRONES__", patrones),
        ("__AYUDAS__", ayudas),
        ("__LINEAS__", lineas),
        ("__CLAVES__", cs.iter().map(|c| c.id.clone()).collect::<Vec<_>>().join(", ")),
        ("__CAMPO_ROTO__", cs[k.campo_roto].id.clone()),
        ("__TIPOS_NODE__", TIPOS_NODE.to_string()),
    ]
}

// ── FastAPI (Python) ────────────────────────────────────────────────────────
fn fastapi(cs: &[C], k: &Cargas) -> Vec<(&'static str, String)> {
    let anotado = hay(cs, &["texto", "correo", "numero", "dinero"]);
    let literal = hay(cs, &["opciones"]);
    let mut imports = String::new();
    if hay(cs, &["fecha"]) {
        imports.push_str("from datetime import date\n");
    }
    match (anotado, literal) {
        (true, true) => imports.push_str("from typing import Annotated, Literal\n"),
        (true, false) => imports.push_str("from typing import Annotated\n"),
        (false, true) => imports.push_str("from typing import Literal\n"),
        _ => {}
    }
    if !imports.is_empty() {
        imports.push('\n');
    }
    let mut pydantic = String::new();
    if hay(cs, &["numero", "dinero"]) {
        pydantic.push_str(", Field");
    }
    if hay(cs, &["texto", "correo"]) {
        pydantic.push_str(", StringConstraints");
    }
    let mut alias = String::new();
    if hay(cs, &["texto"]) {
        alias.push_str("Texto = Annotated[str, StringConstraints(strip_whitespace=True, min_length=1)]\n");
    }
    if hay(cs, &["correo"]) {
        alias.push_str("Correo = Annotated[str, StringConstraints(strip_whitespace=True, pattern=r\"^[^\\s@]+@[^\\s@]+\\.[^\\s@]+$\")]\n");
    }
    if hay(cs, &["numero", "dinero"]) {
        alias.push_str("Cantidad = Annotated[float, Field(ge=0)]\n");
    }
    if !alias.is_empty() {
        alias.insert(0, '\n');
    }
    let lineas: String = cs
        .iter()
        .map(|c| {
            let t = match c.tipo {
                "numero" | "dinero" => "Cantidad".to_string(),
                "correo" => "Correo".to_string(),
                "fecha" => "date".to_string(),
                "si_no" => "bool".to_string(),
                "opciones" => format!("Literal[{}]", c.opciones.iter().map(|o| js(o)).collect::<Vec<_>>().join(", ")),
                _ => "Texto".to_string(),
            };
            format!("    {}: {t}\n", c.id)
        })
        .collect();
    let mensajes: String = cs.iter().map(|c| format!("    {}: {},\n", js(&c.id), js(&mensaje(c)))).collect();
    vec![("__IMPORTS__", imports), ("__PYDANTIC__", pydantic), ("__ALIAS__", alias), ("__CAMPOS__", lineas), ("__MENSAJES__", mensajes), ("__CAMPO_ROTO__", cs[k.campo_roto].id.clone())]
}

// ── Axum (Rust) ─────────────────────────────────────────────────────────────
fn axum(cs: &[C], k: &Cargas) -> Vec<(&'static str, String)> {
    let opciones: String = cs.iter().filter(|c| c.tipo == "opciones").map(|c| format!("\npub const {}: &[&str] = &[{}];\n", opc(c), c.opciones.iter().map(|o| format!("{o:?}")).collect::<Vec<_>>().join(", "))).collect();
    let lineas_campos: String = cs
        .iter()
        .map(|c| {
            let t = match c.tipo {
                "numero" | "dinero" => "f64",
                "si_no" => "bool",
                _ => "String",
            };
            format!("    pub {}: {t},\n", c.id)
        })
        .collect();
    let mut ayudas = String::new();
    if hay(cs, &["correo"]) {
        ayudas.push_str("\nfn es_correo(t: &str) -> bool {\n    let t = t.trim();\n    match t.split_once('@') {\n        Some((antes, dominio)) => !antes.is_empty() && !t.contains(char::is_whitespace) && !dominio.contains('@') && dominio.split('.').count() >= 2 && dominio.split('.').all(|p| !p.is_empty()),\n        None => false,\n    }\n}\n");
    }
    if hay(cs, &["fecha"]) {
        ayudas.push_str("\nfn es_fecha(t: &str) -> bool {\n    let b = t.as_bytes();\n    b.len() == 10 && b.iter().enumerate().all(|(i, c)| if i == 4 || i == 7 { *c == b'-' } else { c.is_ascii_digit() })\n}\n");
    }
    let lineas: String = cs
        .iter()
        .filter_map(|c| {
            let id = &c.id;
            let falla = match c.tipo {
                "numero" | "dinero" => format!("!self.{id}.is_finite() || self.{id} < 0.0"),
                "correo" => format!("!es_correo(&self.{id})"),
                "fecha" => format!("!es_fecha(&self.{id})"),
                "opciones" => format!("!{}.contains(&self.{id}.as_str())", opc(c)),
                "si_no" => return None, // el tipo ya lo garantiza
                _ => format!("self.{id}.trim().is_empty()"),
            };
            Some(format!("        if {falla} {{\n            e.insert({id:?}, {:?}.to_string());\n        }}\n", mensaje(c)))
        })
        .collect();
    let aserto = match k.campo_cambiado {
        Some(i) => {
            let c = &cs[i];
            let esperado = match c.tipo {
                "texto" => format!("json!({:?})", k.cambiado[&c.id].as_str().unwrap_or_default()),
                "si_no" => "json!(false)".to_string(),
                _ => "json!(9.0)".to_string(),
            };
            format!("    assert_eq!(cambiado[{:?}], {esperado});\n", c.id)
        }
        None => String::new(),
    };
    // un sí/no roto no llega a `validar`: lo rechaza el tipo, sin campo al que atribuirlo
    let campo_roto = if cs[k.campo_roto].tipo == "si_no" { "_".to_string() } else { cs[k.campo_roto].id.clone() };
    vec![("__OPCIONES__", opciones), ("__CAMPOS__", lineas_campos), ("__AYUDAS__", ayudas), ("__LINEAS__", lineas), ("__ASERTO_CAMBIO__", aserto), ("__CAMPO_ROTO__", campo_roto)]
}

macro_rules! plantillas {
    ($marco:literal: $($destino:literal <- $origen:literal),+ $(,)?) => {
        &[$(($destino, include_str!(concat!("../recursos/api/", $marco, "/", $origen)))),+]
    };
}

const HONO: &[(&str, &str)] = plantillas!("hono": "package.json" <- "package.json", "tsconfig.json" <- "tsconfig.json", "src/config.ts" <- "src/config.ts", "src/datos.ts" <- "src/datos.ts",
    "src/app.ts" <- "src/app.ts", "src/servidor.ts" <- "src/servidor.ts", "src/app.test.ts" <- "src/app.test.ts", "README.md" <- "README.md");
const FASTAPI: &[(&str, &str)] = plantillas!("fastapi": "pyproject.toml" <- "pyproject.toml", "app/__init__.py" <- "app/__init__.py", "app/__main__.py" <- "app/__main__.py", "app/config.py" <- "app/config.py",
    "app/modelos.py" <- "app/modelos.py", "app/repositorio.py" <- "app/repositorio.py", "app/main.py" <- "app/main.py", "tests/test_api.py" <- "tests/test_api.py", "README.md" <- "README.md");
const AXUM: &[(&str, &str)] = plantillas!("axum": "Cargo.toml" <- "Cargo.toml.txt", "src/lib.rs" <- "src/lib.rs.txt", "src/main.rs" <- "src/main.rs.txt", "tests/api.rs" <- "tests/api.rs.txt", "README.md" <- "README.md");

/// Los archivos (ruta relativa, contenido) de la API de `entidad` en `marco`
/// (`hono` · `fastapi` · `axum`). `nombre` es la carpeta (de ahí sale el nombre del paquete).
pub fn nueva(nombre: &str, marca: &str, marco: &str, entidad: &Entidad) -> anyhow::Result<Vec<(String, Vec<u8>)>> {
    let cs = campos(entidad)?;
    let k = cargas(&cs);
    let (plantillas, propios) = match marco.trim().to_lowercase().as_str() {
        "hono" | "node" | "typescript" | "ts" => (HONO, hono(&cs, &k)),
        "fastapi" | "python" | "py" => (FASTAPI, fastapi(&cs, &k)),
        "axum" | "rust" | "rs" => (AXUM, axum(&cs, &k)),
        otro => gotv_engine::bail_tr!("marco desconocido «{otro}»: usa hono (TypeScript), fastapi (Python) o axum (Rust)", "unknown framework «{otro}»: use hono (TypeScript), fastapi (Python) or axum (Rust)"),
    };
    let paquete = ident(nombre).replace('_', "-");
    let paquete = if paquete.is_empty() || paquete == "c-" { "api".to_string() } else { paquete };
    let plural = mayus(entidad.plural.as_deref().unwrap_or(&format!("{}s", entidad.nombre.trim())));
    let comunes = [
        ("__NOMBRE__", paquete.clone()),
        ("__CRATE__", paquete.replace('-', "_")),
        ("__MARCA__", marca.to_string()),
        ("__PLURAL__", plural.clone()),
        ("__RUTA__", ident(&plural)),
        ("__VALIDO__", k.valido.to_string()),
        ("__ROTO__", k.roto.to_string()),
        ("__CAMBIADO__", k.cambiado.to_string()),
    ];
    let rellenar = |t: &str| -> String {
        let mut s = bilingue(t);
        // primero lo propio del marco (puede traer dentro huecos comunes, nunca al revés)
        for (hueco, valor) in propios.iter().map(|(h, v)| (*h, v.as_str())).chain(comunes.iter().map(|(h, v)| (*h, v.as_str()))) {
            s = s.replace(hueco, valor);
        }
        s
    };
    let mut out: Vec<(String, Vec<u8>)> = plantillas.iter().map(|(destino, t)| (destino.to_string(), rellenar(t).into_bytes())).collect();
    // sin nada que validar, `e` no se muta: el compilador de Rust lo avisaría
    if marco.trim().to_lowercase().starts_with(['a', 'r']) && !cs.iter().any(|c| c.tipo != "si_no") {
        for (ruta, datos) in out.iter_mut().filter(|(r, _)| r == "src/lib.rs") {
            let _ = ruta;
            *datos = String::from_utf8_lossy(datos).replace("let mut e = BTreeMap::new();", "let e = BTreeMap::new();").into_bytes();
        }
    }
    out.push((".gitignore".into(), include_str!("../recursos/api/comun.gitignore").as_bytes().to_vec()));
    out.push((".env.example".into(), bilingue(include_str!("../recursos/api/comun.env")).into_bytes()));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pedido() -> Entidad {
        serde_json::from_str(
            r#"{"nombre":"pedido","campos":[
            {"nombre":"Cliente","tipo":"texto"},{"nombre":"Correo","tipo":"correo"},{"nombre":"Cantidad","tipo":"número"},
            {"nombre":"Precio","tipo":"dinero"},{"nombre":"Entrega","tipo":"fecha"},
            {"nombre":"Estado","opciones":["pendiente","preparando","entregado"]},{"nombre":"Para llevar","tipo":"bool"},{"nombre":"Type","tipo":"texto"}]}"#,
        )
        .unwrap()
    }

    fn texto<'a>(a: &'a [(String, Vec<u8>)], ruta: &str) -> &'a str {
        std::str::from_utf8(&a.iter().find(|(r, _)| r == ruta).unwrap_or_else(|| panic!("falta {ruta}")).1).unwrap()
    }

    #[test]
    fn los_tres_marcos_salen_enteros_y_sin_huecos() {
        for (marco, archivos) in [("hono", ["package.json", "src/datos.ts", "src/app.ts", "src/app.test.ts"]), ("fastapi", ["pyproject.toml", "app/modelos.py", "app/main.py", "tests/test_api.py"]), ("axum", ["Cargo.toml", "src/lib.rs", "src/main.rs", "tests/api.rs"])] {
            let a = nueva("Mi Tienda!", "Grano \"Alto\"", marco, &pedido()).unwrap();
            for r in archivos.iter().chain(&["README.md", ".gitignore", ".env.example"]) {
                let t = texto(&a, r);
                assert!(t.lines().all(|l| !l.contains("__") || l.contains("__init__") || l.contains("__main__") || l.contains("__name__") || l.contains("__pycache__")), "{marco}/{r} con huecos:\n{t}");
                assert!(!t.contains('⟦') && !t.contains('¦') && !t.contains('⟧'), "{marco}/{r} con marcas de idioma");
            }
            // la ruta sale del plural y el campo «Type» no choca con ninguna palabra reservada
            assert!(texto(&a, "README.md").contains("`/pedidos`"), "{marco}");
            assert!(a.iter().any(|(_, d)| String::from_utf8_lossy(d).contains("type_")), "{marco}");
        }
        assert!(texto(&nueva("x", "x", "hono", &pedido()).unwrap(), "package.json").contains("\"name\": \"x\""));
        assert!(nueva("x", "x", "django", &pedido()).is_err());
        assert!(nueva("x", "x", "hono", &Entidad { nombre: "x".into(), plural: None, campos: vec![] }).is_err());
    }

    #[test]
    fn cada_marco_valida_cada_tipo_de_campo() {
        let e = pedido();
        let ts = nueva("x", "x", "hono", &e).unwrap();
        let d = texto(&ts, "src/datos.ts");
        assert!(d.contains("export const OPCIONES_ESTADO = [\"pendiente\", \"preparando\", \"entregado\"] as const;"), "{d}");
        assert!(d.contains("export type Entrada = { cliente: string; correo: string; cantidad: number; precio: number; entrega: string; estado: (typeof OPCIONES_ESTADO)[number]; para_llevar: boolean; type_: string };"), "{d}");
        assert!(d.contains("const correo = _patron(\"correo\", CORREO,") && d.contains("const estado = _opcion(\"estado\", OPCIONES_ESTADO,") && d.contains("valor: { cliente, correo, cantidad, precio, entrega, estado, para_llevar, type_ }"), "{d}");
        let py = nueva("x", "x", "fastapi", &e).unwrap();
        let m = texto(&py, "app/modelos.py");
        assert!(m.contains("from datetime import date\nfrom typing import Annotated, Literal\n\nfrom pydantic import BaseModel, ConfigDict, Field, StringConstraints\n"), "{m}");
        assert!(m.contains("    estado: Literal[\"pendiente\", \"preparando\", \"entregado\"]\n    para_llevar: bool\n") && m.contains("    \"cantidad\": \"Cantidad debe ser"), "{m}");
        let rs = nueva("mi-api", "x", "axum", &e).unwrap();
        let l = texto(&rs, "src/lib.rs");
        assert!(l.contains("pub const OPCIONES_ESTADO: &[&str] = &[\"pendiente\", \"preparando\", \"entregado\"];") && l.contains("    pub cantidad: f64,\n") && l.contains("if !OPCIONES_ESTADO.contains(&self.estado.as_str()) {"), "{l}");
        assert!(texto(&rs, "src/main.rs").starts_with("use mi_api::{app, Config};") && texto(&rs, "tests/api.rs").contains("assert_eq!(cambiado[\"cliente\"], json!(\"Cambiado\"));"));
        // sólo sí/no: nada que validar a mano, y la carga rota la rechaza el tipo
        let solo: Entidad = serde_json::from_str(r#"{"name":"tarea","fields":[{"name":"Hecha","type":"boolean"}]}"#).unwrap();
        let rs = nueva("x", "x", "axum", &solo).unwrap();
        assert!(texto(&rs, "src/lib.rs").contains("let e = BTreeMap::new();") && texto(&rs, "tests/api.rs").contains("Some(vec![\"_\".to_string()])"));
        assert!(!texto(&nueva("x", "x", "fastapi", &solo).unwrap(), "app/modelos.py").contains("from typing"));
    }

    #[test]
    fn en_ingles_sale_en_ingles() {
        use gotv_engine::i18n::{set_thread, Lang};
        set_thread(Some(Lang::En));
        let a = nueva("x", "x", "hono", &pedido()).unwrap();
        set_thread(None);
        let todo: String = a.iter().map(|(_, d)| String::from_utf8_lossy(d).into_owned()).collect();
        assert!(todo.contains("Not found.") && todo.contains("must be a number") && todo.contains("creates, lists, reads"), "{todo}");
        // juez-idioma: bloque — datos: lo que NO debe quedar en la API en inglés
        for es in ["No existe", "debe ser", "Escribe ", "pruebas", "Lógica del negocio"] {
            assert!(!todo.contains(es), "queda «{es}» en la API en inglés");
        }
    }
}
