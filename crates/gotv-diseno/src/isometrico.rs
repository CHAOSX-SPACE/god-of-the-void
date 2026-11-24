//! Isométricas (D6): el modelo DESCRIBE la escena (piezas, posiciones, colores, etiquetas);
//! este motor hace lo que un modelo no hace bien a mano: la proyección, el sombreado de cuatro
//! tonos y el **orden de pintado** (quién tapa a quién).
//!
//! Orden: para cada par de piezas cuyas siluetas se solapan en pantalla se busca el eje que
//! las separa en 3D; la que queda del lado del fondo se pinta antes. Con esas relaciones se
//! hace un orden topológico. (Sumar `x+y+z` del centro, el atajo habitual, falla con piezas
//! altas detrás de piezas planas: lo prueba `torre_detras_de_losa`.)

use crate::color::{Oklch, Rgb8};
use crate::tema::{Pedido, Tema};
use serde::Deserialize;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Escena {
    #[serde(default)]
    pub titulo: String,
    /// píxeles por unidad de la rejilla (por defecto 40)
    #[serde(default)]
    pub escala: Option<f64>,
    /// color de marca para resolver los nombres de token (`acento`, `complemento-500`…)
    #[serde(default)]
    pub marca: Option<String>,
    /// `ninguna` · `construir` (las piezas caen en su orden) · `flotar`
    #[serde(default)]
    pub animacion: Option<String>,
    /// dibuja la rejilla del suelo
    #[serde(default)]
    pub suelo: bool,
    /// dentro de una página: texto y rejilla con `var(--texto)`… para seguir al modo claro/oscuro
    #[serde(skip)]
    pub en_pagina: bool,
    pub piezas: Vec<Pieza>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "tipo", rename_all = "snake_case", deny_unknown_fields)]
pub enum Pieza {
    /// caja alineada a los ejes: esquina (x, y, z) y tamaño (ancho en x, fondo en y, alto en z)
    Caja {
        x: f64,
        y: f64,
        #[serde(default)]
        z: f64,
        ancho: f64,
        fondo: f64,
        alto: f64,
        #[serde(default)]
        color: Option<String>,
        #[serde(default)]
        etiqueta: Option<String>,
        #[serde(default)]
        flota: bool,
    },
    /// cilindro vertical: centro de la base (x, y, z)
    Cilindro {
        x: f64,
        y: f64,
        #[serde(default)]
        z: f64,
        radio: f64,
        alto: f64,
        #[serde(default)]
        color: Option<String>,
        #[serde(default)]
        etiqueta: Option<String>,
        #[serde(default)]
        flota: bool,
    },
    /// flecha entre dos puntos 3D (conexiones, flujos)
    Flecha {
        desde: [f64; 3],
        hasta: [f64; 3],
        #[serde(default)]
        color: Option<String>,
    },
    /// texto suelto anclado a un punto 3D
    Texto {
        x: f64,
        y: f64,
        #[serde(default)]
        z: f64,
        texto: String,
    },
}

const COS30: f64 = 0.866_025_403_784_438_6;

/// Proyección isométrica verdadera (30°): el observador mira desde (+x, +y, +z).
fn proy(x: f64, y: f64, z: f64, s: f64) -> (f64, f64) {
    ((x - y) * COS30 * s, (x + y) * 0.5 * s - z * s)
}

/// Caja envolvente 3D de una pieza (para ordenar).
#[derive(Clone, Copy, Debug)]
struct Caja3 {
    min: [f64; 3],
    max: [f64; 3],
}

impl Pieza {
    fn caja(&self) -> Option<Caja3> {
        match *self {
            Pieza::Caja { x, y, z, ancho, fondo, alto, .. } => Some(Caja3 { min: [x, y, z], max: [x + ancho, y + fondo, z + alto] }),
            Pieza::Cilindro { x, y, z, radio, alto, .. } => Some(Caja3 { min: [x - radio, y - radio, z], max: [x + radio, y + radio, z + alto] }),
            _ => None,
        }
    }
}

