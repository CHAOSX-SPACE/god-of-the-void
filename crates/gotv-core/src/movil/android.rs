//! F18 · M5/M6 · Android: emulador y teléfono real por `adb` (binario del SDK, nivel 2 de L1).
//!
//! - Árbol: `uiautomator dump /dev/tty` con sus tres trampas (ver `arbol.rs`).
//! - Escribir: `input text` sólo aguanta ASCII (con «ñ» revienta: medido en la ficha). Lo demás, en el
//!   EMULADOR, va por su API gRPC: `setClipboard` + pegar (`KEYCODE_PASTE`). **La ficha decía `sendKey` con
//!   texto UTF-8, pero el propio `emulator_controller.proto` del SDK 37.2.13 avisa que `sendKey.text` sólo
//!   traduce bien ASCII imprimible y recomienda el portapapeles**: manda el `.proto`. En un teléfono real no
//!   hay esa API: sólo ASCII, y se dice.
//! - El paquete de un `.apk` sale de su `AndroidManifest.xml` binario (AXML), leído aquí (sin `aapt`).
use super::arbol::{self, Arbol};
use super::grpc::{self, Destino};
use super::pb::Msg;
use super::{Corredor, Dispositivo, Plataforma, Puente, Salida, Sistema, Tecla};
use anyhow::{anyhow, bail, Result};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

const CORTO: Duration = Duration::from_secs(30);
/// `KEYCODE_PASTE`.
const PEGAR: u32 = 279;
/// `ClipData.text` (emulator_controller.proto).
const CLIPDATA_TEXT: u32 = 1;

pub struct Android {
    pub run: Arc<dyn Corredor>,
    pub adb: String,
    pub emulador: String,
    /// Dónde deja el emulador sus `pid_<n>.ini` de descubrimiento (puerto gRPC y su ficha).
    pub descubrimiento: Vec<PathBuf>,
    /// En las pruebas: la API gRPC del emulador ya escuchando.
    pub grpc: Option<(Destino, Option<String>)>,
    /// Cada cuánto se pregunta si terminó de arrancar.
    pub paso_arranque: Duration,
}

/// La carpeta del SDK: `ANDROID_HOME`, `ANDROID_SDK_ROOT`, la de Android Studio, la que instala GOTV
/// (F18.I) o la de Homebrew.
pub fn sdk() -> Option<PathBuf> {
    let casa = PathBuf::from(std::env::var("HOME").unwrap_or_default());
    ["ANDROID_HOME", "ANDROID_SDK_ROOT"]
        .iter()
        .filter_map(|v| std::env::var_os(v).map(PathBuf::from))
        .chain([casa.join("Library/Android/sdk"), casa.join("Library/Application Support/gotv/android-sdk"), PathBuf::from("/opt/homebrew/share/android-commandlinetools")])
        .find(|p| p.join("platform-tools/adb").is_file())
}

/// La carpeta de los dispositivos virtuales (`ANDROID_AVD_HOME` o `~/.android/avd`).
pub fn carpeta_avd() -> PathBuf {
    std::env::var_os("ANDROID_AVD_HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".android/avd"))
}

/// Las imágenes del sistema instaladas en el SDK (`system-images;android-36;google_apis;arm64-v8a`), la más
/// nueva primero.
pub fn imagenes(sdk: &Path) -> Vec<String> {
    let mut v = Vec::new();
    let base = sdk.join("system-images");
    for api in std::fs::read_dir(&base).into_iter().flatten().flatten() {
        for tag in std::fs::read_dir(api.path()).into_iter().flatten().flatten() {
            for abi in std::fs::read_dir(tag.path()).into_iter().flatten().flatten() {
                if abi.path().join("system.img").is_file() {
                    v.push(format!("system-images;{};{};{}", api.file_name().to_string_lossy(), tag.file_name().to_string_lossy(), abi.file_name().to_string_lossy()));
                }
            }
        }
    }
    let nivel = |s: &str| s.split(';').nth(1).and_then(|a| a.trim_start_matches("android-").parse::<u32>().ok()).unwrap_or(0);
    // la más nueva; a igual nivel, google_apis antes que default (trae los servicios que casi toda app pide)
    v.sort_by_key(|s| (std::cmp::Reverse(nivel(s)), !s.contains(";google_apis;")));
    v
}

/// F18 · M7 · Crea un dispositivo virtual SIN `avdmanager` (Java): el `<nombre>.ini` y su `config.ini`
/// mínimo; el emulador completa lo demás al primer arranque (medido con `gotv-prueba`). Un Pixel 8 con
/// teclado físico (para escribir sin el teclado en pantalla). No pisa uno que exista.
pub fn crear_avd(avd: &Path, nombre: &str, sdk: &Path, imagen: &str) -> Result<PathBuf> {
    if nombre.is_empty() || !nombre.chars().all(|c| c.is_ascii_alphanumeric() || "._-".contains(c)) {
        bail!("{}", crate::tr!("nombre «{nombre}»: sólo letras, números, punto, guion y guion bajo", "name «{nombre}»: only letters, digits, dot, dash and underscore"));
    }
    let partes: Vec<&str> = imagen.split(';').collect();
    let [_, api, tag, abi] = partes[..] else { bail!("{}", crate::tr!("imagen «{imagen}»: system-images;android-NN;etiqueta;abi", "image «{imagen}»: system-images;android-NN;tag;abi")) };
    let rel = format!("system-images/{api}/{tag}/{abi}/");
    if !sdk.join(&rel).join("system.img").is_file() {
        bail!("{}", crate::tr!("la imagen {imagen} no está instalada en {}", "image {imagen} is not installed in {}", sdk.display()));
    }
    let dir = avd.join(format!("{nombre}.avd"));
    let ini = avd.join(format!("{nombre}.ini"));
    if dir.exists() || ini.exists() {
        bail!("{}", crate::tr!("ya hay un dispositivo «{nombre}»", "there is already a device «{nombre}»"));
    }
    std::fs::create_dir_all(&dir)?;
    std::fs::write(&ini, format!("avd.ini.encoding=UTF-8\npath={}\npath.rel=avd/{nombre}.avd\ntarget={api}\n", dir.display()))?;
    let cpu = if abi.starts_with("arm64") { "arm64" } else if abi.contains("x86_64") { "x86_64" } else { abi };
    let etiqueta = match tag {
        "google_apis" => "Google APIs",
        "google_apis_playstore" => "Google Play",
        _ => "Default Android System Image",
    };
    let config = [
        "avd.ini.encoding=UTF-8".to_string(),
        format!("AvdId={nombre}"),
        format!("avd.ini.displayname={nombre}"),
        format!("abi.type={abi}"),
        format!("hw.cpu.arch={cpu}"),
        "hw.cpu.ncore=4".into(),
        format!("image.sysdir.1={rel}"),
        format!("tag.id={tag}"),
        format!("tag.display={etiqueta}"),
        format!("target={api}"),
        format!("PlayStore.enabled={}", if tag.contains("playstore") { "yes" } else { "no" }),
        "hw.device.manufacturer=Google".into(),
        "hw.device.name=pixel_8".into(),
        "hw.lcd.width=1080".into(),
        "hw.lcd.height=2400".into(),
        "hw.lcd.density=420".into(),
        "hw.ramSize=2G".into(),
        "vm.heapSize=228M".into(),
        "disk.dataPartition.size=6G".into(),
        "hw.keyboard=yes".into(),
        "hw.gpu.enabled=yes".into(),
        "hw.gpu.mode=auto".into(),
        "hw.sdCard=no".into(),
        "showDeviceFrame=no".into(),
        "fastboot.forceColdBoot=no".into(),
    ];
    std::fs::write(dir.join("config.ini"), config.join("\n") + "\n")?;
    Ok(dir)
}

fn s(v: &[&str]) -> Vec<String> {
    v.iter().map(|x| x.to_string()).collect()
}

/// Escapa un texto ASCII para `input text` (el shell del dispositivo lo vuelve a partir).
pub fn escapar_input(t: &str) -> String {
    let mut o = String::new();
    for c in t.chars() {
        match c {
            ' ' => o.push_str("%s"),
            '\\' | '\'' | '"' | '`' | '(' | ')' | '<' | '>' | '|' | ';' | '&' | '*' | '~' | '$' | '!' | '#' | '?' | '[' | ']' | '{' | '}' => {
                o.push('\\');
                o.push(c);
            }
            _ => o.push(c),
        }
    }
    o
}

/// Las líneas de `adb devices -l`: (serie, estado, modelo).
pub fn leer_devices(t: &str) -> Vec<(String, String, String)> {
    t.lines()
        .skip_while(|l| !l.starts_with("List of devices"))
        .skip(1)
        .filter_map(|l| {
            let mut p = l.split_whitespace();
            let serie = p.next()?.to_string();
            let estado = p.next()?.to_string();
            let modelo = l.split_whitespace().find_map(|x| x.strip_prefix("model:")).unwrap_or("").replace('_', " ");
            Some((serie, estado, modelo))
        })
        .collect()
}

/// El puerto de consola de una serie `emulator-5554`.
fn puerto_de(serie: &str) -> Option<u16> {
    serie.strip_prefix("emulator-")?.parse().ok()
}

/// Del archivo de descubrimiento del emulador (`clave=valor`), (puerto gRPC, ficha) si es el de esa consola.
pub fn leer_descubrimiento(ini: &str, consola: u16) -> Option<(u16, Option<String>)> {
    let v = |k: &str| ini.lines().find_map(|l| l.trim().strip_prefix(k)?.strip_prefix('=').map(|x| x.trim().to_string()));
    if v("port.serial")?.parse::<u16>().ok()? != consola {
        return None;
    }
    Some((v("grpc.port")?.parse().ok()?, v("grpc.token").filter(|t| !t.is_empty())))
}

// --- el paquete de un .apk (AXML) ---

fn u16a(b: &[u8], i: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(i..i + 2)?.try_into().ok()?))
}
fn u32a(b: &[u8], i: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(i..i + 4)?.try_into().ok()?))
}

