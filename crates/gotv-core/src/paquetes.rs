//! Paquetes inventados (Ola D8b): un modelo pequeño escribe `npm install react-super-forms`
//! con toda la confianza. Ese nombre puede NO existir (falla la instalación) o, peor, existir
//! porque alguien lo registró para aprovechar la alucinación (*slopsquatting*). Antes de correr
//! un comando que instala, cada nombre se busca en su registro real (npm, PyPI, crates.io).
//! Sólo se bloquea lo que el registro CONFIRMA que no existe (404); si la red falla, pasa.

use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Registro {
    Npm,
    PyPi,
    Crates,
}

impl Registro {
    pub fn nombre(self) -> &'static str {
        match self {
            Registro::Npm => "npm",
            Registro::PyPi => "PyPI",
            Registro::Crates => "crates.io",
        }
    }
}

/// Nombre sin versión ni extras: `react@18` → `react`, `@a/b@1` → `@a/b`,
/// `requests[socks]>=2` → `requests`, `serde@1.0` → `serde`.
fn limpio(t: &str, r: Registro) -> Option<String> {
    let t = t.trim().trim_matches(|c| c == '\'' || c == '"');
    if t.is_empty()
        || t.starts_with('-')
        || t.starts_with('.')
        || t.starts_with('/')
        || t.starts_with('~')
        || t.contains("://")
        || t.starts_with("git+")
        || t.contains('$')
    {
        return None;
    }
    let n = match r {
        Registro::Npm => {
            if t.starts_with("file:")
                || t.starts_with("github:")
                || t.starts_with("npm:")
                || t.ends_with(".tgz")
            {
                return None;
            }
            match t.strip_prefix('@') {
                Some(rest) => format!("@{}", rest.split('@').next().unwrap_or(rest)),
                None => t.split('@').next().unwrap_or(t).to_string(),
            }
        }
        Registro::PyPi => t
            .split(['=', '>', '<', '~', '!', '[', ';', ' '])
            .next()
            .unwrap_or(t)
            .to_string(),
        Registro::Crates => t.split('@').next().unwrap_or(t).to_string(),
    };
    let ok = !n.is_empty()
        && n.chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_./@".contains(c))
        && !n.ends_with(".txt")
        && !n.ends_with(".whl");
    ok.then_some(n)
}

/// Los paquetes que un comando de shell pide instalar (en cualquiera de sus partes `&&`, `;`).
pub fn pedidos(cmd: &str) -> Vec<(Registro, String)> {
    let mut out = Vec::new();
    for parte in cmd.split(['&', ';', '|', '\n']) {
        let w: Vec<&str> = parte.split_whitespace().collect();
        // (prefijo de palabras, registro)
        let formas: &[(&[&str], Registro)] = &[
            (&["npm", "install"], Registro::Npm),
            (&["npm", "i"], Registro::Npm),
            (&["npm", "add"], Registro::Npm),
            (&["pnpm", "add"], Registro::Npm),
            (&["pnpm", "install"], Registro::Npm),
            (&["yarn", "add"], Registro::Npm),
            (&["bun", "add"], Registro::Npm),
            (&["bun", "install"], Registro::Npm),
            (&["pip", "install"], Registro::PyPi),
            (&["pip3", "install"], Registro::PyPi),
            (&["python", "-m", "pip", "install"], Registro::PyPi),
            (&["python3", "-m", "pip", "install"], Registro::PyPi),
            (&["uv", "add"], Registro::PyPi),
            (&["uv", "pip", "install"], Registro::PyPi),
            (&["poetry", "add"], Registro::PyPi),
            (&["cargo", "add"], Registro::Crates),
        ];
        for i in 0..w.len() {
            for (pre, reg) in formas {
                if w.len() >= i + pre.len() && w[i..i + pre.len()] == **pre {
                    let resto = &w[i + pre.len()..];
                    let mut salta = false;
                    for t in resto {
                        // `-r archivo`, `-e .`, `--index-url x`: la palabra siguiente no es un paquete
                        if salta {
                            salta = false;
                            continue;
                        }
                        if matches!(
                            *t,
                            "-r" | "-e"
                                | "-c"
                                | "--requirement"
                                | "--editable"
                                | "--index-url"
                                | "-i"
                                | "--extra-index-url"
                                | "--registry"
                                | "--features"
                                | "-F"
                                | "--path"
                                | "--git"
                                | "--target"
                                | "--prefix"
                        ) {
                            salta = true;
                            continue;
                        }
                        if let Some(n) = limpio(t, *reg) {
                            if !out.iter().any(|(r, x)| r == reg && *x == n) {
                                out.push((*reg, n));
                            }
                        }
                    }
                }
            }
        }
    }
    out
}

