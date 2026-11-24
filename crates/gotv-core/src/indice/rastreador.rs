//! F23.3 · El rastreador del índice propio: sólo las semillas que elige el usuario (y lo que cuelga de
//! ellas en el MISMO dominio), con `robots.txt`, `noindex`, sin páginas de sesión o de pago, a un ritmo
//! cortés por dominio y con un User-Agent honesto (F23.L-5). Profundidad y número de páginas con tope.

use super::{Indice, Pagina, DIAS_PAGINA};
use crate::buscar::cortesia::{self, Ritmo};
use crate::buscar::fusion::normalizar_url;
use crate::buscar::legal::{Procedencia, REGLA_INDICE};
use anyhow::Result;
use std::collections::{HashSet, VecDeque};
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct Opciones {
    pub profundidad: usize,
    pub max_paginas: usize,
    /// Una petición cada `ritmo` por dominio (F23.L-5: 1 s por omisión).
    pub ritmo: Duration,
    /// Las pruebas hablan con un servidor local; la app, jamás con direcciones privadas.
    pub permitir_privadas: bool,
}

impl Default for Opciones {
    fn default() -> Self {
        Self { profundidad: 2, max_paginas: 200, ritmo: Duration::from_millis(1_000), permitir_privadas: false }
    }
}

#[derive(Debug, Default, Clone)]
pub struct Informe {
    pub indexadas: Vec<String>,
    /// (url, por qué no).
    pub saltadas: Vec<(String, String)>,
}

/// El cuerpo del artículo: `<main>` o `<article>` si los hay; fuera menús, cabeceras, pies y aparte.
pub fn contenido_principal(html: &str) -> String {
    let l = html.to_ascii_lowercase();
    let dentro = |tag: &str| -> Option<&str> {
        let a = l.find(&format!("<{tag}"))?;
        let ini = a + l[a..].find('>')? + 1;
        let fin = ini + l[ini..].rfind(&format!("</{tag}>")).unwrap_or(l.len() - ini);
        html.get(ini..fin)
    };
    let cuerpo = dentro("main").or_else(|| dentro("article")).unwrap_or(html);
    let mut s = cuerpo.to_string();
    for tag in ["nav", "header", "footer", "aside", "form"] {
        s = quitar_bloques(&s, tag);
    }
    let t = crate::web::html_to_text(&s);
    t.lines().map(str::trim).filter(|x| !x.is_empty()).collect::<Vec<_>>().join("\n")
}

fn quitar_bloques(s: &str, tag: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut resto = s;
    loop {
        let l = resto.to_ascii_lowercase();
        let Some(a) = l.find(&format!("<{tag}")).filter(|&a| {
            // <nav> o <nav …>, no <navegador>
            l[a + 1 + tag.len()..].starts_with(['>', ' ', '\n', '\t'])
        }) else {
            out.push_str(resto);
            return out;
        };
        out.push_str(&resto[..a]);
        match l[a..].find(&format!("</{tag}>")) {
            Some(b) => resto = &resto[a + b + tag.len() + 3..],
            None => return out,
        }
    }
}

pub fn titulo_de(html: &str) -> String {
    let l = html.to_ascii_lowercase();
    l.find("<title")
        .and_then(|a| {
            let ini = a + l[a..].find('>')? + 1;
            let fin = ini + l[ini..].find("</title>")?;
            Some(crate::buscar::servicios::limpiar(&html[ini..fin]))
        })
        .unwrap_or_default()
}

/// Los enlaces `href` absolutos (o relativos, resueltos contra `base`) del mismo dominio.
pub fn enlaces(html: &str, base: &str) -> Vec<String> {
    let Ok(b) = reqwest::Url::parse(base) else { return Vec::new() };
    let dominio = super::dominio_de(base);
    let mut v = Vec::new();
    for trozo in html.split("href=").skip(1) {
        let t = trozo.trim_start();
        let (cierre, t) = match t.chars().next() {
            Some(c @ ('"' | '\'')) => (c, &t[1..]),
            _ => (' ', t),
        };
        let h = t.split([cierre, '>', ' ']).next().unwrap_or("").trim();
        if h.is_empty() || h.starts_with('#') || h.starts_with("mailto:") || h.starts_with("javascript:") {
            continue;
        }
        if let Ok(u) = b.join(&h.replace("&amp;", "&")) {
            let s = u.to_string();
            if s.starts_with("http") && super::dominio_de(&s) == dominio {
                v.push(s);
            }
        }
    }
    v
}