fn largo8(c: &[u8], p: usize) -> Option<(usize, usize)> {
    let a = *c.get(p)? as usize;
    Some(if a & 0x80 != 0 { (((a & 0x7f) << 8) | *c.get(p + 1)? as usize, p + 2) } else { (a, p + 1) })
}

fn largo16(c: &[u8], p: usize) -> Option<(usize, usize)> {
    let a = u16a(c, p)? as usize;
    Some(if a & 0x8000 != 0 { (((a & 0x7fff) << 16) | u16a(c, p + 2)? as usize, p + 4) } else { (a, p + 2) })
}

/// El bloque de cadenas (UTF-8 o UTF-16).
fn cadenas(c: &[u8]) -> Option<Vec<String>> {
    let cab = u16a(c, 2)? as usize;
    let n = u32a(c, 8)? as usize;
    let utf8 = u32a(c, 16)? & 0x100 != 0;
    let ini = u32a(c, 20)? as usize;
    (0..n)
        .map(|k| {
            let p = ini + u32a(c, cab + k * 4)? as usize;
            if utf8 {
                let (_, p) = largo8(c, p)?;
                let (n8, p) = largo8(c, p)?;
                Some(String::from_utf8_lossy(c.get(p..p + n8)?).into_owned())
            } else {
                let (n16, p) = largo16(c, p)?;
                let u: Vec<u16> = (0..n16).map(|j| u16a(c, p + 2 * j)).collect::<Option<_>>()?;
                Some(String::from_utf16_lossy(&u))
            }
        })
        .collect()
}

/// El atributo `package` del elemento `manifest` de un AndroidManifest.xml BINARIO.
pub fn paquete_axml(b: &[u8]) -> Option<String> {
    if u16a(b, 0)? != 0x0003 {
        return None;
    }
    let mut i = u16a(b, 2)? as usize;
    let mut pool: Vec<String> = Vec::new();
    while i + 8 <= b.len() {
        let tipo = u16a(b, i)?;
        let cab = u16a(b, i + 2)? as usize;
        let tam = u32a(b, i + 4)? as usize;
        if tam < 8 {
            return None;
        }
        match tipo {
            0x0001 => pool = cadenas(b.get(i..i + tam)?)?,
            0x0102 => {
                let ext = i + cab;
                if pool.get(u32a(b, ext + 4)? as usize).map(String::as_str) == Some("manifest") {
                    let (ini, paso, n) = (u16a(b, ext + 8)? as usize, u16a(b, ext + 10)? as usize, u16a(b, ext + 12)? as usize);
                    for k in 0..n {
                        let a = ext + ini + k * paso;
                        if pool.get(u32a(b, a + 4)? as usize).map(String::as_str) == Some("package") {
                            let crudo = u32a(b, a + 8)?;
                            let idx = if crudo != u32::MAX {
                                crudo
                            } else if *b.get(a + 15)? == 0x03 {
                                u32a(b, a + 16)?
                            } else {
                                return None;
                            };
                            return pool.get(idx as usize).cloned();
                        }
                    }
                    return None;
                }
            }
            _ => {}
        }
        i += tam;
    }
    None
}

/// El paquete de un `.apk`.
pub fn paquete_apk(apk: &Path) -> Result<String> {
    use std::io::Read;
    let mut z = zip::ZipArchive::new(std::fs::File::open(apk)?).map_err(|e| anyhow!("{}: {e}", apk.display()))?;
    let mut b = Vec::new();
    z.by_name("AndroidManifest.xml").map_err(|e| anyhow!("{}: AndroidManifest.xml: {e}", apk.display()))?.read_to_end(&mut b)?;
    paquete_axml(&b).ok_or_else(|| anyhow!("{}", crate::tr!("{}: no hallé el paquete en su AndroidManifest.xml", "{}: could not find the package in its AndroidManifest.xml", apk.display())))
}

impl Android {
    pub fn sistema() -> Self {
        let raiz = sdk();
        let bin = |sub: &str, nombre: &str| raiz.as_ref().map(|r| r.join(sub).join(nombre)).filter(|p| p.is_file()).map(|p| p.display().to_string()).unwrap_or_else(|| nombre.to_string());
        let casa = PathBuf::from(std::env::var("HOME").unwrap_or_default());
        Android {
            run: Arc::new(Sistema),
            adb: bin("platform-tools", "adb"),
            emulador: bin("emulator", "emulator"),
            // macOS: el emulador escribe en <temporal del usuario>/avd/running (sin medir con uno encendido)
            descubrimiento: vec![casa.join("Library/Caches/TemporaryItems/avd/running"), std::env::temp_dir().join("avd/running")],
            grpc: None,
            paso_arranque: Duration::from_secs(2),
        }
    }

    async fn adb(&self, serie: &str, args: &[&str], tope: Duration) -> Result<Salida> {
        let mut a = s(&["-s", serie]);
        a.extend(s(args));
        self.run.correr(&self.adb, &a, tope).await
    }

    async fn shell(&self, serie: &str, args: &[&str]) -> Result<String> {
        let mut a = vec!["shell"];
        a.extend(args);
        let o = self.adb(serie, &a, CORTO).await?.exigir(&format!("adb shell {}", args.first().unwrap_or(&"")))?;
        Ok(o.texto())
    }