/// ¿Existe? `Some(false)` sólo con un 404 del registro; `None` si no se pudo saber.
async fn existe(c: &reqwest::Client, reg: Registro, n: &str) -> Option<bool> {
    let url = match reg {
        Registro::Npm => format!("https://registry.npmjs.org/{}", n.replace('/', "%2F")),
        Registro::PyPi => format!("https://pypi.org/pypi/{n}/json"),
        Registro::Crates => format!("https://crates.io/api/v1/crates/{n}"),
    };
    let r = c.get(&url).send().await.ok()?;
    match r.status().as_u16() {
        200..=299 => Some(true),
        404 => Some(false),
        _ => None,
    }
}

/// Los que el registro confirma que NO existen, ya con su mensaje.
pub async fn inexistentes(cmd: &str) -> Vec<String> {
    let pedidos = pedidos(cmd);
    if pedidos.is_empty() {
        return vec![];
    }
    let Ok(c) = reqwest::Client::builder()
        .user_agent("GOD-OF-THE-VOID (local agent; package check)")
        .timeout(Duration::from_secs(6))
        .build()
    else {
        return vec![];
    };
    let mut malos = Vec::new();
    for (reg, n) in pedidos.into_iter().take(12) {
        if existe(&c, reg, &n).await == Some(false) {
            malos.push(format!("«{n}» no existe en {}", reg.nombre()));
        }
    }
    malos
}

// ───────────────────────── F20.1 · la compuerta de instalación ─────────────────────────

/// Lo que dice el registro de un paquete.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Ficha {
    /// `None`: no se pudo saber (sin red, el registro no respondió)
    pub existe: Option<bool>,
    /// días desde que se publicó por primera vez
    pub dias: Option<u64>,
    /// descargas recientes (npm: la última semana; crates.io: 90 días; PyPI no las da)
    pub descargas: Option<u64>,
    /// guiones que corren al instalar (npm: `preinstall`, `install`, `postinstall`)
    pub guiones: Vec<String>,
    /// la última versión publicada
    pub version: Option<String>,
}

/// La raíz de cada servicio (las pruebas la cambian por un servidor falso).
fn base(var: &str, real: &str) -> String {
    std::env::var(var).unwrap_or_else(|_| real.to_string())
}

/// Días desde una fecha ISO (`2024-03-05T…`), sin dependencias (días civiles de Howard Hinnant).
pub fn dias_desde(iso: &str) -> Option<u64> {
    let f = iso.get(..10)?;
    let mut p = f.split('-');
    let (y, m, d): (i64, i64, i64) = (p.next()?.parse().ok()?, p.next()?.parse().ok()?, p.next()?.parse().ok()?);
    let y2 = if m <= 2 { y - 1 } else { y };
    let era = (if y2 >= 0 { y2 } else { y2 - 399 }) / 400;
    let yoe = y2 - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let dias = era * 146_097 + doe - 719_468;
    let hoy = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).ok()?.as_secs() as i64 / 86_400;
    (hoy >= dias).then(|| (hoy - dias) as u64)
}

fn cliente() -> Option<reqwest::Client> {
    reqwest::Client::builder().user_agent("GOD-OF-THE-VOID (local agent; package check)").timeout(Duration::from_secs(8)).build().ok()
}

