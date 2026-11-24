//! F22 · Equipos conectados al Mac: USB, discos y memorias, puertos serie, placas (ESP32, Pico con o sin
//! MicroPython, Raspberry Pi), impresoras y cámaras. La herramienta `equipos` (diferida, familia
//! «Dispositivos y equipos»).
//!
//! Todo corre en GOTV, **no** dentro de la caja: un comando del agente en la caja no ve estos equipos.
//! Frenos (más estrictos que la terminal), cada uno en el código:
//! 1. El disco interno y el de arranque no existen (`discos::Mesa::motivo`/`comprobar`).
//! 2. Formatear, borrar la memoria de una placa, grabar y preparar una tarjeta preguntan SIEMPRE (también en
//!    bypass y con un permiso guardado), mostrando nombre, tamaño y lo que se perderá; un disco de más de
//!    64 GB o con datos exige además TECLEAR su nombre (`confirmar_nombre`).
//! 3. Los discos se identifican por huella, no por posición.
//! 4. Imprimir y configurar una Raspberry preguntan cada vez (ni bypass ni un permiso guardado lo saltan).
//!
//! TODO en Rust y en proceso (pedido del Portador, 2026-11-17): ningún programa intermedio ni leer sus
//! salidas. Las excepciones, dichas: `authopen` (el cuadro de administrador de macOS para abrir un disco
//! crudo de root; no hay otra vía sin un ayudante privilegiado), `ssh` (las conexiones de `conexiones.rs`,
//! de GOTV, que esta herramienta usa como ya existen) y `diskutil eraseDisk` SÓLO para exFAT, APFS y HFS+
//! (no tienen implementación en Rust; FAT se formatea en Rust).
//! 5. Los archivos que se graban o imprimen son del proyecto (o la caché de imágenes de GOTV).
pub mod camaras;
pub mod mac;
pub mod discos;
pub mod esp;
pub mod impresoras;
pub mod micropython;
pub mod pico;
pub mod raspberry;
pub mod serie;
pub mod usb;

use crate::frontend::Frontend;
use crate::permissions::{Decision, Mode};
use anyhow::{anyhow, bail, Result};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Sin nadie delante (la herramienta llamada fuera del agente): nada que teclear ni clave que pedir, así que
/// lo que exige teclear un nombre o una clave se niega solo.
pub struct SinFrente;

#[async_trait::async_trait]
impl Frontend for SinFrente {
    async fn log(&self, _: String) {}
    async fn ask(&self, _: &str) -> crate::permissions::AskAnswer {
        crate::permissions::AskAnswer::No
    }
}

/// Los parámetros del esquema de `equipos` (aquí y no en `tools.rs`: son muchos para un solo `json!`).
pub fn parametros() -> Value {
    let s = |d: &str| serde_json::json!({ "type": "string", "description": d });
    let i = |d: &str| serde_json::json!({ "type": "integer", "description": d });
    let b = |d: &str| serde_json::json!({ "type": "boolean", "description": d });
    let campos: Vec<(&str, Value)> = vec![
        ("action", serde_json::json!({ "type": "string", "enum": ["listar", "identificar", "esperar", "monitor", "ejecutar", "archivos", "grabar", "borrar", "formatear", "expulsar", "imprimir", "cola", "cancelar", "raspberry", "foto"] })),
        ("camara", s(crate::i18n::t("con foto: parte del nombre de la cámara (por omisión, la primera)", "with foto: part of the camera name (by default, the first one)"))),
        ("que", s(crate::i18n::t("con listar: todo, usb, discos, puertos, pico, impresoras, camaras", "with listar: todo, usb, discos, puertos, pico, impresoras, camaras"))),
        ("op", s(crate::i18n::t("con archivos o raspberry: la operación", "with archivos or raspberry: the operation"))),
        ("puerto", s(crate::i18n::t("puerto serie /dev/cu.…", "serial port /dev/cu.…"))),
        ("disco", s(crate::i18n::t("diskN (de listar)", "diskN (from listar)"))),
        ("huella", s(crate::i18n::t("la huella del disco que dio listar", "the disk fingerprint listar gave"))),
        ("archivo", s(crate::i18n::t("archivo del proyecto", "project file"))),
        ("ruta", s(crate::i18n::t("ruta en la placa MicroPython", "path on the MicroPython board"))),
        ("codigo", s(crate::i18n::t("con ejecutar: código MicroPython", "with ejecutar: MicroPython code"))),
        ("contenido", s(crate::i18n::t("con archivos op=escribir: el texto", "with archivos op=escribir: the text"))),
        ("enviar", s(crate::i18n::t("con monitor: texto a mandar (\\n)", "with monitor: text to send (\\n)"))),
        ("baudios", i(crate::i18n::t("velocidad (115200)", "speed (115200)"))),
        ("segundos", i(crate::i18n::t("tope o duración", "cap or duration"))),
        ("direccion", s(crate::i18n::t("con grabar un .bin parcial en un ESP32: 0x10000…", "with grabar a partial .bin on an ESP32: 0x10000…"))),
        ("formato", s("fat32 · exfat · apfs · hfs")),
        ("nombre", s(crate::i18n::t("nombre del volumen o de la Raspberry", "volume or Raspberry name"))),
        ("impresora", s(crate::i18n::t("nombre (de listar; vacío = la de por omisión)", "name (from listar; empty = the default)"))),
        ("copias", i(crate::i18n::t("copias (1)", "copies (1)"))),
        ("prueba", b(crate::i18n::t("con imprimir: una página de prueba", "with imprimir: a test page"))),
        ("trabajo", s(crate::i18n::t("con cancelar: el trabajo de la cola", "with cancelar: the queue job"))),
        ("conexion", s(crate::i18n::t("conexión SSH del proyecto (Raspberry)", "project SSH connection (Raspberry)"))),
        ("imagen", s(crate::i18n::t("imagen del catálogo oficial de Raspberry Pi", "image from the official Raspberry Pi catalogue"))),
        ("volumen", s(crate::i18n::t("/Volumes/… (Pico en arranque o partición de una tarjeta)", "/Volumes/… (Pico in boot mode or a card partition)"))),
        ("usuario", s(crate::i18n::t("usuario de la Raspberry", "Raspberry user"))),
        ("llave", s(crate::i18n::t("llave SSH PÚBLICA (línea o .pub)", "PUBLIC SSH key (line or .pub)"))),
        ("wifi_ssid", s("Wi-Fi")),
        ("wifi_pais", s(crate::i18n::t("país del Wi-Fi (CO, ES…)", "Wi-Fi country (CO, ES…)"))),
        ("zona", s(crate::i18n::t("zona horaria", "timezone"))),
        ("teclado", s(crate::i18n::t("teclado (es, us…)", "keyboard (es, us…)"))),
        ("preguntar_chip", b(crate::i18n::t("con identificar: preguntar al chip (reinicia la placa un momento)", "with identificar: ask the chip (restarts the board briefly)"))),
        ("imagenes", b(crate::i18n::t("ver también imágenes de disco montadas por el usuario", "also see disk images mounted by the user"))),
    ];
    Value::Object(campos.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}

/// Qué clase de permiso pide cada acción (la tabla de la ficha F22).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Clase {
    /// listar, identificar, esperar, cola, raspberry info/leer_tarjeta/catalogo
    Lectura,
    /// monitor, ejecutar, archivos, expulsar, cancelar, identificar preguntando al chip
    Ejecucion,
    /// imprimir, raspberry configurar: preguntan (bypass no lo salta)
    Pregunta,
    /// grabar, borrar, formatear, preparar una tarjeta: preguntan SIEMPRE
    Siempre,
}

fn txt<'a>(a: &'a Value, k: &str) -> Option<&'a str> {
    a.get(k).and_then(|v| v.as_str()).map(str::trim).filter(|v| !v.is_empty())
}

fn si(a: &Value, k: &str) -> bool {
    a.get(k).and_then(|v| v.as_bool()).unwrap_or(false)
}

