//! F18 · Móvil: el simulador de iOS y el emulador (o teléfono) de Android con UN solo modelo de
//! «dispositivo» y la herramienta `dispositivo` (diferida): listar, encender, apagar, instalar, lanzar,
//! cerrar, árbol con referencias `@d1…`, captura, tocar, deslizar, escribir, tecla, esperar, registro,
//! ajustar y borrar_datos (este último pregunta SIEMPRE).
//!
//! - **iOS:** `xcrun simctl` (Apple) para encender, instalar, lanzar, captura, registro y ajustes; leer y
//!   tocar por **idb** (`idb_companion`, Meta, MIT) hablado desde Rust por gRPC en un socket Unix privado
//!   (`idb.rs`). Siempre por identificador, jamás `booted` (puede haber varios encendidos).
//! - **Android:** el binario `adb` del SDK (nivel 2 de L1) y, para escribir texto que no es ASCII en el
//!   EMULADOR, su API gRPC (`android.rs`).
//! - gRPC y protobuf se hablan a mano (`grpc.rs`, `pb.rs`) sobre el HTTP/2 de `hyper`: sin `build.rs`, sin
//!   `protoc` y sin generar código (ver la entrega de F18).
pub mod android;
pub mod arbol;
pub mod construir;
pub mod grpc;
pub mod idb;
pub mod instalador;
pub mod ios;
pub mod pb;
pub mod xcode;

use anyhow::{anyhow, bail, Result};
use arbol::Arbol;
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Plataforma {
    Ios,
    Android,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Dispositivo {
    /// UDID del simulador, serie de `adb` (`emulator-5554`) o, apagado, el nombre del dispositivo virtual.
    pub id: String,
    pub nombre: String,
    pub plataforma: Plataforma,
    pub sistema: String,
    pub encendido: bool,
    pub fisico: bool,
}

impl Dispositivo {
    pub fn linea(&self) -> String {
        let estado = if self.encendido { crate::i18n::t("encendido", "on") } else { crate::i18n::t("apagado", "off") };
        let tipo = match (self.plataforma, self.fisico) {
            (_, true) => crate::i18n::t("teléfono real", "real phone"),
            (Plataforma::Ios, false) => crate::i18n::t("simulador", "simulator"),
            (Plataforma::Android, false) => crate::i18n::t("emulador", "emulator"),
        };
        format!("{} · {} · {tipo} · {estado} · {}", self.nombre, self.sistema, self.id)
    }
}

/// Teclas y botones comunes a las dos plataformas.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Tecla {
    Inicio,
    Atras,
    Bloquear,
    Apps,
    Intro,
    Borrar,
    Tab,
    Arriba,
    Abajo,
    Izquierda,
    Derecha,
    SubirVolumen,
    BajarVolumen,
}

impl Tecla {
    pub fn de(s: &str) -> Option<Tecla> {
        Some(match s.trim().to_lowercase().as_str() {
            "inicio" | "home" => Tecla::Inicio,
            "atras" | "atrás" | "back" => Tecla::Atras, // juez-idioma: datos (nombres de tecla que acepta la herramienta)
            "bloquear" | "lock" | "power" => Tecla::Bloquear,
            "apps" | "recientes" | "app_switch" => Tecla::Apps,
            "intro" | "enter" | "return" => Tecla::Intro,
            "borrar" | "delete" | "backspace" => Tecla::Borrar,
            "tab" => Tecla::Tab,
            "arriba" | "up" => Tecla::Arriba,
            "abajo" | "down" => Tecla::Abajo,
            "izquierda" | "left" => Tecla::Izquierda,
            "derecha" | "right" => Tecla::Derecha,
            "subir_volumen" | "volume_up" => Tecla::SubirVolumen,
            "bajar_volumen" | "volume_down" => Tecla::BajarVolumen,
            _ => return None,
        })
    }
}

/// Lo que devolvió una orden.
#[derive(Debug, Clone, Default)]
pub struct Salida {
    pub ok: bool,
    pub out: Vec<u8>,
    pub err: String,
}

impl Salida {
    pub fn texto(&self) -> String {
        String::from_utf8_lossy(&self.out).into_owned()
    }
    /// Falla con lo que dijo la orden.
    pub fn exigir(self, orden: &str) -> Result<Salida> {
        if self.ok {
            return Ok(self);
        }
        let dijo = if self.err.trim().is_empty() { self.texto() } else { self.err.clone() };
        bail!("{orden}: {}", dijo.trim().chars().take(800).collect::<String>())
    }
}

/// Quien corre las órdenes (`xcrun`, `adb`, `emulator`). En las pruebas, uno falso que anota y responde.
#[async_trait::async_trait]
pub trait Corredor: Send + Sync {
    async fn correr(&self, prog: &str, args: &[String], tope: Duration) -> Result<Salida>;
    /// Lanza sin esperar (el emulador de Android); la salida va a `registro`. Devuelve el pid.
    fn fondo(&self, prog: &str, args: &[String], registro: &Path) -> Result<u32>;
}

/// El de verdad: procesos con tope de tiempo (`kill_on_drop`).
pub struct Sistema;

#[async_trait::async_trait]
impl Corredor for Sistema {
    async fn correr(&self, prog: &str, args: &[String], tope: Duration) -> Result<Salida> {
        let mut c = crate::entorno::comando_tokio(prog);
        c.args(args).stdin(std::process::Stdio::null()).kill_on_drop(true);
        let o = tokio::time::timeout(tope, c.output())
            .await
            .map_err(|_| anyhow!("{}", crate::tr!("{prog} {} no terminó en {} s", "{prog} {} did not finish in {} s", args.first().map(String::as_str).unwrap_or(""), tope.as_secs())))?
            .map_err(|e| anyhow!("{}", crate::tr!("no pude correr {prog}: {e}", "could not run {prog}: {e}")))?;
        Ok(Salida { ok: o.status.success(), out: o.stdout, err: String::from_utf8_lossy(&o.stderr).into_owned() })
    }

