//! Pruebas de F7 (artefactos). Sin red y sin modelo: el agente corre con un motor de GUION (sus
//! respuestas están escritas aquí) y un frente que hace de usuario. La prueba con el navegador real
//! (`#[ignore]`) necesita agent-browser, Chrome y ffmpeg, no un modelo. Todo este archivo es código de
//! prueba (`#[cfg(test)]` en `artefactos/mod.rs`): el juez de idioma no lo mira.

use super::comentarios::{self, Comentario};
use super::recorrido::{self, Cambio};
use super::sitios::{self, Lista, Sitios};
use super::*;
use crate::checkpoint::Checkpoint;
use crate::frontend::Frontend;
use crate::permissions::{AskAnswer, Decision, Mode, Permissions};
use crate::Agent;
use anyhow::Result;
use async_trait::async_trait;
use gotv_engine::{Capabilities, ChatRequest, ChatResponse, Engine, ToolCall};
use serde_json::{json, Value};
use std::sync::Mutex;

// ── motor de guion y frente que hace de usuario ──────────────────────────────────────────────────

pub(crate) struct Guion {
    respuestas: Mutex<Vec<ChatResponse>>,
}

#[async_trait]
impl Engine for Guion {
    fn name(&self) -> String {
        "guion".into()
    }
    fn capabilities(&self) -> Capabilities {
        Capabilities { text: true, vision: false, audio_in: false }
    }
    async fn start(&mut self) -> Result<()> {
        Ok(())
    }
    async fn complete(&self, _req: ChatRequest) -> Result<ChatResponse> {
        let mut r = self.respuestas.lock().unwrap();
        Ok(if r.is_empty() { texto("fin") } else { r.remove(0) })
    }
    async fn stop(&mut self) -> Result<()> {
        Ok(())
    }
}

type AlElegir = Box<dyn Fn(&[Value]) -> Option<String> + Send + Sync>;

/// El «usuario»: guarda lo que se le muestra y contesta las preguntas con lo que diga la prueba
/// (que puede comentar el plan antes de contestar, como el Portador en la ventana).
#[derive(Default)]
pub(crate) struct Usuario {
    vistas: Mutex<Vec<Value>>,
    preguntas: Mutex<Vec<String>>,
    registro: Mutex<Vec<String>>,
    al_elegir: Mutex<Vec<AlElegir>>,
}

#[async_trait]
impl Frontend for Usuario {
    async fn log(&self, l: String) {
        self.registro.lock().unwrap().push(l);
    }
    async fn ask(&self, _d: &str) -> AskAnswer {
        AskAnswer::Yes
    }
    fn activity(&self, kind: &str, detail: &str) {
        if kind == "vista" {
            self.vistas.lock().unwrap().push(serde_json::from_str(detail).unwrap());
        }
    }
    async fn choose(&self, q: &str, _o: &[String]) -> Option<String> {
        self.preguntas.lock().unwrap().push(q.to_string());
        let mut cola = self.al_elegir.lock().unwrap();
        if cola.is_empty() {
            return None;
        }
        let f = cola.remove(0);
        let vistas = self.vistas.lock().unwrap().clone();
        f(&vistas)
    }
}

fn texto(t: &str) -> ChatResponse {
    ChatResponse { content: t.into(), finish_reason: "stop".into(), ..Default::default() }
}

fn llamadas(cs: &[(&str, Value)]) -> ChatResponse {
    ChatResponse {
        finish_reason: "tool_calls".into(),
        tool_calls: cs.iter().enumerate().map(|(i, (n, a))| ToolCall { id: format!("c{i}"), name: n.to_string(), arguments: a.clone() }).collect(),
        ..Default::default()
    }
}

pub(crate) struct Prueba<E: Engine = Guion> {
    pub(crate) a: Agent<E, Usuario>,
    _raiz: crate::CarpetaPrueba,
}

fn agente(tag: &str, modo: Mode, respuestas: Vec<ChatResponse>) -> Prueba {
    agente_con(tag, modo, Guion { respuestas: Mutex::new(respuestas) })
}

/// El mismo agente de prueba con cualquier motor (el de guion, o el modelo REAL por `gotv serve`).
pub(crate) fn agente_con<E: Engine>(tag: &str, modo: Mode, engine: E) -> Prueba<E> {
    let raiz = crate::CarpetaPrueba::nueva(&format!("gotv-f7-{tag}"));
    let root = raiz.to_path_buf();
    let a = Agent {
        engine,
        perms: Permissions::new(modo, &root, vec![], vec![], vec![]),
        checkpoint: Some(Checkpoint::new(&root)),
        root,
        max_steps: 8,
        max_tokens: 512,
        sampling: gotv_engine::Sampling::GREEDY,
        context_budget: 32768,
        front: Usuario::default(),
        history: vec![],
        session: None,
        cancel: Default::default(),
        mcp: None,
        attachments: Vec::new(),
        user_media: Vec::new(),
        chat_only: false,
        embedder: None,
        lsp: None,
        computer_use: false,
        procs: Default::default(),
        tools_loaded: Default::default(),
        bases_nube: Default::default(),
        guardia: Default::default(),
        perfil: Default::default(),
        pro: Default::default(),
        motor_dormido: false,
        visible: None,
        estado_de: None,
        calibre: None,
        recorto_ubicaciones: false,
        tope_paso: 0,
    };
    Prueba { a, _raiz: raiz }
}

fn chat_del<E: Engine>(a: &Agent<E, Usuario>) -> Chat {
    chat_de(&a.root, None, &a.proc_owner())
}

fn resultados<E: Engine>(a: &Agent<E, Usuario>, herramienta: &str) -> Vec<String> {
    let ids: Vec<String> = a
        .history
        .iter()
        .filter_map(|m| m.tool_calls.as_ref())
        .flatten()
        .filter(|c| c["function"]["name"].as_str().or(c["name"].as_str()) == Some(herramienta))
        .filter_map(|c| c["id"].as_str().map(String::from))
        .collect();
    a.history
        .iter()
        .filter(|m| m.role == "tool" && m.tool_call_id.as_ref().is_some_and(|i| ids.contains(i)))
        .map(|m| m.content.clone())
        .collect()
}