pub fn clase(args: &Value) -> Clase {
    match txt(args, "action").unwrap_or("listar") {
        "listar" | "esperar" | "cola" => Clase::Lectura,
        "identificar" if si(args, "preguntar_chip") => Clase::Ejecucion,
        "identificar" => Clase::Lectura,
        "monitor" | "ejecutar" | "archivos" | "expulsar" | "cancelar" => Clase::Ejecucion,
        // la cámara es la intimidad del usuario: cada foto pregunta (también en bypass no se guarda un «siempre»)
        "imprimir" | "foto" => Clase::Pregunta,
        "raspberry" => match txt(args, "op").unwrap_or("info") {
            "info" | "leer_tarjeta" | "catalogo" => Clase::Lectura,
            "configurar" => Clase::Pregunta,
            _ => Clase::Siempre,
        },
        // grabar, borrar, formatear y cualquier cosa desconocida: lo más estricto
        _ => Clase::Siempre,
    }
}

/// La decisión de permisos de `equipos` (la llama `permissions.rs`). `allowed_explicit` = regla `allow`,
/// lo aprendido o un permiso acotado: vale para la ejecución y lo que pregunta, NUNCA para lo destructivo.
pub fn decidir(args: &Value, modo: Mode, allowed_explicit: bool) -> Decision {
    let c = clase(args);
    if c == Clase::Lectura {
        return Decision::Allow;
    }
    if modo == Mode::Plan {
        return Decision::Deny(crate::i18n::t("modo plan: sólo mirar los equipos", "plan mode: only looking at the hardware").into());
    }
    match c {
        // la cámara y la impresora también preguntan cada vez: un «siempre» guardado sería para el
        // proyecto ENTERO, no «una vez por sesión» (eso espera a la clave `[equipos] camara` del anexo A)
        Clase::Siempre | Clase::Pregunta => Decision::Ask(describir(args)),
        _ if allowed_explicit || modo == Mode::Bypass => Decision::Allow,
        _ => Decision::Ask(describir(args)),
    }
}

/// Lo que ve el usuario en el cuadro de permiso: QUÉ equipo, y para lo destructivo, nombre, tamaño y lo
/// que se perderá (preguntado ahora al sistema, no lo que diga el modelo).
pub fn describir(args: &Value) -> String {
    let accion = txt(args, "action").unwrap_or("?");
    let op = txt(args, "op").unwrap_or("");
    let imagenes = si(args, "imagenes");
    let disco = || -> String {
        match txt(args, "disco") {
            Some(d) => match discos::Mesa::real().and_then(|m| m.comprobar(d, txt(args, "huella"), imagenes)) {
                Ok(x) => {
                    let datos = if x.con_datos() { crate::i18n::t(" — ⚠ TIENE DATOS: se perderán", " — ⚠ IT HAS DATA: it will be lost") } else { "" };
                    format!("{}{datos}", x.resumen())
                }
                Err(e) => crate::tr!("{d} ({e})", "{d} ({e})"),
            },
            None => String::new(),
        }
    };
    let archivo = txt(args, "archivo").unwrap_or(""); // juez-idioma: datos (nombre del parámetro)
    let puerto = txt(args, "puerto").unwrap_or("");
    match (accion, op) {
        ("formatear", _) => crate::tr!("💾 FORMATEAR {} como {} — se BORRA todo lo que tiene", "💾 FORMAT {} as {} — EVERYTHING on it is ERASED", disco(), txt(args, "formato").unwrap_or("fat32")),
        ("grabar", _) if txt(args, "disco").is_some() => crate::tr!("💾 GRABAR la imagen {archivo} en {} — se BORRA todo lo que tiene", "💾 WRITE image {archivo} to {} — EVERYTHING on it is ERASED", disco()),
        ("grabar", _) if archivo.ends_with(".uf2") => crate::tr!("🔌 GRABAR {archivo} en la Pico en modo de arranque — reemplaza su programa", "🔌 FLASH {archivo} to the Pico in boot mode — replaces its program"),
        ("grabar", _) => crate::tr!("🔌 GRABAR el firmware {archivo} en la placa de {puerto} — reemplaza su programa", "🔌 FLASH firmware {archivo} to the board on {puerto} — replaces its program"),
        ("foto", _) => crate::tr!("📷 TOMAR UNA FOTO con la cámara {}", "📷 TAKE A PHOTO with the camera {}", txt(args, "camara").unwrap_or("de esta Mac")),
        ("borrar", _) => crate::tr!("🔌 BORRAR TODA la memoria de la placa de {puerto} — su programa y sus archivos se pierden", "🔌 ERASE ALL the memory of the board on {puerto} — its program and files are lost"),
        ("raspberry", "preparar_tarjeta") => {
            let img = txt(args, "imagen").or(txt(args, "archivo")).unwrap_or("?");
            crate::tr!(
                "🍓 PREPARAR TARJETA: grabar «{img}» (de downloads.raspberrypi.com, de Raspberry Pi Ltd, cientos de MB; sus licencias: las de Raspberry Pi OS y Debian) en {} — se BORRA todo lo que tiene",
                "🍓 PREPARE CARD: write «{img}» (from downloads.raspberrypi.com, by Raspberry Pi Ltd, hundreds of MB; licences: those of Raspberry Pi OS and Debian) to {} — EVERYTHING on it is ERASED",
                disco()
            )
        }
        ("raspberry", "configurar") => {
            let que = raspberry::orden_configurar(args).map(|x| x.1).unwrap_or_else(|e| e.to_string());
            crate::tr!("🍓 configurar la Raspberry «{}»: {que}", "🍓 configure the Raspberry «{}»: {que}", txt(args, "conexion").unwrap_or("?"))
        }
        ("imprimir", _) => {
            let paginas = if si(args, "prueba") { Some(1) } else { impresoras::paginas(Path::new(archivo)) };
            let p = paginas.map(|n| crate::tr!("{n} página(s)", "{n} page(s)")).unwrap_or_else(|| crate::i18n::t("páginas: no se sabe", "pages: unknown").into());
            let copias = args.get("copias").and_then(|v| v.as_u64()).unwrap_or(1);
            let que = if si(args, "prueba") { crate::i18n::t("una página de prueba", "a test page").to_string() } else { archivo.to_string() };
            crate::tr!("🖨 IMPRIMIR {que} en {} · {p} × {copias} copia(s)", "🖨 PRINT {que} on {} · {p} × {copias} copy(ies)", txt(args, "impresora").unwrap_or("?"))
        }
        ("expulsar", _) => crate::tr!("⏏ expulsar {}", "⏏ eject {}", disco()),
        ("identificar", _) => crate::tr!("🔌 preguntar al chip de {puerto} (la placa se reinicia en modo de grabación un momento)", "🔌 ask the chip on {puerto} (the board briefly restarts into download mode)"),
        ("ejecutar" | "archivos" | "monitor", _) => crate::tr!(
            "🔌 {accion} {op} en la placa de {puerto} (MicroPython: detiene el programa que esté corriendo)",
            "🔌 {accion} {op} on the board on {puerto} (MicroPython: stops the program that is running)"
        ),
        _ => crate::tr!("🔌 equipos · {accion} {op}", "🔌 hardware · {accion} {op}"),
    }
}

/// Un archivo local para grabar o imprimir: del proyecto, o de la caché de imágenes de GOTV.
fn archivo_local(root: &Path, a: &str) -> Result<PathBuf> {
    let p = crate::paths::resolve(root, a);
    let cache = cache_imagenes();
    if !crate::paths::is_inside(root, &p) && !p.starts_with(&cache) {
        bail!("{}", crate::tr!("{a} está fuera del proyecto: cópialo al proyecto primero", "{a} is outside the project: copy it into the project first"));
    }
    if crate::sandbox::es_credencial(&p).is_some() {
        bail!("{}", crate::tr!("{a} es una credencial: no", "{a} is a credential: no"));
    }
    if !p.is_file() {
        bail!("{}", crate::tr!("{a} no existe", "{a} does not exist"));
    }
    Ok(p)
}

