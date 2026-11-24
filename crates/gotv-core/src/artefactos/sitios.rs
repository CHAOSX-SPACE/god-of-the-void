//! F7 paso 7 · Sitios permitidos y prohibidos para el navegador propio (`browser`): `.gotv/navegador`,
//! texto simple que escribe el usuario, con dos listas aparte, una para ABRIR y otra para DESCARGAR.
//!
//! ```text
//! abrir permitir docs.rs
//! abrir negar facebook.com
//! descargar permitir github.com
//! descargar negar *
//! ```
//!
//! Lo mismo vale para `investigar` (`navegador::LectorWeb`): allí sólo pesa «abrir negar» (no hay a quién
//! preguntar) y una lista rota no deja leer nada.
//!
//! - Un patrón vale para el dominio y sus subdominios (`facebook.com` casa con `www.facebook.com`);
//!   `*` casa con todos. Se puede pegar una dirección entera: se toma su dominio.
//! - Trampa 4: «negar» manda sobre todo, también sobre el bypass y sobre los permisos guardados. Abrir
//!   en «permitir» abre sin preguntar, pero NO salta el permiso de descarga: descargar tiene su lista y,
//!   fuera de ella, pregunta (en bypass pasa, decisión del Portador: bypass = control total).
//! - Lo escribe el usuario (o la ventana), nunca el modelo ([`super::protegido`]). Si el archivo está
//!   roto, el navegador no abre direcciones ni descarga hasta arreglarlo (falla cerrado, con el porqué).
//! - Una página prohibida a la que se llega por un clic (no por dirección) se detecta después del
//!   comando ([`vigilar`]): GOTV la cierra antes de que el modelo la lea.

use crate::permissions::{Decision, Mode};
use anyhow::{Context, Result};
use gotv_engine::bail_tr;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Mutex, OnceLock};

pub const ARCHIVO: &str = ".gotv/navegador";

#[derive(Serialize, Deserialize, Default, Clone, Debug, PartialEq, Eq)]
pub struct Lista {
    #[serde(default)]
    pub permitir: Vec<String>,
    #[serde(default)]
    pub negar: Vec<String>,
}

impl Lista {
    fn vacia(&self) -> bool {
        self.permitir.is_empty() && self.negar.is_empty()
    }
}

#[derive(Serialize, Deserialize, Default, Clone, Debug, PartialEq, Eq)]
pub struct Sitios {
    #[serde(default)]
    pub abrir: Lista,
    #[serde(default)]
    pub descargar: Lista,
}

impl Sitios {
    /// La lista del proyecto. Sin archivo = vacía. Roto = error (el que llama falla cerrado).
    pub fn leer(root: &Path) -> Result<Sitios> {
        let ruta = root.join(ARCHIVO);
        match std::fs::read_to_string(&ruta) {
            Ok(t) => Sitios::desde_texto(&t).with_context(|| ruta.display().to_string()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Sitios::default()),
            Err(e) => Err(e).with_context(|| ruta.display().to_string()),
        }
    }

    /// El texto que escribe el usuario (regla del Portador: lo nuestro, sólo Rust; nada de JSON): una regla
    /// por línea, `abrir|descargar permitir|negar <dominio o *>`; `#` empieza un comentario. Una línea que no
    /// se entiende es un error: la lista entera no vale (falla cerrado).
    pub fn desde_texto(t: &str) -> Result<Sitios> {
        let mut s = Sitios::default();
        for (n, l) in t.lines().enumerate() {
            let l = l.split('#').next().unwrap_or("").trim();
            if l.is_empty() {
                continue;
            }
            let p: Vec<&str> = l.split_whitespace().collect();
            let lista = match p.first().copied() {
                Some("abrir") => &mut s.abrir,
                Some("descargar") => &mut s.descargar,
                _ => bail_tr!("línea {}: «{l}»: empieza con «abrir» o «descargar»", "line {}: «{l}»: start with «abrir» or «descargar»", n + 1),
            };
            let destino = match p.get(1).copied() {
                Some("permitir") => &mut lista.permitir,
                Some("negar") => &mut lista.negar,
                _ => bail_tr!("línea {}: «{l}»: luego «permitir» o «negar»", "line {}: «{l}»: then «permitir» or «negar»", n + 1),
            };
            if p.len() < 3 {
                bail_tr!("línea {}: «{l}»: falta el dominio (o *)", "line {}: «{l}»: the domain (or *) is missing", n + 1);
            }
            destino.extend(p[2..].iter().map(|x| x.to_string()));
        }
        Ok(s)
    }

    pub fn a_texto(&self) -> String {
        let mut t = format!("# {}\n", crate::i18n::t("Sitios del navegador de GOTV: una regla por línea (abrir|descargar permitir|negar dominio)", "GOTV browser sites: one rule per line (abrir|descargar permitir|negar domain)"));
        for (que, l) in [("abrir", &self.abrir), ("descargar", &self.descargar)] {
            for (como, v) in [("permitir", &l.permitir), ("negar", &l.negar)] {
                for d in v {
                    t.push_str(&format!("{que} {como} {d}\n")); // juez-idioma: datos (las palabras del formato de la lista)
                }
            }
        }
        t
    }

