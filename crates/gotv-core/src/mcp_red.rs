//! F13.3 · Servidores MCP REMOTOS: HTTP «streamable», SSE clásico y WebSocket.
//!
//! Cada transporte es un PUENTE entre un `duplex` en memoria y la red: del lado de GOTV habla el mismo
//! `Canal` de siempre (JSON-RPC por líneas, con sus ids, topes, eventos y respondedor), así el cliente MCP
//! (`mcp.rs`) no cambia nada según el transporte. Del lado de la red:
//! - **HTTP streamable** (MCP 2025-03-26+): un POST por mensaje con `Accept: application/json,
//!   text/event-stream`; la respuesta es JSON (uno o un lote) o un flujo SSE con los mensajes; la sesión
//!   viaja en `Mcp-Session-Id` y la versión acordada en `MCP-Protocol-Version`. Un 202 = aviso aceptado.
//! - **SSE clásico** (MCP 2024-11-05): un GET abre el flujo; su primer evento `endpoint` dice a qué URL
//!   mandar los mensajes por POST; las respuestas llegan por el flujo.
//! - **WebSocket**: un mensaje de texto por mensaje JSON-RPC (subprotocolo `mcp`).
//!
//! Si la red falla al enviar: reintentos a 1, 2, 4, 8 y 16 s. Si no hay forma, la petición recibe un error
//! JSON-RPC (el modelo lo ve claro) en vez de quedarse esperando su tope. Las cabeceras (con sus
//! `${VAR}` ya expandidas) nunca se escriben en un registro.
use crate::mcp::{ServerConfig, Tipo};
use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, ReadHalf, WriteHalf};
use tokio::sync::Mutex;

type Salida = Arc<Mutex<WriteHalf<tokio::io::DuplexStream>>>;
type Entrada = tokio::io::Lines<BufReader<ReadHalf<tokio::io::DuplexStream>>>;
type Cabeceras = Arc<std::sync::Mutex<std::collections::BTreeMap<String, String>>>;

const ESPERAS: [u64; 5] = [1, 2, 4, 8, 16];

/// Abre el transporte remoto y devuelve el `Canal` listo para `initialize`.
pub async fn abrir(c: &ServerConfig) -> Result<crate::canal::Canal> {
    let url = c.url.clone().ok_or_else(|| anyhow!("{}", crate::tr!("MCP «{}»: falta `url`", "MCP «{}»: missing `url`", c.name)))?;
    let (gotv, red) = tokio::io::duplex(1 << 20);
    let (rr, rw) = tokio::io::split(red);
    let entrada: Entrada = BufReader::new(rr).lines();
    let salida: Salida = Arc::new(Mutex::new(rw));
    let mut headers = c.headers.clone();
    // F13.4: con sesión OAuth guardada y sin `Authorization` propia, el token vigente del Llavero
    if !headers.keys().any(|k| k.eq_ignore_ascii_case("authorization")) {
        if let Some(t) = crate::mcp_oauth::vigente(&c.name).await {
            headers.insert("Authorization".into(), format!("Bearer {t}"));
        }
    }
    match c.tipo {
        Tipo::Http => {
            tokio::spawn(puente_http(c.name.clone(), url, headers, entrada, salida));
        }
        Tipo::Sse => {
            // el endpoint tiene que llegar ANTES de dar el canal por abierto: si no, es un error claro aquí
            let endpoint = abrir_sse(&url, &headers, salida.clone()).await?;
            tokio::spawn(puente_sse_envios(endpoint, headers, entrada, salida));
        }
        Tipo::Ws => {
            let ws = abrir_ws(&url, &headers).await?;
            tokio::spawn(puente_ws(ws, entrada, salida));
        }
        Tipo::Stdio => return Err(anyhow!("{}", crate::i18n::t("stdio no va por la red", "stdio does not go over the network"))),
    }
    let (gr, gw) = tokio::io::split(gotv);
    Ok(crate::canal::Canal::sobre(gr, gw, crate::canal::Marco::Linea, &format!("MCP «{}»", c.name)))
}

