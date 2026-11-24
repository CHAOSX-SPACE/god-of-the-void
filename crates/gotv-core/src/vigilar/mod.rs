//! F3 · Vigilar y reaccionar: algo que pasa FUERA (una línea de error en un registro, un CI que termina,
//! un archivo que aparece, un mensaje de un WebSocket) despierta al agente sin que nadie le escriba.
//!
//! - **Una cola por chat** (la clave del dueño de sus procesos, `exec::Owner::key`): cada línea que casa con
//!   el patrón es un [`Evento`]; el agente los recibe como UN aviso ([`gotv_engine::Message::aviso`], no como
//!   mensaje de usuario: F0.10 midió que un mensaje de usuario a mitad de orden obliga a releer todo) entre
//!   dos pasos ([`tomar`]) o, si está libre, como una orden nueva que encola la ventana ([`hay_pendientes`]).
//! - **Frenos:** 3 vigilancias por chat; 5 minutos por omisión, 30 como máximo; líneas de más de 64 KiB se
//!   cortan ANTES de aplicar el patrón; más de 10 eventos por minuto se agrupan y lo de más va entero a
//!   archivo (F6); el vencimiento usa el reloj monotónico (`Instant`: con la Mac dormida no corre).
//! - **Fuentes** ([`fuentes`]): un proceso ya vivo de `exec.rs`, un comando nuevo (vale lo mismo que `bash`),
//!   archivos del proyecto (FSEvents por `notify`, filtrando lo ignorado con `ignore`), un WebSocket (sólo
//!   direcciones públicas o `localhost` explícito; vale lo mismo que la red) y un PR de GitHub con su CI.
use anyhow::{anyhow, bail, Result};
use regex::Regex;
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime};

pub mod fuentes;
pub mod reiniciar;

/// Vigilancias vivas por chat (anexo A: `[vigilar] max = 3`).
pub const MAX_POR_CHAT: usize = 3;
/// Minutos por omisión y tope (anexo A: `[vigilar] minutos = 5`).
pub const MINUTOS_DEF: u64 = 5;
pub const MINUTOS_MAX: u64 = 30;
/// Una línea se corta aquí ANTES de aplicarle el patrón (trampa 2: líneas enormes).
pub const TOPE_LINEA: usize = 64 * 1024;
/// Eventos por minuto y vigilancia que pasan sueltos; los demás se agrupan (trampa 5: ráfagas).
pub const RAFAGA_POR_MINUTO: usize = 10;
/// Eventos esperando en la cola de un chat; lo de más se agrupa igual que una ráfaga.
const COLA_MAX: usize = 100;
/// Lo que se muestra de cada evento al modelo (el evento completo, si es más largo, va a archivo).
const TEXTO_EVENTO_MAX: usize = 2_000;

/// De dónde vienen los eventos.
#[derive(Debug, Clone, PartialEq)]
pub enum Fuente {
    Comando(String),
    Ws(String),
    Archivos(String),
    Pr(u64),
    Proceso(u32),
}

impl Fuente {
    /// Cómo se nombra en los eventos: `registro.log`, `PR #12`, `proceso #3`…
    pub fn etiqueta(&self) -> String {
        match self {
            Fuente::Comando(c) => corto(c, 40),
            Fuente::Ws(u) => corto(u, 60),
            Fuente::Archivos(p) => corto(p, 60),
            Fuente::Pr(n) => format!("PR #{n}"),
            Fuente::Proceso(id) => crate::tr!("proceso #{id}", "process #{id}"),
        }
    }
}

pub struct Vigia {
    pub id: u32,
    pub fuente: Fuente,
    pub patron: Option<Regex>,
    pub vence: Instant,
}

#[derive(Debug, Clone)]
pub struct Evento {
    pub vigia: u32,
    pub cuando: SystemTime,
    pub texto: String,
}

struct Activa {
    vigia: Vigia,
    etiqueta: String,
    dueno: String,
    raiz: PathBuf,
    minutos: u64,
    tarea: Option<tokio::task::JoinHandle<()>>,
    /// con `Comando`: el proceso que lanzó la vigilancia (se para con ella)
    proceso: Option<u32>,
    /// ventana de agrupación: desde cuándo y cuántos van en este minuto
    minuto: (Instant, usize),
    /// lo que pasó del tope de la ventana: va entero a archivo
    agrupados: Vec<String>,
}

#[derive(Default)]
struct Chat {
    activas: Vec<Activa>,
    eventos: VecDeque<Evento>,
    /// avisos del programa (venció, terminó la fuente, no se pudo conectar)
    avisos: Vec<String>,
}

fn registro() -> &'static Mutex<HashMap<String, Chat>> {
    static R: OnceLock<Mutex<HashMap<String, Chat>>> = OnceLock::new();
    R.get_or_init(Default::default)
}

static SIGUIENTE: AtomicU32 = AtomicU32::new(1);

fn corto(s: &str, n: usize) -> String {
    let s = s.trim();
    if s.chars().count() <= n {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(n).collect::<String>())
    }
}

/// Corta en un borde de carácter, a lo sumo `tope` bytes.
fn cortar_bytes(s: &str, tope: usize) -> &str {
    if s.len() <= tope {
        return s;
    }
    let mut i = tope;
    while !s.is_char_boundary(i) {
        i -= 1;
    }
    &s[..i]
}

