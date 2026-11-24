//! Skills compatibles con Claude Code (X1): carpetas con `SKILL.md` (cabecera YAML
//! con `name` y `description`, cuerpo en Markdown).
//!
//! Carga SELECTIVA, pensada para modelos locales con prellenado lento (el 32b lee a
//! 62 tok/s: el índice entero de 32 skills costaría ~20 s por orden):
//! - con cada orden viajan sólo las ≤5 skills RELEVANTES a esa tarea (nombre +
//!   descripción recortada), en el mensaje del usuario — no en el sistema, para no
//!   romper la caché de prefijo;
//! - el cuerpo entero llega sólo cuando el modelo llama `skill(name)`;
//! - `skill("?")` lista todas.
//!
//! Dónde se buscan (gana la primera con el mismo nombre): `.gotv/skills` y
//! `.claude/skills` del proyecto, `~/.config/gotv/skills`, `~/.claude/skills`.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq)]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub dir: PathBuf,
    /// K4 · palabras que bastan SOLAS para sugerirla (`disparadores:` en el encabezado):
    /// una orden de voz corta («¿cuánta batería tengo?») no junta 2 coincidencias
    pub disparadores: Vec<String>,
}

/// Skills que viajan DENTRO de GOTV (W6): se escriben en su caché la primera vez (o si
/// cambiaron) y se buscan al final (las del usuario con el mismo nombre ganan).
const BUILTIN: &[(&str, &str)] = &[
    ("navegador", include_str!("../skills/navegador/SKILL.md")),
    // O2 · adaptada de la skill `xlsx` de Claude Code (Apache-2.0; ver su encabezado)
    ("excel", include_str!("../skills/excel/SKILL.md")),
    // adaptada de la skill `docx` de Claude Code (Apache-2.0; ver su encabezado)
    ("word", include_str!("../skills/word/SKILL.md")),
    // adaptada de la skill `pptx` de Claude Code (Apache-2.0; ver su encabezado)
    ("powerpoint", include_str!("../skills/powerpoint/SKILL.md")),
    // adaptada de la skill `pdf` de Claude Code (Apache-2.0; ver su encabezado)
    ("pdf", include_str!("../skills/pdf/SKILL.md")),
    // K4 · el sistema por comando (recetas medidas en esta Mac)
    ("sistema", include_str!("../skills/sistema/SKILL.md")),
    // F11 · las guías de los paquetes de lenguaje (se cargan sólo cuando el modelo las pide)
    ("lenguaje-rust", include_str!("../paquetes/rust/guia.md")),
    ("lenguaje-node", include_str!("../paquetes/javascript/guia.md")),
    ("lenguaje-python", include_str!("../paquetes/python/guia.md")),
    ("lenguaje-php", include_str!("../paquetes/php/guia.md")),
    ("lenguaje-swift", include_str!("../paquetes/swift/guia.md")),
    ("lenguaje-go", include_str!("../paquetes/go/guia.md")),
];

fn builtin_dir() -> PathBuf {
    let home = PathBuf::from(std::env::var("HOME").unwrap_or_default());
    let d = home.join(".cache/gotv/skills");
    // F12: `entrevista` y `aprender` viven en Rust (constantes en los dos idiomas)
    // F9: y el coordinador de varios subagentes (`enjambre`)
    for (name, text) in BUILTIN.iter().copied().chain(crate::formas::incluidas::skills()).chain(crate::enjambre::coordinador::skills()) {
        let f = d.join(name).join("SKILL.md");
        if std::fs::read_to_string(&f).ok().as_deref() != Some(text) {
            let _ = std::fs::create_dir_all(d.join(name));
            let _ = std::fs::write(&f, text);
        }
    }
    d
}

pub fn dirs(root: &Path) -> Vec<PathBuf> {
    let home = PathBuf::from(std::env::var("HOME").unwrap_or_default());
    // `GOTV_SKILLS_USUARIO=0`: sólo las del proyecto y las propias de GOTV (bancos y exámenes: el
    // rival corre con HOME limpio; medido D11 — el 9B cargó «escritura-humana» del Portador)
    let usuario = std::env::var("GOTV_SKILLS_USUARIO").map(|v| v != "0").unwrap_or(true);
    let mut v = vec![root.join(".gotv/skills"), root.join(".claude/skills")];
    if usuario {
        v.push(home.join(".config/gotv/skills"));
        v.push(home.join(".claude/skills"));
        // F10.4: las de los plugins confiados
        v.extend(crate::plugins::carpetas_de_skills());
    }
    v.push(builtin_dir());
    v
}

