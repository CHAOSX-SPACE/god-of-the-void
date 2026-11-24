//! Ola PRO · el modo profesional de los perfiles Imagen, Video y Música (PLAN.md, «Ola PRO»).
//!
//! Con el switch encendido, la interfaz manda junto a la orden los ajustes del perfil y GOTV llama
//! DIRECTO al generador con ellos (el mismo camino que el video directo): el modelo sólo pasa el
//! texto de la persona a un prompt en inglés. Aquí vive lo que NO depende del modelo: el vocabulario
//! (cada opción con su nombre para la persona y el fragmento en inglés que entiende el peso), el
//! ensamblado del prompt y los tamaños. Se ofrece sólo lo que el peso hace de verdad (PRO-0, PRO-2).

use serde::{Deserialize, Serialize};

/// Una opción de un menú: la clave que se guarda, su nombre (en el idioma de la app) y lo que se
/// añade al prompt en inglés.
#[derive(Serialize, Clone, Debug)]
pub struct Opcion {
    pub clave: &'static str,
    pub nombre: &'static str,
    #[serde(skip)]
    pub prompt: &'static str,
}

fn op(clave: &'static str, nombre: &'static str, prompt: &'static str) -> Opcion {
    Opcion { clave, nombre, prompt }
}

/// Wan 2.2 se entrenó con etiquetas de luz, composición, contraste y tono de color: el vocabulario
/// de cine de su guía (plano, ángulo, lente, movimiento, luz, hora, tono, estilo) sí mueve el video.
pub fn planos() -> Vec<Opcion> {
    vec![
        op("gran-general", crate::i18n::t("plano general extremo", "extreme wide shot"), "extreme wide shot"),
        op("general", crate::i18n::t("plano general", "wide shot"), "wide shot"),
        op("medio", crate::i18n::t("plano medio", "medium shot"), "medium shot"),
        op("medio-corto", crate::i18n::t("plano medio corto", "medium close-up"), "medium close-up shot"),
        op("primer", crate::i18n::t("primer plano", "close-up"), "close-up shot"),
        op("detalle", crate::i18n::t("plano detalle", "extreme close-up"), "extreme close-up shot"),
    ]
}

pub fn angulos() -> Vec<Opcion> {
    vec![
        op("ojos", crate::i18n::t("a la altura de los ojos", "eye level"), "eye-level shot"),
        op("picado", crate::i18n::t("picado (desde arriba)", "high angle"), "high-angle shot"),
        op("contrapicado", crate::i18n::t("contrapicado (desde abajo)", "low angle"), "low-angle shot"),
        op("cenital", crate::i18n::t("cenital", "top-down"), "top-down overhead shot"),
        op("aereo", crate::i18n::t("aéreo (dron)", "aerial (drone)"), "aerial drone shot"),
        op("holandes", crate::i18n::t("holandés (inclinado)", "dutch angle"), "dutch angle"),
        op("hombro", crate::i18n::t("sobre el hombro", "over the shoulder"), "over-the-shoulder shot"),
    ]
}

pub fn lentes() -> Vec<Opcion> {
    vec![
        op("gran-angular", crate::i18n::t("gran angular", "wide angle"), "wide-angle lens"),
        op("normal", crate::i18n::t("normal (50 mm)", "normal (50 mm)"), "50mm lens"),
        op("tele", crate::i18n::t("teleobjetivo", "telephoto"), "telephoto lens, compressed background"),
        op("macro", crate::i18n::t("macro", "macro"), "macro lens, extreme detail"),
        op("ojo-de-pez", crate::i18n::t("ojo de pez", "fisheye"), "fisheye lens"),
        op("anamorfico", crate::i18n::t("anamórfico", "anamorphic"), "anamorphic lens, cinematic widescreen look"),
        op("bokeh", crate::i18n::t("poca profundidad (bokeh)", "shallow depth (bokeh)"), "shallow depth of field, soft bokeh"),
    ]
}

/// El movimiento sin la velocidad: la velocidad se antepone («slow camera pushes in»).
pub fn movimientos() -> Vec<Opcion> {
    vec![
        op("fija", crate::i18n::t("cámara fija", "static camera"), "static camera, locked-off shot"),
        op("acercar", crate::i18n::t("acercamiento (push-in)", "push in"), "camera pushes in toward the subject"),
        op("alejar", crate::i18n::t("alejamiento (pull-out)", "pull out"), "camera pulls back away from the subject"),
        op("pan-izq", crate::i18n::t("paneo a la izquierda", "pan left"), "camera pans left"),
        op("pan-der", crate::i18n::t("paneo a la derecha", "pan right"), "camera pans right"),
        op("tilt-arriba", crate::i18n::t("inclinación hacia arriba", "tilt up"), "camera tilts up"),
        op("tilt-abajo", crate::i18n::t("inclinación hacia abajo", "tilt down"), "camera tilts down"),
        op("seguimiento", crate::i18n::t("travelling (sigue al sujeto)", "tracking shot"), "tracking shot following the subject"),
        op("orbita", crate::i18n::t("órbita alrededor", "orbit"), "camera orbits around the subject"),
        op("grua", crate::i18n::t("grúa (sube)", "crane up"), "crane shot rising upward"),
        op("mano", crate::i18n::t("cámara en mano", "handheld"), "handheld camera, natural shake"),
        op("zoom", crate::i18n::t("zoom de lente", "lens zoom in"), "zoom in"),
    ]
}

pub fn velocidades() -> Vec<Opcion> {
    vec![op("lenta", crate::i18n::t("lenta", "slow"), "slow"), op("normal", crate::i18n::t("normal", "normal"), ""), op("rapida", crate::i18n::t("rápida", "fast"), "fast")]
}

pub fn luces() -> Vec<Opcion> {
    vec![
        op("suave", crate::i18n::t("suave y difusa", "soft, diffused"), "soft diffused lighting"),
        op("dura", crate::i18n::t("dura, sombras marcadas", "hard, crisp shadows"), "hard lighting with crisp shadows"),
        op("contraluz", crate::i18n::t("contraluz", "backlight"), "backlighting"),
        op("borde", crate::i18n::t("luz de borde (rim)", "rim light"), "rim lighting"),
        op("silueta", crate::i18n::t("silueta", "silhouette"), "silhouette against the light"),
        op("volumetrica", crate::i18n::t("rayos volumétricos", "volumetric rays"), "volumetric light rays through haze"),
        op("neon", crate::i18n::t("neón", "neon"), "neon lighting"),
        op("fuego", crate::i18n::t("fuego / velas", "firelight / candles"), "warm firelight"),
        op("estudio", crate::i18n::t("estudio", "studio"), "professional studio lighting"),
    ]
}

pub fn horas() -> Vec<Opcion> {
    vec![
        op("amanecer", crate::i18n::t("amanecer", "dawn"), "at dawn"),
        op("dorada", crate::i18n::t("hora dorada", "golden hour"), "golden hour"),
        op("mediodia", crate::i18n::t("mediodía", "midday"), "harsh midday sun"),
        op("atardecer", crate::i18n::t("atardecer", "sunset"), "at sunset"),
        op("azul", crate::i18n::t("hora azul", "blue hour"), "blue hour"),
        op("noche", crate::i18n::t("noche", "night"), "at night"),
        op("nublado", crate::i18n::t("nublado", "overcast"), "overcast day"),
    ]
}

pub fn tonos() -> Vec<Opcion> {
    vec![
        op("calido", crate::i18n::t("cálido", "warm"), "warm color tone"),
        op("frio", crate::i18n::t("frío", "cool"), "cool color tone"),
        op("saturado", crate::i18n::t("saturado", "saturated"), "vivid saturated colors"),
        op("desaturado", crate::i18n::t("desaturado", "desaturated"), "desaturated muted colors"),
        op("teal-naranja", crate::i18n::t("teal y naranja", "teal & orange"), "teal and orange color grading"),
        op("pastel", crate::i18n::t("pastel", "pastel"), "soft pastel palette"),
        op("bn", crate::i18n::t("blanco y negro", "black & white"), "black and white"),
    ]
}

