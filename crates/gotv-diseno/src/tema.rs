//! Tema calculado (D2): de una intención corta a todos los tokens de diseño, claro y oscuro.
//!
//! El modelo pequeño sólo dice la intención (`{"marca":"morado","croma":"sobrio"}`); aquí se
//! calcula la paleta en OKLCH con **contraste garantizado sobre el hex final**, la escala
//! tipográfica fluida, el espaciado en rejilla de 4 px, radios, sombras y movimiento.
//! Cada garantía queda escrita en [`Tema::garantias`] y la prueba las recorre todas.

use crate::color::{contrast, solve, Oklch, Rgb8};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Lo que decide el modelo (o el usuario). Todo es opcional salvo la marca.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct Pedido {
    /// `#7c3aed`, `#abc` o un nombre de color (`morado`, `teal`, `gris`…).
    pub marca: String,
    /// `sobrio` · `medio` · `vivo`
    pub croma: String,
    /// `recto` · `suave` · `redondo`
    pub radio: String,
    /// `compacta` · `normal` · `amplia` (razón de la escala tipográfica)
    pub escala: String,
    /// `ninguno` · `sereno` · `vivo`
    pub movimiento: String,
    /// `claro` · `oscuro` (fijo, el otro sólo con `data-tema`) · `auto` (sigue al sistema).
    /// Los dos modos se calculan y garantizan siempre.
    pub modo: String,
    /// par tipográfico de [`crate::recursos::PARES`]: `tecnico`, `editorial`, `lujo`,
    /// `jugueton`, `suizo`, `neon`, `brutalista`, `sobrio`
    pub fuentes: String,
}

impl Default for Pedido {
    fn default() -> Self {
        Pedido {
            marca: "azul".into(),
            croma: "medio".into(),
            radio: "suave".into(),
            escala: "normal".into(),
            movimiento: "sereno".into(),
            modo: "auto".into(),
            fuentes: "tecnico".into(),
        }
    }
}

/// Mínimos de contraste (WCAG 2.2): texto normal AA 4,5 · componentes y foco 3 (1.4.11).
/// Se exige un margen sobre AA porque el antialiasing y los perfiles de pantalla restan.
pub const MIN_TEXTO_FUERTE: f64 = 12.0;
pub const MIN_TEXTO: f64 = 4.6;
pub const MIN_TENUE: f64 = 5.0;
pub const MIN_COMPONENTE: f64 = 3.0;
pub const MIN_BORDE: f64 = 1.25;

/// Tono (grados OKLCH) y croma de un nombre de color, en español o inglés.
fn named_hue(name: &str) -> Option<(f64, f64)> {
    let n = name.trim().to_lowercase();
    let t = [
        (&["rojo", "red", "carmesí", "carmesi", "crimson"][..], 25.0), // juez-idioma: datos (nombres de color que acepta la ficha)
        (&["naranja", "orange"][..], 50.0),
        (&["ámbar", "ambar", "amber", "dorado", "gold"][..], 75.0), // juez-idioma: datos (nombres de color que acepta la ficha)
        (&["amarillo", "yellow"][..], 95.0),
        (&["lima", "lime"][..], 128.0),
        (&["verde", "green"][..], 150.0),
        (&["esmeralda", "emerald"][..], 163.0),
        (&["turquesa", "teal"][..], 185.0),
        (&["cian", "cyan", "celeste"][..], 215.0),
        (&["azul", "blue"][..], 258.0),
        (&["índigo", "indigo", "añil"][..], 277.0), // juez-idioma: datos (nombres de color que acepta la ficha)
        (&["morado", "violeta", "purple", "violet", "lila"][..], 298.0),
        (&["fucsia", "magenta", "fuchsia"][..], 330.0),
        (&["rosa", "pink", "rosado"][..], 355.0),
    ];
    if ["gris", "neutro", "gray", "grey", "negro", "black", "blanco", "white"].contains(&n.as_str()) {
        return Some((260.0, 0.0));
    }
    t.iter().find(|(names, _)| names.contains(&n.as_str())).map(|(_, h)| (*h, -1.0))
}

/// Un modo (claro u oscuro): colores con nombre.
#[derive(Clone, Debug, Serialize)]
pub struct Modo {
    pub colores: BTreeMap<String, String>,
}

