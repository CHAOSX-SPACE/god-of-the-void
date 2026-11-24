//! C1 — Ejecución unificada (idea de Codex `exec_command`/`write_stdin` + el segundo
//! plano de Claude Code, mejorada): todo `bash` corre como un PROCESO con número.
//!
//! - Si termina dentro del plazo → devuelve su salida, como siempre.
//! - Si sigue vivo al vencer el plazo (o se pidió `background`) → NO se mata: queda como
//!   proceso #N con nombre, y el agente lo maneja con la herramienta `process`
//!   (`list`, `output` = sólo lo NUEVO desde la última lectura, `wait`, `write`, `stop`).
//! - `tty: true` lo corre en una terminal virtual (PTY): los instaladores que preguntan
//!   (`npm create vite`, `cargo generate`…) y los REPL funcionan; se les responde con `write`.
//! - Cada proceso vive en su propio grupo/sesión: `stop` y el cierre de GOTV matan también
//!   a sus hijos. Interrumpir el turno mata al que se estaba esperando.
//! - La salida completa queda en `.gotv/procesos/<n>.log`; en memoria sólo la cola.
//! - Las URLs locales que imprime (p. ej. `http://localhost:5173`) se muestran en la lista.
//! - AISLAMIENTO: cada proceso tiene DUEÑO (proyecto + chat). Un chat sólo ve, lee, escribe
//!   y para los suyos; al cerrarse el chat, los suyos mueren (`kill_owner`).

use anyhow::{anyhow, Result};
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::os::fd::{FromRawFd, OwnedFd};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use tokio::io::AsyncReadExt;
use tokio::sync::Notify;

/// Cola que se conserva en memoria por flujo.
const KEEP: usize = 256 * 1024;
/// Lo máximo que devuelve una lectura (el resto queda en el log).
const SHOW: usize = 12 * 1024;
/// Procesos vivos a la vez: más es casi siempre un bucle del modelo.
const MAX_LIVE: usize = 16;
/// Con terminal, tanto silencio tras imprimir algo = casi seguro espera una respuesta
/// (medido: `npm create vite` preguntó «Ok to proceed? (y)» a los 3 s y se esperaron 120).
const PROMPT_IDLE: Duration = Duration::from_secs(4);

/// Cómo terminó una espera.
#[derive(Debug, PartialEq)]
pub enum Waited {
    Exited,
    /// con tty: imprimió algo y se quedó quieto → espera una respuesta
    Prompt,
    Timeout,
}

#[derive(Default)]
struct Stream {
    buf: Vec<u8>,
    /// Posición absoluta de `buf[0]`.
    base: usize,
    /// Bytes recibidos en total.
    total: usize,
    /// Hasta dónde ya se le mostró al agente (absoluto).
    seen: usize,
}

impl Stream {
    fn push(&mut self, data: &[u8]) {
        self.total += data.len();
        self.buf.extend_from_slice(data);
        if self.buf.len() > 2 * KEEP {
            let cut = self.buf.len() - KEEP;
            self.buf.drain(..cut);
            self.base += cut;
        }
    }

    /// Lo nuevo desde la última lectura. F6: si es enorme, la CABEZA (un tercio) y la COLA (dos
    /// tercios) con lo omitido en medio; antes sólo la cola, y un error de compilación al principio de
    /// una salida larga se perdía de vista. `omitted` = bytes que no se muestran.
    fn take_new(&mut self) -> (String, String, usize) {
        let from = self.seen.max(self.base);
        let skipped_before = from - self.seen;
        let slice = &self.buf[from - self.base..];
        self.seen = self.total;
        if slice.len() <= SHOW {
            return (String::new(), String::from_utf8_lossy(slice).to_string(), skipped_before);
        }
        // si el principio ya se descartó de la memoria (salida gigante), no hay cabeza que enseñar
        let cabeza = if skipped_before == 0 { SHOW / 3 } else { 0 };
        let cola = SHOW - cabeza;
        let omitted = skipped_before + slice.len() - cabeza - cola;
        (
            String::from_utf8_lossy(&slice[..cabeza]).to_string(),
            String::from_utf8_lossy(&slice[slice.len() - cola..]).to_string(),
            omitted,
        )
    }

    fn all_text(&self) -> String {
        String::from_utf8_lossy(&self.buf).to_string()
    }
}

pub struct Proc {
    pub id: u32,
    /// Proyecto + chat al que pertenece (aislamiento).
    owner: String,
    pub name: String,
    pub cmd: String,
    pub cwd: PathBuf,
    pub tty: bool,
    pub started: Instant,
    /// Líder del grupo (= pid): `killpg` llega a todos sus hijos.
    pgid: i32,
    out: Mutex<Stream>,
    err: Mutex<Stream>,
    exit: Mutex<Option<String>>,
    exit_code: Mutex<Option<i32>>,
    /// Última vez que imprimió algo.
    last_out: Mutex<Instant>,
    changed: Notify,
    /// Extremo maestro del PTY para escribirle (sólo con `tty`).
    writer: Mutex<Option<std::fs::File>>,
    log: Mutex<Option<std::fs::File>>,
    pub log_path: PathBuf,
    /// C2: corre en la caja de arena.
    pub sandboxed: bool,
    /// C2: su credencial de red (lo permitido y lo que se bloqueó).
    net: Option<Arc<crate::netproxy::Grant>>,
    /// F16.4: tapa en su salida los valores de las variables guardadas que se le inyectaron.
    tapador: Option<Mutex<crate::escritorio::variables::Tapador>>,
}

impl Proc {
    pub fn alive(&self) -> bool {
        self.exit.lock().unwrap().is_none()
    }

    /// ¿Queda alguien vivo en SU grupo aunque el líder ya terminó? Medido (D10, 2026-11-09):
    /// `cd x && python3 -m http.server 8080 &` — el `&` propio del modelo hace que el líder salga
    /// al instante y el servidor siga en el grupo; GOTV miraba sólo al líder y lo dejaba vivo 1,5 h.
    pub fn group_alive(&self) -> bool {
        // SAFETY: la señal 0 sólo pregunta si el grupo existe; no lo toca.
        self.pgid > 0 && unsafe { libc::killpg(self.pgid, 0) } == 0
    }

    /// Vivo de verdad: el líder o cualquiera de su grupo.
    pub fn vigente(&self) -> bool {
        self.alive() || self.group_alive()
    }

    pub fn exit_label(&self) -> Option<String> {
        self.exit.lock().unwrap().clone()
    }

    pub fn exit_code(&self) -> Option<i32> {
        *self.exit_code.lock().unwrap()
    }

    fn feed(&self, data: &[u8], is_err: bool) {
        // F16.4: un valor guardado no llega ni al registro ni a la pantalla
        match &self.tapador {
            Some(t) => {
                let limpio = t.lock().unwrap().pasar(data, is_err);
                self.volcar(&limpio, is_err)
            }
            None => self.volcar(data, is_err),
        }
    }

    fn volcar(&self, data: &[u8], is_err: bool) {
        if data.is_empty() {
            return;
        }
        if let Some(f) = self.log.lock().unwrap().as_mut() {
            let _ = f.write_all(data);
        }
        if is_err { &self.err } else { &self.out }
            .lock()
            .unwrap()
            .push(data);
        *self.last_out.lock().unwrap() = Instant::now();
        self.changed.notify_waiters();
    }

    /// Las últimas líneas visibles (la pregunta de un instalador, el estado de un menú).
    pub fn screen_tail(&self, n: usize) -> String {
        let raw = self.out.lock().unwrap().all_text();
        let from = raw.len().saturating_sub(4096);
        let mut cut = from;
        while !raw.is_char_boundary(cut) {
            cut += 1;
        }
        let clean = strip_ansi(&raw[cut..]);
        let lines: Vec<&str> = clean
            .lines()
            .map(str::trim_end)
            .filter(|l| !l.trim().is_empty())
            .collect();
        lines[lines.len().saturating_sub(n)..].join("\n")
    }