    fn fondo(&self, prog: &str, args: &[String], registro: &Path) -> Result<u32> {
        if let Some(p) = registro.parent() {
            std::fs::create_dir_all(p)?;
        }
        let f = std::fs::File::create(registro)?;
        let mut h = crate::entorno::comando(prog).args(args).stdin(std::process::Stdio::null()).stdout(f.try_clone()?).stderr(f).spawn().map_err(|e| anyhow!("{}", crate::tr!("no pude lanzar {prog}: {e}", "could not launch {prog}: {e}")))?;
        let pid = h.id();
        // se recoge al terminar (sin zombis); el proceso sigue aunque el turno acabe
        std::thread::spawn(move || {
            let _ = h.wait();
        });
        Ok(pid)
    }
}

/// Lo que cada plataforma sabe hacer.
#[async_trait::async_trait]
pub trait Puente: Send + Sync {
    async fn listar(&self) -> Result<Vec<Dispositivo>>;
    /// Devuelve el id con que se le habla ya encendido (en Android, la serie `emulator-NNNN`).
    async fn encender(&self, id: &str, ventana: bool) -> Result<String>;
    async fn apagar(&self, id: &str) -> Result<()>;
    /// Devuelve el identificador de la app (bundle de iOS o paquete de Android).
    async fn instalar(&self, id: &str, app: &Path) -> Result<String>;
    async fn lanzar(&self, id: &str, app: &str) -> Result<u32>;
    async fn cerrar(&self, id: &str, app: &str) -> Result<()>;
    async fn arbol(&self, id: &str, todo: bool) -> Result<Arbol>;
    async fn captura(&self, id: &str, a: &Path) -> Result<()>;
    async fn tocar(&self, id: &str, x: i32, y: i32) -> Result<()>;
    async fn deslizar(&self, id: &str, de: (i32, i32), a: (i32, i32), ms: u32) -> Result<()>;
    /// `en` = centro del campo (se toca antes). Devuelve un aviso si algo no pasó (o vacío).
    async fn escribir(&self, id: &str, en: Option<(i32, i32)>, texto: &str) -> Result<String>;
    async fn esperar(&self, id: &str, texto: &str, tope_ms: u32) -> Result<bool>;
    async fn tecla(&self, id: &str, t: Tecla) -> Result<()>;
    async fn registro(&self, id: &str, app: Option<&str>, lineas: usize) -> Result<String>;
    async fn ajustar(&self, id: &str, args: &Value) -> Result<String>;
    async fn borrar_datos(&self, id: &str, app: Option<&str>) -> Result<String>;
    /// Tras tocar: espera a que la app se aquiete (con tope corto).
    async fn aquietar(&self, id: &str);
    /// Construye la app del proyecto para este dispositivo (F18 · M4/M7).
    async fn construir(&self, _id: &str, _root: &Path, _args: &Value) -> Result<construir::Construida> {
        bail!("{}", crate::i18n::t("esta plataforma no sabe construir", "this platform cannot build"))
    }
    /// ¿La app sigue corriendo? (la compuerta lo pregunta tras lanzarla: si no, se cerró al abrir)
    async fn vive(&self, _id: &str, _app: &str) -> Result<bool> {
        Ok(true)
    }
    /// ¿La app está instalada? `None` = no se sabe (no se afirma nada).
    async fn instalada(&self, _id: &str, _app: &str) -> Option<bool> {
        None
    }
}

/// ¿Tiene forma de UDID de simulador (8-4-4-4-12 hexadecimal)?
pub fn es_udid(id: &str) -> bool {
    let p: Vec<&str> = id.split('-').collect();
    p.len() == 5 && p.iter().zip([8, 4, 4, 4, 12]).all(|(t, n)| t.len() == n && t.chars().all(|c| c.is_ascii_hexdigit()))
}

/// ¿Es un aparato REAL? (ni un simulador de iOS ni un emulador de Android). Instalar en él pregunta siempre.
pub fn es_fisico(id: &str) -> bool {
    !id.is_empty() && !es_udid(id) && !id.starts_with("emulator-")
}

/// La clase de permiso de cada acción: `lectura` pasa; `ejecucion` es como `bash`; `pregunta` pregunta
/// SIEMPRE (también en bypass): borrar datos e instalar en un aparato real.
pub fn clase(accion: &str, args: &Value) -> &'static str {
    match accion {
        "listar" | "arbol" | "captura" | "esperar" | "registro" => "lectura",
        "borrar_datos" => "pregunta",
        "instalar" if es_fisico(args.get("dispositivo").and_then(|v| v.as_str()).unwrap_or("")) => "pregunta",
        _ => "ejecucion",
    }
}

/// ¿El proyecto es una app móvil? (trae la herramienta `dispositivo` sin buscarla)
pub fn es_proyecto_movil(root: &Path) -> bool {
    let hay = |p: &str| root.join(p).exists();
    hay("pubspec.yaml")
        || hay("app/src/main/AndroidManifest.xml")
        || hay("android/app/src/main/AndroidManifest.xml")
        || hay("ios/Runner.xcodeproj")
        || std::fs::read_dir(root).into_iter().flatten().flatten().any(|e| e.path().extension().is_some_and(|x| x == "xcodeproj" || x == "xcworkspace"))
}

/// El último árbol leído de cada dispositivo (las `@dN` valen para ÉL).
fn arboles() -> &'static Mutex<HashMap<String, Arbol>> {
    static A: OnceLock<Mutex<HashMap<String, Arbol>>> = OnceLock::new();
    A.get_or_init(Default::default)
}

fn guardar_arbol(id: &str, a: &Arbol) {
    if let Ok(mut m) = arboles().lock() {
        m.insert(id.to_string(), a.clone());
    }
}

