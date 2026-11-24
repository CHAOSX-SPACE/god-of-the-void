//! F18 · M8 · F18.I · Instalar el SDK de Android desde Rust, con sus avisos legales.
//!
//! Sin Java, sin Homebrew, sin `sdkmanager`: GOTV lee el catálogo OFICIAL del repositorio de Android
//! (`dl.google.com/android/repository/repository2-3.xml` y, para las imágenes del sistema,
//! `…/sys-img/google_apis/sys-img2-3.xml`), elige de cada paquete la versión ESTABLE para esta Mac, la baja
//! con `reqwest` comprobando su tamaño y su SHA-1 publicados, la descomprime y la deja en su sitio del SDK
//! con su `package.xml`.
//!
//! Reglas (F18.I): GOTV no redistribuye nada (baja del servidor del dueño, a petición del usuario); antes de
//! bajar se muestra quién publica, de dónde, cuánto pesa y la licencia con su nombre exacto; **acepta el
//! humano, una licencia cada vez** (no hay herramienta del modelo para esto, y jamás se acepta en bloque:
//! falla #2620); queda constancia (qué acuerdo, la suma de su texto, la fecha y el paquete) en el formato
//! propio de GOTV (`programar::codec`, sin JSON); si el texto cambia, se vuelve a pedir; lo bajado se
//! comprueba y, si no casa, no se instala; se desinstala sólo lo que instaló GOTV.
use anyhow::{anyhow, bail, Context, Result};
use quick_xml::events::{BytesStart, Event};
use quick_xml::{Reader, XmlVersion};
use std::path::{Path, PathBuf};

fn valor(e: &BytesStart, clave: &str) -> String {
    e.attributes().flatten().find(|a| a.key.local_name().as_ref() == clave).and_then(|a| a.normalized_value(XmlVersion::Implicit1_0).ok().map(|v| v.trim().to_string())).unwrap_or_default()
}

pub const CATALOGO: &str = "https://dl.google.com/android/repository/repository2-3.xml";
pub const CATALOGO_IMAGENES: &str = "https://dl.google.com/android/repository/sys-img/google_apis/sys-img2-3.xml";

/// Lo que se instala con «un clic» (F18.I, paso 1): herramientas de plataforma, emulador, plataforma e
/// imagen del sistema para Apple Silicon.
pub const DE_UN_CLIC: &[&str] = &["platform-tools", "emulator", "platforms;android-36", "system-images;android-36;google_apis;arm64-v8a"];

/// Un archivo de un paquete (uno por sistema y arquitectura, o uno para todos).
#[derive(Clone, Debug, PartialEq)]
pub struct Archivo {
    pub url: String,
    pub tamano: u64,
    pub sha1: String,
    /// `macosx`, `linux`, `windows`; vacío = todos.
    pub so: String,
    /// `aarch64`, `x64`; vacío = todas.
    pub arq: String,
}

/// Un paquete del catálogo.
#[derive(Clone, Debug, PartialEq)]
pub struct Paquete {
    /// `platforms;android-36` (las partes se vuelven carpetas).
    pub ruta: String,
    pub nombre: String,
    pub version: (u32, u32, u32),
    /// `stable`, `beta`, `dev`, `canary`.
    pub canal: String,
    pub licencia: String,
    pub archivos: Vec<Archivo>,
    /// Dirección base de las URL relativas de sus archivos.
    pub base: String,
}

/// Un catálogo leído.
#[derive(Clone, Debug, Default)]
pub struct Catalogo {
    pub paquetes: Vec<Paquete>,
    /// Licencias: (id, texto EXACTO).
    pub licencias: Vec<(String, String)>,
}

impl Catalogo {
    /// Lee el XML del repositorio. `base`: la carpeta de la que cuelgan las URL relativas.
    pub fn leer(xml: &str, base: &str) -> Result<Catalogo> {
        let mut r = Reader::from_str(xml);
        let mut cat = Catalogo::default();
        let mut canales: Vec<(String, String)> = Vec::new();
        let mut pila: Vec<String> = Vec::new();
        let mut actual: Option<Paquete> = None;
        let mut archivo: Option<Archivo> = None;
        let mut canal_id = String::new();
        let mut licencia_id = String::new();
        let mut texto = String::new();
        loop {
            match r.read_event().map_err(|e| anyhow!("{}", crate::tr!("catálogo de Android: {e}", "Android catalogue: {e}")))? {
                Event::Start(e) => {
                    let n = e.local_name().as_ref().to_string();
                    let attr = |k: &str| valor(&e, k);
                    match n.as_str() {
                        "remotePackage" => actual = Some(Paquete { ruta: attr("path"), nombre: String::new(), version: (0, 0, 0), canal: "stable".into(), licencia: String::new(), archivos: vec![], base: base.into() }),
                        "archive" => archivo = Some(Archivo { url: String::new(), tamano: 0, sha1: String::new(), so: String::new(), arq: String::new() }),
                        "channel" => canal_id = attr("id"),
                        "license" => licencia_id = attr("id"),
                        _ => {}
                    }
                    texto.clear();
                    pila.push(n);
                }
                Event::Empty(e) => {
                    let n = e.local_name().as_ref().to_string();
                    let rf = valor(&e, "ref");
                    if let Some(p) = actual.as_mut() {
                        match n.as_str() {
                            "uses-license" => p.licencia = rf,
                            "channelRef" => p.canal = canales.iter().find(|(id, _)| *id == rf).map(|(_, c)| c.clone()).unwrap_or(rf),
                            _ => {}
                        }
                    }
                }
                Event::Text(t) => texto.push_str(&t),
                Event::CData(t) => texto.push_str(&t),
                // quick-xml 0.38+: `&quot;` y compañía llegan aparte
                Event::GeneralRef(g) => {
                    let n: &str = &g;
                    match n.strip_prefix('#') {
                        Some(c) => texto.extend(c.strip_prefix('x').map(|h| u32::from_str_radix(h, 16).ok()).unwrap_or_else(|| c.parse().ok()).and_then(char::from_u32)),
                        None => texto.push_str(quick_xml::escape::resolve_predefined_entity(n).unwrap_or("")),
                    }
                }
                Event::End(_) => {
                    let n = pila.pop().unwrap_or_default();
                    let padre = pila.last().map(String::as_str).unwrap_or("");
                    let v = texto.trim().to_string();
                    match (n.as_str(), padre) {
                        ("channel", _) => canales.push((canal_id.clone(), v)),
                        ("license", _) if actual.is_none() => cat.licencias.push((licencia_id.clone(), texto.clone())),
                        ("display-name", _) => {
                            if let Some(p) = actual.as_mut() {
                                p.nombre = v;
                            }
                        }
                        ("major" | "minor" | "micro", "revision") if archivo.is_none() => {
                            if let Some(p) = actual.as_mut() {
                                let x = v.parse().unwrap_or(0);
                                match n.as_str() {
                                    "major" => p.version.0 = x,
                                    "minor" => p.version.1 = x,
                                    _ => p.version.2 = x,
                                }
                            }
                        }
                        ("size", "complete") => archivo.iter_mut().for_each(|a| a.tamano = v.parse().unwrap_or(0)),
                        ("checksum", "complete") => archivo.iter_mut().for_each(|a| a.sha1 = v.to_ascii_lowercase()),
                        ("url", "complete") => archivo.iter_mut().for_each(|a| a.url = v.clone()),
                        ("host-os", "archive") => archivo.iter_mut().for_each(|a| a.so = v.clone()),
                        ("host-arch", "archive") => archivo.iter_mut().for_each(|a| a.arq = v.clone()),
                        ("archive", _) => {
                            if let (Some(p), Some(a)) = (actual.as_mut(), archivo.take()) {
                                p.archivos.push(a);
                            }
                        }
                        ("remotePackage", _) => {
                            if let Some(p) = actual.take() {
                                cat.paquetes.push(p);
                            }
                        }
                        _ => {}
                    }
                    texto.clear();
                }
                Event::Eof => break,
                _ => {}
            }
        }
        Ok(cat)
    }

