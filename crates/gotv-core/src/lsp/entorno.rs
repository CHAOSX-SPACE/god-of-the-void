//! F5 · ML5 · el entorno CORRECTO de cada servidor, solo: lo que un servidor necesita saber del proyecto y
//! que sin esto adivina mal (el «import no resuelto» falso de Python sin su `.venv`, el TypeScript global
//! en vez del del proyecto, rust-analyzer compilando en la carpeta del usuario).
//! - `inicio(lenguaje, raiz)` → `initializationOptions`.
//! - `ajustes(lenguaje, raiz)` → la configuración con la que se contesta `workspace/configuration` (antes:
//!   nulos), buscando cada `section` pedida dentro de ella.
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

/// El intérprete de Python del proyecto: `.venv` / `venv` / `.python-version` de uv o pyenv no hace falta:
/// basta con el ejecutable del entorno. Poetry y conda dejan su ruta en el entorno del shell
/// (`VIRTUAL_ENV`, `CONDA_PREFIX`).
pub fn python_del_proyecto(raiz: &Path) -> Option<PathBuf> {
    for d in [".venv", "venv", "env"] {
        let p = raiz.join(d).join("bin/python");
        if p.is_file() {
            return Some(p);
        }
    }
    for var in ["VIRTUAL_ENV", "CONDA_PREFIX"] {
        if let Some(v) = std::env::var_os(var) {
            let p = PathBuf::from(v).join("bin/python");
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}

/// ¿El proyecto de Rust compila sólo en release? (tiene `target/release` y no `target/debug`: el repo de
/// GOTV, donde rige «máximo 3 compilaciones, sólo release»)
fn solo_release(raiz: &Path) -> bool {
    raiz.join("target/release").is_dir() && !raiz.join("target/debug").is_dir()
}

pub fn inicio(lang: &str, raiz: &Path) -> Value {
    match lang {
        // el TypeScript DEL proyecto (sus tipos, su versión), no uno global
        "typescript" => {
            let ts = raiz.join("node_modules/typescript/lib");
            if ts.is_dir() {
                json!({"tsserver": {"path": ts.display().to_string()}})
            } else {
                json!({})
            }
        }
        "rust" => ajustes(lang, raiz)["rust-analyzer"].clone(),
        _ => json!({}),
    }
}

pub fn ajustes(lang: &str, raiz: &Path) -> Value {
    match lang {
        "python" => match python_del_proyecto(raiz) {
            Some(py) => json!({"python": {"pythonPath": py.display().to_string(), "defaultInterpreterPath": py.display().to_string(), "analysis": {"autoSearchPaths": true, "useLibraryCodeForTypes": true}}}),
            None => json!({"python": {"analysis": {"autoSearchPaths": true}}}),
        },
        // rust-analyzer: en el repo «sólo release», compila en `target/release` (no crea otra carpeta);
        // en los demás, en su PROPIA carpeta (`target/rust-analyzer`): no bloquea ni mezcla con la
        // compilación del usuario
        "rust" => {
            if solo_release(raiz) {
                json!({"rust-analyzer": {"cargo": {"extraArgs": ["--release"]}, "check": {"extraArgs": ["--release"]}}})
            } else {
                json!({"rust-analyzer": {"cargo": {"targetDir": true}}})
            }
        }
        _ => json!({}),
    }
}

/// La respuesta a `workspace/configuration`: por cada ítem, lo que haya en `section` (con puntos) dentro de
/// los ajustes; nulo si no hay.
pub fn responder_configuracion(ajustes: &Value, items: &[Value]) -> Value {
    Value::Array(
        items
            .iter()
            .map(|it| match it["section"].as_str() {
                Some(s) if !s.is_empty() => s.split('.').fold(Some(ajustes), |v, k| v.and_then(|v| v.get(k))).cloned().unwrap_or(Value::Null),
                _ => ajustes.clone(),
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn el_entorno_de_cada_lenguaje() {
        let d = crate::CarpetaPrueba::nueva("gotv-lsp-entorno");
        let r = d.canonicalize().unwrap();
        // Python con .venv
        std::fs::create_dir_all(r.join(".venv/bin")).unwrap();
        std::fs::write(r.join(".venv/bin/python"), "").unwrap();
        let a = ajustes("python", &r);
        assert_eq!(a["python"]["pythonPath"], r.join(".venv/bin/python").display().to_string());
        // workspace/configuration: por sección con puntos, o todo
        let c = responder_configuracion(&a, &[json!({"section": "python.pythonPath"}), json!({"section": "python.analysis"}), json!({"section": "nada"}), json!({})]);
        assert_eq!(c[0], a["python"]["pythonPath"]);
        assert_eq!(c[1]["useLibraryCodeForTypes"], true);
        assert!(c[2].is_null() && c[3] == a);
        // TypeScript del proyecto
        assert_eq!(inicio("typescript", &r), json!({}));
        std::fs::create_dir_all(r.join("node_modules/typescript/lib")).unwrap();
        assert!(inicio("typescript", &r)["tsserver"]["path"].as_str().unwrap().ends_with("node_modules/typescript/lib"));
        // Rust: carpeta propia; o release si el proyecto es «sólo release»
        assert_eq!(inicio("rust", &r)["cargo"]["targetDir"], true);
        std::fs::create_dir_all(r.join("target/release")).unwrap();
        assert_eq!(inicio("rust", &r)["cargo"]["extraArgs"][0], "--release");
        assert_eq!(ajustes("rust", &r)["rust-analyzer"]["check"]["extraArgs"][0], "--release");
    }
}
