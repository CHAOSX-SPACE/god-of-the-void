//! F10.5 · El control remoto contra el servidor de `serve.rs` de verdad (puerto libre, red local de esta Mac
//! si la hay). Todo este archivo es código de prueba (`#[cfg(test)]` en `remoto/mod.rs`): el juez de idioma
//! no lo mira.
use super::*;
use crate::hooks_eventos::tests::{agente, llamadas, texto, Mudo};
use crate::permissions::Mode;

/// El servidor de `serve.rs` de verdad, por su `serve::run` (con los dos extremos de cada conexión), en un
/// puerto libre. `par` = cada petición llega como si viniera de esa dirección (una capa de prueba pisa el par
/// y deja la dirección local de verdad).
async fn servidor_con(llave: Option<&str>, registro: Registro, host: &str, par: Option<[u8; 4]>) -> (String, crate::serve::Shared) {
    let mut s = crate::serve::Shared::new(llave.map(String::from));
    s.remoto = registro;
    let l = tokio::net::TcpListener::bind(format!("{host}:0")).await.unwrap();
    let url = format!("http://{}", l.local_addr().unwrap());
    match par {
        None => {
            let (_tx, rx) = tokio::sync::oneshot::channel::<()>();
            let s2 = s.clone();
            tokio::spawn(async move {
                let _tx = _tx;
                crate::serve::run(l, s2, rx).await.unwrap()
            });
        }
        Some(ip) => {
            let app = crate::serve::router(s.clone()).layer(axum::middleware::from_fn(move |mut req: Request<Body>, next: Next| async move {
                if let Some(c) = req.extensions().get::<ConnectInfo<Conexion>>().map(|c| c.0) {
                    req.extensions_mut().insert(ConnectInfo(Conexion { par: SocketAddr::from((ip, 4444)), local: c.local }));
                }
                next.run(req).await
            }));
            tokio::spawn(async move { axum::serve(l, app.into_make_service_with_connect_info::<Conexion>()).await.unwrap() });
        }
    }
    (url, s)
}

async fn servidor(llave: Option<&str>, registro: Registro, host: &str, publico: bool) -> (String, crate::serve::Shared) {
    servidor_con(llave, registro, host, publico.then_some([8, 8, 8, 8])).await
}

fn reg() -> Registro {
    Registro::con_archivo(None)
}

#[test]
fn red_local_si_internet_no() {
    for ip in ["127.0.0.1", "10.0.0.5", "172.20.1.1", "192.168.1.40", "169.254.3.3", "::1", "fd12::1", "fe80::1", "::ffff:192.168.0.2"] {
        assert!(es_red_local(ip.parse().unwrap()), "{ip}");
        assert!(admitir(ip.parse().unwrap(), Some("en0")), "{ip}");
    }
    for ip in ["8.8.8.8", "100.64.1.1", "100.128.0.1", "172.32.0.1", "2001:4860::8888", "::ffff:8.8.8.8"] {
        assert!(!es_red_local(ip.parse().unwrap()), "{ip}");
    }
    // 100.64/10 (Tailscale): sólo si entró por una interfaz de túnel de esta Mac
    for ip in ["100.64.1.1", "100.127.255.254"] {
        assert!(admitir(ip.parse().unwrap(), Some("utun4")), "{ip} por el túnel");
        assert!(!admitir(ip.parse().unwrap(), Some("en0")), "{ip} por la Wi-Fi: un vecino del operador");
        assert!(!admitir(ip.parse().unwrap(), None));
    }
    assert!(!admitir("100.128.0.1".parse().unwrap(), Some("utun4")), "fuera de 100.64/10 no vale ni por túnel");
    assert!(!admitir("8.8.8.8".parse().unwrap(), Some("utun4")));
    // la interfaz de verdad de una dirección de esta Mac
    assert_eq!(interfaz_de("127.0.0.1".parse().unwrap()).as_deref(), Some("lo0"));
    assert_eq!(interfaz_de("203.0.113.9".parse().unwrap()), None);
}