    /// La escribe GOTV (la ventana o las pruebas), siempre generada desde Rust.
    pub fn guardar(&self, root: &Path) -> Result<()> {
        super::escribir_atomico(&root.join(ARCHIVO), &self.a_texto())
    }

    /// ¿Lo prohíbe la lista de ABRIR? (lo único que pesa en una lectura sin nadie a quien preguntar,
    /// como la de `investigar`)
    pub fn abrir_negado(&self, url: &str) -> bool {
        dominio(url).is_some_and(|h| alguna(&self.abrir.negar, &h))
    }
}

/// El dominio de una dirección (`None` para `file:`, `about:`, `data:` y lo que no tenga dominio).
pub fn dominio(url: &str) -> Option<String> {
    let u = url.trim();
    let l = u.to_ascii_lowercase();
    if l.starts_with("file:") || l.starts_with("about:") || l.starts_with("data:") || l.starts_with("javascript:") || l.starts_with("blob:") {
        return None;
    }
    let sin = u.split_once("://").map_or(u, |(_, r)| r);
    let sin = sin.rsplit_once('@').filter(|(a, _)| !a.contains('/')).map_or(sin, |(_, r)| r);
    let host = sin.split(['/', '?', '#']).next().unwrap_or("");
    let host = if host.starts_with('[') { host.split(']').next().map(|h| format!("{h}]")).unwrap_or_default() } else { host.split(':').next().unwrap_or("").to_string() };
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    (!host.is_empty()).then_some(host)
}

/// ¿El patrón casa con el dominio? (`*`, el dominio o uno de sus subdominios)
pub fn casa(patron: &str, host: &str) -> bool {
    let p = patron.trim();
    if p == "*" {
        return true;
    }
    let p = p.strip_prefix("*.").unwrap_or(p);
    let Some(p) = dominio(p) else { return false };
    let host = host.to_ascii_lowercase();
    host == p || host.ends_with(&format!(".{p}"))
}

fn alguna(lista: &[String], host: &str) -> bool {
    lista.iter().any(|p| casa(p, host))
}

fn palabras(args: &[String]) -> Vec<&str> {
    args.iter().map(String::as_str).filter(|a| !a.starts_with('-')).collect()
}

/// Los dominios que este comando del navegador va a ABRIR por dirección.
pub fn dominios_que_abre(args: &[String]) -> Vec<String> {
    let w = palabras(args);
    let mut urls: Vec<&str> = Vec::new();
    match w.as_slice() {
        ["open" | "goto" | "navigate", u, ..] => urls.push(u),
        ["tab", "new", u, ..] => urls.push(u),
        _ => {}
    }
    // cualquier otra dirección completa en el comando (`record start x.webm <url>`, `read <url>`…)
    urls.extend(args.iter().map(String::as_str).filter(|a| a.contains("://")));
    let mut v: Vec<String> = urls.into_iter().filter_map(dominio).collect();
    v.sort();
    v.dedup();
    v
}

/// ¿Es una descarga? (`download <sel> <ruta>` o `wait --download <ruta>`)
pub fn es_descarga(args: &[String]) -> bool {
    let w = palabras(args);
    w.first() == Some(&"download") || (w.first() == Some(&"wait") && args.iter().any(|a| a == "--download" || a == "-d"))
}

fn paginas() -> &'static Mutex<HashMap<String, String>> {
    static M: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
    M.get_or_init(Default::default)
}

/// El dominio de la página abierta en el navegador de este chat, según el último comando vigilado.
pub fn pagina(dueno: &str) -> Option<String> {
    paginas().lock().unwrap_or_else(|e| e.into_inner()).get(dueno).cloned()
}

fn anotar_pagina(dueno: &str, host: Option<String>) {
    let mut m = paginas().lock().unwrap_or_else(|e| e.into_inner());
    match host {
        Some(h) => m.insert(dueno.to_string(), h),
        None => m.remove(dueno),
    };
}