/// Una garantía de contraste: `frente` sobre `fondo` debe dar al menos `min`.
#[derive(Clone, Debug, Serialize)]
pub struct Garantia {
    pub modo: &'static str,
    pub frente: String,
    pub fondo: String,
    pub min: f64,
    pub real: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct Tema {
    pub pedido: Pedido,
    /// El color de marca tal cual (uso decorativo: nunca texto sobre fondo sin garantía).
    pub marca: String,
    pub claro: Modo,
    pub oscuro: Modo,
    /// Tokens que no cambian con el modo (tipografía, espacio, radios, movimiento).
    pub fijos: BTreeMap<String, String>,
    pub garantias: Vec<Garantia>,
}

const PASOS: [(&str, f64); 11] = [
    ("50", 0.975),
    ("100", 0.94),
    ("200", 0.885),
    ("300", 0.81),
    ("400", 0.72),
    ("500", 0.63),
    ("600", 0.55),
    ("700", 0.47),
    ("800", 0.39),
    ("900", 0.31),
    ("950", 0.24),
];

/// Croma a lo largo de la escala: máximo en la claridad media, suave en los extremos.
fn bell(l: f64) -> f64 {
    (1.0 - ((l - 0.6) / 0.55).powi(2)).clamp(0.22, 1.0)
}

struct Constructor {
    h: f64,
    c: f64,
    nc: f64,
    marca_l: f64,
}

impl Constructor {
    fn hue_color(&self, l: f64, c: f64, h: f64) -> Rgb8 {
        Oklch::new(l, c, h).to_rgb8()
    }

    /// Resuelve y anota la garantía. Si no hubiera solución (no ocurre con estos mínimos:
    /// lo prueba `cincuenta_temas_al_azar`), cae al extremo blanco/negro.
    #[allow(clippy::too_many_arguments)]
    fn token(
        &self,
        m: &mut BTreeMap<String, Rgb8>,
        g: &mut Vec<(String, Vec<String>, f64)>,
        name: &str,
        (h, c, l_pref): (f64, f64, f64),
        min: f64,
        fondos: &[&str],
        oscuro: bool,
    ) {
        let fs: Vec<Rgb8> = fondos.iter().map(|f| m[*f]).collect();
        let rgb = solve(h, c, l_pref, min, &fs, oscuro)
            .unwrap_or(if oscuro { Rgb8(0, 0, 0) } else { Rgb8(255, 255, 255) });
        m.insert(name.into(), rgb);
        g.push((name.into(), fondos.iter().map(|s| s.to_string()).collect(), min));
    }