    async fn pid(&self, serie: &str, pkg: &str) -> Option<u32> {
        let o = self.adb(serie, &["shell", "pidof", pkg], CORTO).await.ok()?;
        o.texto().split_whitespace().next()?.parse().ok()
    }

    /// La API gRPC del emulador de esa serie (puerto y ficha, de su archivo de descubrimiento).
    fn grpc_de(&self, serie: &str) -> Option<(Destino, Option<String>)> {
        if let Some(g) = &self.grpc {
            return Some(g.clone());
        }
        let consola = puerto_de(serie)?;
        for d in &self.descubrimiento {
            for e in std::fs::read_dir(d).into_iter().flatten().flatten() {
                if !e.file_name().to_string_lossy().starts_with("pid_") {
                    continue;
                }
                if let Some((p, t)) = std::fs::read_to_string(e.path()).ok().and_then(|ini| leer_descubrimiento(&ini, consola)) {
                    return Some((Destino::Tcp(p), t));
                }
            }
        }
        None
    }
}

#[async_trait::async_trait]
impl Puente for Android {
    async fn construir(&self, _id: &str, root: &Path, _args: &Value) -> Result<super::construir::Construida> {
        let p = super::construir::detectar(root, false).ok_or_else(|| anyhow!("{}", crate::i18n::t("no hay proyecto de Android aquí (gradlew en la raíz o en android/, o Flutter)", "there is no Android project here (gradlew at the root or in android/, or Flutter)")))?;
        super::construir::android(&*self.run, root, &p, sdk().as_deref()).await
    }

    /// Medido 2026-11-17 (emulador, Android 16): una app que se cae al abrir deja el cuadro del sistema
    /// «App info / Close app» y su proceso SIGUE vivo mientras está abierto, así que `pidof` sola decía
    /// «viva». El búfer de cierres lo dice sin dudas: `Process: <paquete>, PID: <ese pid>`.
    /// `pm path`: con la app dice `package:…`; sin ella sale con error y en blanco (lo demás no afirma nada).
    async fn instalada(&self, serie: &str, pkg: &str) -> Option<bool> {
        let o = self.adb(serie, &["shell", "pm", "path", pkg], CORTO).await.ok()?;
        let t = o.texto();
        if t.contains("package:") {
            Some(true)
        } else if !o.ok && t.trim().is_empty() {
            Some(false)
        } else {
            None
        }
    }

    async fn vive(&self, serie: &str, pkg: &str) -> Result<bool> {
        let Some(pid) = self.pid(serie, pkg).await else { return Ok(false) };
        let c = self.adb(serie, &["logcat", "-d", "-b", "crash", "-t", "300"], CORTO).await?.texto();
        Ok(!c.contains(&format!("Process: {pkg}, PID: {pid}")))
    }

