//! F13 · un servidor MCP REMOTO con OAuth, de mentira y local, para el banco (como el de la prueba
//! `mcp_oauth::oauth_de_punta_a_punta`): descubrimiento (401 + RFC 9728/8414), registro dinámico, PKCE S256
//! y token. La página de autorización aprueba sola (redirige a la vuelta de GOTV con el código). Una
//! herramienta, `quien`, devuelve la cuenta del token: el modelo NO la puede inventar.
//!
//! `mcp_oauth_falso <puerto>` · escribe «listo <puerto>» cuando escucha.
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct Estado {
    puerto: u16,
    desafio: String,
    validos: Vec<String>,
}
type E = Arc<Mutex<Estado>>;

const CUENTA: &str = "cuenta-banco-oauth-5521";

async fn mcp(State(e): State<E>, h: HeaderMap, cuerpo: String) -> Response {
    let token = h.get("authorization").and_then(|v| v.to_str().ok()).and_then(|v| v.strip_prefix("Bearer ")).unwrap_or("").to_string();
    let puerto = e.lock().unwrap().puerto;
    if !e.lock().unwrap().validos.contains(&token) {
        let www = format!(r#"Bearer error="invalid_token", resource_metadata="http://127.0.0.1:{puerto}/.well-known/oauth-protected-resource""#);
        return (StatusCode::UNAUTHORIZED, [("www-authenticate", www)], "").into_response();
    }
    let m: Value = serde_json::from_str(&cuerpo).unwrap_or_default();
    let Some(id) = m.get("id").cloned() else { return StatusCode::ACCEPTED.into_response() };
    let r = match m["method"].as_str().unwrap_or("") {
        "initialize" => json!({"protocolVersion": "2025-06-18", "capabilities": {"tools": {}}, "serverInfo": {"name": "seguro", "version": "1"}}),
        "tools/list" => json!({"tools": [{"name": "quien", "description": "Dice con qué cuenta entraste en el servicio seguro.", "inputSchema": {"type": "object", "properties": {}}}]}),
        _ => json!({"content": [{"type": "text", "text": format!("entraste con la cuenta {CUENTA}")}]}),
    };
    axum::Json(json!({"jsonrpc": "2.0", "id": id, "result": r})).into_response()
}

async fn autorizar(State(e): State<E>, Query(q): Query<HashMap<String, String>>) -> Response {
    if q.get("code_challenge_method").map(String::as_str) != Some("S256") {
        return (StatusCode::BAD_REQUEST, "sin PKCE S256").into_response();
    }
    e.lock().unwrap().desafio = q.get("code_challenge").cloned().unwrap_or_default();
    let destino = format!("{}?code=k1&state={}", q.get("redirect_uri").cloned().unwrap_or_default(), q.get("state").cloned().unwrap_or_default());
    (StatusCode::FOUND, [("location", destino)]).into_response()
}

async fn token(State(e): State<E>, cuerpo: String) -> Response {
    let f: HashMap<String, String> = cuerpo
        .split('&')
        .filter_map(|p| p.split_once('='))
        .map(|(k, v)| (k.to_string(), v.replace("%2B", "+").replace("%2F", "/").replace("%3D", "=")))
        .collect();
    let mut e = e.lock().unwrap();
    if f.get("grant_type").map(String::as_str) == Some("authorization_code") && f.get("code").map(String::as_str) == Some("k1") {
        e.validos = vec!["tok-banco".into()];
        return axum::Json(json!({"access_token": "tok-banco", "refresh_token": "r-banco", "expires_in": 3600, "token_type": "Bearer"})).into_response();
    }
    (StatusCode::BAD_REQUEST, axum::Json(json!({"error": "invalid_grant"}))).into_response()
}

#[tokio::main]
async fn main() {
    let puerto: u16 = std::env::args().nth(1).and_then(|p| p.parse().ok()).unwrap_or(0);
    let l = tokio::net::TcpListener::bind(("127.0.0.1", puerto)).await.expect("no pude escuchar");
    let puerto = l.local_addr().unwrap().port();
    let e: E = Arc::new(Mutex::new(Estado { puerto, ..Default::default() }));
    let base = format!("http://127.0.0.1:{puerto}");
    let (b1, b2) = (base.clone(), base.clone());
    let app = axum::Router::new()
        .route("/mcp", axum::routing::post(mcp))
        .route("/.well-known/oauth-protected-resource", axum::routing::get(move || async move { axum::Json(json!({"resource": format!("{b1}/mcp"), "authorization_servers": [format!("{b1}/auth")]})) }))
        .route("/.well-known/oauth-authorization-server/auth", axum::routing::get(move || async move { axum::Json(json!({"issuer": format!("{b2}/auth"), "authorization_endpoint": format!("{b2}/auth/authorize"), "token_endpoint": format!("{b2}/auth/token"), "registration_endpoint": format!("{b2}/auth/register"), "code_challenge_methods_supported": ["S256"]})) }))
        .route("/auth/register", axum::routing::post(|| async { axum::Json(json!({"client_id": "c-banco"})) }))
        .route("/auth/authorize", axum::routing::get(autorizar))
        .route("/auth/token", axum::routing::post(token))
        .with_state(e);
    println!("listo {puerto}");
    use std::io::Write;
    let _ = std::io::stdout().flush();
    axum::serve(l, app).await.unwrap();
}
