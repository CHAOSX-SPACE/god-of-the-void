//! Aplicaciones (D5): esqueleto Vite + React + TypeScript con la biblioteca de componentes
//! (`recursos/react/gotv.tsx`), el tema calculado, las fuentes y un CRUD de ejemplo con sus
//! pruebas. El modelo arma pantallas COMPONIENDO estas piezas y rellena funciones cortas;
//! no escribe CSS ni reinventa la tabla. Versiones fijas: el proyecto se reproduce igual.

use crate::pagina::{direccion, DIRECCIONES};
use crate::recursos;
use crate::tema::{Pedido, Tema};

static GOTV_TSX: &str = include_str!("../recursos/react/gotv.tsx");

/// Los textos VISIBLES de `gotv.tsx` y su versión en inglés (la API —componentes y props— no
/// cambia). Una prueba vigila que cada texto en español siga existiendo en el archivo.
const GOTV_TSX_EN: &[(&str, &str)] = &[
    // juez-idioma: bloque — par: (texto de gotv.tsx en español, el mismo en inglés)
    ("<span className=\"solo-lector\">Cargando…</span>", "<span className=\"solo-lector\">Loading…</span>"),
    ("<strong>Algo salió mal</strong>", "<strong>Something went wrong</strong>"),
    ("onClick={reintentar}>Reintentar</Boton>", "onClick={reintentar}>Retry</Boton>"),
    ("placeholder=\"Buscar…\"", "placeholder=\"Search…\""),
    ("\"Sin resultados\" : \"Aún no hay nada aquí\"", "\"No results\" : \"Nothing here yet\""),
    ("`Nada coincide con «${q}».`", "`Nothing matches «${q}».`"),
    ("aria-label={`Páginas de ${titulo}`}", "aria-label={`Pages of ${titulo}`}"),
    (")} de {visibles.length}</span>", ")} of {visibles.length}</span>"),
    (">Anterior</Boton>", ">Previous</Boton>"),
    (">Siguiente</Boton>", ">Next</Boton>"),
    ("aria-label=\"Cerrar\"", "aria-label=\"Close\""),
    (">Saltar al contenido</a>", ">Skip to content</a>"),
    ("aria-label=\"Navegación\"", "aria-label=\"Navigation\""),
    ("toLocaleString(\"es\",", "toLocaleString(\"en\","),
    ("localeCompare(String(y), \"es\",", "localeCompare(String(y), \"en\","),
    ("<EstadoVacio titulo=\"Sin datos\"", "<EstadoVacio titulo=\"No data\""),
    ("{ etiqueta: \"Otras\",", "{ etiqueta: \"Other\","),
    ("} más en la tabla</li>", "} more in the table</li>"),
    ("Flechas izquierda y derecha para recorrer los puntos.", "Left and right arrows to walk through the points."),
    ("\"Ocultar la tabla\" : \"Ver como tabla\"", "\"Hide the table\" : \"View as table\""),
];

/// La biblioteca en el idioma de GOTV.
fn gotv_tsx() -> String {
    if gotv_engine::i18n::is_en() {
        GOTV_TSX_EN.iter().fold(GOTV_TSX.to_string(), |s, (es, en)| s.replace(es, en))
    } else {
        GOTV_TSX.to_string()
    }
}

/// Dependencias con versión EXACTA (sin `^`): lo que se probó es lo que se instala.
const PAQUETE: &str = r#"{
  "name": "__NOMBRE__",
  "private": true,
  "version": "0.1.0",
  "type": "module",
  "scripts": {
    "dev": "vite",
    "build": "tsc -b && vite build",
    "preview": "vite preview",
    "test": "vitest run",
    "check": "tsc -b --noEmit"
  },
  "dependencies": {
    "react": "18.3.1",
    "react-dom": "18.3.1"
  },
  "devDependencies": {
    "@testing-library/dom": "10.4.0",
    "@testing-library/react": "16.1.0",
    "@types/react": "18.3.12",
    "@types/react-dom": "18.3.1",
    "@vitejs/plugin-react": "4.3.4",
    "jsdom": "25.0.1",
    "typescript": "5.6.3",
    "vite": "5.4.11",
    "vitest": "2.1.8"
  }
}
"#;

const TSCONFIG: &str = r#"{
  "compilerOptions": {
    "target": "ES2022",
    "lib": ["ES2022", "DOM", "DOM.Iterable"],
    "module": "ESNext",
    "moduleResolution": "bundler",
    "jsx": "react-jsx",
    "strict": true,
    "noUnusedLocals": true,
    "noUnusedParameters": true,
    "noFallthroughCasesInSwitch": true,
    "isolatedModules": true,
    "skipLibCheck": true,
    "noEmit": true,
    "types": ["vitest/globals"]
  },
  "include": ["src"]
}
"#;

const VITE: &str = r#"import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

export default defineConfig({
  plugins: [react()],
  test: { environment: "jsdom", globals: true },
});
"#;

const MAIN: &str = r#"import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import "./gotv/estilos.css";
import App from "./App";

createRoot(document.getElementById("raiz")!).render(<StrictMode><App /></StrictMode>);
"#;

/// La app de ejemplo (clientes) en los dos idiomas: `recursos/react/plantilla/{es,en}/`. La app
/// generada habla el idioma de GOTV; los identificadores del código (la API) no se traducen.
struct Plantilla {
    datos: &'static str,
    datos_test: &'static str,
    app_test: &'static str,
    app: &'static str,
}

const PLANTILLA_ES: Plantilla = Plantilla {
    datos: include_str!("../recursos/react/plantilla/es/datos.ts"),
    datos_test: include_str!("../recursos/react/plantilla/es/datos.test.ts"),
    app_test: include_str!("../recursos/react/plantilla/es/App.test.tsx"),
    app: include_str!("../recursos/react/plantilla/es/App.tsx"),
};

const PLANTILLA_EN: Plantilla = Plantilla {
    datos: include_str!("../recursos/react/plantilla/en/datos.ts"),
    datos_test: include_str!("../recursos/react/plantilla/en/datos.test.ts"),
    app_test: include_str!("../recursos/react/plantilla/en/App.test.tsx"),
    app: include_str!("../recursos/react/plantilla/en/App.tsx"),
};

fn plantilla() -> &'static Plantilla {
    if gotv_engine::i18n::is_en() {
        &PLANTILLA_EN
    } else {
        &PLANTILLA_ES
    }
}

// ── generación por ENTIDAD (D5, medido): el 9B tenía que desmontar el CRUD de ejemplo a mano,
// rompía referencias entre archivos y no terminaba en 40 pasos. Ahora describe sus datos y el
// programa genera lógica, validación, pruebas, tabla, formulario y resumen YA coherentes.

#[derive(Clone, Debug, serde::Deserialize)]
pub struct CampoE {
    #[serde(alias = "name")]
    pub nombre: String,
    /// texto · numero · dinero · correo · fecha · opciones · si_no
    #[serde(default, alias = "type")]
    pub tipo: String,
    #[serde(default, alias = "options")]
    pub opciones: Vec<String>,
}

