//! Lo que el chat MUESTRA de un mensaje tuyo guardado en la conversación. El modelo recibe más que
//! tus palabras (la skill que elegiste entera, la nota del perfil, las skills sugeridas, la del
//! lenguaje y la del diseño, tus procesos vivos, los adjuntos de texto, la instrucción de voz); al
//! reabrir el proyecto, al rebobinar o al copiar, sólo cuenta lo que escribiste.
//!
//! Desde 2026-11-10 lo escrito se guarda aparte (`Message::visible`). Las sesiones anteriores no lo
//! tienen: para ellas, [`tu_mensaje`] lo recupera quitando la instrucción de voz del principio y
//! cortando en el primer anexo conocido.

use gotv_engine::Message;

/// La instrucción con la que el modo manos libres abre la conversación por voz (va delante de tu
/// primera frase dicha). Una sola vez por sesión de manos libres.
pub fn instruccion_manos_libres() -> &'static str {
    crate::i18n::t(
        "[Desde aquí te hablo POR VOZ: lo que escribas se lee en voz alta. Responde breve y hablado, en prosa, sin markdown ni listas salvo que te lo pida; si usas herramientas, cuéntalo en una frase.]",
        "[From here on I talk to you BY VOICE: what you write is read aloud. Answer briefly and conversationally, in prose, no markdown or lists unless asked; if you use tools, say what you did in one sentence.]",
    )
}

/// Tu mensaje como lo escribiste: lo guardado aparte si existe; si no (sesiones viejas), el
/// contenido sin anexos.
pub fn visible(m: &Message) -> &str {
    tu_mensaje(m.visible.as_deref().unwrap_or(&m.content))
}

/// ¿Es la «memoria de trabajo» que deja una compactación? Va como mensaje de usuario para el modelo, pero no la
/// escribió el Portador: ni se pinta como suya al reabrir el chat ni se archiva.
pub fn es_memoria_de_trabajo(m: &Message) -> bool {
    // juez-idioma: datos (cómo empieza la memoria de trabajo, en los dos idiomas)
    m.role == "user" && (m.content.starts_with("(Memoria de trabajo") || m.content.starts_with("(Working memory"))
}

/// Cómo empieza cada anexo que GOTV pega DETRÁS de tu orden (siempre tras una línea en blanco), en
/// los dos idiomas: skills sugeridas · skill elegida · perfiles · lenguaje pedido · diseño del
/// proyecto · procesos vivos · adjuntos de texto, imagen sin visión y audio transcrito.
const ANEXOS: &[&str] = &[
    // juez-idioma: bloque — datos (cómo empiezan los anexos, en los dos idiomas)
    "(Skills",
    "[skill «",
    "[Perfil ",
    "[GOTV WEB DESIGN profile]",
    "[GOTV PROGRAMMING profile",
    "[Lenguaje pedido",
    "[Requested language",
    "[El usuario pide REDISEÑAR",
    "[The user asks to REDESIGN",
    "[Este proyecto YA tiene su diseño",
    "[This project ALREADY has its design",
    "Procesos tuyos que siguen vivos",
    "Your processes still running",
    "[adjunto: ",
    "[attachment: ",
    "[imagen adjunta: ",
    "[attached image: ",
    "[audio ",
];

/// Un texto guardado sin `visible`: sin la instrucción de voz delante (reconocida por su FORMA, un
/// primer párrafo entre corchetes que habla de la VOZ, para que valgan las versiones viejas y los dos
/// idiomas) y cortado en el primer anexo.
pub fn tu_mensaje(contenido: &str) -> &str {
    let mut t = contenido.trim();
    if t.starts_with('[') {
        // el primer párrafo, o el mensaje entero si la instrucción iba sola (`gotv conversar`)
        let fin = t.find("\n\n").unwrap_or(t.len());
        let cabeza = &t[..fin];
        if cabeza.ends_with(']') && (cabeza.contains("POR VOZ") || cabeza.contains("BY VOICE") || cabeza.contains("VOICE conversation")) {
            t = t[fin..].trim_start();
        }
    }
    let corte = ANEXOS.iter().filter_map(|a| t.find(&format!("\n\n{a}"))).min().unwrap_or(t.len());
    t[..corte].trim()
}

/// El aviso con el que viajan al modelo los medios que pidió con `view_image`/`listen_audio` (un
/// mensaje `user` con las imágenes o el audio). Es INTERNO: el chat no lo muestra al reabrir.
pub fn adjuntos_pedidos(n: usize) -> String {
    crate::tr!(
        "(los {n} adjunto(s) que pediste con view_image/listen_audio)",
        "(the {n} attachment(s) you requested with view_image/listen_audio)"
    )
}

