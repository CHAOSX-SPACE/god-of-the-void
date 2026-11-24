//! F10.5 · Control remoto desde el teléfono, sobre el servidor de `serve.rs` (mismo `axum`, misma llave).
//!
//! | Ruta | Qué |
//! |---|---|
//! | `GET /gotv/sesiones` | los chats abiertos: proyecto, título, estado (`libre` · `trabajando` · `esperando_permiso`) |
//! | `GET /gotv/sesiones/{id}/eventos` | SSE: texto, actividad, permisos pedidos y estado; admite `Last-Event-ID` |
//! | `POST /gotv/sesiones/{id}/permiso` | `{id_permiso, decision: "si"\|"no"}`; cada `id_permiso` vale UNA vez |
//! | `POST /gotv/sesiones/{id}/mensaje` | `{texto}`: va a la cola del chat y se atiende como su próxima orden |
//! | `POST /gotv/emparejar` | `{codigo, nombre}` → `{token}` y la cookie HttpOnly `gotv_tel` (la página no guarda nada en JS) |
//! | `POST /gotv/salir` | el teléfono se olvida: revoca su token y borra la cookie |
//! | `GET /gotv/movil` | la página del teléfono (HTML y JS dentro del binario) |
//!
//! **Sólo red local:** toda ruta `/gotv/` mira la dirección del PAR (no una cabecera que el cliente pueda
//! inventar): este equipo, 10/8, 172.16/12, 192.168/16, 169.254/16, fc00::/7 o fe80::/10; y 100.64/10
//! (Tailscale) SÓLO si entró por una interfaz de túnel de esta Mac (`utun*`). Lo demás, 403. Un
//! túnel lo pone el Portador (llega por este equipo), nunca GOTV. **Con llave:** la del servicio
//! (`Authorization: Bearer …`) o un token emparejado (se guarda su sha256, nunca el token); sin ellas, 401.
//! El servicio sigue escuchando sólo en `127.0.0.1` salvo que el Portador abra la red en Configuración → API.
//!
//! **Lo que ve el teléfono:** el texto del modelo, los pasos (actividad), los permisos pedidos con su
//! descripción ENTERA (el comando completo o el diff que se aprueba) y el estado. No el registro de
//! herramientas ni el contenido de archivos. Aprobar desde el teléfono vale lo mismo que un clic en la
//! ventana: ni más (la puerta de permisos ya decidió qué se pregunta; en bypass no hay preguntas), ni menos.
//! Las preguntas con opciones y las claves tapadas sólo se contestan en la Mac.
//!
//! **Mensajes:** sin la cola de F16.1 (no está en esta rama) un mensaje NO entra a mitad de un turno: se
//! encola y el chat lo atiende como su próxima orden al quedar libre (lo dice la respuesta y la ventana).
use crate::permissions::AskAnswer;
use crate::programar::codec;
use axum::body::Body;
use axum::extract::{ConnectInfo, Path as Ruta, State};
use axum::http::{header, HeaderMap, Request, StatusCode};
use axum::middleware::{self, Next};
use axum::response::sse::{Event as EventoSse, KeepAlive, Sse};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, VecDeque};
use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::oneshot;

mod frente;
mod pagina;
pub use frente::FrenteRemoto;

/// Eventos que se guardan por sesión para volver con `Last-Event-ID` (los más viejos se descartan y se dice).
pub const EVENTOS_MAX: usize = 5000;
/// Vigencia de un código de emparejar.
pub const CODIGO_VIGENCIA: Duration = Duration::from_secs(300);
/// Intentos fallidos de canjear un código antes de anularlos todos (un código de 8 cifras no se adivina a ciegas).
pub const CODIGO_FALLOS_MAX: u32 = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EstadoSesion {
    Libre,
    Trabajando,
    EsperandoPermiso,
}

