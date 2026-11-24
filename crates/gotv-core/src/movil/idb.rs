//! F18 · M1/M3 · idb desde Rust: leer y tocar el simulador de iOS.
//!
//! `idb_companion` (Meta, licencia MIT; binario nativo, nivel 2 de L1) se lanza POR DISPOSITIVO con
//! `--grpc-domain-sock` en una carpeta privada de GOTV (0700, ruta corta por el tope de 104 bytes de un
//! socket Unix) y no en un puerto TCP: ningún otro programa del equipo puede manejar el simulador por él.
//! Se apaga al soltarse (`kill_on_drop`) y el socket se borra por su ruta exacta (L6).
//!
//! Los mensajes se escriben a mano (`pb.rs`). Sus números de campo son el contrato de `idb.proto` de
//! github.com/meta/idb (licencia MIT, Copyright (c) Meta Platforms, Inc. and affiliates): el 2026-11-17 se
//! compararon los 46 que usa GOTV, uno por uno, contra ese archivo con una prueba; el `.proto` no se guarda
//! en el repo (en GOTV sólo hay Rust). Si idb cambia su contrato, se vuelve a comparar.
use super::grpc::{self, Destino};
use super::pb::{leer, Msg};
use anyhow::{anyhow, bail, Result};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

// --- números de campo (de `idb.proto`; `campos_casan_con_el_proto` los vuelve a comparar) ---
const POINT_X: u32 = 1;
const POINT_Y: u32 = 2;
const TDR_FETCH_DIAGNOSTICS: u32 = 1;
const TDRESP_TARGET_DESCRIPTION: u32 = 1;
const TD_NAME: u32 = 2;
const TD_STATE: u32 = 4;
const TD_OS_VERSION: u32 = 6;
const AIR_FORMAT: u32 = 3;
const AIR_FILTER: u32 = 13;
const AIRESP_JSON: u32 = 1;
const AAR_MARKER: u32 = 1;
const AAR_POINT: u32 = 2;
const AAR_IGNORE_CASE: u32 = 9;
const AAR_SET_VALUE: u32 = 7;
const AAR_WAIT: u32 = 10;
const SETVALUE_VALUE: u32 = 1;
const WAIT_TIMEOUT: u32 = 1;
const WAIT_POLL_INTERVAL: u32 = 2;
const AARESP_WAIT_RESULT: u32 = 1;
const AARESP_WAIT: u32 = 2;
const WAITRESP_RESULT: u32 = 1;
const AQRESP_STATE: u32 = 2;
const STATECHANGED_STATE: u32 = 1;
const HID_PRESS: u32 = 1;
const HID_SWIPE: u32 = 2;
const HID_DELAY: u32 = 3;
const PRESS_ACTION: u32 = 1;
const PRESS_DIRECTION: u32 = 2;
const PRESSACTION_TOUCH: u32 = 1;
const PRESSACTION_BUTTON: u32 = 2;
const PRESSACTION_KEY: u32 = 3;
const TOUCH_POINT: u32 = 1;
const BUTTON_BUTTON: u32 = 1;
const KEY_KEYCODE: u32 = 1;
const SWIPE_START: u32 = 1;
const SWIPE_END: u32 = 2;
const SWIPE_DELTA: u32 = 5;
const SWIPE_DURATION: u32 = 6;
const DELAY_DURATION: u32 = 1;
const SCREENSHOT_IMAGE_DATA: u32 = 1;
// valores de enumeraciones
const FILTER_INTERACTABLE: u64 = 1;
const FORMAT_LEGACY: u64 = 0;
const DOWN: u64 = 0;
const UP: u64 = 1;
const WAIT_FOUND: u64 = 1;
const QUIET: u64 = 3;

/// Botones físicos del simulador (`HIDButtonType`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Boton {
    Inicio = 1,
    Bloquear = 2,
    Lateral = 3,
    Siri = 4,
    SubirVolumen = 6,
    BajarVolumen = 7,
}

const RUTA: &str = "/idb.CompanionService/";
const TOPE: Duration = Duration::from_secs(20);

fn punto(x: f64, y: f64) -> Msg {
    Msg::new().doble(POINT_X, x).doble(POINT_Y, y)
}

fn pulsar(accion: Msg, dir: u64) -> Vec<u8> {
    Msg::new().msg(HID_PRESS, Msg::new().msg(PRESS_ACTION, accion).entero(PRESS_DIRECTION, dir)).into_bytes()
}

fn toque(x: f64, y: f64) -> Msg {
    Msg::new().msg(PRESSACTION_TOUCH, Msg::new().msg(TOUCH_POINT, punto(x, y)))
}