    /// F4 · Ctrl-C SOLO (SIGINT al grupo), sin escalar como `stop`: un kernel de Jupyter lo vuelve
    /// `KeyboardInterrupt` en la celda que corre y sigue vivo con su estado.
    pub fn interrumpir(&self) {
        if self.vigente() {
            // SAFETY: señal al grupo que este módulo creó (pgid = pid del líder).
            unsafe {
                libc::killpg(self.pgid, libc::SIGINT);
            }
        }
    }

    /// F4 · Las últimas `n` líneas de stdout Y stderr juntas (por qué no arrancó un kernel), sin leerlas.
    pub fn cola(&self, n: usize) -> String {
        let mut t = self.out.lock().unwrap().all_text();
        t.push_str(&self.err.lock().unwrap().all_text());
        let limpio = strip_ansi(&t);
        let l: Vec<&str> = limpio.lines().map(str::trim_end).filter(|l| !l.trim().is_empty()).collect();
        l[l.len().saturating_sub(n)..].join("\n")
    }

    /// Las URLs locales que imprimió (servidores de desarrollo).
    pub fn urls(&self) -> Vec<String> {
        let mut text = self.out.lock().unwrap().all_text();
        text.push_str(&self.err.lock().unwrap().all_text());
        local_urls(&strip_ansi(&text))
    }

    /// Lo NUEVO desde la última lectura, limpio para el modelo.
    pub fn take_new(&self) -> String {
        let (oc, o, om) = self.out.lock().unwrap().take_new();
        let (ec, e, em) = self.err.lock().unwrap().take_new();
        let limpio = |t: String| if self.tty { strip_ansi(&t) } else { t };
        let aviso = |kb: usize| {
            crate::tr!(
                "…[se omitieron {} KB; completo en {} · léelo con read_file (offset/limit) o búscalo con grep]",
                "…[{} KB omitted; full log at {} · read it with read_file (offset/limit) or search it with grep]",
                kb.max(1),
                self.log_path.display()
            )
        };
        let mut text = String::new();
        let mut un_flujo = |nombre: &str, cabeza: String, cola: String, omitido: usize| {
            if cola.trim().is_empty() && cabeza.trim().is_empty() && omitido == 0 {
                return;
            }
            if !nombre.is_empty() {
                text.push_str(&format!("\n[{nombre}]\n"));
            }
            if omitido > 0 {
                if cabeza.is_empty() {
                    text.push_str(&aviso(omitido / 1024));
                    text.push('\n');
                } else {
                    text.push_str(limpio(cabeza).trim_end());
                    text.push('\n');
                    text.push_str(&aviso(omitido / 1024));
                    text.push('\n');
                }
            }
            text.push_str(limpio(cola).trim_end());
        };
        un_flujo("", oc, o, om);
        un_flujo("stderr", ec, e, em);
        text
    }

    /// Espera a que salga o a que pase `d`. Devuelve si salió.
    pub async fn wait_exit(&self, d: Duration) -> bool {
        let deadline = Instant::now() + d;
        loop {
            if !self.alive() {
                return true;
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return false;
            }
            let _ = tokio::time::timeout(
                left.min(Duration::from_millis(200)),
                self.changed.notified(),
            )
            .await;
        }
    }

    /// Espera a que salga, a que (con tty) se quede esperando una respuesta, o a `d`.
    pub async fn wait_exit_or_prompt(&self, d: Duration) -> Waited {
        let deadline = Instant::now() + d;
        loop {
            if !self.alive() {
                return Waited::Exited;
            }
            // con tty: imprimió y calla = pregunta. Sin tty: un SERVIDOR que ya publicó su URL
            // y calla está listo (medido: Vite listo en 1 s y se esperaban los 120 del plazo)
            let quiet = self.out.lock().unwrap().total > 0
                && self.last_out.lock().unwrap().elapsed() >= PROMPT_IDLE;
            if quiet && (self.tty || !self.urls().is_empty()) {
                return Waited::Prompt;
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Waited::Timeout;
            }
            let _ = tokio::time::timeout(
                left.min(Duration::from_millis(250)),
                self.changed.notified(),
            )
            .await;
        }
    }

    /// Espera salida NUEVA (o la salida del proceso) hasta `d`.
    pub async fn wait_output(&self, d: Duration) {
        let deadline = Instant::now() + d;
        loop {
            let has_new = {
                let o = self.out.lock().unwrap();
                let e = self.err.lock().unwrap();
                o.total > o.seen || e.total > e.seen
            };
            if has_new || !self.alive() {
                // un respiro para juntar el resto de la ráfaga
                tokio::time::sleep(Duration::from_millis(150)).await;
                return;
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return;
            }
            let _ = tokio::time::timeout(
                left.min(Duration::from_millis(200)),
                self.changed.notified(),
            )
            .await;
        }
    }

    /// Escribe en su entrada (sólo procesos con `tty`).
    pub fn write(&self, chars: &str) -> Result<()> {
        let mut w = self.writer.lock().unwrap();
        let Some(f) = w.as_mut() else {
            return Err(anyhow!("{}", crate::i18n::t(
                "este proceso no tiene terminal: lánzalo con tty=true para poder escribirle",
                "this process has no terminal: launch it with tty=true to be able to write to it",
            )));
        };
        if !self.alive() {
            return Err(anyhow!(
                "{}",
                crate::i18n::t("el proceso ya terminó", "the process already ended")
            ));
        }
        // un teclado real manda \r con Enter: los menús en modo crudo (clack, inquirer…)
        // sólo aceptan \r (medido: create-vite recibió «demo\n», pintó «demo» y no avanzó);
        // en modo canónico la terminal convierte \r en \n (ICRNL), así sirve para ambos
        let bytes = chars.replace("\r\n", "\r").replace('\n', "\r");
        f.write_all(bytes.as_bytes())?;
        f.flush()?;
        Ok(())
    }

    /// Ctrl-C (SIGINT) al grupo y, si sigue, SIGQUIT; luego SIGTERM, y SIGKILL si no sale en 2 s.
    /// Prueba Cable (2026-11-13): el 35B dejó un bucle infinito en su analizador y gastó ~20 pasos
    /// adivinando dónde estaba; SIGTERM mata a Python en silencio. Con SIGINT, Python y Ruby imprimen
    /// la TRAZA de dónde estaban (KeyboardInterrupt / Interrupt), y con SIGQUIT Go vuelca sus pilas
    /// y Java su volcado de hilos: lo que se para dice dónde estaba, y llega al modelo con la salida.
    pub async fn stop(&self) {
        if !self.vigente() {
            return;
        }
        // Lo lanzado con `&` en un shell no interactivo IGNORA SIGINT y SIGQUIT: cada paso espera al
        // GRUPO entero (no sólo al líder) y, si sigue, pasa a la señal siguiente.
        for senal in [libc::SIGINT, libc::SIGQUIT, libc::SIGTERM] {
            // SAFETY: señal al grupo que este módulo creó (pgid = pid del líder).
            unsafe {
                libc::killpg(self.pgid, senal);
            }
            let plazo = if senal == libc::SIGTERM { 2_000 } else { 1_500 };
            if self.esperar_fin(Duration::from_millis(plazo)).await {
                // lo que imprimió al caer (la traza de Ctrl-C) llega con la salida
                self.wait_output(Duration::from_millis(200)).await;
                return;
            }
        }
        unsafe {
            libc::killpg(self.pgid, libc::SIGKILL);
        }
        self.esperar_fin(Duration::from_secs(2)).await;
    }

