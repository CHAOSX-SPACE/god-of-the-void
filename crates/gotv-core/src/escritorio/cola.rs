//! F16.1 · Escribir mientras el agente trabaja.
//!
//! Lo que el Portador escribe con el agente ocupado NO espera a que termine ni lo detiene: se **encola** por
//! chat (la clave es la de sus procesos, `Agent::proc_owner`) y entra en el bucle ENTRE dos pasos, antes de
//! la siguiente llamada al motor, como [`Message::aviso`](gotv_engine::Message::aviso) (F0.10, punto 8: un
//! mensaje de usuario nuevo obliga al motor local a releer la conversación entera).
//!
//! **Trampa 1 de la ficha:** nunca entre una llamada a herramienta y su resultado. [`protocolo_cerrado`] lo
//! comprueba con la conversación misma: si el último turno de la IA pidió herramientas y alguna aún no tiene
//! su resultado, el mensaje espera al paso siguiente.
//!
//! Lo que llega cuando el turno ya está cerrando (después del último paso) no se pierde: quien lleva el turno
//! (la ventana) lo recoge con [`sobrantes`] y lo manda como la orden siguiente.
//!
//! Es la misma puerta de los eventos de F3: los dos entran en el mismo punto del bucle y del mismo modo.
use gotv_engine::Message;
use std::collections::{HashMap, VecDeque};
use std::sync::{Mutex, OnceLock};

/// Tope de mensajes en cola por chat (uno que escribe sin parar no llena la memoria).
pub const TOPE: usize = 20;
/// Tope de caracteres de un mensaje encolado (lo demás se adjunta como archivo, como siempre).
pub const TEXTO_MAX: usize = 20_000;

#[derive(Debug, Clone, PartialEq)]
pub struct Encolado {
    pub id: u64,
    pub texto: String,
    /// de dónde vino: «ventana» · «teléfono» · …
    pub origen: String,
}

#[derive(Default)]
struct Chat {
    cola: VecDeque<Encolado>,
    /// mensajes que ENTRARON a mitad de la orden en curso (lo mira `/objetivo` para cortar su ronda)
    entraron: u32,
}

fn registro() -> &'static Mutex<HashMap<String, Chat>> {
    static R: OnceLock<Mutex<HashMap<String, Chat>>> = OnceLock::new();
    R.get_or_init(Default::default)
}

