//! Recursos empaquetados (D2): fuentes variables OFL y iconos ISC, DENTRO del binario.
//! Una página hecha por GOTV no depende de ningún CDN: las fuentes se copian al proyecto
//! junto a su licencia. Origen: Fontsource 5.3.0 (`@fontsource-variable/*`, subconjunto
//! latino, que cubre el español) y `lucide-static` 1.70.0. Archivo generado por el
//! script de NOTAS §D2; las licencias viajan en `recursos/` y en cada proyecto.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Clase {
    Sans,
    Serif,
    Mono,
}

pub struct Fuente {
    pub slug: &'static str,
    pub familia: &'static str,
    pub peso: (u16, u16),
    pub clase: Clase,
    pub woff2: &'static [u8],
    pub licencia: &'static str,
}

pub static FUENTES: &[Fuente] = &[
    Fuente { slug: "inter", familia: "Inter", peso: (100, 900), clase: Clase::Sans, woff2: include_bytes!("../recursos/fuentes/inter.woff2"), licencia: include_str!("../recursos/fuentes/inter.LICENSE") },
    Fuente { slug: "space-grotesk", familia: "Space Grotesk", peso: (300, 700), clase: Clase::Sans, woff2: include_bytes!("../recursos/fuentes/space-grotesk.woff2"), licencia: include_str!("../recursos/fuentes/space-grotesk.LICENSE") },
    Fuente { slug: "fraunces", familia: "Fraunces", peso: (100, 900), clase: Clase::Serif, woff2: include_bytes!("../recursos/fuentes/fraunces.woff2"), licencia: include_str!("../recursos/fuentes/fraunces.LICENSE") },
    Fuente { slug: "manrope", familia: "Manrope", peso: (200, 800), clase: Clase::Sans, woff2: include_bytes!("../recursos/fuentes/manrope.woff2"), licencia: include_str!("../recursos/fuentes/manrope.LICENSE") },
    Fuente { slug: "cormorant-garamond", familia: "Cormorant Garamond", peso: (300, 700), clase: Clase::Serif, woff2: include_bytes!("../recursos/fuentes/cormorant-garamond.woff2"), licencia: include_str!("../recursos/fuentes/cormorant-garamond.LICENSE") },
    Fuente { slug: "bricolage-grotesque", familia: "Bricolage Grotesque", peso: (200, 800), clase: Clase::Sans, woff2: include_bytes!("../recursos/fuentes/bricolage-grotesque.woff2"), licencia: include_str!("../recursos/fuentes/bricolage-grotesque.LICENSE") },
    Fuente { slug: "sora", familia: "Sora", peso: (100, 800), clase: Clase::Sans, woff2: include_bytes!("../recursos/fuentes/sora.woff2"), licencia: include_str!("../recursos/fuentes/sora.LICENSE") },
    Fuente { slug: "jetbrains-mono", familia: "JetBrains Mono", peso: (100, 800), clase: Clase::Mono, woff2: include_bytes!("../recursos/fuentes/jetbrains-mono.woff2"), licencia: include_str!("../recursos/fuentes/jetbrains-mono.LICENSE") },
    Fuente { slug: "archivo", familia: "Archivo", peso: (100, 900), clase: Clase::Sans, woff2: include_bytes!("../recursos/fuentes/archivo.woff2"), licencia: include_str!("../recursos/fuentes/archivo.LICENSE") }, // juez-idioma: datos (nombre de una fuente)
    Fuente { slug: "geist", familia: "Geist", peso: (100, 900), clase: Clase::Sans, woff2: include_bytes!("../recursos/fuentes/geist.woff2"), licencia: include_str!("../recursos/fuentes/geist.LICENSE") },
    Fuente { slug: "newsreader", familia: "Newsreader", peso: (200, 800), clase: Clase::Serif, woff2: include_bytes!("../recursos/fuentes/newsreader.woff2"), licencia: include_str!("../recursos/fuentes/newsreader.LICENSE") },
    Fuente { slug: "dm-sans", familia: "DM Sans", peso: (100, 1000), clase: Clase::Sans, woff2: include_bytes!("../recursos/fuentes/dm-sans.woff2"), licencia: include_str!("../recursos/fuentes/dm-sans.LICENSE") },
    Fuente { slug: "instrument-sans", familia: "Instrument Sans", peso: (400, 700), clase: Clase::Sans, woff2: include_bytes!("../recursos/fuentes/instrument-sans.woff2"), licencia: include_str!("../recursos/fuentes/instrument-sans.LICENSE") },
];

