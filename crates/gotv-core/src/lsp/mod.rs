//! LSP (S5): el agente ve los errores de su propia edición SIN compilar a mano, y
//! navega el código por nombre de símbolo.
//!
//! Medido con rust-analyzer 1.99 (M1 Max): initialize 0,06 s; un error de tipos llega
//! como diagnóstico de `rustc` a los 4,2 s de abrir el archivo; «ir a la definición»
//! responde en 0,17 s. Por eso tras cada edición se espera a que los diagnósticos se
//! ASIENTEN (hasta 10 s) y se entregan sólo los errores, con línea.
//!
//! Servidores detectados por los archivos del proyecto (y si están instalados):
//! Rust → rust-analyzer · Python → pyright-langserver / pylsp · JS/TS →
//! typescript-language-server · Go → gopls · PHP → intelephense.

use anyhow::{anyhow, bail, Result};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

#[derive(Debug, Clone, PartialEq)]
pub struct Server {
    pub lang: &'static str,
    pub cmd: Vec<String>,
    pub exts: &'static [&'static str],
}

/// F0.2: ¿está `bin` en el `PATH` DEL USUARIO? (no en el heredado: desde el Dock no trae Homebrew ni
/// Cargo). Ver `entorno.rs`.
pub(crate) fn on_path(bin: &str) -> bool {
    crate::entorno::en_path(bin)
}

/// F5: los servidores de lenguaje completos que hay en ESTA Mac (para que el modelo sepa qué
/// parte de su motor está encendida). `(lenguaje, binario)`.
/// ML4: sale de la MISMA tabla que usa el gestor (antes era una lista aparte de 5 lenguajes).
pub fn instalados() -> Vec<(&'static str, String)> {
    registro::defs().into_iter().filter_map(|d| registro::comando(&d).map(|c| (d.lang, c[0].clone()))).collect()
}

/// El servidor para este proyecto (el primer lenguaje de la tabla con su marcador en la raíz), si hay
/// uno instalado. `None` también dice qué falta.
pub fn detect(root: &Path) -> std::result::Result<Server, Option<String>> {
    for d in registro::defs() {
        if !d.raices.iter().any(|m| registro::marca(root, m)) {
            continue;
        }
        // rust-analyzer: el atajo de rustup existe aunque el componente no; `comando` lo prueba de verdad
        return match registro::comando(&d) {
            Some(cmd) => Ok(Server { lang: d.lang, cmd, exts: d.exts }),
            None => Err(Some(crate::tr!(
                "proyecto {} sin servidor de lenguaje: `{}` activa diagnósticos y navegación",
                "{} project without a language server: `{}` turns on diagnostics and navigation",
                d.lang,
                d.instalar
            ))),
        };
    }
    Err(None)
}

#[derive(Debug, Clone, PartialEq)]
pub struct Diag {
    pub line: u64,
    pub severity: u64,
    pub message: String,
}

type Diags = Arc<Mutex<HashMap<String, (Instant, Vec<Value>)>>>;

/// Trabajo en curso que el servidor ANUNCIA (`$/progress`): tokens activos y el último
/// movimiento. Medido: arrancado en frío, rust-analyzer publica primero una lista VACÍA
/// (sus chequeos nativos) y el `cargo check` llega segundos después; esperar «1,5 s sin
/// novedades» daba un falso «sin errores». Se espera a que no quede trabajo activo.
#[derive(Default)]
struct Work {
    active: std::collections::HashSet<String>,
    last: Option<Instant>,
    /// `experimental/serverStatus` (rust-analyzer): ¿terminó de cargar el proyecto?, y cuándo lo dijo. Medido
    /// 2026-11-17 con la Mac a carga 123: en frío publica una lista VACÍA antes de cargar el workspace y el
    /// hueco hasta los diagnósticos de verdad pasaba de 1,5 s → falso negativo. Mientras no esté quieto, lo
    /// publicado no cuenta como asentado.
    quieto: Option<(bool, Instant)>,
}

/// F5: el gestor de varios servidores (ML1), la tabla de servidores y la raíz de cada archivo.
pub mod acciones;
pub mod descargas;
pub mod entorno;
pub mod falso;
pub mod gestor;
pub mod raiz;
pub mod registro;
/// F16.3 · paso 5: el servidor de un proyecto remoto corre allá; aquí se traducen las rutas
pub mod remoto;
pub use gestor::Gestor;

pub struct Lsp {
    pub server: Server,
    root: PathBuf,
    /// F0.3: el canal común con el servidor
    canal: crate::canal::Canal,
    diags: Diags,
    work: Arc<Mutex<Work>>,
    versions: HashMap<String, i64>,
    /// lo que el servidor dijo que sabe hacer (respuesta a `initialize`)
    caps: Value,
    /// F5 ML2: los archivos que GOTV abrió para revisar (usan lo que se iba a tocar)
    vecinos: std::collections::BTreeSet<PathBuf>,
    /// F16.3: proyecto REMOTO: el servidor corre allá y las rutas se traducen (ancla ↔ raíz remota)
    puente: Option<Arc<remoto::Puente>>,
    /// F16.3: lo último que se le mandó de cada archivo remoto (para enseñar la línea de una ubicación)
    textos: HashMap<PathBuf, String>,
}

fn uri(p: &Path) -> String {
    format!("file://{}", p.display())
}

impl Lsp {
    /// La URI que entiende el servidor (en remoto, la ruta de allá).
    fn u(&self, p: &Path) -> String {
        match &self.puente {
            Some(b) => uri(&b.a_remota(p)),
            None => uri(p),
        }
    }

    /// La ruta de aquí de una URI del servidor.
    pub(crate) fn ruta(&self, u: &str) -> PathBuf {
        match &self.puente {
            Some(b) => b.a_local(u),
            None => PathBuf::from(u.trim_start_matches("file://")),
        }
    }