/// Sin llave ni token → 401; con la llave del servicio o con un token emparejado → 200; revocar deja sin
/// acceso al momento; desde fuera de la red local → 403 aunque traiga la llave.
#[tokio::test]
async fn llave_token_revocar_y_red_local() {
    let r = reg();
    let (url, _s) = servidor(Some("llave-del-servicio-123456"), r.clone(), "127.0.0.1", false).await;
    let c = reqwest::Client::new();
    let get = |t: Option<&str>| {
        let mut q = c.get(format!("{url}/gotv/sesiones"));
        if let Some(t) = t {
            q = q.bearer_auth(t);
        }
        q.send()
    };
    assert_eq!(get(None).await.unwrap().status(), 401);
    assert_eq!(get(Some("otra-cosa")).await.unwrap().status(), 401);
    assert_eq!(get(Some("llave-del-servicio-123456")).await.unwrap().status(), 200);
    // emparejar: un código de un solo uso por un token propio
    let codigo = r.codigo_nuevo();
    let tok: Value = c.post(format!("{url}/gotv/emparejar")).json(&json!({"codigo": codigo, "nombre": "iPhone"})).send().await.unwrap().json().await.unwrap();
    let tok = tok["token"].as_str().unwrap().to_string();
    assert_eq!(get(Some(&tok)).await.unwrap().status(), 200);
    assert_eq!(c.post(format!("{url}/gotv/emparejar")).json(&json!({"codigo": codigo})).send().await.unwrap().status(), 401, "el código vale una vez");
    let t = r.tokens();
    assert_eq!((t.len(), t[0].nombre.as_str(), t[0].hash.as_str()), (1, "iPhone", ""), "la lista no enseña el hash");
    assert!(r.revocar(&t[0].id));
    assert_eq!(get(Some(&tok)).await.unwrap().status(), 401, "revocado: sin acceso al momento");
    // el token tampoco abre la API de modelos (sólo /gotv/)
    assert_eq!(c.get(format!("{url}/v1/models")).bearer_auth(&tok).send().await.unwrap().status(), 401);
    // la página se sirve sin token (no lleva datos), pero sólo a la red local
    assert_eq!(c.get(format!("{url}/gotv/movil")).send().await.unwrap().status(), 200);
    let (fuera, _) = servidor(Some("llave-del-servicio-123456"), reg(), "127.0.0.1", true).await;
    for ruta in ["/gotv/sesiones", "/gotv/movil"] {
        let r = c.get(format!("{fuera}{ruta}")).bearer_auth("llave-del-servicio-123456").send().await.unwrap();
        assert_eq!(r.status(), 403, "{ruta}");
    }
    // un par 100.64/10 que NO entró por un túnel (aquí entra por lo0): rechazado aunque traiga la llave
    let (cgnat, _) = servidor_con(Some("llave-del-servicio-123456"), reg(), "127.0.0.1", Some([100, 64, 7, 7])).await;
    assert_eq!(c.get(format!("{cgnat}/gotv/sesiones")).bearer_auth("llave-del-servicio-123456").send().await.unwrap().status(), 403);
    // sin llave de servicio configurada, el control remoto NO queda abierto: hace falta un token
    let (sin, _) = servidor(None, reg(), "127.0.0.1", false).await;
    assert_eq!(c.get(format!("{sin}/gotv/sesiones")).send().await.unwrap().status(), 401);
}

/// El QR: la dirección con el código en el FRAGMENTO (no viaja al servidor), un SVG de sólo cuadros con los tres
/// localizadores en sus esquinas.
#[test]
fn qr_del_emparejado() {
    let url = url_con_codigo("http://192.168.1.20:11435/gotv/movil", "12345678");
    assert_eq!(url, "http://192.168.1.20:11435/gotv/movil#c=12345678");
    let svg = qr_svg(&url).unwrap();
    let q = qrcode::QrCode::with_error_correction_level(url.as_bytes(), qrcode::EcLevel::M).unwrap();
    let (n, c) = (q.width(), q.to_colors());
    assert!(svg.starts_with("<svg") && svg.contains(&format!("viewBox=\"0 0 {0} {0}\"", n + 8)), "módulos + margen de 4: {}", &svg[..120]);
    assert!(!svg.contains("12345678"), "el SVG no lleva el texto, sólo cuadros");
    for (x, y) in [(0, 0), (n - 7, 0), (0, n - 7)] {
        assert!((0..7).all(|i| c[y * n + x + i] == qrcode::Color::Dark), "localizador en ({x},{y})");
        assert!(svg.contains(&format!("M{} {}h1v1h-1z", x + 4, y + 4)));
    }
}

