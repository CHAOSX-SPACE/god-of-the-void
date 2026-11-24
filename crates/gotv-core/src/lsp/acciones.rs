//! F5 · ML3 · preguntas y cambios. Los nombres de las acciones en español, con los ingleses de antes como
//! alias; y los CAMBIOS que calcula el servidor (`renombrar`, `arreglar`) como ediciones que aplica GOTV:
//! el servidor nunca escribe. Un `WorkspaceEdit` que crea, renombra o borra archivos se rechaza (eso va por
//! las herramientas de GOTV), y nada fuera del proyecto se toca.
use anyhow::{anyhow, bail, Result};
use serde_json::Value;
use std::path::{Path, PathBuf};

/// El nombre interno de una acción (español o inglés).
pub fn normal(accion: &str) -> &str {
    match accion {
        "definicion" | "definición" | "definition" => "definition", // juez-idioma: datos (alias de acción)
        "referencias" | "references" => "references",
        "hover" | "info" => "hover",
        "errores" | "diagnostics" => "diagnostics",
        "esquema" | "symbols" => "symbols",
        "buscar_simbolo" | "workspace" => "workspace",
        "implementaciones" | "implementation" => "implementation",
        "tipo" | "type" | "type_definition" => "type",
        "quien_llama" | "quién_llama" | "incoming" | "incoming_calls" => "incoming", // juez-idioma: datos (alias de acción)
        "a_quien_llama" | "a_quién_llama" | "outgoing" | "outgoing_calls" => "outgoing", // juez-idioma: datos (alias de acción)
        "renombrar" | "rename" => "rename",
        "arreglar" | "fix" | "code_action" => "fix",
        otra => otra,
    }
}

/// ¿Cambia archivos? (pasa por los permisos de edición y deja punto de restauración)
pub fn cambia(accion: &str) -> bool {
    matches!(normal(accion), "rename" | "fix")
}

/// Una edición de texto LSP: rango (línea, carácter UTF-16) y texto nuevo.
#[derive(Debug, Clone, PartialEq)]
pub struct Edicion {
    pub desde: (u64, u64),
    pub hasta: (u64, u64),
    pub texto: String,
}

fn pos(v: &Value) -> (u64, u64) {
    (v["line"].as_u64().unwrap_or(0), v["character"].as_u64().unwrap_or(0))
}

/// Un `WorkspaceEdit` → (archivo, ediciones). Rechaza crear/renombrar/borrar archivos.
pub fn de_workspace_edit(we: &Value) -> Result<Vec<(PathBuf, Vec<Edicion>)>> {
    let mut out: Vec<(PathBuf, Vec<Edicion>)> = Vec::new();
    let mut meter = |uri: &str, edits: &Value| {
        let p = PathBuf::from(uri.trim_start_matches("file://"));
        let e: Vec<Edicion> = edits.as_array().into_iter().flatten().map(|t| Edicion { desde: pos(&t["range"]["start"]), hasta: pos(&t["range"]["end"]), texto: t["newText"].as_str().unwrap_or("").to_string() }).collect();
        match out.iter_mut().find(|(q, _)| *q == p) {
            Some((_, v)) => v.extend(e),
            None => out.push((p, e)),
        }
    };
    if let Some(dc) = we["documentChanges"].as_array() {
        for c in dc {
            if let Some(k) = c["kind"].as_str() {
                bail!("{}", crate::tr!("el servidor quiere {k} un archivo: GOTV no deja que el servidor cree, renombre ni borre archivos (hazlo con sus herramientas)", "the server wants to {k} a file: GOTV does not let the server create, rename or delete files (do it with its tools)"));
            }
            meter(c["textDocument"]["uri"].as_str().unwrap_or(""), &c["edits"]);
        }
    }
    for (uri, edits) in we["changes"].as_object().into_iter().flatten() {
        meter(uri, edits);
    }
    if out.iter().all(|(_, e)| e.is_empty()) {
        bail!("{}", crate::i18n::t("el servidor no propuso ningún cambio", "the server proposed no change"));
    }
    Ok(out)
}

