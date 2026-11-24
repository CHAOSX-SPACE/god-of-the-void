//! F15.1 · la corrida ESTRUCTURADA de cada corredor de pruebas. Qué corredor usa el proyecto, la orden
//! para correr todas, un archivo, un nombre o una lista, y el LECTOR de su resultado (todo en Rust): cuántas
//! pasaron, fallaron y se omitieron, y por cada fallo su nombre, `archivo:línea` y el mensaje.
//!
//! Formatos (salidas reales guardadas en `tests/datos/pruebas/`): `cargo test` (texto de libtest),
//! `unittest` (texto `-v`), `pytest`, `node --test`, `bun test`, `deno test`, PHPUnit, Gradle y Maven
//! (JUnit XML), Jest y Vitest (JSON), `go test -json`, `swift test` (texto de XCTest), `dotnet test` (TRX).
use serde_json::Value;
use std::path::{Path, PathBuf};

/// Una prueba que falló.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Fallo {
    pub nombre: String,
    pub archivo: Option<PathBuf>,
    pub linea: Option<u32>,
    pub mensaje: String,
}

/// El resultado de una corrida.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Corrida {
    pub corredor: String,
    pub pasaron: u32,
    pub fallaron: Vec<Fallo>,
    pub omitidas: u32,
    pub segundos: f32,
    /// la salida completa, a archivo (F6)
    pub registro: Option<PathBuf>,
}