fn arbol_guardado(id: &str) -> Option<Arbol> {
    arboles().lock().ok()?.get(id).cloned()
}

fn texto_de<'a>(args: &'a Value, k: &str) -> Option<&'a str> {
    args.get(k).and_then(|v| v.as_str()).map(str::trim).filter(|v| !v.is_empty())
}

fn numero_de(args: &Value, k: &str) -> Option<i64> {
    args.get(k).and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|f| f.round() as i64)).or_else(|| v.as_str().and_then(|s| s.trim().parse().ok())))
}

fn falta(k: &str) -> anyhow::Error {
    anyhow!("{}", crate::tr!("falta `{k}`", "missing `{k}`"))
}

/// Los dos puentes.
pub struct Puentes<'a> {
    pub ios: &'a dyn Puente,
    pub android: &'a dyn Puente,
}

impl Puentes<'_> {
    fn de(&self, p: Plataforma) -> &dyn Puente {
        match p {
            Plataforma::Ios => self.ios,
            Plataforma::Android => self.android,
        }
    }

    /// Todos los dispositivos, y una línea por plataforma que no respondió.
    async fn todos(&self) -> (Vec<Dispositivo>, Vec<String>) {
        let mut v = Vec::new();
        let mut avisos = Vec::new();
        for (p, nombre) in [(self.ios, "iOS"), (self.android, "Android")] {
            match p.listar().await {
                Ok(d) => v.extend(d),
                Err(e) => avisos.push(format!("{nombre}: {e}")),
            }
        }
        (v, avisos)
    }

    /// El dispositivo de la orden: por id o por nombre; sin él, el ÚNICO encendido.
    async fn resolver(&self, args: &Value) -> Result<Dispositivo> {
        let (todos, avisos) = self.todos().await;
        let quiere = texto_de(args, "dispositivo");
        let elegido = match quiere {
            Some(q) => todos.iter().find(|d| d.id == q).or_else(|| todos.iter().find(|d| d.nombre.eq_ignore_ascii_case(q))).cloned(),
            None => {
                let on: Vec<&Dispositivo> = todos.iter().filter(|d| d.encendido).collect();
                match on.len() {
                    1 => Some(on[0].clone()),
                    0 => bail!("{}", crate::i18n::t("no hay ningún dispositivo encendido: `listar` y luego `encender` uno", "no device is on: `listar` and then `encender` one")),
                    _ => bail!("{}", crate::tr!("hay {} encendidos: di cuál con `dispositivo`:\n{}", "{} are on: say which one with `dispositivo`:\n{}", on.len(), on.iter().map(|d| d.linea()).collect::<Vec<_>>().join("\n"))),
                }
            }
        };
        match elegido {
            Some(d) => Ok(d),
            // un id de forma conocida que no salió en la lista (p. ej. sin permiso para listar) se intenta igual
            None if quiere.is_some_and(es_udid) => Ok(Dispositivo { id: quiere.unwrap_or_default().into(), nombre: String::new(), plataforma: Plataforma::Ios, sistema: String::new(), encendido: true, fisico: false }),
            None => {
                let mut msg = crate::tr!("no encuentro el dispositivo «{}». Hay:\n{}", "device «{}» not found. Available:\n{}", quiere.unwrap_or(""), todos.iter().map(|d| d.linea()).collect::<Vec<_>>().join("\n"));
                if !avisos.is_empty() {
                    msg.push('\n');
                    msg.push_str(&avisos.join("\n"));
                }
                bail!("{msg}")
            }
        }
    }
}

/// El punto de una orden: `en: "@d3"` (del último árbol de ese dispositivo) o `x`, `y`.
fn punto(args: &Value, id: &str) -> Result<Option<(i32, i32)>> {
    if let Some(r) = texto_de(args, "en") {
        let a = arbol_guardado(id).ok_or_else(|| anyhow!("{}", crate::tr!("{r}: aún no hay árbol de este dispositivo; pide `arbol` primero", "{r}: there is no tree for this device yet; ask for `arbol` first")))?;
        let n = a.buscar(r).ok_or_else(|| anyhow!("{}", crate::tr!("{r} no está en el último árbol (las referencias cambian en cada lectura): pide `arbol` de nuevo", "{r} is not in the latest tree (references change on every read): ask for `arbol` again")))?;
        return Ok(Some(n.centro()));
    }
    match (numero_de(args, "x"), numero_de(args, "y")) {
        (Some(x), Some(y)) => Ok(Some((x as i32, y as i32))),
        _ => Ok(None),
    }
}

/// Lee el árbol, lo guarda (para las `@dN`) y lo devuelve en texto.
async fn leer_arbol(p: &dyn Puente, id: &str, todo: bool) -> Result<String> {
    let mut a = p.arbol(id, todo).await?;
    // medido 2026-11-17 (iPhone 17e real): el árbol pedido justo tras `lanzar` salió VACÍO (la app aún
    // arrancaba): si no hay nada, se espera a que la pantalla se aquiete y se lee otra vez
    if a.nodos.is_empty() {
        p.aquietar(id).await;
        a = p.arbol(id, todo).await?;
    }
    guardar_arbol(id, &a);
    Ok(a.texto())
}

/// Tras una acción: el árbol nuevo (o, si no se puede leer, por qué).
async fn despues(p: &dyn Puente, id: &str, hecho: String) -> String {
    p.aquietar(id).await;
    match leer_arbol(p, id, false).await {
        Ok(t) => format!("{hecho}\n{t}"),
        Err(e) => format!("{hecho}\n{}", crate::tr!("(no pude leer el árbol nuevo: {e})", "(could not read the new tree: {e})")),
    }
}

/// La carpeta de capturas del proyecto.
fn carpeta_capturas(root: &Path) -> PathBuf {
    root.join(".gotv").join("movil").join("capturas")
}

