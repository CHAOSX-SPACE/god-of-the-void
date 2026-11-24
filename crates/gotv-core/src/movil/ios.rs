//! F18 · M2/M3 · El simulador de iOS: `xcrun simctl` para lo que Apple ya da (encender, instalar, lanzar,
//! captura, registro, ajustes) e idb para leer y tocar. Si idb falta, todo lo de `simctl` sigue valiendo.
use super::arbol::{self, Arbol};
use super::idb::{self, Boton, Idb};
use super::{Corredor, Dispositivo, Plataforma, Puente, Salida, Sistema, Tecla};
use anyhow::{anyhow, bail, Result};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

pub struct Ios {
    pub run: Arc<dyn Corredor>,
    /// En las pruebas: un idb ya escuchando (si no, se lanza `idb_companion` por dispositivo).
    pub idb: Option<Idb>,
}

const CORTO: Duration = Duration::from_secs(30);

/// Los simuladores de iPhone y iPad de `simctl list devices available` (su salida de TEXTO: secciones
/// `-- iOS 27.0 --` y líneas `    Nombre (UDID) (Estado)`; el nombre puede llevar paréntesis, se lee por la
/// derecha).
pub fn leer_lista(texto: &str) -> Vec<Dispositivo> {
    let mut out = Vec::new();
    let mut sistema: Option<String> = None;
    for l in texto.lines() {
        let l = l.trim();
        if let Some(s) = l.strip_prefix("-- ").and_then(|s| s.strip_suffix(" --")) {
            sistema = s.starts_with("iOS ").then(|| s.to_string());
            continue;
        }
        let Some(so) = &sistema else { continue };
        let Some(resto) = l.strip_suffix(')') else { continue };
        let Some((resto, estado)) = resto.rsplit_once(" (") else { continue };
        let Some(resto) = resto.strip_suffix(')') else { continue };
        let Some((nombre, udid)) = resto.rsplit_once(" (") else { continue };
        if !super::es_udid(udid) {
            continue;
        }
        out.push(Dispositivo { id: udid.into(), nombre: nombre.trim().into(), plataforma: Plataforma::Ios, sistema: so.clone(), encendido: estado == "Booted", fisico: false });
    }
    out
}

/// Un valor de texto del `Info.plist` (binario o XML) de una app.
pub fn de_info_plist(app: &Path, clave: &str) -> Result<String> {
    let p = app.join("Info.plist");
    let v = plist::Value::from_file(&p).map_err(|e| anyhow!("{}: {e}", p.display()))?;
    v.as_dictionary().and_then(|d| d.get(clave)).and_then(|x| x.as_string()).map(String::from).ok_or_else(|| anyhow!("{}", crate::tr!("{} no tiene {clave}", "{} has no {clave}", p.display())))
}

fn s(v: &[&str]) -> Vec<String> {
    v.iter().map(|x| x.to_string()).collect()
}

impl Ios {
    pub fn sistema() -> Self {
        Ios { run: Arc::new(Sistema), idb: None }
    }

    async fn simctl(&self, args: &[&str], tope: Duration) -> Result<Salida> {
        let mut a = vec!["simctl".to_string()];
        a.extend(s(args));
        self.run.correr("xcrun", &a, tope).await
    }

    async fn idb(&self, id: &str) -> Result<Idb> {
        match &self.idb {
            Some(i) => Ok(i.clone()),
            None => idb::de(id).await,
        }
    }

    /// El nombre del ejecutable de una app instalada (para filtrar su registro).
    async fn ejecutable(&self, id: &str, bundle: &str) -> Result<String> {
        let o = self.simctl(&["get_app_container", id, bundle, "app"], CORTO).await?.exigir(&crate::tr!("{bundle} no está instalada", "{bundle} is not installed"))?;
        de_info_plist(Path::new(o.texto().trim()), "CFBundleExecutable")
    }

    /// Tras lanzar: espera (máx. 8 s) a que la app esté AL FRENTE, es decir, que el nodo `Application`
    /// del árbol lleve su nombre. Medido 2026-11-17 (iPhone 17e recién encendido): `simctl launch` vuelve
    /// con el proceso vivo, pero el árbol leído enseguida aún era el de la pantalla anterior, sin los
    /// botones de la app (1 de 4 corridas). Si no se sabe el nombre o idb no responde, no espera.
    async fn esperar_al_frente(&self, id: &str, bundle: &str) {
        let Ok(o) = self.simctl(&["get_app_container", id, bundle, "app"], CORTO).await else { return };
        let ruta = o.texto();
        let ruta = Path::new(ruta.trim());
        if !o.ok || ruta.as_os_str().is_empty() {
            return;
        }
        let Some(nombre) = ["CFBundleDisplayName", "CFBundleName", "CFBundleExecutable"].iter().find_map(|k| de_info_plist(ruta, k).ok().filter(|n| !n.is_empty())) else { return };
        let Ok(i) = self.idb(id).await else { return };
        let fin = std::time::Instant::now() + Duration::from_secs(8);
        while std::time::Instant::now() < fin {
            match i.arbol_json(false).await.and_then(|j| arbol::de_ios(&j, false)) {
                Ok(a) if a.app == nombre => return,
                Ok(_) => tokio::time::sleep(Duration::from_millis(300)).await,
                Err(_) => return,
            }
        }
    }
}

