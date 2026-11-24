//! F10.4 · Plugins: ampliar GOTV con paquetes, sin que un paquete abra una puerta que la ventana no abriría.
//!
//! **Dónde:** `~/.config/gotv/plugins/<nombre>/` (o `GOTV_PLUGINS_DIR`). Dos formatos, que se LEEN tal cual:
//!
//! - **De GOTV:** `plugin.toml` (`nombre`, `version`, `descripcion`, `autor`) y, por convención, `skills/`,
//!   `mcp.json` (el formato de `.mcp.json`), `hooks.json` (el de los hooks), `lsp.toml`
//!   (`[servidores.<n>]` con `comando`, `extensiones`, `entorno`, `raices`, `inicio`, `ajustes`),
//!   `paquetes/<id>/senales.toml` (F11), `reglas/` y `agentes/`.
//! - **De Claude Code:** `.claude-plugin/plugin.json` (o sin manifiesto, por convención): `skills/`, `.mcp.json`
//!   y `mcpServers`, `.lsp.json` y `lspServers`, `hooks/hooks.json` y `hooks`. Sus rutas empiezan por `./` y no
//!   salen del plugin; `${CLAUDE_PLUGIN_ROOT}` y `${CLAUDE_PLUGIN_DATA}` se resuelven como allí.
//!
//! **Confianza:** un plugin trae programas que correrán en esta Mac (servidores MCP y de lenguaje, hooks). Se
//! instala enseñando TODO lo que trae y sólo carga si una persona confió en ESE contenido exacto: la huella
//! (sha256 de cada archivo y su ruta) se guarda en `.confianza.bin` (formato propio de GOTV, `programar::codec`).
//! Si un archivo cambia después, el plugin deja de cargar hasta que se confíe otra vez. **Un plugin sin
//! confianza no carga nada.** Lo que carga entra como lo del usuario: sus servidores MCP van a la caja de arena
//! como cualquier otro, y sus herramientas pasan por la misma puerta de permisos.
//!
//! **Quién lo consume:** `hooks::load`, `skills::dirs`, `mcp::load`, `lsp::registro::defs` (y el entorno, las
//! opciones y los ajustes del servidor en `Lsp::start`), `lenguajes::catalogo` (paquetes). `reglas/` (F12, reglas
//! por ruta) y `agentes/` (F16.2) se ofrecen con [`carpetas`] para cuando esas fases las lean.
use crate::lsp::registro::Def;
use crate::programar::codec;
use anyhow::{anyhow, bail, Result};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