    /// El texto de un archivo: del disco, o ALLÁ en un proyecto remoto.
    async fn leer(&mut self, p: &Path) -> Result<String> {
        match self.puente.clone() {
            Some(b) => {
                let t = b.leer(p).await?;
                self.textos.insert(p.to_path_buf(), t.clone());
                Ok(t)
            }
            None => Ok(std::fs::read_to_string(p)?),
        }
    }

    /// Sin esperar: del disco o de lo último que se leyó allá (para enseñar una línea).
    fn texto_de(&self, p: &Path) -> Option<String> {
        match &self.puente {
            Some(_) => self.textos.get(p).cloned(),
            None => std::fs::read_to_string(p).ok(),
        }
    }

    /// ¿Es de un proyecto remoto?
    pub fn remoto(&self) -> bool {
        self.puente.is_some()
    }
}

/// El nombre de una clase de símbolo del protocolo (`SymbolKind`), como se escribe en código.
fn clase_lsp(k: u64) -> &'static str {
    match k {
        1 => "file",
        2 => "mod",
        3 => "namespace",
        4 => "package",
        5 => "class",
        6 => "method",
        7 => "property",
        8 => "field",
        9 => "constructor",
        10 => "enum",
        11 => "interface",
        12 => "fn",
        13 => "var",
        14 => "const",
        22 => "variant",
        23 => "struct",
        24 => "event",
        25 => "operator",
        26 => "type",
        _ => "·",
    }
}

impl Lsp {
    pub async fn start(server: Server, root: &Path) -> Result<Lsp> {
        Lsp::start_con(server, root, None).await
    }

    /// F16.3: el servidor de un proyecto REMOTO, corriendo allá (`path` = el `PATH` de inicio de sesión de allá).
    pub async fn start_remoto(server: Server, ancla: &Path, puente: Arc<remoto::Puente>, path: &str) -> Result<Lsp> {
        Lsp::start_con(server, ancla, Some((puente, path.to_string()))).await
    }

    async fn start_con(server: Server, root: &Path, remoto: Option<(Arc<remoto::Puente>, String)>) -> Result<Lsp> {
        let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
        // F0.3: el canal común (marco Content-Length); el stderr del servidor queda en
        // `.gotv/registros/lsp-<programa>.log` (antes a /dev/null: no se podía depurar)
        let mut cmd = match &remoto {
            Some((b, path)) => b.lanzar(&server.cmd, path)?,
            None => {
                let mut c = crate::entorno::comando_tokio(&server.cmd[0]);
                c.args(&server.cmd[1..]).current_dir(&root);
                c
            }
        };
        let puente = remoto.map(|(b, _)| b);
        // F10.4: el de un plugin trae su entorno, sus opciones de arranque y sus ajustes
        let de_plugin = crate::plugins::lsp_de_comando(&server.cmd[0]);
        for (k, v) in de_plugin.iter().flat_map(|l| l.entorno.iter()) {
            cmd.env(k, v);
        }
        let programa: String = Path::new(&server.cmd[0]).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let registro = root.join(".gotv/registros").join(format!("lsp-{programa}.log"));
        let canal = crate::canal::Canal::lanzar(cmd, crate::canal::Marco::ContentLength, "LSP", Some(registro))
            .await
            .map_err(|e| anyhow!("{}: {e}", server.cmd[0]))?;
        let diags: Diags = Arc::default();
        let work: Arc<Mutex<Work>> = Arc::default();
        // F5 ML5: el entorno real del proyecto (intérprete, TypeScript propio, carpeta de compilación)
        let ajustes = de_plugin.as_ref().and_then(|l| l.ajustes.clone()).unwrap_or_else(|| entorno::ajustes(server.lang, &root));
        let inicio = de_plugin.as_ref().and_then(|l| l.inicio.clone()).unwrap_or_else(|| entorno::inicio(server.lang, &root));
        // Eventos del servidor: diagnósticos → al mapa; progreso → al trabajo en curso; sus
        // peticiones (configuración, progreso, registro) → se contestan, o se cuelga.
        if let Some(mut eventos) = canal.eventos() {
            let (diags, work, resp, ajustes) = (diags.clone(), work.clone(), canal.respondedor(), ajustes.clone());
            tokio::spawn(async move {
                while let Some(m) = eventos.recv().await {
                    match (m.get("id"), m.get("method").and_then(Value::as_str)) {
                        (Some(id), Some(method)) => {
                            let result = if method == "workspace/configuration" {
                                // ML5: con los ajustes reales (antes: nulos → «import no resuelto» falsos)
                                entorno::responder_configuracion(&ajustes, m["params"]["items"].as_array().map(|a| a.as_slice()).unwrap_or(&[]))
                            } else {
                                Value::Null
                            };
                            let _ = resp.responder(id.clone(), result).await;
                        }
                        (None, Some("$/progress")) => {
                            let token = m["params"]["token"].to_string();
                            let kind = m["params"]["value"]["kind"].as_str().unwrap_or("");
                            let mut w = work.lock().await;
                            match kind {
                                "begin" => {
                                    w.active.insert(token);
                                }
                                "end" => {
                                    w.active.remove(&token);
                                }
                                _ => {}
                            }
                            w.last = Some(Instant::now());
                        }
                        (None, Some("experimental/serverStatus")) => {
                            let mut w = work.lock().await;
                            let ahora = Instant::now();
                            w.quieto = Some((m["params"]["quiescent"].as_bool().unwrap_or(true), ahora));
                            w.last = Some(ahora);
                        }
                        (None, Some("textDocument/publishDiagnostics")) => {
                            let u = m["params"]["uri"].as_str().unwrap_or("").to_string();
                            let d = m["params"]["diagnostics"].as_array().cloned().unwrap_or_default();
                            diags.lock().await.insert(u, (Instant::now(), d));
                        }
                        _ => {}
                    }
                }
            });
        }
        let mut lsp = Lsp { server, root, canal, diags, work, versions: HashMap::new(), caps: Value::Null, vecinos: Default::default(), puente, textos: HashMap::new() };
        let raiz_uri = lsp.u(&lsp.root);
        let ini = lsp.request(
            "initialize",
            json!({
                // un servidor remoto no puede mirar un proceso de esta Mac
                "processId": if lsp.puente.is_some() { Value::Null } else { json!(std::process::id()) },
                "initializationOptions": inicio,
                "rootUri": raiz_uri,
                "workspaceFolders": [{"uri": raiz_uri, "name": "gotv"}],
                // ML6: el servidor puede pedir que le avisemos de los cambios por fuera (los avisamos igual)
                // `experimental.serverStatusNotification`: rust-analyzer dice cuándo terminó de cargar (ver `Work`)
                "capabilities": {"experimental": {"serverStatusNotification": true}, "window": {"workDoneProgress": true}, "workspace": {"didChangeWatchedFiles": {"dynamicRegistration": true}}, "textDocument": {
                    "publishDiagnostics": {}, "definition": {}, "references": {}, "hover": {"contentFormat": ["plaintext", "markdown"]},
                    "documentSymbol": {"hierarchicalDocumentSymbolSupport": true},
                    "implementation": {}, "typeDefinition": {}, "callHierarchy": {}, "rename": {"prepareSupport": false},
                    "codeAction": {"codeActionLiteralSupport": {"codeActionKind": {"valueSet": ["quickfix", "source.organizeImports"]}}, "isPreferredSupport": true},
                    "synchronization": {"didSave": true}
                }}
            }),
            Duration::from_secs(60),
        )
        .await?;
        lsp.caps = ini["capabilities"].clone();
        lsp.notify("initialized", json!({})).await?;
        Ok(lsp)
    }