    /// El paquete `ruta` ESTABLE más nuevo con un archivo para macOS de Apple Silicon (o para todos).
    pub fn elegir(&self, ruta: &str) -> Option<(&Paquete, &Archivo)> {
        self.paquetes
            .iter()
            .filter(|p| p.ruta == ruta && p.canal == "stable")
            .filter_map(|p| p.archivos.iter().find(|a| (a.so.is_empty() || a.so == "macosx") && (a.arq.is_empty() || a.arq == "aarch64")).map(|a| (p, a)))
            .max_by_key(|(p, _)| p.version)
    }

    pub fn licencia(&self, id: &str) -> Option<&str> {
        self.licencias.iter().find(|(i, _)| i == id).map(|(_, t)| t.as_str())
    }
}

/// La dirección completa de un archivo.
pub fn url_de(p: &Paquete, a: &Archivo) -> String {
    if a.url.starts_with("https://") {
        a.url.clone()
    } else {
        format!("{}{}", p.base, a.url)
    }
}

/// SHA-1 en hexadecimal.
pub fn sha1_hex(b: &[u8]) -> String {
    use sha1::Digest;
    let mut h = sha1::Sha1::new();
    h.update(b);
    h.finalize().iter().map(|x| format!("{x:02x}")).collect()
}

/// El texto de una licencia como lo guarda el SDK al leer el catálogo (`TrimStringAdapter` de la
/// biblioteca de repositorio de Android, leído con `javap` 2026-11-17): fuera los espacios y tabuladores que
/// siguen a un blanco; cada salto de línea SUELTO pasa a `salto`; las rachas de espacios, a `espacio`; y se
/// recorta. La marca de `licenses/<id>` es el SHA-1 de ESTO, no del texto crudo (que da otra suma).
fn normalizar_licencia(t: &str, salto: &str, espacio: &str) -> String {
    let c: Vec<char> = t.chars().collect();
    // 1. `(?<=\s)[ \t]*` → "" (la mirada atrás es sobre el texto ORIGINAL)
    let mut uno = String::with_capacity(t.len());
    for (i, &ch) in c.iter().enumerate() {
        if (ch == ' ' || ch == '\t') && i > 0 && c[i - 1].is_whitespace() {
            continue;
        }
        uno.push(ch);
    }
    // 2. `(?<!\n)\n(?!\n)` → salto
    let u: Vec<char> = uno.chars().collect();
    let mut dos = String::with_capacity(uno.len());
    for (i, &ch) in u.iter().enumerate() {
        if ch == '\n' && (i == 0 || u[i - 1] != '\n') && u.get(i + 1) != Some(&'\n') {
            dos.push_str(salto);
        } else {
            dos.push(ch);
        }
    }
    // 3. ` +` → espacio
    let mut tres = String::with_capacity(dos.len());
    let mut en_racha = false;
    for ch in dos.chars() {
        if ch == ' ' {
            if !en_racha {
                tres.push_str(espacio);
            }
            en_racha = true;
        } else {
            en_racha = false;
            tres.push(ch);
        }
    }
    // 4. trim() de Java: todo carácter <= ' '
    tres.trim_matches(|c: char| c <= ' ').to_string()
}

/// La suma que el SDK espera en `licenses/<id>` para el texto de una licencia del catálogo.
pub fn suma_de_licencia(texto: &str) -> String {
    sha1_hex(normalizar_licencia(texto, SALTO, ESPACIO).as_bytes())
}
const SALTO: &str = " ";
const ESPACIO: &str = " ";

/// Lo que se le enseña al humano ANTES de bajar (regla 2).
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct Ficha {
    pub ruta: String,
    pub nombre: String,
    pub version: String,
    pub publica: String,
    pub url: String,
    pub tamano: u64,
    pub destino: PathBuf,
    pub licencia: String,
}

/// Las fichas de lo que falta instalar de `rutas` en `sdk` (lo que ya está, no se baja de nuevo).
pub fn plan(cat: &Catalogo, rutas: &[&str], sdk: &Path) -> Result<Vec<Ficha>> {
    let mut v = Vec::new();
    for r in rutas {
        let (p, a) = cat.elegir(r).ok_or_else(|| anyhow!("{}", crate::tr!("el catálogo de Android no tiene «{r}» estable para esta Mac", "the Android catalogue has no stable «{r}» for this Mac")))?;
        let destino = sdk.join(r.replace(';', "/"));
        if destino.join("package.xml").is_file() || destino.join("source.properties").is_file() {
            continue;
        }
        v.push(Ficha { ruta: r.to_string(), nombre: p.nombre.clone(), version: format!("{}.{}.{}", p.version.0, p.version.1, p.version.2), publica: "Google".into(), url: url_de(p, a), tamano: a.tamano, destino, licencia: p.licencia.clone() });
    }
    Ok(v)
}