/// El dispositivo con que se probó la app de cada proyecto (lo usa la compuerta móvil).
fn ultimos() -> &'static Mutex<HashMap<PathBuf, Dispositivo>> {
    static U: OnceLock<Mutex<HashMap<PathBuf, Dispositivo>>> = OnceLock::new();
    U.get_or_init(Default::default)
}

fn recordar(root: &Path, d: &Dispositivo) {
    if let Ok(mut m) = ultimos().lock() {
        m.insert(root.to_path_buf(), d.clone());
    }
}

/// ¿Es un archivo que cambia la app? (lo demás —notas, documentos— no pide construir)
pub fn es_fuente_movil(ruta: &Path) -> bool {
    let ext = ruta.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    matches!(ext.as_str(), "swift" | "m" | "mm" | "h" | "c" | "cpp" | "kt" | "kts" | "java" | "xml" | "gradle" | "dart" | "storyboard" | "xib" | "plist" | "pbxproj" | "xcstrings" | "strings" | "json" | "js" | "jsx" | "ts" | "tsx")
}

/// Del registro de una app que se cerró: primero las líneas que dicen POR QUÉ (Swift, Objective-C,
/// Android) y los informes de cierre; luego la cola corta. Medido 2026-11-17: el registro de iOS de una
/// app que se cae al abrir son ~80 líneas de UIKit y la única útil (`Fatal error: Index out of range`)
/// era la penúltima.
pub fn lo_que_importa(reg: &str) -> String {
    const SENALES: &[&str] = &["Fatal error", "fatal error", "Terminating app", "*** ", "Exception:", "EXCEPTION", "SIGABRT", "SIGSEGV", "EXC_", "Caused by:", "Process: ", "💥", ".ips"];
    let mut lineas: Vec<&str> = reg.lines().filter(|l| !l.trim().is_empty()).collect();
    // el búfer de cierres de Android guarda los de antes (medido: salió el de la corrida anterior): fuera las
    // líneas de AndroidRuntime anteriores al ÚLTIMO «FATAL EXCEPTION»
    if let Some(u) = lineas.iter().rposition(|l| l.contains("FATAL EXCEPTION")) {
        let mut i = 0;
        lineas.retain(|l| {
            i += 1;
            i - 1 >= u || !l.contains("AndroidRuntime")
        });
    }
    let mut clave: Vec<&str> = Vec::new();
    for (i, l) in lineas.iter().enumerate() {
        if SENALES.iter().any(|s| l.contains(s)) && !clave.contains(l) {
            clave.push(l);
            // de una pila de Java/Kotlin, sólo el primer marco tras el «Caused by:» (ahí está el código propio);
            // las ~13 líneas de ActivityThread no dicen nada
            if l.contains("Caused by:") {
                if let Some(m) = lineas.get(i + 1).filter(|m| m.contains("\tat ") || m.trim_start().starts_with("at ")) {
                    clave.push(m);
                }
            }
        }
    }
    let marco = |l: &str| l.contains("\tat ") || l.trim_start().starts_with("at ") || l.contains("... ") && l.contains(" more");
    let cola: Vec<&str> = lineas.iter().copied().filter(|l| !clave.contains(l) && !marco(l)).rev().take(12).collect::<Vec<_>>().into_iter().rev().collect();
    let mut out = String::new();
    if !clave.is_empty() {
        out.push_str(&clave.iter().take(30).copied().collect::<Vec<_>>().join("\n"));
    }
    if !cola.is_empty() {
        out.push_str(crate::i18n::t("\n— últimas líneas —\n", "\n— last lines —\n"));
        out.push_str(&cola.join("\n"));
    }
    out.trim().to_string()
}

/// Lo que dijo la compuerta móvil.
#[derive(Debug, Clone, PartialEq)]
pub struct Compuerta {
    /// Hay que volver a trabajar (no construye o la app se cierra al abrir).
    pub fallo: bool,
    pub texto: String,
}

/// F18 · M10 · La compuerta MÓVIL: tras editar un proyecto móvil, al cerrar el turno: construir →
/// instalar → lanzar → ¿sigue viva? → árbol + captura. Sólo si en este proyecto ya se usó un
/// dispositivo (sin eso no hay a cuál mandarla) y sigue encendido.
pub async fn compuerta(root: &Path) -> Option<Compuerta> {
    let ios = ios::Ios::sistema();
    let android = android::Android::sistema();
    compuerta_con(root, &Puentes { ios: &ios, android: &android }, Duration::from_millis(2500)).await
}

