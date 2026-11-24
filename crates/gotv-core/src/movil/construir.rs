//! F18 · M4/M7 · Construir la app del proyecto para el dispositivo elegido.
//!
//! - **iOS (simulador):** `xcodebuild -scheme S -destination 'platform=iOS Simulator,id=<udid>'` con sus
//!   propios derivados en `.gotv/movil/derivados` (no ensucia los de Xcode del usuario) y un paquete de
//!   resultados por construcción. Los errores salen del paquete (`xcrun xcresulttool get build-results`)
//!   y, si no lo hay, de las líneas `archivo:línea:columna: error:` de la salida. La app, de
//!   `-showBuildSettings -json`.
//! - **Android:** el `./gradlew assembleDebug` del PROYECTO (Gradle y Java son del proyecto: nivel 3); el
//!   `.apk`, de `*/build/outputs/apk/debug/`. Errores de Kotlin (`e: file:///…:l:c msg`), de Java
//!   (`…java:l: error: msg`), de AAPT y el bloque «What went wrong» de Gradle.
//! - **Flutter:** `flutter build ios --simulator --debug` / `flutter build apk --debug`.
//!
//! Medido 2026-11-17 (Xcode 27.2, iPhone 17e, app SwiftUI de juguete): construir 5,6 s en frío y 4,7 s
//! con derivados; `-showBuildSettings -json` 1,2 s. **Trampa:** en el paquete de resultados la línea y la
//! columna cuentan desde CERO (`StartingLineNumber=2` es la línea 3); la salida de texto cuenta desde 1.
//! Otra: `-resultBundlePath` falla si la carpeta ya existe, así que cada construcción lleva la suya.
use super::{Corredor, Salida};
use anyhow::{anyhow, bail, Result};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Tope de una construcción (la primera de Gradle baja medio internet).
const TOPE: Duration = Duration::from_secs(45 * 60);
/// Cuántos paquetes de resultados y registros de construcción se guardan.
const GUARDAR: usize = 3;

/// Qué clase de proyecto hay y dónde.
#[derive(Clone, Debug, PartialEq)]
pub enum Proyecto {
    /// `.xcworkspace` (CocoaPods, varios proyectos) o `.xcodeproj`.
    Xcode { contenedor: PathBuf, espacio: bool },
    /// La carpeta con `gradlew`.
    Gradle { raiz: PathBuf },
    /// La carpeta con `pubspec.yaml`.
    Flutter { raiz: PathBuf },
}

/// Un error del compilador con su sitio.
#[derive(Clone, Debug, PartialEq)]
pub struct Fallo {
    pub archivo: String,
    pub linea: u32,
    pub columna: u32,
    pub mensaje: String,
}

impl Fallo {
    /// `ruta:línea:columna: mensaje`, la ruta relativa al proyecto si cae dentro.
    pub fn texto(&self, root: &Path) -> String {
        // medido 2026-11-17: javac da `/private/var/…` para un proyecto en `/var/…` (enlace de macOS)
        let a = Path::new(&self.archivo);
        let r = match a.strip_prefix(root) {
            Ok(p) => p.display().to_string(),
            Err(_) => root.canonicalize().ok().and_then(|c| a.strip_prefix(&c).ok().map(|p| p.display().to_string())).unwrap_or_else(|| self.archivo.clone()),
        };
        match (r.is_empty(), self.linea) {
            (true, _) => self.mensaje.clone(),
            (false, 0) => format!("{r}: {}", self.mensaje),
            _ => format!("{r}:{}:{}: {}", self.linea, self.columna.max(1), self.mensaje),
        }
    }
}

/// Lo que dejó una construcción buena.
#[derive(Clone, Debug)]
pub struct Construida {
    /// La `.app` (simulador) o el `.apk`.
    pub app: PathBuf,
    pub segundos: f32,
}

/// El proyecto de la plataforma pedida: en la raíz o en `ios/` / `android/` (Flutter, React Native).
pub fn detectar(root: &Path, ios: bool) -> Option<Proyecto> {
    if root.join("pubspec.yaml").is_file() && (root.join(if ios { "ios" } else { "android" }).is_dir()) {
        return Some(Proyecto::Flutter { raiz: root.to_path_buf() });
    }
    let sub = root.join(if ios { "ios" } else { "android" });
    for dir in [root, sub.as_path()] {
        if ios {
            let hay = |ext: &str| -> Option<PathBuf> {
                let mut v: Vec<PathBuf> = std::fs::read_dir(dir).ok()?.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == ext)).collect();
                v.sort();
                v.into_iter().next()
            };
            if let Some(w) = hay("xcworkspace") {
                return Some(Proyecto::Xcode { contenedor: w, espacio: true });
            }
            if let Some(p) = hay("xcodeproj") {
                return Some(Proyecto::Xcode { contenedor: p, espacio: false });
            }
        } else if dir.join("gradlew").is_file() || (dir.join("settings.gradle.kts").is_file() || dir.join("settings.gradle").is_file()) && dir.join("app").is_dir() {
            return Some(Proyecto::Gradle { raiz: dir.to_path_buf() });
        }
    }
    None
}

/// El identificador de la app Android del proyecto: `applicationId` de `app/build.gradle(.kts)`, o el `package`
/// de su `AndroidManifest.xml` (los proyectos viejos). `None` si el proyecto no es de Android o no se lee.
pub fn app_android(root: &Path) -> Option<String> {
    let (Proyecto::Gradle { raiz } | Proyecto::Flutter { raiz }) = detectar(root, false)? else { return None };
    let base = if raiz.join("app").is_dir() { raiz.join("app") } else { raiz.join("android/app") };
    let entre_comillas = |s: &str| -> Option<String> {
        let a = s.find('"')? + 1;
        let b = s[a..].find('"')? + a;
        Some(s[a..b].to_string()).filter(|x| !x.is_empty())
    };
    for f in ["build.gradle.kts", "build.gradle"] {
        let Ok(t) = std::fs::read_to_string(base.join(f)) else { continue };
        if let Some(id) = t.match_indices("applicationId").find_map(|(i, _)| entre_comillas(t[i..].lines().next().unwrap_or(""))) {
            return Some(id);
        }
    }
    let m = std::fs::read_to_string(base.join("src/main/AndroidManifest.xml")).ok()?;
    entre_comillas(&m[m.find("package=")?..])
}