    async fn request(&mut self, method: &str, params: Value, timeout: Duration) -> Result<Value> {
        self.canal.pedir(method, params, timeout).await
    }

    async fn notify(&self, method: &str, params: Value) -> Result<()> {
        self.canal.avisar(method, params).await
    }

    /// F5 ML2: todos los diagnósticos que el servidor publicó, por archivo (uri).
    pub async fn diags_todos(&self) -> HashMap<String, Vec<Value>> {
        self.diags.lock().await.iter().map(|(u, (_, d))| (u.clone(), d.clone())).collect()
    }

    /// El PID del servidor (para medir su memoria).
    pub fn pid(&self) -> Option<u32> {
        self.canal.pid()
    }

    /// ¿Sigue vivo el proceso? (si cayó, el gestor lo relanza)
    pub fn vivo(&self) -> bool {
        self.canal.vivo()
    }

    pub fn handles(&self, p: &Path) -> bool {
        p.extension()
            .map(|e| self.server.exts.contains(&e.to_string_lossy().as_ref()))
            .unwrap_or(false)
    }

    /// F5 ML6 · archivos que cambiaron POR FUERA de las ediciones de GOTV (un `bash`, `git`, un
    /// formateador): `(ruta, tipo)` con el tipo de LSP (1 creado, 2 cambiado, 3 borrado). Se avisan con
    /// `workspace/didChangeWatchedFiles` (los de los lenguajes de este servidor, dentro de su raíz) y los
    /// que el servidor tiene ABIERTOS se resincronizan (o se cierran, si se borraron): si no, seguiría
    /// razonando sobre el texto viejo. Devuelve cuántos avisó.
    pub async fn por_fuera(&mut self, cambios: &[(PathBuf, u8)]) -> Result<usize> {
        let raiz = self.root.canonicalize().unwrap_or_else(|_| self.root.clone());
        // la ruta canónica, también la de un archivo ya borrado (su carpeta sí existe)
        let canon = |p: &Path| -> PathBuf {
            p.canonicalize().unwrap_or_else(|_| match (p.parent().and_then(|d| d.canonicalize().ok()), p.file_name()) {
                (Some(d), Some(n)) => d.join(n),
                _ => p.to_path_buf(),
            })
        };
        let mios: Vec<(PathBuf, u8)> = cambios.iter().filter(|(p, _)| self.handles(p)).map(|(p, t)| (canon(p), *t)).filter(|(p, _)| p.starts_with(&raiz)).collect();
        if mios.is_empty() {
            return Ok(0);
        }
        let avisos: Vec<Value> = mios.iter().map(|(p, t)| json!({"uri": self.u(p), "type": t})).collect();
        self.notify("workspace/didChangeWatchedFiles", json!({"changes": avisos})).await?;
        for (p, t) in &mios {
            let u = self.u(p);
            if !self.versions.contains_key(&u) {
                continue;
            }
            if *t == 3 {
                self.versions.remove(&u);
                self.vecinos.remove(p);
                self.notify("textDocument/didClose", json!({"textDocument": {"uri": u}})).await?;
            } else {
                self.sync(p).await?;
            }
        }
        Ok(mios.len())
    }

    /// Lleva el contenido ACTUAL del archivo al servidor (abrir o cambiar + guardar).
    pub async fn sync(&mut self, p: &Path) -> Result<()> {
        let p = p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
        let text = self.leer(&p).await?;
        let u = self.u(&p);
        let v = self.versions.entry(u.clone()).or_insert(0);
        *v += 1;
        let v = *v;
        if v == 1 {
            // ML4: el id por extensión (javascript, typescriptreact, cpp, objective-c, scss…)
            let lang = registro::id_de_lenguaje(self.server.lang, &p);
            self.notify(
                "textDocument/didOpen",
                json!({"textDocument": {"uri": u, "languageId": lang, "version": v, "text": text}}),
            )
            .await?;
        } else {
            self.notify("textDocument/didChange", json!({"textDocument": {"uri": u, "version": v}, "contentChanges": [{"text": text}]})).await?;
        }
        self.notify("textDocument/didSave", json!({"textDocument": {"uri": u}}))
            .await
    }

