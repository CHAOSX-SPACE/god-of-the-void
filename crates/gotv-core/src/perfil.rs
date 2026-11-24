//! Perfiles (Ola D7): Programar · Diseño web · Imagen (M4/M5) · Video (M6) · Música (M7) · Auto. Un perfil NO es una skill con botón: cambia
//! lo que el modelo VE (herramientas cargadas) y le recuerda la forma de trabajar que el
//! programa impone. La nota viaja con la ORDEN (no en el sistema), así la caché del prefijo
//! del sistema + herramientas sigue valiendo entre turnos.

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Perfil {
    #[default]
    Auto,
    Programar,
    Diseno,
    /// M4/M5: fotos, arte e ilustraciones (también isométricas, con la escena vectorial de guía)
    Imagen,
    /// M6: video corto desde una descripción o animando una imagen (Wan 2.2 TI2V 5B)
    Video,
    /// M7: música instrumental y canciones con letra
    Musica,
}

impl Perfil {
    pub fn parse(s: &str) -> Perfil {
        match s.trim().to_lowercase().as_str() {
            "programar" | "programacion" | "programación" | "code" | "codigo" | "código" // juez-idioma: datos (palabras de una orden o un valor en español)
            | "dev" => Perfil::Programar,
            "diseno" | "diseño" | "design" | "web" | "diseno-web" | "diseño-web" => { // juez-idioma: datos (palabras de una orden o un valor en español)
                Perfil::Diseno
            }
            "imagen" | "imagenes" | "imágenes" | "image" | "images" | "arte" | "art" | "ilustracion" | "ilustración" => Perfil::Imagen, // juez-idioma: datos (palabras de una orden o un valor en español)
            "video" | "vídeo" | "videos" | "vídeos" | "clip" | "clips" => Perfil::Video, // juez-idioma: datos (palabras de una orden o un valor en español)
            "musica" | "música" | "music" | "audio" | "cancion" | "canción" | "canciones" => Perfil::Musica, // juez-idioma: datos (palabras de una orden o un valor en español)
            _ => Perfil::Auto,
        }
    }