    async fn listar(&self) -> Result<Vec<Dispositivo>> {
        let o = self.run.correr(&self.adb, &s(&["devices", "-l"]), CORTO).await?.exigir("adb devices")?;
        let mut out = Vec::new();
        let mut avd_encendidos = Vec::new();
        for (serie, estado, modelo) in leer_devices(&o.texto()) {
            let fisico = !serie.starts_with("emulator-");
            let encendido = estado == "device";
            let mut nombre = modelo;
            if !fisico && encendido {
                // `emu avd name` → el nombre del dispositivo virtual
                if let Ok(a) = self.adb(&serie, &["emu", "avd", "name"], CORTO).await {
                    if let Some(n) = a.texto().lines().next().map(str::trim).filter(|n| !n.is_empty() && *n != "OK") {
                        nombre = n.to_string();
                        avd_encendidos.push(nombre.clone());
                    }
                }
            }
            if estado == "unauthorized" {
                nombre = crate::tr!("{nombre} (sin autorizar: acepta la depuración en el teléfono)", "{nombre} (unauthorized: accept debugging on the phone)");
            }
            let sistema = if encendido { self.shell(&serie, &["getprop", "ro.build.version.release"]).await.map(|v| format!("Android {}", v.trim())).unwrap_or_default() } else { String::new() };
            out.push(Dispositivo { id: serie, nombre, plataforma: Plataforma::Android, sistema, encendido, fisico });
        }
        // los dispositivos virtuales apagados (si el emulador está)
        if let Ok(o) = self.run.correr(&self.emulador, &s(&["-list-avds"]), CORTO).await {
            for avd in o.texto().lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with("INFO") && !l.contains(' ')) {
                if !avd_encendidos.iter().any(|a| a == avd) {
                    out.push(Dispositivo { id: avd.into(), nombre: avd.into(), plataforma: Plataforma::Android, sistema: String::new(), encendido: false, fisico: false });
                }
            }
        }
        Ok(out)
    }

    /// `emulator -avd` sin ventana (por omisión), con su API gRPC protegida por ficha, en el primer puerto
    /// libre; listo cuando `sys.boot_completed` da 1 (44 s medidos en la ficha).
    async fn encender(&self, avd: &str, ventana: bool) -> Result<String> {
        let usados: Vec<u16> = self.run.correr(&self.adb, &s(&["devices"]), CORTO).await.map(|o| leer_devices(&o.texto()).iter().filter_map(|d| puerto_de(&d.0)).collect()).unwrap_or_default();
        let consola = (5554u16..=5682).step_by(2).find(|p| !usados.contains(p)).ok_or_else(|| anyhow!("{}", crate::i18n::t("no queda puerto libre para otro emulador", "no free port left for another emulator")))?;
        let grpc_puerto = 8554 + (consola - 5554);
        let mut args = s(&["-avd", avd, "-port", &consola.to_string(), "-no-audio", "-no-boot-anim", "-grpc", &grpc_puerto.to_string(), "-grpc-use-token"]);
        if !ventana {
            args.push("-no-window".into());
        }
        let registro = std::env::temp_dir().join(format!("gotv-emulador-{consola}.log"));
        self.run.fondo(&self.emulador, &args, &registro)?;
        let serie = format!("emulator-{consola}");
        let t0 = std::time::Instant::now();
        loop {
            if let Ok(o) = self.adb(&serie, &["shell", "getprop", "sys.boot_completed"], Duration::from_secs(10)).await {
                if o.ok && o.texto().trim() == "1" {
                    return Ok(serie);
                }
            }
            if t0.elapsed() > Duration::from_secs(300) {
                bail!("{}", crate::tr!("el emulador «{avd}» no terminó de arrancar en 5 min (registro: {})", "emulator «{avd}» did not finish booting in 5 min (log: {})", registro.display()));
            }
            tokio::time::sleep(self.paso_arranque).await;
        }
    }

    async fn apagar(&self, serie: &str) -> Result<()> {
        if !serie.starts_with("emulator-") {
            bail!("{}", crate::i18n::t("un teléfono real no se apaga desde GOTV", "a real phone is not turned off from GOTV"));
        }
        self.adb(serie, &["emu", "kill"], CORTO).await?.exigir("adb emu kill")?;
        // `emu kill` vuelve al instante y el emulador sigue vivo unos segundos (medido 2026-11-17): se espera a
        // que desaparezca de `adb devices` para que «apagado» sea verdad (tope 30 s)
        for _ in 0..60 {
            let sigue = self.listar().await.map(|l| l.iter().any(|d| d.id == serie && d.encendido)).unwrap_or(false);
            if !sigue {
                return Ok(());
            }
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }
        bail!("{}", crate::tr!("{serie} sigue encendido 30 s después de pedir que se apague", "{serie} is still on 30 s after asking it to turn off"))
    }

    async fn instalar(&self, serie: &str, apk: &Path) -> Result<String> {
        if apk.extension().is_none_or(|e| e != "apk") {
            bail!("{}", crate::tr!("{} no es un .apk", "{} is not an .apk", apk.display()));
        }
        let pkg = paquete_apk(apk)?;
        let o = self.adb(serie, &["install", "-r", &apk.display().to_string()], Duration::from_secs(300)).await?.exigir("adb install")?;
        if !o.texto().contains("Success") {
            bail!("adb install: {}", o.texto().trim());
        }
        Ok(pkg)
    }

    async fn lanzar(&self, serie: &str, pkg: &str) -> Result<u32> {
        // la actividad de inicio y `am start -W` (espera a que abra). Medido 2026-11-17 (Android 16,
        // gotv-prueba): `monkey -p` ABORTA con código 251 («SYS_KEYS has no physical keys»); `am start -W`,
        // 1,6 s. `monkey --pct-syskeys 0` queda de respaldo si no se puede resolver la actividad.
        let r = self.shell(serie, &["cmd", "package", "resolve-activity", "--brief", "-c", "android.intent.category.LAUNCHER", pkg]).await.unwrap_or_default();
        let actividad = r.lines().map(str::trim).rfind(|l| l.starts_with(&format!("{pkg}/"))).map(String::from);
        let o = match &actividad {
            Some(a) => self.adb(serie, &["shell", "am", "start", "-W", "-n", a], Duration::from_secs(60)).await?.texto(),
            None => self.shell(serie, &["monkey", "--pct-syskeys", "0", "-p", pkg, "-c", "android.intent.category.LAUNCHER", "1"]).await?,
        };
        if o.contains("No activities found") || o.contains("monkey aborted") || o.contains("Error: Activity class") {
            bail!("{}", crate::tr!("{pkg} no tiene actividad de inicio (¿está instalada?)", "{pkg} has no launcher activity (is it installed?)"));
        }
        for _ in 0..10 {
            if let Some(p) = self.pid(serie, pkg).await {
                return Ok(p);
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        bail!("{}", crate::tr!("{pkg} se lanzó pero no está corriendo (¿se cerró al abrir? mira `registro`)", "{pkg} was launched but is not running (did it crash on start? look at `registro`)"))
    }

    async fn cerrar(&self, serie: &str, pkg: &str) -> Result<()> {
        self.shell(serie, &["am", "force-stop", pkg]).await?;
        Ok(())
    }

    async fn arbol(&self, serie: &str, todo: bool) -> Result<Arbol> {
        let o = self.adb(serie, &["exec-out", "uiautomator", "dump", "/dev/tty"], CORTO).await?.exigir("uiautomator dump")?;
        arbol::de_android(&o.texto(), todo)
    }

    async fn captura(&self, serie: &str, a: &Path) -> Result<()> {
        let o = self.adb(serie, &["exec-out", "screencap", "-p"], CORTO).await?.exigir("screencap")?;
        if !o.out.starts_with(b"\x89PNG") {
            bail!("{}", crate::i18n::t("screencap no devolvió un PNG", "screencap did not return a PNG"));
        }
        std::fs::write(a, &o.out)?;
        Ok(())
    }

    async fn tocar(&self, serie: &str, x: i32, y: i32) -> Result<()> {
        self.shell(serie, &["input", "tap", &x.to_string(), &y.to_string()]).await?;
        Ok(())
    }

    async fn deslizar(&self, serie: &str, de: (i32, i32), a: (i32, i32), ms: u32) -> Result<()> {
        self.shell(serie, &["input", "swipe", &de.0.to_string(), &de.1.to_string(), &a.0.to_string(), &a.1.to_string(), &ms.to_string()]).await?;
        Ok(())
    }

    async fn escribir(&self, serie: &str, en: Option<(i32, i32)>, texto: &str) -> Result<String> {
        if !texto.is_ascii() && self.grpc_de(serie).is_none() {
            bail!(
                "{}",
                if serie.starts_with("emulator-") {
                    crate::i18n::t("este emulador no expone su API gRPC (enciéndelo desde GOTV): sin ella, sólo se escribe ASCII (`input text` revienta con «ñ»)", "this emulator does not expose its gRPC API (turn it on from GOTV): without it only ASCII can be typed (`input text` crashes on «ñ»)")
                } else {
                    crate::i18n::t("en un teléfono real sólo se escribe ASCII (`input text` revienta con «ñ» y no hay API del emulador): escribe sin tildes o pega el texto a mano", "on a real phone only ASCII can be typed (`input text` crashes on «ñ» and there is no emulator API): type without accents or paste the text by hand")
                }
            );
        }
        if let Some((x, y)) = en {
            self.tocar(serie, x, y).await?;
        }
        if texto.is_ascii() {
            for (i, l) in texto.split('\n').enumerate() {
                if i > 0 {
                    self.shell(serie, &["input", "keyevent", "66"]).await?;
                }
                if !l.is_empty() {
                    self.shell(serie, &["input", "text", &escapar_input(l)]).await?;
                }
            }
            return Ok(String::new());
        }
        let (d, ficha) = self.grpc_de(serie).unwrap_or((Destino::Tcp(0), None));
        let clip = Msg::new().texto(CLIPDATA_TEXT, texto).into_bytes();
        grpc::unario(&d, "/android.emulation.control.EmulatorController/setClipboard", clip, ficha.as_deref(), CORTO).await?;
        self.shell(serie, &["input", "keyevent", &PEGAR.to_string()]).await?;
        Ok(crate::tr!("✓ escrito «{texto}» (por el portapapeles del emulador)", "✓ typed «{texto}» (through the emulator clipboard)"))
    }

    async fn esperar(&self, serie: &str, texto: &str, tope_ms: u32) -> Result<bool> {
        let t0 = std::time::Instant::now();
        loop {
            if self.arbol(serie, true).await.is_ok_and(|a| a.contiene(texto)) {
                return Ok(true);
            }
            if t0.elapsed() >= Duration::from_millis(tope_ms as u64) {
                return Ok(false);
            }
            tokio::time::sleep(Duration::from_millis(300)).await;
        }
    }

    async fn tecla(&self, serie: &str, t: Tecla) -> Result<()> {
        let k = match t {
            Tecla::Inicio => 3,
            Tecla::Atras => 4,
            Tecla::Bloquear => 26,
            Tecla::Apps => 187,
            Tecla::Intro => 66,
            Tecla::Borrar => 67,
            Tecla::Tab => 61,
            Tecla::Arriba => 19,
            Tecla::Abajo => 20,
            Tecla::Izquierda => 21,
            Tecla::Derecha => 22,
            Tecla::SubirVolumen => 24,
            Tecla::BajarVolumen => 25,
        };
        self.shell(serie, &["input", "keyevent", &k.to_string()]).await?;
        Ok(())
    }

    async fn registro(&self, serie: &str, app: Option<&str>, lineas: usize) -> Result<String> {
        let n = lineas.to_string();
        let pid = match app {
            Some(a) => self.pid(serie, a).await,
            None => None,
        };
        let mut args = vec!["logcat", "-d", "-t", n.as_str()];
        let filtro;
        if let Some(p) = pid {
            filtro = format!("--pid={p}");
            args.push(filtro.as_str());
        }
        let o = self.adb(serie, &args, CORTO).await?.exigir("logcat")?;
        let mut out = String::new();
        if let (Some(a), None) = (app, pid) {
            out.push_str(&crate::tr!("({a} no está corriendo: van las últimas líneas de todo el sistema)\n", "({a} is not running: these are the latest lines of the whole system)\n"));
        }
        out.push_str(o.texto().trim_end());
        let c = self.adb(serie, &["logcat", "-d", "-b", "crash", "-t", "50"], CORTO).await?;
        let cierres = c.texto();
        let cierres: Vec<&str> = cierres.lines().filter(|l| !l.starts_with("---------") && app.is_none_or(|a| cierres.contains(a))).collect();
        if !cierres.is_empty() {
            out.push_str(&crate::tr!("\n💥 cierres inesperados (logcat -b crash):\n{}", "\n💥 crashes (logcat -b crash):\n{}", cierres.join("\n")));
        }
        Ok(out)
    }

    async fn ajustar(&self, serie: &str, args: &Value) -> Result<String> {
        let t = |k: &str| args.get(k).and_then(|v| v.as_str()).map(str::trim).filter(|v| !v.is_empty());
        let mut hecho = Vec::new();
        if let Some(a) = t("apariencia") {
            let m = match a {
                "oscuro" | "dark" => "yes",
                "claro" | "light" => "no",
                _ => bail!("{}", crate::tr!("apariencia «{a}»: claro u oscuro", "apariencia «{a}»: claro (light) or oscuro (dark)")),
            };
            self.shell(serie, &["cmd", "uimode", "night", m]).await?;
            hecho.push(format!("uimode night {m}"));
        }
        if let Some(l) = t("letra") {
            let f: f32 = l.parse().map_err(|_| anyhow!("{}", crate::i18n::t("letra en Android: la escala (0.85, 1, 1.15, 1.3, 2)", "letra on Android: the scale (0.85, 1, 1.15, 1.3, 2)")))?;
            self.shell(serie, &["settings", "put", "system", "font_scale", &f.to_string()]).await?;
            hecho.push(format!("font_scale {f}"));
        }
        if let Some(u) = t("ubicacion") {
            if !serie.starts_with("emulator-") {
                bail!("{}", crate::i18n::t("la ubicación sólo se finge en el emulador", "location can only be faked on the emulator"));
            }
            let (la, lo) = u.split_once(',').ok_or_else(|| anyhow!("{}", crate::i18n::t("ubicacion: «latitud,longitud»", "ubicacion: «latitude,longitude»")))?;
            let (la, lo): (f64, f64) = (la.trim().parse()?, lo.trim().parse()?);
            // la consola del emulador pide LONGITUD primero
            self.adb(serie, &["emu", "geo", "fix", &lo.to_string(), &la.to_string()], CORTO).await?.exigir("emu geo fix")?;
            hecho.push(format!("geo fix {lo} {la}"));
        }
        if let Some(url) = t("enlace") {
            self.shell(serie, &["am", "start", "-a", "android.intent.action.VIEW", "-d", url]).await?;
            hecho.push(format!("VIEW {url}"));
        }
        if let Some(p) = t("permiso") {
            let app = t("app").ok_or_else(|| anyhow!("{}", crate::i18n::t("permiso: falta `app`", "permiso: missing `app`")))?;
            let p = if p.contains('.') { p.to_string() } else { format!("android.permission.{}", p.to_uppercase()) };
            let verbo = if args.get("conceder").and_then(|v| v.as_bool()).unwrap_or(true) { "grant" } else { "revoke" };
            self.shell(serie, &["pm", verbo, app, &p]).await?;
            hecho.push(format!("pm {verbo} {app} {p}"));
        }
        if hecho.is_empty() {
            bail!("{}", crate::i18n::t("ajustar: apariencia, letra, ubicacion, enlace o permiso (+app, conceder)", "ajustar: apariencia, letra, ubicacion, enlace or permiso (+app, conceder)"));
        }
        Ok(format!("✓ {}", hecho.join(" · ")))
    }

    async fn borrar_datos(&self, serie: &str, app: Option<&str>) -> Result<String> {
        let Some(a) = app else {
            bail!("{}", crate::i18n::t("en Android sólo se borran los datos de UNA app (`app`): borrar el dispositivo entero no se ofrece", "on Android only ONE app's data can be cleared (`app`): wiping the whole device is not offered"));
        };
        let o = self.shell(serie, &["pm", "clear", a]).await?;
        if !o.contains("Success") {
            bail!("pm clear: {}", o.trim());
        }
        Ok(crate::tr!("✓ datos de {a} borrados", "✓ {a} data cleared"))
    }

    /// `adb` no sabe si la app se aquietó: una pausa corta.
    async fn aquietar(&self, _: &str) {
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::movil::grpc::falso;
    use crate::movil::pb::leer;
    use crate::movil::tests::Falso;
    use crate::movil::{ejecutar_con, Puentes};
    use serde_json::json;
    use std::sync::Mutex;

    fn android(f: Arc<Falso>) -> Android {
        Android { run: f, adb: "adb".into(), emulador: "emulator".into(), descubrimiento: vec![], grpc: None, paso_arranque: Duration::from_millis(1) }
    }

    const DEVICES: &str = "* daemon not running; starting now at tcp:5037\n* daemon started successfully\nList of devices attached\nemulator-5554          device product:sdk_gphone64_arm64 model:sdk_gphone64_arm64 device:emu64a transport_id:1\nR58M12ABCDE            unauthorized usb:1-1 transport_id:2\n\n";

    #[test]
    fn devices_escape_y_descubrimiento() {
        let d = leer_devices(DEVICES);
        assert_eq!(d, vec![("emulator-5554".into(), "device".into(), "sdk gphone64 arm64".into()), ("R58M12ABCDE".into(), "unauthorized".into(), String::new())]);
        assert_eq!(escapar_input("Hola mundo (1) & 'x'?"), "Hola%smundo%s\\(1\\)%s\\&%s\\'x\\'\\?");
        let ini = "port.serial=5556\nport.adb=5557\navd.name=gotv-prueba\ngrpc.port=8556\ngrpc.token=abc123\n";
        assert_eq!(leer_descubrimiento(ini, 5556), Some((8556, Some("abc123".into()))));
        assert_eq!(leer_descubrimiento(ini, 5554), None);
    }

    /// Un AndroidManifest.xml binario mínimo (cadenas en UTF-8), hecho aquí con el formato de AXML.
    fn axml(paquete: &str) -> Vec<u8> {
        let cad = ["manifest", "package", paquete, "versionCode"];
        let mut datos = Vec::new();
        let mut offs = Vec::new();
        for c in cad {
            offs.push(datos.len() as u32);
            datos.extend([c.chars().count() as u8, c.len() as u8]);
            datos.extend(c.as_bytes());
            datos.push(0);
        }
        while datos.len() % 4 != 0 {
            datos.push(0);
        }
        let cab = 28u16;
        let ini = cab as u32 + 4 * cad.len() as u32;
        let mut pool = Vec::new();
        pool.extend(1u16.to_le_bytes());
        pool.extend(cab.to_le_bytes());
        pool.extend((ini + datos.len() as u32).to_le_bytes());
        pool.extend((cad.len() as u32).to_le_bytes());
        pool.extend(0u32.to_le_bytes());
        pool.extend(0x100u32.to_le_bytes());
        pool.extend(ini.to_le_bytes());
        pool.extend(0u32.to_le_bytes());
        offs.iter().for_each(|o| pool.extend(o.to_le_bytes()));
        pool.extend(&datos);
        // <manifest versionCode=… package=…>: dos atributos (el paquete, el segundo)
        let atributo = |nombre: u32, crudo: u32, tipo: u8, dato: u32| {
            let mut a = Vec::new();
            a.extend(u32::MAX.to_le_bytes());
            a.extend(nombre.to_le_bytes());
            a.extend(crudo.to_le_bytes());
            a.extend(8u16.to_le_bytes());
            a.push(0);
            a.push(tipo);
            a.extend(dato.to_le_bytes());
            a
        };
        let mut el = Vec::new();
        el.extend(0x0102u16.to_le_bytes());
        el.extend(16u16.to_le_bytes());
        el.extend((16u32 + 20 + 40).to_le_bytes());
        el.extend(1u32.to_le_bytes());
        el.extend(u32::MAX.to_le_bytes());
        el.extend(u32::MAX.to_le_bytes());
        el.extend(0u32.to_le_bytes());
        el.extend(20u16.to_le_bytes());
        el.extend(20u16.to_le_bytes());
        el.extend(2u16.to_le_bytes());
        el.extend([0u8; 6]);
        el.extend(atributo(3, u32::MAX, 0x10, 7));
        el.extend(atributo(1, 2, 0x03, 2));
        let mut b = Vec::new();
        b.extend(3u16.to_le_bytes());
        b.extend(8u16.to_le_bytes());
        b.extend((8 + pool.len() as u32 + el.len() as u32).to_le_bytes());
        b.extend(pool);
        b.extend(el);
        b
    }

    #[test]
    fn el_paquete_del_manifiesto_binario() {
        assert_eq!(paquete_axml(&axml("com.ejemplo.notas")).as_deref(), Some("com.ejemplo.notas"));
        assert_eq!(paquete_axml(b"<manifest package=\"x\"/>"), None, "un XML de texto no es AXML");
        assert_eq!(paquete_axml(&axml("x")[..40]), None, "cortado: sin pánico");
        // un .apk de verdad con el manifiesto dentro
        let d = crate::CarpetaPrueba::nueva("gotv-movil-apk");
        let apk = d.join("notas.apk");
        let mut z = zip::ZipWriter::new(std::fs::File::create(&apk).unwrap());
        z.start_file("AndroidManifest.xml", zip::write::SimpleFileOptions::default()).unwrap();
        std::io::Write::write_all(&mut z, &axml("com.ejemplo.notas")).unwrap();
        z.finish().unwrap();
        assert_eq!(paquete_apk(&apk).unwrap(), "com.ejemplo.notas");
        // medido con un .apk REAL del SDK de esta Mac (cadenas UTF-16), si está
        let real = Path::new("/opt/homebrew/share/android-commandlinetools/emulator/resources/skins/android-36/user/pixel_10/EmulationPixel10Overlay.apk");
        if real.is_file() {
            assert_eq!(paquete_apk(real).unwrap(), "com.android.internal.emulation.pixel_10");
        }
    }

    /// El campo y la llamada que se usan casan con el `emulator_controller.proto` del SDK INSTALADO (si lo
    /// hay; en esta Mac, el del emulador 37.2.13). El `.proto` no se copia al repo.
    #[test]
    fn campos_casan_con_el_proto_del_emulador() {
        let Some(p) = sdk().map(|s| s.join("emulator/lib/emulator_controller.proto")).and_then(|r| std::fs::read_to_string(r).ok()) else { return };
        assert_eq!(crate::movil::idb::tests::numero(&p, "ClipData", "text"), Some(CLIPDATA_TEXT));
        assert!(p.contains("package android.emulation.control;") && p.contains("service EmulatorController") && p.contains("rpc setClipboard(ClipData)"));
    }

    /// Un servidor gRPC del emulador FALSO que anota lo que llega a `setClipboard`.
    async fn emulador_falso() -> (Destino, Arc<Mutex<Vec<(String, String, Option<String>)>>>) {
        let visto: Arc<Mutex<Vec<(String, String, Option<String>)>>> = Default::default();
        let v2 = visto.clone();
        let m: falso::Manejador = Arc::new(move |ruta, msgs, cab| {
            let t = msgs.first().and_then(|b| leer(b).ok()).and_then(|l| l.texto(CLIPDATA_TEXT)).unwrap_or_default();
            v2.lock().unwrap().push((ruta.to_string(), t, cab.get("authorization").map(|v| v.to_str().unwrap().to_string())));
            Ok(vec![vec![]])
        });
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let p = l.local_addr().unwrap().port();
        tokio::spawn(falso::servir_tcp(l, m));
        (Destino::Tcp(p), visto)
    }

    /// «ñ» en el emulador: por gRPC (portapapeles con su ficha) + pegar; JAMÁS por `input text`. ASCII,
    /// por `input text` escapado. En un teléfono real, «ñ» se niega sin tocar nada.
    #[tokio::test]
    async fn escribir_no_ascii_va_por_grpc_y_nunca_por_input_text() {
        let (d, visto) = emulador_falso().await;
        let f = Falso::con(&[]);
        let mut a = android(f.clone());
        a.grpc = Some((d, Some("ficha-1".into())));
        let r = a.escribir("emulator-5554", Some((540, 313)), "mañana ñandú 日本").await.unwrap();
        assert!(r.contains("portapapeles") || r.contains("clipboard"), "{r}");
        assert_eq!(visto.lock().unwrap().clone(), vec![("/android.emulation.control.EmulatorController/setClipboard".to_string(), "mañana ñandú 日本".to_string(), Some("Bearer ficha-1".to_string()))]);
        a.escribir("emulator-5554", None, "Hola mundo").await.unwrap();
        let h = f.hechas();
        assert_eq!(h, vec!["adb -s emulator-5554 shell input tap 540 313", "adb -s emulator-5554 shell input keyevent 279", "adb -s emulator-5554 shell input text Hola%smundo"]);
        // teléfono real: «ñ» no se escribe, y no se toca la pantalla
        let f2 = Falso::con(&[]);
        let real = android(f2.clone());
        let e = real.escribir("R58M12ABCDE", Some((1, 1)), "año").await.unwrap_err();
        assert!(e.to_string().contains("ASCII"), "{e}");
        assert!(f2.hechas().is_empty());
        // emulador sin API descubierta: tampoco
        assert!(android(Falso::con(&[])).escribir("emulator-5554", None, "ñ").await.is_err());
    }

    /// Viva = su proceso existe Y no está en el búfer de cierres con ESE pid (el cuadro «App info / Close
    /// app» lo mantiene vivo un rato). Un cierre viejo de otro pid no cuenta.
    #[tokio::test]
    async fn viva_no_es_solo_tener_proceso() {
        let f = Falso::con(&[("pidof space.gotv.juguete", "2945\n")]);
        let a = Android { run: f.clone(), adb: "adb".into(), emulador: "emulator".into(), descubrimiento: vec![], grpc: None, paso_arranque: Duration::from_millis(1) };
        f.regla("-b crash", true, b"E AndroidRuntime: FATAL EXCEPTION: main\nE AndroidRuntime: Process: space.gotv.juguete, PID: 2641\n");
        assert!(a.vive("emulator-5554", "space.gotv.juguete").await.unwrap(), "el cierre de otro pid (corrida anterior) no cuenta");
        f.regla("-b crash", true, b"E AndroidRuntime: FATAL EXCEPTION: main\nE AndroidRuntime: Process: space.gotv.juguete, PID: 2945\n");
        assert!(!a.vive("emulator-5554", "space.gotv.juguete").await.unwrap(), "se cayó: el cuadro del sistema lo mantiene vivo, pero no vale");
        f.regla("pidof space.gotv.juguete", false, b"");
        assert!(!a.vive("emulator-5554", "space.gotv.juguete").await.unwrap());
    }

    /// F18 · M7: crear un dispositivo virtual sin avdmanager: la imagen más nueva (google_apis antes que
    /// default), su `.ini` y su `config.ini`; no pisa uno que exista; nombres y imágenes raros, fuera.
    #[test]
    fn crear_un_dispositivo_virtual() {
        let d = crate::CarpetaPrueba::nueva("gotv-crear-avd");
        let sdk = d.join("sdk");
        for img in ["android-34/default/arm64-v8a", "android-36/default/arm64-v8a", "android-36/google_apis/arm64-v8a"] {
            let p = sdk.join("system-images").join(img);
            std::fs::create_dir_all(&p).unwrap();
            std::fs::write(p.join("system.img"), "x").unwrap();
        }
        std::fs::create_dir_all(sdk.join("system-images/android-37/default/arm64-v8a")).unwrap(); // a medias: sin system.img
        let v = imagenes(&sdk);
        assert_eq!(v, ["system-images;android-36;google_apis;arm64-v8a", "system-images;android-36;default;arm64-v8a", "system-images;android-34;default;arm64-v8a"]);
        let avd = d.join("avd");
        let dir = crear_avd(&avd, "gotv-pixel", &sdk, &v[0]).unwrap();
        let ini = std::fs::read_to_string(avd.join("gotv-pixel.ini")).unwrap();
        assert!(ini.contains(&format!("path={}", dir.display())) && ini.contains("target=android-36"), "{ini}");
        let c = std::fs::read_to_string(dir.join("config.ini")).unwrap();
        for k in ["image.sysdir.1=system-images/android-36/google_apis/arm64-v8a/", "abi.type=arm64-v8a", "hw.cpu.arch=arm64", "tag.id=google_apis", "hw.keyboard=yes", "PlayStore.enabled=no"] {
            assert!(c.lines().any(|l| l == k), "{k}\n{c}");
        }
        assert!(crear_avd(&avd, "gotv-pixel", &sdk, &v[0]).is_err(), "no se pisa");
        assert!(crear_avd(&avd, "mal nombre", &sdk, &v[0]).is_err());
        assert!(crear_avd(&avd, "otro", &sdk, "system-images;android-37;default;arm64-v8a").is_err(), "imagen a medias");
        assert!(crear_avd(&avd, "otro", &sdk, "android-36").is_err());
    }

    /// De verdad: `crear` por la herramienta un dispositivo con la imagen más nueva del SDK, que el emulador lo
    /// ARRANQUE (sys.boot_completed = 1, por `encender`), apagarlo y borrar sólo lo creado (sus dos rutas).
    /// `cargo test -p gotv-core crear_dispositivo_de_verdad -- --ignored --nocapture`
    #[tokio::test]
    #[ignore]
    async fn crear_dispositivo_de_verdad() {
        let nombre = "gotv-prueba-crear";
        let avd = carpeta_avd();
        let (dir, ini) = (avd.join(format!("{nombre}.avd")), avd.join(format!("{nombre}.ini")));
        assert!(!dir.exists() && !ini.exists(), "ya existe {nombre}: no lo piso");
        let d = crate::CarpetaPrueba::nueva("gotv-crear-avd-real");
        let r = crate::movil::ejecutar(&json!({"action": "crear", "nombre": nombre}), &d).await.unwrap();
        eprintln!("{r}");
        let t = std::time::Instant::now();
        let e = crate::movil::ejecutar(&json!({"action": "encender", "dispositivo": nombre}), &d).await;
        eprintln!("encender: {e:?} ({:.1} s)", t.elapsed().as_secs_f32());
        let serie = e.as_ref().ok().and_then(|r| r.rsplit('(').next()).map(|s| s.trim_end_matches(')').trim().to_string());
        if let Some(s) = &serie {
            let _ = crate::movil::ejecutar(&json!({"action": "apagar", "dispositivo": s}), &d).await;
        }
        std::fs::remove_dir_all(&dir).unwrap();
        std::fs::remove_file(&ini).unwrap();
        assert!(e.is_ok() && serie.is_some_and(|s| s.starts_with("emulator-")), "{e:?}");
    }

    /// `registro` sin `app` en un proyecto Android usa la app del proyecto; si no está instalada lo DICE (con el
    /// camino: construir y luego registro) en vez de devolver el ruido de todo el sistema (medido 2026-11-18).
    #[tokio::test]
    async fn registro_sin_app_usa_la_del_proyecto_y_avisa_si_no_esta() {
        let d = crate::CarpetaPrueba::nueva("gotv-movil-registro");
        crate::movil::construir::crear_juguete_android(&d, "space.gotv.juguete", "public class MainActivity {}").unwrap();
        assert_eq!(crate::movil::construir::app_android(&d).as_deref(), Some("space.gotv.juguete"));
        let f = Falso::con(&[("adb devices", DEVICES), ("emu avd name", "gotv-prueba\nOK\n"), ("getprop ro.build.version.release", "16\n"), ("logcat -d -t", "11-18 10:38:07.009  2498  2498 I BluetoothSocketManagerBinder: ruido\n")]);
        f.regla("pm path space.gotv.juguete", false, b"");
        let a = android(f.clone());
        let ios = android(Falso::con(&[("adb devices", "List of devices attached\n\n")]));
        let p = Puentes { ios: &ios, android: &a };
        let r = ejecutar_con(&json!({"action": "registro", "dispositivo": "emulator-5554"}), &d, &p).await.unwrap();
        assert!(r.contains("space.gotv.juguete") && (r.contains("no está instalada") || r.contains("not installed")) && r.contains("construir"), "{r}");
        assert!(!r.contains("Bluetooth"), "sin el ruido del sistema: {r}");
        // instalada (y sin correr): el registro de siempre, con su aviso
        f.regla("pm path space.gotv.juguete", true, b"package:/data/app/~~x/space.gotv.juguete-1/base.apk\n");
        let r = ejecutar_con(&json!({"action": "registro", "dispositivo": "emulator-5554"}), &d, &p).await.unwrap();
        assert!(r.contains("space.gotv.juguete") && r.contains("Bluetooth"), "{r}");
        // fuera de un proyecto Android y sin app: como antes (no se sabe qué app)
        let vacia = crate::CarpetaPrueba::nueva("gotv-movil-registro-vacia");
        let r = ejecutar_con(&json!({"action": "registro", "dispositivo": "emulator-5554"}), &vacia, &p).await.unwrap();
        assert!(r.contains("Bluetooth"), "{r}");
    }

    /// La herramienta de punta a punta en Android con `adb` falso: listar (con el apagado y el teléfono
    /// sin autorizar), encender sin ventana con API gRPC protegida, árbol con las trampas, tocar por
    /// referencia, deslizar, tecla, lanzar, registro con cierres y borrar_datos.
    #[tokio::test]
    async fn dispositivo_android_de_punta_a_punta() {
        let d = crate::CarpetaPrueba::nueva("gotv-movil-android");
        let f = Falso::con(&[
            ("adb devices", DEVICES),
            ("emu avd name", "gotv-prueba\nOK\n"),
            ("getprop ro.build.version.release", "16\n"),
            ("emulator -list-avds", "INFO    | Storing crashdata in: /tmp/x\ngotv-prueba\ncx-sala-chrome\n"),
            ("exec-out uiautomator dump", crate::movil::arbol::tests::VOLCADO_ANDROID),
            ("resolve-activity", "priority=0 preferredOrder=0 match=0x108000 specificIndex=-1 isDefault=false\ncom.ejemplo/.Principal\n"),
            ("am start -W", "Starting: Intent { cmp=com.ejemplo/.Principal }\nStatus: ok\nActivity: com.ejemplo/.Principal\nComplete\n"),
            ("shell pidof com.ejemplo", "4321\n"),
            ("logcat -d -t 200 --pid=4321", "11-16 10:00:00.000  4321  4321 I Notas: hola\n"),
            ("logcat -d -b crash", "--------- beginning of crash\n11-16 10:00:01.000  4321  4321 E AndroidRuntime: FATAL EXCEPTION: main\n11-16 10:00:01.000  4321  4321 E AndroidRuntime: Process: com.ejemplo, PID: 4321\n"),
            ("pm clear", "Success\n"),
            ("getprop sys.boot_completed", "1\n"),
        ]);
        let a = android(f.clone());
        let ios = android(Falso::con(&[("adb devices", "List of devices attached\n\n")]));
        let p = Puentes { ios: &ios, android: &a };
        let l = ejecutar_con(&json!({"action": "listar"}), &d, &p).await.unwrap();
        assert!(l.contains("gotv-prueba · Android 16 · ") && l.contains("emulator-5554"), "{l}");
        assert!(l.contains("cx-sala-chrome") && l.contains("R58M12ABCDE") && (l.contains("sin autorizar") || l.contains("unauthorized")), "{l}");
        assert_eq!(l.matches("gotv-prueba").count(), 1, "el AVD encendido no sale dos veces: {l}");
        // encender uno apagado: sin ventana, primer puerto libre, gRPC con ficha
        let e = ejecutar_con(&json!({"action": "encender", "dispositivo": "cx-sala-chrome"}), &d, &p).await.unwrap();
        assert!(e.contains("emulator-5556"), "{e}");
        assert_eq!(f.fondos.lock().unwrap().clone(), vec!["emulator -avd cx-sala-chrome -port 5556 -no-audio -no-boot-anim -grpc 8556 -grpc-use-token -no-window"]);
        let t = ejecutar_con(&json!({"action": "arbol", "dispositivo": "emulator-5554"}), &d, &p).await.unwrap();
        assert!(t.contains("@d3 Tappable «Compras de mañana · hace 2 h»"), "{t}");
        let to = ejecutar_con(&json!({"action": "tocar", "dispositivo": "emulator-5554", "en": "@d3"}), &d, &p).await.unwrap();
        assert!(to.starts_with("✓") && to.contains("(540,480)"), "{to}");
        ejecutar_con(&json!({"action": "deslizar", "dispositivo": "emulator-5554", "en": "@d2", "direccion": "arriba"}), &d, &p).await.unwrap();
        ejecutar_con(&json!({"action": "tecla", "dispositivo": "emulator-5554", "tecla": "atras"}), &d, &p).await.unwrap();
        let la = ejecutar_con(&json!({"action": "lanzar", "dispositivo": "emulator-5554", "app": "com.ejemplo"}), &d, &p).await.unwrap();
        assert!(la.contains("4321"), "{la}");
        let r = ejecutar_con(&json!({"action": "registro", "dispositivo": "emulator-5554", "app": "com.ejemplo"}), &d, &p).await.unwrap();
        assert!(r.contains("Notas: hola") && r.contains("FATAL EXCEPTION") && !r.contains("---------"), "{r}");
        let b = ejecutar_con(&json!({"action": "borrar_datos", "dispositivo": "emulator-5554", "app": "com.ejemplo"}), &d, &p).await.unwrap();
        assert!(b.starts_with("✓"), "{b}");
        assert!(ejecutar_con(&json!({"action": "borrar_datos", "dispositivo": "emulator-5554"}), &d, &p).await.is_err(), "el dispositivo entero no se ofrece");
        let h = f.hechas();
        for o in [
            "adb -s emulator-5554 shell input tap 540 480",
            "adb -s emulator-5554 shell input swipe 540 1200 540 800 300",
            "adb -s emulator-5554 shell input keyevent 4",
            "adb -s emulator-5554 shell am start -W -n com.ejemplo/.Principal",
            "adb -s emulator-5554 shell pm clear com.ejemplo",
        ] {
            assert!(h.iter().any(|x| x == o), "falta «{o}» en {h:?}");
        }
        // un teléfono sin autorizar no se puede manejar
        assert!(ejecutar_con(&json!({"action": "arbol", "dispositivo": "R58M12ABCDE"}), &d, &p).await.unwrap_err().to_string().contains("R58M12ABCDE"));
    }

    /// Con un emulador o teléfono DE VERDAD (`GOTV_PRUEBA_ANDROID=<serie>`, ya encendido): listar, árbol,
    /// captura, «ñ» (por gRPC en el emulador) y registro, con tiempos. Sin medir aún: no se encendió el
    /// emulador en esta Mac (4,9 GB de RAM junto a una prueba larga con el modelo).
    #[tokio::test]
    #[ignore]
    async fn dispositivo_android_real() {
        let d = crate::CarpetaPrueba::nueva("gotv-movil-android-real");
        let a = Android::sistema();
        let ios = android(Falso::con(&[("adb devices", "List of devices attached\n\n")]));
        let p = Puentes { ios: &ios, android: &a };
        // `GOTV_PRUEBA_ANDROID_AVD=<avd>`: lo enciende la propia herramienta (y lo apaga al final)
        let avd = std::env::var("GOTV_PRUEBA_ANDROID_AVD").ok();
        if let Some(avd) = &avd {
            let t = std::time::Instant::now();
            let r = ejecutar_con(&json!({"action": "encender", "dispositivo": avd}), &d, &p).await.unwrap();
            eprintln!("[medido] encender {avd} (sin ventana, hasta sys.boot_completed): {:.2} s\n{r}", t.elapsed().as_secs_f32());
        }
        let serie = match std::env::var("GOTV_PRUEBA_ANDROID") {
            Ok(s) => s,
            Err(_) if avd.is_some() => a.listar().await.unwrap().into_iter().find(|x| x.encendido && x.id.starts_with("emulator-")).map(|x| x.id).expect("el emulador encendido"),
            Err(_) => return,
        };
        for (n, args) in [
            ("listar", json!({"action": "listar"})),
            ("arbol", json!({"action": "arbol", "dispositivo": serie})),
            ("captura", json!({"action": "captura", "dispositivo": serie})),
            ("registro", json!({"action": "registro", "dispositivo": serie, "lineas": 20})),
        ] {
            let t = std::time::Instant::now();
            let r = ejecutar_con(&args, &d, &p).await.unwrap();
            eprintln!("[medido] {n}: {:.2} s\n{r}", t.elapsed().as_secs_f32());
        }
        if serie.starts_with("emulator-") {
            let t = std::time::Instant::now();
            let r = a.escribir(&serie, None, "mañana ñandú").await.unwrap();
            eprintln!("[medido] escribir ñ por gRPC: {:.2} s · {r}", t.elapsed().as_secs_f32());
        }
        // tocar lo primero accionable y ver que el árbol responde
        let arbol = ejecutar_con(&json!({"action": "arbol", "dispositivo": serie}), &d, &p).await.unwrap();
        assert!(arbol.contains("@d1"), "el árbol trae elementos accionables: {arbol}");
        let t = std::time::Instant::now();
        let r = ejecutar_con(&json!({"action": "tocar", "dispositivo": serie, "en": "@d1"}), &d, &p).await.unwrap();
        eprintln!("[medido] tocar @d1 (y el árbol nuevo): {:.2} s\n{r}", t.elapsed().as_secs_f32());
        if avd.is_some() {
            let t = std::time::Instant::now();
            ejecutar_con(&json!({"action": "apagar", "dispositivo": serie}), &d, &p).await.unwrap();
            eprintln!("[medido] apagar: {:.2} s", t.elapsed().as_secs_f32());
        }
    }
}