fn cliente() -> reqwest::Client {
    reqwest::Client::builder().connect_timeout(Duration::from_secs(10)).build().unwrap_or_default()
}

async fn escribir(salida: &Salida, m: &Value) {
    let _ = salida.lock().await.write_all(format!("{m}\n").as_bytes()).await;
}

/// La respuesta de error para la petición `linea` (si es una petición: los avisos no esperan respuesta).
async fn error_para(salida: &Salida, linea: &str, motivo: &str) {
    if let Some(id) = serde_json::from_str::<Value>(linea).ok().and_then(|m| m.get("method").and(m.get("id").cloned())) {
        escribir(salida, &json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32000, "message": motivo}})).await;
    }
}

/// Lector de Server-Sent Events: se le dan trozos y devuelve los eventos completos (nombre, datos).
#[derive(Default)]
pub struct LectorSse {
    buf: String,
    evento: String,
    datos: Vec<String>,
}

impl LectorSse {
    pub fn alimentar(&mut self, trozo: &str) -> Vec<(String, String)> {
        self.buf.push_str(trozo);
        let mut out = Vec::new();
        while let Some(i) = self.buf.find('\n') {
            let linea: String = self.buf.drain(..=i).collect();
            let linea = linea.trim_end_matches(['\n', '\r']);
            if linea.is_empty() {
                if !self.datos.is_empty() {
                    let nombre = if self.evento.is_empty() { "message".to_string() } else { std::mem::take(&mut self.evento) };
                    out.push((nombre, self.datos.join("\n")));
                }
                self.datos.clear();
                self.evento.clear();
            } else if let Some(d) = linea.strip_prefix("data:") {
                self.datos.push(d.strip_prefix(' ').unwrap_or(d).to_string());
            } else if let Some(e) = linea.strip_prefix("event:") {
                self.evento = e.trim().to_string();
            }
            // `id:`, `retry:` y comentarios (`:`) no hacen falta aquí
        }
        out
    }
}

/// Lleva a GOTV cada mensaje de un cuerpo (JSON suelto o lote) y anota la versión acordada.
async fn entregar(salida: &Salida, texto: &str, version: &Arc<std::sync::Mutex<Option<String>>>) {
    let Ok(v) = serde_json::from_str::<Value>(texto) else { return };
    let mensajes = match v {
        Value::Array(a) => a,
        otro => vec![otro],
    };
    for m in mensajes {
        if let Some(pv) = m["result"]["protocolVersion"].as_str() {
            *version.lock().unwrap() = Some(pv.to_string());
        }
        escribir(salida, &m).await;
    }
}

// ── HTTP streamable ────────────────────────────────────────────────────────────────────────────

async fn puente_http(servidor: String, url: String, headers: std::collections::BTreeMap<String, String>, mut entrada: Entrada, salida: Salida) {
    let http = cliente();
    let sesion: Arc<std::sync::Mutex<Option<String>>> = Arc::default();
    let version: Arc<std::sync::Mutex<Option<String>>> = Arc::default();
    let headers: Cabeceras = Arc::new(std::sync::Mutex::new(headers));
    let servidor = Arc::new(servidor);
    while let Ok(Some(linea)) = entrada.next_line().await {
        let (http, url, headers, sesion, version, salida, servidor) = (http.clone(), url.clone(), headers.clone(), sesion.clone(), version.clone(), salida.clone(), servidor.clone());
        // cada mensaje por su lado: una llamada larga no frena un `ping` ni otra llamada
        tokio::spawn(async move { enviar_http(&http, &url, &servidor, &headers, &sesion, &version, &linea, &salida).await });
    }
    // GOTV cerró: se libera la sesión en el servidor (si la había)
    let sid = sesion.lock().unwrap().clone();
    if let Some(s) = sid {
        let _ = http.delete(&url).header("Mcp-Session-Id", s).timeout(Duration::from_secs(3)).send().await;
    }
}