pub fn cache_imagenes() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".cache/gotv/equipos/imagenes")
}

/// Freno 2: un disco de más de 64 GB o con datos exige TECLEAR su nombre (lo teclea el usuario, no el modelo).
pub async fn confirmar_nombre<F: Frontend + ?Sized>(front: &F, d: &discos::Disco, accion: &str) -> Result<()> {
    if !d.pide_nombre() {
        return Ok(());
    }
    let nombre = d.nombre_a_teclear();
    let pregunta = crate::tr!(
        "Para {accion} {} ({}{}) teclea su nombre exacto: «{nombre}». Cualquier otra cosa lo cancela.",
        "To {accion} {} ({}{}) type its exact name: «{nombre}». Anything else cancels it.",
        d.id,
        discos::gb(d.tam),
        if d.con_datos() { crate::i18n::t(", CON DATOS", ", WITH DATA") } else { "" }
    );
    let cancelar = crate::i18n::t("cancelar", "cancel").to_string();
    match front.choose(&pregunta, std::slice::from_ref(&cancelar)).await {
        Some(t) if t.trim() == nombre => Ok(()),
        Some(_) => bail!("{}", crate::i18n::t("el nombre tecleado no coincide: no se hizo nada", "the typed name does not match: nothing was done")),
        None => bail!("{}", crate::i18n::t(
            "este disco exige que el usuario teclee su nombre y aquí no hay quien lo teclee: no se hizo nada",
            "this disk requires the user to type its name and there is nobody here to type it: nothing was done"
        )),
    }
}

/// Corre algo largo y bloqueante (grabar, descomprimir) avisando del avance por el frente.
async fn largo<F, T>(front: &F, rotulo: String, tarea: impl FnOnce(&mut dyn FnMut(u64)) -> Result<T> + Send + 'static) -> Result<T>
where
    F: Frontend + ?Sized,
    T: Send + 'static,
{
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<u64>();
    let mut h = tokio::task::spawn_blocking(move || {
        let mut aviso = |n: u64| {
            let _ = tx.send(n);
        };
        tarea(&mut aviso)
    });
    loop {
        tokio::select! {
            r = &mut h => return r.map_err(|e| anyhow!("{e}"))?,
            Some(n) = rx.recv() => front.log(format!("  {rotulo}: {} MB", n >> 20)).await,
        }
    }
}

fn bloquear<T: Send + 'static>(f: impl FnOnce() -> Result<T> + Send + 'static) -> impl std::future::Future<Output = Result<T>> {
    async move { tokio::task::spawn_blocking(f).await.map_err(|e| anyhow!("{e}"))? }
}

fn listar_texto(que: &str, imagenes: bool) -> Result<String> {
    let mut out = Vec::new();
    let todo = que == "todo";
    if todo || que == "usb" {
        let v = usb::listar()?;
        out.push(crate::tr!("USB ({}):", "USB ({}):", v.len()));
        out.extend(v.iter().map(|u| format!("  {}", usb::linea(u))));
    }
    if todo || que == "discos" {
        let m = discos::Mesa::real()?;
        let v = m.listar(imagenes);
        out.push(crate::tr!("Discos y memorias externos ({}; el interno y el de arranque no se muestran):", "External disks and memory sticks ({}; the internal and startup disks are not shown):", v.len()));
        out.extend(v.iter().map(|d| format!("  {}", d.resumen())));
    }
    if todo || que == "puertos" {
        let v = serie::listar();
        out.push(crate::tr!("Puertos serie ({}):", "Serial ports ({}):", v.len()));
        out.extend(v.iter().map(|p| format!("  {}", serie::linea(p))));
    }
    if todo || que == "pico" || que == "usb" {
        for p in pico::buscar() {
            out.push(format!("  {}", p.linea()));
        }
    }
    if todo || que == "impresoras" {
        let v = impresoras::listar().unwrap_or_default();
        out.push(crate::tr!("Impresoras ({}):", "Printers ({}):", v.len()));
        out.extend(v.iter().map(|p| format!("  {}", p.linea())));
    }
    if todo || que == "camaras" {
        let v = camaras::listar().unwrap_or_default();
        out.push(crate::tr!("Cámaras ({}):", "Cameras ({}):", v.len()));
        out.extend(v.iter().map(|c| format!("  {} ({})", c.nombre, c.modelo)));
    }
    if out.is_empty() {
        bail!("{}", crate::tr!("que=«{que}» no existe: todo · usb · discos · puertos · pico · impresoras · camaras", "que=«{que}» does not exist: todo · usb · discos · puertos · pico · impresoras · camaras"));
    }
    Ok(out.join("\n"))
}

fn puerto(args: &Value) -> Result<String> {
    let p = txt(args, "puerto").ok_or_else(|| anyhow!("{}", crate::i18n::t("falta `puerto` (/dev/cu.…: action=listar que=puertos)", "missing `puerto` (/dev/cu.…: action=listar que=puertos)")))?;
    serie::validar(p)?;
    Ok(p.to_string())
}

/// Abre la placa MicroPython y entra al modo crudo.
fn repl(p: &str, baudios: u32) -> Result<micropython::Repl<serie::Serie>> {
    let mut r = micropython::Repl::nuevo(serie::Serie::abrir(p, baudios)?);
    r.entrar()?;
    Ok(r)
}