    /// Diagnósticos del archivo tras sincronizarlo: espera a que lleguen DESPUÉS del
    /// cambio y a que se asienten 1,5 s sin novedades (máx. `wait`).
    pub async fn diagnostics(&mut self, p: &Path, wait: Duration) -> Result<Vec<Diag>> {
        Ok(self.diagnostics_full(p, wait).await?.0)
    }

    /// Como `diagnostics`, y además si el servidor TERMINÓ (false = venció la espera
    /// con trabajo en curso: el resultado es parcial y se dice).
    pub async fn diagnostics_full(
        &mut self,
        p: &Path,
        wait: Duration,
    ) -> Result<(Vec<Diag>, bool)> {
        let since = Instant::now();
        self.sync(p).await?;
        let u = self.u(&p.canonicalize().unwrap_or_else(|_| p.to_path_buf()));
        let deadline = since + wait;
        let mut complete = false;
        loop {
            tokio::time::sleep(Duration::from_millis(150)).await;
            let (diag_at, cualquiera_at) = {
                let m = self.diags.lock().await;
                // F5 ML2: lo que publique de OTROS archivos tras el cambio también es movimiento (un vecino
                // que se rompió llega en la misma tanda, a veces un poco después)
                (m.get(&u).map(|(t, _)| *t).filter(|t| *t >= since), m.values().map(|(t, _)| *t).filter(|t| *t >= since).max())
            };
            let (busy, work_at, cargando) = {
                let w = self.work.lock().await;
                (!w.active.is_empty(), w.last.filter(|t| *t >= since), w.quieto.is_some_and(|(q, _)| !q))
            };
            // asentado = hubo diagnósticos tras el cambio, el servidor no tiene trabajo
            // anunciado en curso, no está cargando el proyecto y nada se movió en 1,5 s
            let last_move = diag_at.max(work_at).max(cualquiera_at);
            let settled = diag_at.is_some()
                && !busy
                && !cargando
                && last_move.is_some_and(|t| t.elapsed() >= Duration::from_millis(1500));
            if settled {
                complete = true;
                break;
            }
            if Instant::now() >= deadline {
                break;
            }
        }
        // F5 ML2: los vecinos abiertos para revisar, a petición si el servidor lo admite (protocolo 3.17)
        let vecinos: Vec<PathBuf> = self.vecinos.iter().cloned().collect();
        for v in vecinos {
            self.tirar(&v).await;
        }
        let all = self
            .diags
            .lock()
            .await
            .get(&u)
            .filter(|(t, _)| *t >= since)
            .map(|(_, d)| d.clone())
            .unwrap_or_default();
        Ok((
            all.iter()
                .map(|d| Diag {
                    line: d["range"]["start"]["line"].as_u64().unwrap_or(0) + 1,
                    severity: d["severity"].as_u64().unwrap_or(1),
                    message: d["message"]
                        .as_str()
                        .unwrap_or("")
                        .lines()
                        .collect::<Vec<_>>()
                        .join(" — "),
                })
                .collect(),
            complete,
        ))
    }

    /// F5 ML2: diagnósticos A PETICIÓN de un archivo (`textDocument/diagnostic`, protocolo 3.17), si el
    /// servidor lo admite; el resultado entra al mismo mapa que los publicados. Sin soporte, nada.
    async fn tirar(&mut self, p: &Path) {
        if self.caps.get("diagnosticProvider").is_none_or(Value::is_null) {
            return;
        }
        let u = self.u(p);
        if let Ok(r) = self.request("textDocument/diagnostic", json!({"textDocument": {"uri": u}}), Duration::from_secs(10)).await {
            if r["kind"] == "full" {
                let items = r["items"].as_array().cloned().unwrap_or_default();
                self.diags.lock().await.insert(u, (Instant::now(), items));
            }
        }
    }

    /// F5 ML2: para los servidores que sólo miran lo ABIERTO. Los símbolos de `p` que tocan las líneas
    /// `desde..=hasta` (las que se van a editar), sus referencias en el proyecto, y esos archivos se abren
    /// para que el servidor los revise: así la foto de ANTES ya los incluye, y un error que nazca en ellos
    /// sale como nuevo (sin abrirlos, el servidor nunca lo diría). Como mucho 5 símbolos y 20 archivos.
    /// Devuelve los que abrió ahora.
    pub async fn abrir_vecinos(&mut self, p: &Path, desde: u64, hasta: u64) -> Vec<PathBuf> {
        let archivos = self.quienes_usan(p, desde, hasta, 5).await;
        let mut abiertos = Vec::new();
        for f in archivos {
            if self.versions.contains_key(&self.u(&f)) {
                continue;
            }
            if self.sync(&f).await.is_ok() {
                self.vecinos.insert(f.clone());
                abiertos.push(f);
            }
        }
        abiertos
    }

    /// Los archivos del proyecto que usan los símbolos de `p` en las líneas `desde..=hasta` (los más internos
    /// primero; `tope_simbolos` como mucho, 20 archivos). F5 ML2 (vecinos) y F15.1 (pruebas afectadas).
    pub async fn quienes_usan(&mut self, p: &Path, desde: u64, hasta: u64, tope_simbolos: usize) -> Vec<PathBuf> {
        fn juntar(v: &Value, desde: u64, hasta: u64, out: &mut Vec<(u64, u64)>) {
            for s in v.as_array().into_iter().flatten() {
                let r = if s["range"].is_object() { &s["range"] } else { &s["location"]["range"] };
                let (a, b) = (r["start"]["line"].as_u64().unwrap_or(0), r["end"]["line"].as_u64().unwrap_or(0));
                if a <= hasta && b >= desde {
                    let sel = if s["selectionRange"].is_object() { &s["selectionRange"] } else { r };
                    out.push((sel["start"]["line"].as_u64().unwrap_or(0), sel["start"]["character"].as_u64().unwrap_or(0)));
                }
                juntar(&s["children"], desde, hasta, out);
            }
        }
        let p = p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
        if self.sync(&p).await.is_err() {
            return vec![];
        }
        let u = self.u(&p);
        let t = Duration::from_secs(10);
        let Ok(simbolos) = self.request("textDocument/documentSymbol", json!({"textDocument": {"uri": u}}), t).await else { return vec![] };
        let mut posiciones = Vec::new();
        // las líneas de LSP empiezan en 0
        juntar(&simbolos, desde.saturating_sub(1), hasta.saturating_sub(1), &mut posiciones);
        // los más internos primero (lo que de verdad cambia), sin repetir
        posiciones.reverse();
        posiciones.dedup();
        let mut archivos: Vec<PathBuf> = Vec::new();
        for (l, c) in posiciones.into_iter().take(tope_simbolos) {
            let params = json!({"textDocument": {"uri": u}, "position": {"line": l, "character": c}, "context": {"includeDeclaration": false}});
            let Ok(r) = self.request("textDocument/references", params, t).await else { continue };
            for loc in r.as_array().into_iter().flatten() {
                let f = self.ruta(loc["uri"].as_str().unwrap_or(""));
                // (en remoto el archivo vive allá: no se mira en el disco de aquí)
                if f != p && f.starts_with(&self.root) && self.handles(&f) && (self.puente.is_some() || f.is_file()) && !archivos.contains(&f) && archivos.len() < 20 {
                    archivos.push(f);
                }
            }
        }
        archivos
    }