/// La carpeta de los plugins.
pub fn dir() -> PathBuf {
    if let Some(d) = std::env::var_os("GOTV_PLUGINS_DIR").filter(|d| !d.is_empty()) {
        return PathBuf::from(d);
    }
    PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".config/gotv/plugins")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Formato {
    Gotv,
    ClaudeCode,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Plugin {
    pub nombre: String,
    pub version: String,
    pub descripcion: String,
    pub dir: PathBuf,
    pub formato: Formato,
}

/// Un servidor de lenguaje que trae un plugin.
#[derive(Debug, Clone, PartialEq)]
pub struct LspPlugin {
    pub servidor: String,
    /// programa y argumentos (con `${CLAUDE_PLUGIN_ROOT}` ya resuelto)
    pub comando: Vec<String>,
    /// `.ext` → `languageId`
    pub extensiones: Vec<(String, String)>,
    pub entorno: Vec<(String, String)>,
    pub raices: Vec<String>,
    pub inicio: Option<Value>,
    pub ajustes: Option<Value>,
}

/// Lo que trae un plugin.
#[derive(Debug, Clone, Default)]
pub struct Contenido {
    /// carpetas con `<skill>/SKILL.md` dentro
    pub skills: Vec<PathBuf>,
    /// servidores MCP (nombre → configuración, con las rutas ya resueltas)
    pub mcp: Map<String, Value>,
    /// archivos de hooks (el formato de `hooks.json`)
    pub hooks: Vec<Value>,
    pub lsp: Vec<LspPlugin>,
    pub paquetes: Vec<PathBuf>,
    pub reglas: Vec<PathBuf>,
    pub agentes: Vec<PathBuf>,
    /// lo que el plugin trae y GOTV no carga (dicho, no callado)
    pub avisos: Vec<String>,
}

/// Un nombre de plugin aceptable: letras, dígitos, `-`, `_` y `.`, sin empezar por `.`.
fn nombre_valido(n: &str) -> bool {
    !n.is_empty() && !n.starts_with('.') && n.len() <= 64 && n.chars().all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
}

/// La carpeta de datos de un plugin (`${CLAUDE_PLUGIN_DATA}`): se crea al pedirla.
fn datos_de(base: &Path, nombre: &str) -> PathBuf {
    base.join(".datos").join(nombre)
}

/// Resuelve `${CLAUDE_PLUGIN_ROOT}`/`${GOTV_PLUGIN_ROOT}` y `${CLAUDE_PLUGIN_DATA}`/`${GOTV_PLUGIN_DATA}` en
/// todas las cadenas de un valor.
fn resolver(v: &mut Value, raiz: &Path, datos: &Path) {
    match v {
        Value::String(s) => {
            if s.contains("_PLUGIN_") {
                *s = s
                    .replace("${CLAUDE_PLUGIN_ROOT}", &raiz.to_string_lossy())
                    .replace("${GOTV_PLUGIN_ROOT}", &raiz.to_string_lossy())
                    .replace("${CLAUDE_PLUGIN_DATA}", &datos.to_string_lossy())
                    .replace("${GOTV_PLUGIN_DATA}", &datos.to_string_lossy());
            }
        }
        Value::Array(a) => a.iter_mut().for_each(|x| resolver(x, raiz, datos)),
        Value::Object(o) => o.values_mut().for_each(|x| resolver(x, raiz, datos)),
        _ => {}
    }
}

fn tiene_user_config(v: &Value) -> bool {
    match v {
        Value::String(s) => s.contains("${user_config."),
        Value::Array(a) => a.iter().any(tiene_user_config),
        Value::Object(o) => o.values().any(tiene_user_config),
        _ => false,
    }
}

/// Una ruta del manifiesto de Claude Code: empieza por `./` (o es `.`) y no sale del plugin.
fn ruta_dentro(raiz: &Path, r: &str) -> Option<PathBuf> {
    if r != "." && !r.starts_with("./") {
        return None;
    }
    let p = raiz.join(r.trim_start_matches("./"));
    let real = p.canonicalize().ok()?;
    let base = raiz.canonicalize().ok()?;
    real.starts_with(&base).then_some(real)
}

fn leer_json(p: &Path) -> Result<Value> {
    let t = std::fs::read_to_string(p).map_err(|e| anyhow!("{}: {e}", p.display()))?;
    serde_json::from_str(&t).map_err(|e| anyhow!("{}: {e}", p.display()))
}

/// Un componente del manifiesto de Claude Code que puede ser ruta, objeto en línea o lista de ambos.
fn valores(raiz: &Path, v: &Value, que: &str, avisos: &mut Vec<String>) -> Vec<Value> {
    match v {
        Value::Null => vec![],
        Value::Array(a) => a.iter().flat_map(|x| valores(raiz, x, que, avisos)).collect(),
        Value::String(r) => match ruta_dentro(raiz, r) {
            Some(p) => match leer_json(&p) {
                Ok(v) => vec![v],
                Err(e) => {
                    avisos.push(crate::tr!("{que}: {e}", "{que}: {e}"));
                    vec![]
                }
            },
            None => {
                avisos.push(crate::tr!("{que}: la ruta «{r}» no empieza por ./ o sale del plugin; se ignora", "{que}: the path «{r}» does not start with ./ or leaves the plugin; ignored"));
                vec![]
            }
        },
        Value::Object(_) => vec![v.clone()],
        _ => vec![],
    }
}

/// Un mapa de servidores: `{"mcpServers": {...}}` o el mapa suelto.
fn mapa_de_servidores(v: &Value) -> Map<String, Value> {
    let m = if v.get("mcpServers").is_some_and(Value::is_object) { &v["mcpServers"] } else { v };
    m.as_object().cloned().unwrap_or_default()
}

/// `lspServers` de Claude Code (`.lsp.json`: mapa suelto, campos estrictos) → servidores.
fn lsp_de_claude(m: &Value, avisos: &mut Vec<String>) -> Vec<LspPlugin> {
    let mut out = Vec::new();
    for (nombre, s) in m.as_object().into_iter().flatten() {
        let Some(cmd) = s["command"].as_str().filter(|c| !c.trim().is_empty()) else {
            avisos.push(crate::tr!("servidor de lenguaje «{nombre}»: sin `command`; se ignora", "language server «{nombre}»: no `command`; ignored"));
            continue;
        };
        let extensiones: Vec<(String, String)> = s["extensionToLanguage"].as_object().into_iter().flatten().filter_map(|(e, l)| Some((e.clone(), l.as_str()?.to_string()))).filter(|(e, _)| e.starts_with('.') && e.len() > 1).collect();
        if extensiones.is_empty() {
            avisos.push(crate::tr!("servidor de lenguaje «{nombre}»: sin `extensionToLanguage`; se ignora", "language server «{nombre}»: no `extensionToLanguage`; ignored"));
            continue;
        }
        if s["transport"].as_str().is_some_and(|t| t != "stdio") {
            avisos.push(crate::tr!("servidor de lenguaje «{nombre}»: GOTV lo habla por stdio (pide «{}»)", "language server «{nombre}»: GOTV talks to it over stdio (it asks for «{}»)", s["transport"].as_str().unwrap_or("")));
        }
        let mut comando = vec![cmd.to_string()];
        comando.extend(s["args"].as_array().into_iter().flatten().filter_map(|a| a.as_str().map(String::from)));
        out.push(LspPlugin {
            servidor: nombre.clone(),
            comando,
            extensiones,
            entorno: s["env"].as_object().into_iter().flatten().filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string()))).collect(),
            raices: vec![],
            inicio: s.get("initializationOptions").cloned(),
            ajustes: s.get("settings").cloned(),
        });
    }
    out
}

