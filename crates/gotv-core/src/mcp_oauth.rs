//! F13.4 · OAuth para servidores MCP remotos, como manda la especificación de autorización de MCP:
//! 1. **Descubrimiento**: el servidor contesta 401 con `WWW-Authenticate: … resource_metadata="…"` (o se
//!    prueba `/.well-known/oauth-protected-resource`, RFC 9728) → su servidor de autorización → los
//!    metadatos de éste (`/.well-known/oauth-authorization-server`, RFC 8414; o el de OpenID).
//! 2. **Registro dinámico** del cliente (RFC 7591) si el servidor lo ofrece; si no, hace falta un
//!    `client_id` en la configuración.
//! 3. **PKCE S256** + `state` + `resource` (RFC 8707): el navegador del usuario abre la página de entrada; la
//!    vuelta llega a `http://127.0.0.1:<puerto>/callback`, servido por GOTV sólo el tiempo de la entrada.
//! 4. Los tokens van al **Llavero** (`mcp/<servidor>`), nunca a un archivo ni a un registro; se renuevan
//!    solos un minuto antes de vencer, y una vez más si el servidor contesta 401 a mitad de una sesión.
//! Sin el crate `oauth2`: `reqwest`, `sha2` y `base64`, que ya están, y `/dev/urandom` para el azar.
use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Tokens {
    pub access_token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    /// segundos UNIX en que vence (0 = no se sabe)
    #[serde(default)]
    pub vence: u64,
    pub token_endpoint: String,
    pub client_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_secret: Option<String>,
    /// el recurso (la URL del servidor MCP), para pedir tokens SÓLO para él
    pub recurso: String,
}

#[derive(Debug, Clone, Default)]
pub struct Metadatos {
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    pub registration_endpoint: Option<String>,
    pub scopes: Vec<String>,
}

fn ahora() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn http() -> reqwest::Client {
    reqwest::Client::builder().connect_timeout(Duration::from_secs(10)).timeout(Duration::from_secs(30)).redirect(reqwest::redirect::Policy::none()).build().unwrap_or_default()
}

fn azar(n: usize) -> Result<Vec<u8>> {
    use std::io::Read;
    let mut b = vec![0u8; n];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut b)?;
    Ok(b)
}

fn b64url(b: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b)
}

/// PKCE: (verificador, desafío S256).
pub fn pkce() -> Result<(String, String)> {
    use sha2::{Digest, Sha256};
    let v = b64url(&azar(48)?);
    let c = b64url(&Sha256::digest(v.as_bytes()));
    Ok((v, c))
}

// ── almacén de tokens: el Llavero (en pruebas, memoria) ─────────────────────────────────────────

#[cfg(test)]
pub(crate) static PRUEBA: std::sync::Mutex<Option<std::collections::HashMap<String, String>>> = std::sync::Mutex::new(None);

fn cuenta(servidor: &str) -> String {
    format!("mcp/{servidor}")
}

pub fn leer(servidor: &str) -> Option<Tokens> {
    #[cfg(test)]
    if let Some(m) = PRUEBA.lock().unwrap().as_ref() {
        return m.get(&cuenta(servidor)).and_then(|t| serde_json::from_str(t).ok());
    }
    crate::llaves::get(&cuenta(servidor)).and_then(|t| serde_json::from_str(&t).ok())
}

fn guardar(servidor: &str, t: &Tokens) -> Result<()> {
    let s = serde_json::to_string(t)?;
    #[cfg(test)]
    if let Some(m) = PRUEBA.lock().unwrap().as_mut() {
        m.insert(cuenta(servidor), s);
        return Ok(());
    }
    crate::llaves::set(&cuenta(servidor), &s)
}

pub fn olvidar(servidor: &str) {
    #[cfg(test)]
    if let Some(m) = PRUEBA.lock().unwrap().as_mut() {
        m.remove(&cuenta(servidor));
        return;
    }
    crate::llaves::delete(&cuenta(servidor));
}

// ── descubrimiento ─────────────────────────────────────────────────────────────────────────────

/// `resource_metadata="…"` de una cabecera `WWW-Authenticate`.
fn metadatos_en_cabecera(h: &str) -> Option<String> {
    let i = h.find("resource_metadata=")?;
    let resto = &h[i + "resource_metadata=".len()..];
    let resto = resto.trim_start_matches('"');
    Some(resto.split(['"', ',']).next()?.trim().to_string())
}