fn siguiente_id() -> u64 {
    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    N.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

/// Encola `texto` para el chat `dueno`. Devuelve su id (para quitarlo antes de que entre) o el porqué no.
pub fn encolar(dueno: &str, texto: &str, origen: &str) -> Result<u64, String> {
    let t = texto.trim();
    if t.is_empty() {
        return Err(crate::i18n::t("el mensaje está vacío", "the message is empty").into());
    }
    let mut r = registro().lock().unwrap();
    let chat = r.entry(dueno.to_string()).or_default();
    if chat.cola.len() >= TOPE {
        return Err(crate::tr!(
            "ya hay {TOPE} mensajes esperando: espera a que entren",
            "there are already {TOPE} messages waiting: wait for them to go in"
        ));
    }
    let texto: String = t.chars().take(TEXTO_MAX).collect();
    let id = siguiente_id();
    chat.cola.push_back(Encolado { id, texto, origen: origen.into() });
    Ok(id)
}

/// Quita un mensaje que aún no entró (el Portador se arrepintió). `true` si estaba.
pub fn quitar(dueno: &str, id: u64) -> bool {
    let mut r = registro().lock().unwrap();
    let Some(chat) = r.get_mut(dueno) else { return false };
    let antes = chat.cola.len();
    chat.cola.retain(|e| e.id != id);
    chat.cola.len() != antes
}

/// Lo que espera, sin sacarlo (para pintarlo en la ventana).
pub fn pendientes(dueno: &str) -> Vec<Encolado> {
    registro().lock().unwrap().get(dueno).map(|c| c.cola.iter().cloned().collect()).unwrap_or_default()
}

/// ¿El último turno de la IA tiene ya el resultado de TODAS sus llamadas? Sólo entonces puede entrar algo
/// más en la conversación sin romper el protocolo de herramientas.
pub fn protocolo_cerrado(mensajes: &[Message]) -> bool {
    let Some(i) = mensajes.iter().rposition(|m| m.role == "assistant") else { return true };
    let Some(llamadas) = &mensajes[i].tool_calls else { return true };
    let resueltas: std::collections::HashSet<&str> =
        mensajes[i + 1..].iter().filter(|m| m.role == "tool").filter_map(|m| m.tool_call_id.as_deref()).collect();
    llamadas.iter().filter_map(|c| c["id"].as_str()).all(|id| resueltas.contains(id))
}

/// El aviso con lo que escribió el Portador (uno o varios mensajes juntos, en orden).
pub fn aviso(mensajes: &[Encolado]) -> String {
    let mut s = String::new();
    for m in mensajes {
        if !s.is_empty() {
            s.push_str("\n\n");
        }
        s.push_str(&crate::tr!(
            "[Mensaje del usuario escrito mientras trabajabas ({})]\n{}",
            "[Message from the user written while you were working ({})]\n{}",
            m.origen,
            m.texto
        ));
    }
    s.push_str("\n\n");
    s.push_str(crate::i18n::t(
        "(Lo escribió AHORA, a mitad de la tarea, y no te detuvo. Léelo antes de seguir: si cambia lo que hay que hacer, cambia el rumbo; si es un dato o una corrección, aplícalo; si no toca lo que haces, sigue.)",
        "(They wrote this NOW, mid-task, and did not stop you. Read it before going on: if it changes what has to be done, change course; if it is a fact or a correction, apply it; if it does not touch what you are doing, carry on.)",
    ));
    s
}

/// Para el bucle: si hay mensajes y el protocolo está cerrado, los saca y devuelve el aviso. `None` si no
/// hay nada o si aún falta el resultado de alguna llamada (esperan al paso siguiente).
pub fn tomar(dueno: &str, mensajes: &[Message]) -> Option<(String, Vec<Encolado>)> {
    if !protocolo_cerrado(mensajes) {
        return None;
    }
    let mut r = registro().lock().unwrap();
    let chat = r.get_mut(dueno)?;
    if chat.cola.is_empty() {
        return None;
    }
    let sacados: Vec<Encolado> = chat.cola.drain(..).collect();
    chat.entraron += sacados.len() as u32;
    Some((aviso(&sacados), sacados))
}

/// Al cerrar el turno: lo que llegó después del último paso, para mandarlo como la orden siguiente.
/// Varios mensajes van juntos, en orden, separados por una línea en blanco.
pub fn sobrantes(dueno: &str) -> Option<String> {
    let mut r = registro().lock().unwrap();
    let chat = r.get_mut(dueno)?;
    if chat.cola.is_empty() {
        return None;
    }
    let textos: Vec<String> = chat.cola.drain(..).map(|e| e.texto).collect();
    Some(textos.join("\n\n"))
}

/// Cuántos mensajes entraron a mitad de orden desde la última vez que se preguntó (y lo pone a cero).
/// `/objetivo` lo usa para cortar su ronda: el Portador tomó el volante.
pub fn entraron(dueno: &str) -> u32 {
    let mut r = registro().lock().unwrap();
    r.get_mut(dueno).map(|c| std::mem::take(&mut c.entraron)).unwrap_or(0)
}

/// El chat se cierra: lo suyo se olvida.
pub fn soltar(dueno: &str) {
    registro().lock().unwrap().remove(dueno);
}

/// Enganche del bucle del agente (`run_loop`, antes de cada llamada al motor del agente principal).
/// `true` si entró algo.
pub async fn entre_pasos<F: crate::frontend::Frontend>(front: &F, dueno: &str, mensajes: &mut Vec<Message>) -> bool {
    let Some((aviso, sacados)) = tomar(dueno, mensajes) else { return false };
    for m in &sacados {
        let corto: String = m.texto.lines().next().unwrap_or("").chars().take(80).collect();
        front.log(crate::tr!("  ✉ entró tu mensaje: {corto}", "  ✉ your message went in: {corto}")).await;
        front.activity("mensaje_entro", &m.id.to_string());
    }
    mensajes.push(Message::aviso(aviso));
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use gotv_engine::ToolCall;

    fn llamada(id: &str) -> ToolCall {
        ToolCall { id: id.into(), name: "read_file".into(), arguments: serde_json::json!({"path": "a"}) }
    }

    #[test]
    fn el_protocolo_abierto_retiene_el_mensaje() {
        let d = "/p#cola-protocolo";
        soltar(d);
        let (a, b) = (llamada("c1"), llamada("c2"));
        let mut m = vec![Message::user("haz algo"), Message::assistant_calls("", &[a.clone(), b.clone()]), Message::tool_result(&a, "x")];
        encolar(d, "cambia de rumbo", "ventana").unwrap();
        assert!(tomar(d, &m).is_none(), "falta el resultado de c2: no puede entrar");
        m.push(Message::tool_result(&b, "y"));
        let (aviso, sacados) = tomar(d, &m).expect("con todo resuelto entra");
        assert_eq!(sacados.len(), 1);
        assert!(aviso.contains("cambia de rumbo"));
        assert!(tomar(d, &m).is_none(), "se sacó una sola vez");
        assert_eq!(entraron(d), 1);
        assert_eq!(entraron(d), 0, "se pone a cero al preguntar");
    }

    #[test]
    fn quitar_sobrantes_y_topes() {
        let d = "/p#cola-topes";
        soltar(d);
        assert!(encolar(d, "   ", "ventana").is_err());
        let id = encolar(d, "uno", "ventana").unwrap();
        encolar(d, "dos", "teléfono").unwrap();
        assert_eq!(pendientes(d).len(), 2);
        assert!(quitar(d, id));
        assert!(!quitar(d, id));
        assert_eq!(sobrantes(d).as_deref(), Some("dos"));
        assert!(sobrantes(d).is_none());
        for i in 0..TOPE {
            encolar(d, &format!("m{i}"), "ventana").unwrap();
        }
        assert!(encolar(d, "uno más", "ventana").is_err());
        soltar(d);
        assert!(pendientes(d).is_empty());
    }
}