    pub fn nombre(self) -> &'static str {
        match self {
            Perfil::Auto => "auto",
            Perfil::Programar => "programar",
            Perfil::Diseno => "diseno",
            Perfil::Imagen => "imagen",
            Perfil::Video => "video",
            Perfil::Musica => "musica",
        }
    }

    /// El perfil efectivo de UNA orden: el elegido; en Auto, Diseño, Imagen, Video o Música sólo si la orden lo pide
    /// (si no, nada: una charla o una orden de código no cargan notas en cada turno; las leyes
    /// de programación van cuando el usuario ELIGE Programar).
    pub fn para(self, orden: &str) -> Perfil {
        match self {
            Perfil::Auto => match detectar(orden) {
                p @ (Perfil::Diseno | Perfil::Imagen | Perfil::Video | Perfil::Musica) => p,
                _ => Perfil::Auto,
            },
            p => p,
        }
    }

    /// Herramientas que este perfil deja a la vista desde el principio (las demás se buscan).
    pub fn herramientas(self) -> &'static [&'static str] {
        match self {
            Perfil::Diseno => &["pagina", "isometrica", "app_nueva", "browser"],
            Perfil::Imagen => &["generar_imagen", "isometrica", "view_image"],
            // el video puede partir de una imagen: crearla (`generar_imagen`) y mirarla (`view_image`)
            Perfil::Video => &["generar_video", "generar_imagen", "view_image"],
            Perfil::Musica => &["generar_musica", "generar_efecto", "separar_pistas"],
            _ => &[],
        }
    }

    /// Lo que se le recuerda al modelo con la orden.
    pub fn nota(self) -> Option<&'static str> {
        match self {
            Perfil::Auto => None,
            Perfil::Diseno => Some(crate::i18n::t(
                "[Perfil DISEÑO WEB de GOTV]\n\
                 - Página o landing NUEVA (proyecto sin diseño propio): NO escribas HTML ni CSS a mano. Llama `pagina` (es una HERRAMIENTA, no una skill) con una ficha (dirección de estilo + contenido real del negocio); GOTV construye el diseño profesional (tema con contraste garantizado, fuentes, iconos, animaciones). Si algo de la ficha no se entiende, recibes avisos: corrige sólo eso.\n\
                 - Aplicación (panel, CRUD, herramienta): `app_nueva` y luego compón pantallas con los componentes de `src/gotv/gotv.tsx` (Tabla, Dialogo, Campo, Pestanas, Estadistica…); la lógica va en `src/datos.ts` con sus pruebas. Nada de CSS propio salvo lo que no exista.\n\
                 - Ilustraciones y diagramas: `isometrica` (describe piezas; GOTV hace la proyección).\n\
                 - Siempre: verifica en el navegador (`browser`: open, a11y, set viewport 390 844) y decide con lo MEDIDO, no a ojo.",
                "[GOTV WEB DESIGN profile]\n\
                 - Page or landing: do NOT hand-write HTML or CSS. Call `pagina` with a spec (style direction + real business content); GOTV builds the professional design (theme with guaranteed contrast, fonts, icons, animation). If something in the spec is not understood you get warnings: fix only that.\n\
                 - Application (dashboard, CRUD, tool): `app_nueva`, then compose screens with the components in `src/gotv/gotv.tsx` (Tabla, Dialogo, Campo, Pestanas, Estadistica…); logic goes in `src/datos.ts` with its tests. No custom CSS unless it does not exist.\n\
                 - Illustrations and diagrams: `isometrica` (describe pieces; GOTV does the projection).\n\
                 - Always: verify in the browser (`browser`: open, a11y, set viewport 390 844) and decide from what you MEASURE, not by eye.",
            )),
            Perfil::Imagen => Some(crate::i18n::t(
                "[Perfil IMAGEN de GOTV — se genera en ESTA Mac con FLUX.2 klein 4B]\n\
                 - Foto, ilustración o arte: llama `generar_imagen` (es una HERRAMIENTA) con la descripción en INGLÉS y concreta: sujeto, escena, estilo, luz, encuadre y colores. 1024×1024 por defecto; retrato 768×1024, paisaje 1024×768. Tarda ~10 s a 512 px y ~45 s a 1024: díselo al usuario.\n\
                 - Isométrico: un diagrama EXACTO y editable → `isometrica` (SVG). Una ILUSTRACIÓN isométrica con materiales y luz → primero `isometrica` (la geometría) y luego `generar_imagen` con `guia` = ese .svg y un prompt de materiales, luz y estilo («detailed isometric illustration of …»).\n\
                 - Revisa el resultado con `view_image` antes de darlo por bueno; si no sirve, cambia la descripción o la semilla (no repitas lo mismo).\n\
                 - No prometas texto legible dentro de la imagen: este modelo lo garabatea. Para letreros o logotipos con texto, mejor `isometrica` o `pagina`.",
                "[GOTV IMAGE profile — generated on THIS Mac with FLUX.2 klein 4B]\n\
                 - Photo, illustration or art: call `generar_imagen` (it is a TOOL) with the description in ENGLISH and concrete: subject, scene, style, light, framing and colours. 1024×1024 by default; portrait 768×1024, landscape 1024×768. It takes ~10 s at 512 px and ~45 s at 1024: tell the user.\n\
                 - Isometric: an EXACT, editable diagram → `isometrica` (SVG). An isometric ILLUSTRATION with materials and light → first `isometrica` (the geometry), then `generar_imagen` with `guia` = that .svg and a prompt about materials, light and style («detailed isometric illustration of …»).\n\
                 - Check the result with `view_image` before calling it done; if it does not work, change the description or the seed (do not repeat the same).\n\
                 - Do not promise readable text inside the image: this model scribbles it. For signs or logos with text, prefer `isometrica` or `pagina`.",
            )),
            Perfil::Video => Some(crate::i18n::t(
                "[Perfil VIDEO de GOTV — se genera en ESTA Mac con Wan 2.2 TI2V 5B, sin sonido, 24 cuadros por segundo]\n\
                 - Video desde una descripción: llama `generar_video` (es una HERRAMIENTA) con el `prompt` en INGLÉS, concreto y visual: sujeto, ACCIÓN (qué se mueve y cómo), escena, luz, movimiento de cámara y estilo (p. ej. «a red fox trots through fresh snow at dawn, soft golden light, slow tracking shot, cinematic»). 1280×704, o 704×1280 con `vertical` (para el móvil); `segundos` de 1 a 5 (por defecto 2).\n\
                 - ANIMAR una imagen (la del usuario o una que crees): `generar_video` con `imagen` = su ruta en el proyecto; es el primer cuadro y el video sigue desde ella, con el tamaño de su proporción. El `prompt` describe el MOVIMIENTO («the cat slowly opens its eyes and stretches»), no la escena que ya se ve. Si piden un video de algo muy concreto (un personaje, un producto), puedes crear antes el primer cuadro con `generar_imagen`, revisarlo con `view_image` y animarlo.\n\
                 - Es MUY LENTO en esta Mac: unos 48 minutos por cada 2 s a 720p (5 s, más de 2 horas). DILO ANTES de empezar, con ese tiempo, y propón 2 s si no piden otra cosa. Se puede cancelar con ⏹.\n\
                 - Un clip que no convence se repite con otra `semilla` o un prompt más preciso; no prometas texto legible, caras de personas reales ni sonido.",
                "[GOTV VIDEO profile — generated on THIS Mac with Wan 2.2 TI2V 5B, no sound, 24 frames per second]\n\
                 - Video from a description: call `generar_video` (it is a TOOL) with the `prompt` in ENGLISH, concrete and visual: subject, ACTION (what moves and how), scene, light, camera movement and style (e.g. «a red fox trots through fresh snow at dawn, soft golden light, slow tracking shot, cinematic»). 1280×704, or 704×1280 with `vertical` (for phones); `segundos` from 1 to 5 (default 2).\n\
                 - ANIMATE an image (the user's or one you create): `generar_video` with `imagen` = its path in the project; it is the first frame and the video continues from it, at a size that follows its aspect ratio. The `prompt` describes the MOTION («the cat slowly opens its eyes and stretches»), not the scene already visible. For a video of something very specific (a character, a product) you may first create the first frame with `generar_imagen`, check it with `view_image` and animate it.\n\
                 - It is VERY SLOW on this Mac: about 48 minutes per 2 s at 720p (5 s, over 2 hours). SAY SO BEFORE starting, with that time, and suggest 2 s unless they ask otherwise. It can be cancelled with ⏹.\n\
                 - A clip that does not convince is redone with another `semilla` or a more precise prompt; do not promise readable text, faces of real people or sound.",
            )),
            Perfil::Musica => Some(crate::i18n::t(
                "[Perfil MÚSICA de GOTV — se genera en ESTA Mac con ACE-Step 1.5]\n\
                 - Música o canción: llama `generar_musica` (es una HERRAMIENTA). `prompt` en INGLÉS y concreto: género, subgénero, instrumentos, ambiente, tipo de voz (p. ej. «melancholic indie folk, male vocals, fingerpicked acoustic guitar, soft strings»).\n\
                 - Canción con voz: escribe TÚ la letra completa EN EL IDIOMA QUE PIDIÓ EL USUARIO (si pidió español, la letra va en español aunque el `prompt` vaya en inglés), con su estructura en etiquetas en líneas propias ([verse], [chorus], [bridge], [outro]); pasa `letra` e `idioma` (es, en, pt…). Sin letra sale instrumental.\n\
                 - Si el usuario da tempo, tonalidad o compás, pásalos (`bpm`, `tono` como «A minor», `compas` 2/3/4/6); si no, los elige el LM de ACE-Step. Duración por defecto 30 s (5–240). Tarda ~20 s por cada 30 s: díselo.\n\
                 - Con un audio del usuario: OTRA VERSIÓN de su canción → `generar_musica` con `audio` (la ruta) y el estilo nuevo en `prompt` (`fuerza` 0–1: 1 sigue la canción, ~0,2 sólo la idea); REHACER sólo un tramo → además `repintar` «desde-hasta» en segundos (p. ej. «10-18»). Tarda ~7 s por cada 30 s de audio.\n\
                 - Efecto de sonido (lluvia, puerta, explosión, pasos, ambiente, un animal): `generar_efecto` con el sonido en INGLÉS y concreto (fuente, acción, lugar, cómo suena); 1–30 s, por defecto 5. Tarda 3–4 minutos sea cual sea la duración: avísalo antes. No lo uses para música.\n\
                 - Una pieza que no convence se repite con otra `semilla` o una descripción más precisa; no prometas una voz concreta ni imitar a un artista.",
                "[GOTV MUSIC profile — generated on THIS Mac with ACE-Step 1.5]\n\
                 - Music or a song: call `generar_musica` (it is a TOOL). `prompt` in ENGLISH and concrete: genre, subgenre, instruments, mood, voice type (e.g. «melancholic indie folk, male vocals, fingerpicked acoustic guitar, soft strings»).\n\
                 - Song with vocals: write the full lyrics YOURSELF IN THE LANGUAGE THE USER ASKED FOR (if they asked for Spanish, the lyrics are in Spanish even though the `prompt` is in English), with its structure in tags on their own lines ([verse], [chorus], [bridge], [outro]); pass `letra` and `idioma` (es, en, pt…). Without lyrics it is instrumental.\n\
                 - If the user gives tempo, key or time signature, pass them (`bpm`, `tono` like «A minor», `compas` 2/3/4/6); otherwise ACE-Step's LM picks them. Default duration 30 s (5–240). It takes ~20 s per 30 s: tell the user.\n\
                 - With the user's audio: ANOTHER VERSION of their song → `generar_musica` with `audio` (the path) and the new style in `prompt` (`fuerza` 0–1: 1 follows the song, ~0.2 only the idea); REDO only a span → also `repintar` «from-to» in seconds (e.g. «10-18»). It takes ~7 s per 30 s of audio.\n\
                 - Sound effect (rain, a door, an explosion, footsteps, ambience, an animal): `generar_efecto` with the sound in ENGLISH and concrete (source, action, place, how it sounds); 1–30 s, default 5. It takes 3–4 minutes whatever the duration: say so first. Do not use it for music.\n\
                 - A piece that does not convince is redone with another `semilla` or a more precise description; do not promise a specific voice or imitating an artist.",
            )),
            Perfil::Programar => Some(crate::i18n::t(
                "[Perfil PROGRAMACIÓN de GOTV — leyes que valen en todo lenguaje]\n\
                 1) Entiende antes de cambiar: lee el código que tocas y sus pruebas. 2) Cambio mínimo que resuelve; nada de reescrituras no pedidas. \
                 3) Nombres que dicen qué hacen; funciones cortas de una sola tarea; sin duplicar lógica. 4) Valida las entradas en el borde; errores explícitos, jamás tragados. \
                 5) Sin secretos en el código. 6) Para un error: primero una prueba que lo reproduce y falla, luego el arreglo. \
                 7) Antes de terminar corre el formateador, el linter y las pruebas del proyecto y dilo con su resultado real. 8) Dependencias mínimas y que EXISTAN (compruébalo antes de instalar). \
                 9) La API de una librería es la de la versión INSTALADA aquí, no la que recuerdas: si dudas de un nombre o una firma, míralo (`package` con registry «installed» —búscala con tool_search—). \
                 10) Un backend nuevo (API REST de unos datos) no se escribe a mano: `api_nueva` —búscala con tool_search— lo crea en Hono, FastAPI o Axum con validación, configuración y pruebas; tú añades lo que falte.\n\
                 Donde más se tropieza (medido):\n\
                 11) Texto «sin distinguir mayúsculas ni tildes»: normaliza LOS DOS lados con la misma función — Python: `unicodedata.normalize('NFKD', s)`, quita las marcas (`unicodedata.combining(c)`) y luego `.casefold()`; JS: `s.normalize('NFD').replace(/\\p{M}/gu, '').toLowerCase()`. En SQLite `LOWER`, `UPPER` y `LIKE` sólo pliegan ASCII («ÁRBOL» sigue con Á): filtra en el código o registra la función con `create_function`. Pruébalo con MAYÚSCULA CON TILDE («ÁRBOL», «MUÑOZ»), no sólo en minúsculas. Archivos de texto: siempre `encoding='utf-8'`. \
                 12) Cada error de la especificación con SU excepción: «no existe» y «ya hecho» son casos distintos; comprueba primero que exista. \
                 13) Un dato en un solo lugar: no guardes lo mismo dos veces (un total guardado junto a las partes que lo suman); lo derivado se calcula. \
                 14) Recorre la especificación fila por fila: una prueba por cada fila y por cada ejemplo LITERAL que trae (mensajes, posiciones, formatos exactos), y las secuencias que describe (la misma operación dos veces, una operación tras otra que falló). Usa los nombres y casos de TU especificación, no los de estos ejemplos.",
                "[GOTV PROGRAMMING profile — laws that hold in every language]\n\
                 1) Understand before changing: read the code you touch and its tests. 2) Minimal change that solves it; no unrequested rewrites. \
                 3) Names that say what they do; short single-purpose functions; no duplicated logic. 4) Validate inputs at the edge; explicit errors, never swallowed. \
                 5) No secrets in code. 6) For a bug: first a test that reproduces it and fails, then the fix. \
                 7) Before finishing run the project's formatter, linter and tests and report their real result. 8) Minimal dependencies that EXIST (check before installing). \
                 9) A library's API is that of the version INSTALLED here, not the one you remember: if you doubt a name or a signature, look it up (`package` with registry «installed» —find it with tool_search—). \
                 10) A new backend (REST API for some data) is not written by hand: `api_nueva` —find it with tool_search— creates it in Hono, FastAPI or Axum with validation, configuration and tests; you add what is missing.\n\
                 Where it trips most (measured):\n\
                 11) Text «ignoring case and accents»: normalise BOTH sides with the same function — Python: `unicodedata.normalize('NFKD', s)`, drop the marks (`unicodedata.combining(c)`), then `.casefold()`; JS: `s.normalize('NFD').replace(/\\p{M}/gu, '').toLowerCase()`. In SQLite `LOWER`, `UPPER` and `LIKE` only fold ASCII («ÁRBOL» keeps its Á): filter in code or register the function with `create_function`. Test it with an ACCENTED CAPITAL («ÁRBOL», «MUÑOZ»), not only lowercase. Text files: always `encoding='utf-8'`. \
                 12) Each error in the spec with ITS exception: «does not exist» and «already done» are different cases; check existence first. \
                 13) One fact in one place: do not store the same thing twice (a stored total next to the parts that add up to it); derived data is computed. \
                 14) Walk the spec row by row: one test per row and per LITERAL example it gives (messages, positions, exact formats), and the sequences it describes (the same operation twice, an operation after one that failed). Use the names and cases of YOUR spec, not those of these examples.",
            )),
        }
    }
}

