//! Paquetes por lenguaje (F11; nació como las fichas de la Ola D8a): GOTV detecta los lenguajes del
//! proyecto por sus archivos y le dice al modelo, en UNA línea por lenguaje, cómo se formatea, se revisa y
//! se prueba aquí, y la trampa típica; y además QUÉ gestor usa este proyecto (por su archivo de bloqueo y
//! la versión de `packageManager`: yarn 1 y yarn 2+ no comparten banderas) con sus comandos.
//!
//! Los paquetes son DATOS dentro del binario (`crates/gotv-core/paquetes/<id>/senales.toml`, ampliables o
//! reemplazables en `~/.config/gotv/paquetes/<id>/senales.toml`), no código: detección, ficha y ley 9
//! (ES/EN), gestores y sus tareas, corredores de pruebas, variables para no quedarse colgado, comandos que
//! se quedan VIVOS (van directo a `process`) y patrones `archivo:línea` de sus errores (para F15.1).
//!
//! Las leyes universales (entender antes de cambiar, prueba que falla antes del arreglo…) viven en el
//! perfil Programar (`perfil.rs`); aquí sólo lo propio de cada lenguaje.

use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::OnceLock;

/// Un texto en los dos idiomas.
#[derive(Debug, Clone, Deserialize)]
pub struct Texto {
    pub es: String,
    pub en: String,
}

