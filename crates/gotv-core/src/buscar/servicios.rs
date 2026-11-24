//! F23.1 · Los buscadores, cada uno con el formato REAL de su API (leído en su documentación oficial
//! el 2026-11-16; las fichas citan las fuentes). Sin llave: DuckDuckGo, Wikipedia, arXiv, GitHub.
//! Con la llave del usuario (F23.2): Brave, Serper, Tavily, Exa.
//!
//! F23.L-7: la llave viaja SÓLO en una cabecera, SÓLO al `dominio_api` de la ficha y sin seguir
//! redirecciones; ningún error la repite.

use super::legal::Ficha;
use super::{Buscador, Hallazgo};
use anyhow::{anyhow, Result};
use serde_json::Value;
use std::time::Duration;


/// F23.L-5: un User-Agent honesto, sin datos personales. `contacto` (una URL o un correo que el
/// usuario quiera dar, `[buscadores] contacto`) lo piden Wikimedia y GitHub para identificarse.
pub fn user_agent(contacto: &str) -> String {
    let v = env!("CARGO_PKG_VERSION");
    if contacto.trim().is_empty() {
        format!("GOTV/{v} (agente local)") // juez-idioma: datos (User-Agent)
    } else {
        format!("GOTV/{v} (agente local; {})", contacto.trim()) // juez-idioma: datos (User-Agent)
    }
}

/// La dirección de fábrica de cada servicio (las pruebas la cambian por un servidor falso local).
pub fn base_de_fabrica(id: &str) -> Option<String> {
    let wiki = if crate::i18n::is_en() { "https://en.wikipedia.org" } else { "https://es.wikipedia.org" };
    Some(
        match id {
            "duckduckgo" => "https://lite.duckduckgo.com",
            "wikipedia" => wiki,
            "arxiv" => "https://export.arxiv.org",
            "github" => "https://api.github.com",
            "brave" => "https://api.search.brave.com",
            "serper" => "https://google.serper.dev",
            "tavily" => "https://api.tavily.com",
            "exa" => "https://api.exa.ai",
            _ => return None,
        }
        .to_string(),
    )
}

/// Un servicio remoto con su ficha, su dirección y (si la necesita) la llave del usuario.
pub struct Servicio {
    pub ficha: Ficha,
    pub base: String,
    pub llave: Option<String>,
    pub ua: String,
    pub tope: Duration,
}

impl Servicio {
    fn cliente(&self) -> Result<reqwest::Client> {
        // honesto SIEMPRE, también con DuckDuckGo (Portador, 2026-11-17: nunca disfrazarse)
        let ua = self.ua.clone();
        Ok(reqwest::Client::builder()
            .user_agent(ua)
            // el tope lo pone el metabuscador (y dice «sin respuesta»); éste es sólo un seguro detrás
            .timeout(self.tope + Duration::from_secs(1))
            // F23.L-7: una redirección podría llevar la llave a otro dominio
            .redirect(reqwest::redirect::Policy::none())
            .build()?)
    }

    fn url(&self, ruta: &str) -> String {
        format!("{}{}", self.base.trim_end_matches('/'), ruta)
    }

    /// La llave, si la URL va a su dominio exacto (F23.L-7). Sin dominio correcto: error, nada sale.
    fn llave_para(&self, url: &str) -> Result<Option<&str>> {
        match self.llave.as_deref() {
            None if self.ficha.necesita_llave => Err(anyhow!("{}", crate::tr!(
                "{}: no hay llave (Configuración → Conexiones → Buscadores)",
                "{}: no key (Settings → Connections → Search engines)",
                self.ficha.nombre
            ))),
            None => Ok(None),
            Some(_) if !self.ficha.llave_puede_ir_a(url) => Err(anyhow!("{}", crate::tr!(
                "{}: la llave sólo puede ir a {} (F23.L-7); no se mandó nada",
                "{}: the key may only go to {} (F23.L-7); nothing was sent",
                self.ficha.nombre,
                self.ficha.dominio_api
            ))),
            Some(k) => Ok(Some(k)),
        }
    }

    /// Un error del servicio, sin la llave aunque el servidor la repita.
    fn error(&self, estado: reqwest::StatusCode, cuerpo: &str) -> anyhow::Error {
        let mut c: String = cuerpo.chars().take(200).collect();
        if let Some(k) = self.llave.as_deref().filter(|k| !k.is_empty()) {
            c = c.replace(k, "•••");
        }
        anyhow!("{} HTTP {estado}: {}", self.ficha.nombre, c.split_whitespace().collect::<Vec<_>>().join(" "))
    }