#[derive(Clone, Debug, serde::Deserialize)]
pub struct Entidad {
    #[serde(alias = "name")]
    pub nombre: String,
    #[serde(default)]
    pub plural: Option<String>,
    #[serde(alias = "fields")]
    pub campos: Vec<CampoE>,
}

pub(crate) fn ident(s: &str) -> String {
    let mut o = String::new();
    for c in s.trim().to_lowercase().chars() {
        let c = match c {
            'á' | 'à' | 'ä' => 'a',
            'é' | 'è' | 'ë' => 'e',
            'í' | 'ì' | 'ï' => 'i',
            'ó' | 'ò' | 'ö' => 'o',
            'ú' | 'ù' | 'ü' => 'u',
            'ñ' => 'n',
            c => c,
        };
        if c.is_ascii_alphanumeric() {
            o.push(c);
        } else if !o.ends_with('_') && !o.is_empty() {
            o.push('_');
        }
    }
    let o = o.trim_end_matches('_').to_string();
    if o.is_empty() || o.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        format!("c_{o}")
    } else if o == "id" {
        "codigo".into()
    } else {
        o
    }
}

pub(crate) fn tipo_norm(t: &str) -> &'static str {
    match t.trim().to_lowercase().replace(['í', 'ú'], "").as_str() {
        "numero" | "number" | "nmero" | "entero" | "cantidad" | "int" | "integer" | "float" => "numero",
        "dinero" | "precio" | "money" | "moneda" | "valor" | "total" | "currency" => "dinero",
        "correo" | "email" | "e-mail" | "mail" => "correo",
        "fecha" | "date" | "dia" => "fecha",
        "opciones" | "options" | "estado" | "enum" | "select" | "lista" => "opciones",
        "si_no" | "sino" | "booleano" | "boolean" | "bool" | "si/no" => "si_no",
        _ => "texto",
    }
}

pub(crate) fn mayus(s: &str) -> String {
    let mut c = s.trim().chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

fn js(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| "\"\"".into())
}

