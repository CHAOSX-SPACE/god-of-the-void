//! C2 — Proxy de red de la caja de arena (idea de Codex `network-proxy`, hecho a nuestra
//! medida): los comandos en la caja sólo pueden hablar con `localhost`; lo de fuera pasa
//! por este proxy (HTTP CONNECT y HTTP plano), que decide POR DOMINIO.
//!
//! - Cada proceso recibe una CREDENCIAL propia (`HTTPS_PROXY=http://gotv:<token>@127.0.0.1:<port>`):
//!   así el proxy sabe de quién es cada conexión y qué dominios tiene permitidos.
//! - Lo no permitido responde 403 y queda ANOTADO en el proceso: GOTV se lo pregunta al
//!   usuario después («¿permitir la red a github.com en este proyecto?»).
//! - Corre en un hilo con su PROPIO runtime: no muere con el runtime de quien lo pidió.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Lo que un proceso puede alcanzar.
pub struct Grant {
    pub token: String,
    /// Dominios permitidos: `pypi.org` (y sus subdominios) o `*.ejemplo.com`.
    hosts: Vec<String>,
    /// Toda la red (modo bypass).
    all: bool,
    /// Lo que se bloqueó, con cuántas veces.
    blocked: Mutex<BTreeMap<String, u32>>,
}

impl Grant {
    pub fn allows(&self, host: &str) -> bool {
        let h = host.trim().trim_end_matches('.').to_lowercase();
        if self.all || is_loopback(&h) {
            return true;
        }
        self.hosts.iter().any(|p| host_matches(p, &h))
    }

    /// Lo bloqueado hasta ahora (dominio, veces).
    pub fn blocked(&self) -> Vec<(String, u32)> {
        self.blocked
            .lock()
            .unwrap()
            .iter()
            .map(|(h, n)| (h.clone(), *n))
            .collect()
    }

    fn note_blocked(&self, host: &str) {
        *self
            .blocked
            .lock()
            .unwrap()
            .entry(host.to_lowercase())
            .or_insert(0) += 1;
    }
}

/// `pypi.org` casa con `pypi.org` y `x.pypi.org`; `*.ejemplo.com` sólo con subdominios.
pub fn host_matches(pattern: &str, host: &str) -> bool {
    let p = pattern.trim().trim_end_matches('.').to_lowercase();
    if p.is_empty() {
        return false;
    }
    if p == "*" {
        return true;
    }
    if let Some(suffix) = p.strip_prefix("*.") {
        return host.ends_with(&format!(".{suffix}"));
    }
    host == p || host.ends_with(&format!(".{p}"))
}

fn is_loopback(h: &str) -> bool {
    matches!(h, "localhost" | "127.0.0.1" | "::1" | "[::1]") || h.ends_with(".localhost")
}

fn grants() -> &'static Mutex<HashMap<String, Arc<Grant>>> {
    static G: OnceLock<Mutex<HashMap<String, Arc<Grant>>>> = OnceLock::new();
    G.get_or_init(|| Mutex::new(HashMap::new()))
}

