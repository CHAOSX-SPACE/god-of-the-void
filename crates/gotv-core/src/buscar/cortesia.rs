//! F23.L-5 · Rastrear con educación: `robots.txt` (RFC 9309), `noindex`, ritmo por dominio y nada
//! detrás de un inicio de sesión. Lo usa el índice propio (F23.3) antes de guardar una página, y el
//! metabuscador para el ritmo de cada servicio (arXiv: una petición cada 3 s).
//!
//! Escrito aquí y no con `texting_robots`/`governor` (los del plan): son ~150 líneas contra dos
//! crates más en un árbol caliente; la conducta es la del RFC 9309 y está probada abajo.
//! Fuente: <https://www.rfc-editor.org/rfc/rfc9309> (Robots Exclusion Protocol, sept. 2022).

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;
use tokio::time::Instant;

/// El nombre con que GOTV se busca en un `robots.txt` (su «product token», RFC 9309 §2.2.1).
pub const PRODUCTO: &str = "gotv";

// ── ritmo ───────────────────────────────────────────────────────────────

/// Un turno por clave (dominio o servicio) cada `intervalo`: quien llega antes de su turno espera.
#[derive(Default)]
pub struct Ritmo {
    proximo: Mutex<HashMap<String, Instant>>,
}

impl Ritmo {
    /// El de todo el proceso (dos búsquedas a la vez comparten el ritmo de arXiv).
    pub fn global() -> Arc<Ritmo> {
        static G: OnceLock<Arc<Ritmo>> = OnceLock::new();
        G.get_or_init(|| Arc::new(Ritmo::default())).clone()
    }

    pub async fn esperar(&self, clave: &str, intervalo: Duration) {
        let turno = {
            let mut m = self.proximo.lock().unwrap_or_else(|e| e.into_inner());
            let ahora = Instant::now();
            let t = m.get(clave).copied().filter(|t| *t > ahora).unwrap_or(ahora);
            m.insert(clave.to_string(), t + intervalo);
            t
        };
        tokio::time::sleep_until(turno).await;
    }
}

// ── robots.txt (RFC 9309) ───────────────────────────────────────────────

#[derive(Debug, Clone, Default)]
struct Grupo {
    agentes: Vec<String>,
    reglas: Vec<(bool, String)>, // (allow, patrón)
}

#[derive(Debug, Clone)]
pub enum Robots {
    /// Todo permitido (no hay archivo: 4xx, RFC 9309 §2.3.1.3).
    Todo,
    /// Nada permitido (el servidor falla: 5xx o sin red, §2.3.1.4).
    Nada,
    Reglas(Vec<(bool, String)>),
}

impl Robots {
    /// Las reglas que tocan a `producto`: su grupo (o la unión de los suyos); si no hay, el de `*`.
    pub fn desde_texto(texto: &str, producto: &str) -> Robots {
        let mut grupos: Vec<Grupo> = Vec::new();
        let mut actual = Grupo::default();
        let mut en_agentes = false;
        for linea in texto.lines() {
            let l = linea.split('#').next().unwrap_or("").trim();
            let Some((k, v)) = l.split_once(':') else { continue };
            let (k, v) = (k.trim().to_ascii_lowercase(), v.trim().to_string());
            match k.as_str() {
                "user-agent" => {
                    if !en_agentes && (!actual.agentes.is_empty() || !actual.reglas.is_empty()) {
                        grupos.push(std::mem::take(&mut actual));
                    }
                    actual.agentes.push(v.to_ascii_lowercase());
                    en_agentes = true;
                }
                "allow" | "disallow" => {
                    en_agentes = false;
                    if actual.agentes.is_empty() {
                        continue; // regla fuera de grupo: se ignora
                    }
                    // «Disallow:» vacío no prohíbe nada
                    if !v.is_empty() {
                        actual.reglas.push((k == "allow", v));
                    }
                }
                _ => {}
            }
        }
        if !actual.agentes.is_empty() {
            grupos.push(actual);
        }
        let p = producto.to_ascii_lowercase();
        let propias: Vec<(bool, String)> = grupos.iter().filter(|g| g.agentes.iter().any(|a| *a == p)).flat_map(|g| g.reglas.clone()).collect();
        if !propias.is_empty() || grupos.iter().any(|g| g.agentes.iter().any(|a| *a == p)) {
            return Robots::Reglas(propias);
        }
        Robots::Reglas(grupos.iter().filter(|g| g.agentes.iter().any(|a| a == "*")).flat_map(|g| g.reglas.clone()).collect())
    }