/// (datos.ts, datos.test.ts, App.tsx, App.test.tsx) para una entidad.
pub fn generar(e: &Entidad, marca: &str) -> anyhow::Result<[String; 4]> {
    if e.campos.is_empty() {
        gotv_engine::bail_tr!("entidad.campos: hace falta al menos un campo", "entidad.campos: at least one field is needed");
    }
    // juez-idioma: datos — el valor «sí» de un campo sí/no, en el idioma de la app
    let si = gotv_engine::i18n::t("sí", "yes");
    struct C {
        id: String,
        etiqueta: String,
        tipo: &'static str,
        opciones: Vec<String>,
    }
    let mut vistos = std::collections::HashSet::new();
    let mut cs: Vec<C> = Vec::new();
    for c in &e.campos {
        let mut id = ident(&c.nombre);
        while !vistos.insert(id.clone()) {
            id.push('_');
        }
        let mut tipo = tipo_norm(&c.tipo);
        let opciones: Vec<String> = c.opciones.iter().map(|o| o.trim().to_string()).filter(|o| !o.is_empty()).collect();
        if !opciones.is_empty() {
            tipo = "opciones";
        }
        if tipo == "opciones" && opciones.is_empty() {
            tipo = "texto"; // sin lista no hay opciones
        }
        cs.push(C { id, etiqueta: mayus(&c.nombre), tipo, opciones });
    }
    let singular = mayus(&e.nombre);
    let plural = mayus(e.plural.as_deref().unwrap_or(&format!("{}s", e.nombre.trim())));
    let tipo_t = {
        let t: String = ident(&e.nombre).split('_').map(mayus).collect();
        let t = if t.is_empty() { "Registro".to_string() } else { t };
        // no puede llamarse como algo que el código ya importa o declara
        if ["App", "Lista", "Borrador", "Errores", "Columna", "Aplicacion", "Avisos", "Boton", "Campo", "Dialogo", "Estadistica", "InsigniaEstado", "Pestanas", "Selector", "Tabla", "Icono", "Panel", "Barras", "Lineas", "Chispa", "Casilla", "Interruptor"].contains(&t.as_str()) {
            format!("{t}Dato")
        } else {
            t
        }
    };
    let opc = |c: &C| format!("OPCIONES_{}", c.id.to_uppercase());
    let hay = |t: &str| cs.iter().any(|c| c.tipo == t);
    let estado = cs.iter().find(|c| c.tipo == "opciones").map(|c| c.id.clone());
    // con cantidad y precio, el total que importa es Σ cantidad × precio (sumar precios
    // unitarios no significa nada). Sólo si el campo numérico SE LLAMA como una cantidad.
    let importe: Option<(String, String, String, String)> = {
        let cant = cs.iter().find(|c| c.tipo == "numero" && ["cantidad", "unidades", "cant", "qty", "quantity", "units", "cuantos", "numero_de"].iter().any(|k| c.id.starts_with(k)));
        let precio = cs.iter().find(|c| c.tipo == "dinero");
        match (cant, precio) {
            (Some(a), Some(b)) => Some((a.id.clone(), b.id.clone(), a.etiqueta.to_lowercase(), b.etiqueta.to_lowercase())),
            _ => None,
        }
    };

    // ── datos.ts
    let mut d = gotv_engine::tr!("// Lógica del negocio SIN interfaz para «{plural}» (generada por GOTV): fácil de probar y de cambiar.\n", "// Business logic WITHOUT UI for «{plural}» (generated by GOTV): easy to test and to change.\n");
    for c in cs.iter().filter(|c| c.tipo == "opciones") {
        d += &format!("export const {} = [{}] as const;\n", opc(c), c.opciones.iter().map(|o| js(o)).collect::<Vec<_>>().join(", "));
    }
    let ts = |c: &C| match c.tipo {
        "numero" | "dinero" => "number".to_string(),
        "si_no" => "boolean".to_string(),
        "opciones" => format!("(typeof {})[number]", opc(c)),
        _ => "string".to_string(),
    };
    d += &format!("export type {tipo_t} = {{ id: number; {} }};\n", cs.iter().map(|c| format!("{}: {}", c.id, ts(c))).collect::<Vec<_>>().join("; "));
    d += &gotv_engine::tr!("/** Lo que se escribe en el formulario (todo texto). */\nexport type Borrador = {{ {} }};\n", "/** What is typed in the form (all text). */\nexport type Borrador = {{ {} }};\n", cs.iter().map(|c| format!("{}: string", c.id)).collect::<Vec<_>>().join("; "));
    d += "export type Errores = Partial<Record<keyof Borrador, string>>;\n";
    if hay("correo") {
        d += "const CORREO = /^[^\\s@]+@[^\\s@]+\\.[^\\s@]+$/;\n";
    }
    if hay("numero") || hay("dinero") {
        d += "const num = (t: string) => Number(t.trim().replace(\",\", \".\"));\n";
    }
    let vacio = |c: &C| match c.tipo {
        "opciones" => format!("{}[0]", opc(c)),
        "si_no" => "\"no\"".to_string(),
        _ => "\"\"".to_string(),
    };
    d += &format!("export const VACIO: Borrador = {{ {} }};\n\n", cs.iter().map(|c| format!("{}: {}", c.id, vacio(c))).collect::<Vec<_>>().join(", "));
    d += gotv_engine::i18n::t("/** Valida en el borde: devuelve los errores por campo (vacío = válido). */\nexport function validar(b: Borrador): Errores {\n  const e: Errores = {};\n", "/** Validates at the edge: returns the errors per field (empty = valid). */\nexport function validar(b: Borrador): Errores {\n  const e: Errores = {};\n");
    for c in &cs {
        let (f, l) = (&c.id, c.etiqueta.to_lowercase());
        d += &match c.tipo {
            "texto" => format!("  if (b.{f}.trim().length < 2) e.{f} = {};\n", js(&gotv_engine::tr!("Escribe {l} (al menos 2 letras).", "Type {l} (at least 2 letters)."))),
            "numero" | "dinero" => format!(
                "  if (b.{f}.trim() === \"\" || !Number.isFinite(num(b.{f}))) e.{f} = {};\n  else if (num(b.{f}) < 0) e.{f} = {};\n",
                js(&gotv_engine::tr!("Escribe un número en {l}.", "Type a number in {l}.")),
                js(&gotv_engine::tr!("{} no puede ser negativo.", "{} cannot be negative.", c.etiqueta))
            ),
            "correo" => format!("  if (!CORREO.test(b.{f}.trim())) e.{f} = {};\n", js(gotv_engine::i18n::t("Escribe un correo válido, como ana@empresa.com.", "Type a valid email, like ana@company.com."))),
            "fecha" => format!("  if (Number.isNaN(Date.parse(b.{f}))) e.{f} = {};\n", js(&gotv_engine::tr!("Elige {l}.", "Pick {l}."))),
            "opciones" => format!("  if (!({} as readonly string[]).includes(b.{f})) e.{f} = {};\n", opc(c), js(&gotv_engine::tr!("Elige {l}.", "Pick {l}."))),
            _ => String::new(),
        };
    }
    d += "  return e;\n}\n\n";
    let conv = |c: &C| match c.tipo {
        "numero" | "dinero" => format!("num(b.{})", c.id),
        "si_no" => format!("b.{} === \"{si}\"", c.id),
        "opciones" => format!("b.{} as {tipo_t}[\"{}\"]", c.id, c.id),
        _ => format!("b.{}.trim()", c.id),
    };
    d += &gotv_engine::tr!(
        "/** Crea con el siguiente id libre (no muta la lista). */\nexport function crear(b: Borrador, lista: {tipo_t}[]): {tipo_t}[] {{\n  const id = lista.reduce((m, x) => Math.max(m, x.id), 0) + 1;\n  return [...lista, {{ id, {} }}];\n}}\n\n",
        "/** Creates with the next free id (does not mutate the list). */\nexport function crear(b: Borrador, lista: {tipo_t}[]): {tipo_t}[] {{\n  const id = lista.reduce((m, x) => Math.max(m, x.id), 0) + 1;\n  return [...lista, {{ id, {} }}];\n}}\n\n",
        cs.iter().map(|c| format!("{}: {}", c.id, conv(c))).collect::<Vec<_>>().join(", ")
    );
    d += &format!("export function borrar(id: number, lista: {tipo_t}[]): {tipo_t}[] {{\n  return lista.filter((x) => x.id !== id);\n}}\n\n");
    let mut res = vec!["total: lista.length".to_string()];
    for c in cs.iter().filter(|c| matches!(c.tipo, "numero" | "dinero")) {
        res.push(format!("suma_{}: lista.reduce((s, x) => s + x.{}, 0)", c.id, c.id));
    }
    for c in cs.iter().filter(|c| c.tipo == "opciones") {
        res.push(format!("por_{}: Object.fromEntries({}.map((o) => [o, lista.filter((x) => x.{} === o).length])) as Record<string, number>", c.id, opc(c), c.id));
    }
    if let Some((a, b, _, _)) = &importe {
        res.push(format!("importe: lista.reduce((s, x) => s + x.{a} * x.{b}, 0)"));
    }
    // la gráfica del panel: lo que importa (importe o dinero) repartido por el primer campo de
    // opciones; sin medida, el recuento — y sólo si hay más opciones que las tres cifras de arriba.
    // (clave en `resumen`, clave de su total, título, ¿es dinero?)
    let grafica: Option<(String, String, String, bool)> = cs.iter().find(|c| c.tipo == "opciones").and_then(|o| {
        let por = o.etiqueta.to_lowercase();
        if importe.is_some() {
            Some((format!("importe_por_{}", o.id), "importe".to_string(), gotv_engine::tr!("Total por {por}", "Total by {por}"), true))
        } else if let Some(m) = cs.iter().find(|c| c.tipo == "dinero") {
            Some((format!("suma_{}_por_{}", m.id, o.id), format!("suma_{}", m.id), gotv_engine::tr!("{} por {por}", "{} by {por}", m.etiqueta), true))
        } else if o.opciones.len() > 3 {
            Some((format!("por_{}", o.id), "total".to_string(), gotv_engine::tr!("{plural} por {por}", "{plural} by {por}"), false))
        } else {
            None
        }
    });
    if let (Some((clave, _, _, true)), Some(o)) = (&grafica, cs.iter().find(|c| c.tipo == "opciones")) {
        let medida = match &importe {
            Some((a, b, _, _)) => format!("x.{a} * x.{b}"),
            None => format!("x.{}", clave.trim_start_matches("suma_").trim_end_matches(&format!("_por_{}", o.id))),
        };
        res.push(format!("{clave}: Object.fromEntries({}.map((o) => [o, lista.filter((x) => x.{} === o).reduce((s, x) => s + {medida}, 0)])) as Record<string, number>", opc(o), o.id));
    }
    d += &format!("export function resumen(lista: {tipo_t}[]) {{\n  return {{ {} }};\n}}\n\n", res.join(", "));
    let ejemplo = |c: &C, i: usize| match c.tipo {
        "numero" => format!("{}", i * 3),
        "dinero" => format!("{}", i * 12000),
        "correo" => js(&gotv_engine::tr!("persona{i}@ejemplo.com", "person{i}@example.com")),
        "fecha" => js(&format!("2026-11-{:02}", i + 2)),
        "si_no" => if i.is_multiple_of(2) { "true".into() } else { "false".into() },
        "opciones" => format!("{}[{}]", opc(c), (i - 1) % c.opciones.len()),
        _ => js(&format!("{} {i}", c.etiqueta)),
    };
    d += &format!(
        "export const SEMILLA: {tipo_t}[] = [\n{}];\n",
        (1..=3).map(|i| format!("  {{ id: {i}, {} }},\n", cs.iter().map(|c| format!("{}: {}", c.id, ejemplo(c, i))).collect::<Vec<_>>().join(", "))).collect::<String>()
    );

    // ── datos.test.ts
    let valido = |c: &C| match c.tipo {
        "numero" => "\"4\"".to_string(),
        "dinero" => "\"15000\"".to_string(),
        "correo" => js("eva@empresa.co"),
        "fecha" => js("2026-12-01"),
        "si_no" => js(si),
        "opciones" => format!("{}[0]", opc(c)),
        _ => js(&gotv_engine::tr!("{} nuevo", "new {}", c.etiqueta)),
    };
    let malos: Vec<&String> = cs.iter().filter(|c| matches!(c.tipo, "texto" | "numero" | "dinero" | "correo" | "fecha")).map(|c| &c.id).collect();
    let usa_opc: Vec<String> = cs.iter().filter(|c| c.tipo == "opciones").map(opc).collect();
    let mut t = String::from("import { describe, expect, it } from \"vitest\";\n");
    t += &format!(
        "import {{ borrar, crear, resumen, SEMILLA, {}validar, type Borrador{} }} from \"./datos\";\n\n",
        if malos.is_empty() { "" } else { "VACIO, " },
        usa_opc.iter().map(|o| format!(", {o}")).collect::<String>()
    );
    t += &format!("const VALIDO: Borrador = {{ {} }};\n\n", cs.iter().map(|c| format!("{}: {}", c.id, valido(c))).collect::<Vec<_>>().join(", "));
    t += gotv_engine::i18n::t("describe(\"validar\", () => {\n  it(\"acepta un borrador correcto\", () => {\n    expect(validar(VALIDO)).toEqual({});\n  });\n",
"describe(\"validar\", () => {\n  it(\"accepts a correct draft\", () => {\n    expect(validar(VALIDO)).toEqual({});\n  });\n"
    );
    if !malos.is_empty() {
        let mut m = malos.iter().map(|x| js(x)).collect::<Vec<_>>();
        m.sort();
        t += &gotv_engine::tr!("  it(\"nombra cada campo vacío\", () => {{\n    expect(Object.keys(validar(VACIO)).sort()).toEqual([{}]);\n  }});\n",
"  it(\"names every empty field\", () => {{\n    expect(Object.keys(validar(VACIO)).sort()).toEqual([{}]);\n  }});\n",
            m.join(", ")
        );
    }
    t += "});\n\n";
    t += gotv_engine::i18n::t("describe(\"crear y borrar\", () => {\n  it(\"crea con el siguiente id y no muta la lista\", () => {\n    const antes = [...SEMILLA];\n    const nueva = crear(VALIDO, SEMILLA);\n    expect(SEMILLA).toEqual(antes);\n    expect(nueva).toHaveLength(SEMILLA.length + 1);\n    expect(nueva.at(-1)?.id).toBe(SEMILLA.length + 1);\n  });\n  it(\"borra por id\", () => {\n    expect(borrar(1, SEMILLA).some((x) => x.id === 1)).toBe(false);\n  });\n});\n\n",
"describe(\"create and delete\", () => {\n  it(\"creates with the next id and does not mutate the list\", () => {\n    const antes = [...SEMILLA];\n    const nueva = crear(VALIDO, SEMILLA);\n    expect(SEMILLA).toEqual(antes);\n    expect(nueva).toHaveLength(SEMILLA.length + 1);\n    expect(nueva.at(-1)?.id).toBe(SEMILLA.length + 1);\n  });\n  it(\"deletes by id\", () => {\n    expect(borrar(1, SEMILLA).some((x) => x.id === 1)).toBe(false);\n  });\n});\n\n"
    );
    t += gotv_engine::i18n::t("it(\"resume\", () => {\n  expect(resumen(SEMILLA).total).toBe(SEMILLA.length);\n  expect(resumen([]).total).toBe(0);\n",
"it(\"summarizes\", () => {\n  expect(resumen(SEMILLA).total).toBe(SEMILLA.length);\n  expect(resumen([]).total).toBe(0);\n"
    );
    if let Some(c) = cs.iter().find(|c| c.tipo == "opciones") {
        t += &format!("  expect(Object.values(resumen(SEMILLA).por_{}).reduce((a, b) => a + b, 0)).toBe(SEMILLA.length);\n  expect({}.length).toBeGreaterThan(0);\n", c.id, opc(c));
    }
    if let Some((a, b, _, _)) = &importe {
        t += &format!("  const dos = crear({{ ...VALIDO, {a}: \"3\", {b}: \"1000\" }}, crear({{ ...VALIDO, {a}: \"2\", {b}: \"5000\" }}, []));\n  expect(resumen(dos).importe).toBe(2 * 5000 + 3 * 1000);\n");
    }
    if let Some((clave, total, _, _)) = &grafica {
        t += &format!("  expect(Object.values(resumen(SEMILLA).{clave}).reduce((a, b) => a + b, 0)).toBeCloseTo(resumen(SEMILLA).{total});\n");
    }
    t += "});\n";

    // ── App.tsx
    let mut comp = vec!["Aplicacion", "Avisos", "Boton", "Dialogo", "Estadistica", "Tabla", "useAviso"];
    if cs.iter().any(|c| matches!(c.tipo, "texto" | "numero" | "dinero" | "correo" | "fecha")) {
        comp.push("Campo");
    }
    if hay("opciones") || hay("si_no") {
        comp.push("Selector");
    }
    if hay("opciones") {
        comp.push("InsigniaEstado");
        comp.push("Pestanas");
    }
    if grafica.is_some() {
        comp.push("Barras");
        comp.push("Panel");
    }
    comp.sort();
    let mut a = String::from("import { useMemo, useState } from \"react\";\n");
    a += &format!("import {{ {}, type Columna }} from \"./gotv/gotv\";\n", comp.join(", "));
    a += &format!(
        "import {{ borrar, crear, resumen, SEMILLA, VACIO, validar{}, type Borrador, type Errores, type {tipo_t} }} from \"./datos\";\n\n",
        usa_opc.iter().map(|o| format!(", {o}")).collect::<String>()
    );
    if hay("dinero") {
        a += "const DINERO = new Intl.NumberFormat(\"es-CO\", { style: \"currency\", currency: \"COP\", maximumFractionDigits: 0 });\n";
    }
    if hay("opciones") {
        a += "const TONOS = [\"aviso\", \"info\", \"exito\", \"peligro\"] as const;\n";
    }
    a += "\nfunction Lista() {\n  const avisar = useAviso();\n";
    a += &format!("  const [lista, setLista] = useState<{tipo_t}[]>(SEMILLA);\n");
    if estado.is_some() {
        a += "  const [filtro, setFiltro] = useState(\"todos\");\n";
    }
    a += "  const [abierto, setAbierto] = useState(false);\n  const [borrador, setBorrador] = useState<Borrador>(VACIO);\n  const [errores, setErrores] = useState<Errores>({});\n  const r = useMemo(() => resumen(lista), [lista]);\n";
    match &estado {
        Some(e) => a += &format!("  const filas = filtro === \"todos\" ? lista : lista.filter((x) => x.{e} === filtro);\n"),
        None => a += "  const filas = lista;\n",
    }
    a += &format!("\n  const columnas: Columna<{tipo_t}>[] = [\n");
    for c in &cs {
        let render = match c.tipo {
            "dinero" => format!(", num: true, render: (x) => DINERO.format(x.{})", c.id),
            "numero" => ", num: true".to_string(),
            "si_no" => gotv_engine::tr!(", render: (x) => (x.{} ? \"Sí\" : \"No\")", ", render: (x) => (x.{} ? \"Yes\" : \"No\")", c.id),
            "opciones" => format!(", render: (x) => <InsigniaEstado tono={{TONOS[{}.indexOf(x.{}) % TONOS.length]}}>{{x.{}}}</InsigniaEstado>", opc(c), c.id, c.id),
            _ => String::new(),
        };
        a += &format!("    {{ clave: {}, titulo: {}{render} }},\n", js(&c.id), js(&c.etiqueta));
    }
    a += &gotv_engine::tr!(
        "    {{ clave: \"id\", titulo: \"Acciones\", ordenable: false, render: (x) => (\n      <Boton variante=\"peligro\" chico aria-label={{`Borrar ${{x.id}}`}} onClick={{() => {{ setLista(borrar(x.id, lista)); avisar({}, \"info\"); }}}}>Borrar</Boton>\n    ) }},\n  ];\n\n",
        "    {{ clave: \"id\", titulo: \"Actions\", ordenable: false, render: (x) => (\n      <Boton variante=\"peligro\" chico aria-label={{`Delete ${{x.id}}`}} onClick={{() => {{ setLista(borrar(x.id, lista)); avisar({}, \"info\"); }}}}>Delete</Boton>\n    ) }},\n  ];\n\n",
        js(gotv_engine::i18n::t("Registro borrado.", "Record deleted."))
    );
    a += &format!(
        "  const guardar = () => {{\n    const e = validar(borrador);\n    setErrores(e);\n    if (Object.keys(e).length) return;\n    setLista(crear(borrador, lista));\n    setAbierto(false);\n    setBorrador(VACIO);\n    avisar({});\n  }};\n\n",
        js(gotv_engine::i18n::t("Registro guardado.", "Record saved."))
    );
    a += &gotv_engine::tr!(
        "  return (\n    <>\n      <header className=\"app__cabeza\">\n        <div><h1>{{{}}}</h1><p>{{`${{lista.length}} en total · {}`}}</p></div>\n        <Boton icono=\"plus\" onClick={{() => {{ setErrores({{}}); setAbierto(true); }}}}>{{{}}}</Boton>\n      </header>\n",
        "  return (\n    <>\n      <header className=\"app__cabeza\">\n        <div><h1>{{{}}}</h1><p>{{`${{lista.length}} in total · {}`}}</p></div>\n        <Boton icono=\"plus\" onClick={{() => {{ setErrores({{}}); setAbierto(true); }}}}>{{{}}}</Boton>\n      </header>\n",
        js(&plural),
        marca.replace(['`', '\\', '$', '{', '}'], ""),
        js(&gotv_engine::tr!("Agregar {}", "Add {}", singular.to_lowercase()))
    );
    a += gotv_engine::i18n::t("      <section className=\"estadisticas\" aria-label=\"Resumen\">\n", "      <section className=\"estadisticas\" aria-label=\"Summary\">\n");
    a += &format!("        <Estadistica etiqueta={{{}}} valor={{r.total}} />\n", js(&plural));
    if let Some((_, _, ea, eb)) = &importe {
        a += &format!("        <Estadistica etiqueta={{{}}} valor={{DINERO.format(r.importe)}} />\n", js(&format!("Total ({ea} × {eb})")));
    }
    for c in cs.iter().filter(|c| c.tipo == "dinero" && importe.as_ref().is_none_or(|(_, b, _, _)| *b != c.id)) {
        a += &format!("        <Estadistica etiqueta={{{}}} valor={{DINERO.format(r.suma_{})}} />\n", js(&format!("{} total", c.etiqueta)), c.id);
    }
    if let Some(c) = cs.iter().find(|c| c.tipo == "opciones") {
        for (i, o) in c.opciones.iter().take(3).enumerate() {
            a += &format!("        <Estadistica etiqueta={{{}}} valor={{r.por_{}[{}[{i}]] ?? 0}} />\n", js(&mayus(o)), c.id, opc(c));
        }
    }
    a += "      </section>\n";
    if let (Some((clave, _, titulo, dinero)), Some(o)) = (&grafica, cs.iter().find(|c| c.tipo == "opciones")) {
        let t = js(titulo);
        a += &format!(
            "      <Panel titulo={{{t}}}>\n        <Barras titulo={{{t}}} datos={{{}.map((o) => ({{ etiqueta: o.charAt(0).toUpperCase() + o.slice(1), valor: r.{clave}[o] ?? 0 }}))}}{} parte />\n      </Panel>\n",
            opc(o),
            if *dinero { " formato={(n) => DINERO.format(n)}" } else { "" }
        );
    }
    if let Some(c) = cs.iter().find(|c| c.tipo == "opciones") {
        a += &gotv_engine::tr!(
            "      <Pestanas activa={{filtro}} alCambiar={{setFiltro}} pestanas={{[{{ id: \"todos\", titulo: \"Todos\" }}, ...{}.map((o) => ({{ id: o as string, titulo: o.charAt(0).toUpperCase() + o.slice(1) }}))]}} />\n",
            "      <Pestanas activa={{filtro}} alCambiar={{setFiltro}} pestanas={{[{{ id: \"todos\", titulo: \"All\" }}, ...{}.map((o) => ({{ id: o as string, titulo: o.charAt(0).toUpperCase() + o.slice(1) }}))]}} />\n",
            opc(c)
        );
    }
    let buscar: Vec<String> = cs.iter().filter(|c| matches!(c.tipo, "texto" | "correo")).map(|c| js(&c.id)).collect();
    a += &format!(
        "      <Tabla titulo={{{}}} columnas={{columnas}} filas={{filas}}{} />\n",
        js(&gotv_engine::tr!("Lista de {}", "List of {}", plural.to_lowercase())),
        if buscar.is_empty() { String::new() } else { format!(" buscarEn={{[{}]}}", buscar.join(", ")) }
    );
    a += &gotv_engine::tr!(
        "      <Dialogo abierto={{abierto}} titulo={{{}}} alCerrar={{() => setAbierto(false)}}\n        pie={{<><Boton variante=\"secundario\" onClick={{() => setAbierto(false)}}>Cancelar</Boton><Boton onClick={{guardar}}>Guardar</Boton></>}}>\n",
        "      <Dialogo abierto={{abierto}} titulo={{{}}} alCerrar={{() => setAbierto(false)}}\n        pie={{<><Boton variante=\"secundario\" onClick={{() => setAbierto(false)}}>Cancel</Boton><Boton onClick={{guardar}}>Save</Boton></>}}>\n",
        js(&gotv_engine::tr!("Agregar {}", "Add {}", singular.to_lowercase()))
    );
    for c in &cs {
        let set = format!("setBorrador({{ ...borrador, {}: ", c.id);
        a += &match c.tipo {
            "opciones" => format!(
                "        <Selector etiqueta={{{}}} opciones={{[...{}]}} valor={{borrador.{}}} error={{errores.{}}} alCambiar={{(v) => {}v }})}} />\n",
                js(&c.etiqueta), opc(c), c.id, c.id, set
            ),
            "si_no" => format!(
                "        <Selector etiqueta={{{}}} opciones={{[\"no\", \"{si}\"]}} valor={{borrador.{}}} alCambiar={{(v) => {}v }})}} />\n",
                js(&c.etiqueta), c.id, set
            ),
            t => {
                let tipo_input = match t {
                    "numero" | "dinero" => " type=\"number\" inputMode=\"decimal\" min={0}",
                    "correo" => " type=\"email\" autoComplete=\"email\"",
                    "fecha" => " type=\"date\"",
                    _ => "",
                };
                format!(
                    "        <Campo etiqueta={{{}}}{tipo_input} value={{borrador.{}}} error={{errores.{}}} onChange={{(e) => {}e.target.value }})}} />\n",
                    js(&c.etiqueta), c.id, c.id, set
                )
            }
        };
    }
    a += "      </Dialogo>\n    </>\n  );\n}\n\n";
    a += &gotv_engine::tr!(
        "export default function App() {{\n  return (\n    <Avisos>\n      <Aplicacion marca={{{}}} actual=\"lista\" enlaces={{[\n        {{ id: \"inicio\", texto: \"Inicio\", icono: \"house\" }},\n        {{ id: \"lista\", texto: {}, icono: \"clipboard-list\" }},\n        {{ id: \"ajustes\", texto: \"Ajustes\", icono: \"settings\" }},\n      ]}}>\n        <Lista />\n      </Aplicacion>\n    </Avisos>\n  );\n}}\n",
        "export default function App() {{\n  return (\n    <Avisos>\n      <Aplicacion marca={{{}}} actual=\"lista\" enlaces={{[\n        {{ id: \"inicio\", texto: \"Home\", icono: \"house\" }},\n        {{ id: \"lista\", texto: {}, icono: \"clipboard-list\" }},\n        {{ id: \"ajustes\", texto: \"Settings\", icono: \"settings\" }},\n      ]}}>\n        <Lista />\n      </Aplicacion>\n    </Avisos>\n  );\n}}\n",
        js(marca),
        js(&plural)
    );

    // ── App.test.tsx
    let mut at = format!("import {{ {}render, screen, within }} from \"@testing-library/react\";\nimport {{ expect, it }} from \"vitest\";\nimport App from \"./App\";\nimport {{ SEMILLA }} from \"./datos\";\n\n", if malos.is_empty() { "" } else { "fireEvent, " });
    at += gotv_engine::i18n::t("it(\"muestra la lista\", () => {\n  render(<App />);\n  expect(screen.getByRole(\"heading\", { level: 1 })).toBeTruthy();\n  expect(within(screen.getByRole(\"table\")).getAllByRole(\"row\")).toHaveLength(SEMILLA.length + 1);\n});\n",
"it(\"shows the list\", () => {\n  render(<App />);\n  expect(screen.getByRole(\"heading\", { level: 1 })).toBeTruthy();\n  expect(within(screen.getByRole(\"table\")).getAllByRole(\"row\")).toHaveLength(SEMILLA.length + 1);\n});\n"
    );
    if !malos.is_empty() {
        at += gotv_engine::i18n::t("\nit(\"no crea uno inválido y explica por qué\", () => {\n  render(<App />);\n  fireEvent.click(screen.getByRole(\"button\", { name: /^agregar/i }));\n  fireEvent.click(screen.getByRole(\"button\", { name: /^guardar$/i }));\n  expect(screen.getAllByRole(\"alert\").length).toBeGreaterThan(0);\n  expect(within(screen.getByRole(\"table\")).getAllByRole(\"row\")).toHaveLength(SEMILLA.length + 1);\n});\n",
"\nit(\"does not create an invalid one and explains why\", () => {\n  render(<App />);\n  fireEvent.click(screen.getByRole(\"button\", { name: /^add/i }));\n  fireEvent.click(screen.getByRole(\"button\", { name: /^save$/i }));\n  expect(screen.getAllByRole(\"alert\").length).toBeGreaterThan(0);\n  expect(within(screen.getByRole(\"table\")).getAllByRole(\"row\")).toHaveLength(SEMILLA.length + 1);\n});\n"
        );
    }
    Ok([d, t, a, at])
}