/// La ficha de un paquete en su registro (una o dos peticiones).
pub async fn ficha(c: &reqwest::Client, reg: Registro, n: &str) -> Ficha {
    let mut f = Ficha::default();
    let url = match reg {
        Registro::Npm => format!("{}/{}", base("GOTV_REGISTRO_NPM", "https://registry.npmjs.org"), n.replace('/', "%2F")),
        Registro::PyPi => format!("{}/pypi/{n}/json", base("GOTV_REGISTRO_PYPI", "https://pypi.org")),
        Registro::Crates => format!("{}/api/v1/crates/{n}", base("GOTV_REGISTRO_CRATES", "https://crates.io")),
    };
    let Ok(r) = c.get(&url).send().await else { return f };
    match r.status().as_u16() {
        404 => {
            f.existe = Some(false);
            return f;
        }
        200..=299 => f.existe = Some(true),
        _ => return f,
    }
    let Ok(j) = r.json::<serde_json::Value>().await else { return f };
    match reg {
        Registro::Npm => {
            f.dias = j["time"]["created"].as_str().and_then(dias_desde);
            f.version = j["dist-tags"]["latest"].as_str().map(String::from);
            if let Some(v) = &f.version {
                let s = &j["versions"][v]["scripts"];
                f.guiones = ["preinstall", "install", "postinstall"].iter().filter_map(|k| s[k].as_str().map(|x| format!("{k}: {}", x.chars().take(80).collect::<String>()))).collect();
            }
            let d = format!("{}/downloads/point/last-week/{n}", base("GOTV_DESCARGAS_NPM", "https://api.npmjs.org"));
            if let Ok(r) = c.get(&d).send().await {
                f.descargas = r.json::<serde_json::Value>().await.ok().and_then(|x| x["downloads"].as_u64());
            }
        }
        Registro::PyPi => {
            f.version = j["info"]["version"].as_str().map(String::from);
            // la primera subida de cualquier versión
            f.dias = j["releases"].as_object().map(|r| r.values().flat_map(|v| v.as_array().into_iter().flatten()).filter_map(|a| a["upload_time"].as_str().and_then(dias_desde)).max()).unwrap_or(None);
        }
        Registro::Crates => {
            f.dias = j["crate"]["created_at"].as_str().and_then(dias_desde);
            f.version = j["crate"]["max_stable_version"].as_str().or(j["crate"]["max_version"].as_str()).map(String::from);
            f.descargas = j["crate"]["recent_downloads"].as_u64();
        }
    }
    f
}

/// Una vulnerabilidad conocida (OSV).
#[derive(Debug, Clone, PartialEq)]
pub struct Vuln {
    pub id: String,
    pub resumen: String,
    /// CRITICAL · HIGH · MODERATE · LOW · `?`
    pub gravedad: String,
    pub arreglo: Option<String>,
}

pub fn ecosistema_osv(r: Registro) -> &'static str {
    match r {
        Registro::Npm => "npm",
        Registro::PyPi => "PyPI",
        Registro::Crates => "crates.io",
    }
}

/// OSV por lotes: para cada (ecosistema, nombre, versión), los ids de sus vulnerabilidades. `None` si OSV
/// no respondió (sin red): eso se dice «no pude comprobarlo», jamás «seguro».
/// Lo que OSV ya respondió en este proceso: (ecosistema, nombre, versión) → ids. Medido 2026-11-16 (banco
/// `vulnerable`): tras 5 negativas el modelo repitió la instalación justo cuando OSV no respondió y la
/// compuerta, sin datos, la dejó pasar. Una versión vulnerable no deja de serlo en la misma sesión.
/// La clave lleva el servidor OSV al que se preguntó (las pruebas usan los suyos, `GOTV_OSV`).
type ClaveOsv = (String, String, String, String);