/// Una aceptación guardada (regla 4).
#[derive(Clone, Debug, PartialEq)]
pub struct Aceptada {
    pub licencia: String,
    /// SHA-1 del texto exacto que vio el humano: si el texto cambia, se pide otra vez.
    pub suma: String,
    /// Segundos desde 1970.
    pub fecha: i64,
    pub paquete: String,
}

/// La constancia: lo aceptado y lo instalado por GOTV (para desinstalar sólo eso, regla 6).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Constancia {
    pub aceptadas: Vec<Aceptada>,
    pub instalados: Vec<PathBuf>,
}

const TIPO: &[u8; 4] = b"LICE";
const VERSION: u8 = 1;

impl Constancia {
    pub fn ruta() -> PathBuf {
        crate::paths::home().join(".config").join("gotv").join("licencias.gotv")
    }

    pub fn leer(ruta: &Path) -> Result<Constancia> {
        let Ok(b) = std::fs::read(ruta) else { return Ok(Constancia::default()) };
        let (mut l, _) = crate::programar::codec::Lector::nuevo(&b, TIPO, VERSION)?;
        let mut c = Constancia::default();
        for _ in 0..l.u32()? {
            c.aceptadas.push(Aceptada { licencia: l.texto()?, suma: l.texto()?, fecha: l.i64()?, paquete: l.texto()? });
        }
        c.instalados = l.textos()?.into_iter().map(PathBuf::from).collect();
        Ok(c)
    }

    pub fn guardar(&self, ruta: &Path) -> Result<()> {
        let mut e = crate::programar::codec::Escritor::nuevo(TIPO, VERSION);
        e.u32(self.aceptadas.len() as u32);
        for a in &self.aceptadas {
            e.texto(&a.licencia);
            e.texto(&a.suma);
            e.i64(a.fecha);
            e.texto(&a.paquete);
        }
        e.textos(&self.instalados.iter().map(|p| p.display().to_string()).collect::<Vec<_>>());
        if let Some(d) = ruta.parent() {
            std::fs::create_dir_all(d)?;
        }
        crate::programar::codec::guardar(ruta, &e.bytes())
    }

    /// ¿Esta licencia, con ESTE texto, ya la aceptó el humano?
    pub fn acepto(&self, id: &str, texto: &str) -> bool {
        let s = sha1_hex(texto.as_bytes());
        self.aceptadas.iter().any(|a| a.licencia == id && a.suma == s)
    }

    /// El humano aceptó UNA licencia para UN paquete (la llama la ventana con su clic; el modelo no tiene
    /// herramienta para esto). Deja además la marca donde la busca el SDK (`licenses/<id>`: la suma, una
    /// por línea, sin borrar las que ya hubiera).
    pub fn aceptar(&mut self, id: &str, texto: &str, paquete: &str, sdk: &Path) -> Result<()> {
        let suma = sha1_hex(texto.as_bytes());
        if !self.acepto(id, texto) {
            let fecha = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
            self.aceptadas.push(Aceptada { licencia: id.into(), suma: suma.clone(), fecha, paquete: paquete.into() });
        }
        let dir = sdk.join("licenses");
        std::fs::create_dir_all(&dir)?;
        let f = dir.join(id);
        let ya = std::fs::read_to_string(&f).unwrap_or_default();
        let marca = suma_de_licencia(texto);
        if !ya.lines().any(|l| l.trim() == marca) {
            let mut t = ya.trim_end().to_string();
            t.push('\n');
            t.push_str(&marca);
            std::fs::write(&f, t)?;
        }
        Ok(())
    }
}

/// Abre el `.zip` bajado en `destino`: su ÚNICA carpeta de arriba pasa a ser `destino` (los paquetes de
/// Android traen `android-36/`, `platform-tools/`… con nombres que no siempre son el de su ruta).
pub fn abrir_zip(zip: &Path, destino: &Path) -> Result<()> {
    let tmp = destino.with_extension("gotv-abriendo");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp)?;
    let mut z = zip::ZipArchive::new(std::fs::File::open(zip)?).with_context(|| zip.display().to_string())?;
    for i in 0..z.len() {
        let mut e = z.by_index(i)?;
        let Some(rel) = e.enclosed_name() else { bail!("{}", crate::tr!("{}: ruta peligrosa en el zip (sale de su carpeta): no lo abro", "{}: dangerous path in the zip (escapes its folder): not opening it", zip.display())) };
        let a = tmp.join(rel);
        if e.is_dir() {
            std::fs::create_dir_all(&a)?;
            continue;
        }
        if let Some(d) = a.parent() {
            std::fs::create_dir_all(d)?;
        }
        let mut f = std::fs::File::create(&a)?;
        std::io::copy(&mut e, &mut f)?;
        #[cfg(unix)]
        if let Some(m) = e.unix_mode() {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&a, std::fs::Permissions::from_mode(m & 0o777))?;
        }
    }
    let arriba: Vec<PathBuf> = std::fs::read_dir(&tmp)?.flatten().map(|e| e.path()).collect();
    let fuente = if arriba.len() == 1 && arriba[0].is_dir() { arriba[0].clone() } else { tmp.clone() };
    if let Some(d) = destino.parent() {
        std::fs::create_dir_all(d)?;
    }
    if destino.exists() {
        bail!("{}", crate::tr!("{} ya existe: no lo piso", "{} already exists: I won't overwrite it", destino.display()));
    }
    std::fs::rename(&fuente, destino)?;
    let _ = std::fs::remove_dir_all(&tmp);
    Ok(())
}

/// El `package.xml` local con que el SDK (y Gradle) reconocen el paquete instalado.
pub fn package_xml(p: &Paquete) -> String {
    let (a, b, c) = p.version;
    let esc = |s: &str| s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;");
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<ns2:repository xmlns:ns2=\"http://schemas.android.com/repository/android/common/02\"><localPackage path=\"{}\" obsolete=\"false\"><revision><major>{a}</major><minor>{b}</minor><micro>{c}</micro></revision><display-name>{}</display-name><uses-license ref=\"{}\"/></localPackage></ns2:repository>\n",
        esc(&p.ruta),
        esc(&p.nombre),
        esc(&p.licencia)
    )
}