    fn modo(&self, oscuro: bool, g: &mut Vec<(String, Vec<String>, f64)>) -> BTreeMap<String, Rgb8> {
        let (h, c, nc) = (self.h, self.c, self.nc);
        let mut m = BTreeMap::new();
        // texto oscuro sobre fondo claro (modo claro) ⇒ se busca hacia lo oscuro
        let hacia_oscuro = !oscuro;
        let (bg, surf, surf2) = if oscuro { (0.17, 0.205, 0.245) } else { (0.985, 1.0, 0.962) };
        m.insert("fondo".into(), self.hue_color(bg, nc * 0.6, h));
        m.insert("superficie".into(), self.hue_color(surf, if oscuro { nc } else { 0.0 }, h));
        m.insert("superficie-2".into(), self.hue_color(surf2, nc, h));
        let suelo = ["fondo", "superficie", "superficie-2"];
        // el acento suave también es suelo de texto (insignias, la tarjeta grande del bento)
        let (ls, cs) = if oscuro { (0.29, c * 0.35) } else { (0.95, c * 0.22) };
        m.insert("acento-suave".into(), self.hue_color(ls, cs, h));
        let lectura = ["fondo", "superficie", "superficie-2", "acento-suave"];

        let (lt, lm, lb, lbs) = if oscuro { (0.96, 0.76, 0.32, 0.55) } else { (0.22, 0.48, 0.9, 0.66) };
        self.token(&mut m, g, "texto", (h, nc * 1.5, lt), MIN_TEXTO_FUERTE, &lectura, hacia_oscuro);
        self.token(&mut m, g, "texto-tenue", (h, nc * 2.0, lm), MIN_TENUE, &lectura, hacia_oscuro);
        self.token(&mut m, g, "borde", (h, nc, lb), MIN_BORDE, &suelo, hacia_oscuro);
        self.token(&mut m, g, "borde-fuerte", (h, nc * 1.5, lbs), MIN_COMPONENTE, &suelo, hacia_oscuro);

        // el acento como texto/enlace, sobre todos los suelos de lectura
        let con_suave = lectura.to_vec();
        let l_texto_acento = if oscuro { self.marca_l.max(0.74) } else { self.marca_l.min(0.55) };
        self.token(&mut m, g, "acento-texto", (h, c, l_texto_acento), MIN_TEXTO, &con_suave, hacia_oscuro);
        self.token(&mut m, g, "foco", (h, c, if oscuro { 0.74 } else { 0.58 }), MIN_COMPONENTE, &suelo, hacia_oscuro);

        // botón principal: texto blanco sobre acento oscurecido, o texto oscuro sobre acento
        // aclarado; gana el que menos aleja al acento de la marca (amarillo ⇒ texto oscuro)
        let blanco = Rgb8(255, 255, 255);
        let tinta = self.hue_color(0.2, nc * 1.5, h);
        let a_blanco = solve(h, c, self.marca_l, MIN_TEXTO, &[blanco], true);
        let a_tinta = solve(h, c, self.marca_l, MIN_TEXTO, &[tinta], false);
        let dist = |x: Option<Rgb8>| x.map(|r| (r.to_oklch().l - self.marca_l).abs()).unwrap_or(f64::MAX);
        let (sobre, acento) = if dist(a_blanco) <= dist(a_tinta) {
            (blanco, a_blanco.unwrap_or(Rgb8(0, 0, 0)))
        } else {
            (tinta, a_tinta.unwrap_or(blanco))
        };
        m.insert("sobre-acento".into(), sobre);
        m.insert("acento".into(), acento);
        // al pasar el ratón el contraste SUBE: se aleja del color del texto que lleva encima
        let ol = acento.to_oklch();
        let hover_l = if sobre == blanco { ol.l - 0.06 } else { ol.l + 0.05 };
        m.insert("acento-hover".into(), Oklch::new(hover_l, ol.c, ol.h).to_rgb8());
        g.push(("sobre-acento".into(), vec!["acento".into(), "acento-hover".into()], MIN_TEXTO));

        // estados: éxito, aviso, peligro, información — texto y fondo suave propios
        for (name, hue) in [("exito", 150.0), ("aviso", 70.0), ("peligro", 27.0), ("info", 245.0)] {
            let suave = format!("{name}-suave");
            let (l, cc) = if oscuro { (0.28, 0.05) } else { (0.955, 0.045) };
            m.insert(suave.clone(), self.hue_color(l, cc, hue));
            let mut fs = suelo.to_vec();
            fs.push(suave.as_str());
            let fs: Vec<String> = fs.iter().map(|s| s.to_string()).collect();
            let fr: Vec<&str> = fs.iter().map(String::as_str).collect();
            self.token(&mut m, g, name, (hue, 0.16, if oscuro { 0.78 } else { 0.5 }), MIN_TEXTO, &fr, hacia_oscuro);
        }

        // escalas completas (para lo decorativo: degradados, ilustraciones, gráficas)
        for (paso, l) in PASOS {
            m.insert(format!("acento-{paso}"), self.hue_color(l, c * bell(l), h)); // juez-idioma: datos (nombre de una variable CSS)
            m.insert(format!("neutro-{paso}"), self.hue_color(l, nc * 1.4, h)); // juez-idioma: datos (nombre de una variable CSS)
        }
        // colores acompañantes para ilustraciones y gráficas (decorativos, nunca texto):
        // análogo (−55°) y complementario (+150°), con el croma del acento (mínimo 0,12
        // para que una marca gris también tenga ilustraciones con color)
        let cd = c.max(0.12);
        for (nombre, dh) in [("analogo", -55.0), ("complemento", 150.0)] {
            for (paso, l) in [("300", 0.81), ("500", 0.66), ("700", 0.5)] {
                m.insert(format!("{nombre}-{paso}"), self.hue_color(l, cd * bell(l), h + dh)); // juez-idioma: datos (nombre de una variable CSS)
            }
        }
        // series de DATOS (gráficas): seis tonos FIJOS en orden FIJO, los mismos con cualquier marca.
        // El color de una serie dice QUIÉN es, no de qué marca es la app; y el orden es lo que los
        // mantiene distinguibles con daltonismo (medido, ver `SERIES`).
        for (i, (claro, osc)) in SERIES.iter().enumerate() {
            m.insert(format!("serie-{}", i + 1), Rgb8::parse_hex(if oscuro { osc } else { claro }).expect("serie")); // juez-idioma: datos (nombre de una variable CSS)
        }
        m
    }
}

/// Los colores de las series de una gráfica: (claro, oscuro) — azul, naranja, aguamarina, amarillo,
/// magenta, verde. Validados como conjunto (banda de luminosidad, croma ≥ 0,10, vecinos separados
/// con protanopia y deuteranopia ΔE ≥ 8, y ≥ 15 con visión normal). Sobre la superficie oscura todos
/// pasan 3:1; sobre la clara, tres no (aguamarina, amarillo, magenta): por eso toda gráfica de GOTV
/// lleva leyenda con texto, rótulo directo y vista de tabla — el color nunca va solo.
/// NO se reordenan ni se añaden tonos: una séptima serie se agrupa en «Otras».
pub const SERIES: [(&str, &str); 6] = [("#2a78d6", "#3987e5"), ("#eb6834", "#d95926"), ("#1baf7a", "#199e70"), ("#eda100", "#c98500"), ("#e87ba4", "#d55181"), ("#008300", "#008300")];

impl Tema {
    /// `Some` si el nombre de color se reconoce (español o inglés).
    pub fn nombre_de_color(n: &str) -> Option<()> {
        named_hue(n).map(|_| ())
    }

