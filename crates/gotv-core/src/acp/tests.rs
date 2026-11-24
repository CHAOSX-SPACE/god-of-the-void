//! Un editor FALSO en Rust, por `tokio::io::duplex`: saludo, una orden con texto en vivo, una orden que edita
//! y pide permiso (con su diff, leyendo el búfer SIN GUARDAR), una cancelación a mitad, permisos que el editor
//! no contesta bien, la cola de órdenes y basura por la entrada. Todo este archivo es código de prueba
//! (`#[cfg(test)]` en `acp/mod.rs`): el juez de idioma no lo mira.
use super::*;
use crate::hooks_eventos::tests::{agente, llamadas, texto, Guion};
use crate::permissions::Mode;
use gotv_engine::ChatResponse;
use std::path::Path;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, DuplexStream, Lines};

struct Cliente {
    w: DuplexStream,
    r: Lines<BufReader<DuplexStream>>,
    n: u64,
    /// todo lo que llegó del agente
    vistos: Vec<Value>,
}

/// Cómo contesta el editor falso lo que el agente le pide.
type Respuestas = Box<dyn Fn(&Value) -> Value + Send>;

impl Cliente {
    async fn linea(&mut self, s: &str) {
        self.w.write_all(format!("{s}\n").as_bytes()).await.unwrap();
    }
    async fn pedir(&mut self, metodo: &str, params: Value) -> u64 {
        self.n += 1;
        let m = json!({"jsonrpc": "2.0", "id": self.n, "method": metodo, "params": params});
        self.linea(&m.to_string()).await;
        self.n
    }
    async fn avisar(&mut self, metodo: &str, params: Value) {
        self.linea(&json!({"jsonrpc": "2.0", "method": metodo, "params": params}).to_string()).await;
    }
    /// Lee hasta la respuesta a `id`, contestando con `resp` lo que el agente pida por el camino.
    async fn respuesta(&mut self, id: u64, resp: &Respuestas) -> Value {
        loop {
            // (120 s: con el modelo real, leer el contexto la primera vez lleva su rato)
            let l = tokio::time::timeout(Duration::from_secs(120), self.r.next_line()).await.expect("el agente no contestó").unwrap().expect("se cerró");
            let m: Value = serde_json::from_str(&l).unwrap_or_else(|e| panic!("stdout lleva algo que no es JSON ({e}): {l}"));
            assert!(!l.contains('\n'));
            self.vistos.push(m.clone());
            if m.get("method").is_some() && m.get("id").is_some() {
                let r = resp(&m);
                self.linea(&json!({"jsonrpc": "2.0", "id": m["id"], "result": r}).to_string()).await;
            } else if m["id"] == json!(id) && m.get("method").is_none() {
                return m;
            }
        }
    }
    fn actualizaciones(&self, tipo: &str) -> Vec<Value> {
        self.vistos.iter().filter(|m| m["method"] == "session/update" && m["params"]["update"]["sessionUpdate"] == tipo).map(|m| m["params"]["update"].clone()).collect()
    }
}

fn sin_peticiones() -> Respuestas {
    Box::new(|m| panic!("petición inesperada: {m}"))
}

/// Arranca el servidor con una fábrica que da agentes con el guion de cada sesión (en orden).
fn arrancar(modo: Mode, guiones: Vec<Vec<ChatResponse>>, demora: Duration) -> (Cliente, tokio::task::JoinHandle<Result<()>>) {
    let (cli_w, srv_r) = tokio::io::duplex(1 << 20);
    let (srv_w, cli_r) = tokio::io::duplex(1 << 20);
    let guiones = Arc::new(Mutex::new(VecDeque::from(guiones)));
    let fabrica: Fabrica<Guion> = Arc::new(move |cwd, front| {
        let g = guiones.lock().unwrap().pop_front().unwrap_or_default();
        Box::pin(async move {
            let mut a = agente(&cwd, modo, g, front);
            a.engine.demora = demora;
            // en GOTV editar dentro del proyecto no pregunta (lo deshace el checkpoint): una regla «pregunta
            // siempre» del usuario para `edit` hace que pregunte, como en la ventana
            a.perms = crate::permissions::Permissions::new(modo, &cwd, vec![], vec![], vec!["edit".into()]);
            Ok(a)
        })
    });
    let h = tokio::spawn(servir(srv_r, srv_w, fabrica));
    (Cliente { w: cli_w, r: BufReader::new(cli_r).lines(), n: 0, vistos: vec![] }, h)
}