/// ¿Esta página se puede indexar? Mira el `robots.txt` del origen, el código, `noindex` y la sesión.
pub enum Veredicto {
    Indexar(Pagina, String),
    Saltar(String),
}

pub async fn leer_para_indice(cliente: &reqwest::Client, url: &str, permitir_privadas: bool) -> Veredicto {
    if !permitir_privadas {
        if let Some(r) = crate::web::private_reason(url) {
            return Veredicto::Saltar(r.to_string());
        }
    }
    let origen = match reqwest::Url::parse(url) {
        Ok(u) => u.origin().ascii_serialization(),
        Err(e) => return Veredicto::Saltar(e.to_string()),
    };
    let robots = cortesia::robots_de(cliente, &origen).await;
    if !robots.permite(&cortesia::ruta_de(url)) {
        return Veredicto::Saltar("robots.txt".into());
    }
    let resp = match cliente.get(url).send().await {
        Ok(r) => r,
        Err(e) => return Veredicto::Saltar(e.without_url().to_string()),
    };
    let estado = resp.status().as_u16();
    let xrt = resp.headers().get("x-robots-tag").and_then(|v| v.to_str().ok()).map(String::from);
    let es_html = resp.headers().get("content-type").and_then(|v| v.to_str().ok()).is_none_or(|t| t.contains("html"));
    let html = resp.text().await.unwrap_or_default();
    if cortesia::detras_de_sesion(estado, &html) {
        return Veredicto::Saltar(crate::i18n::t("con sesión o de pago", "behind a login or paywall").to_string());
    }
    if !(200..300).contains(&estado) {
        return Veredicto::Saltar(format!("HTTP {estado}"));
    }
    if !es_html {
        return Veredicto::Saltar(crate::i18n::t("no es HTML", "not HTML").to_string());
    }
    if cortesia::tiene_noindex(&html, xrt.as_deref()) {
        return Veredicto::Saltar("noindex".into());
    }
    let texto = contenido_principal(&html);
    Veredicto::Indexar(Pagina { url: url.to_string(), titulo: titulo_de(&html), texto }, html)
}

pub fn cliente() -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .user_agent(format!("GOTV/{} (rastreador local)", env!("CARGO_PKG_VERSION"))) // juez-idioma: datos (User-Agent de F23.L-5)
        .timeout(Duration::from_secs(20))
        .redirect(reqwest::redirect::Policy::limited(3))
        .build()?)
}

