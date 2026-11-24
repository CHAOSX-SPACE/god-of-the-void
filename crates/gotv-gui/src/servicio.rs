//! A4 en la app: GOD OF THE VOID como servicio de API (OpenAI + Ollama) con un
//! interruptor. El modelo que sirve es SIEMPRE el que la app tiene cargado (el
//! worker lo actualiza en cada `publish_status`); abrirlo a la red pide la llave
//! del Llavero (se crea la primera vez).

use crate::Hub;
use gotv_core::serve;
use serde_json::{json, Value};

/// Un servicio corriendo: cómo pararlo y dónde escucha.
pub struct Running {
    stop: tokio::sync::oneshot::Sender<()>,
    pub host: String,
    pub port: u16,
}

/// Enciende (o reenciende con otra config) el servicio.
pub async fn start(hub: &Hub, cfg: &gotv_core::Config) -> Result<(), String> {
    stop(hub).await;
    let host = if cfg.serve_lan { "0.0.0.0" } else { "127.0.0.1" }.to_string();
    let key = if cfg.serve_lan {
        tokio::task::spawn_blocking(|| serve::stored_key(true)).await.map_err(|e| e.to_string())?.map_err(|e| e.to_string())?
    } else {
        None
    };
    serve::check_bind(&host, key.as_deref()).map_err(|e| e.to_string())?;
    hub.api.set_key(key);
    // los de nube sólo si el usuario lo pidió (cada petición gasta SUS créditos)
    if cfg.serve_cloud {
        let (api, c) = (hub.api.clone(), cfg.clone());
        tokio::spawn(async move { api.set_cloud(serve::cloud_upstreams(&c).await) });
    } else {
        hub.api.set_cloud(vec![]);
    }
    let listener = serve::bind(&host, cfg.serve_port).await.map_err(|e| e.to_string())?;
    let port = listener.local_addr().map(|a| a.port()).unwrap_or(cfg.serve_port);
    let (tx, rx) = tokio::sync::oneshot::channel();
    tokio::spawn(serve::run(listener, hub.api.clone(), rx));
    *hub.api_run.lock().await = Some(Running { stop: tx, host, port });
    Ok(())
}

pub async fn stop(hub: &Hub) {
    if let Some(r) = hub.api_run.lock().await.take() {
        let _ = r.stop.send(());
    }
}

/// Lo que la pestaña API muestra.
pub async fn status(hub: &Hub) -> Value {
    let run = hub.api_run.lock().await;
    let models: Vec<String> = hub.api.models().into_iter().map(|u| u.id).collect();
    let hits = hub.api.hits.load(std::sync::atomic::Ordering::Relaxed);
    match run.as_ref() {
        None => json!({"running": false, "models": models, "hits": hits}),
        Some(r) => {
            let lan = !serve::is_loopback(&r.host);
            let mut urls = vec![];
            if lan {
                urls.extend(serve::lan_addresses().into_iter().map(|ip| format!("http://{ip}:{}", r.port)));
            }
            urls.push(format!("http://127.0.0.1:{}", r.port));
            // la llave es TUYA y de tu servicio: la app la muestra (oculta hasta que la pidas)
            let key = if lan { serve::stored_key(false).ok().flatten() } else { None };
            json!({"running": true, "lan": lan, "port": r.port, "urls": urls, "key": key, "models": models, "hits": hits})
        }
    }
}
