//! GOD OF THE VOID — CLI.
//!   gotv "<tarea>"                 → agente sobre la carpeta actual.
//!   gotv --mode bypass "<tarea>"   → fuerza modo de permisos.
//!   gotv --perfil diseno "<tarea>" → perfil: auto | programar | diseno | imagen | video | musica (Ola D7, M4, M6, M7).
//!   gotv config                    → imprime la config efectiva (capas).
//!   gotv models                    → lista modelos disponibles.
//!   gotv tema '<json>' [--json|--muestra f.html] → tema calculado (Ola D2): CSS con contraste garantizado.
//!   gotv app <carpeta> [--marca N] [--direccion d] → aplicación React+TS con la biblioteca (D5).
//!   gotv iso <escena.json> [--salida f.svg]  → isométrica (D6).
//!   gotv pagina <ficha.json|--muestra dirección> [--suelto f.html | --salida carpeta] → página por composición (D3/D4).
//!   gotv --check <tool> <subject>  → prueba el motor de permisos (sin modelo).
//!   gotv --smoke                   → prueba de terreno (F0).
//!   gotv -p "<tarea>"              → sin nadie a quien preguntar; códigos 0/1/2/3 (F10.2).
//!   gotv programadas …             → tareas programadas y su agente de launchd (F2.3).
//! Config: ~/.config/gotv/config.toml ← .gotv/config.toml ← GOTV_* (ver README).

// F10.3 · `gotv acp` (editores) · F10.4 · `gotv plugin …`
mod acp;
mod conversar;
mod plugin_cmd;

/// F10.2/F10.3: con `-p` o `gotv acp` la salida estándar es sólo la respuesta o el protocolo: los avisos del
/// arranque del motor van a stderr.
static SOLO_STDOUT_LIMPIO: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Un aviso del arranque: a stdout como siempre, o a stderr si stdout es del protocolo o de la respuesta.
macro_rules! aviso_arranque {
    ($($t:tt)*) => {
        if SOLO_STDOUT_LIMPIO.load(std::sync::atomic::Ordering::Relaxed) { eprint!($($t)*) } else { print!($($t)*) }
    };
}
mod impreso;
mod programadas;

use anyhow::Result;
use gotv_core::tr;
use gotv_core::{Agent, Checkpoint, Config, Decision, Mode, Permissions, StdFrontend};
use gotv_engine::{ChatRequest, Engine, Message};
use std::io::Write;
use std::time::Instant;

fn args_for(tool: &str, subject: &str) -> serde_json::Value {
    match tool {
        "bash" => serde_json::json!({ "cmd": subject }),
        "glob" | "grep" => serde_json::json!({ "pattern": subject }),
        _ => serde_json::json!({ "path": subject }),
    }
}

fn perms_from(cfg: &Config, root: &std::path::Path) -> Permissions {
    let mut p = Permissions::new(
        Mode::parse(&cfg.mode),
        root,
        cfg.allow.clone(),
        cfg.deny.clone(),
        cfg.ask.clone(),
    );
    // C2: la caja de arena y los dominios extra de la config
    p.sandbox = cfg.sandbox;
    p.net_extra = cfg.net_allow.clone();
    p.sockets_extra = cfg.sockets_allow.clone();
    p.juez_auto = cfg.auto_juez.clone();
    p.credenciales_extra = cfg.credenciales_extra.clone();
    p.entorno_extra = cfg.entorno_allow.clone();
    p
}