/// M4/M5: ¿la orden pide GENERAR una imagen? Un pedido de imagen (verbo de crear + algo que se
/// ve) o una ilustración/dibujo; nunca si es trabajo de código sobre imágenes (redimensionar,
/// comprimir, un script…).
fn pide_imagen(o: &str) -> bool {
    let codigo = [".py", ".js", ".ts", ".rs", "script", "función", "funcion", "redimensiona", "comprime", "optimiza", "recorta", "resize", "compress", " api", "base de datos", "reproductor", "player"]; // juez-idioma: datos (palabras de una orden o un valor en español)
    if codigo.iter().any(|p| o.contains(p)) {
        return false;
    }
    let directo = ["ilustración", "ilustracion", "illustration", "dibuja", "dibújame", "dibujame", "draw ", "pinta un", "pinta una", "píntame", "pintame", "fondo de pantalla", "wallpaper"]; // juez-idioma: datos (palabras de una orden o un valor en español)
    let verbo = ["genera", "hazme", "haz ", "crea", "créame", "creame", "quiero", "make", "create", "generate", "imagina"]; // juez-idioma: datos (palabras de una orden o un valor en español)
    let cosa = ["imagen", "foto", "fotografía", "retrato", "póster", "poster", "image", "photo", "picture", "portrait"]; // juez-idioma: datos (palabras de una orden o un valor en español)
    directo.iter().any(|p| o.contains(p)) || (verbo.iter().any(|p| o.contains(p)) && cosa.iter().any(|p| o.contains(p)))
}

