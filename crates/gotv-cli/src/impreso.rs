//! F10.2 · `gotv -p "<tarea>"`: una orden sin nadie a quien preguntar, para un guion, un CI o las tareas
//! programadas. Lo que habría que preguntar se NIEGA y la orden se corta ahí.
//! La salida estándar lleva SÓLO la respuesta; el registro va a stderr.
//! Códigos de salida: 0 hecho · 1 falló · 2 falta un permiso · 3 interrumpido.
use anyhow::Result;
use gotv_core::flujo::SinVentana;
use gotv_core::{Checkpoint, Config};
use gotv_engine::Engine;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

pub async fn correr(cfg: &mut Config, root: std::path::PathBuf, task: String, resume: bool) -> Result<()> {
    // nadie se queda esperando: lo programado en el chat no correría (la herramienta lo dice al crearlo)
    gotv_core::programar::chat::sin_espera();
    let registro: gotv_core::flujo::Escritor = Box::new(|l: &str| eprintln!("{l}"));
    let t = std::time::Instant::now();
    let engine: gotv_engine::AnyEngine = if gotv_core::nube::is_cloud(&cfg.model, cfg) {
        let (e, _nombre, notas) = gotv_core::nube::launch(cfg).await?;
        for n in notas {
            eprintln!("· {n}");
        }
        e
    } else {
        match super::start_motor(cfg, false, false).await {
            Ok(e) => e,
            Err(e) => {
                // sin motor no hay orden: código 1
                eprintln!("✗ {e:#}");
                std::process::exit(gotv_core::flujo::Estado::Fallo.codigo());
            }
        }
    };
    let perms = super::perms_from(cfg, &root);
    let (session, history) = match resume.then(|| gotv_core::session::Session::resume_last(&root, None)).transpose()?.flatten() {
        Some((s, h)) => (Some(s), h),
        None => (gotv_core::session::Session::open_new(&root, &cfg.model).ok(), vec![]),
    };
    // el primer Ctrl+C interrumpe (código 3); un permiso que falte también corta la orden (código 2)
    let cancel = Arc::new(AtomicBool::new(false));
    {
        let flag = cancel.clone();
        tokio::spawn(async move {
            while tokio::signal::ctrl_c().await.is_ok() {
                if flag.swap(true, Ordering::Relaxed) {
                    gotv_core::exec::kill_all();
                    std::process::exit(130);
                }
            }
        });
    }
    let front = SinVentana::new(Some(registro), Some(cancel.clone()));
    let checkpoint = Some(Checkpoint::new(&root));
    let mut agent = super::armar_agente(cfg, root, engine, front, perms, checkpoint, history, session, cancel);
    let r = agent.run(&task).await;
    agent.cerrar_sesion("other").await; // F10.1: SessionEnd
    let res = agent.front.resultado(&r, t.elapsed().as_secs_f32());
    match (&res.respuesta, &res.error) {
        (Some(a), _) => println!("{a}"),
        (None, Some(e)) => eprintln!("✗ {e}"),
        _ => {}
    }
    eprintln!("{}", gotv_core::tr!("— {:.1} s · código {}", "— {:.1} s · code {}", res.segundos, res.codigo()));
    let _ = agent.engine.stop().await;
    gotv_core::exec::kill_all();
    std::process::exit(res.codigo());
}
