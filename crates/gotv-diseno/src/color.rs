//! Color: OKLCH (perceptual: L = claridad, C = croma, H = tono) ⇄ sRGB de 8 bits.
//!
//! Fórmulas de Björn Ottosson (OKLab, 2020, dominio público) y WCAG 2.2 para la luminancia
//! relativa. **El contraste se mide siempre sobre el hex redondeado**, que es lo que pinta
//! el navegador: una garantía calculada en flotantes podría caer a 4,49 al redondear.

/// Color en OKLCH. `h` en grados.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Oklch {
    pub l: f64,
    pub c: f64,
    pub h: f64,
}

/// Color sRGB de 8 bits por canal (lo que se escribe en CSS).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Rgb8(pub u8, pub u8, pub u8);

fn to_linear(c: f64) -> f64 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn from_linear(c: f64) -> f64 {
    if c <= 0.0031308 {
        12.92 * c
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}

/// OKLCH → sRGB lineal (puede salir de [0,1] si el color no cabe en la gama).
fn oklch_to_linear(o: Oklch) -> [f64; 3] {
    let (a, b) = (o.c * o.h.to_radians().cos(), o.c * o.h.to_radians().sin());
    let l_ = o.l + 0.396_337_777_4 * a + 0.215_803_757_3 * b;
    let m_ = o.l - 0.105_561_345_8 * a - 0.063_854_172_8 * b;
    let s_ = o.l - 0.089_484_177_5 * a - 1.291_485_548_0 * b;
    let (l, m, s) = (l_ * l_ * l_, m_ * m_ * m_, s_ * s_ * s_);
    [
        4.076_741_662_1 * l - 3.307_711_591_3 * m + 0.230_969_929_2 * s,
        -1.268_438_004_6 * l + 2.609_757_401_1 * m - 0.341_319_396_5 * s,
        -0.004_196_086_3 * l - 0.703_418_614_7 * m + 1.707_614_701_0 * s,
    ]
}

fn in_gamut(rgb: [f64; 3]) -> bool {
    rgb.iter().all(|&v| (-1e-6..=1.0 + 1e-6).contains(&v))
}

impl Oklch {
    pub fn new(l: f64, c: f64, h: f64) -> Self {
        Oklch { l: l.clamp(0.0, 1.0), c: c.max(0.0), h: h.rem_euclid(360.0) }
    }

    /// El mismo tono y claridad con el mayor croma ≤ `self.c` que cabe en sRGB
    /// (recorte por croma: conserva tono y claridad, que es lo que el ojo nota).
    pub fn fit(self) -> Self {
        if in_gamut(oklch_to_linear(self)) {
            return self;
        }
        let (mut lo, mut hi) = (0.0, self.c);
        for _ in 0..32 {
            let mid = (lo + hi) / 2.0;
            if in_gamut(oklch_to_linear(Oklch { c: mid, ..self })) {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        Oklch { c: lo, ..self }
    }

    pub fn to_rgb8(self) -> Rgb8 {
        let lin = oklch_to_linear(self.fit());
        let q = |v: f64| (from_linear(v.clamp(0.0, 1.0)) * 255.0).round().clamp(0.0, 255.0) as u8;
        Rgb8(q(lin[0]), q(lin[1]), q(lin[2]))
    }
}

impl Rgb8 {
    pub fn parse_hex(s: &str) -> Option<Rgb8> {
        let h = s.trim().trim_start_matches('#');
        let h: String = match h.len() {
            3 => h.chars().flat_map(|c| [c, c]).collect(),
            6 => h.to_string(),
            _ => return None,
        };
        let v = u32::from_str_radix(&h, 16).ok()?;
        Some(Rgb8((v >> 16) as u8, (v >> 8) as u8, v as u8))
    }

    pub fn hex(self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.0, self.1, self.2)
    }

    fn linear(self) -> [f64; 3] {
        [self.0, self.1, self.2].map(|c| to_linear(c as f64 / 255.0))
    }

    /// Luminancia relativa WCAG 2.2.
    pub fn luminance(self) -> f64 {
        let [r, g, b] = self.linear();
        0.2126 * r + 0.7152 * g + 0.0722 * b
    }

    pub fn to_oklch(self) -> Oklch {
        let [r, g, b] = self.linear();
        let l = (0.412_221_470_8 * r + 0.536_332_536_3 * g + 0.051_445_992_9 * b).cbrt();
        let m = (0.211_903_498_2 * r + 0.680_699_545_1 * g + 0.107_396_956_6 * b).cbrt();
        let s = (0.088_302_461_9 * r + 0.281_718_837_6 * g + 0.629_978_700_5 * b).cbrt();
        let ll = 0.210_454_255_3 * l + 0.793_617_785_0 * m - 0.004_072_046_8 * s;
        let a = 1.977_998_495_1 * l - 2.428_592_205_0 * m + 0.450_593_709_9 * s;
        let bb = 0.025_904_037_1 * l + 0.782_771_766_2 * m - 0.808_675_766_0 * s;
        Oklch::new(ll, (a * a + bb * bb).sqrt(), bb.atan2(a).to_degrees())
    }
}

/// Contraste WCAG entre dos colores (1 a 21).
pub fn contrast(a: Rgb8, b: Rgb8) -> f64 {
    let (x, y) = (a.luminance(), b.luminance());
    let (hi, lo) = if x > y { (x, y) } else { (y, x) };
    (hi + 0.05) / (lo + 0.05)
}

/// El color de tono `h` y croma `c` cuya claridad queda **lo más cerca posible de
/// `l_pref`** cumpliendo `contraste ≥ min` contra TODOS los `fondos`. Busca hacia lo
/// oscuro o hacia lo claro (`oscuro`). `None` si ni el negro/blanco de ese tono llega.
pub fn solve(h: f64, c: f64, l_pref: f64, min: f64, fondos: &[Rgb8], oscuro: bool) -> Option<Rgb8> {
    let ok = |l: f64| {
        let rgb = Oklch::new(l, c, h).to_rgb8();
        fondos.iter().all(|&f| contrast(rgb, f) >= min).then_some(rgb)
    };
    if let Some(rgb) = ok(l_pref) {
        return Some(rgb);
    }
    // extremo: si ni siquiera el extremo cumple, no hay solución en este sentido
    let extremo = if oscuro { 0.0 } else { 1.0 };
    ok(extremo)?;
    // búsqueda binaria entre la preferida (no cumple) y el extremo (cumple)
    let (mut malo, mut bueno) = (l_pref, extremo);
    for _ in 0..40 {
        let mid = (malo + bueno) / 2.0;
        if ok(mid).is_some() {
            bueno = mid;
        } else {
            malo = mid;
        }
    }
    // el redondeo a 8 bits puede no ser monótono: se baja un pelo más si hace falta
    let paso = if oscuro { -0.002 } else { 0.002 };
    let mut l = bueno;
    for _ in 0..50 {
        if let Some(rgb) = ok(l) {
            return Some(rgb);
        }
        l = (l + paso).clamp(0.0, 1.0);
    }
    ok(extremo)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contraste_de_referencia_wcag() {
        let (n, b) = (Rgb8(0, 0, 0), Rgb8(255, 255, 255));
        assert!((contrast(n, b) - 21.0).abs() < 1e-9);
        // #767676 sobre blanco es el gris AA clásico: 4,54
        let g = Rgb8::parse_hex("#767676").unwrap();
        assert!((contrast(g, b) - 4.54).abs() < 0.01, "{}", contrast(g, b));
    }

    #[test]
    fn ida_y_vuelta_hex_oklch() {
        for hx in ["#7c3aed", "#0ea5e9", "#facc15", "#16a34a", "#e11d48", "#111827", "#ffffff", "#000000"] {
            let c = Rgb8::parse_hex(hx).unwrap();
            assert_eq!(c.to_oklch().to_rgb8(), c, "{hx}");
        }
        assert_eq!(Rgb8::parse_hex("#abc"), Rgb8::parse_hex("#aabbcc"));
        assert_eq!(Rgb8::parse_hex("nada"), None);
    }

    #[test]
    fn recorte_a_la_gama_conserva_tono_y_claridad() {
        let fuera = Oklch::new(0.6, 0.5, 145.0); // verde imposible en sRGB
        let dentro = fuera.fit();
        assert!(dentro.c < 0.5 && dentro.c > 0.1);
        let vuelta = dentro.to_rgb8().to_oklch();
        assert!((vuelta.l - 0.6).abs() < 0.01 && (vuelta.h - 145.0).abs() < 2.0, "{vuelta:?}");
    }

    #[test]
    fn resolver_cumple_el_minimo_sobre_el_hex() {
        let blanco = Rgb8(255, 255, 255);
        for h in (0..360).step_by(15) {
            let c = solve(h as f64, 0.2, 0.7, 4.5, &[blanco], true).unwrap();
            assert!(contrast(c, blanco) >= 4.5, "tono {h}: {}", contrast(c, blanco));
        }
        // imposible: 21:1 contra un gris medio
        assert!(solve(0.0, 0.0, 0.5, 21.0, &[Rgb8(128, 128, 128)], true).is_none());
    }
}