/// Una línea de la fuente `id`: se corta al tope, se le aplica el patrón y, si casa, es un evento (o se
/// agrupa si ya van más de [`RAFAGA_POR_MINUTO`] en este minuto).
pub fn entrar(dueno: &str, id: u32, linea: &str) {
    let linea = cortar_bytes(linea.trim_end_matches(['\r', '\n']), TOPE_LINEA);
    if linea.trim().is_empty() {
        return;
    }
    let mut r = registro().lock().unwrap();
    let Some(chat) = r.get_mut(dueno) else { return };
    let Some(a) = chat.activas.iter_mut().find(|a| a.vigia.id == id) else { return };
    let ahora = Instant::now();
    // vencida (aunque nadie la haya retirado aún): lo que llegue ya no cuenta
    if ahora >= a.vigia.vence || a.vigia.patron.as_ref().is_some_and(|p| !p.is_match(linea)) {
        return;
    }
    if ahora.duration_since(a.minuto.0) >= Duration::from_secs(60) {
        a.minuto = (ahora, 0);
    }
    a.minuto.1 += 1;
    if a.minuto.1 > RAFAGA_POR_MINUTO || chat.eventos.len() >= COLA_MAX {
        a.agrupados.push(linea.to_string());
        return;
    }
    chat.eventos.push_back(Evento { vigia: id, cuando: SystemTime::now(), texto: linea.to_string() });
}

/// Un aviso de la fuente `id` que no pasa por el patrón (la fuente terminó, se cerró la conexión…).
pub fn avisar(dueno: &str, id: u32, texto: &str) {
    let mut r = registro().lock().unwrap();
    let Some(chat) = r.get_mut(dueno) else { return };
    let Some(a) = chat.activas.iter().find(|a| a.vigia.id == id) else { return };
    let linea = format!("[{} #{id} · {}] {texto}", crate::i18n::t("Vigilancia", "Watch"), a.etiqueta);
    chat.avisos.push(linea);
}

/// Un aviso para el chat que no es de una vigilancia (un proceso con `reiniciar` que se relanzó).
pub fn avisar_chat(dueno: &str, texto: &str) {
    registro().lock().unwrap().entry(dueno.to_string()).or_default().avisos.push(texto.to_string());
}

/// Saca de la lista lo vencido (reloj monotónico) y lo deja dicho en los avisos.
fn revisar_vencidas(chat: &mut Chat, ahora: Instant) {
    let (vencidas, vivas): (Vec<Activa>, Vec<Activa>) = std::mem::take(&mut chat.activas).into_iter().partition(|a| ahora >= a.vigia.vence);
    chat.activas = vivas;
    for mut a in vencidas {
        let (id, et, min) = (a.vigia.id, a.etiqueta.clone(), a.minutos);
        // lo agrupado antes de vencer no se pierde
        if let Some(l) = linea_de_agrupados(&mut a) {
            chat.avisos.push(l);
        }
        apagar(a);
        chat.avisos.push(crate::tr!(
            "[Vigilancia #{id} · {et}] venció tras {min} min: ya no vigila. Si sigue haciendo falta, vuelve a iniciarla.",
            "[Watch #{id} · {et}] expired after {min} min: no longer watching. If still needed, start it again."
        ));
    }
}

/// Para la tarea de la fuente (y el proceso que lanzó, si fue un `comando`).
fn apagar(a: Activa) {
    if let Some(t) = a.tarea {
        t.abort();
    }
    // el proceso que lanzó: se para con sus hijos (fuera de un runtime, lo para el cierre del chat)
    if let Some(p) = a.proceso.and_then(|id| crate::exec::get(&a.dueno, id)) {
        if let Ok(rt) = tokio::runtime::Handle::try_current() {
            rt.spawn(async move { p.stop().await });
        }
    }
}

/// Si hubo ráfaga: lo agrupado va ENTERO a un archivo y queda una línea con cuántos y dónde.
fn linea_de_agrupados(a: &mut Activa) -> Option<String> {
    if a.agrupados.is_empty() {
        return None;
    }
    let n = a.agrupados.len();
    let todo = std::mem::take(&mut a.agrupados).join("\n");
    let dir = crate::salidas::carpeta(&a.raiz).join("vigilar");
    let ruta = dir.join(format!("vigilancia-{}-{}.log", a.vigia.id, SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0)));
    let donde = match std::fs::create_dir_all(&dir).and_then(|_| std::fs::write(&ruta, &todo)) {
        Ok(()) => ruta.strip_prefix(&a.raiz).unwrap_or(&ruta).display().to_string(),
        Err(_) => crate::i18n::t("(no se pudo guardar)", "(could not be saved)").to_string(),
    };
    let ultima = todo.lines().last().map(|l| corto(l, 300)).unwrap_or_default();
    Some(crate::tr!(
        "[Evento de vigilancia #{} · {}] +{n} más en ráfaga (más de {RAFAGA_POR_MINUTO} por minuto), completos en {donde}; el último: {ultima}",
        "[Watch event #{} · {}] +{n} more in a burst (over {RAFAGA_POR_MINUTO} per minute), all of them in {donde}; the last one: {ultima}",
        a.vigia.id,
        a.etiqueta
    ))
}

/// ¿Hay algo que contarle al agente? (vence lo que toque). La ventana lo pregunta cuando el agente está
/// libre para encolar un turno; entre pasos se usa [`tomar`].
pub fn hay_pendientes(dueno: &str) -> bool {
    let mut r = registro().lock().unwrap();
    let Some(chat) = r.get_mut(dueno) else { return false };
    revisar_vencidas(chat, Instant::now());
    !chat.eventos.is_empty() || !chat.avisos.is_empty() || chat.activas.iter().any(|a| !a.agrupados.is_empty())
}