/// M6: ¿la orden pide GENERAR un video o ANIMAR una imagen? Nunca si es trabajo de código o de
/// edición sobre videos (un reproductor, ffmpeg, recortar, convertir…).
fn pide_video(o: &str) -> bool {
    let codigo = [".py", ".js", ".ts", ".rs", "script", "función", "funcion", " api", "base de datos", "reproductor", "player", "ffmpeg", "recorta", "corta el", "convierte", "comprime", "edita el", "sube el", "subtítul", "subtitul", "youtube", "descarga"]; // juez-idioma: datos (palabras de una orden o un valor en español)
    if codigo.iter().any(|p| o.contains(p)) {
        return false;
    }
    let verbo = ["genera", "hazme", "haz ", "crea", "créame", "creame", "quiero", "make", "create", "generate", "imagina", "graba"]; // juez-idioma: datos (palabras de una orden o un valor en español)
    let cosa = ["video", "vídeo", "clip", "timelapse", "time-lapse", "cámara lenta", "slow motion", "cortometraje"]; // juez-idioma: datos (palabras de una orden o un valor en español)
    let animar = ["anima ", "anímala", "animala", "anímame", "animame", "animar ", "animate ", "dale vida", "dale movimiento", "bring to life"]; // juez-idioma: datos (palabras de una orden o un valor en español)
    let imagen = ["imagen", "foto", "picture", "image", "photo", ".png", ".jpg", ".jpeg", ".webp"]; // juez-idioma: datos (palabras de una orden o un valor en español)
    (verbo.iter().any(|p| o.contains(p)) && cosa.iter().any(|p| o.contains(p))) || (animar.iter().any(|p| o.contains(p)) && imagen.iter().any(|p| o.contains(p)))
}

/// M6: un video que GOTV pide DIRECTO (sin esperar a que el modelo elija la herramienta). Medido
/// (Portador, 9B, 2026-11-13): con el perfil Video, la nota con la ruta exacta y `generar_video` a la
/// vista, el modelo hizo `mdls`, buscó la imagen, probó una skill de Gemini e instaló paquetes con pip,
/// y nunca llamó a `generar_video`. Ahora el modelo sólo escribe el prompt en inglés.
#[derive(Debug, Clone, PartialEq)]
pub struct PedidoVideo {
    /// el texto que describe el video (la orden; en un reintento, la orden original)
    pub texto: String,
    /// el primer cuadro, resuelto en el proyecto
    pub imagen: Option<std::path::PathBuf>,
    pub segundos: f32,
    pub vertical: bool,
    /// Ola PRO: el video del proyecto a CONTINUAR (sale unido al nuevo tramo)
    pub continuar: Option<std::path::PathBuf>,
}

/// ¿«Inténtalo de nuevo», «otra vez», «try again»…?
fn es_reintento(o: &str) -> bool {
    ["inténtalo de nuevo", "intentalo de nuevo", "intenta de nuevo", "otra vez", "de nuevo", "repite", "repítelo", "repitelo", "vuelve a intentar", "hazlo otra", "try again", "retry", "once more"].iter().any(|p| o.contains(p)) // juez-idioma: datos (palabras de una orden o un valor en español)
}

/// La primera imagen del proyecto que nombra un texto (resuelta).
pub fn imagen_nombrada(root: &std::path::Path, texto: &str) -> Option<std::path::PathBuf> {
    texto.split(|c: char| c.is_whitespace() || "\"'«»“”`(),;".contains(c)).map(|t| t.trim_end_matches(['.', ':', '!', '?'])).filter(|t| !t.is_empty()).find_map(|t| resolver_imagen(root, t))
}

/// Los segundos que pide un texto («de 3 segundos», «5 s», «4 seconds»), de 1 a 5; si no, 2.
fn segundos_pedidos(o: &str) -> f32 {
    let p: Vec<&str> = o.split(|c: char| c.is_whitespace() || ",;.()".contains(c)).filter(|t| !t.is_empty()).collect();
    for (i, t) in p.iter().enumerate() {
        let (num, resto) = t.split_at(t.find(|c: char| !c.is_ascii_digit()).unwrap_or(t.len()));
        let Ok(n) = num.parse::<f32>() else { continue };
        let unidad = if resto.is_empty() { p.get(i + 1).copied().unwrap_or("") } else { resto };
        if ["s", "seg", "segs", "segundo", "segundos", "sec", "secs", "second", "seconds"].contains(&unidad) {
            return n.clamp(1.0, 5.0);
        }
    }
    2.0
}

/// ¿Es una pregunta (sobre el video, el tiempo…) y no un pedido?
pub fn es_pregunta(t: &str) -> bool {
    t.contains('?') || ["cuánto", "cuanto", "cómo", "como ", "qué ", "que es", "por qué", "porque", "how ", "what ", "why "].iter().any(|p| t.trim_start_matches('¿').starts_with(p)) // juez-idioma: datos (palabras de una orden o un valor en español)
}

/// Ola PRO: con el modo profesional del perfil Video encendido, TODA orden que no sea una pregunta
/// (ni un comando `/`) es la descripción del video, aunque no diga «video»: el panel ya dijo cómo.
pub fn pedido_de_video_pro(root: &std::path::Path, orden: &str, anteriores: &[String]) -> Option<PedidoVideo> {
    if let Some(p) = pedido_de_video(root, orden, anteriores) {
        return Some(p);
    }
    let o = orden.trim().to_lowercase();
    if o.is_empty() || o.starts_with('/') || es_pregunta(&o) {
        return None;
    }
    let continuar = video_a_continuar(root, orden);
    let imagen = if continuar.is_some() { None } else { imagen_nombrada(root, orden) };
    Some(PedidoVideo { texto: orden.trim().to_string(), imagen, segundos: segundos_pedidos(&o), vertical: o.contains("vertical"), continuar })
}

/// Ola PRO: ¿la orden pide SEGUIR un video («continúa el video», «extiéndelo», «alarga el clip»)?
/// Entonces es el video que nombra, o el último generado del proyecto (`generado/video-*.mp4`).
pub fn video_a_continuar(root: &std::path::Path, orden: &str) -> Option<std::path::PathBuf> {
    let o = orden.to_lowercase();
    let verbo = ["continúa", "continua", "continuar", "extiende", "extiéndelo", "extiendelo", "extender", "alarga", "alárga", "prolonga", "prolónga", "sigue", "continue", "extend"].iter().any(|p| o.contains(p)); // juez-idioma: datos (palabras de una orden o un valor en español)
    // y que hable de un video («continúa con el código» no es esto)
    let objeto = ["video", "vídeo", "clip", ".mp4", "extiéndelo", "extiendelo", "alárgalo", "alargalo", "continúalo", "continualo"].iter().any(|p| o.contains(p)); // juez-idioma: datos (palabras de una orden o un valor en español)
    if !(verbo && objeto) {
        return None;
    }
    let nombrado = orden.split(|c: char| c.is_whitespace() || "\"'«»“”`(),;".contains(c)).map(|t| t.trim_end_matches(['.', ':', '!', '?'])).filter(|t| t.to_lowercase().ends_with(".mp4")).map(|t| crate::paths::resolve(root, t)).find(|p| p.is_file());
    nombrado.or_else(|| {
        let mut v: Vec<(std::time::SystemTime, std::path::PathBuf)> = std::fs::read_dir(root.join("generado"))
            .ok()?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("mp4")) && !p.to_string_lossy().ends_with(".uniendo.mp4"))
            .filter_map(|p| Some((std::fs::metadata(&p).ok()?.modified().ok()?, p)))
            .collect();
        v.sort();
        v.pop().map(|(_, p)| p)
    })
}