pub fn estilos_video() -> Vec<Opcion> {
    vec![
        op("cine", crate::i18n::t("cinematográfico", "cinematic"), "cinematic, film look"),
        op("documental", crate::i18n::t("documental", "documentary"), "documentary style"),
        op("anime", crate::i18n::t("anime", "anime"), "anime style"),
        op("3d", crate::i18n::t("animación 3D", "3D animation"), "3D cartoon animation"),
        op("plastilina", crate::i18n::t("plastilina (stop-motion)", "claymation"), "claymation stop-motion"),
        op("acuarela", crate::i18n::t("acuarela", "watercolor"), "watercolor painting style"),
        op("pixel", crate::i18n::t("pixel art", "pixel art"), "pixel art"),
        op("miniatura", crate::i18n::t("miniatura (tilt-shift)", "tilt-shift miniature"), "tilt-shift miniature effect"),
        op("timelapse", crate::i18n::t("time-lapse", "time-lapse"), "time-lapse"),
        op("lenta", crate::i18n::t("cámara lenta", "slow motion"), "slow motion"),
        op("vhs", crate::i18n::t("VHS de los 80", "80s VHS"), "1980s VHS footage"),
    ]
}

/// Formato y calidad del video. 720p en 16:9 y 9:16 es lo OFICIAL del 5B (`SUPPORTED_SIZES`); el
/// resto va fuera de lo entrenado y se marca «experimental» hasta medirlo.
pub fn formatos() -> Vec<Opcion> {
    vec![op("16:9", crate::i18n::t("16:9 horizontal", "16:9 landscape"), ""), op("9:16", crate::i18n::t("9:16 vertical", "9:16 portrait"), ""), op("1:1", crate::i18n::t("1:1 cuadrado", "1:1 square"), "")]
}

pub fn calidades() -> Vec<Opcion> {
    vec![op("720p", crate::i18n::t("720p (oficial)", "720p (official)"), ""), op("480p", crate::i18n::t("480p, unas 3 veces más rápido (medido)", "480p, about 3 times faster (measured)"), "")]
}

/// Lado × lado de cada formato y calidad (múltiplos de 32, como pide el pipeline). En imagen→video
/// el área es la que manda (`best_output_size`): la proporción sale de la imagen.
pub fn tamano_video(formato: &str, calidad: &str) -> (u32, u32) {
    match (formato, calidad) {
        ("9:16", "480p") => (480, 832),
        ("1:1", "480p") => (640, 640),
        (_, "480p") => (832, 480),
        ("9:16", _) => (704, 1280),
        ("1:1", _) => (960, 960),
        _ => (1280, 704),
    }
}

/// Los ajustes profesionales del perfil Video. Un campo vacío = lo decide el pedido o el oficial.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct ProVideo {
    pub formato: String,
    pub calidad: String,
    /// 1–5 s; 0 = lo que diga la orden (2 por defecto)
    pub segundos: f32,
    /// 0 = 50 (el oficial)
    pub pasos: u32,
    /// 0 = 5 (el oficial)
    pub guia: f32,
    pub plano: String,
    pub angulo: String,
    pub lente: String,
    pub movimiento: String,
    pub velocidad: String,
    pub luz: String,
    pub hora: String,
    pub tono: String,
    pub estilo: String,
    /// lo que NO debe salir (se suma al negativo oficial salvo `sin_negativo_oficial`)
    pub negativo: String,
    pub sin_negativo_oficial: bool,
    pub semilla: Option<u64>,
    /// el modelo pasa el texto de la persona a un prompt en inglés; apagado, va tal cual
    pub pulir: bool,
}

impl Default for ProVideo {
    fn default() -> Self {
        ProVideo {
            formato: "16:9".into(),
            calidad: "720p".into(),
            segundos: 0.0,
            pasos: 0,
            guia: 0.0,
            plano: String::new(),
            angulo: String::new(),
            lente: String::new(),
            movimiento: String::new(),
            velocidad: String::new(),
            luz: String::new(),
            hora: String::new(),
            tono: String::new(),
            estilo: String::new(),
            negativo: String::new(),
            sin_negativo_oficial: false,
            semilla: None,
            pulir: true,
        }
    }
}

fn fragmento(lista: &[Opcion], clave: &str) -> Option<&'static str> {
    lista.iter().find(|o| o.clave == clave).map(|o| o.prompt).filter(|p| !p.is_empty())
}

impl ProVideo {
    /// Lo estético que se suma al prompt (fórmula de Wan: sujeto + escena + movimiento, y DESPUÉS el
    /// control estético), en inglés y separado por comas. Vacío si no se eligió nada.
    pub fn sufijo(&self) -> String {
        let mut partes: Vec<String> = Vec::new();
        for (lista, clave) in [(planos(), &self.plano), (angulos(), &self.angulo), (lentes(), &self.lente)] {
            if let Some(p) = fragmento(&lista, clave) {
                partes.push(p.into());
            }
        }
        if let Some(m) = fragmento(&movimientos(), &self.movimiento) {
            match fragmento(&velocidades(), &self.velocidad).filter(|_| self.movimiento != "fija") {
                Some(v) => partes.push(format!("{v} {m}")),
                None => partes.push(m.into()),
            }
        }
        for (lista, clave) in [(luces(), &self.luz), (horas(), &self.hora), (tonos(), &self.tono), (estilos_video(), &self.estilo)] {
            if let Some(p) = fragmento(&lista, clave) {
                partes.push(p.into());
            }
        }
        partes.join(", ")
    }

    /// El prompt final: la descripción y, detrás, el control estético.
    pub fn prompt(&self, base: &str) -> String {
        let base = base.trim().trim_end_matches(['.', ',', ';']).trim();
        match self.sufijo() {
            s if s.is_empty() => base.to_string(),
            s if base.is_empty() => s,
            s => format!("{base}. {s}."),
        }
    }

    /// El negativo que va al motor: `None` = el oficial tal cual.
    pub fn negativo(&self, oficial: &str) -> Option<String> {
        let propio = self.negativo.trim();
        match (self.sin_negativo_oficial, propio.is_empty()) {
            (false, true) => None,
            (false, false) => Some(format!("{oficial}，{propio}")),
            (true, _) => Some(propio.to_string()),
        }
    }

    pub fn tamano(&self) -> (u32, u32) {
        tamano_video(&self.formato, &self.calidad)
    }

    pub fn experimental(&self) -> bool {
        // medidos: 720p 16:9 y 9:16 (oficiales), 480p (17,5 min) y 1:1 a 480p (17,2 min); falta 1:1 a 720p
        self.formato == "1:1" && self.calidad != "480p"
    }

    pub fn pasos(&self) -> u32 {
        if self.pasos == 0 { 50 } else { self.pasos.clamp(4, 100) }
    }

    pub fn guia(&self) -> f32 {
        if self.guia <= 0.0 { 5.0 } else { self.guia.clamp(1.0, 15.0) }
    }

    /// Minutos estimados con lo MEDIDO en el M1 Max: 720p y 2 s (V7: 54 s por paso con guía = 2
    /// pasadas; VAE + MP4 ~143 s) y 480p y 2 s (19,7 s por paso, 66 s de VAE: 17,5 min). Entre las dos
    /// medidas el paso escala con los tokens^1,24 (ln(19,7/54) / ln(5070/11440)); el VAE, lineal.
    pub fn minutos_estimados(&self, segundos: f32) -> f32 {
        let (w, h) = self.tamano();
        let tokens = |w: u32, h: u32, s: f32| ((w / 32) * (h / 32)) as f32 * (((s * 24.0).round() / 4.0).floor() + 1.0);
        let base = tokens(1280, 704, 2.0);
        let factor = (tokens(w, h, segundos) / base).powf(1.24);
        let pasada = if self.guia() > 1.0 { 54.0 } else { 27.0 };
        (self.pasos() as f32 * pasada * factor + 143.0 * tokens(w, h, segundos) / base) / 60.0
    }
}

