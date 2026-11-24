//! Guardia contra los tropiezos de los modelos PEQUEÑOS (Ola D, medidos en D1 con Qwen3.5-9B):
//!
//! - **preguntas en bucle**: 16 `ask_user` en una tarea sin humano. Tras 2 sin respuesta (o 4
//!   en total) la herramienta desaparece en ese turno: decide y sigue.
//! - **vaivén de ediciones**: cambió la misma línea #222→#000→#333→#111 dudando de un contraste
//!   correcto. Reeditar lo que uno mismo acaba de escribir avisa a la 3.ª y se bloquea a la 5.ª.
//! - **argumentos vacíos**: llamó una herramienta MCP con el texto vacío. Los campos obligatorios
//!   del esquema se comprueban ANTES de ejecutar.
//!
//! Sin estado global: una `Guardia` por turno del agente principal.

use std::collections::HashMap;

pub const MAX_SIN_RESPUESTA: u32 = 2;
pub const MAX_PREGUNTAS: u32 = 4;
pub const VAIVEN_AVISO: u32 = 3;
pub const VAIVEN_BLOQUEO: u32 = 5;

/// Cómo se MIDE según lo que se está cambiando. Obra con Huihui (2026-11-16): el vaivén era en un archivo
/// de pruebas y el aviso hablaba de colores, tamaños y del navegador.
fn como_medir(ruta: &str) -> &'static str {
    let r = ruta.to_lowercase();
    let nombre = r.rsplit('/').next().unwrap_or(&r);
    let de_pruebas = r.contains("/tests/") || r.contains("/test/") || r.contains("__tests__") || nombre.contains(".test.") || nombre.contains(".spec.") || nombre.contains("_test.") || nombre.starts_with("test_");
    let de_pantalla = [".css", ".scss", ".html", ".tsx", ".jsx", ".vue", ".svelte", ".astro"].iter().any(|e| nombre.ends_with(e));
    if de_pruebas {
        crate::i18n::t(
            "corre las pruebas e IMPRIME el error completo o el valor real (un `console.log`/`print` dentro de la prueba, o el error que captura tu código), y arregla la PRIMERA causa impresa —casi siempre en el código que prueban o en cómo se importa, no en la prueba.",
            "run the tests and PRINT the full error or the real value (a `console.log`/`print` inside the test, or the error your code catches), and fix the FIRST printed cause —almost always in the code under test or in how it is imported, not in the test.",
        )
    } else if de_pantalla {
        crate::i18n::t(
            "mídelo en el navegador (a11y, eval: el color, el tamaño o el texto real) o con las pruebas.",
            "measure it in the browser (a11y, eval: the real colour, size or text) or with the tests.",
        )
    } else {
        crate::i18n::t(
            "corre las pruebas o un experimento mínimo que IMPRIMA el valor real o el error, y compáralo con lo que debería ser.",
            "run the tests or a minimal experiment that PRINTS the real value or the error, and compare it with what it should be.",
        )
    }
}

/// Por archivo: tramos leídos `[desde, hasta)`, fecha de modificación vista y relecturas.
type Lecturas = (Vec<(u64, u64)>, Option<std::time::SystemTime>, u32);

#[derive(Debug, Default)]
pub struct Guardia {
    preguntas: u32,
    sin_respuesta: u32,
    /// por archivo: los valores que ya tuvo esa parte, cuántas reediciones seguidas y si en la
    /// racha VOLVIÓ a un valor anterior (eso es vaivén; corregir hacia adelante no lo es)
    escrito: HashMap<String, (Vec<String>, u32, bool)>,
    /// D9: la última revisión de entrega con fallos (None = pasó o no hubo página)
    pub entrega: Option<String>,
    /// D9: ya se le recordó una vez en este turno
    pub entrega_avisada: bool,
    /// D11: la orden de este turno pide REDISEÑAR (la página vieja se respalda y se reemplaza)
    pub rediseno: bool,
    /// N2 (2026-11-14): en este turno el trabajo se cierra con la herramienta `terminar`, no con texto
    /// (perfil Programar). Lo fija `run` con el perfil de la orden.
    pub cierre_explicito: bool,
    /// N1 (2026-11-14): la orden de ESTE turno tal como llegó al modelo; la compactación la conserva
    /// literal aunque no sea la primera de la conversación.
    pub orden: Option<String>,
    /// N3 (2026-11-14): tokens que quedaron tras la última compactación de este turno; mientras el
    /// contexto no crezca un tramo desde ahí, no se compacta otra vez (medido: 57 seguidas en 8k).
    pub tras_compactar: Option<usize>,
    /// N3: ya se avisó en este turno de que la ventana no alcanza.
    pub ventana_avisada: bool,
    /// N7: compactaciones de esta orden (a la tercera se avisa de que la ventana es chica).
    pub compactaciones: u32,
    /// N5: llamadas (herramienta + argumentos) que causaron un atasco en esta orden: no se ejecutan más.
    vetadas: std::collections::HashSet<String>,
    /// D10: por archivo, los tramos ya leídos en este turno, la fecha de modificación vista y
    /// cuántas lecturas pidieron algo que YA estaba cubierto
    lecturas: HashMap<String, Lecturas>,
    /// D5 (medido): la última llamada que FALLÓ (herramienta + argumentos), su error y cuántas
    /// veces seguidas falló IGUAL. El 9B repitió `browser click @e43` quince veces.
    fallo: Option<(String, String, u32)>,
    /// D11 (medido): la última llamada (herramienta + argumentos) y cuántas veces SEGUIDAS se pidió
    /// igual, salga bien o mal. El 9B pidió `ls -la ~/.claude/agents/` cinco veces seguidas.
    ultima: Option<(String, u32)>,
    /// las últimas llamadas (para el vaivén A-B-A-B: medido en la Air, el 0.8B alternó
    /// `write_file`/`edit` sobre el mismo archivo 40 pasos y el freno de «idénticas seguidas» no lo veía)
    recientes: std::collections::VecDeque<String>,
    /// llamadas BLOQUEADAS seguidas (repetidas o fallidas iguales): el modelo está atascado
    bloqueos: u32,
    /// D5 (medido): las apps que creó `app_nueva` → (pruebas en verde, build en verde, ya avisado).
    /// El 9B tuvo la app lista en el paso 6 y deambuló hasta el 40: nadie le dijo que terminó.
    apps: HashMap<std::path::PathBuf, (bool, bool, bool)>,
    /// las carpetas de PÁGINAS (D11): una página está lista cuando `pagina` pasó la revisión de entrega
    paginas: std::collections::HashSet<std::path::PathBuf>,
    /// pasos seguidos SIN cambiar nada desde que una app quedó en verde
    ocioso: u32,
    /// O0 (medido 2026-11-11): la última llamada ejecutada, su salida y cuántas veces seguidas dio
    /// EXACTAMENTE lo mismo. El 9B pidió `desktop list-windows --app "Microsoft Word"` 27 veces
    /// («0 ventanas» cada vez) hasta el paso 40: las herramientas de pantalla están exentas de
    /// «misma llamada» (la pantalla cambia sola), pero no de «misma llamada Y misma respuesta».
    /// No sólo SEGUIDAS: la 2.ª corrida giró en ciclo (launch → snapshot ✗ → list-windows), y la
    /// racha seguida nunca pasaba de 1. Se miran las últimas `QUIETO_VENTANA` llamadas ejecutadas.
    quieto: std::collections::VecDeque<(String, String)>,
    /// D14 (prueba Cable v8 y v10, 2026-11-12): la última corrida de pruebas del turno salió en rojo
    pub pruebas_en_rojo: bool,
    /// F5 ML2: los archivos de código que este turno escribió o editó (para la compuerta del cierre)
    pub tocados: Vec<std::path::PathBuf>,
    /// F5 ML2: la compuerta ya corrió en este turno (una vez basta)
    pub compuerta_hecha: bool,
    /// D14: la edición de pruebas que se frenó (huella); si la repite IGUAL, pasa
    prueba_frenada: Option<u64>,
    /// DM6.5: huella de la edición de diseño frenada por tapar un síntoma (si se repite igual, pasa)
    sintoma_frenado: Option<u64>,
}

/// Veces (entre las últimas `QUIETO_VENTANA` ejecutadas) que una herramienta de pantalla puede dar
/// la MISMA respuesta a la MISMA llamada.
pub const QUIETO_MAX: u32 = 3;
pub const QUIETO_VENTANA: usize = 10;

/// Pasos sin cambiar nada, con la app en verde, tras los que GOTV cierra el turno (D5, medido:
/// avisado de que terminó, el 9B siguió 34 pasos con clics y `cat` de archivos que no tocó).
pub const OCIOSO_MAX: u32 = 6;

/// Llamadas bloqueadas seguidas tras las que GOTV cierra el turno (el modelo no sale del bucle).
pub const ATASCO_MAX: u32 = 6;