/// Cinco códigos fallidos anulan los códigos vigentes (no se adivina a ciegas), y un código vence.
#[test]
fn codigos_con_tope_de_fallos() {
    let r = reg();
    let bueno = r.codigo_nuevo();
    assert_eq!(bueno.len(), 8);
    for i in 0..CODIGO_FALLOS_MAX {
        assert!(r.canjear(&format!("{:08}", i), "x").is_none() || format!("{:08}", i) == bueno);
    }
    assert!(r.canjear(&bueno, "x").is_none(), "tras los fallos el bueno ya no vale");
    let otro = r.codigo_nuevo();
    assert!(r.canjear(&otro, "x").is_some());
}

/// Los tokens se guardan con hash en el formato propio de GOTV y sobreviven a reabrir.
#[test]
fn tokens_guardados_con_hash() {
    let d = crate::CarpetaPrueba::nueva("gotv-remoto-tokens");
    let a = d.join("remoto.bin");
    let r = Registro::con_archivo(Some(a.clone()));
    let c = r.codigo_nuevo();
    let t = r.canjear(&c, "tel").unwrap();
    let b = std::fs::read(&a).unwrap();
    assert_eq!(&b[..8], b"GOTVREMO");
    assert!(!String::from_utf8_lossy(&b).contains(&t), "el token no se guarda, sólo su sha256");
    let r2 = Registro::con_archivo(Some(a));
    assert!(r2.token_valido(&t) && !r2.token_valido("gotvtel-otro"));
}

/// Un `id_permiso` usado dos veces: la segunda se niega (409); uno contestado en la ventana tampoco vale.
#[tokio::test]
async fn un_permiso_vale_una_vez() {
    let r = reg();
    r.abrir("s1", Path::new("/p"), "chat");
    let (url, _) = servidor(Some("llave-del-servicio-123456"), r.clone(), "127.0.0.1", false).await;
    let (idp, rx) = r.pedir_permiso("s1", "bash: rm -rf build/");
    let c = reqwest::Client::new();
    let post = |idp: &str, d: &str| c.post(format!("{url}/gotv/sesiones/s1/permiso")).bearer_auth("llave-del-servicio-123456").json(&json!({"id_permiso": idp, "decision": d})).send();
    assert_eq!(post(&idp, "si").await.unwrap().status(), 200);
    assert_eq!(rx.await.unwrap(), AskAnswer::Yes);
    assert_eq!(post(&idp, "no").await.unwrap().status(), 409);
    assert_eq!(post("inventado", "si").await.unwrap().status(), 404);
    assert_eq!(post(&idp, "quizá").await.unwrap().status(), 400);
    let (idp2, _rx2) = r.pedir_permiso("s1", "otra");
    r.permiso_resuelto("s1", &idp2, AskAnswer::No);
    assert_eq!(post(&idp2, "si").await.unwrap().status(), 409, "ya lo contestó la ventana");
}

/// Lee eventos SSE (id, tipo, datos) hasta tener `n` o pasar el tope.
async fn leer_sse(r: reqwest::Response, n: usize) -> Vec<(u64, String, Value)> {
    leer_sse_hasta(r, |v| v.len() >= n).await
}

/// Lee eventos SSE hasta que `basta` diga que ya (o pasen 10 s).
async fn leer_sse_hasta(r: reqwest::Response, basta: impl Fn(&[(u64, String, Value)]) -> bool) -> Vec<(u64, String, Value)> {
    use futures_util::StreamExt;
    let mut s = r.bytes_stream();
    let mut buf = String::new();
    let mut out = Vec::new();
    let tope = tokio::time::Instant::now() + Duration::from_secs(10);
    while !basta(&out) {
        let Ok(Some(Ok(b))) = tokio::time::timeout_at(tope, s.next()).await else { break };
        buf.push_str(&String::from_utf8_lossy(&b));
        while let Some(i) = buf.find("\n\n") {
            let blk: String = buf.drain(..i + 2).collect();
            let (mut id, mut ev, mut data) = (0u64, String::new(), String::new());
            for l in blk.lines() {
                if let Some(x) = l.strip_prefix("id:") {
                    id = x.trim().parse().unwrap_or(0);
                } else if let Some(x) = l.strip_prefix("event:") {
                    ev = x.trim().to_string();
                } else if let Some(x) = l.strip_prefix("data:") {
                    data.push_str(x.trim());
                }
            }
            if !data.is_empty() {
                out.push((id, ev, serde_json::from_str(&data).unwrap()));
            }
        }
    }
    out
}