/// El byte de (línea, carácter UTF-16) en `texto` (las columnas de LSP cuentan en UTF-16).
fn byte_de(texto: &str, (linea, car): (u64, u64)) -> Result<usize> {
    let mut inicio = 0usize;
    for _ in 0..linea {
        inicio += texto[inicio..].find('\n').ok_or_else(|| anyhow!("{}", crate::i18n::t("el cambio apunta más allá del final del archivo", "the change points past the end of the file")))? + 1;
    }
    let fin_linea = texto[inicio..].find('\n').map(|i| inicio + i).unwrap_or(texto.len());
    let mut u16s = 0u64;
    for (i, ch) in texto[inicio..fin_linea].char_indices() {
        if u16s >= car {
            return Ok(inicio + i);
        }
        u16s += ch.len_utf16() as u64;
    }
    Ok(fin_linea)
}

/// Aplica las ediciones a un texto (de la última a la primera, para no correr las posiciones).
pub fn aplicar(texto: &str, ediciones: &[Edicion]) -> Result<String> {
    let mut rangos: Vec<(usize, usize, &str)> = ediciones.iter().map(|e| Ok((byte_de(texto, e.desde)?, byte_de(texto, e.hasta)?, e.texto.as_str()))).collect::<Result<_>>()?;
    rangos.sort_by(|a, b| b.0.cmp(&a.0));
    let mut s = texto.to_string();
    let mut tope = usize::MAX;
    for (a, b, t) in rangos {
        if b > tope || a > b {
            bail!("{}", crate::i18n::t("el servidor mandó ediciones que se pisan", "the server sent overlapping edits"));
        }
        s.replace_range(a..b, t);
        tope = a;
    }
    Ok(s)
}

/// Aplica un `WorkspaceEdit` a los archivos del proyecto; devuelve (archivo, cuántas ediciones). Primero
/// comprueba TODO (dentro del proyecto, posiciones válidas) y sólo entonces escribe.
pub fn aplicar_en_disco(root: &Path, cambios: &[(PathBuf, Vec<Edicion>)]) -> Result<Vec<(PathBuf, usize)>> {
    let raiz = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let mut listos = Vec::new();
    for (p, e) in cambios {
        let real = p.canonicalize().map_err(|_| anyhow!("{}", crate::tr!("{} no existe", "{} does not exist", p.display())))?;
        if !real.starts_with(&raiz) {
            bail!("{}", crate::tr!("el cambio toca {} fuera del proyecto: no se aplica nada", "the change touches {} outside the project: nothing is applied", p.display()));
        }
        let nuevo = aplicar(&std::fs::read_to_string(&real)?, e)?;
        listos.push((real, nuevo, e.len()));
    }
    let mut out = Vec::new();
    for (p, nuevo, n) in listos {
        std::fs::write(&p, nuevo)?;
        out.push((p, n));
    }
    Ok(out)
}

