//! F2.3 · `gotv programadas …`: las tareas programadas desde la terminal y desde el `LaunchAgent`.
//!   listar · crear --nombre N --cron "…" [--zona Z] [--modelo M] [--permiso R]… <orden>
//!   quitar <id> · pausar <id> · activar <id> · correr <id> (ya, con candado) · pendientes (lo que toca)
//!   agente [estado|plist|instalar|quitar]   (instalar y quitar PREGUNTAN siempre)
use anyhow::{bail, Result};
use gotv_core::flujo::SinVentana;
use gotv_core::programar::{chat::ahora, launchd, tareas};
use gotv_core::{tr, Config};
use gotv_engine::Engine;
use std::io::IsTerminal;
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

fn uso() -> &'static str {
    gotv_core::i18n::t(
        "uso: gotv programadas listar | crear --nombre N --cron \"*/5 * * * *\" [--zona America/Bogota] [--modelo M] [--permiso \"bash(cargo test)\"]… <orden> | quitar <id> | pausar <id> | activar <id> | correr <id> | pendientes | agente [estado|plist|instalar|quitar]",
        "usage: gotv programadas listar | crear --nombre N --cron \"*/5 * * * *\" [--zona America/Bogota] [--modelo M] [--permiso \"bash(cargo test)\"]… <orden> | quitar <id> | pausar <id> | activar <id> | correr <id> | pendientes | agente [estado|plist|instalar|quitar]",
    )
}

fn hora(t: Option<i64>) -> String {
    gotv_core::programar::hora_local(t)
}

/// Una pregunta de sí o no en la terminal (sin terminal: no).
fn confirmar(pregunta: &str) -> bool {
    if !std::io::stdin().is_terminal() {
        return false;
    }
    eprint!("{pregunta} [s/N] ");
    let mut l = String::new();
    std::io::stdin().read_line(&mut l).is_ok() && matches!(l.trim().to_lowercase().as_str(), "s" | "si" | "sí" | "y" | "yes") // juez-idioma: datos (respuestas que se aceptan)
}