async fn saludar(c: &mut Cliente, fs: bool) -> Value {
    let id = c.pedir("initialize", json!({"protocolVersion": 1, "clientCapabilities": {"fs": {"readTextFile": fs, "writeTextFile": fs}, "terminal": false}, "clientInfo": {"name": "editor-falso", "version": "0.1"}})).await;
    c.respuesta(id, &sin_peticiones()).await
}

async fn sesion(c: &mut Cliente, cwd: &Path) -> String {
    let id = c.pedir("session/new", json!({"cwd": cwd, "mcpServers": []})).await;
    let r = c.respuesta(id, &sin_peticiones()).await;
    r["result"]["sessionId"].as_str().unwrap_or_else(|| panic!("{r}")).to_string()
}

/// Saludo, capacidades y sesión nueva; una orden con texto en vivo; y basura por la entrada no tumba nada.
#[tokio::test]
async fn saludo_sesion_y_orden_con_texto_en_vivo() {
    let d = crate::CarpetaPrueba::nueva("gotv-acp-saludo");
    let root = d.canonicalize().unwrap();
    let (mut c, _h) = arrancar(Mode::Default, vec![vec![texto("Hola desde GOTV.")]], Duration::ZERO);
    c.linea("esto no es JSON").await;
    c.linea("{\"a\": 1").await;
    let r = saludar(&mut c, false).await;
    assert_eq!(r["result"]["protocolVersion"], 1);
    assert_eq!(r["result"]["authMethods"], json!([]));
    assert_eq!(r["result"]["agentCapabilities"]["loadSession"], false);
    assert_eq!(r["result"]["agentInfo"]["name"], "gotv");
    // una petición sin método y un método que no existe: errores de JSON-RPC, no caídas
    c.linea(&json!({"jsonrpc": "2.0", "id": 99, "foo": 1}).to_string()).await;
    let id = c.pedir("session/set_mode", json!({"sessionId": "x", "modeId": "bypass"})).await;
    assert_eq!(c.respuesta(id, &sin_peticiones()).await["error"]["code"], -32601);
    let id = c.pedir("session/new", json!({"cwd": "relativa", "mcpServers": []})).await;
    assert_eq!(c.respuesta(id, &sin_peticiones()).await["error"]["code"], -32602);
    let s = sesion(&mut c, &root).await;
    let id = c.pedir("session/prompt", json!({"sessionId": "no-existe", "prompt": [{"type": "text", "text": "x"}]})).await;
    assert_eq!(c.respuesta(id, &sin_peticiones()).await["error"]["code"], -32002);
    let id = c.pedir("session/prompt", json!({"sessionId": s, "prompt": [{"type": "text", "text": "hola"}]})).await;
    let r = c.respuesta(id, &sin_peticiones()).await;
    assert_eq!(r["result"]["stopReason"], "end_turn", "{r}");
    let texto: String = c.actualizaciones("agent_message_chunk").iter().map(|u| u["content"]["text"].as_str().unwrap_or("").to_string()).collect();
    assert!(texto.contains("Hola desde GOTV."), "{texto}");
}