fn tecla_msg(codigo: u64) -> Msg {
    Msg::new().msg(PRESSACTION_KEY, Msg::new().entero(KEY_KEYCODE, codigo))
}

/// El código HID (página 0x07 del USB) de un carácter ASCII y si lleva mayúsculas. Fuera de ASCII: `None`
/// (por teclas sólo pasa ASCII: medido en la ficha).
pub fn tecla_hid(c: char) -> Option<(u64, bool)> {
    Some(match c {
        'a'..='z' => (4 + (c as u64 - 'a' as u64), false),
        'A'..='Z' => (4 + (c as u64 - 'A' as u64), true),
        '1'..='9' => (30 + (c as u64 - '1' as u64), false),
        '0' => (39, false),
        '\n' => (40, false),
        '\t' => (43, false),
        ' ' => (44, false),
        '-' => (45, false),
        '=' => (46, false),
        '[' => (47, false),
        ']' => (48, false),
        '\\' => (49, false),
        ';' => (51, false),
        '\'' => (52, false),
        '`' => (53, false),
        ',' => (54, false),
        '.' => (55, false),
        '/' => (56, false),
        '!' => (30, true),
        '@' => (31, true),
        '#' => (32, true),
        '$' => (33, true),
        '%' => (34, true),
        '^' => (35, true),
        '&' => (36, true),
        '*' => (37, true),
        '(' => (38, true),
        ')' => (39, true),
        '_' => (45, true),
        '+' => (46, true),
        '{' => (47, true),
        '}' => (48, true),
        '|' => (49, true),
        ':' => (51, true),
        '"' => (52, true),
        '~' => (53, true),
        '<' => (54, true),
        '>' => (55, true),
        '?' => (56, true),
        _ => return None,
    })
}

const SHIFT: u64 = 225;

/// Un cliente de idb sobre un servicio ya escuchando.
#[derive(Clone, Debug)]
pub struct Idb(pub Destino);

impl Idb {
    async fn una(&self, metodo: &str, m: Vec<u8>, tope: Duration) -> Result<super::pb::Leido> {
        leer(&grpc::unario(&self.0, &format!("{RUTA}{metodo}"), m, None, tope).await?)
    }

    /// `describe`: nombre, estado y versión del sistema (también sirve para saber si el servicio responde).
    pub async fn describir(&self) -> Result<String> {
        let r = self.una("describe", Msg::new().booleano(TDR_FETCH_DIAGNOSTICS, false).into_bytes(), Duration::from_secs(5)).await?;
        let d = r.msg(TDRESP_TARGET_DESCRIPTION).unwrap_or_default();
        Ok(format!("{} · {} · {}", d.texto(TD_NAME).unwrap_or_default(), d.texto(TD_OS_VERSION).unwrap_or_default(), d.texto(TD_STATE).unwrap_or_default()))
    }

    /// El árbol en JSON (formato LEGACY). Sin `todo`, el servicio filtra lo accionable.
    pub async fn arbol_json(&self, todo: bool) -> Result<String> {
        let mut m = Msg::new().entero(AIR_FORMAT, FORMAT_LEGACY);
        if !todo {
            m = m.entero(AIR_FILTER, FILTER_INTERACTABLE);
        }
        Ok(self.una("accessibility_info", m.into_bytes(), TOPE).await?.texto(AIRESP_JSON).unwrap_or_default())
    }

    async fn hid(&self, eventos: Vec<Vec<u8>>) -> Result<()> {
        grpc::llamar(&self.0, &format!("{RUTA}hid"), &eventos, None, TOPE).await?;
        Ok(())
    }

    pub async fn tocar(&self, x: f64, y: f64) -> Result<()> {
        self.hid(vec![pulsar(toque(x, y), DOWN), pulsar(toque(x, y), UP)]).await
    }

    pub async fn deslizar(&self, de: (f64, f64), a: (f64, f64), ms: u32) -> Result<()> {
        let s = Msg::new().msg(SWIPE_START, punto(de.0, de.1)).msg(SWIPE_END, punto(a.0, a.1)).doble(SWIPE_DELTA, 10.0).doble(SWIPE_DURATION, ms as f64 / 1000.0);
        self.hid(vec![Msg::new().msg(HID_SWIPE, s).into_bytes()]).await
    }

    pub async fn boton(&self, b: Boton) -> Result<()> {
        let accion = || Msg::new().msg(PRESSACTION_BUTTON, Msg::new().entero(BUTTON_BUTTON, b as u64));
        self.hid(vec![pulsar(accion(), DOWN), pulsar(accion(), UP)]).await
    }