    /// Espera hasta `d` a que terminen el líder Y su grupo. `true` si ya no queda nadie.
    async fn esperar_fin(&self, d: Duration) -> bool {
        let t0 = std::time::Instant::now();
        let _ = self.wait_exit(d).await;
        while t0.elapsed() < d {
            if !self.group_alive() {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        !self.group_alive()
    }

    fn kill_now(&self) {
        // SAFETY: ídem.
        unsafe {
            libc::killpg(self.pgid, libc::SIGKILL);
        }
    }

    /// Una línea para la lista.
    pub fn line(&self) -> String {
        let secs = self.started.elapsed().as_secs();
        let state = match self.exit_label() {
            None => crate::tr!("● vivo {}", "● running {}", fmt_secs(secs)),
            Some(_) if self.group_alive() => crate::tr!(
                "● sigue vivo en segundo plano (el comando terminó pero dejó procesos; `process` stop lo apaga) {}",
                "● still running in the background (the command ended but left processes; `process` stop shuts it down) {}",
                fmt_secs(secs)
            ),
            Some(l) => format!("○ {l}"),
        };
        let urls = self.urls();
        let u = if urls.is_empty() {
            String::new()
        } else {
            format!(" · {}", urls.join(" "))
        };
        let tty = if self.tty { " [tty]" } else { "" };
        format!(
            "#{} «{}» {state}{tty}{u}\n   $ {}",
            self.id,
            self.name,
            short(&self.cmd, 160)
        )
    }
}

fn fmt_secs(s: u64) -> String {
    if s < 60 {
        format!("{s} s")
    } else if s < 3600 {
        format!("{} min", s / 60)
    } else {
        format!("{} h {} min", s / 3600, (s % 3600) / 60)
    }
}

fn short(s: &str, n: usize) -> String {
    let one = s.replace('\n', " ⏎ ");
    if one.chars().count() <= n {
        one
    } else {
        format!("{}…", one.chars().take(n).collect::<String>())
    }
}

/// C2: la caja de cada chat (la fija el agente antes de cada herramienta).
fn specs() -> &'static Mutex<std::collections::HashMap<String, crate::sandbox::Spec>> {
    static S: OnceLock<Mutex<std::collections::HashMap<String, crate::sandbox::Spec>>> =
        OnceLock::new();
    S.get_or_init(|| Mutex::new(std::collections::HashMap::new()))
}

/// C2: con qué caja corren los comandos de este dueño (`None` = sin caja).
pub fn set_sandbox(owner: &str, spec: Option<crate::sandbox::Spec>) {
    let mut m = specs().lock().unwrap();
    match spec {
        Some(s) => {
            m.insert(owner.to_string(), s);
        }
        None => {
            m.remove(owner);
        }
    }
}

/// Las notas de la caja tras un comando: red bloqueada (con una línea que el agente lee
/// para preguntarte) y escrituras negadas.
fn notes(p: &Proc, out: &str) -> String {
    let mut s = String::new();
    if let Some(g) = &p.net {
        let b = g.blocked();
        if !b.is_empty() {
            let list: Vec<String> = b
                .iter()
                .map(|(h, n)| {
                    if *n > 1 {
                        format!("{h} ({n}×)")
                    } else {
                        h.clone()
                    }
                })
                .collect();
            s.push_str(&crate::tr!(
                "\n🌐 la caja bloqueó la red a: {} — GOTV le pregunta al usuario si permitirlo",
                "\n🌐 the sandbox blocked the network to: {} — GOTV asks the user whether to allow it",
                list.join(", ")
            ));
            s.push_str(&format!(
                "\n{NET_MARK}{}",
                b.iter()
                    .map(|(h, _)| h.as_str())
                    .collect::<Vec<_>>()
                    .join(",")
            ));
        }
    }
    if p.sandboxed {
        // K1: el aviso dice QUÉ bloqueó. Medido (charla de voz del Portador): un `sysctl` de
        // lectura salía como «bloqueó una escritura» y el modelo acabó pidiendo sudo
        match crate::sandbox::que_bloqueo(out) {
            Some(crate::sandbox::Bloqueo::Escritura) => s.push_str(crate::i18n::t(
                "\n🔒 la caja bloqueó una escritura: sólo se escribe dentro del proyecto (y nunca en .gotv, .git/hooks ni .git/config). Si de verdad hace falta fuera, repite con sandbox=false: el usuario decide.",
                "\n🔒 the sandbox blocked a write: only the project can be written (and never .gotv, .git/hooks or .git/config). If it truly must be outside, repeat with sandbox=false: the user decides.",
            )),
            Some(crate::sandbox::Bloqueo::Sistema) => s.push_str(crate::i18n::t(
                "\n🔒 la caja no deja leer ese dato o servicio del sistema (NO es una escritura ni hace falta sudo). Para MIRAR el sistema usa la orden de inspección SOLA, sin mezclarla con otras: `sysctl -n machdep.cpu.brand_string`, `ps aux | head`, `top -l 1`, `diskutil list`, `pmset -g batt`… — esas corren fuera de la caja sin pedir permiso.",
                "\n🔒 the sandbox does not allow reading that system data or service (it is NOT a write and sudo is not needed). To LOOK at the system use the inspection command ALONE, not mixed with others: `sysctl -n machdep.cpu.brand_string`, `ps aux | head`, `top -l 1`, `diskutil list`, `pmset -g batt`… — those run outside the sandbox without asking.",
            )),
            Some(crate::sandbox::Bloqueo::Ejecutar) => s.push_str(crate::i18n::t(
                "\n🔒 la caja no deja EJECUTAR ese programa (NO es una escritura). Si es para mirar el sistema (ps, top -l 1…), úsalo solo o con filtros (| grep, | head): corre fuera de la caja. Si es otra cosa y de verdad hace falta, repite con sandbox=false: el usuario decide.",
                "\n🔒 the sandbox does not allow RUNNING that program (it is NOT a write). If it is to look at the system (ps, top -l 1…), use it alone or with filters (| grep, | head): it runs outside the sandbox. Otherwise, if truly needed, repeat with sandbox=false: the user decides.",
            )),
            None => {}
        }
    }
    s
}

/// La línea con los dominios bloqueados (la lee el agente).
pub const NET_MARK: &str = "[gotv-red-bloqueada] ";

/// Los dominios bloqueados que trae una salida.
pub fn blocked_hosts(out: &str) -> Vec<String> {
    out.lines()
        .filter_map(|l| l.strip_prefix(NET_MARK))
        .flat_map(|l| l.split(',').map(|h| h.trim().to_string()))
        .filter(|h| !h.is_empty())
        .collect()
}

fn registry() -> &'static Mutex<BTreeMap<u32, Arc<Proc>>> {
    static R: OnceLock<Mutex<BTreeMap<u32, Arc<Proc>>>> = OnceLock::new();
    R.get_or_init(|| Mutex::new(BTreeMap::new()))
}

static NEXT: AtomicU32 = AtomicU32::new(1);

pub fn get(owner: &str, id: u32) -> Option<Arc<Proc>> {
    registry()
        .lock()
        .unwrap()
        .get(&id)
        .filter(|p| p.owner == owner)
        .cloned()
}

pub fn list(owner: &str) -> Vec<Arc<Proc>> {
    registry()
        .lock()
        .unwrap()
        .values()
        .filter(|p| p.owner == owner)
        .cloned()
        .collect()
}

fn all() -> Vec<Arc<Proc>> {
    registry().lock().unwrap().values().cloned().collect()
}

fn remove(id: u32) {
    registry().lock().unwrap().remove(&id);
}

/// Saca de la lista un proceso YA terminado de este dueño (la ventana, tras pararlo).
pub fn forget(owner: &str, id: u32) {
    if get(owner, id).is_some_and(|p| !p.alive()) {
        remove(id);
    }
}

/// Los vivos, para recordárselos al modelo en cada orden.
pub fn live_summary(owner: &str) -> Option<String> {
    let live: Vec<_> = list(owner).into_iter().filter(|p| p.vigente()).collect();
    if live.is_empty() {
        return None;
    }
    let lines: Vec<String> = live.iter().map(|p| p.line()).collect();
    Some(crate::tr!(
        "Procesos tuyos que siguen vivos (manéjalos con `process`):\n{}",
        "Your processes still running (manage them with `process`):\n{}",
        lines.join("\n")
    ))
}

/// Al cerrar GOTV: nadie queda huérfano.
pub fn kill_all() {
    for p in all() {
        if p.vigente() {
            p.kill_now();
        }
    }
    registry().lock().unwrap().clear();
}

/// Al cerrar un chat: mueren SUS procesos, los de los demás siguen.
pub fn kill_owner(owner: &str) {
    for p in list(owner) {
        if p.vigente() {
            p.kill_now();
        }
        remove(p.id);
    }
}