    /// ¿Se puede pedir `ruta` (con su `?consulta`)? Gana la regla más larga; a igual largo, `allow`
    /// (RFC 9309 §2.2.2). `/robots.txt` siempre se puede.
    pub fn permite(&self, ruta: &str) -> bool {
        let ruta = if ruta.is_empty() { "/" } else { ruta };
        if ruta == "/robots.txt" {
            return true;
        }
        match self {
            Robots::Todo => true,
            Robots::Nada => false,
            Robots::Reglas(r) => {
                let mut mejor: Option<(usize, bool)> = None;
                for (allow, patron) in r {
                    if casa(patron, ruta) {
                        let largo = patron.len();
                        mejor = match mejor {
                            Some((l, a)) if l > largo || (l == largo && a) => Some((l, a)),
                            _ => Some((largo, *allow)),
                        };
                    }
                }
                mejor.map(|(_, a)| a).unwrap_or(true)
            }
        }
    }
}

/// `*` = cualquier cosa; `$` al final = fin de la ruta; si no, el patrón es un prefijo.
fn casa(patron: &str, ruta: &str) -> bool {
    let (patron, anclado) = match patron.strip_suffix('$') {
        Some(p) => (p, true),
        None => (patron, false),
    };
    let trozos: Vec<&str> = patron.split('*').collect();
    let mut pos = 0usize;
    for (i, t) in trozos.iter().enumerate() {
        if i == 0 {
            if !ruta.starts_with(t) {
                return false;
            }
            pos = t.len();
            continue;
        }
        if t.is_empty() {
            continue;
        }
        let ultimo = i == trozos.len() - 1;
        if ultimo && anclado {
            return ruta.len() >= pos + t.len() && ruta.ends_with(t);
        }
        match ruta[pos..].find(t) {
            Some(j) => pos += j + t.len(),
            None => return false,
        }
    }
    !anclado || pos == ruta.len() || patron.ends_with('*')
}

/// La ruta (con consulta) de una URL, para preguntarle a su `robots.txt`.
pub fn ruta_de(url: &str) -> String {
    let resto = url.split_once("://").map(|(_, r)| r).unwrap_or(url);
    let resto = resto.split('#').next().unwrap_or("");
    match resto.find('/') {
        Some(i) => resto[i..].to_string(),
        None => "/".to_string(),
    }
}

/// El `robots.txt` de un origen, recordado 24 h (RFC 9309 §2.4) en memoria.
pub async fn robots_de(cliente: &reqwest::Client, origen: &str) -> Robots {
    static MEMO: OnceLock<Mutex<HashMap<String, (std::time::Instant, Robots)>>> = OnceLock::new();
    let memo = MEMO.get_or_init(Default::default);
    if let Some((t, r)) = memo.lock().unwrap_or_else(|e| e.into_inner()).get(origen) {
        if t.elapsed() < Duration::from_secs(24 * 3_600) {
            return r.clone();
        }
    }
    let r = match cliente.get(format!("{}/robots.txt", origen.trim_end_matches('/'))).send().await {
        Ok(resp) if resp.status().is_success() => Robots::desde_texto(&resp.text().await.unwrap_or_default(), PRODUCTO),
        Ok(resp) if resp.status().is_client_error() => Robots::Todo,
        _ => Robots::Nada,
    };
    memo.lock().unwrap_or_else(|e| e.into_inner()).insert(origen.to_string(), (std::time::Instant::now(), r.clone()));
    r
}

// ── noindex y páginas con sesión ─────────────────────────────────────────

/// `<meta name="robots" content="…noindex…">` (o `name="gotv"`), o `X-Robots-Tag: noindex`.
pub fn tiene_noindex(html: &str, x_robots_tag: Option<&str>) -> bool {
    if x_robots_tag.is_some_and(|v| v.to_ascii_lowercase().contains("noindex") || v.to_ascii_lowercase().contains("none")) {
        return true;
    }
    let l = html.to_ascii_lowercase();
    l.split("<meta").skip(1).any(|m| {
        let m = m.split('>').next().unwrap_or("");
        (m.contains("name=\"robots\"") || m.contains("name='robots'") || m.contains(&format!("name=\"{PRODUCTO}\"")))
            && (m.contains("noindex") || m.contains("\"none\""))
    })
}