impl Texto {
    pub fn ver(&self) -> &str {
        if crate::i18n::is_en() {
            &self.en
        } else {
            &self.es
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Detectar {
    /// grupos de archivos de proyecto: basta UN grupo, y en él tienen que estar TODOS (`*.ext` vale)
    #[serde(default)]
    pub marcadores: Vec<Vec<String>>,
    /// extensiones en la raíz y en `src/`
    #[serde(default)]
    pub extensiones: Vec<String>,
    /// si ya se detectó alguno de éstos, éste no (JavaScript cede a TypeScript)
    #[serde(default)]
    pub cede_a: Vec<String>,
    /// por extensiones, no si también hay de éstas (C cede a los `.cpp`)
    #[serde(default)]
    pub salvo_extensiones: Vec<String>,
    /// lo que en una ORDEN delata el lenguaje (proyecto vacío)
    #[serde(default)]
    pub palabras: Vec<String>,
    /// gestores, corredores, variables y servidores de otro paquete (TypeScript los de Node)
    #[serde(default)]
    pub hereda: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Gestor {
    pub nombre: String,
    /// lo elige alguno de estos archivos…
    #[serde(default)]
    pub si_hay_alguno: Vec<String>,
    /// …o `packageManager` de package.json con uno de estos prefijos (`yarn@1`, `pnpm@`)…
    #[serde(default)]
    pub package_manager: Vec<String>,
    /// …o que estos archivos contengan estos textos (`pubspec.yaml` con `flutter:`)…
    #[serde(default)]
    pub si_contiene: BTreeMap<String, String>,
    /// …o nada (el de siempre); y nunca si existe alguno de éstos
    #[serde(default)]
    pub por_omision: bool,
    #[serde(default)]
    pub no_hay: Vec<String>,
    /// cómo se corre un script (`pnpm run`) y un binario del proyecto (`pnpm exec`)
    #[serde(default)]
    pub correr: Option<String>,
    #[serde(default)]
    pub ejecutar: Option<String>,
    /// instalar · compilar · revisar · formatear · probar · una_prueba
    pub tareas: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Corredor {
    pub nombre: String,
    #[serde(default)]
    pub si_depende: Vec<String>,
    #[serde(default)]
    pub si_hay: Vec<String>,
    pub una_prueba: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct NoInteractivo {
    #[serde(default)]
    pub variables: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Vivos {
    #[serde(default)]
    pub patrones: Vec<String>,
    #[serde(default)]
    pub excepto: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Errores {
    #[serde(default)]
    pub archivo_linea: Vec<String>,
}

/// Un paquete de lenguaje.
#[derive(Debug, Clone, Deserialize)]
pub struct Paquete {
    pub id: String,
    pub nombre: String,
    pub ficha: Texto,
    /// la skill con su guía (correr una prueba, añadir una, dependencias, depurar, cobertura)
    #[serde(default)]
    pub guia: Option<String>,
    #[serde(default)]
    pub ley9: Option<Texto>,
    #[serde(default)]
    pub detectar: Detectar,
    #[serde(default)]
    pub gestores: Vec<Gestor>,
    #[serde(default)]
    pub corredores: Vec<Corredor>,
    #[serde(default)]
    pub no_interactivo: NoInteractivo,
    #[serde(default)]
    pub vivos: Vivos,
    #[serde(default)]
    pub errores: Errores,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct Indice {
    marcadores: Vec<String>,
    extensiones: Vec<String>,
    orden: Vec<String>,
}

/// Los paquetes que viajan DENTRO del binario.
const INCLUIDOS: &[(&str, &str)] = &[
    ("rust", include_str!("../paquetes/rust/senales.toml")),
    ("typescript", include_str!("../paquetes/typescript/senales.toml")),
    ("javascript", include_str!("../paquetes/javascript/senales.toml")),
    ("python", include_str!("../paquetes/python/senales.toml")),
    ("go", include_str!("../paquetes/go/senales.toml")),
    ("java", include_str!("../paquetes/java/senales.toml")),
    ("kotlin", include_str!("../paquetes/kotlin/senales.toml")),
    ("csharp", include_str!("../paquetes/csharp/senales.toml")),
    ("c", include_str!("../paquetes/c/senales.toml")),
    ("cpp", include_str!("../paquetes/cpp/senales.toml")),
    ("php", include_str!("../paquetes/php/senales.toml")),
    ("ruby", include_str!("../paquetes/ruby/senales.toml")),
    ("swift", include_str!("../paquetes/swift/senales.toml")),
    ("dart", include_str!("../paquetes/dart/senales.toml")),
    ("elixir", include_str!("../paquetes/elixir/senales.toml")),
    ("zig", include_str!("../paquetes/zig/senales.toml")),
    ("lua", include_str!("../paquetes/lua/senales.toml")),
    ("sql", include_str!("../paquetes/sql/senales.toml")),
    ("bash", include_str!("../paquetes/bash/senales.toml")),
    ("web", include_str!("../paquetes/web/senales.toml")),
];
const INDICE: &str = include_str!("../paquetes/indice.toml");

struct Catalogo {
    paquetes: BTreeMap<String, Paquete>,
    indice: Indice,
}

/// Los del binario y, encima, los del usuario (`~/.config/gotv/paquetes/<id>/senales.toml`: reemplaza el
/// de ese id o agrega uno nuevo, que va al final de cada orden). Un paquete del usuario que no se entiende
/// se ignora (el del binario sigue).
fn catalogo() -> &'static Catalogo {
    static C: OnceLock<Catalogo> = OnceLock::new();
    C.get_or_init(|| {
        let mut paquetes = BTreeMap::new();
        for (id, t) in INCLUIDOS {
            let p: Paquete = toml::from_str(t).unwrap_or_else(|e| panic!("paquete {id} mal formado: {e}")); // juez-idioma: datos (sólo lo vería quien rompa un paquete incluido; lo atrapa la prueba)
            paquetes.insert(id.to_string(), p);
        }
        let mut indice: Indice = toml::from_str(INDICE).unwrap_or_default();
        let home = std::env::var("HOME").unwrap_or_default();
        // F10.4: también los `paquetes/` de los plugins confiados
        let carpetas = std::iter::once(Path::new(&home).join(".config/gotv/paquetes")).chain(crate::plugins::carpetas("paquetes"));
        for e in carpetas.flat_map(|c| std::fs::read_dir(c).into_iter().flatten().flatten()) {
            let Ok(t) = std::fs::read_to_string(e.path().join("senales.toml")) else { continue };
            let Ok(p) = toml::from_str::<Paquete>(&t) else { continue };
            if !paquetes.contains_key(&p.id) {
                for l in [&mut indice.marcadores, &mut indice.extensiones, &mut indice.orden] {
                    l.push(p.id.clone());
                }
            }
            paquetes.insert(p.id.clone(), p);
        }
        Catalogo { paquetes, indice }
    })
}

/// Un paquete por su id.
pub fn paquete(id: &str) -> Option<&'static Paquete> {
    catalogo().paquetes.get(id)
}

/// Todos los paquetes (en el orden de las órdenes).
pub fn todos() -> Vec<&'static Paquete> {
    let c = catalogo();
    let mut v: Vec<&Paquete> = c.indice.orden.iter().filter_map(|i| c.paquetes.get(i)).collect();
    for p in c.paquetes.values() {
        if !v.iter().any(|x| x.id == p.id) {
            v.push(p);
        }
    }
    v
}

impl Paquete {
    pub fn ficha(&self) -> &str {
        self.ficha.ver()
    }

    /// Ley 9 del perfil Programar en ESTE lenguaje: comparar texto sin mayúsculas ni tildes normalizando
    /// los dos lados (prueba Cable, 2026-11-13). Python y JS/TS ya van en la propia ley.
    pub fn texto_sin_tildes(&self) -> Option<&str> {
        self.ley9.as_ref().map(|t| t.ver())
    }

    /// El paquete del que hereda gestores y demás (o él mismo).
    fn base(&self) -> &Paquete {
        self.detectar.hereda.as_deref().and_then(paquete).unwrap_or(self)
    }

    fn gestores(&self) -> &[Gestor] {
        if self.gestores.is_empty() { &self.base().gestores } else { &self.gestores }
    }
}

fn marca(root: &Path, m: &str) -> bool {
    crate::lsp::registro::marca(root, m) || (m.starts_with('.') && root.join(m).exists())
}

/// Lenguajes del proyecto, por sus archivos de proyecto y, si no hay, por extensiones de la raíz y `src/`
/// (sin recorrer todo el árbol: rápido aunque haya node_modules). Como mucho cuatro.
pub fn detectar(root: &Path) -> Vec<&'static Paquete> {
    let c = catalogo();
    let mut v: Vec<&'static Paquete> = Vec::new();
    let cede = |p: &Paquete, v: &[&Paquete]| p.detectar.cede_a.iter().any(|x| v.iter().any(|y| &y.id == x));
    for id in &c.indice.marcadores {
        let Some(p) = c.paquetes.get(id) else { continue };
        if !v.iter().any(|x| x.id == p.id) && !cede(p, &v) && p.detectar.marcadores.iter().any(|g| !g.is_empty() && g.iter().all(|m| marca(root, m))) {
            v.push(p);
        }
    }
    let mut ext = BTreeMap::<String, usize>::new();
    for d in [root.to_path_buf(), root.join("src")] {
        for e in std::fs::read_dir(&d).into_iter().flatten().flatten().take(400) {
            if let Some(x) = e.path().extension().and_then(|x| x.to_str()) {
                *ext.entry(x.to_lowercase()).or_default() += 1;
            }
        }
    }
    let hay = |xs: &[String]| xs.iter().any(|x| ext.get(x).copied().unwrap_or(0) > 0);
    for id in &c.indice.extensiones {
        let Some(p) = c.paquetes.get(id) else { continue };
        if !v.iter().any(|x| x.id == p.id) && !cede(p, &v) && hay(&p.detectar.extensiones) && !hay(&p.detectar.salvo_extensiones) {
            v.push(p);
        }
    }
    v.truncate(4); // como mucho cuatro fichas: el contexto de un modelo pequeño es oro
    v
}

/// Lenguajes que PIDE la orden (para un proyecto vacío, donde los archivos aún no dicen nada).
/// Medido (C5, 9B): en una carpeta vacía no llegaba ninguna ficha y `uv init myproject` creó una
/// subcarpeta; el código quedó fuera y 40 pasos se fueron en buscar el entorno.
pub fn de_la_orden(orden: &str) -> Vec<&'static Paquete> {
    let o = format!(" {} ", orden.to_lowercase().replace(['.', ',', ':', ';', '(', ')'], " "));
    let c = catalogo();
    let mut v: Vec<&'static Paquete> = c.indice.orden.iter().filter_map(|i| c.paquetes.get(i)).filter(|p| p.detectar.palabras.iter().any(|w| o.contains(w.as_str()))).collect();
    v.truncate(3);
    v
}

/// La ley 9 en los lenguajes en juego (los del proyecto y los que pide la orden) que no son Python ni
/// JS/TS, ya escritos en la ley. Va con la nota del perfil Programar.
pub fn recetas_ley9(root: &Path, orden: &str) -> Option<String> {
    let mut ls = detectar(root);
    for l in de_la_orden(orden) {
        if !ls.iter().any(|x| x.id == l.id) {
            ls.push(l);
        }
    }
    let lineas: Vec<&str> = ls.iter().filter_map(|l| l.texto_sin_tildes()).collect();
    if lineas.is_empty() {
        return None;
    }
    let cabeza = crate::i18n::t("Ley 9 en tus otros lenguajes:", "Law 9 in your other languages:");
    Some(format!("{cabeza}\n- {}", lineas.join("\n- ")))
}

/// El `packageManager` de package.json (`pnpm@9.12.0`), si lo declara.
fn package_manager(root: &Path) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(root.join("package.json")).ok()?).ok()?;
    v["packageManager"].as_str().map(String::from)
}

/// El gestor que usa ESTE proyecto para el lenguaje `p`, y por qué (el archivo o la versión que lo eligió).
pub fn gestor(root: &Path, p: &'static Paquete) -> Option<(&'static Gestor, String)> {
    let pm = package_manager(root);
    for g in p.gestores() {
        if g.no_hay.iter().any(|f| root.join(f).exists()) {
            continue;
        }
        if let Some(f) = g.si_hay_alguno.iter().find(|f| marca(root, f)) {
            return Some((g, f.clone()));
        }
        if let Some(pm) = pm.as_deref().filter(|pm| g.package_manager.iter().any(|x| pm.starts_with(x.as_str()))) {
            return Some((g, format!("packageManager {pm}")));
        }
        if !g.si_contiene.is_empty() && g.si_contiene.iter().all(|(f, t)| std::fs::read_to_string(root.join(f)).is_ok_and(|x| x.contains(t.as_str()))) {
            return Some((g, g.si_contiene.keys().cloned().collect::<Vec<_>>().join(", ")));
        }
        if g.por_omision {
            return Some((g, crate::i18n::t("por omisión", "by default").into()));
        }
    }
    None
}

/// Un argumento para la terminal, entre comillas simples (un nombre de prueba con espacios o símbolos de
/// expresión regular llega tal cual).
pub fn para_la_terminal(s: &str) -> String {
    if !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || "_-./:".contains(c)) {
        return s.to_string();
    }
    format!("'{}'", s.replace('\'', r"'\''"))
}

fn rellenar(plantilla: &str, g: &Gestor, nombre: Option<&str>) -> String {
    let mut s = plantilla.replace("{ejecutar}", g.ejecutar.as_deref().unwrap_or("")).replace("{correr}", g.correr.as_deref().unwrap_or(""));
    if let Some(n) = nombre {
        s = s.replace("{nombre}", &para_la_terminal(n));
    }
    s.trim().to_string()
}

/// El comando de una TAREA (`instalar`, `compilar`, `revisar`, `formatear`, `probar`, `una_prueba`) del
/// lenguaje `p` en este proyecto. `una_prueba` sale del corredor si el gestor no la trae (Node: vitest,
/// jest…), con `nombre` ya escapado para la terminal.
pub fn tarea(root: &Path, p: &'static Paquete, cual: &str, nombre: Option<&str>) -> Option<String> {
    let (g, _) = gestor(root, p)?;
    if let Some(t) = g.tareas.get(cual) {
        return Some(rellenar(t, g, nombre));
    }
    if cual != "una_prueba" {
        return None;
    }
    let base = p.base();
    let corredores = if p.corredores.is_empty() { &base.corredores } else { &p.corredores };
    let deps = std::fs::read_to_string(root.join("package.json")).ok().and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok());
    let depende = |n: &str| deps.as_ref().is_some_and(|d| ["dependencies", "devDependencies"].iter().any(|k| d[k].get(n).is_some()));
    let c = corredores.iter().find(|c| {
        let con_condicion = !c.si_depende.is_empty() || !c.si_hay.is_empty();
        !con_condicion || c.si_depende.iter().any(|d| depende(d)) || c.si_hay.iter().any(|f| root.join(f).exists())
    })?;
    Some(rellenar(&c.una_prueba, g, nombre))
}