/// Identidad de un chat para sus procesos. Cada chat nuevo recibe la suya; al
/// soltarse (el chat se cierra) mata sus procesos. Los de otros chats siguen.
pub struct Owner(String);

impl Default for Owner {
    fn default() -> Self {
        static N: AtomicU32 = AtomicU32::new(1);
        Owner(format!(
            "chat-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ))
    }
}

impl Owner {
    /// Clave completa: proyecto + chat.
    pub fn key(&self, root: &Path) -> String {
        format!("{}#{}", root.display(), self.0)
    }
}

impl Drop for Owner {
    fn drop(&mut self) {
        // W1: su navegador también se cierra
        crate::browser::close_session(&self.0);
        let tail = format!("#{}", self.0);
        // F3: sus vigilancias y supervisores se apagan ANTES de matar (nadie relanza en un chat cerrado)
        crate::vigilar::soltar_chat(&tail);
        let mine: Vec<Arc<Proc>> = all()
            .into_iter()
            .filter(|p| p.owner.ends_with(&tail))
            .collect();
        for p in mine {
            if p.vigente() {
                p.kill_now();
            }
            remove(p.id);
        }
    }
}

/// Mata (y saca de la lista) al proceso que se estaba esperando si el turno se
/// interrumpe: la espera abandonada suelta esta guarda.
struct WaitGuard(Option<Arc<Proc>>);

impl Drop for WaitGuard {
    fn drop(&mut self) {
        if let Some(p) = self.0.take() {
            p.kill_now();
            remove(p.id);
        }
    }
}

/// Lanza `cmd` como proceso #N.
pub fn spawn(
    owner: &str,
    cmd: &str,
    cwd: &Path,
    tty: bool,
    name: Option<&str>,
    unsandboxed: bool,
) -> Result<Arc<Proc>> {
    let live = list(owner).iter().filter(|p| p.vigente()).count();
    if live >= MAX_LIVE {
        return Err(anyhow!("{}", crate::tr!(
            "ya hay {live} procesos vivos: para alguno con `process` (action=stop) antes de lanzar otro",
            "there are already {live} live processes: stop one with `process` (action=stop) before launching another"
        )));
    }
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    let name = name
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| short(s, 40))
        .unwrap_or_else(|| {
            short(
                cmd.split_whitespace()
                    .take(3)
                    .collect::<Vec<_>>()
                    .join(" ")
                    .as_str(),
                40,
            )
        });
    let dir = cwd.join(".gotv/procesos");
    let _ = std::fs::create_dir_all(&dir);
    let log_path = dir.join(format!("{id}.log"));
    let log = std::fs::File::create(&log_path).ok();

    // C2: en la caja de su chat (si la hay y no se pidió salir de ella, con permiso)
    let spec = if unsandboxed {
        None
    } else {
        specs()
            .lock()
            .unwrap()
            .get(owner)
            .cloned()
            .filter(|_| crate::sandbox::available())
    };
    let (mut c, net) = match &spec {
        Some(sp) => {
            let g = crate::netproxy::grant(crate::sandbox::hosts(sp), sp.all_net);
            let port = crate::netproxy::ensure()?;
            let mut c = crate::sandbox::command(sp, cmd);
            for (k, v) in crate::netproxy::env_for(port, &g) {
                c.env(k, v);
            }
            (c, Some(g))
        }
        None => {
            let mut c = tokio::process::Command::new("bash");
            c.arg("-lc").arg(cmd);
            (c, None)
        }
    };
    c.current_dir(cwd);
    // F16.4: las variables guardadas del proyecto van en el ENTORNO del hijo, nunca en la orden (dentro de
    // la caja, sólo las que el Portador marcó)
    let variables = crate::escritorio::variables::para_hijo(cwd, spec.is_some());
    for (k, v) in &variables {
        c.env(k, v);
    }
    let tapador = crate::escritorio::variables::Tapador::de(&variables).map(Mutex::new);
    let mut master: Option<OwnedFd> = None;
    if tty {
        let (m, s) = open_pty()?;
        let s_out = s.try_clone()?;
        let s_err = s.try_clone()?;
        c.stdin(std::process::Stdio::from(s))
            .stdout(std::process::Stdio::from(s_out))
            .stderr(std::process::Stdio::from(s_err))
            .env("TERM", "xterm-256color")
            .env("COLUMNS", "120")
            .env("LINES", "40");
        // SAFETY: sólo llamadas async-signal-safe entre fork y exec.
        unsafe {
            c.pre_exec(|| {
                if libc::setsid() == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                // la terminal pasa a ser la de control: Ctrl-C (\x03) llega como SIGINT
                libc::ioctl(0, libc::TIOCSCTTY as _, 0);
                Ok(())
            });
        }
        master = Some(m);
    } else {
        c.stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .process_group(0);
        // F11: sin terminal, las variables de los paquetes del proyecto para que nada se quede esperando
        // una respuesta (`npm_config_yes`, `PIP_NO_INPUT`, `COMPOSER_NO_INTERACTION`…)
        for (k, v) in crate::lenguajes::variables(cwd) {
            c.env(k, v);
        }
    }
    let mut child = c.spawn().map_err(|e| {
        anyhow!(
            "{}",
            crate::tr!("no pude lanzar bash: {e}", "could not launch bash: {e}")
        )
    })?;
    // soltar el Command cierra NUESTRA copia del esclavo: así el maestro ve EOF al salir
    drop(c);
    let pgid = child.id().map(|p| p as i32).unwrap_or(-1);
    let writer = match &master {
        Some(m) => Some(std::fs::File::from(m.try_clone()?)),
        None => None,
    };
    let proc = Arc::new(Proc {
        id,
        owner: owner.to_string(),
        name,
        cmd: cmd.to_string(),
        cwd: cwd.to_path_buf(),
        tty,
        started: Instant::now(),
        pgid,
        out: Mutex::new(Stream::default()),
        err: Mutex::new(Stream::default()),
        exit: Mutex::new(None),
        exit_code: Mutex::new(None),
        last_out: Mutex::new(Instant::now()),
        changed: Notify::new(),
        writer: Mutex::new(writer),
        log: Mutex::new(log),
        log_path,
        sandboxed: spec.is_some(),
        net,
        tapador,
    });

    // lectores
    let mut readers = Vec::new();
    if let Some(m) = master {
        let p = proc.clone();
        let mut f = std::fs::File::from(m);
        // hilo PROPIO, no `spawn_blocking`: el runtime de tokio espera a sus hilos
        // bloqueantes al cerrarse, y este read sólo vuelve cuando el proceso muere
        // (medido: una prueba fallida dejó el runtime colgado 10 min)
        let (done_tx, done_rx) = tokio::sync::oneshot::channel::<()>();
        std::thread::spawn(move || {
            let mut chunk = [0u8; 8192];
            loop {
                match f.read(&mut chunk) {
                    // EIO = el último esclavo se cerró (el proceso salió)
                    Ok(0) | Err(_) => break,
                    Ok(n) => p.feed(&chunk[..n], false),
                }
            }
            let _ = done_tx.send(());
        });
        readers.push(tokio::spawn(async move {
            let _ = done_rx.await;
        }));
    } else {
        for (is_err, stream) in [
            (
                false,
                child
                    .stdout
                    .take()
                    .map(|s| Box::new(s) as Box<dyn tokio::io::AsyncRead + Unpin + Send>),
            ),
            (
                true,
                child
                    .stderr
                    .take()
                    .map(|s| Box::new(s) as Box<dyn tokio::io::AsyncRead + Unpin + Send>),
            ),
        ] {
            if let Some(mut r) = stream {
                let p = proc.clone();
                readers.push(tokio::spawn(async move {
                    let mut chunk = [0u8; 8192];
                    loop {
                        match r.read(&mut chunk).await {
                            Ok(0) | Err(_) => break,
                            Ok(n) => p.feed(&chunk[..n], is_err),
                        }
                    }
                }));
            }
        }
    }

    // quien espera la salida
    let p = proc.clone();
    tokio::spawn(async move {
        let st = child.wait().await;
        // un nieto en segundo plano puede retener las tuberías: espera breve
        let _ = tokio::time::timeout(Duration::from_secs(2), async {
            for r in readers {
                let _ = r.await;
            }
        })
        .await;
        let (label, code) = match st {
            Ok(s) => match s.code() {
                Some(c) => (format!("exit {c}"), Some(c)),
                None => (
                    crate::i18n::t("terminado por señal", "killed by a signal").to_string(),
                    None,
                ),
            },
            Err(e) => (
                crate::tr!("error esperando: {e}", "error while waiting: {e}"),
                None,
            ),
        };
        // F16.4: lo retenido por si empezaba un valor guardado (no lo era) sale ahora
        if let Some(t) = &p.tapador {
            let [o, e] = t.lock().unwrap().fin();
            p.volcar(&o, false);
            p.volcar(&e, true);
        }
        *p.exit_code.lock().unwrap() = code;
        *p.exit.lock().unwrap() = Some(label);
        // ya nadie escribe: soltar el maestro; su credencial de red deja de valer
        p.writer.lock().unwrap().take();
        if let Some(g) = &p.net {
            crate::netproxy::revoke(&g.token);
        }
        p.log.lock().unwrap().take();
        p.changed.notify_waiters();
    });

    registry().lock().unwrap().insert(id, proc.clone());
    Ok(proc)
}

/// (también la usa la terminal del usuario de la ventana, F16.7)
pub(crate) fn open_pty() -> Result<(OwnedFd, OwnedFd)> {
    let mut master: libc::c_int = -1;
    let mut slave: libc::c_int = -1;
    let mut ws = libc::winsize {
        ws_row: 40,
        ws_col: 120,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    // SAFETY: openpty escribe dos descriptores válidos o devuelve -1.
    let r = unsafe {
        libc::openpty(
            &mut master,
            &mut slave,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut ws,
        )
    };
    if r != 0 {
        return Err(anyhow!("openpty: {}", std::io::Error::last_os_error()));
    }
    // SAFETY: recién creados y nuestros.
    unsafe {
        libc::fcntl(master, libc::F_SETFD, libc::FD_CLOEXEC);
        Ok((OwnedFd::from_raw_fd(master), OwnedFd::from_raw_fd(slave)))
    }
}

/// `bash`: corre y espera hasta `timeout_s`. Si sigue vivo, lo deja como proceso.
#[allow(clippy::too_many_arguments)]
/// `./servidor.sh > registro.log &` (o con `nohup`): el modelo quiere un proceso de FONDO, pero un `&` de la
/// shell lo deja SUELTO — GOTV no lo ve, no se puede vigilar ni parar, y queda vivo al cerrar (banco F3 con
/// Huihui, 2026-11-17: 4 servidores huérfanos). Devuelve la orden sin el `&` final (ni `nohup`) para correrla
/// como proceso de GOTV en segundo plano; `None` si no termina en un `&` suelto (`&&` no cuenta).
pub fn fondo_suelto(cmd: &str) -> Option<String> {
    let c = cmd.trim_end();
    if c.contains('\n') || !c.ends_with('&') || c.ends_with("&&") || c.ends_with(">&") {
        return None;
    }
    let sin = c[..c.len() - 1].trim_end();
    let sin = sin.strip_prefix("nohup ").map(str::trim_start).unwrap_or(sin);
    (!sin.is_empty()).then(|| sin.to_string())
}

pub async fn run(
    owner: &str,
    cmd: &str,
    cwd: &Path,
    timeout_s: u64,
    background: bool,
    tty: bool,
    name: Option<&str>,
    unsandboxed: bool,
) -> Result<String> {
    let p = spawn(owner, cmd, cwd, tty, name, unsandboxed)?;
    if background {
        // un vistazo a lo primero que imprime (o a si murió al nacer)
        p.wait_exit(Duration::from_millis(1500)).await;
        return Ok(report(&p, true, false));
    }
    let mut guard = WaitGuard(Some(p.clone()));
    let waited = p.wait_exit_or_prompt(Duration::from_secs(timeout_s)).await;
    guard.0 = None;
    if waited == Waited::Exited {
        let out = p.take_new();
        remove(p.id);
        let n = notes(&p, &out);
        return Ok(format!(
            "({})\n{out}{n}",
            p.exit_label().unwrap_or_default()
        ));
    }
    Ok(report(&p, false, waited == Waited::Prompt))
}

fn prompt_head(p: &Proc) -> String {
    // un servidor que ya imprimió su URL y calla está LISTO, no preguntando
    let urls = p.urls();
    if !urls.is_empty() {
        return crate::tr!(
            "(▶ proceso #{} «{}» sigue vivo y en silencio: sirve en {}. Si es un servidor, ya está listo — \
             verifícalo de verdad (p. ej. bash `curl -s {}`). Si esperaba una respuesta, process action=write.)\n── pantalla ──\n{}",
            "(▶ process #{} «{}» is running and quiet: serving at {}. If it is a server, it is ready — \
             verify it for real (e.g. bash `curl -s {}`). If it was waiting for an answer, process action=write.)\n── screen ──\n{}",
            p.id,
            p.name,
            urls.join(" "),
            urls[0],
            p.screen_tail(8)
        );
    }
    let head = crate::tr!(
        "(⌨ proceso #{} «{}» espera una respuesta: lleva {} s quieto tras preguntar. Respóndele con \
         `process` action=write id={} chars=\"…\\n\" (Enter = \\n; en un menú, flechas = \\u001b[B abajo / \\u001b[A arriba \
         y luego \\n); si sólo tardaba, action=wait)",
        "(⌨ process #{} «{}» is waiting for an answer: quiet for {} s after asking. Answer it with \
         `process` action=write id={} chars=\"…\\n\" (Enter = \\n; in a menu, arrows = \\u001b[B down / \\u001b[A up \
         then \\n); if it was just slow, action=wait)",
        p.id,
        p.name,
        p.last_out.lock().unwrap().elapsed().as_secs(),
        p.id
    );
    // la pregunta SIEMPRE a la vista: la salida «nueva» puede venir vacía (medido: el
    // modelo esperó 4 veces sin ver qué le preguntaban)
    format!(
        "{head}\n{}\n{}",
        crate::i18n::t("── pantalla ──", "── screen ──"),
        p.screen_tail(8)
    )
}

fn report(p: &Proc, asked_background: bool, prompt: bool) -> String {
    let new = p.take_new();
    let head = match p.exit_label() {
        Some(l) => {
            remove(p.id);
            format!("({l}){}", notes(p, &new))
        }
        None if prompt => prompt_head(p),
        None if asked_background => crate::tr!(
            "(▶ en segundo plano como proceso #{} «{}»)",
            "(▶ in the background as process #{} «{}»)",
            p.id,
            p.name
        ),
        None => crate::tr!(
            "(⏳ sigue vivo tras {} s: NO lo maté, quedó como proceso #{} «{}». Para seguir: \
             `process` action=wait (esperar a que termine), action=output (ver lo nuevo), \
             action=write (responderle, si tiene tty) o action=stop (pararlo))",
            "(⏳ still running after {} s: I did NOT kill it, it is now process #{} «{}». Next: \
             `process` action=wait (wait for it), action=output (see what is new), \
             action=write (answer it, if it has a tty) or action=stop (stop it))",
            p.started.elapsed().as_secs(),
            p.id,
            p.name
        ),
    };
    let urls = p.urls();
    let new = if prompt { String::new() } else { new };
    let mut s = head;
    // prueba Cable (2026-11-13): unas pruebas que no terminan casi siempre son un bucle infinito
    if p.exit_label().is_none() && !asked_background && !prompt && crate::pruebas::es_comando_de_pruebas(&p.cmd) {
        s.push_str(crate::i18n::t(
            "\nSon pruebas y no terminan: lo normal es un BUCLE INFINITO en el código. No adivines dónde: páralo con `process` action=stop — GOTV lo interrumpe con Ctrl-C y te devuelve DÓNDE estaba (Python y Ruby imprimen la traza; Go y Java vuelcan sus pilas).",
            "\nThese are tests and they do not finish: usually an INFINITE LOOP in the code. Do not guess where: stop it with `process` action=stop — GOTV interrupts it with Ctrl-C and returns WHERE it was (Python and Ruby print the traceback; Go and Java dump their stacks).",
        ));
    }
    if !urls.is_empty() {
        s.push_str(&crate::tr!("\nURL: {}", "\nURL: {}", urls.join(" ")));
    }
    if !new.is_empty() {
        s.push('\n');
        s.push_str(&new);
    }
    s
}

/// La herramienta `process`.
pub async fn tool(owner: &str, args: &serde_json::Value) -> Result<String> {
    let action = args
        .get("action")
        .and_then(|v| v.as_str())
        .unwrap_or("list");
    if action == "list" {
        let mine = list(owner);
        if mine.is_empty() {
            return Ok(crate::i18n::t("(no hay procesos)", "(no processes)").into());
        }
        return Ok(mine.iter().map(|p| p.line()).collect::<Vec<_>>().join("\n"));
    }
    let id = args
        .get("id")
        .and_then(|v| {
            v.as_u64().or_else(|| {
                v.as_str()
                    .and_then(|s| s.trim_start_matches('#').parse().ok())
            })
        })
        .ok_or_else(|| {
            anyhow!(
                "{}",
                crate::i18n::t(
                    "falta 'id' (el número del proceso)",
                    "missing 'id' (the process number)"
                )
            )
        })? as u32;
    let p = get(owner, id).ok_or_else(|| {
        anyhow!(
            "{}",
            crate::tr!(
                "no existe el proceso #{id} (mira action=list)",
                "process #{id} does not exist (see action=list)"
            )
        )
    })?;
    let secs = |def: u64, max: u64| {
        args.get("timeout")
            .and_then(|v| v.as_u64())
            .unwrap_or(def)
            .clamp(1, max)
    };
    match action {
        "output" => {
            p.wait_output(Duration::from_secs(secs(3, 120))).await;
            Ok(status_then(&p))
        }
        "wait" => {
            let t = secs(60, 1800);
            let mut guard = WaitGuard(Some(p.clone()));
            let waited = p.wait_exit_or_prompt(Duration::from_secs(t)).await;
            guard.0 = None;
            if waited == Waited::Prompt {
                let _ = p.take_new();
                return Ok(prompt_head(&p));
            }
            if waited == Waited::Timeout {
                return Ok(format!(
                    "{}\n{}",
                    crate::tr!(
                        "(⏳ sigue vivo tras {t} s más)",
                        "(⏳ still running after {t} more s)"
                    ),
                    p.take_new()
                ));
            }
            Ok(status_then(&p))
        }
        "write" => {
            let chars = args.get("chars").and_then(|v| v.as_str()).unwrap_or("");
            p.write(chars)?;
            // lo que contesta: hasta que salga, vuelva a preguntar o pase el plazo
            p.wait_output(Duration::from_secs(2)).await;
            if p.wait_exit_or_prompt(Duration::from_secs(secs(30, 600)))
                .await
                == Waited::Prompt
            {
                let _ = p.take_new();
                return Ok(prompt_head(&p));
            }
            Ok(status_then(&p))
        }
        // F3 paso 7: relanzarlo si cae (siempre · si-falla · no)
        "reiniciar" => crate::vigilar::reiniciar::poner(owner, id, args.get("reiniciar").and_then(|v| v.as_str()).unwrap_or("siempre")),
        "stop" => {
            // parado a propósito: su supervisor (si lo tiene) no lo relanza
            crate::vigilar::reiniciar::quitar(id);
            p.stop().await;
            let out = p.take_new();
            remove(p.id);
            Ok(format!(
                "{}\n{out}",
                crate::tr!("(■ proceso #{id} parado)", "(■ process #{id} stopped)")
            ))
        }
        other => Err(anyhow!(
            "{}",
            crate::tr!(
                "acción desconocida: {other} (usa list, output, wait, write, stop, reiniciar)",
                "unknown action: {other} (use list, output, wait, write, stop, reiniciar)"
            )
        )),
    }
}

fn status_then(p: &Proc) -> String {
    let new = p.take_new();
    let head = match p.exit_label() {
        Some(l) => {
            remove(p.id);
            format!("(#{} {l}){}", p.id, notes(p, &new))
        }
        None => crate::tr!("(#{} ● vivo)", "(#{} ● running)", p.id),
    };
    if new.is_empty() {
        format!(
            "{head}\n{}",
            crate::i18n::t("(sin salida nueva)", "(no new output)")
        )
    } else {
        format!("{head}\n{new}")
    }
}

/// Quita los códigos de color/cursor de una terminal y resuelve los `\r` de las barras
/// de progreso (queda la última versión de cada línea).
pub fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        if c == '\x1b' {
            match it.peek() {
                Some('[') => {
                    it.next();
                    // CSI: parámetros hasta un byte final @..~
                    for d in it.by_ref() {
                        if ('@'..='~').contains(&d) {
                            break;
                        }
                    }
                }
                Some(']') => {
                    it.next();
                    // OSC: hasta BEL o ESC \
                    while let Some(d) = it.next() {
                        if d == '\x07' {
                            break;
                        }
                        if d == '\x1b' && it.peek() == Some(&'\\') {
                            it.next();
                            break;
                        }
                    }
                }
                Some(_) => {
                    it.next();
                }
                None => {}
            }
            continue;
        }
        out.push(c);
    }
    // \r\n → \n; un \r suelto reescribe la línea
    let mut lines = Vec::new();
    for line in out.replace("\r\n", "\n").split('\n') {
        let last = line.rsplit('\r').find(|p| !p.is_empty()).unwrap_or("");
        lines.push(last.to_string());
    }
    lines
        .join("\n")
        .chars()
        .filter(|c| *c == '\n' || *c == '\t' || !c.is_control())
        .collect()
}