/// F23.L-5: una página de pago, con sesión o de inicio de sesión no se indexa. Señales: 401/402/403,
/// un campo de contraseña, o la marca de contenido de pago de schema.org (`isAccessibleForFree: false`).
pub fn detras_de_sesion(estado: u16, html: &str) -> bool {
    let l = html.to_ascii_lowercase();
    matches!(estado, 401 | 402 | 403)
        || l.contains("type=\"password\"")
        || l.contains("type='password'")
        || l.replace(' ', "").contains("\"isaccessibleforfree\":false")
        || l.replace(' ', "").contains("\"isaccessibleforfree\":\"false\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn robots_como_el_rfc_9309() {
        let t = "# comentario\nUser-agent: *\nDisallow: /privado\nAllow: /privado/publico\nDisallow: /*.pdf$\n\nUser-agent: OtroBot\nDisallow: /\n";
        let r = Robots::desde_texto(t, PRODUCTO);
        assert!(r.permite("/"));
        assert!(r.permite("/docs/a.html"));
        assert!(!r.permite("/privado"));
        assert!(!r.permite("/privado/x"));
        assert!(r.permite("/privado/publico/x"), "la regla más larga gana");
        assert!(!r.permite("/a/b.pdf") && r.permite("/a/b.pdf?x=1"), "$ ancla el final");
        assert!(r.permite("/robots.txt"));
        // un grupo para GOTV manda sobre el de *
        let t2 = "User-agent: *\nDisallow: /\n\nUser-agent: GOTV\nAllow: /\nDisallow: /lento\n";
        let r2 = Robots::desde_texto(t2, PRODUCTO);
        assert!(r2.permite("/docs") && !r2.permite("/lento/1"));
        // a igual largo, allow
        let r3 = Robots::desde_texto("User-agent: *\nDisallow: /x\nAllow: /x\n", PRODUCTO);
        assert!(r3.permite("/x"));
        // varios agentes en un grupo
        let r4 = Robots::desde_texto("User-agent: a\nUser-agent: *\nDisallow: /no\n", PRODUCTO);
        assert!(!r4.permite("/no"));
        // Disallow vacío no prohíbe
        assert!(Robots::desde_texto("User-agent: *\nDisallow:\n", PRODUCTO).permite("/todo"));
        assert!(!Robots::Nada.permite("/") && Robots::Todo.permite("/x"));
        assert_eq!(ruta_de("https://a.io/b/c?d=1#e"), "/b/c?d=1");
        assert_eq!(ruta_de("https://a.io"), "/");
    }

    #[test]
    fn noindex_y_sesion() {
        assert!(tiene_noindex(r#"<head><meta name="robots" content="noindex, follow"></head>"#, None));
        assert!(tiene_noindex(r#"<meta content="none" name="robots">"#, None));
        assert!(tiene_noindex("<p>hola</p>", Some("noindex")));
        assert!(!tiene_noindex(r#"<meta name="description" content="noindex no es esto">"#, None));
        assert!(detras_de_sesion(200, r#"<form><input type="password" name="p"></form>"#));
        assert!(detras_de_sesion(403, ""));
        assert!(detras_de_sesion(200, r#"{"isAccessibleForFree": false}"#));
        assert!(!detras_de_sesion(200, "<p>libre</p>"));
    }

    #[tokio::test]
    async fn el_ritmo_reparte_los_turnos() {
        let r = Ritmo::default();
        let t0 = std::time::Instant::now();
        for _ in 0..10 {
            r.esperar("a.io", Duration::from_millis(50)).await;
        }
        // 10 peticiones a 1 cada 50 ms: la primera sale ya, las otras 9 esperan su turno
        assert!(t0.elapsed() >= Duration::from_millis(450), "{:?}", t0.elapsed());
        // otro dominio no espera al primero
        let t1 = std::time::Instant::now();
        r.esperar("b.io", Duration::from_secs(5)).await;
        assert!(t1.elapsed() < Duration::from_millis(50));
    }
}
