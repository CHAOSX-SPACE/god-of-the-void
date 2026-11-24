//! F16.2 · Subagentes a medida: `task { agente: "revisor", prompt: "…" }`.
//!
//! **Dónde viven** (el primero que tenga ese nombre gana):
//! 1. `<proyecto>/.gotv/agentes/<nombre>.md` (formato de GOTV)
//! 2. `<proyecto>/.claude/agents/<nombre>.md` (formato de Claude Code, leído tal cual)
//! 3. `~/.config/gotv/agentes/<nombre>.md` (o `GOTV_AGENTES_DIR`) y `~/.claude/agents/`
//! 4. la carpeta `agentes/` de los plugins en los que se confió (F10.4)
//!
//! **Formato:** una cabecera mínima y las instrucciones debajo. Es un archivo del USUARIO en su proyecto
//! (como las reglas de F12), no un dato de GOTV en el repo; se lee con el mismo lector, sin YAML.
//! ```text
//! ---
//! nombre: revisor              (o name:)
//! descripcion: revisa sin tocar (o description:)
//! herramientas: [read_file, grep, glob]   (o tools: Read, Grep, Glob — nombres de Claude Code)
//! modelo: anthropic:claude-haiku-5-5      (o model: haiku | inherit; vacío = el mismo del chat)
//! ---
//! Eres un revisor…
//! ```
//!
//! **Trampa 2 de la ficha:** la lista de herramientas se aplica en `permissions.rs` (`solo_herramientas`),
//! no en el prompt: un modelo que «decide» usar otra se encuentra el no. Además el subagente ni la ve.
//!
//! **Cómo corre:** como un `Agent::hijo` de F9 (`agent/medida.rs`): un agente PROPIO con su motor, sus
//! permisos (los del chat + `solo_herramientas`) y la cara del chat con su nombre. Su conversación es suya (sin
//! sesión: no toca la caché en disco del chat) y, si su modelo es de nube, el hijo enmascara secretos solo
//! (F14.3 mira el motor del propio agente). Un modelo de nube lo conecta GOTV con la llave del Llavero; uno
//! LOCAL lo carga quien sabe cargar modelos (la ventana o la CLI registran una [`Fabrica`]). El último motor de
//! agente se guarda para la siguiente `task` con el mismo modelo.
use anyhow::{anyhow, Result};
use gotv_engine::Engine;
use std::collections::HashMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, Mutex, OnceLock};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Formato {
    Gotv,
    ClaudeCode,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Medida {
    pub nombre: String,
    pub descripcion: String,
    /// `None` = todas las del agente principal (como en Claude Code cuando falta `tools`)
    pub herramientas: Option<Vec<String>>,
    /// `None` = el mismo modelo del chat
    pub modelo: Option<String>,
    pub instrucciones: String,
    pub archivo: PathBuf,
    pub formato: Formato,
}

/// Herramientas que un agente a medida siempre tiene: no hacen nada fuera de su conversación.
pub const SIEMPRE: &[&str] = &["final", "terminar", "todo", "tool_search"];

/// Nombres de Claude Code → los de GOTV.
fn de_claude_code(t: &str) -> Option<&'static str> {
    // juez-idioma: datos (nombres de herramientas de Claude Code)
    Some(match t {
        "Read" => "read_file",
        "Write" => "write_file",
        "Edit" => "edit",
        "MultiEdit" => "multi_edit",
        "Glob" => "glob",
        "Grep" => "grep",
        "LS" => "list_dir",
        "Bash" => "bash",
        "BashOutput" | "KillShell" | "KillBash" => "process",
        "WebFetch" => "web_fetch",
        "WebSearch" => "web_search",
        "TodoWrite" => "todo",
        "Skill" => "skill",
        _ => return None,
    })
}

fn nombre_valido(n: &str) -> bool {
    !n.is_empty() && n.len() <= 64 && n.chars().all(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | '.')) && !n.starts_with('.')
}