fn esquinas(c: &Caja3, s: f64) -> Vec<(f64, f64)> {
    let mut v = Vec::with_capacity(8);
    for &x in &[c.min[0], c.max[0]] {
        for &y in &[c.min[1], c.max[1]] {
            for &z in &[c.min[2], c.max[2]] {
                v.push(proy(x, y, z, s));
            }
        }
    }
    v
}

/// Envolvente convexa (cadena monótona).
fn envolvente(mut p: Vec<(f64, f64)>) -> Vec<(f64, f64)> {
    p.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    p.dedup_by(|a, b| (a.0 - b.0).abs() < 1e-9 && (a.1 - b.1).abs() < 1e-9);
    if p.len() < 3 {
        return p;
    }
    let cruz = |o: (f64, f64), a: (f64, f64), b: (f64, f64)| (a.0 - o.0) * (b.1 - o.1) - (a.1 - o.1) * (b.0 - o.0);
    let mut h: Vec<(f64, f64)> = Vec::new();
    for pass in 0..2 {
        let inicio = h.len();
        let it: Box<dyn Iterator<Item = &(f64, f64)>> = if pass == 0 { Box::new(p.iter()) } else { Box::new(p.iter().rev()) };
        for &q in it {
            while h.len() >= inicio + 2 && cruz(h[h.len() - 2], h[h.len() - 1], q) <= 0.0 {
                h.pop();
            }
            h.push(q);
        }
        h.pop();
    }
    h
}

/// ¿Se solapan dos polígonos convexos? (ejes separadores; tocar el borde no cuenta)
fn solapan(a: &[(f64, f64)], b: &[(f64, f64)]) -> bool {
    for poli in [a, b] {
        for i in 0..poli.len() {
            let (p, q) = (poli[i], poli[(i + 1) % poli.len()]);
            let n = (q.1 - p.1, p.0 - q.0);
            let proyecta = |v: &[(f64, f64)]| v.iter().map(|w| w.0 * n.0 + w.1 * n.1).fold((f64::MAX, f64::MIN), |(lo, hi), d| (lo.min(d), hi.max(d)));
            let (a0, a1) = proyecta(a);
            let (b0, b1) = proyecta(b);
            if a1 <= b0 + 1e-6 || b1 <= a0 + 1e-6 {
                return false;
            }
        }
    }
    true
}

/// `Some(true)` si `a` va DETRÁS de `b` (se pinta antes). Eje separador: en x, y o z, la que
/// queda del lado menor está más lejos del observador (que mira desde +x, +y, +z).
fn detras(a: &Caja3, b: &Caja3) -> Option<bool> {
    for k in [2usize, 0, 1] {
        if a.max[k] <= b.min[k] + 1e-9 {
            return Some(true);
        }
        if b.max[k] <= a.min[k] + 1e-9 {
            return Some(false);
        }
    }
    None // se interpenetran: el llamador cae al orden por profundidad
}

/// Orden de pintado (índices de las piezas sólidas, del fondo al frente).
fn orden(cajas: &[Caja3], s: f64) -> Vec<usize> {
    let n = cajas.len();
    let siluetas: Vec<Vec<(f64, f64)>> = cajas.iter().map(|c| envolvente(esquinas(c, s))).collect();
    let mut antes: Vec<Vec<usize>> = vec![Vec::new(); n]; // antes[j] = las que van antes que j
    let mut grado = vec![0usize; n];
    for i in 0..n {
        for j in (i + 1)..n {
            if !solapan(&siluetas[i], &siluetas[j]) {
                continue;
            }
            match detras(&cajas[i], &cajas[j]) {
                Some(true) => {
                    antes[j].push(i);
                    grado[j] += 1;
                }
                Some(false) => {
                    antes[i].push(j);
                    grado[i] += 1;
                }
                None => {}
            }
        }
    }
    // Kahn; entre las libres, la más profunda primero (estable y natural)
    let prof = |c: &Caja3| (c.min[0] + c.max[0]) + (c.min[1] + c.max[1]) + (c.min[2] + c.max[2]);
    let mut hecho = vec![false; n];
    let mut sal = Vec::with_capacity(n);
    while sal.len() < n {
        let libre = (0..n)
            .filter(|&k| !hecho[k] && grado[k] == 0)
            .min_by(|&a, &b| prof(&cajas[a]).partial_cmp(&prof(&cajas[b])).unwrap_or(std::cmp::Ordering::Equal))
            // un ciclo (piezas que se cruzan): se rompe por profundidad
            .or_else(|| (0..n).filter(|&k| !hecho[k]).min_by(|&a, &b| prof(&cajas[a]).partial_cmp(&prof(&cajas[b])).unwrap_or(std::cmp::Ordering::Equal)));
        let Some(k) = libre else { break };
        hecho[k] = true;
        sal.push(k);
        for j in 0..n {
            if !hecho[j] && antes[j].contains(&k) {
                grado[j] = grado[j].saturating_sub(1);
            }
        }
    }
    sal
}