#[async_trait::async_trait]
impl Puente for Ios {
    async fn construir(&self, id: &str, root: &Path, args: &Value) -> Result<super::construir::Construida> {
        let p = super::construir::detectar(root, true).ok_or_else(|| anyhow!("{}", crate::i18n::t("no hay proyecto de iOS aquí (.xcodeproj, .xcworkspace o Flutter con ios/)", "there is no iOS project here (.xcodeproj, .xcworkspace or Flutter with ios/)")))?;
        super::construir::ios(&*self.run, root, &p, id, args.get("esquema").and_then(|v| v.as_str()).map(str::trim).filter(|s| !s.is_empty())).await
    }

    /// Las apps abiertas son trabajos de `launchd` del simulador llamados `UIKitApplication:<bundle>[…]`.
    async fn vive(&self, id: &str, app: &str) -> Result<bool> {
        let o = self.simctl(&["spawn", id, "launchctl", "list"], CORTO).await?.exigir("simctl spawn launchctl list")?;
        Ok(o.texto().lines().any(|l| l.contains(&format!("UIKitApplication:{app}[")) && !l.trim_start().starts_with('-')))
    }

    async fn listar(&self) -> Result<Vec<Dispositivo>> {
        let o = self.simctl(&["list", "devices", "available"], CORTO).await?.exigir("simctl list")?;
        Ok(leer_lista(&o.texto()))
    }

    async fn encender(&self, id: &str, ventana: bool) -> Result<String> {
        let o = self.simctl(&["boot", id], Duration::from_secs(120)).await?;
        if !o.ok && !o.err.contains("current state: Booted") {
            o.exigir("simctl boot")?;
        }
        self.simctl(&["bootstatus", id, "-b"], Duration::from_secs(300)).await?.exigir("simctl bootstatus")?;
        if ventana {
            self.run.correr("open", &s(&["-a", "Simulator", "--args", "-CurrentDeviceUDID", id]), CORTO).await?.exigir("open -a Simulator")?;
        }
        Ok(id.to_string())
    }

    async fn apagar(&self, id: &str) -> Result<()> {
        idb::soltar(id).await;
        let o = self.simctl(&["shutdown", id], Duration::from_secs(60)).await?;
        if !o.ok && !o.err.contains("current state: Shutdown") {
            o.exigir("simctl shutdown")?;
        }
        Ok(())
    }

    async fn instalar(&self, id: &str, app: &Path) -> Result<String> {
        if app.extension().is_none_or(|e| e != "app") || !app.is_dir() {
            bail!("{}", crate::tr!("{} no es una app compilada para el simulador (carpeta .app)", "{} is not an app built for the simulator (.app folder)", app.display()));
        }
        let bundle = de_info_plist(app, "CFBundleIdentifier")?;
        self.simctl(&["install", id, &app.display().to_string()], Duration::from_secs(300)).await?.exigir("simctl install")?;
        Ok(bundle)
    }

    async fn lanzar(&self, id: &str, app: &str) -> Result<u32> {
        let o = self.simctl(&["launch", "--terminate-running-process", id, app], Duration::from_secs(60)).await?.exigir(&format!("simctl launch {app}"))?;
        // «com.ejemplo.App: 4242»
        let pid = o.texto().trim().rsplit(':').next().and_then(|p| p.trim().parse().ok()).ok_or_else(|| anyhow!("simctl launch: {}", o.texto().trim()))?;
        self.esperar_al_frente(id, app).await;
        Ok(pid)
    }

    async fn cerrar(&self, id: &str, app: &str) -> Result<()> {
        self.simctl(&["terminate", id, app], CORTO).await?.exigir(&format!("simctl terminate {app}"))?;
        Ok(())
    }

    async fn arbol(&self, id: &str, todo: bool) -> Result<Arbol> {
        let j = self.idb(id).await?.arbol_json(todo).await?;
        arbol::de_ios(&j, todo)
    }

    async fn captura(&self, id: &str, a: &Path) -> Result<()> {
        self.simctl(&["io", id, "screenshot", "--type=png", &a.display().to_string()], Duration::from_secs(60)).await?.exigir("simctl io screenshot")?;
        Ok(())
    }

    async fn tocar(&self, id: &str, x: i32, y: i32) -> Result<()> {
        self.idb(id).await?.tocar(x as f64, y as f64).await
    }

    async fn deslizar(&self, id: &str, de: (i32, i32), a: (i32, i32), ms: u32) -> Result<()> {
        self.idb(id).await?.deslizar((de.0 as f64, de.1 as f64), (a.0 as f64, a.1 as f64), ms).await
    }