fn osv_recordado() -> &'static std::sync::Mutex<std::collections::HashMap<ClaveOsv, Vec<String>>> {
    static M: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<ClaveOsv, Vec<String>>>> = std::sync::OnceLock::new();
    M.get_or_init(Default::default)
}

pub async fn osv_lote(c: &reqwest::Client, q: &[(String, String, String)]) -> Option<Vec<Vec<String>>> {
    let servidor = base("GOTV_OSV", "https://api.osv.dev");
    let clave = |(e, n, v): &(String, String, String)| (servidor.clone(), e.clone(), n.clone(), v.clone());
    let faltan: Vec<(String, String, String)> = q.iter().filter(|k| !osv_recordado().lock().unwrap().contains_key(&clave(k))).cloned().collect();
    if !faltan.is_empty() {
        let nuevos = osv_lote_red(c, &faltan).await?;
        let mut m = osv_recordado().lock().unwrap();
        for (k, ids) in faltan.iter().zip(nuevos) {
            m.insert(clave(k), ids);
        }
    }
    let m = osv_recordado().lock().unwrap();
    Some(q.iter().map(|k| m.get(&clave(k)).cloned().unwrap_or_default()).collect())
}

async fn osv_lote_red(c: &reqwest::Client, q: &[(String, String, String)]) -> Option<Vec<Vec<String>>> {
    let mut todos = Vec::new();
    for trozo in q.chunks(500) {
        let cuerpo = serde_json::json!({"queries": trozo.iter().map(|(e, n, v)| serde_json::json!({"package": {"name": n, "ecosystem": e}, "version": v})).collect::<Vec<_>>()});
        let r = c.post(format!("{}/v1/querybatch", base("GOTV_OSV", "https://api.osv.dev"))).json(&cuerpo).send().await.ok()?;
        if !r.status().is_success() {
            return None;
        }
        let j: serde_json::Value = r.json().await.ok()?;
        let res = j["results"].as_array()?;
        for x in res {
            todos.push(x["vulns"].as_array().map(|v| v.iter().filter_map(|y| y["id"].as_str().map(String::from)).collect()).unwrap_or_default());
        }
    }
    Some(todos)
}

/// El detalle de una vulnerabilidad: resumen, gravedad y la versión que la corrige.
pub async fn osv_detalle(c: &reqwest::Client, id: &str) -> Option<Vuln> {
    let j: serde_json::Value = c.get(format!("{}/v1/vulns/{id}", base("GOTV_OSV", "https://api.osv.dev"))).send().await.ok()?.json().await.ok()?;
    let gravedad = j["database_specific"]["severity"].as_str().map(|s| s.to_uppercase()).or_else(|| j["affected"][0]["ecosystem_specific"]["severity"].as_str().map(|s| s.to_uppercase())).unwrap_or_else(|| "?".into());
    let arreglo = j["affected"].as_array().into_iter().flatten().flat_map(|a| a["ranges"].as_array().into_iter().flatten()).flat_map(|r| r["events"].as_array().into_iter().flatten()).find_map(|e| e["fixed"].as_str().map(String::from));
    Some(Vuln { id: id.to_string(), resumen: j["summary"].as_str().or(j["details"].as_str()).unwrap_or("").chars().take(140).collect(), gravedad, arreglo })
}

/// Distancia de edición con trasposiciones (`reqeusts` ↔ `requests` = 1).
pub fn distancia(a: &str, b: &str) -> usize {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let mut d = vec![vec![0usize; b.len() + 1]; a.len() + 1];
    for (i, fila) in d.iter_mut().enumerate() {
        fila[0] = i;
    }
    for j in 0..=b.len() {
        d[0][j] = j;
    }
    for i in 1..=a.len() {
        for j in 1..=b.len() {
            let c = usize::from(a[i - 1] != b[j - 1]);
            d[i][j] = (d[i - 1][j] + 1).min(d[i][j - 1] + 1).min(d[i - 1][j - 1] + c);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                d[i][j] = d[i][j].min(d[i - 2][j - 2] + 1);
            }
        }
    }
    d[a.len()][b.len()]
}