async fn enviar_http(http: &reqwest::Client, url: &str, servidor: &str, headers: &Cabeceras, sesion: &std::sync::Mutex<Option<String>>, version: &Arc<std::sync::Mutex<Option<String>>>, linea: &str, salida: &Salida) {
    let mut renovado = false;
    let mut intento = 0;
    while intento < ESPERAS.len() {
        let espera = &ESPERAS[intento];
        intento += 1;
        let mut rq = http.post(url).header("Content-Type", "application/json").header("Accept", "application/json, text/event-stream");
        for (k, v) in headers.lock().unwrap().iter() {
            rq = rq.header(k, v);
        }
        if let Some(s) = sesion.lock().unwrap().clone() {
            rq = rq.header("Mcp-Session-Id", s);
        }
        if let Some(v) = version.lock().unwrap().clone() {
            rq = rq.header("MCP-Protocol-Version", v);
        }
        let resp = match rq.body(linea.to_string()).send().await {
            Ok(r) => r,
            Err(e) if (e.is_connect() || e.is_timeout()) && intento < ESPERAS.len() => {
                tokio::time::sleep(Duration::from_secs(*espera)).await;
                continue;
            }
            Err(e) => {
                error_para(salida, linea, &crate::tr!("no se pudo hablar con el servidor: {e}", "could not reach the server: {e}")).await;
                return;
            }
        };
        if let Some(s) = resp.headers().get("mcp-session-id").and_then(|v| v.to_str().ok()) {
            *sesion.lock().unwrap() = Some(s.to_string());
        }
        let estado = resp.status();
        if estado.as_u16() == 202 {
            return;
        }
        // F13.4: el token venció a mitad de la sesión: se renueva y se reintenta UNA vez
        if estado.as_u16() == 401 && !renovado && crate::mcp_oauth::leer(servidor).is_some() {
            renovado = true;
            if let Ok(t) = crate::mcp_oauth::renovar(servidor).await {
                headers.lock().unwrap().insert("Authorization".into(), format!("Bearer {}", t.access_token));
                intento = intento.saturating_sub(1);
                continue;
            }
        }
        if !estado.is_success() {
            let cuerpo: String = resp.text().await.unwrap_or_default().chars().take(300).collect();
            let motivo = if estado.as_u16() == 401 {
                crate::tr!("el servidor pide iniciar sesión (HTTP 401): `gotv mcp entrar` o una cabecera Authorization", "the server requires signing in (HTTP 401): `gotv mcp entrar` or an Authorization header")
            } else {
                format!("HTTP {estado}: {cuerpo}")
            };
            error_para(salida, linea, &motivo).await;
            return;
        }
        let sse = resp.headers().get("content-type").and_then(|v| v.to_str().ok()).is_some_and(|t| t.starts_with("text/event-stream"));
        if sse {
            use futures_util::StreamExt;
            let mut lector = LectorSse::default();
            let mut flujo = resp.bytes_stream();
            while let Some(Ok(b)) = flujo.next().await {
                for (_, datos) in lector.alimentar(&String::from_utf8_lossy(&b)) {
                    entregar(salida, &datos, version).await;
                }
            }
        } else {
            let texto = resp.text().await.unwrap_or_default();
            if !texto.trim().is_empty() {
                entregar(salida, &texto, version).await;
            }
        }
        return;
    }
}

// ── SSE clásico ────────────────────────────────────────────────────────────────────────────────