/// Rastrea desde las semillas. Cada página: turno de su dominio → veredicto → índice.
pub async fn rastrear(indice: &Indice, semillas: &[String], op: &Opciones, ritmo: &Ritmo, ahora: i64) -> Result<Informe> {
    let c = cliente()?;
    let mut inf = Informe::default();
    let mut vistas: HashSet<String> = HashSet::new();
    let mut cola: VecDeque<(String, usize)> = semillas.iter().map(|s| (s.clone(), 0)).collect();
    while let Some((url, prof)) = cola.pop_front() {
        if inf.indexadas.len() >= op.max_paginas {
            break;
        }
        if !vistas.insert(normalizar_url(&url)) {
            continue;
        }
        ritmo.esperar(&format!("dominio:{}", super::dominio_de(&url)), op.ritmo).await;
        match leer_para_indice(&c, &url, op.permitir_privadas).await {
            Veredicto::Saltar(r) => inf.saltadas.push((url, r)),
            Veredicto::Indexar(p, html) => {
                let pr = Procedencia { origen: super::dominio_de(&url), url: url.clone(), fecha: ahora, vence: ahora + DIAS_PAGINA * 86_400, regla: REGLA_INDICE.into() };
                // legal: F23.L-5 (semilla del usuario, robots.txt y noindex mirados arriba)
                indice.agregar(&p, &pr)?;
                inf.indexadas.push(url.clone());
                if prof < op.profundidad {
                    for e in enlaces(&html, &url) {
                        if !vistas.contains(&normalizar_url(&e)) {
                            cola.push_back((e, prof + 1));
                        }
                    }
                }
            }
        }
    }
    Ok(inf)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{http::header, response::IntoResponse, routing::get, Router};

    #[test]
    fn el_cuerpo_sin_menus() {
        let h = "<html><head><title>Guía &amp; más</title></head><body><nav>Inicio Blog</nav><main><h1>Tokio</h1><p>Tareas asíncronas.</p><aside>anuncio</aside></main><footer>pie</footer></body></html>";
        let t = contenido_principal(h);
        assert!(t.contains("Tokio") && t.contains("Tareas asíncronas."));
        assert!(!t.contains("Inicio") && !t.contains("anuncio") && !t.contains("pie"));
        assert_eq!(titulo_de(h), "Guía & más");
        let e = enlaces(r##"<a href="/a">a</a> <a href='https://otro.io/x'>x</a> <a href="b?c=1&amp;d=2">b</a> <a href="#top">t</a>"##, "https://doc.io/dir/p");
        assert_eq!(e, vec!["https://doc.io/a", "https://doc.io/dir/b?c=1&d=2"]);
    }

    /// Un sitio falso que prohíbe `/privado` en su robots.txt, con una página `noindex` y una de sesión.
    async fn sitio() -> String {
        let pag = |cuerpo: &'static str| move || async move { ([(header::CONTENT_TYPE, "text/html")], cuerpo).into_response() };
        let app = Router::new()
            .route("/robots.txt", get(|| async { "User-agent: *\nDisallow: /privado\n" }))
            .route("/", get(pag(r#"<title>Inicio</title><main><p>portada del sitio</p><a href="/a">a</a> <a href="/privado/secreto">s</a> <a href="/oculta">o</a> <a href="/entrar">e</a></main>"#)))
            .route("/a", get(pag(r#"<title>A</title><main>pagina a <a href="/b">b</a></main>"#)))
            .route("/b", get(pag(r#"<title>B</title><main>pagina b profunda</main>"#)))
            .route("/privado/secreto", get(pag("<title>S</title><main>secreto</main>")))
            .route("/oculta", get(pag(r#"<head><meta name="robots" content="noindex"></head><main>oculta</main>"#)))
            .route("/entrar", get(pag(r#"<form><input type="password"></form>"#)));
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let dir = format!("http://{}", l.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
        dir
    }

    #[tokio::test]
    async fn respeta_robots_noindex_sesion_y_profundidad() {
        let base = sitio().await;
        let d = crate::CarpetaPrueba::nueva("gotv-rastreador");
        let i = Indice::abrir(d.ruta()).unwrap();
        let op = Opciones { profundidad: 1, max_paginas: 50, ritmo: Duration::from_millis(1), permitir_privadas: true };
        let inf = rastrear(&i, &[format!("{base}/")], &op, &Ritmo::default(), 1_000).await.unwrap();
        let idx: Vec<&str> = inf.indexadas.iter().map(|u| u.trim_start_matches(&base)).collect();
        assert_eq!(idx, vec!["/", "/a"], "b está a profundidad 2: {inf:?}");
        let razon = |r: &str| inf.saltadas.iter().find(|(u, _)| u.ends_with(r)).map(|(_, x)| x.clone()).unwrap_or_default();
        assert_eq!(razon("/privado/secreto"), "robots.txt");
        assert_eq!(razon("/oculta"), "noindex");
        assert!(!razon("/entrar").is_empty());
        assert!(i.consultar("secreto", 5, 2_000).unwrap().is_empty());
        assert_eq!(i.consultar("portada", 5, 2_000).unwrap().len(), 1);
        assert!(i.procedencias().unwrap().iter().all(|p| p.validar().is_ok() && p.regla == REGLA_INDICE));
        // sin permiso para direcciones privadas (la app), nada de 127.0.0.1
        let op2 = Opciones { permitir_privadas: false, ..op };
        let inf2 = rastrear(&i, &[format!("{base}/")], &op2, &Ritmo::default(), 1_000).await.unwrap();
        assert!(inf2.indexadas.is_empty());
    }

    #[tokio::test]
    async fn diez_paginas_del_mismo_dominio_respetan_el_ritmo() {
        let base = sitio().await;
        let d = crate::CarpetaPrueba::nueva("gotv-rastreador-ritmo");
        let i = Indice::abrir(d.ruta()).unwrap();
        // F23 «Pruebas»: 10 páginas a 1/s tardan ≥ 9 s; aquí a 1 cada 100 ms → ≥ 0,9 s
        let semillas: Vec<String> = (0..10).map(|k| format!("{base}/?p={k}")).collect();
        let op = Opciones { profundidad: 0, max_paginas: 50, ritmo: Duration::from_millis(100), permitir_privadas: true };
        let t0 = std::time::Instant::now();
        let inf = rastrear(&i, &semillas, &op, &Ritmo::default(), 1_000).await.unwrap();
        assert_eq!(inf.indexadas.len(), 10);
        assert!(t0.elapsed() >= Duration::from_millis(900), "{:?}", t0.elapsed());
    }
}