/// `lsp.toml` de GOTV → servidores.
fn lsp_de_gotv(t: &str, avisos: &mut Vec<String>) -> Vec<LspPlugin> {
    let v: toml::Value = match toml::from_str(t) {
        Ok(v) => v,
        Err(e) => {
            avisos.push(crate::tr!("lsp.toml: {e}", "lsp.toml: {e}"));
            return vec![];
        }
    };
    let Some(serv) = v.get("servidores").and_then(|s| s.as_table()) else { return vec![] };
    // al formato de Claude Code (mismos campos con sus nombres) y por el mismo camino
    let mut m = Map::new();
    for (n, s) in serv {
        let j = serde_json::to_value(s).unwrap_or(Value::Null);
        let comando: Vec<Value> = j["comando"].as_array().cloned().unwrap_or_default();
        let mut x = json!({
            "command": comando.first().cloned().unwrap_or(Value::Null),
            "args": comando.get(1..).map(|a| a.to_vec()).unwrap_or_default(),
            "extensionToLanguage": j["extensiones"],
            "env": j["entorno"],
        });
        if !j["inicio"].is_null() {
            x["initializationOptions"] = j["inicio"].clone();
        }
        if !j["ajustes"].is_null() {
            x["settings"] = j["ajustes"].clone();
        }
        m.insert(n.clone(), x);
    }
    let mut out = lsp_de_claude(&Value::Object(m), avisos);
    for l in &mut out {
        l.raices = serv.get(&l.servidor).and_then(|s| s.get("raices")).and_then(|r| r.as_array()).map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default();
    }
    out
}