    pub fn new(p: Pedido) -> Tema {
        let tope = match p.croma.as_str() {
            "sobrio" => 0.10,
            "vivo" => 0.24,
            _ => 0.16,
        };
        let neutro = match p.croma.as_str() {
            "sobrio" => 0.006,
            "vivo" => 0.016,
            _ => 0.011,
        };
        let (h, c, marca_l, marca_hex) = match Rgb8::parse_hex(&p.marca) {
            Some(rgb) => {
                let o = rgb.to_oklch();
                // un gris de marca no tiñe nada; un color de marca se respeta (hasta el tope vivo)
                (o.h, o.c.min(0.26), o.l, rgb.hex())
            }
            None => {
                let (h, c) = named_hue(&p.marca).unwrap_or((258.0, -1.0));
                let c = if c < 0.0 { tope } else { c };
                let l = 0.6;
                (h, c, l, Oklch::new(l, c, h).to_rgb8().hex())
            }
        };
        let nc = if c < 0.02 { 0.0 } else { neutro };
        let k = Constructor { h, c, nc, marca_l };

        let mut gc = Vec::new();
        let mut go = Vec::new();
        let claro = k.modo(false, &mut gc);
        let oscuro = k.modo(true, &mut go);

        let mut garantias = Vec::new();
        for (modo, m, g) in [("claro", &claro, gc), ("oscuro", &oscuro, go)] {
            for (frente, fondos, min) in g {
                for f in fondos {
                    garantias.push(Garantia { modo, real: contrast(m[&frente], m[&f]), frente: frente.clone(), fondo: f, min });
                }
            }
        }
        let a_hex = |m: BTreeMap<String, Rgb8>| Modo { colores: m.into_iter().map(|(k, v)| (k, v.hex())).collect() };
        Tema { fijos: fijos(&p, h, nc), pedido: p, marca: marca_hex, claro: a_hex(claro), oscuro: a_hex(oscuro), garantias }
    }

    /// Las fuentes que usa el tema (titular, texto y mono, sin repetir).
    pub fn fuentes(&self) -> Vec<&'static crate::recursos::Fuente> {
        let (t, x) = crate::recursos::par(&self.pedido.fuentes).or_else(|| crate::recursos::par("tecnico")).expect("par por defecto");
        let mut v = vec![t];
        if x.slug != t.slug {
            v.push(x);
        }
        v.push(crate::recursos::fuente(crate::recursos::MONO).expect("mono"));
        v
    }

    /// `@font-face` de las fuentes del tema servidas desde `carpeta` (p. ej. `fuentes/`).
    pub fn font_faces(&self, carpeta: &str) -> String {
        self.fuentes().iter().map(|f| crate::recursos::font_face(f, &format!("{carpeta}{}.woff2", f.slug))).collect()
    }

    /// Las garantías que NO se cumplen (vacío = el tema es válido).
    pub fn incumplidas(&self) -> Vec<&Garantia> {
        self.garantias.iter().filter(|g| g.real < g.min).collect()
    }

    /// CSS listo para pegar. Modos:
    /// - `claro` / `oscuro`: ese modo SIEMPRE (la marca lo decidió; un sitio de neón no se
    ///   vuelve claro porque el sistema lo prefiera); el otro sólo con `data-tema` (botón).
    /// - `auto`: sigue a `prefers-color-scheme` (claro si el sistema no dice nada), y
    ///   `data-tema` lo fija.
    pub fn css(&self) -> String {
        let vars = |m: &Modo| natural(&m.colores).into_iter().map(|(k, v)| format!("  --{k}: {v};\n")).collect::<String>();
        let mut s = gotv_engine::i18n::t("/* Tema calculado por GOD OF THE VOID (gotv-diseno). Contraste garantizado sobre el hex. */\n:root {\n", "/* Theme computed by GOD OF THE VOID (gotv-diseno). Contrast guaranteed over the hex. */\n:root {\n").to_string();
        s += &format!("  --marca: {};\n", self.marca);
        for (k, v) in natural(&self.fijos) {
            s += &format!("  --{k}: {v};\n");
        }
        s += "}\n";
        let (def, otro, def_n, otro_n) = if self.pedido.modo == "oscuro" {
            (&self.oscuro, &self.claro, "oscuro", "claro")
        } else {
            (&self.claro, &self.oscuro, "claro", "oscuro")
        };
        let esquema = |n: &str| if n == "oscuro" { "dark" } else { "light" };
        s += &format!(":root, :root[data-tema=\"{def_n}\"] {{\n  color-scheme: {};\n{}}}\n", esquema(def_n), vars(def));
        if self.pedido.modo == "auto" {
            s += &format!(
                "@media (prefers-color-scheme: dark) {{\n  :root:not([data-tema=\"claro\"]) {{\n    color-scheme: dark;\n{}  }}\n}}\n",
                vars(otro).lines().map(|l| format!("  {l}\n")).collect::<String>()
            );
        }
        s += &format!(":root[data-tema=\"{otro_n}\"] {{\n  color-scheme: {};\n{}}}\n", esquema(otro_n), vars(otro));
        s += "@media (prefers-reduced-motion: reduce) {\n  :root { --dur-1: 0ms; --dur-2: 0ms; --dur-3: 0ms; --distancia: 0px; }\n}\n";
        s
    }
}