// ── paso 1 · escribir, versionar y listar ────────────────────────────────────────────────────────

#[test]
fn tres_escrituras_dejan_tres_versiones_en_el_historial() {
    let raiz = crate::CarpetaPrueba::nueva("gotv-f7-versiones");
    let chat = Chat::new(&raiz, "7");
    assert_eq!(chat.escribir(Doc::Plan, "uno").unwrap(), 1);
    assert_eq!(chat.escribir(Doc::Plan, "dos").unwrap(), 2);
    assert_eq!(chat.escribir(Doc::Plan, "tres").unwrap(), 3);
    let h = raiz.join(".gotv/chats/7/historial");
    let mut nombres: Vec<String> = std::fs::read_dir(&h).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into()).collect();
    nombres.sort();
    assert_eq!(nombres, ["1-plan.md", "2-plan.md", "3-plan.md"]);
    assert_eq!(chat.leer(Doc::Plan).as_deref(), Some("tres"));
    assert_eq!(chat.version(Doc::Plan, 1).as_deref(), Some("uno"));
    // el mismo texto otra vez no es una versión nueva
    assert_eq!(chat.escribir(Doc::Plan, "tres").unwrap(), 3);
    assert_eq!(chat.versiones(Doc::Plan), [1, 2, 3]);
    // los otros dos documentos llevan su propia cuenta
    assert_eq!(chat.escribir(Doc::Tareas, "- [ ] a").unwrap(), 1);
    let l = chat.listar();
    assert_eq!(l.iter().map(|r| (r.archivo, r.versiones.len())).collect::<Vec<_>>(), [("plan.md", 3), ("tareas.md", 1)]);
}

#[test]
fn el_historial_tiene_tope_y_borra_solo_sus_versiones_viejas() {
    let raiz = crate::CarpetaPrueba::nueva("gotv-f7-tope");
    let chat = Chat::new(&raiz, "1");
    // un archivo AJENO en el historial no se toca (L6: sólo rutas exactas propias)
    std::fs::create_dir_all(raiz.join(".gotv/chats/1/historial")).unwrap();
    std::fs::write(raiz.join(".gotv/chats/1/historial/notas-del-usuario.md"), "mío").unwrap();
    for i in 0..(MAX_VERSIONES + 5) {
        chat.escribir(Doc::Plan, &format!("versión {i}")).unwrap();
    }
    let v = chat.versiones(Doc::Plan);
    assert_eq!(v.len(), MAX_VERSIONES);
    assert_eq!(v.first(), Some(&6));
    assert!(raiz.join(".gotv/chats/1/historial/notas-del-usuario.md").exists());
}

#[test]
fn la_clave_del_chat_no_escapa_de_su_carpeta() {
    let raiz = crate::CarpetaPrueba::nueva("gotv-f7-clave");
    let c = Chat::new(&raiz, "../../etc");
    assert!(c.dir().starts_with(raiz.join(".gotv/chats")));
    assert!(!c.clave().contains('/') && !c.clave().contains(".."));
    assert_eq!(clave_chat(Some(12), "x#y"), "12");
    assert_eq!(clave_chat(None, "/proy#chat-3"), "sin-sesion-chat-3");
}

// ── paso 3 · comentarios ─────────────────────────────────────────────────────────────────────────

#[test]
fn el_comentario_sigue_a_su_linea_y_si_desaparece_queda_huerfano() {
    let raiz = crate::CarpetaPrueba::nueva("gotv-f7-reubicar");
    let chat = Chat::new(&raiz, "1");
    chat.escribir(Doc::Plan, "1. leer\n2. editar app.rs\n3. probar").unwrap();
    let c = comentarios::comentar(&chat, "plan.md", 2, "  usa main.rs  ").unwrap();
    assert_eq!((c.linea, c.texto_linea.as_str(), c.texto.as_str(), c.version), (Some(2), "2. editar app.rs", "usa main.rs", 1));
    // se inserta una línea arriba: el comentario baja con su texto
    chat.escribir(Doc::Plan, "0. mirar\n1. leer\n2. editar app.rs\n3. probar").unwrap();
    assert_eq!(comentarios::todos(&chat)[0].linea, Some(3));
    // la línea desaparece: huérfano, no perdido
    chat.escribir(Doc::Plan, "0. mirar\n1. leer\n3. probar").unwrap();
    let c = &comentarios::todos(&chat)[0];
    assert_eq!(c.linea, None);
    assert!(!c.resuelto);
    let m = comentarios::para_el_modelo(&chat).unwrap();
    assert!(m.contains("«2. editar app.rs»") && m.contains("usa main.rs"), "{m}");
    // con dos coincidencias, la más cercana a donde estaba
    let lineas = ["a", "x", "b", "c", "x"];
    assert_eq!(comentarios::ubicar(&lineas, Some(4), "x"), Some(5));
    assert_eq!(comentarios::ubicar(&lineas, Some(1), "x"), Some(2));
}