/// Todas las skills visibles desde `root`, sin repetir nombre.
pub fn discover(root: &Path) -> Vec<Skill> {
    discover_in(&dirs(root))
}

pub fn discover_in(dirs: &[PathBuf]) -> Vec<Skill> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for d in dirs {
        let Ok(rd) = std::fs::read_dir(d) else {
            continue;
        };
        let mut entries: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
        entries.sort();
        for dir in entries {
            let file = dir.join("SKILL.md");
            let Ok(text) = std::fs::read_to_string(&file) else {
                continue;
            };
            let (meta, _) = split_front(&text);
            let fallback = dir
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            let name = field(meta, "name")
                .filter(|n| !n.is_empty())
                .unwrap_or(fallback);
            if name.starts_with('.') || !seen.insert(name.clone()) {
                continue;
            }
            out.push(Skill {
                name,
                description: field(meta, "description").unwrap_or_default(),
                disparadores: field(meta, "disparadores").map(|d| d.split(',').map(|w| w.trim().to_lowercase()).filter(|w| !w.is_empty()).collect()).unwrap_or_default(),
                dir,
            });
        }
    }
    out
}

/// Separa la cabecera `---…---` del cuerpo.
pub fn split_front(text: &str) -> (&str, &str) {
    let t = text.trim_start_matches('\u{feff}');
    if let Some(rest) = t.strip_prefix("---") {
        if let Some(end) = rest.find("\n---") {
            let meta = &rest[..end];
            let body = rest[end + 4..]
                .trim_start_matches(['-', '\r'])
                .trim_start_matches('\n');
            return (meta, body);
        }
    }
    ("", t)
}

/// Lee `clave: valor` de una cabecera YAML simple, con bloques `>`/`|` y comillas.
fn field(meta: &str, key: &str) -> Option<String> {
    let lines: Vec<&str> = meta.lines().collect();
    let i = lines
        .iter()
        .position(|l| l.starts_with(&format!("{key}:")))?;
    let first = lines[i][key.len() + 1..].trim();
    let folded = first.is_empty() || first.starts_with('>') || first.starts_with('|');
    if !folded {
        return Some(first.trim_matches(|c| c == '"' || c == '\'').to_string());
    }
    let mut parts = Vec::new();
    for l in &lines[i + 1..] {
        if l.starts_with(' ') || l.starts_with('\t') || l.trim().is_empty() {
            parts.push(l.trim());
        } else {
            break;
        }
    }
    Some(
        parts
            .join(" ")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" "),
    )
}

/// Palabras útiles: minúsculas, sin tildes, ≥ 4 letras, sin palabras vacías.
/// Todas las palabras (minúsculas, sin tildes), también las cortas: para los disparadores.
fn tokens(s: &str) -> Vec<String> {
    s.to_lowercase()
        .chars()
        .map(|c| match c {
            'á' | 'à' | 'ä' => 'a',
            'é' | 'è' | 'ë' => 'e',
            'í' | 'ì' | 'ï' => 'i',
            'ó' | 'ò' | 'ö' => 'o',
            'ú' | 'ù' | 'ü' => 'u',
            c if c.is_alphanumeric() => c,
            _ => ' ',
        })
        .collect::<String>()
        .split_whitespace()
        .map(String::from)
        .collect()
}

fn words(s: &str) -> Vec<String> {
    // juez-idioma: datos — palabras vacías de AMBOS idiomas — bloque
    const STOP: &[&str] = &[
        "para", "como", "esta", "este", "esto", "sobre", "cuando", "donde", "pero", "porque",
        "with", "when", "that", "this", "from", "your", "into", "about", "usar", "use", "the",
        "and", "una", "unos", "unas", "todo", "toda", "cada", "also", "user", "usuario", "skill",
        "skills", "archivo", "file", "hace", "haga", "quiero", "puedes", "please", "favor",
    ];
    let norm: String = s
        .to_lowercase()
        .chars()
        .map(|c| match c {
            'á' | 'à' | 'ä' => 'a',
            'é' | 'è' | 'ë' => 'e',
            'í' | 'ì' | 'ï' => 'i',
            'ó' | 'ò' | 'ö' => 'o',
            'ú' | 'ù' | 'ü' => 'u',
            c if c.is_alphanumeric() => c,
            _ => ' ',
        })
        .collect();
    norm.split_whitespace()
        .filter(|w| w.chars().count() >= 4 && !STOP.contains(w))
        .map(String::from)
        .collect()
}