#[tokio::main]
async fn main() -> Result<()> {
    // M1: Whisper nativo (sin Python) para los modelos que no oyen
    gotv_core::audio::registrar_transcriptor(gotv_mlx::transcribir_wav);
    // M11: el embebedor del RAG en el núcleo (sin el servidor de mlx_vlm)
    gotv_core::rag::registrar_incrustador(gotv_mlx::incrustar_textos);
    // M4: el motor de imagen (FLUX.2 klein) para la herramienta `generar_imagen`
    gotv_core::generadores::registrar_motor_imagen(gotv_mlx::imagen::generar_png);
    // M7: el motor de música (ACE-Step 1.5) para la herramienta `generar_musica`
    gotv_core::generadores::registrar_motor_musica(gotv_mlx::musica::generar_wav);
    gotv_core::generadores::registrar_motor_efecto(gotv_mlx::efectos::generar_wav);
    gotv_core::generadores::registrar_motor_separar(gotv_mlx::separador::separar_archivo);
    // M6: el motor de video (Wan 2.2 TI2V 5B) para la herramienta `generar_video`
    gotv_core::generadores::registrar_motor_video(gotv_mlx::video::generar_mp4);
    // como cualquier CLI de Unix: si quien lee la salida se va (`gotv … | head`), se termina en
    // silencio en vez de entrar en pánico con «Broken pipe» (medido en la prueba de Ola H)
    #[cfg(unix)]
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }
    gotv_desktop::run_if_requested(); // si soy un hijo de control del Mac, tampoco
    let raw: Vec<String> = std::env::args().skip(1).collect();
    // W3: Chrome me lanza como host de mensajería nativa (relevo extensión ⇄ GOTV)
    if raw.first().map(String::as_str) == Some("__chrome-host") {
        let _ = gotv_core::chrome::run_host();
        return Ok(());
    }

    // --mode <m> en cualquier posición (override de la config)
    let mut mode_override: Option<String> = None;
    let mut lang_override: Option<String> = None;
    let mut perfil_override: Option<String> = None;
    let mut images: Vec<String> = Vec::new();
    let mut audios: Vec<String> = Vec::new();
    let mut resume = false;
    let mut mac = false;
    let mut experto = false;
    // F10.2: `-p` (sin nadie a quien preguntar; códigos 0/1/2/3)
    let mut impreso = false;
    let mut rest: Vec<String> = Vec::new();
    let mut it = raw.iter();
    while let Some(a) = it.next() {
        if a == "-p" || a == "--print" {
            impreso = true;
        } else if a == "--mode" {
            mode_override = it.next().cloned();
        } else if a == "--audio" {
            audios.extend(it.next().cloned()); // S2: adjuntar audio a la orden
        } else if a == "--image" {
            images.extend(it.next().cloned()); // S1: adjuntar imagen a la orden
        } else if a == "--perfil" {
            perfil_override = it.next().cloned(); // D7: auto | programar | diseno
        } else if a == "--lang" {
            lang_override = it.next().cloned(); // es | en (P8)
        } else if a == "--resume" {
            resume = true; // continuar la última conversación de este proyecto
        } else if a == "--experto" || a == "--expert" {
            experto = true; // Ola EXPERTO: la orden va al chat del estudio Experto de este proyecto
        } else if a == "--mac" {
            mac = true; // O1: el check «Control Mac» de la app (sigue haciendo falta `desktop_control`)
        } else {
            rest.push(a.clone());
        }
    }

    let root = std::env::current_dir()?;
    let mut cfg = Config::load(&root);
    if let Some(l) = lang_override {
        gotv_core::i18n::set(gotv_core::i18n::resolve(&l));
        cfg.lang = l;
    }
    // Se muestran una vez aquí y se vacían, para que el arranque no los repita.
    for w in cfg.warnings.drain(..) {
        eprintln!("⚠ {w}");
    }
    // `--mode` manda; si no está, `GOTV_MODE` (el banco mide el modo automático con él: F14.2)
    if let Some(m) = mode_override.or_else(|| std::env::var("GOTV_MODE").ok().filter(|m| !m.trim().is_empty())) {
        cfg.mode = m;
    }
    if let Some(p) = perfil_override {
        cfg.perfil = p;
    }

    match rest.first().map(String::as_str) {
        // Medido 2026-11-09: `gotv --help` se tomaba como TAREA y encendía el modelo de 32B.
        Some("--help" | "-h" | "help") if rest.len() == 1 => {
            println!("{}", ayuda());
            return Ok(());
        }
        Some("--version" | "-V") if rest.len() == 1 => {
            println!("gotv {}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
        Some(b) if rest.len() == 1 && b.starts_with('-') => {
            eprintln!("{}", tr!("✗ opción desconocida «{b}». `gotv --help` lista las que hay.", "✗ unknown option «{b}». `gotv --help` lists them."));
            std::process::exit(2);
        }
        Some("--smoke") => return smoke(&mut cfg).await,
        Some("--dump-prompt") => {
            // diagnóstico: el prompt de sistema y los esquemas EXACTOS que ve el modelo
            let system = match gotv_core::memory::block(&root) {
                Some(m) => format!("{}{m}", gotv_core::agent::system()),
                None => gotv_core::agent::system().to_string(),
            };
            let tools = gotv_core::agent::visible_schemas(gotv_core::tools::schemas(), &Default::default());
            println!("{}", serde_json::json!({ "system": system, "tools": tools }));
            return Ok(());
        }
        Some("preflight") => {
            // Qué haría GOTV en ESTA Mac, sin encender el motor.
            match gotv_core::launch::prepare(&mut cfg) {
                Ok(l) => {
                    println!("🕳️  {}", l.hw.summary());
                    for n in &l.notes {
                        println!("· {n}");
                    }
                    let dir = l.card.as_ref().map(|c| c.path.clone()).unwrap_or_else(|| std::path::PathBuf::from(&cfg.model));
                    match gotv_mlx::NativoEngine::puede(&dir) {
                        Ok(()) => println!("{}", gotv_core::i18n::t("· motor: el núcleo Rust de GOTV (dentro del proceso)", "· engine: GOTV's Rust core (in-process)")),
                        Err(por) => {
                            println!("{}", tr!("✗ el núcleo aún no corre este modelo: {por}", "✗ the core does not run this model yet: {por}"));
                            std::process::exit(1);
                        }
                    }
                }
                Err(e) => {
                    println!("{}", tr!("✗ no arrancaría: {e}", "✗ would not start: {e}"));
                    std::process::exit(1);
                }
            }
            return Ok(());
        }
        Some("config") => {
            println!("{}", tr!("🕳️  GOD OF THE VOID — config efectiva\n{}", "🕳️  GOD OF THE VOID — effective config\n{}", cfg.summary()));
            return Ok(());
        }
        Some("tema") => return tema(&rest[1..]),
        Some("pagina") => return pagina(&rest[1..]),
        Some("iso") => return iso(&rest[1..]),
        Some("app") => return app_nueva(&rest[1..]),
        Some("api") => return api_nueva(&rest[1..]),
        Some("conexiones") => return conexiones_cmd(&root, &rest[1..]).await,
        Some("mcp") => return mcp_cmd(&root, &rest[1..], &cfg).await,
        Some("acp") => {
            SOLO_STDOUT_LIMPIO.store(true, std::sync::atomic::Ordering::Relaxed);
            return acp::correr(cfg, &rest[1..]).await;
        }
        Some("plugin" | "plugins") => return plugin_cmd::correr(&rest[1..]),
        Some("programadas" | "scheduled") => return programadas::cmd(&root, &rest[1..]).await,
        Some("estilo") => {
            // D10: lo que GOTV leería del diseño propio de esta carpeta
            let e = gotv_core::estilo::leer(&root);
            println!("{}", e.nota().unwrap_or_else(|| gotv_core::i18n::t("(sin diseño propio que respetar: GOTV puede proponer el suyo)", "(no own design to respect: GOTV can propose its own)").into()));
            return Ok(());
        }
        Some("hw") => {
            let hw = gotv_core::Hardware::detect();
            println!("🕳️  {}", hw.summary());
            if let Some(a) = gotv_core::hardware::available_now() {
                println!("{}", tr!("   libre ahora: {:.1} GB", "   free now: {:.1} GB", a as f64 / gotv_core::hardware::GIB as f64));
            }
            return Ok(());
        }
        Some("models") if rest.get(1).map(String::as_str) == Some("search") => return models_search(&rest[2..]).await,
        Some("models") if rest.get(1).map(String::as_str) == Some("import") => {
            // S3: un modelo propio de proyectos/ia → ficha de Llama servible
            let Some(src) = rest.get(2) else {
                anyhow::bail!("{}", gotv_core::i18n::t("uso: gotv models import <carpeta exportada> [nombre]", "usage: gotv models import <exported folder> [name]"));
            };
            let out = gotv_core::propios::import(&gotv_core::paths::resolve(&root, src), rest.get(3).map(String::as_str))?;
            let name = format!("propios/{}", out.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default());
            println!("{}", tr!("✅ importado en {} — chatea con `GOTV_MODEL={name} gotv \"hola\"` o `/model {name}`", "✅ imported at {} — chat with `GOTV_MODEL={name} gotv \"hello\"` or `/model {name}`", out.display()));
            return Ok(());
        }
        Some("models") if rest.get(1).map(String::as_str) == Some("pull") => return models_pull(rest.get(2).map(String::as_str).unwrap_or("")).await,
        Some("generadores" | "generators") => return generadores_cmd(&rest[1..]).await,
        Some("decir" | "say") => return decir_cmd(&rest[1..], &cfg),
        Some("imagen" | "image") => return imagen_cmd(&rest[1..]),
        Some("musica" | "música" | "music") => return musica_cmd(&rest[1..]), // juez-idioma: datos (nombre de opción o valor que se lee)
        Some("efecto" | "efectos" | "sfx") => return efecto_cmd(&rest[1..]),
        Some("separar" | "split") => return separar_cmd(&rest[1..]),
        Some("rehacer" | "redo") => return rehacer_cmd(&rest[1..]),
        Some("video" | "vídeo") => return video_cmd(&rest[1..]), // juez-idioma: datos (nombres del comando)
        Some("escuchar" | "listen") => return escuchar_cmd(&rest[1..]),
        Some("conversar" | "talk") => return conversar::conversar(&mut cfg, root.clone(), &rest[1..]).await,
        Some("models") => {
            let gib = gotv_core::hardware::GIB as f64;
            let hw = gotv_core::Hardware::detect();
            let avail = gotv_core::hardware::available_now();
            println!("🕳️  {}", hw.summary());
            if let Some(a) = avail {
                println!("{}", tr!("   libre ahora: {:.1} GB\n", "   free now: {:.1} GB\n", a as f64 / gib));
            }
            if gotv_core::i18n::is_en() {
                println!("{:<46} {:<20} {:>7} {:>8} {:>8}  fits THIS Mac?", "model", "kind", "weights", "max ctx", "KV/tok");
            } else {
                // juez-idioma: par (rama ES de is_en)
                println!("{:<46} {:<20} {:>7} {:>8} {:>8}  ¿cabe en ESTA Mac?", "modelo", "tipo", "pesos", "ctx máx", "KV/tok");
            }
            for c in gotv_core::discover(&cfg.model_dirs) {
                let f = gotv_core::fit(&c, &hw, avail, 16384u64.min(c.max_ctx));
                let verdict = if c.arch == "propia" {
                    gotv_core::i18n::t("— nuestro (se correrá en la ola S3)", "— ours (will run in wave S3)").to_string()
                } else if !c.runnable {
                    gotv_core::i18n::t("— no es un modelo de lenguaje", "— not a language model").to_string()
                } else if f.max_ctx >= 4096 {
                    tr!("✅ hasta {} tokens", "✅ up to {} tokens", f.max_ctx)
                } else {
                    format!("❌ {}", f.why)
                };
                println!(
                    "{:<46} {:<20} {:>6.1}G {:>8} {:>7}K  {verdict}",
                    c.name.chars().take(46).collect::<String>(),
                    c.kind(),
                    c.weights as f64 / gib,
                    c.max_ctx,
                    c.kv_per_token() / 1024
                );
            }
            return Ok(());
        }
        // F8.4 · las carpetas EXTRA del proyecto desde la terminal (lo mismo que la ventana; las elige el usuario,
        // nunca un repositorio): `gotv carpetas [listar | agregar <ruta> [--nombre N] | quitar <nombre>]`
        Some("carpetas" | "folders") => {
            let mut r = gotv_core::raices::Raices::de_proyecto(&root);
            match rest.get(1).map(String::as_str) {
                None | Some("listar" | "list") => {}
                Some("agregar" | "add") => {
                    let Some(ruta) = rest.get(2) else { anyhow::bail!("gotv carpetas agregar <ruta> [--nombre N]") };
                    let nombre = rest.iter().position(|a| a == "--nombre" || a == "--name").and_then(|i| rest.get(i + 1)).map(String::as_str);
                    println!("{}", r.agregar(std::path::Path::new(ruta), nombre)?);
                    r.guardar()?;
                }
                Some("quitar" | "remove") => {
                    let Some(n) = rest.get(2) else { anyhow::bail!("gotv carpetas quitar <nombre>") };
                    r.quitar(n.trim_start_matches('@'))?;
                    r.guardar()?;
                }
                Some(otro) => anyhow::bail!("{}", tr!("«{otro}»: listar · agregar <ruta> [--nombre N] · quitar <nombre>", "«{otro}»: list · add <path> [--name N] · remove <name>")),
            }
            println!("{}", tr!("📁 principal: {}", "📁 main: {}", r.principal.display()));
            for (n, p) in &r.extra {
                println!("📁 @{n}: {}", p.display());
            }
            return Ok(());
        }
        Some("rewind") => {
            let force = rest.iter().any(|a| a == "--force");
            // el último TURNO entero; `--paso` sólo el último paso
            let cp = Checkpoint::new(&root);
            let r = if rest.iter().any(|a| a == "--paso" || a == "--step") { cp.rewind_paso(force) } else { cp.rewind(force) }; // juez-idioma: datos (nombre de opción)
            match r {
                Ok(msg) => println!("{msg}"),
                Err(e) => {
                    eprintln!("{e}");
                    std::process::exit(1);
                }
            }
            return Ok(());
        }
        Some("sessions") => {
            let list = gotv_core::session::Session::list(&root)?;
            if list.is_empty() {
                println!("{}", gotv_core::i18n::t("(sin conversaciones guardadas en este proyecto)", "(no saved conversations in this project)"));
            }
            for s in list {
                println!("{}", tr!("#{:<4} {}  ({} mensajes)  {}", "#{:<4} {}  ({} messages)  {}", s.id, s.created, s.messages, s.title));
            }
            return Ok(());
        }
        Some("checkpoints") => {
            let steps = Checkpoint::new(&root).list();
            if steps.is_empty() {
                println!("{}", gotv_core::i18n::t("(sin pasos guardados)", "(no saved steps)"));
            }
            for (id, label, n) in steps.iter().rev() {
                println!("{}", tr!("#{id:<4} {label}  ({n} archivo(s))", "#{id:<4} {label}  ({n} file(s))"));
            }
            return Ok(());
        }
        Some("index") => {
            let rag = gotv_core::Rag::new(&root);
            let n = rag.build(&root)?;
            println!("{}", tr!("📚 índice BM25: {n} trozos en .gotv/indice", "📚 BM25 index: {n} chunks in .gotv/indice"));
            // S4: vectores si hay embedder (búsqueda semántica)
            if gotv_core::rag::embedder_dir().is_some() {
                let t = Instant::now();
                let mut e = gotv_core::rag::start_embedder().await?;
                let r = gotv_core::rag::index_vectors(&rag, &e).await;
                e.stop().await;
                println!("{}", tr!("🧭 vectores: {} trozos en {:.1} s (búsqueda semántica activa)", "🧭 vectors: {} chunks in {:.1} s (semantic search on)", r?, t.elapsed().as_secs_f32()));
            } else {
                println!("{}", gotv_core::i18n::t("   (sin vectores: `gotv setup rag` activa la búsqueda semántica)", "   (no vectors: `gotv setup rag` turns on semantic search)"));
            }
            return Ok(());
        }
        Some("search") => {
            let q = rest[1..].join(" ");
            let rag = gotv_core::Rag::new(&root);
            let hits = if rag.has_vectors() && gotv_core::rag::embedder_dir().is_some() {
                let mut e = gotv_core::rag::start_embedder().await?;
                let qv = e.embed(&[gotv_core::rag::query_text(&q)]).await;
                e.stop().await;
                rag.search_hybrid(&q, qv?.first().map(|v| v.as_slice()), 10)?
            } else {
                rag.search(&q, 10)?
            };
            if hits.is_empty() {
                println!("{}", gotv_core::i18n::t("(sin resultados)", "(no results)"));
            }
            for h in hits {
                println!("· {}:{}-{}\n    {}", h.path, h.start, h.end, h.snippet);
            }
            return Ok(());
        }
        Some("check") => {
            let r = gotv_core::tools::run_tool("diagnostics", &serde_json::json!({}), &root).await;
            println!("{}", r.output);
            return Ok(());
        }
        Some("--check") => return check(&cfg, &root, &rest[1..]),
        // desde 2026-11-12 el motor es el núcleo Rust: no hay recetas de servidor ni entorno que montar
        Some("tune") => {
            println!("{}", gotv_core::i18n::t("`gotv tune` medía recetas del servidor Python, que ya no existe: el núcleo Rust no las necesita.", "`gotv tune` measured recipes for the Python server, which no longer exists: the Rust core does not need them."));
            return Ok(());
        }
        Some("setup") if rest.len() == 1 || matches!(rest.get(1).map(String::as_str), Some("motor" | "vision")) => {
            println!("{}", gotv_core::i18n::t(
                "✅ No hay nada que instalar: el motor de GOTV es su núcleo Rust y viene dentro (texto, visión y oído).\nSiguiente: `gotv models search --speed rapido` para elegir un modelo que quepa en esta Mac y `gotv models pull <repo>`; voz, imagen, música y oído en Configuración → Generadores (o `gotv generadores`).",
                "✅ Nothing to install: GOTV's engine is its Rust core and it ships inside (text, vision and hearing).\nNext: `gotv models search --speed rapido` to pick a model that fits this Mac and `gotv models pull <repo>`; voice, image, music and hearing in Settings → Generators (or `gotv generadores`).",
            ));
            return Ok(());
        }
        Some("setup") if rest.get(1).map(String::as_str) == Some("audio") => return setup_audio(&cfg).await,
        Some("setup") if rest.get(1).map(String::as_str) == Some("rag") => {
            // S4: el embedder de la búsqueda semántica (0,3 GB). Desde M11 corre en el núcleo: no
            // pide el entorno de Python.
            let repo = gotv_core::rag::embed_repo();
            if gotv_core::rag::embedder_dir().is_none() {
                let progreso = |d: u64, t: u64| {
                    print!("\r⬇ {:>3} %   ", (d * 100).checked_div(t).unwrap_or(100));
                    std::io::stdout().flush().ok();
                };
                // el del catálogo, por el mismo camino que la app: exige la licencia aceptada
                if gotv_core::generadores::ficha(&repo).is_some() {
                    gotv_core::generadores::bajar(&repo, &progreso).await?;
                } else {
                    gotv_core::hub::pull(&repo, &progreso).await?;
                }
                println!();
            }
            println!("{}", tr!("✅ embedder listo ({repo}): corre `gotv index` para vectorizar el proyecto", "✅ embedder ready ({repo}): run `gotv index` to vectorize the project"));
            return Ok(());
        }
        Some("serve") => return serve_cmd(&mut cfg, &rest[1..]).await,
        Some("chrome") => return chrome_cmd(&rest[1..]),
        Some("skills") => {
            let all = gotv_core::skills::discover(&root);
            println!("{}", tr!("🕳️  {} skills (proyecto y usuario; gana la primera con el mismo nombre):\n", "🕳️  {} skills (project and user; the first with the same name wins):\n", all.len()));
            for k in &all {
                let d: String = k.description.chars().take(90).collect();
                println!("{:<26} {d}", k.name);
            }
            // `gotv skills <tarea>`: qué skills viajarían con esa orden
            if rest.len() > 1 {
                let q = rest[1..].join(" ");
                let r: Vec<&str> = gotv_core::skills::relevant(&q, &all, 5).iter().map(|k| k.name.as_str()).collect();
                println!("{}", tr!("\n· para «{q}» viajarían: {:?}", "\n· for «{q}» these would travel: {:?}", r));
            }
            return Ok(());
        }
        _ => {}
    }

    let task = if rest.is_empty() {
        gotv_core::i18n::t(
            "Crea un archivo saludo.txt con el texto 'Hola, Vacío.', muéstralo con `cat saludo.txt`, y termina.",
            "Create a file hello.txt with the text 'Hello, Void.', show it with `cat hello.txt`, and finish.",
        )
        .to_string()
    } else {
        rest.join(" ")
    };
    if impreso {
        SOLO_STDOUT_LIMPIO.store(true, std::sync::atomic::Ordering::Relaxed);
        return impreso::correr(&mut cfg, root, task, resume).await;
    }

    println!(
        "{}",
        tr!(
            "🕳️  GOD OF THE VOID · Dios del Vacío — agente (modo {})\n",
            "🕳️  GOD OF THE VOID — agent (mode {})\n",
            cfg.mode
        )
    );
    // A3: `proveedor:modelo` (p. ej. anthropic:claude-haiku-5-5) va por API, como en la GUI;
    // medido 2026-11-09: la CLI lo trataba como modelo local e intentaba cargarlo en MLX
    let engine: gotv_engine::AnyEngine = if gotv_core::nube::is_cloud(&cfg.model, &cfg) {
        let (e, nombre, notas) = gotv_core::nube::launch(&mut cfg).await?;
        for n in notas {
            println!("· {n}");
        }
        println!("{}", tr!("⏳ motor {nombre} … listo ✅ (por API)", "⏳ engine {nombre} … ready ✅ (via API)"));
        e
    } else {
        // el núcleo ve (M8/M8b) y oye (M1b, Gemma 4); lo que no entienda con este modelo va al servidor
        start_motor(&mut cfg, !images.is_empty(), !audios.is_empty()).await?
    };
    println!("{}", tr!("🗒️  tarea: {task}\n", "🗒️  task: {task}\n"));

    let perms = perms_from(&cfg, &root);
    let checkpoint = Some(Checkpoint::new(&root));
    // Sesión: nueva, o la última de este proyecto con --resume (P4).
    let (session, history) = match resume
        .then(|| gotv_core::session::Session::resume_last(&root, None))
        .transpose()?
        .flatten()
    {
        Some((s, h)) => {
            println!("{}", tr!("· retomando la sesión #{} ({} mensajes)", "· resuming session #{} ({} messages)", s.id, h.len()));
            (Some(s), h)
        }
        None => {
            if resume {
                println!("{}", gotv_core::i18n::t("· no había sesión previa en este proyecto: empiezo una nueva", "· no previous session in this project: starting a new one"));
            }
            (gotv_core::session::Session::open_new(&root, &cfg.model).ok(), vec![])
        }
    };
    // P5: primer Ctrl+C interrumpe el turno; el segundo sale. El motor y los
    // `bash` viven en su propio grupo: la señal de la terminal no los toca.
    let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    // C1: un SIGTERM (kill, cierre de la terminal) tampoco deja procesos huérfanos
    tokio::spawn(async {
        if let Ok(mut t) = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            t.recv().await;
            gotv_core::exec::kill_all();
            gotv_core::browser::close_all();
            std::process::exit(143);
        }
    });
    {
        let flag = cancel.clone();
        tokio::spawn(async move {
            while tokio::signal::ctrl_c().await.is_ok() {
                if flag.swap(true, std::sync::atomic::Ordering::Relaxed) {
                    eprintln!("{}", gotv_core::i18n::t("\n⏹ salgo", "\n⏹ exiting"));
                    gotv_core::exec::kill_all();
                    gotv_core::browser::close_all();
                    std::process::exit(130);
                }
                eprintln!("{}", gotv_core::i18n::t("\n⏹ interrumpiendo el turno… (Ctrl+C otra vez para salir)", "\n⏹ interrupting the turn… (Ctrl+C again to exit)"));
            }
        });
    }
    let mut agent = armar_agente(&cfg, root, engine, StdFrontend::default(), perms, checkpoint, history, session, cancel);
    // como la app: el permiso global viene de la config (Seguridad) y el check, de `--mac`
    agent.perms.desktop_control = cfg.desktop_control;
    agent.perms.desktop_turn = mac;
    if mac && !cfg.desktop_control {
        eprintln!("{}", gotv_core::i18n::t("⚠ --mac sin `desktop_control = true` en la config (Seguridad): el control del Mac sigue apagado", "⚠ --mac without `desktop_control = true` in the config (Security): Mac control stays off"));
    }

    for img in &images {
        let p = gotv_core::paths::resolve(&agent.root, img);
        match gotv_core::images::data_url(&p) {
            Ok((url, n)) => {
                println!("{}", tr!("🖼  adjunto {} ({} KB)", "🖼  attached {} ({} KB)", p.display(), n / 1024));
                agent.user_media.push((img.clone(), url));
            }
            Err(e) => anyhow::bail!("{e}"),
        }
    }
    let mut task = task;
    for a in &audios {
        let p = gotv_core::paths::resolve(&agent.root, a);
        if agent.engine.capabilities().audio_in {
            let (url, secs) = gotv_core::audio::data_url(&p)?;
            println!("{}", tr!("🎧 adjunto {} ({secs:.1} s, nativo)", "🎧 attached {} ({secs:.1} s, native)", p.display()));
            agent.user_media.push((a.clone(), url));
        } else {
            // respaldo NO nativo, declarado
            println!("{}", tr!("🎧 {}: este modelo no oye → Whisper transcribe (NO nativo)…", "🎧 {}: this model cannot hear → Whisper transcribes (NOT native)…", p.display()));
            let t = gotv_core::audio::transcribe(&p)?;
            task = format!("{task}\n\n{}", tr!("[transcripción de {} con Whisper — NO nativa]\n{t}", "[transcript of {} by Whisper — NOT native]\n{t}", p.display()));
        }
    }
    // como la app al cambiar de modelo: una conversación retomada se prepara ANTES de la orden
    if resume {
        let tp = Instant::now();
        if agent.preparar_conversacion().await {
            println!("{}", tr!("· conversación preparada para este modelo ({:.0} s)", "· conversation prepared for this model ({:.0} s)", tp.elapsed().as_secs_f32()));
        }
    }
    let t = Instant::now();
    if experto {
        let r = agent.experto_orden(&task).await?;
        for h in r["hechas"].as_array().into_iter().flatten() {
            println!("✓ {}", h.as_str().unwrap_or(""));
        }
        for q in r["preguntas"].as_array().into_iter().flatten() {
            println!("? {}", q.as_str().unwrap_or(""));
        }
        for e in r["errores"].as_array().into_iter().flatten() {
            println!("✗ {}", e.as_str().unwrap_or(""));
        }
        if let Some(t) = r["texto"].as_str().filter(|t| !t.is_empty()) {
            println!("🧠 {t}");
        }
        println!("{}", tr!("⏱ {:.1} s", "⏱ {:.1} s", t.elapsed().as_secs_f32()));
        return Ok(());
    }
    // F12 · formas de trabajar (`gotv -p "/objetivo todas las pruebas pasan"`): nadie escribe mientras tanto
    let answer = match gotv_core::formas::orden(&mut agent, &task, || false).await {
        Some(r) => r?,
        None => agent.run(&task).await?,
    };
    if answer == gotv_core::agent::interrupted() {
        println!("{}", tr!("\n{answer} (la sesión quedó guardada: `gotv --resume` para seguir)", "\n{answer} (the session was saved: `gotv --resume` to continue)"));
    } else if agent.front.ya_visto(&answer) {
        println!(); // la respuesta ya se vio en vivo
    } else {
        println!("\n🧠 {answer}");
    }
    match agent.front.last_stats() {
        Some(st) => println!("— {:.1} s · {}", t.elapsed().as_secs_f32(), st.summary()),
        None => println!("— {:.1} s", t.elapsed().as_secs_f32()),
    }
    // F2.2 · lo programado en esta orden («avísame en 2 minutos si…») vive lo que vive el proceso: la terminal
    // se queda esperando y lo corre a su hora como una orden nueva; sale cuando no queda nada (o con Ctrl+C,
    // que lo pierde). Antes la herramienta se ofrecía aquí y lo programado no corría nunca (medido 2026-11-17).
    // F3 · lo mismo con las vigilancias: mientras alguna viva (vencen solas, 30 min como mucho) la terminal
    // espera, y lo que vean entra como orden nueva (el error que aparece en un registro se arregla solo)
    use gotv_core::programar::chat as agenda;
    let dueno = agent.proc_owner();
    let mut avisada = (0i64, 0usize);
    loop {
        let prox = agenda::lista(&agent).iter().map(|p| p.proxima).min();
        let vigias = gotv_core::vigilar::activas(&dueno);
        if prox.is_none() && vigias == 0 && !gotv_core::vigilar::hay_pendientes(&dueno) {
            break;
        }
        if (prox.unwrap_or(0), vigias) != avisada {
            avisada = (prox.unwrap_or(0), vigias);
            if let Some(p) = prox {
                println!("{}", tr!(
                    "⏰ {} programada(s) en esta orden: la próxima a las {}. Espero aquí (Ctrl+C la cancela).",
                    "⏰ {} scheduled in this order: the next at {}. Waiting here (Ctrl+C cancels it).",
                    agenda::lista(&agent).len(),
                    gotv_core::programar::hora_local(Some(p))
                ));
            }
            if vigias > 0 {
                println!("{}", tr!("👁 {vigias} vigilancia(s) activa(s): espero lo que vean (Ctrl+C sale).", "👁 {vigias} active watch(es): waiting for what they see (Ctrl+C quits)."));
            }
        }
        let falta = prox.map(|p| (p - agenda::ahora()).max(0) as u64).unwrap_or(1);
        tokio::time::sleep(std::time::Duration::from_secs(falta.clamp(0, 1))).await;
        if let Some(orden) = agenda::turno_pendiente(&mut agent, agenda::ahora()) {
            println!("\n{}", orden.lines().next().unwrap_or(""));
            let t = Instant::now();
            let r = agent.run(&orden).await?;
            if !agent.front.ya_visto(&r) {
                println!("\n🧠 {r}");
            }
            println!("— {:.1} s", t.elapsed().as_secs_f32());
        }
    }
    agent.cerrar_sesion("other").await; // F10.1: SessionEnd
    agent.engine.stop().await?;
    Ok(())
}