/// El video que pide ESTA orden con el perfil Video, o `None` si no pide uno (una pregunta, una
/// charla). Lo pide si dice que se haga un video o que se anime una imagen, si nombra una imagen del
/// proyecto, si pide CONTINUAR un video, o si es un REINTENTO de un pedido de video anterior
/// (`anteriores`: las órdenes previas del usuario, la más reciente primero).
pub fn pedido_de_video(root: &std::path::Path, orden: &str, anteriores: &[String]) -> Option<PedidoVideo> {
    let o = orden.to_lowercase();
    // con el perfil Video elegido basta con nombrar el video (sin verbo), salvo en una pregunta
    let pregunta = es_pregunta;
    let nombra = |t: &str| ["video", "vídeo", "clip", "timelapse", "time-lapse"].iter().any(|p| t.contains(p)) && !pregunta(t); // juez-idioma: datos (palabras de una orden o un valor en español)
    let pide = |t: &str| {
        let l = t.to_lowercase();
        pide_video(&l) || (nombra(&l) && pide_video(&format!("haz {l}"))) || imagen_nombrada(root, t).is_some()
    };
    let continuar = if pregunta(&o) { None } else { video_a_continuar(root, orden) };
    let base: String = if continuar.is_some() || pide(orden) {
        orden.to_string()
    } else if es_reintento(&o) {
        anteriores.iter().find(|a| pide(a))?.clone()
    } else {
        return None;
    };
    let b = base.to_lowercase();
    // al continuar, el primer cuadro sale del video: no se anima además una imagen
    let imagen = if continuar.is_some() { None } else { imagen_nombrada(root, orden).or_else(|| imagen_nombrada(root, &base)) };
    let segundos = if segundos_pedidos(&o) != 2.0 { segundos_pedidos(&o) } else { segundos_pedidos(&b) };
    Some(PedidoVideo { texto: base, imagen, segundos, vertical: o.contains("vertical") || b.contains("vertical"), continuar })
}

/// M6: la imagen del proyecto a la que se refiere un nombre de la orden: tal cual; si no existe, el
/// MISMO nombre con extensión de imagen, sin mirar mayúsculas: «images» → `images.jpeg`, y una
/// extensión mal escrita también («images.gpeg» → `images.jpeg`). Medido (Portador, 9B,
/// 2026-11-13): con «la imagen llamada images» y luego «images.gpeg» el modelo buscó una carpeta
/// `images/` y archivos «*gpeg*», nunca dio con `images.jpeg` y no llegó a llamar a `generar_video`.
pub fn resolver_imagen(root: &std::path::Path, nombre: &str) -> Option<std::path::PathBuf> {
    let nombre = nombre.trim();
    if nombre.is_empty() {
        return None;
    }
    let p = crate::paths::resolve(root, nombre);
    if crate::images::is_image(&p) && p.is_file() {
        return Some(p);
    }
    // sin extensión (o con una que no es de imagen o está mal escrita): se busca por el nombre base
    let tiene_ext = p.extension().is_some_and(|e| !e.is_empty() && e.len() <= 5);
    let base = if tiene_ext { p.with_extension("") } else { p.clone() };
    let (dir, raiz) = (base.parent()?, base.file_name()?.to_string_lossy().to_lowercase());
    if raiz.len() < 2 {
        return None;
    }
    let mut hallados: Vec<std::path::PathBuf> = std::fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|f| f.is_file() && crate::images::is_image(f) && f.file_stem().is_some_and(|s| s.to_string_lossy().to_lowercase() == raiz))
        .collect();
    hallados.sort();
    hallados.into_iter().next()
}

/// Las imágenes del proyecto (su raíz y `generado/`, las más recientes primero), para cuando la orden
/// habla de «la imagen» sin un nombre que se resuelva.
fn imagenes_del_proyecto(root: &std::path::Path, max: usize) -> Vec<String> {
    let mut v: Vec<(std::time::SystemTime, String)> = Vec::new();
    for d in [root.to_path_buf(), root.join("generado")] {
        for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            let f = e.path();
            if f.is_file() && crate::images::is_image(&f) {
                let t = e.metadata().and_then(|m| m.modified()).unwrap_or(std::time::UNIX_EPOCH);
                v.push((t, f.strip_prefix(root).unwrap_or(&f).to_string_lossy().into_owned()));
            }
        }
    }
    v.sort_by_key(|x| std::cmp::Reverse(x.0));
    v.into_iter().take(max).map(|(_, n)| n).collect()
}

/// M6: las imágenes del proyecto que nombra la orden (resueltas: «images» → `images.jpeg`), y cómo se
/// animan (igual que los audios en Música: sin la nota el modelo tiende a abrirlas, describirlas o
/// buscarlas en vez de pasarlas como `imagen`). Si la orden habla de una imagen sin nombre que se
/// resuelva, se le dicen las del proyecto. `None` si no viene al caso.
pub fn nota_imagenes(root: &std::path::Path, orden: &str) -> Option<String> {
    let mut vistas: Vec<(String, String)> = Vec::new();
    for t in orden.split(|c: char| c.is_whitespace() || "\"'«»“”`(),;".contains(c)) {
        let t = t.trim_end_matches(['.', ':', '!', '?']);
        if t.is_empty() || vistas.iter().any(|(v, _)| v == t) {
            continue;
        }
        if let Some(p) = resolver_imagen(root, t) {
            let rel = p.strip_prefix(root).unwrap_or(&p).to_string_lossy().into_owned();
            if !vistas.iter().any(|(_, r)| *r == rel) {
                vistas.push((t.to_string(), rel));
            }
        }
    }
    if vistas.is_empty() {
        let o = orden.to_lowercase();
        let habla_de_imagen = ["imagen", "foto", "image", "photo", "picture", "retrato", "dibujo"].iter().any(|p| o.contains(p)); // juez-idioma: datos (palabras de una orden o un valor en español)
        let hay = imagenes_del_proyecto(root, 12);
        if !habla_de_imagen || hay.is_empty() {
            return None;
        }
        let lista: Vec<String> = hay.iter().map(|v| format!("`{v}`")).collect();
        return Some(crate::tr!(
            "[Imágenes del proyecto (las más recientes primero): {}. Si la orden se refiere a una, pásala tal cual como `imagen` en generar_video; NO la busques con bash. Si no sabes cuál es, `view_image` o pregunta.]",
            "[Project images (most recent first): {}. If the order refers to one, pass it as is as `imagen` to generar_video; do NOT search for it with bash. If unsure which, `view_image` or ask.]",
            lista.join(", ")
        ));
    }
    let lista: Vec<String> = vistas.iter().map(|(t, r)| if t == r { format!("`{r}`") } else { crate::tr!("«{t}» = `{r}`", "«{t}» = `{r}`") }).collect();
    Some(crate::tr!(
        "[Imagen del usuario: {}. Para ANIMARLA pásala tal cual como `imagen` en generar_video (es el primer cuadro; el tamaño sale de su proporción) y en `prompt` describe el MOVIMIENTO en inglés. No hace falta buscarla ni abrirla; si quieres saber qué muestra, `view_image`.]",
        "[User's image: {}. To ANIMATE it pass it as is as `imagen` to generar_video (it is the first frame; the size follows its aspect ratio) and describe the MOTION in English in `prompt`. No need to search for or open it; to see what it shows, `view_image`.]",
        lista.join(", ")
    ))
}