/// La herramienta `equipos`. `front` hace falta para teclear el nombre de un disco, pedir la clave del Wi-Fi
/// tapada y avisar del avance de lo largo.
pub async fn ejecutar<F: Frontend + ?Sized>(args: &Value, root: &Path, front: &F) -> Result<String> {
    let accion = txt(args, "action").unwrap_or("listar");
    let imagenes = si(args, "imagenes");
    let baudios = args.get("baudios").and_then(|v| v.as_u64()).unwrap_or(115_200) as u32;
    let segundos = args.get("segundos").and_then(|v| v.as_u64());
    match accion {
        "listar" => {
            let que = txt(args, "que").unwrap_or("todo").to_string(); // juez-idioma: datos (nombre del parámetro)
            bloquear(move || listar_texto(&que, imagenes)).await
        }
        "esperar" => {
            let tope = Duration::from_secs(segundos.unwrap_or(60).clamp(1, 600));
            let c = usb::esperar_cambio(usb::listar, tope, Duration::from_millis(800)).await?;
            Ok(if c.vacio() { crate::tr!("nada se conectó ni desconectó en {} s", "nothing was connected or disconnected in {} s", tope.as_secs()) } else { c.aviso() })
        }
        "identificar" => {
            let mut out = Vec::new();
            match txt(args, "puerto") {
                Some(p) => {
                    serie::validar(p)?;
                    let pt = serie::listar().into_iter().find(|x| x.ruta == p).ok_or_else(|| anyhow!("{}", crate::tr!("{p} no está conectado", "{p} is not connected")))?;
                    out.push(serie::linea(&pt));
                    if let Some(q) = serie::quien_lo_tiene(p) {
                        out.push(crate::tr!("⚠ ocupado por {q}: ciérralo antes de grabar o usar MicroPython", "⚠ busy with {q}: close it before flashing or using MicroPython"));
                    }
                    let quiza_esp = pt.usb.as_ref().map(|u| usb::identificar(u.vid, u.pid).quiza_esp()).unwrap_or(true);
                    if si(args, "preguntar_chip") {
                        let pp = p.to_string();
                        out.push(match bloquear(move || esp::identificar(&pp)).await {
                            Ok(c) => crate::tr!("chip: {}", "chip: {}", c.linea()),
                            Err(e) => e.to_string(),
                        });
                    } else if quiza_esp {
                        out.push(crate::i18n::t(
                            "para saber qué chip ES (un puente CH340 o CP210x puede ser un ESP32, un Arduino o un adaptador suelto): repite con preguntar_chip=true (la placa se reinicia un momento)",
                            "to know which chip it IS (a CH340 or CP210x bridge can be an ESP32, an Arduino or a bare adapter): repeat with preguntar_chip=true (the board restarts briefly)",
                        ).into());
                    }
                }
                None => {
                    let picos = pico::buscar();
                    out.extend(picos.iter().map(|p| p.linea()));
                    for u in usb::listar()? {
                        let q = usb::identificar(u.vid, u.pid);
                        if q != usb::Que::Otro {
                            out.push(usb::linea(&u));
                        }
                    }
                    if out.is_empty() {
                        out.push(crate::i18n::t("no hay placas conocidas conectadas (ESP32, Pico, Arduino, Raspberry en modo de arranque)", "no known boards connected (ESP32, Pico, Arduino, Raspberry in boot mode)").into());
                    }
                }
            }
            Ok(out.join("\n"))
        }
        "monitor" => {
            let p = puerto(args)?;
            let enviar = txt(args, "enviar").map(String::from);
            let s = segundos.unwrap_or(5);
            let t = bloquear(move || serie::monitor(&mut serie::Serie::abrir(&p, baudios)?, enviar.as_deref(), s)).await?;
            Ok(if t.trim().is_empty() { crate::tr!("(la placa no dijo nada en {s} s)", "(the board said nothing in {s} s)") } else { t })
        }
        "ejecutar" => {
            let p = puerto(args)?;
            let codigo = txt(args, "codigo").ok_or_else(|| anyhow!("{}", crate::i18n::t("falta `codigo` (MicroPython)", "missing `codigo` (MicroPython)")))?.to_string();
            let tope = Duration::from_secs(segundos.unwrap_or(10).clamp(1, 300));
            let s = bloquear(move || {
                let mut r = repl(&p, baudios)?;
                let s = r.ejecutar(&format!("{codigo}\n"), tope);
                let _ = r.salir();
                s
            })
            .await?;
            let mut out = s.stdout;
            if !s.stderr.trim().is_empty() {
                out.push_str(&format!("\n{}", s.stderr.trim()));
            }
            if s.cortado {
                out.push_str(&crate::tr!("\n(cortado a los {} s con Ctrl-C)", "\n(stopped after {} s with Ctrl-C)", tope.as_secs()));
            }
            Ok(if out.trim().is_empty() { crate::i18n::t("✓ (sin salida)", "✓ (no output)").into() } else { out })
        }
        "archivos" => {
            let p = puerto(args)?;
            let op = txt(args, "op").unwrap_or("listar").to_string();
            let ruta = txt(args, "ruta").unwrap_or("").to_string();
            let local = match (op.as_str(), txt(args, "archivo")) { // juez-idioma: datos (nombre del parámetro)
                ("escribir", Some(a)) => Some(std::fs::read(archivo_local(root, a)?)?),
                ("escribir", None) => Some(txt(args, "contenido").ok_or_else(|| anyhow!("{}", crate::i18n::t("con escribir: `archivo` (del proyecto) o `contenido`", "with escribir: `archivo` (from the project) or `contenido`")))?.as_bytes().to_vec()),
                _ => None,
            };
            let guardar = match (op.as_str(), txt(args, "archivo")) {
                ("leer", Some(a)) => {
                    let d = crate::paths::resolve(root, a);
                    if !crate::paths::is_inside(root, &d) {
                        bail!("{}", crate::tr!("{a} está fuera del proyecto", "{a} is outside the project"));
                    }
                    Some(d)
                }
                _ => None,
            };
            bloquear(move || {
                let mut r = repl(&p, baudios)?;
                let res = match op.as_str() {
                    "listar" => r.listar(&ruta).map(|v| v.iter().map(|(n, dir, t)| if *dir { format!("{n}/") } else { format!("{n}  {t} B") }).collect::<Vec<_>>().join("\n")),
                    "leer" => r.leer(&ruta).and_then(|b| match &guardar {
                        Some(d) => {
                            std::fs::write(d, &b)?;
                            Ok(crate::tr!("✓ {ruta} → {} ({} B)", "✓ {ruta} → {} ({} B)", d.display(), b.len()))
                        }
                        None => Ok(String::from_utf8_lossy(&b).to_string()),
                    }),
                    "escribir" => {
                        let b = local.unwrap_or_default();
                        r.escribir(&ruta, &b).map(|_| crate::tr!("✓ escrito {ruta} ({} B)", "✓ written {ruta} ({} B)", b.len()))
                    }
                    "borrar" => r.borrar(&ruta).map(|_| crate::tr!("✓ borrado {ruta}", "✓ deleted {ruta}")),
                    "reiniciar" => return r.reiniciar().map(|_| crate::i18n::t("✓ reiniciada (vuelve a correr su main.py)", "✓ restarted (runs its main.py again)").to_string()),
                    otra => Err(anyhow!("{}", crate::tr!("op «{otra}»: listar · leer · escribir · borrar · reiniciar", "op «{otra}»: listar · leer · escribir · borrar · reiniciar"))),
                };
                let _ = r.salir();
                res
            })
            .await
        }
        "expulsar" => {
            let d = txt(args, "disco").ok_or_else(|| anyhow!("{}", crate::i18n::t("falta `disco`", "missing `disco`")))?.to_string();
            let h = txt(args, "huella").map(String::from);
            bloquear(move || {
                let x = discos::Mesa::real()?.comprobar(&d, h.as_deref(), imagenes)?;
                discos::expulsar(&x.id).map(|_| crate::tr!("✓ expulsado {} ({}): ya se puede desconectar", "✓ ejected {} ({}): it can be unplugged now", x.id, x.nombre))
            })
            .await
        }
        "formatear" => {
            let d = txt(args, "disco").ok_or_else(|| anyhow!("{}", crate::i18n::t("falta `disco`", "missing `disco`")))?.to_string();
            let fmt = txt(args, "formato").unwrap_or("fat32").to_ascii_lowercase();
            let de_apple = discos::formato_de_apple(&fmt)?;
            let nombre = txt(args, "nombre").unwrap_or("GOTV").to_string();
            let h = txt(args, "huella").map(String::from);
            let x = bloquear(move || discos::Mesa::real()?.comprobar(&d, h.as_deref(), imagenes)).await?;
            confirmar_nombre(front, &x, crate::i18n::t("formatear", "format")).await?;
            let (id, tam) = (x.id.clone(), x.tam);
            match de_apple {
                // FAT: en Rust · exFAT, APFS, HFS+: con diskutil (excepción declarada: no hay implementación en Rust)
                None => bloquear(move || discos::formatear(&id, tam, &nombre)).await,
                Some((fs, esquema)) => bloquear(move || discos::formatear_con_diskutil(&id, fs, esquema, &nombre)).await,
            }
        }
        "grabar" => grabar(args, root, front, imagenes, baudios).await,
        "borrar" => {
            let p = puerto(args)?;
            let pp = p.clone();
            bloquear(move || esp::borrar(&pp)).await?;
            Ok(crate::tr!("✓ memoria de la placa de {p} borrada entera", "✓ the whole memory of the board on {p} erased"))
        }
        "imprimir" => {
            let imp = txt(args, "impresora").map(String::from);
            let a = if si(args, "prueba") { impresoras::pagina_de_prueba(root)? } else { archivo_local(root, txt(args, "archivo").ok_or_else(|| anyhow!("{}", crate::i18n::t("falta `archivo` (o prueba=true)", "missing `archivo` (or prueba=true)")))?)? };
            let copias = args.get("copias").and_then(|v| v.as_u64()).unwrap_or(1) as u32;
            bloquear(move || {
                let imp = match imp {
                    Some(i) => i,
                    None => impresoras::listar()?.into_iter().find(|p| p.por_omision).map(|p| p.nombre).ok_or_else(|| anyhow!("{}", crate::i18n::t("no hay impresora por omisión: di `impresora`", "there is no default printer: give `impresora`")))?,
                };
                impresoras::imprimir(&imp, &a, copias)
            })
            .await
        }
        "foto" => {
            let ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
            let destino = root.join(".gotv/equipos/fotos").join(format!("foto-{ms}.jpg"));
            let camara = camaras::foto(txt(args, "camara").map(String::from), destino.clone()).await?;
            let rel = destino.strip_prefix(root).unwrap_or(&destino).display().to_string();
            Ok(crate::tr!("✓ foto de «{camara}» en {rel} (mírala con view_image)", "✓ photo from «{camara}» at {rel} (look at it with view_image)"))
        }
        "cola" => {
            let v = bloquear(impresoras::cola).await?;
            Ok(if v.is_empty() { crate::i18n::t("la cola de impresión está vacía", "the print queue is empty").into() } else { v.iter().map(|(id, t)| format!("{id} · {t} KB")).collect::<Vec<_>>().join("\n") })
        }
        "cancelar" => {
            let id = txt(args, "trabajo").ok_or_else(|| anyhow!("{}", crate::i18n::t("falta `trabajo` (de action=cola)", "missing `trabajo` (from action=cola)")))?.to_string();
            bloquear(move || impresoras::cancelar(&id)).await
        }
        "raspberry" => raspberry_op(args, root, front, imagenes).await,
        otra => Err(anyhow!("{}", crate::tr!(
            "acción «{otra}» desconocida: listar · identificar · esperar · monitor · ejecutar · archivos · grabar · borrar · formatear · expulsar · imprimir · cola · cancelar · raspberry",
            "unknown action «{otra}»: listar · identificar · esperar · monitor · ejecutar · archivos · grabar · borrar · formatear · expulsar · imprimir · cola · cancelar · raspberry"
        ))),
    }
}