/// Los archivos del proyecto nuevo: `(ruta, bytes)`. `nombre` va al package.json (minúsculas,
/// guiones); `marca` es el nombre visible; `direccion` y `tema` como en las páginas.
pub fn nueva(nombre: &str, marca: &str, dir_id: &str, pedido: Option<Pedido>) -> anyhow::Result<Vec<(String, Vec<u8>)>> {
    nueva_con(nombre, marca, dir_id, pedido, None)
}

/// Como [`nueva`], y si llega una `entidad`, la lógica, las pruebas y las pantallas se generan
/// para ESA entidad en vez del CRUD de ejemplo.
pub fn nueva_con(nombre: &str, marca: &str, dir_id: &str, pedido: Option<Pedido>, entidad: Option<&Entidad>) -> anyhow::Result<Vec<(String, Vec<u8>)>> {
    let mut v = nueva_base(nombre, marca, dir_id, pedido)?;
    if let Some(e) = entidad {
        let [datos, datos_test, app, app_test] = generar(e, marca)?;
        for (ruta, cuerpo) in [("src/datos.ts", datos), ("src/datos.test.ts", datos_test), ("src/App.tsx", app), ("src/App.test.tsx", app_test)] {
            if let Some(x) = v.iter_mut().find(|(r, _)| r == ruta) {
                x.1 = cuerpo.into_bytes();
            }
        }
    }
    Ok(v)
}