/// Lee un plugin (sin mirar la confianza). `base` = la carpeta de plugins (para `${CLAUDE_PLUGIN_DATA}`).
pub fn leer(raiz: &Path, base: &Path) -> Result<(Plugin, Contenido)> {
    let mut c = Contenido::default();
    let toml_gotv = raiz.join("plugin.toml");
    let manifiesto_cc = raiz.join(".claude-plugin/plugin.json");
    let por_carpeta = raiz.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let (plugin, cc) = if toml_gotv.is_file() {
        let t = std::fs::read_to_string(&toml_gotv)?;
        let v: toml::Value = toml::from_str(&t).map_err(|e| anyhow!("plugin.toml: {e}"))?;
        let campo = |k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
        (Plugin { nombre: campo("nombre"), version: campo("version"), descripcion: campo("descripcion"), dir: raiz.to_path_buf(), formato: Formato::Gotv }, Value::Null)
    } else if manifiesto_cc.is_file() {
        let v = leer_json(&manifiesto_cc)?;
        let campo = |k: &str| v[k].as_str().unwrap_or("").to_string();
        (Plugin { nombre: campo("name"), version: campo("version"), descripcion: campo("description"), dir: raiz.to_path_buf(), formato: Formato::ClaudeCode }, v)
    } else if [".mcp.json", ".lsp.json", "hooks/hooks.json", "skills"].iter().any(|f| raiz.join(f).exists()) {
        // Claude Code: el manifiesto es opcional; el nombre es el de la carpeta
        (Plugin { nombre: por_carpeta, version: String::new(), descripcion: String::new(), dir: raiz.to_path_buf(), formato: Formato::ClaudeCode }, json!({}))
    } else {
        bail!("{}", crate::tr!("{} no es un plugin: falta plugin.toml (GOTV) o .claude-plugin/plugin.json (Claude Code)", "{} is not a plugin: plugin.toml (GOTV) or .claude-plugin/plugin.json (Claude Code) is missing", raiz.display()));
    };
    if !nombre_valido(&plugin.nombre) {
        bail!("{}", crate::tr!("nombre de plugin no válido: «{}» (letras, dígitos, - _ .)", "invalid plugin name: «{}» (letters, digits, - _ .)", plugin.nombre));
    }
    let datos = datos_de(base, &plugin.nombre);
    match plugin.formato {
        Formato::Gotv => {
            if raiz.join("skills").is_dir() {
                c.skills.push(raiz.join("skills"));
            }
            if let Ok(v) = leer_json(&raiz.join("mcp.json")) {
                c.mcp.extend(mapa_de_servidores(&v));
            } else if raiz.join("mcp.json").exists() {
                c.avisos.push(crate::i18n::t("mcp.json no es JSON válido", "mcp.json is not valid JSON").into());
            }
            if let Ok(v) = leer_json(&raiz.join("hooks.json")) {
                c.hooks.push(v);
            }
            if let Ok(t) = std::fs::read_to_string(raiz.join("lsp.toml")) {
                c.lsp = lsp_de_gotv(&t, &mut c.avisos);
            }
        }
        Formato::ClaudeCode => {
            let m = &cc;
            if raiz.join("skills").is_dir() {
                c.skills.push(raiz.join("skills"));
            }
            for s in valores_ruta(raiz, &m["skills"], "skills", &mut c.avisos) {
                if s != raiz.join("skills") && !c.skills.contains(&s) {
                    c.skills.push(s);
                }
            }
            if raiz.join("SKILL.md").is_file() && c.skills.is_empty() {
                c.avisos.push(crate::i18n::t("SKILL.md en la raíz del plugin: GOTV lee las skills en skills/<nombre>/SKILL.md", "SKILL.md at the plugin root: GOTV reads skills in skills/<name>/SKILL.md").into());
            }
            if let Ok(v) = leer_json(&raiz.join(".mcp.json")) {
                c.mcp.extend(mapa_de_servidores(&v));
            }
            for v in valores(raiz, &m["mcpServers"], "mcpServers", &mut c.avisos) {
                c.mcp.extend(mapa_de_servidores(&v));
            }
            let mut lsp_val = Vec::new();
            if let Ok(v) = leer_json(&raiz.join(".lsp.json")) {
                lsp_val.push(v);
            }
            lsp_val.extend(valores(raiz, &m["lspServers"], "lspServers", &mut c.avisos));
            for v in lsp_val {
                for l in lsp_de_claude(&v, &mut c.avisos) {
                    // el que se declara después con el mismo nombre reemplaza al anterior
                    c.lsp.retain(|x| x.servidor != l.servidor);
                    c.lsp.push(l);
                }
            }
            if let Ok(v) = leer_json(&raiz.join("hooks/hooks.json")) {
                c.hooks.push(v);
            }
            c.hooks.extend(valores(raiz, &m["hooks"], "hooks", &mut c.avisos));
            for (dir, que) in [("commands", "commands/"), ("output-styles", "output-styles/"), ("monitors", "monitors/")] {
                if raiz.join(dir).is_dir() {
                    c.avisos.push(crate::tr!("{que}: GOTV no lo carga (usa skills)", "{que}: GOTV does not load it (it uses skills)"));
                }
            }
            if raiz.join("agents").is_dir() {
                c.agentes.push(raiz.join("agents"));
            }
        }
    }
    for (sub, destino) in [("paquetes", &mut c.paquetes), ("reglas", &mut c.reglas), ("agentes", &mut c.agentes)] {
        if raiz.join(sub).is_dir() {
            destino.push(raiz.join(sub));
        }
    }
    // rutas del plugin y sus datos en lo que corre
    let mut mcp = Value::Object(std::mem::take(&mut c.mcp));
    if tiene_user_config(&mcp) {
        c.avisos.push(crate::i18n::t("${user_config.…}: GOTV no tiene configuración por plugin; esas variables quedan vacías", "${user_config.…}: GOTV has no per-plugin configuration; those variables stay empty").into());
    }
    resolver(&mut mcp, raiz, &datos);
    c.mcp = mcp.as_object().cloned().unwrap_or_default();
    for l in &mut c.lsp {
        let mut v = json!({"c": l.comando, "e": l.entorno.iter().map(|(k, v)| json!([k, v])).collect::<Vec<_>>()});
        resolver(&mut v, raiz, &datos);
        l.comando = v["c"].as_array().into_iter().flatten().filter_map(|x| x.as_str().map(String::from)).collect();
        l.entorno = v["e"].as_array().into_iter().flatten().filter_map(|p| Some((p[0].as_str()?.to_string(), p[1].as_str()?.to_string()))).collect();
    }
    Ok((plugin, c))
}