pub async fn compuerta_con(root: &Path, p: &Puentes<'_>, pausa: Duration) -> Option<Compuerta> {
    let d = ultimos().lock().ok()?.get(root).cloned()?;
    let b = p.de(d.plataforma);
    // ¿sigue encendido? (si lo apagaron, no se enciende por su cuenta)
    let (todos, _) = p.todos().await;
    if !todos.iter().any(|x| x.id == d.id && x.encendido) {
        return None;
    }
    let id = d.id.as_str();
    let mal = |t: String| Some(Compuerta { fallo: true, texto: t });
    let c = match b.construir(id, root, &Value::Null).await {
        Ok(c) => c,
        Err(e) => return mal(crate::tr!("📱 compuerta móvil: no construye para «{}».\n{e}", "📱 mobile gate: it does not build for «{}».\n{e}", d.nombre)),
    };
    let ident = match b.instalar(id, &c.app).await {
        Ok(i) => i,
        Err(e) => return mal(crate::tr!("📱 compuerta móvil: construye, pero no se instala en «{}»: {e}", "📱 mobile gate: it builds, but does not install on «{}»: {e}", d.nombre)),
    };
    if let Err(e) = b.lanzar(id, &ident).await {
        // Android lo sabe ya al lanzar (el proceso no aparece): el mismo informe que si se cierra después
        let reg = lo_que_importa(&b.registro(id, Some(&ident), 200).await.unwrap_or_default());
        return mal(crate::tr!("📱 compuerta móvil: {ident} se CERRÓ al abrir (no arranca) en «{}»: {e}\n{reg}", "📱 mobile gate: {ident} CRASHED on start (does not start) on «{}»: {e}\n{reg}", d.nombre));
    }
    tokio::time::sleep(pausa).await;
    if !b.vive(id, &ident).await.unwrap_or(true) {
        let mut reg = lo_que_importa(&b.registro(id, Some(&ident), 200).await.unwrap_or_default());
        if reg.is_empty() {
            reg = crate::i18n::t("(su registro está vacío: salió sin decir por qué, p. ej. un exit() o un abort() silencioso; busca en el arranque)", "(its log is empty: it quit without saying why, e.g. a silent exit() or abort(); look at the startup code)").into();
        }
        return mal(crate::tr!("📱 compuerta móvil: {ident} se CERRÓ al abrir en «{}». Su registro:\n{reg}", "📱 mobile gate: {ident} CRASHED on start on «{}». Its log:\n{reg}", d.nombre));
    }
    b.aquietar(id).await;
    let arbol = match leer_arbol(b, id, false).await {
        Ok(t) => t,
        Err(e) => crate::tr!("(no pude leer el árbol: {e})", "(could not read the tree: {e})"),
    };
    let cap = carpeta_capturas(root);
    let _ = std::fs::create_dir_all(&cap);
    let ruta = cap.join(format!("compuerta-{}.png", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0)));
    let captura = match b.captura(id, &ruta).await {
        Ok(()) => ruta.strip_prefix(root).unwrap_or(&ruta).display().to_string(),
        Err(e) => crate::tr!("(sin captura: {e})", "(no screenshot: {e})"),
    };
    let lineas: Vec<&str> = arbol.lines().take(15).collect();
    Some(Compuerta {
        fallo: false,
        texto: crate::tr!(
            "📱 compuerta móvil: ✓ construida en {:.0} s, instalada y viva en «{}» ({ident}). Captura: {captura}\n{}",
            "📱 mobile gate: ✓ built in {:.0} s, installed and alive on «{}» ({ident}). Screenshot: {captura}\n{}",
            c.segundos,
            d.nombre,
            lineas.join("\n")
        ),
    })
}

/// La herramienta `dispositivo`.
pub async fn ejecutar(args: &Value, root: &Path) -> Result<String> {
    let ios = ios::Ios::sistema();
    let android = android::Android::sistema();
    ejecutar_con(args, root, &Puentes { ios: &ios, android: &android }).await
}