/// Las vigilancias VIVAS del chat (vence lo que toque). La CLI espera mientras haya alguna o algo pendiente.
pub fn activas(dueno: &str) -> usize {
    let mut r = registro().lock().unwrap();
    let Some(chat) = r.get_mut(dueno) else { return 0 };
    revisar_vencidas(chat, Instant::now());
    chat.activas.len()
}

/// Lo más que puede pedir una config (también la de un proyecto ajeno): vigilancias por chat.
pub const MAX_TOPE: usize = 10;

/// `[vigilar] max, minutos` (anexo A): `~/.config/gotv/config.toml` ← `.gotv/config.toml` del proyecto
/// (manda el proyecto), siempre dentro de los topes fijos (1–10 vigilancias, 1–30 minutos).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Limites {
    pub max: usize,
    pub minutos: u64,
}

impl Default for Limites {
    fn default() -> Self {
        Limites { max: MAX_POR_CHAT, minutos: MINUTOS_DEF }
    }
}

impl Limites {
    pub fn cargar(raiz: &Path) -> Self {
        let mut l = Self::default();
        for c in [crate::config::global_path(), raiz.join(".gotv/config.toml")] {
            l.aplicar(&std::fs::read_to_string(c).unwrap_or_default());
        }
        l
    }

    pub fn aplicar(&mut self, toml_texto: &str) {
        let Ok(v) = toml_texto.parse::<toml::Table>() else { return };
        let Some(t) = v.get("vigilar").and_then(|a| a.as_table()) else { return };
        if let Some(x) = t.get("max").and_then(|x| x.as_integer()) {
            self.max = (x.max(1) as usize).min(MAX_TOPE);
        }
        if let Some(x) = t.get("minutos").and_then(|x| x.as_integer()) {
            self.minutos = (x.max(1) as u64).min(MINUTOS_MAX);
        }
    }
}

/// F3 · ML6: archivos que una vigilancia vio cambiar POR FUERA (el editor del usuario, `git`, otro
/// programa), para avisar a los servidores de lenguaje del chat: si no, siguen razonando sobre el texto
/// viejo. Cola por chat, sin repetir la misma ruta (gana el último tipo), con tope.
fn cola_archivos() -> &'static Mutex<HashMap<String, Vec<(std::path::PathBuf, u8)>>> {
    static C: OnceLock<Mutex<HashMap<String, Vec<(std::path::PathBuf, u8)>>>> = OnceLock::new();
    C.get_or_init(Default::default)
}

pub fn archivos_cambiados(dueno: &str, cambios: Vec<(std::path::PathBuf, u8)>) {
    let mut m = cola_archivos().lock().unwrap();
    let v = m.entry(dueno.to_string()).or_default();
    for (p, t) in cambios {
        v.retain(|(q, _)| q != &p);
        v.push((p, t));
    }
    let sobra = v.len().saturating_sub(COLA_MAX * 10);
    v.drain(..sobra);
}

/// Los archivos cambiados por fuera pendientes del chat (vacía su cola).
pub fn tomar_archivos(dueno: &str) -> Vec<(std::path::PathBuf, u8)> {
    cola_archivos().lock().unwrap().remove(dueno).unwrap_or_default()
}

/// Todo lo pendiente del chat, como UN texto para el agente (o `None`). Vacía la cola.
pub fn tomar(dueno: &str) -> Option<String> {
    tomar_en(dueno, Instant::now())
}

fn tomar_en(dueno: &str, ahora: Instant) -> Option<String> {
    let mut r = registro().lock().unwrap();
    let chat = r.get_mut(dueno)?;
    // las etiquetas ANTES de retirar lo vencido: lo que llegó a tiempo conserva su nombre
    let etiquetas: HashMap<u32, String> = chat.activas.iter().map(|a| (a.vigia.id, a.etiqueta.clone())).collect();
    revisar_vencidas(chat, ahora);
    let mut lineas = Vec::new();
    for e in chat.eventos.drain(..) {
        let et = etiquetas.get(&e.vigia).cloned().unwrap_or_default();
        lineas.push(format!("[{} #{} · {et}] {}", crate::i18n::t("Evento de vigilancia", "Watch event"), e.vigia, corto(&e.texto, TEXTO_EVENTO_MAX)));
    }
    for a in chat.activas.iter_mut() {
        if let Some(l) = linea_de_agrupados(a) {
            lineas.push(l);
        }
    }
    lineas.append(&mut chat.avisos);
    if lineas.is_empty() {
        return None;
    }
    lineas.push(crate::i18n::t(
        "(Esto lo vio una vigilancia tuya, nadie te escribió: reacciona si hace falta y sigue con la tarea.)",
        "(One of your watches saw this, nobody wrote to you: react if needed and carry on with the task.)",
    ).into());
    Some(lineas.join("\n"))
}