// ── Imagen · FLUX.2 klein 4B (PRO-1). Guía oficial de BFL: el orden importa (sujeto → acción →
// estilo → contexto), cámara/lente/película dan fotorrealismo, los colores hex se atan a OBJETOS y el
// texto a pintar va entre comillas. Sin negativo: el destilado no lo tiene.

pub fn tipos_imagen() -> Vec<Opcion> {
    vec![
        op("foto", crate::i18n::t("fotografía", "photograph"), "photograph"),
        op("ilustracion", crate::i18n::t("ilustración digital", "digital illustration"), "digital illustration"),
        op("3d", crate::i18n::t("render 3D", "3D render"), "3D render, octane style"),
        op("anime", crate::i18n::t("anime", "anime"), "anime illustration"),
        op("oleo", crate::i18n::t("pintura al óleo", "oil painting"), "oil painting on canvas"),
        op("acuarela", crate::i18n::t("acuarela", "watercolor"), "watercolor painting"),
        op("lapiz", crate::i18n::t("dibujo a lápiz", "pencil sketch"), "pencil sketch"),
        op("pixel", crate::i18n::t("pixel art", "pixel art"), "pixel art"),
        op("isometrico", crate::i18n::t("isométrico", "isometric"), "isometric illustration"),
        op("vector", crate::i18n::t("vector plano", "flat vector"), "flat vector illustration"),
        op("grabado", crate::i18n::t("grabado japonés", "woodblock print"), "vintage Japanese woodblock print"),
        op("poster", crate::i18n::t("póster / tipografía", "poster / typography"), "graphic design poster"),
    ]
}

pub fn camaras() -> Vec<Opcion> {
    vec![
        op("hasselblad", crate::i18n::t("Hasselblad X2D (medio formato)", "Hasselblad X2D (medium format)"), "shot on Hasselblad X2D"),
        op("sony", crate::i18n::t("Sony A7IV (digital moderna)", "Sony A7IV (modern digital)"), "shot on Sony A7IV, clean sharp, high dynamic range"),
        op("fuji", crate::i18n::t("Fujifilm X-T5", "Fujifilm X-T5"), "shot on Fujifilm X-T5"),
        op("canon", crate::i18n::t("Canon 5D Mark IV", "Canon 5D Mark IV"), "shot on Canon 5D Mark IV"),
        op("digicam", crate::i18n::t("cámara digital de los 2000", "2000s digicam"), "early digital camera, slight noise, flash photography, 2000s digicam style"),
        op("vintage80", crate::i18n::t("foto de los 80", "80s photo"), "film grain, warm color cast, soft focus, 80s vintage photo"),
        op("polaroid", crate::i18n::t("Polaroid", "Polaroid"), "Polaroid instant photo"),
        op("movil", crate::i18n::t("teléfono móvil", "smartphone"), "shot on a smartphone, candid"),
    ]
}

pub fn lentes_foto() -> Vec<Opcion> {
    vec![
        op("24", crate::i18n::t("24 mm (gran angular)", "24 mm (wide)"), "24mm wide-angle lens"),
        op("35", crate::i18n::t("35 mm", "35 mm"), "35mm lens"),
        op("50", crate::i18n::t("50 mm", "50 mm"), "50mm lens"),
        op("85", crate::i18n::t("85 mm (retrato)", "85 mm (portrait)"), "85mm portrait lens"),
        op("135", crate::i18n::t("135 mm (tele)", "135 mm (tele)"), "135mm telephoto lens"),
        op("macro", crate::i18n::t("macro", "macro"), "macro lens, extreme close detail"),
        op("ojo-de-pez", crate::i18n::t("ojo de pez", "fisheye"), "fisheye lens"),
        op("tilt-shift", crate::i18n::t("tilt-shift", "tilt-shift"), "tilt-shift lens, miniature effect"),
    ]
}

pub fn aperturas() -> Vec<Opcion> {
    vec![
        op("1.4", crate::i18n::t("f/1.4 (fondo muy desenfocado)", "f/1.4 (very blurred background)"), "f/1.4, shallow depth of field, creamy bokeh"),
        op("2.8", crate::i18n::t("f/2.8", "f/2.8"), "f/2.8"),
        op("8", crate::i18n::t("f/8 (todo nítido)", "f/8 (everything sharp)"), "f/8, sharp focus throughout"),
        op("16", crate::i18n::t("f/16 (paisaje)", "f/16 (landscape)"), "f/16, deep depth of field"),
    ]
}

pub fn peliculas() -> Vec<Opcion> {
    vec![
        op("portra", crate::i18n::t("Kodak Portra 400", "Kodak Portra 400"), "shot on Kodak Portra 400, natural grain, organic colors"),
        op("ektar", crate::i18n::t("Kodak Ektar 100", "Kodak Ektar 100"), "shot on Kodak Ektar 100, vivid saturated colors"),
        op("trix", crate::i18n::t("Kodak Tri-X (blanco y negro)", "Kodak Tri-X (B&W)"), "shot on Kodak Tri-X 400, black and white, gritty grain"),
        op("cinestill", crate::i18n::t("Cinestill 800T (noche, halos)", "Cinestill 800T (night, halation)"), "shot on Cinestill 800T, tungsten tones, red halation"),
        op("velvia", crate::i18n::t("Fujifilm Velvia", "Fujifilm Velvia"), "shot on Fujifilm Velvia slide film"),
    ]
}

pub fn composiciones() -> Vec<Opcion> {
    vec![
        op("tercios", crate::i18n::t("regla de tercios", "rule of thirds"), "rule of thirds composition"),
        op("centrada", crate::i18n::t("centrada", "centered"), "centered composition"),
        op("simetrica", crate::i18n::t("simétrica", "symmetrical"), "symmetrical composition"),
        op("vacio", crate::i18n::t("mucho espacio vacío", "negative space"), "minimalist composition with generous negative space"),
        op("plana", crate::i18n::t("vista cenital (flat lay)", "flat lay"), "flat lay, top-down view"),
    ]
}

pub fn proporciones() -> Vec<Opcion> {
    ["1:1", "4:3", "3:4", "3:2", "2:3", "16:9", "9:16", "21:9"].into_iter().map(|p| op(p, p, "")).collect()
}

pub fn resoluciones() -> Vec<Opcion> {
    vec![
        op("0.5", crate::i18n::t("0,5 MP (borrador rápido)", "0.5 MP (quick draft)"), ""),
        op("1", crate::i18n::t("1 MP (normal)", "1 MP (normal)"), ""),
        op("2", crate::i18n::t("2 MP (detalle)", "2 MP (detail)"), ""),
        op("4", crate::i18n::t("4 MP (máximo)", "4 MP (maximum)"), ""),
    ]
}

/// Ancho × alto de una proporción y unos megapíxeles: múltiplos de 16 y cada lado en 256–2048 (lo
/// que admite el motor; a 4 MP las proporciones largas quedan topadas por el lado de 2048).
pub fn tamano_imagen(proporcion: &str, mp: &str) -> (u32, u32) {
    let (a, b) = proporcion.split_once(':').and_then(|(a, b)| Some((a.parse::<f64>().ok()?, b.parse::<f64>().ok()?))).filter(|(a, b)| *a > 0.0 && *b > 0.0).unwrap_or((1.0, 1.0));
    let area = mp.parse::<f64>().ok().filter(|m| (0.1..=4.2).contains(m)).unwrap_or(1.0) * 1_048_576.0;
    let mut w = (area * a / b).sqrt();
    let mut h = w * b / a;
    let tope = w.max(h) / 2048.0;
    if tope > 1.0 {
        w /= tope;
        h /= tope;
    }
    let ajusta = |x: f64| ((x / 16.0).round() as u32 * 16).clamp(256, 2048);
    (ajusta(w), ajusta(h))
}