/// `http://localhost:5173/`, `http://127.0.0.1:8000`… (sin repetir).
pub fn local_urls(text: &str) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    for start in [
        "http://localhost",
        "http://127.0.0.1",
        "http://0.0.0.0",
        "https://localhost",
        "http://[::1]",
    ] {
        let mut from = 0;
        while let Some(i) = text[from..].find(start) {
            let s = from + i;
            let end = text[s..]
                .find(|c: char| {
                    c.is_whitespace() || matches!(c, '"' | '\'' | ')' | '>' | '<' | ',')
                })
                .map(|e| s + e)
                .unwrap_or(text.len());
            let url = text[s..end]
                .trim_end_matches(['.', '/'])
                .replace("0.0.0.0", "localhost");
            if url.len() > start.len() && !found.contains(&url) {
                found.push(url);
            }
            from = end;
        }
    }
    found.truncate(4);
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn el_fondo_suelto_pasa_a_proceso_de_gotv() {
        assert_eq!(fondo_suelto("./servidor.sh > registro.log 2>&1 &").as_deref(), Some("./servidor.sh > registro.log 2>&1"));
        assert_eq!(fondo_suelto("nohup ./servidor.sh > registro.log 2>&1 &").as_deref(), Some("./servidor.sh > registro.log 2>&1"));
        assert_eq!(fondo_suelto("npm run dev &  ").as_deref(), Some("npm run dev"));
        assert_eq!(fondo_suelto("cargo build && cargo test"), None);
        assert_eq!(fondo_suelto("make 2>&1"), None);
        assert_eq!(fondo_suelto("a &\necho listo"), None, "varias líneas: no se toca");
        assert_eq!(fondo_suelto("&"), None);
    }

    /// De punta a punta por la herramienta: el `&` suelto queda como proceso de GOTV (se ve y se para).
    #[tokio::test]
    async fn bash_con_ampersand_queda_como_proceso_de_gotv() {
        let d = crate::CarpetaPrueba::nueva("gotv-exec-fondo");
        let r = crate::tools::run_tool("bash", &serde_json::json!({"cmd": "sleep 30 > salida.log &"}), &d).await;
        assert!(r.ok && (r.output.contains("segundo plano como proceso #") || r.output.contains("background as process #")), "{}", r.output);
        let l = crate::tools::run_tool("process", &serde_json::json!({"action": "list"}), &d).await;
        assert!(l.output.contains("sleep 30"), "{}", l.output);
        let id: u32 = r.output.split('#').nth(1).unwrap().split(|c: char| !c.is_ascii_digit()).next().unwrap().parse().unwrap();
        assert!(crate::tools::run_tool("process", &serde_json::json!({"action": "stop", "id": id}), &d).await.ok);
    }
    const DUE: &str = "/proyecto#chat-1";

    fn dir(n: &str) -> crate::CarpetaPrueba {
        let d = crate::CarpetaPrueba::sin_crear(&format!("gotv-exec-{n}"));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn limpia_colores_y_barras_de_progreso() {
        assert_eq!(strip_ansi("\x1b[32mlisto\x1b[0m\r\nok"), "listo\nok");
        assert_eq!(strip_ansi("10%\r50%\r100%\nfin"), "100%\nfin");
        assert_eq!(strip_ansi("\x1b]0;título\x07hola"), "hola");
    }

    #[test]
    fn encuentra_las_urls_locales() {
        let t = "  ➜  Local:   http://localhost:5173/\n  Network: use --host\nserving on http://0.0.0.0:8000.";
        assert_eq!(
            local_urls(t),
            vec!["http://localhost:5173", "http://localhost:8000"]
        );
    }

    /// D10 (medido): `servidor &` dentro del comando — el líder sale al instante y el servidor
    /// sigue en el grupo. GOTV debe verlo vivo, avisarlo y apagarlo con el grupo entero.
    #[tokio::test]
    async fn un_ampersand_propio_no_deja_huerfanos() {
        // su carpeta propia: el registro va a `<cwd>/.gotv/procesos/`, nunca a la raíz del temporal
        let d = crate::CarpetaPrueba::nueva("gotv-exec-amp");
        let p = spawn(
            "/t#chat-amp",
            "sleep 300 &",
            &d,
            false,
            Some("dormilón"),
            true,
        )
        .unwrap();
        for _ in 0..50 {
            if !p.alive() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert!(!p.alive(), "el líder (bash) terminó");
        assert!(
            p.group_alive() && p.vigente(),
            "pero el sleep sigue en su grupo"
        );
        assert!(
            live_summary("/t#chat-amp").is_some_and(|s| s.contains("sigue vivo en segundo plano"))
        );
        p.stop().await;
        assert!(!p.group_alive(), "stop apaga el grupo entero");
        kill_owner("/t#chat-amp");
    }

    #[tokio::test]
    async fn lo_que_termina_a_tiempo_devuelve_su_salida_y_no_queda_en_la_lista() {
        let d = dir("rapido");
        let r = run(
            DUE,
            "echo hola; echo mal >&2; exit 3",
            &d,
            10,
            false,
            false,
            None,
            false,
        )
        .await
        .unwrap();
        assert!(
            r.starts_with("(exit 3)") && r.contains("hola") && r.contains("[stderr]\nmal"),
            "{r}"
        );
        assert!(list(DUE)
            .iter()
            .all(|p| p.cmd != "echo hola; echo mal >&2; exit 3"));
    }

    /// Prueba Cable (2026-11-13): unas pruebas colgadas en un bucle infinito traen la pista y, al
    /// pararlas, la traza de DÓNDE estaban (Ctrl-C antes que SIGTERM).
    #[tokio::test]
    async fn parar_un_bucle_dice_donde_estaba() {
        let d = dir("bucle");
        std::fs::write(d.join("bucle_test.py"), "def analizar():\n    while True:\n        pass\n\nanalizar()\n").unwrap();
        let r = run(DUE, "python3 bucle_test.py", &d, 1, false, false, Some("bucle"), false).await.unwrap();
        assert!(r.contains("BUCLE INFINITO") || r.contains("INFINITE LOOP"), "{r}");
        let id = list(DUE).into_iter().find(|p| p.name == "bucle").expect("en la lista").id;
        let s = tool(DUE, &serde_json::json!({"action": "stop", "id": id})).await.unwrap();
        assert!(s.contains("KeyboardInterrupt") && s.contains("bucle_test.py") && (s.contains("line 2") || s.contains("line 3")), "{s}");
    }

    #[tokio::test]
    async fn lo_largo_no_se_mata_queda_como_proceso_y_se_para() {
        let d = dir("largo");
        let marca = format!("gotv-largo-{}", std::process::id());
        let r = run(
            DUE,
            &format!("echo arranque; exec -a {marca} sleep 30"),
            &d,
            1,
            false,
            false,
            Some("dormilón"),
            false,
        )
        .await
        .unwrap();
        assert!(
            r.contains("sigue vivo") || r.contains("still running"),
            "{r}"
        );
        assert!(r.contains("arranque"), "{r}");
        let id = list(DUE)
            .into_iter()
            .find(|p| p.name == "dormilón")
            .expect("en la lista")
            .id;
        let s = tool(DUE, &serde_json::json!({"action": "stop", "id": id}))
            .await
            .unwrap();
        assert!(s.contains("parado") || s.contains("stopped"), "{s}");
        tokio::time::sleep(Duration::from_millis(300)).await;
        let vivos = std::process::Command::new("pgrep")
            .args(["-f", &marca])
            .output()
            .unwrap();
        assert!(vivos.stdout.is_empty(), "el sleep debió morir con su grupo");
        assert!(get(DUE, id).is_none());
    }

    #[tokio::test]
    async fn segundo_plano_muestra_la_url_y_output_trae_solo_lo_nuevo() {
        let d = dir("servidor");
        let r = run(
            DUE,
            "echo 'Local: http://localhost:5199/'; sleep 3; echo segunda; sleep 30",
            &d,
            120,
            true,
            false,
            Some("servidor"),
            false,
        )
        .await
        .unwrap();
        assert!(
            r.contains("segundo plano") || r.contains("background"),
            "{r}"
        );
        assert!(r.contains("http://localhost:5199"), "{r}");
        let id = list(DUE)
            .into_iter()
            .find(|p| p.name == "servidor")
            .unwrap()
            .id;
        let o = tool(
            DUE,
            &serde_json::json!({"action": "output", "id": id, "timeout": 5}),
        )
        .await
        .unwrap();
        assert!(
            o.contains("segunda") && !o.contains("Local:"),
            "sólo lo nuevo: {o}"
        );
        let l = tool(DUE, &serde_json::json!({"action": "list"}))
            .await
            .unwrap();
        assert!(
            l.contains("servidor") && l.contains("http://localhost:5199"),
            "{l}"
        );
        tool(DUE, &serde_json::json!({"action": "stop", "id": id}))
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn con_tty_se_le_responde_a_un_programa_que_pregunta() {
        let d = dir("tty");
        let r = run(
            DUE,
            "read -p '¿Nombre del proyecto? ' n; echo \"creado: $n\"; [ -t 0 ] && echo 'es terminal'",
            &d, 1, false, true, Some("pregunta"), false,
        )
        .await
        .unwrap();
        assert!(r.contains("Nombre del proyecto"), "la pregunta se ve: {r}");
        let id = list(DUE)
            .into_iter()
            .find(|p| p.name == "pregunta")
            .unwrap()
            .id;
        let w = tool(
            DUE,
            &serde_json::json!({"action": "write", "id": id, "chars": "vacio\n", "timeout": 5}),
        )
        .await
        .unwrap();
        assert!(
            w.contains("creado: vacio") && w.contains("es terminal"),
            "{w}"
        );
        let fin = tool(
            DUE,
            &serde_json::json!({"action": "wait", "id": id, "timeout": 5}),
        )
        .await;
        // ya salió: o lo reporta o ya no está en la lista
        assert!(fin.map(|s| s.contains("exit 0")).unwrap_or(true));
    }

    #[tokio::test]
    async fn interrumpir_la_espera_mata_al_proceso() {
        let d = dir("interrumpe");
        let marca = format!("gotv-int-{}", std::process::id());
        let cmd = format!("exec -a {marca} sleep 30");
        let fut = run(DUE, &cmd, &d, 60, false, false, Some("int"), false);
        let _ = tokio::time::timeout(Duration::from_millis(400), fut).await; // se abandona
        tokio::time::sleep(Duration::from_millis(300)).await;
        let vivos = std::process::Command::new("pgrep")
            .args(["-f", &marca])
            .output()
            .unwrap();
        assert!(
            vivos.stdout.is_empty(),
            "debió morir al abandonarse la espera"
        );
        assert!(list(DUE).iter().all(|p| p.name != "int"));
    }

    #[tokio::test]
    async fn la_salida_completa_queda_en_el_log() {
        let d = dir("log");
        let r = run(DUE, "seq 1 50000", &d, 30, false, false, None, false)
            .await
            .unwrap();
        assert!(r.contains("50000"));
        let logs: Vec<_> = std::fs::read_dir(d.join(".gotv/procesos"))
            .unwrap()
            .flatten()
            .collect();
        let full = std::fs::read_to_string(logs[0].path()).unwrap();
        assert!(full.starts_with("1\n2\n") && full.trim_end().ends_with("50000"));
    }

    #[tokio::test]
    async fn un_chat_no_ve_ni_toca_los_procesos_de_otro() {
        let d = dir("aislado");
        let r = run(
            "/a#chat-A",
            "sleep 30",
            &d,
            120,
            true,
            false,
            Some("de-A"),
            false,
        )
        .await
        .unwrap();
        assert!(r.contains("#"), "{r}");
        let id = list("/a#chat-A")[0].id;
        assert!(list("/b#chat-B").is_empty(), "otro proyecto no lo ve");
        assert!(
            list("/a#chat-A2").is_empty(),
            "otro chat del MISMO proyecto tampoco"
        );
        let e = tool(
            "/b#chat-B",
            &serde_json::json!({"action": "stop", "id": id}),
        )
        .await;
        assert!(e.is_err(), "ni puede pararlo");
        assert!(get("/a#chat-A", id).unwrap().alive());
        kill_owner("/a#chat-A");
        assert!(get("/a#chat-A", id).is_none());
    }

    #[tokio::test]
    async fn cerrar_el_chat_mata_sus_procesos() {
        let d = dir("cierre");
        let o = Owner::default();
        let key = o.key(&d);
        run(
            &key,
            "sleep 30",
            &d,
            120,
            true,
            false,
            Some("huérfano"),
            false,
        )
        .await
        .unwrap();
        let id = list(&key)[0].id;
        drop(o);
        assert!(
            get(&key, id).is_none(),
            "al cerrarse el chat su proceso muere"
        );
    }

    #[tokio::test]
    async fn con_tty_una_pregunta_devuelve_el_control_sin_esperar_el_plazo() {
        let d = dir("pregunta-rapida");
        let t0 = Instant::now();
        let r = run(
            DUE,
            "printf 'Ok to proceed? (y) '; read r; echo \"dijo $r\"",
            &d,
            120,
            false,
            true,
            Some("npx"),
            false,
        )
        .await
        .unwrap();
        assert!(
            t0.elapsed() < Duration::from_secs(10),
            "no debe esperar los 120 s: {:?}",
            t0.elapsed()
        );
        assert!(
            r.contains("Ok to proceed")
                && (r.contains("espera una respuesta") || r.contains("waiting for an answer")),
            "{r}"
        );
        let id = list(DUE).into_iter().find(|p| p.name == "npx").unwrap().id;
        let w = tool(
            DUE,
            &serde_json::json!({"action": "write", "id": id, "chars": "y\n"}),
        )
        .await
        .unwrap();
        assert!(w.contains("dijo y") && w.contains("exit 0"), "{w}");
    }

    #[tokio::test]
    async fn enter_llega_como_retorno_a_un_menu_en_modo_crudo() {
        let d = dir("crudo");
        // como clack/inquirer: terminal en modo crudo, sólo \r confirma
        let cmd = "python3 -I -c 'import sys,tty,termios,os\nfd=0; old=termios.tcgetattr(fd); sys.stdout.write(\"Nombre: \"); sys.stdout.flush(); tty.setraw(fd); n=b\"\"\nwhile True:\n  c=os.read(fd,1)\n  if c==b\"\\r\": break\n  n+=c\ntermios.tcsetattr(fd, termios.TCSADRAIN, old); print(); print(\"nombre=\"+n.decode())'";
        let r = run(DUE, cmd, &d, 60, false, true, Some("menu"), false)
            .await
            .unwrap();
        assert!(
            r.contains("espera una respuesta") && r.contains("Nombre:"),
            "la pregunta se ve: {r}"
        );
        let id = list(DUE).into_iter().find(|p| p.name == "menu").unwrap().id;
        let w = tool(
            DUE,
            &serde_json::json!({"action": "write", "id": id, "chars": "demo\n"}),
        )
        .await
        .unwrap();
        assert!(w.contains("nombre=demo"), "{w}");
    }

    #[tokio::test]
    async fn un_servidor_listo_y_callado_no_se_toma_por_pregunta() {
        let d = dir("listo");
        let yo = "/proyecto#chat-listo";
        let r = run(
            yo,
            "echo '  ➜  Local:   http://localhost:5299/'; sleep 30",
            &d,
            60,
            false,
            true,
            Some("vite"),
            false,
        )
        .await
        .unwrap();
        assert!(
            r.starts_with("(▶")
                && r.contains("http://localhost:5299")
                && !r.contains("espera una respuesta"),
            "{r}"
        );
        kill_owner(yo);
    }

    #[tokio::test]
    async fn un_servidor_sin_tty_devuelve_el_control_al_quedar_listo() {
        let d = dir("servidor-listo");
        let yo = "/proyecto#chat-servidor";
        let t0 = Instant::now();
        let r = run(
            yo,
            "echo 'Local: http://localhost:5311/'; sleep 60",
            &d,
            120,
            false,
            false,
            Some("dev"),
            false,
        )
        .await
        .unwrap();
        assert!(
            t0.elapsed() < Duration::from_secs(12),
            "no debe esperar el plazo: {:?}",
            t0.elapsed()
        );
        assert!(
            r.starts_with("(▶") && r.contains("http://localhost:5311"),
            "{r}"
        );
        kill_owner(yo);
    }

    #[tokio::test]
    async fn en_la_caja_lo_de_fuera_se_niega_y_se_explica() {
        if !crate::sandbox::available() {
            return;
        }
        let base =
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/pruebas-caja");
        let d = base.join(format!("exec-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join(".gotv")).unwrap();
        let d = d.canonicalize().unwrap();
        let yo = "/proyecto#chat-caja";
        set_sandbox(
            yo,
            Some(crate::sandbox::Spec {
                root: d.clone(),
                hosts: vec![],
                all_net: false,
                sockets: vec![],
                credenciales: vec![],
                entorno: vec![],
            }),
        );
        let r = run(
            yo,
            "echo dentro > a.txt && cat a.txt",
            &d,
            30,
            false,
            false,
            None,
            false,
        )
        .await
        .unwrap();
        assert!(r.starts_with("(exit 0)") && r.contains("dentro"), "{r}");
        let fuera = base.join(format!("exec-fuera-{}", std::process::id()));
        let r = run(
            yo,
            &format!("echo x > {}", fuera.display()),
            &d,
            30,
            false,
            false,
            None,
            false,
        )
        .await
        .unwrap();
        assert!(r.contains("🔒") && !fuera.exists(), "{r}");
        // red: pasa por el proxy con SU credencial; lo no permitido queda anotado
        let r = run(
            yo,
            "curl -sS -m 10 -o /dev/null -w '%{http_code}' https://github.com/ || true",
            &d,
            30,
            false,
            false,
            None,
            false,
        )
        .await
        .unwrap();
        assert_eq!(blocked_hosts(&r), vec!["github.com".to_string()], "{r}");
        // sandbox=false (con permiso del usuario): sale de la caja
        let r = run(
            yo,
            &format!("echo x > {} && echo ok", fuera.display()),
            &d,
            30,
            false,
            false,
            None,
            true,
        )
        .await
        .unwrap();
        assert!(r.contains("ok") && fuera.exists(), "{r}");
        let _ = std::fs::remove_file(&fuera);
        set_sandbox(yo, None);
    }
}