    /// Por `set_value` sobre el campo (cualquier texto). Si el campo no lo acepta (un lienzo, un juego), por
    /// teclas, que sólo saben ASCII: lo que no pasó se dice.
    async fn escribir(&self, id: &str, en: Option<(i32, i32)>, texto: &str) -> Result<String> {
        let i = self.idb(id).await?;
        let Some((x, y)) = en else { bail!("{}", crate::i18n::t("escribir en iOS necesita el campo destino", "typing on iOS needs the target field")) };
        match i.poner_valor(x as f64, y as f64, texto).await {
            Ok(()) => Ok(String::new()),
            Err(e) => {
                i.tocar(x as f64, y as f64).await?;
                let fuera = i.teclear(texto).await?;
                Ok(if fuera.is_empty() {
                    crate::tr!("✓ escrito por teclas (el campo no aceptó el valor directo: {e})", "✓ typed key by key (the field did not accept a direct value: {e})")
                } else {
                    crate::tr!("⚠ escrito por teclas SIN «{fuera}» (por teclas sólo pasa ASCII; el campo no aceptó el valor directo: {e})", "⚠ typed key by key WITHOUT «{fuera}» (keys only carry ASCII; the field did not accept a direct value: {e})")
                })
            }
        }
    }

    async fn esperar(&self, id: &str, texto: &str, tope_ms: u32) -> Result<bool> {
        self.idb(id).await?.esperar(texto, tope_ms).await
    }

    async fn tecla(&self, id: &str, t: Tecla) -> Result<()> {
        let i = self.idb(id).await?;
        match t {
            Tecla::Inicio => i.boton(Boton::Inicio).await,
            Tecla::Bloquear => i.boton(Boton::Bloquear).await,
            Tecla::SubirVolumen => i.boton(Boton::SubirVolumen).await,
            Tecla::BajarVolumen => i.boton(Boton::BajarVolumen).await,
            Tecla::Intro => i.tecla(40).await,
            Tecla::Borrar => i.tecla(42).await,
            Tecla::Tab => i.tecla(43).await,
            Tecla::Derecha => i.tecla(79).await,
            Tecla::Izquierda => i.tecla(80).await,
            Tecla::Abajo => i.tecla(81).await,
            Tecla::Arriba => i.tecla(82).await,
            Tecla::Atras | Tecla::Apps => bail!("{}", crate::i18n::t("iOS no tiene tecla de atrás ni de apps: toca el botón de la app (o desliza desde el borde izquierdo)", "iOS has no back or apps key: tap the app's button (or swipe from the left edge)")),
        }
    }

    /// Las últimas líneas del registro (de la app, si se dice cuál) y sus cierres inesperados recientes.
    async fn registro(&self, id: &str, app: Option<&str>, lineas: usize) -> Result<String> {
        let exe = match app {
            Some(a) => Some(self.ejecutable(id, a).await?),
            None => None,
        };
        let pred = exe.as_ref().map(|e| format!("process == \"{}\"", e.replace('"', "")));
        // medido 2026-11-17 (iPhone 17e, iOS 27): `--last 5m` tardó 18,6–35,7 s; `1m`, 4,1 s; `1m` filtrado
        // por proceso, 1,5 s
        let mut args = vec!["spawn", id, "log", "show", "--last", "1m", "--style", "compact"];
        if let Some(p) = &pred {
            args.extend(["--predicate", p.as_str()]);
        }
        let o = self.simctl(&args, Duration::from_secs(60)).await?.exigir("simctl spawn log show")?;
        let texto = o.texto();
        let todas: Vec<&str> = texto.lines().filter(|l| !l.starts_with("Timestamp") && !l.trim().is_empty()).collect();
        let mut out = todas[todas.len().saturating_sub(lineas)..].join("\n");
        if out.is_empty() {
            out = crate::i18n::t("(sin registro en el último minuto)", "(no log in the last minute)").into();
        }
        if let Some(e) = &exe {
            let cierres = cierres_recientes(&PathBuf::from(std::env::var("HOME").unwrap_or_default()).join("Library/Logs/DiagnosticReports"), e);
            if !cierres.is_empty() {
                out.push_str(&crate::tr!("\n💥 cierres inesperados de la última hora (léelos con read_file):\n{}", "\n💥 crashes in the last hour (read them with read_file):\n{}", cierres.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join("\n")));
            }
        }
        Ok(out)
    }