    /// Una tecla suelta por su código HID (Intro = 40, borrar = 42, flechas 79–82…).
    pub async fn tecla(&self, codigo: u64) -> Result<()> {
        self.hid(vec![pulsar(tecla_msg(codigo), DOWN), pulsar(tecla_msg(codigo), UP)]).await
    }

    /// Texto POR TECLAS (sólo ASCII). Devuelve los caracteres que NO pasaron.
    pub async fn teclear(&self, texto: &str) -> Result<String> {
        let mut ev = Vec::new();
        let mut fuera = String::new();
        for c in texto.chars() {
            let Some((k, may)) = tecla_hid(c) else {
                fuera.push(c);
                continue;
            };
            if may {
                ev.push(pulsar(tecla_msg(SHIFT), DOWN));
            }
            ev.push(pulsar(tecla_msg(k), DOWN));
            ev.push(pulsar(tecla_msg(k), UP));
            if may {
                ev.push(pulsar(tecla_msg(SHIFT), UP));
            }
            ev.push(Msg::new().msg(HID_DELAY, Msg::new().doble(DELAY_DURATION, 0.01)).into_bytes());
        }
        if !ev.is_empty() {
            self.hid(ev).await?;
        }
        Ok(fuera)
    }

    /// Escribir DEFINIENDO el valor del campo que está en (x, y): acepta cualquier texto (medido en la
    /// ficha: «mañana ñandú 日本» en 0,85 s, y la app se entera).
    pub async fn poner_valor(&self, x: f64, y: f64, texto: &str) -> Result<()> {
        let m = Msg::new().msg(AAR_POINT, punto(x, y)).msg(AAR_SET_VALUE, Msg::new().texto(SETVALUE_VALUE, texto));
        self.una("accessibility_action", m.into_bytes(), TOPE).await?;
        Ok(())
    }

    /// Espera a que aparezca un elemento con ese texto (sin mayúsculas), con tope. `true` = apareció.
    pub async fn esperar(&self, texto: &str, tope_ms: u32) -> Result<bool> {
        let w = Msg::new().doble(WAIT_TIMEOUT, tope_ms as f64 / 1000.0).doble(WAIT_POLL_INTERVAL, 0.25);
        let m = Msg::new().texto(AAR_MARKER, texto).booleano(AAR_IGNORE_CASE, true).msg(AAR_WAIT, w);
        let r = self.una("accessibility_action", m.into_bytes(), Duration::from_millis(tope_ms as u64) + Duration::from_secs(5)).await?;
        let res = r.msg(AARESP_WAIT).and_then(|w| w.entero(WAITRESP_RESULT)).or(r.entero(AARESP_WAIT_RESULT)).unwrap_or(0);
        Ok(res == WAIT_FOUND)
    }

    /// Espera a que la app de delante se AQUIETE (sin animaciones ni trabajo), con tope. `true` = quieta;
    /// `false` = se acabó el tope (no es error: se sigue).
    pub async fn quieta(&self, tope: Duration) -> bool {
        let quieto = |m: &[u8]| leer(m).ok().and_then(|r| r.msg(AQRESP_STATE)).and_then(|s| s.entero(STATECHANGED_STATE)) == Some(QUIET);
        match grpc::llamar_hasta(&self.0, &format!("{RUTA}accessibility_quiescence"), &[vec![]], None, tope, quieto).await {
            Ok(v) => v.last().is_some_and(|m| quieto(m)),
            Err(_) => false,
        }
    }

    pub async fn captura(&self) -> Result<Vec<u8>> {
        let r = self.una("screenshot", vec![], TOPE).await?;
        r.bytes(SCREENSHOT_IMAGE_DATA).map(<[u8]>::to_vec).filter(|b| !b.is_empty()).ok_or_else(|| anyhow!("{}", crate::i18n::t("idb devolvió una captura vacía", "idb returned an empty screenshot")))
    }
}

// --- el servicio por dispositivo ---

/// Dónde está `idb_companion`: `GOTV_IDB_COMPANION`, lo que bajó GOTV (`~/Library/Application
/// Support/gotv/idb/<versión>/bin/`), o el `PATH` (Homebrew).
pub fn companion() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("GOTV_IDB_COMPANION").map(PathBuf::from).filter(|p| p.is_file()) {
        return Some(p);
    }
    let casa = PathBuf::from(std::env::var("HOME").unwrap_or_default());
    let base = casa.join("Library/Application Support/gotv/idb");
    let mut bajados: Vec<PathBuf> = std::fs::read_dir(&base).into_iter().flatten().flatten().map(|e| e.path().join("bin/idb_companion")).filter(|p| p.is_file()).collect();
    bajados.sort();
    if let Some(p) = bajados.pop() {
        return Some(p);
    }
    let path = std::env::var("PATH").unwrap_or_default();
    let hallado = path.split(':').chain(["/opt/homebrew/bin", "/usr/local/bin"]).map(|d| Path::new(d).join("idb_companion")).find(|p| p.is_file());
    hallado
}

