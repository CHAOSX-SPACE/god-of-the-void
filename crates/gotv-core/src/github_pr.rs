//! F17 · el lado que ESCRIBE de `github`: crear un PR, comentar, responder una revisión, y leer el estado de
//! CI y los comentarios de un PR. Por la API de GitHub con `reqwest`; el token vive en el Llavero
//! (`github`), se puede importar de `gh auth token` con permiso, y jamás se muestra. GOTV NUNCA fusiona un
//! PR ni activa la fusión automática: no existe esa acción. `research.rs` sigue siendo sólo lectura.
use anyhow::{anyhow, bail, Result};
use serde_json::{json, Value};
use std::path::Path;
use std::time::Duration;

/// Las acciones de este módulo (las demás de `github` son de `research.rs`).
pub fn es_suya(accion: &str) -> bool {
    matches!(accion, "crear_pr" | "comentar" | "responder_revision" | "estado_ci" | "comentarios_pr" | "importar_token")
}

/// ¿Publica algo en GitHub? (pregunta siempre)
pub fn publica(accion: &str) -> bool {
    matches!(accion, "crear_pr" | "comentar" | "responder_revision")
}

pub(crate) fn base() -> String {
    std::env::var("GOTV_GITHUB_API").unwrap_or_else(|_| "https://api.github.com".into()).trim_end_matches('/').to_string()
}

/// El token: el del Llavero; si no, el del entorno (`GITHUB_TOKEN`, `GH_TOKEN`).
pub(crate) fn token() -> Result<String> {
    // en las pruebas, jamás el Llavero real del usuario
    #[cfg(not(test))]
    let del_llavero = crate::llaves::get("github");
    #[cfg(test)]
    let del_llavero: Option<String> = None;
    del_llavero
        .or_else(|| std::env::var("GITHUB_TOKEN").ok())
        .or_else(|| std::env::var("GH_TOKEN").ok())
        .filter(|t| !t.trim().is_empty())
        .ok_or_else(|| anyhow!("{}", crate::i18n::t(
            "no hay token de GitHub: action=importar_token (lo toma de `gh auth token`, con tu permiso) o guárdalo en Configuración → Llavero",
            "there is no GitHub token: action=importar_token (takes it from `gh auth token`, with your permission) or save it in Settings → Keychain"
        )))
}

async fn pedir(metodo: reqwest::Method, ruta: &str, cuerpo: Option<Value>) -> Result<Value> {
    let c = reqwest::Client::builder().user_agent("gotv").timeout(Duration::from_secs(30)).build()?;
    let mut r = c.request(metodo, format!("{}/{}", base(), ruta.trim_start_matches('/'))).bearer_auth(token()?).header("Accept", "application/vnd.github+json").header("X-GitHub-Api-Version", "2022-11-28");
    if let Some(b) = cuerpo {
        r = r.json(&b);
    }
    let resp = r.send().await?;
    let estado = resp.status();
    let texto = resp.text().await?;
    if !estado.is_success() {
        let m = serde_json::from_str::<Value>(&texto).ok().and_then(|v| v["message"].as_str().map(String::from)).unwrap_or_else(|| texto.chars().take(300).collect());
        bail!("GitHub HTTP {estado}: {m}");
    }
    Ok(serde_json::from_str(&texto).unwrap_or(Value::Null))
}

fn requiere(args: &Value, k: &str) -> Result<String> {
    args[k].as_str().map(str::trim).filter(|s| !s.is_empty()).map(String::from).ok_or_else(|| anyhow!("{}", crate::tr!("falta '{k}'", "missing '{k}'")))
}

fn numero(args: &Value) -> Result<u64> {
    args["numero"].as_u64().or_else(|| args["numero"].as_str().and_then(|s| s.trim_start_matches('#').parse().ok())).ok_or_else(|| anyhow!("{}", crate::i18n::t("falta 'numero' (el del PR)", "missing 'numero' (the PR's)")))
}

/// `dueño/nombre` del remoto `origin` del proyecto (si es de GitHub).
pub(crate) fn repo_del_proyecto(root: &Path) -> Option<String> {
    let repo = gix::discover(root).ok()?;
    let remoto = repo.find_remote("origin").ok()?;
    let url = remoto.url(gix::remote::Direction::Push)?.to_bstring().to_string();
    let resto = url.split("github.com").nth(1)?.trim_start_matches([':', '/']);
    let resto = resto.trim_end_matches('/').trim_end_matches(".git");
    let mut p = resto.split('/');
    let (d, n) = (p.next()?, p.next()?);
    (!d.is_empty() && !n.is_empty()).then(|| format!("{d}/{n}"))
}

fn repo_de(args: &Value, root: &Path) -> Result<String> {
    if let Some(r) = args["repo"].as_str().map(str::trim).filter(|r| !r.is_empty()) {
        return Ok(r.to_string());
    }
    repo_del_proyecto(root).ok_or_else(|| anyhow!("{}", crate::i18n::t("falta 'repo' (dueño/nombre) y el remoto origin no es de GitHub", "missing 'repo' (owner/name) and the origin remote is not on GitHub")))
}

