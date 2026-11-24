//! F4 paso 3 · La sesión de kernel: lanzarlo (proceso de `exec.rs`, en la caja del chat), conectarse por ZeroMQ
//! con sus mensajes FIRMADOS, `execute_request`, recoger lo que publica (texto, imágenes, errores con su traza),
//! interrumpir y apagar.
//!
//! Trampa 4 de la ficha: nunca se espera sin tope. Pasado el tope de la celda: primero se interrumpe
//! (`interrupt_request` si el kernel lo pide así, si no la señal SIGINT); si tampoco vuelve, se mata.
//! Trampa 6: los puertos son de `127.0.0.1` y el archivo de conexión va en la temporal propia de la caja.

use super::formato::lineas;
use super::kernels::Especificacion;
use super::protocolo::{id_nuevo, Firma, Mensaje};
use anyhow::{anyhow, bail, Result};
use bytes::Bytes;
use serde_json::{json, Map, Value};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use zeromq::{DealerSocket, Socket, SocketOptions, SocketRecv, SocketSend, SubSocket, ZmqMessage};

/// Tras interrumpir, cuánto se espera a que la celda vuelva antes de matar el kernel.
const GRACIA: Duration = Duration::from_secs(8);
/// El arranque de un kernel (importar ipykernel la primera vez tarda).
const ARRANQUE: Duration = Duration::from_secs(90);

/// Los puertos y la clave de un kernel (el archivo de conexión de Jupyter).
#[derive(Clone, Debug)]
pub struct Conexion {
    pub shell: u16,
    pub iopub: u16,
    pub stdin: u16,
    pub control: u16,
    pub hb: u16,
    pub clave: String,
}

impl Conexion {
    /// Cinco puertos libres de `127.0.0.1` y una clave nueva. Se sueltan para que el kernel los ate: entre
    /// soltarlos y atarlos otro programa puede tomar uno (la misma carrera que Jupyter); `lanzar` lo reintenta.
    pub fn nueva() -> Result<Conexion> {
        Ok(Conexion::con_oyentes()?.0)
    }

    /// Los puertos con sus sockets aún ATADOS (shell, iopub, stdin, control, hb): quien los use no compite.
    pub fn con_oyentes() -> Result<(Conexion, Vec<std::net::TcpListener>)> {
        let ls: Vec<std::net::TcpListener> = (0..5).map(|_| std::net::TcpListener::bind("127.0.0.1:0")).collect::<std::io::Result<_>>()?;
        let p: Vec<u16> = ls.iter().map(|l| l.local_addr().map(|a| a.port())).collect::<std::io::Result<_>>()?;
        Ok((Conexion { shell: p[0], iopub: p[1], stdin: p[2], control: p[3], hb: p[4], clave: id_nuevo() }, ls))
    }

    pub fn json(&self, kernel: &str) -> Value {
        json!({
            "shell_port": self.shell, "iopub_port": self.iopub, "stdin_port": self.stdin,
            "control_port": self.control, "hb_port": self.hb, "ip": "127.0.0.1", "key": self.clave,
            "transport": "tcp", "signature_scheme": "hmac-sha256", "kernel_name": kernel,
        })
    }

    pub fn leer(p: &Path) -> Result<Conexion> {
        let v: Value = serde_json::from_str(&std::fs::read_to_string(p)?)?;
        let n = |k: &str| v[k].as_u64().map(|x| x as u16).ok_or_else(|| anyhow!("{k}?"));
        Ok(Conexion { shell: n("shell_port")?, iopub: n("iopub_port")?, stdin: n("stdin_port")?, control: n("control_port")?, hb: n("hb_port")?, clave: v["key"].as_str().unwrap_or("").to_string() })
    }

    /// Lo escribe con permisos 0600 (la clave firma lo que el kernel ejecuta).
    pub fn escribir(&self, p: &Path, kernel: &str) -> Result<()> {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(p)?;
        f.write_all(self.json(kernel).to_string().as_bytes())?;
        Ok(())
    }
}

/// Cómo terminó una celda.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Fin {
    Ok,
    Error,
    /// pasó el tope y se interrumpió (el kernel sigue vivo, con su estado)
    Interrumpida,
    /// tampoco volvió tras interrumpir: se mató (el estado se perdió)
    Matado,
    /// el kernel murió solo durante la celda
    Murio,
}

pub struct Resultado {
    /// las salidas en formato nbformat (como quedan en el `.ipynb`)
    pub salidas: Vec<Value>,
    pub cuenta: Option<u64>,
    pub fin: Fin,
    pub segundos: f32,
}