/// Un `idb_companion` vivo para un dispositivo. Al soltarse, el proceso muere y su socket se borra.
pub struct ServicioIdb {
    pub sock: PathBuf,
    hijo: std::sync::Mutex<tokio::process::Child>,
}

impl Drop for ServicioIdb {
    fn drop(&mut self) {
        if let Ok(mut h) = self.hijo.lock() {
            let _ = h.start_kill();
        }
        let _ = std::fs::remove_file(&self.sock);
    }
}

/// La carpeta privada de los sockets (0700) y el socket de un dispositivo (ruta corta: tope de 104 bytes).
pub fn ruta_socket(udid: &str) -> Result<PathBuf> {
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
    let d = std::env::temp_dir().join("gotv-idb");
    std::fs::DirBuilder::new().recursive(true).mode(0o700).create(&d)?;
    std::fs::set_permissions(&d, std::fs::Permissions::from_mode(0o700))?;
    let corto: String = udid.chars().filter(char::is_ascii_alphanumeric).take(8).collect();
    let p = d.join(format!("{}-{corto}.sock", std::process::id()));
    if p.as_os_str().len() > 100 {
        bail!("{}", crate::tr!("la ruta del socket de idb es demasiado larga ({}): un socket Unix admite 104 bytes", "the idb socket path is too long ({}): a Unix socket allows 104 bytes", p.display()));
    }
    Ok(p)
}

impl ServicioIdb {
    pub fn destino(&self) -> Destino {
        Destino::Unix(self.sock.clone())
    }

    fn vivo(&self) -> bool {
        self.hijo.lock().map(|mut h| matches!(h.try_wait(), Ok(None))).unwrap_or(false)
    }

    /// Lanza `programa args` (con `env`), espera a que `describe` responda por `sock` y lo devuelve.
    pub async fn lanzar_con(programa: &Path, args: &[String], env: &[(String, String)], sock: PathBuf, tope: Duration) -> Result<Self> {
        let _ = std::fs::remove_file(&sock);
        let registro = sock.with_extension("log");
        let f = std::fs::File::create(&registro)?;
        let mut c = tokio::process::Command::new(programa);
        c.args(args).envs(env.iter().cloned()).stdin(std::process::Stdio::null()).stdout(f.try_clone()?).stderr(f).kill_on_drop(true);
        let hijo = c.spawn().map_err(|e| anyhow!("{}", crate::tr!("no pude lanzar {}: {e}", "could not launch {}: {e}", programa.display())))?;
        let s = ServicioIdb { sock, hijo: std::sync::Mutex::new(hijo) };
        let t0 = std::time::Instant::now();
        loop {
            if s.sock.exists() && Idb(s.destino()).describir().await.is_ok() {
                let _ = std::fs::remove_file(&registro);
                return Ok(s);
            }
            if !s.vivo() {
                let cola: String = std::fs::read_to_string(&registro).unwrap_or_default().lines().rev().take(8).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join("\n");
                bail!("{}", crate::tr!("idb_companion terminó al arrancar:\n{cola}", "idb_companion exited while starting:\n{cola}"));
            }
            if t0.elapsed() > tope {
                bail!("{}", crate::tr!("idb_companion no respondió en {} s (registro: {})", "idb_companion did not answer in {} s (log: {})", tope.as_secs(), registro.display()));
            }
            tokio::time::sleep(Duration::from_millis(150)).await;
        }
    }

    pub async fn lanzar(udid: &str) -> Result<Self> {
        let bin = companion().ok_or_else(|| {
            anyhow!(
                "{}",
                crate::i18n::t(
                    "falta idb_companion (Meta, licencia MIT), con el que GOTV lee y toca el simulador. Baja `idb-companion.macos-arm64.tar.gz` de las versiones de github.com/meta/idb y pon la ruta de `bin/idb_companion` en GOTV_IDB_COMPANION. Sin él siguen valiendo listar, encender, captura, instalar, lanzar, registro y ajustar.",
                    "idb_companion (Meta, MIT license) is missing; GOTV reads and taps the simulator with it. Download `idb-companion.macos-arm64.tar.gz` from the github.com/meta/idb releases and put the path of `bin/idb_companion` in GOTV_IDB_COMPANION. Without it, listar, encender, captura, instalar, lanzar, registro and ajustar still work."
                )
            )
        })?;
        let sock = ruta_socket(udid)?;
        let args = vec!["--udid".to_string(), udid.to_string(), "--grpc-domain-sock".to_string(), sock.display().to_string()];
        Self::lanzar_con(&bin, &args, &[], sock, Duration::from_secs(30)).await
    }
}

