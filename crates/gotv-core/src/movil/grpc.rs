//! F18 · gRPC mínimo sobre HTTP/2 (`hyper`), por socket Unix (idb) o por TCP local (emulador de Android).
//!
//! gRPC es HTTP/2 con un `POST /paquete.Servicio/metodo`, `content-type: application/grpc`, cada mensaje con
//! 5 bytes delante (1 de compresión, 4 de largo) y el resultado en los trailers (`grpc-status`,
//! `grpc-message`). Eso cabe aquí sin `tonic`: unario, flujo del cliente (todos los mensajes en un cuerpo) y
//! flujo del servidor leído hasta que una condición se cumple.
use anyhow::{anyhow, bail, Result};
use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::client::conn::http2::SendRequest;
use hyper_util::rt::{TokioExecutor, TokioIo};
use std::path::PathBuf;
use std::time::Duration;

/// Dónde escucha el servicio.
#[derive(Clone, Debug, PartialEq)]
pub enum Destino {
    Unix(PathBuf),
    /// Puerto en 127.0.0.1 (nunca otra máquina).
    Tcp(u16),
}

/// Un error con su código de gRPC (12 = UNIMPLEMENTED, 3 = INVALID_ARGUMENT, 4 = DEADLINE_EXCEEDED…).
#[derive(Debug)]
pub struct ErrorGrpc {
    pub codigo: u32,
    pub mensaje: String,
}

impl std::fmt::Display for ErrorGrpc {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "gRPC {}: {}", self.codigo, self.mensaje)
    }
}
impl std::error::Error for ErrorGrpc {}

/// Cada mensaje con su prefijo de 5 bytes (sin compresión).
pub fn enmarcar(mensajes: &[Vec<u8>]) -> Vec<u8> {
    let mut b = Vec::new();
    for m in mensajes {
        b.push(0);
        b.extend_from_slice(&(m.len() as u32).to_be_bytes());
        b.extend_from_slice(m);
    }
    b
}

/// Saca los mensajes COMPLETOS de `b`; lo que sobra (un mensaje a medias) queda en `b`.
pub fn desenmarcar(b: &mut Vec<u8>) -> Result<Vec<Vec<u8>>> {
    let mut out = Vec::new();
    let mut i = 0;
    while b.len() >= i + 5 {
        if b[i] != 0 {
            bail!("{}", crate::i18n::t("gRPC: el servicio mandó un mensaje comprimido (no se pidió compresión)", "gRPC: the service sent a compressed message (no compression was requested)"));
        }
        let n = u32::from_be_bytes([b[i + 1], b[i + 2], b[i + 3], b[i + 4]]) as usize;
        if b.len() < i + 5 + n {
            break;
        }
        out.push(b[i + 5..i + 5 + n].to_vec());
        i += 5 + n;
    }
    b.drain(..i);
    Ok(out)
}