    async fn enviar(&self, r: reqwest::RequestBuilder) -> Result<String> {
        let resp = r.send().await.map_err(|e| {
            // el texto de reqwest lleva la URL (nunca la llave: va en cabecera); por si acaso, se limpia
            let mut t = e.without_url().to_string();
            if let Some(k) = self.llave.as_deref().filter(|k| !k.is_empty()) {
                t = t.replace(k, "•••");
            }
            anyhow!("{}: {t}", self.ficha.nombre)
        })?;
        let estado = resp.status();
        let cuerpo = resp.text().await.unwrap_or_default();
        if !estado.is_success() {
            return Err(self.error(estado, &cuerpo));
        }
        Ok(cuerpo)
    }
}

#[async_trait::async_trait]
impl Buscador for Servicio {
    fn id(&self) -> &str {
        &self.ficha.id
    }

    async fn buscar(&self, consulta: &str, n: usize) -> Result<Vec<Hallazgo>> {
        let n = n.clamp(1, 20);
        let ns = n.to_string();
        let c = self.cliente()?;
        let id = self.ficha.id.as_str();
        let hallazgos = match id {
            "duckduckgo" => {
                let url = self.url("/lite/");
                let html = self.enviar(c.post(&url).form(&[("q", consulta)])).await?;
                if html.contains("anomaly") {
                    return Err(anyhow!("{}", crate::i18n::t(
                        "DuckDuckGo no responde a un cliente que dice quién es (202 «anomaly»; sólo contesta a navegadores) y GOTV no se disfraza: usa Wikipedia, arXiv, GitHub o un buscador con llave",
                        "DuckDuckGo does not answer a client that says who it is (202 «anomaly»; it only answers browsers) and GOTV does not disguise itself: use Wikipedia, arXiv, GitHub or a search engine with a key"
                    )));
                }
                crate::research::parse_ddg_lite(&html)
                    .into_iter()
                    .map(|h| (h.title, h.url, h.snippet))
                    .collect::<Vec<_>>()
            }
            "wikipedia" => {
                let url = self.url("/w/rest.php/v1/search/page");
                let txt = self.enviar(c.get(&url).query(&[("q", consulta), ("limit", ns.as_str())])).await?;
                parse_wikipedia(&txt, &self.base)?
            }
            "arxiv" => {
                let url = self.url("/api/query");
                let q = format!("all:{consulta}");
                let txt = self.enviar(c.get(&url).query(&[("search_query", q.as_str()), ("max_results", ns.as_str())])).await?;
                parse_arxiv(&txt)
            }
            "github" => {
                let url = self.url("/search/repositories");
                let mut r = c
                    .get(&url)
                    .query(&[("q", consulta), ("per_page", ns.as_str())])
                    .header("Accept", "application/vnd.github+json");
                if let Some(k) = self.llave_para(&url)? {
                    r = r.bearer_auth(k);
                }
                parse_github(&self.enviar(r).await?)?
            }
            "brave" => {
                let url = self.url("/res/v1/web/search");
                let k = self.llave_para(&url)?.unwrap_or_default();
                let r = c
                    .get(&url)
                    .query(&[("q", consulta), ("count", ns.as_str())])
                    .header("Accept", "application/json")
                    .header("X-Subscription-Token", k);
                parse_brave(&self.enviar(r).await?)?
            }
            "serper" => {
                let url = self.url("/search");
                let k = self.llave_para(&url)?.unwrap_or_default();
                let r = c.post(&url).header("X-API-KEY", k).json(&serde_json::json!({ "q": consulta, "num": n }));
                parse_serper(&self.enviar(r).await?)?
            }
            "tavily" => {
                let url = self.url("/search");
                let k = self.llave_para(&url)?.unwrap_or_default();
                let r = c.post(&url).bearer_auth(k).json(&serde_json::json!({ "query": consulta, "max_results": n }));
                parse_tavily(&self.enviar(r).await?)?
            }
            "exa" => {
                let url = self.url("/search");
                let k = self.llave_para(&url)?.unwrap_or_default();
                // sin `contents`: el texto de las páginas se cobra aparte (ficha de Exa)
                let r = c.post(&url).header("x-api-key", k).json(&serde_json::json!({ "query": consulta, "numResults": n }));
                parse_exa(&self.enviar(r).await?)?
            }
            otro => return Err(anyhow!("{}", crate::tr!("buscador desconocido: {otro}", "unknown search engine: {otro}"))),
        };
        Ok(hallazgos
            .into_iter()
            .filter(|(_, u, _)| u.starts_with("http"))
            .take(n)
            .enumerate()
            .map(|(i, (t, u, r))| Hallazgo { titulo: limpiar(&t), url: u, resumen: recortar(&limpiar(&r), 300), fuente: id.to_string(), puesto: i + 1 })
            .collect())
    }
}