#[test]
fn un_comentario_resuelto_no_se_reenvia() {
    let raiz = crate::CarpetaPrueba::nueva("gotv-f7-resuelto");
    let chat = Chat::new(&raiz, "1");
    chat.escribir(Doc::Plan, "1. a\n2. b").unwrap();
    let a = comentarios::comentar(&chat, "plan.md", 1, "primero").unwrap();
    let b = comentarios::comentar(&chat, "plan.md", 2, "segundo").unwrap();
    comentarios::resolver(&chat, a.id, true).unwrap();
    let m = comentarios::para_el_modelo(&chat).unwrap();
    assert!(m.starts_with(&crate::tr!("[Comentarios del usuario sobre plan.md]", "[The user's comments on plan.md]")), "{m}");
    assert!(m.contains("L2 («2. b»): segundo") && !m.contains("primero"), "{m}");
    // ya entregado: no se repite
    assert_eq!(comentarios::para_el_modelo(&chat), None);
    // reabrir uno resuelto lo vuelve a entregar
    comentarios::resolver(&chat, a.id, false).unwrap();
    let m = comentarios::para_el_modelo(&chat).unwrap();
    assert!(m.contains("primero") && !m.contains("segundo"), "{m}");
    // la versión nueva del plan resuelve lo que ya se entregó
    chat.escribir(Doc::Plan, "1. a\n2. b\n3. c").unwrap();
    assert!(comentarios::todos(&chat).iter().all(|c| c.resuelto), "{:?}", comentarios::todos(&chat));
    assert_eq!(comentarios::para_el_modelo(&chat), None);
    let _ = b;
}

#[test]
fn comentar_valida_documento_linea_y_texto() {
    let raiz = crate::CarpetaPrueba::nueva("gotv-f7-validar");
    let chat = Chat::new(&raiz, "1");
    assert!(comentarios::comentar(&chat, "plan.md", 1, "x").is_err(), "sin plan aún");
    chat.escribir(Doc::Plan, "una").unwrap();
    assert!(comentarios::comentar(&chat, "secreto.txt", 1, "x").is_err());
    assert!(comentarios::comentar(&chat, "plan.md", 2, "x").is_err());
    assert!(comentarios::comentar(&chat, "plan.md", 0, "x").is_err());
    assert!(comentarios::comentar(&chat, "plan.md", 1, "   ").is_err());
    let largo = "a".repeat(comentarios::MAX_CHARS + 50);
    assert_eq!(comentarios::comentar(&chat, "plan.md", 1, &largo).unwrap().texto.chars().count(), comentarios::MAX_CHARS);
}

// ── el modelo no escribe lo de la conversación ni la lista de sitios ─────────────────────────────

#[test]
fn el_modelo_no_escribe_comentarios_ni_planes_ni_la_lista_de_sitios() {
    let raiz = crate::CarpetaPrueba::nueva("gotv-f7-protegido");
    let p = Permissions::new(Mode::Bypass, &raiz, vec!["write_file".into(), "bash".into()], vec![], vec![]);
    let niega = |t: &str, a: Value| matches!(p.check(t, &a), Decision::Deny(_));
    assert!(niega("write_file", json!({"path": ".gotv/chats/1/comentarios.gotv", "content": "[]"})));
    assert!(niega("edit", json!({"path": ".gotv/chats/1/recorrido.md", "old": "a", "new": "b"})));
    assert!(niega("write_file", json!({"path": ".GOTV/Chats/1/plan.md", "content": "x"})), "APFS no distingue mayúsculas");
    assert!(niega("write_file", json!({"path": "src/../.gotv/navegador", "content": "abrir permitir *"})));
    assert!(niega("bash", json!({"cmd": "echo 'abrir permitir *' > .gotv/navegador"})));
    assert!(!niega("write_file", json!({"path": "src/navegador/mod.rs", "content": "x"})), "el código de un módulo llamado navegador sí se edita");
    assert!(niega("bash", json!({"cmd": "cp x .gotv/chats/1/comentarios.gotv"})));
    // leer sí; y el resto del proyecto sigue libre
    assert!(!niega("read_file", json!({"path": ".gotv/chats/1/plan.md"})));
    assert!(!niega("bash", json!({"cmd": "cat .gotv/chats/1/plan.md"})));
    assert!(!niega("write_file", json!({"path": "src/main.rs", "content": "fn main(){}"})));
}

// ── paso 2 + 3 · el «hecho cuando» del plan ──────────────────────────────────────────────────────

/// El usuario comenta dos líneas del plan y pide cambios; el agente lo rehace (versión 2) sin tocar
/// archivos; los comentarios le llegan marcados como del usuario y una sola vez.
#[tokio::test]
async fn el_usuario_comenta_dos_lineas_y_el_agente_rehace_el_plan_sin_tocar_archivos() {
    let plan1 = "1. Crear src/boton.css\n2. Pintar el botón de rojo\n3. Probarlo con el navegador";
    let plan2 = "1. Crear src/boton.css\n2. Pintar el botón de azul #1a5fff\n3. Probarlo con el navegador y grabar un video";
    let mut p = agente("plan-comentado", Mode::Plan, vec![
        llamadas(&[("exit_plan", json!({"plan": plan1}))]),
        llamadas(&[("exit_plan", json!({"plan": plan2}))]),
        texto("Plan rehecho con tus dos comentarios."),
    ]);
    let no = crate::i18n::t("No, sigue planeando", "No, keep planning").to_string();
    let root = p.a.root.clone();
    let no1 = no.clone();
    p.a.front.al_elegir.lock().unwrap().push(Box::new(move |vistas: &[Value]| {
        // la ventana: la tarjeta del plan trae su conversación; el usuario comenta dos líneas y pide cambios
        let v = vistas.last().unwrap();
        assert_eq!((v["tipo"].as_str(), v["version"].as_u64()), (Some("plan"), Some(1)));
        let chat = Chat::new(&root, v["sesion"].as_str().unwrap());
        comentarios::comentar(&chat, "plan.md", 2, "mejor azul, el de la marca").unwrap();
        comentarios::comentar(&chat, "plan.md", 3, "graba un video de la prueba").unwrap();
        Some(no1.clone())
    }));
    p.a.front.al_elegir.lock().unwrap().push(Box::new(move |_v: &[Value]| Some(no.clone())));
    let r = p.a.run("planea el cambio de color del botón").await.unwrap();
    assert_eq!(r, "Plan rehecho con tus dos comentarios.");
    let res = resultados(&p.a, "exit_plan");
    assert_eq!(res.len(), 2, "{res:?}");
    let cabecera = crate::tr!("[Comentarios del usuario sobre plan.md]", "[The user's comments on plan.md]");
    assert!(res[0].contains(&cabecera), "{}", res[0]);
    assert!(res[0].contains("L2 («2. Pintar el botón de rojo»): mejor azul, el de la marca"), "{}", res[0]);
    assert!(res[0].contains("L3 («3. Probarlo con el navegador»): graba un video de la prueba"), "{}", res[0]);
    assert!(!res[1].contains(&cabecera), "se entregan una sola vez: {}", res[1]);
    // dos versiones del plan; los comentarios quedaron atendidos por la segunda
    let chat = chat_del(&p.a);
    assert_eq!(chat.versiones(Doc::Plan), [1, 2]);
    assert_eq!(chat.leer(Doc::Plan).as_deref(), Some(plan2));
    let cs: Vec<Comentario> = comentarios::todos(&chat);
    assert_eq!(cs.len(), 2);
    assert!(cs.iter().all(|c| c.resuelto && c.entregado));
    // sin tocar archivos: ningún paso de restauración y ningún recorrido
    assert!(p.a.checkpoint.as_ref().unwrap().pasos_desde(None).is_empty());
    assert!(chat.leer(Doc::Recorrido).is_none());
    assert_eq!(p.a.perms.mode, Mode::Plan, "pidió cambios: sigue en modo plan");
    // la ventana vio las dos versiones y la pregunta exacta (para contestar desde la tarjeta)
    let vistas = p.a.front.vistas.lock().unwrap().clone();
    assert_eq!(vistas.iter().map(|v| v["version"].as_u64().unwrap()).collect::<Vec<_>>(), [1, 2]);
    assert_eq!(vistas[0]["pregunta"].as_str(), p.a.front.preguntas.lock().unwrap().first().map(String::as_str));
    assert_eq!(vistas[1]["comentarios"].as_array().unwrap().len(), 2);
}

