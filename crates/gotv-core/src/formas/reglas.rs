//! F12 · paso 1 · reglas por ruta. Archivos `.gotv/reglas/*.md` (y `.claude/rules/**/*.md`, los de Claude
//! Code) con una cabecera que dice a qué archivos valen:
//!
//! ```text
//! ---
//! rutas: ["**/*.sql", "db/**"]
//! ---
//! Las migraciones van numeradas y nunca se editan una vez aplicadas.
//! ```
//!
//! También valen `paths:` (Claude Code) y `globs:`, en línea (`["a", "b"]`, `a, b`) o como lista YAML
//! (`- "a"`). Un patrón sin `/` casa con el NOMBRE del archivo en cualquier carpeta (como `.gitignore`);
//! con `/`, con la ruta relativa al proyecto; `{a,b}` se expande.
//!
//! Formato: es un archivo DEL USUARIO en SU proyecto (no un dato de GOTV), y es el que pide la ficha y el
//! que ya escribe quien usa Claude Code: Markdown con una cabecera mínima que Rust lee directo (sin YAML).
//!
//! Cuándo llegan al modelo (L4): las que tienen rutas, NUNCA en el prompt de sistema; al cargar sólo se
//! leen sus cabeceras y, cuando una herramienta toca un archivo que casa, su cuerpo se ANUNCIA en la
//! conversación una sola vez, como `Message::aviso` (F0.10). Las que no dicen rutas valen siempre: van con
//! la memoria del proyecto (`memory.rs`), como `AGENTS.md`.
use gotv_engine::Message;
use std::path::{Path, PathBuf};

/// Lo más que se anuncia de una regla (lo demás, con `read_file`).
pub const REGLA_MAX_CHARS: usize = 4_000;
/// Reglas que caben en un mismo anuncio (si un paso toca muchas a la vez, el resto va en el siguiente).
const POR_ANUNCIO: usize = 3;
/// Dónde viven: la de GOTV y la de Claude Code.
const CARPETAS: [&str; 2] = [".gotv/reglas", ".claude/rules"];

/// Una regla con rutas: su archivo (relativo al proyecto) y sus patrones.
#[derive(Debug, Clone)]
pub struct Regla {
    pub archivo: String,
    pub rutas: Vec<String>,
    patrones: Vec<(glob::Pattern, bool)>,
}

impl Regla {
    /// ¿Vale para `rel` (ruta relativa al proyecto, con `/`)?
    pub fn casa(&self, rel: &str) -> bool {
        let opts = glob::MatchOptions { case_sensitive: true, require_literal_separator: true, require_literal_leading_dot: false };
        let nombre = rel.rsplit('/').next().unwrap_or(rel);
        self.patrones.iter().any(|(p, sin_barra)| p.matches_with(rel, opts) || (*sin_barra && p.matches_with(nombre, opts)))
    }
}

/// Los `.md` de las carpetas de reglas (hasta 3 niveles), ordenados. Vacío —y sin más que dos `stat`—
/// si el proyecto no tiene reglas.
fn archivos(root: &Path) -> Vec<PathBuf> {
    fn baja(d: &Path, n: u8, out: &mut Vec<PathBuf>) {
        if n > 3 {
            return;
        }
        for e in std::fs::read_dir(d).into_iter().flatten().flatten() {
            let p = e.path();
            if p.is_dir() {
                baja(&p, n + 1, out);
            } else if p.extension().is_some_and(|x| x == "md") {
                out.push(p);
            }
        }
    }
    let mut out = Vec::new();
    for c in CARPETAS {
        let d = root.join(c);
        if d.is_dir() {
            let mut v = Vec::new();
            baja(&d, 0, &mut v);
            v.sort();
            out.extend(v);
        }
    }
    out
}

/// `{a,b}` → `a` y `b` (también varias llaves).
fn expandir(p: &str) -> Vec<String> {
    let Some(i) = p.find('{') else { return vec![p.to_string()] };
    let Some(f) = p[i..].find('}').map(|f| i + f) else { return vec![p.to_string()] };
    p[i + 1..f].split(',').flat_map(|alt| expandir(&format!("{}{alt}{}", &p[..i], &p[f + 1..]))).collect()
}

