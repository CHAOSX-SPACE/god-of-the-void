//! F23.4 · Cliente CDP propio sobre `tokio-tungstenite` (ya en el árbol; sin `chromiumoxide`).
//!
//! Chrome se lanza con `--remote-debugging-port=0` y un perfil temporal; el puerto se lee del archivo
//! `DevToolsActivePort` de ese perfil. Una pestaña, en modo «flatten» (todo por una conexión).
//! Lectura sin capturas: `Accessibility.getFullAXTree` → texto con referencias `@eN`; clic y escritura
//! por referencia (`backendDOMNodeId`); la captura sólo si se pide. Las descargas, negadas.

use super::Listas;
use anyhow::{anyhow, Result};
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{broadcast, mpsc, oneshot};
use tokio_tungstenite::tungstenite::Message;

type Pendientes = Arc<Mutex<HashMap<u64, oneshot::Sender<std::result::Result<Value, String>>>>>;

/// Una conexión CDP: peticiones con `id` y eventos sin él.
pub struct Cdp {
    salida: mpsc::UnboundedSender<String>,
    pendientes: Pendientes,
    eventos: broadcast::Sender<Value>,
    siguiente: AtomicU64,
}

impl Cdp {
    pub async fn conectar(ws: &str) -> Result<Cdp> {
        let (conexion, _) = tokio_tungstenite::connect_async(ws).await.map_err(|e| anyhow!("CDP: {e}"))?;
        let (mut tx, mut rx) = conexion.split();
        let (salida, mut cola) = mpsc::unbounded_channel::<String>();
        let pendientes: Pendientes = Default::default();
        let (eventos, _) = broadcast::channel(512);
        tokio::spawn(async move {
            while let Some(m) = cola.recv().await {
                if tx.send(Message::text(m)).await.is_err() {
                    break;
                }
            }
        });
        let (p, ev) = (pendientes.clone(), eventos.clone());
        tokio::spawn(async move {
            while let Some(Ok(m)) = rx.next().await {
                let Message::Text(t) = m else { continue };
                let Ok(v) = serde_json::from_str::<Value>(t.as_str()) else { continue };
                match v.get("id").and_then(|i| i.as_u64()) {
                    Some(id) => {
                        if let Some(s) = p.lock().unwrap_or_else(|e| e.into_inner()).remove(&id) {
                            let r = match v.get("error") {
                                Some(e) => Err(e["message"].as_str().unwrap_or("error").to_string()),
                                None => Ok(v.get("result").cloned().unwrap_or(Value::Null)),
                            };
                            let _ = s.send(r);
                        }
                    }
                    None => {
                        let _ = ev.send(v);
                    }
                }
            }
            // la conexión cayó: nadie se queda esperando
            for (_, s) in p.lock().unwrap_or_else(|e| e.into_inner()).drain() {
                let _ = s.send(Err("CDP cerrado".into())); // juez-idioma: datos (error de protocolo)
            }
        });
        Ok(Cdp { salida, pendientes, eventos, siguiente: AtomicU64::new(1) })
    }

    pub fn eventos(&self) -> broadcast::Receiver<Value> {
        self.eventos.subscribe()
    }

    pub async fn llamar(&self, sesion: Option<&str>, metodo: &str, params: Value) -> Result<Value> {
        let id = self.siguiente.fetch_add(1, Ordering::SeqCst);
        let mut m = json!({ "id": id, "method": metodo, "params": params });
        if let Some(s) = sesion {
            m["sessionId"] = json!(s);
        }
        let (tx, rx) = oneshot::channel();
        self.pendientes.lock().unwrap_or_else(|e| e.into_inner()).insert(id, tx);
        self.salida.send(m.to_string()).map_err(|_| anyhow!("CDP: {}", crate::i18n::t("conexión cerrada", "connection closed")))?;
        match tokio::time::timeout(Duration::from_secs(30), rx).await {
            Ok(Ok(Ok(v))) => Ok(v),
            Ok(Ok(Err(e))) => Err(anyhow!("CDP {metodo}: {e}")),
            Ok(Err(_)) => Err(anyhow!("CDP {metodo}: {}", crate::i18n::t("conexión cerrada", "connection closed"))),
            Err(_) => {
                self.pendientes.lock().unwrap_or_else(|e| e.into_inner()).remove(&id);
                Err(anyhow!("CDP {metodo}: {}", crate::i18n::t("sin respuesta en 30 s", "no answer within 30 s")))
            }
        }
    }
}