#[derive(serde::Deserialize)]
struct Populares {
    fecha: String,
    npm: Vec<String>,
    pypi: Vec<String>,
    crates: Vec<String>,
}

fn populares() -> &'static Populares {
    static P: std::sync::OnceLock<Populares> = std::sync::OnceLock::new();
    P.get_or_init(|| toml::from_str(include_str!("../datos/populares.toml")).expect("datos/populares.toml"))
}

/// La fecha de la lista de populares (envejece: se renueva con cada versión de GOTV).
pub fn fecha_populares() -> &'static str {
    &populares().fecha
}

/// ¿Es `n` casi igual (1 o 2 letras; 1 si es corto) a uno MUY usado del registro, sin serlo? Devuelve cuál.
pub fn parecido(reg: Registro, n: &str) -> Option<&'static str> {
    let p = populares();
    let lista = match reg {
        Registro::Npm => &p.npm,
        Registro::PyPi => &p.pypi,
        Registro::Crates => &p.crates,
    };
    let n = n.to_lowercase();
    // PyPI y crates.io no distinguen `-` de `_`
    let normal = |s: &str| if reg == Registro::Npm { s.to_string() } else { s.replace('_', "-") };
    let nn = normal(&n);
    if lista.iter().any(|x| normal(x) == nn) {
        return None;
    }
    let tope = if nn.chars().count() <= 5 { 1 } else { 2 };
    lista.iter().filter(|x| x.chars().count() >= 4).map(|x| (distancia(&nn, &normal(x)), x.as_str())).filter(|(d, _)| *d <= tope).min_by_key(|(d, _)| *d).map(|(_, x)| x)
}

/// La versión exacta pedida (`react@18.2.0`, `requests==2.31.0`, `serde@=1.0.1`), si la hay.
fn version_pedida(cmd: &str, reg: Registro, n: &str) -> Option<String> {
    for w in cmd.split_whitespace() {
        let w = w.trim_matches(|c| c == '\'' || c == '"');
        let resto = match reg {
            Registro::PyPi => w.strip_prefix(n).and_then(|r| r.strip_prefix("==")),
            _ => w.strip_prefix(n).and_then(|r| r.strip_prefix('@')).map(|r| r.trim_start_matches('=')),
        };
        if let Some(v) = resto.filter(|v| v.chars().next().is_some_and(|c| c.is_ascii_digit()) && v.chars().all(|c| c.is_ascii_alphanumeric() || ".-+".contains(c))) {
            return Some(v.to_string());
        }
    }
    None
}

/// Lo que la compuerta decide de un comando que instala.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Revision {
    /// se bloquea (el registro confirma que no existe)
    pub bloquear: Vec<String>,
    /// se le pregunta al usuario (parecido a uno muy usado, recién publicado, casi sin descargas, vulnerabilidad crítica)
    pub preguntar: Vec<String>,
    /// se avisa con el resultado (guiones de instalación, vulnerabilidades, lo que no se pudo comprobar)
    pub avisos: Vec<String>,
}