/// Orden natural para leer el CSS: `e-2` antes que `e-10`, `acento-50` antes que `acento-100`.
fn natural(m: &BTreeMap<String, String>) -> Vec<(&String, &String)> {
    let clave = |k: &str| {
        let (cab, num) = match k.rfind('-') {
            Some(i) if k[i + 1..].chars().all(|c| c.is_ascii_digit()) && i + 1 < k.len() => (&k[..i], k[i + 1..].parse::<u32>().unwrap_or(0)),
            _ => (k, 0),
        };
        (cab.to_string(), num)
    };
    let mut v: Vec<_> = m.iter().collect();
    v.sort_by_key(|(k, _)| clave(k));
    v
}

/// Base64 estándar (para incrustar fuentes en la muestra sin dependencias).
pub(crate) fn base64(b: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::with_capacity(b.len().div_ceil(3) * 4);
    for c in b.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            if i <= c.len() {
                s.push(A[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                s.push('=');
            }
        }
    }
    s
}

fn rem(px: f64) -> String {
    let r = px / 16.0;
    let s = format!("{r:.4}");
    format!("{}rem", s.trim_end_matches('0').trim_end_matches('.'))
}

/// Tipografía fluida, espacio, radios, sombras y movimiento.
fn fijos(p: &Pedido, h: f64, nc: f64) -> BTreeMap<String, String> {
    let mut f = BTreeMap::new();
    // escala modular: en móvil siempre 1,2 (los títulos no revientan a 390 px); en
    // escritorio la razón pedida. Interpolación lineal entre 390 y 1440 px de ancho.
    let razon: f64 = match p.escala.as_str() {
        "compacta" => 1.2,
        "amplia" => 1.333,
        _ => 1.25,
    };
    for n in -2i32..=6 {
        let (min, max) = (16.0 * 1.2f64.powi(n), 16.0 * razon.powi(n));
        let v = if (max - min).abs() < 0.5 {
            rem(max)
        } else {
            let pend = (max - min) / (1440.0 - 390.0);
            let base = min - pend * 390.0;
            format!("clamp({}, {} + {:.3}vw, {})", rem(min), rem(base), pend * 100.0, rem(max))
        };
        let nombre = if n < 0 { format!("t-menos-{}", -n) } else { format!("t-{n}") };
        f.insert(nombre, v);
    }
    for (k, v) in [
        ("interlineado-texto", "1.6"),
        ("interlineado-medio", "1.3"),
        ("interlineado-titulo", "1.1"),
        ("tracking-titulo", "-0.022em"),
        ("tracking-mayus", "0.08em"),
        ("medida", "68ch"),
        ("ancho", "72rem"),
        ("margen", "clamp(1rem, 0.6rem + 2vw, 2rem)"),
        ("seccion", "clamp(4rem, 2.8rem + 5vw, 8rem)"),
    ] {
        f.insert(k.into(), v.into());
    }
    // espacio en rejilla de 4 px
    for (i, px) in [4.0, 8.0, 12.0, 16.0, 20.0, 24.0, 32.0, 40.0, 48.0, 64.0, 80.0, 96.0, 128.0].iter().enumerate() {
        f.insert(format!("e-{}", i + 1), rem(*px));
    }
    let radios = match p.radio.as_str() {
        "recto" => [2.0, 4.0, 6.0],
        "redondo" => [10.0, 16.0, 28.0],
        _ => [6.0, 10.0, 16.0],
    };
    for (k, px) in ["r-1", "r-2", "r-3"].iter().zip(radios) {
        f.insert(k.to_string(), rem(px));
    }
    f.insert("r-pildora".into(), "999px".into());
    // sombras teñidas del tono (las grises puras se ven sucias sobre fondos con color)
    let s = Oklch::new(0.25, nc * 2.0, h).to_rgb8();
    let rgba = |a: f64| format!("rgb({} {} {} / {a})", s.0, s.1, s.2);
    f.insert("sombra-1".into(), format!("0 1px 2px {}, 0 1px 3px {}", rgba(0.06), rgba(0.06)));
    f.insert("sombra-2".into(), format!("0 4px 12px -2px {}, 0 2px 4px -2px {}", rgba(0.10), rgba(0.06)));
    f.insert("sombra-3".into(), format!("0 16px 40px -12px {}, 0 6px 12px -6px {}", rgba(0.18), rgba(0.10)));
    // movimiento
    let (d1, d2, d3, curva) = match p.movimiento.as_str() {
        "ninguno" => (0, 0, 0, "linear"),
        "vivo" => (120, 220, 380, "cubic-bezier(0.34, 1.56, 0.64, 1)"),
        _ => (160, 260, 480, "cubic-bezier(0.22, 1, 0.36, 1)"),
    };
    f.insert("dur-1".into(), format!("{d1}ms"));
    f.insert("dur-2".into(), format!("{d2}ms"));
    f.insert("dur-3".into(), format!("{d3}ms"));
    f.insert("curva".into(), curva.into());
    f.insert("curva-salida".into(), "cubic-bezier(0.22, 1, 0.36, 1)".into());
    f.insert("distancia".into(), if p.movimiento == "ninguno" { "0px".into() } else { "24px".into() });
    // familias: el par tipográfico empaquetado (fuentes OFL dentro del binario)
    let (tit, tex) = crate::recursos::par(&p.fuentes).or_else(|| crate::recursos::par("tecnico")).expect("par por defecto");
    f.insert("fuente-titular".into(), crate::recursos::pila(tit));
    f.insert("fuente-texto".into(), crate::recursos::pila(tex));
    f.insert("fuente-mono".into(), crate::recursos::pila(crate::recursos::fuente(crate::recursos::MONO).expect("mono")));
    f.insert("peso-titular".into(), if p.fuentes == "brutalista" { "800" } else if tit.peso.1 >= 700 && p.fuentes != "lujo" { "650" } else { "500" }.into());
    f
}