/// La línea «aquí» de un lenguaje con varios gestores posibles: cuál usa este proyecto y sus comandos.
fn linea_del_gestor(root: &Path, p: &'static Paquete) -> Option<String> {
    if p.gestores().len() < 2 {
        return None;
    }
    let (g, porque) = gestor(root, p)?;
    let mut partes = Vec::new();
    for (cual, rotulo) in [("instalar", crate::i18n::t("instalar", "install")), ("probar", crate::i18n::t("probar", "test")), ("una_prueba", crate::i18n::t("una prueba", "one test"))] {
        if let Some(t) = tarea(root, p, cual, Some("NOMBRE")) {
            partes.push(format!("{rotulo} `{t}`"));
        }
    }
    Some(crate::tr!(" Aquí: {} ({porque}) → {}.", " Here: {} ({porque}) → {}.", g.nombre, partes.join(" · ")))
}

/// ¿Es `cmd` uno de los que se quedan corriendo (un servidor de desarrollo, un vigilante) en los lenguajes
/// de este proyecto? Va directo a `process` en vez de esperar a que termine.
pub fn es_vivo(root: &Path, cmd: &str) -> bool {
    // lo que antecede al programa y no cambia qué es
    const PREFIJOS: &[&str] = &["npx ", "pnpm exec ", "pnpm dlx ", "bunx ", "yarn ", "uv run ", "poetry run ", "pipenv run ", "bundle exec ", ".venv/bin/python -m ", "python -m ", "python3 -m "];
    let ls = detectar(root);
    let mut pats: Vec<&str> = Vec::new();
    let mut excepto: Vec<&str> = Vec::new();
    for l in &ls {
        for v in [&l.vivos, &l.base().vivos] {
            pats.extend(v.patrones.iter().map(String::as_str));
            excepto.extend(v.excepto.iter().map(String::as_str));
        }
    }
    if pats.is_empty() {
        return false;
    }
    let casa = |s: &str, p: &str| s == p || s.starts_with(&format!("{p} "));
    cmd.split(['&', ';', '|']).map(str::trim).filter(|s| !s.is_empty()).any(|seg| {
        // sin variables delante (`PORT=3000 npm run dev`) ni `cd x`
        let mut s = seg;
        while let Some((a, b)) = s.split_once(' ') {
            if a.contains('=') && !a.starts_with('-') {
                s = b.trim_start();
            } else {
                break;
            }
        }
        if s.starts_with("cd ") {
            return false;
        }
        let mut variantes = vec![s];
        for pre in PREFIJOS {
            if let Some(r) = s.strip_prefix(pre) {
                variantes.push(r.trim_start());
            }
        }
        variantes.iter().any(|v| pats.iter().any(|p| casa(v, p)) && !excepto.iter().any(|e| casa(v, e)))
    })
}

