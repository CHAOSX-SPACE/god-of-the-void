//! F5 · ML2 · la COMPUERTA del turno: al cerrar un turno que tocó código, la verificación COMPLETA de cada
//! lenguaje tocado (no sólo el archivo editado: lo que se rompió en otro lado por una firma cambiada).
//! - Rust: `cargo check --workspace` en la raíz del workspace (con `--release` si el proyecto ya compila
//!   así y no tiene `target/debug`: en el repo de GOTV rige «sólo release», y no se crea otra carpeta).
//! - TypeScript: el `tsc` DEL PROYECTO (`node_modules/.bin/tsc --noEmit`), nunca uno global.
//! - Go: `go vet ./...`. PHP: `php -l` de cada archivo tocado. Python: `py_compile` de cada archivo tocado.
//! Corre dentro de la caja de la sesión (como `bash`), con tope de tiempo; lo que no está instalado se
//! salta en silencio. Devuelve los errores (recortados) o `None` si todo pasó.
use crate::lsp::{raiz::raiz_para, registro};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Una verificación: dónde y qué orden.
#[derive(Debug, Clone, PartialEq)]
pub struct Chequeo {
    pub lenguaje: &'static str,
    pub carpeta: PathBuf,
    pub orden: String,
}

fn comillas(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// Las verificaciones para los archivos tocados (una por lenguaje y raíz).
pub fn chequeos(root: &Path, tocados: &[PathBuf]) -> Vec<Chequeo> {
    let defs = registro::defs();
    let mut por_raiz: BTreeMap<(&'static str, PathBuf), Vec<PathBuf>> = BTreeMap::new();
    for t in tocados {
        if let Some(d) = registro::para(&defs, t) {
            por_raiz.entry((d.id, raiz_para(t, d, root))).or_default().push(t.clone());
        }
    }
    let mut out = Vec::new();
    for ((id, raiz), archivos) in por_raiz {
        let lista = archivos.iter().map(|a| comillas(&a.display().to_string())).collect::<Vec<_>>().join(" ");
        let c = match id {
            "rust" if raiz.join("Cargo.toml").is_file() => {
                let release = raiz.join("target/release").is_dir() && !raiz.join("target/debug").is_dir();
                // ML1 (a): la MISMA carpeta propia que rust-analyzer (`target/rust-analyzer`): no bloquea ni mezcla con
                // la compilación del usuario; en un proyecto «sólo release», `target/release`
                Some(format!("cargo check --workspace --message-format short --quiet{}", if release { " --release" } else { " --target-dir target/rust-analyzer" }))
            }
            "typescript" if raiz.join("node_modules/.bin/tsc").is_file() && raiz.join("tsconfig.json").is_file() => Some("node_modules/.bin/tsc --noEmit --pretty false".to_string()),
            "go" if raiz.join("go.mod").is_file() => Some("go vet ./...".to_string()),
            "php" if crate::entorno::en_path("php") => Some(format!("for f in {lista}; do php -l \"$f\" >/dev/null || php -l \"$f\"; done")),
            // el intérprete DEL PROYECTO si tiene `.venv` (nivel 3 de L1: GOTV no exige uno propio)
            "python" => {
                let py = raiz.join(".venv/bin/python");
                let py = if py.is_file() { comillas(&py.display().to_string()) } else { "python3".to_string() };
                Some(format!("{py} -m py_compile {lista}"))
            }
            _ => None,
        };
        if let Some(orden) = c {
            out.push(Chequeo { lenguaje: registro::defs().into_iter().find(|d| d.id == id).map(|d| d.lang).unwrap_or(id), carpeta: raiz, orden });
        }
    }
    out
}

/// Una orden de shell dentro de la caja de la sesión (como `bash`, con su red por el proxy) si hay `spec`;
/// si no, `bash -lc`. Desde `carpeta`, sin entrada, y se mata si se suelta. (F15.1 la usa para las pruebas.)
pub fn en_caja(spec: Option<&crate::sandbox::Spec>, orden: &str, carpeta: &Path) -> tokio::process::Command {
    let mut k = match spec.filter(|_| crate::sandbox::available()) {
        Some(sp) => {
            let mut k = crate::sandbox::command(sp, orden);
            let g = crate::netproxy::grant(crate::sandbox::hosts(sp), sp.all_net);
            if let Ok(p) = crate::netproxy::ensure() {
                for (a, b) in crate::netproxy::env_for(p, &g) {
                    k.env(a, b);
                }
            }
            k
        }
        None => {
            let mut k = tokio::process::Command::new("bash");
            k.arg("-lc").arg(orden);
            k
        }
    };
    k.current_dir(carpeta).env("PATH", crate::entorno::path()).stdin(std::process::Stdio::null()).kill_on_drop(true);
    k
}

/// Corre las verificaciones (en la caja si hay `spec`); los errores, o `None` si todo pasó.
pub async fn verificar(root: &Path, tocados: &[PathBuf], spec: Option<crate::sandbox::Spec>, tope: Duration) -> Option<String> {
    let mut fallos = Vec::new();
    for c in chequeos(root, tocados) {
        let mut k = en_caja(spec.as_ref(), &c.orden, &c.carpeta);
        let Ok(Ok(o)) = tokio::time::timeout(tope, k.output()).await else { continue };
        if o.status.success() {
            continue;
        }
        let texto = format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr));
        // lo que dice «error» primero (los avisos no frenan el cierre), 40 líneas como mucho
        let mut lineas: Vec<&str> = texto.lines().filter(|l| l.to_lowercase().contains("error")).collect();
        if lineas.is_empty() {
            lineas = texto.lines().collect();
        }
        let mas = lineas.len().saturating_sub(40);
        let mut bloque = format!("{} · {}\n{}", c.lenguaje, c.orden.split_whitespace().take(3).collect::<Vec<_>>().join(" "), lineas.into_iter().take(40).collect::<Vec<_>>().join("\n"));
        if mas > 0 {
            bloque.push_str(&crate::tr!("\n… y {mas} líneas más", "\n… and {mas} more lines"));
        }
        fallos.push(bloque);
    }
    (!fallos.is_empty()).then(|| fallos.join("\n\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn un_chequeo_por_lenguaje_y_raiz() {
        let d = crate::CarpetaPrueba::nueva("gotv-compuerta-planes");
        let r = d.canonicalize().unwrap();
        for f in ["Cargo.toml", "src/main.rs", "src/otro.rs", "web/package.json", "web/tsconfig.json", "web/node_modules/.bin/tsc", "web/src/a.ts", "s.py"] {
            std::fs::create_dir_all(r.join(f).parent().unwrap()).unwrap();
            std::fs::write(r.join(f), "").unwrap();
        }
        std::fs::create_dir_all(r.join("target/release")).unwrap();
        let c = chequeos(&r, &[r.join("src/main.rs"), r.join("src/otro.rs"), r.join("web/src/a.ts"), r.join("s.py"), r.join("README.md")]);
        assert_eq!(c.len(), 3, "{c:?}");
        let rust = c.iter().find(|x| x.lenguaje == "rust").unwrap();
        assert!(rust.orden.contains("cargo check --workspace") && rust.orden.ends_with("--release"), "sólo release si el proyecto ya es así: {}", rust.orden);
        assert_eq!(c.iter().find(|x| x.lenguaje == "typescript").unwrap().carpeta, r.join("web"));
        assert!(c.iter().any(|x| x.lenguaje == "python" && x.orden.contains("s.py")));
    }

    /// De verdad: un proyecto de Rust con un error de tipos en un archivo que NO se tocó (la firma cambió en
    /// otro): la compuerta lo ve; arreglado, pasa.
    #[tokio::test]
    async fn la_compuerta_ve_lo_que_se_rompio_en_otro_archivo() {
        if !crate::entorno::en_path("cargo") {
            return;
        }
        let d = crate::CarpetaPrueba::nueva("gotv-compuerta-rust");
        let r = d.canonicalize().unwrap();
        std::fs::create_dir_all(r.join("src")).unwrap();
        std::fs::write(r.join("Cargo.toml"), "[package]\nname = \"juguete\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[workspace]\n").unwrap();
        std::fs::write(r.join("src/lib.rs"), "pub mod usa;\npub fn suma(a: i32, b: i32, c: i32) -> i32 { a + b + c }\n").unwrap();
        std::fs::write(r.join("src/usa.rs"), "pub fn doble(x: i32) -> i32 { crate::suma(x, x) }\n").unwrap();
        let e = verificar(&r, &[r.join("src/lib.rs")], None, Duration::from_secs(120)).await.expect("debe fallar");
        assert!(e.contains("usa.rs") && e.contains("error"), "{e}");
        std::fs::write(r.join("src/usa.rs"), "pub fn doble(x: i32) -> i32 { crate::suma(x, x, 0) }\n").unwrap();
        assert!(verificar(&r, &[r.join("src/lib.rs")], None, Duration::from_secs(120)).await.is_none());
    }
}
