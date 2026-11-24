//! F5 · ML1 · la raíz de un archivo para su servidor: subiendo desde su carpeta hasta la del proyecto,
//! la carpeta más CERCANA con un marcador (un `package.json` del frontend, un `go.mod`) o, para un
//! workspace de Cargo, la más ALTA (`Cargo.toml` de cada crate y del workspace: manda el de arriba). Sin
//! marcador, la del proyecto.
use super::registro::Def;
use std::path::{Path, PathBuf};

pub fn raiz_para(archivo: &Path, d: &Def, root: &Path) -> PathBuf {
    let principal = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let archivo = archivo.canonicalize().unwrap_or_else(|_| archivo.to_path_buf());
    // F8.4 paso 5: un archivo de una carpeta EXTRA del proyecto se acota por ESA carpeta (antes subía sólo
    // dentro de la principal y el servidor arrancaba allí, sin su Cargo.toml ni su package.json)
    let root = if archivo.starts_with(&principal) {
        principal
    } else {
        crate::raices::extras_de(&principal)
            .into_iter()
            .map(|(_, b)| b.canonicalize().unwrap_or(b))
            .find(|b| archivo.starts_with(b))
            .unwrap_or(principal)
    };
    let mut halladas = Vec::new();
    let mut dir = archivo.parent();
    while let Some(x) = dir {
        if !x.starts_with(&root) {
            break;
        }
        if d.raices.iter().any(|m| super::registro::marca(x, m)) {
            halladas.push(x.to_path_buf());
        }
        if x == root {
            break;
        }
        dir = x.parent();
    }
    let elegida = if d.mas_alta { halladas.last() } else { halladas.first() };
    elegida.cloned().unwrap_or(root)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cercana_o_mas_alta() {
        let d = crate::CarpetaPrueba::nueva("gotv-lsp-raiz");
        let r = d.canonicalize().unwrap();
        for (f, t) in [("package.json", "{}"), ("Cargo.toml", "[workspace]"), ("src-tauri/Cargo.toml", "[package]"), ("web/package.json", "{}")] {
            std::fs::create_dir_all(r.join(f).parent().unwrap()).unwrap();
            std::fs::write(r.join(f), t).unwrap();
        }
        for f in ["src-tauri/src/main.rs", "web/src/App.tsx", "scripts/x.ts"] {
            std::fs::create_dir_all(r.join(f).parent().unwrap()).unwrap();
            std::fs::write(r.join(f), "").unwrap();
        }
        let defs = super::super::registro::defs();
        let rust = defs.iter().find(|d| d.id == "rust").unwrap();
        let ts = defs.iter().find(|d| d.id == "typescript").unwrap();
        assert_eq!(raiz_para(&r.join("src-tauri/src/main.rs"), rust, &r), r, "Cargo: la más alta (el workspace)");
        assert_eq!(raiz_para(&r.join("web/src/App.tsx"), ts, &r), r.join("web"), "la más cercana");
        assert_eq!(raiz_para(&r.join("scripts/x.ts"), ts, &r), r, "sube hasta el proyecto");
    }
}