/// Las `k` skills más relevantes para la tarea. Puntaje: palabras de la tarea que
/// aparecen (por raíz de 5 letras) en el nombre (×3) o en la descripción.
pub fn relevant<'a>(task: &str, skills: &'a [Skill], k: usize) -> Vec<&'a Skill> {
    let tw: Vec<String> = words(task);
    let tt: Vec<String> = tokens(task);
    if tw.is_empty() && tt.is_empty() {
        return vec![];
    }
    let stem = |w: &str| w.chars().take(5).collect::<String>();
    let task_stems: HashSet<String> = tw.iter().map(|w| stem(w)).collect();
    let mut scored: Vec<(usize, &Skill)> = skills
        .iter()
        // F12: las que sólo se piden con su orden de barra no se sugieren por palabras (contexto 0 sin usarlas)
        .filter(|s| !crate::formas::incluidas::SOLO_CON_BARRA.contains(&s.name.as_str()))
        .map(|s| {
            let name_hits = words(&s.name.replace(['-', '_'], " "))
                .iter()
                .filter(|w| task_stems.contains(&stem(w)))
                .count();
            let desc: HashSet<String> = words(&s.description).iter().map(|w| stem(w)).collect();
            let desc_hits = task_stems.iter().filter(|t| desc.contains(*t)).count();
            // K4: un disparador (palabra entera, sin tildes; también cortas como «ip» o «cpu») basta solo
            let disparo = disparada(task, s);
            (name_hits * 3 + desc_hits + if disparo { 2 } else { 0 }, s)
        })
        .filter(|(score, _)| *score >= 2)
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.name.cmp(&b.1.name)));
    // Medido con las 32 skills reales: «IVA y retención en la fuente… persona natural»
    // traía también `chaos` y `escritura-humana` por DOS palabras sueltas. Sólo viaja
    // lo que SUPERA la mitad del puntaje de la mejor (2 de 4 no basta: eran raíces
    // sueltas, «personalidad» ≈ «persona», «naturalizar» ≈ «natural»).
    let best = scored.first().map(|(s, _)| *s).unwrap_or(0);
    scored
        .into_iter()
        .filter(|(sc, _)| sc * 2 > best || *sc == best)
        .take(k)
        .map(|(_, s)| s)
        .collect()
}

/// La nota que acompaña a la orden: qué skills hay a mano para ESTA tarea.
/// K4 · ¿la orden dispara la skill por uno de sus `disparadores`?
pub fn disparada(task: &str, s: &Skill) -> bool {
    let tt = tokens(task);
    s.disparadores.iter().any(|d| {
        let dt = tokens(d);
        !dt.is_empty() && dt.iter().all(|w| tt.contains(w))
    })
}

pub fn note(task: &str, root: &Path) -> Option<String> {
    let all = discover(root);
    let rel = relevant(task, &all, 5);
    if rel.is_empty() {
        return None;
    }
    // K4 · una skill DISPARADA viaja con su contenido, no sólo con su nombre. Medido (banco K4,
    // 9B): con sólo la sugerencia, «¿a cuánto está el volumen?» no la cargó, improvisó 9 pasos e
    // inventó «80» (era 44)
    if let Some(k) = rel.iter().find(|k| disparada(task, k)) {
        if let Ok(texto) = std::fs::read_to_string(k.dir.join("SKILL.md")) {
            let (_, cuerpo) = split_front(&texto);
            let cuerpo: String = cuerpo.trim().chars().take(4000).collect();
            return Some(crate::tr!(
                "(Guía «{}» — viene incluida porque la orden la pide; síguela:\n{cuerpo})",
                "(Guide «{}» — included because the order calls for it; follow it:\n{cuerpo})",
                k.name
            ));
        }
    }
    let mut s = crate::tr!(
        "(Skills que pueden servir para esta tarea — carga la que uses con la herramienta `skill`; hay {} en total, `skill` con name \"?\" las lista:",
        "(Skills that may help with this task — load the one you use with the `skill` tool; there are {} in total, `skill` with name \"?\" lists them:",
        all.len()
    );
    for k in rel {
        let d: String = k.description.chars().take(160).collect();
        s.push_str(&format!("\n- {}: {d}", k.name));
    }
    s.push(')');
    Some(s)
}