/// Cortar el SSE y volver con `Last-Event-ID` no pierde ni repite eventos.
#[tokio::test]
async fn sse_con_last_event_id_no_pierde_ni_repite() {
    let r = reg();
    r.abrir("s", Path::new("/p"), "chat");
    let (url, _) = servidor(Some("llave-del-servicio-123456"), r.clone(), "127.0.0.1", false).await;
    let c = reqwest::Client::new();
    for i in 0..3 {
        r.evento("s", "texto", json!({"texto": format!("a{i}")}));
    }
    let primero = c.get(format!("{url}/gotv/sesiones/s/eventos")).bearer_auth("llave-del-servicio-123456").send().await.unwrap();
    assert_eq!(primero.headers()["content-type"], "text/event-stream");
    let vistos = leer_sse(primero, 3).await;
    assert_eq!(vistos.iter().map(|v| v.2["texto"].as_str().unwrap()).collect::<Vec<_>>(), ["a0", "a1", "a2"]);
    let ultimo = vistos.last().unwrap().0;
    // se corta (se suelta la respuesta); mientras tanto pasan cosas
    for i in 3..6 {
        r.evento("s", "texto", json!({"texto": format!("a{i}")}));
    }
    let vuelta = c.get(format!("{url}/gotv/sesiones/s/eventos")).bearer_auth("llave-del-servicio-123456").header("Last-Event-ID", ultimo.to_string()).send().await.unwrap();
    // y en vivo, con la conexión abierta
    let r2 = r.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(300)).await;
        r2.evento("s", "texto", json!({"texto": "a6"}));
    });
    let vistos2 = leer_sse(vuelta, 4).await;
    assert_eq!(vistos2.iter().map(|v| v.2["texto"].as_str().unwrap()).collect::<Vec<_>>(), ["a3", "a4", "a5", "a6"]);
    let ids: Vec<u64> = vistos.iter().chain(vistos2.iter()).map(|v| v.0).collect();
    assert!(ids.windows(2).all(|w| w[1] == w[0] + 1), "ids seguidos, sin huecos ni repetidos: {ids:?}");
}

/// Más eventos de los que se guardan: volver con un id viejo dice «perdidos» en vez de callarlo.
#[tokio::test]
async fn sse_dice_si_se_perdieron_eventos() {
    let r = reg();
    r.abrir("s", Path::new("/p"), "chat");
    for i in 0..(EVENTOS_MAX + 10) {
        r.evento("s", "texto", json!({"texto": i}));
    }
    let (url, _) = servidor(Some("llave-del-servicio-123456"), r.clone(), "127.0.0.1", false).await;
    let resp = reqwest::Client::new().get(format!("{url}/gotv/sesiones/s/eventos?desde=1")).bearer_auth("llave-del-servicio-123456").send().await.unwrap();
    let v = leer_sse(resp, 2).await;
    assert_eq!(v[0].1, "perdidos");
    assert_eq!(v[1].2["texto"], json!(10));
}

/// Un frente cuya ventana nunca contesta (la persona está lejos de la Mac).
#[derive(Default)]
struct Lejos(Mudo);
#[async_trait::async_trait]
impl crate::Frontend for Lejos {
    async fn log(&self, l: String) {
        self.0.log(l).await
    }
    async fn ask(&self, d: &str) -> AskAnswer {
        self.0.preguntas.lock().unwrap().push(d.to_string());
        std::future::pending().await
    }
}