/// ¿Es el aviso interno de [`adjuntos_pedidos`]? En los dos idiomas y con cualquier número, para
/// que valga también en las sesiones ya guardadas (medido 2026-11-12: al reabrir, el chat mostraba
/// «(los 1 adjunto(s) que pediste…)» como si lo hubieras escrito tú).
pub fn es_adjuntos_pedidos(m: &Message) -> bool {
    let t = m.content.trim();
    let cuerpo = t.strip_prefix("(los ").or_else(|| t.strip_prefix("(the ")).unwrap_or(""); // juez-idioma: datos (cómo empieza el anexo)
    m.role == "user"
        && cuerpo.starts_with(|c: char| c.is_ascii_digit())
        && (cuerpo.ends_with(" adjunto(s) que pediste con view_image/listen_audio)") // juez-idioma: datos (cómo empieza el anexo)
            || cuerpo.ends_with(" attachment(s) you requested with view_image/listen_audio)"))
}

/// LA tarjeta de lo que generó `herramienta` (imagen, música o efecto) a partir del texto de su
/// resultado: la usan el evento EN VIVO y el reabrir (`vista_de_imagen`/`vista_de_audio`), así que
/// no pueden divergir (falla #2610, reincidida con el audio). Lleva el archivo (`html` de la imagen
/// o `audio` como URL `data:`) y su `info` (ruta, ficha de licencia, detalle) para el botón ⓘ: esas
/// frases ya no van en la conversación. Sin archivo (borrado o movido), nada; una ruta relativa se
/// lee desde `root`.
pub fn tarjeta(herramienta: &str, salida: &str, titulo: &str, root: &std::path::Path) -> Option<serde_json::Value> {
    let mut info = info_generado(salida)?;
    let ruta = root.join(info["ruta"].as_str()?);
    // absolutas, para mostrarlas y abrirlas tal cual (la herramienta puede escribirlas relativas)
    info["ruta"] = ruta.display().to_string().into();
    if let Some(f) = info["ficha"].as_str().map(|f| root.join(f).display().to_string()) {
        info["ficha"] = f.into();
    }
    // el video NO se lee entero: viaja por ruta (la app lo pide al verlo), no como URL `data:`
    let b = if herramienta == "generar_video" { ruta.is_file().then(Vec::new)? } else { std::fs::read(&ruta).ok()? };
    let titulo = Some(titulo.trim()).filter(|t| !t.is_empty()).map(String::from).unwrap_or_else(|| ruta.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
    let mut v = serde_json::json!({ "titulo": titulo.chars().take(60).collect::<String>(), "ruta": ruta, "info": info });
    match herramienta {
        "generar_imagen" => {
            let (w, h) = medidas_png(&b)?;
            v["tipo"] = "imagen".into();
            v["imagen"] = crate::agent::png_data_url(&b).into();
            v["ancho"] = w.into();
            v["alto"] = h.into();
        }
        "generar_musica" | "generar_efecto" | "separar_pistas" => {
            v["tipo"] = "audio".into();
            v["audio"] = crate::agent::wav_data_url(&b, segundos_wav(&b)?)?.into();
            v["segundos"] = segundos_wav(&b)?.into();
        }
        "generar_video" => {
            // «(2.0 s, 1280×704, 24 cps, …)»: la duración y el tamaño, para reservar su hueco
            let det = info["detalle"].as_str().unwrap_or("").to_string();
            let mut partes = det.split(", ");
            let segundos: f64 = partes.next().and_then(|s| s.trim_end_matches(" s").parse().ok()).unwrap_or(0.0);
            let (ancho, alto) = partes.next().and_then(|t| t.split_once('×')).and_then(|(a, b)| Some((a.parse::<u32>().ok()?, b.parse::<u32>().ok()?))).unwrap_or((1280, 704));
            v["tipo"] = "video".into();
            v["segundos"] = segundos.into();
            v["ancho"] = ancho.into();
            v["alto"] = alto.into();
        }
        _ => return None,
    }
    Some(v)
}

/// Lo que va en el botón ⓘ, leído del resultado de la herramienta (en los dos idiomas): la `ruta`
/// del archivo, la `ficha` de licencia y el `detalle` (duración, tamaño, semilla y lo que eligió el LM).
/// Cuánto tardó una generación, para leerla de un vistazo: «8.4 s», «3 min 05 s», «1 h 12 min».
pub fn duracion(segundos: f32) -> String {
    let s = segundos.max(0.0);
    if s < 60.0 {
        return format!("{s:.1} s");
    }
    let t = s.round() as u64;
    if t < 3600 {
        format!("{} min {:02} s", t / 60, t % 60)
    } else {
        format!("{} h {:02} min", t / 3600, (t % 3600) / 60)
    }
}

pub fn info_generado(salida: &str) -> Option<serde_json::Value> {
    // juez-idioma: bloque — datos: cómo escribe la herramienta su resultado, en los dos idiomas
    let marcas = [
        ("imagen guardada en ", ".png"),
        ("image saved at ", ".png"),
        ("música guardada en ", ".wav"),
        ("music saved at ", ".wav"),
        ("efecto guardado en ", ".wav"),
        ("sound effect saved at ", ".wav"),
        ("pista guardada en ", ".wav"),
        ("track saved at ", ".wav"),
        ("video guardado en ", ".mp4"),
        ("video saved at ", ".mp4"),
    ];
    let (ruta, tras) = marcas.iter().find_map(|(p, ext)| {
        let resto = &salida[salida.find(p)? + p.len()..];
        let fin = resto.find(ext)? + ext.len();
        Some((resto[..fin].to_string(), &resto[fin..]))
    })?;
    // «(10.0 s, estéreo 48 kHz, …)» justo tras la ruta
    let mut detalle = tras.trim_start().strip_prefix('(').and_then(|r| r.find(')').map(|f| r[..f].to_string())).unwrap_or_default();
    let ficha = ["ficha de licencia: ", "license sheet: "].iter().find_map(|p| {
        let resto = &tras[tras.find(p)? + p.len()..];
        Some(resto[..resto.find(".licencia.txt")? + ".licencia.txt".len()].to_string())
    });
    // lo que eligió el LM de música («El LM eligió 90 bpm, …» / «The LM chose …»)
    if let Some(i) = tras.find(" El LM eligió ").or_else(|| tras.find(" The LM chose ")) {
        let plan = tras[i..].trim_start();
        let plan = &plan[..plan.find(". ").map_or(plan.len(), |f| f + 1)];
        detalle = format!("{detalle} · {}", plan.trim_end_matches('.'));
    }
    // el TIEMPO que tardó, aparte (pedido del Portador: que la ⓘ diga cuánto duró generándose):
    // «…, tardó 3 min 05 s» / «…, took 3 min 05 s» al final del detalle
    let mut tiempo = None;
    for p in [", tardó ", ", took "] {
        if let Some(i) = detalle.find(p) {
            let fin = detalle[i + p.len()..].find(" · ").map_or(detalle.len(), |f| i + p.len() + f);
            tiempo = Some(detalle[i + p.len()..fin].to_string());
            detalle.replace_range(i..fin, "");
            break;
        }
    }
    Some(serde_json::json!({ "ruta": ruta, "ficha": ficha, "detalle": detalle, "tiempo": tiempo }))
}

/// La tarjeta del mensaje `i` si es el resultado de `generar_imagen`, para volver a pintarla al
/// reabrir: la MISMA que se vio en vivo (`tarjeta`).
pub fn vista_de_imagen(historial: &[Message], i: usize, root: &std::path::Path) -> Option<serde_json::Value> {
    vista_de(historial, i, root, &["generar_imagen"])
}

/// El reproductor de la música (`generar_musica`) o del efecto (`generar_efecto`) del mensaje `i`,
/// para volver a pintarlo al reabrir. Pasado el tope de la vista (120 s), nada.
pub fn vista_de_audio(historial: &[Message], i: usize, root: &std::path::Path) -> Option<serde_json::Value> {
    vista_de(historial, i, root, &["generar_musica", "generar_efecto", "separar_pistas"])
}

/// El reproductor del video (`generar_video`) del mensaje `i`, para volver a pintarlo al reabrir.
pub fn vista_de_video(historial: &[Message], i: usize, root: &std::path::Path) -> Option<serde_json::Value> {
    vista_de(historial, i, root, &["generar_video"])
}

fn vista_de(historial: &[Message], i: usize, root: &std::path::Path, herramientas: &[&str]) -> Option<serde_json::Value> {
    let m = historial.get(i)?;
    let nombre = m.name.as_deref().filter(|n| m.role == "tool" && herramientas.contains(n))?;
    let titulo = m.tool_call_id.as_deref().and_then(|id| prompt_de_la_llamada(&historial[..i], id)).unwrap_or_default();
    tarjeta(nombre, &m.content, &titulo, root)
}

/// La respuesta SIN las frases de relleno que nombran dónde quedó un archivo generado
/// (`…/generado/…wav|png…`) o su ficha de licencia (`.licencia.txt`): eso vive en el botón ⓘ de la
/// tarjeta, no en la conversación (pedido del Portador: el parlante y los manos libres lo leían).
/// Estrecho a propósito: una ruta que el usuario pidió en un proyecto de código no se toca.
pub fn sin_ubicaciones(texto: &str) -> String {
    let es_relleno = |f: &str| {
        let f = f.to_lowercase();
        f.contains(".licencia.txt")
            || (f.contains("generado/") && [".wav", ".png", ".mp3", ".m4a", ".flac", ".jpg"].iter().any(|e| f.contains(e)))
    };
    let mut out = String::with_capacity(texto.len());
    for (n, linea) in texto.split('\n').enumerate() {
        if n > 0 {
            out.push('\n');
        }
        // frases: hasta un signo final seguido de espacio (o el final de la línea)
        let mut resto = linea;
        let mut quedan = String::new();
        while !resto.is_empty() {
            let fin = resto
                .char_indices()
                .find(|&(i, c)| matches!(c, '.' | '!' | '?') && resto[i + c.len_utf8()..].starts_with(char::is_whitespace))
                .map(|(i, c)| i + c.len_utf8())
                .unwrap_or(resto.len());
            let (frase, sigue) = resto.split_at(fin);
            if !es_relleno(frase) {
                quedan.push_str(frase);
            }
            resto = sigue;
        }
        out.push_str(quedan.trim_end());
    }
    // sin líneas que quedaron vacías por el recorte en medio del texto
    let mut limpio = out.trim().to_string();
    while limpio.contains("\n\n\n") {
        limpio = limpio.replace("\n\n\n", "\n\n");
    }
    limpio
}

/// Duración de un WAV por su cabecera: bytes de datos ÷ (frecuencia × canales × bytes por muestra).
pub fn segundos_wav(b: &[u8]) -> Option<f32> {
    if b.len() < 44 || &b[..4] != b"RIFF" || &b[8..12] != b"WAVE" {
        return None;
    }
    let canales = u16::from_le_bytes([b[22], b[23]]) as f32;
    let frecuencia = u32::from_le_bytes(b[24..28].try_into().ok()?) as f32;
    let bytes = u16::from_le_bytes([b[34], b[35]]) as f32 / 8.0;
    let datos = b.windows(4).position(|w| w == b"data").map(|i| b.len().saturating_sub(i + 8))? as f32;
    (canales > 0.0 && frecuencia > 0.0 && bytes > 0.0).then(|| datos / (frecuencia * canales * bytes))
}

/// Ancho y alto de un PNG por su cabecera (IHDR).
fn medidas_png(b: &[u8]) -> Option<(u32, u32)> {
    if b.len() < 24 || &b[..8] != b"\x89PNG\r\n\x1a\n" || &b[12..16] != b"IHDR" {
        return None;
    }
    Some((u32::from_be_bytes(b[16..20].try_into().ok()?), u32::from_be_bytes(b[20..24].try_into().ok()?)))
}

/// El `prompt` de la llamada `id` (el título de la tarjeta, como en vivo).
fn prompt_de_la_llamada(antes: &[Message], id: &str) -> Option<String> {
    let c = antes.iter().rev().filter_map(|m| m.tool_calls.as_ref()).flatten().find(|c| c["id"].as_str() == Some(id))?;
    let args = match &c["function"]["arguments"] {
        serde_json::Value::String(s) => serde_json::from_str::<serde_json::Value>(s).ok()?,
        v => v.clone(),
    };
    args["prompt"].as_str().map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pedido del Portador: la ⓘ dice cuánto duró la generación, aparte y legible.
    #[test]
    fn el_tiempo_de_generacion_va_aparte_en_la_info() {
        assert_eq!((duracion(8.44), duracion(185.0), duracion(4320.0), duracion(-1.0)), ("8.4 s".into(), "3 min 05 s".into(), "1 h 12 min".into(), "0.0 s".into()));
        let i = info_generado("video guardado en generado/v.mp4 (2.0 s, 1280×704, 24 cps, 50 pasos, guía 5, 900 KB, semilla 3, tardó 47 min 49 s); ficha de licencia: generado/v.mp4.licencia.txt. La persona…").unwrap();
        assert_eq!(i["tiempo"], "47 min 49 s");
        assert_eq!(i["detalle"], "2.0 s, 1280×704, 24 cps, 50 pasos, guía 5, 900 KB, semilla 3", "el tiempo no se repite en el detalle");
        // con el plan del LM de música detrás, el tiempo sale igual
        let m = info_generado("music saved at m.wav (10.0 s, 48 kHz stereo, 1 KB, seed 7, took 15.2 s); license sheet: m.wav.licencia.txt. The LM chose 90 bpm. The person…").unwrap();
        assert_eq!((m["tiempo"].as_str(), m["detalle"].as_str()), (Some("15.2 s"), Some("10.0 s, 48 kHz stereo, 1 KB, seed 7 · The LM chose 90 bpm")));
        // las tarjetas viejas (sin «tardó») no inventan un tiempo
        assert!(info_generado("imagen guardada en a.png (512×512, 3 KB, semilla 1, 8.1 s); ficha de licencia: a.png.licencia.txt.").unwrap()["tiempo"].is_null());
    }

    #[test]
    fn la_instruccion_de_voz_no_se_muestra() {
        // lo que guardó el modo manos libres (la captura del Portador)
        let m = "[Desde aquí te hablo POR VOZ: lo que escribas se lee en voz alta. Responde breve y hablado, en prosa, sin markdown ni listas salvo que te lo pida; si usas herramientas, cuéntalo en una frase.]\n\nTenemos una página web en el proyecto que ya estaba hecha.";
        assert_eq!(tu_mensaje(m), "Tenemos una página web en el proyecto que ya estaba hecha.");
        // en inglés, y con las skills sugeridas detrás
        let m = "[From here on I talk to you BY VOICE: what you write is read aloud.]\n\nmake it alive\n\n(Skills that may help: x)";
        assert_eq!(tu_mensaje(m), "make it alive");
        // la instrucción sola de `gotv conversar` queda vacía (el historial la omite)
        assert_eq!(tu_mensaje("[Conversación POR VOZ con el Portador: lo que digas se lee en voz alta. Para empezar, responde sólo «Listo.»]"), "");
        assert_eq!(tu_mensaje("[VOICE conversation: what you write is read aloud. To begin, answer only «Ready.»]"), "");
        assert_eq!(tu_mensaje(&format!("{}\n\nhola", instruccion_manos_libres())), "hola");
    }

    #[test]
    fn la_skill_el_perfil_y_los_adjuntos_no_se_muestran() {
        // la captura del Portador: la skill elegida entera detrás de su texto
        let m = "al 70% profesional\n\nLa IA: lo mejor de la última década\n\nEn los últimos diez años…\n\n[skill «escritura-humana» elegida por el usuario]\n[skill escritura-humana · carpeta /Users/x/.claude/skills/escritura-humana — sus archivos…]\n\n# Escritura humana en español (0–100 %)";
        assert_eq!(tu_mensaje(m), "al 70% profesional\n\nLa IA: lo mejor de la última década\n\nEn los últimos diez años…");
        // el perfil (con su nota de diseño del proyecto y los procesos vivos detrás)
        let m = "mejora la web\n\n[Perfil DISEÑO WEB de GOTV]\n- Página…\n\n[Este proyecto YA tiene su diseño — adóptalo, no impongas otro]\n\nProcesos tuyos que siguen vivos (manéjalos con `process`):\n- 1";
        assert_eq!(tu_mensaje(m), "mejora la web");
        assert_eq!(tu_mensaje("fix it\n\n[GOTV PROGRAMMING profile — laws]\n1) …"), "fix it");
        // un adjunto de texto
        assert_eq!(tu_mensaje("resume esto\n\n[adjunto: carta.pdf — PDF]\n```\nEstimado…\n```"), "resume esto");
        // la voz Y la skill a la vez: el anexo más temprano manda
        let m = format!("{}\n\n/escritura-humana hola\n\n[skill «x» elegida por el usuario]\ncuerpo", instruccion_manos_libres());
        assert_eq!(tu_mensaje(&m), "/escritura-humana hola");
    }

    #[test]
    fn lo_escrito_aparte_manda() {
        let mut m = Message::user("al 70%\n\n[skill «escritura-humana» elegida por el usuario]\ncuerpo");
        m.visible = Some("/escritura-humana al 70%".into());
        assert_eq!(visible(&m), "/escritura-humana al 70%");
        // sin `visible` (sesión vieja): el respaldo
        m.visible = None;
        assert_eq!(visible(&m), "al 70%");
        // `visible` jamás viaja al motor
        let mut m = Message::user("orden\n\n(Skills: x)");
        m.visible = Some("orden".into());
        let cable = gotv_engine::wire_messages(&[m.clone()]);
        assert!(cable[0].get("visible").is_none(), "{cable}");
        // y sí se guarda en la sesión (JSON del mensaje)
        assert_eq!(serde_json::to_value(&m).unwrap()["visible"], "orden");
    }

    #[test]
    fn el_aviso_de_los_adjuntos_pedidos_es_interno() {
        let m = Message::user_with_media(adjuntos_pedidos(1), vec!["data:image/png;base64,AA==".into()]);
        assert!(es_adjuntos_pedidos(&m));
        // las sesiones ya guardadas, en los dos idiomas y con cualquier número (la captura del Portador)
        for t in ["(los 1 adjunto(s) que pediste con view_image/listen_audio)", "(the 12 attachment(s) you requested with view_image/listen_audio)"] {
            assert!(es_adjuntos_pedidos(&Message::user(t)), "{t}");
        }
        // lo que escribiste tú, aunque se le parezca, sí se muestra
        assert!(!es_adjuntos_pedidos(&Message::user("(los adjuntos que pediste con view_image/listen_audio)")));
        assert!(!es_adjuntos_pedidos(&Message::user("¿qué son los 2 adjunto(s) que pediste con view_image/listen_audio?")));
        assert!(!es_adjuntos_pedidos(&Message::assistant("(los 1 adjunto(s) que pediste con view_image/listen_audio)")));
    }

    #[test]
    fn la_imagen_generada_vuelve_al_reabrir() {
        use gotv_engine::ToolCall;
        let raiz = crate::CarpetaPrueba::nueva("gotv-mensajes-vista");
        // una carpeta con paréntesis en el nombre (la ruta se corta en `.png`, no en « (»)
        let dir = raiz.join("Mi proyecto (copia)").join("generado");
        std::fs::create_dir_all(&dir).unwrap();
        let png = dir.join("imagen-1.png");
        // cabecera PNG real de 3×2 (IHDR): basta para la vista
        let mut b = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
        b.extend(3u32.to_be_bytes());
        b.extend(2u32.to_be_bytes());
        b.extend([8, 6, 0, 0, 0]);
        std::fs::write(&png, &b).unwrap();
        let call = ToolCall { id: "c7".into(), name: "generar_imagen".into(), arguments: serde_json::json!({"prompt": "un zorro tomando café"}) };
        let h = vec![
            Message::user("genera una imagen de un zorro tomando café"),
            Message::assistant_calls("", std::slice::from_ref(&call)),
            Message::tool_result(&call, format!("imagen guardada en {} (3×2, 0 KB, semilla 7, 1.0 s); ficha de licencia: x. Para verla tú: `view_image` con esa ruta.", png.display())),
            Message::user_with_media(adjuntos_pedidos(1), vec![]),
            Message::assistant("Listo."),
        ];
        let v = vista_de_imagen(&h, 2, &raiz).expect("la tarjeta");
        assert_eq!(v["titulo"], "un zorro tomando café");
        assert_eq!(v["tipo"], "imagen");
        assert_eq!(v["ruta"].as_str().unwrap(), png.to_str().unwrap());
        assert!(v["imagen"].as_str().unwrap().starts_with("data:image/png;base64,iVBORw0KGgo") && v.get("html").is_none(), "{v}");
        assert_eq!((v["ancho"].as_u64(), v["alto"].as_u64()), (Some(3), Some(2)));
        // sólo el resultado de `generar_imagen` lleva tarjeta
        assert!((0..h.len()).filter(|&i| i != 2).all(|i| vista_de_imagen(&h, i, &raiz).is_none()));
        // ruta relativa: desde la raíz del proyecto; en inglés también
        let call_en = ToolCall { id: "c8".into(), name: "generar_imagen".into(), arguments: serde_json::json!({"prompt": "a fox"}) };
        let rel = png.strip_prefix(&*raiz).unwrap().display().to_string();
        let h2 = vec![Message::assistant_calls("", std::slice::from_ref(&call_en)), Message::tool_result(&call_en, format!("image saved at {rel} (3×2, 0 KB, seed 1, 1.0 s)"))];
        assert_eq!(vista_de_imagen(&h2, 1, &raiz).unwrap()["titulo"], "a fox");
        // el archivo ya no está: no hay tarjeta (y nada se rompe)
        std::fs::remove_file(&png).unwrap();
        assert!(vista_de_imagen(&h, 2, &raiz).is_none());
        // un error de la herramienta no es una imagen
        let h3 = vec![Message::assistant_calls("", std::slice::from_ref(&call)), Message::tool_result(&call, "ERROR: sin modelo de imagen")];
        assert!(vista_de_imagen(&h3, 1, &raiz).is_none());
    }

    /// El reproductor de la música y del efecto vuelve al reabrir (antes sólo volvía la ruta escrita).
    #[test]
    fn la_musica_y_el_efecto_vuelven_al_reabrir() {
        use gotv_engine::ToolCall;
        let raiz = crate::CarpetaPrueba::nueva("gotv-mensajes-audio");
        let dir = raiz.join("Mi proyecto (copia)").join("generado");
        std::fs::create_dir_all(&dir).unwrap();
        // un WAV flotante de 32 bits como los de ACE-Step/MOSS: 0,5 s estéreo a 48 kHz
        let wav_flotante = |canales: u16, muestras: usize| {
            let datos = (muestras * canales as usize * 4) as u32;
            let mut w = b"RIFF".to_vec();
            w.extend((36 + datos).to_le_bytes());
            w.extend(b"WAVEfmt ");
            w.extend(16u32.to_le_bytes());
            w.extend(3u16.to_le_bytes());
            w.extend(canales.to_le_bytes());
            w.extend(48_000u32.to_le_bytes());
            w.extend((48_000 * canales as u32 * 4).to_le_bytes());
            w.extend((canales * 4).to_le_bytes());
            w.extend(32u16.to_le_bytes());
            w.extend(b"data");
            w.extend(datos.to_le_bytes());
            w.extend(std::iter::repeat_n(0.25f32.to_le_bytes(), muestras * canales as usize).flatten());
            w
        };
        assert_eq!(segundos_wav(&wav_flotante(2, 24_000)), Some(0.5));
        let musica = dir.join("musica-1.wav");
        std::fs::write(&musica, wav_flotante(2, 24_000)).unwrap();
        let call = ToolCall { id: "m1".into(), name: "generar_musica".into(), arguments: serde_json::json!({"prompt": "electrónica con bombo y trompeta"}) };
        let h = vec![
            Message::user("hazme una canción"),
            Message::assistant_calls("", std::slice::from_ref(&call)),
            Message::tool_result(&call, format!("música guardada en {} (0.5 s, estéreo 48 kHz, 187 KB, semilla 7, 3.0 s); ficha de licencia: {}.licencia.txt. El LM eligió 90 bpm, E menor y compás 4/4. La persona YA ve la tarjeta.", musica.display(), musica.display())),
            Message::assistant("Listo."),
        ];
        let v = vista_de_audio(&h, 2, &raiz).expect("el reproductor");
        assert_eq!((v["tipo"].as_str(), v["titulo"].as_str()), (Some("audio"), Some("electrónica con bombo y trompeta")));
        assert!(v["audio"].as_str().unwrap().starts_with("data:audio/wav;base64,UklGR") && v.get("html").is_none(), "{v}");
        assert_eq!((v["segundos"].as_f64(), v["info"]["ruta"].as_str()), (Some(0.5), Some(musica.to_str().unwrap())));
        assert_eq!(v["info"]["ficha"].as_str().unwrap(), format!("{}.licencia.txt", musica.display()));
        assert_eq!(v["info"]["detalle"], "0.5 s, estéreo 48 kHz, 187 KB, semilla 7, 3.0 s · El LM eligió 90 bpm, E menor y compás 4/4");
        assert!((0..h.len()).filter(|&i| i != 2).all(|i| vista_de_audio(&h, i, &raiz).is_none()));
        assert!(vista_de_imagen(&h, 2, &raiz).is_none(), "la música no es una imagen");
        // el efecto, en inglés y con ruta relativa
        let efecto = dir.join("efecto-1.wav");
        std::fs::write(&efecto, wav_flotante(1, 4_800)).unwrap();
        let ce = ToolCall { id: "e1".into(), name: "generar_efecto".into(), arguments: serde_json::json!({"prompt": "door creak"}) };
        let rel = efecto.strip_prefix(&*raiz).unwrap().display().to_string();
        let h2 = vec![Message::assistant_calls("", std::slice::from_ref(&ce)), Message::tool_result(&ce, format!("sound effect saved at {rel} (0.1 s, 48 kHz mono, 18 KB, seed 1, 2.0 s); license sheet: x."))];
        assert_eq!(vista_de_audio(&h2, 1, &raiz).unwrap()["titulo"], "door creak");
        // el archivo ya no está, o la herramienta falló: no hay reproductor (y nada se rompe)
        std::fs::remove_file(&musica).unwrap();
        assert!(vista_de_audio(&h, 2, &raiz).is_none());
        let h3 = vec![Message::assistant_calls("", std::slice::from_ref(&call)), Message::tool_result(&call, "ERROR: sin modelo de música")];
        assert!(vista_de_audio(&h3, 1, &raiz).is_none());
    }

    /// Sonda con una conversación REAL (sin modelos): `GOTV_SONDA_HISTORIAL=<jsonl de mensajes>`
    /// (las sesiones ya no están en `.gotv/gotv.db` sino en `.gotv/sesiones/<id>.msj`, formato propio: se
    /// leen con `session::Session::resume_last`)
    /// y `GOTV_SONDA_RAIZ=<proyecto>`: cuántos reproductores vuelven al reabrir.
    #[test]
    #[ignore]
    fn sonda_reabrir_audio() {
        let (Ok(f), Ok(raiz)) = (std::env::var("GOTV_SONDA_HISTORIAL"), std::env::var("GOTV_SONDA_RAIZ")) else { return };
        let h: Vec<Message> = std::fs::read_to_string(f).unwrap().lines().filter_map(|l| serde_json::from_str(l).ok()).collect();
        let raiz = std::path::Path::new(&raiz);
        let llamadas = h.iter().filter(|m| matches!(m.name.as_deref(), Some("generar_musica" | "generar_efecto"))).count();
        let vistas: Vec<_> = (0..h.len()).filter_map(|i| vista_de_audio(&h, i, raiz)).collect();
        for v in &vistas {
            println!("♪ {} · {} KB · ⓘ {}", v["titulo"], v["audio"].as_str().unwrap().len() / 1024, v["info"]);
        }
        println!("{} mensajes · {llamadas} resultados de música/efecto · {} reproductores al reabrir", h.len(), vistas.len());
        // las respuestas guardadas ANTES del recorte también se ven limpias al reabrir
        let con = |t: &str| t.contains("generado/") || t.contains(".licencia.txt");
        let resp: Vec<&Message> = h.iter().filter(|m| m.role == "assistant" && con(&m.content)).collect();
        println!("respuestas con ubicación: {} → tras el filtro: {}", resp.len(), resp.iter().filter(|m| con(&sin_ubicaciones(&m.content))).count());
        for m in resp.iter().take(2) {
            println!("  «{}»", sin_ubicaciones(&m.content).chars().take(160).collect::<String>());
        }
    }

    /// La respuesta de la captura del Portador (2026-11-12): queda lo que dijo la IA; la ubicación
    /// y la licencia se van (viven en el botón ⓘ de la tarjeta).
    #[test]
    fn la_ubicacion_y_la_licencia_no_son_conversacion() {
        let r = "He generado la canción con voz en español con `generar_musica`. La música está en `/Users/x/Documents/proyectos/php/generado/musica-1794496225612.wav` (3.75 MB) y su licencia en `.licencia.txt`. El LM eligió 90 bpm, tonalidad E menor y compás 4/4. La voz es femenina, etérea y cálida.";
        assert_eq!(
            sin_ubicaciones(r),
            "He generado la canción con voz en español con `generar_musica`. El LM eligió 90 bpm, tonalidad E menor y compás 4/4. La voz es femenina, etérea y cálida."
        );
        // en inglés y con la imagen; varias líneas
        let en = "Done!\n\nThe image is at `generado/imagen-1.png` (1024×1024). Its license sheet: `generado/imagen-1.png.licencia.txt`.\n\nWant another one?";
        assert_eq!(sin_ubicaciones(en), "Done!\n\nWant another one?");
        // una ruta de CÓDIGO que el usuario pidió no se toca; tampoco un texto sin rutas
        for t in ["El archivo está en `src/main.rs` (línea 4).", "Listo: 3 pruebas pasan.", "Guardé el logo en `assets/logo.png`."] {
            assert_eq!(sin_ubicaciones(t), t);
        }
    }

    #[test]
    fn lo_demas_queda_intacto() {
        assert_eq!(tu_mensaje("lee main.rs\n\n(Skills: x)"), "lee main.rs");
        // corchetes tuyos que no hablan de la voz ni son un anexo
        assert_eq!(tu_mensaje("[nota] arregla el menú"), "[nota] arregla el menú");
        assert_eq!(tu_mensaje("[TODO]\n\nrevisa esto"), "[TODO]\n\nrevisa esto");
        // un anexo sólo cuenta tras una línea en blanco, no a mitad de tu frase
        assert_eq!(tu_mensaje("explica qué hace [Perfil X] en el código"), "explica qué hace [Perfil X] en el código");
        assert_eq!(tu_mensaje("hola"), "hola");
    }
}