/// Las variables que evitan que un programa de estos lenguajes se quede esperando una respuesta (o avise
/// de versiones) cuando corre sin terminal.
pub fn variables(root: &Path) -> Vec<(String, String)> {
    let mut v: Vec<(String, String)> = Vec::new();
    for l in detectar(root) {
        for m in [&l.no_interactivo.variables, &l.base().no_interactivo.variables] {
            for (k, x) in m {
                if !v.iter().any(|(a, _)| a == k) {
                    v.push((k.clone(), x.clone()));
                }
            }
        }
    }
    v
}

/// Los patrones `archivo:línea` de los errores de TODOS los paquetes (los compila una vez).
pub fn patrones_de_error() -> &'static [regex::Regex] {
    static P: OnceLock<Vec<regex::Regex>> = OnceLock::new();
    P.get_or_init(|| catalogo().paquetes.values().flat_map(|p| p.errores.archivo_linea.iter()).filter_map(|r| regex::Regex::new(r).ok()).collect())
}

/// Las fichas de lo que pide la orden y el proyecto aún no tiene (van con la orden).
pub fn nota_de_la_orden(root: &Path, orden: &str) -> Option<String> {
    let ya = detectar(root);
    let nuevas: Vec<&Paquete> = de_la_orden(orden).into_iter().filter(|l| !ya.iter().any(|x| x.id == l.id)).collect();
    if nuevas.is_empty() {
        return None;
    }
    let mut s = crate::i18n::t("[Lenguaje pedido — cómo se trabaja con él]", "[Requested language — how to work with it]").to_string();
    for l in nuevas {
        s.push_str("\n- ");
        s.push_str(l.ficha());
        s.push_str(&nota_de_faltantes(l.ficha()));
    }
    Some(s)
}

