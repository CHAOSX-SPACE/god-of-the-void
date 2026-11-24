//! F12 · las formas de trabajar en la ventana. La lógica vive en `gotv_core::formas`; aquí sólo se cose con
//! el turno de la ventana: cada ronda de `/objetivo` es un turno suyo (`run_order`: su tarea en el panel, el
//! stop que siempre gana, la conversación a salvo si el turno se cae), y entre ronda y ronda se mira si el
//! Portador mandó algo (entonces el objetivo se para y lo suyo va enseguida).
use crate::emit;
use crate::worker::{publish_status, run_order, Ag, Msg};
use crate::Hub;
use gotv_core::formas::objetivo::{Objetivo, Paso};
use gotv_core::formas::Forma;
use serde_json::json;
use tokio::sync::mpsc;

/// Atiende la orden de barra si es una forma de trabajar; `false` si no lo es (sigue su camino).
pub async fn atender(hub: &Hub, agent: &mut Ag, model: &str, task: &str, nombre: &str, arg: &str, rx: &mpsc::UnboundedReceiver<Msg>) -> bool {
    let Some(f) = gotv_core::formas::forma(nombre) else { return false };
    match f {
        Forma::Objetivo => objetivo(hub, agent, model, task, arg, rx).await,
        Forma::Entrevista => run_order(hub, agent, model, gotv_core::formas::orden_de_entrevista(arg), task.trim().to_string()).await,
        _ => {
            let r = gotv_core::formas::orden(agent, task, || false).await;
            publish_status(hub, agent, model).await;
            match r {
                Some(Ok(t)) => emit(&hub.app, json!({"kind": "done", "text": t, "info": f != Forma::Aprender})),
                Some(Err(e)) => emit(&hub.app, json!({"kind": "error", "text": e.to_string()})),
                None => return false,
            }
        }
    }
    true
}

/// `/objetivo`: ronda tras ronda, cada una un turno de la ventana, hasta cumplirse o un tope.
async fn objetivo(hub: &Hub, agent: &mut Ag, model: &str, task: &str, arg: &str, rx: &mpsc::UnboundedReceiver<Msg>) {
    let mut o = match Objetivo::nuevo(arg) {
        Ok(o) => o,
        Err(e) => return emit(&hub.app, json!({"kind": "error", "text": e.to_string()})),
    };
    emit(&hub.app, json!({"kind": "line", "text": o.cabecera(&agent.root)}));
    let (mut orden, mut visible) = o.primera_orden(task);
    let _ = gotv_core::escritorio::cola::entraron(&agent.proc_owner()); // F16.1: se cuenta desde aquí
    loop {
        let desde = agent.history.len();
        run_order(hub, agent, model, orden, visible).await;
        // lo que contestó la ronda: su último mensaje de la IA (si el turno se cayó, la conversación volvió a
        // como estaba y no hay respuesta nueva)
        let respuesta = agent.history.get(desde..).and_then(|h| h.last()).filter(|m| m.role == "assistant" && m.tool_calls.is_none()).map(|m| m.content.clone());
        let r = respuesta.as_deref().ok_or(gotv_core::i18n::t("el turno no terminó con una respuesta", "the turn did not end with an answer"));
        // F16.1: el Portador escribió A MITAD de la ronda (su mensaje ya entró en ella): el objetivo se corta aquí
        let escribio = !rx.is_empty() || gotv_core::escritorio::cola::entraron(&agent.proc_owner()) > 0;
        match o.tras_ronda(agent, r, desde, escribio).await {
            Paso::Seguir { orden: o2, visible: v2 } => {
                // la ronda siguiente se ve como lo que es (la manda GOTV, no el Portador) y la ventana queda ocupada
                emit(&hub.app, json!({"kind": "objetivo", "text": v2}));
                orden = o2;
                visible = v2;
            }
            Paso::Fin(informe) => {
                publish_status(hub, agent, model).await;
                return emit(&hub.app, json!({"kind": "done", "text": informe, "info": true}));
            }
        }
    }
}