/// «Hecho cuando» de F10.5 con el agente de verdad (motor de guion, sin modelo) y el servidor de verdad: desde
/// el «teléfono» (un cliente HTTP con su token emparejado, por la dirección de RED LOCAL de esta Mac si la
/// tiene) se ve el chat trabajando, se aprueba un permiso (el archivo aparece) y se le manda «sigue», que el
/// chat atiende como su próxima orden.
#[tokio::test]
async fn desde_el_telefono_se_aprueba_un_permiso_y_se_manda_sigue() {
    let d = crate::CarpetaPrueba::nueva("gotv-remoto-e2e");
    let root = d.canonicalize().unwrap();
    let r = reg();
    // por la red local de verdad si esta Mac tiene una dirección (si no, por este equipo)
    let lan = crate::serve::lan_addresses().into_iter().next();
    let host = if lan.is_some() { "0.0.0.0" } else { "127.0.0.1" };
    let (url, _) = servidor(Some("llave-del-servicio-123456"), r.clone(), host, false).await;
    let url = match &lan {
        Some(ip) => url.replace("0.0.0.0", ip),
        None => url,
    };
    let c = reqwest::Client::new();
    let token = {
        let codigo = r.codigo_nuevo();
        let v: Value = c.post(format!("{url}/gotv/emparejar")).json(&json!({"codigo": codigo, "nombre": "prueba"})).send().await.unwrap().json().await.unwrap();
        v["token"].as_str().unwrap().to_string()
    };
    let guion = vec![llamadas(&[("bash", json!({"cmd": "echo ok > aprobado.txt"}))]), texto("Creado."), texto("Sigo: todo listo.")];
    let front = FrenteRemoto::new(Lejos::default(), r.clone(), "chat-1");
    r.abrir("chat-1", &root, "proyecto de prueba");
    let mut a = agente(&root, Mode::Default, guion, front);
    let eventos = c.get(format!("{url}/gotv/sesiones/chat-1/eventos")).bearer_auth(&token).send().await.unwrap();
    assert_eq!(eventos.status(), 200);
    let r2 = r.clone();
    let orden = tokio::spawn(async move {
        r2.estado("chat-1", EstadoSesion::Trabajando);
        let res = a.run("crea aprobado.txt").await;
        r2.estado("chat-1", EstadoSesion::Libre);
        (a, res)
    });
    // el teléfono ve el permiso pedido, con el comando ENTERO, y lo aprueba
    let vistos = leer_sse_hasta(eventos, |v| v.iter().any(|e| e.1 == "permiso")).await;
    assert!(vistos.iter().any(|e| e.1 == "estado" && e.2["estado"] == "trabajando"), "{vistos:?}");
    let pedido = vistos.iter().find(|e| e.1 == "permiso").unwrap();
    assert!(pedido.2["descripcion"].as_str().unwrap().contains("echo ok > aprobado.txt"), "el comando ENTERO");
    assert_eq!(pedido.2["partes"]["detalle"], "echo ok > aprobado.txt", "y en piezas, para pintarlo como en la Mac");
    let lista: Value = c.get(format!("{url}/gotv/sesiones")).bearer_auth(&token).send().await.unwrap().json().await.unwrap();
    assert_eq!(lista["sesiones"][0]["estado"], "esperando_permiso");
    assert_eq!(lista["sesiones"][0]["proyecto"], json!(root));
    let ok = c.post(format!("{url}/gotv/sesiones/chat-1/permiso")).bearer_auth(&token).json(&json!({"id_permiso": pedido.2["id_permiso"], "decision": "si"})).send().await.unwrap();
    assert_eq!(ok.status(), 200);
    let (mut a, res) = orden.await.unwrap();
    assert_eq!(res.unwrap(), "Creado.");
    assert!(root.join("aprobado.txt").exists(), "aprobado desde el teléfono: el archivo existe");
    // «sigue» desde el teléfono: a la cola, y el chat lo atiende como su próxima orden
    let m = c.post(format!("{url}/gotv/sesiones/chat-1/mensaje")).bearer_auth(&token).json(&json!({"texto": "sigue"})).send().await.unwrap();
    assert_eq!(m.status(), 202);
    let siguiente = r.tomar_mensaje("chat-1").unwrap();
    assert_eq!(siguiente, "sigue");
    assert_eq!(a.run(&siguiente).await.unwrap(), "Sigo: todo listo.");
    // la ventana vio la misma pregunta (se contesta donde se conteste primero)
    assert_eq!(a.front.interno.0.preguntas.lock().unwrap().len(), 1);
    eprintln!("control remoto por {}", if lan.is_some() { "la red local de esta Mac" } else { "este equipo (sin red local)" });
}