/// El Chromium a usar: `GOTV_CHROMIUM`, el chrome-headless-shell de Playwright si está, Chromium o
/// Google Chrome (siempre con perfil temporal).
pub fn chromium() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("GOTV_CHROMIUM").map(PathBuf::from).filter(|p| p.is_file()) {
        return Some(p);
    }
    let pw = crate::paths::home().join("Library/Caches/ms-playwright");
    if let Ok(rd) = std::fs::read_dir(&pw) {
        let mut v: Vec<PathBuf> = rd
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("chromium_headless_shell-")))
            .collect();
        v.sort();
        for d in v.into_iter().rev() {
            for arq in ["chrome-headless-shell-mac-arm64", "chrome-headless-shell-mac-x64"] {
                let b = d.join(arq).join("chrome-headless-shell");
                if b.is_file() {
                    return Some(b);
                }
            }
        }
    }
    [
        "/Applications/Chromium.app/Contents/MacOS/Chromium",
        "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
    ]
    .iter()
    .map(PathBuf::from)
    .find(|p| p.is_file())
}

/// Un nodo del árbol de accesibilidad, ya reducido.
struct Nodo {
    rol: String,
    nombre: String,
    nivel: Option<i64>,
    backend: Option<i64>,
    hijos: Vec<String>,
    ignorado: bool,
}

/// Roles que reciben referencia (se pueden tocar).
const INTERACTIVOS: &[&str] = &[
    "button", "link", "textbox", "searchbox", "checkbox", "radio", "combobox", "listbox", "option", "menuitem",
    "menuitemcheckbox", "menuitemradio", "tab", "switch", "slider", "spinbutton", "treeitem",
]; // juez-idioma: datos (roles ARIA)