impl Guardia {
    /// Antes de ejecutar: la MISMA llamada pedida por 3.ª vez seguida no se ejecuta (ya tiene su
    /// salida). No cuenta para lo que cambia solo entre llamadas (procesos, navegador, pantalla).
    pub fn misma_llamada(&mut self, herramienta: &str, clave: &str) -> Option<String> {
        if matches!(herramienta, "process" | "browser" | "chrome" | "desktop" | "screen" | "mouse" | "keyboard" | "todo" | "ask_user") {
            self.ultima = None;
            // …salvo que ya dio EXACTAMENTE la misma respuesta QUIETO_MAX veces entre las últimas
            // QUIETO_VENTANA llamadas (seguidas o en ciclo con otras)
            let mismas: Vec<&String> = self.quieto.iter().filter(|(k, _)| k == clave).map(|(_, s)| s).collect();
            let n = mismas.len() as u32;
            if let Some(ultima) = mismas.last() {
                if n >= QUIETO_MAX && mismas.iter().all(|s| s == ultima) {
                    self.bloqueos += 1;
                    // la respuesta va CON el bloqueo (medido: el 9B tenía el total de Excel y
                    // repitió la lectura 6 veces; sin el dato a la vista, no respondió)
                    let dato: String = ultima.chars().take(400).collect();
                    return Some(crate::tr!(
                        "ERROR: NO la ejecuté: esta llamada ya devolvió EXACTAMENTE lo mismo {n} veces en tus últimas llamadas, y fue:\n«{dato}»\nSi ese era el dato que necesitabas, ÚSALO y responde. Si esperabas que cambiara, no está cambiando solo: haz lo que falta (Word, Excel y PowerPoint arrancan SIN documento: [\"recetas\",\"Microsoft Word\"]), usa otra vía, o explica al usuario qué impide seguir.",
                        "ERROR: NOT run: this call already returned EXACTLY the same {n} times in your last calls, and it was:\n«{dato}»\nIf that was the data you needed, USE it and answer. If you expected it to change, it is not changing by itself: do what is missing (Word, Excel and PowerPoint start WITHOUT a document: [\"recetas\",\"Microsoft Word\"]), use another way, or tell the user what blocks you."
                    ));
                }
            }
            return None;
        }
        // vaivén: …A B A B y ahora A otra vez
        let r = &self.recientes;
        let vaiven = r.len() >= 4 && {
            let l = r.len();
            r[l - 1] != r[l - 2] && r[l - 1] == r[l - 3] && r[l - 2] == r[l - 4] && clave == r[l - 2]
        };
        self.recientes.push_back(clave.to_string());
        if self.recientes.len() > 6 {
            self.recientes.pop_front();
        }
        if vaiven {
            self.bloqueos += 1;
            return Some(crate::i18n::t(
                "ERROR: NO la ejecuté: estás alternando las MISMAS dos llamadas (A, B, A, B…) sin avanzar. El resultado ya está: revisa la salida de arriba y, si lo pedido está hecho, termina; si no, cambia de enfoque.",
                "ERROR: NOT run: you are alternating the SAME two calls (A, B, A, B…) without progress. The result is already there: check the output above and, if the request is done, finish; otherwise change approach.",
            )
            .to_string());
        }
        let n = match &self.ultima {
            Some((k, n)) if k == clave => n + 1,
            _ => 1,
        };
        self.ultima = Some((clave.to_string(), n));
        if n >= 3 {
            self.bloqueos += 1;
        }
        (n >= 3).then(|| {
            crate::tr!(
                "ERROR: NO la ejecuté: es la {n}.ª vez SEGUIDA que pides exactamente lo mismo y la salida ya la tienes arriba. Repetirla no cambia nada: usa lo que ya viste, cambia de enfoque o, si ya está hecho, termina.",
                "ERROR: NOT run: this is the {n}th time IN A ROW you ask for exactly the same thing and you already have its output above. Repeating it changes nothing: use what you saw, change approach or, if it is done, finish."
            )
        })
    }

    /// Tras una reorientación (el agente le dio al modelo el resultado real de las pruebas y un camino
    /// nuevo): las rachas de bloqueos, fallos y repeticiones empiezan de cero. Prueba Cable
    /// (2026-11-12): cerrar el turno al atascarse tiraba 22 de 30 minutos.
    pub fn reorientar(&mut self) {
        self.bloqueos = 0;
        // N5 (medido en el Air, 4B): tras cada reorientación la MISMA llamada volvía a ejecutarse dos
        // veces y a bloquearse seis (`pip install ruff`, 45 pasos). La que causó el atasco queda
        // vetada para el resto de la orden.
        if let Some((k, _, n)) = &self.fallo {
            if *n >= 2 {
                self.vetadas.insert(k.clone());
            }
        }
        self.fallo = None;
        self.ultima = None;
        self.recientes.clear();
    }

    /// Antes de ejecutar: si esta MISMA llamada ya falló igual 2 veces seguidas, no se repite.
    pub fn repite_fallo(&mut self, clave: &str) -> Option<String> {
        if self.vetadas.contains(clave) {
            self.bloqueos += 1;
            return Some(
                crate::i18n::t(
                    "ERROR: NO la ejecuté: esta misma llamada ya falló igual varias veces en esta orden y no se ejecutará más. Si era un paso que nadie pidió (formatear, instalar una herramienta), sáltalo. Si lo pedido ya está hecho y comprobado, cierra. Si de verdad hace falta, usa OTRO comando.",
                    "ERROR: NOT run: this same call already failed the same way several times in this order and will not run again. If it was a step nobody asked for (formatting, installing a tool), skip it. If what was requested is done and verified, finish. If it is truly needed, use ANOTHER command.",
                )
                .to_string(),
            );
        }
        let (k, err, n) = self.fallo.as_ref()?;
        let bloquea = k == clave && *n >= 2;
        let (err, n) = (err.clone(), *n);
        if bloquea {
            self.bloqueos += 1;
        }
        bloquea.then(|| {
            crate::tr!(
                "ERROR: NO la ejecuté: esta misma llamada ya falló {n} veces seguidas con el mismo error ({}). Repetirla no cambia nada. Cambia de enfoque: si es el navegador, toma un `snapshot -i` NUEVO (las @e cambian al cambiar la página) o usa otro selector; si es un comando, lee el error y corrige la causa; si lo que querías ya está comprobado de otra forma (p. ej. con las pruebas), sigue o termina.",
                "ERROR: NOT run: this same call already failed {n} times in a row with the same error ({}). Repeating it changes nothing. Change approach: if it is the browser, take a NEW `snapshot -i` (the @e refs change when the page changes) or use another selector; if it is a command, read the error and fix the cause; if what you wanted is already verified another way (e.g. by the tests), move on or finish.",
                recorta(&err)
            )
        })
    }

    /// Después de ejecutar: lleva la racha de fallos idénticos (un éxito o una llamada distinta la corta).
    /// D11 (medido): bloqueada 9 veces seguidas, el 9B siguió pidiendo lo mismo hasta el paso 40.
    /// Con `ATASCO_MAX` bloqueos seguidos, GOTV cierra el turno.
    pub fn atascado(&self) -> bool {
        self.bloqueos >= ATASCO_MAX
    }

    pub fn resultado(&mut self, clave: &str, ok: bool, salida: &str) {
        self.bloqueos = 0; // algo se ejecutó
        self.quieto.push_back((clave.to_string(), salida.to_string()));
        if self.quieto.len() > QUIETO_VENTANA {
            self.quieto.pop_front();
        }
        if ok {
            self.fallo = None;
            return;
        }
        match &mut self.fallo {
            Some((k, e, n)) if k == clave && e == salida => *n += 1,
            _ => self.fallo = Some((clave.to_string(), salida.to_string(), 1)),
        }
    }
}

impl Guardia {
    pub fn app_creada(&mut self, dir: std::path::PathBuf) {
        self.apps.insert(dir, (false, false, false));
    }

    /// Tras cada herramienta: lleva el estado de las apps. Devuelve el aviso «lista: termina»
    /// la primera vez que pruebas Y build pasan en la carpeta de la app.
    pub fn app_paso(
        &mut self,
        root: &std::path::Path,
        herramienta: &str,
        args: &serde_json::Value,
        ok: bool,
    ) -> Option<String> {
        if self.apps.is_empty() {
            return None;
        }
        if matches!(herramienta, "write_file" | "edit" | "multi_edit") {
            let p = crate::paths::resolve(root, args["path"].as_str().unwrap_or(""));
            let mut reinicio = false;
            for (dir, e) in self.apps.iter_mut() {
                if p.starts_with(dir)
                    && !p
                        .components()
                        .any(|c| c.as_os_str() == "dist" || c.as_os_str() == "node_modules")
                {
                    *e = (false, false, false); // cambió el código: hay que volver a comprobar
                    reinicio = true;
                }
            }
            if reinicio {
                self.ocioso = 0;
            }
            return None;
        }
        if herramienta != "bash" || !ok {
            return None;
        }
        let cmd = args["cmd"].as_str().unwrap_or("");
        // carpeta donde corre: el último `cd X` antes del comando, relativo a la raíz
        let mut dir = root.to_path_buf();
        for parte in cmd.split("&&") {
            let w: Vec<&str> = parte.split_whitespace().collect();
            if w.first() == Some(&"cd") {
                if let Some(d) = w.get(1) {
                    dir = crate::paths::resolve(&dir, d.trim_matches(['"', '\'']));
                }
            }
        }
        let pruebas = [
            "npm test",
            "npm run test",
            "vitest",
            "npx vitest",
            "pnpm test",
            "yarn test",
        ]
        .iter()
        .any(|k| cmd.contains(k));
        let build = [
            "npm run build",
            "vite build",
            "pnpm build",
            "yarn build",
            "pnpm run build",
        ]
        .iter()
        .any(|k| cmd.contains(k));
        // `npm test | grep x` puede salir 0 aunque fallen: sólo cuenta sin tubería
        if (!pruebas && !build) || cmd.contains('|') {
            return None;
        }
        let (app, e) = self
            .apps
            .iter_mut()
            .find(|(d, _)| dir.starts_with(d.as_path()))?;
        e.0 |= pruebas;
        e.1 |= build;
        if e.0 && e.1 && !e.2 {
            e.2 = true;
            return Some(crate::tr!(
                "\n✅ La aplicación de {} quedó LISTA: pruebas y build de producción en verde. Si el pedido no exige nada que falte, TERMINA AHORA con un resumen corto para el usuario (qué hace y cómo abrirla: `npm run dev`). No hace falta releer archivos, ni lint, ni más clics en el navegador.",
                "\n✅ The application in {} is READY: tests and production build are green. If the request does not need anything missing, FINISH NOW with a short summary for the user (what it does and how to open it: `npm run dev`). No need to reread files, lint, or click around in the browser.",
                app.display()
            ));
        }
        None
    }
}

