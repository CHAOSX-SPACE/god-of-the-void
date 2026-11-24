//! F3 · las fuentes de una vigilancia. Cada una corre en su tarea de `tokio` y mete líneas con
//! [`super::entrar`] (pasan por el patrón) o avisos con [`super::avisar`] (no pasan).
use anyhow::{anyhow, bail, Result};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

/// Cada cuánto se mira el registro de un proceso.
const RITMO_PROCESO: Duration = Duration::from_millis(200);
/// Cada cuánto se revisan los archivos ya conocidos (por si FSEvents se calló alguno).
const RITMO_ARCHIVOS: Duration = Duration::from_secs(3);
/// PR y CI: cada 30 s con token; sin token, la API deja 60 consultas por hora → cada 3 min.
const RITMO_PR: Duration = Duration::from_secs(30);
const RITMO_PR_SIN_TOKEN: Duration = Duration::from_secs(180);
/// Lo más que se lee de un archivo de una vez (un registro que crece 1 GB entre dos miradas no tumba nada).
const LECTURA_MAX: u64 = 4 * 1024 * 1024;

/// Arranca la fuente. Devuelve su tarea y, con `comando`, el proceso que lanzó.
pub(super) async fn arrancar(dueno: &str, raiz: &Path, id: u32, fuente: &super::Fuente) -> Result<(tokio::task::JoinHandle<()>, Option<u32>)> {
    let d = dueno.to_string();
    Ok(match fuente {
        super::Fuente::Proceso(pid) => {
            let p = crate::exec::get(dueno, *pid).ok_or_else(|| anyhow!("{}", crate::tr!("no existe el proceso #{pid}", "process #{pid} does not exist")))?;
            // banco F3 (Huihui, 2026-11-17): `./servidor.sh > registro.log` vigilado como proceso no veía NADA
            // (su salida va al archivo): se sigue el archivo al que lo manda
            let ruta = salida_redirigida(&p.cmd, &p.cwd).unwrap_or_else(|| p.log_path.clone());
            // de lo que imprima DESDE ahora (lo de antes ya se pudo leer con `process`)
            let desde = std::fs::metadata(&ruta).map(|m| m.len()).unwrap_or(0);
            (tokio::spawn(seguir_proceso(d, id, p, ruta, desde)), None)
        }
        super::Fuente::Comando(cmd) => {
            let nombre = format!("vigilar #{id}");
            let p = crate::exec::spawn(dueno, cmd, raiz, false, Some(&nombre), false)?;
            let pid = p.id;
            let ruta = p.log_path.clone();
            (tokio::spawn(seguir_proceso(d, id, p, ruta, 0)), Some(pid))
        }
        super::Fuente::Archivos(patron) => {
            let v = Vigilante::nuevo(raiz, patron)?;
            (tokio::spawn(seguir_archivos(d, id, v)), None)
        }
        super::Fuente::Ws(url) => {
            let (ws, _) = tokio::time::timeout(Duration::from_secs(10), tokio_tungstenite::connect_async(url.as_str()))
                .await
                .map_err(|_| anyhow!("{}", crate::tr!("{url}: no respondió en 10 s", "{url}: no answer within 10 s")))?
                .map_err(|e| anyhow!("{}", crate::tr!("{url}: no pude conectar ({e})", "{url}: could not connect ({e})")))?;
            (tokio::spawn(seguir_ws(d, id, ws)), None)
        }
        super::Fuente::Pr(n) => {
            let repo = crate::github_pr::repo_del_proyecto(raiz).ok_or_else(|| {
                anyhow!("{}", crate::i18n::t("este proyecto no tiene un remoto `origin` de GitHub: no sé de qué repositorio es el PR", "this project has no GitHub `origin` remote: I do not know which repository the PR belongs to"))
            })?;
            let token = crate::github_pr::token().ok();
            let ritmo = if token.is_some() { RITMO_PR } else { RITMO_PR_SIN_TOKEN };
            (tokio::spawn(seguir_pr(d, id, crate::github_pr::base(), repo, *n, token, ritmo)), None)
        }
    })
}

// ── Proceso y comando: la cola del registro que `exec.rs` ya escribe ──────────────────────────────