/// Texto sin etiquetas, entidades ni espacios de más.
pub fn limpiar(s: &str) -> String {
    let t = if s.contains('<') || s.contains('&') { crate::web::html_to_text(s) } else { s.to_string() };
    t.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn recortar(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut r: String = s.chars().take(max).collect();
    r.push('…');
    r
}

type Crudo = Vec<(String, String, String)>;

fn json(txt: &str) -> Result<Value> {
    serde_json::from_str(txt).map_err(|e| anyhow!("{}: {e}", crate::i18n::t("respuesta ilegible", "unreadable response")))
}

fn s(v: &Value) -> String {
    v.as_str().unwrap_or("").to_string()
}

/// Wikipedia REST `search/page`: `pages[].{key, title, excerpt, description}`.
pub fn parse_wikipedia(txt: &str, base: &str) -> Result<Crudo> {
    let v = json(txt)?;
    Ok(v["pages"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|p| {
                    let key = s(&p["key"]);
                    let url = format!("{}/wiki/{}", base.trim_end_matches('/'), enc_ruta(&key));
                    let mut r = s(&p["excerpt"]);
                    let d = s(&p["description"]);
                    if !d.is_empty() {
                        r = format!("{d} — {r}");
                    }
                    (s(&p["title"]), url, r)
                })
                .collect()
        })
        .unwrap_or_default())
}

/// Un tramo de ruta en `%XX` salvo lo que una ruta admite tal cual (los títulos de Wikipedia
/// llevan paréntesis, comas y dos puntos).
fn enc_ruta(s: &str) -> String {
    s.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-_.~()!*',:;@".contains(&b) {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}

fn etiqueta<'a>(bloque: &'a str, nombre: &str) -> Option<&'a str> {
    let a = bloque.find(&format!("<{nombre}"))?;
    let ini = a + bloque[a..].find('>')? + 1;
    let fin = ini + bloque[ini..].find(&format!("</{nombre}>"))?;
    Some(&bloque[ini..fin])
}

/// arXiv: Atom con `<entry>` → `<title>`, `<summary>`, `<link rel="alternate" href>` (o `<id>`).
pub fn parse_arxiv(xml: &str) -> Crudo {
    xml.split("<entry>")
        .skip(1)
        .map(|e| {
            let e = e.split("</entry>").next().unwrap_or(e);
            let alterno = e
                .split("<link")
                .skip(1)
                .find(|l| l.contains("rel=\"alternate\""))
                .and_then(|l| l.split("href=\"").nth(1))
                .and_then(|h| h.split('"').next())
                .map(String::from);
            let url = alterno.unwrap_or_else(|| etiqueta(e, "id").unwrap_or("").trim().replacen("http://", "https://", 1));
            (etiqueta(e, "title").unwrap_or("").to_string(), url, etiqueta(e, "summary").unwrap_or("").to_string())
        })
        .collect()
}

/// GitHub `search/repositories`: `items[].{full_name, html_url, description, stargazers_count, language}`.
/// Sólo lo del repositorio; nada del dueño (dato personal, ficha de GitHub).
pub fn parse_github(txt: &str) -> Result<Crudo> {
    let v = json(txt)?;
    Ok(v["items"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|r| {
                    let extra = format!(
                        "★{} · {}",
                        r["stargazers_count"].as_u64().unwrap_or(0),
                        r["language"].as_str().unwrap_or("-")
                    );
                    let d = s(&r["description"]);
                    (s(&r["full_name"]), s(&r["html_url"]), if d.is_empty() { extra } else { format!("{extra} — {d}") })
                })
                .collect()
        })
        .unwrap_or_default())
}

/// Brave: `web.results[].{title, url, description}`.
pub fn parse_brave(txt: &str) -> Result<Crudo> {
    let v = json(txt)?;
    Ok(v["web"]["results"]
        .as_array()
        .map(|a| a.iter().map(|r| (s(&r["title"]), s(&r["url"]), s(&r["description"]))).collect())
        .unwrap_or_default())
}

/// Serper: `organic[].{title, link, snippet, position}`.
pub fn parse_serper(txt: &str) -> Result<Crudo> {
    let v = json(txt)?;
    let mut a: Vec<&Value> = v["organic"].as_array().map(|a| a.iter().collect()).unwrap_or_default();
    a.sort_by_key(|r| r["position"].as_u64().unwrap_or(u64::MAX));
    Ok(a.into_iter().map(|r| (s(&r["title"]), s(&r["link"]), s(&r["snippet"]))).collect())
}