impl EstadoSesion {
    pub fn clave(self) -> &'static str {
        match self {
            EstadoSesion::Libre => "libre",
            EstadoSesion::Trabajando => "trabajando",
            EstadoSesion::EsperandoPermiso => "esperando_permiso",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Evento {
    pub id: u64,
    pub tipo: &'static str,
    pub datos: Value,
}

struct Permiso {
    desc: String,
    tx: Option<oneshot::Sender<AskAnswer>>,
}

struct Sesion {
    proyecto: PathBuf,
    titulo: String,
    estado: EstadoSesion,
    eventos: VecDeque<Evento>,
    /// el id del evento más viejo que se descartó (para decir «se perdieron eventos»)
    descartado_hasta: u64,
    permisos: BTreeMap<String, Permiso>,
    cola: VecDeque<String>,
}

/// Un token emparejado (se guarda su sha256).
#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub id: String,
    pub nombre: String,
    pub hash: String,
    /// segundos UNIX
    pub creado: i64,
}

struct Codigo {
    codigo: String,
    vence: Instant,
}

#[derive(Default)]
struct Estado {
    sesiones: BTreeMap<String, Sesion>,
    siguiente: u64,
    codigos: Vec<Codigo>,
    fallos: u32,
    tokens: Vec<Token>,
}

/// El registro de sesiones vivas y de teléfonos emparejados de ESTE proceso (la app). Barato de clonar.
#[derive(Clone)]
pub struct Registro {
    e: Arc<Mutex<Estado>>,
    aviso: Arc<tokio::sync::Notify>,
    /// dónde se guardan los tokens (`None` = sólo en memoria)
    archivo: Option<PathBuf>,
}

impl Default for Registro {
    /// El de la app: los tokens en `~/.config/gotv/remoto.bin`.
    fn default() -> Self {
        Registro::con_archivo(Some(PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".config/gotv/remoto.bin")))
    }
}

/// Por qué no se aceptó una respuesta de permiso.
#[derive(Debug, Clone, PartialEq)]
pub enum Rechazo {
    SinSesion,
    SinPermiso,
    YaUsado,
}

fn ahora_s() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

fn sha(s: &str) -> String {
    format!("{:x}", Sha256::digest(s.as_bytes()))
}

/// Bytes aleatorios de `/dev/urandom`.
fn azar(n: usize) -> Vec<u8> {
    use std::io::Read;
    let mut b = vec![0u8; n];
    if let Ok(mut f) = std::fs::File::open("/dev/urandom") {
        let _ = f.read_exact(&mut b);
    }
    b
}

/// Comparación en tiempo constante.
fn igual(a: &[u8], b: &[u8]) -> bool {
    let mut d = (a.len() ^ b.len()) as u8 | u8::from(a.len() != b.len());
    for (i, x) in b.iter().enumerate() {
        d |= x ^ a.get(i).copied().unwrap_or(0);
    }
    d == 0
}

/// Los dos extremos de una conexión: quién llama (el par) y por qué dirección de ESTA Mac entró. `serve::run`
/// la pone en cada petición (`into_make_service_with_connect_info::<Conexion>`).
#[derive(Clone, Copy, Debug)]
pub struct Conexion {
    pub par: SocketAddr,
    pub local: SocketAddr,
}

impl axum::extract::connect_info::Connected<axum::serve::IncomingStream<'_, tokio::net::TcpListener>> for Conexion {
    fn connect_info(s: axum::serve::IncomingStream<'_, tokio::net::TcpListener>) -> Self {
        let par = *s.remote_addr();
        Conexion { par, local: s.io().local_addr().unwrap_or(par) }
    }
}

/// ¿Es una dirección de la red local (o de este equipo)?
pub fn es_red_local(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v) => v.is_loopback() || v.is_private() || v.is_link_local(),
        IpAddr::V6(v) => {
            if let Some(v4) = v.to_ipv4_mapped() {
                return es_red_local(IpAddr::V4(v4));
            }
            let s = v.segments()[0];
            v.is_loopback() || (s & 0xfe00) == 0xfc00 || (s & 0xffc0) == 0xfe80
        }
    }
}

/// 100.64/10: el espacio compartido que usan los túneles privados (Tailscale). También lo usan algunos operadores
/// para su red interna, así que NO basta con la dirección del par.
fn es_de_tunel(ip: IpAddr) -> bool {
    matches!(ip, IpAddr::V4(v) if v.octets()[0] == 100 && (v.octets()[1] & 0xc0) == 64)
}