pub async fn ejecutar_con(args: &Value, root: &Path, p: &Puentes<'_>) -> Result<String> {
    let accion = texto_de(args, "action").unwrap_or("listar");
    if accion == "listar" {
        let (todos, avisos) = p.todos().await;
        let mut l: Vec<String> = todos.iter().map(Dispositivo::linea).collect();
        l.extend(avisos);
        if l.is_empty() {
            return Ok(crate::i18n::t("No hay simuladores, emuladores ni teléfonos conectados.", "No simulators, emulators or connected phones.").into());
        }
        return Ok(l.join("\n"));
    }
    if accion == "crear" {
        // F18 · M7: un dispositivo virtual de Android nuevo, sin avdmanager
        let sdk = android::sdk().ok_or_else(|| anyhow!("{}", crate::i18n::t("no hay SDK de Android: instálalo desde Configuración → Conexiones → Dispositivos", "there is no Android SDK: install it from Settings → Connections → Devices")))?;
        let nombre = texto_de(args, "nombre").unwrap_or("gotv-pixel");
        let imagen = match texto_de(args, "imagen") {
            Some(i) => i.to_string(),
            None => android::imagenes(&sdk).into_iter().next().ok_or_else(|| anyhow!("{}", crate::i18n::t("el SDK no tiene imágenes del sistema instaladas", "the SDK has no system images installed")))?,
        };
        android::crear_avd(&android::carpeta_avd(), nombre, &sdk, &imagen)?;
        return Ok(crate::tr!("✓ creado «{nombre}» ({imagen}); enciéndelo con `encender` y dispositivo={nombre}", "✓ created «{nombre}» ({imagen}); start it with `encender` and dispositivo={nombre}"));
    }
    let d = p.resolver(args).await?;
    let b = p.de(d.plataforma);
    let id = d.id.as_str();
    let app = texto_de(args, "app");
    let necesita_encendido = !matches!(accion, "encender" | "borrar_datos");
    if necesita_encendido && !d.encendido {
        bail!("{}", crate::tr!("«{}» ({id}) no está listo: `encender` primero (o, si es un teléfono, autorizar la depuración en él)", "«{}» ({id}) is not ready: `encender` first (or, on a phone, authorize debugging on it)", d.nombre));
    }
    match accion {
        "encender" => {
            if d.encendido {
                return Ok(crate::tr!("«{}» ya está encendido ({id})", "«{}» is already on ({id})", d.nombre));
            }
            let t0 = std::time::Instant::now();
            let nuevo = b.encender(id, args.get("ventana").and_then(|v| v.as_bool()).unwrap_or(false)).await?;
            Ok(crate::tr!("✓ «{}» encendido en {:.0} s ({nuevo})", "✓ «{}» on in {:.0} s ({nuevo})", d.nombre, t0.elapsed().as_secs_f32()))
        }
        "apagar" => {
            b.apagar(id).await?;
            if let Ok(mut m) = arboles().lock() {
                m.remove(id);
            }
            Ok(crate::tr!("✓ «{}» apagado", "✓ «{}» off", d.nombre))
        }
        "instalar" => {
            let a = texto_de(args, "ruta").or(app).ok_or_else(|| falta("ruta"))?;
            let ruta = if Path::new(a).is_absolute() { PathBuf::from(a) } else { root.join(a) };
            if !ruta.exists() {
                bail!("{}", crate::tr!("no existe {}", "{} does not exist", ruta.display()));
            }
            let ident = b.instalar(id, &ruta).await?;
            Ok(crate::tr!("✓ instalada {ident} en «{}»", "✓ installed {ident} on «{}»", d.nombre))
        }
        "lanzar" => {
            let a = app.ok_or_else(|| falta("app"))?;
            let pid = b.lanzar(id, a).await?;
            recordar(root, &d);
            Ok(despues(b, id, crate::tr!("✓ {a} en marcha (proceso {pid})", "✓ {a} running (process {pid})")).await)
        }
        "construir" => {
            recordar(root, &d);
            let c = b.construir(id, root, args).await?;
            let rel = c.app.strip_prefix(root).unwrap_or(&c.app).display().to_string();
            if args.get("lanzar").and_then(|v| v.as_bool()) == Some(false) {
                return Ok(crate::tr!("✓ construida en {:.0} s: {rel}", "✓ built in {:.0} s: {rel}", c.segundos));
            }
            let ident = b.instalar(id, &c.app).await?;
            let pid = b.lanzar(id, &ident).await?;
            Ok(despues(b, id, crate::tr!("✓ construida en {:.0} s, instalada y en marcha: {ident} (proceso {pid})", "✓ built in {:.0} s, installed and running: {ident} (process {pid})", c.segundos)).await)
        }
        "cerrar" => {
            let a = app.ok_or_else(|| falta("app"))?;
            b.cerrar(id, a).await?;
            Ok(crate::tr!("✓ {a} cerrada", "✓ {a} closed"))
        }
        "arbol" => leer_arbol(b, id, args.get("todo").and_then(|v| v.as_bool()).unwrap_or(false)).await,
        "captura" => {
            let c = carpeta_capturas(root);
            std::fs::create_dir_all(&c)?;
            let corto: String = id.chars().filter(char::is_ascii_alphanumeric).take(8).collect();
            let ts = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
            let ruta = c.join(format!("{corto}-{ts}.png"));
            b.captura(id, &ruta).await?;
            let rel = ruta.strip_prefix(root).unwrap_or(&ruta).display().to_string();
            Ok(crate::tr!("✓ captura en {rel} (mírala con view_image)", "✓ screenshot at {rel} (look at it with view_image)"))
        }
        "tocar" => {
            let (x, y) = punto(args, id)?.ok_or_else(|| falta("en (@dN) o x, y"))?;
            b.tocar(id, x, y).await?;
            Ok(despues(b, id, crate::tr!("✓ tocado ({x},{y})", "✓ tapped ({x},{y})")).await)
        }
        "deslizar" => {
            let de = punto(args, id)?;
            let ms = numero_de(args, "ms").unwrap_or(300).clamp(50, 5_000) as u32;
            let (de, a) = match (de, numero_de(args, "x2"), numero_de(args, "y2"), texto_de(args, "direccion")) {
                (Some(de), Some(x2), Some(y2), _) => (de, (x2 as i32, y2 as i32)),
                (Some(de), _, _, Some(dir)) => {
                    let paso = numero_de(args, "distancia").unwrap_or(400) as i32;
                    let a = match dir {
                        "arriba" | "up" => (de.0, de.1 - paso),
                        "abajo" | "down" => (de.0, de.1 + paso),
                        "izquierda" | "left" => (de.0 - paso, de.1),
                        "derecha" | "right" => (de.0 + paso, de.1),
                        _ => bail!("{}", crate::tr!("direccion «{dir}»: arriba, abajo, izquierda o derecha", "direccion «{dir}»: up, down, left or right")),
                    };
                    (de, a)
                }
                _ => bail!("{}", crate::i18n::t("deslizar: `en` o x, y como origen, y x2, y2 o `direccion`", "deslizar: `en` or x, y as origin, and x2, y2 or `direccion`")),
            };
            b.deslizar(id, de, a, ms).await?;
            Ok(despues(b, id, crate::tr!("✓ deslizado ({},{})→({},{})", "✓ swiped ({},{})→({},{})", de.0, de.1, a.0, a.1)).await)
        }
        "escribir" => {
            let texto = args.get("texto").and_then(|v| v.as_str()).ok_or_else(|| falta("texto"))?;
            let en = match punto(args, id)? {
                Some(p) => Some(p),
                None => {
                    // sin destino vale SÓLO si el árbol tiene un único campo de texto
                    let a = match arbol_guardado(id) {
                        Some(a) => a,
                        None => {
                            let a = b.arbol(id, false).await?;
                            guardar_arbol(id, &a);
                            a
                        }
                    };
                    let campos = a.campos();
                    match campos.len() {
                        1 => Some(campos[0].centro()),
                        0 => bail!("{}", crate::i18n::t("no hay campos de texto en el último árbol: di dónde con `en` (@dN) o x, y", "there are no text fields in the latest tree: say where with `en` (@dN) or x, y")),
                        _ => bail!("{}", crate::tr!("hay {} campos: di cuál con `en`:\n{}", "there are {} fields: say which one with `en`:\n{}", campos.len(), campos.iter().map(|n| n.linea()).collect::<Vec<_>>().join("\n"))),
                    }
                }
            };
            let aviso = b.escribir(id, en, texto).await?;
            let hecho = if aviso.is_empty() { crate::tr!("✓ escrito «{texto}»", "✓ typed «{texto}»") } else { aviso };
            Ok(despues(b, id, hecho).await)
        }
        "tecla" => {
            let n = texto_de(args, "tecla").ok_or_else(|| falta("tecla"))?;
            let t = Tecla::de(n).ok_or_else(|| anyhow!("{}", crate::tr!("tecla «{n}»: inicio, atras, bloquear, apps, intro, borrar, tab, arriba, abajo, izquierda, derecha, subir_volumen, bajar_volumen", "key «{n}»: inicio, atras, bloquear, apps, intro, borrar, tab, arriba, abajo, izquierda, derecha, subir_volumen, bajar_volumen")))?;
            b.tecla(id, t).await?;
            Ok(despues(b, id, crate::tr!("✓ tecla {n}", "✓ key {n}")).await)
        }
        "esperar" => {
            let t = texto_de(args, "texto").ok_or_else(|| falta("texto"))?;
            let tope = (numero_de(args, "segundos").unwrap_or(10).clamp(1, 120) * 1000) as u32;
            let t0 = std::time::Instant::now();
            let si = b.esperar(id, t, tope).await?;
            let arbol = leer_arbol(b, id, true).await.unwrap_or_default();
            Ok(if si {
                crate::tr!("✓ «{t}» apareció en {:.1} s\n{arbol}", "✓ «{t}» appeared in {:.1} s\n{arbol}", t0.elapsed().as_secs_f32())
            } else {
                crate::tr!("✗ «{t}» NO apareció en {} s\n{arbol}", "✗ «{t}» did NOT appear in {} s\n{arbol}", tope / 1000)
            })
        }
        "registro" => {
            // sin `app`, la del proyecto: el registro de TODO el sistema es ruido (medido 2026-11-18: Huihui leyó
            // 200 líneas de Bluetooth y buscó el cierre con 19 `logcat | grep` en una app que no estaba instalada)
            let propia = if app.is_none() && d.plataforma == Plataforma::Android { construir::app_android(root) } else { None };
            let app = app.or(propia.as_deref());
            if let Some(a) = app {
                if b.instalada(id, a).await == Some(false) {
                    return Ok(crate::tr!(
                        "«{a}» no está instalada en «{}»: no hay registro ni cierre suyo que leer. Para ver por qué se cierra, constrúyela e instálala (`construir`, que también la lanza) y después pide `registro`.",
                        "«{a}» is not installed on «{}»: there is no log or crash of it to read. To see why it closes, build and install it (`construir`, which also launches it) and then ask for `registro`.",
                        d.nombre
                    ));
                }
            }
            b.registro(id, app, numero_de(args, "lineas").unwrap_or(200).clamp(10, 5_000) as usize).await
        }
        "ajustar" => b.ajustar(id, args).await,
        "borrar_datos" => b.borrar_datos(id, app).await,
        otra => bail!("{}", crate::tr!("acción desconocida «{otra}»", "unknown action «{otra}»")),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::sync::Arc;

    /// Un corredor FALSO: anota cada orden («prog arg arg…») y responde la primera regla cuyo texto
    /// contenga la orden; sin regla, éxito vacío.
    #[derive(Default)]
    pub struct Falso {
        pub hechas: Mutex<Vec<String>>,
        pub reglas: Mutex<Vec<(String, Salida)>>,
        pub fondos: Mutex<Vec<String>>,
    }

    impl Falso {
        pub fn con(reglas: &[(&str, &str)]) -> Arc<Falso> {
            let f = Falso::default();
            for (k, v) in reglas {
                f.regla(k, true, v.as_bytes());
            }
            Arc::new(f)
        }
        pub fn regla(&self, k: &str, ok: bool, out: &[u8]) {
            self.reglas.lock().unwrap().insert(0, (k.to_string(), Salida { ok, out: out.to_vec(), err: if ok { String::new() } else { String::from_utf8_lossy(out).into_owned() } }));
        }
        pub fn hechas(&self) -> Vec<String> {
            self.hechas.lock().unwrap().clone()
        }
    }

    #[async_trait::async_trait]
    impl Corredor for Falso {
        async fn correr(&self, prog: &str, args: &[String], _: Duration) -> Result<Salida> {
            let orden = std::iter::once(prog.to_string()).chain(args.iter().cloned()).collect::<Vec<_>>().join(" ");
            self.hechas.lock().unwrap().push(orden.clone());
            Ok(self.reglas.lock().unwrap().iter().find(|(k, _)| orden.contains(k.as_str())).map(|(_, s)| s.clone()).unwrap_or(Salida { ok: true, ..Default::default() }))
        }
        fn fondo(&self, prog: &str, args: &[String], _: &Path) -> Result<u32> {
            self.fondos.lock().unwrap().push(std::iter::once(prog.to_string()).chain(args.iter().cloned()).collect::<Vec<_>>().join(" "));
            Ok(4242)
        }
    }

    /// El porqué del cierre va primero (iOS y Android); el ruido de UIKit, no.
    #[test]
    fn del_registro_del_cierre_lo_que_importa() {
        let mut ios: Vec<String> = (0..80).map(|i| format!("2026-11-17 12:02:{i:02} Df Juguete[1:2] [com.apple.UIKit:InterfaceStyle] Not push traits update {i}")).collect();
        ios.push("2026-11-17 12:02:56 Df Juguete[3:4] (libswiftCore.dylib) Swift/ContiguousArrayBuffer.swift:695: Fatal error: Index out of range".into());
        ios.push("💥 cierres inesperados de la última hora (léelos con read_file):\n/Users/x/Library/Logs/DiagnosticReports/Juguete-2026-11-17-120320.ips".into());
        let r = lo_que_importa(&ios.join("\n"));
        assert!(r.starts_with("2026-11-17 12:02:56 Df Juguete[3:4] (libswiftCore.dylib) Swift/ContiguousArrayBuffer.swift:695: Fatal error"), "{r}");
        assert!(r.contains("Juguete-2026-11-17-120320.ips") && r.lines().count() < 20, "{r}");
        // la pila REAL del emulador (2026-11-17, recortada): el porqué y el primer marco propio, sin los de Android
        let android = "I/ActivityManager: start\nE AndroidRuntime: FATAL EXCEPTION: main\nE AndroidRuntime: Process: space.gotv.juguete, PID: 2641\nE AndroidRuntime: java.lang.RuntimeException: Unable to start activity ComponentInfo{space.gotv.juguete/space.gotv.juguete.MainActivity}: java.lang.IllegalStateException: rota a propósito\nE AndroidRuntime: \tat android.app.ActivityThread.performLaunchActivity(ActivityThread.java:4280)\nE AndroidRuntime: \tat android.os.Looper.loop(Looper.java:338)\nE AndroidRuntime: Caused by: java.lang.IllegalStateException: rota a propósito\nE AndroidRuntime: \tat space.gotv.juguete.MainActivity.onCreate(MainActivity.java:5)\nE AndroidRuntime: \tat android.app.Activity.performCreate(Activity.java:9155)\nE AndroidRuntime: \t... 13 more\nI/otra: ruido";
        let r = lo_que_importa(android);
        assert!(r.starts_with("E AndroidRuntime: FATAL EXCEPTION: main") && r.contains("Caused by: java.lang.IllegalStateException: rota a propósito\nE AndroidRuntime: \tat space.gotv.juguete.MainActivity.onCreate(MainActivity.java:5)"), "{r}");
        assert!(!r.contains("ActivityThread") && !r.contains("performCreate") && !r.contains("13 more"), "sin los marcos de Android: {r}");
        let dos = format!("{}\nE AndroidRuntime: FATAL EXCEPTION: main\nE AndroidRuntime: Caused by: java.lang.NullPointerException: la nueva\nE AndroidRuntime: \tat space.gotv.juguete.MainActivity.onCreate(MainActivity.java:9)", android);
        let r = lo_que_importa(&dos);
        assert!(r.contains("la nueva") && !r.contains("rota a propósito"), "sólo el último cierre: {r}");
    }

    #[test]
    fn udid_fisico_y_clases_de_permiso() {
        assert!(es_udid("081558C1-E257-4271-9B75-8D62DCA52949"));
        assert!(!es_udid("emulator-5554") && !es_udid("081558C1-E257-4271-9B75"));
        assert!(!es_fisico("081558C1-E257-4271-9B75-8D62DCA52949") && !es_fisico("emulator-5554"));
        assert!(es_fisico("R58M12ABCDE") && es_fisico("192.168.1.20:5555"));
        use serde_json::json as j;
        assert_eq!(clase("arbol", &j!({})), "lectura");
        assert_eq!(clase("tocar", &j!({})), "ejecucion");
        assert_eq!(clase("borrar_datos", &j!({})), "pregunta");
        assert_eq!(clase("instalar", &j!({"dispositivo": "emulator-5554"})), "ejecucion");
        assert_eq!(clase("instalar", &j!({"dispositivo": "R58M12ABCDE"})), "pregunta");
        assert_eq!(Tecla::de("Atrás"), Some(Tecla::Atras));
        assert_eq!(Tecla::de("nada"), None);
    }

    /// Mirar pasa en todo modo; manejar pregunta (salvo bypass) y en plan no; borrar datos e instalar en
    /// un teléfono real preguntan SIEMPRE, también en bypass y con un `allow` explícito.
    #[test]
    fn permisos_de_dispositivo() {
        use crate::permissions::{Decision, Mode, Permissions};
        let root = crate::CarpetaPrueba::nueva("gotv-movil-permisos");
        use serde_json::json as j;
        let por_modo = |m: Mode, allow: Vec<String>| Permissions::new(m, &root, allow, vec![], vec![]);
        let pide = |d: Decision| matches!(d, Decision::Ask(_));
        for m in [Mode::Default, Mode::Plan, Mode::Bypass] {
            assert_eq!(por_modo(m, vec![]).check("dispositivo", &j!({"action": "arbol"})), Decision::Allow, "{m:?}");
        }
        assert!(pide(por_modo(Mode::Default, vec![]).check("dispositivo", &j!({"action": "tocar", "en": "@d1"}))));
        assert_eq!(por_modo(Mode::Bypass, vec![]).check("dispositivo", &j!({"action": "tocar"})), Decision::Allow);
        assert!(matches!(por_modo(Mode::Plan, vec![]).check("dispositivo", &j!({"action": "tocar"})), Decision::Deny(_)));
        // bypass = control total (Portador, 2026-11-17); fuera de bypass, borrar e instalar en un teléfono
        // real preguntan SIEMPRE, también con permiso guardado
        let b = por_modo(Mode::Bypass, vec![]);
        assert_eq!(b.check("dispositivo", &j!({"action": "borrar_datos", "app": "com.ejemplo"})), Decision::Allow);
        assert_eq!(b.check("dispositivo", &j!({"action": "instalar", "dispositivo": "R58M12ABCDE", "ruta": "a.apk"})), Decision::Allow);
        let ae = por_modo(Mode::AcceptEdits, vec!["dispositivo".into()]);
        assert!(pide(ae.check("dispositivo", &j!({"action": "borrar_datos", "app": "com.ejemplo"}))), "borrar: siempre");
        assert!(pide(ae.check("dispositivo", &j!({"action": "instalar", "dispositivo": "R58M12ABCDE", "ruta": "a.apk"}))), "teléfono real: siempre");
        assert_eq!(ae.check("dispositivo", &j!({"action": "instalar", "dispositivo": "emulator-5554", "ruta": "a.apk"})), Decision::Allow);
        assert_eq!(crate::permissions::kind("dispositivo"), crate::permissions::Kind::Other);
    }

    #[test]
    fn proyecto_movil_se_reconoce() {
        let d = crate::CarpetaPrueba::nueva("gotv-movil-proyecto");
        assert!(!es_proyecto_movil(&d));
        std::fs::create_dir_all(d.join("Juguete.xcodeproj")).unwrap();
        assert!(es_proyecto_movil(&d));
        let e = crate::CarpetaPrueba::nueva("gotv-movil-proyecto-android");
        std::fs::create_dir_all(e.join("app/src/main")).unwrap();
        std::fs::write(e.join("app/src/main/AndroidManifest.xml"), "<manifest/>").unwrap();
        assert!(es_proyecto_movil(&e));
    }
}
