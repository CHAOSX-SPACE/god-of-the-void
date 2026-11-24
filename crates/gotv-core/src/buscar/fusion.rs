//! F23.1 · La fusión: dirección normalizada → sin repetidos → Reciprocal Rank Fusion → (si hay
//! embebedor) reordenado de los 30 primeros contra la consulta. Todo local.

use super::Hallazgo;

/// La constante de RRF (Cormack, Clarke y Büttcher, SIGIR 2009: k = 60).
pub const RRF_K: f64 = 60.0;
/// Cuántos se reordenan con el embebedor.
pub const REORDENAR: usize = 30;

/// La misma página con y sin adornos cuenta una vez: sin `utm_*` (ni otros rastreadores comunes),
/// sin `www.`, `http` → `https`, sin `/` final, sin `#fragmento`, host en minúsculas.
pub fn normalizar_url(url: &str) -> String {
    let u = url.trim();
    let (esquema, resto) = match u.split_once("://") {
        Some((e, r)) => (e.to_ascii_lowercase(), r),
        None => return u.to_string(),
    };
    let esquema = if esquema == "http" { "https".to_string() } else { esquema };
    let resto = resto.split('#').next().unwrap_or("");
    let (antes, consulta) = match resto.split_once('?') {
        Some((a, q)) => (a, Some(q)),
        None => (resto, None),
    };
    let (host, ruta) = match antes.find('/') {
        Some(i) => (&antes[..i], &antes[i..]),
        None => (antes, ""),
    };
    let mut host = host.to_ascii_lowercase();
    if let Some(h) = host.strip_prefix("www.") {
        host = h.to_string();
    }
    if let Some(h) = host.strip_suffix(":443") {
        host = h.to_string();
    }
    let ruta = ruta.trim_end_matches('/');
    let params: Vec<&str> = consulta
        .unwrap_or("")
        .split('&')
        .filter(|p| !p.is_empty())
        .filter(|p| {
            let k = p.split('=').next().unwrap_or("").to_ascii_lowercase();
            !(k.starts_with("utm_") || matches!(k.as_str(), "fbclid" | "gclid" | "mc_cid" | "mc_eid" | "ref_src"))
        })
        .collect();
    let mut s = format!("{esquema}://{host}{ruta}");
    if !params.is_empty() {
        s.push('?');
        s.push_str(&params.join("&"));
    }
    s
}

/// Un hallazgo ya fusionado: de qué servicios vino y su puntaje.
#[derive(Debug, Clone)]
pub struct Fusionado {
    pub titulo: String,
    pub url: String,
    pub resumen: String,
    /// Los servicios que lo devolvieron, en el orden en que aparecieron.
    pub fuentes: Vec<String>,
    pub puntaje: f64,
}

/// RRF sobre las listas de cada servicio (cada `Vec` ya ordenado por su `puesto`). Quita repetidos
/// por la dirección normalizada; se queda con el título y resumen más largos que vio.
pub fn fusionar(listas: &[Vec<Hallazgo>]) -> Vec<Fusionado> {
    let mut orden: Vec<String> = Vec::new();
    let mut por_url: std::collections::HashMap<String, Fusionado> = Default::default();
    for lista in listas {
        let mut vistos_aqui = std::collections::HashSet::new();
        for (i, h) in lista.iter().enumerate() {
            let k = normalizar_url(&h.url);
            if k.is_empty() || !vistos_aqui.insert(k.clone()) {
                continue; // repetido dentro de la misma lista: cuenta una vez
            }
            let puesto = if h.puesto > 0 { h.puesto } else { i + 1 };
            let s = 1.0 / (RRF_K + puesto as f64);
            let f = por_url.entry(k.clone()).or_insert_with(|| {
                orden.push(k.clone());
                Fusionado { titulo: String::new(), url: h.url.clone(), resumen: String::new(), fuentes: Vec::new(), puntaje: 0.0 }
            });
            f.puntaje += s;
            if h.titulo.chars().count() > f.titulo.chars().count() {
                f.titulo = h.titulo.clone();
            }
            if h.resumen.chars().count() > f.resumen.chars().count() {
                f.resumen = h.resumen.clone();
            }
            if !f.fuentes.contains(&h.fuente) {
                f.fuentes.push(h.fuente.clone());
            }
            // la dirección que se muestra: la https si alguna lo era
            if f.url.starts_with("http://") && h.url.starts_with("https://") {
                f.url = h.url.clone();
            }
        }
    }
    let mut v: Vec<Fusionado> = orden.into_iter().filter_map(|k| por_url.remove(&k)).collect();
    // estable: a igual puntaje, el que apareció antes
    v.sort_by(|a, b| b.puntaje.partial_cmp(&a.puntaje).unwrap_or(std::cmp::Ordering::Equal));
    v
}