/// «Un editor abre un chat de GOTV, se le pide un cambio, pinta el diff, acepta el permiso y el archivo
/// cambia», con un búfer SIN GUARDAR que manda sobre el disco.
#[tokio::test]
async fn una_orden_que_edita_pide_permiso_con_diff_y_el_archivo_cambia() {
    let d = crate::CarpetaPrueba::nueva("gotv-acp-edita");
    let root = d.canonicalize().unwrap();
    let archivo = root.join("saludo.txt");
    std::fs::write(&archivo, "hola mundo\n").unwrap();
    let guion = vec![llamadas(&[("edit", json!({"path": "saludo.txt", "old": "mundo", "new": "GOTV"}))]), texto("Cambiado.")];
    let (mut c, _h) = arrancar(Mode::Default, vec![guion], Duration::ZERO);
    saludar(&mut c, true).await;
    let s = sesion(&mut c, &root).await;
    // el editor tiene el archivo abierto y CAMBIADO sin guardar
    let bufer = Arc::new(Mutex::new("hola mundo — sin guardar\n".to_string()));
    let (b2, ar) = (bufer.clone(), archivo.clone());
    let resp: Respuestas = Box::new(move |m| match m["method"].as_str().unwrap() {
        "fs/read_text_file" if m["params"]["path"] == json!(ar) => json!({"content": b2.lock().unwrap().clone()}),
        "fs/read_text_file" => json!({"content": std::fs::read_to_string(m["params"]["path"].as_str().unwrap()).unwrap_or_default()}),
        "fs/write_text_file" => {
            let t = m["params"]["content"].as_str().unwrap().to_string();
            *b2.lock().unwrap() = t.clone();
            std::fs::write(m["params"]["path"].as_str().unwrap(), t).unwrap(); // el editor guarda
            json!({})
        }
        "session/request_permission" => json!({"outcome": {"outcome": "selected", "optionId": "si"}}),
        otro => panic!("{otro}"),
    });
    let id = c.pedir("session/prompt", json!({"sessionId": s, "prompt": [{"type": "text", "text": "cambia mundo por GOTV en "}, {"type": "resource_link", "uri": format!("file://{}", archivo.display()), "name": "saludo.txt"}]})).await;
    let r = c.respuesta(id, &resp).await;
    assert_eq!(r["result"]["stopReason"], "end_turn", "{r}");
    // la herramienta se pintó: empieza (edit, con su ruta absoluta) y termina con el diff
    let llamada = &c.actualizaciones("tool_call")[0];
    assert_eq!((llamada["kind"].as_str(), llamada["status"].as_str()), (Some("edit"), Some("in_progress")));
    assert_eq!(llamada["locations"][0]["path"], json!(archivo));
    let permiso = c.vistos.iter().find(|m| m["method"] == "session/request_permission").expect("pidió permiso");
    assert!(permiso["params"]["toolCall"].get("title").is_none(), "el título de la herramienta no se pisa (medido en Zed)");
    let diff = permiso["params"]["toolCall"]["content"].as_array().unwrap().iter().find(|x| x["type"] == "diff").expect("el permiso lleva el diff");
    assert_eq!(diff["oldText"], "hola mundo — sin guardar\n", "lo sin guardar manda sobre el disco");
    assert_eq!(diff["newText"], "hola GOTV — sin guardar\n");
    let kinds: Vec<&str> = permiso["params"]["options"].as_array().unwrap().iter().map(|o| o["kind"].as_str().unwrap()).collect();
    assert_eq!(kinds, ["allow_once", "allow_always", "reject_once", "reject_always"]);
    let fin = c.actualizaciones("tool_call_update").into_iter().find(|u| u["status"] == "completed").expect("terminó");
    assert!(fin["content"].as_array().unwrap().iter().any(|x| x["type"] == "diff" && x["newText"] == "hola GOTV — sin guardar\n"));
    assert!(c.vistos.iter().any(|m| m["method"] == "fs/write_text_file"), "escribió POR el editor");
    assert_eq!(std::fs::read_to_string(&archivo).unwrap(), "hola GOTV — sin guardar\n", "el archivo cambió");
    // el modelo recibió la ruta del resource_link
    let _ = r;
}