/// Las líneas `ruta:línea:columna: error: mensaje` (Swift, clang, ld) de la salida de `xcodebuild`, sin
/// repetir (Xcode las dice dos veces: al emitir el módulo y al compilar). Las `xcodebuild: error:` sin
/// sitio también cuentan.
pub fn errores_xcodebuild(log: &str) -> Vec<Fallo> {
    let mut v: Vec<Fallo> = Vec::new();
    for l in log.lines() {
        let l = l.trim_end();
        let f = if let Some((sitio, msg)) = l.split_once(": error: ") {
            let mut p = sitio.rsplitn(3, ':');
            let (c, n, a) = (p.next(), p.next(), p.next());
            match (a, n.and_then(|n| n.parse().ok()), c.and_then(|c| c.parse().ok())) {
                (Some(a), Some(n), Some(c)) if a.starts_with('/') => Fallo { archivo: a.into(), linea: n, columna: c, mensaje: msg.trim().into() },
                _ if sitio == "xcodebuild" || sitio.ends_with("/xcodebuild") => Fallo { archivo: String::new(), linea: 0, columna: 0, mensaje: msg.trim().into() },
                _ => continue,
            }
        } else if let Some(msg) = l.strip_prefix("xcodebuild: error: ") {
            Fallo { archivo: String::new(), linea: 0, columna: 0, mensaje: msg.trim().into() }
        } else {
            continue;
        };
        if !v.contains(&f) {
            v.push(f);
        }
    }
    v
}

/// `%20` y compañía de una URL de archivo.
fn sin_por_ciento(s: &str) -> String {
    let b = s.as_bytes();
    let mut o = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(x) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                o.push(x);
                i += 3;
                continue;
            }
        }
        o.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&o).into_owned()
}

/// Los errores del paquete de resultados (`xcresulttool get build-results`). Línea y columna, +1.
pub fn errores_xcresult(j: &Value) -> Vec<Fallo> {
    let mut v = Vec::new();
    for e in j.get("errors").and_then(|x| x.as_array()).into_iter().flatten() {
        let mensaje = e.get("message").and_then(|m| m.as_str()).unwrap_or("").trim().to_string();
        let url = e.get("sourceURL").and_then(|u| u.as_str()).unwrap_or("");
        let (ruta, frag) = url.split_once('#').unwrap_or((url, ""));
        let archivo = ruta.strip_prefix("file://").map(sin_por_ciento).unwrap_or_default();
        let campo = |k: &str| frag.split('&').find_map(|p| p.strip_prefix(k).and_then(|x| x.strip_prefix('='))).and_then(|x| x.parse::<u32>().ok());
        let (linea, columna) = match (campo("StartingLineNumber"), campo("StartingColumnNumber")) {
            (Some(l), c) => (l + 1, c.map(|c| c + 1).unwrap_or(1)),
            _ => (0, 0),
        };
        let f = Fallo { archivo, linea, columna, mensaje };
        if !f.mensaje.is_empty() && !v.contains(&f) {
            v.push(f);
        }
    }
    v
}

/// Los errores de una construcción de Gradle (Kotlin, Java, AAPT) y, si no hay ninguno con sitio, lo
/// que Gradle dice en «What went wrong».
pub fn errores_gradle(log: &str) -> Vec<Fallo> {
    let mut v: Vec<Fallo> = Vec::new();
    let mut empujar = |f: Fallo| {
        if !v.contains(&f) {
            v.push(f);
        }
    };
    for l in log.lines() {
        let l = l.trim();
        // Kotlin 2: «e: file:///ruta/A.kt:12:5 mensaje»; Kotlin 1: «e: /ruta/A.kt: (12, 5): mensaje»
        if let Some(r) = l.strip_prefix("e: ") {
            let r = r.strip_prefix("file://").unwrap_or(r);
            if let Some((sitio, msg)) = r.split_once(": (") {
                if let Some((nc, m)) = msg.split_once("): ") {
                    let mut p = nc.split(", ");
                    let (n, c) = (p.next().and_then(|x| x.trim().parse().ok()), p.next().and_then(|x| x.trim().parse().ok()));
                    empujar(Fallo { archivo: sin_por_ciento(sitio), linea: n.unwrap_or(0), columna: c.unwrap_or(0), mensaje: m.trim().into() });
                    continue;
                }
            }
            if let Some((sitio, msg)) = r.split_once(' ') {
                let mut p = sitio.rsplitn(3, ':');
                let (c, n, a) = (p.next().and_then(|x| x.parse().ok()), p.next().and_then(|x| x.parse().ok()), p.next());
                if let (Some(a), Some(n), Some(c)) = (a, n, c) {
                    empujar(Fallo { archivo: sin_por_ciento(a), linea: n, columna: c, mensaje: msg.trim().into() });
                }
            }
            continue;
        }
        // Java: «/ruta/A.java:12: error: mensaje»; AAPT: «ERROR: /ruta/x.xml:5: AAPT: error: mensaje»
        let l2 = l.strip_prefix("ERROR: ").or_else(|| l.strip_prefix("ERROR:")).unwrap_or(l).trim();
        if let Some((sitio, msg)) = l2.split_once(": error: ").or_else(|| l2.split_once(": AAPT: error: ")) {
            let sitio = sitio.strip_suffix(": AAPT").unwrap_or(sitio);
            if let Some((a, n)) = sitio.rsplit_once(':') {
                if let (true, Ok(n)) = (a.starts_with('/'), n.parse::<u32>()) {
                    empujar(Fallo { archivo: a.into(), linea: n, columna: 0, mensaje: msg.trim().into() });
                }
            }
        }
    }
    if v.is_empty() {
        if let Some((_, resto)) = log.split_once("* What went wrong:") {
            let dijo: Vec<&str> = resto.lines().map(str::trim).skip_while(|l| l.is_empty()).take_while(|l| !l.starts_with("* Try:") && !l.starts_with("* Get more help")).filter(|l| !l.is_empty()).take(12).collect();
            if !dijo.is_empty() {
                v.push(Fallo { archivo: String::new(), linea: 0, columna: 0, mensaje: dijo.join("\n") });
            }
        }
    }
    v
}