/// Los textos visibles de la página de muestra y su versión en inglés (una prueba vigila que
/// cada uno siga en la página).
const MUESTRA_EN: &[(&str, &str)] = &[
    // juez-idioma: bloque — par: (texto de la muestra en español, el mismo en inglés)
    ("<html lang=\"es\">", "<html lang=\"en\">"),
    ("<title>Muestra del tema</title>", "<title>Theme sample</title>"),
    ("<h1>Muestra del tema</h1>", "<h1>Theme sample</h1>"),
    ("\">Marca ", "\">Brand "),
    (" · croma ", " · chroma "),
    (" · modo ", " · mode "),
    (" · fuentes ", " · fonts "),
    (">Botón principal<", ">Main button<"),
    ("<label>Correo<input", "<label>Email<input"),
    ("placeholder=\"tu@correo.com\" value=\"nadie@ejemplo.com\"", "placeholder=\"you@email.com\" value=\"nobody@example.com\""),
    (">Éxito: texto sobre su fondo suave.", ">Success: text on its soft background."),
    (">Aviso: texto sobre su fondo suave.", ">Warning: text on its soft background."),
    (">Peligro: texto sobre su fondo suave.", ">Danger: text on its soft background."),
    (">Información: texto sobre su fondo suave.", ">Info: text on its soft background."),
    ("<h3>Sobre ", "<h3>On "),
    ("<p>Texto principal sobre ", "<p>Main text on "),
    (">Texto tenue sobre ", ">Muted text on "),
    (">Enlace con el acento</a>", ">Link with the accent</a>"),
    (">Insignia</span>", ">Badge</span>"),
    (">Error de validación sobre ", ">Validation error on "),
];

impl Tema {
    /// Página de muestra que usa TODOS los pares garantizados (texto sobre cada fondo,
    /// botones, insignias, avisos, campos). Sirve de vista previa en el visor (D6) y de
    /// prueba externa: axe la audita en un navegador real (`banco --tema-axe`, crates/gotv-cli/examples/banco).
    pub fn muestra(&self) -> String {
        let h = self.muestra_es();
        if gotv_engine::i18n::is_en() {
            MUESTRA_EN.iter().fold(h, |s, (es, en)| s.replace(es, en))
        } else {
            h
        }
    }