    /// Espera (como mucho `tope`) a que el servidor publique sobre los archivos recién abiertos y se calme;
    /// si admite pedirlos, los pide.
    pub async fn esperar_vecinos(&mut self, abiertos: &[PathBuf], tope: Duration) {
        let desde = Instant::now();
        loop {
            tokio::time::sleep(Duration::from_millis(150)).await;
            let listos = {
                let m = self.diags.lock().await;
                abiertos.iter().all(|f| m.contains_key(&self.u(f)))
            };
            let ocupado = !self.work.lock().await.active.is_empty();
            if (listos && !ocupado) || desde.elapsed() >= tope {
                break;
            }
        }
        for f in abiertos {
            self.tirar(f).await;
        }
    }

    /// Posición del símbolo en el archivo (palabra completa; en `line` si se da).
    fn locate(text: &str, p: &Path, symbol: &str, line: Option<u64>) -> Result<(u64, u64)> {
        for (i, l) in text.lines().enumerate() {
            if line.is_some_and(|n| n != i as u64 + 1) {
                continue;
            }
            for (col, _) in l.match_indices(symbol) {
                let before = l[..col].chars().next_back();
                let after = l[col + symbol.len()..].chars().next();
                let word = |c: Option<char>| c.is_none_or(|c| !(c.is_alphanumeric() || c == '_'));
                if word(before) && word(after) {
                    // columna en UTF-16, como pide LSP
                    return Ok((i as u64, l[..col].encode_utf16().count() as u64));
                }
            }
        }
        bail!(
            "{}",
            crate::tr!(
                "no encontré «{symbol}» en {}",
                "«{symbol}» not found in {}",
                p.display()
            )
        )
    }

    fn fmt_locations(&self, v: &Value) -> String {
        let locs: Vec<Value> = match v {
            Value::Array(a) => a.clone(),
            Value::Object(_) => vec![v.clone()],
            _ => vec![],
        };
        let mut out = Vec::new();
        for l in locs.iter().take(30) {
            let u = l["uri"].as_str().or(l["targetUri"].as_str()).unwrap_or("");
            let r = if l["range"].is_object() {
                &l["range"]
            } else {
                &l["targetSelectionRange"]
            };
            let line = r["start"]["line"].as_u64().unwrap_or(0);
            let path = self.ruta(u);
            let text = self
                .texto_de(&path)
                .and_then(|t| t.lines().nth(line as usize).map(|s| s.trim().to_string()))
                .unwrap_or_default();
            let shown = path
                .strip_prefix(&self.root)
                .map(|p| p.display().to_string())
                .unwrap_or_else(|_| path.display().to_string());
            out.push(format!(
                "{shown}:{}: {}",
                line + 1,
                text.chars().take(140).collect::<String>()
            ));
        }
        if out.is_empty() {
            crate::i18n::t("(sin resultados)", "(no results)").into()
        } else {
            out.join("\n")
        }
    }

    /// D8d: el esquema de un archivo (`textDocument/documentSymbol`): una definición por línea,
    /// sangrada por nivel. Vacío si el servidor no lo sabe (quien llama cae al mapa propio).
    pub async fn symbols(&mut self, p: &Path) -> Result<String> {
        fn baja(v: &Value, nivel: usize, out: &mut Vec<String>) {
            for s in v.as_array().into_iter().flatten() {
                if out.len() >= 300 {
                    return;
                }
                let clase = s["kind"].as_u64().unwrap_or(0);
                // dentro de una función o un tipo, sus variables y campos no son esquema
                if nivel >= 2 && matches!(clase, 7 | 8 | 13 | 14) {
                    continue;
                }
                let linea = s["selectionRange"]["start"]["line"].as_u64().or(s["range"]["start"]["line"].as_u64()).or(s["location"]["range"]["start"]["line"].as_u64()).unwrap_or(0);
                out.push(format!("{}: {}{} {}", linea + 1, "  ".repeat(nivel), clase_lsp(clase), s["name"].as_str().unwrap_or("?")));
                if nivel < 2 {
                    baja(&s["children"], nivel + 1, out);
                }
            }
        }
        self.sync(p).await?;
        let u = self.u(&p.canonicalize().unwrap_or_else(|_| p.to_path_buf()));
        let r = self.request("textDocument/documentSymbol", json!({"textDocument": {"uri": u}}), Duration::from_secs(30)).await?;
        let mut out = Vec::new();
        baja(&r, 0, &mut out);
        Ok(out.join("\n"))
    }