/// Cuatro tonos de una cara: (arriba, izquierda, derecha, borde).
fn tonos(base: Rgb8) -> [String; 4] {
    let o = base.to_oklch();
    let t = |dl: f64, dc: f64| Oklch::new((o.l + dl).clamp(0.05, 0.97), (o.c * dc).max(0.0), o.h).to_rgb8().hex();
    [t(0.09, 0.85), t(0.0, 1.0), t(-0.1, 1.05), t(-0.3, 1.1)]
}

fn esc(s: &str) -> String {
    crate::pagina::esc(s)
}

impl Escena {
    fn color(&self, tema: &Tema, nombre: &Option<String>, i: usize) -> Rgb8 {
        let por_defecto = ["acento-400", "complemento-300", "neutro-200", "analogo-300", "acento-200", "complemento-500"];
        let n = nombre.clone().unwrap_or_else(|| por_defecto[i % por_defecto.len()].into());
        let n = n.trim().trim_start_matches("var(--").trim_end_matches(')').to_string();
        Rgb8::parse_hex(&n)
            .or_else(|| tema.claro.colores.get(&n).and_then(|h| Rgb8::parse_hex(h)))
            .or_else(|| Tema::nombre_de_color(&n).map(|_| Tema::new(Pedido { marca: n.clone(), ..Default::default() })).and_then(|t| Rgb8::parse_hex(&t.claro.colores["acento-400"])))
            .unwrap_or(Rgb8(160, 160, 170))
    }