fn s(v: &[&str]) -> Vec<String> {
    v.iter().map(|x| x.to_string()).collect()
}

fn ms() -> u128 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0)
}

/// Deja sólo las `GUARDAR` entradas más nuevas de una carpeta cuyo nombre empiece por `prefijo`.
fn podar(dir: &Path, prefijo: &str) {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with(prefijo))).collect();
    v.sort();
    let sobran = v.len().saturating_sub(GUARDAR);
    for p in v.into_iter().take(sobran) {
        let _ = if p.is_dir() { std::fs::remove_dir_all(&p) } else { std::fs::remove_file(&p) };
    }
}

fn carpeta(root: &Path, que: &str) -> PathBuf {
    root.join(".gotv").join("movil").join(que)
}

/// Guarda la salida entera de la construcción y devuelve dónde.
fn guardar_registro(root: &Path, o: &Salida) -> PathBuf {
    let dir = carpeta(root, "registros");
    let _ = std::fs::create_dir_all(&dir);
    podar(&dir, "construir-");
    let p = dir.join(format!("construir-{}.txt", ms()));
    let mut t = o.texto();
    if !o.err.trim().is_empty() {
        t.push_str("\n--- stderr ---\n");
        t.push_str(&o.err);
    }
    let _ = std::fs::write(&p, t);
    p
}

/// El error que ve el modelo: los fallos con su sitio (máx. 20) y dónde está la salida entera.
fn informe(root: &Path, fallos: &[Fallo], registro: &Path, cola: &str) -> anyhow::Error {
    let rel = registro.strip_prefix(root).unwrap_or(registro).display().to_string();
    if fallos.is_empty() {
        let ultimas: Vec<&str> = cola.lines().rev().filter(|l| !l.trim().is_empty()).take(25).collect::<Vec<_>>().into_iter().rev().collect();
        return anyhow!("{}", crate::tr!("✗ la construcción falló sin errores con sitio; últimas líneas:\n{}\n(salida entera en {rel})", "✗ the build failed with no located errors; last lines:\n{}\n(full output in {rel})", ultimas.join("\n")));
    }
    let mut l: Vec<String> = fallos.iter().take(20).map(|f| f.texto(root)).collect();
    if fallos.len() > 20 {
        l.push(crate::tr!("… y {} más", "… and {} more", fallos.len() - 20));
    }
    anyhow!("{}", crate::tr!("✗ la construcción falló ({} errores):\n{}\n(salida entera en {rel})", "✗ the build failed ({} errors):\n{}\n(full output in {rel})", fallos.len(), l.join("\n")))
}

/// El esquema: el pedido; si no, el único; si hay varios, el que se llama como el proyecto.
fn elegir_esquema(lista: &Value, contenedor: &Path, pedido: Option<&str>) -> Result<String> {
    let esquemas: Vec<String> = ["project", "workspace"].iter().filter_map(|k| lista.get(k)?.get("schemes")?.as_array().cloned()).flatten().filter_map(|x| x.as_str().map(String::from)).collect();
    if let Some(p) = pedido {
        if esquemas.iter().any(|e| e == p) || esquemas.is_empty() {
            return Ok(p.into());
        }
        bail!("{}", crate::tr!("no hay esquema «{p}»; hay: {}", "there is no scheme «{p}»; available: {}", esquemas.join(", ")));
    }
    let nombre = contenedor.file_stem().and_then(|n| n.to_str()).unwrap_or("");
    match esquemas.len() {
        0 => bail!("{}", crate::tr!("{} no tiene esquemas (ábrelo una vez en Xcode o crea uno compartido)", "{} has no schemes (open it once in Xcode or create a shared one)", contenedor.display())),
        1 => Ok(esquemas[0].clone()),
        _ => esquemas.iter().find(|e| *e == nombre).cloned().ok_or_else(|| anyhow!("{}", crate::tr!("hay varios esquemas: di cuál con `esquema`: {}", "there are several schemes: say which one with `esquema`: {}", esquemas.join(", ")))),
    }
}

/// La `.app` que dejó la construcción, de `-showBuildSettings -json`.
fn app_de_ajustes(j: &Value) -> Option<PathBuf> {
    j.as_array()?.iter().filter_map(|t| t.get("buildSettings")).find(|b| b.get("WRAPPER_EXTENSION").and_then(|x| x.as_str()) == Some("app")).and_then(|b| {
        let dir = b.get("TARGET_BUILD_DIR")?.as_str()?;
        let nombre = b.get("FULL_PRODUCT_NAME")?.as_str()?;
        Some(Path::new(dir).join(nombre))
    })
}