/// Lo que publica el kernel para UNA petición, ya en formato nbformat.
#[derive(Default)]
pub struct Coleccion {
    pub salidas: Vec<Value>,
    /// el `display_id` de cada salida (para `update_display_data`)
    ids: Vec<Option<String>>,
    limpiar_al_siguiente: bool,
    pub cuenta: Option<u64>,
}

/// Los datos MIME como los guarda nbformat: el texto partido en líneas (también JavaScript y SVG); lo demás
/// (PNG en base64, JSON) tal cual.
fn datos(d: &Value) -> Value {
    let Some(m) = d.as_object() else { return json!({}) };
    let mut o = Map::new();
    for (k, v) in m {
        let partir = k.starts_with("text/") || k == "application/javascript" || k == "image/svg+xml";
        o.insert(k.clone(), match v.as_str() {
            Some(s) if partir => lineas(s),
            _ => v.clone(),
        });
    }
    Value::Object(o)
}

impl Coleccion {
    fn antes_de_salida(&mut self) {
        if std::mem::take(&mut self.limpiar_al_siguiente) {
            self.salidas.clear();
            self.ids.clear();
        }
    }

    fn poner(&mut self, v: Value, id: Option<String>) {
        self.antes_de_salida();
        self.salidas.push(v);
        self.ids.push(id);
    }

    /// Suma un mensaje de iopub; `true` cuando el kernel vuelve a quedar ocioso.
    pub fn mensaje(&mut self, m: &Mensaje) -> bool {
        let c = &m.content;
        let id_display = c["transient"]["display_id"].as_str().map(String::from);
        let meta = if c["metadata"].is_object() { c["metadata"].clone() } else { json!({}) };
        match m.tipo() {
            "status" => return c["execution_state"] == "idle",
            "execute_input" => self.cuenta = c["execution_count"].as_u64().or(self.cuenta),
            "clear_output" => {
                if c["wait"].as_bool() == Some(true) {
                    self.limpiar_al_siguiente = true;
                } else {
                    self.salidas.clear();
                    self.ids.clear();
                }
            }
            "stream" => {
                self.antes_de_salida();
                let nombre = c["name"].as_str().unwrap_or("stdout");
                let texto = c["text"].as_str().unwrap_or("");
                // dos `stream` seguidos del mismo nombre son UNA salida (como las junta Jupyter)
                if let Some(u) = self.salidas.last_mut().filter(|u| u["output_type"] == "stream" && u["name"] == nombre) {
                    let junto = super::formato::texto_de(&u["text"]) + texto;
                    u["text"] = lineas(&junto);
                } else {
                    self.poner(json!({"name": nombre, "output_type": "stream", "text": lineas(texto)}), None);
                }
            }
            "display_data" => self.poner(json!({"data": datos(&c["data"]), "metadata": meta, "output_type": "display_data"}), id_display),
            "update_display_data" => {
                for (k, id) in self.ids.iter().enumerate() {
                    if id.is_some() && *id == id_display {
                        self.salidas[k]["data"] = datos(&c["data"]);
                        self.salidas[k]["metadata"] = meta.clone();
                    }
                }
            }
            "execute_result" => {
                let n = c["execution_count"].as_u64();
                self.cuenta = n.or(self.cuenta);
                self.poner(json!({"data": datos(&c["data"]), "execution_count": n, "metadata": meta, "output_type": "execute_result"}), id_display)
            }
            "error" => self.poner(
                json!({"ename": c["ename"].as_str().unwrap_or("Error"), "evalue": c["evalue"].as_str().unwrap_or(""), "output_type": "error", "traceback": c["traceback"].as_array().cloned().unwrap_or_default()}),
                None,
            ),
            _ => {}
        }
        false
    }
}

enum Canal {
    Shell,
    Iopub,
    Control,
}

fn zmq(t: Vec<Bytes>) -> ZmqMessage {
    // nunca vacío: siempre lleva el separador, la firma y las cuatro partes
    ZmqMessage::try_from(t).unwrap_or_else(|_| ZmqMessage::from(""))
}

/// Si el futuro de una celda se suelta a medias (el usuario interrumpió el turno), el kernel recibe Ctrl-C:
/// no se queda corriendo lo que nadie espera.
struct Guarda {
    proc: Option<Arc<crate::exec::Proc>>,
    activa: bool,
}