/// Baja, comprueba (tamaño y SHA-1: regla 5), abre y registra UN paquete. Exige que su licencia esté
/// aceptada con el texto actual. `avance(bajado, total)`.
pub async fn instalar(cat: &Catalogo, ruta: &str, sdk: &Path, constancia: &mut Constancia, avance: &(dyn Fn(u64, u64) + Sync)) -> Result<PathBuf> {
    let (p, a) = cat.elegir(ruta).ok_or_else(|| anyhow!("{}", crate::tr!("el catálogo no tiene «{ruta}»", "the catalogue has no «{ruta}»")))?;
    let texto = cat.licencia(&p.licencia).ok_or_else(|| anyhow!("{}", crate::tr!("el catálogo no trae el texto de la licencia «{}»", "the catalogue lacks the text of licence «{}»", p.licencia)))?;
    if !constancia.acepto(&p.licencia, texto) {
        bail!("{}", crate::tr!("falta que el usuario acepte «{}» para {ruta}", "the user has not accepted «{}» for {ruta}", p.licencia));
    }
    let destino = sdk.join(ruta.replace(';', "/"));
    let url = url_de(p, a);
    let dir = sdk.join(".gotv-bajando");
    std::fs::create_dir_all(&dir)?;
    let zip = dir.join(format!("{}.zip", sha1_hex(url.as_bytes())));
    let mut r = reqwest::get(&url).await?.error_for_status()?;
    let mut f = tokio::fs::File::create(&zip).await?;
    let mut h = <sha1::Sha1 as sha1::Digest>::new();
    let mut n = 0u64;
    while let Some(trozo) = r.chunk().await? {
        sha1::Digest::update(&mut h, &trozo);
        tokio::io::AsyncWriteExt::write_all(&mut f, &trozo).await?;
        n += trozo.len() as u64;
        avance(n, a.tamano);
    }
    tokio::io::AsyncWriteExt::flush(&mut f).await?;
    drop(f);
    let suma: String = sha1::Digest::finalize(h).iter().map(|x| format!("{x:02x}")).collect();
    if n != a.tamano || suma != a.sha1 {
        let _ = std::fs::remove_file(&zip);
        bail!("{}", crate::tr!("{url}: lo bajado no casa con lo publicado (tamaño {n} de {}, SHA-1 {suma} en vez de {}): no lo instalo", "{url}: the download does not match what was published (size {n} of {}, SHA-1 {suma} instead of {}): not installing it", a.tamano, a.sha1));
    }
    let d2 = destino.clone();
    let z2 = zip.clone();
    tokio::task::spawn_blocking(move || abrir_zip(&z2, &d2)).await??;
    let _ = std::fs::remove_file(&zip);
    let _ = std::fs::remove_dir(&dir);
    std::fs::write(destino.join("package.xml"), package_xml(p))?;
    if !constancia.instalados.contains(&destino) {
        constancia.instalados.push(destino.clone());
    }
    Ok(destino)
}

/// Dónde instala GOTV el SDK de Android: el que ya exista (no se duplica, F18.I paso 4) o el propio en
/// `~/Library/Application Support/gotv/android-sdk`.
pub fn sdk_destino() -> PathBuf {
    super::android::sdk().unwrap_or_else(|| crate::paths::home().join("Library/Application Support/gotv/android-sdk"))
}

/// Los dos catálogos oficiales (paquetes e imágenes del sistema) en uno, guardados una hora en memoria.
pub async fn catalogo_vivo() -> Result<Catalogo> {
    static CACHE: std::sync::OnceLock<tokio::sync::Mutex<Option<(std::time::Instant, Catalogo)>>> = std::sync::OnceLock::new();
    let m = CACHE.get_or_init(Default::default);
    let mut g = m.lock().await;
    if let Some((t, c)) = g.as_ref() {
        if t.elapsed() < std::time::Duration::from_secs(3600) {
            return Ok(c.clone());
        }
    }
    let x = reqwest::get(CATALOGO).await?.error_for_status()?.text().await?;
    let mut c = Catalogo::leer(&x, "https://dl.google.com/android/repository/")?;
    let xi = reqwest::get(CATALOGO_IMAGENES).await?.error_for_status()?.text().await?;
    let ci = Catalogo::leer(&xi, "https://dl.google.com/android/repository/sys-img/google_apis/")?;
    c.paquetes.extend(ci.paquetes);
    for (id, t) in ci.licencias {
        if c.licencia(&id).is_none() {
            c.licencias.push((id, t));
        }
    }
    *g = Some((std::time::Instant::now(), c.clone()));
    Ok(c)
}

/// Una licencia tal como la ve el humano en Configuración (regla 2: nombre exacto y TEXTO entero).
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct LicenciaVista {
    pub id: String,
    /// Nombre exacto del acuerdo (su primera línea con contenido, p. ej. «Terms and Conditions…»).
    pub titulo: String,
    pub texto: String,
    pub aceptada: bool,
    /// Los paquetes del plan que la exigen.
    pub paquetes: Vec<String>,
}

/// Lo que muestra la sección «Dispositivos → Android» de Configuración.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct EstadoAndroid {
    /// El SDK que hay (o `None`).
    pub sdk: Option<PathBuf>,
    /// Dónde se instalaría.
    pub destino: PathBuf,
    pub publica: String,
    /// Lo que falta del plan de un clic, con su origen y tamaño.
    pub fichas: Vec<Ficha>,
    pub total: u64,
    pub licencias: Vec<LicenciaVista>,
    /// Lo que instaló GOTV en ESTE SDK, como paquete (`platforms;android-36`): lo único que se puede
    /// desinstalar desde aquí.
    pub instalados: Vec<String>,
    /// Los dispositivos virtuales que hay.
    pub dispositivos: Vec<String>,
}

/// El estado para la ventana, con el catálogo dado (las pruebas pasan uno propio).
pub fn estado_con(cat: &Catalogo, sdk_dir: &Path, hay_sdk: bool, constancia: &Constancia, dispositivos: Vec<String>) -> Result<EstadoAndroid> {
    let fichas = plan(cat, DE_UN_CLIC, sdk_dir)?;
    let mut licencias: Vec<LicenciaVista> = Vec::new();
    for f in &fichas {
        if let Some(l) = licencias.iter_mut().find(|l| l.id == f.licencia) {
            l.paquetes.push(f.ruta.clone());
            continue;
        }
        let texto = cat.licencia(&f.licencia).unwrap_or("").to_string();
        let titulo = texto.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or(&f.licencia).to_string();
        licencias.push(LicenciaVista { id: f.licencia.clone(), titulo, aceptada: constancia.acepto(&f.licencia, &texto), texto, paquetes: vec![f.ruta.clone()] });
    }
    Ok(EstadoAndroid {
        sdk: hay_sdk.then(|| sdk_dir.to_path_buf()),
        destino: sdk_dir.to_path_buf(),
        publica: "Google".into(),
        total: fichas.iter().map(|f| f.tamano).sum(),
        fichas,
        licencias,
        instalados: constancia.instalados.iter().filter_map(|p| p.strip_prefix(sdk_dir).ok()).map(|r| r.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect::<Vec<_>>().join(";")).collect(),
        dispositivos,
    })
}