/// Los ajustes profesionales del perfil Imagen.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct ProImagen {
    pub proporcion: String,
    pub resolucion: String,
    /// 1–4 imágenes (semillas seguidas)
    pub variantes: u32,
    pub tipo: String,
    pub camara: String,
    pub lente: String,
    pub apertura: String,
    pub pelicula: String,
    pub luz: String,
    pub hora: String,
    pub plano: String,
    pub angulo: String,
    pub composicion: String,
    pub estilo_libre: String,
    /// colores atados a objetos: «el coche=#FF0000», o sólo «#1E90FF» (paleta)
    pub colores: Vec<String>,
    /// Ola PRO: hasta 4 imágenes del proyecto de referencia (sujeto, estilo, fondo…: el prompt dice
    /// qué tomar de cada una, «image 1», «image 2»…)
    pub referencias: Vec<String>,
    /// texto que debe aparecer pintado (va entre comillas)
    pub texto: String,
    /// Ola PRO · klein BASE (sin destilar): CFG real y prompt NEGATIVO, 50 pasos
    pub base: bool,
    /// lo que NO debe salir (sólo con el base)
    pub negativo: String,
    /// guía del CFG con el base (0 = 4, el oficial)
    pub guia: f32,
    /// 0 = 4 (el del destilado; 50 con el base)
    pub pasos: u32,
    pub semilla: Option<u64>,
    pub pulir: bool,
}

impl Default for ProImagen {
    fn default() -> Self {
        ProImagen {
            proporcion: "1:1".into(),
            resolucion: "1".into(),
            variantes: 1,
            tipo: String::new(),
            camara: String::new(),
            lente: String::new(),
            apertura: String::new(),
            pelicula: String::new(),
            luz: String::new(),
            hora: String::new(),
            plano: String::new(),
            angulo: String::new(),
            composicion: String::new(),
            estilo_libre: String::new(),
            colores: Vec::new(),
            referencias: Vec::new(),
            base: false,
            negativo: String::new(),
            guia: 0.0,
            texto: String::new(),
            pasos: 0,
            semilla: None,
            pulir: true,
        }
    }
}

/// ¿Es un color hex (#RGB o #RRGGBB)?
fn es_hex(s: &str) -> bool {
    let h = s.trim().trim_start_matches('#');
    s.trim().starts_with('#') && (h.len() == 3 || h.len() == 6) && h.chars().all(|c| c.is_ascii_hexdigit())
}

impl ProImagen {
    pub fn tamano(&self) -> (u32, u32) {
        tamano_imagen(&self.proporcion, &self.resolucion)
    }

    pub fn variantes(&self) -> u32 {
        self.variantes.clamp(1, 4)
    }

    pub fn pasos(&self) -> u32 {
        match (self.pasos, self.usa_base()) {
            (0, true) => 50,
            (0, false) => 4,
            (p, _) => p.clamp(1, 50),
        }
    }

    /// El base cuando se pide o cuando hay negativo (sólo el base lo entiende)
    pub fn usa_base(&self) -> bool {
        self.base || !self.negativo.trim().is_empty()
    }

    /// El prompt en el orden de BFL: tipo + la descripción (sujeto y acción) PRIMERO; luego estilo,
    /// cámara y luz; los colores atados a objetos; el texto entre comillas al final.
    pub fn prompt(&self, base: &str) -> String {
        let base = base.trim().trim_end_matches(['.', ',', ';']).trim();
        let mut cabeza = String::new();
        if let Some(t) = fragmento(&tipos_imagen(), &self.tipo) {
            cabeza = format!("{t} of ");
        }
        // «photograph of An old man» → «photograph of an old man» (una sigla como «NASA» se respeta)
        let base = {
            let mut c = base.chars();
            match (c.next(), c.next()) {
                (Some(a), Some(b)) if !cabeza.is_empty() && a.is_uppercase() && !b.is_uppercase() => format!("{}{}", a.to_lowercase(), &base[a.len_utf8()..]),
                _ => base.to_string(),
            }
        };
        let mut partes: Vec<String> = vec![format!("{cabeza}{base}")];
        let estilo = self.estilo_libre.trim();
        if !estilo.is_empty() {
            partes.push(estilo.to_string());
        }
        for (lista, clave) in [
            (composiciones(), &self.composicion),
            (planos(), &self.plano),
            (angulos(), &self.angulo),
            (camaras(), &self.camara),
            (lentes_foto(), &self.lente),
            (aperturas(), &self.apertura),
            (peliculas(), &self.pelicula),
            (luces(), &self.luz),
            (horas(), &self.hora),
        ] {
            if let Some(p) = fragmento(&lista, clave) {
                partes.push(p.into());
            }
        }
        // colores: «objeto=#hex» → «the objeto is #HEX»; un hex suelto va a la paleta
        let (mut atados, mut paleta) = (Vec::new(), Vec::new());
        for c in self.colores.iter().map(|c| c.trim()).filter(|c| !c.is_empty()) {
            match c.split_once('=') {
                Some((obj, hex)) if es_hex(hex) && !obj.trim().is_empty() => {
                    let obj = obj.trim();
                    let l = obj.to_lowercase();
                    let art = if ["the ", "a ", "an ", "el ", "la ", "los ", "las "].iter().any(|a| l.starts_with(a)) { "" } else { "the " }; // juez-idioma: datos (artículos que ya trae el objeto)
                    atados.push(format!("{art}{obj} is {}", hex.trim().to_uppercase()))
                }
                _ if es_hex(c) => paleta.push(c.to_uppercase()),
                _ => {}
            }
        }
        partes.extend(atados);
        if !paleta.is_empty() {
            partes.push(format!("color palette {}", paleta.join(", ")));
        }
        let texto = self.texto.trim().replace('"', "'");
        if !texto.is_empty() {
            partes.push(format!("with the text \"{texto}\" clearly written"));
        }
        let mut p = partes.join(", ");
        p.push('.');
        p
    }
}

// ── Música · ACE-Step 1.5 turbo + LM 1,7B (PRO-3). La guía oficial (`docs/en/Tutorial.md`): el
// CAPTION es lo que más pesa y se arma por dimensiones (estilo, ánimo, instrumentos, textura, época,
// producción, voz, ritmo); sin choques; la letra lleva la estructura en etiquetas.

pub fn generos() -> Vec<Opcion> {
    let p = |c: &'static str, n: &'static str| op(c, n, c);
    vec![
        p("pop", "pop"),
        p("rock", "rock"),
        p("indie rock", "indie rock"),
        p("metal", "metal"),
        p("punk", "punk"),
        p("jazz", "jazz"),
        p("blues", "blues"),
        p("funk", "funk"),
        p("soul", "soul"),
        p("R&B", "R&B"),
        p("hip-hop", "hip-hop"),
        p("trap", "trap"),
        p("reggaeton", "reggaeton"),
        p("latin pop", "latin pop"),
        p("salsa", "salsa"),
        p("cumbia", "cumbia"),
        p("bachata", "bachata"),
        p("bossa nova", "bossa nova"),
        p("flamenco", "flamenco"),
        p("tango", "tango"),
        p("folk", "folk"),
        p("country", "country"),
        op("classical", crate::i18n::t("clásica", "classical"), "classical"),
        op("cinematic orchestral", crate::i18n::t("orquestal de cine", "cinematic orchestral"), "cinematic orchestral"),
        op("electronic", crate::i18n::t("electrónica", "electronic"), "electronic"),
        p("house", "house"),
        p("techno", "techno"),
        p("drum and bass", "drum and bass"),
        p("synthwave", "synthwave"),
        p("lo-fi hip hop", "lo-fi hip hop"),
        p("ambient", "ambient"),
        p("gospel", "gospel"),
        p("k-pop", "k-pop"),
        p("afrobeat", "afrobeat"),
        p("reggae", "reggae"),
    ]
}