impl Drop for Guarda {
    fn drop(&mut self) {
        if self.activa {
            if let Some(p) = &self.proc {
                p.interrumpir();
            }
        }
    }
}

pub struct Kernel {
    pub spec: Especificacion,
    proc: Option<Arc<crate::exec::Proc>>,
    archivo: Option<PathBuf>,
    shell: DealerSocket,
    control: DealerSocket,
    iopub: SubSocket,
    firma: Firma,
    sesion: String,
    /// el lenguaje y su versión, como los dice el kernel (`kernel_info_reply`)
    pub lenguaje: String,
    pub version: String,
    pub ejecutadas: u64,
    pub ultimo: Instant,
}

fn comillas(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

impl Kernel {
    /// Lanza el kernel como proceso de `exec` (en la caja del chat, si la hay) y se conecta. Si otro programa le
    /// ganó un puerto entre elegirlo y atarlo, se reintenta UNA vez con puertos nuevos.
    pub async fn lanzar(dueno: &str, root: &Path, cwd: &Path, spec: Especificacion) -> Result<Kernel> {
        match Kernel::lanzar_una(dueno, root, cwd, spec.clone()).await {
            Err(e) if e.to_string().contains("Address already in use") => Kernel::lanzar_una(dueno, root, cwd, spec).await,
            r => r,
        }
    }

    async fn lanzar_una(dueno: &str, root: &Path, cwd: &Path, spec: Especificacion) -> Result<Kernel> {
        let con = Conexion::nueva()?;
        let tmp = crate::sandbox::temporal_propia(root);
        std::fs::create_dir_all(&tmp)?;
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o700));
        }
        let archivo = tmp.join(format!("kernel-{}.json", &id_nuevo()[..12]));
        con.escribir(&archivo, &spec.nombre)?;
        let (argv, mut env) = spec.orden(&archivo);
        // medido 2026-11-17 con ipykernel real en la caja: matplotlib no puede escribir `~/.matplotlib`, crea una
        // caché temporal NUEVA en cada kernel y rehace la de fuentes (59 s la primera gráfica, cada vez). Su
        // configuración va a la temporal propia de la caja (escribible y persistente), salvo que ya la fijen.
        if !env.contains_key("MPLCONFIGDIR") && std::env::var_os("MPLCONFIGDIR").is_none() {
            let mpl = tmp.join("matplotlib");
            let _ = std::fs::create_dir_all(&mpl);
            env.insert("MPLCONFIGDIR".into(), mpl.display().to_string());
        }
        let mut cmd = String::new();
        if cwd != root {
            cmd.push_str(&format!("cd {} && ", comillas(&cwd.display().to_string())));
        }
        for (k, v) in env.iter().filter(|(k, _)| !k.is_empty() && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')) {
            cmd.push_str(&format!("export {k}={} && ", comillas(v)));
        }
        cmd.push_str("exec");
        for a in &argv {
            cmd.push(' ');
            cmd.push_str(&comillas(a));
        }
        let proc = crate::exec::spawn(dueno, &cmd, root, false, Some(&format!("kernel {}", spec.nombre)), false)?;
        match Kernel::conectar(&con, Some(proc.clone()), spec, Some(archivo.clone())).await {
            Ok(k) => Ok(k),
            Err(e) => {
                tokio::time::sleep(Duration::from_millis(300)).await;
                let cola = proc.cola(12);
                proc.stop().await;
                let _ = std::fs::remove_file(&archivo);
                bail!(
                    "{}",
                    crate::tr!(
                        "el kernel no arrancó: {e}{}",
                        "the kernel did not start: {e}{}",
                        if cola.is_empty() { String::new() } else { format!("\n{cola}") }
                    )
                )
            }
        }
    }

    /// Se conecta a un kernel que ya escucha en `con` (el lanzado, o el falso de las pruebas) y lo saluda.
    pub async fn conectar(con: &Conexion, proc: Option<Arc<crate::exec::Proc>>, spec: Especificacion, archivo: Option<PathBuf>) -> Result<Kernel> {
        let base = "tcp://127.0.0.1";
        let opciones = || {
            let mut o = SocketOptions::default();
            o.connect_timeout(ARRANQUE);
            o
        };
        let vivo = {
            let p = proc.clone();
            move || p.as_ref().is_none_or(|p| p.vigente())
        };
        let conectar = async {
            let mut iopub = SubSocket::with_options(opciones());
            iopub.subscribe("").await?;
            iopub.connect(&format!("{base}:{}", con.iopub)).await?;
            let mut shell = DealerSocket::with_options(opciones());
            shell.connect(&format!("{base}:{}", con.shell)).await?;
            let mut control = DealerSocket::with_options(opciones());
            control.connect(&format!("{base}:{}", con.control)).await?;
            Ok::<_, zeromq::ZmqError>((shell, control, iopub))
        };
        // la conexión compite con la muerte del proceso: un kernel que no arranca no se espera 90 s
        let (shell, control, iopub) = {
            tokio::pin!(conectar);
            loop {
                tokio::select! {
                    r = &mut conectar => break r.map_err(|e| anyhow!("ZeroMQ: {e}"))?,
                    _ = tokio::time::sleep(Duration::from_millis(200)) => {
                        if !vivo() {
                            bail!("{}", crate::i18n::t("el proceso del kernel terminó al arrancar", "the kernel process exited while starting"));
                        }
                    }
                }
            }
        };
        let mut k = Kernel {
            spec,
            proc,
            archivo,
            shell,
            control,
            iopub,
            firma: Firma::new(&con.clave),
            sesion: id_nuevo(),
            lenguaje: String::new(),
            version: String::new(),
            ejecutadas: 0,
            ultimo: Instant::now(),
        };
        k.saludar().await?;
        Ok(k)
    }

    pub fn vivo(&self) -> bool {
        self.proc.as_ref().is_none_or(|p| p.vigente())
    }

    /// El número de proceso (#N de `process`), si lo lanzó GOTV.
    pub fn proceso(&self) -> Option<u32> {
        self.proc.as_ref().map(|p| p.id)
    }

    async fn enviar(&mut self, canal: Canal, m: &Mensaje) -> Result<()> {
        let t = zmq(m.tramas(&self.firma));
        match canal {
            Canal::Control => self.control.send(t).await,
            _ => self.shell.send(t).await,
        }
        .map_err(|e| anyhow!("ZeroMQ: {e}"))
    }

    /// El siguiente mensaje de cualquiera de los tres canales (los de firma mala se descartan), o `None` al
    /// pasar `hasta` o si el proceso del kernel murió.
    async fn recibir(&mut self, hasta: Instant) -> Option<(Canal, Mensaje)> {
        loop {
            if Instant::now() >= hasta || !self.vivo() {
                return None;
            }
            let espera = (hasta - Instant::now()).min(Duration::from_millis(250));
            let Kernel { shell, control, iopub, firma, .. } = self;
            let r = tokio::select! {
                m = iopub.recv() => m.map(|m| (Canal::Iopub, m)),
                m = shell.recv() => m.map(|m| (Canal::Shell, m)),
                m = control.recv() => m.map(|m| (Canal::Control, m)),
                _ = tokio::time::sleep(espera) => continue,
            };
            match r {
                Ok((c, m)) => {
                    if let Ok(m) = Mensaje::de_tramas(m.into_vec(), firma) {
                        return Some((c, m));
                    }
                }
                // un canal caído: se sigue mirando el tope y si el proceso vive
                Err(_) => tokio::time::sleep(Duration::from_millis(50)).await,
            }
        }
    }

    /// `kernel_info_request` hasta tener respuesta Y ver su estado por iopub (la suscripción de ZeroMQ tarda en
    /// valer: lo publicado antes se pierde).
    async fn saludar(&mut self) -> Result<()> {
        let hasta = Instant::now() + ARRANQUE;
        let mut respuesta = false;
        while Instant::now() < hasta {
            let m = Mensaje::nuevo("kernel_info_request", &self.sesion, json!({}));
            let id = m.id().to_string();
            self.enviar(Canal::Shell, &m).await?;
            let corte = (Instant::now() + Duration::from_secs(if respuesta { 1 } else { 30 })).min(hasta);
            let mut visto = false;
            while let Some((c, r)) = self.recibir(corte).await {
                if r.padre() != id {
                    continue;
                }
                match c {
                    Canal::Shell if r.tipo() == "kernel_info_reply" => {
                        respuesta = true;
                        let li = &r.content["language_info"];
                        self.lenguaje = li["name"].as_str().unwrap_or("").to_string();
                        self.version = li["version"].as_str().unwrap_or("").to_string();
                    }
                    Canal::Iopub => visto = true,
                    _ => {}
                }
                if respuesta && visto {
                    return Ok(());
                }
            }
            if !self.vivo() {
                bail!("{}", crate::i18n::t("el proceso del kernel terminó al arrancar", "the kernel process exited while starting"));
            }
        }
        bail!(
            "{}",
            if respuesta {
                crate::i18n::t("el kernel responde pero no publica su estado (iopub)", "the kernel answers but does not publish its state (iopub)")
            } else {
                crate::i18n::t("el kernel no respondió a kernel_info_request (¿mensajes sin la firma que espera?)", "the kernel did not answer kernel_info_request (messages without the signature it expects?)")
            }
        )
    }

    /// Ejecuta `codigo` con tope. Pasado el tope: interrumpe; si no vuelve en unos segundos, lo mata.
    pub async fn ejecutar(&mut self, codigo: &str, tope: Duration) -> Result<Resultado> {
        let t0 = Instant::now();
        self.ultimo = t0;
        let req = Mensaje::nuevo(
            "execute_request",
            &self.sesion,
            json!({"code": codigo, "silent": false, "store_history": true, "user_expressions": {}, "allow_stdin": false, "stop_on_error": true}),
        );
        let id = req.id().to_string();
        self.enviar(Canal::Shell, &req).await?;
        let mut guarda = Guarda { proc: self.proc.clone(), activa: true };
        let mut col = Coleccion::default();
        let (mut ocioso, mut respuesta): (bool, Option<Value>) = (false, None);
        let mut hasta = t0 + tope;
        let mut interrumpida = false;
        let fin = loop {
            if ocioso && respuesta.is_some() {
                break None;
            }
            match self.recibir(hasta).await {
                Some((Canal::Iopub, m)) if m.padre() == id => ocioso |= col.mensaje(&m),
                Some((Canal::Shell, m)) if m.padre() == id && m.tipo() == "execute_reply" => {
                    respuesta = Some(m.content);
                    // tras la respuesta, lo que falte de iopub llega enseguida (el estado ocioso)
                    hasta = hasta.min(Instant::now() + Duration::from_secs(3));
                }
                Some(_) => {}
                None if !self.vivo() => break Some(Fin::Murio),
                None if respuesta.is_some() || ocioso => break None,
                None if !interrumpida => {
                    interrumpida = true;
                    self.interrumpir().await;
                    hasta = Instant::now() + GRACIA;
                }
                None => {
                    self.matar().await;
                    break Some(Fin::Matado);
                }
            }
        };
        guarda.activa = false;
        let estado = respuesta.as_ref().and_then(|r| r["status"].as_str()).unwrap_or("");
        let fin = fin.unwrap_or(if interrumpida {
            Fin::Interrumpida
        } else if estado == "ok" {
            Fin::Ok
        } else {
            Fin::Error
        });
        let cuenta = respuesta.as_ref().and_then(|r| r["execution_count"].as_u64()).or(col.cuenta);
        self.ejecutadas += 1;
        Ok(Resultado { salidas: col.salidas, cuenta, fin, segundos: t0.elapsed().as_secs_f32() })
    }

    /// Ctrl-C a la celda que corre: por mensaje si el kernel lo pide (`interrupt_mode: message`), si no la señal.
    pub async fn interrumpir(&mut self) {
        match (&self.proc, self.spec.interrumpe_por_mensaje) {
            (Some(p), false) => p.interrumpir(),
            _ => {
                let m = Mensaje::nuevo("interrupt_request", &self.sesion, json!({}));
                let _ = self.enviar(Canal::Control, &m).await;
            }
        }
    }

    /// Lo mata (SIGINT → SIGQUIT → SIGTERM → SIGKILL, como `process stop`) y borra su archivo de conexión.
    pub async fn matar(&mut self) {
        if let Some(p) = &self.proc {
            p.stop().await;
        }
        if let Some(a) = self.archivo.take() {
            let _ = std::fs::remove_file(a);
        }
    }

    /// `shutdown_request` por control; si no sale en 3 s, se mata.
    pub async fn apagar(mut self) {
        let m = Mensaje::nuevo("shutdown_request", &self.sesion, json!({"restart": false}));
        if self.enviar(Canal::Control, &m).await.is_ok() {
            if let Some(p) = &self.proc {
                let _ = p.wait_exit(Duration::from_secs(3)).await;
            } else {
                let _ = self.recibir(Instant::now() + Duration::from_secs(2)).await;
            }
        }
        self.matar().await;
    }
}