/// «Hecho cuando» de F10.3 con el MOTOR LOCAL (Huihui por `gotv serve`): el editor abre un chat de GOTV, se le
/// pide un cambio, llega el permiso con su diff, el editor lo acepta y el archivo cambia (escrito POR el editor).
/// `GOTV_PRUEBA_OPENAI=http://127.0.0.1:11437/v1 GOTV_PRUEBA_MODELO=<nombre> cargo test -p gotv-core --release
///  --lib -- --ignored --nocapture acp_con_el_motor_local`
#[tokio::test]
#[ignore]
async fn acp_con_el_motor_local() {
    let (Ok(url), Ok(modelo)) = (std::env::var("GOTV_PRUEBA_OPENAI"), std::env::var("GOTV_PRUEBA_MODELO")) else { return };
    let d = crate::CarpetaPrueba::nueva("gotv-acp-real");
    let root = d.canonicalize().unwrap();
    let archivo = root.join("saludo.txt");
    std::fs::write(&archivo, "hola mundo\n").unwrap();
    let (cli_w, srv_r) = tokio::io::duplex(1 << 20);
    let (srv_w, cli_r) = tokio::io::duplex(1 << 20);
    let fabrica: Fabrica<gotv_engine::cloud::OpenAiEngine> = Arc::new(move |cwd, front| {
        let (url, modelo) = (url.clone(), modelo.clone());
        Box::pin(async move {
            let motor = gotv_engine::cloud::OpenAiEngine::new("gotv", &url, String::new(), &modelo, false);
            let mut a = crate::hooks_eventos::tests::agente_con(&cwd, Mode::Default, motor, front);
            a.max_steps = 20;
            a.max_tokens = 4096;
            // como en la ventana con «preguntar siempre» para editar: el editor ve el permiso con su diff
            a.perms = crate::permissions::Permissions::new(Mode::Default, &cwd, vec![], vec![], vec!["edit".into()]);
            Ok(a)
        })
    });
    let _h = tokio::spawn(servir(srv_r, srv_w, fabrica));
    let mut c = Cliente { w: cli_w, r: BufReader::new(cli_r).lines(), n: 0, vistos: vec![] };
    saludar(&mut c, true).await;
    let s = sesion(&mut c, &root).await;
    let ar = archivo.clone();
    let resp: Respuestas = Box::new(move |m| match m["method"].as_str().unwrap() {
        "fs/read_text_file" => json!({"content": std::fs::read_to_string(m["params"]["path"].as_str().unwrap()).unwrap_or_default()}),
        "fs/write_text_file" => {
            std::fs::write(m["params"]["path"].as_str().unwrap(), m["params"]["content"].as_str().unwrap()).unwrap();
            json!({})
        }
        "session/request_permission" => json!({"outcome": {"outcome": "selected", "optionId": "si"}}),
        otro => panic!("{otro} ({ar:?})"),
    });
    let t = std::time::Instant::now();
    let id = c.pedir("session/prompt", json!({"sessionId": s, "prompt": [{"type": "text", "text": "En saludo.txt cambia «mundo» por «GOTV»."}]})).await;
    let r = c.respuesta(id, &resp).await;
    eprintln!("{} s · stopReason {}", t.elapsed().as_secs(), r["result"]["stopReason"]);
    assert_eq!(r["result"]["stopReason"], "end_turn", "{r}");
    let permiso = c.vistos.iter().find(|m| m["method"] == "session/request_permission").expect("pidió permiso");
    let diff = permiso["params"]["toolCall"]["content"].as_array().unwrap().iter().find(|x| x["type"] == "diff").expect("el permiso lleva el diff");
    eprintln!("diff: {} → {}", diff["oldText"], diff["newText"]);
    assert!(c.vistos.iter().any(|m| m["method"] == "fs/write_text_file"), "escribió POR el editor");
    assert_eq!(std::fs::read_to_string(&archivo).unwrap(), "hola GOTV\n", "el archivo cambió");
    let textos: String = c.actualizaciones("agent_message_chunk").iter().filter_map(|u| u["content"]["text"].as_str()).collect();
    eprintln!("respuesta en vivo: {}", textos.chars().take(200).collect::<String>());
    assert!(!textos.is_empty(), "el texto llegó en vivo");
}

/// Un permiso cancelado o que el editor no sabe contestar se NIEGA (nunca se concede).
#[tokio::test]
async fn un_permiso_que_el_editor_no_contesta_bien_se_niega() {
    for respuesta in [json!({"outcome": {"outcome": "cancelled"}}), json!({"cosa": "rara"}), json!({"outcome": {"outcome": "selected", "optionId": "inventada"}})] {
        let d = crate::CarpetaPrueba::nueva("gotv-acp-niega");
        let root = d.canonicalize().unwrap();
        let guion = vec![llamadas(&[("bash", json!({"cmd": "echo x > no-debe.txt"}))]), texto("No me dejaron.")];
        let (mut c, _h) = arrancar(Mode::Default, vec![guion], Duration::ZERO);
        saludar(&mut c, false).await;
        let s = sesion(&mut c, &root).await;
        let r2 = respuesta.clone();
        let resp: Respuestas = Box::new(move |_| r2.clone());
        let id = c.pedir("session/prompt", json!({"sessionId": s, "prompt": [{"type": "text", "text": "crea el archivo"}]})).await;
        let r = c.respuesta(id, &resp).await;
        assert_eq!(r["result"]["stopReason"], "end_turn");
        assert!(!root.join("no-debe.txt").exists(), "con {respuesta} no se concede");
        assert!(c.actualizaciones("tool_call_update").iter().any(|u| u["status"] == "failed"));
    }
}