/// Medido con Huihui (2026-11-18): en modo plan dio el plan como TEXTO, sin `exit_plan`. GOTV lo presenta igual:
/// tarjeta comentable, comentarios de vuelta, versión 2; y si nadie responde, el texto queda como respuesta.
#[tokio::test]
async fn un_plan_dado_como_texto_se_presenta_y_se_comenta() {
    let plan1 = "1. Cambiar el fondo de .comprar\n2. Probarlo";
    let plan2 = "1. Cambiar el fondo de .comprar a #1a5fff\n2. Probarlo y grabar un video";
    let mut p = agente("plan-texto", Mode::Plan, vec![texto(plan1), texto(plan2)]);
    let root = p.a.root.clone();
    let no = crate::i18n::t("No, sigue planeando", "No, keep planning").to_string();
    p.a.front.al_elegir.lock().unwrap().push(Box::new(move |vistas: &[Value]| {
        let v = vistas.last().unwrap();
        assert_eq!((v["tipo"].as_str(), v["version"].as_u64()), (Some("plan"), Some(1)));
        let chat = Chat::new(&root, v["sesion"].as_str().unwrap());
        comentarios::comentar(&chat, "plan.md", 1, "el azul de la marca: #1a5fff").unwrap();
        Some(no.clone())
    }));
    // a la versión 2 nadie responde: queda como la respuesta final
    let r = p.a.run("planea el cambio de color").await.unwrap();
    assert_eq!(r.trim(), plan2);
    let chat = chat_del(&p.a);
    assert_eq!(chat.versiones(Doc::Plan), [1, 2]);
    assert!(comentarios::todos(&chat).iter().all(|c| c.entregado));
    assert!(p.a.checkpoint.as_ref().unwrap().pasos_desde(None).is_empty());
    assert_eq!(p.a.perms.mode, Mode::Plan);
}

/// El mismo «hecho cuando» con un MODELO REAL (Huihui por `gotv serve`): el usuario comenta dos líneas del plan
/// (un color exacto y «graba un video»), el modelo rehace el plan recogiéndolas y no toca ningún archivo.
/// `GOTV_PRUEBA_OPENAI=http://127.0.0.1:11437/v1 GOTV_PRUEBA_MODELO=<nombre> cargo test -p gotv-core --release
///  --lib -- --ignored --nocapture plan_comentado_con_el_modelo_real`
#[tokio::test]
#[ignore]
async fn plan_comentado_con_el_modelo_real() {
    let (Ok(url), Ok(modelo)) = (std::env::var("GOTV_PRUEBA_OPENAI"), std::env::var("GOTV_PRUEBA_MODELO")) else { return };
    let motor = gotv_engine::cloud::OpenAiEngine::new("gotv", &url, String::new(), &modelo, false);
    let mut p = agente_con("plan-real", Mode::Plan, motor);
    p.a.max_steps = 20;
    p.a.max_tokens = 4096;
    let html = "<!doctype html><button class=\"comprar\">Comprar</button>\n<style>.comprar{background:red;color:white}</style>\n";
    std::fs::write(p.a.root.join("index.html"), html).unwrap();
    // el «usuario»: ante el PRIMER plan comenta dos líneas y pide cambios; ante otra pregunta (el modelo
    // real puede preguntar antes de planear), le dice que decida con lo que sabe; luego sigue sin aprobar
    let root = p.a.root.clone();
    let no = crate::i18n::t("No, sigue planeando", "No, keep planning").to_string();
    let comentado = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    for _ in 0..8 {
        let (root, no, comentado) = (root.clone(), no.clone(), comentado.clone());
        p.a.front.al_elegir.lock().unwrap().push(Box::new(move |vistas: &[Value]| {
            let Some(v) = vistas.last().filter(|v| v["tipo"].as_str() == Some("plan")) else {
                return Some("Decide tú con lo que sabes y preséntame el plan.".into());
            };
            if comentado.swap(true, std::sync::atomic::Ordering::Relaxed) {
                return Some(no.clone());
            }
            let chat = Chat::new(&root, v["sesion"].as_str().unwrap());
            let n = chat.leer(Doc::Plan).map(|t| t.lines().count()).unwrap_or(1).max(1);
            eprintln!("— plan v1 —\n{}", chat.leer(Doc::Plan).unwrap_or_default());
            comentarios::comentar(&chat, "plan.md", 1, "el botón debe ser del azul de la marca: #1a5fff").unwrap();
            comentarios::comentar(&chat, "plan.md", n.min(2), "y al final graba un video de la prueba en el navegador").unwrap();
            Some(no.clone())
        }));
    }
    let r = p.a.run("Planea cómo cambiar el color del botón de index.html. Todavía no cambies nada: primero quiero ver el plan.").await.unwrap();
    let chat = chat_del(&p.a);
    let plan = chat.leer(Doc::Plan).unwrap_or_default();
    eprintln!("— plan final (versiones {:?}) —\n{plan}\n— respuesta —\n{r}", chat.versiones(Doc::Plan));
    assert!(chat.versiones(Doc::Plan).len() >= 2, "rehizo el plan");
    let p2 = plan.to_lowercase();
    assert!(p2.contains("1a5fff") && p2.contains("video"), "la versión nueva recoge los dos comentarios: {plan}");
    assert!(comentarios::todos(&chat).iter().all(|c| c.entregado), "los comentarios le llegaron");
    // sin tocar archivos
    assert_eq!(std::fs::read_to_string(p.a.root.join("index.html")).unwrap(), html);
    assert!(p.a.checkpoint.as_ref().unwrap().pasos_desde(None).is_empty(), "ningún paso de restauración");
    assert_eq!(p.a.perms.mode, Mode::Plan);
}