fn token() -> String {
    let mut b = [0u8; 16];
    if let Ok(mut f) = std::fs::File::open("/dev/urandom") {
        let _ = std::io::Read::read_exact(&mut f, &mut b);
    }
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// Da una credencial nueva con estos permisos.
pub fn grant(hosts: Vec<String>, all: bool) -> Arc<Grant> {
    let g = Arc::new(Grant {
        token: token(),
        hosts,
        all,
        blocked: Mutex::new(BTreeMap::new()),
    });
    grants().lock().unwrap().insert(g.token.clone(), g.clone());
    g
}

/// La retira (el proceso terminó).
pub fn revoke(token: &str) {
    grants().lock().unwrap().remove(token);
}

/// Arranca el proxy (una sola vez) y devuelve su puerto.
pub fn ensure() -> std::io::Result<u16> {
    static PORT: OnceLock<u16> = OnceLock::new();
    if let Some(p) = PORT.get() {
        return Ok(*p);
    }
    let l = std::net::TcpListener::bind("127.0.0.1:0")?;
    let port = l.local_addr()?.port();
    l.set_nonblocking(true)?;
    std::thread::Builder::new()
        .name("gotv-netproxy".into())
        .spawn(move || {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .expect("netproxy runtime");
            rt.block_on(async move {
                let l = tokio::net::TcpListener::from_std(l).expect("proxy");
                loop {
                    if let Ok((s, _)) = l.accept().await {
                        tokio::spawn(async move {
                            let _ = handle(s).await;
                        });
                    }
                }
            });
        })?;
    let _ = PORT.set(port);
    Ok(*PORT.get().unwrap())
}

/// Lo que se le pone al comando para que use el proxy con SU credencial.
pub fn env_for(port: u16, g: &Grant) -> Vec<(String, String)> {
    let url = format!("http://gotv:{}@127.0.0.1:{port}", g.token);
    let mut v = Vec::new();
    for k in [
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "ALL_PROXY",
        "http_proxy",
        "https_proxy",
        "all_proxy",
    ] {
        v.push((k.to_string(), url.clone()));
    }
    for k in ["NO_PROXY", "no_proxy"] {
        v.push((
            k.to_string(),
            "localhost,127.0.0.1,::1,.localhost".to_string(),
        ));
    }
    // Node ≥ 24 sólo respeta *_PROXY en fetch() con esto
    v.push(("NODE_USE_ENV_PROXY".into(), "1".into()));
    v
}

async fn handle(mut s: tokio::net::TcpStream) -> std::io::Result<()> {
    // la cabecera de la petición (con tope)
    let mut buf = Vec::with_capacity(4096);
    let mut chunk = [0u8; 4096];
    let head_end = loop {
        let n = tokio::time::timeout(Duration::from_secs(15), s.read(&mut chunk))
            .await
            .map_err(|_| std::io::ErrorKind::TimedOut)??;
        if n == 0 {
            return Ok(());
        }
        buf.extend_from_slice(&chunk[..n]);
        if let Some(i) = find(&buf, b"\r\n\r\n") {
            break i + 4;
        }
        if buf.len() > 32 * 1024 {
            return reply(
                &mut s,
                431,
                crate::i18n::t("cabecera demasiado grande", "header too large"),
            )
            .await;
        }
    };
    let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
    let rest = buf[head_end..].to_vec();
    let mut lines = head.split("\r\n");
    let first = lines.next().unwrap_or("");
    let mut parts = first.split_whitespace();
    let (method, target, version) = (
        parts.next().unwrap_or(""),
        parts.next().unwrap_or(""),
        parts.next().unwrap_or("HTTP/1.1"),
    );
    let auth = lines.clone().find_map(|l| {
        l.split_once(':')
            .filter(|(k, _)| k.trim().eq_ignore_ascii_case("proxy-authorization"))
            .map(|(_, v)| v.trim().to_string())
    });
    let Some(g) = auth
        .as_deref()
        .and_then(token_of)
        .and_then(|t| grants().lock().unwrap().get(&t).cloned())
    else {
        return reply(
            &mut s,
            407,
            crate::i18n::t(
                "GOD OF THE VOID: proceso sin credencial de la caja",
                "GOD OF THE VOID: process without sandbox credential",
            ),
        )
        .await;
    };

    if method.eq_ignore_ascii_case("CONNECT") {
        let (host, port) = split_host_port(target, 443);
        if !g.allows(&host) {
            g.note_blocked(&host);
            return reply(&mut s, 403, &crate::tr!("GOD OF THE VOID: la caja bloqueó la red a {host} (hay que permitirla)", "GOD OF THE VOID: the sandbox blocked the network to {host} (it must be allowed)")).await;
        }
        let mut up = match tokio::time::timeout(
            Duration::from_secs(20),
            tokio::net::TcpStream::connect((host.as_str(), port)),
        )
        .await
        {
            Ok(Ok(u)) => u,
            _ => {
                return reply(
                    &mut s,
                    502,
                    &crate::tr!(
                        "no pude conectar con {host}:{port}",
                        "could not connect to {host}:{port}"
                    ),
                )
                .await
            }
        };
        s.write_all(b"HTTP/1.1 200 Connection established\r\n\r\n")
            .await?;
        if !rest.is_empty() {
            up.write_all(&rest).await?;
        }
        let _ = tokio::io::copy_bidirectional(&mut s, &mut up).await;
        return Ok(());
    }

    // HTTP plano: «GET http://host:port/ruta HTTP/1.1»
    let Some(after) = target.strip_prefix("http://") else {
        return reply(
            &mut s,
            400,
            crate::i18n::t(
                "sólo CONNECT o http:// absoluto",
                "only CONNECT or absolute http://",
            ),
        )
        .await;
    };
    let (authority, path) = match after.find('/') {
        Some(i) => (&after[..i], &after[i..]),
        None => (after, "/"),
    };
    let (host, port) = split_host_port(authority, 80);
    if !g.allows(&host) {
        g.note_blocked(&host);
        return reply(
            &mut s,
            403,
            &crate::tr!(
                "GOD OF THE VOID: la caja bloqueó la red a {host} (hay que permitirla)",
                "GOD OF THE VOID: the sandbox blocked the network to {host} (it must be allowed)"
            ),
        )
        .await;
    }
    let mut up = match tokio::time::timeout(
        Duration::from_secs(20),
        tokio::net::TcpStream::connect((host.as_str(), port)),
    )
    .await
    {
        Ok(Ok(u)) => u,
        _ => {
            return reply(
                &mut s,
                502,
                &crate::tr!(
                    "no pude conectar con {host}:{port}",
                    "could not connect to {host}:{port}"
                ),
            )
            .await
        }
    };
    // la misma petición en forma de origen, sin las cabeceras del proxy
    let mut out = format!("{method} {path} {version}\r\n");
    for l in lines.filter(|l| !l.is_empty()) {
        let k = l
            .split(':')
            .next()
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase();
        if k != "proxy-authorization" && k != "proxy-connection" {
            out.push_str(l);
            out.push_str("\r\n");
        }
    }
    out.push_str("\r\n");
    up.write_all(out.as_bytes()).await?;
    if !rest.is_empty() {
        up.write_all(&rest).await?;
    }
    let _ = tokio::io::copy_bidirectional(&mut s, &mut up).await;
    Ok(())
}

fn find(h: &[u8], n: &[u8]) -> Option<usize> {
    h.windows(n.len()).position(|w| w == n)
}

/// `Basic base64(gotv:<token>)` → token.
fn token_of(auth: &str) -> Option<String> {
    use base64::Engine;
    let b64 = auth
        .strip_prefix("Basic ")
        .or_else(|| auth.strip_prefix("basic "))?;
    let raw = base64::engine::general_purpose::STANDARD
        .decode(b64.trim())
        .ok()?;
    let s = String::from_utf8(raw).ok()?;
    s.split_once(':').map(|(_, t)| t.to_string())
}

fn split_host_port(s: &str, default: u16) -> (String, u16) {
    if let Some(rest) = s.strip_prefix('[') {
        // [::1]:443
        if let Some((h, p)) = rest.split_once("]:") {
            return (h.to_string(), p.parse().unwrap_or(default));
        }
        return (rest.trim_end_matches(']').to_string(), default);
    }
    match s.rsplit_once(':') {
        Some((h, p)) if p.chars().all(|c| c.is_ascii_digit()) && !p.is_empty() => {
            (h.to_lowercase(), p.parse().unwrap_or(default))
        }
        _ => (s.to_lowercase(), default),
    }
}

async fn reply(s: &mut tokio::net::TcpStream, code: u16, msg: &str) -> std::io::Result<()> {
    let reason = match code {
        400 => "Bad Request",
        403 => "Forbidden",
        407 => "Proxy Authentication Required",
        431 => "Request Header Fields Too Large",
        _ => "Bad Gateway",
    };
    let body = format!("{msg}\n");
    let r = format!("HTTP/1.1 {code} {reason}\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
    s.write_all(r.as_bytes()).await?;
    s.shutdown().await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn los_dominios_casan_con_sus_subdominios_y_nada_mas() {
        assert!(host_matches("pypi.org", "pypi.org") && host_matches("pypi.org", "files.pypi.org"));
        assert!(!host_matches("pypi.org", "evilpypi.org"));
        assert!(host_matches(
            "*.githubusercontent.com",
            "objects.githubusercontent.com"
        ));
        assert!(!host_matches(
            "*.githubusercontent.com",
            "githubusercontent.com"
        ));
        let g = grant(vec!["registry.npmjs.org".into()], false);
        assert!(g.allows("registry.npmjs.org") && g.allows("localhost") && !g.allows("github.com"));
        revoke(&g.token);
    }

    #[test]
    fn host_y_puerto() {
        assert_eq!(
            split_host_port("pypi.org:443", 80),
            ("pypi.org".into(), 443)
        );
        assert_eq!(
            split_host_port("ejemplo.com", 80),
            ("ejemplo.com".into(), 80)
        );
        assert_eq!(split_host_port("[::1]:8080", 80), ("::1".into(), 8080));
    }

    /// Por el proxy de verdad: un dominio permitido llega; uno no permitido da 403 y queda
    /// anotado; sin credencial, 407.
    #[tokio::test]
    async fn el_proxy_deja_pasar_lo_permitido_y_anota_lo_bloqueado() {
        // un «servidor externo» en localhost que se pide por un nombre: lo hacemos con
        // CONNECT a 127.0.0.1 (siempre permitido) y a un dominio no permitido
        let up = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let up_port = up.local_addr().unwrap().port();
        tokio::spawn(async move {
            while let Ok((mut c, _)) = up.accept().await {
                tokio::spawn(async move {
                    let mut b = [0u8; 512];
                    let _ = c.read(&mut b).await;
                    let _ = c.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\nhola!").await;
                });
            }
        });
        let port = ensure().unwrap();
        let g = grant(vec!["pypi.org".into()], false);
        let auth = {
            use base64::Engine;
            base64::engine::general_purpose::STANDARD.encode(format!("gotv:{}", g.token))
        };
        let ask = |req: String| async move {
            let mut c = tokio::net::TcpStream::connect(("127.0.0.1", port))
                .await
                .unwrap();
            c.write_all(req.as_bytes()).await.unwrap();
            let mut out = Vec::new();
            let _ = tokio::time::timeout(Duration::from_secs(5), c.read_to_end(&mut out)).await;
            String::from_utf8_lossy(&out).to_string()
        };
        // permitido (loopback) por HTTP plano
        let r = ask(format!("GET http://127.0.0.1:{up_port}/ HTTP/1.1\r\nHost: x\r\nProxy-Authorization: Basic {auth}\r\n\r\n")).await;
        assert!(r.contains("hola!"), "{r}");
        // no permitido
        let r = ask(format!(
            "CONNECT github.com:443 HTTP/1.1\r\nProxy-Authorization: Basic {auth}\r\n\r\n"
        ))
        .await;
        assert!(
            r.starts_with("HTTP/1.1 403") && r.contains("github.com"),
            "{r}"
        );
        assert_eq!(g.blocked(), vec![("github.com".to_string(), 1)]);
        // sin credencial
        let r = ask("CONNECT pypi.org:443 HTTP/1.1\r\n\r\n".into()).await;
        assert!(r.starts_with("HTTP/1.1 407"), "{r}");
        revoke(&g.token);
    }
}