pub fn epocas() -> Vec<Opcion> {
    vec![
        op("60s", crate::i18n::t("años 60", "60s"), "1960s"),
        op("70s", crate::i18n::t("años 70", "70s"), "1970s"),
        op("80s", crate::i18n::t("años 80", "80s"), "1980s"),
        op("90s", crate::i18n::t("años 90", "90s"), "1990s"),
        op("2000s", crate::i18n::t("años 2000", "2000s"), "2000s"),
        op("moderno", crate::i18n::t("moderno", "modern"), "modern"),
    ]
}

pub fn animos() -> Vec<Opcion> {
    vec![
        op("alegre", crate::i18n::t("alegre", "uplifting"), "uplifting"),
        op("melancolico", crate::i18n::t("melancólico", "melancholic"), "melancholic"),
        op("energico", crate::i18n::t("enérgico", "energetic"), "energetic"),
        op("sonador", crate::i18n::t("soñador", "dreamy"), "dreamy"),
        op("oscuro", crate::i18n::t("oscuro", "dark"), "dark"),
        op("nostalgico", crate::i18n::t("nostálgico", "nostalgic"), "nostalgic"),
        op("euforico", crate::i18n::t("eufórico", "euphoric"), "euphoric"),
        op("intimo", crate::i18n::t("íntimo", "intimate"), "intimate"),
        op("romantico", crate::i18n::t("romántico", "romantic"), "romantic"),
        op("epico", crate::i18n::t("épico", "epic"), "epic"),
        op("tranquilo", crate::i18n::t("tranquilo", "calm"), "calm"),
        op("agresivo", crate::i18n::t("agresivo", "aggressive"), "aggressive"),
        op("jugueton", crate::i18n::t("juguetón", "playful"), "playful"),
    ]
}

/// Los instrumentos van por familias (la interfaz los agrupa por el prefijo de la clave).
pub fn instrumentos() -> Vec<Opcion> {
    vec![
        op("teclas:piano", crate::i18n::t("piano", "piano"), "piano"),
        op("teclas:rhodes", crate::i18n::t("piano eléctrico (Rhodes)", "electric piano (Rhodes)"), "Rhodes electric piano"),
        op("teclas:organo", crate::i18n::t("órgano", "organ"), "Hammond organ"),
        op("teclas:pads", crate::i18n::t("colchones de sintetizador", "synth pads"), "synth pads"),
        op("teclas:lead", crate::i18n::t("sintetizador solista", "synth lead"), "synth lead"),
        op("cuerdas:acustica", crate::i18n::t("guitarra acústica", "acoustic guitar"), "acoustic guitar"),
        op("cuerdas:nylon", crate::i18n::t("guitarra española (nylon)", "nylon guitar"), "nylon-string guitar"),
        op("cuerdas:electrica", crate::i18n::t("guitarra eléctrica", "electric guitar"), "electric guitar"),
        op("cuerdas:distorsion", crate::i18n::t("guitarra con distorsión", "distorted guitar"), "distorted electric guitar"),
        op("cuerdas:ukulele", crate::i18n::t("ukelele", "ukulele"), "ukulele"),
        op("cuerdas:mandolina", crate::i18n::t("mandolina", "mandolin"), "mandolin"),
        op("bajo:electrico", crate::i18n::t("bajo eléctrico", "bass guitar"), "electric bass"),
        op("bajo:contrabajo", crate::i18n::t("contrabajo", "upright bass"), "upright bass"),
        op("bajo:808", crate::i18n::t("bajo 808", "808 bass"), "808 bass"),
        op("ritmo:bateria", crate::i18n::t("batería", "drums"), "drums"),
        op("ritmo:808", crate::i18n::t("batería 808", "808 drums"), "808 drums"),
        op("ritmo:escobillas", crate::i18n::t("batería con escobillas", "brush drums"), "brush drums"),
        op("ritmo:percusion", crate::i18n::t("percusión", "percussion"), "percussion"),
        op("ritmo:congas", crate::i18n::t("congas", "congas"), "congas"),
        op("ritmo:timbales", crate::i18n::t("timbales", "timbales"), "timbales"),
        op("orquesta:cuerdas", crate::i18n::t("sección de cuerdas", "string section"), "string section"),
        op("orquesta:violin", crate::i18n::t("violín", "violin"), "violin"),
        op("orquesta:cello", crate::i18n::t("violonchelo", "cello"), "cello"),
        op("orquesta:arpa", crate::i18n::t("arpa", "harp"), "harp"),
        op("viento:metales", crate::i18n::t("sección de metales", "brass section"), "brass section"),
        op("viento:trompeta", crate::i18n::t("trompeta", "trumpet"), "trumpet"),
        op("viento:saxo", crate::i18n::t("saxofón", "saxophone"), "saxophone"),
        op("viento:trombon", crate::i18n::t("trombón", "trombone"), "trombone"),
        op("viento:flauta", crate::i18n::t("flauta", "flute"), "flute"),
        op("viento:clarinete", crate::i18n::t("clarinete", "clarinet"), "clarinet"),
        op("viento:acordeon", crate::i18n::t("acordeón", "accordion"), "accordion"),
        op("otros:coro", crate::i18n::t("coro", "choir"), "choir"),
        op("otros:marimba", crate::i18n::t("marimba", "marimba"), "marimba"),
        op("otros:campanas", crate::i18n::t("campanas / glockenspiel", "bells / glockenspiel"), "glockenspiel"),
    ]
}

pub fn texturas() -> Vec<Opcion> {
    vec![
        op("calida", crate::i18n::t("cálida", "warm"), "warm"),
        op("brillante", crate::i18n::t("brillante", "bright"), "bright"),
        op("nitida", crate::i18n::t("nítida", "crisp"), "crisp"),
        op("aerea", crate::i18n::t("aérea", "airy"), "airy"),
        op("contundente", crate::i18n::t("contundente", "punchy"), "punchy"),
        op("exuberante", crate::i18n::t("exuberante", "lush"), "lush"),
        op("cruda", crate::i18n::t("cruda", "raw"), "raw"),
        op("granulada", crate::i18n::t("granulada", "gritty"), "gritty"),
    ]
}

pub fn producciones() -> Vec<Opcion> {
    vec![
        op("estudio", crate::i18n::t("estudio pulido", "studio-polished"), "studio-polished, high-fidelity"),
        op("vivo", crate::i18n::t("grabación en vivo", "live recording"), "live recording"),
        op("lofi", crate::i18n::t("lo-fi", "lo-fi"), "lo-fi"),
        op("dormitorio", crate::i18n::t("casero (bedroom pop)", "bedroom pop"), "bedroom pop"),
        op("cinta", crate::i18n::t("cinta analógica vintage", "vintage analog tape"), "vintage analog tape warmth"),
    ]
}

pub fn voces() -> Vec<Opcion> {
    vec![
        op("instrumental", crate::i18n::t("instrumental (sin voz)", "instrumental (no vocals)"), "instrumental"),
        op("femenina", crate::i18n::t("voz femenina", "female vocal"), "female vocal"),
        op("masculina", crate::i18n::t("voz masculina", "male vocal"), "male vocal"),
        op("duo", crate::i18n::t("dúo (hombre y mujer)", "duet (male and female)"), "male and female duet vocals"),
        op("coro", crate::i18n::t("coro", "choir"), "choir vocals"),
        op("rap", crate::i18n::t("rap", "rap"), "rap vocals"),
    ]
}