/// F20.1 · antes de un comando que instala: existencia, parecido, edad, descargas, guiones y vulnerabilidades
/// de cada paquete pedido. Sin red: «no pude comprobarlo», nunca «seguro».
pub async fn revisar(cmd: &str) -> Revision {
    let mut r = Revision::default();
    let pedidos = pedidos(cmd);
    if pedidos.is_empty() {
        return r;
    }
    let Some(c) = cliente() else { return r };
    let mut consultas = Vec::new();
    for (reg, n) in pedidos.into_iter().take(12) {
        if let Some(p) = parecido(reg, &n) {
            r.preguntar.push(crate::tr!("«{n}» se parece a «{p}», uno de los más usados de {}: ¿quisiste decir «{p}»?", "«{n}» looks like «{p}», one of the most used on {}: did you mean «{p}»?", reg.nombre()));
        }
        let f = ficha(&c, reg, &n).await;
        match f.existe {
            Some(false) => {
                r.bloquear.push(crate::tr!("«{n}» no existe en {}", "«{n}» does not exist on {}", reg.nombre()));
                continue;
            }
            None => {
                r.avisos.push(crate::tr!("no pude comprobar «{n}» en {} (sin respuesta): no lo des por seguro", "could not check «{n}» on {} (no answer): do not assume it is safe", reg.nombre()));
                continue;
            }
            Some(true) => {}
        }
        if let Some(d) = f.dias.filter(|d| *d < 7) {
            r.preguntar.push(crate::tr!("«{n}» se publicó por primera vez hace {d} día(s)", "«{n}» was first published {d} day(s) ago"));
        }
        let pocas = match reg {
            Registro::Npm => 50,
            Registro::Crates => 100,
            Registro::PyPi => 0,
        };
        if let Some(x) = f.descargas.filter(|x| *x < pocas) {
            r.preguntar.push(crate::tr!("«{n}» casi no se descarga ({x} recientes)", "«{n}» is barely downloaded ({x} recent)"));
        }
        if !f.guiones.is_empty() {
            r.avisos.push(crate::tr!("«{n}» trae guiones que corren al instalar ({}); dentro de la caja corren sin red fuera de lo permitido", "«{n}» ships scripts that run on install ({}); inside the sandbox they run without network beyond what is allowed", f.guiones.join("; ")));
        }
        if let Some(v) = version_pedida(cmd, reg, &n).or(f.version) {
            consultas.push((ecosistema_osv(reg).to_string(), n.clone(), v));
        }
    }
    if !consultas.is_empty() {
        match osv_lote(&c, &consultas).await {
            None => r.avisos.push(crate::i18n::t("no pude comprobar vulnerabilidades (OSV sin respuesta): no lo des por seguro", "could not check vulnerabilities (OSV did not answer): do not assume it is safe").into()),
            Some(ids) => {
                for ((_, n, v), ids) in consultas.iter().zip(ids) {
                    for id in ids.iter().take(5) {
                        let d = osv_detalle(&c, id).await.unwrap_or(Vuln { id: id.clone(), resumen: String::new(), gravedad: "?".into(), arreglo: None });
                        let arreglo = d.arreglo.as_deref().map(|a| crate::tr!(", se corrige en {a}", ", fixed in {a}")).unwrap_or_default();
                        let t = crate::tr!("«{n}» {v} tiene la vulnerabilidad {} ({}{arreglo}): {}", "«{n}» {v} has the vulnerability {} ({}{arreglo}): {}", d.id, d.gravedad, d.resumen);
                        // medido 2026-11-16 (banco `vulnerable`): con sólo CRITICAL, `npm install
                        // lodash@4.17.15` (MODERATE, corregida en 4.17.21) se instaló y el aviso de después
                        // ni llegó a la respuesta. Se frena ANTES lo grave y lo que ya tiene arreglo.
                        if matches!(d.gravedad.as_str(), "CRITICAL" | "HIGH") || (d.gravedad == "MODERATE" && d.arreglo.is_some()) {
                            r.preguntar.push(t);
                        } else {
                            r.avisos.push(t);
                        }
                    }
                }
            }
        }
    }
    r
}