/// Lee lo nuevo de `ruta` desde `desde`: devuelve las líneas COMPLETAS y el nuevo desplazamiento (lo que
/// queda sin salto de línea espera a la próxima lectura). Si el archivo se achicó, vuelve a empezar.
pub(crate) fn lineas_nuevas(ruta: &Path, desde: u64) -> (Vec<String>, u64) {
    use std::io::{Read, Seek, SeekFrom};
    let Ok(mut f) = std::fs::File::open(ruta) else { return (vec![], desde) };
    let largo = f.metadata().map(|m| m.len()).unwrap_or(0);
    let desde = if largo < desde { 0 } else { desde };
    if largo == desde {
        return (vec![], desde);
    }
    let hasta = largo.min(desde + LECTURA_MAX);
    let mut buf = vec![0u8; (hasta - desde) as usize];
    if f.seek(SeekFrom::Start(desde)).is_err() || f.read_exact(&mut buf).is_err() {
        return (vec![], desde);
    }
    // sólo hasta el último salto de línea (salvo que el trozo llene la lectura entera)
    let corte = match buf.iter().rposition(|b| *b == b'\n') {
        Some(i) => i + 1,
        None if buf.len() as u64 >= LECTURA_MAX => buf.len(),
        None => return (vec![], desde),
    };
    let texto = String::from_utf8_lossy(&buf[..corte]);
    let lineas = texto.lines().map(|l| crate::exec::strip_ansi(l).replace('\r', "")).collect();
    (lineas, desde + corte as u64)
}

/// ¿El comando manda su salida estándar a un archivo (`> x`, `>> x`, `&> x`)? El ÚLTIMO destino, resuelto contra
/// la carpeta del proceso; `/dev/null` y `2>` (sólo errores) no cuentan.
pub(crate) fn salida_redirigida(cmd: &str, cwd: &Path) -> Option<std::path::PathBuf> {
    let w: Vec<&str> = cmd.split_whitespace().collect();
    let mut destino = None;
    for (i, t) in w.iter().enumerate() {
        let (op, resto) = if let Some(r) = t.strip_prefix("&>>").or_else(|| t.strip_prefix("&>")) {
            (true, r)
        } else if t.starts_with("2>") {
            (false, "")
        } else if let Some(r) = t.strip_prefix("1>>").or_else(|| t.strip_prefix("1>")).or_else(|| t.strip_prefix(">>")).or_else(|| t.strip_prefix('>')) {
            (true, r)
        } else {
            (false, "")
        };
        if !op || resto.starts_with('&') {
            continue;
        }
        let objetivo = if resto.is_empty() { w.get(i + 1).copied().unwrap_or("") } else { resto };
        let objetivo = objetivo.trim_matches(['"', '\'']);
        if !objetivo.is_empty() && objetivo != "/dev/null" {
            destino = Some(crate::paths::resolve(cwd, objetivo));
        }
    }
    destino
}

async fn seguir_proceso(dueno: String, id: u32, p: Arc<crate::exec::Proc>, ruta: std::path::PathBuf, mut desde: u64) {
    loop {
        let vivo = p.vigente();
        let (lineas, nuevo) = lineas_nuevas(&ruta, desde);
        desde = nuevo;
        for l in &lineas {
            super::entrar(&dueno, id, l);
        }
        if !vivo && lineas.is_empty() {
            let como = p.exit_label().unwrap_or_default();
            super::avisar(&dueno, id, &crate::tr!("el proceso #{} terminó ({como})", "process #{} ended ({como})", p.id));
            return;
        }
        tokio::time::sleep(RITMO_PROCESO).await;
    }
}

// ── Archivos: FSEvents (notify) + .gitignore (ignore) ─────────────────────────────────────────────

/// ¿El patrón de archivos se queda dentro del proyecto?
pub(super) fn validar_patron_archivos(_raiz: &Path, patron: &str) -> Result<()> {
    let p = Path::new(patron);
    if p.is_absolute() || p.components().any(|c| matches!(c, std::path::Component::ParentDir)) {
        bail!("{}", crate::i18n::t("`archivos` es un patrón DENTRO del proyecto (p. ej. «logs/*.log»), sin rutas absolutas ni «..»", "`archivos` is a pattern INSIDE the project (e.g. «logs/*.log»), no absolute paths or «..»"));
    }
    glob::Pattern::new(patron).map_err(|e| anyhow!("{}", crate::tr!("patrón de archivos inválido: {e}", "invalid file pattern: {e}")))?;
    Ok(())
}

fn tiene_comodin(s: &str) -> bool {
    s.contains(['*', '?', '['])
}

/// El estado de una vigilancia de archivos: qué se conoce de cada uno y hasta dónde se leyó.
pub(crate) struct Vigilante {
    raiz: PathBuf,
    base: PathBuf,
    patron: glob::Pattern,
    /// el patrón nombra UN archivo: las líneas no llevan su nombre delante
    literal: bool,
    ignorados: Option<ignore::gitignore::Gitignore>,
    /// ruta relativa → (largo, fecha, leído hasta)
    vistos: HashMap<String, (u64, Option<SystemTime>, u64)>,
    /// F3 · ML6: lo que cambió de verdad desde la última vez (ruta absoluta, tipo de LSP: 1 creado, 2
    /// cambiado, 3 borrado), para avisar a los servidores de lenguaje que ya corren.
    pub(crate) cambios: Vec<(PathBuf, u8)>,
}

