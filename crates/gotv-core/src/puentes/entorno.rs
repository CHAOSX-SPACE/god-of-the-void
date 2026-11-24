//! F5 ML9 · el puente del ENTORNO: las variables que lee el código ↔ las que documenta `.env.example`
//! (o `.env.sample`, `.env.template`). Una variable que el código lee y el ejemplo no trae es la que
//! falta al desplegar o al clonar el proyecto. Se leen por la forma de la llamada de cada lenguaje:
//! `process.env.X`, `import.meta.env.X`, `std::env::var("X")`, `env!("X")`, `os.environ["X"]`,
//! `os.getenv("X")`, `getenv('X')`, `env('X')` (Laravel), `os.Getenv("X")`. Sólo si hay ejemplo.
use super::{Desajuste, Fuente};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// Las del sistema o las que pone la herramienta (no van en un `.env`).
fn del_sistema(v: &str) -> bool {
    matches!(v, "PATH" | "HOME" | "USER" | "PWD" | "SHELL" | "TMPDIR" | "TERM" | "LANG" | "CI" | "NODE_ENV" | "MODE" | "DEV" | "PROD" | "SSR" | "BASE_URL" | "OUT_DIR" | "TARGET" | "PROFILE")
        || v.starts_with("CARGO_")
        || v.starts_with("RUST_")
        || v.starts_with("TAURI_")
}

fn es_nombre(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// Las variables que lee un texto, con su línea (la primera vez).
pub(crate) fn leidas(texto: &str) -> BTreeMap<String, usize> {
    // (inicio, ¿entre comillas?)
    const FORMAS: &[(&str, bool)] = &[
        ("process.env.", false),
        ("import.meta.env.", false),
        ("env::var(\"", true),
        ("env::var_os(\"", true),
        ("env!(\"", true),
        ("os.environ[\"", true),
        ("os.environ['", true),
        ("os.environ.get(\"", true),
        ("os.environ.get('", true),
        ("os.getenv(\"", true),
        ("os.getenv('", true),
        ("os.Getenv(\"", true),
        ("getenv('", true),
        ("getenv(\"", true),
        ("env('", true),
        ("env(\"", true),
    ];
    let mut out = BTreeMap::new();
    for (i, l) in texto.lines().enumerate() {
        let sin_espacio = l.trim_start();
        if sin_espacio.starts_with("//") || sin_espacio.starts_with('#') || sin_espacio.starts_with('*') {
            continue;
        }
        for (forma, _) in FORMAS {
            let mut desde = 0;
            while let Some(j) = l[desde..].find(forma) {
                let ini = desde + j;
                desde = ini + forma.len();
                // una forma corta no vale DENTRO de otro nombre (`dotenv(`, `getenv(` de `os.getenv(`, que
                // cuenta su propia forma); `std::env::var` y `option_env!` sí
                let previo = l[..ini].chars().next_back();
                let vale = match *forma {
                    "env('" | "env(\"" => !previo.is_some_and(|c| es_nombre(c) || matches!(c, '.' | ':' | '>')),
                    "env!(\"" => !previo.is_some_and(|c| es_nombre(c) && c != '_'),
                    "getenv('" | "getenv(\"" => !previo.is_some_and(|c| es_nombre(c) || c == '.'),
                    f if f.starts_with("env::") => !previo.is_some_and(es_nombre),
                    _ => true,
                };
                if !vale {
                    continue;
                }
                let nombre: String = l[desde..].chars().take_while(|c| es_nombre(*c)).collect();
                if nombre.len() >= 2 && nombre.chars().next().is_some_and(|c| c.is_ascii_uppercase() || c == '_') {
                    out.entry(nombre).or_insert(i + 1);
                }
            }
        }
    }
    out
}

/// Las variables que documenta el ejemplo del proyecto; `None` si no hay ejemplo.
fn del_ejemplo(root: &Path) -> Option<BTreeSet<String>> {
    let t = [".env.example", ".env.sample", ".env.template", ".env.dist"].iter().find_map(|n| std::fs::read_to_string(root.join(n)).ok())?;
    Some(
        t.lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .filter_map(|l| l.trim_start_matches("export ").split('=').next())
            .map(|k| k.trim().to_string())
            .filter(|k| !k.is_empty())
            .collect(),
    )
}

pub(crate) fn revisar(root: &Path, fuentes: &[Fuente]) -> Vec<Desajuste> {
    let Some(ejemplo) = del_ejemplo(root) else { return vec![] };
    let mut v = Vec::new();
    let mut vistas = BTreeSet::new();
    for f in fuentes {
        for (var, linea) in leidas(&f.texto) {
            if del_sistema(&var) || ejemplo.contains(&var) || !vistas.insert(var.clone()) {
                continue;
            }
            v.push(Desajuste {
                puente: "entorno".into(),
                clave: format!("sin-ejemplo|{var}"), // juez-idioma: datos (clave interna estable)
                texto: crate::tr!(
                    "{}:{linea}: lee la variable de entorno «{var}», que `.env.example` no documenta (quien clone o despliegue no sabrá ponerla)",
                    "{}:{linea}: reads the environment variable «{var}», which `.env.example` does not document (whoever clones or deploys will not know to set it)",
                    f.rel
                ),
            });
        }
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn las_formas_de_cada_lenguaje() {
        let t = "const a = process.env.API_URL;\nconst b = import.meta.env.VITE_CLAVE;\nlet c = std::env::var(\"DATABASE_URL\");\nlet d = env!(\"CARGO_PKG_NAME\");\nx = os.environ[\"SECRETO\"]\ny = os.getenv('PUERTO')\n$z = getenv('PHP_VAR');\n$w = env('APP_KEY');\nload_dotenv('x')\n// process.env.COMENTADA\nv := os.Getenv(\"GO_VAR\")\n";
        let l: Vec<String> = leidas(t).into_keys().collect();
        assert_eq!(l, ["API_URL", "APP_KEY", "CARGO_PKG_NAME", "DATABASE_URL", "GO_VAR", "PHP_VAR", "PUERTO", "SECRETO", "VITE_CLAVE"]);
    }

    #[test]
    fn contra_el_ejemplo() {
        let d = crate::CarpetaPrueba::nueva("gotv-puentes-entorno");
        let f = Fuente { rel: "src/cfg.ts".into(), texto: "export const url = process.env.API_URL;\nexport const k = process.env.NUEVA;\nconst m = process.env.NODE_ENV;\n".into(), lenguaje: gotv_sintaxis::Lenguaje::TypeScript };
        assert!(revisar(&d, std::slice::from_ref(&f)).is_empty(), "sin ejemplo no se juzga");
        std::fs::write(d.join(".env.example"), "# api\nAPI_URL=http://localhost\nexport OTRA=1\n").unwrap();
        let v = revisar(&d, &[f]);
        assert_eq!(v.len(), 1, "{v:?}");
        assert_eq!(v[0].clave, "sin-ejemplo|NUEVA");
        assert!(v[0].texto.contains("src/cfg.ts:2"));
    }
}