/// Los valores de `clave:` en la cabecera: en línea (`["a", "b"]`, `a, b`) o lista YAML (`- a`).
pub(crate) fn lista(meta: &str, claves: &[&str]) -> Option<Vec<String>> {
    let lineas: Vec<&str> = meta.lines().collect();
    let i = lineas.iter().position(|l| claves.iter().any(|k| l.trim_start().strip_prefix(k).is_some_and(|r| r.trim_start().starts_with(':'))))?;
    let valor = lineas[i].split_once(':').map(|(_, v)| v.trim()).unwrap_or("");
    let limpia = |s: &str| s.trim().trim_matches(|c| c == '"' || c == '\'').trim().to_string();
    let mut v: Vec<String> = if valor.is_empty() {
        lineas[i + 1..].iter().map(|l| l.trim()).take_while(|l| l.starts_with('-') || l.is_empty()).filter_map(|l| l.strip_prefix('-')).map(limpia).collect()
    } else {
        // en línea; un `[` sin cerrar sigue en las líneas de abajo
        let mut t = valor.to_string();
        if t.starts_with('[') && !t.contains(']') {
            for l in &lineas[i + 1..] {
                t.push(' ');
                t.push_str(l.trim());
                if l.contains(']') {
                    break;
                }
            }
        }
        // las comas dentro de `{a,b}` no separan patrones
        let t = t.trim().trim_start_matches('[').trim_end_matches(']');
        let (mut partes, mut actual, mut llaves) = (Vec::new(), String::new(), 0i32);
        for c in t.chars() {
            match c {
                '{' => llaves += 1,
                '}' => llaves -= 1,
                ',' if llaves == 0 => {
                    partes.push(std::mem::take(&mut actual));
                    continue;
                }
                _ => {}
            }
            actual.push(c);
        }
        partes.push(actual);
        partes.iter().map(|s| limpia(s)).collect()
    };
    v.retain(|s| !s.is_empty());
    Some(v)
}

/// La regla que describe `texto`, o `None` si no dice rutas (entonces vale siempre).
fn leer(archivo: String, texto: &str) -> Option<Regla> {
    let (meta, _) = crate::skills::split_front(texto);
    let rutas = lista(meta, &["rutas", "paths", "globs"])?;
    let patrones: Vec<(glob::Pattern, bool)> = rutas
        .iter()
        .flat_map(|r| expandir(r.trim_start_matches("./").trim_start_matches('/')))
        .filter_map(|r| glob::Pattern::new(&r).ok().map(|p| (p, !r.contains('/'))))
        .collect();
    (!patrones.is_empty()).then_some(Regla { archivo, rutas, patrones })
}

fn relativa(root: &Path, p: &Path) -> String {
    p.strip_prefix(root).unwrap_or(p).display().to_string()
}

/// Las cabeceras de las reglas con rutas (se leen al abrir y en cada paso: sólo la cabecera, sin cuerpo).
pub fn cabeceras(root: &Path) -> Vec<Regla> {
    archivos(root).into_iter().filter_map(|p| leer(relativa(root, &p), &std::fs::read_to_string(&p).ok()?)).collect()
}

/// Las reglas SIN rutas: valen siempre y van con la memoria del proyecto (`memory::block`).
pub fn siempre(root: &Path) -> Vec<PathBuf> {
    archivos(root).into_iter().filter(|p| std::fs::read_to_string(p).is_ok_and(|t| leer(String::new(), &t).is_none())).collect()
}

/// La ruta del proyecto (relativa, con `/`) que nombra `s`, si cae dentro de él.
fn del_proyecto(root: &Path, s: &str) -> Option<String> {
    let s = s.trim().trim_matches(|c| c == '"' || c == '\'' || c == '`');
    if s.is_empty() || s.contains('$') || s.contains("://") {
        return None;
    }
    let p = crate::paths::resolve(root, s);
    let canon = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let rel = p.strip_prefix(root).or_else(|_| p.strip_prefix(&canon)).ok()?;
    let rel = rel.to_string_lossy().replace('\\', "/");
    (!rel.is_empty()).then_some(rel)
}