/// Abre el flujo y espera el evento `endpoint` (10 s). Los mensajes que lleguen después van a GOTV.
async fn abrir_sse(url: &str, headers: &std::collections::BTreeMap<String, String>, salida: Salida) -> Result<String> {
    let mut rq = cliente().get(url).header("Accept", "text/event-stream");
    for (k, v) in headers {
        rq = rq.header(k, v);
    }
    let resp = rq.send().await.map_err(|e| anyhow!("{}", crate::tr!("no se pudo abrir el flujo SSE: {e}", "could not open the SSE stream: {e}")))?;
    if !resp.status().is_success() {
        return Err(anyhow!("SSE: HTTP {}", resp.status()));
    }
    let base = resp.url().clone();
    let (tx, rx) = tokio::sync::oneshot::channel::<String>();
    tokio::spawn(async move {
        use futures_util::StreamExt;
        let mut tx = Some(tx);
        let mut lector = LectorSse::default();
        let version = Arc::default();
        let mut flujo = resp.bytes_stream();
        while let Some(Ok(b)) = flujo.next().await {
            for (evento, datos) in lector.alimentar(&String::from_utf8_lossy(&b)) {
                if evento == "endpoint" {
                    if let Some(t) = tx.take() {
                        let _ = t.send(base.join(datos.trim()).map(|u| u.to_string()).unwrap_or(datos));
                    }
                } else {
                    entregar(&salida, &datos, &version).await;
                }
            }
        }
        // el flujo se cerró: GOTV lo ve como canal caído (sus peticiones fallan al momento)
        let _ = salida.lock().await.shutdown().await;
    });
    tokio::time::timeout(Duration::from_secs(10), rx)
        .await
        .map_err(|_| anyhow!("{}", crate::i18n::t("el servidor SSE no dijo a dónde mandar los mensajes (evento `endpoint`) en 10 s", "the SSE server did not say where to send messages (`endpoint` event) within 10 s")))?
        .map_err(|_| anyhow!("{}", crate::i18n::t("el flujo SSE se cerró antes del evento `endpoint`", "the SSE stream closed before the `endpoint` event")))
}

async fn puente_sse_envios(endpoint: String, headers: std::collections::BTreeMap<String, String>, mut entrada: Entrada, salida: Salida) {
    let http = cliente();
    while let Ok(Some(linea)) = entrada.next_line().await {
        let mut enviado = false;
        for (intento, espera) in ESPERAS.iter().enumerate() {
            let mut rq = http.post(&endpoint).header("Content-Type", "application/json");
            for (k, v) in &headers {
                rq = rq.header(k, v);
            }
            match rq.body(linea.clone()).send().await {
                Ok(r) if r.status().is_success() => {
                    enviado = true;
                    break;
                }
                Ok(r) => {
                    error_para(&salida, &linea, &format!("HTTP {}", r.status())).await;
                    enviado = true;
                    break;
                }
                Err(e) if (e.is_connect() || e.is_timeout()) && intento + 1 < ESPERAS.len() => tokio::time::sleep(Duration::from_secs(*espera)).await,
                Err(e) => {
                    error_para(&salida, &linea, &e.to_string()).await;
                    enviado = true;
                    break;
                }
            }
        }
        if !enviado {
            error_para(&salida, &linea, crate::i18n::t("no se pudo enviar", "could not send")).await;
        }
    }
}

// ── WebSocket ──────────────────────────────────────────────────────────────────────────────────

type Ws = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn abrir_ws(url: &str, headers: &std::collections::BTreeMap<String, String>) -> Result<Ws> {
    // primero con el subprotocolo `mcp`; un servidor que no lo devuelve se reintenta sin él
    match abrir_ws_con(url, headers, true).await {
        Err(e) if e.to_string().contains("SubProtocol") => abrir_ws_con(url, headers, false).await,
        otro => otro,
    }
}