impl Vigilante {
    pub(crate) fn nuevo(raiz: &Path, patron: &str) -> Result<Vigilante> {
        let raiz = raiz.canonicalize().unwrap_or_else(|_| raiz.to_path_buf());
        // la base: el tramo literal del patrón que existe como carpeta (si no, la raíz)
        let mut base = raiz.clone();
        let comps: Vec<&str> = patron.split('/').collect();
        for c in &comps[..comps.len().saturating_sub(1)] {
            if tiene_comodin(c) || !base.join(c).is_dir() {
                break;
            }
            base = base.join(c);
        }
        // lo ignorado (.gitignore + lo de siempre), salvo que la base misma esté dentro de lo ignorado: si
        // el usuario pide vigilar `target/x.log`, lo pide a propósito
        let mut b = ignore::gitignore::GitignoreBuilder::new(&raiz);
        let _ = b.add(raiz.join(".gitignore"));
        for l in [".git/", "target/", "node_modules/", ".gotv/"] {
            let _ = b.add_line(None, l);
        }
        let gi = b.build().ok();
        let base_ignorada = base != raiz && gi.as_ref().is_some_and(|g| g.matched_path_or_any_parents(&base, true).is_ignore());
        let mut v = Vigilante {
            literal: !tiene_comodin(patron),
            patron: glob::Pattern::new(patron)?,
            ignorados: if base_ignorada { None } else { gi },
            raiz,
            base,
            vistos: HashMap::new(),
            cambios: Vec::new(),
        };
        // foto inicial: lo que ya existe cuenta desde su final (sólo lo NUEVO es evento)
        for e in ignore::WalkBuilder::new(&v.base).hidden(false).git_ignore(false).ignore(false).parents(false).build().flatten() {
            let ruta = e.path().to_path_buf();
            if let Some(rel) = v.relativa(&ruta) {
                if let Ok(m) = std::fs::metadata(&ruta) {
                    if m.is_file() {
                        v.vistos.insert(rel, (m.len(), m.modified().ok(), m.len()));
                    }
                }
            }
        }
        Ok(v)
    }

    /// La ruta relativa si es de las vigiladas (casa con el patrón y no está ignorada).
    fn relativa(&self, ruta: &Path) -> Option<String> {
        let rel = ruta.strip_prefix(&self.raiz).ok()?;
        if let Some(g) = &self.ignorados {
            if g.matched_path_or_any_parents(rel, ruta.is_dir()).is_ignore() {
                return None;
            }
        }
        let s = rel.to_string_lossy().to_string();
        self.patron.matches(&s).then_some(s)
    }

    /// Qué pasó con `ruta` desde la última vez: líneas (con patrón) o «apareció / cambió / se borró».
    /// `con_lineas` = la vigilancia tiene patrón de texto: interesa lo que se escribe, no el hecho.
    pub(crate) fn revisar(&mut self, ruta: &Path, con_lineas: bool) -> Vec<String> {
        let Some(rel) = self.relativa(ruta) else { return vec![] };
        let completa = self.raiz.join(&rel);
        let antes = self.vistos.get(&rel).copied();
        let ahora = std::fs::metadata(&completa).ok().filter(|m| m.is_file());
        let pre = |l: String| if self.literal { l } else { format!("{rel}: {l}") };
        match (antes, ahora) {
            (None, None) => vec![],
            (Some(_), None) => {
                self.vistos.remove(&rel);
                self.cambios.push((completa.clone(), 3));
                if con_lineas { vec![] } else { vec![crate::tr!("se borró {rel}", "{rel} was deleted")] }
            }
            (antes, Some(m)) => {
                let (largo, fecha) = (m.len(), m.modified().ok());
                if antes.is_some_and(|(l, f, _)| l == largo && f == fecha) {
                    return vec![]; // FSEvents repite: nada cambió de verdad
                }
                let leido = antes.map(|(_, _, h)| h).unwrap_or(0);
                self.cambios.push((completa.clone(), if antes.is_none() { 1 } else { 2 }));
                if con_lineas {
                    let (lineas, hasta) = lineas_nuevas(&completa, leido);
                    self.vistos.insert(rel.clone(), (largo, fecha, hasta));
                    lineas.into_iter().map(pre).collect()
                } else {
                    self.vistos.insert(rel.clone(), (largo, fecha, largo));
                    vec![if antes.is_none() { crate::tr!("apareció {rel}", "{rel} appeared") } else { crate::tr!("cambió {rel}", "{rel} changed") }]
                }
            }
        }
    }
}