pub fn tecnicas() -> Vec<Opcion> {
    vec![
        op("suave", crate::i18n::t("suave", "soft"), "soft gentle vocals"),
        op("aireada", crate::i18n::t("aireada (breathy)", "breathy"), "breathy"),
        op("potente", crate::i18n::t("potente", "powerful"), "powerful belting"),
        op("falsete", crate::i18n::t("falsete", "falsetto"), "falsetto"),
        op("rasposa", crate::i18n::t("rasposa", "raspy"), "raspy"),
        op("susurrada", crate::i18n::t("susurrada", "whispered"), "whispered vocals"),
        op("autotune", crate::i18n::t("con autotune", "auto-tuned"), "auto-tuned vocals"),
    ]
}

pub fn ritmos() -> Vec<Opcion> {
    vec![
        op("lento", crate::i18n::t("lento", "slow"), "slow tempo"),
        op("medio", crate::i18n::t("medio", "mid-tempo"), "mid-tempo"),
        op("rapido", crate::i18n::t("rápido", "fast"), "fast-paced"),
        op("groove", crate::i18n::t("con groove", "groovy"), "groovy"),
        op("empuje", crate::i18n::t("con empuje", "driving"), "driving rhythm"),
        op("relajado", crate::i18n::t("relajado", "laid-back"), "laid-back"),
    ]
}

/// PRO-6 · las pistas que separa HTDemucs de verdad (guitarra y piano, con el modelo de 6 pistas).
pub fn pistas_separables() -> Vec<Opcion> {
    vec![
        op("vocals", crate::i18n::t("voz", "vocals"), "vocals"),
        op("drums", crate::i18n::t("batería", "drums"), "drums"),
        op("bass", crate::i18n::t("bajo", "bass"), "bass"),
        op("other", crate::i18n::t("resto (otros instrumentos)", "the rest (other instruments)"), "other"),
        op("guitar", crate::i18n::t("guitarra", "guitar"), "guitar"),
        op("piano", crate::i18n::t("piano", "piano"), "piano"),
        // EXP-7: la batería partida (afinado de GOTV sobre HTDemucs; sale de la batería de la canción)
        op("kick", crate::i18n::t("batería: bombo", "drums: kick"), "kick"),
        op("snare", crate::i18n::t("batería: caja", "drums: snare"), "snare"),
        op("cymbals", crate::i18n::t("batería: platos", "drums: cymbals"), "cymbals"),
        op("toms", crate::i18n::t("batería: toms", "drums: toms"), "toms"),
    ]
}

/// Las 12 pistas que entrenó ACE-Step (`TRACK_NAMES`): separar, añadir o completar con el DiT base.
pub fn pistas_musica() -> Vec<Opcion> {
    vec![
        op("vocals", crate::i18n::t("voz principal", "lead vocals"), "vocals"),
        op("backing_vocals", crate::i18n::t("coros", "backing vocals"), "backing_vocals"),
        op("drums", crate::i18n::t("batería", "drums"), "drums"),
        op("bass", crate::i18n::t("bajo", "bass"), "bass"),
        op("guitar", crate::i18n::t("guitarra", "guitar"), "guitar"),
        op("keyboard", crate::i18n::t("teclados", "keyboard"), "keyboard"),
        op("percussion", crate::i18n::t("percusión", "percussion"), "percussion"),
        op("strings", crate::i18n::t("cuerdas", "strings"), "strings"),
        op("synth", crate::i18n::t("sintetizador", "synth"), "synth"),
        op("brass", crate::i18n::t("metales", "brass"), "brass"),
        op("woodwinds", crate::i18n::t("maderas", "woodwinds"), "woodwinds"),
        op("fx", crate::i18n::t("efectos", "fx"), "fx"),
    ]
}

pub fn idiomas_voz() -> Vec<Opcion> {
    vec![
        op("es", crate::i18n::t("español", "Spanish"), "es"),
        op("en", crate::i18n::t("inglés", "English"), "en"),
        op("pt", crate::i18n::t("portugués", "Portuguese"), "pt"),
        op("fr", crate::i18n::t("francés", "French"), "fr"),
        op("it", crate::i18n::t("italiano", "Italian"), "it"),
        op("de", crate::i18n::t("alemán", "German"), "de"),
        op("ja", crate::i18n::t("japonés", "Japanese"), "ja"),
        op("ko", crate::i18n::t("coreano", "Korean"), "ko"),
        op("zh", crate::i18n::t("chino", "Chinese"), "zh"),
    ]
}

/// Las 24 tonalidades como las escribe el oficial («C major», «A minor»).
pub fn tonalidades() -> Vec<serde_json::Value> {
    let notas = ["C", "C#", "D", "Eb", "E", "F", "F#", "G", "Ab", "A", "Bb", "B"];
    let (may, men) = (crate::i18n::t("mayor", "major"), crate::i18n::t("menor", "minor"));
    notas
        .iter()
        .flat_map(|n| [(format!("{n} major"), format!("{n} {may}")), (format!("{n} minor"), format!("{n} {men}"))])
        .map(|(clave, nombre)| serde_json::json!({"clave": clave, "nombre": nombre}))
        .collect()
}

/// Las etiquetas de la letra que entrenó ACE-Step (estructura, voz, energía, instrumentales).
pub const ETIQUETAS_LETRA: &[&str] = &[
    "[Intro]", "[Verse]", "[Pre-Chorus]", "[Chorus]", "[Bridge]", "[Outro]", "[Build]", "[Drop]", "[Breakdown]", "[Instrumental]", "[Guitar Solo]", "[Piano Interlude]", "[Fade Out]", "[whispered]", "[falsetto]", "[raspy vocal]",
    "[powerful belting]", "[spoken word]", "[harmonies]", "[ad-lib]", "[high energy]", "[building energy]", "[explosive]", "[melancholic]",
];

/// Los ajustes profesionales del perfil Música.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct ProMusica {
    pub genero: String,
    pub epoca: String,
    pub animo: String,
    pub instrumentos: Vec<String>,
    pub textura: String,
    pub produccion: String,
    pub voz: String,
    pub tecnica: String,
    pub ritmo: String,
    pub estilo_libre: String,
    pub idioma: String,
    /// la letra tal cual (con sus etiquetas); vacía y con voz = la escribe el modelo
    pub letra: String,
    /// 10–600 s (0 = 30; 600 medido: 5,1 min y 19,4 GB de pico)
    pub segundos: f32,
    /// 0 = la elige el LM
    pub bpm: u32,
    pub tonalidad: String,
    pub compas: String,
    /// el LM planificador (CoT + códigos): apagado, sólo el DiT
    pub pensar: bool,
    /// temperatura del LM (0 = 0,85 oficial)
    pub creatividad: f32,
    /// cfg del LM (0 = 2,0 oficial)
    pub fidelidad: f32,
    pub excluir: String,
    /// 1–3 (1 = el del manejador oficial)
    pub shift: u32,
    pub variantes: u32,
    pub semilla: Option<u64>,
    /// Ola PRO · el DiT BASE de ACE-Step: componer con CFG (`base`) o una tarea sobre `audio`:
    /// «extraer» una pista, «lego» (añadir una) o «completar» (con varias)
    pub base: bool,
    pub tarea: String,
    pub pistas: Vec<String>,
    /// Ola PRO · PRO-6 · entregar TAMBIÉN la canción en pistas separadas (stems) con HTDemucs: tras cada
    /// canción, una `separar_pistas` por cada una de éstas (`vocals`, `drums`, `bass`, `other`, `guitar`, `piano`)
    pub separar: Vec<String>,
    /// «ft» = separación fina (HTDemucs ft, unas 4 veces más lenta); vacío = la rápida
    pub separador: String,
    /// PRO-4 · el DiT de música: "" (turbo) · sft · xl-turbo · xl-sft · xl-base
    #[serde(default)]
    pub modelo: String,
    /// PRO-4 · el planificador: "" (1,7B) · 4b
    #[serde(default)]
    pub planificador: String,
    /// Ola PRO · audio del proyecto de referencia de TIMBRE y mezcla (global; no copia la melodía)
    pub timbre: String,
    /// cover / repaint con un audio del proyecto
    pub audio: String,
    pub fuerza_cover: f32,
    pub ruido_cover: f32,
    pub repintar: String,
    pub fuerza_repintar: f32,
    pub pulir: bool,
}