/// W3: `gotv chrome install` — puente local con la extensión «GOD OF THE VOID» de Chrome.
/// El agente con la configuración de la CLI (la tarea normal y `gotv conversar` lo comparten).
#[allow(clippy::too_many_arguments)]
fn armar_agente<E: Engine, F: gotv_core::Frontend>(
    cfg: &Config,
    root: std::path::PathBuf,
    engine: E,
    front: F,
    perms: Permissions,
    checkpoint: Option<Checkpoint>,
    history: Vec<Message>,
    session: Option<gotv_core::session::Session>,
    cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
) -> Agent<E, F> {
    Agent {
        engine,
        root,
        max_steps: cfg.max_steps,
        max_tokens: cfg.max_tokens,
        sampling: cfg.sampling,
        context_budget: cfg.context_budget,
        perms,
        checkpoint,
        front,
        history,
        session,
        cancel,
        mcp: None,
        attachments: Vec::new(),
        user_media: Vec::new(),
        chat_only: cfg.chat_only,
        embedder: None,
        lsp: None,
        computer_use: cfg.computer_use,
        procs: Default::default(),
        tools_loaded: Default::default(),
        bases_nube: Default::default(),
        guardia: Default::default(),
        perfil: gotv_core::perfil::Perfil::parse(&cfg.perfil),
        pro: Default::default(),
        motor_dormido: false,
        visible: None,
        estado_de: None,
        calibre: None,
        recorto_ubicaciones: false,
        tope_paso: 0,
    }
}

fn chrome_cmd(rest: &[String]) -> Result<()> {
    match rest.first().map(String::as_str) {
        Some("install") => {
            let me = std::env::current_exe()?;
            let f = gotv_core::chrome::install_host(&gotv_core::chrome::user_nm_dir(), &me, &[])?;
            let ext = gotv_core::chrome::carpeta_extension();
            println!("{}", gotv_core::tr!(
                "✓ puente instalado: {}\n\nFalta cargar la extensión en TU Chrome (una vez):\n  1. abre chrome://extensions\n  2. activa «Modo de desarrollador» (arriba a la derecha)\n  3. «Cargar descomprimida» → elige {}\n\nGOD OF THE VOID sólo controla las pestañas del grupo «GOTV»; bancos, pagos, correo y cuentas quedan bloqueados, y jamás escribe contraseñas.",
                "✓ bridge installed: {}\n\nNow load the extension in YOUR Chrome (once):\n  1. open chrome://extensions\n  2. turn on «Developer mode» (top right)\n  3. «Load unpacked» → pick {}\n\nGOD OF THE VOID only controls the tabs in the «GOTV» group; banks, payments, mail and accounts stay blocked, and it never types passwords.",
                f.display(),
                ext.canonicalize().unwrap_or(ext).display()
            ));
            Ok(())
        }
        Some("uninstall") => {
            let d = gotv_core::chrome::user_nm_dir();
            let _ = std::fs::remove_file(d.join(format!("{}.json", gotv_core::chrome::HOST_NAME)));
            let _ = std::fs::remove_file(d.join("gotv-chrome-host.sh"));
            println!("{}", gotv_core::i18n::t("✓ puente quitado (la extensión se quita en chrome://extensions)", "✓ bridge removed (remove the extension in chrome://extensions)"));
            Ok(())
        }
        _ => {
            eprintln!("{}", gotv_core::i18n::t("uso: gotv chrome install | uninstall", "usage: gotv chrome install | uninstall"));
            std::process::exit(2);
        }
    }
}

fn check(cfg: &Config, root: &std::path::Path, rest: &[String]) -> Result<()> {
    if rest.len() < 2 {
        eprintln!("{}", gotv_core::i18n::t("uso: gotv [--mode <m>] --check <tool> <subject...>", "usage: gotv [--mode <m>] --check <tool> <subject...>"));
        std::process::exit(2);
    }
    let tool = &rest[0];
    let subject = rest[1..].join(" ");
    let args = args_for(tool, &subject);
    let perms = perms_from(cfg, root);
    let verdict = match perms.check(tool, &args) {
        Decision::Allow => "ALLOW ✅".to_string(),
        Decision::Ask(d) => format!("ASK ❓ ({d})"),
        Decision::Deny(w) => format!("DENY 🚫 ({w})"),
    };
    println!("[{}] {tool} «{subject}» → {verdict}", cfg.mode);
    Ok(())
}

/// Arranque único: resolver máquina/modelo/contexto (E7), avisar, encender el NÚCLEO (Rust + MLX
/// dentro del proceso) y recordar el modelo. Sin Python desde 2026-11-12: lo que el núcleo aún no
/// corre se dice con claridad (no hay servidor de respaldo). Un modelo que no ve o no oye EN el
/// núcleo trabaja sin el adjunto (el audio le llega como transcripción de Whisper).
async fn start_motor(cfg: &mut Config, con_imagenes: bool, con_audio: bool) -> Result<gotv_engine::AnyEngine> {
    let launch = gotv_core::launch::prepare(cfg)?;
    let dir = launch.card.as_ref().map(|c| c.path.clone()).unwrap_or_else(|| std::path::PathBuf::from(&cfg.model));
    gotv_mlx::NativoEngine::puede(&dir).map_err(|por| anyhow::anyhow!("{}", tr!("el núcleo de GOTV aún no corre este modelo: {por}", "GOTV's core does not run this model yet: {por}")))?;
    for n in &launch.notes {
        aviso_arranque!("· {n}\n");
    }
    let (ve, oye) = launch.card.as_ref().map(|c| (c.vision, c.audio)).unwrap_or((false, false));
    if con_imagenes && ve && !gotv_mlx::NativoEngine::ve(&dir) {
        aviso_arranque!("{}\n", tr!("· este modelo trae vista, pero el núcleo aún no se la da con esta familia (sí Qwen3.5 y Gemma 4): va sin la imagen", "· this model has vision, but the core does not give it to this family yet (Qwen3.5 and Gemma 4 do): it goes without the image"));
    }
    if con_audio && oye && !gotv_mlx::NativoEngine::oye(&dir) {
        aviso_arranque!("{}\n", tr!("· este modelo oye, pero el núcleo aún no le da el oído con esta familia (sí Gemma 4): le llega la transcripción", "· this model hears, but the core does not give it hearing with this family yet (Gemma 4 does): it gets the transcript"));
    }
    let mut e = gotv_mlx::NativoEngine::new(&dir);
    aviso_arranque!("{}", tr!("⏳ motor {} … ", "⏳ engine {} … ", e.name()));
    std::io::stdout().flush().ok();
    let t0 = Instant::now();
    e.start().await.map_err(|por| anyhow::anyhow!("{}", tr!("✗ el núcleo no arrancó: {por}", "✗ the core did not start: {por}")))?;
    aviso_arranque!("{}\n", tr!("vivo ✅ (dentro del proceso · {:.1} s)", "up ✅ (in-process · {:.1} s)", t0.elapsed().as_secs_f32()));
    gotv_core::launch::remember_model(&cfg.model);
    Ok(gotv_engine::AnyEngine::Otro(Box::new(e)))
}

async fn smoke(cfg: &mut Config) -> Result<()> {
    println!("{}", gotv_core::i18n::t("🕳️  GOD OF THE VOID — prueba de terreno\n", "🕳️  GOD OF THE VOID — smoke test\n"));
    let mut engine = start_motor(cfg, false, false).await?;
    let req = ChatRequest {
        messages: vec![
            Message::system(gotv_core::i18n::t("Eres GOD OF THE VOID. Directo y breve. /no_think", "You are GOD OF THE VOID. Direct and brief. /no_think")),
            Message::user(gotv_core::i18n::t("Preséntate en una frase. /no_think", "Introduce yourself in one sentence. /no_think")),
        ],
        max_tokens: 128,
        sampling: cfg.sampling,
        tools: None,
        conversacion: None,
        herramientas_aparte: None,
    };
    let t = Instant::now();
    let r = engine.complete(req).await?;
    let secs = t.elapsed().as_secs_f32();
    println!(
        "\n🧠 {}\n— {} tokens · {:.1} tok/s",
        r.content.trim(),
        r.tokens,
        r.tokens as f32 / secs.max(0.001)
    );
    engine.stop().await?;
    Ok(())
}

/// `gotv models search <texto> [--mem <GB>] [--org <org>]` (P12): el catálogo de
/// Hugging Face filtrado por lo que cabe en ESTA Mac — o en otra: `--mem 512`
/// simula una Mac de 512 GB (presupuesto de GPU de fábrica: ¾ de la memoria).
async fn models_search(args: &[String]) -> Result<()> {
    use gotv_core::biblioteca as bib;
    let mut query = Vec::new();
    let (mut mem, mut mac, mut org, mut sort, mut speed, mut all) = (None::<u64>, None::<String>, String::new(), "downloads".to_string(), String::new(), false);
    let mut as_json = false;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--mem" => mem = it.next().and_then(|v| v.trim_end_matches("GB").parse().ok()),
            "--mac" => mac = it.next().cloned(),
            "--org" => org = it.next().cloned().unwrap_or_default(),
            "--sort" => sort = it.next().cloned().unwrap_or_default(),
            "--speed" => speed = it.next().cloned().unwrap_or_default(),
            "--all" => all = true,
            "--json" => as_json = true,
            _ => query.push(a.clone()),
        }
    }
    let q = query.join(" ");
    // la Mac contra la que se mide: esta, `--mac "M4 Pro|48"`, o `--mem 512` (chip de esta Mac)
    let this = bib::MacProfile::this_mac(&gotv_core::Hardware::detect());
    let profile = match (mac, mem) {
        (Some(id), _) => bib::MacProfile::simulated(&id).ok_or_else(|| anyhow::anyhow!("{}", tr!("--mac «{id}»: usa la forma \"M4 Pro|48\"", "--mac «{id}»: use the form \"M4 Pro|48\"")))?,
        (None, Some(gb)) => bib::MacProfile::simulated(&format!("{}|{gb}", this.device.trim_start_matches("Apple "))).unwrap_or(this),
        _ => this,
    };
    if as_json {
        let installed = bib::installed_set(&[]);
        let found = bib::search(&q, &org, &sort, 100, &profile, &installed).await?;
        println!("{}", serde_json::json!({"mac": profile, "items": found}));
        return Ok(());
    }
    println!("🕳️  {} · {:.0} GB/s{}", profile.label, profile.bandwidth, if profile.exact { "" } else { " (≈)" });
    println!("{}", tr!("· buscando «{q}» en Hugging Face…\n", "· searching «{q}» on Hugging Face…\n"));
    let installed = bib::installed_set(&[]);
    let found = bib::search(&q, &org, &sort, 60, &profile, &installed).await?;
    let shown: Vec<_> = found.iter().filter(|e| (all || e.fits) && (speed.is_empty() || e.speed == speed)).take(25).collect();
    if shown.is_empty() {
        println!("{}", gotv_core::i18n::t("(sin resultados que quepan; --all muestra también los que no)", "(no results that fit; --all also shows the ones that don't)"));
        return Ok(());
    }
    let tag = |s: &str| match s {
        "muy-rapido" => gotv_core::i18n::t("muy rápido", "very fast"),
        "rapido" => gotv_core::i18n::t("rápido", "fast"),
        "medio" => gotv_core::i18n::t("medio", "medium"),
        "lento" => gotv_core::i18n::t("lento", "slow"),
        _ => gotv_core::i18n::t("muy lento", "very slow"),
    };
    for e in shown {
        let mark = if e.installed { "●" } else { " " };
        let size = match e.active {
            Some(a) => format!("{:.0}B·{a:.0}B act", e.params),
            None => format!("{:.1}B", e.params),
        };
        let fit = if !e.fits { "❌" } else if e.tight { "⚠" } else { "✅" };
        println!(
            "{mark} {:<50} {fit} {:>5.1} GB  {:<13} {:>4.0} tok/s {:<10} {}",
            e.repo.chars().take(50).collect::<String>(),
            e.bytes as f64 / 1e9,
            size,
            e.tok_s,
            tag(e.speed),
            if e.vision { "👁" } else { "" }
        );
    }
    println!("{}", gotv_core::i18n::t(
        "\n● instalado · ✅ cabe · ⚠ cabe justo · ❌ no cabe · tok/s estimados por el ancho de banda del chip\nbajar: gotv models pull <repo> · filtrar: --speed rapido · otra Mac: --mac \"M4 Pro|48\"",
        "\n● installed · ✅ fits · ⚠ tight · ❌ doesn't fit · tok/s estimated from the chip's memory bandwidth\ndownload: gotv models pull <repo> · filter: --speed rapido · another Mac: --mac \"M4 Pro|48\""
    ));
    Ok(())
}

/// `gotv serve [--lan | --host H] [--port N] [--no-cloud]` (A4): GOTV como servicio,
/// compatible con OpenAI y con Ollama. Si la app (u otro `gotv`) ya tiene un motor
/// vivo en el puerto de la config, lo COMPARTE en vez de cargar otro modelo.
async fn serve_cmd(cfg: &mut Config, args: &[String]) -> Result<()> {
    use gotv_core::serve;
    let (mut host, mut port, mut cloud) = ("127.0.0.1".to_string(), serve::DEFAULT_PORT, true);
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--lan" => host = "0.0.0.0".into(),
            "--host" => host = it.next().cloned().unwrap_or(host),
            "--port" => port = it.next().and_then(|p| p.parse().ok()).unwrap_or(port),
            "--no-cloud" => cloud = false,
            other => anyhow::bail!("{}", tr!("opción desconocida: {other} (usa --lan, --host, --port, --no-cloud)", "unknown option: {other} (use --lan, --host, --port, --no-cloud)")),
        }
    }
    // fuera de este equipo, SIEMPRE con llave (la del Llavero; se crea la primera vez)
    let key = if serve::is_loopback(&host) {
        None
    } else {
        if std::env::var("GOTV_SERVE_KEY").is_err() {
            println!("{}", gotv_core::i18n::t("🔑 leyendo la llave del Llavero (macOS puede pedir permiso la primera vez)…", "🔑 reading the key from the Keychain (macOS may ask for permission the first time)…"));
        }
        serve::stored_key(true)?
    };
    serve::check_bind(&host, key.as_deref())?;
    let shared = serve::Shared::new(key.clone());
    // el modelo local: el NÚCLEO en este proceso (sin servidor aparte); se sirve por su asa
    let mut own = start_motor(cfg, false, false).await?;
    {
        let card = gotv_core::discover(&cfg.model_dirs).into_iter().find(|c| c.path.to_string_lossy() == cfg.model);
        let name = serve::public_name(&cfg.model);
        let asa = own.compartir().ok_or_else(|| anyhow::anyhow!("{}", gotv_core::i18n::t("el núcleo no se pudo compartir", "the core could not be shared")))?;
        shared.set_local(Some(serve::local_upstream(&name, &cfg.model, asa, own.capabilities().vision, card.map_or(0, |c| c.weights))));
    }
    if cloud {
        let ups = serve::cloud_upstreams(cfg).await;
        if !ups.is_empty() {
            println!("{}", tr!("· más {} modelos de nube (con tus llaves)", "· plus {} cloud models (with your keys)", ups.len()));
        }
        shared.set_cloud(ups);
    }
    let listener = serve::bind(&host, port).await?;
    let addr = listener.local_addr()?;
    let shown: Vec<String> = if serve::is_loopback(&host) {
        vec![format!("http://127.0.0.1:{}", addr.port())]
    } else {
        let mut v: Vec<String> = serve::lan_addresses().into_iter().map(|ip| format!("http://{ip}:{}", addr.port())).collect();
        v.push(format!("http://127.0.0.1:{}", addr.port()));
        v
    };
    println!("\n🕳️  {}", tr!("GOD OF THE VOID sirve en:", "GOD OF THE VOID is serving at:"));
    for u in &shown {
        println!("   {u}");
    }
    let base = &shown[0];
    println!("\n   OpenAI  · {base}/v1   (GET /v1/models · POST /v1/chat/completions)");
    println!("   Ollama  · {base}      (GET /api/tags · POST /api/chat · POST /api/generate)");
    if let Some(k) = &key {
        println!("\n🔑 {}", tr!("llave (Authorization: Bearer …): {k}", "key (Authorization: Bearer …): {k}"));
    } else {
        println!("\n🔒 {}", gotv_core::i18n::t("sólo este equipo (127.0.0.1): sin llave. Para la red: gotv serve --lan", "this machine only (127.0.0.1): no key. For the network: gotv serve --lan"));
    }
    let auth = key.as_ref().map(|_| " -H \"Authorization: Bearer $GOTV_KEY\"").unwrap_or("");
    println!("\n   curl{auth} {base}/v1/chat/completions -H 'content-type: application/json' \\\n     -d '{{\"model\":\"default\",\"messages\":[{{\"role\":\"user\",\"content\":\"hola\"}}]}}'");
    println!("\n{}", gotv_core::i18n::t("Ctrl+C para parar.", "Ctrl+C to stop."));
    let (tx, rx) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(serve::run(listener, shared.clone(), rx));
    tokio::signal::ctrl_c().await.ok();
    let _ = tx.send(());
    let _ = server.await;
    shared.set_local(None);
    let _ = own.stop().await;
    println!("{}", tr!("· servicio parado ({} peticiones atendidas)", "· service stopped ({} requests served)", shared.hits.load(std::sync::atomic::Ordering::Relaxed)));
    Ok(())
}