/// Qué correr.
#[derive(Debug, Clone, PartialEq)]
pub enum Que {
    Todas,
    Archivo(PathBuf),
    Nombre(String),
    /// varias por nombre (las que fallaron, las afectadas)
    Varias(Vec<String>),
    /// varios archivos de prueba (las afectadas)
    Archivos(Vec<PathBuf>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Corredor {
    Cargo { release: bool },
    Pytest { py: String },
    Unittest { py: String },
    Jest { bin: String },
    Vitest { bin: String },
    NodeTest,
    Bun,
    Deno,
    Go,
    Phpunit,
    Swift,
    Gradle { orden: String },
    Maven,
    Dotnet,
}

/// Comillas simples de shell.
pub fn comillas(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

fn rel(root: &Path, p: &Path) -> String {
    p.strip_prefix(root).unwrap_or(p).display().to_string()
}

fn hay(root: &Path, f: &str) -> bool {
    root.join(f).exists()
}

fn contiene(root: &Path, f: &str, t: &str) -> bool {
    std::fs::read_to_string(root.join(f)).is_ok_and(|s| s.contains(t))
}

/// El intérprete de Python del proyecto (`.venv`), o `python3`.
fn python(root: &Path) -> String {
    crate::lsp::entorno::python_del_proyecto(root).map(|p| comillas(&p.display().to_string())).unwrap_or_else(|| "python3".into())
}

/// ¿Hay archivos de prueba de Python? (hasta 3 niveles, sin entornos ni dependencias)
fn hay_pruebas_python(root: &Path) -> bool {
    fn baja(d: &Path, n: u8) -> bool {
        if n > 3 {
            return false;
        }
        std::fs::read_dir(d).into_iter().flatten().flatten().any(|e| {
            let p = e.path();
            let nombre = e.file_name().to_string_lossy().into_owned();
            if p.is_dir() {
                !nombre.starts_with('.') && !matches!(nombre.as_str(), "node_modules" | "venv" | "env" | "target" | "__pycache__") && baja(&p, n + 1)
            } else {
                nombre.ends_with(".py") && (nombre.starts_with("test_") || nombre.ends_with("_test.py"))
            }
        })
    }
    baja(root, 0)
}

/// El filtro de un archivo de `src/` de Rust: su módulo (`src/a/b.rs` → `b`; `mod.rs` → la carpeta; `lib.rs`
/// y `main.rs` en `src/` → sin filtro).
pub fn modulo_rust(p: &Path) -> String {
    let stem = p.file_stem().unwrap_or_default().to_string_lossy().into_owned();
    if matches!(stem.as_str(), "mod" | "lib" | "main") {
        let padre = p.parent().and_then(|d| d.file_name()).map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        return if padre == "src" { String::new() } else { padre };
    }
    stem
}

/// Escapa lo que una expresión regular de JS leería como especial.
fn escapa_regex(s: &str) -> String {
    s.chars().flat_map(|c| if "\\^$.|?*+()[]{}".contains(c) { vec!['\\', c] } else { vec![c] }).collect()
}

/// La selección de los corredores de JS (`bandera` es la del nombre en cada uno).
fn sel_js(que: &Que, root: &Path, bandera: &str) -> String {
    match que {
        Que::Todas => String::new(),
        Que::Archivo(p) => format!(" {}", comillas(&rel(root, p))),
        Que::Archivos(v) => v.iter().map(|p| format!(" {}", comillas(&rel(root, p)))).collect(),
        Que::Nombre(n) => format!(" {bandera} {}", comillas(n)),
        Que::Varias(v) => format!(" {bandera} {}", comillas(&v.iter().map(|n| escapa_regex(n)).collect::<Vec<_>>().join("|"))),
    }
}

impl Corredor {
    /// Los corredores del proyecto (puede haber varios: el Rust y el TypeScript de una app Tauri en la raíz).
    pub fn detectar(root: &Path) -> Vec<Corredor> {
        let mut v = Vec::new();
        if hay(root, "Cargo.toml") {
            v.push(Corredor::Cargo { release: hay(root, "target/release") && !hay(root, "target/debug") });
        }
        if hay(root, "go.mod") {
            v.push(Corredor::Go);
        }
        let py = python(root);
        let pytest_cfg = hay(root, "pytest.ini") || hay(root, "conftest.py") || contiene(root, "pyproject.toml", "[tool.pytest") || contiene(root, "setup.cfg", "[tool:pytest]") || contiene(root, "tox.ini", "[pytest]");
        let pytest_en_venv = crate::lsp::entorno::python_del_proyecto(root).is_some_and(|p| p.with_file_name("pytest").is_file());
        if pytest_cfg || pytest_en_venv {
            v.push(Corredor::Pytest { py });
        } else if hay_pruebas_python(root) {
            v.push(Corredor::Unittest { py });
        }
        if let Ok(pkg) = std::fs::read_to_string(root.join("package.json")).map(|s| serde_json::from_str::<Value>(&s).unwrap_or_default()) {
            let dep = |n: &str| ["dependencies", "devDependencies"].iter().any(|k| pkg[k].get(n).is_some());
            let bin = |n: &str| if root.join("node_modules/.bin").join(n).is_file() { format!("node_modules/.bin/{n}") } else { format!("npx {n}") };
            let script = pkg["scripts"]["test"].as_str().unwrap_or("");
            if dep("vitest") {
                v.push(Corredor::Vitest { bin: bin("vitest") });
            } else if dep("jest") {
                v.push(Corredor::Jest { bin: bin("jest") });
            } else if hay(root, "bun.lock") || hay(root, "bun.lockb") || script.contains("bun test") {
                v.push(Corredor::Bun);
            } else if script.contains("node --test") {
                v.push(Corredor::NodeTest);
            }
        }
        if hay(root, "deno.json") || hay(root, "deno.jsonc") {
            v.push(Corredor::Deno);
        }
        if hay(root, "vendor/bin/phpunit") {
            v.push(Corredor::Phpunit);
        }
        if hay(root, "Package.swift") {
            v.push(Corredor::Swift);
        }
        if hay(root, "build.gradle") || hay(root, "build.gradle.kts") {
            v.push(Corredor::Gradle { orden: if hay(root, "gradlew") { "./gradlew".into() } else { "gradle".into() } });
        }
        if hay(root, "pom.xml") {
            v.push(Corredor::Maven);
        }
        let dotnet = std::fs::read_dir(root).into_iter().flatten().flatten().any(|e| e.path().extension().is_some_and(|x| x == "sln" || x == "csproj" || x == "fsproj"));
        if dotnet {
            v.push(Corredor::Dotnet);
        }
        v
    }

    pub fn nombre(&self) -> &'static str {
        match self {
            Corredor::Cargo { .. } => "cargo",
            Corredor::Pytest { .. } => "pytest",
            Corredor::Unittest { .. } => "unittest",
            Corredor::Jest { .. } => "jest",
            Corredor::Vitest { .. } => "vitest",
            Corredor::NodeTest => "node",
            Corredor::Bun => "bun",
            Corredor::Deno => "deno",
            Corredor::Go => "go",
            Corredor::Phpunit => "phpunit",
            Corredor::Swift => "swift",
            Corredor::Gradle { .. } => "gradle",
            Corredor::Maven => "maven",
            Corredor::Dotnet => "dotnet",
        }
    }

    /// ¿Prueba archivos de esta extensión? (para elegir corredor por archivo)
    pub fn atiende(&self, p: &Path) -> bool {
        let e = p.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
        let exts: &[&str] = match self {
            Corredor::Cargo { .. } => &["rs"],
            Corredor::Pytest { .. } | Corredor::Unittest { .. } => &["py"],
            Corredor::Jest { .. } | Corredor::Vitest { .. } | Corredor::NodeTest | Corredor::Bun | Corredor::Deno => &["js", "jsx", "ts", "tsx", "mjs", "cjs", "mts", "cts"],
            Corredor::Go => &["go"],
            Corredor::Phpunit => &["php"],
            Corredor::Swift => &["swift"],
            Corredor::Gradle { .. } | Corredor::Maven => &["java", "kt", "kts", "scala", "groovy"],
            Corredor::Dotnet => &["cs", "fs", "vb"],
        };
        exts.contains(&e.as_str())
    }

    /// La extensión del informe que deja (`None` = se lee la salida de texto).
    pub fn informe(&self) -> Option<&'static str> {
        match self {
            Corredor::Pytest { .. } | Corredor::NodeTest | Corredor::Bun | Corredor::Deno | Corredor::Phpunit => Some("xml"),
            Corredor::Jest { .. } | Corredor::Vitest { .. } => Some("json"),
            Corredor::Dotnet => Some("trx"),
            _ => None,
        }
    }

    /// La orden de shell para correr `que`, con el informe en `inf` (si el corredor usa uno). Se corre
    /// desde la raíz del proyecto.
    pub fn orden(&self, que: &Que, root: &Path, inf: &Path) -> String {
        let inf_s = inf.display().to_string();
        let inf = comillas(&inf_s);
        let archivo = |p: &Path| comillas(&rel(root, p));
        match self {
            Corredor::Cargo { release } => {
                let r = if *release { " --release" } else { "" };
                let base = format!("cargo test --workspace --no-fail-fast{r}");
                let prueba = |p: &Path| comillas(&p.file_stem().unwrap_or_default().to_string_lossy());
                match que {
                    Que::Todas => base,
                    // una prueba de integración: `tests/x.rs` → `--test x`; un archivo de `src/`, por su módulo
                    Que::Archivo(p) if rel(root, p).contains("tests/") => format!("{base} --test {}", prueba(p)),
                    Que::Archivo(p) => format!("{base} -- {}", comillas(&modulo_rust(p))),
                    // varias de `tests/`: `--test a --test b`
                    Que::Archivos(v) => format!("{base}{}", v.iter().map(|p| format!(" --test {}", prueba(p))).collect::<String>()),
                    Que::Nombre(n) => format!("{base} -- {}", comillas(n)),
                    Que::Varias(v) => format!("{base} -- {}", v.iter().map(|n| comillas(n)).collect::<Vec<_>>().join(" ")),
                }
            }
            Corredor::Pytest { py } => {
                let sel = match que {
                    Que::Todas => String::new(),
                    Que::Archivo(p) => format!(" {}", archivo(p)),
                    Que::Archivos(v) => v.iter().map(|p| format!(" {}", archivo(p))).collect(),
                    Que::Nombre(n) => format!(" -k {}", comillas(n)),
                    Que::Varias(v) => format!(" -k {}", comillas(&v.join(" or "))),
                };
                format!("{py} -m pytest -q --junitxml={inf}{sel}")
            }
            Corredor::Unittest { py } => {
                // `tests/test_x.py` → `tests.test_x`
                let modulo = |p: &Path| comillas(&rel(root, p).trim_end_matches(".py").replace('/', "."));
                let sel = match que {
                    Que::Todas => String::new(),
                    Que::Archivo(p) => format!(" {}", modulo(p)),
                    Que::Archivos(v) => v.iter().map(|p| format!(" {}", modulo(p))).collect(),
                    Que::Nombre(n) => format!(" -k {}", comillas(n)),
                    Que::Varias(v) => v.iter().map(|n| format!(" -k {}", comillas(n))).collect(),
                };
                format!("{py} -m unittest -v{sel}")
            }
            Corredor::Jest { bin } => format!("{bin} --json --outputFile={inf}{}", sel_js(que, root, "-t")),
            Corredor::Vitest { bin } => format!("{bin} run --reporter=json --outputFile={inf}{}", sel_js(que, root, "-t")),
            Corredor::NodeTest => format!("node --test --test-reporter=junit --test-reporter-destination={inf}{}", sel_js(que, root, "--test-name-pattern")),
            Corredor::Bun => format!("bun test --reporter=junit --reporter-outfile={inf}{}", sel_js(que, root, "-t")),
            Corredor::Deno => format!("deno test --junit-path={inf}{}", sel_js(que, root, "--filter")),
            Corredor::Go => {
                let paquete = |p: &Path| {
                    let d = rel(root, p.parent().unwrap_or(root));
                    if d.is_empty() {
                        ".".to_string()
                    } else {
                        format!("./{d}")
                    }
                };
                match que {
                    Que::Todas => "go test -json ./...".into(),
                    Que::Archivo(p) => format!("go test -json {}", paquete(p)),
                    Que::Archivos(v) => {
                        let mut dirs: Vec<String> = v.iter().map(|p| paquete(p)).collect();
                        dirs.sort();
                        dirs.dedup();
                        format!("go test -json {}", dirs.join(" "))
                    }
                    Que::Nombre(n) => format!("go test -json -run {} ./...", comillas(n)),
                    Que::Varias(v) => format!("go test -json -run {} ./...", comillas(&format!("^({})$", v.join("|")))),
                }
            }
            Corredor::Phpunit => {
                let sel = match que {
                    Que::Todas => String::new(),
                    Que::Archivo(p) => format!(" {}", archivo(p)),
                    // PHPUnit toma UN archivo: con varios, todas
                    Que::Archivos(v) if v.len() == 1 => format!(" {}", archivo(&v[0])),
                    Que::Archivos(_) => String::new(),
                    Que::Nombre(n) => format!(" --filter {}", comillas(n)),
                    Que::Varias(v) => format!(" --filter {}", comillas(&v.join("|"))),
                };
                format!("vendor/bin/phpunit --log-junit {inf}{sel}")
            }
            Corredor::Swift => match que {
                Que::Todas | Que::Archivo(_) | Que::Archivos(_) => "swift test".into(),
                Que::Nombre(n) => format!("swift test --filter {}", comillas(n)),
                Que::Varias(v) => format!("swift test --filter {}", comillas(&v.join("|"))),
            },
            Corredor::Gradle { orden } => match que {
                Que::Todas | Que::Archivo(_) | Que::Archivos(_) => format!("{orden} test"),
                Que::Nombre(n) => format!("{orden} test --tests {}", comillas(n)),
                Que::Varias(v) => format!("{orden} test {}", v.iter().map(|n| format!("--tests {}", comillas(n))).collect::<Vec<_>>().join(" ")),
            },
            Corredor::Maven => match que {
                Que::Todas | Que::Archivo(_) | Que::Archivos(_) => "mvn -q test".into(),
                Que::Nombre(n) => format!("mvn -q test -Dtest={}", comillas(n)),
                Que::Varias(v) => format!("mvn -q test -Dtest={}", comillas(&v.join(","))),
            },
            Corredor::Dotnet => {
                let sel = match que {
                    Que::Todas | Que::Archivo(_) | Que::Archivos(_) => String::new(),
                    Que::Nombre(n) => format!(" --filter {}", comillas(n)),
                    Que::Varias(v) => format!(" --filter {}", comillas(&v.join("|"))),
                };
                format!("dotnet test --logger {}{sel}", comillas(&format!("trx;LogFileName={inf_s}")))
            }
        }
    }

    /// Cómo se corre UNA prueba (para `descubrir`).
    pub fn como_una(&self) -> String {
        // los huecos de la plantilla, en el idioma del usuario
        let (a, n, p, c, m, k) = (
            crate::i18n::t("<archivo>", "<file>"),
            crate::i18n::t("<nombre>", "<name>"),
            crate::i18n::t("<prueba>", "<test>"),
            crate::i18n::t("<Clase>", "<Class>"),
            crate::i18n::t("<modulo>", "<module>"),
            crate::i18n::t("<paquete>", "<package>"),
        );
        match self {
            Corredor::Cargo { .. } => format!("cargo test --workspace -- {} --exact", crate::i18n::t("<ruta::de::la_prueba>", "<path::to::the_test>")),
            Corredor::Pytest { py } => format!("{py} -m pytest {a}::{c}::{p}"),
            Corredor::Unittest { py } => format!("{py} -m unittest {m}.{c}.{p}"),
            Corredor::Jest { bin } => format!("{bin} {a} -t {n}"),
            Corredor::Vitest { bin } => format!("{bin} run {a} -t {n}"),
            Corredor::NodeTest => format!("node --test --test-name-pattern={n} {a}"),
            Corredor::Bun => format!("bun test {a} -t {n}"),
            Corredor::Deno => format!("deno test {a} --filter {n}"),
            Corredor::Go => format!("go test -run '^{n}$' ./{k}"),
            Corredor::Phpunit => format!("vendor/bin/phpunit --filter {n}"),
            Corredor::Swift => format!("swift test --filter <Suite>/{p}"),
            Corredor::Gradle { orden } => format!("{orden} test --tests {c}.{p}"),
            Corredor::Maven => format!("mvn -q test -Dtest={c}#{p}"),
            Corredor::Dotnet => format!("dotnet test --filter {n}"),
        }
    }

    /// Lee el resultado: el informe si lo hay, si no la salida de texto. Una salida truncada o rara nunca
    /// rompe: da lo que se pudo leer.
    pub fn leer(&self, salida: &str, informe: Option<&str>, root: &Path) -> Corrida {
        let salida = &*super::limpia(salida);
        let mut c = match (self, informe) {
            (Corredor::Jest { .. } | Corredor::Vitest { .. }, Some(j)) => leer_jest(j, root),
            (Corredor::Dotnet, Some(x)) => super::junit::leer_trx(x, root),
            (_, Some(x)) if x.trim_start().starts_with('<') => super::junit::leer_junit(x, root),
            (Corredor::Cargo { .. }, _) => leer_libtest(salida),
            (Corredor::Unittest { .. }, _) => leer_unittest(salida, root),
            (Corredor::Go, _) => leer_go(salida, root),
            (Corredor::Swift, _) => leer_xctest(salida),
            _ => Corrida::default(),
        };
        c.corredor = self.nombre().to_string();
        c
    }
}

/// `archivo:línea` del primer lugar de un texto que sea del PROYECTO: rutas absolutas bajo `root`, o
/// relativas con extensión; nunca `node_modules`, `node:`, `http`. Entiende `(ruta:12:5)`, `file:///ruta:5:3`,
/// `ruta:12: Error` y `File "ruta", line 12`.
pub fn lugar(texto: &str, root: &Path) -> Option<(PathBuf, u32)> {
    let raiz = root.display().to_string();
    let valida = |r: &str| -> Option<PathBuf> {
        if r.contains("node_modules") || r.starts_with("node:") || r.starts_with("http") || r.contains("/rustc/") || r.contains(".cargo/registry") {
            return None;
        }
        let p = Path::new(r);
        let ext = p.extension()?.to_string_lossy().to_lowercase();
        if ext.is_empty() || ext.len() > 6 || !ext.chars().all(|c| c.is_ascii_alphanumeric()) {
            return None;
        }
        if p.is_absolute() {
            r.starts_with(&raiz).then(|| p.to_path_buf())
        } else {
            Some(root.join(p))
        }
    };
    for l in texto.lines() {
        // Python: File "ruta", line 12
        if let Some(i) = l.find("File \"") {
            if let Some((r, tras)) = l[i + 6..].split_once('"') {
                let n = tras.trim_start_matches(", line ").split(|c: char| !c.is_ascii_digit()).next().and_then(|n| n.parse().ok());
                if let (Some(n), Some(p)) = (n, valida(r)) {
                    return Some((p, n));
                }
            }
        }
        for pieza in l.split(|c: char| c.is_whitespace() || matches!(c, '(' | ')' | '\'' | '"' | ',')) {
            let pieza = pieza.trim_start_matches("file://").trim_end_matches(':');
            let mut partes = pieza.split(':');
            let (Some(r), Some(n)) = (partes.next(), partes.next()) else { continue };
            let Ok(n) = n.parse::<u32>() else { continue };
            if let Some(p) = valida(r) {
                return Some((p, n));
            }
        }
    }
    None
}

/// Hasta 10 líneas no vacías de un mensaje.
pub fn recorta(m: &str) -> String {
    m.lines().map(str::trim_end).filter(|l| !l.trim().is_empty()).take(10).collect::<Vec<_>>().join("\n")
}

/// `cargo test` (libtest, texto). Suma los `test result:` de cada binario; los fallos salen de sus bloques
/// `---- nombre stdout ----` (el pánico dice `archivo:línea`). Si no compiló, un fallo «(compilación)» con
/// el primer error.
pub fn leer_libtest(salida: &str) -> Corrida {
    let mut c = Corrida::default();
    let mut resumenes = false;
    let (mut ok_l, mut ign_l) = (0u32, 0u32);
    let mut fallidos: Vec<String> = Vec::new();
    for l in salida.lines() {
        let t = l.trim();
        if let Some(r) = t.strip_prefix("test result: ") {
            resumenes = true;
            for parte in r.split(';') {
                // «FAILED. 1 passed», « 1 ignored»: el número va justo antes de la palabra
                let p = parte.trim();
                let num = p.split_whitespace().rev().nth(1).and_then(|n| n.parse::<u32>().ok()).unwrap_or(0);
                if p.ends_with(" passed") {
                    c.pasaron += num;
                } else if p.ends_with(" ignored") {
                    c.omitidas += num;
                }
            }
        } else if let Some(r) = t.strip_prefix("test ") {
            if let Some((n, estado)) = r.rsplit_once(" ... ") {
                match estado.trim() {
                    "ok" => ok_l += 1,
                    "ignored" => ign_l += 1,
                    e if e.starts_with("FAILED") => fallidos.push(n.trim().to_string()),
                    _ => {}
                }
            }
        }
    }
    if !resumenes {
        // salida truncada: lo que dijeron las líneas
        c.pasaron = ok_l;
        c.omitidas = ign_l;
    }
    let lineas: Vec<&str> = salida.lines().collect();
    for n in &fallidos {
        let cab = format!("---- {n} stdout ----");
        let mut f = Fallo { nombre: n.clone(), ..Default::default() };
        if let Some(i) = lineas.iter().position(|l| l.trim() == cab) {
            let cuerpo: Vec<&str> = lineas[i + 1..].iter().take_while(|l| !l.starts_with("---- ") && l.trim() != "failures:").copied().collect();
            if let Some(j) = cuerpo.iter().position(|l| l.contains("panicked at ")) {
                let tras = cuerpo[j].split("panicked at ").nth(1).unwrap_or("").trim_end_matches(':');
                let mut p = tras.split(':');
                if let (Some(a), Some(b)) = (p.next(), p.next()) {
                    f.archivo = Some(PathBuf::from(a));
                    f.linea = b.parse().ok();
                }
                f.mensaje = recorta(&cuerpo[j + 1..].iter().filter(|l| !l.starts_with("note: run with")).copied().collect::<Vec<_>>().join("\n"));
            } else {
                f.mensaje = recorta(&cuerpo.join("\n"));
            }
        }
        c.fallaron.push(f);
    }
    // no compiló: ni un resumen ni una prueba, y un error de rustc
    if !resumenes && fallidos.is_empty() && ok_l == 0 {
        if let Some(i) = lineas.iter().position(|l| l.starts_with("error[") || l.starts_with("error:")) {
            let mut f = Fallo { nombre: crate::i18n::t("(compilación)", "(build)").into(), mensaje: recorta(&lineas[i..].join("\n")), ..Default::default() };
            if let Some(d) = lineas[i..].iter().find_map(|l| l.trim_start().strip_prefix("--> ")) {
                let mut p = d.split(':');
                f.archivo = p.next().map(PathBuf::from);
                f.linea = p.next().and_then(|n| n.parse().ok());
            }
            c.fallaron.push(f);
        }
    }
    c
}

/// El número tras `clave` (p. ej. `skipped=3`).
fn cuenta_en(t: &str, clave: &str) -> u32 {
    t.rfind(clave).and_then(|i| t[i + clave.len()..].split(|c: char| !c.is_ascii_digit()).next()?.parse().ok()).unwrap_or(0)
}

/// `python -m unittest -v` (texto).
pub fn leer_unittest(salida: &str, root: &Path) -> Corrida {
    let mut c = Corrida::default();
    let mut corridas: Option<u32> = None;
    let (mut ok_l, mut skip_l) = (0u32, 0u32);
    for l in salida.lines() {
        let t = l.trim();
        if let Some(r) = t.strip_prefix("Ran ") {
            corridas = r.split_whitespace().next().and_then(|n| n.parse().ok());
        }
        if t.ends_with(" ... ok") {
            ok_l += 1;
        } else if t.contains(" ... skipped") {
            skip_l += 1;
        }
    }
    // los bloques FAIL: / ERROR:
    let lineas: Vec<&str> = salida.lines().collect();
    for (i, l) in lineas.iter().enumerate() {
        let Some(cab) = l.strip_prefix("FAIL: ").or_else(|| l.strip_prefix("ERROR: ")) else { continue };
        let nombre = cab.split_whitespace().next().unwrap_or(cab).to_string();
        let cuerpo: Vec<&str> = lineas[i + 1..].iter().skip_while(|l| l.starts_with("---")).take_while(|l| !l.starts_with("======") && !l.starts_with("------")).copied().collect();
        let mut f = Fallo { nombre, ..Default::default() };
        // el ÚLTIMO `File "…"` del proyecto (el más cercano al fallo); el mensaje, al final
        for cl in &cuerpo {
            if cl.trim_start().starts_with("File \"") {
                if let Some((p, n)) = lugar(cl, root) {
                    f.archivo = Some(p);
                    f.linea = Some(n);
                }
            }
        }
        let ultimas: Vec<&str> = cuerpo.iter().rev().filter(|l| !l.trim().is_empty()).take(3).copied().collect::<Vec<_>>().into_iter().rev().collect();
        f.mensaje = recorta(&ultimas.join("\n"));
        c.fallaron.push(f);
    }
    c.omitidas = skip_l.max(cuenta_en(salida, "skipped="));
    c.pasaron = match corridas {
        Some(n) => n.saturating_sub(c.fallaron.len() as u32 + c.omitidas),
        None => ok_l,
    };
    c
}

/// `go test -json` (una línea JSON por evento). El `archivo:línea` de la salida de una prueba trae sólo el
/// nombre del archivo: la carpeta sale del paquete (sin el módulo de `go.mod`).
pub fn leer_go(salida: &str, root: &Path) -> Corrida {
    let modulo = std::fs::read_to_string(root.join("go.mod")).ok().and_then(|s| s.lines().find_map(|l| l.strip_prefix("module ").map(|m| m.trim().to_string()))).unwrap_or_default();
    let mut c = Corrida::default();
    let mut textos: std::collections::HashMap<(String, String), String> = Default::default();
    // lo que dijo el compilador, por paquete (`build-output`, con `ImportPath` = «paquete [paquete.test]»)
    let mut compilacion: std::collections::HashMap<String, String> = Default::default();
    let mut con_fallos: std::collections::HashSet<String> = Default::default();
    let mut paquetes_rotos: Vec<String> = Vec::new();
    for l in salida.lines() {
        let Ok(e) = serde_json::from_str::<Value>(l) else { continue };
        let pkg = e["Package"].as_str().unwrap_or("").to_string();
        let test = e["Test"].as_str().unwrap_or("").to_string();
        match e["Action"].as_str().unwrap_or("") {
            "build-output" => {
                let ip = e["ImportPath"].as_str().unwrap_or("").split(' ').next().unwrap_or("").to_string();
                compilacion.entry(ip).or_default().push_str(e["Output"].as_str().unwrap_or(""));
            }
            "output" => textos.entry((pkg, test)).or_default().push_str(e["Output"].as_str().unwrap_or("")),
            "pass" if !test.is_empty() => c.pasaron += 1,
            "skip" if !test.is_empty() => c.omitidas += 1,
            "fail" if !test.is_empty() => {
                con_fallos.insert(pkg.clone());
                let salida = textos.get(&(pkg.clone(), test.clone())).cloned().unwrap_or_default();
                let carpeta = root.join(pkg.strip_prefix(&modulo).unwrap_or(&pkg).trim_start_matches('/'));
                let mut f = Fallo { nombre: test, ..Default::default() };
                let utiles: Vec<&str> = salida.lines().filter(|l| !l.starts_with("=== ") && !l.starts_with("--- ")).collect();
                let donde = utiles.iter().find_map(|l| {
                    let (a, resto) = l.trim().split_once(':')?;
                    a.ends_with(".go").then_some(())?;
                    Some((carpeta.join(a), resto.split(':').next()?.parse::<u32>().ok()?))
                });
                if let Some((a, n)) = donde {
                    f.archivo = Some(a);
                    f.linea = Some(n);
                }
                f.mensaje = recorta(&utiles.join("\n"));
                c.fallaron.push(f);
            }
            "fail" => paquetes_rotos.push(pkg),
            _ => {}
        }
    }
    // un paquete que falla sin ninguna prueba que falle: no compiló (o un `TestMain` que cayó)
    for pkg in paquetes_rotos.into_iter().filter(|p| !con_fallos.contains(p)) {
        let texto = format!("{}{}", compilacion.get(&pkg).map(String::as_str).unwrap_or(""), textos.get(&(pkg.clone(), String::new())).map(String::as_str).unwrap_or(""));
        let utiles: Vec<&str> = texto.lines().filter(|l| !l.starts_with("# ") && !l.starts_with("FAIL")).collect();
        let mut f = Fallo { nombre: pkg, mensaje: recorta(&utiles.join("\n")), ..Default::default() };
        if let Some((p, n)) = lugar(&texto, root) {
            f.archivo = Some(p);
            f.linea = Some(n);
        }
        c.fallaron.push(f);
    }
    c
}

/// Jest y Vitest (`--json` / `--reporter=json`; el mismo esquema). Un archivo que ni corrió (no compiló)
/// es un fallo con su mensaje.
pub fn leer_jest(json: &str, root: &Path) -> Corrida {
    let Ok(j) = serde_json::from_str::<Value>(json) else { return Corrida::default() };
    let n = |k: &str| j[k].as_u64().unwrap_or(0) as u32;
    let mut c = Corrida { pasaron: n("numPassedTests"), omitidas: n("numPendingTests") + n("numTodoTests"), ..Default::default() };
    for r in j["testResults"].as_array().into_iter().flatten() {
        let archivo = r["name"].as_str().map(PathBuf::from);
        let pruebas = r["assertionResults"].as_array().cloned().unwrap_or_default();
        for a in pruebas.iter().filter(|a| a["status"] == "failed") {
            let mensaje = a["failureMessages"].as_array().into_iter().flatten().filter_map(Value::as_str).collect::<Vec<_>>().join("\n");
            // el lugar: la línea del ARCHIVO de la prueba en la pila; si no, la primera del proyecto; si no, la
            // que diga el corredor
            let en_archivo = archivo.as_ref().and_then(|ar| mensaje.lines().find_map(|l| lugar(l, root).filter(|(p, _)| p == ar)));
            let (p, l) = match en_archivo.or_else(|| lugar(&mensaje, root)) {
                Some((p, l)) => (Some(p), Some(l)),
                None => (archivo.clone(), a["location"]["line"].as_u64().map(|l| l as u32)),
            };
            let limpio: Vec<&str> = mensaje.lines().filter(|l| !l.trim_start().starts_with("at ")).collect();
            c.fallaron.push(Fallo { nombre: a["fullName"].as_str().or(a["title"].as_str()).unwrap_or("?").to_string(), archivo: p, linea: l, mensaje: recorta(&limpio.join("\n")) });
        }
        if pruebas.is_empty() && r["status"] == "failed" {
            let m = r["message"].as_str().unwrap_or("");
            let (p, l) = lugar(m, root).map(|(p, l)| (Some(p), Some(l))).unwrap_or((archivo.clone(), None));
            c.fallaron.push(Fallo { nombre: archivo.as_ref().map(|a| rel(root, a)).unwrap_or_default(), archivo: p, linea: l, mensaje: recorta(m) });
        }
    }
    c
}

/// `swift test` (texto de XCTest: `ruta:línea: error: -[Suite prueba] : mensaje`).
pub fn leer_xctest(salida: &str) -> Corrida {
    let mut c = Corrida::default();
    let mut mensajes: std::collections::HashMap<String, (PathBuf, u32, String)> = Default::default();
    for l in salida.lines() {
        let t = l.trim();
        if let Some(i) = t.find(": error: -[") {
            let (donde, resto) = (&t[..i], &t[i + 9..]);
            let nombre = resto.split(']').next().unwrap_or("").trim_start_matches("-[").to_string();
            let msg = resto.split_once("] : ").map(|x| x.1).unwrap_or(resto).to_string();
            let mut p = donde.rsplitn(2, ':');
            let (n, a) = (p.next().and_then(|n| n.parse().ok()).unwrap_or(0), p.next().unwrap_or(""));
            mensajes.entry(nombre).or_insert((PathBuf::from(a), n, msg));
        } else if let Some(resto) = t.strip_prefix("Test Case '") {
            let nombre = resto.split('\'').next().unwrap_or("").trim_start_matches("-[").trim_end_matches(']').to_string();
            if t.contains("' passed (") {
                c.pasaron += 1;
            } else if t.contains("' skipped (") {
                c.omitidas += 1;
            } else if t.contains("' failed (") {
                let (a, n, m) = mensajes.get(&nombre).cloned().unwrap_or_default();
                c.fallaron.push(Fallo { nombre, archivo: (!a.as_os_str().is_empty()).then_some(a), linea: (n > 0).then_some(n), mensaje: recorta(&m) });
            }
        }
    }
    c
}

impl Corrida {
    pub fn total(&self) -> u32 {
        self.pasaron + self.fallaron.len() as u32 + self.omitidas
    }