/// Como [`valores`], pero para componentes que son CARPETAS (skills): sólo rutas.
fn valores_ruta(raiz: &Path, v: &Value, que: &str, avisos: &mut Vec<String>) -> Vec<PathBuf> {
    match v {
        Value::Array(a) => a.iter().flat_map(|x| valores_ruta(raiz, x, que, avisos)).collect(),
        Value::String(r) => match ruta_dentro(raiz, r) {
            Some(p) if p.is_dir() => vec![p],
            _ => {
                avisos.push(crate::tr!("{que}: «{r}» no es una carpeta dentro del plugin; se ignora", "{que}: «{r}» is not a folder inside the plugin; ignored"));
                vec![]
            }
        },
        _ => vec![],
    }
}

/// Lo que trae, en líneas para una persona (lo que se enseña antes de confiar).
pub fn resumen(p: &Plugin, c: &Contenido) -> Vec<String> {
    let mut v = vec![crate::tr!(
        "plugin «{}» {} ({}){}",
        "plugin «{}» {} ({}){}",
        p.nombre,
        p.version,
        match p.formato {
            Formato::Gotv => "GOTV",
            Formato::ClaudeCode => "Claude Code",
        },
        if p.descripcion.is_empty() { String::new() } else { format!(": {}", p.descripcion) }
    )];
    let skills = crate::skills::discover_in(&c.skills);
    if !skills.is_empty() {
        v.push(crate::tr!("  skills: {}", "  skills: {}", skills.iter().map(|s| s.name.clone()).collect::<Vec<_>>().join(", ")));
    }
    for (n, s) in &c.mcp {
        let lanza = match s["url"].as_str() {
            Some(u) => u.to_string(),
            None => format!("{} {}", s["command"].as_str().unwrap_or(""), s["args"].as_array().into_iter().flatten().filter_map(|a| a.as_str()).collect::<Vec<_>>().join(" ")),
        };
        v.push(crate::tr!("  servidor MCP «{n}» (correrá en esta Mac): {}", "  MCP server «{n}» (will run on this Mac): {}", lanza.trim()));
    }
    for l in &c.lsp {
        v.push(crate::tr!("  servidor de lenguaje «{}» para {} (correrá en esta Mac): {}", "  language server «{}» for {} (will run on this Mac): {}", l.servidor, l.extensiones.iter().map(|(e, _)| e.as_str()).collect::<Vec<_>>().join(" "), l.comando.join(" ")));
    }
    let mut h = crate::hooks::Hooks::default();
    for x in &c.hooks {
        crate::hooks::parse_en(x, false, Some(&p.dir), &mut h);
    }
    for x in &h.list {
        v.push(crate::tr!("  hook {} [{}] (correrá en esta Mac): {}", "  hook {} [{}] (will run on this Mac): {}", x.event.key(), if x.matcher.is_empty() { "*" } else { &x.matcher }, x.accion.destino()));
    }
    for (que, l) in [("paquetes", &c.paquetes), ("reglas", &c.reglas), ("agentes", &c.agentes)] {
        if !l.is_empty() {
            v.push(crate::tr!("  {que}: {}", "  {que}: {}", l.iter().map(|d| d.display().to_string()).collect::<Vec<_>>().join(", ")));
        }
    }
    for a in c.avisos.iter().chain(h.avisos.iter()) {
        v.push(format!("  ⚠ {a}"));
    }
    v
}

// ── la huella y la confianza ───────────────────────────────────────────────────────────────────────