    /// D8d: dónde se define un nombre en el proyecto (`workspace/symbol`). Sólo lo de DENTRO de la
    /// raíz: las dependencias y la biblioteca estándar no son del proyecto.
    pub async fn workspace(&mut self, consulta: &str) -> Result<String> {
        let r = self.request("workspace/symbol", json!({"query": consulta}), Duration::from_secs(30)).await?;
        let mut out = Vec::new();
        for s in r.as_array().into_iter().flatten() {
            let path = self.ruta(s["location"]["uri"].as_str().unwrap_or(""));
            let Ok(rel) = path.strip_prefix(&self.root) else { continue };
            let linea = s["location"]["range"]["start"]["line"].as_u64().unwrap_or(0);
            let dentro = s["containerName"].as_str().filter(|c| !c.is_empty()).map(|c| format!(" ({c})")).unwrap_or_default();
            out.push(format!("{}:{}: {} {}{dentro}", rel.display(), linea + 1, clase_lsp(s["kind"].as_u64().unwrap_or(0)), s["name"].as_str().unwrap_or("?")));
            if out.len() >= 40 {
                break;
            }
        }
        Ok(out.join("\n"))
    }

    /// `definition` · `references` · `hover` · `diagnostics` sobre un símbolo de un archivo;
    /// `symbols` (esquema del archivo) y `workspace` (un nombre en todo el proyecto).
    pub async fn query(
        &mut self,
        action: &str,
        p: &Path,
        symbol: &str,
        line: Option<u64>,
    ) -> Result<String> {
        // F5 ML3: los nombres en español, con los ingleses de antes como alias
        let action = acciones::normal(action);
        if action == "symbols" {
            return self.symbols(p).await;
        }
        if action == "workspace" {
            return self.workspace(symbol).await;
        }
        if action == "diagnostics" {
            let (d, complete) = self.diagnostics_full(p, Duration::from_secs(30)).await?;
            return Ok(format_diags(p, &d, true, complete));
        }
        self.sync(p).await?;
        let texto = self.leer(&p.canonicalize().unwrap_or_else(|_| p.to_path_buf())).await?;
        let (l, c) = Self::locate(&texto, p, symbol, line)?;
        let pos = json!({"textDocument": {"uri": self.u(&p.canonicalize().unwrap_or_else(|_| p.to_path_buf()))}, "position": {"line": l, "character": c}});
        let t = Duration::from_secs(30);
        match action {
            "definition" => {
                let r = self.request("textDocument/definition", pos, t).await?;
                Ok(self.fmt_locations(&r))
            }
            "references" => {
                let mut params = pos;
                params["context"] = json!({"includeDeclaration": true});
                let r = self.request("textDocument/references", params, t).await?;
                Ok(self.fmt_locations(&r))
            }
            "hover" => {
                let r = self.request("textDocument/hover", pos, t).await?;
                let c = &r["contents"];
                let text = c["value"].as_str().map(String::from).or_else(|| c.as_str().map(String::from)).unwrap_or_else(|| {
                    c.as_array().map(|a| a.iter().filter_map(|x| x["value"].as_str().or(x.as_str())).collect::<Vec<_>>().join("\n")).unwrap_or_default()
                });
                Ok(if text.trim().is_empty() { crate::i18n::t("(sin información)", "(no information)").into() } else { text.chars().take(3000).collect() })
            }
            "implementation" | "type" => {
                let metodo = if action == "type" { "textDocument/typeDefinition" } else { "textDocument/implementation" };
                let r = self.request(metodo, pos, t).await?;
                Ok(self.fmt_locations(&r))
            }
            // quién llama a esto / a quién llama esto (jerarquía de llamadas)
            "incoming" | "outgoing" => {
                let items = self.request("textDocument/prepareCallHierarchy", pos, t).await?;
                let Some(item) = items.as_array().and_then(|a| a.first()).cloned() else {
                    return Ok(crate::i18n::t("(el servidor no da la jerarquía de llamadas de ese símbolo)", "(the server gives no call hierarchy for that symbol)").into());
                };
                let (metodo, lado) = if action == "incoming" { ("callHierarchy/incomingCalls", "from") } else { ("callHierarchy/outgoingCalls", "to") };
                let r = self.request(metodo, json!({"item": item}), t).await?;
                let lineas: Vec<String> = r
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|c| {
                        let it = &c[lado];
                        let ruta = self.ruta(it["uri"].as_str().unwrap_or(""));
                        let rel = ruta.strip_prefix(&self.root).unwrap_or(&ruta).display().to_string();
                        format!("{}:{} {}", rel, it["selectionRange"]["start"]["line"].as_u64().or(it["range"]["start"]["line"].as_u64()).unwrap_or(0) + 1, it["name"].as_str().unwrap_or(""))
                    })
                    .collect();
                Ok(if lineas.is_empty() { crate::i18n::t("(nadie)", "(none)").into() } else { lineas.join("\n") })
            }
            other => bail!("{}", crate::tr!(
                "acción LSP desconocida: {other} (definicion · referencias · hover · errores · esquema · buscar_simbolo · implementaciones · tipo · quien_llama · a_quien_llama · renombrar · arreglar)",
                "unknown LSP action: {other} (definicion · referencias · hover · errores · esquema · buscar_simbolo · implementaciones · tipo · quien_llama · a_quien_llama · renombrar · arreglar)"
            )),
        }
    }

    /// F5 ML3: los CAMBIOS que propone el servidor (no los aplica): `rename` con `nuevo`, o `fix` (el arreglo
    /// preferido para los errores de la línea: un import que falta, organizar imports).
    pub async fn cambios(&mut self, action: &str, p: &Path, symbol: &str, line: Option<u64>, nuevo: Option<&str>) -> Result<Vec<(PathBuf, Vec<acciones::Edicion>)>> {
        // F16.3: lo que cambia el servidor se aplicaría en el disco de AQUÍ: en remoto, se edita con `edit`
        if self.puente.is_some() {
            bail!("{}", crate::i18n::t(
                "renombrar y los arreglos del servidor no se aplican en un proyecto REMOTO todavía: usa `edit` (y `lsp referencias` para ver dónde)",
                "rename and server fixes are not applied in a REMOTE project yet: use `edit` (and `lsp referencias` to see where)"
            ));
        }
        self.sync(p).await?;
        let u = self.u(&p.canonicalize().unwrap_or_else(|_| p.to_path_buf()));
        let t = Duration::from_secs(60);
        match acciones::normal(action) {
            "rename" => {
                let nuevo = nuevo.filter(|n| !n.trim().is_empty()).ok_or_else(|| anyhow!("{}", crate::i18n::t("falta `nuevo` (el nombre nuevo)", "missing `nuevo` (the new name)")))?;
                let texto = std::fs::read_to_string(p)?;
                let (l, c) = Self::locate(&texto, p, symbol, line)?;
                let r = self.request("textDocument/rename", json!({"textDocument": {"uri": u}, "position": {"line": l, "character": c}, "newName": nuevo}), t).await?;
                acciones::de_workspace_edit(&r)
            }
            "fix" => {
                // los diagnósticos de esa línea (o de todo el archivo si no se da)
                let diags = self.diags.lock().await.get(&u).map(|(_, d)| d.clone()).unwrap_or_default();
                let de_la_linea: Vec<Value> = diags.into_iter().filter(|d| line.is_none_or(|l| d["range"]["start"]["line"].as_u64() == Some(l.saturating_sub(1)))).collect();
                let (desde, hasta) = match line {
                    Some(l) => (l.saturating_sub(1), l.saturating_sub(1) + 1),
                    None => (0, u64::from(u32::MAX)),
                };
                let r = self
                    .request(
                        "textDocument/codeAction",
                        json!({"textDocument": {"uri": u}, "range": {"start": {"line": desde, "character": 0}, "end": {"line": hasta, "character": 0}}, "context": {"diagnostics": de_la_linea, "only": ["quickfix", "source.organizeImports"]}}),
                        t,
                    )
                    .await?;
                let acciones_srv = r.as_array().cloned().unwrap_or_default();
                // la preferida si la hay; si no, la primera que traiga su edición
                let elegida = acciones_srv.iter().find(|a| a["isPreferred"].as_bool() == Some(true) && a["edit"].is_object()).or_else(|| acciones_srv.iter().find(|a| a["edit"].is_object()));
                match elegida {
                    Some(a) => acciones::de_workspace_edit(&a["edit"]),
                    None if acciones_srv.is_empty() => bail!("{}", crate::i18n::t("el servidor no tiene arreglo para esa línea", "the server has no fix for that line")),
                    None => bail!("{}", crate::i18n::t("el arreglo del servidor es una orden suya, no una edición: GOTV no deja que el servidor escriba", "the server's fix is a command of its own, not an edit: GOTV does not let the server write")),
                }
            }
            otra => bail!("{}", crate::tr!("«{otra}» no cambia archivos", "«{otra}» does not change files")),
        }
    }
}