/// El estado real (catálogo vivo, SDK y constancia de esta Mac).
pub async fn estado() -> Result<EstadoAndroid> {
    let cat = catalogo_vivo().await?;
    let destino = sdk_destino();
    let constancia = Constancia::leer(&Constancia::ruta())?;
    let dispositivos = avds_de(&super::android::carpeta_avd());
    estado_con(&cat, &destino, super::android::sdk().is_some(), &constancia, dispositivos)
}

/// Los dispositivos virtuales (los `<nombre>.ini` de la carpeta).
pub fn avds_de(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir).into_iter().flatten().flatten().filter_map(|e| e.file_name().to_str().and_then(|n| n.strip_suffix(".ini")).map(String::from)).collect();
    v.sort();
    v
}

/// El humano pulsó «Acepto» en la licencia `id`: el TEXTO sale del catálogo, nunca de la ventana.
pub async fn aceptar_licencia(id: &str) -> Result<()> {
    let cat = catalogo_vivo().await?;
    let texto = cat.licencia(id).ok_or_else(|| anyhow!("{}", crate::tr!("el catálogo no tiene la licencia «{id}»", "the catalogue has no licence «{id}»")))?.to_string();
    let paquetes: Vec<String> = cat.paquetes.iter().filter(|p| p.licencia == id && DE_UN_CLIC.contains(&p.ruta.as_str())).map(|p| p.ruta.clone()).collect();
    let ruta = Constancia::ruta();
    let mut c = Constancia::leer(&ruta)?;
    c.aceptar(id, &texto, &paquetes.join(","), &sdk_destino())?;
    c.guardar(&ruta)
}

/// Instala UN paquete del plan (la licencia ya aceptada) y guarda la constancia.
pub async fn instalar_paquete(ruta_paquete: &str, avance: &(dyn Fn(u64, u64) + Sync)) -> Result<PathBuf> {
    if !DE_UN_CLIC.contains(&ruta_paquete) {
        bail!("{}", crate::tr!("«{ruta_paquete}» no está en lo que se instala desde aquí", "«{ruta_paquete}» is not among what is installed from here"));
    }
    let cat = catalogo_vivo().await?;
    let ruta = Constancia::ruta();
    let mut c = Constancia::leer(&ruta)?;
    let d = instalar(&cat, ruta_paquete, &sdk_destino(), &mut c, avance).await;
    c.guardar(&ruta)?;
    d
}

/// Desinstala un paquete que instaló GOTV y guarda la constancia.
pub fn desinstalar_paquete(ruta_paquete: &str) -> Result<()> {
    let ruta = Constancia::ruta();
    let mut c = Constancia::leer(&ruta)?;
    desinstalar(ruta_paquete, &sdk_destino(), &mut c)?;
    c.guardar(&ruta)
}

/// Lo que se sabe de Xcode en esta Mac (F18.I, Xcode pasos 2–5). GOTV nunca toca la cuenta de Apple.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct EstadoXcode {
    /// `xcode-select -p`.
    pub carpeta: Option<String>,
    /// `Xcode 27.2`.
    pub version: Option<String>,
    /// Plataformas de simulador instaladas (`iOS 27.0`).
    pub simuladores: Vec<String>,
    /// La versión de idb que GOTV bajó (leer y tocar el simulador), si está.
    pub idb: Option<String>,
    /// La página de Xcode en la Mac App Store (paso 1: la descarga y su licencia las maneja Apple).
    pub tienda: String,
    /// Paso 3: licencia aceptada y primer arranque hecho (y dónde está el acuerdo para mostrarlo).
    pub preparacion: Option<super::xcode::Preparacion>,
    /// Paso 5: la versión de idb que GOTV instala (fijada) y su tamaño y licencia.
    pub idb_fijado: String,
    pub idb_tamano: u64,
    pub idb_licencia: String,
}

pub async fn estado_xcode() -> EstadoXcode {
    let correr = |p: &'static str, a: &'static [&'static str]| async move {
        let o = crate::entorno::comando_tokio(p).args(a).output().await.ok()?;
        o.status.success().then(|| String::from_utf8_lossy(&o.stdout).trim().to_string()).filter(|s| !s.is_empty())
    };
    let carpeta = correr("xcode-select", &["-p"]).await;
    let version = correr("xcodebuild", &["-version"]).await.and_then(|v| v.lines().next().map(String::from));
    let simuladores = correr("xcrun", &["simctl", "list", "runtimes"]).await.map(|t| t.lines().filter(|l| l.starts_with("iOS ")).filter_map(|l| l.split(" (").next().map(String::from)).collect()).unwrap_or_default();
    let idb_dir = crate::paths::home().join("Library/Application Support/gotv/idb");
    let mut idb: Vec<String> = std::fs::read_dir(&idb_dir).into_iter().flatten().flatten().filter(|e| e.path().join("bin/idb_companion").is_file()).map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    idb.sort();
    let preparacion = if version.is_some() { Some(super::xcode::preparacion().await) } else { None };
    EstadoXcode {
        carpeta,
        version,
        simuladores,
        idb: idb.pop(),
        tienda: "macappstore://apps.apple.com/app/xcode/id497799835".into(),
        preparacion,
        idb_fijado: super::xcode::IDB.version.into(),
        idb_tamano: super::xcode::IDB_TAMANO,
        idb_licencia: super::xcode::IDB_LICENCIA.into(),
    }
}