impl Guardia {
    /// Al cerrar cada paso: con una app en verde y nada cambiado, cuenta; al llegar al tope
    /// devuelve la carpeta para que GOTV cierre el turno con el resumen.
    pub fn app_ociosa(&mut self) -> Option<std::path::PathBuf> {
        let lista = self.apps.iter().find(|(_, e)| e.0 && e.1 && e.2).map(|(d, _)| d.clone())?;
        self.ocioso += 1;
        (self.ocioso >= OCIOSO_MAX).then_some(lista)
    }

    /// D11 (medido): la página quedó lista y revisada en el paso 2 y el 9B siguió 38 pasos (copias
    /// fuera, `browser` como comando). Una página que PASA la revisión de entrega cuenta como lista:
    /// si el modelo no la rehace en `OCIOSO_MAX` pasos, GOTV cierra. Editarla la vuelve «no lista».
    pub fn pagina_lista(&mut self, dir: std::path::PathBuf) {
        self.paginas.insert(dir.clone());
        self.apps.insert(dir, (true, true, true));
        self.ocioso = 0;
    }

    pub fn es_pagina(&self, dir: &std::path::Path) -> bool {
        self.paginas.contains(dir)
    }
}

/// Prueba del Portador (2026-11-18): `mkdir -p ~/Escritorio` creó una carpeta NUEVA en vez de usar el escritorio de
/// verdad (`~/Desktop`: Finder lo muestra traducido). Devuelve (lo escrito, la ruta real) si la orden usa una
/// carpeta traducida que no existe en disco.
pub fn carpeta_traducida(cmd: &str) -> Option<(String, String)> {
    let casa = crate::paths::home();
    let casa_txt = casa.display().to_string();
    for (t, real) in crate::paths::CARPETAS_TRADUCIDAS {
        if casa.join(t).exists() || !casa.join(real).is_dir() {
            continue;
        }
        for forma in [format!("~/{t}"), format!("$HOME/{t}"), format!("${{HOME}}/{t}"), format!("{casa_txt}/{t}")] {
            if let Some(i) = cmd.find(&forma) {
                let fin = cmd[i + forma.len()..].chars().next();
                if fin.is_none_or(|c| c == '/' || c.is_whitespace() || c == '"' || c == '\'' || c == ';' || c == '&') {
                    return Some((forma, format!("~/{real}")));
                }
            }
        }
    }
    None
}

/// F2.2 (banco con Huihui, 2026-11-17): «avísame en 2 minutos si…» se resolvió con `sleep 120 && curl …`,
/// que BLOQUEA el turno. Devuelve los segundos de un `sleep` de 60 s o más (lo mínimo de `programar`); los
/// cortos (esperar a que arranque un servidor) no cuentan.
pub fn espera_larga(cmd: &str) -> Option<u64> {
    let w: Vec<&str> = cmd.split(|c: char| c.is_whitespace() || matches!(c, ';' | '&' | '|' | '(' | ')')).filter(|t| !t.is_empty()).collect();
    w.windows(2)
        .filter(|p| p[0] == "sleep")
        .filter_map(|p| {
            let n = p[1];
            let (num, mult) = match n.chars().last()? {
                's' => (&n[..n.len() - 1], 1.0),
                'm' => (&n[..n.len() - 1], 60.0),
                'h' => (&n[..n.len() - 1], 3600.0),
                _ => (n, 1.0),
            };
            num.parse::<f64>().ok().map(|x| (x * mult) as u64)
        })
        .filter(|s| *s >= crate::programar::chat::MIN_SEGUNDOS)
        .max()
}

/// D11 (medido): el 9B corrió `./pagina --ficha ficha.json --path .` en la terminal y luego buscó
/// ese «programa» dentro de ~/.claude. Devuelve la herramienta de GOTV que se usó como comando y,
/// para `pagina`, el archivo de la ficha (resuelto contra el `cd` previo).
pub fn herramienta_como_comando(root: &std::path::Path, cmd: &str) -> Option<(String, Option<std::path::PathBuf>)> {
    const HERRAMIENTAS: &[&str] = &["pagina", "isometrica", "generar_imagen", "generar_musica", "generar_efecto", "separar_pistas", "generar_video", "app_nueva", "api_nueva", "web_search", "web_fetch", "tool_search", "ask_user", "view_image", "rag_search", "browser", "listen_audio", "skill"];
    let mut dir = root.to_path_buf();
    for parte in cmd.split("&&").flat_map(|p| p.split(';')) {
        let w: Vec<&str> = parte.split_whitespace().collect();
        let Some(primera) = w.first() else { continue };
        if *primera == "cd" {
            if let Some(d) = w.get(1) {
                dir = crate::paths::resolve(&dir, d.trim_matches(['"', '\'']));
            }
            continue;
        }
        let (nombre, resto) = match primera.trim_start_matches("./") {
            "gotv" if w.get(1).is_some_and(|x| HERRAMIENTAS.contains(x)) => (w[1], &w[2..]),
            x if HERRAMIENTAS.contains(&x) => (x, &w[1..]),
            _ => continue,
        };
        let ficha = (nombre == "pagina")
            .then(|| {
                resto
                    .iter()
                    .position(|t| *t == "--ficha")
                    .and_then(|i| resto.get(i + 1))
                    .or_else(|| resto.iter().find(|t| t.ends_with(".json")))
                    .map(|f| crate::paths::resolve(&dir, f.trim_matches(['"', '\''])))
            })
            .flatten();
        return Some((nombre.to_string(), ficha));
    }
    None
}

fn recorta(t: &str) -> String {
    let t = t.trim().trim_start_matches("ERROR:").trim();
    if t.is_empty() {
        return crate::i18n::t("sin mensaje", "no message").into();
    }
    let c: String = t.chars().take(160).collect();
    if c.len() < t.len() {
        format!("{c}…")
    } else {
        c
    }
}

/// Qué hacer con una edición.
#[derive(Debug, PartialEq, Eq)]
pub enum Vaiven {
    Sigue,
    Aviso(String),
    Bloquea(String),
}

impl Guardia {
    pub fn nueva() -> Self {
        Self::default()
    }

    /// D14: ¿esta edición DOBLA una prueba? Con las pruebas en rojo, quitar o cambiar aserciones de
    /// un archivo de pruebas es la trampa medida (v8: cambió el valor esperado del segundo pago; v10:
    /// borró el segundo pago parcial; las dos «en verde» y la prueba oculta seguía fallando). La
    /// primera vez se frena con el porqué; si el modelo repite EXACTAMENTE la misma edición (porque la
    /// prueba contradice la especificación), pasa.
    pub fn doblar_prueba(&mut self, ruta: &str, viejo: &str, nuevo: &str) -> Option<String> {
        if !self.pruebas_en_rojo || !es_archivo_de_pruebas(ruta) {
            return None;
        }
        let quitadas = aserciones_quitadas(viejo, nuevo);
        if quitadas.is_empty() {
            return None;
        }
        let huella = {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            (ruta, viejo, nuevo).hash(&mut h);
            h.finish()
        };
        if self.prueba_frenada == Some(huella) {
            self.prueba_frenada = None;
            return None;
        }
        self.prueba_frenada = Some(huella);
        let lista: Vec<String> = quitadas.iter().take(4).map(|l| format!("  {l}")).collect();
        Some(crate::tr!(
            "GOTV frenó esta edición: con las pruebas en ROJO estás quitando o cambiando aserciones de `{ruta}`:\n{}\nDoblar la prueba para que pase es la trampa más común: la prueba queda verde y el error sigue en el código. Antes de tocarla: 1) busca en la especificación la regla de ese caso; 2) si la prueba dice lo mismo que la especificación, el error está en el CÓDIGO: arréglalo ahí. Sólo si la prueba CONTRADICE la especificación (cita la línea en tu siguiente mensaje), repite EXACTAMENTE esta edición y GOTV la dejará pasar.",
            "GOTV stopped this edit: with the tests RED you are removing or changing assertions in `{ruta}`:\n{}\nBending the test so it passes is the most common trap: the test turns green and the bug stays in the code. Before touching it: 1) find the spec rule for that case; 2) if the test says the same as the spec, the bug is in the CODE: fix it there. Only if the test CONTRADICTS the spec (quote the line in your next message), repeat EXACTLY this edit and GOTV will let it through.",
            lista.join("\n")
        ))
    }