/// `gotv models pull <org/nombre>` (P12): comprueba que quepa ANTES de bajar, baja
/// con reanudación y lo deja listo para `gotv` / `/model`.
async fn models_pull(repo: &str) -> Result<()> {
    if !gotv_core::hub::valid_repo(repo) {
        anyhow::bail!("{}", gotv_core::i18n::t("uso: gotv models pull <org/nombre>  (p. ej. mlx-community/Qwen3-8B-4bit)", "usage: gotv models pull <org/name>  (e.g. mlx-community/Qwen3-8B-4bit)"));
    }
    let gib = gotv_core::hardware::GIB as f64;
    let (card, _) = gotv_core::hub::inspect(repo).await?;
    let hw = gotv_core::Hardware::detect();
    let fit = gotv_core::fit(&card, &hw, None, 4096u64.min(card.max_ctx));
    println!("{}", tr!("· {repo}: {:.1} GB · {}", "· {repo}: {:.1} GB · {}", card.weights as f64 / gib, card.kind()));
    if !fit.ok {
        // con la Mac libre no cabría ni con 4k de contexto: no tiene sentido bajarlo
        anyhow::bail!("{}", tr!("no lo bajo: {}", "not downloading it: {}", fit.why));
    }
    let t = Instant::now();
    let dest = gotv_core::hub::pull(repo, &|done, total| {
        let pct = (done * 100).checked_div(total).unwrap_or(100);
        let rate = done as f64 / 1e6 / t.elapsed().as_secs_f64().max(0.1);
        print!("\r{}", tr!("⬇ {pct:>3} %  {:.2} de {:.2} GB  ({rate:.0} MB/s)   ", "⬇ {pct:>3} %  {:.2} of {:.2} GB  ({rate:.0} MB/s)   ", done as f64 / 1e9, total as f64 / 1e9));
        std::io::stdout().flush().ok();
    })
    .await?;
    println!("\n{}", tr!("✅ listo en {} — úsalo con `GOTV_MODEL={repo} gotv …` o `/model` en la TUI", "✅ ready at {} — use it with `GOTV_MODEL={repo} gotv …` or `/model` in the TUI", dest.display()));
    Ok(())
}

/// Lee una línea sin eco (el token no se ve ni queda en el historial de la terminal).
fn leer_oculto() -> Result<String> {
    use std::io::BufRead;
    // SAFETY: termios de la entrada estándar; se restaura el modo original antes de salir.
    let mut original: libc::termios = unsafe { std::mem::zeroed() };
    let es_tty = unsafe { libc::isatty(0) == 1 && libc::tcgetattr(0, &mut original) == 0 };
    if es_tty {
        let mut sin_eco = original;
        sin_eco.c_lflag &= !libc::ECHO;
        unsafe { libc::tcsetattr(0, libc::TCSANOW, &sin_eco) };
    }
    let mut linea = String::new();
    let r = std::io::stdin().lock().read_line(&mut linea);
    if es_tty {
        unsafe { libc::tcsetattr(0, libc::TCSANOW, &original) };
        println!();
    }
    r?;
    Ok(linea.trim().to_string())
}

/// `gotv generadores …` (M0): el catálogo de generadores con sus licencias.
/// `gotv escuchar [--archivo x.wav]` (M3): micrófono (o un archivo de audio: WAV de cualquier frecuencia, MP3, FLAC, OGG, M4A; se remuestrea a 16 kHz en Rust) → VAD Silero v6 → turnos
/// (700 ms de silencio cierran, medido con el Portador) → Whisper nativo. Todo en Rust, sin Python.
fn escuchar_cmd(args: &[String]) -> Result<()> {
    use gotv_sonido::vad::{Evento, Turnos, Vad, VENTANA};
    let archivo = args.iter().position(|a| a == "--archivo" || a == "--file").and_then(|i| args.get(i + 1)).map(std::path::PathBuf::from); // juez-idioma: datos (nombre de opción o valor que se lee)
    let mut vad = Vad::nuevo()?;
    let mut turnos = Turnos::por_defecto();
    let t0 = Instant::now();
    let mut n_frases = 0usize;
    let mut atender = |ev: Evento| -> Result<()> {
        match ev {
            Evento::Inicio(m) => println!("{}", gotv_core::tr!("🎙  voz a los {:.2} s", "🎙  voice at {:.2} s", m as f64 / 16000.0)),
            Evento::Corto(d) => println!("·  {:.2} s: muy corto, ruido", d),
            Evento::Fin { audio, desde, hasta } => {
                n_frases += 1;
                let tmp = std::env::temp_dir().join(format!("gotv-escuchar-{}-{n_frases}.wav", std::process::id()));
                std::fs::write(&tmp, gotv_mlx::wav_pcm16(&audio, 16000))?;
                let t = Instant::now();
                let texto = gotv_core::audio::transcribe(&tmp);
                let _ = std::fs::remove_file(&tmp);
                println!("📝 [{:.2}–{:.2} s] {} ({:.2} s de Whisper)", desde as f64 / 16000.0, hasta as f64 / 16000.0, texto.unwrap_or_else(|e| format!("✗ {e}")), t.elapsed().as_secs_f64());
            }
        }
        Ok(())
    };
    if let Some(f) = archivo {
        // cualquier audio (WAV de cualquier frecuencia, MP3, FLAC, OGG, M4A) → mono a 16 kHz, en Rust
        let (canales, de) = gotv_mlx::musica_fuente::leer(&f)?;
        let a = gotv_mlx::musica_fuente::mono(&canales, de, 16000);
        for c in a.as_chunks::<VENTANA>().0 {
            let p = vad.prob(c);
            if let Some(ev) = turnos.paso(c, p) {
                atender(ev)?;
            }
        }
        println!("{}", tr!("✓ {} s de audio en {:.1} s", "✓ {} s of audio in {:.1} s", a.len() / 16000, t0.elapsed().as_secs_f64()));
        return Ok(());
    }
    let mic = gotv_sonido::Microfono::abrir(16000, VENTANA)?;
    println!("{}", gotv_core::i18n::t("🎧 escuchando (Ctrl-C para salir): habla y haz una pausa", "🎧 listening (Ctrl-C to quit): speak and pause"));
    let mut resto: Vec<f32> = Vec::new();
    loop {
        let Some(b) = mic.leer(std::time::Duration::from_secs(2)) else {
            anyhow::bail!("{}", gotv_core::i18n::t("el micrófono no entrega audio (¿permiso de micrófono para esta terminal en Privacidad y seguridad?)", "the microphone delivers no audio (microphone permission for this terminal in Privacy & Security?)"));
        };
        resto.extend(b);
        while resto.len() >= VENTANA {
            let c: Vec<f32> = resto.drain(..VENTANA).collect();
            let p = vad.prob(&c);
            if let Some(ev) = turnos.paso(&c, p) {
                atender(ev)?;
            }
        }
    }
}

/// `gotv decir "texto" [--salida x.wav] [--voraz] [--idioma spanish|english|…] [--mudo]`
/// M7c: `gotv efecto "descripción en inglés" [--segundos 5] [--semilla N] [--salida x.wav]`; para el
/// juez: `--pasos N`, `--latente S` (segundos que se denoisean; el oficial, 30), `--ruido`, `--volcado`.
/// `gotv video "descripción"` (M6): Wan 2.2 TI2V 5B → MP4. `--ruido` (el del juez) compara con la
/// misma entrada que diffusers.
fn video_cmd(args: &[String]) -> Result<()> {
    let mut p = gotv_mlx::video::Pedido { cuadros: 49, ..Default::default() };
    let (mut salida, mut ruido) = (None::<std::path::PathBuf>, None::<std::path::PathBuf>);
    let mut continuar = None::<std::path::PathBuf>;
    let mut semilla = None::<u64>;
    let mut palabras = Vec::new();
    let uso = gotv_core::i18n::t(
        "uso: gotv video \"descripción (en inglés)\" [--segundos 2] [--vertical] [--imagen primer-cuadro.png] [--continuar clip.mp4] [--pasos 50] [--guia 5] [--semilla N] [--salida x.mp4]",
        "usage: gotv video \"description\" [--seconds 2] [--vertical] [--image first-frame.png] [--continue clip.mp4] [--steps 50] [--guidance 5] [--seed N] [--out x.mp4]",
    );
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let mut sig = || it.next().cloned().ok_or_else(|| gotv_engine::anyhow_tr!("{a}: falta el valor", "{a}: the value is missing"));
        match a.as_str() {
            // medido 2026-11-13: «--help» caía en el prompt y lanzaba una generación de minutos
            "--help" | "-h" | "--ayuda" => {
                println!("{uso}");
                return Ok(());
            }
            "--segundos" | "--seconds" => p.cuadros = (sig()?.parse::<f32>()?.max(0.05) * gotv_mlx::video::CPS as f32).round() as u32 + 1,
            "--cuadros" | "--frames" => p.cuadros = sig()?.parse()?,
            "--vertical" => (p.ancho, p.alto) = (704, 1280),
            "--ancho" | "--width" => p.ancho = sig()?.parse()?,
            "--alto" | "--height" => p.alto = sig()?.parse()?,
            "--pasos" | "--steps" => p.pasos = sig()?.parse()?,
            "--guia" | "--guidance" => p.guia = sig()?.parse()?,
            "--negativo" | "--negative" => p.negativo = sig()?,
            "--semilla" | "--seed" => semilla = Some(sig()?.parse()?),
            "--salida" | "--out" => salida = Some(sig()?.into()),
            "--ruido" | "--noise" => ruido = Some(sig()?.into()),
            "--imagen" | "--image" => p.imagen = Some(sig()?.into()),
            "--continuar" | "--continue" => continuar = Some(sig()?.into()),
            o if o.starts_with("--") => gotv_engine::bail_tr!("opción desconocida «{o}»\n{uso}", "unknown option «{o}»\n{uso}"),
            _ => palabras.push(a.clone()),
        }
    }
    p.prompt = palabras.join(" ");
    if p.prompt.trim().is_empty() {
        anyhow::bail!("{uso}");
    }
    let ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0);
    p.semilla = semilla.unwrap_or(ms);
    let dir = gotv_core::generadores::video_dir()?;
    let salida = match salida {
        Some(s) => s,
        None => {
            std::fs::create_dir_all("generado")?;
            std::path::PathBuf::from(format!("generado/video-{ms}.mp4"))
        }
    };
    let t0 = std::time::Instant::now();
    let cancelar = std::sync::atomic::AtomicBool::new(false);
    // Ola PRO · EXTENDER: el mismo camino que la app (`generar_mp4` con `continuar`): sale el clip unido
    if let Some(previo) = &continuar {
        let segundos = (p.cuadros.max(2) - 1) as f32 / gotv_mlx::video::CPS as f32;
        let progreso = |s: String| eprintln!("  [{:6.1} s] {s}", t0.elapsed().as_secs_f32());
        let (seg, w, h) = gotv_mlx::video::generar_mp4(&dir, &p.prompt, Some(&p.negativo), segundos, p.ancho, p.alto, p.pasos, p.guia, p.semilla, None, Some(previo), &salida, &progreso, &cancelar)?;
        gotv_core::generadores::escribir_ficha(&salida, gotv_core::generadores::VIDEO_REPO)?;
        println!(
            "{}",
            gotv_core::tr!(
                "✅ {} ({seg:.1} s en total, {w}×{h}, semilla {}) en {:.0} s",
                "✅ {} ({seg:.1} s in total, {w}×{h}, seed {}) in {:.0} s",
                salida.display(),
                p.semilla,
                t0.elapsed().as_secs_f32()
            )
        );
        return Ok(());
    }
    let v = gotv_mlx::video::generar(
        &dir,
        &p,
        &|e| {
            let s = t0.elapsed().as_secs_f32();
            match e {
                gotv_mlx::video::Etapa::Texto => eprintln!("  [{s:6.1} s] {}", gotv_core::i18n::t("leyendo la descripción…", "reading the description…")),
                gotv_mlx::video::Etapa::Imagen => eprintln!("  [{s:6.1} s] {}", gotv_core::i18n::t("codificando el primer cuadro…", "encoding the first frame…")),
                gotv_mlx::video::Etapa::Paso(i, n) => eprintln!("  [{s:6.1} s] {} {i}/{n}", gotv_core::i18n::t("paso", "step")),
                gotv_mlx::video::Etapa::Video(i, n) => eprintln!("  [{s:6.1} s] {} {i}/{n}", gotv_core::i18n::t("sacando los cuadros", "rendering the frames")),
            }
        },
        &cancelar,
        ruido.as_deref(),
    )?;
    let mut m = gotv_mlx::video_mp4::Mp4::nuevo(&salida, v.ancho as usize, v.alto as usize, v.cps as i32)?;
    for c in &v.cuadros {
        m.cuadro(c)?;
    }
    m.cerrar()?;
    let ficha = gotv_core::generadores::escribir_ficha(&salida, gotv_core::generadores::VIDEO_REPO)?;
    println!(
        "{}",
        gotv_core::tr!(
            "✅ {} ({:.1} s, {}×{}, {} cuadros, semilla {}) en {:.0} s\n   ficha: {}",
            "✅ {} ({:.1} s, {}×{}, {} frames, seed {}) in {:.0} s\n   sheet: {}",
            salida.display(),
            v.segundos(),
            v.ancho,
            v.alto,
            v.cuadros.len(),
            p.semilla,
            t0.elapsed().as_secs_f32(),
            ficha.display()
        )
    );
    Ok(())
}

fn efecto_cmd(args: &[String]) -> Result<()> {
    let mut p = gotv_mlx::efectos::Pedido::default();
    let (mut salida, mut volcado, mut ruido) = (None::<std::path::PathBuf>, None::<std::path::PathBuf>, None::<std::path::PathBuf>);
    let mut semilla = None::<u64>;
    let mut palabras = Vec::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let mut sig = || it.next().cloned().ok_or_else(|| gotv_engine::anyhow_tr!("{a}: falta el valor", "{a}: the value is missing"));
        match a.as_str() {
            "--segundos" | "--seconds" => p.segundos = sig()?.parse()?,
            "--semilla" | "--seed" => semilla = Some(sig()?.parse()?),
            "--pasos" | "--steps" => p.pasos = sig()?.parse()?,
            "--latente" => p.segundos_latente = sig()?.parse()?,
            "--salida" | "--out" => salida = Some(sig()?.into()),
            "--volcado" | "--dump" => volcado = Some(sig()?.into()),
            "--ruido" | "--noise" => ruido = Some(sig()?.into()),
            _ => palabras.push(a.clone()),
        }
    }
    p.descripcion = palabras.join(" ");
    if p.descripcion.trim().is_empty() {
        anyhow::bail!(
            "{}",
            gotv_core::i18n::t(
                "uso: gotv efecto \"descripción (en inglés)\" [--segundos 5] [--semilla N] [--salida x.wav]",
                "usage: gotv sfx \"description\" [--seconds 5] [--seed N] [--out x.wav]"
            )
        );
    }
    let ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0);
    p.semilla = semilla.unwrap_or(ms);
    let dir = gotv_core::generadores::efecto_dir()?;
    let salida = match salida {
        Some(s) => s,
        None => {
            std::fs::create_dir_all("generado")?;
            std::path::PathBuf::from(format!("generado/efecto-{ms}.wav"))
        }
    };
    let t0 = std::time::Instant::now();
    let cancelar = std::sync::atomic::AtomicBool::new(false);
    let audio = gotv_mlx::efectos::generar(
        &dir,
        &p,
        &|e| {
            let s = t0.elapsed().as_secs_f32();
            match e {
                gotv_mlx::efectos::Etapa::Texto => eprintln!("  [{s:5.1} s] {}", gotv_core::i18n::t("leyendo la descripción…", "reading the description…")),
                gotv_mlx::efectos::Etapa::Paso(i, n) => {
                    if i == 1 || i % 10 == 0 || i == n {
                        eprintln!("  [{s:5.1} s] {} {i}/{n}", gotv_core::i18n::t("paso", "step"))
                    }
                }
                gotv_mlx::efectos::Etapa::Audio => eprintln!("  [{s:5.1} s] {}", gotv_core::i18n::t("sacando el audio…", "rendering the audio…")),
            }
        },
        &cancelar,
        ruido.as_deref(),
        volcado.as_deref(),
    )?;
    std::fs::write(&salida, audio.wav())?;
    let ficha = gotv_core::generadores::escribir_ficha(&salida, gotv_core::generadores::EFECTO_REPO)?;
    println!(
        "{}",
        gotv_core::tr!(
            "✅ {} ({:.1} s de audio mono a 48 kHz, semilla {}) en {:.1} s\n   ficha: {}",
            "✅ {} ({:.1} s of 48 kHz mono audio, seed {}) in {:.1} s\n   sheet: {}",
            salida.display(),
            audio.segundos(),
            p.semilla,
            t0.elapsed().as_secs_f32(),
            ficha.display()
        )
    );
    Ok(())
}