/// El árbol `Accessibility.getFullAXTree` como texto, al estilo del `snapshot` de agent-browser
/// (`- button "Pulsar" [ref=e2]`). Devuelve el texto y la tabla `eN → backendDOMNodeId`.
pub fn arbol_a_texto(nodos: &[Value], solo_interactivos: bool) -> (String, HashMap<String, i64>) {
    let mut m: HashMap<String, Nodo> = HashMap::new();
    let mut raices: Vec<String> = Vec::new();
    for n in nodos {
        let id = n["nodeId"].as_str().unwrap_or("").to_string();
        let nivel = n["properties"].as_array().and_then(|p| p.iter().find(|x| x["name"] == "level")).and_then(|x| x["value"]["value"].as_i64());
        if n.get("parentId").is_none() {
            raices.push(id.clone());
        }
        m.insert(
            id,
            Nodo {
                rol: n["role"]["value"].as_str().unwrap_or("").to_string(),
                nombre: n["name"]["value"].as_str().unwrap_or("").to_string(),
                nivel,
                backend: n["backendDOMNodeId"].as_i64(),
                hijos: n["childIds"].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default(),
                ignorado: n["ignored"].as_bool().unwrap_or(false),
            },
        );
    }
    let mut out = String::new();
    let mut refs = HashMap::new();
    fn visitar(id: &str, prof: usize, m: &HashMap<String, Nodo>, solo: bool, out: &mut String, refs: &mut HashMap<String, i64>, vistos: &mut std::collections::HashSet<String>) {
        if !vistos.insert(id.to_string()) {
            return;
        }
        let Some(n) = m.get(id) else { return };
        let interactivo = INTERACTIVOS.contains(&n.rol.as_str());
        let mostrar = !n.ignorado
            && match (solo, n.rol.as_str()) {
                (true, r) => interactivo || r == "heading",
                (false, "generic" | "none" | "RootWebArea" | "InlineTextBox" | "LineBreak" | "") => false,
                (false, "StaticText") => !n.nombre.trim().is_empty(),
                (false, _) => !n.nombre.trim().is_empty() || interactivo,
            };
        let mut sub = prof;
        if mostrar {
            let mut attrs: Vec<String> = Vec::new();
            if let Some(l) = n.nivel.filter(|_| n.rol == "heading") {
                attrs.push(format!("level={l}"));
            }
            if let (true, Some(b)) = (interactivo || n.rol == "heading", n.backend) {
                let r = format!("e{}", refs.len() + 1);
                refs.insert(r.clone(), b);
                attrs.push(format!("ref={r}"));
            }
            let rol = if n.rol == "StaticText" { "text" } else { n.rol.as_str() };
            let sangria = if solo { String::new() } else { "  ".repeat(prof) };
            let nombre = n.nombre.replace('"', "'");
            out.push_str(&format!("{sangria}- {rol} \"{nombre}\""));
            if !attrs.is_empty() {
                out.push_str(&format!(" [{}]", attrs.join(", ")));
            }
            out.push('\n');
            sub = prof + 1;
        }
        for h in &n.hijos {
            visitar(h, sub, m, solo, out, refs, vistos);
        }
    }
    let mut vistos = std::collections::HashSet::new();
    for r in &raices {
        visitar(r, 0, &m, solo_interactivos, &mut out, &mut refs, &mut vistos);
    }
    (out, refs)
}

/// Un Chromium aislado con una pestaña, hablado por CDP.
pub struct Navegador {
    cdp: Cdp,
    hijo: Option<tokio::process::Child>,
    perfil: Option<PathBuf>,
    sesion: String,
    refs: HashMap<String, i64>,
    listas: Listas,
}