/// Construye para el simulador `udid`.
pub async fn ios(run: &dyn Corredor, root: &Path, proyecto: &Proyecto, udid: &str, esquema: Option<&str>) -> Result<Construida> {
    let t0 = std::time::Instant::now();
    match proyecto {
        Proyecto::Flutter { raiz } => {
            let o = run.correr("env", &s(&["-C", &raiz.display().to_string(), "flutter", "build", "ios", "--simulator", "--debug", "--no-codesign"]), TOPE).await?;
            if !o.ok {
                let r = guardar_registro(root, &o);
                let todo = format!("{}\n{}", o.texto(), o.err);
                return Err(informe(root, &errores_xcodebuild(&todo), &r, &todo));
            }
            let app = raiz.join("build/ios/iphonesimulator/Runner.app");
            if !app.is_dir() {
                bail!("{}", crate::tr!("flutter construyó pero no está {}", "flutter built but {} is missing", app.display()));
            }
            return Ok(Construida { app, segundos: t0.elapsed().as_secs_f32() });
        }
        Proyecto::Gradle { .. } => bail!("{}", crate::i18n::t("éste es un proyecto de Android: elige un emulador o teléfono Android", "this is an Android project: choose an Android emulator or phone")),
        Proyecto::Xcode { .. } => {}
    }
    let Proyecto::Xcode { contenedor, espacio } = proyecto else { unreachable!() };
    let que = if *espacio { "-workspace" } else { "-project" };
    let cont = contenedor.display().to_string();
    let lista = run.correr("xcodebuild", &s(&["-list", "-json", que, &cont]), Duration::from_secs(120)).await?.exigir("xcodebuild -list")?;
    let lista: Value = serde_json::from_slice(&lista.out).unwrap_or(Value::Null);
    let esquema = elegir_esquema(&lista, contenedor, esquema)?;
    let derivados = carpeta(root, "derivados");
    let resultados = carpeta(root, "resultados");
    std::fs::create_dir_all(&resultados)?;
    podar(&resultados, "construir-");
    let paquete = resultados.join(format!("construir-{}.xcresult", ms()));
    let destino = format!("platform=iOS Simulator,id={udid}");
    let base = s(&[que, &cont, "-scheme", &esquema, "-destination", &destino, "-derivedDataPath", &derivados.display().to_string()]);
    let mut args = base.clone();
    args.extend(s(&["-resultBundlePath", &paquete.display().to_string(), "build"]));
    let o = run.correr("xcodebuild", &args, TOPE).await?;
    if !o.ok {
        let r = guardar_registro(root, &o);
        let mut fallos = Vec::new();
        if paquete.exists() {
            if let Ok(x) = run.correr("xcrun", &s(&["xcresulttool", "get", "build-results", "--path", &paquete.display().to_string(), "--compact"]), Duration::from_secs(60)).await {
                fallos = errores_xcresult(&serde_json::from_slice(&x.out).unwrap_or(Value::Null));
            }
        }
        let todo = format!("{}\n{}", o.texto(), o.err);
        if fallos.is_empty() {
            fallos = errores_xcodebuild(&todo);
        }
        return Err(informe(root, &fallos, &r, &todo));
    }
    let mut a = base;
    a.extend(s(&["-showBuildSettings", "-json"]));
    let aj = run.correr("xcodebuild", &a, Duration::from_secs(120)).await?.exigir("xcodebuild -showBuildSettings")?;
    let app = app_de_ajustes(&serde_json::from_slice(&aj.out).unwrap_or(Value::Null)).ok_or_else(|| anyhow!("{}", crate::tr!("el esquema «{esquema}» no produce una .app", "scheme «{esquema}» does not produce an .app")))?;
    Ok(Construida { app, segundos: t0.elapsed().as_secs_f32() })
}

/// El `.apk` de depuración más nuevo bajo `raiz` (`<módulo>/build/outputs/apk/debug/*.apk`).
fn apk_mas_nuevo(raiz: &Path) -> Option<PathBuf> {
    let mut mejor: Option<(std::time::SystemTime, PathBuf)> = None;
    let mut pila = vec![(raiz.to_path_buf(), 0)];
    while let Some((d, n)) = pila.pop() {
        for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            let p = e.path();
            let nombre = e.file_name().to_string_lossy().into_owned();
            if p.is_dir() && n < 7 && !nombre.starts_with('.') && nombre != "node_modules" && nombre != "intermediates" {
                pila.push((p, n + 1));
            } else if nombre.ends_with(".apk") && p.parent().is_some_and(|x| x.ends_with("outputs/apk/debug") || x.ends_with("outputs/flutter-apk")) {
                let t = e.metadata().and_then(|m| m.modified()).unwrap_or(std::time::UNIX_EPOCH);
                if mejor.as_ref().is_none_or(|(m, _)| t > *m) {
                    mejor = Some((t, p));
                }
            }
        }
    }
    mejor.map(|(_, p)| p)
}

/// Construye el `.apk` de depuración.
pub async fn android(run: &dyn Corredor, root: &Path, proyecto: &Proyecto, sdk: Option<&Path>) -> Result<Construida> {
    let t0 = std::time::Instant::now();
    let (raiz, orden): (&Path, Vec<String>) = match proyecto {
        // el envoltorio del proyecto; sin él, el `gradle` instalado
        Proyecto::Gradle { raiz } => (raiz, s(&[if raiz.join("gradlew").is_file() { "./gradlew" } else { "gradle" }, "assembleDebug", "--console=plain"])),
        Proyecto::Flutter { raiz } => (raiz, s(&["flutter", "build", "apk", "--debug"])),
        Proyecto::Xcode { .. } => bail!("{}", crate::i18n::t("éste es un proyecto de Xcode: elige un simulador de iOS", "this is an Xcode project: choose an iOS simulator")),
    };
    let mut args = s(&["-C", &raiz.display().to_string()]);
    // Gradle encuentra el SDK por `local.properties` o por ANDROID_HOME: si no hay ninguno, se le da el que hay
    let tiene_local = raiz.join("local.properties").is_file() && std::fs::read_to_string(raiz.join("local.properties")).is_ok_and(|t| t.contains("sdk.dir"));
    if let (false, None, Some(sdk)) = (tiene_local, std::env::var_os("ANDROID_HOME"), sdk) {
        args.push(format!("ANDROID_HOME={}", sdk.display()));
    }
    args.extend(orden);
    let o = run.correr("env", &args, TOPE).await?;
    if !o.ok {
        let r = guardar_registro(root, &o);
        let todo = format!("{}\n{}", o.texto(), o.err);
        return Err(informe(root, &errores_gradle(&todo), &r, &todo));
    }
    let app = apk_mas_nuevo(raiz).ok_or_else(|| anyhow!("{}", crate::tr!("la construcción terminó pero no encuentro el .apk en {}", "the build finished but I can't find the .apk under {}", raiz.display())))?;
    Ok(Construida { app, segundos: t0.elapsed().as_secs_f32() })
}

/// Una app SwiftUI mínima (un `.xcodeproj` escrito a mano, sin generadores) que muestra `texto`: la usan
/// las pruebas reales y el banco `movil-ios`.
pub fn crear_juguete_ios(dir: &Path, nombre: &str, bundle: &str, swift: &str) -> Result<PathBuf> {
    let src = dir.join(nombre);
    std::fs::create_dir_all(&src)?;
    std::fs::write(src.join("App.swift"), swift)?;
    let proj = dir.join(format!("{nombre}.xcodeproj"));
    std::fs::create_dir_all(&proj)?;
    std::fs::write(proj.join("project.pbxproj"), PBXPROJ.replace("@NOMBRE@", nombre).replace("@BUNDLE@", bundle))?;
    Ok(proj)
}