/// M7: ¿la orden pide CREAR música? Un verbo de crear + una canción/música/beat, o «compón»; nunca
/// escuchar o reproducir música, ni código sobre audio.
/// M7d: los audios del proyecto que nombra la orden, con su duración, y cómo se usan (medido con
/// el 9B: «mi canción en mi_cancion.m4a» → intentaba leerlo e investigarlo en vez de pasarlo como
/// `audio`). `None` si la orden no nombra ninguno que exista.
pub fn nota_audios(root: &std::path::Path, orden: &str) -> Option<String> {
    let mut vistos: Vec<String> = Vec::new();
    for t in orden.split(|c: char| c.is_whitespace() || "\"'«»“”`(),;".contains(c)) {
        let t = t.trim_end_matches(['.', ':', '!', '?']);
        if t.is_empty() || vistos.iter().any(|v| v == t) {
            continue;
        }
        let p = crate::paths::resolve(root, t);
        if crate::audio::is_audio(&p) && p.is_file() {
            vistos.push(t.to_string());
        }
    }
    if vistos.is_empty() {
        return None;
    }
    let lista: Vec<String> = vistos
        .iter()
        .map(|v| match crate::audio::seconds(&crate::paths::resolve(root, v)) {
            Some(s) => format!("`{v}` ({s:.1} s)"),
            None => format!("`{v}`"),
        })
        .collect();
    Some(crate::tr!(
        "[Audio del usuario: {}. NO lo abras ni lo investigues: pásalo tal cual como `audio` en generar_musica.\n- OTRA VERSIÓN de la canción entera (otro estilo): `audio` + `prompt` con el estilo nuevo, SIN `repintar`.\n- Rehacer SÓLO un tramo y dejar el resto igual: `audio` + `repintar` «desde-hasta» en segundos + `prompt` con cómo debe sonar ese tramo.\n- Si pide las dos cosas, son DOS llamadas: la versión entera SIN `repintar` y el tramo CON `repintar`.]",
        "[User's audio: {}. Do NOT open or investigate it: pass it as is as `audio` to generar_musica.\n- ANOTHER VERSION of the whole song (another style): `audio` + `prompt` with the new style, WITHOUT `repintar`.\n- Redo ONLY a span and keep the rest: `audio` + `repintar` «from-to» in seconds + `prompt` with how that span should sound.\n- If they ask for both, that is TWO calls: the whole version WITHOUT `repintar` and the span WITH `repintar`.]",
        lista.join(", ")
    ))
}

fn pide_musica(o: &str) -> bool {
    let fuera = [".py", ".js", ".ts", ".rs", "script", "función", "funcion", " api", "reproduce", "reprodúce", "escuchar", "escucha ", "oír", "pon música", "pon musica", "pon la canción", "play ", "listen", "spotify", "youtube", "playlist", "lista de reproducción"]; // juez-idioma: datos (palabras de una orden o un valor en español)
    if fuera.iter().any(|p| o.contains(p)) {
        return false;
    }
    let directo = ["compón", "componme", "compóneme", "compose", "jingle", "banda sonora", "soundtrack", "efecto de sonido", "efectos de sonido", "sound effect", "sfx", "foley"]; // juez-idioma: datos (palabras de una orden o un valor en español)
    let verbo = ["genera", "hazme", "haz ", "crea", "créame", "creame", "quiero", "make", "create", "generate", "escribe y canta"]; // juez-idioma: datos (palabras de una orden o un valor en español)
    let cosa = ["canción", "cancion", "música", "musica", "song", "beat", "melodía", "melodia", "instrumental", "pista musical", "tema musical", "lo-fi", "lofi"]; // juez-idioma: datos (palabras de una orden o un valor en español)
    directo.iter().any(|p| o.contains(p)) || (verbo.iter().any(|p| o.contains(p)) && cosa.iter().any(|p| o.contains(p)))
}