/// Tavily: `results[].{title, url, content}`.
pub fn parse_tavily(txt: &str) -> Result<Crudo> {
    let v = json(txt)?;
    Ok(v["results"]
        .as_array()
        .map(|a| a.iter().map(|r| (s(&r["title"]), s(&r["url"]), s(&r["content"]))).collect())
        .unwrap_or_default())
}

/// Exa: `results[].{title, url, publishedDate}` (sin texto: no se pide).
pub fn parse_exa(txt: &str) -> Result<Crudo> {
    let v = json(txt)?;
    Ok(v["results"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|r| {
                    let fecha = s(&r["publishedDate"]).chars().take(10).collect::<String>();
                    (s(&r["title"]), s(&r["url"]), fecha)
                })
                .collect()
        })
        .unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cada_formato_real_se_lee() {
        // formatos abreviados de los ejemplos reales (2026-11-16)
        let w = r#"{"pages":[{"id":5792625,"key":"Rust_(lenguaje_de_programación)","title":"Rust (lenguaje de programación)","excerpt":"<span class=\"searchmatch\">Rust</span> es un lenguaje","matched_title":null,"anchor":null,"description":"lenguaje de programación","thumbnail":null}]}"#;
        let v = parse_wikipedia(w, "https://es.wikipedia.org").unwrap();
        assert_eq!(v[0].0, "Rust (lenguaje de programación)");
        assert!(v[0].1.starts_with("https://es.wikipedia.org/wiki/Rust_(lenguaje_de_programaci%C3%B3n)"), "{}", v[0].1);
        assert!(limpiar(&v[0].2).contains("Rust es un lenguaje"));

        let x = r#"<feed><entry>
      <id>http://arxiv.org/abs/cond-mat/0011267v1</id>
      <title>The electronic structure of cuprates</title>
      <link href="https://arxiv.org/abs/cond-mat/0011267v1" rel="alternate" type="text/html"/>
      <link href="https://arxiv.org/pdf/cond-mat/0011267v1" rel="related" type="application/pdf" title="pdf"/>
      <summary>  We report studies
      of the electronic structure </summary></entry></feed>"#;
        let v = parse_arxiv(x);
        assert_eq!(v[0].1, "https://arxiv.org/abs/cond-mat/0011267v1");
        assert_eq!(limpiar(&v[0].2), "We report studies of the electronic structure");

        let g = r#"{"total_count":1,"items":[{"full_name":"MinusKelvin/cold-clear","html_url":"https://github.com/MinusKelvin/cold-clear","description":"Tetris Bot","owner":{"login":"MinusKelvin"},"stargazers_count":278,"language":"Rust"}]}"#;
        let v = parse_github(g).unwrap();
        assert_eq!(v[0].2, "★278 · Rust — Tetris Bot");

        let b = r#"{"web":{"type":"search","results":[{"title":"Python Web Frameworks","url":"https://example.com/python-frameworks","description":"Main snippet text..."}]}}"#;
        assert_eq!(parse_brave(b).unwrap()[0].1, "https://example.com/python-frameworks");

        let sp = r#"{"organic":[{"title":"B","link":"https://b.io","snippet":"s","position":2},{"title":"Apple","link":"https://www.apple.com/","snippet":"Discover","position":1}]}"#;
        assert_eq!(parse_serper(sp).unwrap()[0].0, "Apple", "por su posición");

        let t = r#"{"query":"q","results":[{"title":"Lionel Messi Facts | Britannica","url":"https://www.britannica.com/facts/Lionel-Messi","content":"Lionel Messi, an Argentine footballer","score":0.81}]}"#;
        assert_eq!(parse_tavily(t).unwrap()[0].2, "Lionel Messi, an Argentine footballer");

        let e = r#"{"requestId":"r","results":[{"title":"T","url":"https://t.io","id":"https://t.io","publishedDate":"2025-03-11T00:00:00.000Z","author":null}]}"#;
        assert_eq!(parse_exa(e).unwrap()[0].2, "2025-03-11");

        assert!(parse_brave("<html>no</html>").is_err());
    }

    #[test]
    fn el_user_agent_es_honesto_y_sin_datos_personales() {
        let ua = user_agent("");
        assert!(ua.starts_with("GOTV/") && !ua.contains("Mozilla"));
        assert!(!ua.contains(&*crate::paths::home().to_string_lossy()));
        assert!(user_agent("https://ejemplo.org").ends_with("; https://ejemplo.org)"));
    }
}