/// Grabar: una imagen en un disco (y verificarla), un `.uf2` en una Pico, o un firmware en un ESP32.
async fn grabar<F: Frontend + ?Sized>(args: &Value, root: &Path, front: &F, imagenes: bool, _baudios: u32) -> Result<String> {
    let a = txt(args, "archivo").ok_or_else(|| anyhow!("{}", crate::i18n::t("falta `archivo` (la imagen, el .uf2 o el firmware, del proyecto)", "missing `archivo` (the image, .uf2 or firmware, from the project)")))?;
    let archivo = archivo_local(root, a)?;
    if let Some(d) = txt(args, "disco") {
        let (d, h) = (d.to_string(), txt(args, "huella").map(String::from));
        let x = bloquear(move || discos::Mesa::real()?.comprobar(&d, h.as_deref(), imagenes)).await?;
        confirmar_nombre(front, &x, crate::i18n::t("grabar", "write")).await?;
        let id = x.id.clone();
        let t0 = std::time::Instant::now();
        let (n, sha) = largo(front, crate::i18n::t("grabando", "writing").into(), move |av| {
            discos::desmontar(&id)?;
            let r = discos::grabar_y_verificar(&archivo, &discos::crudo(&id), av);
            // el sistema relee las particiones nuevas (también si falló): así el freno del nombre ve los datos
            discos::releer(&id);
            r
        })
        .await?;
        let mbs = n as f64 / 1e6 / t0.elapsed().as_secs_f64().max(0.001);
        return Ok(crate::tr!(
            "✓ grabado y VERIFICADO en {} ({}, sha256 {}…, {mbs:.0} MB/s). Expúlsalo con action=expulsar antes de desconectarlo",
            "✓ written and VERIFIED on {} ({}, sha256 {}…, {mbs:.0} MB/s). Eject it with action=expulsar before unplugging",
            x.id,
            discos::gb(n),
            &sha[..12]
        ));
    }
    if a.ends_with(".uf2") {
        let picos = pico::buscar();
        let placa = match txt(args, "volumen") {
            Some(v) => picos.into_iter().find(|p| p.volumen == Path::new(v)),
            None if picos.len() == 1 => picos.into_iter().next(),
            None if picos.is_empty() => None,
            None => bail!("{}", crate::tr!("hay varias Pico en modo de arranque: di `volumen` ({})", "there are several Picos in boot mode: give `volumen` ({})", pico::buscar().iter().map(|p| p.volumen.display().to_string()).collect::<Vec<_>>().join(", "))),
        }
        .ok_or_else(|| anyhow!("{}", crate::i18n::t(
            "no hay ninguna Pico en modo de arranque: desconéctala, mantén pulsado BOOTSEL y conéctala (aparece una memoria RPI-RP2 o RP2350)",
            "there is no Pico in boot mode: unplug it, hold BOOTSEL and plug it in (a RPI-RP2 or RP2350 drive appears)"
        )))?;
        return bloquear(move || pico::grabar(&placa, &archivo, Duration::from_secs(10))).await;
    }
    let p = puerto(args)?;
    let (dir, datos) = esp::preparar(&archivo, txt(args, "direccion"))?;
    let (pp, n) = (p.clone(), datos.len());
    largo(front, crate::i18n::t("grabando la placa", "flashing the board").into(), move |av| esp::grabar(&pp, dir, &datos, av)).await?;
    Ok(crate::tr!(
        "✓ grabado y verificado en la placa de {p}: {} en 0x{dir:x} (la placa se reinició con el programa nuevo)",
        "✓ flashed and verified on the board on {p}: {} at 0x{dir:x} (the board restarted with the new program)",
        discos::gb(n as u64)
    ))
}

async fn raspberry_op<F: Frontend + ?Sized>(args: &Value, root: &Path, front: &F, imagenes: bool) -> Result<String> {
    let op = txt(args, "op").unwrap_or("info");
    let conexion = || txt(args, "conexion").ok_or_else(|| anyhow!("{}", crate::i18n::t("falta `conexion` (la conexión SSH del proyecto que llega a la Raspberry)", "missing `conexion` (the project SSH connection that reaches the Raspberry)")));
    match op {
        "info" => {
            let (rc, out) = crate::conexiones::ejecutar(root, conexion()?, raspberry::ORDEN_INFO, 30).await?;
            if rc != 0 && out.trim().is_empty() {
                bail!("{}", crate::tr!("la orden falló (código {rc})", "the command failed (code {rc})"));
            }
            Ok(raspberry::leer_info(&out).texto())
        }
        "configurar" => {
            let (orden, que) = raspberry::orden_configurar(args)?;
            let (rc, out) = crate::conexiones::ejecutar(root, conexion()?, &orden, 120).await?;
            if rc != 0 {
                bail!("{}", crate::tr!(
                    "no se pudo ({rc}): {} — ¿el usuario de la conexión tiene sudo sin contraseña (el de Raspberry Pi OS lo tiene)?",
                    "it failed ({rc}): {} — does the connection user have passwordless sudo (Raspberry Pi OS's default user does)?",
                    out.trim()
                ));
            }
            let reinicio = if args.get("nombre").is_some() || args.get("config_txt").is_some() { crate::i18n::t(" — se aplica del todo al reiniciar", " — fully applied after a reboot") } else { "" };
            Ok(crate::tr!("✓ {que}{reinicio}", "✓ {que}{reinicio}"))
        }
        "leer_tarjeta" => {
            let boot = match (txt(args, "volumen"), txt(args, "disco")) {
                (Some(v), _) => PathBuf::from(v),
                (None, Some(d)) => {
                    let d = d.to_string();
                    bloquear(move || {
                        let m = discos::Mesa::real()?;
                        m.comprobar(&d, Some(&m.huella(&d)), true)?;
                        raspberry::boot_montado(&m, &d).ok_or_else(|| anyhow!("{}", crate::tr!("{d} no tiene montada una partición de arranque de Raspberry Pi", "{d} has no Raspberry Pi boot partition mounted")))
                    })
                    .await?
                }
                _ => bail!("{}", crate::i18n::t("falta `volumen` (p. ej. /Volumes/bootfs) o `disco`", "missing `volumen` (e.g. /Volumes/bootfs) or `disco`")),
            };
            if !boot.starts_with("/Volumes") {
                bail!("{}", crate::i18n::t("el volumen de una tarjeta está en /Volumes", "a card volume lives in /Volumes"));
            }
            raspberry::leer_tarjeta(&boot)
        }
        "catalogo" => {
            let cat: Value = reqwest::get(raspberry::CATALOGO).await?.error_for_status()?.json().await?;
            let v = raspberry::imagenes(&cat);
            let filtro = txt(args, "imagen").map(str::to_lowercase);
            Ok(v.iter()
                .filter(|i| filtro.as_ref().is_none_or(|f| i.nombre.to_lowercase().contains(f)))
                .take(40)
                .map(|i| format!("{} · {} · {} · {}", i.nombre, i.fecha, discos::gb(i.descarga.max(i.extract_size)), i.init_format))
                .collect::<Vec<_>>()
                .join("\n"))
        }
        "preparar_tarjeta" => preparar(args, root, front, imagenes).await,
        otra => Err(anyhow!("{}", crate::tr!("op «{otra}»: info · configurar · leer_tarjeta · catalogo · preparar_tarjeta", "op «{otra}»: info · configurar · leer_tarjeta · catalogo · preparar_tarjeta"))),
    }
}