/// Las palabras de una orden de terminal que pueden ser rutas (tienen `.` o `/`).
fn rutas_de_comando(cmd: &str) -> Vec<&str> {
    cmd.split(|c: char| c.is_whitespace() || ";|&<>()=,".contains(c)).filter(|w| !w.starts_with('-') && (w.contains('.') || w.contains('/'))).collect()
}

/// Los archivos del proyecto que TOCARON las herramientas en `messages` (leer, editar, escribir, buscar
/// dentro, o nombrarlos en una orden de terminal), en orden y sin repetir.
pub fn tocados(root: &Path, messages: &[Message]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut anota = |s: &str| {
        if let Some(r) = del_proyecto(root, s) {
            if !out.contains(&r) {
                out.push(r);
            }
        }
    };
    for v in messages.iter().filter_map(|m| m.tool_calls.as_ref()).flatten() {
        let nombre = v["function"]["name"].as_str().unwrap_or("");
        // un patrón de búsqueda (`glob`) no toca ningún archivo
        if nombre == "glob" {
            continue;
        }
        let args: serde_json::Value = match &v["function"]["arguments"] {
            serde_json::Value::String(s) => serde_json::from_str(s).unwrap_or_default(),
            otro => otro.clone(),
        };
        // juez-idioma: datos (nombres de argumentos de las herramientas)
        for k in ["path", "file_path", "file", "ruta", "archivo"] {
            if let Some(s) = args[k].as_str() {
                anota(s);
            }
        }
        for k in ["paths", "files", "archivos"] {
            for s in args[k].as_array().into_iter().flatten().filter_map(|x| x.as_str()) {
                anota(s);
            }
        }
        for e in args["edits"].as_array().into_iter().flatten() {
            if let Some(s) = e["path"].as_str() {
                anota(s);
            }
        }
        if matches!(nombre, "bash" | "process" | "ssh") {
            for k in ["cmd", "command"] {
                for w in rutas_de_comando(args[k].as_str().unwrap_or("")) {
                    anota(w);
                }
            }
        }
    }
    out
}

/// La marca con la que una regla queda anunciada en la conversación (en los dos idiomas: una conversación
/// puede cambiar de idioma a mitad).
fn marcas(archivo: &str) -> [String; 2] {
    // juez-idioma: datos (las dos marcas, ES y EN, se BUSCAN en la conversación; se escriben con la del idioma activo)
    [format!("«{archivo}» · {}", "regla del proyecto"), format!("«{archivo}» · {}", "project rule")]
}

fn ya_anunciada(messages: &[Message], archivo: &str) -> bool {
    let [es, en] = marcas(archivo);
    messages.iter().any(|m| m.content.contains(&es) || m.content.contains(&en))
}

/// El anuncio de las reglas que casan con lo tocado y aún no están en la conversación: `(aviso para el
/// modelo, línea para el usuario)`. `None` sin reglas, sin nada que case o con todo ya anunciado.
pub fn anuncio(root: &Path, messages: &[Message]) -> Option<(String, String)> {
    let reglas = cabeceras(root);
    if reglas.is_empty() {
        return None;
    }
    let tocados = tocados(root, messages);
    let mut partes = Vec::new();
    let mut nombres = Vec::new();
    for r in &reglas {
        if partes.len() >= POR_ANUNCIO || ya_anunciada(messages, &r.archivo) {
            continue;
        }
        let Some(quien) = tocados.iter().find(|t| r.casa(t)) else { continue };
        let Ok(texto) = std::fs::read_to_string(root.join(&r.archivo)) else { continue };
        let (_, cuerpo) = crate::skills::split_front(&texto);
        let cuerpo = cuerpo.trim();
        let n = cuerpo.chars().count();
        let mut c: String = cuerpo.chars().take(REGLA_MAX_CHARS).collect();
        if n > REGLA_MAX_CHARS {
            c.push_str(&crate::tr!("\n(… recortada: el resto con read_file en {})", "\n(… truncated: the rest with read_file at {})", r.archivo));
        }
        let ms = marcas(&r.archivo);
        let marca = &ms[usize::from(crate::i18n::is_en())];
        partes.push(crate::tr!(
            "{marca} (vale para {}; la activó {quien}). Respétala en todo lo que hagas con esos archivos:\n{c}",
            "{marca} (applies to {}; triggered by {quien}). Follow it in everything you do with those files:\n{c}",
            r.rutas.join(", ")
        ));
        nombres.push(r.archivo.clone());
    }
    if partes.is_empty() {
        return None;
    }
    let aviso = format!("GOTV · {}", partes.join("\n\n"));
    let linea = crate::tr!("· 📏 regla del proyecto: {}", "· 📏 project rule: {}", nombres.join(", "));
    Some((aviso, linea))
}