impl Default for ProMusica {
    fn default() -> Self {
        ProMusica {
            genero: String::new(),
            epoca: String::new(),
            animo: String::new(),
            instrumentos: Vec::new(),
            textura: String::new(),
            produccion: String::new(),
            voz: String::new(),
            tecnica: String::new(),
            ritmo: String::new(),
            estilo_libre: String::new(),
            idioma: "es".into(),
            letra: String::new(),
            segundos: 30.0,
            bpm: 0,
            tonalidad: String::new(),
            compas: String::new(),
            pensar: true,
            creatividad: 0.0,
            fidelidad: 0.0,
            excluir: String::new(),
            shift: 1,
            variantes: 1,
            semilla: None,
            base: false,
            tarea: String::new(),
            pistas: Vec::new(),
            separar: Vec::new(),
            separador: String::new(),
            modelo: String::new(),
            planificador: String::new(),
            timbre: String::new(),
            audio: String::new(),
            fuerza_cover: 1.0,
            ruido_cover: 0.0,
            repintar: String::new(),
            fuerza_repintar: 0.5,
            pulir: true,
        }
    }
}

impl ProMusica {
    pub fn instrumental(&self) -> bool {
        self.voz == "instrumental"
    }

    pub fn variantes(&self) -> u32 {
        self.variantes.clamp(1, 4)
    }

    pub fn segundos(&self) -> f32 {
        if self.segundos <= 0.0 { 30.0 } else { self.segundos.clamp(10.0, 600.0) }
    }

    /// El caption: la descripción y detrás las dimensiones elegidas, sin repetir (máx. 512
    /// caracteres, el tope oficial).
    pub fn caption(&self, base: &str) -> String {
        let mut partes: Vec<String> = Vec::new();
        let base = base.trim().trim_end_matches(['.', ',', ';']).trim();
        if !base.is_empty() {
            partes.push(base.to_string());
        }
        let estilo = self.estilo_libre.trim();
        if !estilo.is_empty() {
            partes.push(estilo.to_string());
        }
        // la época va pegada al género («1980s synthwave»)
        match (fragmento(&epocas(), &self.epoca), fragmento(&generos(), &self.genero)) {
            (Some(e), Some(g)) => partes.push(format!("{e} {g}")),
            (None, Some(g)) => partes.push(g.into()),
            (Some(e), None) => partes.push(format!("{e} style")),
            _ => {}
        }
        for (lista, clave) in [(animos(), &self.animo), (voces(), &self.voz)] {
            if let Some(p) = fragmento(&lista, clave) {
                partes.push(p.into());
            }
        }
        if !self.instrumental() {
            if let Some(t) = fragmento(&tecnicas(), &self.tecnica) {
                partes.push(t.into());
            }
        }
        let todos = instrumentos();
        let inst: Vec<&str> = self.instrumentos.iter().filter_map(|c| fragmento(&todos, c)).collect();
        if !inst.is_empty() {
            partes.push(inst.join(", "));
        }
        for (lista, clave) in [(texturas(), &self.textura), (producciones(), &self.produccion), (ritmos(), &self.ritmo)] {
            if let Some(p) = fragmento(&lista, clave) {
                partes.push(p.into());
            }
        }
        let mut vistos = std::collections::HashSet::new();
        partes.retain(|p| vistos.insert(p.to_lowercase()));
        let mut c = partes.join(", ");
        if c.chars().count() > 512 {
            c = c.chars().take(512).collect();
        }
        c
    }

    /// Las perillas del motor (`generar_wav`): sólo lo que se cambió del oficial.
    pub fn opciones(&self) -> serde_json::Value {
        let mut o = serde_json::json!({"pensar": self.pensar, "shift": self.shift.clamp(1, 3)});
        if self.creatividad > 0.0 {
            o["temperatura"] = serde_json::json!(self.creatividad.clamp(0.3, 1.5));
        }
        if self.fidelidad > 0.0 {
            o["cfg_lm"] = serde_json::json!(self.fidelidad.clamp(1.0, 5.0));
        }
        if !self.excluir.trim().is_empty() {
            o["excluir"] = serde_json::json!(self.excluir.trim());
        }
        if !self.timbre.trim().is_empty() {
            o["timbre"] = serde_json::json!(self.timbre.trim());
        }
        if !self.audio.trim().is_empty() {
            o["ruido_cover"] = serde_json::json!(self.ruido_cover.clamp(0.0, 0.99));
            o["fuerza_repintar"] = serde_json::json!(self.fuerza_repintar.clamp(0.0, 1.0));
        }
        o
    }
}

/// Lo que llega junto a la orden: sólo el perfil con el switch encendido trae sus ajustes.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct Pro {
    pub video: Option<ProVideo>,
    pub imagen: Option<ProImagen>,
    pub musica: Option<ProMusica>,
}