async fn seguir_archivos(dueno: String, id: u32, mut v: Vigilante) {
    use notify::Watcher;
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Vec<PathBuf>>();
    let vigia = notify::recommended_watcher(move |r: notify::Result<notify::Event>| {
        if let Ok(e) = r {
            let _ = tx.send(e.paths);
        }
    });
    let mut vigia = match vigia {
        Ok(w) => w,
        Err(e) => {
            super::avisar(&dueno, id, &crate::tr!("no pude vigilar los archivos ({e})", "could not watch the files ({e})"));
            return;
        }
    };
    if let Err(e) = vigia.watch(&v.base, notify::RecursiveMode::Recursive) {
        super::avisar(&dueno, id, &crate::tr!("no pude vigilar {} ({e})", "could not watch {} ({e})", v.base.display()));
        return;
    }
    let con_lineas = super::registro().lock().unwrap().get(&dueno).and_then(|c| c.activas.iter().find(|a| a.vigia.id == id)).is_some_and(|a| a.vigia.patron.is_some());
    let mut tic = tokio::time::interval(RITMO_ARCHIVOS);
    loop {
        let rutas: Vec<PathBuf> = tokio::select! {
            r = rx.recv() => match r {
                Some(mut p) => {
                    // lo que llegó del mismo golpe, junto
                    while let Ok(mas) = rx.try_recv() {
                        p.extend(mas);
                    }
                    p
                }
                None => return,
            },
            _ = tic.tick() => v.vistos.keys().map(|r| v.raiz.join(r)).collect(),
        };
        let mut hechas = std::collections::HashSet::new();
        for r in rutas {
            if hechas.insert(r.clone()) {
                for l in v.revisar(&r, con_lineas) {
                    super::entrar(&dueno, id, &l);
                }
            }
        }
        if !v.cambios.is_empty() {
            super::archivos_cambiados(&dueno, std::mem::take(&mut v.cambios));
        }
    }
}

// ── WebSocket: sólo direcciones públicas o `localhost` explícito ───────────────────────────────────

/// ¿La dirección nombra esta máquina a propósito (`localhost`, `127.0.0.1`, `[::1]`)?
pub fn localhost_explicito(url: &str) -> bool {
    reqwest::Url::parse(url.trim()).ok().and_then(|u| u.host_str().map(|h| h.to_lowercase())).is_some_and(|h| matches!(h.as_str(), "localhost" | "127.0.0.1" | "[::1]" | "::1"))
}

fn ip_privada(ip: std::net::IpAddr) -> bool {
    crate::web::private_reason(&format!("http://{}/", match ip {
        std::net::IpAddr::V4(v) => v.to_string(),
        std::net::IpAddr::V6(v) => format!("[{v}]"),
    }))
    .is_some()
}

/// Un WebSocket se acepta si es `ws://`/`wss://` y va a una dirección pública (también tras resolver el
/// nombre: un dominio que apunta a 192.168.x.x es una red privada) o a `localhost` explícito.
pub(super) async fn validar_ws(url: &str) -> Result<()> {
    let u = reqwest::Url::parse(url.trim()).map_err(|e| anyhow!("{}", crate::tr!("dirección inválida: {e}", "invalid address: {e}")))?;
    if !matches!(u.scheme(), "ws" | "wss") {
        bail!("{}", crate::i18n::t("`ws` es una dirección ws:// o wss://", "`ws` is a ws:// or wss:// address"));
    }
    if localhost_explicito(url) {
        return Ok(());
    }
    let http = url.trim().replacen("wss://", "https://", 1).replacen("ws://", "http://", 1);
    if let Some(por) = crate::web::private_reason(&http) {
        bail!("{}", crate::tr!("{url}: {por}. Sólo se vigilan direcciones públicas o `localhost` explícito", "{url}: {por}. Only public addresses or explicit `localhost` are watched"));
    }
    let host = u.host_str().unwrap_or("").trim_matches(['[', ']']).to_string();
    let puerto = u.port_or_known_default().unwrap_or(443);
    let ips: Vec<std::net::IpAddr> = tokio::time::timeout(Duration::from_secs(5), tokio::net::lookup_host((host.as_str(), puerto)))
        .await
        .map_err(|_| anyhow!("{}", crate::tr!("{host}: el nombre no resolvió en 5 s", "{host}: the name did not resolve within 5 s")))?
        .map_err(|e| anyhow!("{}", crate::tr!("{host}: el nombre no resuelve ({e})", "{host}: the name does not resolve ({e})")))?
        .map(|a| a.ip())
        .collect();
    if let Some(ip) = ips.iter().find(|ip| ip_privada(**ip)) {
        bail!("{}", crate::tr!("{host} resuelve a {ip}, una dirección local o privada: no se vigila (escribe `localhost` si es a propósito)", "{host} resolves to {ip}, a local or private address: not watched (write `localhost` if on purpose)"));
    }
    Ok(())
}