    /// SVG autónomo (colores ya resueltos: se puede guardar y abrir en cualquier lado).
    pub fn svg(&self) -> String {
        let s = self.escala.unwrap_or(40.0).clamp(8.0, 200.0);
        let tema = Tema::new(Pedido { marca: self.marca.clone().unwrap_or_else(|| "azul".into()), ..Default::default() });
        let (texto, tenue, rejilla) = if self.en_pagina {
            ("var(--texto)".to_string(), "var(--texto-tenue)".to_string(), "var(--borde-fuerte)".to_string())
        } else {
            (tema.claro.colores["texto"].clone(), tema.claro.colores["texto-tenue"].clone(), tema.claro.colores["borde-fuerte"].clone())
        };
        let solidas: Vec<(usize, Caja3)> = self.piezas.iter().enumerate().filter_map(|(i, p)| p.caja().map(|c| (i, c))).collect();
        let cajas: Vec<Caja3> = solidas.iter().map(|(_, c)| *c).collect();
        let ord = orden(&cajas, s);

        // límites de la vista
        let mut pts: Vec<(f64, f64)> = cajas.iter().flat_map(|c| esquinas(c, s)).collect();
        for p in &self.piezas {
            match p {
                Pieza::Flecha { desde, hasta, .. } => {
                    pts.push(proy(desde[0], desde[1], desde[2], s));
                    pts.push(proy(hasta[0], hasta[1], hasta[2], s));
                }
                Pieza::Texto { x, y, z, .. } => pts.push(proy(*x, *y, *z, s)),
                _ => {}
            }
        }
        if pts.is_empty() {
            pts.push((0.0, 0.0));
        }
        let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
        for (x, y) in &pts {
            x0 = x0.min(*x);
            y0 = y0.min(*y);
            x1 = x1.max(*x);
            y1 = y1.max(*y);
        }
        let m = s * 1.2;
        let (vx, vy, vw, vh) = (x0 - m * 2.0, y0 - m * 1.5, (x1 - x0) + m * 4.0, (y1 - y0) + m * 3.0);
        let f = |v: f64| format!("{v:.1}");
        let pt = |x: f64, y: f64, z: f64| {
            let (a, b) = proy(x, y, z, s);
            format!("{},{}", f(a), f(b))
        };
        let anim = self.animacion.as_deref().unwrap_or("ninguna");
        let mut cuerpo = String::new();
        let mut etiquetas = String::new();

        if self.suelo {
            let (gx0, gy0, gx1, gy1) = cajas.iter().fold((0.0f64, 0.0f64, 4.0f64, 4.0f64), |(a, b, c, d), k| (a.min(k.min[0].floor() - 1.0), b.min(k.min[1].floor() - 1.0), c.max(k.max[0].ceil() + 1.0), d.max(k.max[1].ceil() + 1.0)));
            cuerpo += &format!("<g class=\"iso-suelo\" style=\"stroke:{rejilla}\" stroke-width=\"1\" opacity=\".35\">");
            let mut x = gx0;
            while x <= gx1 + 1e-9 {
                let (a, b) = (proy(x, gy0, 0.0, s), proy(x, gy1, 0.0, s));
                cuerpo += &format!("<line x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\"/>", f(a.0), f(a.1), f(b.0), f(b.1));
                x += 1.0;
            }
            let mut y = gy0;
            while y <= gy1 + 1e-9 {
                let (a, b) = (proy(gx0, y, 0.0, s), proy(gx1, y, 0.0, s));
                cuerpo += &format!("<line x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\"/>", f(a.0), f(a.1), f(b.0), f(b.1));
                y += 1.0;
            }
            cuerpo += "</g>";
        }

        for (paso, &k) in ord.iter().enumerate() {
            let (i, c) = solidas[k];
            let pieza = &self.piezas[i];
            let (color, etiqueta, flota) = match pieza {
                Pieza::Caja { color, etiqueta, flota, .. } | Pieza::Cilindro { color, etiqueta, flota, .. } => (color, etiqueta, *flota),
                _ => continue,
            };
            let [arriba, izq, der, borde] = tonos(self.color(&tema, color, i));
            let clase = match (anim, flota) {
                ("construir", _) => format!(" class=\"iso-cae\" style=\"animation-delay:{}ms\"", paso * 90),
                (_, true) | ("flotar", _) => format!(" class=\"iso-flota\" style=\"animation-delay:{}ms\"", paso * 400),
                _ => String::new(),
            };
            let [ax, ay, az] = c.min;
            let [bx, by, bz] = c.max;
            let trazo = format!("stroke=\"{borde}\" stroke-width=\"1\" stroke-linejoin=\"round\"");
            match pieza {
                Pieza::Caja { .. } => {
                    cuerpo += &format!(
                        "<g{clase}><polygon points=\"{} {} {} {}\" fill=\"{izq}\" {trazo}/><polygon points=\"{} {} {} {}\" fill=\"{der}\" {trazo}/><polygon points=\"{} {} {} {}\" fill=\"{arriba}\" {trazo}/></g>",
                        // cara +y (izquierda en pantalla)
                        pt(ax, by, az), pt(bx, by, az), pt(bx, by, bz), pt(ax, by, bz),
                        // cara +x (derecha en pantalla)
                        pt(bx, ay, az), pt(bx, by, az), pt(bx, by, bz), pt(bx, ay, bz),
                        // tapa
                        pt(ax, ay, bz), pt(bx, ay, bz), pt(bx, by, bz), pt(ax, by, bz),
                    );
                }
                Pieza::Cilindro { x, y, z, radio, alto, .. } => {
                    let (cx, cyb) = proy(*x, *y, *z, s);
                    let (_, cyt) = proy(*x, *y, z + alto, s);
                    // el círculo horizontal de radio r se ve como elipse de semiejes r·√(3/2)·… → en
                    // isométrica verdadera: rx = r·√2·cos30·s, ry = r·√2·0,5·s
                    let rx = radio * std::f64::consts::SQRT_2 * COS30 * s;
                    let ry = radio * std::f64::consts::SQRT_2 * 0.5 * s;
                    let gid = format!("iso-c{i}");
                    cuerpo += &format!(
                        "<g{clase}><defs><linearGradient id=\"{gid}\" x1=\"0\" x2=\"1\"><stop offset=\"0\" stop-color=\"{izq}\"/><stop offset=\"1\" stop-color=\"{der}\"/></linearGradient></defs><path d=\"M{} {} L{} {} A{} {} 0 0 0 {} {} L{} {} A{} {} 0 0 1 {} {}Z\" fill=\"url(#{gid})\" {trazo}/><ellipse cx=\"{}\" cy=\"{}\" rx=\"{}\" ry=\"{}\" fill=\"{arriba}\" {trazo}/></g>",
                        f(cx - rx), f(cyt), f(cx - rx), f(cyb), f(rx), f(ry), f(cx + rx), f(cyb), f(cx + rx), f(cyt), f(rx), f(ry), f(cx - rx), f(cyt),
                        f(cx), f(cyt), f(rx), f(ry)
                    );
                    let _ = (bx, by, bz, ax, ay, az);
                }
                _ => {}
            }
            if let Some(e) = etiqueta {
                // etiqueta sobre la tapa, con una línea guía hacia arriba
                let (tx, ty) = proy((ax + bx) / 2.0, (ay + by) / 2.0, bz, s);
                etiquetas += &format!(
                    "<g class=\"iso-etiqueta\"><line x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\" style=\"stroke:{tenue}\" stroke-width=\"1\"/><circle cx=\"{}\" cy=\"{}\" r=\"2.5\" style=\"fill:{texto}\"/><text x=\"{}\" y=\"{}\" text-anchor=\"middle\" font-size=\"{}\" font-weight=\"600\" style=\"fill:{texto}\">{}</text></g>",
                    f(tx), f(ty), f(tx), f(ty - s * 0.9), f(tx), f(ty), f(tx), f(ty - s * 1.05), f((s * 0.34).max(11.0)), esc(e)
                );
            }
        }
        for p in &self.piezas {
            match p {
                Pieza::Flecha { desde, hasta, color } => {
                    let col = color.as_ref().map(|_| self.color(&tema, color, 0).hex()).unwrap_or_else(|| tema.claro.colores["acento"].clone());
                    let (a, b) = (proy(desde[0], desde[1], desde[2], s), proy(hasta[0], hasta[1], hasta[2], s));
                    let ang = (b.1 - a.1).atan2(b.0 - a.0);
                    let l = s * 0.28;
                    let p1 = (b.0 - l * (ang - 0.45).cos(), b.1 - l * (ang - 0.45).sin());
                    let p2 = (b.0 - l * (ang + 0.45).cos(), b.1 - l * (ang + 0.45).sin());
                    etiquetas += &format!(
                        "<g class=\"iso-flecha\"><line x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\" stroke=\"{col}\" stroke-width=\"2.5\" stroke-linecap=\"round\" stroke-dasharray=\"6 6\"/><polygon points=\"{},{} {},{} {},{}\" fill=\"{col}\"/></g>",
                        f(a.0), f(a.1), f(b.0), f(b.1), f(b.0), f(b.1), f(p1.0), f(p1.1), f(p2.0), f(p2.1)
                    );
                }
                Pieza::Texto { x, y, z, texto: t } => {
                    let (a, b) = proy(*x, *y, *z, s);
                    etiquetas += &format!("<text x=\"{}\" y=\"{}\" text-anchor=\"middle\" font-size=\"{}\" style=\"fill:{tenue}\">{}</text>", f(a), f(b), f((s * 0.3).max(11.0)), esc(t));
                }
                _ => {}
            }
        }
        let estilo = "<style>.iso-cae{animation:iso-cae .6s cubic-bezier(.22,1,.36,1) both}.iso-flota{animation:iso-flota 5s ease-in-out infinite}.iso-flecha line{animation:iso-corre 1.2s linear infinite}@keyframes iso-cae{from{opacity:0;transform:translateY(-24px)}to{opacity:1;transform:none}}@keyframes iso-flota{0%,100%{transform:translateY(0)}50%{transform:translateY(-8px)}}@keyframes iso-corre{to{stroke-dashoffset:-24}}@media (prefers-reduced-motion:reduce){.iso-cae,.iso-flota,.iso-flecha line{animation:none}}</style>";
        // un <svg role="img"> SIN nombre es una falla grave de accesibilidad (axe svg-img-alt; medido
        // D11: la escena del héroe sin «titulo» tumbó la revisión de entrega de un rediseño)
        let nombre = if self.titulo.trim().is_empty() { gotv_engine::tr!("Ilustración", "Illustration") } else { self.titulo.clone() };
        format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"{} {} {} {}\" role=\"img\" aria-label=\"{}\" font-family=\"{}\"><title>{}</title>{estilo}{cuerpo}{etiquetas}</svg>",
            f(vx), f(vy), f(vw), f(vh),
            esc(&nombre), esc(&tema.fijos["fuente-texto"]).replace("&quot;", "'"), esc(&nombre)
        )
    }
}