/// Un comentario hecho entre dos órdenes llega con la orden siguiente, antes de que el modelo actúe.
#[tokio::test]
async fn el_comentario_llega_con_la_orden_siguiente() {
    let mut p = agente("comentario-orden", Mode::Bypass, vec![texto("visto")]);
    let chat = chat_del(&p.a);
    chat.escribir(Doc::Plan, "1. a\n2. b").unwrap();
    comentarios::comentar(&chat, "plan.md", 2, "b no, c").unwrap();
    p.a.run("sigue").await.unwrap();
    let orden = p.a.history.iter().find(|m| m.role == "user").unwrap().content.clone();
    assert!(orden.contains("L2 («2. b»): b no, c"), "{orden}");
    // y no se repite en la siguiente
    p.a.engine.respuestas.lock().unwrap().push(texto("ok"));
    p.a.run("otra").await.unwrap();
    let ultima = p.a.history.iter().rev().find(|m| m.role == "user").unwrap().content.clone();
    assert!(!ultima.contains("b no, c"), "{ultima}");
}

// ── paso 5 · recorrido ───────────────────────────────────────────────────────────────────────────

#[test]
fn el_cambio_neto_ignora_lo_creado_y_borrado_en_el_mismo_turno() {
    use crate::checkpoint::{Change, Step};
    let c = |p: &str, b: Option<&str>, a: Option<&str>| Change { path: p.into(), before: b.map(Into::into), after: a.map(Into::into) };
    let pasos = vec![
        Step { id: 0, label: "1".into(), changes: vec![c("nuevo.rs", None, Some("h1")), c("tmp.txt", None, Some("h2")), c("viejo.rs", Some("h3"), Some("h4"))] },
        Step { id: 1, label: "2".into(), changes: vec![c("tmp.txt", Some("h2"), None), c("viejo.rs", Some("h4"), Some("h3")), c("fuera.rs", Some("h5"), None), c("otro.rs", Some("h6"), Some("h7"))] },
    ];
    assert_eq!(
        recorrido::cambios_netos(&pasos),
        [("fuera.rs".to_string(), Cambio::Borrado), ("nuevo.rs".into(), Cambio::Nuevo), ("otro.rs".into(), Cambio::Cambiado)]
    );
}

fn archivos_del_recorrido(t: &str) -> Vec<String> {
    let desde = t.find("\n## ").and_then(|i| t[i + 4..].find("\n## ").map(|j| i + 4 + j)).unwrap();
    let hasta = t[desde + 4..].find("\n## ").map_or(t.len(), |k| desde + 4 + k);
    t[desde..hasta]
        .lines()
        .filter(|l| l.starts_with("- `"))
        .map(|l| l.split('`').nth(3).unwrap().to_string())
        .collect()
}

/// El recorrido lista EXACTAMENTE los archivos del changeset del turno: lo que escribió el agente, lo que
/// hizo la terminal (crear y borrar) y nada más (ni lo que no se tocó, ni lo de turnos anteriores).
#[tokio::test]
async fn el_recorrido_lista_exactamente_los_archivos_del_changeset() {
    let mut p = agente("recorrido", Mode::Bypass, vec![
        llamadas(&[("write_file", json!({"path": "a.txt", "content": "uno"})), ("write_file", json!({"path": "src/b.txt", "content": "dos"}))]),
        llamadas(&[("bash", json!({"cmd": "echo tres > c.txt && rm z.txt"}))]),
        texto("Escribí a y b, la terminal creó c y borró z. Lo comprobé con ls."),
    ]);
    std::fs::write(p.a.root.join("z.txt"), "se va").unwrap();
    std::fs::write(p.a.root.join("y.txt"), "se queda igual").unwrap();
    // un turno anterior que ya tocó un archivo: no entra en el recorrido de este
    std::fs::write(p.a.root.join("previo.txt"), "0").unwrap();
    let cp = p.a.checkpoint.as_ref().unwrap();
    let pend = cp.begin().unwrap();
    std::fs::write(p.a.root.join("previo.txt"), "1").unwrap();
    cp.commit(pend, "turno anterior").unwrap();

    p.a.run("haz los archivos").await.unwrap();
    let chat = chat_del(&p.a);
    let t = chat.leer(Doc::Recorrido).expect("hay recorrido");
    let mut archivos = archivos_del_recorrido(&t);
    archivos.sort();
    assert_eq!(archivos, ["a.txt", "c.txt", "src/b.txt", "z.txt"], "{t}");
    assert!(t.contains("Lo comprobé con ls"), "{t}");
    assert!(t.contains(crate::i18n::t("`+` `c.txt` (nuevo)", "`+` `c.txt` (new)")) && t.contains("`−` `z.txt`"), "{t}");
    assert!(p.a.front.registro.lock().unwrap().iter().any(|l| l.contains("recorrido.md")));
    // un turno que no toca archivos no deja otro recorrido
    p.a.engine.respuestas.lock().unwrap().push(texto("nada que cambiar"));
    p.a.run("¿qué hiciste?").await.unwrap();
    assert_eq!(chat.versiones(Doc::Recorrido), [1]);
}

