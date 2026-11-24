//! F4 paso 5 · La vista del notebook en la ventana: sus celdas con sus salidas (texto, imágenes, errores con su
//! traza), la tocada resaltada. Se saca SIEMPRE del archivo tal como quedó (la misma al reabrir la conversación);
//! la ventana sólo pinta (`ui/cuaderno-vista.js`) y cambia la tarjeta anterior del mismo notebook por la nueva.

use super::formato::{texto_de, Libro};
use gotv_engine::Message;
use serde_json::{json, Value};
use std::path::Path;

const CELDAS_MAX: usize = 80;
const FUENTE_MAX: usize = 6_000;
const TEXTO_MAX: usize = 4_000;
/// Lo que pueden pesar las imágenes de una tarjeta (viajan en base64 dentro de ella).
const IMAGENES_MAX: usize = 6 << 20;

fn corto(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        return s.to_string();
    }
    format!("{}…", s.chars().take(n).collect::<String>())
}

/// Las acciones que muestran el notebook (las del kernel solo, no).
fn muestra(accion: &str) -> bool {
    matches!(accion, "leer" | "crear" | "insertar" | "reemplazar" | "borrar" | "limpiar_salidas" | "ejecutar")
}

/// La tarjeta del notebook de `args["path"]` tras una llamada a `notebook`.
pub fn tarjeta(root: &Path, args: &Value) -> Option<Value> {
    let accion = args["action"].as_str().or(args["accion"].as_str()).unwrap_or("leer");
    if !muestra(accion) {
        return None;
    }
    let nb = crate::paths::resolve(root, args["path"].as_str()?);
    let libro = Libro::leer(&nb).ok()?;
    // la celda que tocó la llamada
    let marcada: Option<usize> = match accion {
        "insertar" => Some(args["posicion"].as_u64().map(|p| p as usize).unwrap_or(libro.n())),
        "reemplazar" | "limpiar_salidas" | "ejecutar" | "borrar" if !args["celda"].is_null() && args["todas"].as_bool() != Some(true) => {
            libro.indice(&args["celda"]).ok().map(|i| i + 1).filter(|_| accion != "borrar")
        }
        _ => None,
    };
    let n = libro.n();
    // si son demasiadas, las que rodean a la tocada
    let desde = match marcada {
        Some(m) if n > CELDAS_MAX => m.saturating_sub(CELDAS_MAX / 2).min(n - CELDAS_MAX),
        _ => 0,
    };
    let mut peso = 0usize;
    let celdas: Vec<Value> = libro
        .celdas()
        .iter()
        .enumerate()
        .skip(desde)
        .take(CELDAS_MAX)
        .map(|(i, c)| {
            let mut salidas = Vec::new();
            for s in c["outputs"].as_array().into_iter().flatten() {
                let d = &s["data"];
                match s["output_type"].as_str().unwrap_or("") {
                    "stream" => salidas.push(json!({"texto": corto(&texto_de(&s["text"]), TEXTO_MAX), "flujo": s["name"]})),
                    "execute_result" | "display_data" => {
                        if let Some((mime, b64)) = ["image/png", "image/jpeg"].iter().find_map(|m| d[*m].as_str().map(|b| (*m, b))) {
                            let limpio: String = b64.chars().filter(|c| !c.is_whitespace()).collect();
                            if peso + limpio.len() <= IMAGENES_MAX {
                                peso += limpio.len();
                                salidas.push(json!({"imagen": format!("data:{mime};base64,{limpio}")}));
                            } else {
                                salidas.push(json!({"texto": crate::i18n::t("🖼 imagen (no cabe en la vista)", "🖼 image (does not fit in the view)")}));
                            }
                        } else {
                            salidas.push(json!({"texto": corto(&texto_de(&d["text/plain"]), TEXTO_MAX), "resultado": s["output_type"] == "execute_result"}));
                        }
                    }
                    "error" => {
                        let traza: Vec<String> = s["traceback"].as_array().into_iter().flatten().filter_map(|l| l.as_str()).map(crate::exec::strip_ansi).collect();
                        salidas.push(json!({"error": format!("{}: {}", s["ename"].as_str().unwrap_or("Error"), s["evalue"].as_str().unwrap_or("")), "traza": corto(&traza.join("\n"), TEXTO_MAX)}));
                    }
                    _ => {}
                }
            }
            json!({
                "n": i + 1,
                "tipo": c["cell_type"],
                "fuente": corto(&texto_de(&c["source"]), FUENTE_MAX),
                "cuenta": c["execution_count"],
                "salidas": salidas,
            })
        })
        .collect();
    Some(json!({
        "tipo": "cuaderno",
        "titulo": nb.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default(),
        "ruta": nb.display().to_string(),
        "lenguaje": libro.lenguaje().unwrap_or_default(),
        "total": n,
        "marcada": marcada,
        "celdas": celdas,
    }))
}

/// Los argumentos de la llamada `id` (buscada hacia atrás en el historial).
fn args_de(antes: &[Message], id: &str) -> Option<Value> {
    let c = antes.iter().rev().filter_map(|m| m.tool_calls.as_ref()).flatten().find(|c| c["id"].as_str() == Some(id))?;
    match &c["function"]["arguments"] {
        Value::String(s) => serde_json::from_str(s).ok(),
        v => Some(v.clone()),
    }
}

/// Al reabrir la conversación: la tarjeta del notebook en el mensaje `i`, sólo si es la ÚLTIMA llamada a
/// `notebook` sobre ese archivo (una por notebook, con su estado de hoy).
pub fn de_historial(h: &[Message], i: usize, root: &Path) -> Option<Value> {
    let ruta_de = |k: usize| -> Option<(String, Value)> {
        let m = h.get(k)?;
        if m.role != "tool" || m.name.as_deref() != Some("notebook") || m.content.starts_with("ERROR") {
            return None;
        }
        let a = args_de(&h[..k], m.tool_call_id.as_deref()?)?;
        let accion = a["action"].as_str().or(a["accion"].as_str()).unwrap_or("leer");
        muestra(accion).then(|| (crate::paths::resolve(root, a["path"].as_str().unwrap_or("")).display().to_string(), a))
    };
    let (ruta, a) = ruta_de(i)?;
    if (i + 1..h.len()).any(|k| ruta_de(k).is_some_and(|(r, _)| r == ruta)) {
        return None;
    }
    tarjeta(root, &a)
}