    /// DM6.5 (2026-11-13): el «doblar la prueba» del diseño. Esconder el desborde con
    /// `overflow: hidden` en `html`/`body` o ganar especificidad con `!important` hace pasar la
    /// compuerta y deja el defecto. Se frena la 1.ª vez con el porqué y cómo se arregla la causa; si
    /// repite EXACTAMENTE la misma edición (es lo correcto y lo dice), pasa.
    pub fn tapar_sintoma(&mut self, ruta: &str, viejo: &str, nuevo: &str, previo: &str) -> Option<String> {
        let ext = ruta.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
        if !matches!(ext.as_str(), "css" | "scss" | "less" | "html" | "htm" | "jsx" | "tsx" | "vue" | "svelte") {
            return None;
        }
        let tapados = crate::estilo::sintomas_tapados(viejo, nuevo, previo);
        if tapados.is_empty() {
            return None;
        }
        let huella = {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            (ruta, viejo, nuevo).hash(&mut h);
            h.finish()
        };
        if self.sintoma_frenado == Some(huella) {
            self.sintoma_frenado = None;
            return None;
        }
        self.sintoma_frenado = Some(huella);
        let lista: Vec<String> = tapados.iter().take(4).map(|l| format!("  {l}")).collect();
        Some(crate::tr!(
            "GOTV frenó esta edición de `{ruta}`: TAPA el síntoma en vez de arreglar la causa:\n{}\nEl desborde se arregla en el elemento que se sale (la revisión de entrega lo nombra): `max-width: 100%` en imágenes y medios, `min-width: 0` en hijos de grid/flex, `overflow-wrap: anywhere` en textos largos, sin anchos fijos en px. La especificidad se gana con un selector más preciso o `@layer`, no con `!important`. Si de verdad es lo correcto aquí (di por qué en tu siguiente mensaje), repite EXACTAMENTE esta edición y GOTV la dejará pasar.",
            "GOTV stopped this edit of `{ruta}`: it HIDES the symptom instead of fixing the cause:\n{}\nOverflow is fixed in the element that overflows (the delivery review names it): `max-width: 100%` on images and media, `min-width: 0` on grid/flex children, `overflow-wrap: anywhere` on long text, no fixed px widths. Specificity is won with a more precise selector or `@layer`, not `!important`. If it truly is right here (say why in your next message), repeat EXACTLY this edit and GOTV will let it through.",
            lista.join("\n")
        ))
    }

    /// ¿Se le quita `ask_user` al modelo en lo que queda del turno?
    pub fn sin_preguntas(&self) -> bool {
        self.sin_respuesta >= MAX_SIN_RESPUESTA || self.preguntas >= MAX_PREGUNTAS
    }

    /// Anota una pregunta; devuelve el texto extra para el modelo si ya no puede preguntar más.
    pub fn pregunta(&mut self, respondida: bool) -> Option<&'static str> {
        self.preguntas += 1;
        if !respondida {
            self.sin_respuesta += 1;
        }
        self.sin_preguntas().then(|| {
            crate::i18n::t(
                " Ya no puedes preguntar más en este turno: decide lo más sensato con lo que sabes, hazlo y explícalo al final.",
                " You cannot ask any more questions this turn: decide what is most sensible with what you know, do it and explain at the end.",
            )
        })
    }

    /// Estado de la racha de un archivo (para restaurarlo si la edición FALLA: medido en D5, un
    /// «no encontré 'old'» repetido contaba como vaivén sin haber cambiado nada).
    pub fn estado(&self, ruta: &str) -> Option<(Vec<String>, u32, bool)> {
        self.escrito.get(ruta).cloned()
    }

    pub fn restaurar(&mut self, ruta: &str, e: Option<(Vec<String>, u32, bool)>) {
        match e {
            Some(e) => {
                self.escrito.insert(ruta.to_string(), e);
            }
            None => {
                self.escrito.remove(ruta);
            }
        }
    }

    /// Una edición `old → new` en `ruta`. Reeditar lo que el modelo mismo escribió (su `old`
    /// es un `new` anterior) cuenta como vaivén; editar texto nuevo lo reinicia.
    pub fn edicion(&mut self, ruta: &str, old: &str, new: &str) -> Vaiven {
        let (escritos, seguidas, volvio) = self.escrito.entry(ruta.to_string()).or_default();
        let reedita = !old.trim().is_empty()
            && escritos
                .iter()
                .any(|w| w.contains(old.trim()) || old.contains(w.trim()));
        if !reedita {
            // empieza otra racha: el texto original también es un valor «visto»
            escritos.clear();
            *volvio = false;
        }
        let vuelve = escritos.iter().any(|w| w.trim() == new.trim());
        *volvio |= vuelve;
        *seguidas = if reedita { *seguidas + 1 } else { 0 };
        if *seguidas + 1 >= VAIVEN_BLOQUEO && *volvio {
            return Vaiven::Bloquea(crate::tr!(
                "BLOQUEADO: ya cambiaste esta misma parte de {ruta} {n} veces seguidas sin avanzar. No la toques más: {medir} Decide con lo que imprima; si ya está bien, termina.",
                "BLOCKED: you changed this same part of {ruta} {n} times in a row without progress. Leave it: {medir} Decide with what it prints; if it is already fine, finish.",
                n = *seguidas + 1,
                medir = como_medir(ruta)
            ));
        }
        if escritos.is_empty() {
            escritos.push(old.to_string());
        }
        escritos.push(new.to_string());
        if escritos.len() > 12 {
            escritos.remove(0);
        }
        if *seguidas + 1 >= VAIVEN_AVISO || (reedita && vuelve) {
            return Vaiven::Aviso(crate::tr!(
                "⚠ Llevas {n} cambios seguidos de la misma parte de {ruta}{extra}. Antes de otro cambio, MIDE (no adivines): {medir} Al 5.º se bloquea.",
                "⚠ That is {n} changes in a row to the same part of {ruta}{extra}. Before another change, MEASURE (do not guess): {medir} The 5th is blocked.",
                n = *seguidas + 1,
                medir = como_medir(ruta),
                extra = if vuelve { crate::i18n::t(" (y volviste a un valor que ya habías puesto)", " (and you went back to a value you had already set)") } else { "" }
            ));
        }
        Vaiven::Sigue
    }
}

impl Guardia {
    /// Medido (D10, 9B): recorrió la misma hoja de 2.240 líneas ~4 veces en UN turno, con
    /// desplazamientos distintos (1–400, 401–800, luego 1120–1519…), hasta 37 k tokens de contexto.
    /// Se mide COBERTURA: si el tramo pedido ya está entero dentro de lo leído (y el archivo no
    /// cambió), es relectura; la 1.ª se permite (por si el contexto se compactó), desde la 2.ª
    /// no se reenvía.
    pub fn lectura(&mut self, ruta: &std::path::Path, desde: u64, cuantas: u64) -> Option<String> {
        let mtime = std::fs::metadata(ruta).and_then(|m| m.modified()).ok();
        let e = self
            .lecturas
            .entry(ruta.display().to_string())
            .or_insert((Vec::new(), mtime, 0));
        if e.1 != mtime {
            *e = (Vec::new(), mtime, 0); // cambió en disco: leerlo de nuevo es legítimo
        }
        let (a, b) = (desde.max(1), desde.max(1) + cuantas.max(1));
        // ¿[a, b) está cubierto por la unión de lo leído?
        let mut tramos = e.0.clone();
        tramos.sort();
        let mut hasta = a;
        for (x, y) in &tramos {
            if *x <= hasta && *y > hasta {
                hasta = *y;
            }
        }
        let cubierto = hasta >= b;
        e.0.push((a, b));
        // Una hoja de estilos se busca, no se lee entera (medido, D10: 23 pasos y 2.240 líneas para
        // un 9B cuando los colores y las fuentes ya le llegaban medidos). Pasadas 600 líneas: grep.
        let css = matches!(
            ruta.extension().and_then(|x| x.to_str()),
            Some("css" | "scss" | "sass" | "less")
        );
        let leidas: u64 = {
            let mut t = e.0.clone();
            t.sort();
            let (mut total, mut fin) = (0u64, 0u64);
            for (x, y) in t {
                let x = x.max(fin);
                if y > x {
                    total += y - x;
                }
                fin = fin.max(y);
            }
            total
        };
        if css && !cubierto && leidas > 600 {
            e.2 += 1;
            // si insiste, el aviso pasa a ORDEN: no se le deja en bucle preguntando por la hoja
            return Some(if e.2 >= 3 {
                crate::tr!(
                    "BASTA de leer {}: ya tienes TODO lo necesario (colores, fuentes, clases y el <head> que llegaron con la orden, y lo que ya leíste). Escribe AHORA el archivo pedido con `write_file` usando esas clases; no se reenviará más de esta hoja en este turno.",
                    "STOP reading {}: you already have EVERYTHING you need (colours, fonts, classes and the <head> that came with the request, plus what you read). Write the requested file NOW with `write_file` using those classes; no more of this stylesheet will be sent this turn.",
                    ruta.display()
                )
            } else {
                crate::tr!(
                    "Ya llevas {leidas} líneas de {} en este turno. Una hoja de estilos no se lee entera: los colores, fuentes y clases del proyecto ya te llegaron medidos con la orden. Busca SÓLO lo que te falte con `grep` y escribe la página.",
                    "You have read {leidas} lines of {} this turn. A stylesheet is not read whole: the project's colours, fonts and classes already came measured with the request. Search ONLY what is missing with `grep` and write the page.",
                    ruta.display()
                )
            });
        }
        if !cubierto {
            return None;
        }
        e.2 += 1;
        // Prueba Cable (2026-11-12): tres «ya leíste» seguidos y el 35B dio la tarea por terminada
        // («Ahora leo almacen.py…» sin llamar a nada), a los 18 de 30 minutos. Un freno sin salida
        // parece un final. Ahora: la 1.ª relectura pasa; la 2.ª se frena CON alternativas; si insiste
        // una 3.ª vez, es que lo necesita (p. ej. el compactado se lo quitó) y se le da.
        if e.2 >= 3 {
            e.2 = 0;
            return None;
        }
        (e.2 == 2).then(|| {
            crate::tr!(
                "Ya leíste ese tramo de {} hace poco y no ha cambiado. No te detengas: AVANZA con otra acción. (1) Si buscas algo concreto, usa `grep` con el nombre (clase, función, variable). (2) Ejecuta las pruebas y quédate con el PRIMER error. (3) Si ya sabes qué cambiar, edítalo. Si de verdad necesitas ver ese tramo otra vez, vuelve a pedirlo y se te dará.",
                "You read that part of {} recently and it has not changed. Do not stop: MOVE ON with another action. (1) If you are looking for something specific, use `grep` with the name (class, function, variable). (2) Run the tests and take the FIRST error. (3) If you already know what to change, edit it. If you really need that part again, ask for it again and you will get it.",
                ruta.display()
            )
        })
    }