/// N6 (medido en el Air con el 4B, 2026-11-14): la ficha de Python manda `ruff format` · `ruff check`
/// «antes de terminar»; en una Mac sin ruff el modelo resolvió la tarea y gastó 50 pasos intentando
/// instalarlo. Los programas que nombra una ficha: la primera palabra de cada comando entre
/// acentos graves (dos palabras o más, sin signos de código).
fn programas_de(ficha: &str) -> Vec<&str> {
    let mut v: Vec<&str> = Vec::new();
    for trozo in ficha.split('`').skip(1).step_by(2) {
        let mut palabras = trozo.split_whitespace();
        let (Some(p), Some(_)) = (palabras.next(), palabras.next()) else { continue };
        // palabras del lenguaje, no programas (`if let`, `guard let`, `try await`…)
        const DEL_LENGUAJE: [&str; 24] = ["if", "let", "var", "def", "fn", "for", "try", "new", "async", "await", "except", "guard", "const", "use", "import", "return", "throw", "catch", "defer", "match", "while", "else", "select", "set"];
        let es_codigo = trozo.contains(['(', '=', ':', '{', '<', '!']) || DEL_LENGUAJE.contains(&p);
        let es_programa = p.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_');
        if !es_codigo && es_programa && !v.contains(&p) {
            v.push(p);
        }
    }
    v
}

/// N6: ¿está este programa donde lo buscará `bash` (la shell de inicio de sesión, la misma que usa
/// la herramienta)? Se pregunta una vez por programa y proceso.
fn instalado(programa: &str) -> bool {
    static VISTOS: OnceLock<std::sync::Mutex<std::collections::HashMap<String, bool>>> = OnceLock::new();
    let vistos = VISTOS.get_or_init(Default::default);
    if let Some(&b) = vistos.lock().unwrap().get(programa) {
        return b;
    }
    let esta = crate::lsp::on_path(programa)
        || std::process::Command::new("bash")
            .args(["-lc", &format!("command -v {programa}")])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
    vistos.lock().unwrap().insert(programa.to_string(), esta);
    esta
}

/// N6: la coletilla de una ficha con lo que NO hay en esta Mac (vacía si está todo).
fn nota_de_faltantes(ficha: &str) -> String {
    let faltan: Vec<&str> = programas_de(ficha).into_iter().filter(|p| !instalado(p)).collect();
    if faltan.is_empty() {
        return String::new();
    }
    crate::tr!(
        " (En esta Mac NO están instalados: {}. No los uses ni intentes instalarlos: ese paso se omite.)",
        " (NOT installed on this Mac: {}. Do not use them or try to install them: that step is skipped.)",
        faltan.join(", ")
    )
}