/// Herramienta `skill`: el cuerpo de una skill, o la lista con `?`.
pub fn load(root: &Path, name: &str) -> anyhow::Result<String> {
    let all = discover(root);
    let name = name.trim();
    if name.is_empty() || name == "?" || name == "list" {
        let mut s = crate::tr!("{} skills:", "{} skills:", all.len());
        for k in &all {
            let d: String = k.description.chars().take(110).collect();
            s.push_str(&format!("\n- {}: {d}", k.name));
        }
        return Ok(s);
    }
    let Some(k) = all.iter().find(|k| k.name.eq_ignore_ascii_case(name)) else {
        let near: Vec<&str> = all
            .iter()
            .filter(|k| k.name.contains(name) || name.contains(&k.name))
            .map(|k| k.name.as_str())
            .collect();
        // O1 (medido, 4 corridas): lo PRIMERO que hace el 9B para buscar las recetas de Control Mac
        // es `skill «recetas Microsoft Word»` / «Microsoft Excel» / «platform». Se le atiende donde
        // busca: si nombra una app con recetas, se le dan aquí mismo (y se le dice con qué se ejecutan).
        if let Some(r) = crate::apps::recetas_para(name) {
            return Ok(crate::tr!(
                "(No es una skill: son las RECETAS de la herramienta `desktop` para esa app. Se ejecutan con `desktop`: [\"applescript\", \"<bloque>\"], [\"guardar\", …], [\"abrir\", …].)\n\n{r}",
                "(Not a skill: these are the `desktop` tool RECIPES for that app. Run them with `desktop`: [\"applescript\", \"<block>\"], [\"guardar\", …], [\"abrir\", …].)\n\n{r}"
            ));
        }
        if name.eq_ignore_ascii_case("platform") || name.eq_ignore_ascii_case("desktop") || name.to_lowercase().starts_with("recetas") || crate::apps::ruta_app(name).is_some() {
            anyhow::bail!(
                "{}",
                crate::tr!(
                    "no hay una skill «{name}»: lo de las apps del Mac está en la herramienta `desktop` → [\"recetas\",\"{name}\"] (Word, Excel, PowerPoint probadas) o [\"diccionario\",\"{name}\",\"palabra\"]",
                    "there is no skill «{name}»: Mac apps live in the `desktop` tool → [\"recetas\",\"{name}\"] (Word, Excel, PowerPoint tested) or [\"diccionario\",\"{name}\",\"word\"]"
                )
            );
        }
        anyhow::bail!(
            "{}",
            crate::tr!(
                "no hay una skill «{name}»{}; usa name \"?\" para ver la lista",
                "there is no skill «{name}»{}; use name \"?\" to see the list",
                if near.is_empty() {
                    String::new()
                } else {
                    format!(" ({})", near.join(", "))
                }
            )
        );
    };
    let text = std::fs::read_to_string(k.dir.join("SKILL.md"))?;
    let (_, body) = split_front(&text);
    Ok(crate::tr!(
        "[skill {} · carpeta {} — sus archivos de referencia se leen con read_file usando esa ruta]\n\n{body}",
        "[skill {} · folder {} — its reference files are read with read_file using that path]\n\n{body}",
        k.name,
        k.dir.display()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> crate::CarpetaPrueba {
        let d = crate::CarpetaPrueba::sin_crear(&format!("gotv-skills-{tag}"));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn put(dir: &Path, folder: &str, front: &str, body: &str) {
        std::fs::create_dir_all(dir.join(folder)).unwrap();
        std::fs::write(
            dir.join(folder).join("SKILL.md"),
            format!("---\n{front}\n---\n\n{body}"),
        )
        .unwrap();
    }

    #[test]
    fn cabeceras_reales_de_claude_code() {
        let d = tmp("cab");
        put(&d, "tributario", "name: tributario\ndescription: >-\n  Asesor experto en derecho tributario colombiano. Usar cuando el usuario\n  pregunte sobre impuestos en Colombia: renta, IVA, retención.", "# Tributario\ncuerpo");
        put(&d, "anatomy", "name: anatomy\ndescription: Build crafted isometric SVG figures\nlicense: MIT\nmetadata:\n  opencode/slash: \"true\"", "# Anatomy");
        put(&d, "sin-nombre", "description: \"una skill sin name\"", "x");
        std::fs::create_dir_all(d.join("vacia")).unwrap(); // sin SKILL.md: se ignora
        let s = discover_in(&[d.to_path_buf()]);
        let names: Vec<&str> = s.iter().map(|k| k.name.as_str()).collect();
        assert_eq!(names, vec!["anatomy", "sin-nombre", "tributario"]);
        let t = s.iter().find(|k| k.name == "tributario").unwrap();
        assert_eq!(t.description, "Asesor experto en derecho tributario colombiano. Usar cuando el usuario pregunte sobre impuestos en Colombia: renta, IVA, retención.");
        assert_eq!(
            s.iter()
                .find(|k| k.name == "sin-nombre")
                .unwrap()
                .description,
            "una skill sin name"
        );
    }

    #[test]
    fn el_proyecto_gana_y_la_relevancia_elige() {
        let (p, u) = (tmp("proy"), tmp("user"));
        put(
            &p,
            "saludo",
            "name: saludo\ndescription: Cómo saludar en ESTE proyecto",
            "versión del proyecto",
        );
        put(
            &u,
            "saludo",
            "name: saludo\ndescription: saludo genérico",
            "versión del usuario",
        );
        // descripción como la real: «impuesto de renta (personas naturales o jurídicas), IVA, retención en la fuente»
        put(&u, "tributario", "name: tributario\ndescription: impuestos colombianos: impuesto de renta (personas naturales o jurídicas), IVA, retención en la fuente", "x");
        put(
            &u,
            "kicad",
            "name: kicad\ndescription: Analyze KiCad schematics and PCB layouts",
            "x",
        );
        let all = discover_in(&[p.to_path_buf(), u.to_path_buf()]);
        assert_eq!(
            all.iter().find(|k| k.name == "saludo").unwrap().description,
            "Cómo saludar en ESTE proyecto"
        );
        let r: Vec<&str> = relevant(
            "¿Cuánto IVA y retención pago por la renta de mi local?",
            &all,
            5,
        )
        .iter()
        .map(|k| k.name.as_str())
        .collect();
        assert_eq!(r, vec!["tributario"]);
        let r: Vec<&str> = relevant("revisa el esquemático de kicad de la placa", &all, 5)
            .iter()
            .map(|k| k.name.as_str())
            .collect();
        assert_eq!(r, vec!["kicad"]);
        assert!(
            relevant("cambia el color del botón", &all, 5).is_empty(),
            "nada relevante: no se ensucia el contexto"
        );
        // K4: un disparador solo basta (órdenes de voz cortas), también si es corto («ip»),
        // y sin él no se cuela en lo que no es suyo
        let mut con_sistema = all.clone();
        con_sistema.push(Skill {
            name: "sistema".into(),
            description: "La Mac por comando".into(),
            dir: PathBuf::new(),
            disparadores: vec!["batería".into(), "ip".into(), "modo oscuro".into()],
        });
        let nombres = |t: &str| relevant(t, &con_sistema, 5).iter().map(|s| s.name.clone()).collect::<Vec<_>>();
        assert!(nombres("¿cuánta bateria tengo?").contains(&"sistema".to_string()));
        assert!(nombres("dime mi IP").contains(&"sistema".to_string()));
        assert!(nombres("pon el modo oscuro").contains(&"sistema".to_string()));
        assert!(!nombres("arregla el login de la app").contains(&"sistema".to_string()));
        assert!(!nombres("oscuro como la noche").contains(&"sistema".to_string()), "las dos palabras de «modo oscuro»");
        // el caso real que se coló: dos raíces sueltas no alcanzan a la mejor
        let mut con_ruido = all.clone();
        con_ruido.push(Skill {
            name: "chaos".into(),
            description: "gateway con personalidad que devora cualquier fuente".into(),
            dir: PathBuf::new(),
            disparadores: vec![],
        });
        let r: Vec<&str> = relevant(
            "IVA y retención en la fuente como persona natural y renta",
            &con_ruido,
            5,
        )
        .iter()
        .map(|k| k.name.as_str())
        .collect();
        assert_eq!(r, vec!["tributario"]);
    }

    #[test]
    fn la_skill_navegador_viaja_con_gotv() {
        let d = crate::CarpetaPrueba::sin_crear("gotv-skills-builtin");
        std::fs::create_dir_all(&d).unwrap();
        let all = discover(&d);
        let nav = all
            .iter()
            .find(|s| s.name == "navegador")
            .expect("incluida");
        assert!(nav.description.contains("browser"));
        let body = load(&d, "navegador").unwrap();
        assert!(
            body.contains("[\"upload\",\"@e3\"") && body.contains("a11y"),
            "{}",
            &body[..200]
        );
        // y aparece cuando la tarea es web
        let n = note("crea una página web y pruébala en el navegador", &d).unwrap_or_default();
        assert!(n.contains("navegador"), "{n}");
    }
}