/// Pares tipográficos (titular, texto) que eligen las direcciones de estilo (D3).
pub static PARES: &[(&str, &str, &str)] = &[
    ("tecnico", "space-grotesk", "inter"), // SaaS y producto técnico: geométrica con carácter + texto neutro
    ("editorial", "fraunces", "newsreader"), // revista y contenido largo: serif expresiva + serif de lectura
    ("lujo", "cormorant-garamond", "manrope"), // marca premium: serif fina de alto contraste + sans limpia
    ("jugueton", "bricolage-grotesque", "dm-sans"), // cercano y creativo: grotesca con gracia + sans amable
    ("suizo", "geist", "geist"), // minimal y preciso: una sola familia, jerarquía por peso y tamaño
    ("neon", "sora", "inter"), // oscuro tecnológico: geométrica amplia + texto neutro
    ("brutalista", "archivo", "archivo"), // crudo y directo: grotesca pesada, mayúsculas, bordes duros — juez-idioma: datos (nombre de una fuente)
    ("sobrio", "instrument-sans", "instrument-sans"), // institucional y discreto: una sans contenida
];

pub static MONO: &str = "jetbrains-mono";

pub static ICONOS: &[(&str, &str)] = &[
    ("aperture", include_str!("../recursos/iconos/aperture.svg")),
    ("arrow-down", include_str!("../recursos/iconos/arrow-down.svg")),
    ("arrow-left", include_str!("../recursos/iconos/arrow-left.svg")),
    ("arrow-right", include_str!("../recursos/iconos/arrow-right.svg")),
    ("arrow-up-right", include_str!("../recursos/iconos/arrow-up-right.svg")),
    ("award", include_str!("../recursos/iconos/award.svg")),
    ("baby", include_str!("../recursos/iconos/baby.svg")),
    ("badge-check", include_str!("../recursos/iconos/badge-check.svg")),
    ("banknote", include_str!("../recursos/iconos/banknote.svg")),
    ("bed", include_str!("../recursos/iconos/bed.svg")),
    ("bell", include_str!("../recursos/iconos/bell.svg")),
    ("bike", include_str!("../recursos/iconos/bike.svg")),
    ("book-open", include_str!("../recursos/iconos/book-open.svg")),
    ("briefcase", include_str!("../recursos/iconos/briefcase.svg")),
    ("brush", include_str!("../recursos/iconos/brush.svg")),
    ("building-2", include_str!("../recursos/iconos/building-2.svg")),
    ("calculator", include_str!("../recursos/iconos/calculator.svg")),
    ("calendar", include_str!("../recursos/iconos/calendar.svg")),
    ("camera", include_str!("../recursos/iconos/camera.svg")),
    ("car", include_str!("../recursos/iconos/car.svg")),
    ("chart-column", include_str!("../recursos/iconos/chart-column.svg")),
    ("chart-line", include_str!("../recursos/iconos/chart-line.svg")),
    ("chart-pie", include_str!("../recursos/iconos/chart-pie.svg")),
    ("check", include_str!("../recursos/iconos/check.svg")),
    ("chevron-down", include_str!("../recursos/iconos/chevron-down.svg")),
    ("chevron-left", include_str!("../recursos/iconos/chevron-left.svg")),
    ("chevron-right", include_str!("../recursos/iconos/chevron-right.svg")),
    ("chevron-up", include_str!("../recursos/iconos/chevron-up.svg")),
    ("circle-check", include_str!("../recursos/iconos/circle-check.svg")),
    ("circle-x", include_str!("../recursos/iconos/circle-x.svg")),
    ("clipboard-list", include_str!("../recursos/iconos/clipboard-list.svg")),
    ("clock", include_str!("../recursos/iconos/clock.svg")),
    ("cloud", include_str!("../recursos/iconos/cloud.svg")),
    ("code", include_str!("../recursos/iconos/code.svg")),
    ("coffee", include_str!("../recursos/iconos/coffee.svg")),
    ("compass", include_str!("../recursos/iconos/compass.svg")),
    ("copy", include_str!("../recursos/iconos/copy.svg")),
    ("cpu", include_str!("../recursos/iconos/cpu.svg")),
    ("credit-card", include_str!("../recursos/iconos/credit-card.svg")),
    ("croissant", include_str!("../recursos/iconos/croissant.svg")),
    ("database", include_str!("../recursos/iconos/database.svg")),
    ("dog", include_str!("../recursos/iconos/dog.svg")),
    ("download", include_str!("../recursos/iconos/download.svg")),
    ("dumbbell", include_str!("../recursos/iconos/dumbbell.svg")),
    ("external-link", include_str!("../recursos/iconos/external-link.svg")),
    ("eye", include_str!("../recursos/iconos/eye.svg")),
    ("eye-off", include_str!("../recursos/iconos/eye-off.svg")),
    ("file-text", include_str!("../recursos/iconos/file-text.svg")),
    ("filter", include_str!("../recursos/iconos/filter.svg")),
    ("fish", include_str!("../recursos/iconos/fish.svg")),
    ("flower", include_str!("../recursos/iconos/flower.svg")),
    ("folder", include_str!("../recursos/iconos/folder.svg")),
    ("gavel", include_str!("../recursos/iconos/gavel.svg")),
    ("gift", include_str!("../recursos/iconos/gift.svg")),
    ("globe", include_str!("../recursos/iconos/globe.svg")),
    ("graduation-cap", include_str!("../recursos/iconos/graduation-cap.svg")),
    ("hammer", include_str!("../recursos/iconos/hammer.svg")),
    ("handshake", include_str!("../recursos/iconos/handshake.svg")),
    ("headphones", include_str!("../recursos/iconos/headphones.svg")),
    ("heart", include_str!("../recursos/iconos/heart.svg")),
    ("heart-pulse", include_str!("../recursos/iconos/heart-pulse.svg")),
    ("hotel", include_str!("../recursos/iconos/hotel.svg")),
    ("house", include_str!("../recursos/iconos/house.svg")),
    ("image", include_str!("../recursos/iconos/image.svg")),
    ("info", include_str!("../recursos/iconos/info.svg")),
    ("landmark", include_str!("../recursos/iconos/landmark.svg")),
    ("languages", include_str!("../recursos/iconos/languages.svg")),
    ("layers", include_str!("../recursos/iconos/layers.svg")),
    ("layout-grid", include_str!("../recursos/iconos/layout-grid.svg")),
    ("leaf", include_str!("../recursos/iconos/leaf.svg")),
    ("loader-circle", include_str!("../recursos/iconos/loader-circle.svg")),
    ("lock", include_str!("../recursos/iconos/lock.svg")),
    ("log-in", include_str!("../recursos/iconos/log-in.svg")),
    ("log-out", include_str!("../recursos/iconos/log-out.svg")),
    ("mail", include_str!("../recursos/iconos/mail.svg")),
    ("map", include_str!("../recursos/iconos/map.svg")),
    ("map-pin", include_str!("../recursos/iconos/map-pin.svg")),
    ("menu", include_str!("../recursos/iconos/menu.svg")),
    ("message-circle", include_str!("../recursos/iconos/message-circle.svg")),
    ("mic", include_str!("../recursos/iconos/mic.svg")),
    ("minus", include_str!("../recursos/iconos/minus.svg")),
    ("monitor", include_str!("../recursos/iconos/monitor.svg")),
    ("moon", include_str!("../recursos/iconos/moon.svg")),
    ("music", include_str!("../recursos/iconos/music.svg")),
    ("package", include_str!("../recursos/iconos/package.svg")),
    ("palette", include_str!("../recursos/iconos/palette.svg")),
    ("pause", include_str!("../recursos/iconos/pause.svg")),
    ("paw-print", include_str!("../recursos/iconos/paw-print.svg")),
    ("pen-tool", include_str!("../recursos/iconos/pen-tool.svg")),
    ("pencil", include_str!("../recursos/iconos/pencil.svg")),
    ("percent", include_str!("../recursos/iconos/percent.svg")),
    ("phone", include_str!("../recursos/iconos/phone.svg")),
    ("piggy-bank", include_str!("../recursos/iconos/piggy-bank.svg")),
    ("pill", include_str!("../recursos/iconos/pill.svg")),
    ("plane", include_str!("../recursos/iconos/plane.svg")),
    ("play", include_str!("../recursos/iconos/play.svg")),
    ("plus", include_str!("../recursos/iconos/plus.svg")),
    ("quote", include_str!("../recursos/iconos/quote.svg")),
    ("receipt", include_str!("../recursos/iconos/receipt.svg")),
    ("rocket", include_str!("../recursos/iconos/rocket.svg")),
    ("scale", include_str!("../recursos/iconos/scale.svg")),
    ("scissors", include_str!("../recursos/iconos/scissors.svg")),
    ("search", include_str!("../recursos/iconos/search.svg")),
    ("send", include_str!("../recursos/iconos/send.svg")),
    ("server", include_str!("../recursos/iconos/server.svg")),
    ("settings", include_str!("../recursos/iconos/settings.svg")),
    ("shield-check", include_str!("../recursos/iconos/shield-check.svg")),
    ("shirt", include_str!("../recursos/iconos/shirt.svg")),
    ("shopping-cart", include_str!("../recursos/iconos/shopping-cart.svg")),
    ("smile", include_str!("../recursos/iconos/smile.svg")),
    ("sliders-horizontal", include_str!("../recursos/iconos/sliders-horizontal.svg")),
    ("smartphone", include_str!("../recursos/iconos/smartphone.svg")),
    ("sparkles", include_str!("../recursos/iconos/sparkles.svg")),
    ("sprout", include_str!("../recursos/iconos/sprout.svg")),
    ("star", include_str!("../recursos/iconos/star.svg")),
    ("stethoscope", include_str!("../recursos/iconos/stethoscope.svg")),
    ("store", include_str!("../recursos/iconos/store.svg")),
    ("sun", include_str!("../recursos/iconos/sun.svg")),
    ("tag", include_str!("../recursos/iconos/tag.svg")),
    ("target", include_str!("../recursos/iconos/target.svg")),
    ("terminal", include_str!("../recursos/iconos/terminal.svg")),
    ("ticket", include_str!("../recursos/iconos/ticket.svg")),
    ("trash-2", include_str!("../recursos/iconos/trash-2.svg")),
    ("tree-pine", include_str!("../recursos/iconos/tree-pine.svg")),
    ("trending-up", include_str!("../recursos/iconos/trending-up.svg")),
    ("triangle-alert", include_str!("../recursos/iconos/triangle-alert.svg")),
    ("trophy", include_str!("../recursos/iconos/trophy.svg")),
    ("truck", include_str!("../recursos/iconos/truck.svg")),
    ("upload", include_str!("../recursos/iconos/upload.svg")),
    ("user", include_str!("../recursos/iconos/user.svg")),
    ("users", include_str!("../recursos/iconos/users.svg")),
    ("utensils-crossed", include_str!("../recursos/iconos/utensils-crossed.svg")),
    ("video", include_str!("../recursos/iconos/video.svg")),
    ("wallet", include_str!("../recursos/iconos/wallet.svg")),
    ("waves", include_str!("../recursos/iconos/waves.svg")),
    ("wifi", include_str!("../recursos/iconos/wifi.svg")),
    ("wrench", include_str!("../recursos/iconos/wrench.svg")),
    ("x", include_str!("../recursos/iconos/x.svg")),
    ("zap", include_str!("../recursos/iconos/zap.svg")),
];