/// F5 ML2: las líneas (desde 1, inclusivas) que una edición VA a tocar en `texto` (el archivo de antes):
/// `edit` y `multi_edit` por dónde está su `old`; `write_file` por la primera y la última línea que
/// cambian. `None` si no se sabe (archivo nuevo, `old` que no está, añadir al final).
pub fn zona(herramienta: &str, args: &Value, texto: &str) -> Option<(u64, u64)> {
    let de = |old: &str| -> Option<(u64, u64)> {
        let i = texto.find(old).filter(|_| !old.is_empty())?;
        let desde = texto[..i].matches('\n').count() as u64 + 1;
        Some((desde, desde + old.trim_end_matches('\n').matches('\n').count() as u64))
    };
    match herramienta {
        "edit" => de(args["old"].as_str()?),
        "multi_edit" => args["edits"].as_array()?.iter().filter_map(|e| de(e["old"].as_str()?)).reduce(|a, b| (a.0.min(b.0), a.1.max(b.1))),
        "write_file" if args["append"].as_bool() != Some(true) => {
            let (a, b): (Vec<&str>, Vec<&str>) = (texto.lines().collect(), args["content"].as_str()?.lines().collect());
            let pre = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
            let suf = a[pre..].iter().rev().zip(b[pre..].iter().rev()).take_while(|(x, y)| x == y).count();
            (pre < a.len().max(b.len())).then(|| ((pre + 1) as u64, (a.len() - suf).max(pre + 1) as u64))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn la_zona_que_toca_cada_edicion() {
        let t = "uno\ndos\ntres\ncuatro\n";
        assert_eq!(zona("edit", &json!({"old": "dos\ntres"}), t), Some((2, 3)));
        assert_eq!(zona("edit", &json!({"old": "cuatro\n"}), t), Some((4, 4)));
        assert_eq!(zona("edit", &json!({"old": "cinco"}), t), None);
        assert_eq!(zona("multi_edit", &json!({"edits": [{"old": "tres"}, {"old": "uno"}]}), t), Some((1, 3)));
        assert_eq!(zona("write_file", &json!({"content": "uno\nDOS\ntres\ncuatro\n"}), t), Some((2, 2)));
        assert_eq!(zona("write_file", &json!({"content": "uno\ndos\ntres\ncuatro\ncinco\n"}), t), Some((5, 5)));
        assert_eq!(zona("write_file", &json!({"content": t}), t), None, "sin cambios");
        assert_eq!(zona("write_file", &json!({"content": "x", "append": true}), t), None);
    }

    #[test]
    fn las_columnas_son_utf16_y_las_ediciones_no_se_pisan() {
        // «ñ» es 1 en UTF-16 pero 2 bytes; «🕳» es 2 en UTF-16 y 4 bytes
        let t = "let año = 1;\nlet 🕳x = año;\n";
        let e = |l1, c1, l2, c2, s: &str| Edicion { desde: (l1, c1), hasta: (l2, c2), texto: s.into() };
        assert_eq!(aplicar(t, &[e(0, 4, 0, 7, "anio"), e(1, 10, 1, 13, "anio")]).unwrap(), "let anio = 1;\nlet 🕳x = anio;\n");
        assert_eq!(aplicar(t, &[e(1, 4, 1, 7, "y")]).unwrap(), "let año = 1;\nlet y = año;\n", "el emoji cuenta 2");
        assert!(aplicar(t, &[e(0, 0, 0, 5, "a"), e(0, 3, 0, 8, "b")]).is_err(), "se pisan");
        assert!(aplicar(t, &[e(9, 0, 9, 1, "x")]).is_err());
    }

    #[test]
    fn workspace_edit_y_disco() {
        let d = crate::CarpetaPrueba::nueva("gotv-lsp-we");
        let r = d.canonicalize().unwrap();
        std::fs::write(r.join("a.rs"), "fn uno() {}\nfn main() { uno(); }\n").unwrap();
        let u = format!("file://{}", r.join("a.rs").display());
        let we = json!({"changes": {u.clone(): [
            {"range": {"start": {"line": 0, "character": 3}, "end": {"line": 0, "character": 6}}, "newText": "primero"},
            {"range": {"start": {"line": 1, "character": 12}, "end": {"line": 1, "character": 15}}, "newText": "primero"}]}});
        let c = de_workspace_edit(&we).unwrap();
        let hecho = aplicar_en_disco(&r, &c).unwrap();
        assert_eq!(hecho[0].1, 2);
        assert_eq!(std::fs::read_to_string(r.join("a.rs")).unwrap(), "fn primero() {}\nfn main() { primero(); }\n");
        // crear o renombrar archivos: no
        assert!(de_workspace_edit(&json!({"documentChanges": [{"kind": "rename", "oldUri": u, "newUri": "file:///x"}]})).is_err());
        // fuera del proyecto: no se aplica NADA (ni lo de dentro)
        let fuera = crate::CarpetaPrueba::nueva("gotv-lsp-we-fuera");
        std::fs::write(fuera.join("b.rs"), "x").unwrap();
        let mal = vec![(r.join("a.rs"), vec![Edicion { desde: (0, 0), hasta: (0, 2), texto: "XX".into() }]), (fuera.join("b.rs"), vec![Edicion { desde: (0, 0), hasta: (0, 1), texto: "y".into() }])];
        assert!(aplicar_en_disco(&r, &mal).is_err());
        assert!(std::fs::read_to_string(r.join("a.rs")).unwrap().starts_with("fn primero"), "nada se escribió");
        assert_eq!(normal("renombrar"), "rename");
        assert_eq!(normal("definition"), "definition");
        assert!(cambia("arreglar") && !cambia("referencias"));
    }
}