/// M7: `gotv musica "descripción" [--letra "…"|--letra-archivo f.txt] [--idioma es] [--segundos 30]
/// [--bpm 90] [--tono "A minor"] [--compas 4] [--semilla N] [--salida x.wav]`
fn musica_cmd(args: &[String]) -> Result<()> {
    let mut p = gotv_mlx::musica::Pedido::default();
    let (mut tarea, mut pistas) = (None::<String>, Vec::<String>::new());
    let (mut salida, mut volcado, mut ruido) = (None::<std::path::PathBuf>, None::<std::path::PathBuf>, None::<std::path::PathBuf>);
    let mut semilla = None::<u64>;
    let mut palabras = Vec::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let mut sig = || it.next().cloned().ok_or_else(|| gotv_engine::anyhow_tr!("{a}: falta el valor", "{a}: the value is missing"));
        match a.as_str() {
            "--letra" | "--lyrics" => p.letra = sig()?,
            "--letra-archivo" | "--lyrics-file" => p.letra = std::fs::read_to_string(sig()?)?, // juez-idioma: datos (nombre de opción o valor que se lee)
            "--idioma" | "--language" => p.idioma = sig()?,
            "--segundos" | "--seconds" => p.segundos = sig()?.parse()?,
            "--bpm" => p.bpm = Some(sig()?.parse()?),
            "--tono" | "--key" => p.tonalidad = sig()?,
            "--compas" | "--time-signature" => p.compas = sig()?,
            "--shift" => p.shift = sig()?.parse()?,
            "--sin-pensar" | "--no-think" => p.pensar = false, // juez-idioma: datos (nombre de opción o valor que se lee)
            "--timbre" | "--reference-audio" => p.timbre = Some(sig()?.into()),
            // Ola PRO · el DiT base (su carpeta) y sus tareas sobre `--fuente`: separar, añadir o completar
            "--base" => p.base = Some(sig()?.into()),
            "--tarea" | "--task" => tarea = Some(sig()?),
            "--pista" | "--track" => pistas.push(sig()?),
            "--pasos-base" | "--base-steps" => p.pasos_base = sig()?.parse()?,
            "--guia-dit" | "--dit-guidance" => p.guia_dit = sig()?.parse()?,
            // M7d: audio propio — cover (otra versión) o repaint (rehacer una parte)
            "--fuente" | "--source" => p.fuente = Some(gotv_mlx::musica::Fuente::cover(sig()?.into())),
            "--repintar" | "--repaint" => {
                let v = sig()?;
                let (a, b) = v.split_once(':').ok_or_else(|| gotv_engine::anyhow_tr!("--repintar desde:hasta (segundos; hasta -1 = el final)", "--repintar from:to (seconds; to -1 = the end)"))?;
                p.fuente.as_mut().ok_or_else(|| gotv_engine::anyhow_tr!("--repintar va después de --fuente", "--repintar goes after --fuente"))?.repintar = Some((a.trim().parse()?, b.trim().parse()?));
            }
            "--fuerza-cover" => p.fuente.as_mut().ok_or_else(|| gotv_engine::anyhow_tr!("--fuerza-cover va después de --fuente", "--fuerza-cover goes after --fuente"))?.fuerza_cover = sig()?.parse()?,
            "--ruido-cover" => p.fuente.as_mut().ok_or_else(|| gotv_engine::anyhow_tr!("--ruido-cover va después de --fuente", "--ruido-cover goes after --fuente"))?.ruido_cover = sig()?.parse()?,
            "--fuerza-repintar" => p.fuente.as_mut().ok_or_else(|| gotv_engine::anyhow_tr!("--fuerza-repintar va después de --fuente", "--fuerza-repintar goes after --fuente"))?.fuerza_repintar = sig()?.parse()?,
            "--latentes-fuente" => p.fuente.as_mut().ok_or_else(|| gotv_engine::anyhow_tr!("--latentes-fuente va después de --fuente", "--latentes-fuente goes after --fuente"))?.latentes = Some(sig()?.into()),
            // códigos de audio ya hechos (los del juez, o los de otra canción): «<|audio_code_N|>…» o números
            "--codigos" | "--codes" => {
                let t = std::fs::read_to_string(sig()?)?;
                p.codigos = Some(t.split(|c: char| !c.is_ascii_digit()).filter(|s| !s.is_empty()).map(|s| s.parse::<u32>()).collect::<Result<Vec<_>, _>>()?);
            }
            "--semilla" | "--seed" => semilla = Some(sig()?.parse()?),
            "--salida" | "--out" => salida = Some(sig()?.into()),
            "--volcado" | "--dump" => volcado = Some(sig()?.into()),
            "--ruido" | "--noise" => ruido = Some(sig()?.into()),
            _ => palabras.push(a.clone()),
        }
    }
    p.descripcion = palabras.join(" ");
    if p.descripcion.trim().is_empty() {
        anyhow::bail!(
            "{}",
            gotv_core::i18n::t(
                "uso: gotv musica \"descripción (en inglés)\" [--letra \"…\"] [--idioma es] [--segundos 30] [--bpm 90] [--tono \"A minor\"] [--compas 4] [--semilla N] [--salida x.wav]",
                "usage: gotv music \"description\" [--lyrics \"…\"] [--language en] [--seconds 30] [--bpm 90] [--key \"A minor\"] [--time-signature 4] [--seed N] [--out x.wav]"
            )
        );
    }
    if p.letra.trim().is_empty() {
        p.idioma = "unknown".into();
    } else if p.idioma == "unknown" {
        p.idioma = "en".into();
    }
    // Ola PRO · la tarea del base (con `--tarea extraer --pista vocals`, etc.)
    if let Some(t) = &tarea {
        p.tarea = gotv_mlx::musica::tarea_de(&serde_json::json!({"tarea": {"tipo": t, "pistas": pistas}}));
        if p.tarea.is_none() {
            gotv_engine::bail_tr!("--tarea {t}: usa extraer|lego|completar con --pista (una de las 12 del modelo)", "--task {t}: use extraer|lego|completar with --track (one of the model's 12)");
        }
        if p.base.is_none() {
            p.base = Some(gotv_core::generadores::musica_base_dir()?);
        }
    }
    let ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0);
    p.semilla = semilla.unwrap_or(ms);
    let dir = gotv_core::generadores::musica_dir()?;
    let salida = match salida {
        Some(s) => s,
        None => {
            std::fs::create_dir_all("generado")?;
            std::path::PathBuf::from(format!("generado/musica-{ms}.wav"))
        }
    };
    let t0 = std::time::Instant::now();
    let cancelar = std::sync::atomic::AtomicBool::new(false);
    let audio = gotv_mlx::musica::generar(
        &dir,
        &p,
        &|e| {
            let s = t0.elapsed().as_secs_f32();
            match e {
                gotv_mlx::musica::Etapa::Plan => eprintln!("  [{s:5.1} s] {}", gotv_core::i18n::t("el LM planea tempo, tono y compás…", "the LM is planning tempo, key and meter…")),
                gotv_mlx::musica::Etapa::Fuente => eprintln!("  [{s:5.1} s] {}", gotv_core::i18n::t("leyendo y codificando el audio propio…", "reading and encoding your audio…")),
                gotv_mlx::musica::Etapa::Codigos(i, n) => {
                    if i == 1 || i % 50 == 0 || i == n {
                        eprintln!("  [{s:5.1} s] {} {i}/{n}", gotv_core::i18n::t("códigos de audio", "audio codes"))
                    }
                }
                gotv_mlx::musica::Etapa::Texto => eprintln!("  [{s:5.1} s] {}", gotv_core::i18n::t("leyendo la descripción y la letra…", "reading the description and lyrics…")),
                gotv_mlx::musica::Etapa::Paso(i, n) => eprintln!("  [{s:5.1} s] {} {i}/{n}", gotv_core::i18n::t("paso", "step")),
                gotv_mlx::musica::Etapa::Audio => eprintln!("  [{s:5.1} s] {}", gotv_core::i18n::t("sacando el audio…", "rendering the audio…")),
            }
        },
        &cancelar,
        ruido.as_deref(),
        volcado.as_deref(),
    )?;
    std::fs::write(&salida, audio.wav())?;
    let ficha = gotv_core::generadores::escribir_ficha(&salida, gotv_core::generadores::MUSICA_REPO)?;
    if let Some(plan) = &audio.plan {
        eprintln!("  {} {plan}", gotv_core::i18n::t("el LM eligió:", "the LM chose:"));
    }
    println!(
        "{}",
        gotv_core::tr!(
            "✅ {} ({:.1} s de audio estéreo a 48 kHz, semilla {}) en {:.1} s\n   ficha: {}",
            "✅ {} ({:.1} s of 48 kHz stereo audio, seed {}) in {:.1} s\n   sheet: {}",
            salida.display(),
            audio.segundos(),
            p.semilla,
            t0.elapsed().as_secs_f32(),
            ficha.display()
        )
    );
    Ok(())
}

/// M4: `gotv imagen "descripción" [--ancho 1024] [--alto 1024] [--pasos 4] [--semilla N] [--salida x.png]`
fn imagen_cmd(args: &[String]) -> Result<()> {
    let mut p = gotv_mlx::imagen::Pedido::default();
    let (mut salida, mut volcado, mut guia) = (None::<std::path::PathBuf>, None::<std::path::PathBuf>, None::<std::path::PathBuf>);
    let (mut refs, mut libre) = (Vec::<std::path::PathBuf>::new(), false);
    let mut semilla = None::<u64>;
    let mut palabras = Vec::new();
    let mut it = args.iter();
    fn sig_img(it: &mut std::slice::Iter<String>, que: &str) -> Result<String> {
        it.next().cloned().ok_or_else(|| gotv_engine::anyhow_tr!("{que}: falta el valor", "{que}: the value is missing"))
    }
    let num = |v: Option<&String>, que: &str| -> Result<u32> { v.and_then(|s| s.parse().ok()).ok_or_else(|| gotv_engine::anyhow_tr!("{que}: falta un número", "{que}: a number is missing")) };
    while let Some(a) = it.next() {
        match a.as_str() {
            "--ancho" | "--width" => p.ancho = num(it.next(), a)?,
            "--alto" | "--height" => p.alto = num(it.next(), a)?,
            "--pasos" | "--steps" => p.pasos = num(it.next(), a)?,
            "--semilla" | "--seed" => semilla = it.next().and_then(|s| s.parse().ok()),
            "--salida" | "--out" => salida = it.next().map(Into::into),
            "--volcado" | "--dump" => volcado = it.next().map(Into::into),
            "--guia" | "--guide" => guia = it.next().map(Into::into),
            // Ola PRO: hasta 3 referencias más (4 con la guía); con `--ancho` y `--alto` la salida es de ese tamaño
            "--referencia" | "--reference" => refs.push(it.next().map(std::path::PathBuf::from).ok_or_else(|| gotv_engine::anyhow_tr!("--referencia: falta la imagen", "--reference: the image is missing"))?),
            "--tamano-libre" | "--free-size" => libre = true,
            // Ola PRO · klein base: su carpeta (con transformer/), la guía del CFG y el negativo
            "--base" => p.transformador = Some(std::path::PathBuf::from(sig_img(&mut it, a)?).join("transformer")),
            "--guia-cfg" | "--cfg" => p.guia_cfg = sig_img(&mut it, a)?.parse()?,
            "--negativo" | "--negative" => p.negativo = sig_img(&mut it, a)?,
            _ => palabras.push(a.clone()),
        }
    }
    p.prompt = palabras.join(" ");
    if p.prompt.trim().is_empty() {
        anyhow::bail!(
            "{}",
            gotv_core::i18n::t(
                "uso: gotv imagen \"descripción\" [--ancho 1024] [--alto 1024] [--pasos 4] [--semilla N] [--guia x.svg|x.png] [--referencia y.png …] [--tamano-libre] [--salida x.png]",
                "usage: gotv imagen \"description\" [--width 1024] [--height 1024] [--steps 4] [--seed N] [--guide x.svg|x.png] [--reference y.png …] [--free-size] [--out x.png]"
            )
        );
    }
    let ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0);
    p.semilla = semilla.unwrap_or(ms);
    // M5: con guía (SVG/PNG/JPEG) la imagen sale de su tamaño y la sigue; Ola PRO: hasta 4 guías y,
    // con `--tamano-libre`, del tamaño pedido
    let todas: Vec<std::path::PathBuf> = guia.into_iter().chain(refs).collect();
    if todas.len() > 4 {
        gotv_engine::bail_tr!("hasta 4 imágenes de referencia ({} dadas)", "up to 4 reference images ({} given)", todas.len());
    }
    for g in &todas {
        p.guias.push(gotv_mlx::imagen::Guia::de_archivo(g, p.ancho.min(1024), p.alto.min(1024))?);
    }
    if let Some(g) = p.guias.first().filter(|_| !libre) {
        (p.ancho, p.alto) = (g.ancho, g.alto);
    }
    let dir = gotv_core::generadores::imagen_dir()?;
    let salida = match salida {
        Some(s) => s,
        None => {
            std::fs::create_dir_all("generado")?;
            std::path::PathBuf::from(format!("generado/imagen-{ms}.png"))
        }
    };
    let t0 = std::time::Instant::now();
    let cancelar = std::sync::atomic::AtomicBool::new(false);
    let img = gotv_mlx::imagen::generar(
        &dir,
        &p,
        &|e| {
            let s = t0.elapsed().as_secs_f32();
            match e {
                gotv_mlx::imagen::Etapa::Texto => eprintln!("  [{s:5.1} s] {}", gotv_core::i18n::t("leyendo la descripción…", "reading the description…")),
                gotv_mlx::imagen::Etapa::Paso(i, n) => eprintln!("  [{s:5.1} s] {} {i}/{n}", gotv_core::i18n::t("paso", "step")),
                gotv_mlx::imagen::Etapa::Vae => eprintln!("  [{s:5.1} s] {}", gotv_core::i18n::t("pintando los píxeles…", "painting the pixels…")),
            }
        },
        &cancelar,
        volcado.as_deref(),
    )?;
    img.png(&salida)?;
    let ficha = gotv_core::generadores::escribir_ficha(&salida, gotv_core::generadores::IMAGEN_REPO)?;
    println!(
        "{}",
        gotv_core::tr!(
            "✅ {} ({}×{}, {} pasos, semilla {}) en {:.1} s\n   ficha: {}",
            "✅ {} ({}×{}, {} steps, seed {}) in {:.1} s\n   sheet: {}",
            salida.display(),
            img.ancho,
            img.alto,
            p.pasos,
            p.semilla,
            t0.elapsed().as_secs_f32(),
            ficha.display()
        )
    );
    Ok(())
}

fn decir_cmd(args: &[String], cfg: &Config) -> Result<()> {
    let (mut salida, mut voraz, mut mudo) = (None::<std::path::PathBuf>, false, false);
    let mut idioma = if cfg.lang.starts_with("en") { "english".to_string() } else { "spanish".to_string() };
    let mut palabras = Vec::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--salida" | "--out" => salida = it.next().map(Into::into),
            "--voraz" | "--greedy" => voraz = true,
            "--mudo" | "--mute" => mudo = true,
            "--idioma" | "--language" => idioma = it.next().cloned().unwrap_or(idioma),
            _ => palabras.push(a.clone()),
        }
    }
    let texto = palabras.join(" ");
    if texto.trim().is_empty() {
        anyhow::bail!("{}", gotv_core::i18n::t("uso: gotv decir \"texto\" [--salida x.wav] [--voraz] [--idioma spanish] [--mudo]", "usage: gotv say \"text\" [--out x.wav] [--greedy] [--language english] [--mute]"));
    }
    let dir = gotv_core::generadores::voz_dir()?;
    let mut onda: Vec<f32> = Vec::new();
    // suena EN VIVO, trozo a trozo, mientras se genera (M2)
    let altavoz = if mudo { None } else { Some(gotv_sonido::Altavoz::abrir(24000)?) };
    let h = gotv_mlx::hablar(&dir, &texto, &idioma, voraz, &gotv_mlx::Callar::default(), &mut |w| {
        if let Some(a) = &altavoz {
            a.empujar(w);
        }
        onda.extend_from_slice(w);
        Ok(())
    })?;
    if let Some(s) = &salida {
        std::fs::write(s, gotv_mlx::wav_pcm16(&onda, 24000))?;
    }
    let habla = h.muestras as f64 / 24000.0;
    let generar = (h.total - h.carga).as_secs_f64();
    println!(
        "{}",
        tr!(
            "🕳️  {:.1} s de voz · carga {:.1} s · primer audio {:.2} s tras la carga · {:.1} fotogramas/s ({:.1}× el tiempo real){}",
            "🕳️  {:.1} s of speech · load {:.1} s · first audio {:.2} s after loading · {:.1} frames/s ({:.1}× real time){}",
            habla,
            h.carga.as_secs_f64(),
            (h.primer_audio - h.carga).as_secs_f64(),
            h.fotogramas as f64 / generar.max(1e-6),
            habla / generar.max(1e-6),
            salida.as_ref().map(|s| format!(" → {}", s.display())).unwrap_or_default()
        )
    );
    if let Some(a) = altavoz {
        a.esperar();
        if a.cortes() > 0 {
            println!("{}", tr!("⚠ la voz se quedó sin onda {} veces (la generación fue más lenta que el habla)", "⚠ the voice ran dry {} times (generation was slower than speech)", a.cortes()));
        }
    }
    Ok(())
}