pub fn coseno(a: &[f32], b: &[f32]) -> f32 {
    let (mut p, mut na, mut nb) = (0f32, 0f32, 0f32);
    for (x, y) in a.iter().zip(b) {
        p += x * y;
        na += x * x;
        nb += y * y;
    }
    if na == 0.0 || nb == 0.0 { 0.0 } else { p / (na.sqrt() * nb.sqrt()) }
}

/// Reordena los `REORDENAR` primeros por su parecido con la consulta (`vectores[0]` = la consulta,
/// luego uno por hallazgo). A igual parecido manda el orden de RRF. El resto queda detrás, igual.
pub fn reordenar(v: &mut [Fusionado], vectores: &[Vec<f32>]) {
    let n = v.len().min(REORDENAR);
    if vectores.len() != n + 1 || n < 2 {
        return;
    }
    let q = &vectores[0];
    let mut idx: Vec<(usize, f32)> = (0..n).map(|i| (i, coseno(q, &vectores[i + 1]))).collect();
    idx.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal).then(a.0.cmp(&b.0)));
    let copia: Vec<Fusionado> = v[..n].to_vec();
    for (dest, (orig, _)) in idx.into_iter().enumerate() {
        v[dest] = copia[orig].clone();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(url: &str, fuente: &str, puesto: usize) -> Hallazgo {
        Hallazgo { titulo: format!("t {url}"), url: url.into(), resumen: String::new(), fuente: fuente.into(), puesto }
    }

    #[test]
    fn la_misma_pagina_con_y_sin_adornos_es_una() {
        let a = normalizar_url("http://www.Ejemplo.com/doc/?utm_source=x&utm_medium=y#parte");
        let b = normalizar_url("https://ejemplo.com/doc");
        assert_eq!(a, b);
        assert_eq!(normalizar_url("https://ejemplo.com/?q=1&utm_campaign=z"), "https://ejemplo.com?q=1");
        assert_ne!(normalizar_url("https://ejemplo.com/a?id=1"), normalizar_url("https://ejemplo.com/a?id=2"));
        assert_eq!(normalizar_url("https://ejemplo.com:443/x/"), "https://ejemplo.com/x");
    }

    #[test]
    fn rrf_premia_el_acuerdo_y_quita_repetidos() {
        let ddg = vec![h("https://a.io/x", "duckduckgo", 1), h("https://b.io", "duckduckgo", 2), h("https://c.io", "duckduckgo", 3)];
        let brave = vec![h("http://www.c.io/?utm_source=brave", "brave", 1), h("https://d.io", "brave", 2)];
        let f = fusionar(&[ddg, brave]);
        assert_eq!(f.len(), 4, "c.io con y sin utm_ cuenta una vez");
        assert_eq!(f[0].url, "https://c.io", "el que dos servicios dan gana");
        assert_eq!(f[0].fuentes, vec!["duckduckgo", "brave"]);
        assert!(f[1].puntaje > f[2].puntaje || (f[1].url == "https://a.io/x"));
    }

    #[test]
    fn reordenar_usa_el_parecido_y_deja_el_resto() {
        let mut v = fusionar(&[vec![h("https://a.io", "x", 1), h("https://b.io", "x", 2), h("https://c.io", "x", 3)]]);
        // consulta ≈ c, luego b, luego a
        let vect = vec![vec![1.0, 0.0], vec![0.0, 1.0], vec![0.5, 0.5], vec![1.0, 0.1]];
        reordenar(&mut v, &vect);
        let urls: Vec<&str> = v.iter().map(|f| f.url.as_str()).collect();
        assert_eq!(urls, ["https://c.io", "https://b.io", "https://a.io"]);
        // con vectores que no cuadran no toca nada
        let antes: Vec<String> = v.iter().map(|f| f.url.clone()).collect();
        reordenar(&mut v, &[vec![1.0]]);
        assert_eq!(antes, v.iter().map(|f| f.url.clone()).collect::<Vec<_>>());
    }
}