fn origen(url: &str) -> Result<reqwest::Url> {
    let mut u = reqwest::Url::parse(url)?;
    u.set_path("");
    u.set_query(None);
    Ok(u)
}

pub async fn descubrir(url_mcp: &str) -> Result<Metadatos> {
    let c = http();
    // 1. ¿dónde están los metadatos del recurso? (la cabecera del 401, o la ruta conocida)
    let r = c.post(url_mcp).header("Content-Type", "application/json").header("Accept", "application/json, text/event-stream").body(r#"{"jsonrpc":"2.0","id":0,"method":"ping"}"#).send().await?;
    let prm_url = r
        .headers()
        .get("www-authenticate")
        .and_then(|h| h.to_str().ok())
        .and_then(metadatos_en_cabecera)
        .map(Ok)
        .unwrap_or_else(|| origen(url_mcp).and_then(|o| Ok(o.join("/.well-known/oauth-protected-resource")?.to_string())))?;
    let prm: Value = c.get(&prm_url).send().await?.error_for_status().context("oauth-protected-resource")?.json().await?;
    let servidor_auth = prm["authorization_servers"][0].as_str().ok_or_else(|| anyhow!("{}", crate::i18n::t("el servidor no dice cuál es su servidor de autorización", "the server does not name its authorization server")))?.to_string();
    // 2. los metadatos del servidor de autorización (RFC 8414, o OpenID)
    let base = reqwest::Url::parse(&servidor_auth)?;
    let ruta = base.path().trim_end_matches('/').to_string();
    let mut candidatos = vec![format!("/.well-known/oauth-authorization-server{ruta}"), format!("/.well-known/openid-configuration{ruta}")];
    if !ruta.is_empty() {
        candidatos.push(format!("{ruta}/.well-known/openid-configuration"));
    }
    for cand in candidatos {
        let u = origen(&servidor_auth)?.join(&cand)?;
        if let Ok(r) = c.get(u).send().await {
            if r.status().is_success() {
                let m: Value = r.json().await?;
                return Ok(Metadatos {
                    authorization_endpoint: m["authorization_endpoint"].as_str().unwrap_or_default().to_string(),
                    token_endpoint: m["token_endpoint"].as_str().unwrap_or_default().to_string(),
                    registration_endpoint: m["registration_endpoint"].as_str().map(String::from),
                    scopes: prm["scopes_supported"].as_array().or(m["scopes_supported"].as_array()).into_iter().flatten().filter_map(|s| s.as_str().map(String::from)).collect(),
                });
            }
        }
    }
    bail!("{}", crate::i18n::t("no hallé los metadatos del servidor de autorización", "could not find the authorization server metadata"))
}

/// Registro dinámico del cliente (RFC 7591): devuelve (client_id, client_secret).
pub async fn registrar(registro: &str, redirect: &str) -> Result<(String, Option<String>)> {
    let cuerpo = serde_json::json!({
        "client_name": "GOD OF THE VOID",
        "redirect_uris": [redirect],
        "grant_types": ["authorization_code", "refresh_token"],
        "response_types": ["code"],
        "token_endpoint_auth_method": "none"
    });
    let r: Value = http().post(registro).json(&cuerpo).send().await?.error_for_status().context(crate::i18n::t("registro del cliente", "client registration"))?.json().await?;
    let id = r["client_id"].as_str().ok_or_else(|| anyhow!("{}", crate::i18n::t("el registro no devolvió client_id", "registration returned no client_id")))?.to_string();
    Ok((id, r["client_secret"].as_str().map(String::from)))
}

fn de_respuesta(r: &Value, previo: Option<&Tokens>, base: Tokens) -> Result<Tokens> {
    let access = r["access_token"].as_str().ok_or_else(|| anyhow!("{}", crate::tr!("el servidor no dio token: {}", "the server gave no token: {}", r.to_string().chars().take(200).collect::<String>())))?;
    Ok(Tokens {
        access_token: access.to_string(),
        // un servidor que no rota el de renovación deja vigente el anterior
        refresh_token: r["refresh_token"].as_str().map(String::from).or_else(|| previo.and_then(|p| p.refresh_token.clone())),
        vence: r["expires_in"].as_u64().map(|s| ahora() + s).unwrap_or(0),
        ..base
    })
}

/// La entrada completa: descubre, registra si hace falta, abre la página con `abrir` (el navegador) y
/// espera la vuelta (5 min). Guarda los tokens en el Llavero.
pub async fn entrar(servidor: &str, url_mcp: &str, client_id: Option<&str>, abrir: impl FnOnce(&str) -> Result<()>) -> Result<()> {
    let m = descubrir(url_mcp).await?;
    // la vuelta: un puerto libre de 127.0.0.1, sólo mientras dura la entrada
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let redirect = format!("http://127.0.0.1:{}/callback", l.local_addr()?.port());
    let (client_id, secreto) = match client_id {
        Some(c) => (c.to_string(), None),
        None => match &m.registration_endpoint {
            Some(r) => registrar(r, &redirect).await?,
            None => bail!("{}", crate::i18n::t("el servidor no permite registrar clientes: pon su `client_id` en la configuración del servidor MCP", "the server does not allow client registration: put its `client_id` in the MCP server configuration")),
        },
    };
    let (verificador, desafio) = pkce()?;
    let estado = b64url(&azar(16)?);
    let mut u = reqwest::Url::parse(&m.authorization_endpoint)?;
    {
        let mut q = u.query_pairs_mut();
        q.append_pair("response_type", "code").append_pair("client_id", &client_id).append_pair("redirect_uri", &redirect).append_pair("code_challenge", &desafio).append_pair("code_challenge_method", "S256").append_pair("state", &estado).append_pair("resource", url_mcp);
        if !m.scopes.is_empty() {
            q.append_pair("scope", &m.scopes.join(" "));
        }
    }
    let (tx, rx) = tokio::sync::oneshot::channel::<std::collections::HashMap<String, String>>();
    let tx = std::sync::Arc::new(std::sync::Mutex::new(Some(tx)));
    let app = axum::Router::new().route(
        "/callback",
        axum::routing::get(move |axum::extract::Query(q): axum::extract::Query<std::collections::HashMap<String, String>>| {
            let tx = tx.clone();
            async move {
                if let Some(t) = tx.lock().unwrap().take() {
                    let _ = t.send(q);
                }
                axum::response::Html(crate::i18n::t("<p>GOD OF THE VOID: listo. Ya puedes cerrar esta pestaña.</p>", "<p>GOD OF THE VOID: done. You can close this tab.</p>"))
            }
        }),
    );
    let servidor_vuelta = tokio::spawn(async move {
        let _ = axum::serve(l, app).await;
    });
    abrir(u.as_str())?;
    let q = tokio::time::timeout(Duration::from_secs(300), rx).await.map_err(|_| anyhow!("{}", crate::i18n::t("la entrada no volvió en 5 minutos", "sign-in did not come back within 5 minutes")));
    servidor_vuelta.abort();
    let q = q?.map_err(|_| anyhow!("{}", crate::i18n::t("la vuelta se cerró", "the callback closed")))?;
    if q.get("state") != Some(&estado) {
        bail!("{}", crate::i18n::t("la vuelta no trae el `state` de esta entrada: se descarta", "the callback does not carry this sign-in's `state`: discarded"));
    }
    if let Some(e) = q.get("error") {
        bail!("{}", crate::tr!("el servidor negó la entrada: {e}", "the server denied sign-in: {e}"));
    }
    let code = q.get("code").ok_or_else(|| anyhow!("{}", crate::i18n::t("la vuelta no trae `code`", "the callback carries no `code`")))?;
    let mut form = vec![("grant_type", "authorization_code"), ("code", code.as_str()), ("redirect_uri", redirect.as_str()), ("client_id", client_id.as_str()), ("code_verifier", verificador.as_str()), ("resource", url_mcp)];
    if let Some(s) = &secreto {
        form.push(("client_secret", s.as_str()));
    }
    let r: Value = http().post(&m.token_endpoint).form(&form).send().await?.json().await?;
    let t = de_respuesta(&r, None, Tokens { token_endpoint: m.token_endpoint.clone(), client_id: client_id.clone(), client_secret: secreto.clone(), recurso: url_mcp.to_string(), ..Default::default() })?;
    guardar(servidor, &t)
}

/// Renueva con el token de renovación; guarda y devuelve los nuevos.
pub async fn renovar(servidor: &str) -> Result<Tokens> {
    let t = leer(servidor).ok_or_else(|| anyhow!("{}", crate::tr!("«{servidor}» no tiene sesión iniciada: `gotv mcp entrar {servidor}`", "«{servidor}» is not signed in: `gotv mcp entrar {servidor}`")))?;
    let rt = t.refresh_token.clone().ok_or_else(|| anyhow!("{}", crate::tr!("la sesión de «{servidor}» venció y no se puede renovar: `gotv mcp entrar {servidor}`", "«{servidor}»'s session expired and cannot be renewed: `gotv mcp entrar {servidor}`")))?;
    let mut form = vec![("grant_type", "refresh_token"), ("refresh_token", rt.as_str()), ("client_id", t.client_id.as_str()), ("resource", t.recurso.as_str())];
    if let Some(s) = &t.client_secret {
        form.push(("client_secret", s.as_str()));
    }
    let r: Value = http().post(&t.token_endpoint).form(&form).send().await?.json().await?;
    let nuevo = de_respuesta(&r, Some(&t), t.clone())?;
    guardar(servidor, &nuevo)?;
    Ok(nuevo)
}

/// El token de acceso vigente (renovado si vence en menos de un minuto), o `None` si no hay sesión.
pub async fn vigente(servidor: &str) -> Option<String> {
    let t = leer(servidor)?;
    if t.vence != 0 && t.vence < ahora() + 60 {
        return renovar(servidor).await.ok().map(|t| t.access_token);
    }
    Some(t.access_token)
}

#[cfg(test)]
mod tests {
    use super::*;

    use axum::http::{HeaderMap, StatusCode};
    use axum::response::{IntoResponse, Response};
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    struct Proveedor {
        puerto: u16,
        desafio: String,
        validos: Vec<String>,
        canjes: u32,
    }
    type P = Arc<Mutex<Proveedor>>;

    async fn mcp(axum::extract::State(p): axum::extract::State<P>, h: HeaderMap, cuerpo: String) -> Response {
        let token = h.get("authorization").and_then(|v| v.to_str().ok()).and_then(|v| v.strip_prefix("Bearer ")).unwrap_or("").to_string();
        let puerto = p.lock().unwrap().puerto;
        if !p.lock().unwrap().validos.contains(&token) {
            let www = format!(r#"Bearer error="invalid_token", resource_metadata="http://127.0.0.1:{puerto}/.well-known/oauth-protected-resource""#);
            return (StatusCode::UNAUTHORIZED, [("www-authenticate", www)], "").into_response();
        }
        let m: Value = serde_json::from_str(&cuerpo).unwrap();
        let Some(id) = m.get("id").cloned() else { return StatusCode::ACCEPTED.into_response() };
        let r = match m["method"].as_str().unwrap_or("") {
            "initialize" => serde_json::json!({"protocolVersion": "2025-06-18", "capabilities": {"tools": {}}, "serverInfo": {"name": "seguro", "version": "1"}}),
            "tools/list" => serde_json::json!({"tools": [{"name": "quien", "description": "qué token llegó", "inputSchema": {"type": "object"}}]}), // juez-idioma: datos (servidor falso de la prueba)
            _ => serde_json::json!({"content": [{"type": "text", "text": format!("token: {token}")}]}),
        };
        axum::Json(serde_json::json!({"jsonrpc": "2.0", "id": id, "result": r})).into_response()
    }

    async fn autorizar(axum::extract::State(p): axum::extract::State<P>, axum::extract::Query(q): axum::extract::Query<HashMap<String, String>>) -> Response {
        assert_eq!(q["code_challenge_method"], "S256");
        assert_eq!(q["client_id"], "c1");
        assert!(q["resource"].ends_with("/mcp"), "RFC 8707");
        p.lock().unwrap().desafio = q["code_challenge"].clone();
        let destino = format!("{}?code=k1&state={}", q["redirect_uri"], q["state"]);
        (StatusCode::FOUND, [("location", destino)]).into_response()
    }

    async fn token(axum::extract::State(p): axum::extract::State<P>, cuerpo: String) -> Response {
        let f: HashMap<String, String> = reqwest::Url::parse(&format!("http://x/?{cuerpo}")).unwrap().query_pairs().into_owned().collect();
        use sha2::{Digest, Sha256};
        let mut p = p.lock().unwrap();
        p.canjes += 1;
        match f["grant_type"].as_str() {
            "authorization_code" => {
                assert_eq!(f["code"], "k1");
                assert_eq!(b64url(&Sha256::digest(f["code_verifier"].as_bytes())), p.desafio, "PKCE verificado");
                p.validos = vec!["tok1".into()];
                axum::Json(serde_json::json!({"access_token": "tok1", "refresh_token": "r1", "expires_in": 3600, "token_type": "Bearer"})).into_response()
            }
            "refresh_token" if f["refresh_token"] == "r1" => {
                p.validos = vec!["tok2".into()];
                axum::Json(serde_json::json!({"access_token": "tok2", "expires_in": 3600, "token_type": "Bearer"})).into_response()
            }
            _ => (StatusCode::BAD_REQUEST, axum::Json(serde_json::json!({"error": "invalid_grant"}))).into_response(),
        }
    }

    /// F13.4 de punta a punta: descubrir, registrar, entrar con PKCE (un «navegador» que sigue la
    /// redirección), usar el token, y renovarlo solo cuando el servidor lo deja de aceptar a mitad de sesión.
    #[tokio::test]
    async fn oauth_de_punta_a_punta() {
        *PRUEBA.lock().unwrap() = Some(HashMap::new());
        let p: P = Arc::default();
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let puerto = l.local_addr().unwrap().port();
        p.lock().unwrap().puerto = puerto;
        let base = format!("http://127.0.0.1:{puerto}");
        let (b1, b2) = (base.clone(), base.clone());
        let app = axum::Router::new()
            .route("/mcp", axum::routing::post(mcp))
            .route("/.well-known/oauth-protected-resource", axum::routing::get(move || async move { axum::Json(serde_json::json!({"resource": format!("{b1}/mcp"), "authorization_servers": [format!("{b1}/auth")]})) }))
            .route("/.well-known/oauth-authorization-server/auth", axum::routing::get(move || async move { axum::Json(serde_json::json!({"issuer": format!("{b2}/auth"), "authorization_endpoint": format!("{b2}/auth/authorize"), "token_endpoint": format!("{b2}/auth/token"), "registration_endpoint": format!("{b2}/auth/register"), "code_challenge_methods_supported": ["S256"]})) }))
            .route("/auth/register", axum::routing::post(|| async { axum::Json(serde_json::json!({"client_id": "c1"})) }))
            .route("/auth/authorize", axum::routing::get(autorizar))
            .route("/auth/token", axum::routing::post(token))
            .with_state(p.clone());
        tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
        let url = format!("{base}/mcp");

        // sin sesión: el servidor dice 401 con cómo entrar
        let d = crate::CarpetaPrueba::nueva("gotv-mcp-oauth");
        let cfg = crate::mcp::ServerConfig { name: "seguro".into(), tipo: crate::mcp::Tipo::Http, url: Some(url.clone()), ..Default::default() };
        let (_, notas) = crate::mcp::Mcp::start(&[cfg.clone()], &d).await;
        assert!(notas.iter().any(|n| n.contains("401")), "{notas:?}");

        // entrar: el «navegador» sigue la redirección hasta la vuelta de GOTV
        entrar("seguro", &url, None, |u| {
            let u = u.to_string();
            tokio::spawn(async move { reqwest::get(u).await.unwrap() });
            Ok(())
        })
        .await
        .unwrap();
        let t = leer("seguro").unwrap();
        assert_eq!((t.access_token.as_str(), t.refresh_token.as_deref(), t.client_id.as_str()), ("tok1", Some("r1"), "c1"));

        // con sesión: el puente pone el token solo
        let (mut m, notas) = crate::mcp::Mcp::start(&[cfg], &d).await;
        assert!(notas.iter().any(|n| n.contains('1')), "{notas:?}");
        assert_eq!(m.call("mcp__seguro__quien", &serde_json::json!({})).await.unwrap(), "token: tok1");
        // el servidor deja de aceptar tok1 a mitad de sesión: se renueva y se reintenta una vez
        p.lock().unwrap().validos = vec![];
        let canjes = p.lock().unwrap().canjes;
        // (la renovación activa tok2 en el proveedor falso)
        assert_eq!(m.call("mcp__seguro__quien", &serde_json::json!({})).await.unwrap(), "token: tok2");
        assert_eq!(p.lock().unwrap().canjes, canjes + 1);
        assert_eq!(leer("seguro").unwrap().refresh_token.as_deref(), Some("r1"), "el de renovación sigue si el servidor no lo rota");
        olvidar("seguro");
        assert!(leer("seguro").is_none());
        *PRUEBA.lock().unwrap() = None;
    }

    #[test]
    fn pkce_y_cabecera() {
        let (v, c) = pkce().unwrap();
        assert!(v.len() >= 43 && c.len() == 43 && !c.contains('='));
        assert_eq!(metadatos_en_cabecera(r#"Bearer error="invalid_token", resource_metadata="https://x/.well-known/oauth-protected-resource""#).as_deref(), Some("https://x/.well-known/oauth-protected-resource"));
    }
}