fn nueva_base(nombre: &str, marca: &str, dir_id: &str, pedido: Option<Pedido>) -> anyhow::Result<Vec<(String, Vec<u8>)>> {
    let dir = direccion(dir_id).ok_or_else(|| {
        gotv_engine::anyhow_tr!("direccion: «{dir_id}» no existe; usa una de: {}", "direccion: «{dir_id}» does not exist; use one of: {}", DIRECCIONES.iter().map(|d| d.id).collect::<Vec<_>>().join(", "))
    })?;
    let paquete: String = nombre
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .trim_matches('-')
        .to_string();
    if paquete.is_empty() {
        gotv_engine::bail_tr!("nombre: necesito letras o números para el package.json", "nombre: I need letters or digits for the package.json");
    }
    let pedido = pedido.unwrap_or(Pedido {
        marca: dir.marca.into(),
        croma: dir.croma.into(),
        radio: dir.radio.into(),
        escala: dir.escala.into(),
        movimiento: dir.movimiento.into(),
        // una aplicación sigue al sistema salvo que la dirección sea oscura por naturaleza
        modo: if dir.modo == "oscuro" { "oscuro".into() } else { "auto".into() },
        fuentes: dir.id.into(),
    });
    let tema = Tema::new(pedido);
    let marca_ts = marca.replace(['<', '>'], "");
    let fondo = if tema.pedido.modo == "oscuro" { &tema.oscuro.colores["fondo"] } else { &tema.claro.colores["fondo"] };
    let mut v: Vec<(String, Vec<u8>)> = vec![
        ("package.json".into(), PAQUETE.replace("__NOMBRE__", &paquete).into_bytes()),
        ("tsconfig.json".into(), TSCONFIG.as_bytes().to_vec()),
        ("vite.config.ts".into(), VITE.as_bytes().to_vec()),
        (
            "index.html".into(),
            format!(
                "<!doctype html>\n<html lang=\"es\" data-direccion=\"{}\">\n  <head>\n    <meta charset=\"utf-8\">\n    <meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\n    <meta name=\"theme-color\" content=\"{fondo}\">\n    <title>{}</title>\n  </head>\n  <body>\n    <div id=\"raiz\"></div>\n    <script type=\"module\" src=\"/src/main.tsx\"></script>\n  </body>\n</html>\n",
                dir.id,
                crate::pagina::esc(marca)
            )
            .into_bytes(),
        ),
        (".gitignore".into(), b"node_modules\ndist\n*.local\n".to_vec()),
        ("src/main.tsx".into(), MAIN.as_bytes().to_vec()),
        ("src/App.tsx".into(), plantilla().app.replace("__MARCA__", &serde_json::to_string(&marca_ts)?).into_bytes()),
        ("src/App.test.tsx".into(), plantilla().app_test.as_bytes().to_vec()),
        ("src/datos.ts".into(), plantilla().datos.as_bytes().to_vec()),
        ("src/datos.test.ts".into(), plantilla().datos_test.as_bytes().to_vec()),
        ("src/gotv/gotv.tsx".into(), gotv_tsx().into_bytes()),
    ];
    // iconos: los mismos SVG de las páginas, sin el comentario de licencia (va en LICENCIAS)
    let mut iconos = String::from("// Iconos Lucide (ISC), generados por GOD OF THE VOID. Licencia en ./LICENCIAS.txt\nexport const ICONOS = {\n");
    for (n, _) in recursos::ICONOS {
        let svg = recursos::icono(n, None).unwrap_or_default().split_whitespace().collect::<Vec<_>>().join(" ");
        iconos += &format!("  {}: {},\n", serde_json::to_string(n)?, serde_json::to_string(&svg)?);
    }
    iconos += "} as const;\n";
    v.push(("src/gotv/iconos.ts".into(), iconos.into_bytes()));
    let css = gotv_engine::tr!(
        "/* Tema calculado + biblioteca de GOD OF THE VOID. Cambia el tema con `gotv tema`, no a mano. */\n{}{}\n.js .revela {{ transition-delay: calc(var(--i, 0) * 70ms); }}\n{}",
        "/* Computed theme + GOD OF THE VOID library. Change the theme with `gotv tema`, not by hand. */\n{}{}\n.js .revela {{ transition-delay: calc(var(--i, 0) * 70ms); }}\n{}",
        tema.font_faces("./fuentes/"),
        tema.css(),
        crate::pagina::CSS_BASE
    );
    v.push(("src/gotv/estilos.css".into(), css.into_bytes()));
    let mut licencias = String::from("Fuentes (SIL Open Font License 1.1):\n\n");
    for f in tema.fuentes() {
        v.push((format!("src/gotv/fuentes/{}.woff2", f.slug), f.woff2.to_vec()));
        licencias += &format!("── {} ──\n{}\n\n", f.familia, f.licencia);
    }
    licencias += &format!("── Iconos: Lucide ──\n{}\n", recursos::LICENCIA_ICONOS);
    v.push(("src/gotv/LICENCIAS.txt".into(), licencias.into_bytes()));
    v.push((
        "README.md".into(),
        gotv_engine::tr!(
            "# {marca}\n\nHecho con GOD OF THE VOID (dirección «{}»).\n\n```bash\nnpm install\nnpm run dev      # desarrollo\nnpm test         # pruebas\nnpm run build    # producción\n```\n\n- `src/datos.ts`: la lógica del negocio, sin interfaz (y `datos.test.ts`).\n- `src/App.tsx`: las pantallas, armadas con `src/gotv/gotv.tsx`.\n- `src/gotv/estilos.css`: tema calculado (contraste garantizado) + biblioteca. No se edita a mano: se regenera.\n\n## Piezas (`src/gotv/gotv.tsx`)\n\n- Armazón: `Aplicacion` (barra lateral + principal), `Panel`, `Pestanas`, `Avisos` + `useAviso`.\n- Datos: `Tabla` (orden, filtro, paginación), `Estadistica` (con `cambio` y `serie` para su tendencia), `InsigniaEstado`.\n- Gráficas: `Barras` (cantidades por categoría, valor en la punta) y `Lineas` (cambio en el tiempo, hasta 6 series; leyenda, vista de tabla, ratón y flechas). Una escala por gráfica; los colores de serie son `--serie-1`…`--serie-6`, fijos y en ese orden.\n- Formulario: `Campo`, `AreaTexto`, `Selector`, `Casilla` (espera a guardar), `Interruptor` (se aplica al instante), `Boton`, `Dialogo`.\n- Estados: `EstadoVacio`, `Cargando`, `EstadoError`.\n",
            "# {marca}\n\nMade with GOD OF THE VOID (direction «{}»).\n\n```bash\nnpm install\nnpm run dev      # development\nnpm test         # tests\nnpm run build    # production\n```\n\n- `src/datos.ts`: the business logic, without UI (and `datos.test.ts`).\n- `src/App.tsx`: the screens, built with `src/gotv/gotv.tsx`.\n- `src/gotv/estilos.css`: computed theme (guaranteed contrast) + library. Not edited by hand: it is regenerated.\n\n## Pieces (`src/gotv/gotv.tsx`)\n\n- Shell: `Aplicacion` (sidebar + main), `Panel`, `Pestanas`, `Avisos` + `useAviso`.\n- Data: `Tabla` (sort, filter, pagination), `Estadistica` (with `cambio` and `serie` for its trend), `InsigniaEstado`.\n- Charts: `Barras` (amounts by category, value at the tip) and `Lineas` (change over time, up to 6 series; legend, table view, mouse and arrow keys). One scale per chart; the series colors are `--serie-1`…`--serie-6`, fixed and in that order.\n- Form: `Campo`, `AreaTexto`, `Selector`, `Casilla` (waits for save), `Interruptor` (applies at once), `Boton`, `Dialogo`.\n- States: `EstadoVacio`, `Cargando`, `EstadoError`.\n",
            dir.id
        )
        .into_bytes(),
    ));
    Ok(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn el_esqueleto_trae_todo_y_fija_versiones() {
        let a = nueva("Mi Panel!", "Mi \"Panel\"", "tecnico", None).unwrap();
        let rutas: Vec<&str> = a.iter().map(|(r, _)| r.as_str()).collect();
        for r in ["package.json", "tsconfig.json", "vite.config.ts", "index.html", "src/App.tsx", "src/datos.ts", "src/datos.test.ts", "src/App.test.tsx", "src/gotv/gotv.tsx", "src/gotv/iconos.ts", "src/gotv/estilos.css", "src/gotv/LICENCIAS.txt"] {
            assert!(rutas.contains(&r), "falta {r}");
        }
        let pk = String::from_utf8(a[0].1.clone()).unwrap();
        let v: serde_json::Value = serde_json::from_str(&pk).unwrap();
        assert_eq!(v["name"], "mi-panel");
        for (_, ver) in v["dependencies"].as_object().unwrap().iter().chain(v["devDependencies"].as_object().unwrap()) {
            let ver = ver.as_str().unwrap();
            assert!(ver.chars().next().unwrap().is_ascii_digit(), "versión sin fijar: {ver}");
        }
        let app = String::from_utf8(a.iter().find(|(r, _)| r == "src/App.tsx").unwrap().1.clone()).unwrap();
        assert!(app.contains("marca={\"Mi \\\"Panel\\\"\"}"), "la marca entra como expresión JSON: en un atributo JSX `\\\"` NO escapa");
        let ico = String::from_utf8(a.iter().find(|(r, _)| r == "src/gotv/iconos.ts").unwrap().1.clone()).unwrap();
        assert!(ico.contains("\"arrow-right\":") && !ico.contains("@license"));
        // todos los nombres de icono que usa el código existen
        let tsx = format!("{}{app}", gotv_tsx());
        for n in ["loader-circle", "triangle-alert", "search", "chevron-up", "x", "circle-x", "info", "circle-check", "folder", "plus", "house", "users", "chart-column", "settings", "layers"] {
            assert!(tsx.contains(&format!("\"{n}\"")) || n == "layers", "{n}");
            assert!(recursos::icono(n, None).is_some(), "el icono {n} no existe");
        }
        assert!(nueva("x", "x", "nada", None).is_err());
        assert!(nueva("!!!", "x", "tecnico", None).is_err());
    }

    #[test]
    fn en_ingles_la_app_generada_habla_ingles() {
        use gotv_engine::i18n::{set_thread, Lang};
        // cada texto de la tabla sigue en la biblioteca: si gotv.tsx cambia, la traducción no se pierde callada
        for (es, _) in GOTV_TSX_EN {
            assert!(GOTV_TSX.contains(es), "gotv.tsx ya no trae «{es}»");
        }
        set_thread(Some(Lang::En));
        let base = nueva("panel", "Panel", "tecnico", None).unwrap();
        let [d, t, a, at] = generar(&cafe(), "Grano").unwrap();
        set_thread(None);
        let textos = |v: &[(String, Vec<u8>)]| {
            v.iter().filter(|(r, _)| r.ends_with(".ts") || r.ends_with(".tsx") || r == "README.md").map(|(_, b)| String::from_utf8_lossy(b).into_owned()).collect::<String>()
        };
        let todo = textos(&base) + &d + &t + &a + &at;
        // juez-idioma: bloque — datos: lo que NO debe quedar en la app en inglés
        for es in ["Cargando…", "Sin resultados", "Anterior", "Siguiente", "Guardar", "Cancelar", "Agregar", "Escribe", "Elige", "Nuevo cliente", "Inicio", "Ajustes", "acepta un borrador", "Hecho con", "Lógica del negocio"] {
            assert!(!todo.contains(es), "queda «{es}» en la app en inglés");
        }
        assert!(todo.contains("No results") && todo.contains(">Save<") && todo.contains("New customer") && a.contains("\"yes\"") && d.contains("=== \"yes\""), "{a}");
        // y en español, nada cambió
        let es = nueva("panel", "Panel", "tecnico", None).unwrap();
        assert!(textos(&es).contains("Sin resultados") && textos(&es).contains("Nuevo cliente"));
    }

    pub(crate) fn cafe() -> Entidad {
        serde_json::from_str(r#"{"nombre":"pedido","campos":[
            {"nombre":"Cliente","tipo":"texto"},{"nombre":"Bebida","tipo":"texto"},
            {"nombre":"Cantidad","tipo":"número"},{"nombre":"Precio","tipo":"dinero"},
            {"nombre":"Estado","opciones":["pendiente","preparando","entregado"]},
            {"nombre":"Para llevar","tipo":"bool"},{"nombre":"id","tipo":"texto"}]}"#).unwrap()
    }

    #[test]
    fn la_entidad_genera_logica_pruebas_y_pantallas_coherentes() {
        let [d, t, a, at] = generar(&cafe(), "Grano {Alto}").unwrap();
        assert!(d.contains("export const OPCIONES_ESTADO = [\"pendiente\", \"preparando\", \"entregado\"] as const;"), "{d}");
        assert!(d.contains("export type Pedido = { id: number; cliente: string; bebida: string; cantidad: number; precio: number; estado: (typeof OPCIONES_ESTADO)[number]; para_llevar: boolean; codigo: string };"), "{d}");
        assert!(d.contains("suma_precio") && d.contains("por_estado") && d.contains("importe: lista.reduce((s, x) => s + x.cantidad * x.precio, 0)"), "{d}");
        assert!(t.contains("toBe(2 * 5000 + 3 * 1000)") && a.contains("Total (cantidad × precio)") && !a.contains("\"Precio total\""), "{a}");
        assert!(t.contains("VACIO") && t.contains("[\"bebida\", \"cantidad\", \"cliente\", \"codigo\", \"precio\"]"), "{t}");
        assert!(a.contains("<Selector") && a.contains("<Pestanas") && a.contains("DINERO.format(x.precio)") && a.contains("marca={\"Grano {Alto}\"}") && a.contains("· Grano Alto`"), "{a}");
        assert!(at.contains("fireEvent"));
        // con importe y un campo de opciones, la app nace con su gráfica y la prueba de que cuadra
        assert!(d.contains("importe_por_estado: Object.fromEntries(OPCIONES_ESTADO.map((o) => [o, lista.filter((x) => x.estado === o).reduce((s, x) => s + x.cantidad * x.precio, 0)]))"), "{d}");
        assert!(t.contains("resumen(SEMILLA).importe_por_estado).reduce((a, b) => a + b, 0)).toBeCloseTo(resumen(SEMILLA).importe)"), "{t}");
        assert!(a.contains("<Barras titulo={\"Total por estado\"} datos={OPCIONES_ESTADO.map(") && a.contains("formato={(n) => DINERO.format(n)} parte />") && a.contains("Barras, Boton"), "{a}");
        // sin campos que validar no se importa lo que no se usa (noUnusedLocals)
        let solo: Entidad = serde_json::from_str(r#"{"name":"tarea","fields":[{"name":"Hecha","type":"boolean"}]}"#).unwrap();
        let [_, t, a, at] = generar(&solo, "x").unwrap();
        assert!(!t.contains("VACIO,") && !at.contains("fireEvent") && !a.contains("Campo,") && !a.contains("Pestanas"), "{t}\n{a}");
        let tabla: Entidad = serde_json::from_str(r#"{"nombre":"tabla","campos":[{"nombre":"x"}]}"#).unwrap();
        assert!(generar(&tabla, "x").unwrap()[0].contains("export type TablaDato"));
        assert!(generar(&Entidad { nombre: "x".into(), plural: None, campos: vec![] }, "x").is_err());
    }
}