/// Una app Android mínima (Gradle con el plugin de Android 9.5.1, una actividad en Java sin AndroidX) cuya
/// actividad es `java` (el cuerpo de `MainActivity.java`): la usan las pruebas reales y el banco
/// `movil-android`. Sin `gradlew`: quien la use pone el suyo.
pub fn crear_juguete_android(dir: &Path, paquete: &str, java: &str) -> Result<()> {
    let src = dir.join("app/src/main/java").join(paquete.replace('.', "/"));
    std::fs::create_dir_all(&src)?;
    std::fs::write(dir.join("settings.gradle.kts"), "pluginManagement { repositories { google(); mavenCentral(); gradlePluginPortal() } }\ndependencyResolutionManagement { repositories { google(); mavenCentral() } }\nrootProject.name = \"Juguete\"\ninclude(\":app\")\n")?;
    std::fs::write(dir.join("build.gradle.kts"), "plugins { id(\"com.android.application\") version \"9.5.1\" apply false }\n")?;
    std::fs::write(dir.join("gradle.properties"), "org.gradle.jvmargs=-Xmx2g\n")?;
    std::fs::write(
        dir.join("app/build.gradle.kts"),
        format!("plugins {{ id(\"com.android.application\") }}\nandroid {{\n    namespace = \"{paquete}\"\n    compileSdk = 36\n    defaultConfig {{ applicationId = \"{paquete}\"; minSdk = 24; targetSdk = 36; versionCode = 1; versionName = \"1\" }}\n}}\n"),
    )?;
    std::fs::write(
        dir.join("app/src/main/AndroidManifest.xml"),
        "<manifest xmlns:android=\"http://schemas.android.com/apk/res/android\">\n  <application android:label=\"Juguete\">\n    <activity android:name=\".MainActivity\" android:exported=\"true\">\n      <intent-filter><action android:name=\"android.intent.action.MAIN\"/><category android:name=\"android.intent.category.LAUNCHER\"/></intent-filter>\n    </activity>\n  </application>\n</manifest>\n",
    )?;
    std::fs::write(src.join("MainActivity.java"), format!("package {paquete};\n{java}"))?;
    Ok(())
}