    /// Tras compactar el contexto, lo leído ya NO está a la vista del modelo (el resumen lo sustituye):
    /// el freno de relecturas olvida lo que vio. Medido (prueba Cable, 2026-11-12): tras compactar, el
    /// 35B pidió releer almacen.py y se le negó tres veces «porque estaba en su contexto».
    pub fn olvidar_lecturas(&mut self) {
        self.lecturas.clear();
    }
}

/// `open` de macOS abre apps REALES del usuario (Safari, Chrome…): medido (D10) el 9B lo intentó
/// 6 veces para «ver» su página. Se le indica el navegador propio.
pub fn abre_app(cmd: &str) -> Option<String> {
    let usa_open = cmd.split(['&', ';', '|']).any(|parte| {
        let w: Vec<&str> = parte.split_whitespace().collect();
        w.first()
            .is_some_and(|x| *x == "open" || x.ends_with("/open"))
            && (w.iter().any(|t| *t == "-a" || *t == "-b")
                || w.iter().skip(1).any(|t| {
                    let t = t.trim_matches(['"', '\'']).to_lowercase();
                    t.ends_with(".html")
                        || t.ends_with(".htm")
                        || t.ends_with(".svg")
                        || t.starts_with("http://")
                        || t.starts_with("https://")
                        || t.starts_with("file://")
                }))
    });
    // O1 (medido 2026-11-11): el 9B pidió `osascript` en `bash` (la caja lo deja sin acceso a las
    // apps: «No puede obtenerse application "Microsoft Word" (-1728)», y creyó que Word no existía)
    // y `open -a "Word"` (y el aviso hablaba de Safari). Las apps se manejan con `desktop`.
    let a_una_app = |p: &str| {
        let w: Vec<&str> = p.split_whitespace().collect();
        let i = w.iter().position(|t| *t == "-a" || *t == "-b");
        let app = i.and_then(|i| w.get(i + 1)).map(|t| t.trim_matches(['"', '\'']).to_lowercase()).unwrap_or_default();
        w.first().is_some_and(|x| *x == "open" || x.ends_with("/open")) && i.is_some() && !["safari", "google", "chrome", "firefox", "arc", "brave", "microsoft edge"].iter().any(|b| app.starts_with(b))
    };
    if cmd.split_whitespace().any(|t| t == "osascript" || t.ends_with("/osascript")) || cmd.split(['&', ';', '|']).any(a_una_app) {
        return Some(
            crate::i18n::t(
                "Las apps del Mac NO se manejan desde `bash` (la caja no les llega). Usa la herramienta `desktop`: [\"recetas\",\"Microsoft Word\"] (lo probado para Word, Excel y PowerPoint), [\"applescript\",\"tell application …\"] para ejecutar, [\"launch\",\"App\"] para abrir y [\"guardar\",\"Microsoft Word\",\"carta.pdf\"] para guardar en el proyecto.",
                "Mac apps are NOT driven from `bash` (the sandbox does not reach them). Use the `desktop` tool: [\"recetas\",\"Microsoft Word\"] (what is tested for Word, Excel and PowerPoint), [\"applescript\",\"tell application …\"] to run, [\"launch\",\"App\"] to open and [\"guardar\",\"Microsoft Word\",\"letter.pdf\"] to save into the project.",
            )
            .to_string(),
        );
    }
    // D5 (medido): `npm run dev -- --port 5173 --open` → Vite abre el navegador del SISTEMA
    let abre_servidor = cmd
        .split_whitespace()
        .any(|t| t == "--open" || t.starts_with("--open="))
        && [
            "npm", "pnpm", "yarn", "bun", "npx", "vite", "next", "astro", "serve", "webpack",
        ]
        .iter()
        .any(|h| {
            cmd.split_whitespace()
                .any(|t| t == *h || t.ends_with(&format!("/{h}")))
        });
    // `--host` sin valor (o 0.0.0.0) expone el servidor de desarrollo a TODA la red local
    let palabras: Vec<&str> = cmd.split_whitespace().collect();
    let expone = palabras.windows(2).any(|w| {
        w[0] == "--host" && (w[1].starts_with('-') || w[1] == "0.0.0.0" || w[1] == "::")
    }) || palabras.contains(&"--host=0.0.0.0")
        || palabras.last() == Some(&"--host");
    if expone
        && ["npm", "pnpm", "yarn", "bun", "npx", "vite", "next", "astro"]
            .iter()
            .any(|h| palabras.contains(h))
    {
        return Some(
            crate::i18n::t(
                "No uses `--host`: expone el servidor de desarrollo a TODA la red local. Córrelo sin `--host` (escucha sólo en localhost, que es donde lo ve la herramienta `browser`).",
                "Do not use `--host`: it exposes the dev server to the WHOLE local network. Run it without `--host` (it listens on localhost only, which is where the `browser` tool sees it).",
            )
            .to_string(),
        );
    }
    if abre_servidor {
        return Some(
            crate::i18n::t(
                "No uses `--open`: abre el navegador del USUARIO. Corre el mismo comando SIN `--open` (queda en segundo plano) y mira la página con la herramienta `browser`: [\"open\",\"http://localhost:PUERTO\"] y luego [\"snapshot\",\"-i\"].",
                "Do not use `--open`: it opens the USER's browser. Run the same command WITHOUT `--open` (it stays in the background) and look at the page with the `browser` tool: [\"open\",\"http://localhost:PORT\"] then [\"snapshot\",\"-i\"].",
            )
            .to_string(),
        );
    }
    usa_open.then(|| {
        crate::i18n::t(
            "No abras apps del usuario (Safari, Chrome, Firefox) con `open`: para ver o comprobar una página usa la herramienta `browser` (navegador propio de GOTV): [\"open\",\"file:///ruta/ABSOLUTA/en/el/proyecto/pagina.html\"] o [\"open\",\"http://localhost:8080\"], y luego [\"snapshot\",\"-i\"] o [\"a11y\"]. Ábrela DONDE ESTÁ: si la copias a otra carpeta sus estilos relativos (./style.css) dejan de cargarse y verás una página sin diseño.",
            "Do not open the user's apps (Safari, Chrome, Firefox) with `open`: to view or check a page use the `browser` tool (GOTV's own browser): [\"open\",\"file:///ABSOLUTE/path/in/the/project/page.html\"] or [\"open\",\"http://localhost:8080\"], then [\"snapshot\",\"-i\"] or [\"a11y\"]. Open it WHERE IT IS: if you copy it elsewhere its relative styles (./style.css) stop loading and you will see an unstyled page.",
        )
        .to_string()
    })
}

/// Sinónimos de campos que los modelos usan (muchos se entrenaron con otros agentes: `command`,
/// `file_path`, `old_string`…). Medido (C5, 9B): mandó `{"command": "uv init"}` a `bash`, que pide
/// `cmd`, 24 veces seguidas. Se traducen al nombre del esquema SÓLO si ese campo falta.
const SINONIMOS: &[(&str, &[&str])] = &[
    (
        "path",
        &[
            "file_path",
            "filepath",
            "file",
            "filename",
            "file_name",
            "ruta",
            "archivo", // juez-idioma: datos (sinónimo de campo)
            "directory",
            "dir",
            "folder",
            "carpeta",
        ],
    ),
    ("cmd", &["command", "cmdline", "script", "comando", "shell"]),
    (
        "content",
        &["contents", "text", "data", "body", "contenido", "texto"],
    ),
    (
        "old",
        &[
            "old_string",
            "old_str",
            "old_text",
            "search",
            "find",
            "buscar",
            "original",
        ],
    ),
    (
        "new",
        &[
            "new_string",
            "new_str",
            "new_text",
            "replace",
            "replacement",
            "reemplazo",
        ],
    ),
    (
        "pattern",
        &["query", "regex", "search", "patron", "patrón", "term"], // juez-idioma: datos (sinónimos de campo)
    ),
    ("question", &["pregunta", "prompt", "q"]),
    ("url", &["link", "href", "uri"]),
    ("name", &["skill", "nombre", "skill_name"]),
];

/// Traduce los sinónimos al nombre que pide el esquema (`propiedades` = las de la herramienta).
/// Devuelve los argumentos arreglados y los cambios hechos (para el registro).
pub fn normaliza_args(
    propiedades: Option<&serde_json::Value>,
    args: &serde_json::Value,
) -> (serde_json::Value, Vec<String>) {
    let (Some(props), Some(obj)) = (propiedades.and_then(|p| p.as_object()), args.as_object())
    else {
        return (args.clone(), vec![]);
    };
    let mut o = obj.clone();
    let mut cambios = Vec::new();
    for (canon, alias) in SINONIMOS {
        if !props.contains_key(*canon) || o.contains_key(*canon) {
            continue;
        }
        if let Some(a) = alias
            .iter()
            .find(|a| o.contains_key(**a) && !props.contains_key(**a))
        {
            if let Some(v) = o.remove(*a) {
                o.insert((*canon).to_string(), v);
                cambios.push(format!("{a}→{canon}"));
            }
        }
    }
    (serde_json::Value::Object(o), cambios)
}