type Ws = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn seguir_ws(dueno: String, id: u32, mut ws: Ws) {
    use futures_util::StreamExt;
    use tokio_tungstenite::tungstenite::Message as M;
    while let Some(m) = ws.next().await {
        match m {
            Ok(M::Text(t)) => {
                for l in t.as_str().lines() {
                    super::entrar(&dueno, id, l);
                }
            }
            Ok(M::Binary(b)) => super::entrar(&dueno, id, &crate::tr!("(mensaje binario de {} bytes)", "(binary message of {} bytes)", b.len())),
            Ok(M::Close(_)) => break,
            Ok(_) => {}
            Err(e) => {
                super::avisar(&dueno, id, &crate::tr!("la conexión falló ({e})", "the connection failed ({e})"));
                return;
            }
        }
    }
    super::avisar(&dueno, id, crate::i18n::t("el servidor cerró la conexión", "the server closed the connection"));
}

// ── PR y CI de GitHub: la API, desde Rust ──────────────────────────────────────────────────────────

/// Lo que se recuerda de un PR para decir qué CAMBIÓ.
#[derive(Default, PartialEq, Debug)]
struct FotoPr {
    estado: String,
    sha: String,
    /// nombre → «estado · conclusión»
    ci: std::collections::BTreeMap<String, String>,
}

async fn foto_pr(c: &reqwest::Client, base: &str, repo: &str, n: u64, token: Option<&str>) -> Result<FotoPr> {
    let pedir = |ruta: String| {
        let mut r = c.get(format!("{base}/{ruta}")).header("Accept", "application/vnd.github+json").header("X-GitHub-Api-Version", "2022-11-28");
        if let Some(t) = token {
            r = r.bearer_auth(t);
        }
        r
    };
    let pr: serde_json::Value = pedir(format!("repos/{repo}/pulls/{n}")).send().await?.error_for_status()?.json().await?;
    let estado = if pr["merged"].as_bool() == Some(true) { "merged".to_string() } else { pr["state"].as_str().unwrap_or("?").to_string() };
    let sha = pr["head"]["sha"].as_str().unwrap_or("").to_string();
    let mut ci = std::collections::BTreeMap::new();
    if !sha.is_empty() {
        let runs: serde_json::Value = pedir(format!("repos/{repo}/commits/{sha}/check-runs")).send().await?.error_for_status()?.json().await?;
        for r in runs["check_runs"].as_array().into_iter().flatten() {
            let nombre = r["name"].as_str().unwrap_or("?").to_string();
            let est = match r["conclusion"].as_str() {
                Some(c) => format!("{} · {c}", r["status"].as_str().unwrap_or("")),
                None => r["status"].as_str().unwrap_or("?").to_string(),
            };
            ci.insert(nombre, est);
        }
    }
    Ok(FotoPr { estado, sha, ci })
}

/// Los cambios entre dos fotos, una línea cada uno.
fn cambios_pr(n: u64, antes: &FotoPr, ahora: &FotoPr) -> Vec<String> {
    let mut v = Vec::new();
    if antes.estado != ahora.estado {
        v.push(crate::tr!("PR #{n}: {} → {}", "PR #{n}: {} → {}", antes.estado, ahora.estado));
    }
    if antes.sha != ahora.sha && !antes.sha.is_empty() {
        v.push(crate::tr!("PR #{n}: commit nuevo {}", "PR #{n}: new commit {}", ahora.sha.chars().take(7).collect::<String>()));
    }
    for (nombre, est) in &ahora.ci {
        let previo = if antes.sha == ahora.sha { antes.ci.get(nombre) } else { None };
        if previo != Some(est) {
            let de = previo.cloned().unwrap_or_else(|| "—".into());
            v.push(crate::tr!("CI «{nombre}»: {de} → {est}", "CI «{nombre}»: {de} → {est}"));
        }
    }
    v
}