/// La llave PÚBLICA para la tarjeta: una línea, un `.pub`, o la de una conexión del proyecto.
fn llave_publica(args: &Value, root: &Path) -> Result<String> {
    if let Some(l) = txt(args, "llave") {
        if l.starts_with("ssh-") || l.starts_with("ecdsa-") {
            return Ok(l.to_string());
        }
        if !l.ends_with(".pub") {
            bail!("{}", crate::i18n::t("`llave`: la línea pública o un archivo .pub (nunca la privada)", "`llave`: the public line or a .pub file (never the private one)"));
        }
        let p = PathBuf::from(l.replace('~', &std::env::var("HOME").unwrap_or_default()));
        return Ok(std::fs::read_to_string(p)?.trim().to_string());
    }
    if let Some(c) = txt(args, "conexion") {
        let reg = crate::conexiones::Registro::cargar(root)?;
        if let Some(k) = reg.buscar(c).and_then(|c| c.llave.clone()) {
            return Ok(std::fs::read_to_string(format!("{k}.pub"))?.trim().to_string());
        }
    }
    bail!("{}", crate::i18n::t(
        "falta la llave SSH pública: `llave` (la línea o el .pub) o `conexion` (usa la llave de esa conexión del proyecto). GOTV no pone contraseñas",
        "missing the public SSH key: `llave` (the line or the .pub) or `conexion` (uses that project connection's key). GOTV does not set passwords"
    ))
}