/// El bloque para el sistema (vacío si no se reconoce ningún lenguaje).
pub fn bloque(root: &Path) -> String {
    let ls = detectar(root);
    if ls.is_empty() {
        return String::new();
    }
    let mut s = crate::i18n::t(
        "\n\nLenguajes de este proyecto (antes de terminar: formatea, revisa y prueba con SUS comandos):",
        "\n\nLanguages of this project (before finishing: format, lint and test with THEIR commands):",
    )
    .to_string();
    for l in ls {
        s.push_str("\n- ");
        s.push_str(l.ficha());
        let aqui = linea_del_gestor(root, l).unwrap_or_default();
        s.push_str(&aqui);
        if let Some(g) = &l.guia {
            s.push_str(&crate::tr!(" Guía: skill «{g}».", " Guide: skill «{g}»."));
        }
        s.push_str(&nota_de_faltantes(&format!("{}{aqui}", l.ficha())));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Proyecto falso con esos archivos vacíos; se borra solo al soltarse.
    fn proyecto(archivos: &[&str]) -> crate::CarpetaPrueba {
        let d = crate::CarpetaPrueba::sin_crear(&format!("gotv-leng-{}", archivos.join("_").replace(['/', '.', '@'], "-")));
        for a in archivos {
            let p = d.join(a);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, "").unwrap();
        }
        d
    }

    fn ids(v: &[&Paquete]) -> Vec<String> {
        v.iter().map(|p| p.id.clone()).collect()
    }

    /// F11: cada paquete incluido se entiende, tiene ficha corta en los dos idiomas, y los que tienen
    /// gestores traen las 5 tareas mínimas en cada uno; los patrones de error compilan.
    #[test]
    fn cada_paquete_se_entiende_y_trae_lo_minimo() {
        assert_eq!(todos().len(), INCLUIDOS.len());
        for p in todos() {
            assert!(p.ficha.es.len() < 470 && p.ficha.en.len() < 470 && !p.ficha.es.is_empty(), "{}", p.id);
            for g in &p.gestores {
                for t in ["instalar", "revisar", "formatear", "probar"] {
                    assert!(g.tareas.contains_key(t), "{} / {}: falta «{t}»", p.id, g.nombre);
                }
                let una = g.tareas.contains_key("una_prueba") || !p.base().corredores.is_empty();
                assert!(una, "{} / {}: sin «una_prueba» ni corredores", p.id, g.nombre);
            }
            let con_por_omision = p.gestores().iter().filter(|g| g.por_omision).count();
            assert!(p.gestores().is_empty() || con_por_omision == 1, "{}: un gestor por omisión, el último", p.id);
            assert!(p.gestores().last().is_none_or(|g| g.por_omision), "{}", p.id);
            for r in &p.errores.archivo_linea {
                assert!(regex::Regex::new(r).is_ok_and(|x| x.capture_names().flatten().any(|n| n == "archivo")), "{}: {r}", p.id);
            }
        }
        assert!(todos().len() >= 20, "~20 lenguajes");
    }

    #[test]
    fn detecta_los_principales_por_sus_archivos() {
        let casos: &[(&[&str], &str)] = &[
            (&["Cargo.toml"], "rust"),
            (&["package.json", "tsconfig.json"], "typescript"),
            (&["package.json"], "javascript"),
            (&["pyproject.toml"], "python"),
            (&["main.py"], "python"),
            (&["go.mod"], "go"),
            (&["pom.xml"], "java"),
            (&["build.gradle.kts"], "kotlin"),
            (&["App.csproj"], "csharp"),
            (&["CMakeLists.txt"], "cpp"),
            (&["main.c"], "c"),
            (&["composer.json"], "php"),
            (&["Gemfile"], "ruby"),
            (&["src/pagos.rb"], "ruby"),
            (&["Package.swift"], "swift"),
            (&["migraciones.sql"], "sql"),
            (&["deploy.sh"], "bash"),
            (&["index.html", "estilo.css"], "web"),
            (&["src/app.tsx"], "typescript"),
            (&["pubspec.yaml"], "dart"),
            (&["mix.exs"], "elixir"),
            (&["build.zig"], "zig"),
            (&["init.lua"], "lua"),
        ];
        for (archivos, esperado) in casos {
            let d = proyecto(archivos);
            assert_eq!(detectar(&d).first().map(|p| p.id.as_str()), Some(*esperado), "{archivos:?}");
        }
        // el mismo orden y la misma cesión de antes
        let d = proyecto(&["package.json", "src/a.ts", "src/b.js"]);
        assert_eq!(ids(&detectar(&d)), ["javascript", "typescript"]);
        let d = proyecto(&["CMakeLists.txt", "src/x.c"]);
        assert_eq!(ids(&detectar(&d)), ["cpp", "c"], "como antes: C cede a los .cpp, no al CMakeLists");
        let d = proyecto(&["main.cpp", "util.c"]);
        assert_eq!(ids(&detectar(&d)), ["cpp"]);
        let d = proyecto(&["Cargo.toml", "package.json", "tsconfig.json", "pyproject.toml", "go.mod", "x.sh"]);
        assert_eq!(ids(&detectar(&d)), ["rust", "typescript", "python", "go"], "como mucho cuatro");
    }

    #[test]
    fn la_orden_dice_el_lenguaje_en_un_proyecto_vacio() {
        assert_eq!(ids(&de_la_orden("Crea aquí un proyecto Python con uv y pytest")), ["python"]);
        assert_eq!(ids(&de_la_orden("una API en FastAPI con postgres")), ["python", "sql"]);
        assert_eq!(ids(&de_la_orden("haz un CLI en Rust con cargo")), ["rust"]);
        assert_eq!(ids(&de_la_orden("una app en Flutter")), ["dart"]);
        assert!(de_la_orden("ordena mis fotos por fecha").is_empty());
        assert!(de_la_orden("vamos a jugar go").is_empty(), "«go» suelto no es Go");
        let vacio = proyecto(&["LEEME.txt"]);
        assert!(nota_de_la_orden(&vacio, "proyecto Python con uv").unwrap().contains("`uv init --bare`"));
        let py = proyecto(&["pyproject.toml"]);
        assert!(nota_de_la_orden(&py, "arregla el test de Python").is_none(), "si el proyecto ya es Python, la ficha va en el sistema");
    }

    /// N6: de cada ficha salen sus PROGRAMAS (no los trozos de código de ejemplo), y lo que no hay en
    /// la Mac se dice con la orden de no instalarlo.
    #[test]
    fn la_ficha_dice_que_programas_faltan() {
        assert_eq!(programas_de(paquete("python").unwrap().ficha()), vec!["uv", "ruff"]);
        assert_eq!(programas_de(paquete("rust").unwrap().ficha()), vec!["cargo"]);
        assert_eq!(programas_de(paquete("go").unwrap().ficha()), vec!["gofmt", "go"]);
        for l in todos() {
            for p in programas_de(l.ficha()) {
                assert!(p.len() >= 2 && !["def", "if", "let", "fn", "var", "for", "except", "async", "try", "new", "no", "sin"].contains(&p), "{}: «{p}» no es un programa", l.id);
            }
        }
        let falta = nota_de_faltantes("`programa-que-no-existe-gotv check` · `bash -n x.sh`");
        assert!(falta.contains("programa-que-no-existe-gotv") && !falta.contains("bash"), "{falta}");
        assert_eq!(nota_de_faltantes("`bash -n x.sh`"), "");
    }

    /// Prueba Cable (2026-11-13): la ley 9 llega en el lenguaje del proyecto o de la orden; Python y
    /// JS/TS ya van en la ley y no se repiten.
    #[test]
    fn la_ley_9_en_cada_lenguaje() {
        let go = proyecto(&["go.mod"]);
        let r = recetas_ley9(&go, "arregla la búsqueda").unwrap();
        assert!(r.contains("runes.Remove") && !r.contains("Rust:"), "{r}");
        let r = recetas_ley9(&go, "y un servicio en Rust con cargo").unwrap();
        assert!(r.contains("runes.Remove") && r.contains("unicode-normalization"), "{r}");
        let py = proyecto(&["pyproject.toml"]);
        assert!(recetas_ley9(&py, "busca sin tildes").is_none());
        for id in ["rust", "go", "java", "kotlin", "csharp", "c", "php", "ruby", "swift", "sql", "dart", "elixir"] {
            assert!(paquete(id).unwrap().texto_sin_tildes().is_some_and(|t| t.len() < 260), "{id}");
        }
    }

    #[test]
    fn el_bloque_trae_los_comandos_y_cabe() {
        let d = proyecto(&["Cargo.toml", "web/x.txt", "deploy.sh"]);
        let b = bloque(&d);
        assert!(b.contains("cargo test") && b.contains("set -euo pipefail"), "{b}");
        assert!(b.len() < 1500, "debe caber barato: {}", b.len());
        assert!(!b.contains("Aquí:") && !b.contains("Here:"), "Rust tiene un solo gestor: sin línea de gestor");
        assert!(b.contains("«lenguaje-rust»"), "la guía se nombra: {b}");
        // cada guía nombrada existe como skill dentro de GOTV
        for p in todos() {
            if let Some(g) = &p.guia {
                assert!(crate::skills::load(&d, g).is_ok_and(|t| t.contains("## Correr una prueba")), "{}: la guía «{g}» no carga", p.id);
            }
        }
        let vacio = proyecto(&["LEEME.txt"]);
        assert_eq!(bloque(&vacio), "");
    }

    /// F11 · el gestor sale del archivo de bloqueo o de `packageManager` CON su versión, y la prueba suelta
    /// del corredor; el nombre de una prueba llega escapado a la terminal.
    #[test]
    fn el_gestor_y_sus_comandos_por_proyecto() {
        let js = paquete("javascript").unwrap();
        let ts = paquete("typescript").unwrap();
        let caso = |archivos: &[&str], pkg: &str| {
            let d = proyecto(archivos);
            std::fs::write(d.join("package.json"), pkg).unwrap();
            d
        };
        let d = caso(&["pnpm-lock.yaml"], r#"{"devDependencies": {"vitest": "^2"}}"#);
        assert_eq!(gestor(&d, js).unwrap().0.nombre, "pnpm");
        assert_eq!(tarea(&d, js, "una_prueba", Some("suma dos")).unwrap(), "pnpm exec vitest run -t 'suma dos'");
        let d = caso(&["bun.lockb"], r#"{"devDependencies": {"jest": "29"}}"#);
        assert_eq!(tarea(&d, js, "probar", None).unwrap(), "bun run test", "`bun test` es otro corredor");
        assert_eq!(tarea(&d, js, "una_prueba", Some("a.*b")).unwrap(), "bunx jest -t 'a.*b'");
        // la VERSIÓN de yarn decide: 1 con su lockfile, 4 por packageManager
        let d = caso(&["yarn.lock"], r#"{"packageManager": "yarn@1.22.19"}"#);
        assert_eq!(gestor(&d, js).unwrap().0.nombre, "yarn 1");
        let d = caso(&["yarn.lock"], r#"{"packageManager": "yarn@4.1.0"}"#);
        let (g, porque) = gestor(&d, js).unwrap();
        assert_eq!((g.nombre.as_str(), porque.as_str()), ("yarn (2 o más)", "packageManager yarn@4.1.0"));
        let d = caso(&[".yarnrc.yml", "yarn.lock"], "{}");
        assert_eq!(gestor(&d, js).unwrap().0.nombre, "yarn (2 o más)");
        let d = caso(&["tsconfig.json"], "{}");
        assert_eq!(gestor(&d, ts).unwrap().0.nombre, "npm", "TypeScript hereda los gestores de Node");
        assert_eq!(tarea(&d, ts, "una_prueba", Some("x")).unwrap(), "node --test --test-name-pattern=x");
        let b = bloque(&d);
        assert!(b.contains("npm (") && b.contains("`npm install`"), "{b}");
        // Python: uv por su candado; sin nada, uv para empezar
        let py = paquete("python").unwrap();
        assert_eq!(tarea(&proyecto(&["pyproject.toml", "uv.lock"]), py, "una_prueba", Some("test_x")).unwrap(), "uv run pytest -k test_x");
        assert_eq!(gestor(&proyecto(&["pyproject.toml", "poetry.lock"]), py).unwrap().0.nombre, "poetry");
        // Dart: Flutter si su pubspec lo dice
        let d = proyecto(&["pubspec.yaml"]);
        std::fs::write(d.join("pubspec.yaml"), "name: x\ndependencies:\n  flutter:\n    sdk: flutter\n").unwrap();
        assert_eq!(tarea(&d, paquete("dart").unwrap(), "probar", None).unwrap(), "flutter test");
        assert_eq!(para_la_terminal("it's"), r"'it'\''s'");
    }

    /// F11 · los servidores y vigilantes van directo a `process`; un `build` no.
    #[test]
    fn lo_que_se_queda_vivo() {
        let d = proyecto(&["package.json", "tsconfig.json", "requirements.txt"]);
        for c in ["npm run dev", "PORT=3000 pnpm dev", "npx vite", "vite --port 5173", "cd web && npm start", "uv run uvicorn app:app --reload", "flask run"] {
            assert!(es_vivo(&d, c), "{c}");
        }
        for c in ["npm run build", "vite build", "npx vite build", "pytest -q", "cd web", "npm test"] {
            assert!(!es_vivo(&d, c), "{c}");
        }
        assert!(!es_vivo(&proyecto(&["Cargo.toml"]), "npm run dev"), "sólo los de los lenguajes del proyecto");
        assert!(variables(&d).iter().any(|(k, _)| k == "npm_config_yes") && variables(&d).iter().any(|(k, _)| k == "PIP_NO_INPUT"));
    }
}