pub async fn cmd(root: &Path, rest: &[String]) -> Result<()> {
    // una tarea programada corre y termina: lo que programe dentro de su chat no correría nunca
    gotv_core::programar::chat::sin_espera();
    let ruta = tareas::ruta_por_omision();
    match rest.first().map(String::as_str) {
        None | Some("listar" | "list") => {
            let r = tareas::Registro::cargar(&ruta)?;
            if r.tareas.is_empty() {
                println!("{}", gotv_core::i18n::t("(no hay tareas programadas)", "(no scheduled tasks)"));
            }
            for t in &r.tareas {
                let activa = if t.activa { "●" } else { "○" };
                println!("{activa} {:<10} {:<24} «{}» {} · {}", t.id, t.nombre, t.cron, if t.zona.is_empty() { "local" } else { &t.zona }, t.proyecto.display());
                println!("{}", tr!(
                    "    próxima {} · última {} · {} · permisos: {}",
                    "    next {} · last {} · {} · permissions: {}",
                    hora(t.proxima),
                    hora(t.ultima),
                    t.estado.as_deref().unwrap_or("—"),
                    if t.permisos.is_empty() { "—".to_string() } else { t.permisos.join(", ") }
                ));
            }
            Ok(())
        }
        Some("crear" | "create") => {
            let mut n = tareas::Nueva { proyecto: root.to_path_buf(), ..Default::default() };
            let mut orden = Vec::new();
            let mut it = rest[1..].iter();
            while let Some(a) = it.next() {
                let mut valor = || it.next().cloned().unwrap_or_default();
                match a.as_str() {
                    "--nombre" | "--name" => n.nombre = valor(),
                    "--cron" => n.cron = valor(),
                    "--zona" | "--tz" => n.zona = valor(),
                    "--modelo" | "--model" => n.modelo = valor(), // juez-idioma: datos (nombres de opción)
                    "--permiso" | "--permission" => n.permisos.push(valor()),
                    otra => orden.push(otra.to_string()),
                }
            }
            n.orden = orden.join(" ");
            if n.nombre.is_empty() || n.cron.is_empty() || n.orden.is_empty() {
                bail!("{}", uso());
            }
            let t = tareas::Registro::con(&ruta, |r| r.crear(n, ahora()))?;
            println!("{}", tr!("✅ tarea {} «{}» · próxima {}", "✅ task {} «{}» · next {}", t.id, t.nombre, hora(t.proxima)));
            if t.permisos.is_empty() {
                println!("{}", gotv_core::i18n::t(
                    "   sin `--permiso`: correrá sólo con lo que ya está concedido; lo demás la detendrá y te avisará",
                    "   no `--permiso`: it will run only with what is already granted; anything else stops it and notifies you",
                ));
            }
            // la primera tarea: el agente de launchd (cambia la configuración del sistema → pregunta)
            let casa = std::path::PathBuf::from(std::env::var("HOME").unwrap_or_default());
            if launchd::estado(&casa) == launchd::Estado::NoInstalado {
                let q = tr!(
                    "Para que corra con la app cerrada hace falta un agente de launchd ({}: lanza `gotv programadas pendientes` cada {} s). ¿Lo instalo?",
                    "To run with the app closed a launchd agent is needed ({}: runs `gotv programadas pendientes` every {} s). Install it?",
                    launchd::ruta_plist(&casa).display(),
                    launchd::INTERVALO
                );
                if confirmar(&q) {
                    println!("{}", launchd::instalar(true)?);
                } else {
                    println!("{}", gotv_core::i18n::t(
                        "   sin el agente, corre sólo con `gotv programadas pendientes` (o la app abierta). Instálalo cuando quieras: `gotv programadas agente instalar`",
                        "   without the agent it only runs with `gotv programadas pendientes` (or the app open). Install it whenever you like: `gotv programadas agente instalar`",
                    ));
                }
            }
            Ok(())
        }
        Some(accion @ ("quitar" | "remove" | "pausar" | "pause" | "activar" | "resume")) => {
            let Some(id) = rest.get(1) else { bail!("{}", uso()) };
            match accion {
                "quitar" | "remove" => {
                    let t = tareas::Registro::con(&ruta, |r| r.quitar(id))?;
                    println!("{}", tr!("tarea {} «{}» quitada", "task {} «{}» removed", t.id, t.nombre));
                }
                a => {
                    let si = matches!(a, "activar" | "resume");
                    tareas::Registro::con(&ruta, |r| r.activar(id, si, ahora()))?;
                    println!("{}", if si { tr!("tarea {id} activa", "task {id} active") } else { tr!("tarea {id} en pausa", "task {id} paused") });
                }
            }
            Ok(())
        }
        Some("correr" | "run") => {
            let Some(id) = rest.get(1) else { bail!("{}", uso()) };
            match correr_una(&ruta, id, true).await? {
                Some(c) => {
                    println!("{}", tr!("{} · código {}", "{} · code {}", c.estado, c.codigo()));
                    std::process::exit(c.codigo());
                }
                None => {
                    println!("{}", tr!("la tarea {id} ya está corriendo (o no existe)", "task {id} is already running (or does not exist)"));
                    Ok(())
                }
            }
        }
        Some("pendientes" | "due") => {
            // lo que lanza el agente de launchd cada 60 s: si no toca nada, sale sin cargar ningún modelo; si lo
            // lanzó launchd y su .plist ya se quitó, no hace nada (quitar vale al instante, sin `launchctl`)
            let casa = std::path::PathBuf::from(std::env::var("HOME").unwrap_or_default());
            if !launchd::vigente(std::env::var(launchd::MARCA).ok().as_deref(), &casa) {
                return Ok(());
            }
            let t = ahora();
            let ids = tareas::Registro::cargar(&ruta)?.pendientes(t);
            for id in ids {
                match correr_una(&ruta, &id, false).await {
                    Ok(Some(c)) => println!("[{}] {id}: {}", hora(Some(ahora())), c.estado),
                    Ok(None) => {}
                    Err(e) => eprintln!("[{}] {id}: {e:#}", hora(Some(ahora()))),
                }
            }
            Ok(())
        }
        Some("agente" | "agent") => {
            let casa = std::path::PathBuf::from(std::env::var("HOME").unwrap_or_default());
            match rest.get(1).map(String::as_str) {
                None | Some("estado" | "status") => {
                    println!("{}", match launchd::estado(&casa) {
                        launchd::Estado::NoInstalado => gotv_core::i18n::t("no instalado", "not installed").to_string(),
                        launchd::Estado::Instalado => tr!("instalado ({}; launchd lo carga al iniciar sesión)", "installed ({}; launchd loads it at login)", launchd::ruta_plist(&casa).display()),
                    });
                    Ok(())
                }
                Some("plist") => {
                    // sólo enseña lo que se instalaría (no escribe nada)
                    print!("{}", launchd::plist(&std::env::current_exe()?, gotv_core::entorno::path(), &casa.join(".cache/gotv/programadas.log")));
                    Ok(())
                }
                Some("instalar" | "install") => {
                    let q = tr!("Esto escribe {} y lo carga en launchd (cambia la configuración de tu Mac). ¿Sigo?", "This writes {} and loads it into launchd (changes your Mac's configuration). Continue?", launchd::ruta_plist(&casa).display());
                    println!("{}", launchd::instalar(confirmar(&q))?);
                    Ok(())
                }
                Some("quitar" | "remove") => {
                    let q = tr!("Esto descarga el agente de launchd y borra {}. ¿Sigo?", "This unloads the launchd agent and deletes {}. Continue?", launchd::ruta_plist(&casa).display());
                    println!("{}", launchd::quitar(confirmar(&q))?);
                    Ok(())
                }
                _ => bail!("{}", uso()),
            }
        }
        _ => bail!("{}", uso()),
    }
}