/// Los alias de modelo de Claude Code → un modelo de nube de GOTV (`proveedor:modelo`).
fn modelo_de(valor: &str) -> Option<String> {
    let v = valor.trim().trim_matches(|c| c == '"' || c == '\'');
    // juez-idioma: datos (alias de modelos)
    match v.to_lowercase().as_str() {
        "" | "inherit" | "heredar" | "mismo" | "same" => None,
        "sonnet" => Some("anthropic:claude-sonnet-5-5".into()),
        "opus" => Some("anthropic:claude-opus-5-5".into()),
        "haiku" => Some("anthropic:claude-haiku-5-5".into()),
        "fable" => Some("anthropic:claude-fable-5-2".into()),
        _ => Some(v.to_string()),
    }
}

/// Un valor de una línea de la cabecera (`clave: valor`).
fn valor(meta: &str, claves: &[&str]) -> Option<String> {
    meta.lines().find_map(|l| {
        let (k, v) = l.split_once(':')?;
        claves.contains(&k.trim()).then(|| v.trim().trim_matches(|c| c == '"' || c == '\'').to_string())
    })
}

/// Lee un archivo de agente. `Err` con la causa si no sirve (nombre inválido, sin instrucciones).
pub fn leer(archivo: &Path, texto: &str, formato: Formato) -> Result<Medida> {
    let (meta, cuerpo) = crate::skills::split_front(texto);
    let del_archivo = archivo.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    let nombre = valor(meta, &["nombre", "name"]).filter(|n| !n.is_empty()).unwrap_or(del_archivo);
    if !nombre_valido(&nombre) {
        return Err(anyhow!("{}", crate::tr!("nombre de agente inválido: «{nombre}»", "invalid agent name: «{nombre}»")));
    }
    let instrucciones = cuerpo.trim().to_string();
    if instrucciones.is_empty() {
        return Err(anyhow!("{}", crate::tr!("el agente «{nombre}» no tiene instrucciones", "agent «{nombre}» has no instructions")));
    }
    let herramientas = crate::formas::reglas::lista(meta, &["herramientas", "tools"]).filter(|v| !v.is_empty()).map(|v| {
        let mut out: Vec<String> = Vec::new();
        for t in v {
            let t = t.trim();
            let g = de_claude_code(t).map(String::from).unwrap_or_else(|| t.to_string());
            if !out.contains(&g) {
                out.push(g);
            }
        }
        out
    });
    Ok(Medida {
        nombre,
        // juez-idioma: datos (las claves de la cabecera que se reconocen, en los dos idiomas) — bloque
        descripcion: valor(meta, &["descripcion", "descripción", "description"]).unwrap_or_default(),
        herramientas,
        modelo: valor(meta, &["modelo", "model"]).and_then(|m| modelo_de(&m)),
        instrucciones,
        archivo: archivo.to_path_buf(),
        formato,
    })
}

/// La carpeta de los agentes del usuario.
pub fn dir_usuario() -> PathBuf {
    if let Some(d) = std::env::var_os("GOTV_AGENTES_DIR").filter(|d| !d.is_empty()) {
        return PathBuf::from(d);
    }
    PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".config/gotv/agentes")
}

/// Las carpetas donde se buscan, en orden de precedencia.
pub fn carpetas(root: &Path) -> Vec<(PathBuf, Formato)> {
    let mut v = vec![(root.join(".gotv/agentes"), Formato::Gotv), (root.join(".claude/agents"), Formato::ClaudeCode), (dir_usuario(), Formato::Gotv)];
    // el ~/.claude/agents del usuario sólo si no se pidió una carpeta propia (las pruebas no ven el real)
    if std::env::var_os("GOTV_AGENTES_DIR").is_none() {
        v.push((PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".claude/agents"), Formato::ClaudeCode));
        v.extend(crate::plugins::carpetas("agentes").into_iter().map(|d| (d, Formato::Gotv)));
    }
    v
}