pub(crate) async fn seguir_pr(dueno: String, id: u32, base: String, repo: String, n: u64, token: Option<String>, ritmo: Duration) {
    let Ok(c) = reqwest::Client::builder().user_agent("gotv").timeout(Duration::from_secs(20)).build() else { return };
    let mut antes: Option<FotoPr> = None;
    let mut ultimo_error = String::new();
    loop {
        match foto_pr(&c, &base, &repo, n, token.as_deref()).await {
            Ok(f) => {
                ultimo_error.clear();
                if let Some(a) = &antes {
                    for l in cambios_pr(n, a, &f) {
                        super::entrar(&dueno, id, &l);
                    }
                }
                let cerrado = f.estado != "open";
                antes = Some(f);
                if cerrado {
                    super::avisar(&dueno, id, &crate::tr!("PR #{n} ya no está abierto: dejo de mirarlo", "PR #{n} is no longer open: I stop watching it"));
                    return;
                }
            }
            Err(e) => {
                // un error se dice UNA vez (no cada 30 s)
                let e = e.to_string();
                if e != ultimo_error {
                    super::avisar(&dueno, id, &crate::tr!("no pude leer el PR #{n} ({e})", "could not read PR #{n} ({e})"));
                    ultimo_error = e;
                }
            }
        }
        tokio::time::sleep(ritmo).await;
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::{dueno, esperar_evento, falso};
    use super::*;
    use serde_json::json;

    #[test]
    fn lineas_nuevas_respeta_la_linea_a_medias_y_el_archivo_que_se_achica() {
        let d = crate::CarpetaPrueba::nueva("gotv-vigilar-lineas");
        let f = d.join("x.log");
        std::fs::write(&f, "uno\ndos\ntre").unwrap();
        let (l, h) = lineas_nuevas(&f, 0);
        assert_eq!(l, ["uno", "dos"]);
        assert_eq!(h, 8);
        std::fs::write(&f, "uno\ndos\ntres\n\x1b[31mcuatro\x1b[0m\r\n").unwrap();
        let (l, h2) = lineas_nuevas(&f, h);
        assert_eq!(l, ["tres", "cuatro"]);
        // rotado: vuelve a empezar
        std::fs::write(&f, "nuevo\n").unwrap();
        assert_eq!(lineas_nuevas(&f, h2).0, ["nuevo"]);
    }

    #[test]
    fn la_salida_redirigida_del_proceso() {
        let c = Path::new("/p");
        assert_eq!(salida_redirigida("./servidor.sh > registro.log 2>&1", c), Some("/p/registro.log".into()));
        assert_eq!(salida_redirigida("nohup ./s >>logs/app.log &", c), Some("/p/logs/app.log".into()));
        assert_eq!(salida_redirigida("npm start &> /tmp/x.log", c), Some("/tmp/x.log".into()));
        assert_eq!(salida_redirigida("./s 2> errores.log", c), None, "sólo errores: la salida sigue en el proceso");
        assert_eq!(salida_redirigida("./s > /dev/null", c), None);
        assert_eq!(salida_redirigida("./s 2>&1", c), None);
        assert_eq!(salida_redirigida("cargo run", c), None);
    }

    /// Banco F3 (Huihui): el modelo lanzó `./servidor.sh > registro.log` y vigiló el PROCESO; antes no llegaba
    /// nada (la salida iba al archivo). Ahora se sigue el archivo y el ERROR llega solo.
    #[tokio::test]
    async fn un_proceso_que_escribe_en_un_archivo_se_vigila_por_su_archivo() {
        let raiz = crate::CarpetaPrueba::nueva("gotv-vigilar-redirigido");
        let d = dueno("redirigido");
        let cmd = format!("{} > registro.log", falso("e|INFO arranca;d|600;e|ERROR: modo inválido;d|30000"));
        let p = crate::exec::spawn(&d, &cmd, &raiz, false, None, true).unwrap();
        tokio::time::sleep(Duration::from_millis(200)).await;
        super::super::herramienta(&d, &raiz, &json!({"proceso": p.id, "patron": "ERROR"})).await.unwrap();
        let t = esperar_evento(&d, Duration::from_secs(5)).await.expect("el ERROR del archivo debía llegar solo");
        assert!(t.contains("ERROR: modo inválido") && !t.contains("INFO"), "{t}");
        p.stop().await;
    }

    #[tokio::test]
    async fn un_proceso_vivo_y_un_comando_despiertan_con_su_linea() {
        let raiz = crate::CarpetaPrueba::nueva("gotv-vigilar-proceso");
        let d = dueno("proceso");
        // F3 paso 2 · un proceso que YA estaba vivo (como lo deja `bash` en segundo plano)
        let p = crate::exec::spawn(&d, &falso("e|antes;d|600;e|INFO listo;e|ERROR: puerto ocupado;d|30000"), &raiz, false, None, true).unwrap();
        tokio::time::sleep(Duration::from_millis(200)).await;
        let r = super::super::herramienta(&d, &raiz, &json!({"proceso": p.id, "patron": "ERROR"})).await.unwrap();
        assert!(r.contains("vigilancia #") || r.contains("watch #"), "{r}");
        let t = esperar_evento(&d, Duration::from_secs(5)).await.expect("el ERROR debía llegar solo");
        assert!(t.contains("ERROR: puerto ocupado") && !t.contains("INFO") && !t.contains("antes"), "{t}");
        p.stop().await;
        let t = esperar_evento(&d, Duration::from_secs(5)).await.unwrap();
        assert!(t.contains("terminó") || t.contains("ended"), "{t}");
        // paso 3 · un comando nuevo: lo lanza la vigilancia y lo para al detenerla
        let r = super::super::herramienta(&d, &raiz, &json!({"comando": falso("e|linea 1;d|200;e|linea 2;d|200;e|linea 3;d|200;e|FALLO total;d|30000"), "patron": "FALLO"})).await.unwrap();
        let t = esperar_evento(&d, Duration::from_secs(6)).await.unwrap();
        // (la etiqueta repite el comando, con «linea 1» dentro: se mira el EVENTO, «] linea 1»)
        assert!(t.contains("] FALLO total") && !t.contains("] linea 1"), "{t}");
        let pid: u32 = r.split("proceso #").nth(1).or_else(|| r.split("process #").nth(1)).unwrap().split(')').next().unwrap().parse().unwrap();
        let id: u32 = r.split('#').nth(1).unwrap().split(' ').next().unwrap().parse().unwrap();
        super::super::detener(&d, id).unwrap();
        let proc_ = crate::exec::get(&d, pid).unwrap();
        assert!(proc_.wait_exit(Duration::from_secs(8)).await, "detener la vigilancia para el comando que lanzó");
        crate::exec::kill_owner(&d);
        super::super::soltar_chat(&d);
    }

    #[tokio::test]
    async fn archivos_nuevo_cambiado_borrado_y_lo_ignorado_no() {
        let raiz = crate::CarpetaPrueba::nueva("gotv-vigilar-archivos");
        std::fs::create_dir_all(raiz.join("logs")).unwrap();
        std::fs::create_dir_all(raiz.join("target")).unwrap();
        std::fs::write(raiz.join("logs/viejo.log"), "ya estaba\n").unwrap();
        // estado sin vigilante de fondo: `revisar` directo (lo que hace la tarea en cada aviso)
        let mut v = Vigilante::nuevo(&raiz, "**/*.log").unwrap();
        let raiz_c = raiz.canonicalize().unwrap();
        assert!(v.revisar(&raiz_c.join("logs/viejo.log"), false).is_empty(), "lo que ya estaba no es evento");
        std::fs::write(raiz.join("logs/nuevo.log"), "x\n").unwrap();
        assert_eq!(v.revisar(&raiz_c.join("logs/nuevo.log"), false).len(), 1);
        assert!(v.revisar(&raiz_c.join("logs/nuevo.log"), false).is_empty(), "un FSEvents repetido no es otro evento");
        std::fs::write(raiz.join("target/build.log"), "ruido\n").unwrap();
        assert!(v.revisar(&raiz_c.join("target/build.log"), false).is_empty(), "target/ se ignora");
        std::fs::write(raiz.join("logs/nuevo.log"), "x\ny\n").unwrap();
        let c = v.revisar(&raiz_c.join("logs/nuevo.log"), false);
        assert!(c[0].contains("cambió") || c[0].contains("changed"), "{c:?}");
        std::fs::remove_file(raiz.join("logs/nuevo.log")).unwrap();
        let c = v.revisar(&raiz_c.join("logs/nuevo.log"), false);
        assert!(c[0].contains("borró") || c[0].contains("deleted"), "{c:?}");
        // F3 · ML6: lo que cambió DE VERDAD, con el tipo de LSP (1 creado, 2 cambiado, 3 borrado); ni lo que ya
        // estaba, ni el FSEvents repetido, ni lo ignorado
        let n = raiz_c.join("logs/nuevo.log");
        assert_eq!(std::mem::take(&mut v.cambios), vec![(n.clone(), 1), (n.clone(), 2), (n, 3)]);
        // la cola del chat: una entrada por ruta (gana el último tipo) y se vacía al tomarla
        let q = dueno("archivos-cola");
        super::super::archivos_cambiados(&q, vec![(raiz_c.join("a.rs"), 1), (raiz_c.join("b.rs"), 2)]);
        super::super::archivos_cambiados(&q, vec![(raiz_c.join("a.rs"), 2)]);
        assert_eq!(super::super::tomar_archivos(&q), vec![(raiz_c.join("b.rs"), 2), (raiz_c.join("a.rs"), 2)]);
        assert!(super::super::tomar_archivos(&q).is_empty());
        // y de verdad, con FSEvents: un registro que crece y una línea con ERROR
        let d = dueno("archivos");
        std::fs::write(raiz.join("registro.log"), "arranque\n").unwrap();
        super::super::herramienta(&d, &raiz, &json!({"archivos": "registro.log", "patron": "ERROR"})).await.unwrap();
        tokio::time::sleep(Duration::from_millis(300)).await;
        {
            use std::io::Write;
            let mut f = std::fs::OpenOptions::new().append(true).open(raiz.join("registro.log")).unwrap();
            writeln!(f, "INFO petición 1").unwrap();
            writeln!(f, "ERROR conexión rechazada").unwrap();
        }
        let t = esperar_evento(&d, Duration::from_secs(8)).await.expect("FSEvents (o la revisión de 3 s) debía avisar");
        assert!(t.contains("· registro.log] ERROR conexión rechazada") && !t.contains("INFO") && !t.contains("arranque"), "{t}");
        // vigilar `target/` a propósito sí vale (la base ignorada no se filtra)
        let mut v2 = Vigilante::nuevo(&raiz, "target/*.log").unwrap();
        std::fs::write(raiz.join("target/otro.log"), "z\n").unwrap();
        assert_eq!(v2.revisar(&raiz_c.join("target/otro.log"), false).len(), 1);
        super::super::soltar_chat(&d);
        assert!(validar_patron_archivos(&raiz, "/etc/*.log").is_err() && validar_patron_archivos(&raiz, "../x/*.log").is_err());
    }

    #[tokio::test]
    async fn websocket_falso_y_la_red_privada_se_niega() {
        // un servidor WebSocket falso en Rust (tungstenite, el mismo cliente del otro lado)
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let puerto = l.local_addr().unwrap().port();
        tokio::spawn(async move {
            use futures_util::SinkExt;
            use tokio_tungstenite::tungstenite::Message as M;
            let (s, _) = l.accept().await.unwrap();
            let mut ws = tokio_tungstenite::accept_async(s).await.unwrap();
            ws.send(M::text("build: compilando")).await.unwrap();
            ws.send(M::text("build: ERROR en main.rs:3\nbuild: hecho")).await.unwrap();
            tokio::time::sleep(Duration::from_millis(200)).await;
            let _ = ws.close(None).await;
        });
        let raiz = crate::CarpetaPrueba::nueva("gotv-vigilar-ws");
        let d = dueno("ws");
        super::super::herramienta(&d, &raiz, &json!({"ws": format!("ws://127.0.0.1:{puerto}/eventos"), "patron": "ERROR"})).await.unwrap();
        let mut t = String::new();
        let inicio = std::time::Instant::now();
        while inicio.elapsed() < Duration::from_secs(5) && !(t.contains("cerró") || t.contains("closed")) {
            if let Some(x) = esperar_evento(&d, Duration::from_secs(1)).await {
                t.push_str(&x);
            }
        }
        assert!(t.contains("build: ERROR en main.rs:3") && !t.contains("compilando") && !t.contains("hecho"), "{t}");
        assert!(t.contains("cerró") || t.contains("closed"), "{t}");
        super::super::soltar_chat(&d);
        // la red privada se niega (literal y por nombre local); localhost explícito no pasa por aquí
        for u in ["ws://192.168.1.10:8080/x", "ws://10.0.0.2/x", "ws://impresora.local/x", "wss://[fe80::1]/x"] {
            assert!(validar_ws(u).await.is_err(), "{u}");
        }
        assert!(validar_ws("http://ejemplo.com").await.is_err(), "sólo ws/wss");
        assert!(validar_ws(&format!("ws://localhost:{puerto}")).await.is_ok());
        assert!(super::super::herramienta(&d, &raiz, &json!({"ws": "ws://192.168.1.10:8080/x"})).await.is_err());
        assert!(super::super::listar(&d).contains("no hay") || super::super::listar(&d).contains("no watches"), "lo negado no gasta un hueco");
    }

    #[tokio::test]
    async fn pr_y_ci_contra_un_github_falso() {
        use axum::{extract::State, routing::get, Json, Router};
        type Estado = Arc<std::sync::Mutex<(String, String)>>; // (estado del PR, conclusión del CI)
        let est: Estado = Arc::new(std::sync::Mutex::new(("open".into(), String::new())));
        let app = Router::new()
            .route("/repos/yo/repo/pulls/7", get(|State(s): State<Estado>| async move { Json(json!({"state": if s.lock().unwrap().0 == "merged" { "closed".to_string() } else { s.lock().unwrap().0.clone() }, "merged": s.lock().unwrap().0 == "merged", "head": {"sha": "abc1234def"}})) }))
            .route("/repos/yo/repo/commits/abc1234def/check-runs", get(|State(s): State<Estado>| async move {
                let c = s.lock().unwrap().1.clone();
                Json(json!({"check_runs": [if c.is_empty() { json!({"name": "build", "status": "in_progress", "conclusion": null}) } else { json!({"name": "build", "status": "completed", "conclusion": c}) }]}))
            }))
            .with_state(est.clone());
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", l.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
        let d = dueno("pr");
        let id = super::super::registrar(&d, Path::new("/tmp"), super::super::Fuente::Pr(7), None, Duration::from_secs(60), 5).unwrap();
        let t = tokio::spawn(seguir_pr(d.clone(), id, base, "yo/repo".into(), 7, None, Duration::from_millis(100)));
        tokio::time::sleep(Duration::from_millis(350)).await;
        assert!(!super::super::hay_pendientes(&d), "el estado inicial no es un evento");
        est.lock().unwrap().1 = "failure".into();
        let ev = esperar_evento(&d, Duration::from_secs(3)).await.unwrap();
        assert!(ev.contains("CI «build»: in_progress → completed · failure"), "{ev}");
        est.lock().unwrap().0 = "merged".into();
        let ev = esperar_evento(&d, Duration::from_secs(3)).await.unwrap();
        assert!(ev.contains("PR #7: open → merged"), "{ev}");
        // cerrado: la tarea termina sola
        let fin = tokio::time::timeout(Duration::from_secs(3), t).await;
        assert!(fin.is_ok(), "un PR cerrado deja de consultarse");
        super::super::soltar_chat(&d);
    }
}