/// `/reglas`: qué reglas tiene el proyecto y a qué archivos valen.
pub fn listado(root: &Path) -> String {
    let con = cabeceras(root);
    let sin = siempre(root);
    if con.is_empty() && sin.is_empty() {
        return crate::i18n::t(
            "Este proyecto no tiene reglas. Crea `.gotv/reglas/<nombre>.md` con la cabecera `rutas: [\"**/*.sql\"]` (o usa `.claude/rules/`): se anuncian al tocar un archivo que casa.",
            "This project has no rules. Create `.gotv/reglas/<name>.md` with the header `rutas: [\"**/*.sql\"]` (or use `.claude/rules/`): they are announced when a matching file is touched.",
        )
        .to_string();
    }
    let mut s = String::new();
    for r in &con {
        s.push_str(&crate::tr!("\n- {} → al tocar {}", "\n- {} → when touching {}", r.archivo, r.rutas.join(", ")));
    }
    for p in &sin {
        s.push_str(&crate::tr!("\n- {} → siempre (con la memoria del proyecto)", "\n- {} → always (with the project memory)", relativa(root, p)));
    }
    crate::tr!("Reglas del proyecto:{s}", "Project rules:{s}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use gotv_engine::ToolCall;

    fn llamada(nombre: &str, args: serde_json::Value) -> Message {
        Message::assistant_calls("", &[ToolCall { id: "c1".into(), name: nombre.into(), arguments: args }])
    }

    fn proyecto(tag: &str) -> crate::CarpetaPrueba {
        let d = crate::CarpetaPrueba::nueva(&format!("gotv-reglas-{tag}"));
        std::fs::create_dir_all(d.join(".gotv/reglas")).unwrap();
        std::fs::write(d.join(".gotv/reglas/sql.md"), "---\nrutas: [\"**/*.sql\"]\n---\nLas migraciones van numeradas: 0001_x.sql. Nunca edites una aplicada.\n").unwrap();
        d
    }

    #[test]
    fn cabeceras_en_linea_en_lista_y_de_claude_code() {
        let d = proyecto("cab");
        std::fs::create_dir_all(d.join(".claude/rules/front")).unwrap();
        std::fs::write(d.join(".claude/rules/front/ts.md"), "---\npaths:\n  - \"src/**/*.{ts,tsx}\"\n  - 'lib/*.ts'\n---\nUsa tipos estrictos.\n").unwrap();
        std::fs::write(d.join(".gotv/reglas/varias.md"), "---\nglobs: a/*.rs, b/**\n---\nx\n").unwrap();
        std::fs::write(d.join(".gotv/reglas/general.md"), "Responde siempre en español.\n").unwrap();
        let r = cabeceras(&d);
        let nombres: Vec<&str> = r.iter().map(|x| x.archivo.as_str()).collect();
        assert_eq!(nombres, [".gotv/reglas/sql.md", ".gotv/reglas/varias.md", ".claude/rules/front/ts.md"]);
        let ts = &r[2];
        assert!(ts.casa("src/ui/app.tsx") && ts.casa("src/a.ts") && ts.casa("lib/x.ts"));
        assert!(!ts.casa("lib/deep/x.ts") && !ts.casa("src/a.js"));
        let sql = &r[0];
        assert!(sql.casa("schema.sql") && sql.casa("db/migraciones/0001_x.sql"));
        assert!(!sql.casa("schema.sql.bak") && !sql.casa("src/main.rs"));
        assert!(r[1].casa("a/x.rs") && r[1].casa("b/c/d.txt") && !r[1].casa("a/b/x.rs"));
        // la que no dice rutas vale siempre: va con la memoria, no con los anuncios
        assert_eq!(siempre(&d), vec![d.join(".gotv/reglas/general.md")]);
        assert_eq!(expandir("x.{a,b}/{c,d}"), ["x.a/c", "x.a/d", "x.b/c", "x.b/d"]);
    }

    #[test]
    fn la_regla_de_sql_aparece_al_tocar_un_sql_y_una_sola_vez() {
        let d = proyecto("sql");
        std::fs::create_dir_all(d.join("db")).unwrap();
        let mut m = vec![Message::system("s"), Message::user("arregla el login")];
        // tocar código que no es SQL: nada
        m.push(llamada("read_file", serde_json::json!({"path": "src/login.rs"})));
        m.push(llamada("bash", serde_json::json!({"cmd": "cargo test login"})));
        m.push(llamada("glob", serde_json::json!({"pattern": "**/*.sql"})));
        assert!(anuncio(&d, &m).is_none(), "ni el código ni un glob de *.sql tocan un .sql");
        // tocar un .sql (por la terminal, con ruta absoluta): se anuncia
        m.push(llamada("bash", serde_json::json!({"cmd": format!("sqlite3 app.db < {}/db/schema.sql", d.display())})));
        let (aviso, linea) = anuncio(&d, &m).expect("tocó un .sql");
        assert!(aviso.contains("Nunca edites una aplicada") && aviso.contains("db/schema.sql"), "{aviso}");
        assert!(linea.contains(".gotv/reglas/sql.md"));
        m.push(Message::aviso(aviso));
        // otro .sql (con edit, por argumentos en texto como los guarda el historial): ya está en la conversación
        m.push(llamada("edit", serde_json::json!({"path": "db/0002_y.sql", "old": "a", "new": "b"})));
        assert!(anuncio(&d, &m).is_none(), "una sola vez");
        // en la otra lengua la marca también se reconoce
        let [_, en] = marcas(".gotv/reglas/sql.md");
        let mut m2 = vec![llamada("write_file", serde_json::json!({"path": "q.sql", "content": ""}))];
        m2.push(Message::aviso(format!("GOTV · {en} …")));
        assert!(anuncio(&d, &m2).is_none());
    }

    #[test]
    fn rutas_de_ediciones_multiples_y_listas() {
        let d = proyecto("listas");
        let m = vec![
            llamada("multi_edit", serde_json::json!({"path": "a.rs", "edits": [{"path": "b.rs", "old": "x", "new": "y"}]})),
            llamada("lsp", serde_json::json!({"paths": ["c.rs", "../fuera.rs"]})),
        ];
        assert_eq!(tocados(&d, &m), ["a.rs", "b.rs", "c.rs"], "lo de fuera del proyecto no cuenta");
    }

    /// Las de siempre van con la memoria (sistema, estable por proyecto); las de ruta NUNCA en el sistema.
    #[test]
    fn las_reglas_sin_rutas_van_con_la_memoria_y_las_de_ruta_no() {
        let d = proyecto("memoria");
        let sin = crate::memory::block(&d).unwrap_or_default();
        assert!(!sin.contains("Nunca edites una aplicada"), "la regla de ruta no va al sistema");
        std::fs::write(d.join(".gotv/reglas/general.md"), "Responde siempre en español.\n").unwrap();
        let con = crate::memory::block(&d).unwrap();
        assert!(con.contains("Responde siempre en español") && !con.contains("Nunca edites una aplicada"), "{con}");
    }

    #[test]
    fn sin_carpeta_de_reglas_no_cuesta_nada() {
        let d = crate::CarpetaPrueba::nueva("gotv-reglas-vacio");
        let m = vec![llamada("read_file", serde_json::json!({"path": "x.sql"}))];
        assert!(cabeceras(&d).is_empty() && siempre(&d).is_empty() && anuncio(&d, &m).is_none());
        assert!(listado(&d).contains(".gotv/reglas"));
    }
}