/// Texto para el modelo: sólo errores (y avisos si `warnings`), con línea.
pub fn format_diags(p: &Path, d: &[Diag], warnings: bool, complete: bool) -> String {
    let keep: Vec<&Diag> = d
        .iter()
        .filter(|x| x.severity == 1 || (warnings && x.severity == 2))
        .collect();
    if keep.is_empty() {
        return if complete {
            crate::tr!("[LSP] {}: sin errores", "[LSP] {}: no errors", p.display())
        } else {
            crate::tr!(
                "[LSP] {}: sin errores POR AHORA (el servidor seguía analizando; usa lsp diagnostics para confirmar)",
                "[LSP] {}: no errors SO FAR (the server was still analyzing; use lsp diagnostics to confirm)",
                p.display()
            )
        };
    }
    let mut s = crate::tr!(
        "[LSP] {} problema(s) en {}:",
        "[LSP] {} problem(s) in {}:",
        keep.len(),
        p.display()
    );
    for x in keep.iter().take(12) {
        let kind = if x.severity == 1 {
            "error"
        } else {
            crate::i18n::t("aviso", "warning")
        };
        s.push_str(&format!(
            "\n  {}:{} {}",
            x.line,
            kind,
            x.message.chars().take(220).collect::<String>()
        ));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detecta_por_archivos_del_proyecto() {
        let d = crate::CarpetaPrueba::sin_crear("gotv-lsp-det");
        std::fs::create_dir_all(&d).unwrap();
        assert_eq!(detect(&d), Err(None), "sin marcas: ningún lenguaje");
        std::fs::write(d.join("go.mod"), "module x").unwrap();
        if !on_path("gopls") {
            assert!(
                matches!(detect(&d), Err(Some(m)) if m.contains("gopls")),
                "dice qué instalar"
            );
        }
    }

    /// El caso que falló en vivo: servidor arrancado EN FRÍO sobre un archivo YA roto
    /// (la firma devuelve i64 y el que llama sigue esperando i32). Debe verse el error.
    #[tokio::test]
    async fn arranque_en_frio_no_da_falso_negativo() {
        let d = crate::CarpetaPrueba::sin_crear("gotv-lsp-frio");
        std::fs::create_dir_all(d.join("src")).unwrap();
        std::fs::write(
            d.join("Cargo.toml"),
            "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        std::fs::write(d.join("src/main.rs"), "fn sumar(a: i32, b: i32) -> i64 {\n    a as i64 + b as i64\n}\n\nfn main() {\n    let x: i32 = sumar(1, 2);\n    println!(\"{}\", x);\n}\n").unwrap();
        let Ok(server) = detect(&d) else { return };
        let mut lsp = Lsp::start(server, &d).await.unwrap();
        // tope amplio: con la Mac cargada rust-analyzer tarda en cargar el proyecto (lo normal son ~4 s; la
        // espera termina en cuanto el servidor está quieto y con sus diagnósticos)
        let bad = lsp
            .diagnostics(&d.join("src/main.rs"), Duration::from_secs(120))
            .await
            .unwrap();
        assert!(
            bad.iter().any(|x| x.severity == 1 && x.line == 6),
            "falso negativo: {bad:?}"
        );
    }

    /// ML4 · clangd de verdad (viene con Xcode): un error de C en su línea y el esquema de símbolos.
    #[tokio::test]
    async fn clangd_de_verdad() {
        let d = crate::CarpetaPrueba::sin_crear("gotv-lsp-clangd");
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("compile_flags.txt"), "-std=c11\n").unwrap();
        std::fs::write(d.join("main.c"), "int sumar(int a, int b) {\n    return a + b;\n}\n\nint main(void) {\n    return sumar(1, desconocida);\n}\n").unwrap();
        // sin clangd (Xcode) no aplica; instalado y sin detectar, es un fallo (no una prueba saltada)
        let server = match detect(&d) {
            Ok(s) => s,
            Err(e) => return assert!(!on_path("clangd"), "clangd está y no se detecta: {e:?}"),
        };
        assert_eq!(server.lang, "c");
        let mut lsp = Lsp::start(server, &d).await.unwrap();
        let f = d.join("main.c");
        let diags = lsp.diagnostics(&f, Duration::from_secs(20)).await.unwrap();
        assert!(diags.iter().any(|x| x.severity == 1 && x.line == 6 && x.message.contains("desconocida")), "{diags:?}");
        let esq = lsp.symbols(&f).await.unwrap();
        assert!(esq.contains("sumar") && esq.contains("main"), "{esq}");
    }

    /// ML4 · sourcekit-lsp de verdad (viene con Xcode): un paquete de Swift con un error de tipos.
    #[tokio::test]
    async fn sourcekit_lsp_de_verdad() {
        let d = crate::CarpetaPrueba::sin_crear("gotv-lsp-swift");
        std::fs::create_dir_all(d.join("Sources/Demo")).unwrap();
        std::fs::write(d.join("Package.swift"), "// swift-tools-version:5.9\nimport PackageDescription\nlet package = Package(name: \"Demo\", targets: [.target(name: \"Demo\")])\n").unwrap();
        std::fs::write(d.join("Sources/Demo/Demo.swift"), "func sumar(_ a: Int, _ b: Int) -> Int {\n    a + b\n}\n\nlet x: String = sumar(1, 2)\n").unwrap();
        let server = match detect(&d) {
            Ok(s) => s,
            Err(e) => return assert!(!on_path("sourcekit-lsp"), "sourcekit-lsp está y no se detecta: {e:?}"),
        };
        assert_eq!(server.lang, "swift");
        let mut lsp = Lsp::start(server, &d).await.unwrap();
        let f = d.join("Sources/Demo/Demo.swift");
        let diags = lsp.diagnostics(&f, Duration::from_secs(60)).await.unwrap();
        assert!(diags.iter().any(|x| x.severity == 1 && x.line == 5), "{diags:?}");
        let esq = lsp.symbols(&f).await.unwrap();
        assert!(esq.contains("sumar"), "{esq}");
    }

    /// Integración real con rust-analyzer (si está instalado): error de tipos tras una
    /// edición, y definición por nombre de símbolo.
    #[tokio::test]
    async fn rust_analyzer_de_verdad() {
        let d = crate::CarpetaPrueba::sin_crear("gotv-lsp-ra");
        std::fs::create_dir_all(d.join("src")).unwrap();
        std::fs::write(
            d.join("Cargo.toml"),
            "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        std::fs::write(d.join("src/main.rs"), "fn sumar(a: i32, b: i32) -> i32 {\n    a + b\n}\n\nfn main() {\n    let x: i32 = sumar(1, 2);\n    println!(\"{}\", x);\n}\n").unwrap();
        let Ok(server) = detect(&d) else { return }; // sin rust-analyzer: no aplica
        let mut lsp = Lsp::start(server, &d).await.unwrap();
        let f = d.join("src/main.rs");
        let ok = lsp.diagnostics(&f, Duration::from_secs(20)).await.unwrap();
        assert!(ok.iter().all(|x| x.severity != 1), "sano: {ok:?}");
        // el agente mete un error de tipos
        std::fs::write(&f, "fn sumar(a: i32, b: i32) -> i32 {\n    a + b\n}\n\nfn main() {\n    let x: String = sumar(1, 2);\n    println!(\"{}\", x);\n}\n").unwrap();
        let bad = lsp.diagnostics(&f, Duration::from_secs(20)).await.unwrap();
        assert!(
            bad.iter()
                .any(|x| x.severity == 1 && x.line == 6 && x.message.contains("mismatched")),
            "{bad:?}"
        );
        let def = lsp.query("definition", &f, "sumar", Some(6)).await.unwrap();
        assert!(def.starts_with("src/main.rs:1:"), "{def}");
        // D8d: el esquema del archivo y un nombre en todo el proyecto, por el servidor
        let esq = lsp.query("symbols", &f, "", None).await.unwrap();
        assert_eq!(esq, "1: fn sumar\n5: fn main", "{esq}");
        let ws = lsp.query("workspace", &d, "sumar", None).await.unwrap();
        assert!(ws.is_empty() || ws.starts_with("src/main.rs:1: fn sumar"), "{ws}");
        // y el respaldo sin servidor dice lo mismo del mismo archivo
        assert_eq!(crate::mapa::esquema(&f, &std::fs::read_to_string(&f).unwrap()), esq);
        // F5 ML3 con el servidor real: quién llama, y renombrar (rust-analyzer manda `documentChanges`)
        assert!(lsp.query("quien_llama", &f, "sumar", Some(1)).await.unwrap().contains("main"), "quien_llama");
        let c = lsp.cambios("renombrar", &f, "sumar", Some(1), Some("adicion")).await.unwrap();
        acciones::aplicar_en_disco(&d, &c).unwrap();
        let t = std::fs::read_to_string(&f).unwrap();
        assert!(t.contains("fn adicion(") && t.contains("adicion(1, 2)") && !t.contains("sumar"), "{t}");
    }
}