/// Lee, valida y dibuja. Errores con la ruta exacta, como las fichas de página.
pub fn dibujar(json: &str) -> Result<String, Vec<String>> {
    dibujar_con(json, false)
}

/// Igual que [`dibujar`]; `en_pagina` deja texto y rejilla con las variables del tema.
pub fn dibujar_con(json: &str, en_pagina: bool) -> Result<String, Vec<String>> {
    let v: serde_json::Value = serde_json::from_str(json).map_err(|e| vec![gotv_engine::tr!("el JSON no se puede leer: {e}", "the JSON cannot be read: {e}")])?;
    let mut errores = Vec::new();
    if let Some(ps) = v.get("piezas").and_then(|p| p.as_array()) {
        for (i, p) in ps.iter().enumerate() {
            let tipo = p.get("tipo").and_then(|t| t.as_str()).unwrap_or("?");
            if let Err(e) = serde_json::from_value::<Pieza>(p.clone()) {
                errores.push(format!("piezas[{i}] ({tipo}): {}", e.to_string().split(" at line ").next().unwrap_or("").replace('`', "«")));
            }
            for k in ["ancho", "fondo", "alto", "radio"] {
                if p.get(k).and_then(|x| x.as_f64()).is_some_and(|x| x <= 0.0) {
                    errores.push(gotv_engine::tr!("piezas[{i}] ({tipo}).{k}: debe ser mayor que 0", "piezas[{i}] ({tipo}).{k}: must be greater than 0"));
                }
            }
        }
    } else {
        errores.push(gotv_engine::tr!("falta «piezas»: una lista de cajas, cilindros, flechas o textos", "«piezas» is missing: a list of boxes, cylinders, arrows or texts"));
    }
    if !errores.is_empty() {
        return Err(errores);
    }
    let mut e: Escena = serde_json::from_value(v).map_err(|e| vec![e.to_string()])?;
    e.en_pagina = en_pagina;
    if e.piezas.len() > 400 {
        return Err(vec![gotv_engine::tr!("demasiadas piezas ({}); el máximo es 400", "too many pieces ({}); the maximum is 400", e.piezas.len())]);
    }
    Ok(e.svg())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn caja(x: f64, y: f64, z: f64, w: f64, d: f64, h: f64) -> Caja3 {
        Caja3 { min: [x, y, z], max: [x + w, y + d, z + h] }
    }

    fn pos(o: &[usize], k: usize) -> usize {
        o.iter().position(|&x| x == k).unwrap()
    }

    #[test]
    fn torre_detras_de_losa() {
        // torre alta en x 0..1 y una losa plana justo delante (x 1..3): la torre va DETRÁS.
        // El atajo «suma x+y+z del centro» pinta la losa primero (2,0+0,5+0,1 < 0,5+0,5+3) y
        // la torre la tapa: mal.
        let torre = caja(0.0, 0.0, 0.0, 1.0, 1.0, 6.0);
        let losa = caja(1.0, 0.0, 0.0, 2.0, 1.0, 0.2);
        let suma = |c: &Caja3| (0..3).map(|k| (c.min[k] + c.max[k]) / 2.0).sum::<f64>();
        assert!(suma(&losa) < suma(&torre), "el caso debe engañar al atajo");
        let o = orden(&[torre, losa], 40.0);
        assert!(pos(&o, 0) < pos(&o, 1), "la torre se pinta antes que la losa: {o:?}");
    }

    #[test]
    fn apilado_y_filas() {
        // base y encima; fila de tres de atrás hacia adelante en y
        let base = caja(0.0, 0.0, 0.0, 2.0, 2.0, 1.0);
        let encima = caja(0.5, 0.5, 1.0, 1.0, 1.0, 1.0);
        let o = orden(&[encima, base], 40.0);
        assert!(pos(&o, 1) < pos(&o, 0), "la base antes que lo que tiene encima");
        let fila: Vec<Caja3> = (0..3).rev().map(|i| caja(0.0, i as f64 * 1.5, 0.0, 1.0, 1.0, 1.0)).collect();
        let o = orden(&fila, 40.0);
        assert_eq!(o, vec![2, 1, 0], "de atrás (y menor) hacia adelante");
    }

    #[test]
    fn todo_par_que_se_solapa_respeta_su_eje_separador() {
        // rejilla de 5×5 con alturas variadas: comprobación exhaustiva de cada par
        let mut v = Vec::new();
        for i in 0..5 {
            for j in 0..5 {
                v.push(caja(i as f64 * 1.2, j as f64 * 1.2, 0.0, 1.0, 1.0, 0.5 + ((i * 7 + j * 3) % 5) as f64));
            }
        }
        v.push(caja(1.0, 1.0, 6.0, 3.0, 3.0, 0.3)); // una losa flotando encima de todo
        let o = orden(&v, 40.0);
        let sil: Vec<_> = v.iter().map(|c| envolvente(esquinas(c, 40.0))).collect();
        for a in 0..v.len() {
            for b in 0..v.len() {
                if a != b && solapan(&sil[a], &sil[b]) && detras(&v[a], &v[b]) == Some(true) {
                    assert!(pos(&o, a) < pos(&o, b), "{a} va detrás de {b} pero se pinta después");
                }
            }
        }
    }

    #[test]
    fn la_escena_se_dibuja_y_se_valida() {
        let j = r#"{"titulo":"Arquitectura","marca":"teal","animacion":"construir","suelo":true,"piezas":[
          {"tipo":"caja","x":0,"y":0,"ancho":2,"fondo":2,"alto":1,"color":"acento","etiqueta":"API"},
          {"tipo":"cilindro","x":4,"y":1,"radio":0.8,"alto":1.5,"color":"complemento-300","etiqueta":"Base de datos"},
          {"tipo":"flecha","desde":[2,1,0.5],"hasta":[3.2,1,0.5]},
          {"tipo":"texto","x":1,"y":3,"texto":"<región>"}]}"#;
        let svg = dibujar(j).unwrap();
        assert!(svg.starts_with("<svg xmlns=") && svg.contains("role=\"img\"") && svg.contains("<title>Arquitectura</title>"));
        // sin título, el SVG igual tiene nombre accesible
        let sin = dibujar(r#"{"piezas":[{"tipo":"caja","x":0,"y":0,"ancho":1,"fondo":1,"alto":1}]}"#).unwrap();
        assert!(sin.contains("aria-label=\"Ilustración\"") && sin.contains("<title>Ilustración</title>"), "{}", &sin[..200]);
        assert!(svg.contains("API") && svg.contains("Base de datos") && svg.contains("&lt;región&gt;"));
        assert!(svg.contains("iso-cae") && svg.contains("prefers-reduced-motion"));
        // fuera del <style> (que dice «infinite»), ningún número infinito o NaN
        let sin_estilo = svg.split("</style>").nth(1).unwrap_or(&svg);
        assert!(!sin_estilo.contains("NaN") && !sin_estilo.contains("inf"), "números rotos en el SVG");
        let e = dibujar(r#"{"piezas":[{"tipo":"esfera"},{"tipo":"caja","x":0,"y":0,"ancho":0,"fondo":1,"alto":1}]}"#).unwrap_err();
        let t = e.join(" | ");
        assert!(t.contains("piezas[0] (esfera)") && t.contains("piezas[1] (caja).ancho"), "{t}");
    }
}