    /// Junta otra corrida (los informes de Gradle y Maven vienen en varios archivos).
    pub fn sumar(&mut self, otra: Corrida) {
        self.pasaron += otra.pasaron;
        self.omitidas += otra.omitidas;
        self.fallaron.extend(otra.fallaron);
    }

    /// `Pasaron 212 · Fallaron 2 · Omitidas 3 (41 s) · cargo` y, por cada fallo (15 como mucho), su nombre,
    /// `archivo:línea` y hasta 10 líneas del mensaje.
    pub fn texto(&self, root: &Path) -> String {
        let raiz = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
        let mut s = crate::tr!(
            "Pasaron {} · Fallaron {} · Omitidas {} ({:.0} s) · {}",
            "Passed {} · Failed {} · Skipped {} ({:.0} s) · {}",
            self.pasaron,
            self.fallaron.len(),
            self.omitidas,
            self.segundos,
            self.corredor
        );
        for f in self.fallaron.iter().take(15) {
            let donde = match (&f.archivo, f.linea) {
                (Some(a), Some(l)) => format!(" — {}:{l}", rel(&raiz, a)),
                (Some(a), None) => format!(" — {}", rel(&raiz, a)),
                _ => String::new(),
            };
            s.push_str(&format!("\n✗ {}{donde}", f.nombre));
            for l in f.mensaje.lines() {
                s.push_str(&format!("\n    {l}"));
            }
        }
        if self.fallaron.len() > 15 {
            s.push_str(&crate::tr!("\n… y {} fallos más", "\n… and {} more failures", self.fallaron.len() - 15));
        }
        if let Some(r) = &self.registro {
            s.push_str(&crate::tr!("\nsalida completa: {}", "\nfull output: {}", rel(&raiz, r)));
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dato(n: &str) -> String {
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/datos/pruebas").join(n)).unwrap()
    }

    fn raiz() -> PathBuf {
        PathBuf::from("/RAIZ")
    }

    #[test]
    fn cargo_libtest() {
        let c = leer_libtest(&dato("cargo.txt"));
        assert_eq!((c.pasaron, c.fallaron.len(), c.omitidas), (2, 2, 1), "{c:?}");
        assert_eq!(c.fallaron[0].nombre, "tests::suma_mal");
        assert_eq!((c.fallaron[0].archivo.clone(), c.fallaron[0].linea), (Some(PathBuf::from("src/lib.rs")), Some(9)));
        assert!(c.fallaron[0].mensaje.contains("dos y dos") && c.fallaron[0].mensaje.contains("right: 5"), "{}", c.fallaron[0].mensaje);
        assert_eq!((c.fallaron[1].nombre.as_str(), c.fallaron[1].linea), ("integra_rota", Some(4)));
        // truncada a la mitad: no rompe, cuenta lo que vio
        let medio = dato("cargo.txt").lines().take(12).collect::<Vec<_>>().join("\n");
        let c = leer_libtest(&medio);
        assert_eq!((c.pasaron, c.fallaron.len(), c.omitidas), (1, 1, 1));
        // no compiló
        let c = leer_libtest("   Compiling x v0.1.0\nerror[E0308]: mismatched types\n  --> src/lib.rs:3:5\n   |\n3  |     \"a\"\n");
        assert_eq!((c.fallaron[0].archivo.clone(), c.fallaron[0].linea), (Some(PathBuf::from("src/lib.rs")), Some(3)));
    }

    #[test]
    fn python_unittest() {
        let c = leer_unittest(&dato("unittest.txt"), &raiz());
        assert_eq!((c.pasaron, c.fallaron.len(), c.omitidas), (1, 2, 1), "{c:?}");
        let mal = c.fallaron.iter().find(|f| f.nombre == "test_mal").unwrap();
        assert_eq!((mal.archivo.clone(), mal.linea), (Some(PathBuf::from("/RAIZ/tests/test_calc.py")), Some(9)));
        assert!(mal.mensaje.contains("AssertionError: 6 != 7"), "{}", mal.mensaje);
        assert!(c.fallaron.iter().any(|f| f.nombre == "test_error" && f.linea == Some(12)));
    }

    #[test]
    fn go_json() {
        let d = crate::CarpetaPrueba::nueva("gotv-pruebas-go");
        std::fs::write(d.join("go.mod"), "module juguete\n\ngo 1.22\n").unwrap();
        let c = leer_go(&dato("go.jsonl"), &d);
        assert_eq!((c.pasaron, c.fallaron.len(), c.omitidas), (1, 1, 1), "{c:?}");
        assert_eq!(c.fallaron[0].nombre, "TestMal");
        assert_eq!((c.fallaron[0].archivo.clone(), c.fallaron[0].linea), (Some(d.join("calc/calc_test.go")), Some(13)));
        assert!(c.fallaron[0].mensaje.contains("quiero 7"));
        // no compiló (salida real): el paquete es el fallo, con el error del compilador
        let c = leer_go(&dato("go-roto.jsonl"), &d);
        assert_eq!(c.fallaron.len(), 1, "{c:?}");
        assert_eq!((c.fallaron[0].nombre.as_str(), c.fallaron[0].archivo.clone(), c.fallaron[0].linea), ("juguete/calc", Some(d.join("calc/calc.go")), Some(3)));
        assert!(c.fallaron[0].mensaje.contains("mismatched types"), "{}", c.fallaron[0].mensaje);
    }

    #[test]
    fn jest_y_vitest() {
        for (f, archivo, linea) in [("jest.json", "/RAIZ/calc.test.js", 4), ("vitest.json", "/RAIZ/v/calc.test.mjs", 5)] {
            let c = leer_jest(&dato(f), &raiz());
            assert_eq!((c.pasaron, c.fallaron.len(), c.omitidas), (1, 1, 1), "{f}: {c:?}");
            assert_eq!(c.fallaron[0].nombre, "mal");
            assert_eq!((c.fallaron[0].archivo.clone(), c.fallaron[0].linea), (Some(PathBuf::from(archivo)), Some(linea)), "{f}");
            assert!(c.fallaron[0].mensaje.contains('7') && !c.fallaron[0].mensaje.contains("node_modules"), "{}", c.fallaron[0].mensaje);
        }
        assert_eq!(leer_jest("{trunc", &raiz()), Corrida::default());
    }

    #[test]
    fn swift_xctest() {
        let c = leer_xctest(&dato("swift.txt"));
        assert_eq!((c.pasaron, c.fallaron.len()), (1, 1), "{c:?}");
        assert_eq!(c.fallaron[0].nombre, "CalcTests.CalcTests testMal");
        assert_eq!((c.fallaron[0].archivo.clone(), c.fallaron[0].linea), (Some(PathBuf::from("Tests/CalcTests/CalcTests.swift")), Some(7)));
        assert!(c.fallaron[0].mensaje.contains("is not equal to"));
    }

    #[test]
    fn deteccion_y_ordenes() {
        let d = crate::CarpetaPrueba::nueva("gotv-pruebas-detectar");
        let r = d.canonicalize().unwrap();
        std::fs::write(r.join("Cargo.toml"), "[package]").unwrap();
        std::fs::write(r.join("package.json"), r#"{"devDependencies": {"vitest": "3"}}"#).unwrap();
        std::fs::create_dir_all(r.join("tests")).unwrap();
        std::fs::write(r.join("tests/test_a.py"), "").unwrap();
        let v = Corredor::detectar(&r);
        assert_eq!(v.iter().map(Corredor::nombre).collect::<Vec<_>>(), ["cargo", "unittest", "vitest"]);
        let inf = r.join(".gotv/pruebas/informe.json");
        let cargo = &v[0];
        assert_eq!(cargo.orden(&Que::Archivo(r.join("tests/integra.rs")), &r, &inf), "cargo test --workspace --no-fail-fast --test 'integra'");
        assert_eq!(cargo.orden(&Que::Archivo(r.join("src/datos/mod.rs")), &r, &inf), "cargo test --workspace --no-fail-fast -- 'datos'");
        assert_eq!(cargo.orden(&Que::Varias(vec!["a::b".into(), "c".into()]), &r, &inf), "cargo test --workspace --no-fail-fast -- 'a::b' 'c'");
        assert_eq!(cargo.orden(&Que::Archivos(vec![r.join("tests/a.rs"), r.join("tests/b.rs")]), &r, &inf), "cargo test --workspace --no-fail-fast --test 'a' --test 'b'");
        assert_eq!(v[1].orden(&Que::Archivo(r.join("tests/test_a.py")), &r, &inf), "python3 -m unittest -v 'tests.test_a'");
        assert!(v[2].orden(&Que::Varias(vec!["a.b".into(), "c".into()]), &r, &inf).ends_with("-t 'a\\.b|c'"));
        assert!(v[2].atiende(Path::new("x.tsx")) && !v[2].atiende(Path::new("x.rs")));
        assert_eq!(Corredor::Go.orden(&Que::Archivo(r.join("calc/calc_test.go")), &r, &inf), "go test -json ./calc");
        assert_eq!(Corredor::Go.orden(&Que::Archivos(vec![r.join("a/x.go"), r.join("a/y.go"), r.join("z.go")]), &r, &inf), "go test -json . ./a");
        // con pytest configurado gana pytest
        std::fs::write(r.join("pytest.ini"), "[pytest]").unwrap();
        assert!(Corredor::detectar(&r).iter().any(|c| c.nombre() == "pytest"));
    }

    #[test]
    fn el_lugar_de_un_fallo() {
        let r = raiz();
        assert_eq!(lugar("    at Object.toBe (/RAIZ/calc.test.js:4:20)", &r), Some((PathBuf::from("/RAIZ/calc.test.js"), 4)));
        assert_eq!(lugar("    at file:///RAIZ/calc_test.ts:5:3", &r), Some((PathBuf::from("/RAIZ/calc_test.ts"), 5)));
        assert_eq!(lugar("tests/test_calc.py:12: ValueError", &r), Some((PathBuf::from("/RAIZ/tests/test_calc.py"), 12)));
        assert_eq!(lugar("  File \"/RAIZ/t.py\", line 7, in x", &r), Some((PathBuf::from("/RAIZ/t.py"), 7)));
        assert_eq!(lugar("at x (/RAIZ/node_modules/j/a.js:1:2)\n at node:internal/x:3:4", &r), None);
        assert_eq!(lugar("at (/otro/sitio/a.js:1:2)", &r), None, "fuera del proyecto");
    }
}