    async fn ajustar(&self, id: &str, args: &Value) -> Result<String> {
        let t = |k: &str| args.get(k).and_then(|v| v.as_str()).map(str::trim).filter(|v| !v.is_empty());
        let mut hecho = Vec::new();
        if let Some(a) = t("apariencia") {
            let m = match a {
                "oscuro" | "dark" => "dark",
                "claro" | "light" => "light",
                _ => bail!("{}", crate::tr!("apariencia «{a}»: claro u oscuro", "apariencia «{a}»: claro (light) or oscuro (dark)")),
            };
            self.simctl(&["ui", id, "appearance", m], CORTO).await?.exigir("simctl ui appearance")?;
            hecho.push(format!("appearance {m}"));
        }
        if let Some(l) = t("letra") {
            const TAMANOS: &[&str] = &["extra-small", "small", "medium", "large", "extra-large", "extra-extra-large", "extra-extra-extra-large", "accessibility-medium", "accessibility-large", "accessibility-extra-large", "accessibility-extra-extra-large", "accessibility-extra-extra-extra-large"];
            if !TAMANOS.contains(&l) {
                bail!("{}", crate::tr!("letra «{l}»: {}", "letra «{l}»: {}", TAMANOS.join(", ")));
            }
            self.simctl(&["ui", id, "content_size", l], CORTO).await?.exigir("simctl ui content_size")?;
            hecho.push(format!("content_size {l}"));
        }
        if let Some(u) = t("ubicacion") {
            let (la, lo) = u.split_once(',').ok_or_else(|| anyhow!("{}", crate::i18n::t("ubicacion: «latitud,longitud»", "ubicacion: «latitude,longitude»")))?;
            let (la, lo): (f64, f64) = (la.trim().parse()?, lo.trim().parse()?);
            self.simctl(&["location", id, "set", &format!("{la},{lo}")], CORTO).await?.exigir("simctl location")?;
            hecho.push(format!("location {la},{lo}"));
        }
        if let Some(url) = t("enlace") {
            self.simctl(&["openurl", id, url], CORTO).await?.exigir("simctl openurl")?;
            hecho.push(format!("openurl {url}"));
        }
        if let Some(p) = t("permiso") {
            let app = t("app").ok_or_else(|| anyhow!("{}", crate::i18n::t("permiso: falta `app`", "permiso: missing `app`")))?;
            let verbo = if args.get("conceder").and_then(|v| v.as_bool()).unwrap_or(true) { "grant" } else { "revoke" };
            self.simctl(&["privacy", id, verbo, p, app], CORTO).await?.exigir("simctl privacy")?;
            hecho.push(format!("privacy {verbo} {p} {app}"));
        }
        // (la notificación de prueba no está: `simctl push` sólo acepta su carga en JSON)
        if hecho.is_empty() {
            bail!("{}", crate::i18n::t("ajustar: apariencia, letra, ubicacion, enlace o permiso (+app, conceder)", "ajustar: apariencia, letra, ubicacion, enlace or permiso (+app, conceder)"));
        }
        Ok(format!("✓ {}", hecho.join(" · ")))
    }

    async fn borrar_datos(&self, id: &str, app: Option<&str>) -> Result<String> {
        match app {
            // iOS no tiene «borrar datos» de una app: se desinstala (con sus datos) y se reinstala después
            Some(a) => {
                self.simctl(&["uninstall", id, a], CORTO).await?.exigir("simctl uninstall")?;
                Ok(crate::tr!("✓ {a} desinstalada con sus datos (iOS no borra datos sin desinstalar): `instalar` de nuevo", "✓ {a} uninstalled with its data (iOS cannot clear data without uninstalling): `instalar` again"))
            }
            None => {
                self.apagar(id).await?;
                self.simctl(&["erase", id], Duration::from_secs(120)).await?.exigir("simctl erase")?;
                Ok(crate::i18n::t("✓ simulador borrado de fábrica (apagado)", "✓ simulator erased to factory state (off)").into())
            }
        }
    }

    async fn aquietar(&self, id: &str) {
        if let Ok(i) = self.idb(id).await {
            i.quieta(Duration::from_secs(2)).await;
        }
    }
}