async fn abrir_ws_con(url: &str, headers: &std::collections::BTreeMap<String, String>, subprotocolo: bool) -> Result<Ws> {
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    let mut rq = url.into_client_request()?;
    if subprotocolo {
        rq.headers_mut().insert("Sec-WebSocket-Protocol", "mcp".parse()?);
    }
    for (k, v) in headers {
        rq.headers_mut().insert(reqwest::header::HeaderName::from_bytes(k.as_bytes())?, v.parse()?);
    }
    // TLS con el proveedor `ring` (el del árbol) y las raíces de Mozilla
    let raices = rustls::RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let cfg = rustls::ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider())).with_safe_default_protocol_versions()?.with_root_certificates(raices).with_no_client_auth();
    let (ws, _) = tokio::time::timeout(Duration::from_secs(12), tokio_tungstenite::connect_async_tls_with_config(rq, None, false, Some(tokio_tungstenite::Connector::Rustls(Arc::new(cfg)))))
        .await
        .map_err(|_| anyhow!("{}", crate::i18n::t("el WebSocket no abrió en 12 s", "the WebSocket did not open within 12 s")))??;
    Ok(ws)
}

async fn puente_ws(ws: Ws, mut entrada: Entrada, salida: Salida) {
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message;
    let (mut envio, mut recibo) = ws.split();
    let s2 = salida.clone();
    let lectura = tokio::spawn(async move {
        let version = Arc::default();
        while let Some(Ok(m)) = recibo.next().await {
            match m {
                Message::Text(t) => entregar(&s2, t.as_str(), &version).await,
                Message::Binary(b) => entregar(&s2, &String::from_utf8_lossy(&b), &version).await,
                Message::Close(_) => break,
                _ => {}
            }
        }
        let _ = s2.lock().await.shutdown().await;
    });
    while let Ok(Some(linea)) = entrada.next_line().await {
        if envio.send(Message::Text(linea.clone().into())).await.is_err() {
            error_para(&salida, &linea, crate::i18n::t("el WebSocket se cerró", "the WebSocket closed")).await;
            break;
        }
    }
    let _ = envio.close().await;
    lectura.abort();
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::{HeaderMap, StatusCode};
    use axum::response::{IntoResponse, Response};
    use std::collections::HashMap;

    /// Una sesión del servidor de eco (F0.4) detrás de un transporte falso.
    struct Eco {
        hacia: WriteHalf<tokio::io::DuplexStream>,
        desde: tokio::io::Lines<BufReader<ReadHalf<tokio::io::DuplexStream>>>,
    }
    fn eco_nuevo() -> Eco {
        let (a, b) = tokio::io::duplex(1 << 16);
        let (br, bw) = tokio::io::split(b);
        tokio::spawn(crate::mcp_eco::servir(br, bw));
        let (ar, aw) = tokio::io::split(a);
        Eco { hacia: aw, desde: BufReader::new(ar).lines() }
    }
    /// Manda un mensaje al eco; si es petición, junta lo que llega hasta su respuesta (avisos incluidos).
    async fn al_eco(e: &mut Eco, m: &Value) -> Vec<String> {
        e.hacia.write_all(format!("{m}\n").as_bytes()).await.unwrap();
        let Some(id) = m.get("id") else { return vec![] };
        let mut out = vec![];
        while let Ok(Some(l)) = e.desde.next_line().await {
            let v: Value = serde_json::from_str(&l).unwrap();
            let fin = v.get("id") == Some(id);
            out.push(l);
            if fin {
                break;
            }
        }
        out
    }

    type Estado = Arc<Mutex<HashMap<String, Eco>>>;

    /// HTTP streamable falso: exige `Authorization: Bearer t0k`, sesión por cabecera, y `tools/call` contesta
    /// en flujo SSE (con el aviso del eco delante).
    async fn http_falso(axum::extract::State(st): axum::extract::State<Estado>, h: HeaderMap, cuerpo: String) -> Response {
        if h.get("authorization").and_then(|v| v.to_str().ok()) != Some("Bearer t0k") {
            return (StatusCode::UNAUTHORIZED, "sin token").into_response();
        }
        let m: Value = serde_json::from_str(&cuerpo).unwrap();
        let mut sesiones = st.lock().await;
        let sid = if m["method"] == "initialize" {
            let sid = format!("s{}", sesiones.len() + 1);
            sesiones.insert(sid.clone(), eco_nuevo());
            sid
        } else {
            match h.get("mcp-session-id").and_then(|v| v.to_str().ok()) {
                Some(s) if sesiones.contains_key(s) => s.to_string(),
                _ => return (StatusCode::BAD_REQUEST, "sin sesión").into_response(),
            }
        };
        if m["method"] != "initialize" && h.get("mcp-protocol-version").is_none() {
            return (StatusCode::BAD_REQUEST, "falta MCP-Protocol-Version").into_response();
        }
        let lineas = al_eco(sesiones.get_mut(&sid).unwrap(), &m).await;
        if lineas.is_empty() {
            return StatusCode::ACCEPTED.into_response();
        }
        if m["method"] == "tools/call" {
            let cuerpo: String = lineas.iter().map(|l| format!("event: message\ndata: {l}\n\n")).collect();
            return ([("content-type", "text/event-stream"), ("mcp-session-id", sid.as_str())], cuerpo).into_response();
        }
        ([("content-type", "application/json"), ("mcp-session-id", sid.as_str())], lineas.last().unwrap().clone()).into_response()
    }

    async fn sse_abrir(axum::extract::State(st): axum::extract::State<(Estado, Arc<Mutex<HashMap<String, tokio::sync::mpsc::UnboundedSender<String>>>>)>) -> Response {
        let (sesiones, canales) = st;
        let sid = format!("c{}", sesiones.lock().await.len() + 1);
        sesiones.lock().await.insert(sid.clone(), eco_nuevo());
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<String>();
        tx.send(format!("event: endpoint\ndata: /mensajes?s={sid}\n\n")).unwrap();
        canales.lock().await.insert(sid, tx);
        let flujo = futures_util::stream::unfold(rx, |mut rx| async move { rx.recv().await.map(|t| (Ok::<_, std::convert::Infallible>(bytes::Bytes::from(t)), rx)) });
        ([("content-type", "text/event-stream")], axum::body::Body::from_stream(flujo)).into_response()
    }

    async fn sse_mensaje(axum::extract::State(st): axum::extract::State<(Estado, Arc<Mutex<HashMap<String, tokio::sync::mpsc::UnboundedSender<String>>>>)>, axum::extract::Query(q): axum::extract::Query<HashMap<String, String>>, cuerpo: String) -> Response {
        let (sesiones, canales) = st;
        let sid = q.get("s").cloned().unwrap_or_default();
        let m: Value = serde_json::from_str(&cuerpo).unwrap();
        let lineas = al_eco(sesiones.lock().await.get_mut(&sid).unwrap(), &m).await;
        let tx = canales.lock().await.get(&sid).cloned().unwrap();
        for l in lineas {
            let _ = tx.send(format!("event: message\ndata: {l}\n\n"));
        }
        StatusCode::ACCEPTED.into_response()
    }

    async fn servir_en(app: axum::Router) -> u16 {
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let p = l.local_addr().unwrap().port();
        tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
        p
    }

    async fn probar(c: ServerConfig) {
        let d = crate::CarpetaPrueba::nueva("gotv-mcp-red");
        let (mut mcp, notas) = crate::mcp::Mcp::start(&[c.clone()], &d).await;
        assert!(notas.iter().any(|n| n.contains('4')), "{:?} {notas:?}", c.tipo);
        assert_eq!(mcp.call("mcp__eco_srv__eco", &json!({"texto": "hola"})).await.unwrap(), "eco: hola");
        assert!(mcp.call("mcp__eco_srv__falla", &json!({})).await.is_err());
        // varias llamadas seguidas por la misma sesión
        assert_eq!(mcp.call("mcp__eco_srv__eco", &json!({"texto": "otra"})).await.unwrap(), "eco: otra");
    }

    /// F13.3 de punta a punta: los tres transportes remotos contra servidores falsos con el eco detrás.
    #[tokio::test]
    async fn los_tres_transportes_remotos() {
        // HTTP streamable, con la cabecera de la configuración (ya expandida)
        let p = servir_en(axum::Router::new().route("/mcp", axum::routing::post(http_falso)).with_state(Estado::default())).await;
        let mut c = ServerConfig { name: "eco srv".into(), tipo: Tipo::Http, url: Some(format!("http://127.0.0.1:{p}/mcp")), ..Default::default() };
        c.headers.insert("Authorization".into(), "Bearer t0k".into());
        probar(c.clone()).await;
        // sin la cabecera: el 401 llega como error claro, sin colgarse
        c.headers.clear();
        let d = crate::CarpetaPrueba::nueva("gotv-mcp-red-401");
        let t = std::time::Instant::now();
        let (_, notas) = crate::mcp::Mcp::start(&[c], &d).await;
        assert!(notas.iter().any(|n| n.contains("401")), "{notas:?}");
        assert!(t.elapsed() < Duration::from_secs(5));

        // SSE clásico
        let st: (Estado, Arc<Mutex<HashMap<String, tokio::sync::mpsc::UnboundedSender<String>>>>) = Default::default();
        let p = servir_en(axum::Router::new().route("/sse", axum::routing::get(sse_abrir)).route("/mensajes", axum::routing::post(sse_mensaje)).with_state(st)).await;
        probar(ServerConfig { name: "eco srv".into(), tipo: Tipo::Sse, url: Some(format!("http://127.0.0.1:{p}/sse")), ..Default::default() }).await;

        // WebSocket
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let p = l.local_addr().unwrap().port();
        tokio::spawn(async move {
            while let Ok((s, _)) = l.accept().await {
                tokio::spawn(async move {
                    use futures_util::{SinkExt, StreamExt};
                    let ws = tokio_tungstenite::accept_async(s).await.unwrap();
                    let (mut envio, mut recibo) = ws.split();
                    let mut eco = eco_nuevo();
                    while let Some(Ok(tokio_tungstenite::tungstenite::Message::Text(t))) = recibo.next().await {
                        let m: Value = serde_json::from_str(t.as_str()).unwrap();
                        for l in al_eco(&mut eco, &m).await {
                            envio.send(tokio_tungstenite::tungstenite::Message::Text(l.into())).await.unwrap();
                        }
                    }
                });
            }
        });
        probar(ServerConfig { name: "eco srv".into(), tipo: Tipo::Ws, url: Some(format!("ws://127.0.0.1:{p}/")), ..Default::default() }).await;
    }

    /// Un servidor que no existe: error al momento (reintentos de conexión aparte), no un cuelgue largo.
    #[tokio::test]
    async fn sin_servidor_falla_claro() {
        let d = crate::CarpetaPrueba::nueva("gotv-mcp-red-nada");
        let c = ServerConfig { name: "nada".into(), tipo: Tipo::Sse, url: Some("http://127.0.0.1:1/sse".into()), ..Default::default() };
        let (m, notas) = crate::mcp::Mcp::start(&[c], &d).await;
        assert!(m.is_empty() && notas.iter().any(|n| n.contains("SSE")), "{notas:?}");
    }

    #[test]
    fn el_lector_sse_arma_los_eventos() {
        let mut l = LectorSse::default();
        assert!(l.alimentar("event: endpoint\r\ndata: /mens").is_empty());
        let e = l.alimentar("ajes?s=1\r\n\r\n: comentario\ndata: {\"a\":\ndata: 1}\n\n");
        assert_eq!(e, vec![("endpoint".to_string(), "/mensajes?s=1".to_string()), ("message".to_string(), "{\"a\":\n1}".to_string())]);
    }
}