async fn generadores_cmd(args: &[String]) -> Result<()> {
    use gotv_core::generadores as gen;
    let gb = |b: u64| b as f64 / 1e9;
    let arg = |i: usize| args.get(i).map(String::as_str).unwrap_or("");
    let ficha = |repo: &str| gen::ficha(repo).ok_or_else(|| anyhow::anyhow!("{}", tr!("{repo} no está en el catálogo: `gotv generadores` lo lista", "{repo} is not in the catalog: `gotv generadores` lists it")));
    match arg(0) {
        "" | "lista" | "list" => {
            println!("{}", tr!("🕳️  Generadores (verificados en Hugging Face el {}): el motor es de GOTV; los pesos, de su autor.
", "🕳️  Generators (verified on Hugging Face on {}): the engine is GOTV's; the weights, their author's.
", gen::VERIFICADO));
            for e in gen::estados() {
                let estado = match (&e.aceptada, e.bytes_local) {
                    (_, b) if b > 0 => tr!("bajado {:.1} GB", "downloaded {:.1} GB", gb(b)),
                    (Some(_), _) => tr!("licencia aceptada", "license accepted"),
                    _ => String::new(),
                };
                println!("  {:<8} {:<26} {:<24} {:<44} {}{:>7.1} GB  {}", e.area_nombre, e.g.nombre, e.etiqueta, e.g.repo, if e.g.restringido { "🔒" } else { "  " }, gb(e.g.bytes_repo), estado);
            }
            println!("{}", tr!("
🔒 = acceso restringido en Hugging Face (aceptas allí con tu cuenta y das tu token). Tamaño = repo entero; su motor (M1–M7) bajará sólo lo que use.
`gotv generadores ver <repo>` · `aceptar <repo>` · `bajar <repo>` · `verificar`", "
🔒 = gated on Hugging Face (you accept there with your account and give your token). Size = whole repo; its engine (M1–M7) will download only what it uses.
`gotv generadores ver <repo>` · `aceptar <repo>` · `bajar <repo>` · `verificar`"));
        }
        "ver" | "show" => {
            let g = ficha(arg(1))?;
            println!("{} · {}
", g.nombre, g.repo);
            println!("{}", tr!("  área: {} · fase: {} (memoria y tiempo en esta Mac: sin medir hasta que llegue su motor)", "  area: {} · phase: {} (memory and time on this Mac: not measured until its engine arrives)", g.area.nombre(), g.fase));
            println!("{}", tr!("  licencia: {} — {}
  clase: {}{}", "  license: {} — {}
  class: {}{}", g.licencia, g.url_licencia, g.clase.etiqueta(), if g.restringido { tr!(" · 🔒 acceso restringido", " · 🔒 gated") } else { String::new() }));
            if g.publicado {
                println!("{}", tr!("  repo: {:.1} GB · commit {}", "  repo: {:.1} GB · commit {}", gb(g.bytes_repo), g.sha));
            } else {
                println!("{}", tr!("  repo: {:.1} GB · sin publicar aún en Hugging Face", "  repo: {:.1} GB · not published on Hugging Face yet", gb(g.bytes_repo)));
            }
            match gen::aceptacion(g.repo) {
                Some(a) if a.sha.is_empty() => println!("{}", tr!("  ✓ aceptada el {} (sin publicar aún)", "  ✓ accepted on {} (not published yet)", a.fecha)),
                Some(a) => println!("{}", tr!("  ✓ aceptada el {} (commit {})", "  ✓ accepted on {} (commit {})", a.fecha, a.sha)),
                None => println!("{}", tr!("  ✗ licencia sin aceptar", "  ✗ license not accepted")),
            }
            println!("
{}", gen::aviso(g));
        }
        "aceptar" | "accept" => {
            let g = ficha(arg(1))?;
            println!("{}
", gen::aviso(g));
            println!("{}", tr!("Lee la licencia ({}) antes de aceptar. Clase: {}.", "Read the license ({}) before accepting. Class: {}.", g.url_licencia, g.clase.etiqueta()));
            let ok = if args.iter().any(|a| a == "--acepto" || a == "--i-accept") {
                true
            } else {
                print!("{}", tr!("Escribe «acepto» para aceptarla: ", "Type «accept» to accept it: "));
                std::io::stdout().flush().ok();
                let mut l = String::new();
                std::io::stdin().read_line(&mut l)?;
                matches!(l.trim().to_lowercase().as_str(), "acepto" | "accept")
            };
            if !ok {
                anyhow::bail!("{}", gotv_core::i18n::t("no aceptada: no se descargará", "not accepted: it will not be downloaded"));
            }
            let a = gen::aceptar(g.repo)?;
            println!("{}", tr!("✓ aceptada {} ({}) el {}", "✓ accepted {} ({}) on {}", g.nombre, a.licencia, a.fecha));
            if g.restringido && !gotv_core::llaves::has(gen::LLAVE_HF) {
                println!("
{}", gen::explicar_restringido(g));
            }
        }
        "revocar" | "revoke" => {
            let g = ficha(arg(1))?;
            gen::revocar(g.repo)?;
            println!("{}", tr!("✓ aceptación retirada de {} (los archivos bajados no se borran: {})", "✓ acceptance withdrawn for {} (downloaded files are not deleted: {})", g.nombre, gen::carpeta(g).display()));
        }
        "bajar" | "pull" => {
            let g = ficha(arg(1))?;
            println!("{}", tr!("⏳ {} ({}) · repo de hasta {:.1} GB", "⏳ {} ({}) · repo of up to {:.1} GB", g.nombre, g.licencia, gb(g.bytes_repo)));
            let t = Instant::now();
            let dest = gen::bajar(g.repo, &|done, total| {
                let pct = (done * 100).checked_div(total).unwrap_or(100);
                print!("\r⬇ {pct:>3} %  {:.2} / {:.2} GB  ({:.0} MB/s)   ", done as f64 / 1e9, total as f64 / 1e9, done as f64 / 1e6 / t.elapsed().as_secs_f64().max(0.1));
                std::io::stdout().flush().ok();
            })
            .await?;
            println!("\n{}", tr!("✅ {} en {}", "✅ {} at {}", g.nombre, dest.display()));
        }
        "verificar" | "verify" => {
            println!("{}", tr!("Consultando la API de Hugging Face (catálogo del {})…", "Querying the Hugging Face API (catalog of {})…", gen::VERIFICADO));
            let mut con_cambios = 0;
            for g in gen::CATALOGO {
                match gen::verificar(g).await {
                    Ok(v) if v.cambios.is_empty() => println!("  ✓ {}", g.repo),
                    Ok(v) => {
                        con_cambios += 1;
                        println!("  ⚠ {}: {}", g.repo, v.cambios.join(" · "));
                    }
                    Err(e) => println!("  ✗ {}: {e}", g.repo),
                }
            }
            println!("{}", if con_cambios == 0 { gotv_core::i18n::t("el catálogo sigue al día", "the catalog is up to date").to_string() } else { tr!("{con_cambios} con cambios: revisar su licencia antes de usarlos", "{con_cambios} changed: review their license before using them") });
        }
        "token" => {
            println!("{}", gotv_core::i18n::t("Token de lectura de Hugging Face (https://huggingface.co/settings/tokens). Va al Llavero de macOS; no se muestra ni se guarda en archivos. Vacío = borrarlo.", "Hugging Face read token (https://huggingface.co/settings/tokens). It goes to the macOS Keychain; it is neither shown nor stored in files. Empty = delete it."));
            print!("token: ");
            std::io::stdout().flush().ok();
            let t = leer_oculto()?;
            if t.is_empty() {
                gotv_core::llaves::delete(gen::LLAVE_HF);
                println!("{}", gotv_core::i18n::t("✓ token borrado del Llavero", "✓ token deleted from the Keychain"));
            } else {
                gotv_core::llaves::set(gen::LLAVE_HF, &t)?;
                println!("{}", gotv_core::i18n::t("✓ token guardado en el Llavero", "✓ token saved in the Keychain"));
            }
        }
        otro => anyhow::bail!("{}", tr!("subcomando desconocido «{otro}»: lista · ver · aceptar · revocar · bajar · verificar · token", "unknown subcommand «{otro}»: lista · ver · aceptar · revocar · bajar · verificar · token")),
    }
    Ok(())
}

/// `gotv setup audio` (S2): baja Whisper large-v3-turbo (el oído de respaldo para los modelos
/// que no oyen) con la descarga en Rust de P12. Desde M1 corre en el núcleo: no pide Python.
async fn setup_audio(_cfg: &Config) -> Result<()> {
    if let Some(d) = gotv_core::audio::whisper_dir() {
        println!("{}", tr!("✅ Whisper ya está en {}", "✅ Whisper is already at {}", d.display()));
        return Ok(());
    }
    let repo = gotv_core::audio::stt_repo();
    let repo = repo.as_str();
    println!("{}", tr!("⏳ bajando {repo} (Whisper nativo: el oído de respaldo para modelos que no oyen)…", "⏳ downloading {repo} (native Whisper: the fallback hearing for models that cannot hear)…"));
    let t = Instant::now();
    let progreso = |done: u64, total: u64| {
        let pct = (done * 100).checked_div(total).unwrap_or(100);
        print!("\r⬇ {pct:>3} %  {:.2} / {:.2} GB  ({:.0} MB/s)   ", done as f64 / 1e9, total as f64 / 1e9, done as f64 / 1e6 / t.elapsed().as_secs_f64().max(0.1));
        std::io::stdout().flush().ok();
    };
    // el del catálogo, por el mismo camino que la app: exige la licencia aceptada
    if gotv_core::generadores::ficha(repo).is_some() {
        gotv_core::generadores::bajar(repo, &progreso).await?;
    } else {
        gotv_core::hub::pull(repo, &progreso).await?;
    }
    println!("\n{}", gotv_core::i18n::t("✅ listo: listen_audio y --audio transcriben con Whisper cuando el modelo no oye", "✅ done: listen_audio and --audio transcribe with Whisper when the model cannot hear"));
    Ok(())
}

/// `gotv tema '{"marca":"morado","croma":"sobrio"}' [--json | --muestra archivo.html]`
/// Imprime el CSS del tema (o su JSON con las garantías); `--muestra` escribe la página
/// de muestra. Sale con 1 si alguna garantía de contraste no se cumple.
fn tema(args: &[String]) -> Result<()> {
    let mut pedido = gotv_diseno::tema::Pedido::default();
    let (mut json, mut muestra) = (false, None);
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--json" => json = true,
            "--muestra" => muestra = it.next().cloned(),
            s if s.trim_start().starts_with('{') => {
                pedido = serde_json::from_str(s).map_err(|e| anyhow::anyhow!("{}", tr!("pedido de tema inválido: {e}", "invalid theme request: {e}")))?
            }
            s => pedido.marca = s.to_string(),
        }
    }
    let t = gotv_diseno::tema::Tema::new(pedido);
    if let Some(f) = muestra {
        std::fs::write(&f, t.muestra())?;
        eprintln!("{}", tr!("muestra escrita en {f}", "sample written to {f}"));
    } else if json {
        println!("{}", serde_json::to_string_pretty(&t)?);
    } else {
        print!("{}", t.css());
    }
    let malas = t.incumplidas();
    if !malas.is_empty() {
        for g in &malas {
            eprintln!("✗ {} · {} sobre {}: {:.2} < {:.2}", g.modo, g.frente, g.fondo, g.real, g.min);
        }
        std::process::exit(1);
    }
    Ok(())
}

/// `gotv pagina <ficha.json> --salida <carpeta>` escribe el proyecto (index.html, assets/…);
/// `--suelto f.html` un solo archivo con todo dentro; `--muestra <dirección>` usa la ficha
/// de muestra; `--direcciones` lista las direcciones de estilo.
fn pagina(args: &[String]) -> Result<()> {
    use gotv_diseno::pagina::{ficha_muestra, Pagina, DIRECCIONES};
    let (mut ficha, mut suelto, mut salida) = (None, None, None);
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--direcciones" => {
                for d in DIRECCIONES {
                    println!("{:<11} {}", d.id, d.texto());
                }
                return Ok(());
            }
            "--muestra" => ficha = Some(ficha_muestra(it.next().map(String::as_str).unwrap_or("tecnico"))),
            "--estricto" => {}
            "--esquema" => {
                println!("{}", gotv_diseno::pagina::esquema());
                return Ok(());
            }
            "--iconos" => {
                println!("{}", gotv_diseno::recursos::ICONOS.iter().map(|(n, _)| *n).collect::<Vec<_>>().join(" "));
                return Ok(());
            }
            "--suelto" => suelto = it.next().cloned(),
            "--salida" => salida = it.next().cloned(),
            f => {
                let txt = std::fs::read_to_string(f).map_err(|e| anyhow::anyhow!("{f}: {e}"))?;
                let estricto = args.iter().any(|a| a == "--estricto");
                let r = if estricto { gotv_diseno::pagina::validar(&txt).map(|f| (f, vec![])) } else { gotv_diseno::pagina::validar_tolerante(&txt) };
                match r {
                    Ok((x, avisos)) => {
                        for a in &avisos {
                            eprintln!("  ⚠ {a}");
                        }
                        ficha = Some(x)
                    }
                    Err(errores) => {
                        eprintln!("{}", tr!("✗ la ficha tiene {} error(es):", "✗ the spec has {} error(s):", errores.len()));
                        for e in &errores {
                            eprintln!("  · {e}");
                        }
                        std::process::exit(2);
                    }
                }
            }
        }
    }
    let ficha = ficha.ok_or_else(|| anyhow::anyhow!("{}", tr!("falta la ficha: gotv pagina <ficha.json> o --muestra <dirección>", "missing spec: gotv pagina <spec.json> or --muestra <direction>")))?;
    let p = Pagina::new(ficha)?;
    if let Some(dir) = salida {
        let inf = gotv_diseno::pagina::escribir_proyecto(std::path::Path::new(&dir), &p.archivos())?;
        for r in &inf.escritos {
            println!("✓ {dir}/{r}");
        }
        if !inf.iguales.is_empty() {
            println!("{}", tr!("= {} archivo(s) sin cambios", "= {} file(s) unchanged", inf.iguales.len()));
        }
        for (r, lado) in &inf.respetados {
            println!("{}", tr!("⚠ {r} fue editado a mano: no lo pisé; la versión nueva está en {lado}", "⚠ {r} was edited by hand: kept it; the new version is in {lado}"));
        }
    } else {
        let html = p.html_suelto();
        match suelto {
            Some(f) => {
                std::fs::write(&f, html)?;
                eprintln!("{}", tr!("página escrita en {f}", "page written to {f}"));
            }
            None => print!("{html}"),
        }
    }
    Ok(())
}

/// `gotv iso escena.json [--salida f.svg]`: dibuja la isométrica (o lista sus errores).
fn iso(args: &[String]) -> Result<()> {
    let (mut entrada, mut salida) = (None, None);
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--salida" => salida = it.next().cloned(),
            f => entrada = Some(f.to_string()),
        }
    }
    let f = entrada.ok_or_else(|| anyhow::anyhow!("{}", tr!("uso: gotv iso <escena.json> [--salida f.svg]", "usage: gotv iso <scene.json> [--salida f.svg]")))?;
    let txt = std::fs::read_to_string(&f).map_err(|e| anyhow::anyhow!("{f}: {e}"))?;
    match gotv_diseno::isometrico::dibujar(&txt) {
        Ok(svg) => match salida {
            Some(o) => {
                std::fs::write(&o, svg)?;
                eprintln!("✓ {o}");
            }
            None => println!("{svg}"),
        },
        Err(errores) => {
            eprintln!("{}", tr!("✗ la escena tiene {} error(es):", "✗ the scene has {} error(s):", errores.len()));
            for e in errores {
                eprintln!("  · {e}");
            }
            std::process::exit(2);
        }
    }
    Ok(())
}

/// `gotv app <carpeta> [--marca Nombre] [--direccion tecnico] [--entidad '{json}'|archivo.json]`.
fn app_nueva(args: &[String]) -> Result<()> {
    let (mut carpeta, mut marca, mut dir, mut entidad) = (None, None, "tecnico".to_string(), None::<String>);
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--marca" => marca = it.next().cloned(),
            "--direccion" => dir = it.next().cloned().unwrap_or(dir),
            "--entidad" => entidad = it.next().cloned(),
            c => carpeta = Some(c.to_string()),
        }
    }
    let carpeta = carpeta.ok_or_else(|| anyhow::anyhow!("{}", tr!("uso: gotv app <carpeta> [--marca N] [--direccion d]", "usage: gotv app <folder> [--marca N] [--direccion d]")))?;
    let base = std::path::Path::new(&carpeta);
    let nombre = base.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "app".into());
    let marca = marca.unwrap_or_else(|| nombre.clone());
    let entidad: Option<gotv_diseno::app::Entidad> = match entidad {
        Some(e) => {
            let txt = if std::path::Path::new(&e).is_file() { std::fs::read_to_string(&e)? } else { e };
            Some(serde_json::from_str(&txt)?)
        }
        None => None,
    };
    let archivos = gotv_diseno::app::nueva_con(&nombre, &marca, &dir, None, entidad.as_ref())?;
    let inf = gotv_diseno::pagina::escribir_proyecto(base, &archivos)?;
    println!("{}", tr!("✓ {} archivo(s) en {carpeta}", "✓ {} file(s) in {carpeta}", inf.escritos.len()));
    for (r, lado) in &inf.respetados {
        println!("{}", tr!("⚠ {r} ya existía: no lo pisé; el nuevo está en {lado}", "⚠ {r} already existed: kept it; the new one is in {lado}"));
    }
    println!("{}", tr!("siguiente: cd {carpeta} && npm install && npm test && npm run dev", "next: cd {carpeta} && npm install && npm test && npm run dev"));
    Ok(())
}