/// Cancelar a mitad: la respuesta de la orden sale con `cancelled`.
#[tokio::test]
async fn cancelar_a_mitad() {
    let d = crate::CarpetaPrueba::nueva("gotv-acp-cancela");
    let root = d.canonicalize().unwrap();
    let (mut c, _h) = arrancar(Mode::Default, vec![vec![texto("tarda")]], Duration::from_secs(5));
    saludar(&mut c, false).await;
    let s = sesion(&mut c, &root).await;
    let t0 = std::time::Instant::now();
    let id = c.pedir("session/prompt", json!({"sessionId": s, "prompt": [{"type": "text", "text": "algo largo"}]})).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    c.avisar("session/cancel", json!({"sessionId": s})).await;
    let r = c.respuesta(id, &sin_peticiones()).await;
    assert_eq!(r["result"]["stopReason"], "cancelled", "{r}");
    assert!(t0.elapsed() < Duration::from_secs(4), "no esperó al motor: {:?}", t0.elapsed());
}

/// El motor local atiende una orden a la vez: la segunda (de otro chat) se encola, se dice, y se atiende
/// después; una en cola que se cancela sale `cancelled` sin correr.
#[tokio::test]
async fn la_segunda_orden_se_encola_y_se_dice() {
    let d = crate::CarpetaPrueba::nueva("gotv-acp-cola");
    let root = d.canonicalize().unwrap();
    let (mut c, _h) = arrancar(Mode::Default, vec![vec![texto("primera")], vec![texto("segunda")], vec![texto("tercera")]], Duration::from_millis(600));
    saludar(&mut c, false).await;
    let (s1, s2, s3) = (sesion(&mut c, &root).await, sesion(&mut c, &root).await, sesion(&mut c, &root).await);
    let a = c.pedir("session/prompt", json!({"sessionId": s1, "prompt": [{"type": "text", "text": "1"}]})).await;
    let b = c.pedir("session/prompt", json!({"sessionId": s2, "prompt": [{"type": "text", "text": "2"}]})).await;
    let x = c.pedir("session/prompt", json!({"sessionId": s3, "prompt": [{"type": "text", "text": "3"}]})).await;
    c.avisar("session/cancel", json!({"sessionId": s3})).await;
    let ra = c.respuesta(a, &sin_peticiones()).await;
    let rb = c.respuesta(b, &sin_peticiones()).await;
    let rx = c.vistos.iter().find(|m| m["id"] == json!(x) && m.get("method").is_none()).cloned();
    let rx = match rx {
        Some(r) => r,
        None => c.respuesta(x, &sin_peticiones()).await,
    };
    assert_eq!((ra["result"]["stopReason"].as_str(), rb["result"]["stopReason"].as_str(), rx["result"]["stopReason"].as_str()), (Some("end_turn"), Some("end_turn"), Some("cancelled")));
    let en_cola: Vec<Value> = c.actualizaciones("agent_message_chunk").into_iter().filter(|u| u["content"]["text"].as_str().unwrap_or("").contains("⏳")).collect();
    assert_eq!(en_cola.len(), 2, "se le dijo al editor que esperaban");
    let pos = |id: u64| c.vistos.iter().position(|m| m["id"] == json!(id) && m.get("method").is_none()).unwrap();
    assert!(pos(a) < pos(b), "en orden");
}

/// Al cerrar el editor (fin de la entrada), cada chat termina y el servidor sale limpio.
#[tokio::test]
async fn cerrar_la_entrada_termina_limpio() {
    let d = crate::CarpetaPrueba::nueva("gotv-acp-cierra");
    let root = d.canonicalize().unwrap();
    let (mut c, h) = arrancar(Mode::Default, vec![vec![]], Duration::ZERO);
    saludar(&mut c, false).await;
    sesion(&mut c, &root).await;
    drop(c);
    tokio::time::timeout(Duration::from_secs(10), h).await.expect("salió").unwrap().unwrap();
}

#[test]
fn texto_de_los_bloques() {
    let (t, avisos) = texto_de_prompt(&[json!({"type": "text", "text": "mira"}), json!({"type": "resource_link", "uri": "file:///p/a.rs", "name": "a.rs"}), json!({"type": "resource", "resource": {"uri": "file:///p/b.rs", "text": "fn b() {}"}}), json!({"type": "image", "data": "x"})]);
    assert_eq!(t, "mira @/p/a.rs\n\n[/p/b.rs]\nfn b() {}");
    assert_eq!(avisos.len(), 1);
}