fn perfil_temporal() -> Result<PathBuf> {
    static N: AtomicU64 = AtomicU64::new(0);
    let p = std::env::temp_dir().join(format!("gotv-cdp-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
    std::fs::create_dir_all(&p)?;
    Ok(p)
}

async fn esperar_puerto(perfil: &Path) -> Result<(u16, String)> {
    let archivo = perfil.join("DevToolsActivePort");
    for _ in 0..300 {
        if let Ok(t) = std::fs::read_to_string(&archivo) {
            let mut l = t.lines();
            if let (Some(p), Some(ruta)) = (l.next().and_then(|x| x.trim().parse().ok()), l.next()) {
                return Ok((p, ruta.trim().to_string()));
            }
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    Err(anyhow!("{}", crate::i18n::t("Chromium no abrió su puerto de depuración en 15 s", "Chromium did not open its debugging port within 15 s")))
}

impl Navegador {
    /// Lanza un Chromium aislado (perfil temporal) y abre una pestaña en blanco.
    pub async fn lanzar(listas: Listas) -> Result<Navegador> {
        let bin = chromium().ok_or_else(|| anyhow!("{}", crate::i18n::t(
            "no hay un Chromium para el navegador CDP (instala Google Chrome o pon GOTV_CHROMIUM)",
            "there is no Chromium for the CDP browser (install Google Chrome or set GOTV_CHROMIUM)"
        )))?;
        let perfil = perfil_temporal()?;
        let mut c = tokio::process::Command::new(&bin);
        if !bin.to_string_lossy().contains("headless-shell") {
            c.arg("--headless=new");
        }
        c.arg("--remote-debugging-port=0")
            .arg(format!("--user-data-dir={}", perfil.display()))
            .args(["--no-first-run", "--no-default-browser-check", "--disable-extensions", "--disable-sync", "--disable-background-networking", "--mute-audio", "--password-store=basic", "--use-mock-keychain"])
            .arg("about:blank")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true);
        let hijo = c.spawn().map_err(|e| anyhow!("Chromium: {e}"))?;
        // si algo falla de aquí en adelante, soltar `n` mata el proceso y borra el perfil
        let mut n = Navegador { cdp: Cdp::muerto(), hijo: Some(hijo), perfil: Some(perfil.clone()), sesion: String::new(), refs: HashMap::new(), listas };
        let (puerto, ruta) = esperar_puerto(&perfil).await?;
        n.cdp = Cdp::conectar(&format!("ws://127.0.0.1:{puerto}{ruta}")).await?;
        n.preparar().await?;
        Ok(n)
    }

    /// Sobre una conexión ya hecha (pruebas con un CDP falso).
    pub async fn con(cdp: Cdp, listas: Listas) -> Result<Navegador> {
        let mut n = Navegador { cdp, hijo: None, perfil: None, sesion: String::new(), refs: HashMap::new(), listas };
        n.preparar().await?;
        Ok(n)
    }

    async fn preparar(&mut self) -> Result<()> {
        // descargas negadas en todo el navegador
        let _ = self.cdp.llamar(None, "Browser.setDownloadBehavior", json!({ "behavior": "deny" })).await;
        let t = self.cdp.llamar(None, "Target.createTarget", json!({ "url": "about:blank" })).await?;
        let objetivo = t["targetId"].as_str().unwrap_or("").to_string();
        let s = self.cdp.llamar(None, "Target.attachToTarget", json!({ "targetId": objetivo, "flatten": true })).await?;
        self.sesion = s["sessionId"].as_str().ok_or_else(|| anyhow!("CDP: sin sessionId"))?.to_string(); // juez-idioma: datos (protocolo)
        self.llamar("Page.enable", json!({})).await?;
        Ok(())
    }

    async fn llamar(&self, metodo: &str, p: Value) -> Result<Value> {
        self.cdp.llamar(Some(&self.sesion), metodo, p).await
    }

    /// Abre `url` y espera su `load` (20 s como mucho). Devuelve el título.
    pub async fn ir(&mut self, url: &str) -> Result<String> {
        if !self.listas.abrir.deja(url) {
            return Err(anyhow!("{}", self.listas.abrir.porque(url)));
        }
        let mut ev = self.cdp.eventos();
        let r = self.llamar("Page.navigate", json!({ "url": url })).await?;
        if let Some(e) = r["errorText"].as_str().filter(|e| !e.is_empty()) {
            return Err(anyhow!("{url}: {e}"));
        }
        let sesion = self.sesion.clone();
        let _ = tokio::time::timeout(Duration::from_secs(20), async {
            while let Ok(v) = ev.recv().await {
                if v["method"] == "Page.loadEventFired" && v["sessionId"].as_str().is_none_or(|s| s == sesion) {
                    break;
                }
            }
        })
        .await;
        self.refs.clear();
        Ok(self.evaluar("document.title").await?.as_str().unwrap_or("").to_string())
    }

    /// El árbol de accesibilidad como texto (sin capturas); `solo_interactivos` = como `snapshot -i`.
    pub async fn arbol(&mut self, solo_interactivos: bool) -> Result<String> {
        let r = self.llamar("Accessibility.getFullAXTree", json!({})).await?;
        let (t, refs) = arbol_a_texto(r["nodes"].as_array().map(|a| a.as_slice()).unwrap_or(&[]), solo_interactivos);
        self.refs = refs;
        Ok(t)
    }

    fn backend(&self, r: &str) -> Result<i64> {
        self.refs.get(r.trim_start_matches('@')).copied().ok_or_else(|| anyhow!("{}", crate::tr!(
            "la referencia {r} ya no existe (cambian con cada página): pide el árbol de nuevo",
            "reference {r} no longer exists (they change with each page): ask for the tree again"
        )))
    }

    /// Clic por referencia: al centro de su caja; si no tiene caja, `click()` del DOM.
    pub async fn clic(&mut self, r: &str) -> Result<()> {
        let b = self.backend(r)?;
        let _ = self.llamar("DOM.scrollIntoViewIfNeeded", json!({ "backendNodeId": b })).await;
        match self.llamar("DOM.getBoxModel", json!({ "backendNodeId": b })).await {
            Ok(m) => {
                let q: Vec<f64> = m["model"]["content"].as_array().map(|a| a.iter().filter_map(|x| x.as_f64()).collect()).unwrap_or_default();
                if q.len() < 8 {
                    return self.funcion(b, "function(){this.click()}").await.map(|_| ());
                }
                let (x, y) = ((q[0] + q[2] + q[4] + q[6]) / 4.0, (q[1] + q[3] + q[5] + q[7]) / 4.0);
                for tipo in ["mouseMoved", "mousePressed", "mouseReleased"] {
                    self.llamar("Input.dispatchMouseEvent", json!({ "type": tipo, "x": x, "y": y, "button": "left", "clickCount": 1 })).await?;
                }
                Ok(())
            }
            Err(_) => self.funcion(b, "function(){this.click()}").await.map(|_| ()),
        }
    }

    /// Escribe en un campo por referencia (borra lo que tuviera).
    pub async fn escribir(&mut self, r: &str, texto: &str) -> Result<()> {
        let b = self.backend(r)?;
        self.llamar("DOM.focus", json!({ "backendNodeId": b })).await?;
        self.funcion(b, "function(){ if ('value' in this) { this.value = ''; } }").await?;
        self.llamar("Input.insertText", json!({ "text": texto })).await?;
        Ok(())
    }

    async fn funcion(&self, backend: i64, f: &str) -> Result<Value> {
        let o = self.llamar("DOM.resolveNode", json!({ "backendNodeId": backend })).await?;
        let id = o["object"]["objectId"].as_str().ok_or_else(|| anyhow!("CDP: sin objectId"))?; // juez-idioma: datos (protocolo)
        self.llamar("Runtime.callFunctionOn", json!({ "objectId": id, "functionDeclaration": f, "returnByValue": true })).await
    }

    pub async fn evaluar(&self, expr: &str) -> Result<Value> {
        let r = self.llamar("Runtime.evaluate", json!({ "expression": expr, "returnByValue": true, "awaitPromise": true })).await?;
        if let Some(e) = r.get("exceptionDetails") {
            return Err(anyhow!("JS: {}", e["text"].as_str().unwrap_or("error")));
        }
        Ok(r["result"]["value"].clone())
    }

    /// El texto principal de la página (sin menús ni pies), como lo guarda el índice.
    pub async fn texto(&self) -> Result<String> {
        let html = self.evaluar("document.documentElement.outerHTML").await?;
        Ok(crate::indice::rastreador::contenido_principal(html.as_str().unwrap_or("")))
    }

    /// El HTML ya armado por JavaScript.
    pub async fn html(&self) -> Result<String> {
        Ok(self.evaluar("document.documentElement.outerHTML").await?.as_str().unwrap_or("").to_string())
    }

    /// Sólo cuando el modelo la pide.
    pub async fn captura(&self) -> Result<Vec<u8>> {
        use base64::Engine;
        let r = self.llamar("Page.captureScreenshot", json!({ "format": "png" })).await?;
        Ok(base64::engine::general_purpose::STANDARD.decode(r["data"].as_str().unwrap_or(""))?)
    }

    /// Cierra el navegador y borra SU perfil temporal (ruta exacta, L6).
    pub async fn cerrar(mut self) {
        let _ = tokio::time::timeout(Duration::from_secs(3), self.cdp.llamar(None, "Browser.close", json!({}))).await;
        if let Some(mut h) = self.hijo.take() {
            if tokio::time::timeout(Duration::from_secs(3), h.wait()).await.is_err() {
                let _ = h.kill().await;
            }
        }
        if let Some(p) = self.perfil.take() {
            let _ = std::fs::remove_dir_all(&p);
        }
    }
}

impl Drop for Navegador {
    fn drop(&mut self) {
        if let Some(h) = self.hijo.as_mut() {
            let _ = h.start_kill();
        }
        if let Some(p) = self.perfil.take() {
            let _ = std::fs::remove_dir_all(&p);
        }
    }
}

impl Cdp {
    /// Una conexión que no lleva a ningún lado (sólo como valor inicial mientras Chromium arranca).
    fn muerto() -> Cdp {
        let (salida, _) = mpsc::unbounded_channel();
        let (eventos, _) = broadcast::channel(1);
        Cdp { salida, pendientes: Default::default(), eventos, siguiente: AtomicU64::new(1) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nodo(id: &str, padre: Option<&str>, rol: &str, nombre: &str, backend: i64, hijos: &[&str]) -> Value {
        let mut v = json!({"nodeId": id, "role": {"value": rol}, "name": {"value": nombre}, "backendDOMNodeId": backend, "childIds": hijos, "ignored": false});
        if let Some(p) = padre {
            v["parentId"] = json!(p);
        }
        v
    }

    fn arbol_ejemplo() -> Vec<Value> {
        let mut h = nodo("2", Some("1"), "heading", "Hola", 10, &[]);
        h["properties"] = json!([{"name": "level", "value": {"type": "integer", "value": 1}}]);
        vec![
            nodo("1", None, "RootWebArea", "Prueba CDP", 1, &["2", "3", "4", "5", "6"]),
            h,
            nodo("3", Some("1"), "StaticText", "Texto principal", 11, &[]),
            nodo("4", Some("1"), "button", "Pulsar", 12, &[]),
            nodo("5", Some("1"), "textbox", "nombre", 13, &[]),
            json!({"nodeId": "6", "parentId": "1", "role": {"value": "button"}, "name": {"value": "oculto"}, "ignored": true, "childIds": []}),
        ]
    }

    #[test]
    fn el_arbol_como_el_snapshot_de_hoy() {
        let (t, refs) = arbol_a_texto(&arbol_ejemplo(), true);
        assert_eq!(t, "- heading \"Hola\" [level=1, ref=e1]\n- button \"Pulsar\" [ref=e2]\n- textbox \"nombre\" [ref=e3]\n");
        assert_eq!(refs["e2"], 12);
        let (t, _) = arbol_a_texto(&arbol_ejemplo(), false);
        assert!(t.contains("- text \"Texto principal\"") && !t.contains("oculto"), "{t}");
    }

    /// Un CDP FALSO (WebSocket local) que contesta como Chrome: sin navegador real.
    async fn cdp_falso(registro: Arc<Mutex<Vec<Value>>>) -> String {
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let dir = format!("ws://{}/devtools/browser/x", l.local_addr().unwrap());
        tokio::spawn(async move {
            let (s, _) = l.accept().await.unwrap();
            let ws = tokio_tungstenite::accept_async(s).await.unwrap();
            let (mut tx, mut rx) = ws.split();
            while let Some(Ok(Message::Text(t))) = rx.next().await {
                let v: Value = serde_json::from_str(t.as_str()).unwrap();
                registro.lock().unwrap().push(v.clone());
                let res = match v["method"].as_str().unwrap() {
                    "Target.createTarget" => json!({"targetId": "T1"}),
                    "Target.attachToTarget" => json!({"sessionId": "S1"}),
                    "Accessibility.getFullAXTree" => json!({"nodes": arbol_ejemplo()}),
                    "DOM.getBoxModel" => json!({"model": {"content": [10.0, 20.0, 30.0, 20.0, 30.0, 40.0, 10.0, 40.0]}}),
                    "DOM.resolveNode" => json!({"object": {"objectId": "O1"}}),
                    "Runtime.evaluate" => json!({"result": {"type": "string", "value": "Prueba CDP"}}),
                    "Page.navigate" => json!({"frameId": "F1"}),
                    _ => json!({}),
                };
                tx.send(Message::text(json!({"id": v["id"], "result": res}).to_string())).await.unwrap();
                if v["method"] == "Page.navigate" {
                    tx.send(Message::text(json!({"method": "Page.loadEventFired", "sessionId": "S1", "params": {}}).to_string())).await.unwrap();
                }
            }
        });
        dir
    }

    #[tokio::test]
    async fn contra_un_cdp_falso() {
        let reg = Arc::new(Mutex::new(Vec::new()));
        let dir = cdp_falso(reg.clone()).await;
        let listas = Listas::de_texto("abrir negar malo.io");
        let mut n = Navegador::con(Cdp::conectar(&dir).await.unwrap(), listas).await.unwrap();
        assert_eq!(n.ir("https://ejemplo.io/").await.unwrap(), "Prueba CDP");
        assert!(n.ir("https://malo.io/").await.is_err(), "F7: negado ni se pide");
        assert!(n.clic("@e2").await.is_err(), "sin árbol no hay referencias");
        let t = n.arbol(true).await.unwrap();
        assert!(t.contains("[ref=e2]"));
        n.clic("@e2").await.unwrap();
        n.escribir("e3", "Ana").await.unwrap();
        let r = reg.lock().unwrap().clone();
        let metodos: Vec<&str> = r.iter().filter_map(|v| v["method"].as_str()).collect();
        assert!(metodos.contains(&"Browser.setDownloadBehavior"), "descargas negadas");
        assert!(r.iter().any(|v| v["method"] == "Browser.setDownloadBehavior" && v["params"]["behavior"] == "deny"));
        // el clic cae en el centro de la caja: (20, 30)
        let clic = r.iter().find(|v| v["method"] == "Input.dispatchMouseEvent" && v["params"]["type"] == "mousePressed").unwrap();
        assert_eq!((clic["params"]["x"].as_f64(), clic["params"]["y"].as_f64()), (Some(20.0), Some(30.0)));
        assert!(r.iter().any(|v| v["method"] == "Input.insertText" && v["params"]["text"] == "Ana"));
        // todo lo de la pestaña va con su sesión
        assert!(r.iter().filter(|v| v["method"] == "Page.navigate").all(|v| v["sessionId"] == "S1"));
        assert_eq!(metodos.iter().filter(|m| **m == "Page.navigate").count(), 1, "malo.io no se pidió");
    }

    /// L10 · Lo que cuesta cada acción del `browser` de hoy (agent-browser, un proceso por acción con su
    /// daemon), sobre la misma página que `cdp_de_verdad`, con el mismo Chromium aislado.
    /// `GOTV_PRUEBA_CHROME=1 cargo test … -- --ignored agent_browser_medido --nocapture`.
    #[tokio::test]
    #[ignore]
    async fn agent_browser_medido() {
        if std::env::var("GOTV_PRUEBA_CHROME").as_deref() != Ok("1") {
            return;
        }
        let (Some(ab), Some(chrome)) = (crate::browser::binary(), chromium()) else {
            eprintln!("sin agent-browser o sin Chromium: no se mide");
            return;
        };
        use axum::{response::Html, routing::get, Router};
        let app = Router::new().route("/", get(|| async { Html(r#"<html><head><title>Prueba</title></head><body><main><h1>Hola</h1><button id="b" onclick="this.textContent='ok'">Pulsar</button><input id="i"></main></body></html>"#) }));
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/", l.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
        let sesion = format!("gotv-medida-{}", std::process::id());
        let correr = |args: Vec<String>| {
            let (ab, chrome, sesion) = (ab.clone(), chrome.clone(), sesion.clone());
            async move {
                let t = std::time::Instant::now();
                let o = tokio::process::Command::new(ab).args(&args).env("AGENT_BROWSER_SESSION", sesion).env("AGENT_BROWSER_SOCKET_DIR", crate::browser::carpeta_sockets()).env("AGENT_BROWSER_EXECUTABLE_PATH", chrome).output().await.unwrap();
                (args.join(" "), o.status.success(), t.elapsed())
            }
        };
        let mut v = vec![correr(vec!["open".into(), url.clone()]).await];
        for _ in 0..3 {
            v.push(correr(vec!["snapshot".into(), "-i".into()]).await);
        }
        v.push(correr(vec!["get".into(), "text".into(), "body".into()]).await);
        v.push(correr(vec!["click".into(), "#b".into()]).await);
        v.push(correr(vec!["fill".into(), "#i".into(), "Ana".into()]).await);
        v.push(correr(vec!["close".into()]).await);
        for (a, ok, t) in &v {
            eprintln!("agent-browser {a}: {} · {:.1} ms", if *ok { "ok" } else { "FALLÓ" }, t.as_secs_f64() * 1_000.0);
        }
        assert!(v.iter().all(|x| x.1));
    }

    /// Con Chromium de verdad (sin ventana, perfil temporal): `GOTV_PRUEBA_CHROME=1 cargo test … -- --ignored cdp_de_verdad`.
    #[tokio::test]
    #[ignore]
    async fn cdp_de_verdad() {
        if std::env::var("GOTV_PRUEBA_CHROME").as_deref() != Ok("1") {
            return;
        }
        use axum::{response::Html, routing::get, Router};
        let app = Router::new().route("/", get(|| async {
            Html(r#"<html><head><title>Prueba CDP</title></head><body><main><h1>Hola</h1><p>Texto principal de la prueba.</p>
<button id="b" onclick="document.getElementById('r').textContent='pulsado'">Pulsar</button>
<input id="i" placeholder="nombre"><p id="r">sin pulsar</p></main><script>document.title = 'Prueba CDP';</script></body></html>"#)
        }));
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/", l.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
        let t = std::time::Instant::now();
        let mut n = Navegador::lanzar(Listas::default()).await.unwrap();
        let perfil = n.perfil.clone().unwrap();
        let lanzar = t.elapsed();
        let t = std::time::Instant::now();
        assert_eq!(n.ir(&url).await.unwrap(), "Prueba CDP");
        let abrir = t.elapsed();
        let mut ms = Vec::new();
        for _ in 0..3 {
            let t = std::time::Instant::now();
            let a = n.arbol(true).await.unwrap();
            ms.push(("arbol", t.elapsed()));
            assert!(a.contains("button \"Pulsar\""), "{a}");
        }
        let b = n.refs.iter().find(|(_, _)| true).map(|_| ()).is_some();
        assert!(b);
        let r_boton = n.arbol(true).await.unwrap().lines().find(|l| l.contains("Pulsar")).and_then(|l| l.split("ref=").nth(1)).map(|x| x.trim_end_matches(']').to_string()).unwrap();
        let r_campo = n.arbol(true).await.unwrap().lines().find(|l| l.contains("textbox")).and_then(|l| l.split("ref=").nth(1)).map(|x| x.trim_end_matches(']').to_string()).unwrap();
        let t = std::time::Instant::now();
        n.clic(&r_boton).await.unwrap();
        ms.push(("clic", t.elapsed()));
        let t = std::time::Instant::now();
        n.escribir(&r_campo, "Ana").await.unwrap();
        ms.push(("escribir", t.elapsed()));
        let t = std::time::Instant::now();
        let texto = n.texto().await.unwrap();
        ms.push(("texto", t.elapsed()));
        assert!(texto.contains("pulsado"), "{texto}");
        assert_eq!(n.evaluar("document.getElementById('i').value").await.unwrap(), "Ana");
        eprintln!("CDP: lanzar {lanzar:?} · abrir {abrir:?} · {ms:?}");
        n.cerrar().await;
        assert!(!perfil.exists(), "el perfil temporal se borró");
    }
}