impl Drop for Kernel {
    fn drop(&mut self) {
        if let Some(a) = self.archivo.take() {
            let _ = std::fs::remove_file(a);
        }
    }
}

#[cfg(test)]
pub(crate) mod falso {
    //! Un kernel de Jupyter FALSO en Rust que responde por ZeroMQ como uno de verdad: verifica la firma (lo que
    //! llega sin ella se ignora en silencio, como en ipykernel) y entiende un lenguaje mínimo:
    //! `print(X)` → stdout · `imagen` → un PNG · `1/0` → ZeroDivisionError con traza en color · `dormir` → no
    //! vuelve hasta que lo interrumpan por mensaje · `colgado` → no vuelve nunca · lo demás → `execute_result`.
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use zeromq::{PubSocket, RouterSocket};

    /// Un PNG de 2×1 de verdad en base64 (con su `\n` final, como lo publica matplotlib).
    pub fn png() -> String {
        use base64::Engine;
        let img = image::RgbImage::from_pixel(2, 1, image::Rgb([200, 30, 30]));
        let mut b = std::io::Cursor::new(Vec::new());
        img.write_to(&mut b, image::ImageFormat::Png).unwrap();
        format!("{}\n", base64::engine::general_purpose::STANDARD.encode(b.into_inner()))
    }