#[tokio::test]
async fn la_lista_de_tareas_es_un_documento() {
    let mut p = agente("tareas", Mode::Bypass, vec![
        llamadas(&[("todo", json!({"items": [{"content": "leer app.py", "status": "completed"}, {"content": "correr pruebas", "status": "pending"}]}))]),
        texto("listo"),
    ]);
    p.a.run("planea").await.unwrap();
    let t = chat_del(&p.a).leer(Doc::Tareas).expect("tareas.md");
    assert!(t.contains("leer app.py") && t.contains("correr pruebas"), "{t}");
}

// ── paso 6 · grabación ───────────────────────────────────────────────────────────────────────────

#[test]
fn record_stop_da_una_tarjeta_de_video_con_su_ruta_y_nada_en_base64() {
    let raiz = crate::CarpetaPrueba::nueva("gotv-f7-video");
    let dueno = "gotv-f7-video#chat";
    let a = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    let mut salida = String::from("✓ Recording started");
    assert!(grabacion::tras_browser(dueno, &raiz, &a(&["record", "start", "prueba.webm", "--fps", "30"]), &mut salida).is_none());
    assert!(salida.contains("prueba.webm"));
    std::fs::write(raiz.join("prueba.webm"), [0x1a, 0x45, 0xdf, 0xa3]).unwrap();
    let mut salida = String::from("✓ Recording saved");
    let v = grabacion::tras_browser(dueno, &raiz, &a(&["record", "stop"]), &mut salida).expect("tarjeta de video");
    assert_eq!(v["tipo"], "video");
    assert_eq!(v["ruta"].as_str(), Some(raiz.join("prueba.webm").to_str().unwrap()));
    assert!(!v.to_string().contains("base64") && v.as_object().unwrap().keys().all(|k| k != "video" && k != "datos"));
    assert!(salida.contains("prueba.webm") && !salida.contains("data:"));
    // la captura y el video quedan para el recorrido del turno
    let mut s = format!("✓ Screenshot saved to {}", raiz.join("captura.png").display());
    std::fs::write(raiz.join("captura.png"), b"png").unwrap();
    grabacion::tras_browser(dueno, &raiz, &a(&["screenshot", "captura.png"]), &mut s);
    let medios = recorrido::tomar_medios(dueno);
    assert_eq!(medios, [raiz.join("prueba.webm"), raiz.join("captura.png")]);
    let t = recorrido::armar(&raiz, "hecho", &[("index.html".into(), Cambio::Cambiado)], &medios, "hoy");
    assert!(t.contains("video: `prueba.webm`") && t.contains("`captura.png`"), "{t}");
    // parar sin grabación ni archivo: nada
    let mut s = String::new();
    assert!(grabacion::tras_browser(dueno, &raiz, &a(&["record", "stop"]), &mut s).is_none());
}