/// `gotv api <carpeta> --marco hono|fastapi|axum --entidad '{json}'|archivo.json [--marca Nombre]` (D8f).
fn api_nueva(args: &[String]) -> Result<()> {
    let (mut carpeta, mut marca, mut marco, mut entidad) = (None, None, "hono".to_string(), None::<String>);
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--marca" => marca = it.next().cloned(),
            "--marco" => marco = it.next().cloned().unwrap_or(marco),
            "--entidad" => entidad = it.next().cloned(),
            c => carpeta = Some(c.to_string()),
        }
    }
    let uso = || anyhow::anyhow!("{}", tr!("uso: gotv api <carpeta> --marco hono|fastapi|axum --entidad '{{json}}'|archivo.json [--marca N]", "usage: gotv api <folder> --marco hono|fastapi|axum --entidad '{{json}}'|file.json [--marca N]"));
    let carpeta = carpeta.ok_or_else(uso)?;
    let e = entidad.ok_or_else(uso)?;
    let txt = if std::path::Path::new(&e).is_file() { std::fs::read_to_string(&e)? } else { e };
    let entidad: gotv_diseno::app::Entidad = serde_json::from_str(&txt)?;
    let base = std::path::Path::new(&carpeta);
    let nombre = base.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "api".into());
    let archivos = gotv_diseno::api::nueva(&nombre, &marca.unwrap_or_else(|| nombre.clone()), &marco, &entidad)?;
    let inf = gotv_diseno::pagina::escribir_proyecto(base, &archivos)?;
    println!("{}", tr!("✓ {} archivo(s) en {carpeta}", "✓ {} file(s) in {carpeta}", inf.escritos.len()));
    for (r, lado) in &inf.respetados {
        println!("{}", tr!("⚠ {r} ya existía: no lo pisé; el nuevo está en {lado}", "⚠ {r} already existed: kept it; the new one is in {lado}"));
    }
    println!("{}", tr!("siguiente: lee {carpeta}/README.md (instalar, probar, arrancar)", "next: read {carpeta}/README.md (install, test, start)"));
    Ok(())
}