/// Medido 2026-11-16 (banco `arreglar`, 9B): mezcla `edit` y `multi_edit` — llama a `edit` con la lista
/// `edits` de `multi_edit`, la manda como TEXTO (un JSON dentro de una cadena) y pone `path` dentro de cada
/// edición en vez de arriba. Con todo eso GOTV respondía «faltan argumentos» y se fueron 30 pasos. Aquí se
/// deja la llamada como la quiso: `multi_edit`, con su lista de verdad y su `path`. Devuelve qué cambió.
pub fn arregla_ediciones(nombre: &mut String, args: &mut serde_json::Value) -> Option<String> {
    let Some(o) = args.as_object_mut() else { return None };
    let mut cambios = Vec::new();
    if let Some(serde_json::Value::String(s)) = o.get("edits") {
        if let Ok(v @ serde_json::Value::Array(_)) = serde_json::from_str::<serde_json::Value>(s) {
            o.insert("edits".into(), v);
            cambios.push("edits: texto→lista".to_string());
        }
    }
    let lista = o.get("edits").and_then(|v| v.as_array()).filter(|a| !a.is_empty()).cloned();
    let Some(lista) = lista else { return (!cambios.is_empty()).then(|| cambios.join(", ")) };
    if nombre == "edit" {
        *nombre = "multi_edit".into();
        cambios.push("edit→multi_edit".into());
    }
    if nombre == "multi_edit" && o.get("path").and_then(|p| p.as_str()).is_none_or(|p| p.trim().is_empty()) {
        let rutas: std::collections::BTreeSet<&str> = lista.iter().filter_map(|e| e.get("path").and_then(|p| p.as_str())).collect();
        if rutas.len() == 1 {
            let p = rutas.into_iter().next().unwrap_or_default().to_string();
            o.insert("path".into(), serde_json::Value::String(p));
            cambios.push(crate::i18n::t("path de las ediciones→arriba", "path from the edits→top").into());
        }
    }
    (!cambios.is_empty()).then(|| cambios.join(", "))
}