/// Desinstala un paquete SÓLO si lo instaló GOTV (regla 6).
pub fn desinstalar(ruta: &str, sdk: &Path, constancia: &mut Constancia) -> Result<()> {
    let destino = sdk.join(ruta.replace(';', "/"));
    if !constancia.instalados.contains(&destino) {
        bail!("{}", crate::tr!("{} no lo instaló GOTV: no lo toco", "{} was not installed by GOTV: I won't touch it", destino.display()));
    }
    if destino.exists() {
        std::fs::remove_dir_all(&destino)?;
    }
    constancia.instalados.retain(|p| p != &destino);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Trozo FIEL del catálogo real (bajado 2026-11-17): canales, una licencia, el emulador en dos canales y
    /// cuatro sistemas, y una plataforma con un archivo para todos.
    const XML: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<sdk:sdk-repository xmlns:sdk="http://schemas.android.com/sdk/android/repo/repository2/03" xmlns:common="http://schemas.android.com/repository/android/common/02" xmlns:generic="http://schemas.android.com/repository/android/generic/02" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">
  <license id="android-sdk-license" type="text">Terms and Conditions

This is the Android &quot;SDK&quot; License Agreement
</license>
  <channel id="channel-0">stable</channel>
  <channel id="channel-1">beta</channel>
  <channel id="channel-2">dev</channel>
  <remotePackage path="emulator">
    <type-details xsi:type="generic:genericDetailsType"/>
    <revision><major>37</major><minor>4</minor><micro>1</micro></revision>
    <display-name>Android Emulator</display-name>
    <uses-license ref="android-sdk-license"/>
    <channelRef ref="channel-2"/>
    <archives>
      <archive><complete><size>424581828</size><checksum type="sha1">893994ff1590e8a53a6b9efad62c04714cde2dcf</checksum><url>emulator-darwin_aarch64-16916650.zip</url></complete><host-os>macosx</host-os><host-arch>aarch64</host-arch></archive>
    </archives>
  </remotePackage>
  <remotePackage path="emulator">
    <type-details xsi:type="generic:genericDetailsType"/>
    <revision><major>36</major><minor>2</minor><micro>12</micro></revision>
    <display-name>Android Emulator</display-name>
    <uses-license ref="android-sdk-license"/>
    <channelRef ref="channel-0"/>
    <archives>
      <archive><complete><size>354492534</size><checksum type="sha1">4fabd9f4550607092819d4e39811e2cb8694135c</checksum><url>emulator-linux_x64.zip</url></complete><host-os>linux</host-os><host-arch>x64</host-arch></archive>
      <archive><complete><size>492015961</size><checksum type="sha1">71798999f8debb685a12e5ecb13772aca086e575</checksum><url>emulator-darwin_x64.zip</url></complete><host-os>macosx</host-os><host-arch>x64</host-arch></archive>
      <archive><complete><size>424000000</size><checksum type="sha1">AAAA94ff1590e8a53a6b9efad62c04714cde2dcf</checksum><url>emulator-darwin_aarch64.zip</url></complete><host-os>macosx</host-os><host-arch>aarch64</host-arch></archive>
    </archives>
  </remotePackage>
  <remotePackage path="platforms;android-36">
    <type-details xsi:type="sdk:platformDetailsType"><api-level>36</api-level></type-details>
    <revision><major>2</major></revision>
    <display-name>Android SDK Platform 36</display-name>
    <uses-license ref="android-sdk-license"/>
    <channelRef ref="channel-0"/>
    <archives>
      <archive><complete><size>65878410</size><checksum type="sha1">2c1a80dd4d9f7d0e6dd336ec603d9b5c55a6f576</checksum><url>platform-36_r02.zip</url></complete></archive>
    </archives>
  </remotePackage>
</sdk:sdk-repository>"#;

    /// Lee canales, licencia (texto EXACTO, con sus entidades resueltas), versiones y archivos; elige el
    /// ESTABLE para Apple Silicon aunque haya uno más nuevo en «dev».
    #[test]
    fn el_catalogo_y_la_eleccion() {
        let c = Catalogo::leer(XML, "https://dl.google.com/android/repository/").unwrap();
        assert_eq!(c.paquetes.len(), 3);
        assert_eq!(c.licencia("android-sdk-license").unwrap(), "Terms and Conditions\n\nThis is the Android \"SDK\" License Agreement\n");
        let (p, a) = c.elegir("emulator").unwrap();
        assert_eq!((p.version, p.canal.as_str(), a.url.as_str(), a.sha1.as_str()), ((36, 2, 12), "stable", "emulator-darwin_aarch64.zip", "aaaa94ff1590e8a53a6b9efad62c04714cde2dcf"));
        let (p, a) = c.elegir("platforms;android-36").unwrap();
        assert_eq!((p.nombre.as_str(), p.version, a.tamano), ("Android SDK Platform 36", (2, 0, 0), 65878410));
        assert_eq!(url_de(p, a), "https://dl.google.com/android/repository/platform-36_r02.zip");
        assert!(c.elegir("no-existe").is_none());
    }

    /// El plan dice quién publica, de dónde, cuánto y qué licencia; lo ya instalado no entra.
    #[test]
    fn el_plan_salta_lo_instalado() {
        let c = Catalogo::leer(XML, "https://dl.google.com/android/repository/").unwrap();
        let sdk = crate::CarpetaPrueba::nueva("gotv-instalador-plan");
        let v = plan(&c, &["emulator", "platforms;android-36"], &sdk).unwrap();
        assert_eq!(v.len(), 2);
        assert_eq!((v[1].publica.as_str(), v[1].licencia.as_str(), v[1].destino.clone()), ("Google", "android-sdk-license", sdk.join("platforms/android-36")));
        std::fs::create_dir_all(sdk.join("emulator")).unwrap();
        std::fs::write(sdk.join("emulator/package.xml"), "x").unwrap();
        assert_eq!(plan(&c, &["emulator", "platforms;android-36"], &sdk).unwrap().len(), 1);
        assert!(plan(&c, &["nada"], &sdk).is_err());
    }

    /// La constancia: una licencia por clic, con la suma de su texto (si cambia, se pide otra vez), la
    /// marca del SDK sin borrar las que había, y vuelve igual de disco (formato propio, no JSON).
    #[test]
    fn la_constancia_de_licencias() {
        let d = crate::CarpetaPrueba::nueva("gotv-instalador-constancia");
        let sdk = d.join("sdk");
        std::fs::create_dir_all(sdk.join("licenses")).unwrap();
        std::fs::write(sdk.join("licenses/android-sdk-license"), "\n24333f8a63b6825ea9c5514f83c2829b004d1fee").unwrap();
        let mut c = Constancia::default();
        assert!(!c.acepto("android-sdk-license", "texto v1"));
        c.aceptar("android-sdk-license", "texto v1", "emulator", &sdk).unwrap();
        assert!(c.acepto("android-sdk-license", "texto v1") && !c.acepto("android-sdk-license", "texto v2"), "si el texto cambia, se pide otra vez");
        let marca = std::fs::read_to_string(sdk.join("licenses/android-sdk-license")).unwrap();
        assert!(marca.contains("24333f8a63b6825ea9c5514f83c2829b004d1fee") && marca.contains(&suma_de_licencia("texto v1")), "{marca}");
        c.instalados.push(sdk.join("emulator"));
        let r = d.join("licencias.gotv");
        c.guardar(&r).unwrap();
        assert_eq!(&std::fs::read(&r).unwrap()[..4], b"GOTV");
        assert_eq!(Constancia::leer(&r).unwrap(), c);
        assert_eq!(Constancia::leer(&d.join("no-hay")).unwrap(), Constancia::default());
    }

    /// El zip: su única carpeta de arriba pasa a ser el destino (con permisos), lo existente no se pisa, y
    /// sólo se desinstala lo que instaló GOTV.
    #[test]
    fn abrir_y_desinstalar() {
        use std::io::Write;
        let d = crate::CarpetaPrueba::nueva("gotv-instalador-zip");
        let z = d.join("p.zip");
        let mut w = zip::ZipWriter::new(std::fs::File::create(&z).unwrap());
        let o = zip::write::SimpleFileOptions::default().unix_permissions(0o755);
        w.start_file("android-16/bin/herramienta", o).unwrap();
        w.write_all(b"#!/bin/sh\n").unwrap();
        w.start_file("android-16/source.properties", zip::write::SimpleFileOptions::default()).unwrap();
        w.write_all(b"Pkg.Revision=2").unwrap();
        w.finish().unwrap();
        let sdk = d.join("sdk");
        let destino = sdk.join("platforms/android-36");
        abrir_zip(&z, &destino).unwrap();
        assert!(destino.join("source.properties").is_file());
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(destino.join("bin/herramienta")).unwrap().permissions().mode() & 0o777, 0o755);
        assert!(abrir_zip(&z, &destino).is_err(), "no se pisa");
        let mut c = Constancia::default();
        assert!(desinstalar("platforms;android-36", &sdk, &mut c).is_err(), "no lo instaló GOTV");
        assert!(destino.exists());
        c.instalados.push(destino.clone());
        desinstalar("platforms;android-36", &sdk, &mut c).unwrap();
        assert!(!destino.exists() && c.instalados.is_empty());
    }

    /// De verdad, con la red: los dos catálogos vivos de Google, el plan de un clic con sus tamaños, y
    /// `platform-tools` instalado en un SDK temporal (bajado, SHA-1 comprobado, abierto, `adb` que corre) y
    /// desinstalado. `cargo test -p gotv-core catalogo_e_instalacion_reales -- --ignored --nocapture`
    #[tokio::test]
    #[ignore]
    async fn catalogo_e_instalacion_reales() {
        let t = std::time::Instant::now();
        let x = reqwest::get(CATALOGO).await.unwrap().text().await.unwrap();
        let c = Catalogo::leer(&x, "https://dl.google.com/android/repository/").unwrap();
        let xi = reqwest::get(CATALOGO_IMAGENES).await.unwrap().text().await.unwrap();
        let ci = Catalogo::leer(&xi, "https://dl.google.com/android/repository/sys-img/google_apis/").unwrap();
        eprintln!("catálogos: {} + {} paquetes, {} licencias, {:.1} s", c.paquetes.len(), ci.paquetes.len(), c.licencias.len(), t.elapsed().as_secs_f32());
        let mut todo = c.clone();
        todo.paquetes.extend(ci.paquetes.clone());
        for (id, l) in &ci.licencias {
            if todo.licencia(id).is_none() {
                todo.licencias.push((id.clone(), l.clone()));
            }
        }
        let vacio = crate::CarpetaPrueba::nueva("gotv-instalador-real");
        let fichas = plan(&todo, DE_UN_CLIC, &vacio.join("sdk")).unwrap();
        let mut total = 0;
        for f in &fichas {
            total += f.tamano;
            eprintln!("{} {} · {} · {:.0} MB · {} · {}", f.nombre, f.version, f.url, f.tamano as f64 / 1e6, f.licencia, sha1_hex(todo.licencia(&f.licencia).unwrap_or("").as_bytes()));
        }
        eprintln!("total {:.2} GB", total as f64 / 1e9);
        // la marca que el SDK acepta para el texto de HOY (la de un SDK donde el plugin de Gradle instaló
        // build-tools sin quejarse: 24333f8a…)
        let lic = todo.licencia("android-sdk-license").unwrap();
        for (s, e) in [("", ""), ("", " "), (" ", ""), (" ", " ")] {
            eprintln!("salto «{s}» espacio «{e}»: {}", sha1_hex(normalizar_licencia(lic, s, e).as_bytes()));
        }
        assert_eq!(suma_de_licencia(lic), "24333f8a63b6825ea9c5514f83c2829b004d1fee");
        assert_eq!(fichas.len(), 4);
        assert!(fichas.iter().all(|f| f.url.starts_with("https://dl.google.com/android/repository/") && f.tamano > 1_000_000));
        // una instalación de verdad, pequeña
        let sdk = vacio.join("sdk");
        let mut k = Constancia::default();
        let (p, _) = todo.elegir("platform-tools").unwrap();
        k.aceptar(&p.licencia, todo.licencia(&p.licencia).unwrap(), "platform-tools", &sdk).unwrap();
        let t = std::time::Instant::now();
        let d = instalar(&todo, "platform-tools", &sdk, &mut k, &|_, _| {}).await.unwrap();
        eprintln!("platform-tools instalado en {:.1} s: {}", t.elapsed().as_secs_f32(), d.display());
        let o = std::process::Command::new(d.join("adb")).arg("version").output().unwrap();
        assert!(String::from_utf8_lossy(&o.stdout).contains("Android Debug Bridge"), "{o:?}");
        assert!(d.join("package.xml").is_file() && k.instalados == vec![d.clone()]);
        desinstalar("platform-tools", &sdk, &mut k).unwrap();
        assert!(!d.exists());
    }

    /// Contra un servidor falso (axum): con la suma publicada, se instala; si lo servido no casa con la suma
    /// (archivo cambiado en el camino), NO se instala ni queda nada a medias; aceptar una licencia no acepta
    /// otra.
    #[tokio::test]
    async fn suma_que_no_casa_no_se_instala() {
        use std::io::Write;
        let mut zb = Vec::new();
        {
            let mut w = zip::ZipWriter::new(std::io::Cursor::new(&mut zb));
            w.start_file("platform-tools/adb", zip::write::SimpleFileOptions::default()).unwrap();
            w.write_all(b"adb falso").unwrap();
            w.finish().unwrap();
        }
        let bueno = zb.clone();
        let app = axum::Router::new().route("/bueno.zip", axum::routing::get(move || { let b = bueno.clone(); async move { b } })).route("/cambiado.zip", axum::routing::get(|| async { b"PK no es lo publicado".to_vec() }));
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}/", l.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
        let paquete = |ruta: &str, url: &str, tamano: usize, suma: &str, lic: &str| format!("<remotePackage path=\"{ruta}\"><revision><major>1</major></revision><display-name>{ruta}</display-name><uses-license ref=\"{lic}\"/><channelRef ref=\"channel-0\"/><archives><archive><complete><size>{tamano}</size><checksum type=\"sha1\">{suma}</checksum><url>{url}</url></complete></archive></archives></remotePackage>");
        let xml = format!(
            "<sdk:sdk-repository xmlns:sdk=\"x\"><license id=\"lic-a\" type=\"text\">A</license><license id=\"lic-b\" type=\"text\">B</license><channel id=\"channel-0\">stable</channel>{}{}</sdk:sdk-repository>",
            paquete("platform-tools", "bueno.zip", zb.len(), &sha1_hex(&zb), "lic-a"),
            paquete("emulator", "cambiado.zip", zb.len(), &sha1_hex(&zb), "lic-b")
        );
        let c = Catalogo::leer(&xml, &base).unwrap();
        let sdk = crate::CarpetaPrueba::nueva("gotv-instalador-falso");
        let mut k = Constancia::default();
        k.aceptar("lic-a", "A", "platform-tools", &sdk).unwrap();
        assert!(!k.acepto("lic-b", "B"), "aceptar una licencia no acepta otra");
        let e = instalar(&c, "emulator", &sdk, &mut k, &|_, _| {}).await.unwrap_err().to_string();
        assert!(e.contains("lic-b"), "{e}");
        let d = instalar(&c, "platform-tools", &sdk, &mut k, &|_, _| {}).await.unwrap();
        assert_eq!(std::fs::read(d.join("adb")).unwrap(), b"adb falso");
        k.aceptar("lic-b", "B", "emulator", &sdk).unwrap();
        let e = instalar(&c, "emulator", &sdk, &mut k, &|_, _| {}).await.unwrap_err().to_string();
        assert!(e.contains("no casa"), "{e}");
        assert!(!sdk.join("emulator").exists() && !sdk.join(".gotv-bajando").read_dir().map(|mut r| r.next().is_some()).unwrap_or(false), "nada a medias");
        assert_eq!(k.instalados, vec![d]);
    }

    /// El estado de Configuración: lo que falta del plan con su tamaño total, UNA entrada por licencia (con
    /// el texto ENTERO, su título y qué paquetes la exigen), si ya se aceptó con ese texto, y los
    /// dispositivos virtuales.
    #[test]
    fn el_estado_para_configuracion() {
        let mut c = Catalogo::leer(XML, "https://dl.google.com/android/repository/").unwrap();
        // el plan de un clic con lo que trae el trozo del catálogo
        for r in ["platform-tools", "system-images;android-36;google_apis;arm64-v8a"] {
            let mut p = c.paquetes[2].clone();
            p.ruta = r.into();
            c.paquetes.push(p);
        }
        let d = crate::CarpetaPrueba::nueva("gotv-instalador-estado");
        let sdk = d.join("sdk");
        let avd = d.join("avd");
        std::fs::create_dir_all(&avd).unwrap();
        std::fs::write(avd.join("gotv-pixel.ini"), "x").unwrap();
        let mut k = Constancia::default();
        let e = estado_con(&c, &sdk, false, &k, avds_de(&avd)).unwrap();
        assert_eq!((e.sdk.clone(), e.fichas.len(), e.licencias.len(), e.dispositivos.clone()), (None, 4, 1, vec!["gotv-pixel".to_string()]));
        let l = &e.licencias[0];
        assert_eq!((l.id.as_str(), l.titulo.as_str(), l.aceptada, l.paquetes.len()), ("android-sdk-license", "Terms and Conditions", false, 4));
        assert!(l.texto.contains("License Agreement"), "el texto entero, no un resumen");
        assert_eq!(e.total, e.fichas.iter().map(|f| f.tamano).sum::<u64>());
        k.aceptar("android-sdk-license", c.licencia("android-sdk-license").unwrap(), "x", &sdk).unwrap();
        assert!(estado_con(&c, &sdk, true, &k, vec![]).unwrap().licencias[0].aceptada);
        // lo instalado sale como PAQUETE (lo que pide desinstalar), y sólo lo de ESTE SDK
        k.instalados = vec![sdk.join("platforms/android-36"), d.join("otro-sdk/emulator")];
        assert_eq!(estado_con(&c, &sdk, true, &k, vec![]).unwrap().instalados, ["platforms;android-36"]);
    }

    /// De verdad y sólo leyendo: el estado de esta Mac para Configuración (catálogo vivo, el SDK que haya, la
    /// constancia) y el de Xcode. `cargo test -p gotv-core estado_real -- --ignored --nocapture`
    #[tokio::test]
    #[ignore]
    async fn estado_real() {
        let e = estado().await.unwrap();
        eprintln!("SDK {:?} · destino {} · faltan {} ({:.2} GB) · licencias {:?} · AVD {:?}", e.sdk, e.destino.display(), e.fichas.len(), e.total as f64 / 1e9, e.licencias.iter().map(|l| (&l.id, l.aceptada)).collect::<Vec<_>>(), e.dispositivos);
        let x = estado_xcode().await;
        eprintln!("{x:?}");
        assert!(x.version.is_some_and(|v| v.starts_with("Xcode")) && !x.simuladores.is_empty());
    }

    /// Instalar sin la licencia aceptada no baja nada.
    #[tokio::test]
    async fn sin_aceptar_no_se_baja() {
        let c = Catalogo::leer(XML, "http://127.0.0.1:9/").unwrap();
        let sdk = crate::CarpetaPrueba::nueva("gotv-instalador-sin-aceptar");
        let e = instalar(&c, "platforms;android-36", &sdk, &mut Constancia::default(), &|_, _| {}).await.unwrap_err().to_string();
        assert!(e.contains("android-sdk-license"), "{e}");
    }
}