/// Auto: ¿la orden es de imagen, de video, de música o de diseño web? (palabras de la orden; ante la duda, programar).
pub fn detectar(orden: &str) -> Perfil {
    let o = orden.to_lowercase();
    if pide_musica(&o) {
        return Perfil::Musica;
    }
    // antes que la imagen: «anima esta foto» o «un video de…» nombran también una imagen
    if pide_video(&o) {
        return Perfil::Video;
    }
    if pide_imagen(&o) {
        return Perfil::Imagen;
    }
    let diseno = [
        // juez-idioma: bloque — datos (palabras de una orden en español)
        "página web",
        "pagina web",
        "landing",
        "sitio web",
        "web de",
        "página de",
        "pagina de",
        "diseña",
        "diseño",
        "diseno",
        "rediseña",
        "isométric",
        "isometric",
        "portafolio",
        "portfolio",
        "hero",
        "maqueta",
        "mockup",
        "interfaz",
        "ui ",
        "ux",
        "panel de",
        "dashboard",
        "tienda en línea",
        "tienda online",
        "web page",
        "website",
        "design",
        "app web",
        "aplicación web",
        "aplicacion web",
        "frontend",
        "front-end",
    ];
    if diseno.iter().any(|p| o.contains(p)) {
        Perfil::Diseno
    } else {
        Perfil::Programar
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_acierta_en_un_juego_de_ordenes() {
        let diseno = [
            "hazme una landing para mi cafetería",
            "diseña la página web de un abogado",
            "quiero un dashboard para ver mis ventas",
            "rediseña el hero de index.html, se ve feo",
            "haz una isométrica de la arquitectura",
            "crea una app web para gestionar clientes",
            "portafolio para una fotógrafa",
            "mejora la interfaz del formulario de registro",
            "make a website for my gym",
            "página de precios con tres planes",
            "tienda online de ropa sostenible",
            "arma el frontend del panel de pedidos",
            "la web de mi ONG",
            "maqueta de una app de inglés",
            "pagina web para una conferencia",
        ];
        let programar = [
            "arregla el test que falla en calc.py",
            "agrega una función que cuente palabras en texto.py",
            "crea un script que descargue el clima y lo guarde en csv",
            "migra la base de datos a postgres",
            "por qué este programa en rust no compila",
            "optimiza la consulta sql de reportes",
            "escribe pruebas para el módulo de pagos",
            "configura el linter del proyecto",
            "crea una API REST con FastAPI",
            "refactoriza utils.go",
            "instala las dependencias y corre las pruebas",
            "busca la mejor librería de fechas para node",
            "haz un bot de telegram",
            "lee el pdf y resume la hoja de datos",
            "corrige el bug de concurrencia en el worker",
        ];
        let aciertos = diseno
            .iter()
            .filter(|o| detectar(o) == Perfil::Diseno)
            .count()
            + programar
                .iter()
                .filter(|o| detectar(o) == Perfil::Programar)
                .count();
        let total = diseno.len() + programar.len();
        assert!(
            aciertos * 100 >= total * 90,
            "Auto acertó {aciertos}/{total} (meta ≥ 90 %)"
        );
        // M4/M5: ninguna orden de diseño o de código se va a Imagen
        for o in diseno.iter().chain(programar.iter()) {
            assert_ne!(detectar(o), Perfil::Imagen, "«{o}» no pide una imagen");
        }
    }

    /// M4/M5: los pedidos de imagen van a Imagen; el código SOBRE imágenes, no.
    #[test]
    fn auto_reconoce_los_pedidos_de_imagen() {
        let imagen = [
            "genera una imagen de un zorro en la nieve",
            "hazme una foto de un café en París al atardecer",
            "dibuja un gato astronauta",
            "quiero un retrato estilo óleo de una abuela",
            "crea una ilustración isométrica de una panadería",
            "un fondo de pantalla de montañas para mi Mac",
            "make an image of a red bicycle",
            "draw a castle on a hill",
            "convierte esta foto en acuarela, créame una imagen así",
            "hazme el póster del concierto",
        ];
        for o in imagen {
            assert_eq!(detectar(o), Perfil::Imagen, "«{o}»");
            assert_eq!(Perfil::Auto.para(o), Perfil::Imagen, "«{o}»");
        }
        let codigo = [
            "redimensiona las imágenes de la carpeta fotos a 800 px",
            "crea un script que comprima las fotos",
            "optimiza las imágenes de la web",
            "escribe una función en rust que lea una imagen png",
            "hazme una api que guarde fotos en la base de datos",
            "crea una página con un reproductor de video",
            "make a video player in react",
        ];
        for o in codigo {
            assert_ne!(detectar(o), Perfil::Imagen, "«{o}» es código, no un pedido de imagen");
        }
        assert_eq!(Perfil::parse("imagen"), Perfil::Imagen);
        assert_eq!(Perfil::Imagen.nombre(), "imagen");
        assert!(Perfil::Imagen.herramientas().contains(&"generar_imagen"));
        assert!(!Perfil::Imagen.herramientas().contains(&"generar_video") && !Perfil::Imagen.nota().unwrap().contains("generar_video"), "el video tiene su perfil");
        assert!(Perfil::Imagen.nota().unwrap().contains("guia"));
    }

    /// M6: generar un video o ANIMAR una imagen va a Video (antes que Imagen); el código o la edición
    /// sobre videos, no; y una imagen nombrada en la orden se anuncia para pasarla como `imagen`.
    #[test]
    fn auto_reconoce_los_pedidos_de_video() {
        let video = [
            "hazme un video de un zorro corriendo en la nieve",
            "genera un vídeo corto de olas en la playa",
            "make a video of a city at night",
            "crea un clip de 3 segundos de lluvia en la ventana",
            "anima esta foto: gato.png",
            "anima la imagen del logo para que gire",
            "dale vida a mi foto de la playa",
            "animate this image of a lighthouse",
            "quiero un timelapse de nubes sobre la montaña",
        ];
        for o in video {
            assert_eq!(detectar(o), Perfil::Video, "«{o}»");
            assert_eq!(Perfil::Auto.para(o), Perfil::Video, "«{o}»");
        }
        let no = [
            "crea una página con un reproductor de video",
            "make a video player in react",
            "convierte el video a mp4 con ffmpeg",
            "recorta el video de la boda a 30 segundos",
            "descarga el video de youtube",
            "genera una imagen de un zorro en la nieve",
            "haz una animación css para el botón",
        ];
        for o in no {
            assert_ne!(detectar(o), Perfil::Video, "«{o}» no pide generar un video");
        }
        assert_eq!(detectar("genera una imagen de un zorro en la nieve"), Perfil::Imagen);
        assert_eq!(Perfil::parse("vídeo"), Perfil::Video);
        assert_eq!(Perfil::Video.nombre(), "video");
        assert_eq!(Perfil::Video.herramientas(), &["generar_video", "generar_imagen", "view_image"]);
        let n = Perfil::Video.nota().unwrap();
        assert!(n.contains("generar_video") && n.contains("`imagen`") && n.contains("48 minutos"), "{n}");
        // la imagen que nombra la orden (si existe) se anuncia para pasarla como `imagen`
        let root = crate::CarpetaPrueba::sin_crear("gotv-perfil-video");
        std::fs::create_dir_all(root.join("fotos")).unwrap();
        std::fs::write(root.join("fotos/gato.png"), b"png").unwrap();
        let n = nota_imagenes(&root, "anima fotos/gato.png, que estire la pata; y no-existe.jpg").expect("la nota");
        assert!(n.contains("`fotos/gato.png`") && !n.contains("no-existe") && n.contains("`imagen`"), "{n}");
        assert!(nota_imagenes(&root, "hazme un video de olas").is_none());
        // el caso del Portador (2026-11-13): `images.jpeg` en la raíz y una carpeta `images/` vacía;
        // la orden dice «llamada images» y luego «images.gpeg»: las dos dan con el archivo
        std::fs::write(root.join("images.jpeg"), b"jpg").unwrap();
        std::fs::create_dir_all(root.join("images")).unwrap();
        let n = nota_imagenes(&root, "dentro de la carpeta del proyecto hay la imagen de una mujer llamada images quiero hagas un video de ella haciendo gestos tiernos").expect("la nota");
        assert!(n.contains("«images» = `images.jpeg`"), "{n}");
        let n = nota_imagenes(&root, "la imagen se llama images.gpeg no una carpeta").expect("la nota");
        assert!(n.contains("`images.jpeg`") && !n.contains("buscarla con bash"), "{n}");
        assert_eq!(resolver_imagen(&root, "IMAGES.GPEG"), Some(root.join("images.jpeg")));
        assert_eq!(resolver_imagen(&root, "fotos/gato.jpg"), Some(root.join("fotos/gato.png")), "extensión equivocada");
        assert_eq!(resolver_imagen(&root, "perro"), None);
        // «la foto» sin nombre: se le dicen las imágenes del proyecto
        let n = nota_imagenes(&root, "anima la foto de la mujer para que sonría").expect("la lista");
        assert!(n.contains("`images.jpeg`") && n.contains("NO la busques"), "{n}");
        // el pedido DIRECTO: la orden del Portador lleva su imagen; el reintento recoge la anterior
        let orden = "dentro de la carpeta del poryecto hay la imagen de una mujer  llamada images.jpeg quiero hagas un video de ella haciendo gestos tiernos y que tenga una voz dulce";
        let p = pedido_de_video(&root, orden, &[]).expect("pide video");
        assert_eq!((p.imagen.clone(), p.segundos, p.vertical), (Some(root.join("images.jpeg")), 2.0, false));
        let r = pedido_de_video(&root, "intentalo de nuevo cambie algo en la app", &["¿cuánto tarda?".into(), orden.into()]).expect("el reintento");
        assert_eq!((r.texto.as_str(), r.imagen), (orden, Some(root.join("images.jpeg"))));
        assert_eq!(pedido_de_video(&root, "haz un clip vertical de 4 segundos de olas", &[]).map(|p| (p.segundos, p.vertical, p.imagen)), Some((4.0, true, None)));
        assert_eq!(pedido_de_video(&root, "un video de 9 s de lluvia", &[]).map(|p| p.segundos), Some(5.0), "tope de 5 s");
        assert!(pedido_de_video(&root, "¿cuánto tarda un video?", &[]).is_none(), "una pregunta no genera");
        assert!(pedido_de_video(&root, "intentalo de nuevo", &["arregla el css".into()]).is_none(), "un reintento de otra cosa no");

        // Ola PRO · CONTINUAR: el último video de generado/ o el que se nombra; sin video no hay qué
        // seguir, y «continúa con el código» no es esto
        assert_eq!(pedido_de_video(&root, "continúa el video con el zorro saltando", &[]).map(|p| p.continuar), Some(None), "sin videos en el proyecto: es un video nuevo");
        std::fs::create_dir_all(root.join("generado")).unwrap();
        std::fs::write(root.join("generado/video-1.mp4"), b"a").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(root.join("generado/video-2.mp4"), b"b").unwrap();
        std::fs::write(root.join("mio.mp4"), b"c").unwrap();
        let p = pedido_de_video(&root, "continúa el video con el zorro saltando al río, 3 segundos", &[]).expect("continuar");
        assert_eq!((p.continuar.as_deref().and_then(|c| c.file_name()).map(|f| f.to_string_lossy().into_owned()), p.segundos, p.imagen), (Some("video-2.mp4".into()), 3.0, None));
        let p = pedido_de_video(&root, "extiende mio.mp4 con un atardecer", &[]).expect("el nombrado");
        assert!(p.continuar.unwrap().ends_with("mio.mp4"));
        assert!(pedido_de_video(&root, "continúa con el código", &[]).is_none());
        assert!(pedido_de_video(&root, "¿puedes continuar el video?", &[]).is_none(), "una pregunta no");
        assert!(pedido_de_video_pro(&root, "alárgalo con el zorro durmiendo", &[]).unwrap().continuar.is_some());
    }

    /// M7: crear música va a Música; escuchar, reproducir o programar con audio, no.
    #[test]
    fn la_orden_lleva_los_audios_que_nombra() {
        let d = crate::CarpetaPrueba::nueva("gotv-nota-audios");
        std::fs::create_dir_all(d.join("musica")).unwrap();
        std::fs::write(d.join("musica/mi tema.wav"), b"RIFF").unwrap();
        std::fs::write(d.join("mi_cancion.m4a"), b"x").unwrap();
        let n = nota_audios(&d, "Tengo mi canción en mi_cancion.m4a. Hazme una versión jazz").unwrap();
        assert!(n.contains("`mi_cancion.m4a`") && n.contains("SIN `repintar`") && n.contains("DOS llamadas"), "{n}");
        assert!(nota_audios(&d, "rehaz el coro de «otra.mp3»").is_none(), "no existe: nada");
        assert!(nota_audios(&d, "hazme una canción de cumpleaños").is_none());
    }

    #[test]
    fn auto_reconoce_los_pedidos_de_musica() {
        let musica = [
            "hazme una canción de cumpleaños para mi mamá",
            "crea un beat de lo-fi para estudiar",
            "compón una melodía tranquila de piano",
            "genera música épica para un tráiler",
            "quiero una canción de reggaetón sobre el verano",
            "make a song about the ocean",
            "un jingle para mi panadería",
            "la banda sonora de un videojuego retro",
            "create an instrumental track with synths",
            // M7c: efectos de sonido, en el mismo perfil
            "hazme un efecto de sonido de una puerta que cruje",
            "genera efectos de sonido de lluvia para mi juego",
            "make a sound effect of a glass breaking",
            "necesito un sfx de explosión",
        ];
        for o in musica {
            assert_eq!(detectar(o), Perfil::Musica, "«{o}»");
            assert_eq!(Perfil::Auto.para(o), Perfil::Musica, "«{o}»");
        }
        let otra = [
            "quiero escuchar música relajante",
            "reproduce una canción de Shakira",
            "pon música en spotify",
            "crea un script que convierta mp3 a wav",
            "hazme una playlist de rock",
            "hazme una imagen de un gato",
        ];
        for o in otra {
            assert_ne!(detectar(o), Perfil::Musica, "«{o}» no pide crear música");
        }
        assert_eq!(Perfil::parse("música"), Perfil::Musica);
        assert_eq!(Perfil::Musica.nombre(), "musica");
        assert_eq!(Perfil::Musica.herramientas(), &["generar_musica", "generar_efecto", "separar_pistas"]);
        assert!(Perfil::Musica.nota().unwrap().contains("[chorus]"));
    }

    #[test]
    fn un_perfil_elegido_manda_sobre_la_orden() {
        assert_eq!(
            Perfil::Programar.para("diseña una landing"),
            Perfil::Programar
        );
        assert_eq!(Perfil::Auto.para("diseña una landing"), Perfil::Diseno);
        assert_eq!(
            Perfil::Auto.para("hola, la clave es 42"),
            Perfil::Auto,
            "en Auto una charla no carga notas"
        );
        assert_eq!(Perfil::parse("Diseño"), Perfil::Diseno);
        assert_eq!(Perfil::parse("x"), Perfil::Auto);
        assert!(Perfil::Diseno.herramientas().contains(&"pagina"));
        assert!(Perfil::Auto.nota().is_none());
        // lo medido en el banco «biblioteca» (35B): tildes con SQLite, excepción por caso, un dato un lugar
        let p = Perfil::Programar.nota().unwrap();
        for clave in ["create_function", "casefold", "ÁRBOL", "encoding", "LITERAL"] {
            assert!(p.contains(clave), "falta «{clave}» en la nota de Programar");
        }
    }

    /// Prueba Cable (2026-11-13): la ley 12 decía «pagar dos veces, devolver dos veces» (de la tarea
    /// de la biblioteca) y el 35B escribió `test_secuencia_pagar_dos_veces` en una CALCULADORA, con
    /// una aserción que contradecía su especificación. Las leyes no llevan datos de ninguna tarea.
    #[test]
    fn las_leyes_no_traen_datos_de_las_tareas_del_banco() {
        // (el idioma global no se toca: las pruebas corren en paralelo; el texto inglés es el mismo)
        let p = Perfil::Programar.nota().unwrap().to_lowercase();
        for palabra in ["pagar", "devolver", "cólera", "garcía", "saldo", "préstamo", "multa", "biblioteca", "pay twice", "balance column", "loan"] {
            assert!(!p.contains(palabra), "la nota de Programar trae «{palabra}», de una tarea del banco");
        }
    }
}