/// sha256 de cada archivo del plugin (ruta relativa, si es ejecutable y contenido), en orden. `.git` no
/// cuenta; un enlace simbólico cuenta como su destino escrito (no se sigue).
pub fn huella(raiz: &Path) -> Result<String> {
    let mut archivos = Vec::new();
    recorrer(raiz, raiz, &mut archivos)?;
    archivos.sort();
    let mut h = Sha256::new();
    for rel in archivos {
        let p = raiz.join(&rel);
        let meta = std::fs::symlink_metadata(&p)?;
        if meta.file_type().is_symlink() {
            h.update(format!("L {rel} {}\n", std::fs::read_link(&p)?.display()));
            continue;
        }
        use std::os::unix::fs::PermissionsExt;
        let x = meta.permissions().mode() & 0o111 != 0;
        h.update(format!("F {rel} {x} {}\n", meta.len()));
        h.update(std::fs::read(&p)?);
    }
    Ok(format!("{:x}", h.finalize()))
}

fn recorrer(raiz: &Path, d: &Path, out: &mut Vec<String>) -> Result<()> {
    for e in std::fs::read_dir(d)? {
        let e = e?;
        let p = e.path();
        let t = e.file_type()?;
        if p.file_name().is_some_and(|n| n == ".git") {
            continue;
        }
        if t.is_dir() {
            recorrer(raiz, &p, out)?;
        } else {
            out.push(p.strip_prefix(raiz)?.to_string_lossy().into_owned());
        }
    }
    Ok(())
}

fn archivo_confianza(base: &Path) -> PathBuf {
    base.join(".confianza.bin")
}

/// (nombre, huella) de los plugins en los que se confió.
pub fn confianzas(base: &Path) -> Vec<(String, String)> {
    let Ok(b) = std::fs::read(archivo_confianza(base)) else { return vec![] };
    let leer = || -> Result<Vec<(String, String)>> {
        let (mut l, _) = codec::Lector::nuevo(&b, b"PLUG", 1)?;
        let n = l.u32()?;
        (0..n).map(|_| Ok((l.texto()?, l.texto()?))).collect()
    };
    leer().unwrap_or_default()
}

fn guardar_confianzas(base: &Path, v: &[(String, String)]) -> Result<()> {
    let mut e = codec::Escritor::nuevo(b"PLUG", 1);
    e.u32(v.len() as u32);
    for (n, h) in v {
        e.texto(n);
        e.texto(h);
    }
    codec::guardar(&archivo_confianza(base), &e.bytes())
}

/// Las rutas de los plugins cambian su carga (servidores de lenguaje ya leídos): se rehace.
static GENERACION: AtomicU64 = AtomicU64::new(0);

/// Confía en el contenido ACTUAL de un plugin instalado.
pub fn confiar_en(base: &Path, nombre: &str) -> Result<String> {
    let raiz = base.join(nombre);
    let (p, _) = leer(&raiz, base)?;
    if p.nombre != nombre {
        bail!("{}", crate::tr!("la carpeta «{nombre}» trae el plugin «{}»", "the folder «{nombre}» holds the plugin «{}»", p.nombre));
    }
    let h = huella(&raiz)?;
    let mut v = confianzas(base);
    v.retain(|(n, _)| n != nombre);
    v.push((nombre.to_string(), h.clone()));
    guardar_confianzas(base, &v)?;
    GENERACION.fetch_add(1, Ordering::Relaxed);
    Ok(h)
}

/// Cómo está un plugin instalado.
#[derive(Debug, Clone, PartialEq)]
pub enum Estado {
    /// confiado y sin cambios: carga
    Activo,
    /// nadie confió en él: no carga nada
    SinConfianza,
    /// cambió desde que se confió: no carga hasta confiar otra vez
    Cambiado,
    /// no se pudo leer
    Roto(String),
}

/// Los plugins de una carpeta, con su estado y lo que traen.
pub fn instalados_en(base: &Path) -> Vec<(Plugin, Contenido, Estado)> {
    let conf = confianzas(base);
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(base).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.is_dir() && p.file_name().is_some_and(|n| !n.to_string_lossy().starts_with('.'))).collect();
    dirs.sort();
    let mut out = Vec::new();
    for d in dirs {
        let carpeta = d.file_name().unwrap().to_string_lossy().into_owned();
        match leer(&d, base) {
            Ok((p, c)) => {
                let estado = match conf.iter().find(|(n, _)| *n == carpeta) {
                    None => Estado::SinConfianza,
                    Some((_, h)) if huella(&d).ok().as_deref() == Some(h.as_str()) && p.nombre == carpeta => Estado::Activo,
                    Some(_) => Estado::Cambiado,
                };
                out.push((p, c, estado));
            }
            Err(e) => out.push((Plugin { nombre: carpeta, version: String::new(), descripcion: String::new(), dir: d.clone(), formato: Formato::Gotv }, Contenido::default(), Estado::Roto(format!("{e:#}")))),
        }
    }
    out
}