/// `project.pbxproj` mínimo: un objetivo de app, una fuente, Debug sin firma. Identificadores fijos.
const PBXPROJ: &str = r#"// !$*UTF8*$!
{
	archiveVersion = 1;
	classes = {
	};
	objectVersion = 77;
	objects = {
		A0000000000000000000000B = {isa = PBXBuildFile; fileRef = A0000000000000000000000F; };
		A0000000000000000000000F = {isa = PBXFileReference; lastKnownFileType = sourcecode.swift; path = App.swift; sourceTree = "<group>"; };
		A000000000000000000000AA = {isa = PBXFileReference; explicitFileType = wrapper.application; includeInIndex = 0; path = "@NOMBRE@.app"; sourceTree = BUILT_PRODUCTS_DIR; };
		A0000000000000000000000C = {isa = PBXGroup; children = (A0000000000000000000000F, ); path = "@NOMBRE@"; sourceTree = "<group>"; };
		A0000000000000000000000D = {isa = PBXGroup; children = (A000000000000000000000AA, ); name = Products; sourceTree = "<group>"; };
		A0000000000000000000000E = {isa = PBXGroup; children = (A0000000000000000000000C, A0000000000000000000000D, ); sourceTree = "<group>"; };
		A00000000000000000000001 = {isa = PBXSourcesBuildPhase; buildActionMask = 2147483647; files = (A0000000000000000000000B, ); runOnlyForDeploymentPostprocessing = 0; };
		A00000000000000000000002 = {isa = PBXNativeTarget; buildConfigurationList = A00000000000000000000005; buildPhases = (A00000000000000000000001, ); buildRules = ( ); dependencies = ( ); name = "@NOMBRE@"; productName = "@NOMBRE@"; productReference = A000000000000000000000AA; productType = "com.apple.product-type.application"; };
		A00000000000000000000003 = {isa = XCBuildConfiguration; buildSettings = {CODE_SIGNING_ALLOWED = NO; GENERATE_INFOPLIST_FILE = YES; INFOPLIST_KEY_UILaunchScreen_Generation = YES; IPHONEOS_DEPLOYMENT_TARGET = 17.0; LD_RUNPATH_SEARCH_PATHS = ("$(inherited)", "@executable_path/Frameworks", ); PRODUCT_BUNDLE_IDENTIFIER = "@BUNDLE@"; PRODUCT_NAME = "$(TARGET_NAME)"; SDKROOT = iphoneos; SWIFT_VERSION = 5.0; TARGETED_DEVICE_FAMILY = "1,2"; }; name = Debug; };
		A00000000000000000000004 = {isa = XCBuildConfiguration; buildSettings = {ONLY_ACTIVE_ARCH = YES; SDKROOT = iphoneos; SWIFT_OPTIMIZATION_LEVEL = "-Onone"; }; name = Debug; };
		A00000000000000000000005 = {isa = XCConfigurationList; buildConfigurations = (A00000000000000000000003, ); defaultConfigurationIsVisible = 0; defaultConfigurationName = Debug; };
		A00000000000000000000006 = {isa = XCConfigurationList; buildConfigurations = (A00000000000000000000004, ); defaultConfigurationIsVisible = 0; defaultConfigurationName = Debug; };
		A00000000000000000000007 = {isa = PBXProject; attributes = {BuildIndependentTargetsInParallel = YES; LastUpgradeCheck = 1430; }; buildConfigurationList = A00000000000000000000006; developmentRegion = en; hasScannedForEncodings = 0; knownRegions = (Base, en, ); mainGroup = A0000000000000000000000E; productRefGroup = A0000000000000000000000D; projectDirPath = ""; projectRoot = ""; targets = (A00000000000000000000002, ); };
	};
	rootObject = A00000000000000000000007;
}
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::movil::tests::Falso;
    use serde_json::json;

    /// Las líneas de la salida de Xcode (medidas en Xcode 27.2): sitio, sin repetir, y la del propio
    /// `xcodebuild` sin sitio. Las notas y avisos no cuentan.
    #[test]
    fn errores_de_la_salida_de_xcodebuild() {
        let log = "/p/App.swift:3:14: error: cannot convert value of type 'String' to specified type 'Int'\n  |              `- error: cannot convert\n/p/App.swift:3:14: error: cannot convert value of type 'String' to specified type 'Int'\n/p/Vista.swift:9:2: warning: unused\n/p/Vista.swift:12:7: error: cannot find 'nombre' in scope\nxcodebuild: error: Unable to find a destination matching the provided destination specifier\n";
        let f = errores_xcodebuild(log);
        assert_eq!(f.len(), 3, "{f:?}");
        assert_eq!(f[0], Fallo { archivo: "/p/App.swift".into(), linea: 3, columna: 14, mensaje: "cannot convert value of type 'String' to specified type 'Int'".into() });
        assert_eq!(f[1].texto(Path::new("/p")), "Vista.swift:12:7: cannot find 'nombre' in scope");
        assert_eq!(f[2].texto(Path::new("/p")), "Unable to find a destination matching the provided destination specifier");
    }

    /// El paquete de resultados cuenta desde CERO (medido): `StartingLineNumber=2` es la línea 3. Las rutas
    /// llegan como URL (`%20`).
    #[test]
    fn errores_del_paquete_de_resultados() {
        let j = json!({"errorCount": 2, "errors": [
            {"issueType": "Swift Compiler Error", "message": "Cannot convert value of type 'String' to specified type 'Int'", "sourceURL": "file:///p/Mi%20App/App.swift#EndingColumnNumber=13&EndingLineNumber=2&StartingColumnNumber=13&StartingLineNumber=2&Timestamp=816627066.45"},
            {"issueType": "Error", "message": "Signing requires a development team"}
        ]});
        let f = errores_xcresult(&j);
        assert_eq!(f.len(), 2);
        assert_eq!(f[0].texto(Path::new("/p")), "Mi App/App.swift:3:14: Cannot convert value of type 'String' to specified type 'Int'");
        assert_eq!(f[1].texto(Path::new("/p")), "Signing requires a development team");
    }

    /// Kotlin 2 y 1, Java, AAPT; y, sin ninguno de ésos, el «What went wrong» de Gradle.
    #[test]
    fn errores_de_gradle() {
        let log = "> Task :app:compileDebugKotlin FAILED\ne: file:///p/app/src/main/java/MainActivity.kt:12:5 Unresolved reference 'saludo'.\ne: /p/app/src/main/java/Viejo.kt: (7, 9): Type mismatch\n/p/app/src/main/java/A.java:4: error: cannot find symbol\nERROR: /p/app/src/main/res/layout/main.xml:5: AAPT: error: attribute android:textt not found.\n\n* What went wrong:\nExecution failed for task ':app:compileDebugKotlin'.\n";
        let f = errores_gradle(log);
        let t: Vec<String> = f.iter().map(|x| x.texto(Path::new("/p"))).collect();
        assert_eq!(t, ["app/src/main/java/MainActivity.kt:12:5: Unresolved reference 'saludo'.", "app/src/main/java/Viejo.kt:7:9: Type mismatch", "app/src/main/java/A.java:4:1: cannot find symbol", "app/src/main/res/layout/main.xml:5:1: attribute android:textt not found."]);
        let g = errores_gradle("FAILURE: Build failed with an exception.\n\n* What went wrong:\nSDK location not found. Define a valid SDK location with an ANDROID_HOME environment variable.\n\n* Try:\n> Run with --stacktrace\n");
        assert_eq!(g.len(), 1);
        assert!(g[0].mensaje.starts_with("SDK location not found"), "{g:?}");
    }

    /// Detecta Xcode (el espacio de trabajo antes que el proyecto), Gradle en la raíz o en `android/`, y
    /// Flutter.
    #[test]
    fn detecta_el_proyecto() {
        let d = crate::CarpetaPrueba::nueva("gotv-construir-detectar");
        assert_eq!(detectar(&d, true), None);
        std::fs::create_dir_all(d.join("A.xcodeproj")).unwrap();
        assert_eq!(detectar(&d, true), Some(Proyecto::Xcode { contenedor: d.join("A.xcodeproj"), espacio: false }));
        std::fs::create_dir_all(d.join("A.xcworkspace")).unwrap();
        assert_eq!(detectar(&d, true), Some(Proyecto::Xcode { contenedor: d.join("A.xcworkspace"), espacio: true }));
        std::fs::create_dir_all(d.join("android")).unwrap();
        std::fs::write(d.join("android/gradlew"), "#!/bin/sh").unwrap();
        assert_eq!(detectar(&d, false), Some(Proyecto::Gradle { raiz: d.join("android") }));
        std::fs::write(d.join("pubspec.yaml"), "name: x").unwrap();
        assert_eq!(detectar(&d, false), Some(Proyecto::Flutter { raiz: d.to_path_buf() }));
    }

    /// iOS con el corredor falso: elige el único esquema, construye con derivados y paquete propios (por
    /// UDID), y la `.app` sale de los ajustes. Si falla, los errores vienen del paquete y la salida entera
    /// queda en disco.
    #[tokio::test]
    async fn construir_ios_con_el_falso() {
        let d = crate::CarpetaPrueba::nueva("gotv-construir-ios");
        let proj = crear_juguete_ios(&d, "Juguete", "space.gotv.juguete", "import SwiftUI").unwrap();
        let p = detectar(&d, true).unwrap();
        let udid = "B179907A-9485-4F1C-9343-BEEC74AFA2AC";
        let ajustes = json!([{"action": "build", "buildSettings": {"WRAPPER_EXTENSION": "app", "TARGET_BUILD_DIR": "/der/Debug-iphonesimulator", "FULL_PRODUCT_NAME": "Juguete.app"}, "target": "Juguete"}]).to_string();
        let f = Falso::con(&[("-list -json", r#"{"project":{"schemes":["Juguete"]}}"#), ("-showBuildSettings", &ajustes)]);
        let c = ios(&*f, &d, &p, udid, None).await.unwrap();
        assert_eq!(c.app, Path::new("/der/Debug-iphonesimulator/Juguete.app"));
        let h = f.hechas();
        let b = h.iter().find(|o| o.ends_with(" build")).unwrap();
        assert!(b.contains(&format!("-project {}", proj.display())) && b.contains("-scheme Juguete") && b.contains(&format!("platform=iOS Simulator,id={udid}")) && b.contains(".gotv/movil/derivados") && b.contains(".gotv/movil/resultados/construir-"), "{b}");

        // falla: el falso no deja paquete de resultados, así que los errores salen de las líneas de la
        // salida (el paquete se prueba de verdad en `construir_ios_de_verdad`)
        let f = Falso::con(&[("-list -json", r#"{"project":{"schemes":["Juguete","Otro"]}}"#)]);
        f.regla(" build", false, b"/x.swift:1:1: error: otra cosa\n** BUILD FAILED **");
        let e = ios(&*f, &d, &p, udid, None).await.unwrap_err().to_string();
        assert!(e.contains("✗") && e.contains("/x.swift:1:1: otra cosa") && e.contains(".gotv/movil/registros/construir-"), "{e}");
        // dos esquemas: elige el que se llama como el proyecto
        assert!(f.hechas().iter().any(|o| o.contains("-scheme Juguete")), "{:?}", f.hechas());
        // un esquema que no existe
        let e = ios(&*f, &d, &p, udid, Some("Nada")).await.unwrap_err().to_string();
        assert!(e.contains("Juguete, Otro"), "{e}");
    }

    /// Android con el corredor falso: `./gradlew assembleDebug` en la carpeta del proyecto (por `env -C`),
    /// ANDROID_HOME sólo si el proyecto no dice dónde está el SDK, y el `.apk` más nuevo.
    #[tokio::test]
    async fn construir_android_con_el_falso() {
        let d = crate::CarpetaPrueba::nueva("gotv-construir-android");
        std::fs::write(d.join("gradlew"), "#!/bin/sh").unwrap();
        let apk = d.join("app/build/outputs/apk/debug/app-debug.apk");
        std::fs::create_dir_all(apk.parent().unwrap()).unwrap();
        std::fs::write(&apk, "PK").unwrap();
        let p = detectar(&d, false).unwrap();
        let f = Falso::con(&[]);
        let c = android(&*f, &d, &p, Some(Path::new("/sdk"))).await.unwrap();
        assert_eq!(c.app, apk);
        let o = &f.hechas()[0];
        assert!(o.starts_with(&format!("env -C {}", d.display())) && o.ends_with("./gradlew assembleDebug --console=plain"), "{o}");
        assert_eq!(o.contains("ANDROID_HOME=/sdk"), std::env::var_os("ANDROID_HOME").is_none(), "{o}");
        std::fs::write(d.join("local.properties"), "sdk.dir=/otro").unwrap();
        let f = Falso::con(&[]);
        android(&*f, &d, &p, Some(Path::new("/sdk"))).await.unwrap();
        assert!(!f.hechas()[0].contains("ANDROID_HOME"), "{:?}", f.hechas());
        // falla con un error de Kotlin
        let f = Falso::con(&[]);
        f.regla("gradlew", false, format!("e: file://{}/app/src/main/java/M.kt:3:1 Unresolved reference 'x'.\nBUILD FAILED", d.display()).as_bytes());
        let e = android(&*f, &d, &p, None).await.unwrap_err().to_string();
        assert!(e.contains("app/src/main/java/M.kt:3:1: Unresolved reference 'x'."), "{e}");
    }

    /// De verdad: escribe la app de juguete, la construye para el iPhone 17e con Xcode, mete un error y
    /// lo lee del paquete de resultados con su línea. `cargo test -p gotv-core construir_ios_de_verdad --
    /// --ignored` (Xcode y el simulador `GOTV_PRUEBA_IOS_UDID` o el 17e).
    #[tokio::test]
    #[ignore]
    async fn construir_ios_de_verdad() {
        let udid = std::env::var("GOTV_PRUEBA_IOS_UDID").unwrap_or_else(|_| "B179907A-9485-4F1C-9343-BEEC74AFA2AC".into());
        let d = crate::CarpetaPrueba::nueva("gotv-construir-ios-real");
        crear_juguete_ios(&d, "Juguete", "space.gotv.juguete", "import SwiftUI\n@main struct JugueteApp: App { var body: some Scene { WindowGroup { Text(\"Hola ñandú\") } } }\n").unwrap();
        let p = detectar(&d, true).unwrap();
        let run = super::super::Sistema;
        let c = ios(&run, &d, &p, &udid, None).await.unwrap();
        eprintln!("construida en {:.1} s: {}", c.segundos, c.app.display());
        assert!(c.app.join("Info.plist").is_file(), "{}", c.app.display());
        assert_eq!(super::super::ios::de_info_plist(&c.app, "CFBundleIdentifier").unwrap(), "space.gotv.juguete");
        std::fs::write(d.join("Juguete/App.swift"), "import SwiftUI\n@main struct JugueteApp: App { var body: some Scene { WindowGroup { Text(\"x\") } } }\nlet x: Int = \"no\"\n").unwrap();
        let e = ios(&run, &d, &p, &udid, None).await.unwrap_err().to_string();
        eprintln!("{e}");
        assert!(e.contains("Juguete/App.swift:3:14: Cannot convert value of type 'String' to specified type 'Int'"), "{e}");

        // de punta a punta con la herramienta: encender, construir (instala y lanza), ver el texto, la
        // compuerta viva, y una app que se cae al abrir
        std::fs::write(d.join("Juguete/App.swift"), "import SwiftUI\n@main struct JugueteApp: App { var body: some Scene { WindowGroup { VStack { Text(\"Hola ñandú\"); Button(\"Pulsa\") {} } } } }\n").unwrap();
        let t = std::time::Instant::now();
        let r = super::super::ejecutar(&json!({"action": "encender", "dispositivo": udid}), &d).await.unwrap();
        eprintln!("{r} ({:.1} s)", t.elapsed().as_secs_f32());
        let t = std::time::Instant::now();
        let r = super::super::ejecutar(&json!({"action": "construir", "dispositivo": udid}), &d).await.unwrap();
        eprintln!("construir+instalar+lanzar: {:.1} s\n{r}", t.elapsed().as_secs_f32());
        assert!(r.contains("space.gotv.juguete") && r.contains("Pulsa"), "{r}");
        let a = super::super::ejecutar(&json!({"action": "arbol", "dispositivo": udid, "todo": true}), &d).await.unwrap();
        assert!(a.contains("Hola ñandú"), "{a}");
        let t = std::time::Instant::now();
        let c = super::super::compuerta(&d).await.unwrap();
        eprintln!("compuerta: {:.1} s\n{}", t.elapsed().as_secs_f32(), c.texto);
        assert!(!c.fallo && c.texto.contains("viva"), "{c:?}");
        std::fs::write(d.join("Juguete/App.swift"), "import SwiftUI\n@main struct JugueteApp: App { init() { exit(3) }\n var body: some Scene { WindowGroup { Text(\"nunca\") } } }\n").unwrap();
        let c = super::super::compuerta(&d).await.unwrap();
        eprintln!("compuerta con la app rota:\n{}", c.texto);
        assert!(c.fallo && c.texto.contains("CERRÓ"), "{c:?}");
        let _ = super::super::ejecutar(&json!({"action": "apagar", "dispositivo": udid}), &d).await;
    }

    /// De verdad en Android: la app de juguete con el Gradle de `GOTV_PRUEBA_GRADLE` (por omisión el bajado a
    /// `~/Library/Application Support/gotv/gradle/gradle-9.9.1`), construir y leer un error de Java con su
    /// línea; luego, con el emulador `GOTV_PRUEBA_ANDROID_AVD` (o `gotv-prueba`): encender, construir
    /// (instala y lanza), ver el texto, la compuerta viva y una app que se cae al abrir (su porqué, primero).
    /// `cargo test -p gotv-core construir_android_de_verdad -- --ignored --nocapture`
    #[tokio::test]
    #[ignore]
    async fn construir_android_de_verdad() {
        let casa = std::path::PathBuf::from(std::env::var("HOME").unwrap());
        let gradle = std::env::var("GOTV_PRUEBA_GRADLE").map(std::path::PathBuf::from).unwrap_or_else(|_| casa.join("Library/Application Support/gotv/gradle/gradle-9.9.1/bin/gradle"));
        assert!(gradle.is_file(), "falta Gradle en {}", gradle.display());
        let avd = std::env::var("GOTV_PRUEBA_ANDROID_AVD").unwrap_or_else(|_| "gotv-prueba".into());
        let d = crate::CarpetaPrueba::nueva("gotv-construir-android-real");
        let bien = "import android.app.Activity; import android.os.Bundle; import android.widget.*;\npublic class MainActivity extends Activity {\n  @Override protected void onCreate(Bundle b) {\n    super.onCreate(b);\n    LinearLayout l = new LinearLayout(this); l.setOrientation(LinearLayout.VERTICAL);\n    TextView t = new TextView(this); t.setText(\"Hola ñandú\");\n    Button p = new Button(this); p.setText(\"Pulsa\");\n    l.addView(t); l.addView(p); setContentView(l);\n  }\n}\n";
        crear_juguete_android(&d, "space.gotv.juguete", bien).unwrap();
        // el envoltorio del proyecto apunta al Gradle bajado
        let gw = d.join("gradlew");
        std::fs::write(&gw, format!("#!/bin/sh\nexec \"{}\" \"$@\"\n", gradle.display())).unwrap();
        std::fs::set_permissions(&gw, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        let p = detectar(&d, false).unwrap();
        let run = super::super::Sistema;
        let sdk = super::super::android::sdk();
        let c = android(&run, &d, &p, sdk.as_deref()).await.unwrap();
        eprintln!("construida en {:.1} s: {}", c.segundos, c.app.display());
        assert!(c.app.extension().is_some_and(|e| e == "apk"));
        let java = d.join("app/src/main/java/space/gotv/juguete/MainActivity.java");
        std::fs::write(&java, format!("package space.gotv.juguete;\n{}", bien.replace("t.setText(\"Hola ñandú\");", "t.setText(nombre);"))).unwrap();
        let e = android(&run, &d, &p, sdk.as_deref()).await.unwrap_err().to_string();
        eprintln!("{e}");
        assert!(e.contains("app/src/main/java/space/gotv/juguete/MainActivity.java:7:1: cannot find symbol"), "{e}");
        std::fs::write(&java, format!("package space.gotv.juguete;\n{bien}")).unwrap();

        let t = std::time::Instant::now();
        let r = super::super::ejecutar(&json!({"action": "encender", "dispositivo": avd}), &d).await.unwrap();
        eprintln!("{r} ({:.1} s)", t.elapsed().as_secs_f32());
        let serie = r.rsplit('(').next().unwrap().trim_end_matches(')').trim().to_string();
        assert!(serie.starts_with("emulator-"), "{r}");
        let t = std::time::Instant::now();
        let r = super::super::ejecutar(&json!({"action": "construir", "dispositivo": serie}), &d).await.unwrap();
        eprintln!("construir+instalar+lanzar: {:.1} s\n{r}", t.elapsed().as_secs_f32());
        assert!(r.contains("space.gotv.juguete") && r.to_lowercase().contains("pulsa"), "{r}");
        let a = super::super::ejecutar(&json!({"action": "arbol", "dispositivo": serie, "todo": true}), &d).await.unwrap();
        assert!(a.contains("Hola ñandú"), "{a}");
        let c = super::super::compuerta(&d).await.unwrap();
        eprintln!("compuerta:\n{}", c.texto);
        assert!(!c.fallo && c.texto.contains("viva"), "{c:?}");
        std::fs::write(&java, format!("package space.gotv.juguete;\n{}", bien.replace("super.onCreate(b);", "super.onCreate(b); if (b == null) throw new IllegalStateException(\"rota a propósito\");"))).unwrap();
        let c = super::super::compuerta(&d).await.unwrap();
        eprintln!("compuerta con la app rota:\n{}", c.texto);
        assert!(c.fallo && c.texto.contains("CERRÓ") && c.texto.contains("rota a propósito"), "{c:?}");
        let _ = super::super::ejecutar(&json!({"action": "apagar", "dispositivo": serie}), &d).await;
    }
}
