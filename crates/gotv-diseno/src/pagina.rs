//! Páginas por composición (D3 + base de D4): el modelo escribe una **ficha** corta
//! (dirección, contenido, secciones) y aquí se arma la página profesional completa con
//! la biblioteca de componentes (`recursos/gotv.css`), el tema calculado y las
//! ilustraciones generadas. Todo texto se escapa; todo enlace se filtra.

use crate::recursos;
use crate::tema::{Pedido, Tema};
use serde::{Deserialize, Serialize};

pub static CSS_BASE: &str = include_str!("../recursos/gotv.css");

/// Script mínimo: marca `js` (para que sin JS todo se vea) y revela al entrar en pantalla.
const SCRIPT: &str = "document.documentElement.classList.add('js');addEventListener('DOMContentLoaded',()=>{const e=document.querySelectorAll('.revela');if(!('IntersectionObserver'in window)||matchMedia('(prefers-reduced-motion: reduce)').matches){e.forEach(x=>x.classList.add('visto'));return}const o=new IntersectionObserver(s=>{for(const x of s)if(x.isIntersecting){x.target.classList.add('visto');o.unobserve(x.target)}},{rootMargin:'0px 0px -6% 0px'});e.forEach(x=>o.observe(x))});";

// ── direcciones de estilo ──────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, Serialize)]
pub struct Direccion {
    pub id: &'static str,
    pub descripcion: &'static str,
    pub descripcion_en: &'static str,
    /// marca por defecto si la ficha no trae color
    pub marca: &'static str,
    pub croma: &'static str,
    pub radio: &'static str,
    pub escala: &'static str,
    pub movimiento: &'static str,
    pub modo: &'static str,
    pub heroe: &'static str,
    pub caracteristicas: &'static str,
    pub arte: &'static str,
}

pub static DIRECCIONES: &[Direccion] = &[
    Direccion { id: "tecnico", descripcion: "SaaS y producto: héroe dividido con la app dibujada, bento, sombras suaves", descripcion_en: "SaaS and product: split hero with the app drawn, bento, soft shadows", marca: "#4f46e5", croma: "medio", radio: "suave", escala: "normal", movimiento: "sereno", modo: "auto", heroe: "dividido", caracteristicas: "bento", arte: "ventana" }, // juez-idioma: par (descripcion · descripcion_en)
    Direccion { id: "editorial", descripcion: "revista y marca de autor: la tipografía es la imagen, líneas finas, cero sombras", descripcion_en: "magazine and author brand: type is the image, thin rules, zero shadows", marca: "#b4532a", croma: "sobrio", radio: "recto", escala: "amplia", movimiento: "sereno", modo: "claro", heroe: "editorial", caracteristicas: "lista", arte: "ninguno" }, // juez-idioma: par (descripcion · descripcion_en)
    Direccion { id: "lujo", descripcion: "premium: aire, serif fina, mayúsculas espaciadas, líneas de un pelo", descripcion_en: "premium: air, fine serif, spaced capitals, hairline rules", marca: "#8a6a2f", croma: "sobrio", radio: "recto", escala: "amplia", movimiento: "sereno", modo: "claro", heroe: "centrado", caracteristicas: "tarjetas", arte: "arcos" }, // juez-idioma: par (descripcion · descripcion_en)
    Direccion { id: "jugueton", descripcion: "cercano y creativo: redondo, color, orbes, inclinaciones al pasar", descripcion_en: "friendly and creative: round, color, orbs, tilts on hover", marca: "#7c3aed", croma: "vivo", radio: "redondo", escala: "normal", movimiento: "vivo", modo: "claro", heroe: "dividido", caracteristicas: "tarjetas", arte: "orbes" }, // juez-idioma: par (descripcion · descripcion_en)
    Direccion { id: "suizo", descripcion: "minimal y preciso: rejilla con líneas, alineado a la izquierda, geometría", descripcion_en: "minimal and precise: ruled grid, left-aligned, geometry", marca: "#dc2626", croma: "medio", radio: "recto", escala: "normal", movimiento: "sereno", modo: "claro", heroe: "editorial", caracteristicas: "tarjetas", arte: "geometria" }, // juez-idioma: par (descripcion · descripcion_en)
    Direccion { id: "neon", descripcion: "oscuro tecnológico: rejilla luminosa, anillos, bordes que brillan", descripcion_en: "dark tech: glowing grid, rings, shining edges", marca: "#06b6d4", croma: "vivo", radio: "suave", escala: "amplia", movimiento: "sereno", modo: "oscuro", heroe: "inmersivo", caracteristicas: "bento", arte: "anillos" }, // juez-idioma: par (descripcion · descripcion_en)
    Direccion { id: "brutalista", descripcion: "crudo y directo: bordes gruesos, sombras duras, mayúsculas", descripcion_en: "raw and direct: thick borders, hard shadows, capitals", marca: "#facc15", croma: "vivo", radio: "recto", escala: "amplia", movimiento: "vivo", modo: "claro", heroe: "dividido", caracteristicas: "tarjetas", arte: "bloques" }, // juez-idioma: par (descripcion · descripcion_en)
    Direccion { id: "sobrio", descripcion: "institucional: centrado, tarjetas con borde, sin floritura", descripcion_en: "institutional: centered, bordered cards, no flourish", marca: "#1d4ed8", croma: "sobrio", radio: "suave", escala: "compacta", movimiento: "sereno", modo: "claro", heroe: "centrado", caracteristicas: "tarjetas", arte: "ventana" }, // juez-idioma: par (descripcion · descripcion_en)
];

impl Direccion {
    /// La descripción en el idioma activo.
    pub fn texto(&self) -> &'static str {
        gotv_engine::i18n::t(self.descripcion, self.descripcion_en)
    }
}

pub fn direccion(id: &str) -> Option<&'static Direccion> {
    DIRECCIONES.iter().find(|d| d.id == id)
}