/// Sólo los que cargan (confiados y sin cambios).
pub fn cargados_en(base: &Path) -> Vec<(Plugin, Contenido)> {
    instalados_en(base).into_iter().filter(|(_, _, e)| *e == Estado::Activo).map(|(p, c, _)| (p, c)).collect()
}

fn cargados() -> Vec<(Plugin, Contenido)> {
    let base = dir();
    if !base.is_dir() {
        return vec![];
    }
    cargados_en(&base)
}

// ── instalar y quitar ──────────────────────────────────────────────────────────────────────────────

fn copiar(de: &Path, a: &Path) -> Result<()> {
    std::fs::create_dir_all(a)?;
    for e in std::fs::read_dir(de)? {
        let e = e?;
        let p = e.path();
        if p.file_name().is_some_and(|n| n == ".git") {
            continue;
        }
        let t = e.file_type()?;
        let dest = a.join(e.file_name());
        if t.is_symlink() {
            // un enlace puede apuntar fuera: se copia el enlace tal cual (la huella lo cuenta como enlace)
            std::os::unix::fs::symlink(std::fs::read_link(&p)?, &dest)?;
        } else if t.is_dir() {
            copiar(&p, &dest)?;
        } else {
            std::fs::copy(&p, &dest)?;
        }
    }
    Ok(())
}

/// Instala la carpeta `origen` como plugin en `base` (se copia). `confiar` = la persona ya dijo que sí a su
/// resumen: se guarda la huella de lo copiado. Sin eso queda instalado pero NO carga.
pub fn instalar_en(base: &Path, origen: &Path, confiar: bool, reemplazar: bool) -> Result<Plugin> {
    let (p, _) = leer(origen, base)?;
    let destino = base.join(&p.nombre);
    if destino.exists() {
        if !reemplazar {
            bail!("{}", crate::tr!("ya hay un plugin «{}» instalado (quítalo antes o reinstala con --reemplazar)", "a plugin «{}» is already installed (remove it first or reinstall with --replace)", p.nombre));
        }
        quitar_en(base, &p.nombre)?;
    }
    copiar(origen, &destino)?;
    if confiar {
        confiar_en(base, &p.nombre)?;
    }
    GENERACION.fetch_add(1, Ordering::Relaxed);
    let (p, _) = leer(&destino, base)?;
    Ok(p)
}

/// Quita un plugin: su carpeta EXACTA (`base/<nombre>`) y su confianza. Nada por patrón.
pub fn quitar_en(base: &Path, nombre: &str) -> Result<()> {
    if !nombre_valido(nombre) {
        bail!("{}", crate::tr!("nombre de plugin no válido: «{nombre}»", "invalid plugin name: «{nombre}»"));
    }
    let raiz = base.join(nombre);
    if !raiz.join("plugin.toml").is_file() && !raiz.join(".claude-plugin").is_dir() && leer(&raiz, base).is_err() {
        bail!("{}", crate::tr!("no hay un plugin «{nombre}» en {}", "there is no plugin «{nombre}» in {}", base.display()));
    }
    std::fs::remove_dir_all(&raiz)?;
    let mut v = confianzas(base);
    v.retain(|(n, _)| n != nombre);
    guardar_confianzas(base, &v)?;
    GENERACION.fetch_add(1, Ordering::Relaxed);
    Ok(())
}

// ── lo que consumen los demás módulos ──────────────────────────────────────────────────────────────

/// `hooks::load`: (carpeta del plugin, archivo de hooks) de los plugins que cargan.
pub fn hooks_de_plugins() -> Vec<(PathBuf, Value)> {
    cargados().into_iter().flat_map(|(p, c)| c.hooks.into_iter().map(move |h| (p.dir.clone(), h))).collect()
}

/// `skills::dirs`: las carpetas de skills de los plugins que cargan.
pub fn carpetas_de_skills() -> Vec<PathBuf> {
    cargados().into_iter().flat_map(|(_, c)| c.skills).collect()
}

/// `mcp::load`: los servidores MCP de los plugins que cargan, con el nombre `plugin_<plugin>_<servidor>`
/// (las herramientas quedan `mcp__plugin_<plugin>_<servidor>__<herramienta>`, como en Claude Code).
pub fn servidores_mcp() -> Value {
    servidores_mcp_de(&cargados())
}