/// La decisión de la lista para un comando de `browser`. `None` = la lista no dice nada (sigue la
/// política de siempre). `pagina_actual` = dominio de la página abierta (para las descargas).
pub fn decidir(root: &Path, args: &[String], modo: Mode, pagina_actual: Option<&str>) -> Option<Decision> {
    let abre = dominios_que_abre(args);
    let descarga = es_descarga(args);
    let s = match Sitios::leer(root) {
        Ok(s) => s,
        Err(e) if !abre.is_empty() || descarga => {
            return Some(Decision::Deny(crate::tr!(
                "la lista de sitios del navegador no se puede leer ({e:#}): hasta que el usuario la arregle, no se abren direcciones ni se descarga",
                "the browser site list cannot be read ({e:#}): until the user fixes it, no addresses are opened and nothing is downloaded"
            )))
        }
        Err(_) => return None,
    };
    if let Some(h) = abre.iter().find(|h| alguna(&s.abrir.negar, h)) {
        return Some(Decision::Deny(crate::tr!(
            "SITIO PROHIBIDO por el usuario ({h}, en {ARCHIVO}): no se abre, ni en bypass ni con permiso. No lo intentes por otra vía.",
            "SITE FORBIDDEN by the user ({h}, in {ARCHIVO}): it is not opened, not in bypass nor with permission. Do not try another way."
        )));
    }
    if descarga {
        if modo == Mode::Plan {
            return Some(Decision::Deny(crate::i18n::t("modo plan: no se descarga", "plan mode: no downloads").into()));
        }
        let h = pagina_actual.unwrap_or("");
        let negada = s.descargar.negar.iter().any(|p| p.trim() == "*") || (!h.is_empty() && alguna(&s.descargar.negar, h));
        if negada {
            return Some(Decision::Deny(crate::tr!(
                "DESCARGA PROHIBIDA por el usuario ({}, en {ARCHIVO}): no se descarga, ni en bypass.",
                "DOWNLOAD FORBIDDEN by the user ({}, in {ARCHIVO}): nothing is downloaded, not in bypass.",
                if h.is_empty() { "?" } else { h }
            )));
        }
        if !h.is_empty() && alguna(&s.descargar.permitir, h) {
            return Some(Decision::Allow);
        }
        let desde = if h.is_empty() { crate::i18n::t("página desconocida", "unknown page").to_string() } else { h.to_string() };
        return Some(Decision::Ask(crate::tr!(
            "⬇ DESCARGA · desde {desde} · browser {}",
            "⬇ DOWNLOAD · from {desde} · browser {}",
            args.join(" ")
        )));
    }
    if !abre.is_empty() && abre.iter().all(|h| alguna(&s.abrir.permitir, h)) {
        return Some(Decision::Allow);
    }
    None
}

/// Comandos tras los que no hace falta mirar a dónde quedó la página.
const NO_NAVEGAN: &[&str] = &["get", "snapshot", "console", "errors", "screenshot", "close", "record", "skills", "a11y", "vitals", "diff", "network", "cookies", "storage", "highlight"];

/// La lista, sólo si dice algo que haya que vigilar en ejecución.
fn vigilada(root: &Path) -> Option<Sitios> {
    Sitios::leer(root).ok().filter(|s| !s.abrir.negar.is_empty() || !s.descargar.vacia())
}

async fn dominio_actual<F, Fut>(ejecutar: &F) -> Option<String>
where
    F: Fn(Vec<String>) -> Fut,
    Fut: std::future::Future<Output = Result<String>>,
{
    ejecutar(vec!["get".into(), "url".into()]).await.ok().and_then(|u| dominio(u.trim()))
}

/// ANTES de un comando de `browser` (sólo con lista): una descarga mira el dominio REAL de la página y
/// se niega si está prohibido. `ejecutar` corre un comando en el navegador de este chat.
pub async fn antes<F, Fut>(dueno: &str, root: &Path, args: &[String], ejecutar: F) -> Result<()>
where
    F: Fn(Vec<String>) -> Fut,
    Fut: std::future::Future<Output = Result<String>>,
{
    let Some(s) = vigilada(root).filter(|_| es_descarga(args)) else { return Ok(()) };
    let h = dominio_actual(&ejecutar).await;
    anotar_pagina(dueno, h.clone());
    match h.filter(|h| alguna(&s.descargar.negar, h)) {
        Some(h) => bail_tr!(
            "DESCARGA PROHIBIDA por el usuario ({h}, en {ARCHIVO}): no se descarga, ni en bypass.",
            "DOWNLOAD FORBIDDEN by the user ({h}, in {ARCHIVO}): nothing is downloaded, not in bypass."
        ),
        None => Ok(()),
    }
}

/// DESPUÉS de un comando de `browser` que pudo navegar (sólo con lista): anota el dominio de la página
/// y, si quedó en un sitio prohibido (se llegó por un clic, no por dirección), la cierra y lo dice.
pub async fn despues<F, Fut>(dueno: &str, root: &Path, args: &[String], ejecutar: F, r: Result<String>) -> Result<String>
where
    F: Fn(Vec<String>) -> Fut,
    Fut: std::future::Future<Output = Result<String>>,
{
    let Some(s) = vigilada(root) else { return r };
    if palabras(args).first().is_some_and(|c| NO_NAVEGAN.contains(c)) {
        return r;
    }
    let h = dominio_actual(&ejecutar).await;
    anotar_pagina(dueno, h.clone());
    match h.filter(|h| alguna(&s.abrir.negar, h)) {
        Some(h) => {
            let _ = ejecutar(vec!["open".into(), "about:blank".into()]).await;
            anotar_pagina(dueno, None);
            bail_tr!(
                "SITIO PROHIBIDO por el usuario ({h}, en {ARCHIVO}): la página llegó ahí y GOTV la cerró sin leerla. No vuelvas a ese sitio por ninguna vía.",
                "SITE FORBIDDEN by the user ({h}, in {ARCHIVO}): the page landed there and GOTV closed it without reading it. Do not go back to that site by any route."
            )
        }
        None => r,
    }
}