    fn muestra_es(&self) -> String {
        let caras: String = self
            .fuentes()
            .iter()
            .map(|f| crate::recursos::font_face(f, &format!("data:font/woff2;base64,{}", base64(f.woff2))))
            .collect();
        let estados = [("exito", "Éxito"), ("aviso", "Aviso"), ("peligro", "Peligro"), ("info", "Información")] // juez-idioma: par (MUESTRA_EN)
            .iter()
            .map(|(e, nombre)| format!(r#"<p class="estado" style="background:var(--{e}-suave);color:var(--{e})">{nombre}: texto sobre su fondo suave.</p>"#))
            .collect::<String>();
        let tarjeta = |fondo: &str| {
            format!(
                r##"<article class="tarjeta" style="background:var(--{fondo})"><h3>Sobre {fondo}</h3><p>Texto principal sobre {fondo}.</p><p class="tenue">Texto tenue sobre {fondo}.</p><p><a href="#">Enlace con el acento</a> · <span class="insignia">Insignia</span></p><p class="aviso-texto" style="color:var(--peligro)">Error de validación sobre {fondo}.</p></article>"## // juez-idioma: par (MUESTRA_EN)
            )
        };
        format!(
            r#"<!doctype html><html lang="es"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>Muestra del tema</title><style>{css}
*{{box-sizing:border-box}} body{{margin:0;background:var(--fondo);color:var(--texto);font-family:var(--fuente-texto);font-size:var(--t-0);line-height:var(--interlineado-texto)}}
main{{max-width:var(--ancho);margin:0 auto;padding:var(--seccion) var(--margen);display:grid;gap:var(--e-8)}}
h1,h3{{font-family:var(--fuente-titular);font-weight:var(--peso-titular)}} h1{{font-size:var(--t-6);line-height:var(--interlineado-titulo);letter-spacing:var(--tracking-titulo);margin:0}}
h3{{margin:0 0 var(--e-2);font-size:var(--t-2)}} .tenue{{color:var(--texto-tenue)}} a{{color:var(--acento-texto)}}
.rejilla{{display:grid;grid-template-columns:repeat(auto-fit,minmax(16rem,1fr));gap:var(--e-6)}}
.tarjeta{{border:1px solid var(--borde);border-radius:var(--r-3);padding:var(--e-6);box-shadow:var(--sombra-2)}}
.insignia{{background:var(--acento-suave);color:var(--acento-texto);border-radius:var(--r-pildora);padding:0 var(--e-2);font-size:var(--t-menos-1)}}
.boton{{background:var(--acento);color:var(--sobre-acento);border:0;border-radius:var(--r-2);padding:var(--e-3) var(--e-6);font:inherit;font-weight:600;cursor:pointer;transition:background var(--dur-1) var(--curva-salida)}}
.boton:hover{{background:var(--acento-hover)}} .boton:focus-visible,input:focus-visible{{outline:3px solid var(--foco);outline-offset:2px}}
label{{display:grid;gap:var(--e-1)}} input{{font:inherit;color:var(--texto);background:var(--superficie);border:1px solid var(--borde-fuerte);border-radius:var(--r-1);padding:var(--e-2) var(--e-3)}}
.estado{{padding:var(--e-3) var(--e-4);border-radius:var(--r-2);margin:0}}
</style></head><body><main><header><h1>Muestra del tema</h1><p class="tenue">Marca {marca} · croma {croma} · modo {modo} · fuentes {fuentes}</p></header>
<section class="rejilla">{t1}{t2}{t3}</section>
<section class="rejilla"><div style="display:flex;gap:var(--e-4);flex-wrap:wrap;align-items:end"><button class="boton">Botón principal</button><label>Correo<input type="email" placeholder="tu@correo.com" value="nadie@ejemplo.com"></label></div><div style="display:grid;gap:var(--e-3)">{estados}</div></section>
</main></body></html>"#,
            css = caras + &self.css(),
            marca = self.marca,
            croma = self.pedido.croma,
            modo = self.pedido.modo,
            fuentes = self.pedido.fuentes,
            t1 = tarjeta("fondo"),
            t2 = tarjeta("superficie"),
            t3 = tarjeta("superficie-2"),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pseudoaleatorio propio (xorshift): reproducible y sin dependencias.
    struct Xs(u64);
    impl Xs {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
        fn pick<'a>(&mut self, xs: &[&'a str]) -> &'a str {
            xs[(self.next() % xs.len() as u64) as usize]
        }
    }

    pub(crate) fn al_azar(semilla: u64, n: usize) -> Vec<Pedido> {
        let mut r = Xs(semilla);
        (0..n)
            .map(|i| {
                // la mitad con hex al azar (incluye grises, amarillos, casi negros), la otra con nombres
                let marca = if i % 2 == 0 {
                    format!("#{:06x}", r.next() % 0x100_0000)
                } else {
                    r.pick(&["morado", "teal", "amarillo", "rojo", "gris", "lima", "azul", "naranja", "rosa", "cian"]).to_string()
                };
                Pedido {
                    marca,
                    croma: r.pick(&["sobrio", "medio", "vivo"]).into(),
                    radio: r.pick(&["recto", "suave", "redondo"]).into(),
                    escala: r.pick(&["compacta", "normal", "amplia"]).into(),
                    movimiento: r.pick(&["ninguno", "sereno", "vivo"]).into(),
                    modo: r.pick(&["claro", "oscuro", "auto"]).into(),
                    fuentes: r.pick(&["tecnico", "editorial", "lujo", "jugueton", "suizo", "neon", "brutalista", "sobrio", "inexistente"]).into(),
                }
            })
            .collect()
    }

    #[test]
    fn cincuenta_temas_al_azar_cumplen_todo_el_contraste() {
        for p in al_azar(0x9e37_79b9_7f4a_7c15, 50) {
            let t = Tema::new(p.clone());
            let malas = t.incumplidas();
            assert!(malas.is_empty(), "{p:?}: {malas:?}");
            assert!(t.garantias.len() > 40, "pocas garantías: {}", t.garantias.len());
        }
    }

    #[test]
    fn la_muestra_del_tema_se_traduce_entera() {
        use gotv_engine::i18n::{set_thread, Lang};
        let t = Tema::new(Pedido::default());
        let es = t.muestra_es();
        for (de, _) in MUESTRA_EN {
            assert!(es.contains(de), "la muestra ya no trae «{de}»");
        }
        set_thread(Some(Lang::En));
        let en = t.muestra();
        set_thread(None);
        for (de, a) in MUESTRA_EN {
            assert!(!en.contains(de) && en.contains(a), "«{de}» → «{a}»");
        }
    }

    #[test]
    fn amarillo_lleva_texto_oscuro_y_morado_texto_blanco() {
        let y = Tema::new(Pedido { marca: "#facc15".into(), ..Default::default() });
        assert_ne!(y.claro.colores["sobre-acento"], "#ffffff", "sobre amarillo va tinta oscura");
        let m = Tema::new(Pedido { marca: "#7c3aed".into(), ..Default::default() });
        assert_eq!(m.claro.colores["sobre-acento"], "#ffffff");
        // el morado de marca ya cumple con blanco: el botón conserva el color exacto
        assert_eq!(m.claro.colores["acento"], "#7c3aed");
    }

    /// Las series de datos no dependen de la marca; en oscuro se ven solas (≥ 3:1 sobre la superficie
    /// de CUALQUIER marca) y en claro al menos el primero, el que pinta toda gráfica de una serie.
    #[test]
    fn las_series_de_datos_son_fijas_y_se_ven() {
        for marca in ["azul", "#facc15", "rojo", "teal", "gris", "#7c3aed", "#84cc16"] {
            let t = Tema::new(Pedido { marca: marca.into(), ..Default::default() });
            for (i, (claro, osc)) in SERIES.iter().enumerate() {
                let k = format!("serie-{}", i + 1);
                assert_eq!((t.claro.colores[&k].as_str(), t.oscuro.colores[&k].as_str()), (*claro, *osc), "{marca}");
                let sup = Rgb8::parse_hex(&t.oscuro.colores["superficie"]).unwrap();
                let c = contrast(Rgb8::parse_hex(osc).unwrap(), sup);
                assert!(c >= 3.0, "{marca}: {k} oscuro {osc} sobre {} = {c:.2}", t.oscuro.colores["superficie"]);
            }
            let sup = Rgb8::parse_hex(&t.claro.colores["superficie"]).unwrap();
            assert!(contrast(Rgb8::parse_hex(SERIES[0].0).unwrap(), sup) >= 3.0, "{marca}");
        }
        assert!(Tema::new(Pedido::default()).css().contains("--serie-6:"));
    }

    #[test]
    fn gris_no_tine_los_neutros() {
        let t = Tema::new(Pedido { marca: "gris".into(), ..Default::default() });
        let n = Rgb8::parse_hex(&t.claro.colores["neutro-500"]).unwrap();
        assert!(n.0.abs_diff(n.1) <= 1 && n.1.abs_diff(n.2) <= 1, "{n:?}");
    }

    #[test]
    fn css_trae_los_dos_modos_y_el_movimiento_reducido() {
        let t = Tema::new(Pedido { marca: "teal".into(), modo: "oscuro".into(), ..Default::default() });
        let css = t.css();
        for k in ["--texto:", "--acento:", "--sobre-acento:", "--t-6:", "--e-13:", "--sombra-3:", "--dur-2:", "--peligro-suave:"] {
            assert!(css.contains(k), "falta {k}");
        }
        // oscuro FIJO: no se vuelve claro porque el sistema lo prefiera
        assert!(!css.contains("prefers-color-scheme"), "un tema oscuro fijo no sigue al sistema");
        assert!(css.contains(":root[data-tema=\"claro\"]"), "el botón de modo sigue pudiendo pasar a claro");
        let auto = Tema::new(Pedido { modo: "auto".into(), ..Default::default() }).css();
        assert!(auto.contains("@media (prefers-color-scheme: dark)") && auto.contains(":root:not([data-tema=\"claro\"])"));
        assert!(css.contains("prefers-reduced-motion: reduce"));
        // títulos fluidos: el t-6 de escritorio es mayor que el de móvil
        assert!(t.fijos["t-6"].starts_with("clamp("), "{}", t.fijos["t-6"]);
    }

    #[test]
    fn base64_de_referencia() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn el_par_tipografico_llega_al_css() {
        let t = Tema::new(Pedido { fuentes: "editorial".into(), ..Default::default() });
        assert!(t.fijos["fuente-titular"].starts_with("\"Fraunces\""), "{}", t.fijos["fuente-titular"]);
        assert!(t.fijos["fuente-texto"].contains("serif"));
        assert!(t.font_faces("fuentes/").contains("url(\"fuentes/newsreader.woff2\")"));
        // par desconocido: cae al técnico, nunca rompe
        let t = Tema::new(Pedido { fuentes: "no-existe".into(), ..Default::default() });
        assert!(t.fijos["fuente-titular"].contains("Space Grotesk"));
    }

    #[test]
    fn el_pedido_llega_como_json_corto() {
        let p: Pedido = serde_json::from_str(r#"{"marca":"morado","croma":"sobrio"}"#).unwrap();
        assert_eq!(p.radio, "suave");
        assert!(Tema::new(p).incumplidas().is_empty());
    }
}