pub fn servidores_mcp_de(cargados: &[(Plugin, Contenido)]) -> Value {
    let mut m = Map::new();
    for (p, c) in cargados {
        for (n, s) in &c.mcp {
            m.insert(format!("plugin_{}_{n}", p.nombre), s.clone());
        }
    }
    json!({"mcpServers": m})
}

/// `reglas`, `paquetes` o `agentes` de los plugins que cargan.
pub fn carpetas(que: &str) -> Vec<PathBuf> {
    cargados()
        .into_iter()
        .flat_map(|(_, c)| match que {
            "reglas" => c.reglas,
            "paquetes" => c.paquetes,
            "agentes" => c.agentes,
            _ => vec![],
        })
        .collect()
}

fn fijo(s: String) -> &'static str {
    Box::leak(s.into_boxed_str())
}

/// Los servidores de lenguaje de plugins como filas del registro (F5). Sólo los que tienen su programa: uno
/// que falta no tapa al servidor de GOTV para esas extensiones.
pub fn defs_lsp_de(cargados: &[(Plugin, Contenido)]) -> Vec<Def> {
    let mut out = Vec::new();
    for (p, c) in cargados {
        for l in &c.lsp {
            let id = fijo(format!("plugin:{}:{}", p.nombre, l.servidor));
            let d = Def {
                id,
                lang: id,
                exts: Box::leak(l.extensiones.iter().map(|(e, _)| fijo(e.trim_start_matches('.').to_lowercase())).collect::<Vec<_>>().into_boxed_slice()),
                candidatos: vec![l.comando.clone()],
                raices: Box::leak(l.raices.iter().map(|r| fijo(r.clone())).collect::<Vec<_>>().into_boxed_slice()),
                mas_alta: false,
                instalar: fijo(crate::tr!("viene con el plugin «{}»", "comes with the plugin «{}»", p.nombre)),
                solo_abiertos: false,
                requiere: &[],
            };
            if crate::lsp::registro::comando(&d).is_some() {
                out.push(d);
            }
        }
    }
    out
}

/// Lo leído de los servidores de lenguaje de plugins, por generación (las cadenas fijas no se repiten).
struct CacheLsp {
    generacion: u64,
    base: PathBuf,
    defs: Vec<Def>,
    servidores: Vec<(String, LspPlugin)>,
    /// (id, extensión sin punto, `languageId`)
    lenguajes: Vec<(String, String, &'static str)>,
}
static CACHE_LSP: Mutex<Option<CacheLsp>> = Mutex::new(None);

fn con_cache<T>(f: impl FnOnce(&CacheLsp) -> T) -> T {
    let (g, base) = (GENERACION.load(Ordering::Relaxed), dir());
    let mut c = CACHE_LSP.lock().unwrap();
    if !c.as_ref().is_some_and(|c| c.generacion == g && c.base == base) {
        let cargados = if base.is_dir() { cargados_en(&base) } else { vec![] };
        let servidores: Vec<(String, LspPlugin)> = cargados.iter().flat_map(|(p, c)| c.lsp.iter().map(move |l| (format!("plugin:{}:{}", p.nombre, l.servidor), l.clone()))).collect();
        let lenguajes = servidores.iter().flat_map(|(id, l)| l.extensiones.iter().map(move |(e, lang)| (id.clone(), e.trim_start_matches('.').to_lowercase(), fijo(lang.clone())))).collect();
        *c = Some(CacheLsp { generacion: g, base, defs: defs_lsp_de(&cargados), servidores, lenguajes });
    }
    f(c.as_ref().unwrap())
}

/// `lsp::registro::defs`: las filas de los plugins (van primero).
pub fn defs_lsp() -> Vec<Def> {
    con_cache(|c| c.defs.clone())
}

/// El servidor de plugin que se lanza con este programa (para su entorno, opciones y ajustes).
pub fn lsp_de_comando(programa: &str) -> Option<LspPlugin> {
    con_cache(|c| c.servidores.iter().find(|(_, l)| l.comando.first().map(String::as_str) == Some(programa)).map(|(_, l)| l.clone()))
}

/// El `languageId` que el plugin declaró para esta extensión (`id` = `plugin:<p>:<servidor>`).
pub fn lenguaje_lsp(id: &str, ext: &str) -> Option<&'static str> {
    let ext = ext.to_lowercase();
    con_cache(|c| c.lenguajes.iter().find(|(i, e, _)| i == id && *e == ext).map(|(_, _, l)| *l))
}

#[cfg(test)]
mod tests;