/// Las vigilancias vivas del chat, una por línea.
pub fn listar(dueno: &str) -> String {
    let mut r = registro().lock().unwrap();
    let Some(chat) = r.get_mut(dueno) else { return crate::i18n::t("(no hay vigilancias)", "(no watches)").into() };
    revisar_vencidas(chat, Instant::now());
    if chat.activas.is_empty() {
        return crate::i18n::t("(no hay vigilancias)", "(no watches)").into();
    }
    let ahora = Instant::now();
    chat.activas
        .iter()
        .map(|a| {
            let quedan = a.vigia.vence.saturating_duration_since(ahora).as_secs().div_ceil(60);
            let patron = a.vigia.patron.as_ref().map(|p| format!(" · /{}/", p.as_str())).unwrap_or_default();
            crate::tr!("#{} {}{patron} · quedan {quedan} min", "#{} {}{patron} · {quedan} min left", a.vigia.id, a.etiqueta)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Para una vigilancia del chat.
pub fn detener(dueno: &str, id: u32) -> Result<String> {
    let mut r = registro().lock().unwrap();
    let chat = r.get_mut(dueno).ok_or_else(|| anyhow!("{}", crate::tr!("no existe la vigilancia #{id}", "watch #{id} does not exist")))?;
    let i = chat.activas.iter().position(|a| a.vigia.id == id).ok_or_else(|| anyhow!("{}", crate::tr!("no existe la vigilancia #{id} (mira action=listar)", "watch #{id} does not exist (see action=listar)")))?;
    let a = chat.activas.remove(i);
    let et = a.etiqueta.clone();
    apagar(a);
    Ok(crate::tr!("⏹ vigilancia #{id} ({et}) detenida", "⏹ watch #{id} ({et}) stopped"))
}

/// Al cerrar un chat: se apagan sus vigilancias. `cola` = la parte final de la clave del dueño (`#chat-…`).
pub fn soltar_chat(cola: &str) {
    reiniciar::soltar_chat(cola);
    let mut r = registro().lock().unwrap();
    let claves: Vec<String> = r.keys().filter(|k| k.ends_with(cola)).cloned().collect();
    for k in claves {
        if let Some(chat) = r.remove(&k) {
            for a in chat.activas {
                apagar(a);
            }
        }
    }
}

/// Registra una vigilancia nueva (sin fuente aún) y devuelve su id. Falla si el chat ya tiene el tope.
fn registrar(dueno: &str, raiz: &Path, fuente: Fuente, patron: Option<Regex>, dura: Duration, minutos: u64) -> Result<u32> {
    let mut r = registro().lock().unwrap();
    let chat = r.entry(dueno.to_string()).or_default();
    revisar_vencidas(chat, Instant::now());
    let max = Limites::cargar(raiz).max;
    if chat.activas.len() >= max {
        bail!("{}", crate::tr!(
            "ya hay {max} vigilancias en este chat: detén alguna (action=detener) antes de iniciar otra",
            "this chat already has {max} watches: stop one (action=detener) before starting another"
        ));
    }
    let id = SIGUIENTE.fetch_add(1, Ordering::Relaxed);
    let etiqueta = fuente.etiqueta();
    chat.activas.push(Activa {
        vigia: Vigia { id, fuente, patron, vence: Instant::now() + dura },
        etiqueta,
        dueno: dueno.to_string(),
        raiz: raiz.to_path_buf(),
        minutos,
        tarea: None,
        proceso: None,
        minuto: (Instant::now(), 0),
        agrupados: Vec::new(),
    });
    Ok(id)
}

/// Le pone a la vigilancia su tarea (y el proceso que lanzó, si lo hay).
fn adjuntar(dueno: &str, id: u32, tarea: tokio::task::JoinHandle<()>, proceso: Option<u32>) {
    let mut r = registro().lock().unwrap();
    match r.get_mut(dueno).and_then(|c| c.activas.iter_mut().find(|a| a.vigia.id == id)) {
        Some(a) => {
            a.tarea = Some(tarea);
            a.proceso = proceso;
        }
        // se detuvo mientras arrancaba
        None => tarea.abort(),
    }
}

/// Quita una vigilancia que no llegó a arrancar.
fn deshacer(dueno: &str, id: u32) {
    let mut r = registro().lock().unwrap();
    if let Some(c) = r.get_mut(dueno) {
        c.activas.retain(|a| a.vigia.id != id);
    }
}

/// ¿Qué herramienta y argumentos «vale lo mismo» que esta vigilancia, para los permisos? `None` = sólo
/// mira (archivos del proyecto, un proceso propio, listar, detener) o un WebSocket a `localhost` explícito.
pub fn permiso_equivalente(args: &serde_json::Value) -> Option<(&'static str, serde_json::Value)> {
    if args["action"].as_str().unwrap_or("iniciar") != "iniciar" {
        return None;
    }
    if let Some(c) = args["comando"].as_str().filter(|c| !c.trim().is_empty()) {
        return Some(("bash", serde_json::json!({ "cmd": c })));
    }
    if let Some(u) = args["ws"].as_str().filter(|u| !u.trim().is_empty()) {
        if fuentes::localhost_explicito(u) {
            return None;
        }
        // la red se juzga como la de `web_fetch`: un host privado es un no duro
        let http = u.trim().replacen("wss://", "https://", 1).replacen("ws://", "http://", 1);
        return Some(("web_fetch", serde_json::json!({ "url": http })));
    }
    if let Some(n) = numero(&args["pr"]) {
        return Some(("github", serde_json::json!({ "action": "estado_ci", "numero": n })));
    }
    None
}

fn numero(v: &serde_json::Value) -> Option<u64> {
    v.as_u64().or_else(|| v.as_str().and_then(|s| s.trim().trim_start_matches('#').parse().ok()))
}

/// La ficha de la herramienta (va en `tools::schemas`; nace diferida).
pub fn esquema() -> serde_json::Value {
    use serde_json::json;
    let s = |d: &str| json!({ "type": "string", "description": d });
    let i = |d: &str| json!({ "type": "integer", "description": d });
    json!({
        "type": "function",
        "function": {
            "name": "vigilar",
            "description": crate::i18n::t(
                "Vigila algo de FUERA y te despierta cuando pasa, sin preguntar a cada rato: cada línea que casa con `patron` (regex) te llega sola como «[Evento de vigilancia #N · …]» entre dos pasos o cuando estás libre. action: iniciar (UNA fuente: comando = lo lanza y lee su salida · proceso = un #N tuyo ya vivo · archivos = patrón del proyecto, p. ej. «logs/*.log» · ws = ws:// o wss:// público o localhost · pr = número de PR de GitHub, su estado y su CI; minutos 5, máx. 30) · listar · detener (id). Hasta 3 por chat; ráfagas agrupadas.",
                "Watches something OUTSIDE and wakes you when it happens, without polling: each line matching `patron` (regex) reaches you by itself as «[Watch event #N · …]» between two steps or when you are idle. action: iniciar (ONE source: comando = launches it and reads its output · proceso = one of your live #N · archivos = a project pattern, e.g. «logs/*.log» · ws = a public or localhost ws:// or wss:// · pr = a GitHub PR number, its state and its CI; minutos 5, max 30) · listar · detener (id). Up to 3 per chat; bursts grouped.",
            ),
            "parameters": {
                "type": "object",
                "properties": {
                    "action": { "type": "string", "enum": ["iniciar", "listar", "detener"] },
                    "comando": s(crate::i18n::t("orden de shell a lanzar y vigilar (vale lo mismo que bash)", "shell command to launch and watch (same as bash)")),
                    "proceso": i(crate::i18n::t("un proceso tuyo ya vivo (#N de `process`)", "one of your live processes (#N from `process`)")),
                    "archivos": s(crate::i18n::t("patrón de archivos del proyecto", "project file pattern")),
                    "ws": s(crate::i18n::t("dirección ws:// o wss://", "ws:// or wss:// address")),
                    "pr": i(crate::i18n::t("número de PR de GitHub (del remoto origin)", "GitHub PR number (of the origin remote)")),
                    "patron": s(crate::i18n::t("regex: sólo las líneas que casan son eventos (sin él, todas)", "regex: only matching lines are events (without it, all)")),
                    "minutos": i(crate::i18n::t("cuánto vigila (5, máx. 30)", "how long it watches (5, max 30)")),
                    "id": i(crate::i18n::t("con detener: el número de la vigilancia", "with detener: the watch number")),
                },
                "required": ["action"],
            },
        },
    })
}

/// La herramienta `vigilar`.
pub async fn herramienta(dueno: &str, raiz: &Path, args: &serde_json::Value) -> Result<String> {
    match args["action"].as_str().unwrap_or("iniciar") {
        "listar" | "list" => Ok(listar(dueno)),
        "detener" | "stop" => {
            let id = numero(&args["id"]).ok_or_else(|| anyhow!("{}", crate::i18n::t("falta 'id' (el número de la vigilancia)", "missing 'id' (the watch number)")))? as u32;
            detener(dueno, id)
        }
        "iniciar" | "start" => {
            let minutos = args["minutos"].as_u64().or_else(|| args["minutos"].as_str().and_then(|s| s.trim().parse().ok())).unwrap_or_else(|| Limites::cargar(raiz).minutos).clamp(1, MINUTOS_MAX);
            iniciar(dueno, raiz, args, Duration::from_secs(minutos * 60), minutos).await
        }
        otra => bail!("{}", crate::tr!("acción desconocida «{otra}»: iniciar, listar o detener", "unknown action «{otra}»: iniciar, listar or detener")),
    }
}

/// `iniciar` con una duración dada (las pruebas la acortan).
pub(crate) async fn iniciar(dueno: &str, raiz: &Path, args: &serde_json::Value, dura: Duration, minutos: u64) -> Result<String> {
    let g = |k: &str| args[k].as_str().map(str::trim).filter(|s| !s.is_empty()).map(String::from);
    let mut fuentes_dadas = Vec::new();
    if let Some(c) = g("comando") {
        fuentes_dadas.push(Fuente::Comando(c));
    }
    if let Some(u) = g("ws") {
        fuentes_dadas.push(Fuente::Ws(u));
    }
    if let Some(p) = g("archivos") {
        fuentes_dadas.push(Fuente::Archivos(p));
    }
    if let Some(n) = numero(&args["pr"]) {
        fuentes_dadas.push(Fuente::Pr(n));
    }
    if let Some(n) = numero(&args["proceso"]) {
        fuentes_dadas.push(Fuente::Proceso(n as u32));
    }
    let fuente = match fuentes_dadas.len() {
        1 => fuentes_dadas.remove(0),
        0 => bail!("{}", crate::i18n::t("falta la fuente: comando, ws, archivos, pr o proceso (una)", "missing the source: comando, ws, archivos, pr or proceso (one)")),
        _ => bail!("{}", crate::i18n::t("una sola fuente por vigilancia: comando, ws, archivos, pr o proceso", "one source per watch: comando, ws, archivos, pr or proceso")),
    };
    let patron = match g("patron") {
        Some(p) => Some(Regex::new(&p).map_err(|e| anyhow!("{}", crate::tr!("patrón inválido: {e}", "invalid pattern: {e}")))?),
        None => None,
    };
    // lo que se valida ANTES de registrar (para no gastar un hueco en algo que no arranca)
    match &fuente {
        Fuente::Proceso(pid) if crate::exec::get(dueno, *pid).is_none() => {
            bail!("{}", crate::tr!("no existe el proceso #{pid} en este chat (mira `process` action=list)", "process #{pid} does not exist in this chat (see `process` action=list)"))
        }
        Fuente::Archivos(p) => fuentes::validar_patron_archivos(raiz, p)?,
        Fuente::Ws(u) => fuentes::validar_ws(u).await?,
        _ => {}
    }
    let texto_patron = patron.as_ref().map(|p| crate::tr!(" · patrón /{}/", " · pattern /{}/", p.as_str())).unwrap_or_default();
    let id = registrar(dueno, raiz, fuente.clone(), patron, dura, minutos)?;
    let arranque = fuentes::arrancar(dueno, raiz, id, &fuente).await;
    let (tarea, proceso) = match arranque {
        Ok(x) => x,
        Err(e) => {
            deshacer(dueno, id);
            return Err(e);
        }
    };
    adjuntar(dueno, id, tarea, proceso);
    let et = fuente.etiqueta();
    let extra = proceso.map(|p| crate::tr!(" (lanzado como proceso #{p})", " (launched as process #{p})")).unwrap_or_default();
    Ok(crate::tr!(
        "👁 vigilancia #{id} iniciada: {et}{extra}{texto_patron} · vence en {minutos} min. Lo que vea te llegará solo, como «[Evento de vigilancia #{id} · …]», entre dos pasos o cuando estés libre: no hace falta preguntar con `process`.",
        "👁 watch #{id} started: {et}{extra}{texto_patron} · expires in {minutos} min. What it sees reaches you by itself, as «[Watch event #{id} · …]», between two steps or when you are idle: no need to poll with `process`."
    ))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use serde_json::json;

    /// `[vigilar] max, minutos`: por omisión 3 y 5; manda la config; un proyecto no pasa de los topes
    /// fijos (10 vigilancias, 30 minutos) ni baja de 1; una tabla rota no cambia nada. Y la config con
    /// `[vigilar]` sigue valiendo entera (no se ignora el archivo por la tabla nueva).
    #[test]
    fn los_limites_de_la_config() {
        let mut l = Limites::default();
        assert_eq!(l, Limites { max: 3, minutos: 5 });
        l.aplicar("[vigilar]\nmax = 5\nminutos = 12\n");
        assert_eq!(l, Limites { max: 5, minutos: 12 });
        l.aplicar("[vigilar]\nmax = 500\nminutos = 9999\n");
        assert_eq!(l, Limites { max: MAX_TOPE, minutos: MINUTOS_MAX });
        l.aplicar("[vigilar]\nmax = 0\nminutos = -3\n");
        assert_eq!(l, Limites { max: 1, minutos: 1 });
        l.aplicar("[vigilar\nroto");
        assert_eq!(l, Limites { max: 1, minutos: 1 });
        let d = crate::CarpetaPrueba::nueva("gotv-vigilar-limites");
        std::fs::create_dir_all(d.join(".gotv")).unwrap();
        std::fs::write(d.join(".gotv/config.toml"), "mode = \"plan\"\n[vigilar]\nmax = 4\n").unwrap();
        assert_eq!(Limites::cargar(&d).max, 4);
        let c = crate::Config::load(&d);
        assert_eq!(c.mode, "plan", "la config con [vigilar] no se ignora: {:?}", c.warnings);
    }

    /// Un dueño propio por prueba: las pruebas corren en paralelo y comparten el registro.
    pub(crate) fn dueno(tag: &str) -> String {
        static N: AtomicU32 = AtomicU32::new(1);
        format!("/prueba/{tag}#chat-vig-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed))
    }

    /// Un programa de prueba ESCRITO EN RUST: el propio binario de pruebas, que con `GOTV_FALSO` hace los
    /// pasos que se le piden (como el servidor MCP de eco de F0.4). Pasos separados por `;`, campos por `|`:
    /// `d|ms` duerme · `e|texto` imprime una línea · `x|código` sale · `c|ruta|n|código` cuenta en `ruta` y,
    /// si iban menos de `n`, sale con `código`. Devuelve la orden para `exec::spawn`.
    pub(crate) fn falso(pasos: &str) -> String {
        assert!(!pasos.contains('\''));
        format!("GOTV_FALSO='{pasos}' '{}' --exact vigilar::tests::programa_falso --nocapture --quiet", std::env::current_exe().unwrap().display())
    }

    #[test]
    fn programa_falso() {
        use std::io::Write;
        let Ok(pasos) = std::env::var("GOTV_FALSO") else { return };
        for paso in pasos.split(';') {
            let c: Vec<&str> = paso.split('|').collect();
            match c.as_slice() {
                ["d", ms] => std::thread::sleep(Duration::from_millis(ms.parse().unwrap())),
                ["e", t] => {
                    println!("{t}");
                    let _ = std::io::stdout().flush();
                }
                ["x", codigo] => std::process::exit(codigo.parse().unwrap()),
                ["c", ruta, n, codigo] => {
                    let previas: u32 = std::fs::read_to_string(ruta).ok().and_then(|s| s.trim().parse().ok()).unwrap_or(0);
                    std::fs::write(ruta, (previas + 1).to_string()).unwrap();
                    if previas < n.parse().unwrap() {
                        std::process::exit(codigo.parse().unwrap());
                    }
                }
                otro => panic!("paso desconocido {otro:?}"),
            }
        }
        std::process::exit(0);
    }

    /// Espera (hasta `tope`) a que haya algo y lo toma.
    pub(crate) async fn esperar_evento(d: &str, tope: Duration) -> Option<String> {
        let t = Instant::now();
        while t.elapsed() < tope {
            if hay_pendientes(d) {
                // un respiro: que lleguen las líneas del mismo golpe
                tokio::time::sleep(Duration::from_millis(150)).await;
                return tomar(d);
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        None
    }

    fn falsa(d: &str, patron: Option<&str>, dura: Duration) -> u32 {
        registrar(d, Path::new("/tmp"), Fuente::Comando("eco".into()), patron.map(|p| Regex::new(p).unwrap()), dura, 5).unwrap()
    }

    #[test]
    fn el_patron_casa_y_no_casa() {
        let d = dueno("patron");
        let id = falsa(&d, Some(r"ERROR|panic"), Duration::from_secs(60));
        entrar(&d, id, "INFO todo bien");
        entrar(&d, id, "GET /salud 200");
        assert!(!hay_pendientes(&d), "nada casó: nada que contar");
        entrar(&d, id, "2026-11-16 ERROR: base caída\n");
        let t = tomar(&d).unwrap();
        assert!(t.contains(&format!("[Evento de vigilancia #{id} · eco] 2026-11-16 ERROR: base caída")) || t.contains(&format!("[Watch event #{id} · eco] 2026-11-16 ERROR: base caída")), "{t}");
        assert!(!t.contains("INFO"), "{t}");
        assert!(tomar(&d).is_none(), "la cola se vacía al tomarla");
        // sin patrón, toda línea no vacía es un evento
        let d2 = dueno("sin-patron");
        let id2 = falsa(&d2, None, Duration::from_secs(60));
        entrar(&d2, id2, "   ");
        entrar(&d2, id2, "hola");
        assert!(tomar(&d2).unwrap().contains("hola"));
        soltar_chat(&d);
        soltar_chat(&d2);
    }

    #[test]
    fn una_linea_enorme_se_corta_antes_del_patron() {
        let d = dueno("enorme");
        // el patrón casa con lo que va DESPUÉS del tope: tras cortar ya no casa
        let id = falsa(&d, Some("FINAL"), Duration::from_secs(60));
        let enorme = format!("{}FINAL", "x".repeat(TOPE_LINEA + 10));
        entrar(&d, id, &enorme);
        assert!(!hay_pendientes(&d), "el patrón no debe ver más allá de 64 KiB");
        // y lo que sí casa se guarda cortado (con caracteres de varios bytes en el borde)
        let id_b = {
            soltar_chat(&d);
            falsa(&d, Some("^ñ"), Duration::from_secs(60))
        };
        let multibyte = "ñ".repeat(TOPE_LINEA); // 2 bytes cada una: el tope cae en medio
        entrar(&d, id_b, &multibyte);
        let r = registro().lock().unwrap();
        let ev = &r.get(&d).unwrap().eventos[0];
        assert!(ev.texto.len() <= TOPE_LINEA && ev.texto.len() >= TOPE_LINEA - 1, "{}", ev.texto.len());
        drop(r);
        soltar_chat(&d);
    }

    #[test]
    fn una_rafaga_de_100_lineas_se_agrupa_y_lo_demas_va_a_archivo() {
        let raiz = crate::CarpetaPrueba::nueva("gotv-vigilar-rafaga");
        let d = dueno("rafaga");
        let id = registrar(&d, &raiz, Fuente::Archivos("registro.log".into()), Some(Regex::new("ERROR").unwrap()), Duration::from_secs(60), 5).unwrap();
        for i in 0..100 {
            entrar(&d, id, &format!("ERROR número {i}"));
        }
        let t = tomar(&d).unwrap();
        let sueltos = t.lines().filter(|l| l.contains("] ERROR número")).count();
        assert_eq!(sueltos, RAFAGA_POR_MINUTO, "{t}");
        assert!(t.contains("+90"), "{t}");
        assert!(t.contains("ERROR número 99"), "el último se nombra: {t}");
        // los 90 de más, ENTEROS en su archivo
        let ruta = t.split(".gotv/salidas/vigilar/").nth(1).unwrap().split(';').next().unwrap().trim();
        let guardado = std::fs::read_to_string(raiz.join(".gotv/salidas/vigilar").join(ruta.trim_end_matches(','))).unwrap();
        assert_eq!(guardado.lines().count(), 90);
        assert!(guardado.starts_with("ERROR número 10\n"));
        // la ventana mide en «menos de 13 líneas»: la prueba entera cuesta lo que cuesta 11 eventos
        assert!(t.lines().count() <= RAFAGA_POR_MINUTO + 3, "{t}");
        soltar_chat(&d);
    }

    #[test]
    fn vence_con_el_reloj_monotonico_y_lo_dice() {
        let d = dueno("vence");
        let id = falsa(&d, None, Duration::from_millis(0));
        // ya vencida: lo que llegue no cuenta y se avisa una vez
        entrar(&d, id, "tarde");
        let t = tomar(&d).unwrap();
        assert!(t.contains("venció") || t.contains("expired"), "{t}");
        assert!(!t.contains("tarde"), "{t}");
        assert!(tomar(&d).is_none(), "el aviso es uno solo");
        assert!(listar(&d).contains("no hay") || listar(&d).contains("no watches"));
        // una viva NO vence antes de su hora (instante inyectado)
        let d2 = dueno("vence-2");
        let id2 = falsa(&d2, None, Duration::from_secs(300));
        entrar(&d2, id2, "a tiempo");
        let t2 = tomar_en(&d2, Instant::now() + Duration::from_secs(299)).unwrap();
        assert!(t2.contains("a tiempo") && !t2.contains("venció") && !t2.contains("expired"), "{t2}");
        let t3 = tomar_en(&d2, Instant::now() + Duration::from_secs(301)).unwrap();
        assert!(t3.contains("venció") || t3.contains("expired"), "{t3}");
    }

    #[test]
    fn tope_de_tres_por_chat_y_otro_chat_no_cuenta() {
        let d = dueno("tope");
        for _ in 0..MAX_POR_CHAT {
            falsa(&d, None, Duration::from_secs(60));
        }
        let e = registrar(&d, Path::new("/tmp"), Fuente::Pr(1), None, Duration::from_secs(60), 5).unwrap_err().to_string();
        assert!(e.contains('3'), "{e}");
        let otro = dueno("tope-otro");
        assert!(registrar(&otro, Path::new("/tmp"), Fuente::Pr(1), None, Duration::from_secs(60), 5).is_ok());
        // detener libera el hueco
        let id = registro().lock().unwrap().get(&d).unwrap().activas[0].vigia.id;
        assert!(detener(&d, id).is_ok());
        assert!(falsa(&d, None, Duration::from_secs(60)) > 0);
        soltar_chat(&d);
        soltar_chat(&otro);
        assert!(!hay_pendientes(&d));
    }

    #[test]
    fn permisos_equivalentes() {
        assert_eq!(permiso_equivalente(&json!({"action": "iniciar", "comando": "tail -f x.log"})), Some(("bash", json!({"cmd": "tail -f x.log"}))));
        assert_eq!(permiso_equivalente(&json!({"ws": "wss://ejemplo.com/eventos"})), Some(("web_fetch", json!({"url": "https://ejemplo.com/eventos"}))));
        assert_eq!(permiso_equivalente(&json!({"ws": "ws://localhost:8080/x"})), None);
        assert_eq!(permiso_equivalente(&json!({"ws": "ws://127.0.0.1:9/x"})), None);
        assert_eq!(permiso_equivalente(&json!({"pr": "#12"})).map(|x| x.0), Some("github"));
        assert_eq!(permiso_equivalente(&json!({"archivos": "logs/*.log"})), None);
        assert_eq!(permiso_equivalente(&json!({"action": "detener", "comando": "rm -rf /"})), None);
    }

    /// F3 paso 1 · la cola hacia el agente: lo visto a mitad de una orden entra ENTRE dos pasos como aviso
    /// (`Message::aviso`, no mensaje del usuario: F0.10) y, con el agente libre, es la orden siguiente.
    #[tokio::test]
    async fn el_evento_entra_entre_dos_pasos_como_aviso_y_libre_como_orden() {
        use crate::flujo::tests::{agente, bufer, llamadas, texto};
        use crate::flujo::SinVentana;
        let raiz = crate::CarpetaPrueba::nueva("gotv-vigilar-agente");
        let (esc, _) = bufer();
        let mut a = agente(
            &raiz,
            crate::permissions::Mode::Bypass,
            vec![
                llamadas(&[("vigilar", json!({"action": "iniciar", "comando": falso("d|600;e|ERROR: boom en el servidor;d|30000"), "patron": "ERROR"}))]),
                llamadas(&[("list_dir", json!({"path": ""}))]),
                texto("Vi el ERROR y lo arreglé."),
            ],
            SinVentana::new(Some(esc), None),
        );
        // cada paso del «modelo» tarda 1,5 s: el evento (≈0,7 s tras iniciar) cae entre el paso 2 y el 3
        a.engine.demora = Duration::from_millis(1500);
        a.tools_loaded.insert("vigilar".into());
        assert_eq!(a.run("arranca el servidor y vigílalo").await.unwrap(), "Vi el ERROR y lo arreglé.");
        let vistos = a.engine.vistos.lock().unwrap();
        assert_eq!(vistos.len(), 3);
        let ultimo = &vistos[2].messages;
        let aviso = ultimo.iter().find(|m| m.content.contains("ERROR: boom en el servidor")).expect("el evento debía llegar al modelo");
        assert!(aviso.es_aviso(), "como aviso, no como mensaje del usuario");
        // (el resultado de `vigilar` repite el comando cortado a 40 caracteres: se busca el evento completo)
        assert!(!vistos[1].messages.iter().any(|m| m.content.contains("ERROR: boom en el servidor")), "en el paso 2 aún no había pasado");
        drop(vistos);
        // con el agente libre: lo siguiente que vea es la orden del próximo turno
        let clave = a.proc_owner();
        avisar_chat(&clave, "[Vigilancia #9 · x] algo pasó");
        let orden = crate::programar::chat::turno_pendiente(&mut a, crate::programar::chat::ahora()).unwrap();
        assert!(orden.contains("algo pasó"), "{orden}");
        assert!(crate::programar::chat::turno_pendiente(&mut a, crate::programar::chat::ahora()).is_none());
        crate::exec::kill_owner(&clave);
        soltar_chat(&clave);
    }

    #[tokio::test]
    async fn iniciar_valida_antes_de_gastar_un_hueco() {
        let raiz = crate::CarpetaPrueba::nueva("gotv-vigilar-valida");
        let d = dueno("valida");
        let e = herramienta(&d, &raiz, &json!({"action": "iniciar"})).await.unwrap_err().to_string();
        assert!(e.contains("comando, ws"), "{e}");
        let e = herramienta(&d, &raiz, &json!({"comando": "x", "archivos": "y"})).await.unwrap_err().to_string();
        assert!(e.contains("una sola") || e.contains("one source"), "{e}");
        let e = herramienta(&d, &raiz, &json!({"archivos": "*.log", "patron": "("})).await.unwrap_err().to_string();
        assert!(e.contains("patrón inválido") || e.contains("invalid pattern"), "{e}");
        let e = herramienta(&d, &raiz, &json!({"proceso": 999_999})).await.unwrap_err().to_string();
        assert!(e.contains("#999999"), "{e}");
        assert!(listar(&d).contains("no hay") || listar(&d).contains("no watches"));
    }
}