/// El vocabulario para la interfaz (una sola fuente: el núcleo), con los nombres en el idioma de la app.
pub fn opciones() -> serde_json::Value {
    serde_json::json!({
        "musica": {
            "genero": generos(),
            "epoca": epocas(),
            "animo": animos(),
            "instrumentos": instrumentos(),
            "textura": texturas(),
            "produccion": producciones(),
            "voz": voces(),
            "tecnica": tecnicas(),
            "ritmo": ritmos(),
            "idioma": idiomas_voz(),
            "tonalidad": tonalidades(),
            "etiquetas": ETIQUETAS_LETRA,
            "pistas": pistas_musica(),
            "separables": pistas_separables(),
        },
        "imagen": {
            "proporcion": proporciones(),
            "resolucion": resoluciones(),
            "tipo": tipos_imagen(),
            "camara": camaras(),
            "lente": lentes_foto(),
            "apertura": aperturas(),
            "pelicula": peliculas(),
            "luz": luces(),
            "hora": horas(),
            "plano": planos(),
            "angulo": angulos(),
            "composicion": composiciones(),
        },
        "video": {
            "formato": formatos(),
            "calidad": calidades(),
            "plano": planos(),
            "angulo": angulos(),
            "lente": lentes(),
            "movimiento": movimientos(),
            "velocidad": velocidades(),
            "luz": luces(),
            "hora": horas(),
            "tono": tonos(),
            "estilo": estilos_video(),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sin_ajustes_el_prompt_queda_igual_y_el_negativo_es_el_oficial() {
        let p = ProVideo::default();
        assert_eq!(p.prompt("a fox runs in the snow."), "a fox runs in the snow");
        assert_eq!(p.negativo("OFICIAL"), None);
        assert_eq!((p.tamano(), p.pasos(), p.guia()), ((1280, 704), 50, 5.0));
        assert!(!p.experimental());
    }

    #[test]
    fn el_control_estetico_va_detras_en_el_orden_de_wan() {
        let p = ProVideo { plano: "primer".into(), angulo: "contrapicado".into(), movimiento: "acercar".into(), velocidad: "lenta".into(), luz: "borde".into(), hora: "dorada".into(), tono: "calido".into(), estilo: "cine".into(), ..Default::default() };
        assert_eq!(
            p.prompt("a red fox trots through fresh snow"),
            "a red fox trots through fresh snow. close-up shot, low-angle shot, slow camera pushes in toward the subject, rim lighting, golden hour, warm color tone, cinematic, film look."
        );
        // la cámara fija no lleva velocidad; una clave desconocida no añade nada
        let f = ProVideo { movimiento: "fija".into(), velocidad: "rapida".into(), lente: "inventada".into(), ..Default::default() };
        assert_eq!(f.sufijo(), "static camera, locked-off shot");
    }

    #[test]
    fn el_negativo_propio_se_suma_al_oficial_o_lo_reemplaza() {
        let p = ProVideo { negativo: "texto, marcas de agua".into(), ..Default::default() };
        assert_eq!(p.negativo("OFICIAL").as_deref(), Some("OFICIAL，texto, marcas de agua"));
        let q = ProVideo { sin_negativo_oficial: true, ..p.clone() };
        assert_eq!(q.negativo("OFICIAL").as_deref(), Some("texto, marcas de agua"));
        // sin oficial y sin propio: negativo vacío (la guía sigue, contra el «nada»)
        let r = ProVideo { sin_negativo_oficial: true, ..Default::default() };
        assert_eq!(r.negativo("OFICIAL").as_deref(), Some(""));
    }

    #[test]
    fn tamanos_multiplos_de_32_y_lo_oficial_no_es_experimental() {
        for f in ["16:9", "9:16", "1:1"] {
            for c in ["720p", "480p"] {
                let (w, h) = tamano_video(f, c);
                assert_eq!((w % 32, h % 32), (0, 0), "{f} {c}");
            }
        }
        assert_eq!(tamano_video("9:16", "720p"), (704, 1280));
        assert!(ProVideo { formato: "1:1".into(), ..Default::default() }.experimental(), "1:1 a 720p (960×960): sin medir");
        assert!(!ProVideo { formato: "1:1".into(), calidad: "480p".into(), ..Default::default() }.experimental(), "1:1 a 480p MEDIDO: 17,2 min");
        assert!(!ProVideo { calidad: "480p".into(), ..Default::default() }.experimental(), "480p MEDIDO (17,5 min, nítido): ya no es experimental");
    }

    #[test]
    fn la_estimacion_coincide_con_lo_medido_y_480p_es_mas_rapido() {
        // V7: 2 s a 720p con 50 pasos = 47,8 min medidos
        let m = ProVideo::default().minutos_estimados(2.0);
        assert!((m - 47.4).abs() < 1.5, "{m}");
        let r = ProVideo { calidad: "480p".into(), ..Default::default() }.minutos_estimados(2.0);
        assert!(r < m / 2.0, "{r} vs {m}");
        assert!((r - 17.5).abs() < 0.8, "480p y 2 s: 17,5 min MEDIDOS; estimado {r}");
        let menos = ProVideo { pasos: 25, ..Default::default() }.minutos_estimados(2.0);
        assert!(menos < m * 0.6);
    }

    #[test]
    fn los_ajustes_llegan_de_la_interfaz_con_campos_sueltos() {
        let p: Pro = serde_json::from_value(serde_json::json!({"video": {"formato": "9:16", "movimiento": "orbita", "pasos": 30}})).unwrap();
        let v = p.video.unwrap();
        assert_eq!((v.tamano(), v.pasos(), v.pulir), ((704, 1280), 30, true));
        // el vocabulario sale entero para la interfaz, sin el prompt (es del núcleo)
        let o = opciones();
        assert_eq!(o["video"]["movimiento"].as_array().unwrap().len(), movimientos().len());
        assert!(o["video"]["plano"][0].get("prompt").is_none());
    }

    #[test]
    fn musica_caption_por_dimensiones_y_opciones_solo_lo_cambiado() {
        let m = ProMusica {
            genero: "synthwave".into(),
            epoca: "80s".into(),
            animo: "nostalgico".into(),
            voz: "femenina".into(),
            tecnica: "aireada".into(),
            instrumentos: vec!["teclas:pads".into(), "bajo:electrico".into(), "ritmo:808".into(), "no-existe".into()],
            textura: "calida".into(),
            produccion: "cinta".into(),
            ritmo: "relajado".into(),
            ..Default::default()
        };
        assert_eq!(
            m.caption("a night drive through a neon city."),
            "a night drive through a neon city, 1980s synthwave, nostalgic, female vocal, breathy, synth pads, electric bass, 808 drums, warm, vintage analog tape warmth, laid-back"
        );
        // instrumental: la técnica vocal no entra
        let i = ProMusica { voz: "instrumental".into(), tecnica: "potente".into(), ..Default::default() };
        assert_eq!(i.caption("calm piano"), "calm piano, instrumental");
        assert!(i.instrumental());
        // opciones: lo oficial no se manda; lo cambiado sí, con sus topes
        assert_eq!(ProMusica::default().opciones(), serde_json::json!({"pensar": true, "shift": 1}));
        let o = ProMusica { creatividad: 9.0, fidelidad: 3.0, excluir: " male vocals ".into(), shift: 7, pensar: false, audio: "a.wav".into(), ruido_cover: 0.4, ..Default::default() }.opciones();
        assert_eq!(o, serde_json::json!({"pensar": false, "shift": 3, "temperatura": 1.5, "cfg_lm": 3.0, "excluir": "male vocals", "ruido_cover": 0.4f32, "fuerza_repintar": 0.5}));
        assert_eq!((ProMusica::default().segundos(), ProMusica { segundos: 900.0, ..Default::default() }.segundos()), (30.0, 600.0));
        assert_eq!(tonalidades().len(), 24);
        // el caption nunca pasa del tope oficial de 512 caracteres
        assert!(ProMusica { estilo_libre: "x".repeat(900), ..Default::default() }.caption("y").chars().count() <= 512);
    }

    #[test]
    fn imagen_tamanos_por_proporcion_y_megapixeles() {
        assert_eq!(tamano_imagen("1:1", "1"), (1024, 1024));
        assert_eq!(tamano_imagen("1:1", "4"), (2048, 2048));
        let (w, h) = tamano_imagen("16:9", "1");
        assert!((w, h) == (1360, 768) && w % 16 == 0 && h % 16 == 0, "{w}×{h}");
        // largo a 4 MP: el lado largo se topa en 2048 y conserva la proporción
        let (w, h) = tamano_imagen("21:9", "4");
        assert_eq!(w, 2048);
        assert!((w as f64 / h as f64 - 21.0 / 9.0).abs() < 0.03, "{w}×{h}");
        assert_eq!(tamano_imagen("9:16", "1"), (768, 1360));
        assert_eq!(tamano_imagen("basura", "x"), (1024, 1024), "lo que no se entiende = 1:1 a 1 MP");
        assert_eq!(tamano_imagen("1:1", "0.5"), (720, 720), "724 px redondeado a múltiplo de 16");
    }

    #[test]
    fn imagen_prompt_en_el_orden_de_bfl_con_colores_y_texto() {
        let p = ProImagen {
            tipo: "foto".into(),
            camara: "hasselblad".into(),
            lente: "85".into(),
            apertura: "1.4".into(),
            luz: "suave".into(),
            hora: "dorada".into(),
            colores: vec!["the scarf=#ff0000".into(), "#1E90FF".into(), "no-es-color".into(), "x=#zz0000".into()],
            texto: "Café \"Luna\"".into(),
            ..Default::default()
        };
        assert_eq!(
            p.prompt("an old man reading a newspaper on a bench."),
            "photograph of an old man reading a newspaper on a bench, shot on Hasselblad X2D, 85mm portrait lens, f/1.4, shallow depth of field, creamy bokeh, soft diffused lighting, golden hour, the scarf is #FF0000, color palette #1E90FF, with the text \"Café 'Luna'\" clearly written."
        );
        // sin nada elegido: la descripción tal cual
        assert_eq!(ProImagen::default().prompt("a cat"), "a cat.");
        assert_eq!((ProImagen::default().variantes(), ProImagen { variantes: 9, ..Default::default() }.variantes(), ProImagen::default().pasos()), (1, 4, 4));
    }
}