/// ¿Se admite esta conexión? La red local siempre; una 100.64/10 SÓLO si entró por una interfaz de túnel de
/// esta Mac (`utun*`: Tailscale, WireGuard) — un vecino de la red de un operador que también use 100.x llega
/// por la Wi-Fi o el cable y se rechaza.
pub fn admitir(par: IpAddr, interfaz_local: Option<&str>) -> bool {
    es_red_local(par) || (es_de_tunel(par) && interfaz_local.is_some_and(|i| i.starts_with("utun")))
}

/// El nombre de la interfaz de esta Mac que tiene la dirección `ip` (`en0`, `lo0`, `utun3`…).
pub fn interfaz_de(ip: IpAddr) -> Option<String> {
    let mut lista: *mut libc::ifaddrs = std::ptr::null_mut();
    // SAFETY: getifaddrs llena una lista enlazada que se recorre sólo mientras vive y se libera con freeifaddrs
    unsafe {
        if libc::getifaddrs(&mut lista) != 0 {
            return None;
        }
        let mut p = lista;
        let mut hallada = None;
        while !p.is_null() {
            let ifa = &*p;
            if !ifa.ifa_addr.is_null() {
                let fam = (*ifa.ifa_addr).sa_family as i32;
                let dir = match fam {
                    libc::AF_INET => {
                        let a = &*(ifa.ifa_addr as *const libc::sockaddr_in);
                        Some(IpAddr::from(u32::from_be(a.sin_addr.s_addr).to_be_bytes()))
                    }
                    libc::AF_INET6 => {
                        let a = &*(ifa.ifa_addr as *const libc::sockaddr_in6);
                        Some(IpAddr::from(a.sin6_addr.s6_addr))
                    }
                    _ => None,
                };
                if dir == Some(ip) {
                    hallada = Some(std::ffi::CStr::from_ptr(ifa.ifa_name).to_string_lossy().into_owned());
                    break;
                }
            }
            p = ifa.ifa_next;
        }
        libc::freeifaddrs(lista);
        hallada
    }
}

