//! F10.3 · `gotv acp`: GOTV como agente de un editor que habla el Agent Client Protocol (Zed, y los que lo
//! adopten), por stdio. El protocolo vive en `gotv_core::acp`; aquí sólo se arma cada chat con la
//! configuración de la CLI. El motor se carga UNA vez, al abrir el primer chat (el saludo no espera al
//! modelo), y lo comparten todos los chats del editor. La salida estándar ES el canal: todo lo demás va a
//! stderr.
use anyhow::Result;
use gotv_core::acp::{Compartido, Fabrica, FrenteAcp};
use gotv_core::{Checkpoint, Config};
use std::sync::Arc;
use tokio::sync::OnceCell;

/// `gotv acp [--guion <archivo>]`. Con `--guion`, el motor es un guion de texto (ver `gotv_core::acp::guion`):
/// para probar un editor de verdad sin cargar ningún modelo.
pub async fn correr(cfg: Config, args: &[String]) -> Result<()> {
    let cfg = Arc::new(tokio::sync::Mutex::new(cfg));
    let motor: Arc<OnceCell<Compartido<gotv_engine::AnyEngine>>> = Arc::new(OnceCell::new());
    if let Some(i) = args.iter().position(|a| a == "--guion" || a == "--script") {
        let ruta = args.get(i + 1).ok_or_else(|| gotv_engine::anyhow_tr!("uso: gotv acp --guion <archivo>", "usage: gotv acp --guion <file>"))?;
        let m = gotv_core::acp::guion::MotorGuion::de_texto(&std::fs::read_to_string(ruta)?)?;
        eprintln!("{}", gotv_core::tr!("· ACP con motor de GUION ({ruta}): sin modelo", "· ACP with a SCRIPT engine ({ruta}): no model"));
        let _ = motor.set(Compartido::new(gotv_engine::AnyEngine::Otro(Box::new(m))));
    }
    let fabrica: Fabrica<Compartido<gotv_engine::AnyEngine>> = Arc::new(move |cwd: std::path::PathBuf, front: FrenteAcp| {
        let (cfg, motor) = (cfg.clone(), motor.clone());
        Box::pin(async move {
            // la configuración del PROYECTO que abrió el editor (sus permisos, su modo), como en la ventana
            let mut base = cfg.lock().await.clone();
            let mut de_proyecto = Config::load(&cwd);
            for w in de_proyecto.warnings.drain(..) {
                eprintln!("⚠ {w}");
            }
            de_proyecto.model = base.model.clone();
            base = de_proyecto;
            let engine = motor
                .get_or_try_init(|| async {
                    let mut c = base.clone();
                    let e: gotv_engine::AnyEngine = if gotv_core::nube::is_cloud(&c.model, &c) {
                        let (e, _nombre, notas) = gotv_core::nube::launch(&mut c).await?;
                        for n in notas {
                            eprintln!("· {n}");
                        }
                        e
                    } else {
                        super::start_motor(&mut c, false, false).await?
                    };
                    anyhow::Ok(Compartido::new(e))
                })
                .await?
                .clone();
            let perms = super::perms_from(&base, &cwd);
            let session = gotv_core::session::Session::open_new(&cwd, &base.model).ok();
            let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
            let checkpoint = Some(Checkpoint::new(&cwd));
            Ok(super::armar_agente(&base, cwd, engine, front, perms, checkpoint, vec![], session, cancel))
        })
    });
    gotv_core::acp::servir(tokio::io::stdin(), tokio::io::stdout(), fabrica).await?;
    gotv_core::exec::kill_all();
    Ok(())
}