#[test]
fn la_ficha_del_navegador_dice_como_grabar() {
    let p = agente("ficha", Mode::Default, vec![]);
    let todas = p.a.all_tool_schemas().to_string();
    assert!(todas.contains(r#"[\"record\",\"start\","#) && todas.contains(r#"[\"record\",\"stop\"]"#), "la ficha de browser nombra record start/stop");
}

// ── paso 7 · sitios del navegador ────────────────────────────────────────────────────────────────

fn args(v: &[&str]) -> Value {
    json!({ "args": v })
}

fn con_sitios(tag: &str, s: Sitios) -> crate::CarpetaPrueba {
    let raiz = crate::CarpetaPrueba::nueva(&format!("gotv-f7-sitios-{tag}"));
    s.guardar(&raiz).unwrap();
    raiz
}

/// El formato propio de la lista de sitios (texto, nada de JSON): comentarios y líneas vacías fuera, varias
/// por regla, ida y vuelta exacta; lo que no se entiende es un error con su línea (falla cerrado).
#[test]
fn la_lista_de_sitios_en_texto() {
    let s = Sitios::desde_texto("# mis sitios\n\nabrir permitir docs.rs crates.io\nabrir negar facebook.com  # nunca\ndescargar negar *\n").unwrap();
    assert_eq!(s.abrir.permitir, ["docs.rs", "crates.io"]);
    assert_eq!((s.abrir.negar.clone(), s.descargar.negar.clone()), (vec!["facebook.com".to_string()], vec!["*".to_string()]));
    assert_eq!(Sitios::desde_texto(&s.a_texto()).unwrap(), s);
    for mal in ["abrir prohibir x.io", "bajar negar x.io", "abrir negar", "{\"abrir\": {}}"] {
        assert!(Sitios::desde_texto(mal).is_err(), "{mal}");
    }
    assert!(Sitios::desde_texto("x\nabrir negar").unwrap_err().to_string().contains("1"));
}

/// Los comentarios del usuario vuelven idénticos de disco, en el formato propio (cabecera GOTV, no JSON).
#[test]
fn los_comentarios_se_guardan_en_formato_propio() {
    let d = crate::CarpetaPrueba::nueva("gotv-f7-comentarios-formato");
    let chat = Chat::new(&d, "s1");
    chat.escribir(Doc::Plan, "uno\ndos\ntres\n").unwrap();
    comentarios::comentar(&chat, "plan.md", 2, "más corto, ñandú").unwrap();
    let crudo = std::fs::read(chat.dir().join("comentarios.gotv")).unwrap();
    assert_eq!(&crudo[..4], b"GOTV");
    let v = comentarios::todos(&chat);
    assert_eq!((v.len(), v[0].linea, v[0].texto.as_str(), v[0].texto_linea.as_str()), (1, Some(2), "más corto, ñandú", "dos"));
}

#[test]
fn dominios_y_patrones() {
    assert_eq!(sitios::dominio("https://www.Facebook.com:443/x?y").as_deref(), Some("www.facebook.com"));
    assert_eq!(sitios::dominio("http://user@evil.com/").as_deref(), Some("evil.com"));
    assert_eq!(sitios::dominio("file:///tmp/a.html"), None);
    assert!(sitios::casa("facebook.com", "www.facebook.com") && sitios::casa("facebook.com", "facebook.com"));
    assert!(!sitios::casa("facebook.com", "notfacebook.com"));
    assert!(sitios::casa("*", "a.b") && sitios::casa("*.x.com", "a.x.com") && sitios::casa("https://x.com/ruta", "x.com"));
    let a = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    assert_eq!(sitios::dominios_que_abre(&a(&["open", "https://a.com"])), ["a.com"]);
    assert_eq!(sitios::dominios_que_abre(&a(&["tab", "new", "b.com/x"])), ["b.com"]);
    assert_eq!(sitios::dominios_que_abre(&a(&["record", "start", "v.webm", "https://c.com"])), ["c.com"]);
    assert!(sitios::dominios_que_abre(&a(&["click", "@e3"])).is_empty());
    assert!(sitios::es_descarga(&a(&["download", "@e5", "x.pdf"])) && sitios::es_descarga(&a(&["wait", "--download", "x.pdf"])));
    assert!(!sitios::es_descarga(&a(&["wait", "--text", "Listo"])));
}

/// Un sitio en «negar» no se abre ni en bypass ni con un permiso guardado para `browser`.
#[test]
fn un_sitio_negado_no_se_abre_ni_en_bypass() {
    let raiz = con_sitios("negar", Sitios { abrir: Lista { permitir: vec![], negar: vec!["facebook.com".into()] }, ..Default::default() });
    let p = Permissions::new(Mode::Bypass, &raiz, vec!["browser".into()], vec![], vec![]);
    for orden in [&["open", "https://www.facebook.com/login"][..], &["goto", "facebook.com"], &["tab", "new", "http://m.facebook.com"], &["record", "start", "v.webm", "https://facebook.com"]] {
        match p.check("browser", &args(orden)) {
            Decision::Deny(r) => assert!(r.contains("facebook.com"), "{r}"),
            d => panic!("{orden:?} → {d:?}"),
        }
    }
    // lo demás sigue como siempre (en bypass, abrir la web pasa)
    assert!(matches!(p.check("browser", &args(&["open", "https://docs.rs"])), Decision::Allow));
    assert!(matches!(p.check("browser", &args(&["open", "http://localhost:5173"])), Decision::Allow));
}

/// «permitir» para abrir quita la pregunta de abrir, pero NO la de descargar (es una lista aparte).
#[test]
fn abrir_permitido_no_salta_el_permiso_de_descarga() {
    let raiz = con_sitios("permitir", Sitios { abrir: Lista { permitir: vec!["docs.rs".into()], negar: vec![] }, ..Default::default() });
    let mut p = Permissions::new(Mode::Default, &raiz, vec![], vec![], vec![]);
    assert!(matches!(p.check("browser", &args(&["open", "https://docs.rs/serde"])), Decision::Allow));
    assert!(matches!(p.check("browser", &args(&["open", "https://crates.io"])), Decision::Ask(_)), "fuera de la lista: pregunta como siempre");
    p.sitio_browser = Some("docs.rs".into());
    match p.check("browser", &args(&["download", "@e5", "manual.pdf"])) {
        Decision::Ask(d) => assert!(d.contains("docs.rs") && d.contains("manual.pdf"), "{d}"),
        d => panic!("la descarga debe preguntar: {d:?}"),
    }
    // en bypass la pregunta se vuelve sí (decisión del Portador); en plan, nada se descarga
    p.mode = Mode::Bypass;
    assert!(matches!(p.check("browser", &args(&["download", "@e5", "manual.pdf"])), Decision::Allow));
    p.mode = Mode::Plan;
    assert!(matches!(p.check("browser", &args(&["wait", "--download", "x.pdf"])), Decision::Deny(_)));
}

#[test]
fn la_lista_de_descargas_manda() {
    let raiz = con_sitios("descargar", Sitios {
        abrir: Lista::default(),
        descargar: Lista { permitir: vec!["github.com".into()], negar: vec!["mega.nz".into()] },
    });
    let mut p = Permissions::new(Mode::Bypass, &raiz, vec!["browser".into()], vec![], vec![]);
    p.sitio_browser = Some("mega.nz".into());
    assert!(matches!(p.check("browser", &args(&["download", "@e1", "a.zip"])), Decision::Deny(_)), "negada también en bypass");
    p.mode = Mode::Default;
    p.sitio_browser = Some("objects.github.com".into());
    assert!(matches!(p.check("browser", &args(&["download", "@e1", "a.zip"])), Decision::Allow), "permitida: sin pregunta");
    // negar todo
    Sitios { descargar: Lista { permitir: vec![], negar: vec!["*".into()] }, ..Default::default() }.guardar(&raiz).unwrap();
    p.sitio_browser = None;
    assert!(matches!(p.check("browser", &args(&["download", "@e1", "a.zip"])), Decision::Deny(_)));
}

#[test]
fn una_lista_rota_falla_cerrada() {
    let raiz = crate::CarpetaPrueba::nueva("gotv-f7-sitios-rota");
    std::fs::create_dir_all(raiz.join(".gotv")).unwrap();
    std::fs::write(raiz.join(sitios::ARCHIVO), "{ \"abrir\": { \"negar\": [\"x.com\" ").unwrap();
    let p = Permissions::new(Mode::Bypass, &raiz, vec![], vec![], vec![]);
    assert!(matches!(p.check("browser", &args(&["open", "https://y.com"])), Decision::Deny(_)));
    // lo que no abre ni descarga sigue funcionando (leer la página que ya está abierta)
    assert!(matches!(p.check("browser", &args(&["snapshot", "-i"])), Decision::Allow));
}

/// Llegar a un sitio prohibido por un clic (no por dirección): GOTV lo ve después del comando, cierra la
/// página sin devolver su contenido y lo dice. Y antes de una descarga mira la página REAL.
#[tokio::test]
async fn un_clic_que_llega_a_un_sitio_negado_se_cierra_sin_leerlo() {
    let raiz = con_sitios("clic", Sitios {
        abrir: Lista { permitir: vec![], negar: vec!["facebook.com".into()] },
        descargar: Lista { permitir: vec![], negar: vec!["mega.nz".into()] },
    });
    let dueno = "gotv-f7-clic#chat";
    let corridas: Mutex<Vec<Vec<String>>> = Mutex::new(vec![]);
    let url = Mutex::new("https://www.facebook.com/home".to_string());
    let ejecutar = |a: Vec<String>| {
        corridas.lock().unwrap().push(a.clone());
        let u = url.lock().unwrap().clone();
        async move { Ok::<_, anyhow::Error>(if a[0] == "get" { u } else { "ok".into() }) }
    };
    let a = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    let r = sitios::despues(dueno, &raiz, &a(&["click", "@e7"]), &ejecutar, Ok("contenido de la página".into())).await;
    let e = r.unwrap_err().to_string();
    assert!(e.contains("facebook.com") && !e.contains("contenido de la página"), "{e}");
    assert_eq!(corridas.lock().unwrap().last().unwrap(), &a(&["open", "about:blank"]));
    assert_eq!(sitios::pagina(dueno), None);
    // en un sitio permitido, el resultado pasa tal cual y la página queda anotada
    *url.lock().unwrap() = "https://mega.nz/file/x".into();
    let r = sitios::despues(dueno, &raiz, &a(&["click", "@e8"]), &ejecutar, Ok("ok".into())).await;
    assert_eq!(r.unwrap(), "ok");
    assert_eq!(sitios::pagina(dueno).as_deref(), Some("mega.nz"));
    // antes de descargar se mira la página real: mega.nz está en «negar» para descargar
    assert!(sitios::antes(dueno, &raiz, &a(&["download", "@e2", "x.zip"]), &ejecutar).await.is_err());
    assert!(sitios::antes(dueno, &raiz, &a(&["click", "@e2"]), &ejecutar).await.is_ok());
    // sin lista, ni una llamada de más
    let sin = crate::CarpetaPrueba::nueva("gotv-f7-sitios-sin");
    let antes = corridas.lock().unwrap().len();
    assert_eq!(sitios::despues(dueno, &sin, &a(&["click", "@e1"]), &ejecutar, Ok("x".into())).await.unwrap(), "x");
    assert_eq!(corridas.lock().unwrap().len(), antes);
}

// ── «hecho cuando», con el navegador DE VERDAD (agent-browser + Chrome + ffmpeg; sin modelo) ─────

/// Una corrección visual de punta a punta: el agente (guionado) arregla el color de un botón, abre la
/// página en el navegador propio, graba un video, toma una captura y para la grabación. Al cerrar el
/// turno hay un recorrido con el archivo cambiado, la captura y el video, y la ventana recibió la
/// tarjeta de video con su ruta. `cargo test … -- --ignored recorrido_con_captura_y_video_de_verdad`
#[tokio::test]
#[ignore]
async fn recorrido_con_captura_y_video_de_verdad() {
    if crate::browser::binary().is_none() {
        eprintln!("no aplica: falta agent-browser");
        return;
    }
    let mut p = agente("navegador-real", Mode::Bypass, vec![]);
    let root = p.a.root.clone();
    std::fs::write(root.join("index.html"), "<!doctype html><button style=\"background:red\">Pagar</button>").unwrap();
    let url = format!("file://{}", root.join("index.html").display());
    *p.a.engine.respuestas.lock().unwrap() = vec![
        llamadas(&[("edit", json!({"path": "index.html", "old": "background:red", "new": "background:#1a5fff"}))]),
        llamadas(&[("browser", json!({"args": ["open", url]}))]),
        llamadas(&[("browser", json!({"args": ["record", "start", "prueba.webm"]}))]),
        llamadas(&[("browser", json!({"args": ["wait", "800"]}))]),
        llamadas(&[("browser", json!({"args": ["screenshot", "captura.png"]}))]),
        llamadas(&[("browser", json!({"args": ["record", "stop"]}))]),
        llamadas(&[("browser", json!({"args": ["close"]}))]),
        texto("El botón ya es azul #1a5fff; lo comprobé en el navegador (captura y video)."),
    ];
    p.a.max_steps = 12;
    let t0 = std::time::Instant::now();
    p.a.run("pinta el botón de azul y pruébalo").await.unwrap();
    let res = resultados(&p.a, "browser");
    eprintln!("{:.1} s · browser: {res:#?}", t0.elapsed().as_secs_f32());
    let video = root.join("prueba.webm");
    let tam = std::fs::metadata(&video).map(|m| m.len()).unwrap_or(0);
    eprintln!("video: {} bytes", tam);
    assert!(tam > 1000, "el video existe y tiene contenido: {res:?}");
    let vistas = p.a.front.vistas.lock().unwrap().clone();
    assert!(vistas.iter().any(|v| v["tipo"] == "video" && v["ruta"].as_str() == video.to_str()), "{vistas:?}");
    let t = chat_del(&p.a).leer(Doc::Recorrido).expect("recorrido");
    eprintln!("{t}");
    assert!(t.contains("`index.html`") && t.contains("captura.png") && t.contains("prueba.webm"), "{t}");
}