/// `grpc-message` viene con `%XX`.
fn des_por_ciento(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Some(v) = std::str::from_utf8(&b[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

async fn mano<I>(io: I) -> Result<SendRequest<Full<Bytes>>>
where
    I: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let (envio, conexion) = hyper::client::conn::http2::handshake(TokioExecutor::new(), TokioIo::new(io)).await?;
    tokio::spawn(async move {
        let _ = conexion.await;
    });
    Ok(envio)
}

async fn conectar(d: &Destino) -> Result<SendRequest<Full<Bytes>>> {
    match d {
        Destino::Unix(p) => mano(tokio::net::UnixStream::connect(p).await.map_err(|e| anyhow!("{}: {e}", p.display()))?).await,
        Destino::Tcp(puerto) => mano(tokio::net::TcpStream::connect(("127.0.0.1", *puerto)).await.map_err(|e| anyhow!("127.0.0.1:{puerto}: {e}"))?).await,
    }
}

fn estado(h: Option<&hyper::HeaderMap>) -> Option<Result<(), ErrorGrpc>> {
    let h = h?;
    let c: u32 = h.get("grpc-status")?.to_str().ok()?.trim().parse().ok()?;
    if c == 0 {
        return Some(Ok(()));
    }
    // un servicio que no codifica con %XX sus bytes no ASCII: se leen igual (no se pierde el mensaje)
    let m = h.get("grpc-message").map(|v| des_por_ciento(&String::from_utf8_lossy(v.as_bytes()))).unwrap_or_default();
    Some(Err(ErrorGrpc { codigo: c, mensaje: m }))
}

/// Llama a `ruta` («/idb.CompanionService/hid») con `mensajes` y lee las respuestas hasta el final o hasta
/// que `basta` diga que ya (flujo del servidor sin fin, como la quietud de idb). Todo con tope de tiempo.
pub async fn llamar_hasta(
    d: &Destino,
    ruta: &str,
    mensajes: &[Vec<u8>],
    token: Option<&str>,
    tope: Duration,
    mut basta: impl FnMut(&[u8]) -> bool + Send,
) -> Result<Vec<Vec<u8>>> {
    let trabajo = async {
        let mut envio = conectar(d).await?;
        let mut req = hyper::Request::post(format!("http://localhost{ruta}")).header("content-type", "application/grpc").header("te", "trailers");
        if let Some(t) = token {
            req = req.header("authorization", format!("Bearer {t}"));
        }
        let resp = envio.send_request(req.body(Full::new(Bytes::from(enmarcar(mensajes))))?).await?;
        let (partes, mut cuerpo) = resp.into_parts();
        if !partes.status.is_success() {
            bail!("{}", crate::tr!("gRPC {ruta}: HTTP {}", "gRPC {ruta}: HTTP {}", partes.status));
        }
        // «solo trailers»: el error viene ya en las cabeceras
        if let Some(Err(e)) = estado(Some(&partes.headers)) {
            return Err(anyhow::Error::new(e));
        }
        let mut pendiente = Vec::new();
        let mut out = Vec::new();
        while let Some(marco) = cuerpo.frame().await {
            let marco = marco?;
            if let Some(datos) = marco.data_ref() {
                pendiente.extend_from_slice(datos);
                for m in desenmarcar(&mut pendiente)? {
                    let fin = basta(&m);
                    out.push(m);
                    if fin {
                        return Ok(out);
                    }
                }
            } else if let Some(t) = marco.trailers_ref() {
                if let Some(Err(e)) = estado(Some(t)) {
                    return Err(anyhow::Error::new(e));
                }
            }
        }
        Ok(out)
    };
    tokio::time::timeout(tope, trabajo).await.map_err(|_| anyhow!("{}", crate::tr!("gRPC {ruta}: sin respuesta en {} s", "gRPC {ruta}: no answer in {} s", tope.as_secs_f32())))?
}

/// Unario o flujo del cliente: todas las respuestas.
pub async fn llamar(d: &Destino, ruta: &str, mensajes: &[Vec<u8>], token: Option<&str>, tope: Duration) -> Result<Vec<Vec<u8>>> {
    llamar_hasta(d, ruta, mensajes, token, tope, |_| false).await
}

/// Unario: la única respuesta.
pub async fn unario(d: &Destino, ruta: &str, mensaje: Vec<u8>, token: Option<&str>, tope: Duration) -> Result<Vec<u8>> {
    llamar(d, ruta, &[mensaje], token, tope).await?.into_iter().next().ok_or_else(|| anyhow!("{}", crate::tr!("gRPC {ruta}: respuesta vacía", "gRPC {ruta}: empty answer")))
}

/// El código de gRPC de un error, si lo es.
pub fn codigo(e: &anyhow::Error) -> Option<u32> {
    e.downcast_ref::<ErrorGrpc>().map(|g| g.codigo)
}

/// Un servidor gRPC FALSO para las pruebas (el mismo HTTP/2 de `hyper`): cada llamada va a `manejar` con
/// su ruta, sus mensajes y sus cabeceras; la respuesta lleva sus trailers como uno de verdad.
#[cfg(test)]
pub mod falso {
    use super::*;
    use std::convert::Infallible;
    use std::pin::Pin;
    use std::sync::Arc;
    use std::task::{Context, Poll};

    pub type Manejador = Arc<dyn Fn(&str, Vec<Vec<u8>>, &hyper::HeaderMap) -> Result<Vec<Vec<u8>>, (u32, String)> + Send + Sync>;

    struct Cuerpo {
        datos: Option<Bytes>,
        trailers: Option<hyper::HeaderMap>,
    }

    impl hyper::body::Body for Cuerpo {
        type Data = Bytes;
        type Error = Infallible;
        fn poll_frame(mut self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Option<Result<hyper::body::Frame<Bytes>, Infallible>>> {
            if let Some(d) = self.datos.take() {
                return Poll::Ready(Some(Ok(hyper::body::Frame::data(d))));
            }
            if let Some(t) = self.trailers.take() {
                return Poll::Ready(Some(Ok(hyper::body::Frame::trailers(t))));
            }
            Poll::Ready(None)
        }
    }

    async fn atender<I>(io: I, m: Manejador)
    where
        I: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
    {
        let svc = hyper::service::service_fn(move |req: hyper::Request<hyper::body::Incoming>| {
            let m = m.clone();
            async move {
                let ruta = req.uri().path().to_string();
                let cab = req.headers().clone();
                let mut b = req.into_body().collect().await.map(|c| c.to_bytes().to_vec()).unwrap_or_default();
                let msgs = desenmarcar(&mut b).unwrap_or_default();
                let mut t = hyper::HeaderMap::new();
                let datos = match m(&ruta, msgs, &cab) {
                    Ok(r) => {
                        t.insert("grpc-status", "0".parse().unwrap());
                        Some(Bytes::from(enmarcar(&r)))
                    }
                    Err((c, s)) => {
                        t.insert("grpc-status", c.to_string().parse().unwrap());
                        // como pide gRPC: lo que no es ASCII imprimible (ni `%`), en %XX
                        let cod: String = s.bytes().map(|b| if (0x20..0x7f).contains(&b) && b != b'%' { (b as char).to_string() } else { format!("%{b:02X}") }).collect();
                        t.insert("grpc-message", cod.parse().unwrap());
                        None
                    }
                };
                Ok::<_, Infallible>(hyper::Response::builder().header("content-type", "application/grpc").body(Cuerpo { datos, trailers: Some(t) }).unwrap())
            }
        });
        let _ = hyper::server::conn::http2::Builder::new(TokioExecutor::new()).serve_connection(TokioIo::new(io), svc).await;
    }

    pub async fn servir_unix(l: tokio::net::UnixListener, m: Manejador) {
        while let Ok((s, _)) = l.accept().await {
            tokio::spawn(atender(s, m.clone()));
        }
    }

    pub async fn servir_tcp(l: tokio::net::TcpListener, m: Manejador) {
        while let Ok((s, _)) = l.accept().await {
            tokio::spawn(atender(s, m.clone()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[test]
    fn marcos_partidos_esperan_al_resto() {
        let mut b = enmarcar(&[b"hola".to_vec(), vec![], "ñ".as_bytes().to_vec()]);
        let cola = b.split_off(b.len() - 2);
        let primeros = desenmarcar(&mut b).unwrap();
        assert_eq!(primeros, vec![b"hola".to_vec(), vec![]]);
        assert_eq!(b.len(), 5, "el tercero queda a medias");
        b.extend(cola);
        assert_eq!(desenmarcar(&mut b).unwrap(), vec!["ñ".as_bytes().to_vec()]);
        assert!(b.is_empty());
        let mut comprimido = vec![1, 0, 0, 0, 0];
        assert!(desenmarcar(&mut comprimido).is_err());
        assert_eq!(des_por_ciento("no%20hay%3A%20%C3%B1"), "no hay: ñ");
        assert_eq!(des_por_ciento("100%"), "100%");
    }

    /// De punta a punta por socket Unix Y por TCP: unario, flujo del cliente, error con código y
    /// mensaje, la cabecera de autorización y el tope de tiempo.
    #[tokio::test]
    async fn cliente_contra_servidor_falso_unix_y_tcp() {
        let d = crate::CarpetaPrueba::nueva("gotv-grpc");
        let sock = d.join("s.sock");
        let vistos: Arc<Mutex<Vec<(String, usize, Option<String>)>>> = Default::default();
        let v2 = vistos.clone();
        let m: falso::Manejador = Arc::new(move |ruta, msgs, cab| {
            v2.lock().unwrap().push((ruta.to_string(), msgs.len(), cab.get("authorization").map(|v| v.to_str().unwrap().to_string())));
            match ruta {
                "/x.S/eco" => Ok(msgs),
                "/x.S/contar" => Ok(vec![vec![msgs.len() as u8]]),
                "/x.S/dormir" => {
                    std::thread::sleep(Duration::from_millis(800));
                    Ok(vec![])
                }
                _ => Err((12, "método desconocido: no".into())),
            }
        });
        tokio::spawn(falso::servir_unix(tokio::net::UnixListener::bind(&sock).unwrap(), m.clone()));
        let tcp = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let puerto = tcp.local_addr().unwrap().port();
        tokio::spawn(falso::servir_tcp(tcp, m));
        let t = Duration::from_secs(5);
        for dest in [Destino::Unix(sock.clone()), Destino::Tcp(puerto)] {
            assert_eq!(unario(&dest, "/x.S/eco", "ñandú".as_bytes().to_vec(), None, t).await.unwrap(), "ñandú".as_bytes());
            assert_eq!(llamar(&dest, "/x.S/contar", &[vec![1], vec![2], vec![3]], Some("ficha"), t).await.unwrap(), vec![vec![3]]);
            let e = unario(&dest, "/x.S/nada", vec![], None, t).await.unwrap_err();
            assert_eq!(codigo(&e), Some(12), "{e}");
            assert!(e.to_string().contains("método desconocido: no"), "{e}");
            let e = unario(&dest, "/x.S/dormir", vec![], None, Duration::from_millis(200)).await.unwrap_err();
            assert!(codigo(&e).is_none() && e.to_string().contains("0.2"), "{e}");
        }
        let v = vistos.lock().unwrap().clone();
        assert!(v.iter().any(|(r, n, a)| r == "/x.S/contar" && *n == 3 && a.as_deref() == Some("Bearer ficha")), "{v:?}");
        // nadie escuchando: error claro, sin colgarse
        let e = unario(&Destino::Unix(d.join("no.sock")), "/x.S/eco", vec![], None, t).await.unwrap_err();
        assert!(e.to_string().contains("no.sock"), "{e}");
    }
}