/// Los informes de cierre (`.ips`) de la última hora de un ejecutable.
fn cierres_recientes(carpeta: &Path, exe: &str) -> Vec<PathBuf> {
    if exe.is_empty() {
        return vec![];
    }
    let hora = std::time::SystemTime::now() - Duration::from_secs(3600);
    let mut v: Vec<PathBuf> = std::fs::read_dir(carpeta)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with(&format!("{exe}-")) && e.path().extension().is_some_and(|x| x == "ips"))
        .filter(|e| e.metadata().and_then(|m| m.modified()).is_ok_and(|t| t > hora))
        .map(|e| e.path())
        .collect();
    v.sort();
    v.into_iter().rev().take(3).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::movil::grpc::{falso, Destino};
    use crate::movil::idb::tests::{eventos, manejador, set_value_de, Visto};
    use crate::movil::tests::Falso;
    use crate::movil::{ejecutar_con, Puentes};
    use serde_json::json;

    /// Un recorte de la salida REAL de `xcrun simctl list devices available` en esta Mac (2026-11-17,
    /// Xcode 27.2), con su espacio final en cada línea.
    const LISTA_REAL: &str = "== Devices ==\n-- iOS 27.0 --\n    iPhone 18 Pro (081558C1-E257-4271-9B75-8D62DCA52949) (Shutdown) \n    iPhone 17e (B179907A-9485-4F1C-9343-BEEC74AFA2AC) (Shutdown) \n    iPad Pro 13-inch (M5) (0D0C5288-649D-4399-9011-2164A063B77C) (Shutdown) \n-- tvOS 27.0 --\n    Apple TV 4K (3rd generation) (at 1080p) (758445E4-FCB4-4227-B88E-FE9441E94BCA) (Shutdown) \n-- watchOS 27.0 --\n    Apple Watch Series 12 (46mm) (31CEE3F9-C927-48AC-ADCD-D11477DE1F29) (Shutdown) \n";

    fn encendida() -> String {
        LISTA_REAL.replacen("(081558C1-E257-4271-9B75-8D62DCA52949) (Shutdown)", "(081558C1-E257-4271-9B75-8D62DCA52949) (Booted)", 1)
    }

    #[test]
    fn la_lista_real_de_simctl() {
        let v = leer_lista(LISTA_REAL);
        assert_eq!(v.len(), 3, "sólo iPhone y iPad: {v:?}");
        assert_eq!((v[0].nombre.as_str(), v[0].sistema.as_str(), v[0].id.as_str()), ("iPhone 18 Pro", "iOS 27.0", "081558C1-E257-4271-9B75-8D62DCA52949"));
        assert_eq!(v[2].nombre, "iPad Pro 13-inch (M5)", "el nombre con paréntesis se lee entero");
        assert!(!v[0].encendido && !v[0].fisico);
        assert!(leer_lista(&encendida())[0].encendido);
        assert!(leer_lista("basura\n-- iOS 27.0 --\n    sin udid (Shutdown)\n").is_empty());
    }

    const UDID: &str = "081558C1-E257-4271-9B75-8D62DCA52949";

    async fn montar(nombre: &str) -> (Arc<Falso>, Ios, Visto, crate::CarpetaPrueba) {
        let d = crate::CarpetaPrueba::nueva(nombre);
        let sock = d.join("idb.sock");
        let visto: Visto = Default::default();
        tokio::spawn(falso::servir_unix(tokio::net::UnixListener::bind(&sock).unwrap(), manejador(visto.clone())));
        let f = Falso::con(&[("simctl list devices", &encendida()),("simctl launch", "com.ejemplo.Juguete: 4242\n")]);
        let ios = Ios { run: f.clone(), idb: Some(Idb(Destino::Unix(sock))) };
        (f, ios, visto, d)
    }

    /// Un Android que no hay (`adb` no responde): la herramienta sigue con iOS y lo dice al listar.
    struct SinAndroid;
    #[async_trait::async_trait]
    impl Puente for SinAndroid {
        async fn listar(&self) -> Result<Vec<Dispositivo>> {
            bail!("adb: no instalado")
        }
        async fn encender(&self, _: &str, _: bool) -> Result<String> { unreachable!() }
        async fn apagar(&self, _: &str) -> Result<()> { unreachable!() }
        async fn instalar(&self, _: &str, _: &Path) -> Result<String> { unreachable!() }
        async fn lanzar(&self, _: &str, _: &str) -> Result<u32> { unreachable!() }
        async fn cerrar(&self, _: &str, _: &str) -> Result<()> { unreachable!() }
        async fn arbol(&self, _: &str, _: bool) -> Result<Arbol> { unreachable!() }
        async fn captura(&self, _: &str, _: &Path) -> Result<()> { unreachable!() }
        async fn tocar(&self, _: &str, _: i32, _: i32) -> Result<()> { unreachable!() }
        async fn deslizar(&self, _: &str, _: (i32, i32), _: (i32, i32), _: u32) -> Result<()> { unreachable!() }
        async fn escribir(&self, _: &str, _: Option<(i32, i32)>, _: &str) -> Result<String> { unreachable!() }
        async fn esperar(&self, _: &str, _: &str, _: u32) -> Result<bool> { unreachable!() }
        async fn tecla(&self, _: &str, _: Tecla) -> Result<()> { unreachable!() }
        async fn registro(&self, _: &str, _: Option<&str>, _: usize) -> Result<String> { unreachable!() }
        async fn ajustar(&self, _: &str, _: &Value) -> Result<String> { unreachable!() }
        async fn borrar_datos(&self, _: &str, _: Option<&str>) -> Result<String> { unreachable!() }
        async fn aquietar(&self, _: &str) {}
    }

    /// F18 · M4 + M10 con falsos: `construir` construye, instala y lanza (y devuelve el árbol); la
    /// compuerta móvil del proyecto repite el ciclo y dice si la app sigue viva, si se cerró al abrir
    /// (`launchctl` la lista sin proceso) o si no construye. Otro proyecto, sin dispositivo usado: nada.
    #[tokio::test]
    async fn construir_y_la_compuerta_movil() {
        let (f, ios, _visto, d) = montar("gotv-movil-construir").await;
        let p = Puentes { ios: &ios, android: &SinAndroid };
        super::super::construir::crear_juguete_ios(&d, "Juguete", "space.gotv.juguete", "import SwiftUI").unwrap();
        let app = d.join("der/Juguete.app");
        std::fs::create_dir_all(&app).unwrap();
        let mut info = plist::Dictionary::new();
        info.insert("CFBundleIdentifier".into(), "space.gotv.juguete".into());
        info.insert("CFBundleExecutable".into(), "Juguete".into());
        // el árbol del idb falso es el de «Ajustes»: con ese nombre la app ya está al frente y `lanzar` no espera
        info.insert("CFBundleDisplayName".into(), "Ajustes".into());
        plist::Value::Dictionary(info).to_file_xml(app.join("Info.plist")).unwrap();
        let ajustes = json!([{"buildSettings": {"WRAPPER_EXTENSION": "app", "TARGET_BUILD_DIR": d.join("der").display().to_string(), "FULL_PRODUCT_NAME": "Juguete.app"}}]).to_string();
        f.regla("-list -json", true, br#"{"project":{"schemes":["Juguete"]}}"#);
        f.regla("-showBuildSettings", true, ajustes.as_bytes());
        f.regla("get_app_container", true, app.display().to_string().as_bytes());
        f.regla("launchctl list", true, b"PID\tStatus\tLabel\n4242\t0\tUIKitApplication:space.gotv.juguete[1a2b][rb-legacy]\n");

        assert_eq!(arbol::de_ios(&crate::movil::arbol::tests::ajustes_ios(false), false).unwrap().app, "Ajustes", "el árbol sabe qué app está al frente");
        let t = std::time::Instant::now();
        let r = ejecutar_con(&json!({"action": "construir"}), &d, &p).await.unwrap();
        assert!(r.contains("instalada y en marcha: space.gotv.juguete") && r.contains("@d1"), "{r}");
        assert!(t.elapsed() < Duration::from_secs(3), "al frente desde el principio: no espera ({:?})", t.elapsed());
        assert!(f.hechas().iter().any(|o| o.contains("simctl install 081558C1-E257-4271-9B75-8D62DCA52949") && o.contains("Juguete.app")), "{:?}", f.hechas());

        let c = super::super::compuerta_con(&d, &p, Duration::ZERO).await.unwrap();
        assert!(!c.fallo && c.texto.contains("instalada y viva") && c.texto.contains(".gotv/movil/capturas/compuerta-"), "{c:?}");

        f.regla("launchctl list", true, b"PID\tStatus\tLabel\n-\t9\tUIKitApplication:space.gotv.juguete[1a2b][rb-legacy]\n");
        let c = super::super::compuerta_con(&d, &p, Duration::ZERO).await.unwrap();
        assert!(c.fallo && c.texto.contains("se CERRÓ al abrir"), "{c:?}");

        f.regla(" build", false, format!("{}/Juguete/App.swift:1:1: error: no such module 'SwiftU'\n", d.display()).as_bytes());
        let c = super::super::compuerta_con(&d, &p, Duration::ZERO).await.unwrap();
        assert!(c.fallo && c.texto.contains("no construye") && c.texto.contains("Juguete/App.swift:1:1: no such module 'SwiftU'"), "{c:?}");

        let otro = crate::CarpetaPrueba::nueva("gotv-movil-construir-otro");
        assert_eq!(super::super::compuerta_con(&otro, &p, Duration::ZERO).await, None, "sin dispositivo usado en ese proyecto no hay compuerta");
    }

    /// La herramienta de punta a punta en iOS con el idb falso y `simctl` falso: árbol con referencias,
    /// tocar por referencia (y el árbol nuevo tras aquietarse), escribir «ñ» SIN destino (un solo campo)
    /// va por `set_value` y JAMÁS por teclas, esperar, lanzar, captura y ajustes por `simctl` con el UDID
    /// (nunca `booted`).
    #[tokio::test]
    async fn dispositivo_ios_de_punta_a_punta() {
        let (f, ios, visto, d) = montar("gotv-movil-ios").await;
        let p = Puentes { ios: &ios, android: &SinAndroid };
        let lista = ejecutar_con(&json!({"action": "listar"}), &d, &p).await.unwrap();
        assert!(lista.contains("iPhone 18 Pro · iOS 27.0") && lista.contains("Android: adb: no instalado"), "{lista}");
        let a = ejecutar_con(&json!({"action": "arbol"}), &d, &p).await.unwrap();
        assert!(a.starts_with("@d1 SearchField «Buscar»") && a.contains("@d3 Cell «General»"), "{a}");
        let t = ejecutar_con(&json!({"action": "tocar", "en": "@d3"}), &d, &p).await.unwrap();
        assert!(t.starts_with("✓") && t.contains("(201,316)") && t.contains("@d1 SearchField"), "{t}");
        let e = ejecutar_con(&json!({"action": "escribir", "texto": "mañana ñandú 日本"}), &d, &p).await.unwrap();
        assert!(e.contains("= «mañana ñandú 日本»"), "el árbol nuevo trae lo escrito: {e}");
        let w = ejecutar_con(&json!({"action": "esperar", "texto": "general", "segundos": 2}), &d, &p).await.unwrap();
        assert!(w.starts_with("✓ «general» apareció"), "{w}");
        let l = ejecutar_con(&json!({"action": "lanzar", "app": "com.ejemplo.Juguete"}), &d, &p).await.unwrap();
        assert!(l.contains("4242"), "{l}");
        let c = ejecutar_con(&json!({"action": "captura"}), &d, &p).await.unwrap();
        assert!(c.contains(".gotv/movil/capturas/081558C1-"), "{c}");
        let j = ejecutar_con(&json!({"action": "ajustar", "apariencia": "oscuro", "ubicacion": "4.71,-74.07"}), &d, &p).await.unwrap();
        assert!(j.contains("appearance dark") && j.contains("location 4.71,-74.07"), "{j}");
        assert!(ejecutar_con(&json!({"action": "tecla", "tecla": "atras"}), &d, &p).await.unwrap_err().to_string().contains("iOS"));
        // una referencia que no existe se dice
        assert!(ejecutar_con(&json!({"action": "tocar", "en": "@d99"}), &d, &p).await.unwrap_err().to_string().contains("@d99"));
        let v = visto.lock().unwrap().clone();
        let sv: Vec<_> = v.iter().filter(|(m, _)| m == "accessibility_action").filter_map(|(_, b)| set_value_de(&b[0])).collect();
        assert_eq!(sv, vec![(201.0, 168.0, "mañana ñandú 日本".to_string())], "escribir apunta al centro del ÚNICO campo");
        let hids: Vec<String> = v.iter().filter(|(m, _)| m == "hid").flat_map(|(_, b)| eventos(b)).collect();
        assert_eq!(hids, vec!["toque abajo 201,316", "toque arriba 201,316"], "escribir con ñ no pasó por teclas: {hids:?}");
        assert!(v.iter().filter(|(m, _)| m == "accessibility_quiescence").count() >= 2, "tocar y escribir esperan a que se aquiete");
        let h = f.hechas();
        assert!(h.iter().any(|o| o == &format!("xcrun simctl launch --terminate-running-process {UDID} com.ejemplo.Juguete")), "{h:?}");
        assert!(h.iter().any(|o| o.starts_with(&format!("xcrun simctl io {UDID} screenshot --type=png ")) && o.contains(".gotv/movil/capturas/")), "{h:?}");
        assert!(h.iter().any(|o| o == &format!("xcrun simctl ui {UDID} appearance dark")), "{h:?}");
        assert!(h.iter().any(|o| o == &format!("xcrun simctl location {UDID} set 4.71,-74.07")), "{h:?}");
        assert!(!h.iter().any(|o| o.contains("booted")), "jamás `booted`: {h:?}");
    }

    /// Un campo que no acepta `set_value`: se escribe por teclas y se avisa de lo que no pasó.
    #[tokio::test]
    async fn sin_set_value_escribe_por_teclas_y_avisa() {
        let d = crate::CarpetaPrueba::nueva("gotv-movil-ios-teclas");
        let sock = d.join("idb.sock");
        let visto: Visto = Default::default();
        let base = manejador(visto.clone());
        let m: falso::Manejador = Arc::new(move |ruta, msgs, cab| if ruta.ends_with("accessibility_action") { Err((3, "set_value no admitido".into())) } else { base(ruta, msgs, cab) });
        tokio::spawn(falso::servir_unix(tokio::net::UnixListener::bind(&sock).unwrap(), m));
        let ios = Ios { run: Falso::con(&[]), idb: Some(Idb(Destino::Unix(sock))) };
        let r = ios.escribir(UDID, Some((10, 20)), "año").await.unwrap();
        assert!(r.contains("«ñ»"), "{r}");
        let hids: Vec<String> = visto.lock().unwrap().iter().filter(|(m, _)| m == "hid").flat_map(|(_, b)| eventos(b)).collect();
        assert_eq!(hids[..2], ["toque abajo 10,20".to_string(), "toque arriba 10,20".to_string()]);
        assert_eq!(hids.iter().filter(|e| e.starts_with("tecla abajo")).count(), 2, "a y o: {hids:?}");
    }

    /// Con un simulador DE VERDAD (`GOTV_PRUEBA_IOS=<udid>`, apagado al empezar): la herramienta por
    /// `simctl` real —encender sin ventana, captura PNG, modo oscuro, registro y apagar— con tiempos. Lo de
    /// idb (árbol, tocar, escribir) sólo si `idb_companion` está.
    #[tokio::test]
    #[ignore]
    async fn dispositivo_ios_real() {
        let Ok(udid) = std::env::var("GOTV_PRUEBA_IOS") else { return };
        let d = crate::CarpetaPrueba::nueva("gotv-movil-ios-real");
        let ios = Ios::sistema();
        let p = Puentes { ios: &ios, android: &SinAndroid };
        let paso = |n: &str, t0: std::time::Instant| eprintln!("[medido] {n}: {:.2} s", t0.elapsed().as_secs_f32());
        let t = std::time::Instant::now();
        let l = ejecutar_con(&json!({"action": "listar"}), &d, &p).await.unwrap();
        paso("listar", t);
        assert!(l.contains(&udid), "{l}");
        let t = std::time::Instant::now();
        let e = ejecutar_con(&json!({"action": "encender", "dispositivo": udid}), &d, &p).await.unwrap();
        paso("encender (boot + bootstatus -b)", t);
        eprintln!("{e}");
        let t = std::time::Instant::now();
        let c = ejecutar_con(&json!({"action": "captura", "dispositivo": udid}), &d, &p).await.unwrap();
        paso("captura", t);
        let png = std::fs::read_dir(d.join(".gotv/movil/capturas")).unwrap().next().unwrap().unwrap().path();
        let img = image::open(&png).unwrap();
        eprintln!("{c} · {}×{} px", img.width(), img.height());
        let t = std::time::Instant::now();
        let a = ejecutar_con(&json!({"action": "ajustar", "dispositivo": udid, "apariencia": "oscuro"}), &d, &p).await.unwrap();
        paso("ajustar apariencia oscuro", t);
        assert!(a.contains("appearance dark"), "{a}");
        let o = ios.run.correr("xcrun", &s(&["simctl", "ui", &udid, "appearance"]), CORTO).await.unwrap();
        assert_eq!(o.texto().trim(), "dark", "el simulador dice que quedó oscuro");
        let t = std::time::Instant::now();
        let r = ejecutar_con(&json!({"action": "registro", "dispositivo": udid, "lineas": 20}), &d, &p).await.unwrap();
        paso("registro (log show 1 min, 20 líneas)", t);
        eprintln!("{} líneas de registro", r.lines().count());
        if idb::companion().is_some() {
            // Ajustes (viene en todo simulador): árbol, tocar y el árbol nuevo
            ejecutar_con(&json!({"action": "lanzar", "dispositivo": udid, "app": "com.apple.Preferences"}), &d, &p).await.unwrap();
            let t = std::time::Instant::now();
            let ar = ejecutar_con(&json!({"action": "arbol", "dispositivo": udid}), &d, &p).await.unwrap();
            paso("arbol por idb (primera vez, con el companion arrancando)", t);
            eprintln!("{ar}");
            assert!(ar.contains("@d1"), "el árbol de Ajustes trae elementos accionables: {ar}");
            let t = std::time::Instant::now();
            let ar2 = ejecutar_con(&json!({"action": "arbol", "dispositivo": udid}), &d, &p).await.unwrap();
            paso("arbol por idb (segunda vez)", t);
            let t = std::time::Instant::now();
            let tr = ejecutar_con(&json!({"action": "tocar", "dispositivo": udid, "en": "@d1"}), &d, &p).await.unwrap();
            paso("tocar @d1 (y esperar quietud + árbol nuevo)", t);
            eprintln!("{tr}");
            assert_ne!(ar2, tr, "tocar cambió la pantalla");
            let t = std::time::Instant::now();
            ejecutar_con(&json!({"action": "tecla", "dispositivo": udid, "tecla": "inicio"}), &d, &p).await.unwrap();
            paso("tecla inicio", t);
        } else {
            eprintln!("[sin medir] arbol/tocar/escribir: idb_companion no está en esta Mac");
        }
        ejecutar_con(&json!({"action": "ajustar", "dispositivo": udid, "apariencia": "claro"}), &d, &p).await.unwrap();
        let t = std::time::Instant::now();
        ejecutar_con(&json!({"action": "apagar", "dispositivo": udid}), &d, &p).await.unwrap();
        paso("apagar", t);
        assert!(!leer_lista(&ios.simctl(&["list", "devices", "available"], CORTO).await.unwrap().texto()).iter().any(|x| x.id == udid && x.encendido));
    }

    #[test]
    fn info_plist_xml_y_binario() {
        let d = crate::CarpetaPrueba::nueva("gotv-movil-plist");
        let app = d.join("Juguete.app");
        std::fs::create_dir_all(&app).unwrap();
        let mut dic = plist::Dictionary::new();
        dic.insert("CFBundleIdentifier".into(), "com.ejemplo.Juguete".into());
        dic.insert("CFBundleExecutable".into(), "Juguete".into());
        plist::Value::Dictionary(dic.clone()).to_file_binary(app.join("Info.plist")).unwrap();
        assert_eq!(de_info_plist(&app, "CFBundleIdentifier").unwrap(), "com.ejemplo.Juguete");
        plist::Value::Dictionary(dic).to_file_xml(app.join("Info.plist")).unwrap();
        assert_eq!(de_info_plist(&app, "CFBundleExecutable").unwrap(), "Juguete");
        assert!(de_info_plist(&app, "Nada").is_err());
    }
}