/// Todos los agentes a medida que ve este proyecto (sin repetir nombre) y los archivos que no sirven.
pub fn todos(root: &Path) -> (Vec<Medida>, Vec<String>) {
    let (mut out, mut malos): (Vec<Medida>, Vec<String>) = (Vec::new(), Vec::new());
    for (d, formato) in carpetas(root) {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        let mut archivos: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "md") && p.is_file()).collect();
        archivos.sort();
        for a in archivos {
            let Ok(t) = std::fs::read_to_string(&a) else { continue };
            match leer(&a, &t, formato) {
                Ok(m) if !out.iter().any(|x| x.nombre == m.nombre) => out.push(m),
                Ok(_) => {}
                Err(e) => malos.push(format!("{}: {e}", a.display())),
            }
        }
    }
    (out, malos)
}

pub fn buscar(root: &Path, nombre: &str) -> Option<Medida> {
    todos(root).0.into_iter().find(|m| m.nombre == nombre.trim())
}

/// La lista para el modelo (`task { agente: "?" }`) y para `/agentes`.
pub fn lista(root: &Path) -> String {
    let (ms, malos) = todos(root);
    let mut s = if ms.is_empty() {
        crate::i18n::t(
            "No hay agentes a medida. Se crean en .gotv/agentes/<nombre>.md (o .claude/agents/, o ~/.config/gotv/agentes/): cabecera con nombre, descripcion, herramientas y modelo; las instrucciones debajo.",
            "There are no custom agents. They are created in .gotv/agentes/<name>.md (or .claude/agents/, or ~/.config/gotv/agentes/): a header with name, description, tools and model; the instructions below.",
        )
        .to_string()
    } else {
        crate::i18n::t("Agentes a medida (task con agente=<nombre>):", "Custom agents (task with agente=<name>):").to_string()
    };
    for m in &ms {
        let h = m.herramientas.as_ref().map(|h| h.join(", ")).unwrap_or_else(|| crate::i18n::t("todas", "all").into());
        let modelo = m.modelo.clone().unwrap_or_else(|| crate::i18n::t("el del chat", "the chat's").into());
        s.push_str(&format!("\n· {} — {} [{}: {h} · {}: {modelo}]", m.nombre, m.descripcion, crate::i18n::t("herramientas", "tools"), crate::i18n::t("modelo", "model")));
    }
    for m in malos {
        s.push_str(&format!("\n⚠ {m}"));
    }
    s
}

/// ¿Puede usar `tool` un agente con esta lista? (`permissions.rs` la aplica; `agent.rs` filtra lo que ve)
pub fn permitida(lista: &[String], tool: &str) -> bool {
    SIEMPRE.contains(&tool)
        || lista.iter().any(|t| t == tool || t.strip_suffix('*').is_some_and(|p| !p.is_empty() && tool.starts_with(p)) || (t.starts_with("mcp__") && !t[5..].contains("__") && tool.starts_with(&format!("{t}__"))))
}

/// Lo que se le dice a `permissions.rs` cuando niega.
pub fn negada(lista: &[String], tool: &str) -> String {
    crate::tr!(
        "aquí no tienes la herramienta «{tool}» (este agente o este turno sólo tienen unas): las tuyas son {}. Hazlo con ellas o di en tu respuesta lo que hace falta hacer",
        "you do not have the «{tool}» tool here (this agent or this turn only has some): yours are {}. Do it with them or say in your answer what needs to be done",
        lista.join(", ")
    )
}

// ───────────────────────── el motor de un agente ─────────────────────────

pub type Motor = Arc<dyn Engine>;
pub type Fabrica = Arc<dyn Fn(String) -> Pin<Box<dyn Future<Output = Result<Motor>> + Send>> + Send + Sync>;

fn fabricas() -> &'static Mutex<HashMap<String, Fabrica>> {
    static F: OnceLock<Mutex<HashMap<String, Fabrica>>> = OnceLock::new();
    F.get_or_init(Default::default)
}