pub async fn ejecutar(args: &Value, root: &Path) -> Result<String> {
    let accion = args["action"].as_str().unwrap_or("");
    match accion {
        "importar_token" => {
            let o = crate::entorno::comando_tokio("gh").args(["auth", "token"]).stdin(std::process::Stdio::null()).output().await.map_err(|_| anyhow!("{}", crate::i18n::t("`gh` no está instalado", "`gh` is not installed")))?;
            let t = String::from_utf8_lossy(&o.stdout).trim().to_string();
            if !o.status.success() || t.is_empty() {
                bail!("{}", crate::i18n::t("`gh auth token` no dio un token (¿`gh auth login`?)", "`gh auth token` gave no token (`gh auth login`?)"));
            }
            crate::llaves::set("github", &t)?;
            Ok(crate::i18n::t("token de GitHub guardado en el Llavero (no se muestra)", "GitHub token saved in the Keychain (not shown)").into())
        }
        "crear_pr" => {
            let repo = repo_de(args, root)?;
            let cabeza = match args["cabeza"].as_str().filter(|s| !s.is_empty()) {
                Some(c) => c.to_string(),
                None => gix::discover(root).ok().and_then(|r| r.head_name().ok().flatten()).map(|n| n.shorten().to_string()).ok_or_else(|| anyhow!("{}", crate::i18n::t("falta 'cabeza' (la rama del PR)", "missing 'cabeza' (the PR's branch)")))?,
            };
            let cuerpo = json!({
                "title": requiere(args, "titulo")?,
                "body": args["cuerpo"].as_str().unwrap_or(""),
                "head": cabeza,
                "base": args["base"].as_str().filter(|s| !s.is_empty()).unwrap_or("main"),
                "draft": args["borrador"].as_bool().unwrap_or(false),
            });
            let v = pedir(reqwest::Method::POST, &format!("repos/{repo}/pulls"), Some(cuerpo)).await?;
            Ok(crate::tr!("PR #{} abierto: {}", "PR #{} opened: {}", v["number"], v["html_url"].as_str().unwrap_or("")))
        }
        "comentar" => {
            let repo = repo_de(args, root)?;
            let v = pedir(reqwest::Method::POST, &format!("repos/{repo}/issues/{}/comments", numero(args)?), Some(json!({"body": requiere(args, "cuerpo")?}))).await?;
            Ok(crate::tr!("comentario publicado: {}", "comment posted: {}", v["html_url"].as_str().unwrap_or("")))
        }
        "responder_revision" => {
            let repo = repo_de(args, root)?;
            let id = args["comentario_id"].as_u64().ok_or_else(|| anyhow!("{}", crate::i18n::t("falta 'comentario_id' (de comentarios_pr)", "missing 'comentario_id' (from comentarios_pr)")))?;
            let v = pedir(reqwest::Method::POST, &format!("repos/{repo}/pulls/{}/comments/{id}/replies", numero(args)?), Some(json!({"body": requiere(args, "cuerpo")?}))).await?;
            Ok(crate::tr!("respuesta publicada: {}", "reply posted: {}", v["html_url"].as_str().unwrap_or("")))
        }
        "comentarios_pr" => {
            let repo = repo_de(args, root)?;
            let n = numero(args)?;
            let mut out = Vec::new();
            for c in pedir(reqwest::Method::GET, &format!("repos/{repo}/issues/{n}/comments?per_page=50"), None).await?.as_array().into_iter().flatten() {
                out.push(format!("· {}: {}", c["user"]["login"].as_str().unwrap_or("?"), c["body"].as_str().unwrap_or("").chars().take(400).collect::<String>()));
            }
            // los de revisión, con su archivo:línea y su id (para responder)
            for c in pedir(reqwest::Method::GET, &format!("repos/{repo}/pulls/{n}/comments?per_page=50"), None).await?.as_array().into_iter().flatten() {
                out.push(format!(
                    "· [{}] {} en {}:{}: {}",
                    c["id"],
                    c["user"]["login"].as_str().unwrap_or("?"),
                    c["path"].as_str().unwrap_or("?"),
                    c["line"].as_u64().or(c["original_line"].as_u64()).unwrap_or(0),
                    c["body"].as_str().unwrap_or("").chars().take(400).collect::<String>()
                ));
            }
            Ok(if out.is_empty() { crate::i18n::t("(sin comentarios)", "(no comments)").into() } else { out.join("\n") })
        }
        "estado_ci" => {
            let repo = repo_de(args, root)?;
            // de un PR, su último commit; si no, la referencia dada
            let referencia = match args["numero"].is_null() {
                false => pedir(reqwest::Method::GET, &format!("repos/{repo}/pulls/{}", numero(args)?), None).await?["head"]["sha"].as_str().unwrap_or("").to_string(),
                true => requiere(args, "ref")?,
            };
            let v = pedir(reqwest::Method::GET, &format!("repos/{repo}/commits/{referencia}/check-runs?per_page=50"), None).await?;
            let filas: Vec<String> = v["check_runs"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|r| format!("{} · {} · {}", r["name"].as_str().unwrap_or("?"), r["status"].as_str().unwrap_or("?"), r["conclusion"].as_str().unwrap_or("—")))
                .collect();
            Ok(if filas.is_empty() { crate::tr!("(sin comprobaciones en {referencia})", "(no checks on {referencia})") } else { filas.join("\n") })
        }
        otra => bail!("{}", crate::tr!("acción de github desconocida: {otra}", "unknown github action: {otra}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    /// Contra un servidor FALSO de la API (axum): crear un PR lleva el token como `Bearer` y el cuerpo con
    /// título, rama, base y borrador; los comentarios de revisión traen archivo:línea e id; CI resume las
    /// comprobaciones; sin token, se dice cómo conseguirlo. GOTV no tiene acción de fusionar.
    #[tokio::test]
    async fn crear_pr_y_leer_contra_un_servidor_falso() {
        use axum::{routing::{get, post}, Json, Router};
        let visto: Arc<Mutex<Vec<(String, Value)>>> = Arc::default();
        let v1 = visto.clone();
        let app = Router::new()
            .route("/repos/ana/juguete/pulls", post(move |h: axum::http::HeaderMap, Json(b): Json<Value>| {
                let v1 = v1.clone();
                async move {
                    v1.lock().unwrap().push((h.get("authorization").and_then(|x| x.to_str().ok()).unwrap_or("").to_string(), b));
                    Json(json!({"number": 7, "html_url": "http://falso/ana/juguete/pull/7"}))
                }
            }))
            .route("/repos/ana/juguete/pulls/7/comments", get(|| async { Json(json!([{"id": 99, "user": {"login": "beto"}, "path": "src/a.rs", "line": 12, "body": "¿y si es cero?"}])) }))
            .route("/repos/ana/juguete/issues/7/comments", get(|| async { Json(json!([{"user": {"login": "beto"}, "body": "bien"}])) }))
            .route("/repos/ana/juguete/pulls/7", get(|| async { Json(json!({"head": {"sha": "abc123"}})) }))
            .route("/repos/ana/juguete/commits/abc123/check-runs", get(|| async { Json(json!({"check_runs": [{"name": "pruebas", "status": "completed", "conclusion": "failure"}]})) }));
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let dir = l.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
        std::env::set_var("GOTV_GITHUB_API", format!("http://{dir}"));
        std::env::set_var("GITHUB_TOKEN", "tok-de-prueba");
        let d = crate::CarpetaPrueba::nueva("gotv-gh-pr");
        let r = ejecutar(&json!({"action": "crear_pr", "repo": "ana/juguete", "titulo": "Arregla el cero", "cuerpo": "porque sí", "cabeza": "arreglo", "borrador": true}), &d).await.unwrap();
        assert!(r.contains("PR #7 abierto: http://falso/ana/juguete/pull/7"), "{r}");
        let (auth, b) = visto.lock().unwrap()[0].clone();
        assert_eq!(auth, "Bearer tok-de-prueba");
        assert_eq!((b["title"].as_str(), b["head"].as_str(), b["base"].as_str(), b["draft"].as_bool()), (Some("Arregla el cero"), Some("arreglo"), Some("main"), Some(true)));
        let c = ejecutar(&json!({"action": "comentarios_pr", "repo": "ana/juguete", "numero": 7}), &d).await.unwrap();
        assert!(c.contains("· beto: bien") && c.contains("[99] beto en src/a.rs:12: ¿y si es cero?"), "{c}");
        let ci = ejecutar(&json!({"action": "estado_ci", "repo": "ana/juguete", "numero": "#7"}), &d).await.unwrap();
        assert_eq!(ci, "pruebas · completed · failure");
        assert!(ejecutar(&json!({"action": "fusionar_pr", "repo": "ana/juguete", "numero": 7}), &d).await.is_err(), "GOTV nunca fusiona");
        assert!(!es_suya("merge") && publica("crear_pr") && !publica("estado_ci"));
        std::env::remove_var("GOTV_GITHUB_API");
        std::env::remove_var("GITHUB_TOKEN");
    }

    fn sh(d: &Path, orden: &str) {
        std::process::Command::new("bash").arg("-c").arg(orden).current_dir(d).env("GIT_CONFIG_GLOBAL", "/dev/null").env("GIT_CONFIG_NOSYSTEM", "1").output().unwrap();
    }

    #[test]
    fn el_repo_sale_del_remoto_origin() {
        let d = crate::CarpetaPrueba::nueva("gotv-gh-origin");
        sh(&d, "git init -q && git remote add origin git@github.com:ana/juguete.git");
        assert_eq!(repo_del_proyecto(&d).as_deref(), Some("ana/juguete"));
        sh(&d, "git remote set-url origin https://github.com/ana/otro/");
        assert_eq!(repo_del_proyecto(&d).as_deref(), Some("ana/otro"));
    }
}