fn ayuda() -> String {
    gotv_core::i18n::t(
        "🕳️  GOD OF THE VOID — agente local de programación y diseño

USO
  gotv                         conversación (TUI) en esta carpeta
  gotv \"orden\"                 una tarea y termina
  gotv --resume [\"orden\"]      continúa la última conversación de esta carpeta

OPCIONES (en cualquier posición)
  --perfil auto|programar|diseno|imagen|video|musica   --mode <modo de permisos>   --lang es|en   --mac (Control Mac en esta orden)
  --image <archivo>   --audio <archivo>
  GOTV_MODEL=<repo|proveedor:modelo>   elige el modelo (p. ej. anthropic:claude-haiku-5-5)

DISEÑO SIN MODELO
  gotv tema · gotv pagina [--muestra|--esquema|--iconos] · gotv iso · gotv estilo
  gotv app <carpeta> [--marca N] [--direccion d] [--entidad '{json}'|archivo.json]
  gotv api <carpeta> --marco hono|fastapi|axum --entidad '{json}'|archivo.json   la API REST de esos datos, con validación y pruebas

MODELOS Y MÁQUINA
  gotv hw · gotv models [search|pull|import] · gotv setup [audio|rag] · gotv preflight · gotv config
  gotv generadores [ver|aceptar|revocar|bajar|verificar|token]   (voz, imagen, video, música y sus licencias)
  gotv decir \"texto\" [--salida x.wav] [--voraz]   (la voz de GOTV, en el núcleo: Qwen3-TTS 1.7B)
  gotv imagen \"descripción\" [--ancho 1024] [--alto 1024] [--semilla N] [--guia x.svg] [--salida x.png]   (imágenes en el núcleo: FLUX.2 klein 4B)
  gotv musica \"descripción\" [--letra \"…\"] [--idioma es] [--segundos 30] [--bpm 90] [--fuente mia.mp3 [--repintar 10:18] [--fuerza-cover 0.5]] [--salida x.wav]   (música en el núcleo: ACE-Step 1.5)
  gotv efecto \"descripción en inglés\" [--segundos 5] [--semilla N] [--salida x.wav]   (efectos de sonido en el núcleo: MOSS-SoundEffect 2.0)
  gotv escuchar [--archivo x.wav|mp3|m4a]   (micrófono → detector de voz → Whisper, en el núcleo)
  gotv conversar [--herramientas] [--modelo <repo>]   (hablar con GOTV por voz: te escucha y te responde hablando; si hablas encima, calla)
  gotv serve   (API compatible con OpenAI)

PROYECTO
  gotv index · gotv search \"…\" · gotv check · gotv rewind · gotv sessions · gotv checkpoints · gotv skills
  gotv mcp [lista|entrar <servidor>|salir <servidor>|recursos|leer <servidor> <uri>|prompts|importar|serve]   (servidores MCP; la sesión OAuth de los remotos va al Llavero)
  gotv plugin [lista|instalar <carpeta> [--si] [--reemplazar]|confiar <nombre>|quitar <nombre>]   (plugins de GOTV y de Claude Code; sin confianza no cargan nada)
  gotv acp [--guion <archivo>]  GOTV para un editor que habla el Agent Client Protocol (por stdio; con --guion, sin modelo)
  gotv chrome · gotv install · gotv uninstall",
        "🕳️  GOD OF THE VOID — local coding and design agent

USAGE
  gotv                         conversation (TUI) in this folder
  gotv \"task\"                  one task, then exit
  gotv --resume [\"task\"]       continue this folder's last conversation

OPTIONS (any position)
  --perfil auto|programar|diseno|imagen|video|musica   --mode <permission mode>   --lang es|en   --mac (Mac control for this order)
  --image <file>   --audio <file>
  GOTV_MODEL=<repo|provider:model>     pick the model (e.g. anthropic:claude-haiku-5-5)

DESIGN WITHOUT A MODEL
  gotv tema · gotv pagina [--muestra|--esquema|--iconos] · gotv iso · gotv estilo
  gotv app <folder> [--marca N] [--direccion d] [--entidad '{json}'|file.json]
  gotv api <folder> --marco hono|fastapi|axum --entidad '{json}'|file.json   the REST API for that data, with validation and tests

MODELS AND MACHINE
  gotv hw · gotv models [search|pull|import] · gotv setup [audio|rag] · gotv preflight · gotv config
  gotv generadores [ver|aceptar|revocar|bajar|verificar|token]   (voice, image, video, music and their licenses)
  gotv say \"text\" [--out x.wav] [--greedy]   (GOTV's voice, in the core: Qwen3-TTS 1.7B)
  gotv image \"description\" [--width 1024] [--height 1024] [--seed N] [--guide x.svg] [--out x.png]   (images in the core: FLUX.2 klein 4B)
  gotv music \"description\" [--lyrics \"…\"] [--language en] [--seconds 30] [--bpm 90] [--source mine.mp3 [--repaint 10:18] [--fuerza-cover 0.5]] [--out x.wav]   (music in the core: ACE-Step 1.5)
  gotv sfx \"description\" [--seconds 5] [--seed N] [--out x.wav]   (sound effects in the core: MOSS-SoundEffect 2.0)
  gotv listen [--file x.wav]   (microphone → voice detector → Whisper, in the core)
  gotv talk [--tools] [--model <repo>]   (voice conversation with GOTV: it listens and answers aloud; talk over it to interrupt)
  gotv serve   (OpenAI-compatible API)

PROJECT
  gotv index · gotv search \"…\" · gotv check · gotv rewind · gotv sessions · gotv checkpoints · gotv skills
  gotv mcp [lista|entrar <server>|salir <server>|recursos|leer <server> <uri>|prompts|importar|serve]   (MCP servers; remote OAuth sign-in goes to the Keychain)
  gotv plugin [lista|instalar <folder> [--si] [--reemplazar]|confiar <name>|quitar <name>]   (GOTV and Claude Code plugins; untrusted ones load nothing)
  gotv acp [--guion <file>]     GOTV for an editor that speaks the Agent Client Protocol (over stdio; with --guion, no model)
  gotv chrome · gotv install · gotv uninstall",
    )
    .to_string()
}

/// F13.4: `gotv mcp lista | entrar <servidor> | salir <servidor>` — los servidores MCP de este proyecto y
/// la sesión OAuth de los remotos (los tokens van al Llavero; nunca se muestran).
async fn mcp_cmd(root: &std::path::Path, args: &[String], cfg: &Config) -> Result<()> {
    use gotv_core::mcp::{self, Tipo};
    let (cfgs, avisos, _) = mcp::load(root);
    for a in avisos {
        eprintln!("⚠ {a}");
    }
    let buscar = |n: Option<&String>| -> Result<mcp::ServerConfig> {
        let n = n.ok_or_else(|| anyhow::anyhow!("{}", tr!("falta el nombre del servidor", "missing the server name")))?;
        cfgs.iter().find(|c| &c.name == n).cloned().ok_or_else(|| anyhow::anyhow!("{}", tr!("no hay un servidor MCP «{n}» (mira `gotv mcp lista`)", "there is no MCP server «{n}» (see `gotv mcp lista`)")))
    };
    match args.first().map(String::as_str).unwrap_or("lista") {
        "lista" => {
            if cfgs.is_empty() {
                println!("{}", tr!("  (ninguno) — se configuran en .mcp.json, .gotv/mcp.local.json o ~/.config/gotv/mcp.json", "  (none) — configure them in .mcp.json, .gotv/mcp.local.json or ~/.config/gotv/mcp.json"));
            }
            for c in &cfgs {
                let donde = match (&c.tipo, &c.url) {
                    (Tipo::Stdio, _) => format!("{} {}", c.command, c.args.join(" ")),
                    (t, Some(u)) => format!("{t:?} {u}"),
                    (t, None) => format!("{t:?}"),
                };
                let sesion = if c.tipo != Tipo::Stdio && gotv_core::mcp_oauth::leer(&c.name).is_some() { tr!(" · 🔑 sesión iniciada", " · 🔑 signed in") } else { String::new() };
                let apagado = if c.activo { String::new() } else { tr!(" · apagado", " · off") };
                println!("  {} ({:?}): {donde}{sesion}{apagado}", c.name, c.ambito);
            }
            Ok(())
        }
        "entrar" => {
            let c = buscar(args.get(1))?;
            let url = c.url.clone().filter(|_| c.tipo != Tipo::Stdio).ok_or_else(|| anyhow::anyhow!("{}", tr!("«{}» corre en esta Mac (stdio): no lleva sesión", "«{}» runs on this Mac (stdio): it has no sign-in", c.name)))?;
            println!("{}", tr!("· abriendo el navegador para entrar en «{}»… (la vuelta llega a 127.0.0.1; 5 min)", "· opening the browser to sign in to «{}»… (the callback comes to 127.0.0.1; 5 min)", c.name));
            gotv_core::mcp_oauth::entrar(&c.name, &url, None, |u| {
                // el banco entra sin abrir el navegador de nadie: `GOTV_ABRIR_CON=curl…` sigue la redirección
                match std::env::var("GOTV_ABRIR_CON").ok().filter(|p| !p.trim().is_empty()) {
                    Some(p) => {
                        let mut partes = p.split_whitespace();
                        let prog = partes.next().unwrap_or("open");
                        std::process::Command::new(prog).args(partes).arg(u).stdout(std::process::Stdio::null()).spawn()?;
                    }
                    None => {
                        std::process::Command::new("open").arg(u).status()?;
                    }
                }
                Ok(())
            })
            .await?;
            println!("{}", tr!("✓ sesión iniciada en «{}» (los tokens están en el Llavero de macOS)", "✓ signed in to «{}» (the tokens are in the macOS Keychain)", c.name));
            Ok(())
        }
        // F13.10: GOTV como servidor MCP por stdio (lo que se escribe por stdout es SÓLO el protocolo)
        "serve" => {
            let ofrecidas: Vec<String> = args
                .iter()
                .position(|a| a == "--ofrecer")
                .and_then(|i| args.get(i + 1))
                .map(|s| s.split(',').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect())
                .unwrap_or_else(|| gotv_core::mcp_serve::OFRECIDAS.iter().map(|s| s.to_string()).collect());
            eprintln!("{}", tr!("GOD OF THE VOID · servidor MCP por stdio en {} · ofrece: {}", "GOD OF THE VOID · MCP server over stdio in {} · offers: {}", root.display(), ofrecidas.join(", ")));
            gotv_core::mcp_serve::servir(tokio::io::stdin(), tokio::io::stdout(), root.to_path_buf(), perms_from(cfg, root), ofrecidas).await;
            Ok(())
        }
        // F13.9: traer lo que ya está configurado en Claude Code / Claude Desktop (con permiso, uno por uno)
        "importar" => {
            let todos = args.iter().any(|a| a == "--todo");
            let imp = mcp::importables(root);
            if imp.is_empty() {
                println!("{}", tr!("  (nada que importar: GOTV ya tiene todo lo de Claude Code y Claude Desktop)", "  (nothing to import: GOTV already has everything from Claude Code and Claude Desktop)"));
                return Ok(());
            }
            let destino = mcp::user_file();
            for i in &imp {
                let que = i.def["url"].as_str().map(String::from).unwrap_or_else(|| format!("{} {}", i.def["command"].as_str().unwrap_or(""), i.def["args"].as_array().into_iter().flatten().filter_map(|a| a.as_str()).collect::<Vec<_>>().join(" ")));
                let secretos = if i.secretos.is_empty() { String::new() } else { tr!(" · secretos al Llavero: {}", " · secrets to the Keychain: {}", i.secretos.join(", ")) };
                println!("  {} ({}): {que}{secretos}", i.nombre, i.origen);
                if !todos {
                    use std::io::Write;
                    print!("{}", tr!("    ¿importar? [s/N] ", "    import? [y/N] "));
                    std::io::stdout().flush().ok();
                    let mut r = String::new();
                    std::io::stdin().read_line(&mut r).ok();
                    if !matches!(r.trim().to_lowercase().as_str(), "s" | "si" | "sí" | "y" | "yes") {
                        continue;
                    }
                }
                let n = mcp::importar(i, &destino)?;
                println!("{}", tr!("    ✓ importado en {} ({n} secreto(s) al Llavero)", "    ✓ imported into {} ({n} secret(s) to the Keychain)", destino.display()));
            }
            Ok(())
        }
        // F13.6: recursos y prompts de los servidores (las menciones con @ y las órdenes /mcp__… son de F8)
        "recursos" | "leer" | "prompts" => {
            let activos: Vec<mcp::ServerConfig> = cfgs.iter().filter(|c| c.activo && args.get(1).is_none_or(|n| &c.name == n)).cloned().collect();
            let (mut m, notas) = mcp::Mcp::start(&activos, root).await;
            for n in notas {
                eprintln!("· {n}");
            }
            match args[0].as_str() {
                "recursos" => {
                    for c in &activos {
                        match m.recursos(&c.name).await {
                            Ok(rs) => {
                                for (uri, nombre, desc) in rs {
                                    println!("  {} · {nombre} <{uri}> {desc}", c.name);
                                }
                            }
                            Err(e) => eprintln!("  {}: {e}", c.name),
                        }
                    }
                }
                "leer" => {
                    let (Some(srv), Some(uri)) = (args.get(1), args.get(2)) else {
                        anyhow::bail!("{}", tr!("uso: gotv mcp leer <servidor> <uri>", "usage: gotv mcp leer <server> <uri>"));
                    };
                    println!("{}", m.leer_recurso(srv, uri).await?);
                }
                _ => {
                    for (srv, nombre, desc, a) in m.prompts().await {
                        println!("  /mcp__{srv}__{nombre} {}  {desc}", a.iter().map(|x| format!("<{x}>")).collect::<Vec<_>>().join(" "));
                    }
                }
            }
            Ok(())
        }
        "salir" => {
            let c = buscar(args.get(1))?;
            gotv_core::mcp_oauth::olvidar(&c.name);
            println!("{}", tr!("✓ sesión de «{}» cerrada (tokens borrados del Llavero)", "✓ «{}» signed out (tokens removed from the Keychain)", c.name));
            Ok(())
        }
        otro => anyhow::bail!("{}", tr!("«{otro}»: gotv mcp lista | entrar <servidor> | salir <servidor> | recursos [servidor] | leer <servidor> <uri> | prompts [servidor] | importar [--todo] | serve [--ofrecer word,excel,…]", "«{otro}»: gotv mcp lista | entrar <server> | salir <server> | recursos [server] | leer <server> <uri> | prompts [server] | importar [--todo] | serve [--ofrecer word,excel,…]")),
    }
}

/// Ola H: `gotv conexiones …` — las conexiones SSH de ESTE proyecto (cada proyecto ve las suyas).
async fn conexiones_cmd(root: &std::path::Path, args: &[String]) -> Result<()> {
    use gotv_core::conexiones::{self as cx, Conexion, Registro};
    let mut reg = Registro::cargar(root)?;
    let opcion = |k: &str| args.iter().position(|a| a == k).and_then(|i| args.get(i + 1)).cloned();
    let bandera = |k: &str| args.iter().any(|a| a == k);
    let sueltos: Vec<&String> = {
        let mut v = Vec::new();
        let mut salta = false;
        for a in args.iter().skip(1) {
            if salta {
                salta = false;
                continue;
            }
            if matches!(a.as_str(), "--llave" | "--salto" | "--carpeta" | "--nota" | "--como") {
                salta = true;
                continue;
            }
            if !a.starts_with("--") {
                v.push(a);
            }
        }
        v
    };
    let confirma = |pregunta: &str| -> bool {
        if bandera("--si") {
            return true;
        }
        use std::io::{BufRead, Write};
        print!("{pregunta} [s/N] ");
        let _ = std::io::stdout().flush();
        let mut l = String::new();
        let _ = std::io::stdin().lock().read_line(&mut l);
        matches!(l.trim().to_lowercase().as_str(), "s" | "si" | "sí" | "y" | "yes") // juez-idioma: datos (nombre de opción o valor que se lee)
    };
    match args.first().map(String::as_str).unwrap_or("lista") {
        "lista" | "list" => {
            println!("{}", tr!("🛰  Conexiones de {} ({})", "🛰  Connections of {} ({})", root.display(), cx::ruta_registro(root).display()));
            if reg.conexiones.is_empty() {
                println!("{}", tr!("  (ninguna) — agrega una: gotv conexiones agregar prod deploy@servidor.com", "  (none) — add one: gotv conexiones agregar prod deploy@server.com"));
            }
            let kh = std::fs::read_to_string(cx::ruta_known_hosts(root)).unwrap_or_default();
            for c in &reg.conexiones {
                let fijada = kh.lines().any(|l| l.split_whitespace().next().is_some_and(|h| h == c.host || h == format!("[{}]:{}", c.host, c.puerto.unwrap_or(22))));
                println!("  {} {}{}", if fijada { "🔒" } else { "○" }, c.resumen(), if fijada { String::new() } else { tr!("  (sin huella: `gotv conexiones probar {}`)", "  (no fingerprint: `gotv conexiones probar {}`)", c.nombre) });
            }
        }
        "agregar" | "add" => {
            let (Some(nombre), Some(dest)) = (sueltos.first(), sueltos.get(1)) else {
                anyhow::bail!("{}", tr!("uso: gotv conexiones agregar <nombre> <usuario@host[:puerto]> [--llave ruta] [--salto otra] [--carpeta /srv/app] [--nota \"…\"] [--solo-lectura]", "usage: gotv conexiones agregar <name> <user@host[:port]> [--llave path] [--salto other] [--carpeta /srv/app] [--nota \"…\"] [--solo-lectura]"));
            };
            let (usuario, resto) = match dest.split_once('@') {
                Some((u, h)) => (Some(u.to_string()), h.to_string()),
                None => (None, dest.to_string()),
            };
            let (host, puerto) = match resto.rsplit_once(':') {
                Some((h, p)) if !h.contains(':') && p.chars().all(|c| c.is_ascii_digit()) => (h.to_string(), p.parse().ok()),
                _ => (resto, None),
            };
            let c = Conexion { nombre: nombre.to_string(), host, usuario, puerto, llave: opcion("--llave"), salto: opcion("--salto"), carpeta: opcion("--carpeta"), nota: opcion("--nota"), solo_lectura: bandera("--solo-lectura") };
            reg.agregar(c.clone())?;
            reg.guardar(root)?;
            println!("{}", tr!("✓ {} — siguiente: `gotv conexiones llave {}` (si no tiene llave) y `gotv conexiones probar {}`", "✓ {} — next: `gotv conexiones llave {}` (if it has no key) and `gotv conexiones probar {}`", c.resumen(), c.nombre, c.nombre));
        }
        "quitar" | "remove" => {
            let nombre = sueltos.first().ok_or_else(|| anyhow::anyhow!("gotv conexiones quitar <nombre>"))?;
            let c = reg.quitar(nombre)?;
            reg.guardar(root)?;
            println!("{}", tr!("✓ quitada: {} (su llave, si la creó GOTV, sigue en {})", "✓ removed: {} (its key, if GOTV created it, stays in {})", c.resumen(), cx::carpeta_llaves(root).display()));
        }
        "llave" | "key" => {
            let nombre = sueltos.first().ok_or_else(|| anyhow::anyhow!("gotv conexiones llave <nombre>"))?.to_string();
            let mut c = reg.buscar(&nombre).cloned().ok_or_else(|| anyhow::anyhow!("{}", tr!("no hay «{nombre}» en este proyecto", "no «{nombre}» in this project")))?;
            let (privada, publica) = cx::generar_llave(root, &nombre)?;
            c.llave = Some(privada.display().to_string());
            reg.agregar(c.clone())?;
            reg.guardar(root)?;
            println!("{}", tr!("🔑 llave de «{nombre}»: {} (privada, fuera del proyecto)\n\nAutoriza ESTA línea en el servidor (en ~/.ssh/authorized_keys de {}):\n\n{publica}\n", "🔑 «{nombre}»'s key: {} (private, outside the project)\n\nAuthorize THIS line on the server (in {}'s ~/.ssh/authorized_keys):\n\n{publica}\n", privada.display(), c.destino()));
        }
        "probar" | "test" => {
            let nombre = sueltos.first().ok_or_else(|| anyhow::anyhow!("gotv conexiones probar <nombre>"))?.to_string();
            let c = reg.buscar(&nombre).cloned().ok_or_else(|| anyhow::anyhow!("{}", tr!("no hay «{nombre}» en este proyecto", "no «{nombre}» in this project")))?;
            // el salto primero: sin su huella no se llega a la de atrás
            let mut cadena = vec![c.clone()];
            if let Some(j) = c.salto.as_ref().and_then(|j| reg.buscar(j)) {
                cadena.insert(0, j.clone());
            }
            for x in &cadena {
                if c.salto.is_some() && x.nombre == c.nombre {
                    println!("{}", tr!("· «{}» está detrás de «{}»: su huella se fija con la primera conexión (TOFU por el bastión ya verificado)", "· «{}» is behind «{}»: its fingerprint is pinned on first connection (TOFU through the already verified bastion)", x.nombre, c.salto.as_deref().unwrap_or("")));
                    continue;
                }
                let (lineas, huellas) = cx::escanear(x)?;
                println!("{}", tr!("🛡  Huellas que ofrece {} (compáralas con las del servidor: `ssh-keygen -lf /etc/ssh/ssh_host_ed25519_key.pub`):", "🛡  Fingerprints offered by {} (compare them with the server's: `ssh-keygen -lf /etc/ssh/ssh_host_ed25519_key.pub`):", x.host));
                for h in &huellas {
                    println!("   {h}");
                }
                if !confirma(&tr!("¿Fijar estas huellas para «{}» en ESTE proyecto?", "Pin these fingerprints for «{}» in THIS project?", x.nombre)) {
                    println!("{}", gotv_core::i18n::t("✗ no se fijó nada", "✗ nothing pinned"));
                    return Ok(());
                }
                cx::fijar_huella(root, x, &lineas)?;
            }
            if c.salto.is_some() {
                // la máquina de atrás: se aprende por el túnel y se fija (accept-new SÓLO esta vez)
                let reg2 = Registro::cargar(root)?;
                let mut o = cx::opciones(root, &reg2, &c, false)?;
                if let Some(i) = o.iter().position(|a| a == "StrictHostKeyChecking=yes") {
                    o[i] = "StrictHostKeyChecking=accept-new".into();
                }
                o.extend([c.destino(), "--".into(), "true".into()]);
                let st = std::process::Command::new("ssh").args(&o).status()?;
                if !st.success() {
                    anyhow::bail!("{}", tr!("no pude llegar a «{nombre}» por el salto", "could not reach «{nombre}» through the jump"));
                }
            }
            let (rc, out) = cx::ejecutar(root, &nombre, "echo \"$(whoami)@$(hostname) · $(uname -sr)\" && uptime", 30).await?;
            println!("{} {}", if rc == 0 { "✓" } else { "✗" }, out.trim());
        }
        "importar" | "import" => {
            let alias = sueltos.first().ok_or_else(|| anyhow::anyhow!("gotv conexiones importar <alias de ~/.ssh/config> [--como nombre]"))?;
            let nombre = opcion("--como").unwrap_or_else(|| alias.to_string());
            let c = cx::importar_de_ssh_config(alias, &nombre)?;
            reg.agregar(c.clone())?;
            reg.guardar(root)?;
            println!("{}", tr!("✓ importada a ESTE proyecto: {} — siguiente: `gotv conexiones probar {}`", "✓ imported into THIS project: {} — next: `gotv conexiones probar {}`", c.resumen(), c.nombre));
        }
        "ssh" | "run" => {
            let nombre = sueltos.first().ok_or_else(|| anyhow::anyhow!("gotv conexiones ssh <nombre> <orden…>"))?.to_string();
            let cmd = sueltos[1..].iter().map(|s| s.as_str()).collect::<Vec<_>>().join(" ");
            if cmd.trim().is_empty() {
                anyhow::bail!("gotv conexiones ssh <nombre> <orden…>");
            }
            let (rc, out) = cx::ejecutar(root, &nombre, &cmd, 120).await?;
            println!("{}", out.trim_end());
            if rc != 0 {
                std::process::exit(rc);
            }
        }
        "subir" | "bajar" | "upload" | "download" => {
            // origen → destino, como scp/cp: subir <local> <remoto> · bajar <remoto> <local>
            let subir = matches!(args[0].as_str(), "subir" | "upload");
            let (Some(nombre), Some(origen), Some(destino)) = (sueltos.first(), sueltos.get(1), sueltos.get(2)) else {
                anyhow::bail!("gotv conexiones subir <nombre> <ruta-local> <ruta-remota>  ·  gotv conexiones bajar <nombre> <ruta-remota> <ruta-local>");
            };
            let (local, remoto) = if subir { (origen, destino) } else { (destino, origen) };
            println!("{}", cx::transferir(root, nombre, subir, local, remoto, 600).await?);
        }
        "registro" | "log" => {
            let n: usize = sueltos.first().and_then(|x| x.parse().ok()).unwrap_or(20);
            for b in cx::registro(root, n) {
                println!("{} · {} · exit {} · {} ms · {}", b.cuando, b.conexion, b.salida, b.ms, b.orden);
            }
        }
        otro => anyhow::bail!("{}", tr!("subcomando desconocido «{otro}»: lista · agregar · quitar · llave · probar · importar · ssh · subir · bajar · registro", "unknown subcommand «{otro}»: lista · agregar · quitar · llave · probar · importar · ssh · subir · bajar · registro")),
    }
    Ok(())
}


/// PRO-6 · `gotv separar <audio> [--fino] [--seis] [--bateria]`: separa un audio en sus pistas con HTDemucs (las
/// del catálogo, aceptadas y bajadas en Configuración → Generadores) y las deja junto al audio.
/// `--bateria` (EXP-7) parte una pista de batería en bombo, caja, platos y toms con el afinado de GOTV.
fn separar_cmd(args: &[String]) -> Result<()> {
    let (mut fino, mut seis, mut bateria) = (false, false, false);
    let mut audio = None::<std::path::PathBuf>;
    for a in args {
        match a.as_str() {
            "--fino" | "--fine" => fino = true,
            "--seis" | "--six" | "--6" => seis = true,
            "--bateria" | "--batería" | "--drums" => bateria = true, // juez-idioma: datos (nombres de la opción)
            _ => audio = Some(a.into()),
        }
    }
    let Some(audio) = audio else {
        anyhow::bail!("{}", gotv_core::i18n::t("uso: gotv separar <audio> [--fino] [--seis] [--bateria]", "usage: gotv split <audio> [--fine] [--six] [--drums]"));
    };
    let modelo = if bateria { "bateria" } else if seis { "6" } else if fino { "ft" } else { "4" };
    let t0 = std::time::Instant::now();
    let cancelar = std::sync::atomic::AtomicBool::new(false);
    let avance = |s: String| eprintln!("{}", tr!("separando · {s}", "splitting · {s}"));
    // la batería se parte como en la app: de una MEZCLA primero sale su batería (medido 2026-11-16: el
    // afinado de batería sobre la mezcla entera contaba el piano como platos)
    let (pistas, _) = if bateria {
        gotv_core::generadores::separar_bateria(&audio, gotv_core::generadores::parece_bateria_sola(&audio), &avance, &cancelar)?
    } else {
        gotv_core::generadores::separar_audio(&audio, modelo, &avance, &cancelar)?
    };
    for (p, r) in &pistas {
        println!("{p}: {}", r.display());
    }
    println!("{}", tr!("✅ {} pistas con {} en {}", "✅ {} tracks with {} in {}", pistas.len(), gotv_core::generadores::repo_del_separador(modelo), gotv_core::mensajes::duracion(t0.elapsed().as_secs_f32())));
    Ok(())
}

/// Ola EXPERTO · `gotv rehacer <pista> <desde> <hasta> "descripción" [--letra-archivo f] [--idioma es]`:
/// rehace con IA un tramo de una pista del estudio Experto de ESTA carpeta (sin el modelo de chat).
fn rehacer_cmd(args: &[String]) -> Result<()> {
    let (mut letra, mut idioma, mut libres) = (String::new(), String::new(), Vec::new());
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--letra-archivo" | "--lyrics-file" => letra = std::fs::read_to_string(it.next().ok_or_else(|| anyhow::anyhow!("--letra-archivo"))?)?, // juez-idioma: datos (nombre de la bandera)
            "--idioma" | "--language" => idioma = it.next().cloned().unwrap_or_default(),
            _ => libres.push(a.clone()),
        }
    }
    let uso = || anyhow::anyhow!("{}", gotv_core::i18n::t("uso: gotv rehacer <pista> <desde> <hasta> \"descripción en inglés\" [--letra-archivo f] [--idioma es]", "usage: gotv redo <track> <from> <to> \"description\" [--lyrics-file f] [--language en]"));
    if libres.len() < 4 {
        return Err(uso());
    }
    let (pista, desde, hasta): (u32, f64, f64) = (libres[0].parse().map_err(|_| uso())?, libres[1].parse().map_err(|_| uso())?, libres[2].parse().map_err(|_| uso())?);
    let descripcion = libres[3..].join(" ");
    let root = std::env::current_dir()?;
    let mut s = gotv_core::experto::cargar(&root);
    let t0 = std::time::Instant::now();
    let cancelar = std::sync::atomic::AtomicBool::new(false);
    gotv_core::experto::rehacer_tramo(&root, &mut s, pista, desde, hasta, &descripcion, &letra, &idioma, &|t| eprint!("\r{}   ", tr!("rehaciendo · {t}", "redoing · {t}")), &cancelar)?;
    gotv_core::experto::guardar(&root, &s)?;
    eprintln!();
    println!("{}", tr!("✅ pista {pista}, {desde}–{hasta} s rehecha en {}", "✅ track {pista}, {desde}–{hasta} s redone in {}", gotv_core::mensajes::duracion(t0.elapsed().as_secs_f32())));
    Ok(())
}