/// Arma una [`Fabrica`] con una función asíncrona `modelo → motor`.
pub fn fabrica<F, Fut>(f: F) -> Fabrica
where
    F: Fn(String) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<Motor>> + Send + 'static,
{
    Arc::new(move |m: String| -> Pin<Box<dyn Future<Output = Result<Motor>> + Send>> { Box::pin(f(m)) })
}

/// La ventana o la CLI dicen cómo cargar un modelo (local o de nube) para un agente.
pub fn registrar_fabrica(f: Fabrica) {
    fabricas().lock().unwrap().insert("*".into(), f);
}

/// Sólo para un chat (las pruebas, que corren a la vez).
pub fn registrar_fabrica_para(dueno: &str, f: Fabrica) {
    fabricas().lock().unwrap().insert(dueno.into(), f);
}

/// El último motor de agente cargado (modelo, motor): la siguiente `task` con ese modelo no recarga.
fn guardado() -> &'static Mutex<Option<(String, Motor)>> {
    static G: OnceLock<Mutex<Option<(String, Motor)>>> = OnceLock::new();
    G.get_or_init(Default::default)
}

/// Suelta el motor de agente guardado (libera su memoria). La ventana lo llama al cambiar de modelo.
pub fn soltar_guardado() {
    guardado().lock().unwrap().take();
}

/// Un modelo de NUBE con la configuración del proyecto (lo que la ventana haría para el chat).
async fn de_nube(root: &Path, modelo: &str) -> Result<Motor> {
    let mut cfg = crate::config::Config::load(root);
    cfg.model = modelo.to_string();
    if !crate::nube::is_cloud(modelo, &cfg) {
        return Err(anyhow!(
            "{}",
            crate::tr!(
                "«{modelo}» es un modelo local: lo carga la ventana o la CLI de GOTV (aquí no hay quien lo cargue)",
                "«{modelo}» is a local model: the GOTV window or CLI loads it (nobody here can load it)"
            )
        ));
    }
    let (motor, _, _) = crate::nube::launch(&mut cfg).await?;
    Ok(Arc::new(motor))
}

/// El motor del modelo de un agente (el guardado si es el mismo; si no, la fábrica de la ventana/CLI o la nube).
pub(crate) async fn motor_para(dueno: &str, root: &Path, modelo: &str) -> Result<Motor> {
    if let Some((m, e)) = guardado().lock().unwrap().as_ref() {
        if m == modelo {
            return Ok(e.clone());
        }
    }
    let f = {
        let fs = fabricas().lock().unwrap();
        fs.get(dueno).or_else(|| fs.get("*")).cloned()
    };
    let motor = match f {
        Some(f) => f(modelo.to_string()).await?,
        None => de_nube(root, modelo).await?,
    };
    *guardado().lock().unwrap() = Some((modelo.to_string(), motor.clone()));
    Ok(motor)
}