/// Corre UNA tarea con su modelo (cargado aquí y soltado al acabar), sin ventana y sólo con lo concedido.
async fn correr_una(ruta: &Path, id: &str, forzar: bool) -> Result<Option<tareas::Corrida>> {
    let notificador = gotv_core::avisos::Sistema;
    tareas::una_pendiente(
        ruta,
        id,
        ahora(),
        forzar,
        |t| async move {
            let fallo = |e: anyhow::Error| tareas::Corrida::fallida(&e);
            let mut cfg = Config::load(&t.proyecto);
            if !t.modelo.is_empty() {
                cfg.model = t.modelo.clone();
            }
            let engine = if gotv_core::nube::is_cloud(&cfg.model, &cfg) {
                match gotv_core::nube::launch(&mut cfg).await {
                    Ok((e, _, _)) => e,
                    Err(e) => return fallo(e),
                }
            } else {
                match super::start_motor(&mut cfg, false, false).await {
                    Ok(e) => e,
                    Err(e) => return fallo(e),
                }
            };
            let detener = Arc::new(AtomicBool::new(false));
            let front = SinVentana::new(None, Some(detener.clone()));
            let perms = super::perms_from(&cfg, &t.proyecto);
            let mut agent = super::armar_agente(&cfg, t.proyecto.clone(), engine, front, perms, Some(gotv_core::Checkpoint::new(&t.proyecto)), vec![], None, detener.clone());
            let c = tareas::correr_con(&mut agent, &t, &cfg, detener).await;
            agent.cerrar_sesion("other").await; // F10.1: SessionEnd
            let _ = agent.engine.stop().await;
            c
        },
        &notificador,
        &gotv_core::avisos::Interruptores::cargar(None),
    )
    .await
}