// ── la ficha ───────────────────────────────────────────────────────────────

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TemaParcial {
    pub marca: Option<String>,
    pub croma: Option<String>,
    pub radio: Option<String>,
    pub escala: Option<String>,
    pub movimiento: Option<String>,
    pub modo: Option<String>,
    pub fuentes: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Marca {
    pub nombre: String,
    #[serde(default)]
    pub icono: Option<String>,
    /// frase corta para el pie
    #[serde(default)]
    pub lema: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Ficha {
    pub direccion: String,
    #[serde(default)]
    pub tema: TemaParcial,
    #[serde(default = "es")]
    pub idioma: String,
    /// `<title>` y Open Graph
    pub titulo: String,
    #[serde(default)]
    pub descripcion: String,
    pub marca: Marca,
    pub secciones: Vec<Seccion>,
}

fn es() -> String {
    "es".into()
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Enlace {
    pub texto: String,
    #[serde(default = "almohadilla")]
    pub href: String,
}

fn almohadilla() -> String {
    "#".into()
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Item {
    #[serde(default)]
    pub icono: Option<String>,
    pub titulo: String,
    #[serde(default)]
    pub texto: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Cifra {
    pub valor: String,
    pub etiqueta: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Testimonio {
    pub cita: String,
    pub nombre: String,
    #[serde(default)]
    pub cargo: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Plan {
    pub nombre: String,
    pub precio: String,
    #[serde(default)]
    pub periodo: String,
    #[serde(default)]
    pub descripcion: String,
    pub incluye: Vec<String>,
    pub cta: Enlace,
    #[serde(default)]
    pub destacado: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Pregunta {
    pub pregunta: String,
    pub respuesta: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ColumnaPie {
    pub titulo: String,
    pub enlaces: Vec<Enlace>,
}

/// Cabecera común de las secciones de contenido.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Comun {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub etiqueta: Option<String>,
    #[serde(default)]
    pub titulo: Option<String>,
    #[serde(default)]
    pub texto: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "tipo", rename_all = "snake_case", deny_unknown_fields)]
pub enum Seccion {
    Navegacion {
        enlaces: Vec<Enlace>,
        #[serde(default)]
        cta: Option<Enlace>,
    },
    Heroe {
        #[serde(default)]
        variante: Option<String>,
        #[serde(default)]
        etiqueta: Option<String>,
        titulo: String,
        #[serde(default)]
        texto: Option<String>,
        #[serde(default)]
        cta: Option<Enlace>,
        #[serde(default)]
        cta2: Option<Enlace>,
        /// ilustración: nombre (`ventana`, `anillos`, `orbes`, `geometria`, `bloques`, `arcos`,
        /// `barras`, `ninguno`) o una escena isométrica `{"piezas":[…]}` (ver `gotv iso`)
        #[serde(default)]
        arte: Option<serde_json::Value>,
    },
    Logos {
        #[serde(default)]
        titulo: Option<String>,
        nombres: Vec<String>,
    },
    Caracteristicas {
        #[serde(default)]
        variante: Option<String>,
        #[serde(default)]
        id: Option<String>,
        #[serde(default)]
        etiqueta: Option<String>,
        #[serde(default)]
        titulo: Option<String>,
        #[serde(default)]
        texto: Option<String>,
        items: Vec<Item>,
    },
    Cifras {
        #[serde(default)]
        id: Option<String>,
        #[serde(default)]
        etiqueta: Option<String>,
        #[serde(default)]
        titulo: Option<String>,
        #[serde(default)]
        texto: Option<String>,
        items: Vec<Cifra>,
    },
    Pasos {
        #[serde(default)]
        id: Option<String>,
        #[serde(default)]
        etiqueta: Option<String>,
        #[serde(default)]
        titulo: Option<String>,
        #[serde(default)]
        texto: Option<String>,
        items: Vec<Item>,
    },
    Testimonios {
        #[serde(default)]
        id: Option<String>,
        #[serde(default)]
        etiqueta: Option<String>,
        #[serde(default)]
        titulo: Option<String>,
        #[serde(default)]
        texto: Option<String>,
        items: Vec<Testimonio>,
    },
    Precios {
        #[serde(default)]
        id: Option<String>,
        #[serde(default)]
        etiqueta: Option<String>,
        #[serde(default)]
        titulo: Option<String>,
        #[serde(default)]
        texto: Option<String>,
        planes: Vec<Plan>,
    },
    Faq {
        #[serde(default)]
        id: Option<String>,
        #[serde(default)]
        etiqueta: Option<String>,
        #[serde(default)]
        titulo: Option<String>,
        #[serde(default)]
        texto: Option<String>,
        items: Vec<Pregunta>,
    },
    Llamada {
        #[serde(default)]
        id: Option<String>,
        titulo: String,
        #[serde(default)]
        texto: Option<String>,
        cta: Enlace,
        #[serde(default)]
        cta2: Option<Enlace>,
    },
    Contacto {
        #[serde(default)]
        id: Option<String>,
        #[serde(default)]
        etiqueta: Option<String>,
        #[serde(default)]
        titulo: Option<String>,
        #[serde(default)]
        texto: Option<String>,
        /// a dónde se envía (`#` si aún no hay servidor)
        #[serde(default = "almohadilla")]
        accion: String,
        #[serde(default)]
        boton: Option<String>,
    },
    Pie {
        #[serde(default)]
        columnas: Vec<ColumnaPie>,
        #[serde(default)]
        legal: Option<String>,
    },
    /// Vía de escape: lo que la biblioteca no tiene, escrito a mano por el modelo. Usa los
    /// tokens (`var(--acento)`…); sin `<script>`, sin `on…=`, sin `javascript:`. Los jueces
    /// de entrega lo miden igual que al resto.
    Libre {
        #[serde(default)]
        id: Option<String>,
        html: String,
        #[serde(default)]
        css: Option<String>,
    },
}

// ── utilidades seguras ─────────────────────────────────────────────────────

/// Escapa texto para HTML (contenido y atributos).
pub fn esc(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            '\'' => o.push_str("&#39;"),
            c => o.push(c),
        }
    }
    o
}

/// Texto con realce: `*palabra*` → `<em class="realce">palabra</em>` (tras escapar).
pub fn realce(s: &str) -> String {
    let e = esc(s);
    let partes: Vec<&str> = e.split('*').collect();
    if partes.len() < 3 {
        return e;
    }
    let mut o = String::new();
    for (i, p) in partes.iter().enumerate() {
        // un `*` suelto al final (impar) se queda como texto
        if i % 2 == 1 && i + 1 < partes.len() {
            o += &format!("<em class=\"realce\">{p}</em>");
        } else {
            if i % 2 == 1 {
                o.push('*');
            }
            o += p;
        }
    }
    o
}

/// Sólo enlaces seguros: `#ancla`, rutas relativas, http(s), mailto y tel.
pub fn href(s: &str) -> String {
    let t = s.trim();
    let l = t.to_ascii_lowercase();
    let ok = t.starts_with('#')
        || l.starts_with("https://")
        || l.starts_with("http://")
        || l.starts_with("mailto:")
        || l.starts_with("tel:")
        || (!l.contains(':') && !t.is_empty());
    if ok {
        esc(t)
    } else {
        "#".into()
    }
}

/// `id` de ancla: minúsculas, letras, números y guiones.
pub fn ancla(s: &str) -> String {
    let mut o = String::new();
    for c in s.trim().to_lowercase().chars() {
        let c = match c {
            'á' | 'à' | 'ä' => 'a',
            'é' | 'è' | 'ë' => 'e',
            'í' | 'ì' | 'ï' => 'i',
            'ó' | 'ò' | 'ö' => 'o',
            'ú' | 'ù' | 'ü' => 'u',
            'ñ' => 'n',
            c => c,
        };
        if c.is_ascii_alphanumeric() {
            o.push(c);
        } else if !o.ends_with('-') && !o.is_empty() {
            o.push('-');
        }
    }
    o.trim_end_matches('-').to_string()
}

fn ico(nombre: &str) -> String {
    recursos::icono(nombre, None).unwrap_or_default()
}

fn id_attr(id: &Option<String>) -> String {
    match id.as_deref().map(ancla).filter(|s| !s.is_empty()) {
        Some(i) => format!(" id=\"{i}\""),
        None => String::new(),
    }
}

fn cabeza(c: &Comun, centro: bool) -> String {
    if c.titulo.is_none() && c.texto.is_none() && c.etiqueta.is_none() {
        return String::new();
    }
    let mut s = format!("<div class=\"cabeza{} revela\">", if centro { " cabeza--centro" } else { "" });
    if let Some(e) = &c.etiqueta {
        s += &format!("<p class=\"etiqueta\">{}</p>", esc(e));
    }
    if let Some(t) = &c.titulo {
        s += &format!("<h2>{}</h2>", realce(t));
    }
    if let Some(t) = &c.texto {
        s += &format!("<p class=\"entrada\">{}</p>", realce(t));
    }
    s + "</div>"
}

fn boton(e: &Enlace, primario: bool) -> String {
    format!(
        "<a class=\"boton boton--{}\" href=\"{}\">{}{}</a>",
        if primario { "primario" } else { "secundario" },
        href(&e.href),
        esc(&e.texto),
        if primario { ico("arrow-right") } else { String::new() }
    )
}

fn botones(a: &Option<Enlace>, b: &Option<Enlace>) -> String {
    if a.is_none() && b.is_none() {
        return String::new();
    }
    let mut s = String::from("<div class=\"botones\">");
    if let Some(e) = a {
        s += &boton(e, true);
    }
    if let Some(e) = b {
        s += &boton(e, false);
    }
    s + "</div>"
}

fn iniciales(nombre: &str) -> String {
    nombre.split_whitespace().filter_map(|p| p.chars().next()).take(2).collect::<String>().to_uppercase()
}

// ── ilustraciones generadas (decorativas, con los colores del tema) ────────

/// SVG decorativo según la dirección. `aria-hidden`: no aporta información.
pub fn arte(tipo: &str) -> String {
    let a = |s: &str| format!("<svg class=\"arte\" viewBox=\"0 0 480 360\" aria-hidden=\"true\" focusable=\"false\">{s}</svg>");
    match tipo {
        "ventana" => a(r#"<defs><linearGradient id="g-area" x1="0" y1="0" x2="0" y2="1"><stop offset="0" style="stop-color:var(--acento-400);stop-opacity:.45"/><stop offset="1" style="stop-color:var(--acento-400);stop-opacity:0"/></linearGradient><filter id="g-sombra" x="-20%" y="-20%" width="140%" height="160%"><feDropShadow dx="0" dy="14" stdDeviation="16" flood-opacity=".18"/></filter></defs>
<g filter="url(#g-sombra)"><rect x="10" y="14" width="430" height="300" rx="16" style="fill:var(--superficie);stroke:var(--borde)"/></g>
<rect x="10" y="14" width="430" height="38" rx="16" style="fill:var(--superficie-2)"/><rect x="10" y="38" width="430" height="14" style="fill:var(--superficie-2)"/>
<circle cx="32" cy="33" r="5" style="fill:var(--peligro)"/><circle cx="50" cy="33" r="5" style="fill:var(--aviso)"/><circle cx="68" cy="33" r="5" style="fill:var(--exito)"/>
<rect x="24" y="66" width="86" height="234" rx="10" style="fill:var(--superficie-2)"/>
<rect x="36" y="82" width="58" height="9" rx="4.5" style="fill:var(--acento)"/><rect x="36" y="104" width="50" height="8" rx="4" style="fill:var(--neutro-300)"/><rect x="36" y="124" width="62" height="8" rx="4" style="fill:var(--neutro-300)"/><rect x="36" y="144" width="44" height="8" rx="4" style="fill:var(--neutro-300)"/>
<rect x="124" y="66" width="96" height="62" rx="10" style="fill:var(--superficie);stroke:var(--borde)"/><rect x="136" y="80" width="40" height="7" rx="3.5" style="fill:var(--neutro-300)"/><rect x="136" y="98" width="64" height="16" rx="5" style="fill:var(--texto)"/>
<rect x="230" y="66" width="96" height="62" rx="10" style="fill:var(--superficie);stroke:var(--borde)"/><rect x="242" y="80" width="40" height="7" rx="3.5" style="fill:var(--neutro-300)"/><rect x="242" y="98" width="52" height="16" rx="5" style="fill:var(--acento)"/>
<rect x="336" y="66" width="92" height="62" rx="10" style="fill:var(--superficie);stroke:var(--borde)"/><rect x="348" y="80" width="40" height="7" rx="3.5" style="fill:var(--neutro-300)"/><rect x="348" y="98" width="58" height="16" rx="5" style="fill:var(--texto)"/>
<rect x="124" y="140" width="304" height="160" rx="10" style="fill:var(--superficie);stroke:var(--borde)"/>
<path d="M138 270 C170 250 190 262 214 236 S262 200 290 214 S338 172 364 178 S404 150 416 152 L416 288 L138 288Z" style="fill:url(#g-area)"/>
<path d="M138 270 C170 250 190 262 214 236 S262 200 290 214 S338 172 364 178 S404 150 416 152" style="fill:none;stroke:var(--acento);stroke-width:3;stroke-linecap:round"/>
<circle cx="364" cy="178" r="6" style="fill:var(--superficie);stroke:var(--acento);stroke-width:3"/>
<g class="arte-flota"><rect x="300" y="236" width="170" height="98" rx="14" style="fill:var(--superficie);stroke:var(--borde)" filter="url(#g-sombra)"/><circle cx="328" cy="268" r="14" style="fill:var(--acento-suave)"/><path d="M321 268l5 5 9-10" style="fill:none;stroke:var(--acento-texto);stroke-width:2.5;stroke-linecap:round;stroke-linejoin:round"/><rect x="352" y="258" width="92" height="8" rx="4" style="fill:var(--texto)"/><rect x="352" y="274" width="64" height="7" rx="3.5" style="fill:var(--neutro-300)"/><rect x="318" y="302" width="134" height="12" rx="6" style="fill:var(--acento-suave)"/><rect x="318" y="302" width="92" height="12" rx="6" style="fill:var(--acento)"/></g>"#),
        "anillos" => a(r#"<defs><radialGradient id="g-nucleo"><stop offset="0" style="stop-color:var(--acento-300)"/><stop offset=".55" style="stop-color:var(--acento)"/><stop offset="1" style="stop-color:var(--acento);stop-opacity:0"/></radialGradient></defs>
<circle cx="240" cy="180" r="150" style="fill:none;stroke:var(--borde-fuerte)"/>
<g class="arte-gira"><circle cx="240" cy="180" r="118" style="fill:none;stroke:var(--acento-texto);stroke-width:1.5;stroke-dasharray:4 10;opacity:.8"/><circle cx="358" cy="180" r="6" style="fill:var(--acento-texto)"/></g>
<circle cx="240" cy="180" r="84" style="fill:none;stroke:var(--acento);stroke-width:2;opacity:.9"/>
<circle cx="240" cy="180" r="50" style="fill:url(#g-nucleo)"/>
<circle cx="240" cy="180" r="18" style="fill:var(--texto)"/>
<circle cx="171" cy="132" r="4" style="fill:var(--acento-texto)"/><circle cx="318" cy="250" r="3" style="fill:var(--texto-tenue)"/><circle cx="120" cy="240" r="3" style="fill:var(--texto-tenue)"/><circle cx="364" cy="96" r="4" style="fill:var(--acento-texto)"/>"#),
        "orbes" => a(r#"<g class="arte-flota"><path d="M168 64c52-22 120 4 136 58 16 52-18 104-74 116-58 12-118-18-126-74-6-44 22-82 64-100z" style="fill:var(--acento-300)"/></g>
<path d="M300 120c40-10 86 16 92 60 6 46-28 82-72 86-46 4-80-28-80-70 0-38 24-66 60-76z" style="fill:var(--complemento-300)"/>
<path d="M120 210c34-16 80-2 92 34 12 38-12 74-50 82-40 8-76-16-80-52-4-28 12-52 38-64z" style="fill:var(--analogo-500)"/>
<circle cx="214" cy="190" r="34" style="fill:var(--acento)"/>
<path d="M372 58l7 17 17 7-17 7-7 17-7-17-17-7 17-7z" style="fill:var(--texto)"/><path d="M92 112l5 12 12 5-12 5-5 12-5-12-12-5 12-5z" style="fill:var(--texto)"/><path d="M402 268l4 10 10 4-10 4-4 10-4-10-10-4 10-4z" style="fill:var(--acento)"/>"#),
        "geometria" => a(r#"<rect x="40" y="20" width="400" height="320" style="fill:none;stroke:var(--borde-fuerte)"/>
<line x1="173" y1="20" x2="173" y2="340" style="stroke:var(--borde-fuerte)"/><line x1="306" y1="20" x2="306" y2="340" style="stroke:var(--borde-fuerte)"/><line x1="40" y1="180" x2="440" y2="180" style="stroke:var(--borde-fuerte)"/>
<circle cx="106" cy="100" r="60" style="fill:var(--acento)"/>
<path d="M173 180 A133 133 0 0 1 306 47 L306 180Z" style="fill:var(--texto)"/>
<rect x="326" y="40" width="94" height="94" style="fill:var(--neutro-200)"/><circle cx="373" cy="87" r="30" style="fill:var(--acento)"/>
<path d="M40 340 L173 207 L173 340Z" style="fill:var(--neutro-300)"/>
<circle cx="240" cy="260" r="58" style="fill:none;stroke:var(--texto);stroke-width:14"/>
<rect x="306" y="180" width="134" height="160" style="fill:var(--texto)"/><circle cx="373" cy="260" r="40" style="fill:var(--acento)"/>"#),
        "bloques" => a(r#"<rect x="70" y="70" width="210" height="150" style="fill:var(--texto)"/><rect x="58" y="58" width="210" height="150" style="fill:var(--acento);stroke:var(--texto);stroke-width:4"/>
<rect x="232" y="160" width="190" height="140" style="fill:var(--texto)"/><rect x="220" y="148" width="190" height="140" style="fill:var(--superficie);stroke:var(--texto);stroke-width:4"/>
<circle cx="315" cy="218" r="40" style="fill:var(--complemento-500);stroke:var(--texto);stroke-width:4"/>
<g class="arte-flota"><path d="M96 252 L160 252 L128 306Z" style="fill:var(--peligro);stroke:var(--texto);stroke-width:4;stroke-linejoin:round"/></g>
<rect x="82" y="90" width="120" height="14" style="fill:var(--texto)"/><rect x="82" y="116" width="80" height="14" style="fill:var(--texto)"/>
<path d="M388 64l10 26 26 10-26 10-10 26-10-26-26-10 26-10z" style="fill:var(--texto)"/>"#),
        "arcos" => a(r#"<g style="fill:none;stroke:var(--texto-tenue)"><path d="M60 330 A180 180 0 0 1 420 330" /><path d="M100 330 A140 140 0 0 1 380 330"/><path d="M140 330 A100 100 0 0 1 340 330"/><path d="M180 330 A60 60 0 0 1 300 330"/></g>
<line x1="20" y1="330" x2="460" y2="330" style="stroke:var(--texto)"/>
<g class="arte-flota"><circle cx="240" cy="150" r="36" style="fill:none;stroke:var(--acento-texto);stroke-width:1.5"/><path d="M240 122 L252 150 L240 178 L228 150Z" style="fill:var(--acento)"/></g>
<line x1="240" y1="40" x2="240" y2="100" style="stroke:var(--texto-tenue)"/>"#),
        "barras" => a(r#"<g style="fill:var(--acento-200)"><rect x="40" y="200" width="44" height="120" rx="6"/><rect x="104" y="160" width="44" height="160" rx="6"/><rect x="168" y="180" width="44" height="140" rx="6"/><rect x="232" y="120" width="44" height="200" rx="6"/></g>
<rect x="296" y="80" width="44" height="240" rx="6" style="fill:var(--acento)"/><rect x="360" y="110" width="44" height="210" rx="6" style="fill:var(--acento-300)"/>
<path d="M62 190 L126 150 L190 168 L254 110 L318 70 L382 98" style="fill:none;stroke:var(--texto);stroke-width:3;stroke-linecap:round;stroke-linejoin:round"/>
<circle cx="318" cy="70" r="7" style="fill:var(--superficie);stroke:var(--texto);stroke-width:3"/>"#),
        _ => String::new(),
    }
}

// ── secciones ──────────────────────────────────────────────────────────────

struct Ctx<'a> {
    dir: &'a Direccion,
    marca: &'a Marca,
    /// el color de marca del tema (para resolver los colores de una escena isométrica)
    marca_color: String,
    /// la PÁGINA va en inglés (`idioma` de la ficha): sus etiquetas de accesibilidad también
    en: bool,
}

fn nav(ctx: &Ctx, enlaces: &[Enlace], cta: &Option<Enlace>) -> String {
    let lis: String = enlaces.iter().map(|e| format!("<li><a href=\"{}\">{}</a></li>", href(&e.href), esc(&e.texto))).collect();
    let icono = ctx.marca.icono.as_deref().map(ico).unwrap_or_default();
    // juez-idioma: datos — etiquetas de la PÁGINA generada, en el idioma de la página
    let (principal, abrir) = if ctx.en { ("Main", "Open menu") } else { ("Principal", "Abrir menú") };
    format!(
        "<header class=\"nav\"><div class=\"contenedor nav__fila\"><a class=\"marca\" href=\"#\">{icono}<span>{}</span></a><nav aria-label=\"{principal}\"><ul class=\"nav__enlaces\">{lis}</ul></nav><div class=\"botones\">{}<details class=\"nav__menu\"><summary aria-label=\"{abrir}\">{}</summary><ul>{lis}</ul></details></div></div></header>",
        esc(&ctx.marca.nombre),
        cta.as_ref().map(|c| format!("<span class=\"nav__cta\">{}</span>", boton(c, true))).unwrap_or_default(),
        ico("menu"),
    )
}

#[allow(clippy::too_many_arguments)]
fn heroe(ctx: &Ctx, variante: &Option<String>, etiqueta: &Option<String>, titulo: &str, texto: &Option<String>, cta: &Option<Enlace>, cta2: &Option<Enlace>, arte_pedido: &Option<serde_json::Value>) -> String {
    let v = variante.as_deref().filter(|v| ["dividido", "centrado", "editorial", "inmersivo"].contains(v)).unwrap_or(ctx.dir.heroe);
    let eti = etiqueta.as_ref().map(|e| format!("<p class=\"etiqueta\">{}</p>", esc(e))).unwrap_or_default();
    let ent = texto.as_ref().map(|t| format!("<p class=\"entrada\">{}</p>", realce(t))).unwrap_or_default();
    let bts = botones(cta, cta2);
    let arte_svg = match arte_pedido {
        Some(serde_json::Value::String(n)) => arte(n),
        Some(v @ serde_json::Value::Object(_)) => {
            // escena isométrica con los colores de la marca de la página
            let mut v = v.clone();
            if v.get("marca").is_none() {
                v["marca"] = serde_json::Value::String(ctx.marca_color.clone());
            }
            crate::isometrico::dibujar_con(&v.to_string(), true)
                .map(|svg| svg.replacen("<svg ", "<svg class=\"arte\" ", 1))
                .unwrap_or_default()
        }
        _ => arte(ctx.dir.arte),
    };
    let arte_html = if arte_svg.is_empty() { String::new() } else { format!("<div class=\"heroe__arte\">{arte_svg}</div>") };
    match v {
        "editorial" => format!(
            "<section class=\"heroe heroe--editorial\"><div class=\"contenedor\"><div class=\"heroe__texto\">{eti}<h1>{}</h1></div><div class=\"heroe__pie revela\">{ent}{bts}</div>{}</div></section>",
            realce(titulo),
            if arte_html.is_empty() { String::new() } else { format!("<div class=\"heroe__arte\" style=\"margin-top:var(--e-10)\">{arte_svg}</div>") }
        ),
        _ => format!(
            "<section class=\"heroe heroe--{v}\"><div class=\"contenedor\"><div class=\"heroe__texto\">{eti}<h1>{}</h1>{ent}{bts}</div>{arte_html}</div></section>",
            realce(titulo)
        ),
    }
}

fn tarjeta_item(it: &Item, i: usize, arte_extra: &str) -> String {
    let icono = it.icono.as_deref().and_then(|n| recursos::icono(n, None)).map(|s| format!("<span class=\"icono\">{s}</span>")).unwrap_or_default();
    format!(
        "<article class=\"tarjeta revela\" style=\"--i:{}\">{icono}<h3>{}</h3>{}{arte_extra}</article>",
        i % 6,
        realce(&it.titulo),
        if it.texto.is_empty() { String::new() } else { format!("<p>{}</p>", realce(&it.texto)) }
    )
}

fn seccion(clase: &str, id: &Option<String>, dentro: String) -> String {
    format!("<section class=\"seccion {clase}\"{}><div class=\"contenedor\">{dentro}</div></section>", id_attr(id))
}

fn render(ctx: &Ctx, s: &Seccion, alterna: bool) -> String {
    let fondo = if alterna { "seccion--alterna" } else { "" };
    match s {
        Seccion::Navegacion { .. } | Seccion::Pie { .. } => String::new(), // van en su lugar fijo
        // ya validado por `validar`; si llega sin validar y trae algo prohibido, no se pinta
        Seccion::Libre { id, html, css } => match libre_prohibido(html).or_else(|| css.as_deref().and_then(libre_prohibido)) {
            Some(_) => String::new(),
            None => format!(
                "{}{}",
                css.as_ref().map(|c| format!("<style>{}</style>", c.replace("</", "<\\/"))).unwrap_or_default(),
                seccion(&format!("libre {fondo}"), id, html.clone())
            ),
        },
        Seccion::Heroe { variante, etiqueta, titulo, texto, cta, cta2, arte } => heroe(ctx, variante, etiqueta, titulo, texto, cta, cta2, arte),
        Seccion::Logos { titulo, nombres } => format!(
            "<section class=\"logos\"><div class=\"contenedor revela\">{}<ul>{}</ul></div></section>",
            titulo.as_ref().map(|t| format!("<p>{}</p>", esc(t))).unwrap_or_default(),
            nombres.iter().map(|n| format!("<li>{}</li>", esc(n))).collect::<String>()
        ),
        Seccion::Caracteristicas { variante, id, etiqueta, titulo, texto, items } => {
            let c = Comun { id: id.clone(), etiqueta: etiqueta.clone(), titulo: titulo.clone(), texto: texto.clone() };
            let v = variante.as_deref().filter(|v| ["tarjetas", "bento", "lista"].contains(v)).unwrap_or(ctx.dir.caracteristicas);
            let dentro = match v {
                "lista" => format!(
                    "<div class=\"lista-larga\">{}<ol>{}</ol></div>",
                    cabeza(&c, false),
                    items.iter().map(|it| format!("<li class=\"revela\"><h3>{}</h3><p>{}</p></li>", realce(&it.titulo), realce(&it.texto))).collect::<String>()
                ),
                "bento" => format!(
                    "{}<div class=\"bento\">{}</div>",
                    cabeza(&c, false),
                    items.iter().enumerate().map(|(i, it)| tarjeta_item(it, i, &if i == 0 { format!("<div class=\"tarjeta__arte\">{}</div>", arte("barras")) } else { String::new() })).collect::<String>()
                ),
                _ => format!(
                    "{}<div class=\"rejilla\">{}</div>",
                    cabeza(&c, ctx.dir.heroe == "centrado"),
                    items.iter().enumerate().map(|(i, it)| tarjeta_item(it, i, "")).collect::<String>()
                ),
            };
            seccion(fondo, id, dentro)
        }
        Seccion::Cifras { id, etiqueta, titulo, texto, items } => {
            let c = Comun { id: id.clone(), etiqueta: etiqueta.clone(), titulo: titulo.clone(), texto: texto.clone() };
            seccion(fondo, id, format!(
                "{}<div class=\"cifras\">{}</div>",
                cabeza(&c, false),
                items.iter().enumerate().map(|(i, x)| format!("<p class=\"cifra revela\" style=\"--i:{i}\"><strong>{}</strong><span>{}</span></p>", esc(&x.valor), esc(&x.etiqueta))).collect::<String>()
            ))
        }
        Seccion::Pasos { id, etiqueta, titulo, texto, items } => {
            let c = Comun { id: id.clone(), etiqueta: etiqueta.clone(), titulo: titulo.clone(), texto: texto.clone() };
            seccion(fondo, id, format!(
                "{}<ol class=\"pasos\">{}</ol>",
                cabeza(&c, false),
                items.iter().enumerate().map(|(i, it)| format!("<li class=\"revela\" style=\"--i:{i}\"><h3>{}</h3><p>{}</p></li>", realce(&it.titulo), realce(&it.texto))).collect::<String>()
            ))
        }
        Seccion::Testimonios { id, etiqueta, titulo, texto, items } => {
            let c = Comun { id: id.clone(), etiqueta: etiqueta.clone(), titulo: titulo.clone(), texto: texto.clone() };
            seccion(fondo, id, format!(
                "{}<div class=\"rejilla\">{}</div>",
                cabeza(&c, true),
                items.iter().enumerate().map(|(i, t)| format!(
                    "<figure class=\"tarjeta cita revela\" style=\"--i:{i}\"><blockquote><p>{}</p></blockquote><figcaption><span class=\"avatar\" aria-hidden=\"true\">{}</span><span><strong>{}</strong><small>{}</small></span></figcaption></figure>",
                    esc(&t.cita), esc(&iniciales(&t.nombre)), esc(&t.nombre), esc(&t.cargo)
                )).collect::<String>()
            ))
        }
        Seccion::Precios { id, etiqueta, titulo, texto, planes } => {
            let c = Comun { id: id.clone(), etiqueta: etiqueta.clone(), titulo: titulo.clone(), texto: texto.clone() };
            seccion(fondo, id, format!(
                "{}<div class=\"planes\">{}</div>",
                cabeza(&c, true),
                planes.iter().enumerate().map(|(i, p)| format!(
                    "<article class=\"tarjeta plan{} revela\" style=\"--i:{i}\">{}<h3>{}</h3>{}<p class=\"plan__precio\"><strong>{}</strong><span>{}</span></p><ul>{}</ul>{}</article>",
                    if p.destacado { " plan--destacado" } else { "" },
                    if p.destacado { "<span class=\"plan__sello\">Recomendado</span>" } else { "" },
                    esc(&p.nombre),
                    if p.descripcion.is_empty() { String::new() } else { format!("<p>{}</p>", esc(&p.descripcion)) },
                    esc(&p.precio), esc(&p.periodo),
                    p.incluye.iter().map(|x| format!("<li>{}<span>{}</span></li>", ico("check"), esc(x))).collect::<String>(),
                    boton(&p.cta, p.destacado)
                )).collect::<String>()
            ))
        }
        Seccion::Faq { id, etiqueta, titulo, texto, items } => {
            let c = Comun { id: id.clone(), etiqueta: etiqueta.clone(), titulo: titulo.clone(), texto: texto.clone() };
            seccion(fondo, id, format!(
                "{}<div class=\"faq revela\">{}</div>",
                cabeza(&c, true),
                items.iter().map(|q| format!("<details><summary>{}{}</summary><p>{}</p></details>", esc(&q.pregunta), ico("plus"), realce(&q.respuesta))).collect::<String>()
            ))
        }
        Seccion::Llamada { id, titulo, texto, cta, cta2 } => seccion(
            "",
            id,
            format!(
                "<div class=\"llamada revela\"><h2>{}</h2>{}{}</div>",
                realce(titulo),
                texto.as_ref().map(|t| format!("<p>{}</p>", esc(t))).unwrap_or_default(),
                botones(&Some(cta.clone()), cta2)
            ),
        ),
        Seccion::Contacto { id, etiqueta, titulo, texto, accion, boton: b } => {
            let c = Comun { id: id.clone(), etiqueta: etiqueta.clone(), titulo: titulo.clone(), texto: texto.clone() };
            seccion(fondo, id, format!(
                "<div class=\"lista-larga\">{}<form class=\"formulario revela\" action=\"{}\" method=\"post\"><div class=\"campo\"><label for=\"c-nombre\">Nombre</label><input id=\"c-nombre\" name=\"nombre\" autocomplete=\"name\" required></div><div class=\"campo\"><label for=\"c-correo\">Correo</label><input id=\"c-correo\" name=\"correo\" type=\"email\" autocomplete=\"email\" required aria-describedby=\"c-correo-ayuda\"><small id=\"c-correo-ayuda\">Te respondemos a este correo.</small></div><div class=\"campo\"><label for=\"c-mensaje\">Mensaje</label><textarea id=\"c-mensaje\" name=\"mensaje\" required></textarea></div><button class=\"boton boton--primario\" type=\"submit\">{}{}</button></form></div>",
                cabeza(&c, false),
                href(accion),
                esc(b.as_deref().unwrap_or("Enviar")),
                ico("send")
            ))
        }
    }
}

fn pie(ctx: &Ctx, columnas: &[ColumnaPie], legal: &Option<String>) -> String {
    let icono = ctx.marca.icono.as_deref().map(ico).unwrap_or_default();
    format!(
        "<footer class=\"pie\"><div class=\"contenedor\"><div class=\"pie__rejilla\"><div class=\"pie__marca\"><a class=\"marca\" href=\"#\">{icono}<span>{}</span></a>{}</div>{}</div><p class=\"pie__legal\">{}</p></div></footer>",
        esc(&ctx.marca.nombre),
        ctx.marca.lema.as_ref().map(|l| format!("<p>{}</p>", esc(l))).unwrap_or_default(),
        columnas.iter().map(|c| format!(
            "<div><h3>{}</h3><ul>{}</ul></div>",
            esc(&c.titulo),
            c.enlaces.iter().map(|e| format!("<li><a href=\"{}\">{}</a></li>", href(&e.href), esc(&e.texto))).collect::<String>()
        )).collect::<String>(),
        esc(legal.as_deref().unwrap_or(&format!("© {}", ctx.marca.nombre)))
    )
}

// ── armado ─────────────────────────────────────────────────────────────────

/// Cómo se entrega: un archivo suelto (vista previa) o un proyecto con sus archivos.
pub enum Salida {
    /// todo dentro del HTML (fuentes en base64): para el visor y las muestras
    Suelta,
    /// `index.html` + `assets/estilos.css` + `assets/fuentes/*.woff2` + licencias
    Proyecto,
}

pub struct Pagina {
    pub ficha: Ficha,
    pub tema: Tema,
    pub dir: &'static Direccion,
}

impl Pagina {
    /// Arma la página. Una dirección desconocida es un error con la lista de las válidas.
    pub fn new(ficha: Ficha) -> anyhow::Result<Pagina> {
        let dir = direccion(&ficha.direccion).ok_or_else(|| {
            gotv_engine::anyhow_tr!(
                "direccion: «{}» no existe; elige una de: {}",
                "direccion: «{}» does not exist; pick one of: {}",
                ficha.direccion,
                DIRECCIONES.iter().map(|d| d.id).collect::<Vec<_>>().join(", ")
            )
        })?;
        let t = &ficha.tema;
        let pedido = Pedido {
            marca: t.marca.clone().unwrap_or_else(|| dir.marca.into()),
            croma: t.croma.clone().unwrap_or_else(|| dir.croma.into()),
            radio: t.radio.clone().unwrap_or_else(|| dir.radio.into()),
            escala: t.escala.clone().unwrap_or_else(|| dir.escala.into()),
            movimiento: t.movimiento.clone().unwrap_or_else(|| dir.movimiento.into()),
            modo: t.modo.clone().unwrap_or_else(|| dir.modo.into()),
            fuentes: t.fuentes.clone().unwrap_or_else(|| dir.id.into()),
        };
        Ok(Pagina { tema: Tema::new(pedido), ficha, dir })
    }

    fn cuerpo(&self) -> String {
        let ctx = Ctx { dir: self.dir, marca: &self.ficha.marca, marca_color: self.tema.marca.clone(), en: self.ficha.idioma.trim().to_lowercase().starts_with("en") };
        let mut nav_html = String::new();
        let mut pie_html = String::new();
        let mut main = String::new();
        let mut alterna = false;
        // D6: cada sección lleva su índice en la ficha (`data-gotv`): señalar un elemento en la
        // vista dice QUÉ parte de la ficha cambiar
        for (i, s) in self.ficha.secciones.iter().enumerate() {
            let t = serde_json::to_value(s).ok().and_then(|v| v["tipo"].as_str().map(String::from)).unwrap_or_default();
            match s {
                Seccion::Navegacion { enlaces, cta } => nav_html = marcar(&nav(&ctx, enlaces, cta), i, &t),
                Seccion::Pie { columnas, legal } => pie_html = marcar(&pie(&ctx, columnas, legal), i, &t),
                Seccion::Heroe { .. } | Seccion::Logos { .. } | Seccion::Llamada { .. } => main += &marcar(&render(&ctx, s, false), i, &t),
                _ => {
                    main += &marcar(&render(&ctx, s, alterna), i, &t);
                    alterna = !alterna;
                }
            }
        }
        if pie_html.is_empty() {
            pie_html = pie(&ctx, &[], &None);
        }
        format!("<a class=\"salta\" href=\"#contenido\">Saltar al contenido</a>{nav_html}<main id=\"contenido\">{main}</main>{pie_html}")
    }

    fn cabeza_html(&self, estilos: &str) -> String {
        let f = &self.ficha;
        let modo = if self.tema.pedido.modo == "oscuro" { &self.tema.oscuro } else { &self.tema.claro };
        format!(
            "<!doctype html>\n<html lang=\"{}\" data-direccion=\"{}\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>{}</title><meta name=\"description\" content=\"{}\"><meta property=\"og:title\" content=\"{}\"><meta property=\"og:description\" content=\"{}\"><meta property=\"og:type\" content=\"website\"><meta name=\"theme-color\" content=\"{}\"><meta name=\"generator\" content=\"GOD OF THE VOID\">{estilos}<script>{SCRIPT}</script></head>",
            esc(&f.idioma),
            self.dir.id,
            esc(&f.titulo),
            esc(&f.descripcion),
            esc(&f.titulo),
            esc(&f.descripcion),
            modo.colores["fondo"],
        )
    }

    /// CSS completo: tema + escalonado de la aparición + biblioteca.
    pub fn css(&self) -> String {
        format!("{}\n.js .revela {{ transition-delay: calc(var(--i, 0) * 70ms); }}\n{}", self.tema.css(), CSS_BASE)
    }

    /// Un solo archivo HTML (fuentes incrustadas): vista previa y muestras.
    pub fn html_suelto(&self) -> String {
        let caras: String = self
            .tema
            .fuentes()
            .iter()
            .map(|f| recursos::font_face(f, &format!("data:font/woff2;base64,{}", crate::tema::base64(f.woff2))))
            .collect();
        format!("{}<body>{}</body></html>\n", self.cabeza_html(&format!("<style>{caras}{}</style>", self.css())), self.cuerpo())
    }

    /// Los archivos del proyecto: `(ruta relativa, bytes)`. Incluye `ficha.json`: la fuente de la
    /// página (D6: variantes sin modelo y cambios por sección se hacen sobre ELLA, no sobre el HTML).
    pub fn archivos(&self) -> Vec<(String, Vec<u8>)> {
        let mut v = Vec::new();
        if let Ok(mut f) = serde_json::to_value(&self.ficha) {
            podar(&mut f);
            if let Ok(t) = serde_json::to_string_pretty(&f) {
                v.push(("ficha.json".into(), (t + "\n").into_bytes()));
            }
        }
        let caras = self.tema.font_faces("fuentes/");
        let precarga: String = self
            .tema
            .fuentes()
            .iter()
            .take(2)
            .map(|f| format!("<link rel=\"preload\" href=\"assets/fuentes/{}.woff2\" as=\"font\" type=\"font/woff2\" crossorigin>", f.slug))
            .collect();
        let html = format!(
            "{}<body>{}</body></html>\n",
            self.cabeza_html(&format!("{precarga}<link rel=\"stylesheet\" href=\"assets/estilos.css\">")),
            self.cuerpo()
        );
        v.push(("index.html".into(), html.into_bytes()));
        v.push(("assets/estilos.css".into(), format!("{caras}{}", self.css()).into_bytes()));
        let mut licencias = String::from("Fuentes (SIL Open Font License 1.1), empaquetadas por GOD OF THE VOID:\n\n");
        for f in self.tema.fuentes() {
            v.push((format!("assets/fuentes/{}.woff2", f.slug), f.woff2.to_vec()));
            licencias += &format!("── {} ──\n{}\n\n", f.familia, f.licencia);
        }
        licencias += &format!("── Iconos: Lucide ──\n{}\n", recursos::LICENCIA_ICONOS);
        v.push(("assets/LICENCIAS.txt".into(), licencias.into_bytes()));
        v
    }

    pub fn html(&self, salida: Salida) -> String {
        match salida {
            Salida::Suelta => self.html_suelto(),
            Salida::Proyecto => String::from_utf8(self.archivos().remove(0).1).unwrap_or_default(),
        }
    }
}

// ── validación con ruta exacta (D4) ───────────────────────────────────────

/// Valores válidos de los campos con lista cerrada (para el error y para el esquema).
pub const CROMAS: &[&str] = &["sobrio", "medio", "vivo"];
pub const RADIOS: &[&str] = &["recto", "suave", "redondo"];
pub const ESCALAS: &[&str] = &["compacta", "normal", "amplia"];
pub const MOVIMIENTOS: &[&str] = &["ninguno", "sereno", "vivo"];
pub const MODOS: &[&str] = &["claro", "oscuro", "auto"];
pub const HEROES: &[&str] = &["dividido", "centrado", "editorial", "inmersivo"];
pub const CARACTERISTICAS: &[&str] = &["tarjetas", "bento", "lista"];
pub const ARTES: &[&str] = &["ventana", "anillos", "orbes", "geometria", "bloques", "arcos", "barras", "ninguno"];
pub const TIPOS: &[&str] = &["navegacion", "heroe", "logos", "caracteristicas", "cifras", "pasos", "testimonios", "precios", "faq", "llamada", "contacto", "pie", "libre"];

/// Lo que un bloque libre no puede traer (se rechaza con su motivo; no se «limpia» a escondidas).
fn libre_prohibido(x: &str) -> Option<&'static str> {
    let l = x.to_lowercase();
    if l.contains("<script") {
        return Some(gotv_engine::i18n::t("trae <script> (la página ya trae su script; pide la pieza que falta)", "has <script> (the page already has its script; ask for the missing piece)"));
    }
    if l.contains("javascript:") {
        return Some("trae «javascript:»");
    }
    if l.contains("<iframe") || l.contains("<object") || l.contains("<embed") {
        return Some("trae <iframe>/<object>/<embed>");
    }
    let b = l.as_bytes();
    for i in 0..b.len().saturating_sub(3) {
        // ` onclick=`, ` onload =`… (atributo de evento)
        if (b[i] == b' ' || b[i] == b'\n' || b[i] == b'\t') && b[i + 1] == b'o' && b[i + 2] == b'n' {
            let resto = &l[i + 3..];
            let nombre: String = resto.chars().take_while(|c| c.is_ascii_alphabetic()).collect();
            if !nombre.is_empty() && resto[nombre.len()..].trim_start().starts_with('=') {
                return Some("trae un atributo de evento (on…=)");
            }
        }
    }
    if l.contains("@import") || l.contains("url(http") {
        return Some("trae recursos externos (@import o url(http…)); todo debe ir dentro");
    }
    None
}

/// Traduce un error de serde a una frase corta y accionable en español.
fn traducir(e: &str) -> String {
    let e = e.split(" at line ").next().unwrap_or(e);
    // comillas bien cerradas: `a` → «a»
    let tick = |s: &str| {
        let mut o = String::new();
        let mut abre = true;
        for c in s.chars() {
            if c == '`' {
                o.push(if abre { '«' } else { '»' });
                abre = !abre;
            } else {
                o.push(c);
            }
        }
        o
    };
    if let Some(r) = e.strip_prefix("unknown field `") {
        let (campo, resto) = r.split_once('`').unwrap_or((r, ""));
        let validos = resto.split("expected one of ").nth(1).or_else(|| resto.split("expected ").nth(1)).unwrap_or("").replace('`', "");
        return gotv_engine::tr!("el campo «{campo}» no existe aquí; válidos: {validos}", "field «{campo}» does not exist here; valid: {validos}");
    }
    if let Some(r) = e.strip_prefix("missing field `") {
        return gotv_engine::tr!("falta el campo obligatorio «{}»", "required field «{}» is missing", r.trim_end_matches('`'));
    }
    if let Some(r) = e.strip_prefix("unknown variant `") {
        let (v, _) = r.split_once('`').unwrap_or((r, ""));
        return gotv_engine::tr!("el tipo «{v}» no existe; tipos: {}", "type «{v}» does not exist; types: {}", TIPOS.join(", "));
    }
    if e.starts_with("invalid type: ") {
        return format!("tipo de dato equivocado ({})", tick(e.trim_start_matches("invalid type: ")).replace("expected", "se esperaba"));
    }
    e.to_string()
}

fn uno_de(campo: &str, v: &str, lista: &[&str], errores: &mut Vec<String>) {
    if !lista.contains(&v) {
        errores.push(gotv_engine::tr!("{campo}: «{v}» no es válido; usa uno de: {}", "{campo}: «{v}» is not valid; use one of: {}", lista.join(", ")));
    }
}

/// Distancia de edición (para sugerir el icono o la dirección que el modelo quiso decir).
fn distancia(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut cur = vec![i + 1];
        for (j, cb) in b.iter().enumerate() {
            cur.push((prev[j] + usize::from(ca != *cb)).min(prev[j + 1] + 1).min(cur[j] + 1));
        }
        prev = cur;
    }
    prev[b.len()]
}

fn parecidos(v: &str, opciones: impl Iterator<Item = &'static str>) -> String {
    let mut o: Vec<(usize, &str)> = opciones.map(|x| (distancia(v, x).min(if x.contains(v) || v.contains(x) { 1 } else { usize::MAX }), x)).collect();
    o.sort();
    o.iter().take(5).map(|(_, x)| *x).collect::<Vec<_>>().join(", ")
}

fn icono_ok(campo: &str, n: &Option<String>, errores: &mut Vec<String>) {
    if let Some(n) = n {
        if recursos::icono(n, None).is_none() {
            errores.push(gotv_engine::tr!("{campo}: el icono «{n}» no existe; parecidos: {} (también valen nombres en español: rayo, cohete, escudo…; lista: `gotv pagina --iconos`)", "{campo}: icon «{n}» does not exist; similar: {} (Spanish names also work: rayo, cohete, escudo…; list: `gotv pagina --iconos`)", parecidos(n, recursos::ICONOS.iter().map(|(x, _)| *x))));
        }
    }
}

/// Valida una ficha y devuelve TODOS los errores con su ruta (`secciones[3].items[1].icono: …`),
/// para que un modelo pequeño la arregle de una vez. `Ok` trae la ficha lista para [`Pagina::new`].
pub fn validar(json: &str) -> Result<Ficha, Vec<String>> {
    let v: serde_json::Value = serde_json::from_str(json).map_err(|e| vec![gotv_engine::tr!("el JSON no se puede leer: {e} (revisa comas, comillas y llaves)", "the JSON cannot be read: {e} (check commas, quotes and braces)")])?;
    let obj = v.as_object().ok_or_else(|| vec![gotv_engine::tr!("la ficha debe ser un objeto JSON {{ … }}", "the spec must be a JSON object {{ … }}")])?;
    let mut errores = Vec::new();
    for k in obj.keys() {
        if !["direccion", "tema", "idioma", "titulo", "descripcion", "marca", "secciones"].contains(&k.as_str()) {
            errores.push(gotv_engine::tr!("«{k}» no es un campo de la ficha; válidos: direccion, tema, idioma, titulo, descripcion, marca, secciones", "«{k}» is not a spec field; valid: direccion, tema, idioma, titulo, descripcion, marca, secciones"));
        }
    }
    for k in ["direccion", "titulo", "marca", "secciones"] {
        if !obj.contains_key(k) {
            errores.push(gotv_engine::tr!("falta el campo obligatorio «{k}»", "required field «{k}» is missing"));
        }
    }
    if let Some(d) = obj.get("direccion").and_then(|d| d.as_str()) {
        if direccion(d).is_none() {
            errores.push(gotv_engine::tr!("direccion: «{d}» no existe; usa una de: {}", "direccion: «{d}» does not exist; use one of: {}", DIRECCIONES.iter().map(|d| d.id).collect::<Vec<_>>().join(", ")));
        }
    }
    for (k, campo) in [("titulo", "titulo"), ("descripcion", "descripcion"), ("idioma", "idioma"), ("direccion", "direccion")] {
        if let Some(x) = obj.get(k) {
            if !x.is_string() {
                errores.push(format!("{campo}: debe ser texto entre comillas"));
            }
        }
    }
    if let Some(m) = obj.get("marca") {
        match serde_json::from_value::<Marca>(m.clone()) {
            Ok(m) => icono_ok("marca.icono", &m.icono, &mut errores),
            Err(e) => errores.push(format!("marca: {}", traducir(&e.to_string()))),
        }
    }
    if let Some(t) = obj.get("tema") {
        match serde_json::from_value::<TemaParcial>(t.clone()) {
            Ok(t) => {
                for (campo, val, lista) in [("tema.croma", &t.croma, CROMAS), ("tema.radio", &t.radio, RADIOS), ("tema.escala", &t.escala, ESCALAS), ("tema.movimiento", &t.movimiento, MOVIMIENTOS), ("tema.modo", &t.modo, MODOS)] {
                    if let Some(x) = val {
                        uno_de(campo, x, lista, &mut errores);
                    }
                }
                if let Some(f) = &t.fuentes {
                    if recursos::par(f).is_none() {
                        errores.push(gotv_engine::tr!("tema.fuentes: «{f}» no existe; usa uno de: {}", "tema.fuentes: «{f}» does not exist; use one of: {}", recursos::PARES.iter().map(|p| p.0).collect::<Vec<_>>().join(", ")));
                    }
                }
                if let Some(m) = &t.marca {
                    if crate::color::Rgb8::parse_hex(m).is_none() && Tema::nombre_de_color(m).is_none() {
                        errores.push(format!("tema.marca: «{m}» no es un color; usa un hex (#7c3aed) o un nombre (morado, azul, teal…)"));
                    }
                }
            }
            Err(e) => errores.push(format!("tema: {}", traducir(&e.to_string()))),
        }
    }
    match obj.get("secciones") {
        Some(serde_json::Value::Array(secs)) => {
            if secs.is_empty() {
                errores.push(gotv_engine::tr!("secciones: está vacía; una página necesita al menos un «heroe»", "secciones: it is empty; a page needs at least one «heroe»"));
            }
            let mut heroes = 0;
            for (i, sv) in secs.iter().enumerate() {
                let tipo = sv.get("tipo").and_then(|t| t.as_str()).unwrap_or("?");
                let ruta = format!("secciones[{i}] ({tipo})");
                if tipo == "heroe" {
                    heroes += 1; // se cuenta por su tipo, aunque traiga otro error
                }
                if sv.get("tipo").is_none() {
                    errores.push(format!("secciones[{i}]: falta «tipo»; tipos: {}", TIPOS.join(", ")));
                    continue;
                }
                match serde_json::from_value::<Seccion>(sv.clone()) {
                    Err(e) => errores.push(format!("{ruta}: {}", traducir(&e.to_string()))),
                    Ok(sec) => match &sec {
                        Seccion::Heroe { variante, arte: a, .. } => {
                            if let Some(x) = variante {
                                uno_de(&format!("{ruta}.variante"), x, HEROES, &mut errores);
                            }
                            match a {
                                Some(serde_json::Value::String(n)) => uno_de(&format!("{ruta}.arte"), n, ARTES, &mut errores),
                                Some(o @ serde_json::Value::Object(_)) => {
                                    if let Err(es) = crate::isometrico::dibujar(&o.to_string()) {
                                        errores.extend(es.into_iter().map(|e| format!("{ruta}.arte.{e}")));
                                    }
                                }
                                Some(_) => errores.push(gotv_engine::tr!("{ruta}.arte: debe ser un nombre ({}) o una escena {{\"piezas\":[…]}}", "{ruta}.arte: must be a name ({}) or a scene {{\"piezas\":[…]}}", ARTES.join(", "))),
                                None => {}
                            }
                        }
                        Seccion::Caracteristicas { variante, items, .. } => {
                            if let Some(x) = variante {
                                uno_de(&format!("{ruta}.variante"), x, CARACTERISTICAS, &mut errores);
                            }
                            for (j, it) in items.iter().enumerate() {
                                icono_ok(&format!("{ruta}.items[{j}].icono"), &it.icono, &mut errores);
                            }
                        }
                        Seccion::Libre { html, css, .. } => {
                            if let Some(m) = libre_prohibido(html).or_else(|| css.as_deref().and_then(libre_prohibido)) {
                                errores.push(gotv_engine::tr!("{ruta}: el bloque libre {m}", "{ruta}: the free block {m}"));
                            }
                            if html.to_lowercase().contains("<h1") {
                                errores.push(gotv_engine::tr!("{ruta}: el bloque libre no puede traer <h1> (sólo el héroe lo lleva); usa <h2>", "{ruta}: the free block cannot have <h1> (only the hero has it); use <h2>"));
                            }
                        }
                        Seccion::Precios { planes, .. } if planes.iter().filter(|p| p.destacado).count() > 1 => {
                            errores.push(gotv_engine::tr!("{ruta}: sólo un plan puede ser «destacado»", "{ruta}: only one plan can be «destacado»"));
                        }
                        _ => {}
                    },
                }
            }
            if heroes == 0 && !secs.is_empty() {
                errores.push(gotv_engine::tr!("secciones: falta un «heroe» (la cabecera grande con el h1)", "secciones: a «heroe» is missing (the big header with the h1)"));
            }
            if heroes > 1 {
                errores.push(gotv_engine::tr!("secciones: hay {heroes} «heroe»; una página lleva uno solo (un solo h1)", "secciones: there are {heroes} «heroe»; a page has only one (a single h1)"));
            }
        }
        Some(_) => errores.push(gotv_engine::tr!("secciones: debe ser una lista [ … ]", "secciones: must be a list [ … ]")),
        None => {}
    }
    if !errores.is_empty() {
        return Err(errores);
    }
    serde_json::from_value(v).map_err(|e| vec![traducir(&e.to_string())])
}

// ── escribir el proyecto sin pisar lo editado a mano (D4) ─────────────────

/// Huella FNV-1a de 64 bits: detecta cambios (no es seguridad).
fn huella(b: &[u8]) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for x in b {
        h ^= *x as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{h:016x}")
}

#[derive(Debug, Default)]
pub struct Informe {
    pub escritos: Vec<String>,
    pub iguales: Vec<String>,
    /// editados a mano después de la última compilación: NO se pisan; el nuevo va al lado
    pub respetados: Vec<(String, String)>,
}

/// El manifiesto de lo que `pagina` escribió (ruta → huella), en formato propio (códec, PAGI). Hasta
/// 2026-11-17 era `.gotv/pagina.json`: se migra solo.
pub fn manifiesto(base: &std::path::Path) -> std::path::PathBuf {
    base.join(".gotv/pagina.gotv")
}

/// ¿Este proyecto lo generó `pagina`? (hay manifiesto, nuevo o de antes)
pub fn es_de_gotv(base: &std::path::Path) -> bool {
    manifiesto(base).exists() || base.join(".gotv/pagina.json").exists()
}

/// Las huellas que dejó `pagina` (vacío si nunca escribió aquí).
pub fn huellas(base: &std::path::Path) -> std::collections::BTreeMap<String, String> {
    gotv_engine::codec::mapa_migrado(&manifiesto(base), &base.join(".gotv/pagina.json"), b"PAGI")
}

/// Escribe los archivos en `base`. Guarda el manifiesto con la huella de cada uno;
/// si al recompilar un archivo ya no tiene la huella que GOTV dejó (lo editó alguien), no se
/// pisa: el nuevo se escribe como `nombre.gotv-nuevo.ext` y se informa.
pub fn escribir_proyecto(base: &std::path::Path, archivos: &[(String, Vec<u8>)]) -> anyhow::Result<Informe> {
    let previo = huellas(base);
    let mut nuevo = previo.clone();
    let mut inf = Informe::default();
    for (ruta, bytes) in archivos {
        if ruta.contains("..") || ruta.starts_with('/') {
            gotv_engine::bail_tr!("ruta insegura en la salida: {ruta}", "unsafe path in the output: {ruta}");
        }
        let destino = base.join(ruta);
        if let Some(padre) = destino.parent() {
            std::fs::create_dir_all(padre)?;
        }
        let h_nueva = huella(bytes);
        match std::fs::read(&destino) {
            Ok(actual) if huella(&actual) == h_nueva => {
                inf.iguales.push(ruta.clone());
                nuevo.insert(ruta.clone(), h_nueva);
            }
            // ficha.json es la FUENTE que GOTV mantiene (D6/D11): se actualiza siempre — si el modelo la
            // editó a mano, esa edición ya entró en la ficha que se está escribiendo (medido D11: dos
            // fichas, ficha.json y ficha.gotv-nuevo.json, y el 9B perdido entre ellas)
            Ok(actual) if ruta != "ficha.json" && (previo.get(ruta).is_some_and(|h| *h != huella(&actual)) || (!previo.contains_key(ruta))) => {
                // editado a mano (o existía antes de GOTV): se respeta
                let p = std::path::Path::new(ruta);
                let lado = match (p.file_stem(), p.extension()) {
                    (Some(st), Some(ext)) => p.with_file_name(format!("{}.gotv-nuevo.{}", st.to_string_lossy(), ext.to_string_lossy())),
                    _ => p.with_file_name(format!("{}.gotv-nuevo", p.to_string_lossy())),
                };
                std::fs::write(base.join(&lado), bytes)?;
                inf.respetados.push((ruta.clone(), lado.to_string_lossy().into_owned()));
            }
            _ => {
                std::fs::write(&destino, bytes)?;
                inf.escritos.push(ruta.clone());
                nuevo.insert(ruta.clone(), h_nueva);
            }
        }
    }
    gotv_engine::codec::guardar_mapa(&manifiesto(base), b"PAGI", &nuevo)?;
    Ok(inf)
}

/// Guía compacta de la ficha para el modelo (va en la descripción de la herramienta).
pub fn esquema() -> String {
    format!(
        r##"FICHA DE PÁGINA (JSON). Tú decides contenido y estilo; GOTV construye el diseño.
{{"direccion": uno de [{dirs}],
 "tema": {{"marca":"#hex o nombre (morado, azul, teal…)","croma":"{cromas}","radio":"{radios}","escala":"{escalas}","movimiento":"{movs}","modo":"{modos}","fuentes":"{pares}"}}  (todo opcional; la dirección ya trae buenos valores),
 "idioma":"es", "titulo":"<title>", "descripcion":"meta description (1 frase)",
 "marca": {{"nombre":"…","icono":"opcional","lema":"opcional"}},
 "secciones": [ … en orden … ]}}
SECCIONES ("tipo" + campos; los marcados ? son opcionales):
 navegacion: enlaces[{{texto,href}}], cta?{{texto,href}}
 heroe (UNO, lleva el h1): titulo, etiqueta?, texto?, cta?, cta2?, variante?[{heroes}],
   arte? = nombre [{artes}] o escena isométrica {{"piezas":[{{"tipo":"caja","x":0,"y":0,"ancho":2,"fondo":2,"alto":1,"color":"acento","etiqueta":"API"}},{{"tipo":"cilindro",…}},{{"tipo":"flecha","desde":[x,y,z],"hasta":[x,y,z]}}]}}
 logos: nombres[], titulo?
 caracteristicas: items[{{titulo,texto,icono?}}], titulo?, texto?, etiqueta?, id?, variante?[{carac}]
 cifras: items[{{valor,etiqueta}}] · pasos: items[{{titulo,texto}}] · testimonios: items[{{cita,nombre,cargo?}}]
 precios: planes[{{nombre,precio,periodo?,descripcion?,incluye[],cta{{texto,href}},destacado?}}] (un solo destacado)
 faq: items[{{pregunta,respuesta}}] · llamada: titulo, texto?, cta, cta2? · contacto: titulo?, texto?, accion?, boton?
 pie: columnas[{{titulo,enlaces[]}}], legal? · libre: html (+css?) SOLO para lo que no existe arriba
 (secciones de contenido aceptan titulo?, texto?, etiqueta?, id? — el id sirve para enlaces #ancla)
REGLAS: *palabra* en un título la resalta. Iconos: nombre en inglés o español (rayo, cohete, escudo, usuarios…).
Textos concretos del negocio, nada de «Lorem» ni «Solución innovadora». Si algo falla, recibes la lista exacta de errores."##,
        dirs = DIRECCIONES.iter().map(|d| format!("{} ({})", d.id, d.texto())).collect::<Vec<_>>().join("; "),
        cromas = CROMAS.join("|"),
        radios = RADIOS.join("|"),
        escalas = ESCALAS.join("|"),
        movs = MOVIMIENTOS.join("|"),
        modos = MODOS.join("|"),
        pares = recursos::PARES.iter().map(|p| p.0).collect::<Vec<_>>().join("|"),
        heroes = HEROES.join("|"),
        carac = CARACTERISTICAS.join("|"),
        artes = ARTES.join("|"),
    )
}

// ── tolerancia (D4b): el programa se adapta al modelo pequeño ─────────────
//
// Medido (NOTAS §D4): con validación estricta + un reintento el 9B dejó 2/10 fichas válidas
// (emojis por iconos, secciones y campos inventados, objetos donde va texto). Aquí se
// normaliza lo que se entiende, se AVISA lo que se ignoró y sólo se falla ante lo que impide
// pintar. Nada se inventa en silencio: cada cambio deja un aviso.

use serde_json::{json, Map, Value};

fn campos(tipo: &str) -> &'static [&'static str] {
    match tipo {
        "navegacion" => &["tipo", "enlaces", "cta"],
        "heroe" => &["tipo", "variante", "etiqueta", "titulo", "texto", "cta", "cta2", "arte"],
        "logos" => &["tipo", "titulo", "nombres"],
        "caracteristicas" => &["tipo", "variante", "id", "etiqueta", "titulo", "texto", "items"],
        "cifras" | "pasos" | "testimonios" | "faq" => &["tipo", "id", "etiqueta", "titulo", "texto", "items"],
        "precios" => &["tipo", "id", "etiqueta", "titulo", "texto", "planes"],
        "llamada" => &["tipo", "id", "titulo", "texto", "cta", "cta2"],
        "contacto" => &["tipo", "id", "etiqueta", "titulo", "texto", "accion", "boton"],
        "pie" => &["tipo", "columnas", "legal"],
        "libre" => &["tipo", "id", "html", "css"],
        _ => &[],
    }
}

/// Sinónimos de tipo que los modelos inventan → el tipo real.
fn sinonimo(t: &str) -> Option<&'static str> {
    let t = ancla(t);
    let m: &[(&[&str], &str)] = &[
        (&["nav", "menu", "header", "cabecera", "barra", "navbar", "encabezado"], "navegacion"),
        (&["hero", "portada", "banner", "inicio", "cabecera-principal", "principal", "bienvenida"], "heroe"),
        (&["features", "servicios", "beneficios", "productos", "sobre-mi", "sobre-nosotros", "nosotros", "about", "quienes-somos", "equipo", "galeria", "portafolio", "clases", "programa", "agenda", "menu-cafe", "especialidades", "tratamientos", "ventajas", "por-que", "porque", "valores", "mision", "historia", "cursos", "niveles", "colecciones", "catalogo", "areas", "practicas"], "caracteristicas"), // juez-idioma: datos (alias de una sección)
        (&["stats", "estadisticas", "numeros", "impacto", "resultados", "metricas", "logros"], "cifras"),
        (&["proceso", "como-funciona", "metodo", "etapas", "pasos-a-seguir", "cronograma", "timeline"], "pasos"),
        (&["testimonials", "opiniones", "resenas", "reseñas", "clientes-dicen", "comentarios"], "testimonios"), // juez-idioma: datos (alias de una sección)
        (&["pricing", "planes", "suscripcion", "suscripciones", "tarifas", "paquetes", "entradas", "membresias", "precio"], "precios"),
        (&["preguntas", "faqs", "preguntas-frecuentes", "dudas"], "faq"),
        (&["cta", "accion", "llamada-a-la-accion", "call-to-action", "cierre"], "llamada"), // juez-idioma: datos (alias de una sección)
        (&["contact", "contactanos", "contactenos", "formulario", "reservas", "agenda-cita", "cita", "agendar"], "contacto"),
        (&["footer", "pie-de-pagina", "piedepagina"], "pie"),
        (&["clientes", "marcas", "aliados", "patrocinadores", "partners"], "logos"),
    ];
    m.iter().find(|(xs, _)| xs.contains(&t.as_str())).map(|(_, r)| *r)
}

/// Texto desde lo que sea: objeto con texto/titulo/nombre, número, lista de textos.
fn a_texto(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(_) | Value::Null => None,
        Value::Array(a) => {
            let t: Vec<String> = a.iter().filter_map(a_texto).collect();
            (!t.is_empty()).then(|| t.join(" · "))
        }
        Value::Object(o) => ["texto", "text", "titulo", "title", "nombre", "label", "valor"].iter().find_map(|k| o.get(*k).and_then(a_texto)),
    }
}

/// Enlace desde un texto suelto o un objeto con otros nombres de campo.
fn a_enlace(v: &Value) -> Option<Value> {
    match v {
        Value::String(s) if !s.trim().is_empty() => Some(json!({"texto": s, "href": "#"})),
        Value::Object(o) => {
            let texto = ["texto", "text", "label", "titulo", "nombre"].iter().find_map(|k| o.get(*k).and_then(a_texto))?;
            let href = ["href", "url", "enlace", "link", "destino"].iter().find_map(|k| o.get(*k).and_then(|x| x.as_str().map(String::from))).unwrap_or_else(|| "#".into());
            Some(json!({"texto": texto, "href": href}))
        }
        _ => None,
    }
}

fn renombra(o: &mut Map<String, Value>, de: &[&str], a: &str) {
    if o.contains_key(a) {
        return;
    }
    for d in de {
        if let Some(v) = o.remove(*d) {
            o.insert(a.into(), v);
            return;
        }
    }
}

fn icono_tolerante(o: &mut Map<String, Value>, ruta: &str, avisos: &mut Vec<String>) {
    if let Some(v) = o.get("icono").cloned() {
        match v.as_str().and_then(recursos::resolver) {
            Some(real) => {
                o.insert("icono".into(), Value::String(real.into()));
            }
            None => {
                o.remove("icono");
                avisos.push(gotv_engine::tr!("{ruta}.icono: «{}» no existe; se quitó el icono", "{ruta}.icono: «{}» does not exist; the icon was removed", a_texto(&v).unwrap_or_default()));
            }
        }
    }
}

fn limpia(o: &mut Map<String, Value>, validos: &[&str], ruta: &str, avisos: &mut Vec<String>) {
    let sobran: Vec<String> = o.keys().filter(|k| !validos.contains(&k.as_str())).cloned().collect();
    for k in sobran {
        o.remove(&k);
        avisos.push(gotv_engine::tr!("{ruta}: se ignoró el campo «{k}»", "{ruta}: field «{k}» was ignored"));
    }
}

fn textos(o: &mut Map<String, Value>, claves: &[&str]) {
    for k in claves {
        if let Some(v) = o.get(*k) {
            if !v.is_string() {
                match a_texto(v) {
                    Some(t) => {
                        o.insert((*k).into(), Value::String(t));
                    }
                    None => {
                        o.remove(*k);
                    }
                }
            }
        }
    }
}

fn item(v: &Value, tipo: &str, ruta: &str, avisos: &mut Vec<String>) -> Option<Value> {
    let mut o = match v {
        Value::Object(o) => o.clone(),
        Value::String(s) => {
            let mut m = Map::new();
            m.insert(if tipo == "testimonios" { "cita" } else if tipo == "cifras" { "valor" } else if tipo == "faq" { "pregunta" } else { "titulo" }.into(), Value::String(s.clone()));
            m
        }
        _ => return None,
    };
    match tipo {
        "testimonios" => {
            renombra(&mut o, &["texto", "testimonio", "comentario", "quote", "opinion"], "cita");
            renombra(&mut o, &["autor", "persona", "name", "cliente"], "nombre");
            renombra(&mut o, &["rol", "puesto", "empresa", "role", "ciudad"], "cargo");
            limpia(&mut o, &["cita", "nombre", "cargo"], ruta, avisos);
            textos(&mut o, &["cita", "nombre", "cargo"]);
            if !o.contains_key("nombre") {
                o.insert("nombre".into(), Value::String("Cliente".into()));
            }
            o.contains_key("cita").then_some(Value::Object(o))
        }
        "faq" => {
            renombra(&mut o, &["q", "question", "titulo", "pregunta_"], "pregunta");
            renombra(&mut o, &["a", "answer", "texto", "respuesta_"], "respuesta");
            limpia(&mut o, &["pregunta", "respuesta"], ruta, avisos);
            textos(&mut o, &["pregunta", "respuesta"]);
            (o.contains_key("pregunta") && o.contains_key("respuesta")).then_some(Value::Object(o))
        }
        "cifras" => {
            renombra(&mut o, &["numero", "cifra", "dato", "value", "titulo"], "valor");
            renombra(&mut o, &["texto", "descripcion", "label", "nombre"], "etiqueta");
            limpia(&mut o, &["valor", "etiqueta"], ruta, avisos);
            textos(&mut o, &["valor", "etiqueta"]);
            (o.contains_key("valor") && o.contains_key("etiqueta")).then_some(Value::Object(o))
        }
        _ => {
            renombra(&mut o, &["nombre", "title", "pregunta", "dia", "paso"], "titulo"); // juez-idioma: datos (alias de una sección)
            renombra(&mut o, &["descripcion", "description", "detalle", "respuesta", "resumen"], "texto");
            icono_tolerante(&mut o, ruta, avisos);
            limpia(&mut o, &["icono", "titulo", "texto"], ruta, avisos);
            textos(&mut o, &["titulo", "texto"]);
            o.contains_key("titulo").then_some(Value::Object(o))
        }
    }
}

fn tipo_por_forma(o: &Map<String, Value>) -> Option<&'static str> {
    if o.contains_key("planes") {
        return Some("precios");
    }
    if o.contains_key("columnas") || o.contains_key("legal") {
        return Some("pie");
    }
    if o.contains_key("nombres") {
        return Some("logos");
    }
    if o.contains_key("html") {
        return Some("libre");
    }
    if let Some(Value::Array(items)) = o.get("items") {
        let k = |c: &str| items.iter().any(|i| i.get(c).is_some());
        return Some(if k("cita") || k("testimonio") || k("autor") {
            "testimonios"
        } else if k("pregunta") || k("respuesta") || k("q") {
            "faq"
        } else if k("valor") || k("numero") || k("cifra") {
            "cifras"
        } else {
            "caracteristicas"
        });
    }
    if o.contains_key("enlaces") && !o.contains_key("titulo") {
        return Some("navegacion");
    }
    (o.contains_key("titulo") || o.contains_key("texto")).then_some("caracteristicas")
}

/// Normaliza una ficha escrita por un modelo. Devuelve la ficha arreglada y los avisos de lo
/// que se cambió o ignoró.
pub fn normalizar(entrada: Value) -> (Value, Vec<String>) {
    let mut avisos = Vec::new();
    let Value::Object(mut f) = entrada else { return (entrada, avisos) };
    // raíz
    renombra(&mut f, &["direction", "estilo", "style"], "direccion");
    renombra(&mut f, &["title", "nombre_pagina"], "titulo");
    renombra(&mut f, &["sections", "bloques", "contenido"], "secciones");
    renombra(&mut f, &["brand", "negocio", "empresa"], "marca");
    if let Some(d) = f.get("direccion").and_then(|d| d.as_str()).map(ancla) {
        if direccion(&d).is_none() {
            let cerca = DIRECCIONES.iter().map(|x| (distancia(&d, x.id), x.id)).min().map(|(_, x)| x).unwrap_or("tecnico");
            let elegida = if distancia(&d, cerca) <= 3 { cerca } else { "tecnico" };
            avisos.push(gotv_engine::tr!("direccion: «{d}» no existe; se usó «{elegida}»", "direccion: «{d}» does not exist; «{elegida}» was used"));
            f.insert("direccion".into(), Value::String(elegida.into()));
        } else {
            f.insert("direccion".into(), Value::String(d));
        }
    } else {
        avisos.push(gotv_engine::tr!("direccion: faltaba; se usó «tecnico»", "direccion: it was missing; «tecnico» was used"));
        f.insert("direccion".into(), Value::String("tecnico".into()));
    }
    // marca: texto suelto → objeto
    match f.get("marca").cloned() {
        Some(Value::String(n)) => {
            f.insert("marca".into(), json!({"nombre": n}));
        }
        Some(Value::Object(mut m)) => {
            renombra(&mut m, &["name", "titulo"], "nombre");
            renombra(&mut m, &["eslogan", "slogan", "tagline", "frase"], "lema");
            icono_tolerante(&mut m, "marca", &mut avisos);
            limpia(&mut m, &["nombre", "icono", "lema"], "marca", &mut avisos);
            textos(&mut m, &["nombre", "lema"]);
            f.insert("marca".into(), Value::Object(m));
        }
        _ => {}
    }
    if f.get("marca").and_then(|m| m.get("nombre")).is_none() {
        let n = f.get("titulo").and_then(a_texto).map(|t| t.split(['|', '—', '-', ':']).next().unwrap_or(&t).trim().to_string()).unwrap_or_else(|| "Marca".into());
        avisos.push(gotv_engine::tr!("marca.nombre: faltaba; se usó «{n}»", "marca.nombre: it was missing; «{n}» was used"));
        f.insert("marca".into(), json!({"nombre": n}));
    }
    textos(&mut f, &["titulo", "descripcion", "idioma"]);
    if !f.contains_key("titulo") {
        let n = f["marca"]["nombre"].as_str().unwrap_or(if f["idioma"].as_str().is_some_and(|i| i.trim().to_lowercase().starts_with("en")) { "Page" } else { "Página" }).to_string(); // juez-idioma: datos (título por defecto, en el idioma de la PÁGINA)
        f.insert("titulo".into(), Value::String(n));
    }
    // tema: valores fuera de lista → se quitan (manda la dirección)
    if let Some(Value::Object(t)) = f.get_mut("tema") {
        for (k, lista) in [("croma", CROMAS), ("radio", RADIOS), ("escala", ESCALAS), ("movimiento", MOVIMIENTOS), ("modo", MODOS)] {
            if let Some(v) = t.get(k).and_then(|v| v.as_str()) {
                if !lista.contains(&v) {
                    avisos.push(gotv_engine::tr!("tema.{k}: «{v}» no vale; manda la dirección", "tema.{k}: «{v}» is not valid; the direction decides"));
                    t.remove(k);
                }
            }
        }
        if let Some(v) = t.get("fuentes").and_then(|v| v.as_str()) {
            if recursos::par(v).is_none() {
                avisos.push(gotv_engine::tr!("tema.fuentes: «{v}» no existe; manda la dirección", "tema.fuentes: «{v}» does not exist; the direction decides"));
                t.remove("fuentes");
            }
        }
        if let Some(v) = t.get("marca").and_then(|v| v.as_str()) {
            if crate::color::Rgb8::parse_hex(v).is_none() && Tema::nombre_de_color(v).is_none() {
                avisos.push(gotv_engine::tr!("tema.marca: «{v}» no es un color; manda la dirección", "tema.marca: «{v}» is not a color; the direction decides"));
                t.remove("marca");
            }
        }
        let sobran: Vec<String> = t.keys().filter(|k| !["marca", "croma", "radio", "escala", "movimiento", "modo", "fuentes"].contains(&k.as_str())).cloned().collect();
        for k in sobran {
            t.remove(&k);
            avisos.push(gotv_engine::tr!("tema: se ignoró «{k}»", "tema: «{k}» was ignored"));
        }
    }
    let raiz_validos = ["direccion", "tema", "idioma", "titulo", "descripcion", "marca", "secciones"];
    let sobran: Vec<String> = f.keys().filter(|k| !raiz_validos.contains(&k.as_str())).cloned().collect();
    for k in sobran {
        f.remove(&k);
        avisos.push(gotv_engine::tr!("se ignoró «{k}» en la raíz de la ficha", "«{k}» was ignored at the root of the spec"));
    }
    // secciones
    let mut out = Vec::new();
    let mut heroes = 0;
    if let Some(Value::Array(secs0)) = f.get("secciones").cloned() {
        // una sección metida como CAMPO de otra (`"precios": {…}` dentro de testimonios):
        // se saca como sección propia, justo después
        let mut secs = Vec::new();
        for sv in secs0 {
            let mut hijas = Vec::new();
            let mut sv = sv;
            if let Value::Object(o) = &mut sv {
                let tipo_propio = o.get("tipo").and_then(|t| t.as_str()).unwrap_or("").to_string();
                let anidadas: Vec<String> = o.keys().filter(|k| TIPOS.contains(&k.as_str()) && **k != tipo_propio && o[*k].is_object()).cloned().collect();
                for k in anidadas {
                    if let Some(Value::Object(mut h)) = o.remove(&k) {
                        h.entry("tipo").or_insert(Value::String(k.clone()));
                        avisos.push(gotv_engine::tr!("«{k}» venía metida dentro de otra sección; se sacó como sección propia", "«{k}» came nested inside another section; it was taken out as its own section"));
                        hijas.push(Value::Object(h));
                    }
                }
            }
            secs.push(sv);
            secs.extend(hijas);
        }
        for (i, sv) in secs.into_iter().enumerate() {
            let Value::Object(mut o) = sv else {
                avisos.push(gotv_engine::tr!("secciones[{i}]: no era un objeto; se quitó", "secciones[{i}]: it was not an object; removed"));
                continue;
            };
            let ruta0 = format!("secciones[{i}]");
            let dado = o.get("tipo").and_then(|t| t.as_str()).map(ancla).unwrap_or_default();
            let tipo: &str = if TIPOS.contains(&dado.as_str()) {
                TIPOS.iter().find(|t| **t == dado).copied().unwrap_or("caracteristicas")
            } else if let Some(t) = sinonimo(&dado).or_else(|| tipo_por_forma(&o)) {
                avisos.push(gotv_engine::tr!("{ruta0}: el tipo «{dado}» no existe; se armó como «{t}»", "{ruta0}: type «{dado}» does not exist; it was built as «{t}»"));
                t
            } else {
                avisos.push(gotv_engine::tr!("{ruta0}: el tipo «{dado}» no existe y no se entendió su forma; se quitó", "{ruta0}: type «{dado}» does not exist and its shape was not understood; removed"));
                continue;
            };
            let ruta = format!("{ruta0} ({tipo})");
            o.insert("tipo".into(), Value::String(tipo.into()));
            renombra(&mut o, &["title", "encabezado", "heading", "nombre"], "titulo");
            renombra(&mut o, &["subtitulo", "descripcion", "subtitle", "description", "parrafo", "contenido"], "texto");
            renombra(&mut o, &["kicker", "antetitulo", "eyebrow", "badge"], "etiqueta");
            renombra(&mut o, &["boton", "button", "accion_principal", "boton_principal"], if tipo == "contacto" { "boton" } else { "cta" });
            renombra(&mut o, &["boton2", "boton_secundario", "secundario"], "cta2");
            renombra(&mut o, &["elementos", "lista", "tarjetas", "servicios", "caracteristicas", "beneficios", "features", "pasos", "preguntas", "testimonios", "cifras", "datos"], "items");
            renombra(&mut o, &["plans", "paquetes", "tarifas", "opciones"], "planes");
            renombra(&mut o, &["links", "menu", "opciones_menu"], "enlaces");
            renombra(&mut o, &["marcas", "clientes", "logos", "empresas"], "nombres");
            renombra(&mut o, &["copyright", "derechos"], "legal");
            if tipo == "heroe" {
                if let Some(p) = o.remove("piezas") {
                    o.insert("arte".into(), json!({"piezas": p}));
                }
            }
            if let Some(Value::Array(_)) = o.get("items") {
            } else if o.contains_key("items") {
                o.remove("items");
            }
            textos(&mut o, &["titulo", "texto", "etiqueta", "id", "legal", "html", "css"]);
            for k in ["cta", "cta2"] {
                if let Some(v) = o.get(k).cloned() {
                    match a_enlace(&v) {
                        Some(e) => {
                            o.insert(k.into(), e);
                        }
                        None => {
                            o.remove(k);
                            avisos.push(gotv_engine::tr!("{ruta}.{k}: no era un enlace; se quitó", "{ruta}.{k}: it was not a link; removed"));
                        }
                    }
                }
            }
            match tipo {
                "contacto" => {
                    if let Some(b) = o.get("boton").cloned() {
                        match a_texto(&b) {
                            Some(t) => {
                                o.insert("boton".into(), Value::String(t));
                            }
                            None => {
                                o.remove("boton");
                            }
                        }
                    }
                    if let Some(a) = o.get("accion").cloned() {
                        let h = a_enlace(&a).and_then(|e| e["href"].as_str().map(String::from)).or_else(|| a.as_str().map(String::from)).unwrap_or_else(|| "#".into());
                        o.insert("accion".into(), Value::String(h));
                    }
                }
                "navegacion" | "pie" => {
                    if let Some(Value::Array(es)) = o.get("enlaces").cloned() {
                        o.insert("enlaces".into(), Value::Array(es.iter().filter_map(a_enlace).collect()));
                    }
                    if tipo == "navegacion" && !o.contains_key("enlaces") {
                        o.insert("enlaces".into(), json!([]));
                    }
                    if tipo == "pie" {
                        if let Some(Value::Array(cs)) = o.get("columnas").cloned() {
                            let cols: Vec<Value> = cs
                                .iter()
                                .filter_map(|c| {
                                    let t = c.get("titulo").and_then(a_texto)?;
                                    let es: Vec<Value> = c.get("enlaces").and_then(|e| e.as_array()).map(|a| a.iter().filter_map(a_enlace).collect()).unwrap_or_default();
                                    Some(json!({"titulo": t, "enlaces": es}))
                                })
                                .collect();
                            o.insert("columnas".into(), Value::Array(cols));
                        } else if let Some(Value::Array(es)) = o.remove("enlaces") {
                            o.insert("columnas".into(), json!([{"titulo": "Enlaces", "enlaces": es}]));
                        }
                    }
                }
                "logos" => {
                    let ns: Vec<Value> = o.get("nombres").and_then(|n| n.as_array()).map(|a| a.iter().filter_map(a_texto).map(Value::String).collect()).unwrap_or_default();
                    if ns.is_empty() {
                        avisos.push(gotv_engine::tr!("{ruta}: sin nombres; se quitó", "{ruta}: no names; removed"));
                        continue;
                    }
                    o.insert("nombres".into(), Value::Array(ns));
                }
                "precios" => {
                    let planes: Vec<Value> = o
                        .get("planes")
                        .and_then(|p| p.as_array())
                        .map(|ps| {
                            ps.iter()
                                .enumerate()
                                .filter_map(|(j, p)| {
                                    let mut q = p.as_object()?.clone();
                                    let r = format!("{ruta}.planes[{j}]");
                                    renombra(&mut q, &["titulo", "name", "plan"], "nombre");
                                    renombra(&mut q, &["costo", "valor", "price", "monto"], "precio");
                                    renombra(&mut q, &["features", "beneficios", "caracteristicas", "items", "incluye_"], "incluye");
                                    renombra(&mut q, &["texto", "description"], "descripcion");
                                    renombra(&mut q, &["boton", "button"], "cta");
                                    renombra(&mut q, &["recomendado", "popular", "destacada"], "destacado");
                                    textos(&mut q, &["nombre", "precio", "periodo", "descripcion"]);
                                    let inc: Vec<Value> = match q.get("incluye") {
                                        Some(Value::Array(a)) => a.iter().filter_map(a_texto).map(Value::String).collect(),
                                        Some(Value::String(s)) => s.split([',', ';', '\n']).map(|x| Value::String(x.trim().into())).filter(|x| x.as_str() != Some("")).collect(),
                                        _ => vec![],
                                    };
                                    q.insert("incluye".into(), Value::Array(inc));
                                    let cta = q.get("cta").and_then(a_enlace).unwrap_or_else(|| json!({"texto": "Elegir", "href": "#contacto"}));
                                    q.insert("cta".into(), cta);
                                    let d = q.get("destacado").map(|d| d.as_bool().unwrap_or(!d.is_null())).unwrap_or(false);
                                    q.insert("destacado".into(), Value::Bool(d));
                                    limpia(&mut q, &["nombre", "precio", "periodo", "descripcion", "incluye", "cta", "destacado"], &r, &mut avisos);
                                    (q.contains_key("nombre") && q.contains_key("precio")).then_some(Value::Object(q))
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    // un solo destacado
                    let mut visto = false;
                    let planes: Vec<Value> = planes
                        .into_iter()
                        .map(|mut p| {
                            if p["destacado"] == Value::Bool(true) {
                                if visto {
                                    p["destacado"] = Value::Bool(false);
                                }
                                visto = true;
                            }
                            p
                        })
                        .collect();
                    if planes.is_empty() {
                        avisos.push(gotv_engine::tr!("{ruta}: sin planes con nombre y precio; se quitó", "{ruta}: no plans with a name and price; removed"));
                        continue;
                    }
                    o.insert("planes".into(), Value::Array(planes));
                }
                "llamada" => {
                    if !o.contains_key("cta") {
                        o.insert("cta".into(), json!({"texto": "Empezar", "href": "#contacto"}));
                    }
                    if !o.contains_key("titulo") {
                        avisos.push(gotv_engine::tr!("{ruta}: sin título; se quitó", "{ruta}: no title; removed"));
                        continue;
                    }
                }
                "libre" => {
                    if let Some(m) = o.get("html").and_then(|h| h.as_str()).and_then(libre_prohibido) {
                        avisos.push(gotv_engine::tr!("{ruta}: el bloque libre {m}; se quitó", "{ruta}: the free block {m}; removed"));
                        continue;
                    }
                }
                _ => {}
            }
            if ["caracteristicas", "cifras", "pasos", "testimonios", "faq"].contains(&tipo) {
                let items: Vec<Value> = o
                    .get("items")
                    .and_then(|x| x.as_array())
                    .map(|a| a.iter().enumerate().filter_map(|(j, it)| item(it, tipo, &format!("{ruta}.items[{j}]"), &mut avisos)).collect())
                    .unwrap_or_default();
                if items.is_empty() && tipo != "caracteristicas" {
                    avisos.push(gotv_engine::tr!("{ruta}: sin elementos válidos; se quitó", "{ruta}: no valid items; removed"));
                    continue;
                }
                o.insert("items".into(), Value::Array(items));
            }
            if let Some(v) = o.get("variante").and_then(|v| v.as_str()) {
                let lista = if tipo == "heroe" { HEROES } else { CARACTERISTICAS };
                if !lista.contains(&v) {
                    avisos.push(gotv_engine::tr!("{ruta}.variante: «{v}» no vale; manda la dirección", "{ruta}.variante: «{v}» is not valid; the direction decides"));
                    o.remove("variante");
                }
            }
            if tipo == "heroe" {
                heroes += 1;
                if heroes > 1 {
                    avisos.push(gotv_engine::tr!("{ruta}: un segundo héroe se armó como sección normal (un solo h1)", "{ruta}: a second hero was built as a normal section (a single h1)"));
                    o.insert("tipo".into(), Value::String("caracteristicas".into()));
                    o.retain(|k, _| ["tipo", "etiqueta", "titulo", "texto"].contains(&k.as_str()));
                    o.insert("items".into(), json!([]));
                }
                if !o.contains_key("titulo") {
                    let t = f.get("titulo").and_then(a_texto).unwrap_or_default();
                    o.insert("titulo".into(), Value::String(t));
                    avisos.push(gotv_engine::tr!("{ruta}.titulo: faltaba; se usó el título de la página", "{ruta}.titulo: it was missing; the page title was used"));
                }
                match o.get("arte").cloned() {
                    Some(Value::String(n)) if !ARTES.contains(&n.as_str()) => {
                        o.remove("arte");
                        avisos.push(gotv_engine::tr!("{ruta}.arte: «{n}» no existe; manda la dirección", "{ruta}.arte: «{n}» does not exist; the direction decides"));
                    }
                    Some(Value::Object(mut a)) => {
                        let piezas: Vec<Value> = a.get("piezas").and_then(|p| p.as_array()).map(|ps| ps.iter().filter(|p| serde_json::from_value::<crate::isometrico::Pieza>((*p).clone()).is_ok()).cloned().collect()).unwrap_or_default();
                        let antes = a.get("piezas").and_then(|p| p.as_array()).map(|x| x.len()).unwrap_or(0);
                        if piezas.is_empty() {
                            o.remove("arte");
                            avisos.push(gotv_engine::tr!("{ruta}.arte: ninguna pieza isométrica válida; manda la dirección", "{ruta}.arte: no valid isometric piece; the direction decides"));
                        } else {
                            if piezas.len() < antes {
                                avisos.push(gotv_engine::tr!("{ruta}.arte: se quitaron {} pieza(s) que no existen (sólo caja, cilindro, flecha, texto)", "{ruta}.arte: {} piece(s) that do not exist were removed (only caja, cilindro, flecha, texto)", antes - piezas.len()));
                            }
                            a.insert("piezas".into(), Value::Array(piezas));
                            a.retain(|k, _| ["titulo", "escala", "marca", "animacion", "suelo", "piezas"].contains(&k.as_str()));
                            o.insert("arte".into(), Value::Object(a));
                        }
                    }
                    Some(Value::String(_)) | None => {}
                    Some(_) => {
                        o.remove("arte");
                    }
                }
            }
            limpia(&mut o, campos(tipo), &ruta, &mut avisos);
            out.push(Value::Object(o));
        }
    }
    if heroes == 0 {
        let t = f.get("titulo").and_then(a_texto).unwrap_or_default();
        let pos = out.iter().position(|s| s["tipo"] != "navegacion").unwrap_or(out.len());
        out.insert(pos, json!({"tipo": "heroe", "titulo": t, "texto": f.get("descripcion").and_then(a_texto)}));
        avisos.push(gotv_engine::tr!("secciones: faltaba el héroe; se armó con el título y la descripción", "secciones: the hero was missing; it was built from the title and the description"));
    }
    f.insert("secciones".into(), Value::Array(out));
    (Value::Object(f), avisos)
}

/// Byte de inicio de la línea/columna (1-based) que da serde_json.
fn desplazamiento(t: &str, linea: usize, col: usize) -> usize {
    let mut l = 1;
    for (i, c) in t.char_indices() {
        if l == linea {
            return t[i..].char_indices().nth(col.saturating_sub(1)).map(|(k, _)| i + k).unwrap_or(t.len());
        }
        if c == '\n' {
            l += 1;
        }
    }
    t.len()
}

/// Lo que falta cerrar al final (fuera de cadenas), en orden.
fn cierres_pendientes(t: &str) -> String {
    let (mut pila, mut dentro, mut esc) = (Vec::new(), false, false);
    for c in t.chars() {
        if dentro {
            match (esc, c) {
                (true, _) => esc = false,
                (false, '\\') => esc = true,
                (false, '"') => dentro = false,
                _ => {}
            }
            continue;
        }
        match c {
            '"' => dentro = true,
            '{' => pila.push('}'),
            '[' => pila.push(']'),
            '}' | ']' => {
                pila.pop();
            }
            _ => {}
        }
    }
    pila.iter().rev().collect()
}

/// Repara el JSON roto típico de un modelo pequeño guiándose por el error exacto de serde:
/// coma sobrante, objeto sin cerrar antes de un hermano, cierre que no corresponde, final
/// cortado. Hasta 40 arreglos; si no converge, devuelve el error original.
pub fn reparar_json(t: &str) -> Result<(Value, Vec<String>), String> {
    let mut txt = t.to_string();
    let mut arreglos = Vec::new();
    let original = match serde_json::from_str::<Value>(&txt) {
        Ok(v) => return Ok((v, arreglos)),
        Err(e) => e.to_string(),
    };
    for _ in 0..40 {
        let e = match serde_json::from_str::<Value>(&txt) {
            Ok(v) => return Ok((v, arreglos)),
            Err(e) => e,
        };
        let msg = e.to_string();
        let pos = desplazamiento(&txt, e.line(), e.column());
        let antes = txt[..pos.min(txt.len())].trim_end();
        if msg.starts_with("EOF while parsing") {
            let c = cierres_pendientes(&txt);
            if c.is_empty() {
                break;
            }
            txt.push_str(&c);
            arreglos.push(gotv_engine::tr!("se cerró el final que faltaba ({c})", "the missing ending was closed ({c})"));
        } else if msg.starts_with("trailing comma") {
            if let Some(k) = antes.rfind(',') {
                txt.remove(k);
                arreglos.push(gotv_engine::tr!("se quitó una coma sobrante (línea {})", "an extra comma was removed (line {})", e.line()));
            } else {
                break;
            }
        } else if msg.starts_with("key must be a string") && txt[pos.saturating_sub(1)..].trim_start().starts_with('{') {
            // `}, {` dentro de un objeto: faltó cerrar el objeto anterior antes del hermano
            match antes.rfind(',') {
                Some(k) => {
                    txt.insert(k, '}');
                    arreglos.push(gotv_engine::tr!("se cerró un objeto que quedó abierto (línea {})", "an object left open was closed (line {})", e.line()));
                }
                None => break,
            }
        } else if msg.starts_with("expected `,` or `}`") && txt[pos.saturating_sub(1)..].trim_start().starts_with(']') {
            let k = pos.saturating_sub(1) + txt[pos.saturating_sub(1)..].find(']').unwrap_or(0);
            txt.insert(k, '}');
            arreglos.push(gotv_engine::tr!("se cerró un objeto antes de «]» (línea {})", "an object was closed before «]» (line {})", e.line()));
        } else if msg.starts_with("expected `,` or `]`") && txt[pos.saturating_sub(1)..].trim_start().starts_with('}') {
            let k = pos.saturating_sub(1) + txt[pos.saturating_sub(1)..].find('}').unwrap_or(0);
            txt.insert(k, ']');
            arreglos.push(gotv_engine::tr!("se cerró una lista antes de «}}» (línea {})", "a list was closed before «}}» (line {})", e.line()));
        } else if msg.starts_with("expected `,` or `}`") || msg.starts_with("expected `,` or `]`") {
            // faltó una coma entre dos elementos
            txt.insert(antes.len(), ',');
            arreglos.push(gotv_engine::tr!("se puso una coma que faltaba (línea {})", "a missing comma was added (line {})", e.line()));
        } else {
            break;
        }
    }
    Err(original)
}

/// Lo que usa GOTV con fichas de modelos: normaliza, avisa y sólo falla si no se puede pintar.
/// Quita lo VACÍO (`null`, `""`, `[]`, `{}`) que deja serializar una ficha: releída, el compilador
/// tolerante lo reportaba como error («marca.icono: «» no existe»), ruido para el modelo.
fn podar(v: &mut Value) {
    match v {
        Value::Object(m) => {
            for x in m.values_mut() {
                podar(x);
            }
            m.retain(|_, x| !(x.is_null() || x.as_str() == Some("") || x.as_array().is_some_and(|a| a.is_empty()) || x.as_object().is_some_and(|o| o.is_empty())));
        }
        Value::Array(a) => a.iter_mut().for_each(podar),
        _ => {}
    }
}

/// Pone `data-gotv="i"` en la etiqueta raíz de una sección ya pintada.
fn marcar(html: &str, i: usize, tipo: &str) -> String {
    let Some(ini) = html.find('<') else { return html.to_string() };
    let fin = html[ini + 1..].find(|c: char| !c.is_ascii_alphanumeric()).map(|n| ini + 1 + n).unwrap_or(html.len());
    if fin == ini + 1 {
        return html.to_string();
    }
    format!("{} data-gotv=\"{i}\" data-gotv-tipo=\"{}\"{}", &html[..fin], esc(tipo), &html[fin..])
}

/// D6: aplica un CAMBIO a una ficha guardada — sólo los campos que cambian. `seccion` = índice en
/// `secciones` (el de `data-gotv`); sin sección, los campos van a la ficha (titulo, direccion…).
/// Las claves de `campos` reemplazan a las de la sección; `null` borra una. El resultado pasa
/// por el compilador TOLERANTE (avisos incluidos). Con un 9B, reescribir la ficha entera para
/// cambiar un titular es donde se rompe; mandar `{"titulo":"…"}` no.
pub fn aplicar_cambio(ficha: &Ficha, seccion: Option<usize>, campos: &Value) -> Result<(Ficha, Vec<String>), Vec<String>> {
    let Some(campos) = campos.as_object() else {
        return Err(vec![gotv_engine::tr!("`campos` debe ser un objeto con los campos que cambian, p. ej. {{\"titulo\":\"…\"}}", "`campos` must be an object with the fields that change, e.g. {{\"titulo\":\"…\"}}")]);
    };
    let mut v = serde_json::to_value(ficha).map_err(|e| vec![e.to_string()])?;
    podar(&mut v);
    let destino = match seccion {
        None => &mut v,
        Some(i) => {
            let n = v["secciones"].as_array().map(|a| a.len()).unwrap_or(0);
            if i >= n {
                return Err(vec![gotv_engine::tr!("la ficha tiene {n} secciones (0 a {}); no hay sección {i}", "the spec has {n} sections (0 to {}); there is no section {i}", n.saturating_sub(1))]);
            }
            &mut v["secciones"][i]
        }
    };
    let obj = destino.as_object_mut().ok_or_else(|| vec![gotv_engine::tr!("la sección no es un objeto", "the section is not an object")])?;
    for (k, val) in campos {
        if seccion.is_some() && k == "tipo" && obj.get("tipo") != Some(val) {
            return Err(vec![gotv_engine::tr!("no se cambia el `tipo` de una sección con un cambio: manda la ficha completa", "a section's `tipo` is not changed with a change: send the whole spec")]);
        }
        if val.is_null() {
            obj.remove(k);
        } else {
            obj.insert(k.clone(), val.clone());
        }
    }
    validar_tolerante(&v.to_string())
}

/// D11 (medido): el 9B intentó once veces editar ficha.json a mano y jamás usó `cambio`: no sabía
/// el NÚMERO de cada sección. El mapa: «0: navegacion · 1: heroe «Tu bici…» · 2: caracteristicas…».
pub fn mapa_secciones(ficha: &Ficha) -> String {
    ficha
        .secciones
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let v = serde_json::to_value(s).unwrap_or_default();
            let tipo = v["tipo"].as_str().unwrap_or("?").to_string();
            match v["titulo"].as_str().filter(|t| !t.trim().is_empty()) {
                Some(t) => format!("{i}: {tipo} «{}»", t.chars().take(40).collect::<String>()),
                None => format!("{i}: {tipo}"),
            }
        })
        .collect::<Vec<_>>()
        .join(" · ")
}

/// D11 (medido): el 9B puso el teléfono y el correo sólo en el `href` («Teléfono», «Correo») y el
/// visitante no los veía. Un enlace `tel:`/`mailto:` cuyo texto no muestra el dato lo muestra.
fn contacto_visible(v: &mut Value, avisos: &mut Vec<String>) {
    match v {
        Value::Object(m) => {
            let href = m.get("href").and_then(|h| h.as_str()).unwrap_or("").to_string();
            let dato = href.strip_prefix("tel:").or_else(|| href.strip_prefix("mailto:")).map(|d| d.split('?').next().unwrap_or(d).to_string());
            if let (Some(dato), Some(Value::String(t))) = (dato.filter(|d| !d.is_empty()), m.get_mut("texto")) {
                let solo = |x: &str| x.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '@').collect::<String>().to_lowercase();
                if !solo(t).contains(&solo(&dato)) {
                    let antes = t.clone();
                    *t = if antes.trim().is_empty() { dato.clone() } else { format!("{}: {dato}", antes.trim()) };
                    avisos.push(gotv_engine::tr!("enlace «{antes}» ({href}): el dato no se veía; ahora dice «{t}»", "link «{antes}» ({href}): the value was not visible; now it says «{t}»"));
                }
            }
            for x in m.values_mut() {
                contacto_visible(x, avisos);
            }
        }
        Value::Array(a) => a.iter_mut().for_each(|x| contacto_visible(x, avisos)),
        _ => {}
    }
}

pub fn validar_tolerante(json_txt: &str) -> Result<(Ficha, Vec<String>), Vec<String>> {
    let (v, reparos) = match reparar_json(json_txt) {
        Ok(x) => x,
        Err(e) => return Err(vec![gotv_engine::tr!("el JSON no se puede leer: {e} (revisa comas, comillas y llaves)", "the JSON cannot be read: {e} (check commas, quotes and braces)")]),
    };
    if !v.is_object() {
        return Err(vec![gotv_engine::tr!("la ficha debe ser un objeto JSON {{ … }}", "the spec must be a JSON object {{ … }}")]);
    }
    let (mut n, mut avisos) = normalizar(v);
    contacto_visible(&mut n, &mut avisos);
    avisos.splice(0..0, reparos.into_iter().map(|r| format!("JSON: {r}")));
    validar(&n.to_string()).map(|f| (f, avisos))
}

/// Ficha de muestra con TODAS las secciones: la usa `gotv pagina --muestra <dirección>` y
/// las pruebas (cada dirección debe pasar HIGIENE y OFICIO con ella).
pub fn ficha_muestra(dir: &str) -> Ficha {
    // la página de ejemplo (en español, `idioma: "es"`) vive como dato en recursos/muestra.json
    let json = include_str!("../recursos/muestra.json").replace("__DIRECCION__", dir);
    serde_json::from_str(&json).unwrap_or_else(|e| panic!("{}", gotv_engine::tr!("la ficha de muestra no es válida: {e}", "the sample spec is not valid: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Carpeta del temporal que se borra sola al soltarse, pase o falle la prueba. Es la
    /// misma idea que `gotv_engine::prueba::CarpetaPrueba` (gotv-diseno no depende del
    /// motor): borra SÓLO `temp_dir()/<base>-<pid>-<n>`, jamás por patrón (falla #2571).
    struct CarpetaPrueba(std::path::PathBuf);

    impl CarpetaPrueba {
        fn sin_crear(base: &str) -> Self {
            assert!(base.starts_with("gotv-") && !base.contains(['/', '\\']) && !base.contains(".."));
            // pid + contador: dos pruebas paralelas con la misma base no comparten carpeta
            static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
            let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let c = Self(std::env::temp_dir().join(format!("{base}-{}-{n}", std::process::id())));
            let _ = std::fs::remove_dir_all(&c.0);
            c
        }
    }

    impl Drop for CarpetaPrueba {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    impl std::ops::Deref for CarpetaPrueba {
        type Target = std::path::Path;
        fn deref(&self) -> &std::path::Path {
            &self.0
        }
    }

    impl AsRef<std::path::Path> for CarpetaPrueba {
        fn as_ref(&self) -> &std::path::Path {
            &self.0
        }
    }

    #[test]
    fn escapa_y_filtra() {
        assert_eq!(esc("<a href=\"x\">&'"), "&lt;a href=&quot;x&quot;&gt;&amp;&#39;");
        assert_eq!(realce("Tus *datos* <b>"), "Tus <em class=\"realce\">datos</em> &lt;b&gt;");
        assert_eq!(realce("3 * 4"), "3 * 4");
        assert_eq!(href("javascript:alert(1)"), "#");
        assert_eq!(href(" JaVaScRiPt:alert(1)"), "#");
        assert_eq!(href("data:text/html,x"), "#");
        assert_eq!(href("#precios"), "#precios");
        assert_eq!(href("https://ejemplo.com/?a=1&b=2"), "https://ejemplo.com/?a=1&amp;b=2");
        assert_eq!(href("contacto.html"), "contacto.html");
        assert_eq!(ancla(" Cómo funciona! "), "como-funciona");
    }

    #[test]
    fn cada_direccion_arma_su_muestra_con_contraste_garantizado() {
        for d in DIRECCIONES {
            let p = Pagina::new(ficha_muestra(d.id)).unwrap();
            assert!(p.tema.incumplidas().is_empty(), "{}", d.id);
            let h = p.html_suelto();
            assert!(h.contains(&format!("data-direccion=\"{}\"", d.id)));
            assert_eq!(h.matches("<h1").count(), 1, "{}: un solo h1", d.id);
            assert!(h.contains("Saltar al contenido") && h.contains("<main id=\"contenido\">"));
            assert!(!h.contains("<script>alert"));
        }
    }

    #[test]
    fn la_ficha_rechaza_campos_inventados_con_su_nombre() {
        let mala = r#"{"direccion":"tecnico","titulo":"x","marca":{"nombre":"x"},"secciones":[{"tipo":"heroe","titulo":"x","boton":"y"}]}"#;
        let e = serde_json::from_str::<Ficha>(mala).unwrap_err().to_string();
        assert!(e.contains("boton"), "{e}");
        let mala = r#"{"direccion":"tecnico","titulo":"x","marca":{"nombre":"x"},"secciones":[{"tipo":"carrusel"}]}"#;
        let e = serde_json::from_str::<Ficha>(mala).unwrap_err().to_string();
        assert!(e.contains("carrusel"), "{e}");
        let f: Ficha = serde_json::from_str(r#"{"direccion":"otra","titulo":"x","marca":{"nombre":"x"},"secciones":[]}"#).unwrap();
        let e = Pagina::new(f).err().unwrap().to_string();
        assert!(e.contains("tecnico") && e.contains("brutalista"), "{e}");
    }

    #[test]
    fn validar_da_todos_los_errores_con_su_ruta() {
        let mala = r#"{"direccion":"tecnica","titulo":"x","marca":{"nombre":"N","icono":"rocet"},"tema":{"croma":"brillante","fuentes":"comic"},"extra":1,
          "secciones":[{"tipo":"heroe","titulo":"Hola","boton":"y"},{"tipo":"carrusel"},{"tipo":"caracteristicas","variante":"cuadricula","items":[{"titulo":"a","icono":"rayito"}]},{"titulo":"sin tipo"}]}"#;
        let e = validar(mala).unwrap_err();
        let todo = e.join("\n");
        for esperado in [
            "direccion: «tecnica» no existe",
            "marca.icono: el icono «rocet» no existe; parecidos: rocket",
            "tema.croma: «brillante» no es válido",
            "tema.fuentes: «comic» no existe",
            "«extra» no es un campo",
            "secciones[0] (heroe): el campo «boton» no existe aquí",
            "secciones[1] (carrusel): el tipo «carrusel» no existe",
            "secciones[2] (caracteristicas).variante: «cuadricula» no es válido",
            "secciones[2] (caracteristicas).items[0].icono: el icono «rayito» no existe",
            "secciones[3]: falta «tipo»",
        ] {
            assert!(todo.contains(esperado), "falta «{esperado}» en:\n{todo}");
        }
        assert!(e.len() >= 10, "{todo}");
        assert!(validar("{ roto").unwrap_err()[0].contains("no se puede leer"));
        let sin_heroe = r#"{"direccion":"tecnico","titulo":"x","marca":{"nombre":"N"},"secciones":[{"tipo":"logos","nombres":["a"]}]}"#;
        assert!(validar(sin_heroe).unwrap_err().join(" ").contains("falta un «heroe»"));
        // la muestra de cada dirección es válida
        for d in DIRECCIONES {
            let j = serde_json::to_string(&ficha_muestra(d.id)).unwrap();
            assert!(validar(&j).is_ok(), "{}: {:?}", d.id, validar(&j).err());
        }
    }

    #[test]
    fn el_bloque_libre_rechaza_lo_peligroso_y_pinta_lo_sano() {
        let base = |html: &str| format!(r#"{{"direccion":"tecnico","titulo":"x","marca":{{"nombre":"N"}},"secciones":[{{"tipo":"heroe","titulo":"Hola"}},{{"tipo":"libre","html":{}}}]}}"#, serde_json::to_string(html).unwrap());
        for malo in ["<script>alert(1)</script>", "<a href=\"javascript:x\">a</a>", "<div onclick=\"x()\">a</div>", "<img src=x onerror = y>", "<h1>otro</h1>", "<iframe src=x></iframe>"] {
            assert!(validar(&base(malo)).is_err(), "dejó pasar: {malo}");
        }
        let f = validar(&base("<div class=\"tarjeta\"><h2>Mapa de sedes</h2><p>Texto</p></div>")).unwrap();
        let h = Pagina::new(f).unwrap().html_suelto();
        assert!(h.contains("Mapa de sedes") && h.contains("seccion libre"));
        // «onda» o «once» no son eventos
        assert!(validar(&base("<p>Una onda de once metros</p>")).is_ok());
    }

    #[test]
    fn recompilar_no_pisa_lo_editado_a_mano() {
        let d = CarpetaPrueba::sin_crear("gotv-pagina");
        let a = vec![("index.html".to_string(), b"v1".to_vec()), ("assets/estilos.css".to_string(), b"css1".to_vec())];
        let i = escribir_proyecto(&d, &a).unwrap();
        assert_eq!(i.escritos.len(), 2);
        // el usuario edita index.html a mano
        std::fs::write(d.join("index.html"), b"v1 + mi cambio").unwrap();
        let b = vec![("index.html".to_string(), b"v2".to_vec()), ("assets/estilos.css".to_string(), b"css2".to_vec())];
        let i = escribir_proyecto(&d, &b).unwrap();
        assert_eq!(std::fs::read(d.join("index.html")).unwrap(), b"v1 + mi cambio", "pisó lo editado");
        assert_eq!(std::fs::read(d.join("index.gotv-nuevo.html")).unwrap(), b"v2");
        assert_eq!(i.respetados.len(), 1);
        assert_eq!(std::fs::read(d.join("assets/estilos.css")).unwrap(), b"css2", "lo no editado sí se actualiza");
        // un archivo que ya existía ANTES de GOTV tampoco se pisa
        let e = d.join("otro");
        std::fs::create_dir_all(&e).unwrap();
        std::fs::write(e.join("index.html"), b"del usuario").unwrap();
        escribir_proyecto(&e, &a).unwrap();
        assert_eq!(std::fs::read(e.join("index.html")).unwrap(), b"del usuario");
        assert!(escribir_proyecto(&d, &[("../fuera".to_string(), vec![])]).is_err());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn el_heroe_lleva_una_isometrica_con_los_colores_de_la_marca() {
        let j = r##"{"direccion":"tecnico","tema":{"marca":"#16a34a"},"titulo":"x","marca":{"nombre":"N"},"secciones":[{"tipo":"heroe","titulo":"Hola","arte":{"titulo":"Mi sistema","piezas":[{"tipo":"caja","x":0,"y":0,"ancho":2,"fondo":2,"alto":1,"color":"acento","etiqueta":"API"}]}}]}"##;
        let h = Pagina::new(validar(j).unwrap()).unwrap().html_suelto();
        assert!(h.contains("<title>Mi sistema</title>") && h.contains("class=\"arte\""), "la escena no llegó al héroe");
        assert!(h.contains("style=\"fill:var(--texto)\""), "las etiquetas de la escena deben seguir al modo claro/oscuro");
        let mala = r#"{"direccion":"tecnico","titulo":"x","marca":{"nombre":"N"},"secciones":[{"tipo":"heroe","titulo":"Hola","arte":{"piezas":[{"tipo":"esfera"}]}}]}"#;
        assert!(validar(mala).unwrap_err().join(" ").contains("secciones[0] (heroe).arte.piezas[0] (esfera)"));
        let mala = r#"{"direccion":"tecnico","titulo":"x","marca":{"nombre":"N"},"secciones":[{"tipo":"heroe","titulo":"Hola","arte":"foto"}]}"#;
        assert!(validar(mala).unwrap_err().join(" ").contains("arte: «foto» no es válido"));
    }

    #[test]
    fn la_tolerancia_arregla_lo_que_el_9b_escribio() {
        // errores REALES del 9B (target/banco/fichas-20261109-1810): emojis, tipos inventados,
        // campos fuera de lugar, objeto donde va texto, piezas inventadas, cta en precios
        let j = r##"{"direccion":"lujoso","titulo":"Grano Alto | Café","marca":{"nombre":"Grano Alto","icono":"☕"},"tema":{"croma":"brillante"},"secciones":[
          {"tipo":"heroe","titulo":"Café de altura","piezas":[{"tipo":"orbe"},{"tipo":"caja","x":0,"y":0,"ancho":1,"fondo":1,"alto":1}]},
          {"tipo":"caracteristicas","items":[{"titulo":"Tueste propio","icono":"🔥","dia":"lunes"},{"nombre":"Brunch","descripcion":"Sábados","icono":"🥑"}]},
          {"tipo":"suscripcion","titulo":"Planes","cta":{"texto":"x"},"planes":[{"nombre":"Mensual","precio":"$45.000","incluye":"2 bolsas, envío"}]},
          {"tipo":"sobre-mi","titulo":"Quiénes somos","texto":"Tostadores desde 2012"},
          {"tipo":"testimonios","items":[{"cita":"El mejor café","nombre":"Ana","icono":"⭐"}]},
          {"tipo":"logos","texto":"x","nombres":["A","B"]},
          {"tipo":"contacto","titulo":"Escríbenos","boton":{"texto":"Enviar","href":"#"}},
          {"tipo":"pie","legal":true}]}"##;
        let (f, avisos) = validar_tolerante(j).expect("debe salir pintable");
        assert_eq!(f.direccion, "lujo");
        assert_eq!(f.marca.icono.as_deref(), Some("coffee"));
        let tipos: Vec<String> = serde_json::to_value(&f.secciones).unwrap().as_array().unwrap().iter().map(|s| s["tipo"].as_str().unwrap().to_string()).collect();
        assert_eq!(tipos, ["heroe", "caracteristicas", "precios", "caracteristicas", "testimonios", "logos", "contacto", "pie"]);
        let todo = avisos.join("\n");
        for a in ["«suscripcion» no existe; se armó como «precios»", "se ignoró el campo «dia»", "se quitaron 1 pieza(s)", "tema.croma"] {
            assert!(todo.contains(a), "falta el aviso «{a}» en:\n{todo}");
        }
        let h = Pagina::new(f).unwrap().html_suelto();
        assert!(h.contains("Tueste propio") && h.contains("$45.000") && h.contains("envío") && h.contains("Tostadores desde 2012"));
        // lo que NO se puede pintar sigue fallando
        assert!(validar_tolerante("{ roto").is_err());
        assert!(validar_tolerante("[1,2]").is_err());
    }

    #[test]
    fn el_json_roto_de_un_modelo_se_repara() {
        // el caso real del 9B (ficha 04): una sección metida como campo y el cierre desajustado
        let roto = r##"{"direccion":"tecnico","titulo":"Hierro 24","marca":{"nombre":"Hierro 24"},"secciones":[
          {"tipo":"heroe","titulo":"Entrena a cualquier hora"},
          {"tipo":"testimonios","items":[{"cita":"Me siento una bestia","nombre":"Sara"}],
           "precios":{"planes":[{"nombre":"Pase","precio":"29€","incluye":["24/7"]}]},
          {"tipo":"llamada","titulo":"¿Listo?","cta":{"texto":"Empieza","href":"#"}}
        ]}"##;
        let (f, avisos) = validar_tolerante(roto).expect("debe repararse");
        let tipos: Vec<String> = serde_json::to_value(&f.secciones).unwrap().as_array().unwrap().iter().map(|s| s["tipo"].as_str().unwrap().to_string()).collect();
        assert_eq!(tipos, ["heroe", "testimonios", "precios", "llamada"], "{avisos:?}");
        assert!(avisos.iter().any(|a| a.starts_with("JSON:")), "{avisos:?}");
        // coma sobrante, final cortado
        assert!(reparar_json(r#"{"a":[1,2,],"b":{"c":1,}"#).is_ok());
        // basura no se «repara» en silencio
        assert!(reparar_json("esto no es json").is_err());
    }

    #[test]
    fn el_esquema_nombra_todo_lo_valido() {
        let e = esquema();
        for t in TIPOS {
            assert!(e.contains(&format!(" {t}")), "el esquema no nombra «{t}»");
        }
        for d in DIRECCIONES {
            assert!(e.contains(d.id));
        }
        assert!(e.len() < 3500, "el esquema debe caber barato en el contexto: {} caracteres", e.len());
    }

    #[test]
    fn el_proyecto_trae_fuentes_y_licencias() {
        let p = Pagina::new(ficha_muestra("editorial")).unwrap();
        let a = p.archivos();
        let rutas: Vec<&str> = a.iter().map(|(r, _)| r.as_str()).collect();
        assert!(rutas.contains(&"index.html") && rutas.contains(&"assets/estilos.css"));
        assert!(rutas.contains(&"assets/fuentes/fraunces.woff2") && rutas.contains(&"assets/fuentes/newsreader.woff2"));
        let lic = String::from_utf8(a.iter().find(|(r, _)| r == "assets/LICENCIAS.txt").unwrap().1.clone()).unwrap();
        assert!(lic.contains("SIL Open Font License") && lic.contains("ISC"));
    }

    #[test]
    fn d6_secciones_marcadas_ficha_guardada_y_cambio_por_seccion() {
        let f = ficha_muestra("tecnico");
        let n = f.secciones.len();
        let p = Pagina::new(f.clone()).unwrap();
        let html = p.html_suelto();
        for i in 0..n {
            assert!(html.contains(&format!(" data-gotv=\"{i}\"")), "falta la marca de la sección {i}");
        }
        // la ficha viaja con la página y vuelve igual
        let a = p.archivos();
        let guardada = String::from_utf8(a.iter().find(|(r, _)| r == "ficha.json").unwrap().1.clone()).unwrap();
        let (vuelta, avisos) = validar_tolerante(&guardada).unwrap();
        assert!(avisos.is_empty(), "releer la ficha guardada no debe dar avisos: {avisos:?}");
        assert_eq!(serde_json::to_value(&vuelta).unwrap(), serde_json::to_value(&f).unwrap());
        // cambio de UNA sección: sólo sus campos; el resto intacto
        let v = serde_json::to_value(&f).unwrap();
        let h = v["secciones"].as_array().unwrap().iter().position(|x| x["tipo"] == "heroe").unwrap();
        let (nueva, avisos) = aplicar_cambio(&f, Some(h), &serde_json::json!({"titulo": "Café de altura, tostado esta semana"})).unwrap();
        assert!(avisos.is_empty(), "un cambio limpio no da avisos: {avisos:?}");
        let nv = serde_json::to_value(&nueva).unwrap();
        assert_eq!(nv["secciones"][h]["titulo"], "Café de altura, tostado esta semana");
        for i in (0..n).filter(|i| *i != h) {
            assert_eq!(nv["secciones"][i], v["secciones"][i], "la sección {i} no debía cambiar");
        }
        assert!(Pagina::new(nueva).unwrap().html_suelto().contains("Café de altura, tostado esta semana"));
        // cambio de la ficha (sin sección): otra dirección, sin modelo
        let (otra, _) = aplicar_cambio(&f, None, &serde_json::json!({"direccion": "lujo"})).unwrap();
        assert_eq!(otra.direccion, "lujo");
        // errores con causa
        assert!(aplicar_cambio(&f, Some(n), &serde_json::json!({"titulo": "x"})).unwrap_err()[0].contains(&format!("{n} secciones")));
        assert!(aplicar_cambio(&f, Some(h), &serde_json::json!({"tipo": "faq"})).is_err());
        assert!(aplicar_cambio(&f, Some(h), &serde_json::json!("texto")).is_err());
        assert_eq!(marcar("<section class=\"x\">", 3, "faq"), "<section data-gotv=\"3\" data-gotv-tipo=\"faq\" class=\"x\">");
        assert_eq!(marcar("texto sin etiqueta", 1, "faq"), "texto sin etiqueta");
        assert!(html.contains(&format!("data-gotv=\"{h}\" data-gotv-tipo=\"heroe\"")));
    }

    #[test]
    fn d11_el_contacto_se_ve_y_ficha_json_siempre_se_actualiza() {
        let f = r#"{"direccion":"suizo","titulo":"El Pedal — taller","marca":{"nombre":"El Pedal"},"secciones":[
            {"tipo":"heroe","titulo":"Tu bici"},
            {"tipo":"pie","columnas":[{"titulo":"Contacto","enlaces":[{"texto":"Teléfono","href":"tel:3105552847"},{"texto":"taller@elpedal.co","href":"mailto:taller@elpedal.co"}]}]}]}"#;
        let (ficha, avisos) = validar_tolerante(f).unwrap();
        let html = Pagina::new(ficha.clone()).unwrap().html_suelto();
        assert!(html.contains("Teléfono: 3105552847"), "el número se ve");
        assert_eq!(avisos.iter().filter(|a| a.contains("no se veía")).count(), 1, "el correo ya se veía: {avisos:?}");
        // ficha.json editada a mano: se ACTUALIZA (es la fuente), no se crea ficha.gotv-nuevo.json
        let d = CarpetaPrueba::sin_crear("gotv-ficha");
        let p = Pagina::new(ficha).unwrap();
        escribir_proyecto(&d, &p.archivos()).unwrap();
        std::fs::write(d.join("ficha.json"), "{\"a mano\": true}").unwrap();
        let inf = escribir_proyecto(&d, &p.archivos()).unwrap();
        assert!(inf.respetados.iter().all(|(r, _)| r != "ficha.json") && !d.join("ficha.gotv-nuevo.json").exists());
        assert!(std::fs::read_to_string(d.join("ficha.json")).unwrap().contains("El Pedal"));
        let _ = std::fs::remove_dir_all(&d);
    }
}