type Servicios = tokio::sync::Mutex<HashMap<String, Arc<ServicioIdb>>>;

fn servicios() -> &'static Servicios {
    static S: OnceLock<Servicios> = OnceLock::new();
    S.get_or_init(Default::default)
}

/// El idb de un dispositivo: el que ya corre o uno nuevo.
pub async fn de(udid: &str) -> Result<Idb> {
    let mut m = servicios().lock().await;
    if let Some(s) = m.get(udid).filter(|s| s.vivo()) {
        return Ok(Idb(s.destino()));
    }
    let s = Arc::new(ServicioIdb::lanzar(udid).await?);
    let d = s.destino();
    m.insert(udid.to_string(), s);
    Ok(Idb(d))
}

/// Suelta el idb de un dispositivo (al apagarlo).
pub async fn soltar(udid: &str) {
    servicios().lock().await.remove(udid);
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::movil::grpc::falso;
    use crate::movil::pb::Leido;
    use std::sync::Mutex;

    /// El número del campo `campo` declarado DIRECTAMENTE en `message <mensaje> {` del `.proto`.
    /// (Los `oneof` son transparentes: sus campos son del mensaje. Si el nombre se repite —`enum State`
    /// en varios mensajes— vale el primero que declare ese campo.)
    pub fn numero(proto: &str, mensaje: &str, campo: &str) -> Option<u32> {
        let cab = format!("message {mensaje} {{");
        proto.match_indices(&cab).find_map(|(ini, _)| numero_en(&proto[ini + cab.len()..], campo))
    }

    fn numero_en(cuerpo: &str, campo: &str) -> Option<u32> {
        let mut pila = vec![false]; // true = `oneof` (transparente)
        let efectiva = |p: &Vec<bool>| p.iter().filter(|t| !**t).count();
        let mut linea = String::new();
        for c in cuerpo.chars() {
            match c {
                '{' => {
                    let t = linea.split("//").next().unwrap_or("").trim_start().starts_with("oneof");
                    pila.push(t);
                    linea.clear();
                }
                '}' => {
                    pila.pop();
                    linea.clear();
                    if pila.is_empty() {
                        return None;
                    }
                }
                '\n' => {
                    if efectiva(&pila) == 1 {
                        let l = linea.split("//").next().unwrap_or("").to_string();
                        let t: Vec<&str> = l.split_whitespace().collect();
                        if let Some(i) = t.iter().position(|w| *w == "=") {
                            if i >= 1 && t[i - 1] == campo {
                                return t.get(i + 1)?.trim_end_matches(';').parse().ok();
                            }
                        }
                    }
                    linea.clear();
                }
                _ => linea.push(c),
            }
        }
        None
    }

    /// Vuelve a comparar los números con un `idb.proto` (de github.com/meta/idb) que se le indica con
    /// `GOTV_IDB_PROTO=<ruta>`: el archivo no vive en el repo.
    #[test]
    #[ignore]
    fn campos_casan_con_el_proto() {
        let Some(ruta) = std::env::var_os("GOTV_IDB_PROTO") else { return };
        let p = &std::fs::read_to_string(ruta).unwrap();
        let tabla: &[(&str, &str, u32)] = &[
            ("Point", "x", POINT_X),
            ("Point", "y", POINT_Y),
            ("TargetDescriptionRequest", "fetch_diagnostics", TDR_FETCH_DIAGNOSTICS),
            ("TargetDescriptionResponse", "target_description", TDRESP_TARGET_DESCRIPTION),
            ("TargetDescription", "name", TD_NAME),
            ("TargetDescription", "state", TD_STATE),
            ("TargetDescription", "os_version", TD_OS_VERSION),
            ("AccessibilityInfoRequest", "format", AIR_FORMAT),
            ("AccessibilityInfoRequest", "filter", AIR_FILTER),
            ("AccessibilityInfoResponse", "json", AIRESP_JSON),
            ("AccessibilityActionRequest", "marker", AAR_MARKER),
            ("AccessibilityActionRequest", "point", AAR_POINT),
            ("AccessibilityActionRequest", "ignore_case", AAR_IGNORE_CASE),
            ("AccessibilityActionRequest", "set_value", AAR_SET_VALUE),
            ("AccessibilityActionRequest", "wait", AAR_WAIT),
            ("SetValue", "value", SETVALUE_VALUE),
            ("Wait", "timeout", WAIT_TIMEOUT),
            ("Wait", "poll_interval", WAIT_POLL_INTERVAL),
            ("AccessibilityActionResponse", "wait_result", AARESP_WAIT_RESULT),
            ("AccessibilityActionResponse", "wait", AARESP_WAIT),
            ("WaitResponse", "result", WAITRESP_RESULT),
            ("AccessibilityQuiescenceResponse", "state", AQRESP_STATE),
            ("StateChanged", "state", STATECHANGED_STATE),
            ("HIDEvent", "press", HID_PRESS),
            ("HIDEvent", "swipe", HID_SWIPE),
            ("HIDEvent", "delay", HID_DELAY),
            ("HIDPress", "action", PRESS_ACTION),
            ("HIDPress", "direction", PRESS_DIRECTION),
            ("HIDPressAction", "touch", PRESSACTION_TOUCH),
            ("HIDPressAction", "button", PRESSACTION_BUTTON),
            ("HIDPressAction", "key", PRESSACTION_KEY),
            ("HIDTouch", "point", TOUCH_POINT),
            ("HIDButton", "button", BUTTON_BUTTON),
            ("HIDKey", "keycode", KEY_KEYCODE),
            ("HIDSwipe", "start", SWIPE_START),
            ("HIDSwipe", "end", SWIPE_END),
            ("HIDSwipe", "delta", SWIPE_DELTA),
            ("HIDSwipe", "duration", SWIPE_DURATION),
            ("HIDDelay", "duration", DELAY_DURATION),
            ("ScreenshotResponse", "image_data", SCREENSHOT_IMAGE_DATA),
            // valores de enumeraciones
            ("Filter", "FILTER_INTERACTABLE", FILTER_INTERACTABLE as u32),
            ("Format", "LEGACY", FORMAT_LEGACY as u32),
            ("HIDDirection", "DOWN", DOWN as u32),
            ("HIDDirection", "UP", UP as u32),
            ("WaitResult", "FOUND", WAIT_FOUND as u32),
            ("State", "QUIET", QUIET as u32),
        ];
        for (m, c, n) in tabla {
            let n_proto = numero(p, m, c).or_else(|| {
                // las enumeraciones se declaran con `enum`, no con `message`
                numero(&p.replace("enum ", "message "), m, c)
            });
            assert_eq!(n_proto, Some(*n), "{m}.{c}");
        }
        for (b, n) in [(Boton::Inicio, "HOME"), (Boton::Bloquear, "LOCK"), (Boton::Lateral, "SIDE_BUTTON"), (Boton::Siri, "SIRI"), (Boton::SubirVolumen, "VOLUME_UP"), (Boton::BajarVolumen, "VOLUME_DOWN")] {
            assert_eq!(numero(&p.replace("enum ", "message "), "HIDButtonType", n), Some(b as u32), "{n}");
        }
        for r in ["describe", "accessibility_info", "accessibility_action", "accessibility_quiescence", "hid", "screenshot"] {
            assert!(p.contains(&format!("rpc {r}(")), "{r}");
        }
        assert!(p.contains("package idb;") && p.contains("service CompanionService"));
    }

    #[test]
    fn teclas_hid_ascii_y_lo_demas_fuera() {
        assert_eq!(tecla_hid('a'), Some((4, false)));
        assert_eq!(tecla_hid('Z'), Some((29, true)));
        assert_eq!(tecla_hid('0'), Some((39, false)));
        assert_eq!(tecla_hid('?'), Some((56, true)));
        assert_eq!(tecla_hid('ñ'), None);
        assert_eq!(tecla_hid('日'), None);
    }

    /// Lo que el companion falso recibió: (método, mensajes).
    pub type Visto = Arc<Mutex<Vec<(String, Vec<Vec<u8>>)>>>;

    /// Un idb FALSO: responde `describe`, da el árbol de Ajustes (con teclado tras un `set_value`), acepta
    /// `hid` y `screenshot`, y `wait` encuentra lo que el árbol tiene.
    pub fn manejador(visto: Visto) -> falso::Manejador {
        let escrito: Arc<Mutex<Option<String>>> = Default::default();
        Arc::new(move |ruta, msgs, _| {
            let metodo = ruta.trim_start_matches(RUTA).to_string();
            visto.lock().unwrap().push((metodo.clone(), msgs.clone()));
            let m = msgs.first().map(|b| leer(b).unwrap()).unwrap_or_default();
            match metodo.as_str() {
                "describe" => Ok(vec![Msg::new().msg(TDRESP_TARGET_DESCRIPTION, Msg::new().texto(TD_NAME, "iPhone 18 Pro").texto(TD_STATE, "Booted").texto(TD_OS_VERSION, "iOS 27.0")).into_bytes()]),
                "accessibility_info" => {
                    let mut j = crate::movil::arbol::tests::ajustes_ios(escrito.lock().unwrap().is_some());
                    if let Some(t) = escrito.lock().unwrap().as_ref() {
                        j = j.replace("ñandú", t);
                    }
                    Ok(vec![Msg::new().texto(AIRESP_JSON, &j).into_bytes()])
                }
                "accessibility_action" => {
                    if let Some(v) = m.msg(AAR_SET_VALUE) {
                        *escrito.lock().unwrap() = v.texto(SETVALUE_VALUE);
                        return Ok(vec![vec![]]);
                    }
                    let marca = m.texto(AAR_MARKER).unwrap_or_default().to_lowercase();
                    let hay = crate::movil::arbol::tests::ajustes_ios(false).to_lowercase().contains(&marca) || escrito.lock().unwrap().as_deref().is_some_and(|e| e.to_lowercase().contains(&marca));
                    Ok(vec![Msg::new().msg(AARESP_WAIT, Msg::new().entero(WAITRESP_RESULT, if hay { WAIT_FOUND } else { 2 })).into_bytes()])
                }
                "accessibility_quiescence" => Ok(vec![Msg::new().msg(AQRESP_STATE, Msg::new().entero(STATECHANGED_STATE, 1)).into_bytes(), Msg::new().msg(AQRESP_STATE, Msg::new().entero(STATECHANGED_STATE, QUIET)).into_bytes()]),
                "hid" => Ok(vec![vec![]]),
                "screenshot" => Ok(vec![Msg::new().bytes(SCREENSHOT_IMAGE_DATA, b"\x89PNG\r\n\x1a\nfalso").into_bytes()]),
                _ => Err((12, format!("{metodo}: no implementado"))),
            }
        })
    }

    /// Los eventos HID de una llamada, como (tipo, dirección, x, y, código).
    pub fn eventos(msgs: &[Vec<u8>]) -> Vec<String> {
        msgs.iter()
            .map(|b| {
                let e = leer(b).unwrap();
                if let Some(p) = e.msg(HID_PRESS) {
                    let a = p.msg(PRESS_ACTION).unwrap_or_default();
                    let dir = if p.entero(PRESS_DIRECTION) == Some(UP) { "arriba" } else { "abajo" };
                    if let Some(t) = a.msg(PRESSACTION_TOUCH) {
                        let pt = t.msg(TOUCH_POINT).unwrap_or_default();
                        return format!("toque {dir} {},{}", pt.doble(POINT_X).unwrap_or(-1.0), pt.doble(POINT_Y).unwrap_or(-1.0));
                    }
                    if let Some(k) = a.msg(PRESSACTION_KEY) {
                        return format!("tecla {dir} {}", k.entero(KEY_KEYCODE).unwrap_or(0));
                    }
                    if let Some(k) = a.msg(PRESSACTION_BUTTON) {
                        return format!("boton {dir} {}", k.entero(BUTTON_BUTTON).unwrap_or(0));
                    }
                }
                if let Some(s) = e.msg(HID_SWIPE) {
                    let (a, b) = (s.msg(SWIPE_START).unwrap_or_default(), s.msg(SWIPE_END).unwrap_or_default());
                    return format!("deslizar {},{}→{},{} {}s", a.doble(POINT_X).unwrap_or(-1.0), a.doble(POINT_Y).unwrap_or(-1.0), b.doble(POINT_X).unwrap_or(-1.0), b.doble(POINT_Y).unwrap_or(-1.0), s.doble(SWIPE_DURATION).unwrap_or(-1.0));
                }
                if e.tiene(HID_DELAY) {
                    return "pausa".into();
                }
                "?".into()
            })
            .collect()
    }

    pub fn set_value_de(m: &[u8]) -> Option<(f64, f64, String)> {
        let l: Leido = leer(m).ok()?;
        let p = l.msg(AAR_POINT)?;
        Some((p.doble(POINT_X)?, p.doble(POINT_Y)?, l.msg(AAR_SET_VALUE)?.texto(SETVALUE_VALUE)?))
    }

    /// El cliente contra el idb falso: cada llamada lleva lo que el contrato dice.
    #[tokio::test]
    async fn cliente_idb_contra_servidor_falso() {
        let d = crate::CarpetaPrueba::nueva("gotv-idb-cliente");
        let sock = d.join("i.sock");
        let visto: Visto = Default::default();
        tokio::spawn(falso::servir_unix(tokio::net::UnixListener::bind(&sock).unwrap(), manejador(visto.clone())));
        let i = Idb(Destino::Unix(sock));
        assert_eq!(i.describir().await.unwrap(), "iPhone 18 Pro · iOS 27.0 · Booted");
        assert!(i.arbol_json(false).await.unwrap().contains("Modo Avión"));
        i.tocar(201.0, 316.0).await.unwrap();
        i.deslizar((200.0, 700.0), (200.0, 200.0), 300).await.unwrap();
        i.boton(Boton::Inicio).await.unwrap();
        assert_eq!(i.teclear("Hé!").await.unwrap(), "é");
        i.poner_valor(201.0, 168.0, "mañana ñandú 日本").await.unwrap();
        assert!(i.esperar("MODO AVIÓN", 1000).await.unwrap());
        assert!(!i.esperar("no existe", 1000).await.unwrap());
        assert!(i.quieta(Duration::from_secs(2)).await);
        assert!(i.captura().await.unwrap().starts_with(b"\x89PNG"));
        let v = visto.lock().unwrap().clone();
        let metodos: Vec<&str> = v.iter().map(|(m, _)| m.as_str()).collect();
        assert_eq!(metodos, vec!["describe", "accessibility_info", "hid", "hid", "hid", "hid", "accessibility_action", "accessibility_action", "accessibility_action", "accessibility_quiescence", "screenshot"]);
        // el árbol pide lo accionable y en formato LEGACY
        let air = leer(&v[1].1[0]).unwrap();
        assert_eq!((air.entero(AIR_FILTER), air.entero(AIR_FORMAT)), (Some(FILTER_INTERACTABLE), Some(FORMAT_LEGACY)));
        assert_eq!(eventos(&v[2].1), vec!["toque abajo 201,316", "toque arriba 201,316"]);
        assert_eq!(eventos(&v[3].1), vec!["deslizar 200,700→200,200 0.3s"]);
        assert_eq!(eventos(&v[4].1), vec!["boton abajo 1", "boton arriba 1"]);
        // «H» lleva mayúsculas; «é» no pasa por teclas; «!» es mayúsculas + 1
        assert_eq!(eventos(&v[5].1), vec!["tecla abajo 225", "tecla abajo 11", "tecla arriba 11", "tecla arriba 225", "pausa", "tecla abajo 225", "tecla abajo 30", "tecla arriba 30", "tecla arriba 225", "pausa"]);
        assert_eq!(set_value_de(&v[6].1[0]), Some((201.0, 168.0, "mañana ñandú 日本".to_string())));
    }

    /// `ServicioIdb` lanza un companion de verdad (el binario de pruebas haciendo de companion falso),
    /// espera a que `describe` responda, y al soltarlo el proceso muere y el socket desaparece.
    #[tokio::test]
    async fn el_servicio_arranca_espera_y_muere_al_soltarse() {
        let sock = ruta_socket("FALSO-0000-PRUEBA").unwrap();
        assert!(sock.as_os_str().len() <= 100, "{}", sock.display());
        let carpeta = sock.parent().unwrap().to_path_buf();
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&carpeta).unwrap().permissions().mode() & 0o777, 0o700);
        let exe = std::env::current_exe().unwrap();
        let args: Vec<String> = ["--exact", "movil::idb::tests::companion_falso", "--ignored", "-q", "--test-threads=1"].iter().map(|s| s.to_string()).collect();
        let env = vec![("GOTV_IDB_FALSO".to_string(), sock.display().to_string())];
        let s = ServicioIdb::lanzar_con(&exe, &args, &env, sock.clone(), Duration::from_secs(30)).await.unwrap();
        assert!(s.vivo());
        assert!(Idb(s.destino()).describir().await.unwrap().contains("iPhone 18 Pro"));
        drop(s);
        assert!(!sock.exists());
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(Idb(Destino::Unix(sock.clone())).describir().await.is_err());
        // un companion que se cae al arrancar: el error trae lo que dijo
        let e = ServicioIdb::lanzar_con(Path::new("/bin/sh"), &["-c".into(), "echo sin simulador >&2; exit 3".into()], &[], sock.clone(), Duration::from_secs(10)).await.err().unwrap();
        assert!(e.to_string().contains("sin simulador"), "{e}");
    }

    /// No es una prueba: el companion falso (lo lanza la prueba de arriba con `GOTV_IDB_FALSO=<socket>`).
    #[tokio::test]
    #[ignore]
    async fn companion_falso() {
        if let Some(s) = std::env::var_os("GOTV_IDB_FALSO") {
            let l = tokio::net::UnixListener::bind(PathBuf::from(s)).unwrap();
            falso::servir_unix(l, manejador(Default::default())).await;
        }
    }
}