/// Alias en español (sin tildes) → icono. Un modelo que piensa en español escribe «rayo»,
/// no «zap»: el programa se adapta al modelo, no al revés.
pub static ALIAS: &[(&str, &str)] = &[
    ("ahorro", "piggy-bank"),
    ("arbol", "tree-pine"),
    ("arte", "palette"),
    ("auto", "car"),
    ("avion", "plane"),
    ("balanza", "scale"),
    ("banco", "landmark"),
    ("bebe", "baby"),
    ("bicicleta", "bike"),
    ("boleto", "ticket"),
    ("brujula", "compass"),
    ("cafe", "coffee"),
    ("calculadora", "calculator"),
    ("cama", "bed"),
    ("camara", "camera"),
    ("camisa", "shirt"),
    ("carro", "car"),
    ("charla", "mic"),
    ("comida", "utensils-crossed"),
    ("conferencia", "mic"),
    ("crecimiento", "chart-line"),
    ("cubiertos", "utensils-crossed"),
    ("dental", "smile"),
    ("diente", "smile"),
    ("dinero", "banknote"),
    ("diseno", "pen-tool"),
    ("doctor", "stethoscope"),
    ("donacion", "heart"),
    ("entrada", "ticket"),
    ("factura", "receipt"),
    ("flor", "flower"),
    ("fotografia", "camera"),
    ("gimnasio", "dumbbell"),
    ("herramienta", "wrench"),
    ("hotel", "hotel"),
    ("idioma", "languages"),
    ("idiomas", "languages"),
    ("justicia", "scale"),
    ("ley", "gavel"),
    ("lista", "clipboard-list"),
    ("mancuerna", "dumbbell"),
    ("mar", "waves"),
    ("martillo", "hammer"),
    ("mascota", "paw-print"),
    ("mazo", "gavel"),
    ("medico", "stethoscope"),
    ("microfono", "mic"),
    ("musica", "music"),
    ("odontologia", "smile"),
    ("olas", "waves"),
    ("paleta", "palette"),
    ("pan", "croissant"),
    ("panaderia", "croissant"),
    ("pastilla", "pill"),
    ("perro", "dog"),
    ("pesas", "dumbbell"),
    ("pez", "fish"),
    ("pincel", "brush"),
    ("planta", "sprout"),
    ("playa", "waves"),
    ("porcentaje", "percent"),
    ("recibo", "receipt"),
    ("restaurante", "utensils-crossed"),
    ("ropa", "shirt"),
    ("ruta", "map"),
    ("salud", "heart-pulse"),
    ("sonrisa", "smile"),
    ("taller", "wrench"),
    ("taza", "coffee"),
    ("tienda", "store"),
    ("tijeras", "scissors"),
    ("trofeo", "trophy"),
    ("verificado", "badge-check"),
    ("viaje", "plane"),
    ("video", "video"),
    ("voluntariado", "handshake"),
    ("acuerdo", "handshake"),
    ("ajustar", "sliders-horizontal"),
    ("ajustes", "settings"),
    ("alerta", "triangle-alert"),
    ("archivo", "file-text"), // juez-idioma: datos (alias de un icono)
    ("auriculares", "headphones"),
    ("aviso", "triangle-alert"),
    ("barras", "chart-column"),
    ("base-de-datos", "database"),
    ("billetera", "wallet"),
    ("birrete", "graduation-cap"),
    ("borrar", "trash-2"),
    ("buscar", "search"),
    ("caja", "package"),
    ("calendario", "calendar"),
    ("camion", "truck"),
    ("campana", "bell"),
    ("candado", "lock"),
    ("capas", "layers"),
    ("cargando", "loader-circle"),
    ("carpeta", "folder"),
    ("carrito", "shopping-cart"),
    ("cartera", "wallet"),
    ("casa", "house"),
    ("celular", "smartphone"),
    ("cerrar", "x"),
    ("chat", "message-circle"),
    ("chispas", "sparkles"),
    ("cita", "quote"),
    ("codigo", "code"),
    ("cohete", "rocket"),
    ("comillas", "quote"),
    ("compra", "shopping-cart"),
    ("comprobar", "check"),
    ("computador", "monitor"),
    ("configuracion", "settings"),
    ("consola", "terminal"),
    ("copiar", "copy"),
    ("corazon", "heart"),
    ("correo", "mail"),
    ("cuadricula", "layout-grid"),
    ("datos", "database"),
    ("descargar", "download"),
    ("diana", "target"),
    ("documento", "file-text"),
    ("ecologia", "leaf"),
    ("edificio", "building-2"),
    ("editar", "pencil"),
    ("educacion", "graduation-cap"),
    ("email", "mail"),
    ("empresa", "building-2"),
    ("energia", "zap"),
    ("enlace-externo", "external-link"),
    ("entrar", "log-in"),
    ("enviar", "send"),
    ("envio", "truck"),
    ("equipo", "users"),
    ("equis", "x"),
    ("error", "circle-x"),
    ("escudo", "shield-check"),
    ("estrella", "star"),
    ("etiqueta", "tag"),
    ("exito", "circle-check"),
    ("filtro", "filter"),
    ("flecha", "arrow-right"),
    ("flecha-abajo", "arrow-down"),
    ("flecha-derecha", "arrow-right"),
    ("flecha-izquierda", "arrow-left"),
    ("foto", "image"),
    ("globo", "globe"),
    ("grafica", "chart-column"),
    ("grafico", "chart-column"),
    ("hoja", "leaf"),
    ("imagen", "image"),
    ("informacion", "info"),
    ("inicio", "house"),
    ("lapiz", "pencil"),
    ("libro", "book-open"),
    ("luna", "moon"),
    ("lupa", "search"),
    ("magia", "sparkles"),
    ("maletin", "briefcase"),
    ("mano", "handshake"),
    ("mapa", "map-pin"),
    ("mas", "plus"),
    ("medalla", "award"),
    ("menos", "minus"),
    ("mensaje", "message-circle"),
    ("menu", "menu"),
    ("movil", "smartphone"),
    ("mundo", "globe"),
    ("notificacion", "bell"),
    ("nube", "cloud"),
    ("objetivo", "target"),
    ("ocultar", "eye-off"),
    ("ojo", "eye"),
    ("pago", "credit-card"),
    ("pantalla", "monitor"),
    ("papelera", "trash-2"),
    ("paquete", "package"),
    ("pastel", "chart-pie"),
    ("pausa", "pause"),
    ("persona", "user"),
    ("personas", "users"),
    ("precio", "tag"),
    ("premio", "award"),
    ("procesador", "cpu"),
    ("rayo", "zap"),
    ("regalo", "gift"),
    ("reloj", "clock"),
    ("reproducir", "play"),
    ("salir", "log-out"),
    ("seguridad", "shield-check"),
    ("servidor", "server"),
    ("sol", "sun"),
    ("soporte", "headphones"),
    ("subir", "upload"),
    ("tarjeta", "credit-card"),
    ("telefono", "phone"),
    ("telefono-movil", "smartphone"),
    ("tendencia", "trending-up"),
    ("torta", "chart-pie"),
    ("trabajo", "briefcase"),
    ("ubicacion", "map-pin"),
    ("usuario", "user"),
    ("usuarios", "users"),
    ("ver", "eye"),
    ("visto", "check"),
    ("wifi", "wifi"),
];