    struct Estado {
        firma: Firma,
        sesion: String,
        cuenta: AtomicU64,
        interrumpir: AtomicBool,
        apagar: AtomicBool,
        iopub: tokio::sync::Mutex<PubSocket>,
    }

    impl Estado {
        async fn publicar(&self, tipo: &str, padre: &Mensaje, content: Value) {
            let mut m = Mensaje::respuesta(tipo, &self.sesion, padre, content);
            m.ids = vec![Bytes::from(format!("kernel.{tipo}"))];
            let _ = self.iopub.lock().await.send(zmq(m.tramas(&self.firma))).await;
        }
    }

    async fn atender(e: &Estado, m: &Mensaje) -> Option<Mensaje> {
        e.publicar("status", m, json!({"execution_state": "busy"})).await;
        let r = match m.tipo() {
            "kernel_info_request" => Mensaje::respuesta(
                "kernel_info_reply",
                &e.sesion,
                m,
                json!({"status": "ok", "protocol_version": "5.3", "implementation": "falso", "language_info": {"name": "python", "version": "3.99"}}),
            ),
            "execute_request" => {
                let n = e.cuenta.fetch_add(1, Ordering::SeqCst) + 1;
                let code = m.content["code"].as_str().unwrap_or("").trim().to_string();
                e.publicar("execute_input", m, json!({"code": code, "execution_count": n})).await;
                let mut status = "ok";
                if let Some(x) = code.strip_prefix("print(").and_then(|x| x.strip_suffix(')')) {
                    // dos trozos: Jupyter los junta en una salida
                    let (a, b) = x.split_at(x.len() / 2);
                    e.publicar("stream", m, json!({"name": "stdout", "text": a})).await;
                    e.publicar("stream", m, json!({"name": "stdout", "text": format!("{b}\n")})).await;
                } else if code == "imagen" {
                    e.publicar("display_data", m, json!({"data": {"image/png": png(), "text/plain": "<Figure size 2x1 with 1 Axes>"}, "metadata": {}, "transient": {}})).await;
                } else if code == "1/0" {
                    status = "error";
                    e.publicar("error", m, json!({"ename": "ZeroDivisionError", "evalue": "division by zero", "traceback": ["\u{1b}[0;31m---------------------------------------------------------------------------\u{1b}[0m", "\u{1b}[0;31mZeroDivisionError\u{1b}[0m: division by zero"]})).await;
                } else if code == "dormir" || code == "colgado" {
                    let hasta = Instant::now() + Duration::from_secs(60);
                    while Instant::now() < hasta {
                        if code == "dormir" && e.interrumpir.swap(false, Ordering::SeqCst) {
                            status = "error";
                            e.publicar("error", m, json!({"ename": "KeyboardInterrupt", "evalue": "", "traceback": ["KeyboardInterrupt: "]})).await;
                            break;
                        }
                        if e.apagar.load(Ordering::SeqCst) {
                            return None;
                        }
                        tokio::time::sleep(Duration::from_millis(50)).await;
                    }
                } else {
                    e.publicar("execute_result", m, json!({"data": {"text/plain": code}, "metadata": {}, "execution_count": n})).await;
                }
                Mensaje::respuesta("execute_reply", &e.sesion, m, json!({"status": status, "execution_count": n}))
            }
            _ => return None,
        };
        e.publicar("status", m, json!({"execution_state": "idle"})).await;
        Some(r)
    }

