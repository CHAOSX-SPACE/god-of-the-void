//! Memoria del proyecto (X5): las instrucciones que el proyecto deja para CUALQUIER
//! agente — `AGENTS.md` (el estándar común), `CLAUDE.md` (Claude Code) y
//! `.gotv/GOTV.md` — más las del usuario para GOTV (`~/.config/gotv/GOTV.md`).
//!
//! Van en el prompt de sistema: son estables por proyecto, así que la caché de
//! prefijo las paga UNA vez. Tope de caracteres para no ahogar a un modelo local;
//! lo recortado se avisa (se puede leer con `read_file`).
//! NO se carga `~/.claude/CLAUDE.md`: es de Claude Code (sus ritos y herramientas)
//! y a un modelo local sólo le metería instrucciones que no puede cumplir.

use std::path::{Path, PathBuf};

pub const MEMORY_MAX_CHARS: usize = 8_000;

pub fn files(root: &Path) -> Vec<PathBuf> {
    let home = PathBuf::from(std::env::var("HOME").unwrap_or_default());
    vec![
        home.join(".config/gotv/GOTV.md"),
        root.join("AGENTS.md"),
        root.join("CLAUDE.md"),
        root.join(".gotv/GOTV.md"),
    ]
}

/// El bloque de memoria para el sistema, o `None` si el proyecto no deja nada.
pub fn block(root: &Path) -> Option<String> {
    // F12: las reglas SIN rutas (`.gotv/reglas/`, `.claude/rules/`) valen siempre, como esta memoria; las
    // que tienen rutas NO van aquí: se anuncian al tocar un archivo que casa (`formas::reglas`)
    let mut todos = files(root);
    todos.extend(crate::formas::reglas::siempre(root));
    block_from(&todos)
}

pub fn block_from(paths: &[PathBuf]) -> Option<String> {
    let mut out = String::new();
    let mut used = 0usize;
    for p in paths {
        let Ok(text) = std::fs::read_to_string(p) else {
            continue;
        };
        let text = text.trim();
        if text.is_empty() {
            continue;
        }
        let left = MEMORY_MAX_CHARS.saturating_sub(used);
        if left < 200 {
            out.push_str(&crate::tr!(
                "\n\n(… no cupo {}: léelo con read_file si lo necesitas)",
                "\n\n(… {} did not fit: read it with read_file if you need it)",
                p.display()
            ));
            continue;
        }
        let n = text.chars().count();
        let shown: String = text.chars().take(left).collect();
        out.push_str(&crate::tr!(
            "\n\n# Memoria del proyecto: {}\n",
            "\n\n# Project memory: {}\n",
            p.display()
        ));
        out.push_str(&shown);
        if n > left {
            out.push_str(&crate::tr!(
                "\n(… recortado: {} de {n} caracteres; el resto con read_file)",
                "\n(… truncated: {} of {n} characters; the rest with read_file)",
                left
            ));
        }
        used += shown.chars().count();
    }
    (!out.is_empty()).then(|| {
        let head = crate::i18n::t(
            "\n\nInstrucciones que dejó el proyecto (respétalas; si chocan con una orden del usuario, manda el usuario):",
            "\n\nInstructions left by the project (follow them; if they clash with a user order, the user wins):",
        );
        format!("{head}{out}")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn junta_recorta_y_avisa() {
        let d = crate::CarpetaPrueba::sin_crear("gotv-mem");
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(
            d.join("AGENTS.md"),
            "Usa prefijo zz_ en los archivos de prueba.",
        )
        .unwrap();
        std::fs::write(d.join("CLAUDE.md"), "x".repeat(MEMORY_MAX_CHARS * 2)).unwrap();
        let b = block_from(&[
            d.join("AGENTS.md"),
            d.join("CLAUDE.md"),
            d.join("no-existe.md"),
        ])
        .unwrap();
        assert!(b.contains("zz_"));
        assert!(b.contains("recortado"));
        assert!(b.chars().count() < MEMORY_MAX_CHARS + 1_000);
        assert!(block_from(&[d.join("no-existe.md")]).is_none());
    }
}