/// Emojis → icono (los modelos pequeños escriben «☕» donde va un icono).
pub static EMOJIS: &[(&str, &str)] = &[
    ("☕", "coffee"),
    ("🔥", "zap"),
    ("⚡", "zap"),
    ("🚀", "rocket"),
    ("🛡️", "shield-check"),
    ("🛡", "shield-check"),
    ("🔒", "lock"),
    ("⭐", "star"),
    ("❤️", "heart"),
    ("❤", "heart"),
    ("💪", "dumbbell"),
    ("🏋️", "dumbbell"),
    ("🏋", "dumbbell"),
    ("🦷", "smile"),
    ("😁", "smile"),
    ("😊", "smile"),
    ("📷", "camera"),
    ("📸", "camera"),
    ("💍", "heart"),
    ("⚖️", "scale"),
    ("⚖", "scale"),
    ("📞", "phone"),
    ("📱", "smartphone"),
    ("💻", "monitor"),
    ("📧", "mail"),
    ("✉️", "mail"),
    ("📍", "map-pin"),
    ("🗺️", "map"),
    ("🕒", "clock"),
    ("⏰", "clock"),
    ("📅", "calendar"),
    ("🗓️", "calendar"),
    ("👥", "users"),
    ("👤", "user"),
    ("🌍", "globe"),
    ("🌎", "globe"),
    ("🌱", "sprout"),
    ("🌿", "leaf"),
    ("🌊", "waves"),
    ("🐟", "fish"),
    ("🏖️", "waves"),
    ("♻️", "leaf"),
    ("🎁", "gift"),
    ("🛒", "shopping-cart"),
    ("💳", "credit-card"),
    ("💰", "banknote"),
    ("📈", "chart-line"),
    ("📊", "chart-column"),
    ("🏆", "trophy"),
    ("🎓", "graduation-cap"),
    ("📚", "book-open"),
    ("🎤", "mic"),
    ("🎵", "music"),
    ("🎨", "palette"),
    ("👕", "shirt"),
    ("🧵", "scissors"),
    ("🚚", "truck"),
    ("📦", "package"),
    ("🏠", "house"),
    ("🏢", "building-2"),
    ("🔧", "wrench"),
    ("🛠️", "wrench"),
    ("💡", "sparkles"),
    ("✨", "sparkles"),
    ("🎯", "target"),
    ("🤝", "handshake"),
    ("🍞", "croissant"),
    ("🥐", "croissant"),
    ("🥑", "utensils-crossed"),
    ("🍳", "utensils-crossed"),
    ("🍽️", "utensils-crossed"),
    ("🥤", "coffee"),
    ("🏔️", "leaf"),
    ("🏔", "leaf"),
    ("✅", "circle-check"),
    ("🔍", "search"),
    ("🎫", "ticket"),
    ("🗣️", "languages"),
    ("🌐", "globe"),
    ("🧘", "heart-pulse"),
    ("🩺", "stethoscope"),
    ("💊", "pill"),
    ("🐶", "dog"),
    ("🐾", "paw-print"),
    ("🚗", "car"),
    ("✈️", "plane"),
    ("🏨", "hotel"),
];