/// El QR de `texto` como SVG (corrección M, margen de 4 módulos, negro sobre blanco para que lo lea cualquier
/// cámara también en modo oscuro). `None` si no cabe en un QR.
pub fn qr_svg(texto: &str) -> Option<String> {
    let q = qrcode::QrCode::with_error_correction_level(texto.as_bytes(), qrcode::EcLevel::M).ok()?;
    let n = q.width();
    let total = n + 8;
    let mut d = String::new();
    for (i, c) in q.to_colors().iter().enumerate() {
        if *c == qrcode::Color::Dark {
            d.push_str(&format!("M{} {}h1v1h-1z", i % n + 4, i / n + 4));
        }
    }
    Some(format!(r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {total} {total}" shape-rendering="crispEdges"><rect width="{total}" height="{total}" fill="#fff"/><path d="{d}" fill="#000"/></svg>"##))
}

/// La dirección de la página del teléfono con el código en el FRAGMENTO (`#c=…`): el navegador no lo manda al
/// servidor ni queda en ningún registro; la página lo lee, empareja y lo borra de la barra.
pub fn url_con_codigo(base: &str, codigo: &str) -> String {
    format!("{base}#c={codigo}")
}

impl Registro {
    pub fn con_archivo(archivo: Option<PathBuf>) -> Self {
        let tokens = archivo.as_deref().map(leer_tokens).unwrap_or_default();
        Registro { e: Arc::new(Mutex::new(Estado { tokens, ..Default::default() })), aviso: Arc::new(tokio::sync::Notify::new()), archivo }
    }

    // ── las sesiones (lado de la app) ─────────────────────────────────────────────────────────────

    /// Una sesión que se puede ver desde el teléfono (si ya estaba, se le cambian proyecto y título).
    pub fn abrir(&self, id: &str, proyecto: &Path, titulo: &str) {
        let mut e = self.e.lock().unwrap();
        let s = e.sesiones.entry(id.to_string()).or_insert_with(|| Sesion { proyecto: proyecto.to_path_buf(), titulo: titulo.to_string(), estado: EstadoSesion::Libre, eventos: VecDeque::new(), descartado_hasta: 0, permisos: BTreeMap::new(), cola: VecDeque::new() });
        s.proyecto = proyecto.to_path_buf();
        s.titulo = titulo.to_string();
    }

    /// Se cierra el chat: deja de verse; un permiso pendiente queda NEGADO.
    pub fn cerrar(&self, id: &str) {
        let s = self.e.lock().unwrap().sesiones.remove(id);
        for (_, mut p) in s.map(|s| s.permisos).unwrap_or_default() {
            if let Some(tx) = p.tx.take() {
                let _ = tx.send(AskAnswer::No);
            }
        }
        self.aviso.notify_waiters();
    }

    fn empujar(&self, e: &mut Estado, id: &str, tipo: &'static str, datos: Value) {
        e.siguiente += 1;
        let n = e.siguiente;
        if let Some(s) = e.sesiones.get_mut(id) {
            s.eventos.push_back(Evento { id: n, tipo, datos });
            while s.eventos.len() > EVENTOS_MAX {
                if let Some(v) = s.eventos.pop_front() {
                    s.descartado_hasta = v.id;
                }
            }
        }
    }

    pub fn evento(&self, id: &str, tipo: &'static str, datos: Value) {
        {
            let mut e = self.e.lock().unwrap();
            if !e.sesiones.contains_key(id) {
                return;
            }
            self.empujar(&mut e, id, tipo, datos);
        }
        self.aviso.notify_waiters();
    }

    pub fn estado(&self, id: &str, estado: EstadoSesion) {
        {
            let mut e = self.e.lock().unwrap();
            let Some(s) = e.sesiones.get_mut(id) else { return };
            if s.estado == estado {
                return;
            }
            s.estado = estado;
            self.empujar(&mut e, id, "estado", json!({"estado": estado.clave()}));
        }
        self.aviso.notify_waiters();
    }

    /// Un permiso que también se puede contestar desde el teléfono. Devuelve su id (de un solo uso) y por
    /// dónde llegará la respuesta del teléfono.
    pub fn pedir_permiso(&self, id: &str, desc: &str) -> (String, oneshot::Receiver<AskAnswer>) {
        let (tx, rx) = oneshot::channel();
        let idp = azar(9).iter().map(|b| format!("{b:02x}")).collect::<String>();
        {
            let mut e = self.e.lock().unwrap();
            if let Some(s) = e.sesiones.get_mut(id) {
                s.permisos.insert(idp.clone(), Permiso { desc: desc.to_string(), tx: Some(tx) });
                s.estado = EstadoSesion::EsperandoPermiso;
                self.empujar(&mut e, id, "permiso", json!({"id_permiso": idp, "descripcion": desc, "partes": crate::permissions::pregunta_en_partes(desc)}));
                // el cambio de estado también se cuenta (antes el teléfono seguía diciendo «trabajando»)
                self.empujar(&mut e, id, "estado", json!({"estado": EstadoSesion::EsperandoPermiso.clave()}));
            }
        }
        self.aviso.notify_waiters();
        (idp, rx)
    }

    /// El permiso se contestó en la ventana: el del teléfono ya no vale.
    pub fn permiso_resuelto(&self, id: &str, idp: &str, decision: AskAnswer) {
        {
            let mut e = self.e.lock().unwrap();
            let Some(s) = e.sesiones.get_mut(id) else { return };
            let Some(p) = s.permisos.get_mut(idp) else { return };
            if p.tx.take().is_none() {
                return;
            }
            s.estado = EstadoSesion::Trabajando;
            self.empujar(&mut e, id, "permiso_resuelto", json!({"id_permiso": idp, "aprobado": matches!(decision, AskAnswer::Yes | AskAnswer::Always), "desde": "ventana"}));
            self.empujar(&mut e, id, "estado", json!({"estado": EstadoSesion::Trabajando.clave()}));
        }
        self.aviso.notify_waiters();
    }

    /// La respuesta del teléfono: cada `id_permiso` vale una vez.
    pub fn responder(&self, id: &str, idp: &str, si: bool) -> Result<(), Rechazo> {
        {
            let mut e = self.e.lock().unwrap();
            let Some(s) = e.sesiones.get_mut(id) else { return Err(Rechazo::SinSesion) };
            let Some(p) = s.permisos.get_mut(idp) else { return Err(Rechazo::SinPermiso) };
            let Some(tx) = p.tx.take() else { return Err(Rechazo::YaUsado) };
            let _ = tx.send(if si { AskAnswer::Yes } else { AskAnswer::No });
            s.estado = EstadoSesion::Trabajando;
            self.empujar(&mut e, id, "permiso_resuelto", json!({"id_permiso": idp, "aprobado": si, "desde": "telefono"}));
            self.empujar(&mut e, id, "estado", json!({"estado": EstadoSesion::Trabajando.clave()}));
        }
        self.aviso.notify_waiters();
        Ok(())
    }

    /// Un mensaje para el chat (su próxima orden). Devuelve cuántos esperan.
    pub fn encolar(&self, id: &str, texto: &str) -> Option<usize> {
        let n = {
            let mut e = self.e.lock().unwrap();
            let s = e.sesiones.get_mut(id)?;
            s.cola.push_back(texto.to_string());
            let n = s.cola.len();
            self.empujar(&mut e, id, "mensaje", json!({"texto": texto, "en_cola": n}));
            n
        };
        self.aviso.notify_waiters();
        Some(n)
    }

    /// El siguiente mensaje que mandó el teléfono, si hay.
    pub fn tomar_mensaje(&self, id: &str) -> Option<String> {
        self.e.lock().unwrap().sesiones.get_mut(id)?.cola.pop_front()
    }

    /// Las sesiones vivas, para la lista del teléfono.
    pub fn lista(&self) -> Vec<Value> {
        let e = self.e.lock().unwrap();
        e.sesiones
            .iter()
            .map(|(id, s)| {
                let pendientes: Vec<Value> = s.permisos.iter().filter(|(_, p)| p.tx.is_some()).map(|(k, p)| json!({"id_permiso": k, "descripcion": p.desc, "partes": crate::permissions::pregunta_en_partes(&p.desc)})).collect();
                // `ruta`: la carpeta para leerla en pantalla (la casa como `~`)
                let casa = crate::paths::home();
                let ruta = s.proyecto.strip_prefix(&casa).map(|r| format!("~/{}", r.display())).unwrap_or_else(|_| s.proyecto.display().to_string());
                json!({"id": id, "proyecto": s.proyecto, "ruta": ruta, "titulo": s.titulo, "estado": s.estado.clave(), "permisos": pendientes, "en_cola": s.cola.len()})
            })
            .collect()
    }

    /// Los eventos posteriores a `ultimo`; `Err(descartados)` si el teléfono pide algo que ya se descartó.
    pub fn eventos_desde(&self, id: &str, ultimo: u64) -> Option<(Vec<Evento>, bool)> {
        let e = self.e.lock().unwrap();
        let s = e.sesiones.get(id)?;
        let perdidos = ultimo < s.descartado_hasta;
        Some((s.eventos.iter().filter(|v| v.id > ultimo).cloned().collect(), perdidos))
    }

    // ── emparejar teléfonos ───────────────────────────────────────────────────────────────────────

    /// Un código de un solo uso (8 cifras, 5 minutos) para enseñar en la ventana.
    pub fn codigo_nuevo(&self) -> String {
        let n = azar(8).iter().fold(0u64, |a, b| (a << 8) | *b as u64) % 100_000_000;
        let codigo = format!("{n:08}");
        let mut e = self.e.lock().unwrap();
        e.codigos.retain(|c| c.vence > Instant::now());
        e.codigos.push(Codigo { codigo: codigo.clone(), vence: Instant::now() + CODIGO_VIGENCIA });
        e.fallos = 0;
        codigo
    }

    /// Cambia un código por un token propio del teléfono (se guarda su sha256). `None` = código malo o
    /// vencido; tras [`CODIGO_FALLOS_MAX`] fallos se anulan todos los códigos.
    pub fn canjear(&self, codigo: &str, nombre: &str) -> Option<String> {
        let token = {
            let mut e = self.e.lock().unwrap();
            e.codigos.retain(|c| c.vence > Instant::now());
            let Some(i) = e.codigos.iter().position(|c| igual(c.codigo.as_bytes(), codigo.trim().as_bytes())) else {
                e.fallos += 1;
                if e.fallos >= CODIGO_FALLOS_MAX {
                    e.codigos.clear();
                }
                return None;
            };
            e.codigos.remove(i);
            let token = format!("gotvtel-{}", azar(24).iter().map(|b| format!("{b:02x}")).collect::<String>());
            let id = azar(4).iter().map(|b| format!("{b:02x}")).collect::<String>();
            let nombre: String = nombre.trim().chars().take(40).collect();
            e.tokens.push(Token { id, nombre: if nombre.is_empty() { crate::i18n::t("teléfono", "phone").into() } else { nombre }, hash: sha(&token), creado: ahora_s() });
            token
        };
        self.guardar();
        Some(token)
    }

    pub fn token_valido(&self, t: &str) -> bool {
        let h = sha(t.trim());
        self.e.lock().unwrap().tokens.iter().any(|x| igual(x.hash.as_bytes(), h.as_bytes()))
    }

    /// Los teléfonos emparejados (sin el hash).
    pub fn tokens(&self) -> Vec<Token> {
        self.e.lock().unwrap().tokens.iter().map(|t| Token { hash: String::new(), ..t.clone() }).collect()
    }

    /// «Olvidar este teléfono» desde el propio teléfono: revoca SU token (por su valor).
    pub fn revocar_token(&self, t: &str) -> bool {
        let h = sha(t.trim());
        let id = self.e.lock().unwrap().tokens.iter().find(|x| igual(x.hash.as_bytes(), h.as_bytes())).map(|x| x.id.clone());
        id.is_some_and(|id| self.revocar(&id))
    }

    /// Revoca un teléfono: sin acceso al momento (la próxima petición ya es 401).
    pub fn revocar(&self, id: &str) -> bool {
        let quitado = {
            let mut e = self.e.lock().unwrap();
            let antes = e.tokens.len();
            e.tokens.retain(|t| t.id != id);
            e.tokens.len() != antes
        };
        if quitado {
            self.guardar();
        }
        quitado
    }

    fn guardar(&self) {
        let Some(a) = &self.archivo else { return };
        let mut w = codec::Escritor::nuevo(b"REMO", 1);
        let tokens = self.e.lock().unwrap().tokens.clone();
        w.u32(tokens.len() as u32);
        for t in &tokens {
            w.texto(&t.id);
            w.texto(&t.nombre);
            w.texto(&t.hash);
            w.i64(t.creado);
        }
        let _ = codec::guardar(a, &w.bytes());
    }
}

fn leer_tokens(a: &Path) -> Vec<Token> {
    let Ok(b) = std::fs::read(a) else { return vec![] };
    let leer = || -> anyhow::Result<Vec<Token>> {
        let (mut l, _) = codec::Lector::nuevo(&b, b"REMO", 1)?;
        let n = l.u32()?;
        (0..n).map(|_| Ok(Token { id: l.texto()?, nombre: l.texto()?, hash: l.texto()?, creado: l.i64()? })).collect()
    };
    leer().unwrap_or_default()
}

// ── las rutas ────────────────────────────────────────────────────────────────────────────────────

/// ¿La cabecera trae la llave del servicio? (la decide `serve.rs`)
pub type Llave = Arc<dyn Fn(&HeaderMap) -> bool + Send + Sync>;

#[derive(Clone)]
struct Remoto {
    registro: Registro,
    llave: Llave,
}

/// Las rutas `/gotv/…` (se montan en el router de `serve.rs`).
pub fn rutas(registro: Registro, llave: Llave) -> Router {
    let st = Remoto { registro, llave };
    Router::new()
        .route("/gotv/movil", get(|| async { Html(pagina::PAGINA) }))
        .route("/gotv/emparejar", post(emparejar))
        .route("/gotv/sesiones", get(sesiones))
        .route("/gotv/sesiones/{id}/eventos", get(eventos))
        .route("/gotv/sesiones/{id}/permiso", post(permiso))
        .route("/gotv/sesiones/{id}/mensaje", post(mensaje))
        .route("/gotv/salir", post(salir))
        .layer(middleware::from_fn_with_state(st.clone(), guardia))
        .with_state(st)
}

fn rechazo(code: StatusCode, msg: &str) -> Response {
    (code, Json(json!({"error": msg}))).into_response()
}

/// Red local y llave, ANTES que nada. La dirección es la del par TCP; sin ella, se niega.
async fn guardia(State(st): State<Remoto>, req: Request<Body>, next: Next) -> Response {
    let con = req.extensions().get::<ConnectInfo<Conexion>>().map(|c| c.0);
    let admitida = con.is_some_and(|c| {
        // la interfaz sólo se mira si hace falta (un par 100.64/10): leerla cuesta una llamada al sistema
        let interfaz = if es_de_tunel(c.par.ip()) { interfaz_de(c.local.ip()) } else { None };
        admitir(c.par.ip(), interfaz.as_deref())
    });
    if !admitida {
        return rechazo(StatusCode::FORBIDDEN, crate::i18n::t("sólo desde la red local", "only from the local network"));
    }
    let ruta = req.uri().path();
    if ruta == "/gotv/movil" || ruta == "/gotv/emparejar" {
        return next.run(req).await;
    }
    let h = req.headers();
    let bearer = h.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()).and_then(|v| v.strip_prefix("Bearer ").or_else(|| v.strip_prefix("bearer "))).unwrap_or("");
    // la página del teléfono lleva su token en una cookie HttpOnly (la pone el emparejado): el JS no lo toca
    let bearer = if bearer.is_empty() { token_de_cookie(h).unwrap_or("") } else { bearer };
    if !(st.llave)(h) && (bearer.is_empty() || !st.registro.token_valido(bearer)) {
        return rechazo(StatusCode::UNAUTHORIZED, crate::i18n::t("falta la llave o el token del teléfono", "missing key or phone token"));
    }
    next.run(req).await
}

/// La cookie del teléfono emparejado. HttpOnly (el JS de la página no la lee: nada de `localStorage`),
/// SameSite=Strict (otra página no puede usarla para aprobar nada) y sólo para `/gotv`.
pub const COOKIE: &str = "gotv_tel";

fn token_de_cookie(h: &HeaderMap) -> Option<&str> {
    h.get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .find_map(|par| par.trim().strip_prefix(COOKIE).and_then(|r| r.strip_prefix('=')))
        .filter(|t| !t.is_empty())
}

fn con_cookie(mut r: Response, valor: &str, vida_s: u64) -> Response {
    if let Ok(v) = format!("{COOKIE}={valor}; Path=/gotv; HttpOnly; SameSite=Strict; Max-Age={vida_s}").parse() {
        r.headers_mut().append(header::SET_COOKIE, v);
    }
    r
}

async fn emparejar(State(st): State<Remoto>, Json(v): Json<Value>) -> Response {
    match st.registro.canjear(v["codigo"].as_str().unwrap_or(""), v["nombre"].as_str().unwrap_or("")) {
        // el token va también en el cuerpo (un cliente de API lo usa como `Authorization: Bearer`)
        Some(t) => con_cookie(Json(json!({"token": t})).into_response(), &t, 400 * 24 * 3600),
        None => rechazo(StatusCode::UNAUTHORIZED, crate::i18n::t("código malo o vencido", "wrong or expired code")),
    }
}

/// «Olvidar este teléfono»: revoca su token y borra la cookie.
async fn salir(State(st): State<Remoto>, h: HeaderMap) -> Response {
    if let Some(t) = token_de_cookie(&h) {
        st.registro.revocar_token(t);
    }
    con_cookie(Json(json!({"ok": true})).into_response(), "", 0)
}

async fn sesiones(State(st): State<Remoto>) -> Json<Value> {
    Json(json!({"sesiones": st.registro.lista()}))
}

async fn permiso(State(st): State<Remoto>, Ruta(id): Ruta<String>, Json(v): Json<Value>) -> Response {
    let si = match v["decision"].as_str() {
        Some("si" | "sí" | "yes" | "aprobar") => true, // juez-idioma: datos (valores del protocolo)
        Some("no" | "negar" | "deny") => false,        // juez-idioma: datos (valores del protocolo)
        _ => return rechazo(StatusCode::BAD_REQUEST, "decision: si | no"),
    };
    match st.registro.responder(&id, v["id_permiso"].as_str().unwrap_or(""), si) {
        Ok(()) => Json(json!({"ok": true})).into_response(),
        Err(Rechazo::SinSesion) => rechazo(StatusCode::NOT_FOUND, crate::i18n::t("no hay ese chat", "no such chat")),
        Err(Rechazo::SinPermiso) => rechazo(StatusCode::NOT_FOUND, crate::i18n::t("no hay ese permiso", "no such permission")),
        Err(Rechazo::YaUsado) => rechazo(StatusCode::CONFLICT, crate::i18n::t("ese permiso ya se contestó", "that permission was already answered")),
    }
}

async fn mensaje(State(st): State<Remoto>, Ruta(id): Ruta<String>, Json(v): Json<Value>) -> Response {
    let t = v["texto"].as_str().unwrap_or("").trim();
    if t.is_empty() {
        return rechazo(StatusCode::BAD_REQUEST, "texto");
    }
    match st.registro.encolar(&id, t) {
        Some(n) => (StatusCode::ACCEPTED, Json(json!({"en_cola": n, "nota": crate::i18n::t("se atiende como la próxima orden del chat", "it is handled as the chat's next order")}))).into_response(),
        None => rechazo(StatusCode::NOT_FOUND, crate::i18n::t("no hay ese chat", "no such chat")),
    }
}

/// SSE con `Last-Event-ID` (o `?desde=`): repite lo posterior y sigue en vivo. Si lo pedido ya se descartó,
/// el primer evento es `perdidos`.
async fn eventos(State(st): State<Remoto>, Ruta(id): Ruta<String>, h: HeaderMap, q: axum::extract::Query<BTreeMap<String, String>>) -> Response {
    let ultimo: u64 = h.get("last-event-id").and_then(|v| v.to_str().ok()).or(q.get("desde").map(String::as_str)).and_then(|v| v.trim().parse().ok()).unwrap_or(0);
    let Some((_, perdidos)) = st.registro.eventos_desde(&id, ultimo) else {
        return rechazo(StatusCode::NOT_FOUND, crate::i18n::t("no hay ese chat", "no such chat"));
    };
    let estado = (st.registro, id, ultimo, VecDeque::<Evento>::new(), perdidos);
    let flujo = futures_util::stream::unfold(estado, |(reg, id, mut ultimo, mut buf, mut perdidos)| async move {
        if perdidos {
            perdidos = false;
            let ev = EventoSse::default().event("perdidos").data(json!({"desde": ultimo}).to_string());
            return Some((Ok::<_, std::convert::Infallible>(ev), (reg, id, ultimo, buf, perdidos)));
        }
        loop {
            if let Some(v) = buf.pop_front() {
                ultimo = v.id;
                let ev = EventoSse::default().id(v.id.to_string()).event(v.tipo).data(v.datos.to_string());
                return Some((Ok(ev), (reg, id, ultimo, buf, perdidos)));
            }
            // se apunta ANTES de mirar: un evento que llegue entre mirar y esperar no se pierde
            let aviso = reg.aviso.clone();
            let espera = aviso.notified();
            tokio::pin!(espera);
            espera.as_mut().enable();
            match reg.eventos_desde(&id, ultimo) {
                None => return None, // el chat se cerró: fin del flujo
                Some((nuevos, _)) if !nuevos.is_empty() => buf.extend(nuevos),
                Some(_) => {
                    let _ = tokio::time::timeout(Duration::from_secs(30), espera).await;
                }
            }
        }
    });
    Sse::new(flujo).keep_alive(KeepAlive::new().interval(Duration::from_secs(15))).into_response()
}

#[cfg(test)]
mod tests;