    /// Sirve en los puertos de `con` hasta un `shutdown_request`. Con `oyentes` (los de `Conexion::con_oyentes`)
    /// ata esos mismos sockets: ninguna otra prueba en paralelo puede ganarle un puerto.
    pub async fn servir(con: Conexion, oyentes: Option<Vec<std::net::TcpListener>>) {
        let firma = Firma::new(&con.clave);
        let mut oy: Vec<Option<std::net::TcpListener>> = oyentes.map(|v| v.into_iter().map(Some).collect()).unwrap_or_default();
        let mut tomar = |i: usize| oy.get_mut(i).and_then(Option::take);
        let (o_shell, o_iopub, o_control) = (tomar(0), tomar(1), tomar(3));
        macro_rules! atar {
            ($s:expr, $o:expr, $p:expr) => {
                match $o {
                    Some(l) => $s.bind_listener(l).await.map(|_| ()),
                    None => $s.bind(&format!("tcp://127.0.0.1:{}", $p)).await.map(|_| ()),
                }
                .unwrap()
            };
        }
        let mut iopub = PubSocket::new();
        atar!(iopub, o_iopub, con.iopub);
        let e = Arc::new(Estado { firma: firma.clone(), sesion: id_nuevo(), cuenta: AtomicU64::new(0), interrumpir: AtomicBool::new(false), apagar: AtomicBool::new(false), iopub: tokio::sync::Mutex::new(iopub) });
        let mut control = RouterSocket::new();
        atar!(control, o_control, con.control);
        let mut shell = RouterSocket::new();
        atar!(shell, o_shell, con.shell);
        // control aparte: interrumpe y apaga MIENTRAS el shell está ocupado (como ipykernel)
        let ec = e.clone();
        tokio::spawn(async move {
            while let Ok(t) = control.recv().await {
                let Ok(m) = Mensaje::de_tramas(t.into_vec(), &ec.firma) else { continue };
                let r = match m.tipo() {
                    "interrupt_request" => {
                        ec.interrumpir.store(true, Ordering::SeqCst);
                        Mensaje::respuesta("interrupt_reply", &ec.sesion, &m, json!({"status": "ok"}))
                    }
                    "shutdown_request" => {
                        ec.apagar.store(true, Ordering::SeqCst);
                        Mensaje::respuesta("shutdown_reply", &ec.sesion, &m, json!({"status": "ok", "restart": false}))
                    }
                    _ => continue,
                };
                let _ = control.send(zmq(r.tramas(&ec.firma))).await;
            }
        });
        while !e.apagar.load(Ordering::SeqCst) {
            let t = tokio::select! {
                t = shell.recv() => t,
                _ = tokio::time::sleep(Duration::from_millis(100)) => continue,
            };
            let Ok(t) = t else { break };
            // trampa 3: sin la firma buena, NADA (ni error ni respuesta)
            let Ok(m) = Mensaje::de_tramas(t.into_vec(), &e.firma) else { continue };
            if let Some(r) = atender(&e, &m).await {
                let _ = shell.send(zmq(r.tramas(&e.firma))).await;
            }
        }
    }