/// Prueba del Portador (2026-11-18): la página ya no guarda el token en `localStorage`. Emparejar pone una cookie
/// HttpOnly + SameSite=Strict sólo para `/gotv`; con ella se entra; «olvidar este teléfono» la revoca y la borra.
#[tokio::test]
async fn el_telefono_entra_con_su_cookie_y_se_olvida() {
    let r = reg();
    let (url, _) = servidor(Some("llave-del-servicio-123456"), r.clone(), "127.0.0.1", false).await;
    let c = reqwest::Client::new();
    let codigo = r.codigo_nuevo();
    let res = c.post(format!("{url}/gotv/emparejar")).json(&json!({"codigo": codigo, "nombre": "iPhone"})).send().await.unwrap();
    let galleta = res.headers().get(header::SET_COOKIE).and_then(|v| v.to_str().ok()).unwrap().to_string();
    for parte in ["gotv_tel=gotvtel-", "HttpOnly", "SameSite=Strict", "Path=/gotv"] {
        assert!(galleta.contains(parte), "{galleta}");
    }
    let par = galleta.split(';').next().unwrap().to_string();
    assert_eq!(c.get(format!("{url}/gotv/sesiones")).header(header::COOKIE, &par).send().await.unwrap().status(), 200);
    assert_eq!(c.get(format!("{url}/gotv/sesiones")).header(header::COOKIE, "gotv_tel=inventado").send().await.unwrap().status(), 401);
    let fuera = c.post(format!("{url}/gotv/salir")).header(header::COOKIE, &par).send().await.unwrap();
    assert!(fuera.headers().get(header::SET_COOKIE).and_then(|v| v.to_str().ok()).unwrap().contains("Max-Age=0"));
    assert_eq!(c.get(format!("{url}/gotv/sesiones")).header(header::COOKIE, &par).send().await.unwrap().status(), 401, "revocado al momento");
    assert!(r.tokens().is_empty());
}

/// Para MIRAR la página con un teléfono (o el simulador): un servidor en `127.0.0.1:11499` con un chat que
/// trabaja solo — orden, texto que llega a trozos, pasos, un permiso cada vuelta — durante `GOTV_PRUEBA_SEG`
/// segundos (600 por omisión). Imprime la dirección con el código de emparejar.
#[tokio::test]
#[ignore]
async fn pagina_en_vivo_para_mirarla() {
    let r = reg();
    let s = {
        let mut s = crate::serve::Shared::new(Some("llave-de-la-prueba-123456".into()));
        s.remoto = r.clone();
        s
    };
    let l = tokio::net::TcpListener::bind("127.0.0.1:11499").await.unwrap();
    let (_tx, rx) = tokio::sync::oneshot::channel::<()>();
    tokio::spawn(async move { crate::serve::run(l, s, rx).await.unwrap() });
    let id = "/Users/x/Documents/proyectos/demo";
    r.abrir(id, Path::new(id), "demo");
    eprintln!("PAGINA http://127.0.0.1:11499/gotv/movil#c={}", r.codigo_nuevo());
    let fin = Instant::now() + Duration::from_secs(std::env::var("GOTV_PRUEBA_SEG").ok().and_then(|v| v.parse().ok()).unwrap_or(600));
    let mut vuelta = 0;
    while Instant::now() < fin {
        vuelta += 1;
        tokio::time::sleep(Duration::from_secs(4)).await;
        r.evento(id, "orden", json!({"texto": format!("crea notas-{vuelta}.txt en el escritorio")}));
        r.estado(id, EstadoSesion::Trabajando);
        for trozo in ["Voy a crear ", "el archivo ", &format!("notas-{vuelta}.txt "), "en tu escritorio (~/Desktop)."] {
            tokio::time::sleep(Duration::from_millis(600)).await;
            r.evento(id, "texto", json!({"texto": trozo}));
        }
        r.evento(id, "actividad", json!({"tipo": "step", "detalle": "bash · touch ~/Desktop/notas.txt"}));
        let (_idp, rx) = r.pedir_permiso(id, &format!("🔓 FUERA DEL PROYECTO · write_file  /Users/x/Desktop/notas-{vuelta}.txt  — la caja de arena sólo deja escribir dentro del proyecto"));
        let a = tokio::time::timeout(Duration::from_secs(60), rx).await;
        eprintln!("vuelta {vuelta}: {a:?}");
        r.estado(id, EstadoSesion::Trabajando);
        r.evento(id, "texto", json!({"texto": "\n\nListo."}));
        r.estado(id, EstadoSesion::Libre);
        if let Some(m) = r.tomar_mensaje(id) {
            r.evento(id, "orden", json!({"texto": m}));
        }
    }
}