/// Las pruebas que cambian las raíces de los servicios (`GOTV_OSV`, `GOTV_REGISTRO_*`) no corren a la vez.
#[cfg(test)]
pub(crate) static CANDADO_RED: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parecidos_distancias_y_fechas() {
        assert_eq!(distancia("reqeusts", "requests"), 1, "una trasposición cuenta uno");
        assert_eq!(distancia("lodahs", "lodash"), 1);
        assert_eq!(distancia("expresss", "express"), 1);
        assert_eq!(parecido(Registro::PyPi, "reqeusts"), Some("requests"));
        assert_eq!(parecido(Registro::Npm, "lodahs"), Some("lodash"));
        assert_eq!(parecido(Registro::Crates, "serde_jsno"), Some("serde_json"));
        assert_eq!(parecido(Registro::Npm, "react"), None, "el bueno no se parece a sí mismo");
        assert_eq!(parecido(Registro::Npm, "preact"), None, "uno muy usado tampoco");
        assert_eq!(parecido(Registro::Crates, "serde-json"), None, "crates.io no distingue - de _");
        assert_eq!(parecido(Registro::Npm, "mi-libreria-propia"), None);
        assert_eq!(parecido(Registro::Npm, "ws2"), None, "lo corto (≤ 4 letras) no se compara");
        assert!(fecha_populares().starts_with("20"));
        assert_eq!(dias_desde("1970-01-01T00:00:00Z").map(|d| d > 20_000), Some(true));
        assert!(dias_desde("no es una fecha").is_none());
        assert_eq!(version_pedida("npm install react@18.2.0 zod", Registro::Npm, "react").as_deref(), Some("18.2.0"));
        assert_eq!(version_pedida("pip install requests==2.19.0", Registro::PyPi, "requests").as_deref(), Some("2.19.0"));
        assert_eq!(version_pedida("npm i react@latest", Registro::Npm, "react"), None);
    }

    /// F20.1 contra un registro y un OSV falsos: lo inventado se bloquea; lo parecido, lo nuevo, lo casi sin
    /// descargas y lo críticamente vulnerable se pregunta; los guiones se avisan; sin red, «no pude».
    #[tokio::test]
    async fn la_compuerta_de_instalacion() {
        use axum::routing::{get, post};
        let _g = CANDADO_RED.lock().await;
        let hace = |d: u64| {
            let s = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() - d * 86_400;
            // a ISO por el mismo camino inverso de días civiles (sólo la fecha)
            let z = (s / 86_400) as i64 + 719_468;
            let era = z.div_euclid(146_097);
            let doe = z - era * 146_097;
            let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
            let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
            let mp = (5 * doy + 2) / 153;
            let dia = doy - (153 * mp + 2) / 5 + 1;
            let mes = if mp < 10 { mp + 3 } else { mp - 9 };
            let anio = yoe + era * 400 + i64::from(mes <= 2);
            format!("{anio:04}-{mes:02}-{dia:02}T00:00:00.000Z")
        };
        let (nuevo, viejo) = (hace(2), hace(900));
        let app = axum::Router::new()
            .route("/npm/{n}", get(move |axum::extract::Path(n): axum::extract::Path<String>| {
                let (nuevo, viejo) = (nuevo.clone(), viejo.clone());
                async move {
                    match n.as_str() {
                        "inventado-gotv" => (axum::http::StatusCode::NOT_FOUND, axum::Json(serde_json::json!({}))),
                        "lodahs" => (axum::http::StatusCode::OK, axum::Json(serde_json::json!({"time": {"created": nuevo}, "dist-tags": {"latest": "1.0.0"}, "versions": {"1.0.0": {"scripts": {"postinstall": "node robar.js"}}}}))),
                        _ => (axum::http::StatusCode::OK, axum::Json(serde_json::json!({"time": {"created": viejo}, "dist-tags": {"latest": "4.17.20"}, "versions": {"4.17.20": {}}}))),
                    }
                }
            }))
            .route("/dl/downloads/point/last-week/{n}", get(|axum::extract::Path(n): axum::extract::Path<String>| async move { axum::Json(serde_json::json!({"downloads": if n == "lodahs" { 3 } else { 9_000_000 }})) }))
            .route("/osv/v1/querybatch", post(|axum::Json(q): axum::Json<serde_json::Value>| async move {
                let r: Vec<serde_json::Value> = q["queries"].as_array().unwrap().iter().map(|x| if x["package"]["name"] == "lodash" { serde_json::json!({"vulns": [{"id": "GHSA-x"}]}) } else { serde_json::json!({}) }).collect();
                axum::Json(serde_json::json!({"results": r}))
            }))
            .route("/osv/v1/vulns/{id}", get(|| async { axum::Json(serde_json::json!({"summary": "prototipo contaminado", "database_specific": {"severity": "CRITICAL"}, "affected": [{"ranges": [{"events": [{"fixed": "4.17.21"}]}]}]})) }));
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let dir = l.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
        std::env::set_var("GOTV_REGISTRO_NPM", format!("http://{dir}/npm"));
        std::env::set_var("GOTV_DESCARGAS_NPM", format!("http://{dir}/dl"));
        std::env::set_var("GOTV_OSV", format!("http://{dir}/osv"));
        let r = revisar("npm install inventado-gotv").await;
        assert!(r.bloquear.len() == 1 && r.bloquear[0].contains("inventado-gotv"), "{r:?}");
        let r = revisar("npm install lodahs").await;
        let p = r.preguntar.join(" | ");
        assert!(p.contains("lodash") && (p.contains("hace 2") || p.contains("2 day")) && (p.contains("3 recientes") || p.contains("3 recent")), "{p}");
        assert!(r.avisos.iter().any(|a| a.contains("postinstall: node robar.js")), "{r:?}");
        let r = revisar("npm install lodash@4.17.20").await;
        assert!(r.preguntar.iter().any(|x| x.contains("GHSA-x") && x.contains("CRITICAL") && x.contains("4.17.21")), "{r:?}");
        // sin red: nada se da por seguro
        std::env::set_var("GOTV_REGISTRO_NPM", "http://127.0.0.1:9/npm");
        let r = revisar("npm install zod").await;
        assert!(r.bloquear.is_empty() && r.avisos.iter().any(|a| a.contains("no pude") || a.contains("could not")), "{r:?}");
        for v in ["GOTV_REGISTRO_NPM", "GOTV_DESCARGAS_NPM", "GOTV_OSV"] {
            std::env::remove_var(v);
        }
    }

    #[test]
    fn lee_lo_que_cada_gestor_instala() {
        let n = |c: &str| {
            pedidos(c)
                .into_iter()
                .map(|(r, n)| format!("{}:{n}", r.nombre()))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            n("npm install react@18 @tanstack/react-query@5 -D vitest"),
            ["npm:react", "npm:@tanstack/react-query", "npm:vitest"]
        );
        assert_eq!(n("cd app && pnpm add zod"), ["npm:zod"]);
        assert_eq!(
            n("pip install 'requests[socks]>=2.31' -r requirements.txt flask"),
            ["PyPI:requests", "PyPI:flask"]
        );
        assert_eq!(
            n("python3 -m pip install -e . pydantic==2.8"),
            ["PyPI:pydantic"]
        );
        assert_eq!(
            n("uv add httpx; uv pip install rich"),
            ["PyPI:httpx", "PyPI:rich"]
        );
        assert_eq!(
            n("cargo add serde@1.0 --features derive tokio"),
            ["crates.io:serde", "crates.io:tokio"]
        );
        assert!(
            n("npm install").is_empty(),
            "instalar lo del package.json no pide nada nuevo"
        );
        assert!(n("npm install ./local-pkg git+https://x/y.git file:../z").is_empty());
        assert!(
            n("echo npm install falso").len() == 1,
            "aun citado se revisa: no ejecuta nada si existe"
        );
        assert!(n("ls -la").is_empty());
    }

    /// Con la red real: un paquete famoso existe; uno inventado se frena. (`--ignored`)
    #[tokio::test]
    #[ignore]
    async fn frena_lo_inventado_con_la_red_real() {
        let m = inexistentes("npm install react gotv-paquete-que-no-existe-7c1f && pip install requests pypi-inventado-gotv-91x && cargo add serde").await;
        assert_eq!(m.len(), 2, "{m:?}");
        assert!(
            m[0].contains("gotv-paquete-que-no-existe-7c1f")
                && m[1].contains("pypi-inventado-gotv-91x")
        );
    }
}