    /// El kernel falso como PROCESO (el propio binario de pruebas, lanzado desde un `kernel.json`): lee su archivo
    /// de conexión de `GOTV_KERNEL_FALSO` e ignora SIGINT (para probar que, si no vuelve, se le mata).
    #[test]
    #[ignore = "lo lanza la prueba del kernel por proceso, no se corre suelto"]
    fn kernel_falso_proceso() {
        let Some(p) = std::env::var_os("GOTV_KERNEL_FALSO") else { return };
        // SAFETY: ignorar SIGINT en este proceso de prueba (el kernel falso «colgado»).
        unsafe {
            libc::signal(libc::SIGINT, libc::SIG_IGN);
        }
        let con = Conexion::leer(Path::new(&p)).unwrap();
        tokio::runtime::Runtime::new().unwrap().block_on(servir(con, None));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(por_mensaje: bool) -> Especificacion {
        Especificacion {
            nombre: "falso".into(),
            mostrar: "Falso".into(),
            lenguaje: "python".into(),
            argv: vec![],
            env: Default::default(),
            interrumpe_por_mensaje: por_mensaje,
            carpeta: PathBuf::new(),
            del_proyecto: true,
        }
    }

    #[tokio::test]
    async fn kernel_falso_por_zeromq_salidas_error_imagen_e_interrupcion() {
        let (con, oyentes) = Conexion::con_oyentes().unwrap();
        tokio::spawn(falso::servir(con.clone(), Some(oyentes)));
        let mut k = Kernel::conectar(&con, None, spec(true), None).await.unwrap();
        assert_eq!((k.lenguaje.as_str(), k.version.as_str()), ("python", "3.99"));
        // texto en dos trozos → una sola salida stream, con sus líneas
        let r = k.ejecutar("print(hola mundo)", Duration::from_secs(10)).await.unwrap();
        assert_eq!((r.fin, r.cuenta), (Fin::Ok, Some(1)));
        assert_eq!(r.salidas, vec![json!({"name": "stdout", "output_type": "stream", "text": ["hola mundo\n"]})]);
        // execute_result con su número
        let r = k.ejecutar("41+1", Duration::from_secs(10)).await.unwrap();
        assert_eq!(r.salidas[0]["output_type"], "execute_result");
        assert_eq!((r.salidas[0]["execution_count"].as_u64(), r.salidas[0]["data"]["text/plain"].clone()), (Some(2), json!(["41+1"])));
        // la imagen queda en base64 como una cadena (nbformat no parte el PNG)
        let r = k.ejecutar("imagen", Duration::from_secs(10)).await.unwrap();
        assert_eq!(r.salidas[0]["data"]["image/png"], falso::png());
        // el error con su traza
        let r = k.ejecutar("1/0", Duration::from_secs(10)).await.unwrap();
        assert_eq!((r.fin, r.salidas[0]["ename"].as_str()), (Fin::Error, Some("ZeroDivisionError")));
        // pasado el tope: interrupt_request por control y la celda vuelve con KeyboardInterrupt; el kernel sigue
        let t0 = Instant::now();
        let r = k.ejecutar("dormir", Duration::from_millis(400)).await.unwrap();
        assert_eq!((r.fin, r.salidas[0]["ename"].as_str()), (Fin::Interrumpida, Some("KeyboardInterrupt")));
        assert!(t0.elapsed() < Duration::from_secs(5), "{:?}", t0.elapsed());
        let r = k.ejecutar("print(sigo vivo)", Duration::from_secs(10)).await.unwrap();
        assert_eq!(r.fin, Fin::Ok, "el estado no se perdió");
        k.apagar().await;
    }

    /// Trampa 3: sin la firma, el kernel IGNORA el mensaje en silencio (no hay error que leer: sólo silencio).
    #[tokio::test]
    async fn sin_la_firma_el_kernel_no_contesta() {
        let (con, oyentes) = Conexion::con_oyentes().unwrap();
        tokio::spawn(falso::servir(con.clone(), Some(oyentes)));
        let mut mala = con.clone();
        mala.clave = "otra-clave".into();
        let t0 = Instant::now();
        // el saludo reintenta hasta su tope; aquí basta ver que con la clave mala no hay respuesta en 3 s
        let r = tokio::time::timeout(Duration::from_secs(3), Kernel::conectar(&mala, None, spec(true), None)).await;
        assert!(r.is_err(), "con la firma mala no debió contestar");
        assert!(t0.elapsed() >= Duration::from_secs(3));
        // y con la buena, sí
        let k = Kernel::conectar(&con, None, spec(true), None).await.unwrap();
        k.apagar().await;
    }

    #[test]
    fn coleccion_limpiar_y_actualizar_display() {
        let m = |tipo: &str, c: Value| Mensaje::nuevo(tipo, "s", c);
        let mut c = Coleccion::default();
        c.mensaje(&m("display_data", json!({"data": {"text/plain": "1 %"}, "transient": {"display_id": "barra"}})));
        c.mensaje(&m("update_display_data", json!({"data": {"text/plain": "100 %"}, "transient": {"display_id": "barra"}})));
        assert_eq!(c.salidas[0]["data"]["text/plain"], json!(["100 %"]));
        c.mensaje(&m("clear_output", json!({"wait": true})));
        assert_eq!(c.salidas.len(), 1, "con wait, se limpia al llegar la siguiente");
        c.mensaje(&m("stream", json!({"name": "stderr", "text": "a\nb"})));
        assert_eq!(c.salidas, vec![json!({"name": "stderr", "output_type": "stream", "text": ["a\n", "b"]})]);
        assert!(c.mensaje(&m("status", json!({"execution_state": "idle"}))));
    }
}