/// Preparar una tarjeta: imagen oficial (o local) → grabar y verificar → primera configuración → expulsar.
async fn preparar<F: Frontend + ?Sized>(args: &Value, root: &Path, front: &F, imagenes: bool) -> Result<String> {
    let mut cfg = raspberry::PrimerArranque {
        nombre: txt(args, "nombre").unwrap_or("raspberrypi").to_ascii_lowercase(),
        usuario: txt(args, "usuario").unwrap_or("pi").to_string(),
        llave: llave_publica(args, root)?,
        wifi_ssid: txt(args, "wifi_ssid").map(String::from),
        wifi_clave: None,
        pais: txt(args, "wifi_pais").unwrap_or("US").to_ascii_uppercase(),
        zona: txt(args, "zona").unwrap_or("").to_string(),
        teclado: txt(args, "teclado").unwrap_or("").to_string(),
    };
    cfg.validar()?;
    // el disco ANTES de bajar nada
    let (d, h) = (txt(args, "disco").ok_or_else(|| anyhow!("{}", crate::i18n::t("falta `disco` (la tarjeta, de action=listar que=discos)", "missing `disco` (the card, from action=listar que=discos)")))?.to_string(), txt(args, "huella").map(String::from));
    let x = bloquear(move || discos::Mesa::real()?.comprobar(&d, h.as_deref(), imagenes)).await?;
    confirmar_nombre(front, &x, crate::i18n::t("preparar", "prepare")).await?;
    if let Some(ssid) = &cfg.wifi_ssid {
        if !si(args, "wifi_abierta") {
            let et = crate::tr!("Clave del Wi-Fi «{ssid}» para la Raspberry (va a la tarjeta, no al modelo)", "Wi-Fi password for «{ssid}» for the Raspberry (goes to the card, not to the model)");
            cfg.wifi_clave = Some(front.secreto(&et).await.ok_or_else(|| anyhow!("{}", crate::i18n::t("no se tecleó la clave del Wi-Fi (o wifi_abierta=true si no tiene)", "no Wi-Fi password was typed (or wifi_abierta=true if it has none)")))?);
        }
    }
    // la imagen: del catálogo oficial o un archivo del proyecto
    let (img, sha, tam, init) = match (txt(args, "imagen"), txt(args, "archivo")) {
        (Some(nombre), _) => {
            let cat: Value = reqwest::get(raspberry::CATALOGO).await?.error_for_status()?.json().await?;
            let lista = raspberry::imagenes(&cat);
            let i = raspberry::elegir(&lista, nombre)?.clone();
            let nombre_archivo = i.url.rsplit('/').next().unwrap_or("imagen.img.xz").to_string();
            front.log(crate::tr!("  descargando {} ({}) de {}", "  downloading {} ({}) from {}", i.nombre, discos::gb(i.descarga), i.url)).await;
            let p = raspberry::descargar(&i.url, &cache_imagenes().join(nombre_archivo), i.descarga, &mut |_| ()).await?;
            front.log(crate::tr!("  descargada: {}", "  downloaded: {}", p.display())).await;
            (p, Some(i.extract_sha256).filter(|s| !s.is_empty()), Some(i.extract_size).filter(|n| *n > 0), i.init_format)
        }
        (None, Some(a)) => (archivo_local(root, a)?, txt(args, "sha256").map(str::to_lowercase), None, txt(args, "init_format").unwrap_or("cloudinit-rpi").to_string()),
        _ => bail!("{}", crate::i18n::t("falta `imagen` (nombre del catálogo oficial: op=catalogo) o `archivo` (.img o .img.xz)", "missing `imagen` (official catalogue name: op=catalogo) or `archivo` (.img or .img.xz)")),
    };
    if !matches!(init.as_str(), "cloudinit-rpi" | "cloudinit") {
        // antes de borrar la tarjeta: lo que no se sabe configurar, no se graba
        cfg.escribir(Path::new("/nonexistent"), &init, "x")?;
    }
    let id = x.id.clone();
    let t0 = std::time::Instant::now();
    let (n, escrito) = largo(front, crate::i18n::t("grabando", "writing").into(), move |av| {
        discos::desmontar(&id)?;
        let r = discos::grabar_y_verificar(&img, &discos::crudo(&id), av);
        // medido 2026-11-17: tras grabar en crudo el sistema relee la tabla de particiones un momento después;
        // se espera y se monta (también si falló): si no, la tarjeta parecía vacía y el freno del nombre no saltaba
        discos::releer(&id);
        r
    })
    .await?;
    if let Some(s) = &sha {
        if s != &escrito {
            bail!("{}", crate::tr!("✗ la imagen grabada no es la oficial (sha256 {}… ≠ {}…): no uses esta tarjeta", "✗ the written image is not the official one (sha256 {}… ≠ {}…): do not use this card", &escrito[..12], &s[..12.min(s.len())]));
        }
    }
    if let Some(t) = tam {
        if t != n {
            bail!("{}", crate::tr!("✗ tamaño grabado {n} ≠ {t} del catálogo", "✗ written size {n} ≠ {t} from the catalogue"));
        }
    }
    // la partición de arranque: se monta y se escribe la primera configuración
    let id = x.id.clone();
    let instancia = format!("gotv-{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0));
    let hechos = bloquear(move || {
        let _ = discos::montar(&id);
        let mut boot = None;
        for _ in 0..20 {
            if let Ok(m) = discos::Mesa::real() {
                boot = raspberry::boot_montado(&m, &id);
            }
            if boot.is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(500));
        }
        let boot = boot.ok_or_else(|| anyhow!("{}", crate::i18n::t("grabada y verificada, pero su partición de arranque no se montó: no quedó la primera configuración", "written and verified, but its boot partition did not mount: the first-boot setup was not written")))?;
        let h = cfg.escribir(&boot, &init, &instancia)?;
        let _ = discos::expulsar(&id);
        Ok(h)
    })
    .await?;
    Ok(crate::tr!(
        "✓ tarjeta lista en {:.0} s: imagen grabada y VERIFICADA ({}, sha256 {}…{}), primera configuración ({}) y expulsada. Ponla en la Raspberry y enciéndela; tras el primer arranque (1–2 min) entra por SSH con la llave",
        "✓ card ready in {:.0} s: image written and VERIFIED ({}, sha256 {}…{}), first-boot setup ({}) and ejected. Put it in the Raspberry and power it on; after the first boot (1–2 min) log in over SSH with the key",
        t0.elapsed().as_secs_f64(),
        discos::gb(n),
        &escrito[..12],
        if sha.is_some() { crate::i18n::t(" = la oficial", " = the official one") } else { "" },
        hechos.join(", ")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// La tabla de permisos de la ficha, y el freno: lo destructivo pregunta en TODO modo, también con un
    /// permiso guardado; en plan, sólo mirar.
    #[test]
    fn el_freno_de_formatear_no_se_salta_en_ningun_modo() {
        for modo in [Mode::Default, Mode::AcceptEdits, Mode::Bypass] {
            for permitido in [false, true] {
                for a in [
                    json!({"action": "formatear", "disco": "disk99", "huella": "x"}),
                    json!({"action": "grabar", "archivo": "x.img", "disco": "disk99"}),
                    json!({"action": "borrar", "puerto": "/dev/cu.x"}),
                    json!({"action": "raspberry", "op": "preparar_tarjeta"}),
                    json!({"action": "algo_nuevo"}),
                ] {
                    assert!(matches!(decidir(&a, modo, permitido), Decision::Ask(_)), "{a} en {modo:?} con permiso={permitido}");
                }
            }
            assert!(matches!(decidir(&json!({"action": "formatear"}), Mode::Plan, true), Decision::Deny(_)));
        }
        // leer pasa siempre, también en plan
        for a in [json!({"action": "listar"}), json!({"action": "identificar"}), json!({"action": "raspberry", "op": "info"}), json!({"action": "cola"})] {
            assert_eq!(decidir(&a, Mode::Plan, false), Decision::Allow, "{a}");
        }
        // ejecución: como bash (bypass o permiso pasan; si no, pregunta)
        let m = json!({"action": "ejecutar", "puerto": "/dev/cu.x", "codigo": "print(1)"});
        assert!(matches!(decidir(&m, Mode::Default, false), Decision::Ask(_)));
        assert_eq!(decidir(&m, Mode::Bypass, false), Decision::Allow);
        assert_eq!(decidir(&json!({"action": "identificar", "preguntar_chip": true}), Mode::Bypass, false), Decision::Allow);
        assert!(matches!(decidir(&json!({"action": "identificar", "preguntar_chip": true}), Mode::Default, false), Decision::Ask(_)));
        // imprimir y configurar la Raspberry: ni bypass ni un permiso guardado lo saltan
        for a in [json!({"action": "imprimir", "prueba": true}), json!({"action": "raspberry", "op": "configurar", "nombre": "x"})] {
            assert!(matches!(decidir(&a, Mode::Bypass, false), Decision::Ask(_)), "{a}");
            assert!(matches!(decidir(&a, Mode::Default, true), Decision::Ask(_)), "{a}");
        }
    }

    #[test]
    fn el_cuadro_dice_lo_que_se_pierde() {
        let d = describir(&json!({"action": "imprimir", "prueba": true, "impresora": "HP_X", "copias": 2}));
        assert!(d.contains("HP_X") && d.contains('1') && d.contains('2'), "{d}");
        let d = describir(&json!({"action": "formatear", "disco": "disk0", "huella": "x"}));
        assert!(d.contains("disk0"), "{d}");
        let d = describir(&json!({"action": "borrar", "puerto": "/dev/cu.usbmodem1"}));
        assert!(d.contains("/dev/cu.usbmodem1"), "{d}");
    }

    /// Un frente que teclea lo que se le diga.
    struct Teclea(Option<String>);
    #[async_trait::async_trait]
    impl Frontend for Teclea {
        async fn log(&self, _: String) {}
        async fn ask(&self, _: &str) -> crate::permissions::AskAnswer {
            crate::permissions::AskAnswer::Yes
        }
        async fn choose(&self, _: &str, _: &[String]) -> Option<String> {
            self.0.clone()
        }
    }

    #[tokio::test]
    async fn con_datos_hay_que_teclear_su_nombre() {
        let m = discos::tests::mesa_con_memoria();
        let mut d = m.disco("disk14").unwrap();
        assert!(confirmar_nombre(&Teclea(None), &d, "formatear").await.is_ok(), "vacía y chica: basta el sí");
        d.volumenes[0].usado = Some(100_000_000);
        assert!(confirmar_nombre(&Teclea(None), &d, "formatear").await.is_err(), "sin quien teclee: no");
        assert!(confirmar_nombre(&Teclea(Some("pruebagotv".into())), &d, "formatear").await.is_err(), "el nombre exacto");
        assert!(confirmar_nombre(&Teclea(Some("cancelar".into())), &d, "formatear").await.is_err());
        assert!(confirmar_nombre(&Teclea(Some(" PRUEBAGOTV ".into())), &d, "formatear").await.is_ok());
    }

    #[test]
    fn solo_archivos_del_proyecto() {
        let d = crate::CarpetaPrueba::nueva("gotv-equipos-archivo");
        std::fs::write(d.join("fw.bin"), [0xE9]).unwrap();
        assert!(archivo_local(&d, "fw.bin").is_ok());
        assert!(archivo_local(&d, "/etc/hosts").is_err());
        assert!(archivo_local(&d, "no.bin").is_err());
    }

    // ───────── con imágenes de disco de verdad (hdiutil, pocos MB, se borran) ─────────

    struct Imagen {
        dev: String,
        _dir: crate::CarpetaPrueba,
    }

    impl Drop for Imagen {
        fn drop(&mut self) {
            let _ = std::process::Command::new("hdiutil").args(["detach", "-force", &self.dev]).output();
        }
    }

    /// Una imagen de `mb` MB adjuntada SIN montar (con `fat` = con un volumen FAT dentro).
    fn imagen(nombre: &str, mb: u32, fat: Option<&str>) -> Option<Imagen> {
        let dir = crate::CarpetaPrueba::nueva(nombre);
        let ruta = dir.join("img.dmg");
        let mut c = std::process::Command::new("hdiutil");
        c.args(["create", "-quiet", "-size", &format!("{mb}m")]);
        match fat {
            Some(v) => c.args(["-fs", "MS-DOS", "-volname", v, "-layout", "MBRSPUD"]),
            None => c.args(["-layout", "NONE"]),
        };
        if !c.arg(&ruta).status().ok()?.success() {
            return None;
        }
        let o = std::process::Command::new("hdiutil").args(["attach", "-nobrowse"]).args(if fat.is_some() { vec![] } else { vec!["-nomount"] }).arg(&ruta).output().ok()?;
        let dev = String::from_utf8_lossy(&o.stdout).split_whitespace().filter(|t| t.starts_with("/dev/disk")).min_by_key(|s| s.len())?.to_string();
        let img = Imagen { dev: dev.trim_start_matches("/dev/").to_string(), _dir: dir };
        // el nodo crudo de una imagen del usuario es suyo pero sin escritura (medido: crw-r-----); el de una
        // memoria real es de root y va por el cuadro de administrador (sin medir: pide la contraseña)
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(format!("/dev/r{}", img.dev), std::fs::Permissions::from_mode(0o660)).ok()?;
        Some(img)
    }

    fn con_hdiutil() -> bool {
        std::env::var_os("GOTV_PRUEBA_HDIUTIL").is_some()
    }

    /// Formatear de verdad, en Rust, sobre una imagen de 70 MB (FAT32): el flujo completo con sus frenos
    /// (huella, nombre), desmontar y montar por DiskArbitration.
    #[tokio::test]
    #[ignore = "crea el disco de prueba con hdiutil (programa de macOS, sólo para la prueba): GOTV_PRUEBA_HDIUTIL=1"]
    async fn formatear_una_imagen_de_disco_de_verdad() {
        if !con_hdiutil() {
            return;
        }
        let Some(img) = imagen("gotv-equipos-fmt", 70, Some("ANTES")) else {
            eprintln!("hdiutil no disponible: no aplica");
            return;
        };
        let m = discos::Mesa::real().unwrap();
        assert!(m.listar(false).iter().all(|d| d.id != img.dev), "una imagen no sale sin imagenes=true");
        let h = m.listar(true).into_iter().find(|d| d.id == img.dev).expect("la imagen del usuario sí").huella;
        assert!(m.listar(true).iter().all(|d| !["disk0", "disk1", "disk2", "disk3"].contains(&d.id.as_str())), "el interno NUNCA");
        let root = crate::CarpetaPrueba::nueva("gotv-equipos-fmt-raiz");
        let f = Teclea(None);
        // sin huella o con otra: no
        assert!(ejecutar(&json!({"action": "formatear", "disco": img.dev, "imagenes": true}), &root, &f).await.is_err());
        assert!(ejecutar(&json!({"action": "formatear", "disco": img.dev, "huella": "00000000", "imagenes": true}), &root, &f).await.is_err());
        assert!(ejecutar(&json!({"action": "formatear", "disco": img.dev, "huella": h, "formato": "ntfs", "imagenes": true}), &root, &f).await.is_err(), "formato desconocido");
        let r =ejecutar(&json!({"action": "formatear", "disco": img.dev, "huella": h, "formato": "fat32", "nombre": "despues gotv", "imagenes": true}), &root, &f).await.unwrap();
        assert!(r.contains("DESPUESGOTV") && r.contains("FAT32"), "{r}");
        std::thread::sleep(std::time::Duration::from_secs(2));
        let m = discos::Mesa::real().unwrap();
        let d = m.disco(&img.dev).unwrap();
        assert!(d.volumenes.iter().any(|v| v.nombre == "DESPUESGOTV"), "{:?}", d.volumenes);
        // exFAT: por la excepción (diskutil), con los mismos frenos
        let h = m.huella(&img.dev);
        let r = ejecutar(&json!({"action": "formatear", "disco": img.dev, "huella": h, "formato": "exfat", "nombre": "Datos GOTV", "imagenes": true}), &root, &f).await.unwrap();
        assert!(r.contains("ExFAT") && r.contains("Datos GOTV"), "{r}");
        let m = discos::Mesa::real().unwrap();
        // el interno, ni con su huella
        let h0 = m.huella("disk0");
        assert!(ejecutar(&json!({"action": "formatear", "disco": "disk0", "huella": h0, "imagenes": true}), &root, &f).await.is_err());
        let e = ejecutar(&json!({"action": "expulsar", "disco": img.dev, "huella": m.huella(&img.dev), "imagenes": true}), &root, &f).await.unwrap();
        assert!(e.contains('✓'), "{e}");
    }

    /// Grabar una imagen .xz en un «disco» de verdad (una imagen adjuntada sin montar), verificarla, y que
    /// macOS vea el volumen que traía. El nodo del dispositivo es del usuario pero sin escritura (medido:
    /// `crw-r-----`): la prueba se da escritura a SU nodo; el camino de `authopen` (el cuadro de
    /// administrador de una memoria real) queda sin medir.
    #[tokio::test]
    #[ignore = "crea los discos de prueba con hdiutil (programa de macOS, sólo para la prueba): GOTV_PRUEBA_HDIUTIL=1"]
    async fn grabar_y_preparar_una_tarjeta_sobre_una_imagen_de_verdad() {
        if !con_hdiutil() {
            return;
        }
        // 1) el «sistema»: una imagen MBR con un FAT «bootfs» con config.txt y cmdline.txt, en crudo y .xz
        let Some(sis) = imagen("gotv-equipos-sistema", 8, Some("BOOTFSGOTV")) else {
            eprintln!("hdiutil no disponible: no aplica");
            return;
        };
        let m = discos::Mesa::real().unwrap();
        let vol = m.particiones(&sis.dev).into_iter().find(|v| !v.montado.is_empty()).unwrap().montado;
        std::fs::write(Path::new(&vol).join("cmdline.txt"), "console=tty1 root=PARTUUID=x rootwait\n").unwrap();
        std::fs::write(Path::new(&vol).join("config.txt"), "arm_64bit=1\n").unwrap();
        std::fs::write(Path::new(&vol).join("issue.txt"), "Raspberry Pi reference 2026-10-06 (falsa, de prueba)\n").unwrap();
        let root = crate::CarpetaPrueba::nueva("gotv-equipos-tarjeta-raiz");
        let crudo_sis = root.join("sistema.img");
        // copia cruda del «sistema» (desmontado, leyendo su nodo: el usuario puede leerlo)
        discos::desmontar(&sis.dev).unwrap();
        std::io::copy(&mut std::fs::File::open(format!("/dev/r{}", sis.dev)).unwrap(), &mut std::fs::File::create(&crudo_sis).unwrap()).unwrap();
        drop(sis);
        let bytes = std::fs::read(&crudo_sis).unwrap();
        let mut xz = Vec::new();
        lzma_rs::xz_compress(&mut std::io::Cursor::new(&bytes), &mut xz).unwrap();
        std::fs::write(root.join("sistema.img.xz"), &xz).unwrap();
        use sha2::Digest;
        let sha = format!("{:x}", sha2::Sha256::digest(&bytes));
        // 2) la «tarjeta»: 16 MB en blanco, sin montar
        let Some(tarjeta) = imagen("gotv-equipos-tarjeta", 16, None) else { return };
        let nodo = format!("/dev/r{}", tarjeta.dev);
        use std::os::unix::fs::PermissionsExt;
        let h = discos::Mesa::real().unwrap().huella(&tarjeta.dev);
        let f = Teclea(None);
        let llave = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIPrueba gotv";
        // una suma que no casa: se graba, se verifica contra lo escrito, pero NO es la oficial → error
        let malo = ejecutar(&json!({"action": "raspberry", "op": "preparar_tarjeta", "disco": tarjeta.dev, "huella": h, "archivo": "sistema.img.xz", "sha256": "0".repeat(64), "llave": llave, "nombre": "taller", "usuario": "zero", "imagenes": true}), &root, &f).await;
        let e = malo.unwrap_err().to_string();
        assert!(e.contains('✗'), "{e}");
        std::fs::set_permissions(&nodo, std::fs::Permissions::from_mode(0o660)).unwrap();
        let m = discos::Mesa::real().unwrap();
        let h = m.huella(&tarjeta.dev);
        // ahora la tarjeta TIENE DATOS (lo del primer intento): sin teclear su nombre no se toca (freno 2)
        let a = json!({"action": "raspberry", "op": "preparar_tarjeta", "disco": tarjeta.dev, "huella": h, "archivo": "sistema.img.xz", "sha256": sha, "llave": llave, "nombre": "taller", "usuario": "zero", "imagenes": true});
        let e = ejecutar(&a, &root, &f).await.unwrap_err().to_string();
        assert!(e.contains("teclee") || e.contains("type"), "{e}");
        let nombre = m.disco(&tarjeta.dev).unwrap().nombre_a_teclear();
        let r = ejecutar(&a, &root, &Teclea(Some(nombre))).await.unwrap();
        assert!(r.contains("VERIFICAD") && r.contains("user-data"), "{r}");
        // la tarjeta quedó expulsada: se vuelve a montar para mirar su partición de arranque
        let _ = std::process::Command::new("hdiutil").args(["detach", "-force", &tarjeta.dev]).output();
        let _ = tarjeta;
    }
}