/// Campos obligatorios que faltan o vienen vacíos, según el esquema JSON de la herramienta
/// (`parameters.required`). `None` si la llamada está completa.
pub fn faltan(parametros: &serde_json::Value, args: &serde_json::Value) -> Option<String> {
    let req = parametros.get("required")?.as_array()?;
    let props = parametros.get("properties");
    let mut malos = Vec::new();
    for r in req.iter().filter_map(|x| x.as_str()) {
        let tipo = props
            .and_then(|p| p.get(r))
            .and_then(|p| p.get("type"))
            .and_then(|t| t.as_str())
            .unwrap_or("");
        // sólo los campos de TEXTO: una lista u objeto ausente lo decide cada herramienta
        // (p. ej. `ask_user` sin `options` = pregunta abierta, y es válido)
        let es_texto = tipo == "string" || tipo.is_empty();
        match args.get(r) {
            None | Some(serde_json::Value::Null) if es_texto => {
                malos.push(format!("«{r}» (falta)"))
            }
            Some(serde_json::Value::String(s))
                if s.trim().is_empty() && (tipo == "string" || tipo.is_empty()) =>
            {
                malos.push(crate::tr!("«{r}» (vacío)", "«{r}» (empty)"))
            }
            _ => {}
        }
    }
    (!malos.is_empty()).then(|| malos.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Las formas que mandó el 9B en el banco `arreglar` (2026-11-16), tal cual.
    #[test]
    fn edit_con_edits_en_texto_es_un_multi_edit() {
        let (mut n, mut a) = ("edit".to_string(), json!({"arregla": ["index.html:2"], "edits": "[{\"new\": \"b\", \"old\": \"a\"}]", "path": "index.html"}));
        let m = arregla_ediciones(&mut n, &mut a).unwrap();
        assert_eq!(n, "multi_edit");
        assert_eq!(a["edits"], json!([{"new": "b", "old": "a"}]));
        assert_eq!(a["path"], "index.html");
        assert!(m.contains("edit→multi_edit") && m.contains("texto→lista"), "{m}");
        // el path dentro de cada edición sube arriba
        let (mut n, mut a) = ("multi_edit".to_string(), json!({"edits": [{"old": "a", "new": "b", "path": "index.html"}, {"old": "c", "new": "d", "path": "index.html"}]}));
        arregla_ediciones(&mut n, &mut a).unwrap();
        assert_eq!(a["path"], "index.html");
        // dos archivos distintos: no se adivina
        let (mut n, mut a) = ("multi_edit".to_string(), json!({"edits": [{"old": "a", "new": "b", "path": "x"}, {"old": "c", "new": "d", "path": "y"}]}));
        arregla_ediciones(&mut n, &mut a);
        assert!(a.get("path").is_none());
        // un edit normal no se toca
        let (mut n, mut a) = ("edit".to_string(), json!({"path": "x", "old": "a", "new": "b"}));
        assert!(arregla_ediciones(&mut n, &mut a).is_none());
        assert_eq!(n, "edit");
    }

    /// O0, la secuencia real (2026-11-11): Word abrió sin ventana y el 9B pidió `list-windows` 27
    /// veces con «0 ventanas». Se deja repetir (la pantalla cambia sola) hasta QUIETO_MAX respuestas
    /// iguales; la siguiente se bloquea. Si la respuesta cambia, o es otra llamada, la cuenta vuelve.
    #[test]
    fn pantalla_que_no_cambia_se_corta() {
        let mut g = Guardia::nueva();
        let k = r#"desktop {"args":["list-windows","--app","Microsoft Word"]}"#;
        for _ in 0..QUIETO_MAX {
            assert!(g.misma_llamada("desktop", k).is_none(), "repetir la pantalla vale");
            g.resultado(k, true, "0 ventanas");
        }
        let m = g.misma_llamada("desktop", k).expect("la 4.ª con la misma respuesta se bloquea");
        assert!(m.contains("EXACTAMENTE lo mismo") || m.contains("EXACTLY the same"));
        assert!(m.contains("«0 ventanas»"), "el bloqueo trae el dato: {m}");
        // si la ventana aparece (respuesta distinta), vuelve a contar desde cero
        let mut g = Guardia::nueva();
        g.resultado(k, true, "0 ventanas");
        g.resultado(k, true, "0 ventanas");
        g.resultado(k, true, "1 ventana: w-12 «Documento1»");
        assert!(g.misma_llamada("desktop", k).is_none());
        // EN CICLO con otras llamadas (la 2.ª corrida real: launch → snapshot ✗ → list-windows)
        // también se corta: lo que cuenta son las últimas QUIETO_VENTANA, no sólo las seguidas
        let mut g = Guardia::nueva();
        for _ in 0..QUIETO_MAX {
            g.resultado(r#"desktop {"args":["launch","Microsoft Word"]}"#, true, "✓ launch");
            g.resultado(r#"desktop {"args":["snapshot"]}"#, false, "✗ WINDOW_NOT_FOUND");
            g.resultado(k, true, "0 ventanas");
        }
        assert!(g.misma_llamada("desktop", k).is_some(), "el ciclo se corta");
        // …pero lo viejo (fuera de la ventana) ya no cuenta
        let mut g = Guardia::nueva();
        for _ in 0..QUIETO_MAX {
            g.resultado(k, true, "0 ventanas");
        }
        for i in 0..QUIETO_VENTANA {
            g.resultado(&format!("desktop {i}"), true, "otra cosa");
        }
        assert!(g.misma_llamada("desktop", k).is_none());
    }

    #[test]
    fn preguntas_sin_respuesta_se_cortan() {
        let mut g = Guardia::nueva();
        assert!(g.pregunta(false).is_none());
        assert!(!g.sin_preguntas());
        assert!(
            g.pregunta(false).is_some(),
            "a la 2.ª sin respuesta: ya no pregunta"
        );
        assert!(g.sin_preguntas());
        // respondidas cuentan hasta 4
        let mut g = Guardia::nueva();
        for _ in 0..3 {
            assert!(g.pregunta(true).is_none());
        }
        assert!(g.pregunta(true).is_some());
    }

    /// El aviso y el bloqueo dicen cómo medir según el archivo: una prueba → imprimir el error; una
    /// pantalla → el navegador; lo demás → un experimento que imprima.
    #[test]
    fn el_vaiven_dice_como_medir_segun_el_archivo() {
        let bloqueo = |ruta: &str| {
            let mut g = Guardia::nueva();
            let mut ultimo = Vaiven::Sigue;
            for (o, n) in [("a", "b"), ("b", "c"), ("c", "d"), ("d", "b"), ("b", "a")] {
                ultimo = g.edicion(ruta, &format!("x = {o}"), &format!("x = {n}"));
            }
            match ultimo {
                Vaiven::Bloquea(m) => m,
                otro => panic!("{ruta}: {otro:?}"),
            }
        };
        let p = bloqueo("api/src/api.test.ts");
        assert!(p.contains("IMPRIME") || p.contains("PRINT"), "{p}");
        assert!(!p.contains("color") && !p.contains("navegador") && !p.contains("browser"), "{p}");
        let w = bloqueo("web/src/App.tsx");
        assert!(w.contains("navegador") || w.contains("browser"), "{w}");
        let c = bloqueo("api/src/dominio/totales.ts");
        assert!((c.contains("experimento") || c.contains("experiment")) && !c.contains("navegador"), "{c}");
        assert_eq!(como_medir("tests/test_pagos.py"), como_medir("src/x_test.go"));
    }

    #[test]
    fn el_vaiven_real_del_9b_avisa_y_bloquea() {
        // la secuencia medida en D1 (arreglar): #222 → #000 → #333 → #111 → #000 → …
        let mut g = Guardia::nueva();
        let pasos = [
            ("#222222", "#000000"),
            ("#000000", "#333333"),
            ("#333333", "#111111"),
            ("#111111", "#000000"),
            ("#000000", "#222222"),
        ];
        let r: Vec<Vaiven> = pasos
            .iter()
            .map(|(o, n)| {
                g.edicion(
                    "index.html",
                    &format!(".suave{{color:{o}}}"),
                    &format!(".suave{{color:{n}}}"),
                )
            })
            .collect();
        assert_eq!(r[0], Vaiven::Sigue, "el primer cambio es legítimo");
        assert_eq!(r[1], Vaiven::Sigue);
        assert!(matches!(r[2], Vaiven::Aviso(_)), "{:?}", r[2]);
        assert!(
            matches!(r[3], Vaiven::Aviso(ref m) if m.contains("volviste")),
            "{:?}",
            r[3]
        );
        assert!(matches!(r[4], Vaiven::Bloquea(_)), "{:?}", r[4]);
    }

    #[test]
    fn editar_partes_distintas_no_es_vaiven() {
        let mut g = Guardia::nueva();
        for i in 0..8 {
            assert_eq!(
                g.edicion(
                    "a.rs",
                    &format!("fn f{i}() {{}}"),
                    &format!("fn f{i}() {{ todo!() }}")
                ),
                Vaiven::Sigue
            );
        }
        // depurar hacia adelante (reeditar lo suyo SIN volver atrás) avisa pero nunca bloquea
        let mut g = Guardia::nueva();
        let mut prev = "fn f() { 1 }".to_string();
        for i in 2..10 {
            let next = format!("fn f() {{ {i} }}");
            assert!(
                !matches!(g.edicion("a.rs", &prev, &next), Vaiven::Bloquea(_)),
                "bloqueó una depuración legítima"
            );
            prev = next;
        }
        // otro archivo no se mezcla
        let mut g = Guardia::nueva();
        g.edicion("a.css", "x{color:red}", "x{color:blue}");
        assert_eq!(
            g.edicion("b.css", "x{color:blue}", "x{color:green}"),
            Vaiven::Sigue
        );
    }

    #[test]
    fn la_tercera_lectura_igual_no_se_reenvia() {
        let f = crate::CarpetaPrueba::archivo("gotv-lect", "txt");
        std::fs::write(&f, "a{}").unwrap();
        let mut g = Guardia::nueva();
        // 1.ª pasada por la hoja: todo nuevo
        for d in [1, 401, 801] {
            assert!(g.lectura(&f, d, 400).is_none());
        }
        // el patrón real del 9B: otro desplazamiento, pero DENTRO de lo ya leído
        assert!(
            g.lectura(&f, 300, 400).is_none(),
            "la 1.ª relectura se permite"
        );
        let freno = g.lectura(&f, 600, 400).expect("la 2.ª relectura cubierta se frena");
        assert!(freno.contains("grep") && freno.contains("pruebas") && freno.contains("se te dará"), "el freno da alternativas: {freno}");
        assert!(g.lectura(&f, 600, 400).is_none(), "si insiste una 3.ª vez, se le da (lo necesita)");
        // tras compactar, lo leído ya no está a la vista: se olvida y se puede releer sin freno
        assert!(g.lectura(&f, 1, 400).is_none());
        assert!(g.lectura(&f, 1, 400).is_some());
        g.olvidar_lecturas();
        assert!(g.lectura(&f, 1, 400).is_none(), "tras compactar, releer es legítimo");
        assert!(
            g.lectura(&f, 1100, 400).is_none(),
            "un tramo nuevo (1201+) sí se lee"
        );
        // si el archivo cambia, leerlo es legítimo otra vez
        std::thread::sleep(std::time::Duration::from_millis(1100));
        std::fs::write(&f, "b{}").unwrap();
        assert!(g.lectura(&f, 1, 400).is_none());
        let _ = std::fs::remove_file(&f);
    }

    #[test]
    fn una_hoja_de_estilos_se_busca_no_se_lee_entera() {
        let f = crate::CarpetaPrueba::archivo("gotv-hoja", "css");
        std::fs::write(&f, "a{}").unwrap();
        let mut g = Guardia::nueva();
        // el patrón medido: de 100 en 100
        let cortes: Vec<bool> = (0..10)
            .map(|i| g.lectura(&f, 1 + i * 100, 100).is_some())
            .collect();
        assert_eq!(
            cortes.iter().position(|c| *c),
            Some(6),
            "se corta al pasar de 600 líneas: {cortes:?}"
        );
        // un .rs no tiene ese tope
        let r = crate::CarpetaPrueba::archivo("gotv-codigo", "rs");
        std::fs::write(&r, "fn a(){}").unwrap();
        let mut g = Guardia::nueva();
        assert!((0..10).all(|i| g.lectura(&r, 1 + i * 100, 100).is_none()));
        let _ = std::fs::remove_file(&f);
        let _ = std::fs::remove_file(&r);
    }

    #[test]
    fn open_de_apps_se_desvia_al_navegador_propio() {
        for malo in [
            "open -a Safari /x/ayuda.html",
            "open -a \"Google Chrome\" a.html",
            "cd x && open ayuda.html",
            "open http://localhost:8080",
            "/usr/bin/open -b com.apple.Safari x",
        ] {
            assert!(abre_app(malo).is_some(), "{malo}");
        }
        for bueno in [
            "openssl rand -hex 8",
            "echo open -a",
            "npm run open",
            "open-cli --help",
            "ls open",
        ] {
            assert!(abre_app(bueno).is_none(), "{bueno}");
        }
    }

    /// O1: lo que el 9B intentó en la corrida real para llegar a Word desde `bash` va a `desktop`
    /// (no al navegador); abrir un navegador sigue yendo al navegador propio.
    #[test]
    fn las_apps_desde_bash_van_a_desktop() {
        for (cmd, a_desktop) in [
            ("osascript -e 'tell application \"Microsoft Word\" to activate' 2>&1", true),
            ("/usr/bin/osascript carta.scpt", true),
            ("open -a \"Word\"", true),
            ("open -a 'Microsoft Excel' datos.xlsx", true),
            ("open -b com.microsoft.Word", true),
            ("open -a Safari ayuda.html", false),
            ("open -a \"Google Chrome\" a.html", false),
        ] {
            let m = abre_app(cmd).unwrap_or_default();
            assert_eq!(m.contains("`desktop`"), a_desktop, "{cmd} → {m}");
            assert!(!m.is_empty(), "{cmd} se intercepta");
        }
    }

    #[test]
    fn los_sinonimos_de_otros_agentes_se_traducen() {
        use serde_json::json;
        let bash = json!({"cmd": {"type": "string"}, "timeout": {"type": "integer"}});
        let (a, c) = normaliza_args(Some(&bash), &json!({"command": "uv init", "timeout": 30}));
        assert_eq!(a, json!({"cmd": "uv init", "timeout": 30}));
        assert_eq!(c, vec!["command→cmd"]);
        let edit = json!({"path": {}, "old": {}, "new": {}});
        let (a, _) = normaliza_args(
            Some(&edit),
            &json!({"file_path": "a.rs", "old_string": "x", "new_string": "y"}),
        );
        assert_eq!(a, json!({"path": "a.rs", "old": "x", "new": "y"}));
        // si el campo propio ya está, no se toca nada
        let (a, c) = normaliza_args(Some(&bash), &json!({"cmd": "ls", "command": "rm -rf /"}));
        assert_eq!(a["cmd"], "ls");
        assert!(c.is_empty());
        // un campo que SÍ es del esquema no se roba: grep tiene `pattern`; `search` sólo si falta
        let grep = json!({"pattern": {}, "path": {}});
        let (a, _) = normaliza_args(Some(&grep), &json!({"query": "TODO"}));
        assert_eq!(a, json!({"pattern": "TODO"}));
    }

    #[test]
    fn argumentos_obligatorios() {
        let p = json!({"type":"object","properties":{"texto":{"type":"string"},"n":{"type":"integer"}},"required":["texto","n"]});
        assert_eq!(faltan(&p, &json!({"texto":"vacio","n":0})), None);
        assert_eq!(
            faltan(&p, &json!({"texto":"","n":1})).as_deref(),
            Some("«texto» (vacío)")
        );
        assert_eq!(
            faltan(&p, &json!({"n":1})).as_deref(),
            Some("«texto» (falta)")
        );
        assert_eq!(
            faltan(&json!({"type":"object"}), &json!({})),
            None,
            "sin required no se exige nada"
        );
        let q = json!({"type":"object","properties":{"question":{"type":"string"},"options":{"type":"array"}},"required":["question","options"]});
        assert_eq!(
            faltan(&q, &json!({"question":"¿A o B?"})),
            None,
            "una lista ausente no es error (pregunta abierta)"
        );
    }

    #[test]
    fn la_misma_llamada_fallida_no_se_repite_una_tercera_vez() {
        let mut g = Guardia::default();
        let k = "browser {\"args\":[\"click\",\"@e43\"]}";
        assert!(g.repite_fallo(k).is_none());
        g.resultado(k, false, "ERROR: ");
        assert!(g.repite_fallo(k).is_none(), "un fallo se puede reintentar");
        g.resultado(k, false, "ERROR: ");
        let m = g.repite_fallo(k).expect("la tercera no se ejecuta");
        assert!(
            m.contains("snapshot -i") && m.contains("sin mensaje"),
            "{m}"
        );
        assert!(
            g.repite_fallo("browser {\"args\":[\"snapshot\",\"-i\"]}")
                .is_none(),
            "otra llamada sí"
        );
        // un error DISTINTO reinicia la racha (algo cambió); un éxito también
        g.resultado(k, false, "ERROR: otro");
        assert!(g.repite_fallo(k).is_none());
        g.resultado(k, false, "ERROR: otro");
        g.resultado(k, true, "ok");
        assert!(g.repite_fallo(k).is_none());
    }

    #[test]
    fn el_servidor_no_abre_el_navegador_del_usuario() {
        assert!(abre_app("cd cafeteria && npm run dev -- --port 5173 --open").is_some());
        assert!(abre_app("npx vite --open=/admin").is_some());
        assert!(abre_app("cd cafeteria && npm run dev -- --port 5173").is_none());
        assert!(
            abre_app("grep -- --open README.md").is_none(),
            "no es un servidor"
        );
        assert!(abre_app("cd cafeteria && npm run dev -- --host --port 5173").is_some());
        assert!(abre_app("npx vite --host 0.0.0.0").is_some());
        assert!(abre_app("npm run dev -- --host").is_some());
        assert!(abre_app("npx vite --host 127.0.0.1").is_none(), "localhost explícito está bien");
    }

    #[test]
    fn la_app_en_verde_pide_terminar_una_vez() {
        let root = std::path::Path::new("/tmp/proy");
        let mut g = Guardia::default();
        let b = |c: &str| serde_json::json!({ "cmd": c });
        assert!(
            g.app_paso(root, "bash", &b("cd cafe && npm test"), true)
                .is_none(),
            "sin app registrada no hay nada"
        );
        g.app_creada(root.join("cafe"));
        assert!(
            g.app_paso(root, "bash", &b("cd cafe && npm test"), true)
                .is_none(),
            "falta el build"
        );
        assert!(
            g.app_paso(root, "bash", &b("cd cafe && npm run build"), false)
                .is_none(),
            "un build que falla no cuenta"
        );
        let m = g
            .app_paso(root, "bash", &b("cd cafe && npm run build"), true)
            .expect("lista");
        assert!(m.contains("TERMINA AHORA"), "{m}");
        assert!(
            g.app_paso(root, "bash", &b("cd cafe && npm run build"), true)
                .is_none(),
            "se avisa una vez"
        );
        // editar dentro de la app obliga a volver a comprobar
        g.app_paso(
            root,
            "edit",
            &serde_json::json!({"path": "cafe/src/App.tsx"}),
            true,
        );
        assert!(g
            .app_paso(root, "bash", &b("cd cafe && npm run build"), true)
            .is_none());
        assert!(
            g.app_paso(root, "bash", &b("cd cafe && npm test 2>&1 | tail -5"), true)
                .is_none(),
            "con tubería el código de salida no prueba nada"
        );
        assert!(g
            .app_paso(root, "bash", &b("cd cafe && npx vitest run"), true)
            .is_some());
        // con la app en verde, pasos sin cambiar nada cierran el turno al tope
        for _ in 1..OCIOSO_MAX {
            assert!(g.app_ociosa().is_none());
        }
        assert_eq!(g.app_ociosa(), Some(root.join("cafe")));
        // otra carpeta no cuenta
        assert!(g
            .app_paso(
                root,
                "bash",
                &b("cd otra && npm test && npm run build"),
                true
            )
            .is_none());
    }

    #[test]
    fn espera_larga_en_la_terminal() {
        assert_eq!(espera_larga("sleep 120 && curl -s http://127.0.0.1:18779"), Some(120));
        assert_eq!(espera_larga("sleep 2m; ls"), Some(120));
        assert_eq!(espera_larga("(sleep 90)"), Some(90));
        assert_eq!(espera_larga("sleep 5 && curl localhost:3000"), None, "esperar a que arranque algo, sí");
        assert_eq!(espera_larga("echo sleep 300"), Some(300), "en la duda, se trata como espera");
        assert_eq!(espera_larga("cargo test"), None);
    }

    #[test]
    fn d11_la_misma_llamada_tres_veces_y_la_herramienta_como_comando() {
        let mut g = Guardia::default();
        let k = "bash {\"cmd\":\"ls -la ~/.claude/agents/\"}";
        assert!(g.misma_llamada("bash", k).is_none());
        assert!(g.misma_llamada("bash", k).is_none());
        assert!(g.misma_llamada("bash", k).expect("la 3.ª no corre").contains("3.ª vez"));
        for _ in 1..ATASCO_MAX {
            assert!(!g.atascado());
            g.misma_llamada("bash", k);
        }
        assert!(g.atascado(), "{} bloqueos seguidos: atascado", ATASCO_MAX);
        g.resultado("otra", true, "ok");
        assert!(!g.atascado(), "algo se ejecutó: ya no");
        assert!(g.misma_llamada("bash", "bash {\"cmd\":\"ls\"}").is_none(), "otra llamada reinicia");
        // vaivén A-B-A-B-A: la quinta no corre
        let mut v = Guardia::default();
        let (a, b) = ("write_file ok.txt", "edit ok.txt");
        for k in [a, b, a, b] {
            assert!(v.misma_llamada("x", k).is_none(), "{k}");
        }
        assert!(v.misma_llamada("x", a).expect("vaivén").contains("alternando"));
        // un trabajo normal que repite herramienta con OTROS argumentos no es vaivén
        let mut n = Guardia::default();
        for k in ["edit a1", "bash t", "edit a2", "bash t", "edit a3"] {
            assert!(n.misma_llamada("x", k).is_none(), "{k}");
        }
        for _ in 0..4 {
            assert!(g.misma_llamada("browser", "browser snapshot").is_none(), "el navegador cambia solo");
        }
        let root = std::path::Path::new("/p");
        let (h, f) = herramienta_como_comando(root, "cd web && ./pagina --ficha ficha.json --path . 2>&1").unwrap();
        assert_eq!((h.as_str(), f), ("pagina", Some(std::path::PathBuf::from("/p/web/ficha.json"))));
        assert_eq!(herramienta_como_comando(root, "gotv pagina datos/f.json").unwrap().1, Some(std::path::PathBuf::from("/p/datos/f.json")));
        assert_eq!(herramienta_como_comando(root, "web_search café").unwrap(), ("web_search".to_string(), None));
        assert!(herramienta_como_comando(root, "npm run build && ls pagina/").is_none());
        assert_eq!(herramienta_como_comando(root, "browser [\"open\", \"file:///x/index.html\"]").unwrap().0, "browser");
        // una página que pasó la revisión cuenta como lista; editarla la vuelve «no lista»
        let mut p = Guardia::default();
        p.pagina_lista(root.join("web"));
        assert!(p.es_pagina(&root.join("web")));
        p.app_paso(root, "edit", &serde_json::json!({"path": "web/index.html"}), true);
        for _ in 0..OCIOSO_MAX {
            assert!(p.app_ociosa().is_none(), "editada: ya no está lista");
        }
        p.pagina_lista(root.join("web"));
        for _ in 1..OCIOSO_MAX {
            assert!(p.app_ociosa().is_none());
        }
        assert_eq!(p.app_ociosa(), Some(root.join("web")));
    }
}

// D14: qué es un archivo de pruebas y qué es una aserción, en cada lenguaje (motor de programación)
pub use crate::pruebas::{aserciones_quitadas, es_archivo_de_pruebas};

#[cfg(test)]
mod d14 {
    use super::*;

    #[test]
    fn reconoce_archivos_de_pruebas() {
        for r in ["tests/test_biblioteca.py", "pkg/tests/x.py", "test_a.py", "a_test.go", "src/x.test.ts", "web/__tests__/a.js", "b.spec.js"] {
            assert!(es_archivo_de_pruebas(r), "{r}");
        }
        for r in ["biblioteca/almacen.py", "src/testigo.rs", "contest.py"] {
            assert!(!es_archivo_de_pruebas(r), "{r}");
        }
    }

    #[test]
    fn frena_doblar_una_prueba_con_rojo_y_deja_pasar_si_insiste() {
        let viejo = "        self.assertEqual(self.b.pagar(1, 400), 600)\n        self.assertEqual(self.b.pagar(1, 600), 0)\n";
        let nuevo = "        self.assertEqual(self.b.pagar(1, 400), 600)\n        self.assertEqual(self.b.pagar(1, 600), 1000)\n";
        let mut g = Guardia::nueva();
        // en verde: libre
        assert!(g.doblar_prueba("tests/test_b.py", viejo, nuevo).is_none());
        g.pruebas_en_rojo = true;
        // código, no pruebas: libre
        assert!(g.doblar_prueba("biblioteca/almacen.py", viejo, nuevo).is_none());
        // añadir aserciones: libre
        assert!(g.doblar_prueba("tests/test_b.py", viejo, &format!("{viejo}        self.assertTrue(x)\n")).is_none());
        // cambiar el valor esperado con rojo: frenado, con la línea
        let m = g.doblar_prueba("tests/test_b.py", viejo, nuevo).expect("frena");
        assert!(m.contains("pagar(1, 600), 0)"), "{m}");
        // la misma edición otra vez: pasa (insiste en que la prueba contradice la especificación)
        assert!(g.doblar_prueba("tests/test_b.py", viejo, nuevo).is_none());
        // borrar el segundo pago (v10): frenado
        let sin_segundo = "        self.assertEqual(self.b.pagar(1, 400), 600)\n";
        assert!(g.doblar_prueba("tests/test_b.py", viejo, sin_segundo).is_some());
    }
}