pub static LICENCIA_ICONOS: &str = include_str!("../recursos/iconos/LICENSE");

pub fn fuente(slug: &str) -> Option<&'static Fuente> {
    FUENTES.iter().find(|f| f.slug == slug)
}

/// `(titular, texto)` de un par; `None` si el par no existe.
pub fn par(id: &str) -> Option<(&'static Fuente, &'static Fuente)> {
    let (_, t, x) = PARES.iter().find(|p| p.0 == id)?;
    Some((fuente(t)?, fuente(x)?))
}

/// Pila CSS con respaldo del sistema adecuado a la clase.
pub fn pila(f: &Fuente) -> String {
    let respaldo = match f.clase {
        Clase::Sans => "ui-sans-serif, system-ui, -apple-system, \"Segoe UI\", Roboto, sans-serif",
        Clase::Serif => "\"Iowan Old Style\", \"Palatino Linotype\", Georgia, serif",
        Clase::Mono => "ui-monospace, \"SF Mono\", Menlo, Consolas, monospace",
    };
    format!("\"{}\", {respaldo}", f.familia)
}

/// `@font-face` de una fuente servida en `url` (relativa a la página).
pub fn font_face(f: &Fuente, url: &str) -> String {
    format!(
        "@font-face {{ font-family: \"{}\"; src: url(\"{url}\") format(\"woff2\"); font-weight: {} {}; font-style: normal; font-display: swap; unicode-range: U+0000-00FF, U+0131, U+0152-0153, U+02BB-02BC, U+02C6, U+02DA, U+02DC, U+0304, U+0308, U+0329, U+2000-206F, U+20AC, U+2122, U+2191, U+2193, U+2212, U+2215, U+FEFF, U+FFFD; }}\n",
        f.familia, f.peso.0, f.peso.1
    )
}