/// La orden del agente a medida: quién es, sus instrucciones y la subtarea.
pub fn orden(m: &Medida, prompt: &str) -> String {
    let h = m.herramientas.as_ref().map(|h| h.join(", ")).unwrap_or_else(|| crate::i18n::t("todas", "all").into());
    crate::tr!(
        "Eres el agente «{}», un SUBAGENTE a medida del agente principal. Haz SÓLO esta subtarea, con tus herramientas ({h}), y termina con un informe breve y concreto para él (qué encontraste o hiciste, con archivos y líneas). No le preguntes nada al usuario.\n\n# Tus instrucciones\n{}\n\n# Subtarea\n{prompt}",
        "You are the «{}» agent, a custom SUBAGENT of the main agent. Do ONLY this subtask, with your tools ({h}), and finish with a short, concrete report for it (what you found or did, with files and lines). Do not ask the user anything.\n\n# Your instructions\n{}\n\n# Subtask\n{prompt}",
        m.nombre,
        m.instrucciones
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lee_los_dos_formatos() {
        let g = leer(Path::new("/x/.gotv/agentes/revisor.md"), "---\nnombre: revisor\ndescripcion: revisa sin tocar\nherramientas: [read_file, grep]\nmodelo: anthropic:claude-haiku-5-5\n---\nRevisa con calma.\n", Formato::Gotv).unwrap();
        assert_eq!(g.nombre, "revisor");
        assert_eq!(g.herramientas.as_deref(), Some(&["read_file".to_string(), "grep".to_string()][..]));
        assert_eq!(g.modelo.as_deref(), Some("anthropic:claude-haiku-5-5"));
        let c = leer(Path::new("/x/.claude/agents/code-reviewer.md"), "---\nname: code-reviewer\ndescription: Reviews code\ntools: Read, Grep, Glob, Bash\nmodel: inherit\n---\nYou review code.", Formato::ClaudeCode).unwrap();
        assert_eq!(c.herramientas.unwrap(), vec!["read_file", "grep", "glob", "bash"]);
        assert_eq!(c.modelo, None, "inherit = el del chat");
        let s = leer(Path::new("/x/a/sin-cabecera.md"), "Sólo instrucciones", Formato::Gotv).unwrap();
        assert_eq!((s.nombre.as_str(), s.herramientas.is_none(), s.modelo.is_none()), ("sin-cabecera", true, true));
        assert!(leer(Path::new("/x/a/vacio.md"), "---\nname: vacio\n---\n", Formato::Gotv).is_err());
        assert!(leer(Path::new("/x/a/x.md"), "---\nname: ../fuera\n---\nhola", Formato::Gotv).is_err());
        assert_eq!(modelo_de("haiku").as_deref(), Some("anthropic:claude-haiku-5-5"));
    }

    #[test]
    fn la_lista_permite_lo_suyo_y_lo_inocuo() {
        let l = vec!["read_file".to_string(), "mcp__docs".to_string(), "git*".to_string()];
        assert!(permitida(&l, "read_file"));
        assert!(permitida(&l, "todo") && permitida(&l, "final"));
        assert!(!permitida(&l, "write_file") && !permitida(&l, "bash"));
        assert!(permitida(&l, "mcp__docs__buscar") && !permitida(&l, "mcp__otro__x"));
        assert!(permitida(&l, "github") && !permitida(&l, "glob"));
    }

    #[test]
    fn precedencia_proyecto_sobre_usuario() {
        let d = crate::CarpetaPrueba::nueva("gotv-agentes-prec");
        let u = crate::CarpetaPrueba::nueva("gotv-agentes-usuario");
        std::fs::create_dir_all(d.join(".gotv/agentes")).unwrap();
        std::fs::create_dir_all(d.join(".claude/agents")).unwrap();
        std::fs::write(d.join(".gotv/agentes/revisor.md"), "---\ndescripcion: el del proyecto\n---\nA").unwrap();
        std::fs::write(d.join(".claude/agents/revisor.md"), "---\nname: revisor\ndescription: el de claude\n---\nB").unwrap();
        std::fs::write(d.join(".claude/agents/otro.md"), "---\nname: otro\n---\nC").unwrap();
        std::fs::write(u.join("revisor.md"), "---\ndescripcion: el del usuario\n---\nD").unwrap();
        std::fs::write(u.join("roto.md"), "---\nname: roto\n---\n").unwrap();
        // la carpeta del usuario por variable de entorno: sin ella, se leería la real
        let antes = std::env::var_os("GOTV_AGENTES_DIR");
        std::env::set_var("GOTV_AGENTES_DIR", &*u);
        let (ms, malos) = todos(&d);
        match antes {
            Some(v) => std::env::set_var("GOTV_AGENTES_DIR", v),
            None => std::env::remove_var("GOTV_AGENTES_DIR"),
        }
        let r = ms.iter().find(|m| m.nombre == "revisor").unwrap();
        assert_eq!(r.descripcion, "el del proyecto");
        assert!(ms.iter().any(|m| m.nombre == "otro" && m.formato == Formato::ClaudeCode));
        assert_eq!(ms.iter().filter(|m| m.nombre == "revisor").count(), 1);
        assert!(malos.iter().any(|m| m.contains("roto")), "{malos:?}");
    }
}