/// Nombre real del icono: exacto, o por su alias en español (sin importar tildes,
/// mayúsculas ni espacios). `None` si no se reconoce.
pub fn resolver(nombre: &str) -> Option<&'static str> {
    let n: String = nombre
        .trim()
        .to_lowercase()
        .chars()
        .map(|c| match c {
            'á' => 'a',
            'é' => 'e',
            'í' => 'i',
            'ó' => 'o',
            'ú' | 'ü' => 'u',
            'ñ' => 'n',
            ' ' | '_' => '-',
            c => c,
        })
        .collect();
    let sin_variante = nombre.trim().trim_end_matches('\u{fe0f}');
    if let Some((_, x)) = EMOJIS.iter().find(|(e, _)| e.trim_end_matches('\u{fe0f}') == sin_variante) {
        return Some(x);
    }
    ICONOS.iter().map(|(x, _)| *x).find(|x| *x == n).or_else(|| ALIAS.iter().find(|(a, _)| *a == n).map(|(_, x)| *x))
}

/// El SVG de un icono, decorativo por defecto (`aria-hidden`), con `class` opcional.
/// Acepta el nombre real o su alias en español.
pub fn icono(nombre: &str, class: Option<&str>) -> Option<String> {
    let real = resolver(nombre)?;
    let svg = ICONOS.iter().find(|(n, _)| *n == real)?.1;
    let svg = svg.trim_start_matches(|c: char| c != '<');
    let svg = &svg[svg.find("<svg")?..];
    let extra = match class {
        Some(c) => format!("<svg aria-hidden=\"true\" focusable=\"false\" class=\"{c}\""),
        None => "<svg aria-hidden=\"true\" focusable=\"false\"".to_string(),
    };
    Some(svg.replacen("<svg", &extra, 1).replace("class=\"lucide", "data-icono=\"lucide").trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fuentes_completas_y_con_licencia_ofl() {
        assert_eq!(FUENTES.len(), 13);
        for f in FUENTES {
            assert_eq!(&f.woff2[..4], b"wOF2", "{}", f.slug);
            assert!(f.licencia.contains("SIL Open Font License"), "{}", f.slug);
        }
        for (id, ..) in PARES {
            assert!(par(id).is_some(), "par roto: {id}");
        }
        assert!(fuente(MONO).is_some());
    }

    #[test]
    fn iconos_limpios_y_decorativos() {
        assert!(ICONOS.len() >= 130);
        assert_eq!(resolver("☕"), Some("coffee"));
        assert_eq!(resolver("🏋️"), Some("dumbbell"));
        for (e, x) in EMOJIS {
            assert!(ICONOS.iter().any(|(n, _)| n == x), "emoji {e} apunta a «{x}», que no existe");
        }
        assert!(LICENCIA_ICONOS.contains("ISC"));
        for (n, s) in ICONOS {
            assert!(!s.contains("<script") && !s.contains("href"), "{n} trae algo ejecutable");
        }
        let svg = icono("arrow-right", Some("i")).unwrap();
        assert!(svg.starts_with("<svg aria-hidden=\"true\" focusable=\"false\" class=\"i\""), "{svg}");
        assert!(icono("no-existe", None).is_none());
        assert_eq!(resolver("Rayo"), Some("zap"));
        assert_eq!(resolver("teléfono móvil"), Some("smartphone"));
        assert_eq!(resolver("zap"), Some("zap"));
        for (a, x) in ALIAS {
            assert!(ICONOS.iter().any(|(n, _)| n == x), "alias «{a}» apunta a «{x}», que no existe");
        }
    }
}
