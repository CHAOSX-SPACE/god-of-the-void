//! F15.1 · la herramienta `pruebas`. Corre el corredor del proyecto dentro de la caja de la sesión (como
//! `bash`), lee su resultado ESTRUCTURADO y se lo da al modelo resumido (`Pasaron · Fallaron · Omitidas` y
//! cada fallo con `archivo:línea`); la salida completa va a `.gotv/registros/pruebas-<corredor>.log`. Lo que
//! falló la última vez queda en `.gotv/pruebas/ultima.gotv` (formato propio) para `fallidas`.
use super::corrida::{Corredor, Corrida, Fallo, Que};
use anyhow::{bail, Result};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Lo que la herramienta necesita del agente.
pub struct Contexto<'a> {
    pub root: &'a Path,
    pub spec: Option<crate::sandbox::Spec>,
    /// los archivos que tocó el turno (para `afectadas`)
    pub tocados: &'a [PathBuf],
    /// archivos que USAN lo tocado, según el servidor de lenguaje (F5): las pruebas entre ellos son afectadas
    pub usan: Vec<PathBuf>,
}

/// La ruta real (sin enlaces), o la misma si no existe.
fn canon(p: &Path) -> PathBuf {
    p.canonicalize().unwrap_or_else(|_| p.to_path_buf())
}

fn carpeta(root: &Path) -> PathBuf {
    root.join(".gotv/pruebas")
}

/// Lo que falló la última vez, por corredor.
fn ultima(root: &Path) -> BTreeMap<String, Vec<String>> {
    let c = carpeta(root);
    crate::programar::codec::leer_serde_migrado(&c.join("ultima.gotv"), &c.join("ultima.json"), b"PRUL").unwrap_or_default()
}

fn guardar_ultima(root: &Path, c: &Corrida) {
    let mut u = ultima(root);
    // los fallos de compilación o de un paquete no son pruebas que se puedan repetir por nombre
    let nombres: Vec<String> = c.fallaron.iter().map(|f| f.nombre.clone()).filter(|n| !n.starts_with('(') && !n.contains('/')).collect();
    u.insert(c.corredor.clone(), nombres);
    let _ = crate::programar::codec::guardar_serde(&carpeta(root).join("ultima.gotv"), b"PRUL", &u);
}

/// Los informes JUnit de Gradle y Maven que se escribieron desde `desde` (varios archivos).
fn informes_de_carpetas(root: &Path, desde: std::time::SystemTime) -> Vec<String> {
    fn baja(d: &Path, n: u8, desde: std::time::SystemTime, out: &mut Vec<String>) {
        if n > 6 || out.len() > 500 {
            return;
        }
        for e in std::fs::read_dir(d).into_iter().flatten().flatten() {
            let p = e.path();
            let nombre = e.file_name().to_string_lossy().into_owned();
            if p.is_dir() {
                if !nombre.starts_with('.') && nombre != "node_modules" {
                    baja(&p, n + 1, desde, out);
                }
            } else if nombre.starts_with("TEST-") && nombre.ends_with(".xml") && e.metadata().and_then(|m| m.modified()).is_ok_and(|t| t >= desde) {
                if let Ok(s) = std::fs::read_to_string(&p) {
                    out.push(s);
                }
            }
        }
    }
    let mut out = Vec::new();
    baja(root, 0, desde, &mut out);
    out
}

/// Corre `que` con `corredor`: la corrida leída, con su registro a archivo. Si el corredor no dijo nada
/// legible y salió mal, un fallo con las últimas líneas (no compiló, falta una dependencia…).
pub async fn correr(cx: &Contexto<'_>, corredor: &Corredor, que: &Que, tope: Duration) -> Corrida {
    // rutas canónicas: los corredores informan la ruta real (en macOS `/var` es `/private/var`), y con una
    // raíz sin canonizar se perdían el `archivo:línea` y la ruta relativa (medido en la compuerta)
    let raiz = canon(cx.root);
    let root = raiz.as_path();
    let que = &match que {
        Que::Archivo(p) => Que::Archivo(canon(p)),
        Que::Archivos(v) => Que::Archivos(v.iter().map(|p| canon(p)).collect()),
        otro => otro.clone(),
    };
    let _ = std::fs::create_dir_all(carpeta(root));
    let inf = carpeta(root).join(format!("informe-{}.{}", corredor.nombre(), corredor.informe().unwrap_or("txt")));
    let _ = std::fs::remove_file(&inf);
    let orden = corredor.orden(que, root, &inf);
    let inicio = Instant::now();
    let reloj = std::time::SystemTime::now();
    let mut k = crate::compuerta::en_caja(cx.spec.as_ref(), &orden, root);
    let (salida, exito, a_tiempo) = match tokio::time::timeout(tope, k.output()).await {
        Ok(Ok(o)) => (format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr)), o.status.success(), true),
        Ok(Err(e)) => (e.to_string(), false, true),
        Err(_) => (String::new(), false, false),
    };
    let registro = root.join(".gotv/registros").join(format!("pruebas-{}.log", corredor.nombre()));
    let _ = std::fs::create_dir_all(registro.parent().unwrap_or(root));
    let _ = std::fs::write(&registro, format!("$ {orden}\n{salida}"));
    let informe = corredor.informe().and_then(|_| std::fs::read_to_string(&inf).ok());
    let mut c = match corredor {
        Corredor::Gradle { .. } | Corredor::Maven => {
            let mut c = Corrida { corredor: corredor.nombre().into(), ..Default::default() };
            for x in informes_de_carpetas(root, reloj) {
                c.sumar(super::junit::leer_junit(&x, root));
            }
            c
        }
        _ => corredor.leer(&salida, informe.as_deref(), root),
    };
    if !a_tiempo {
        c.fallaron.push(Fallo { nombre: crate::i18n::t("(tiempo agotado)", "(timed out)").into(), mensaje: crate::tr!("las pruebas pasaron de {} s: se cortaron", "the tests went over {} s: they were stopped", tope.as_secs()), ..Default::default() });
    } else if !exito && c.fallaron.is_empty() {
        let ultimas: Vec<&str> = salida.lines().rev().filter(|l| !l.trim().is_empty()).take(15).collect::<Vec<_>>().into_iter().rev().collect();
        let mut f = Fallo { nombre: crate::i18n::t("(no corrió)", "(did not run)").into(), mensaje: ultimas.join("\n"), ..Default::default() };
        if let Some((p, l)) = super::corrida::lugar(&salida, root) {
            f.archivo = Some(p);
            f.linea = Some(l);
        }
        c.fallaron.push(f);
    }
    c.segundos = inicio.elapsed().as_secs_f32();
    c.registro = Some(registro);
    c
}

/// ¿Es un archivo de pruebas? (el criterio de D14, en cada lenguaje)
fn es_prueba(root: &Path, p: &Path) -> bool {
    super::es_archivo_de_pruebas(&p.strip_prefix(root).unwrap_or(p).display().to_string())
}

/// Los archivos de prueba del proyecto (sin dependencias ni compilados; 2000 como mucho).
fn archivos_de_prueba(root: &Path) -> Vec<PathBuf> {
    fn baja(root: &Path, d: &Path, n: u8, out: &mut Vec<PathBuf>) {
        if n > 6 || out.len() >= 2000 {
            return;
        }
        for e in std::fs::read_dir(d).into_iter().flatten().flatten() {
            let p = e.path();
            let nombre = e.file_name().to_string_lossy().into_owned();
            if p.is_dir() {
                if !nombre.starts_with('.') && !matches!(nombre.as_str(), "node_modules" | "target" | "venv" | "env" | "__pycache__" | "vendor" | "build" | "dist") {
                    baja(root, &p, n + 1, out);
                }
            } else if es_prueba(root, &p) {
                out.push(p);
            }
        }
    }
    let mut out = Vec::new();
    baja(root, root, 0, &mut out);
    out
}

/// ¿El texto nombra `palabra` como palabra entera?
fn nombra(texto: &str, palabra: &str) -> bool {
    let es = |c: char| c.is_alphanumeric() || c == '_';
    texto.match_indices(palabra).any(|(i, _)| {
        let antes = texto[..i].chars().next_back().is_none_or(|c| !es(c));
        let despues = texto[i + palabra.len()..].chars().next().is_none_or(|c| !es(c));
        antes && despues
    })
}

/// Qué correr para las pruebas AFECTADAS por `archivos` con este corredor: los archivos de prueba tocados,
/// los que se llaman como el módulo (`test_x.py`, `x.test.ts`, `XTest.php`), los que lo nombran, y los que el
/// servidor de lenguaje dice que lo usan. Vacío = nada que correr.
pub fn afectadas(corredor: &Corredor, root: &Path, archivos: &[PathBuf], usan: &[PathBuf]) -> Vec<Que> {
    let raiz = canon(root);
    let root = raiz.as_path();
    let archivos: Vec<PathBuf> = archivos.iter().map(|p| canon(p)).collect();
    let usan: Vec<PathBuf> = usan.iter().map(|p| canon(p)).collect();
    let propios: Vec<&PathBuf> = archivos.iter().filter(|p| corredor.atiende(p)).collect();
    if propios.is_empty() {
        return vec![];
    }
    let todas = archivos_de_prueba(root);
    let mut pruebas: Vec<PathBuf> = Vec::new();
    let mut modulos: Vec<String> = Vec::new();
    for p in &propios {
        if es_prueba(root, p) {
            pruebas.push((*p).clone());
            continue;
        }
        let stem = p.file_stem().unwrap_or_default().to_string_lossy().into_owned();
        if matches!(corredor, Corredor::Cargo { .. }) {
            // en Rust las pruebas unitarias viven en el mismo archivo: el filtro es el módulo
            let m = super::corrida::modulo_rust(p);
            if !m.is_empty() {
                modulos.push(m);
            }
        }
        if matches!(stem.as_str(), "mod" | "lib" | "main" | "index" | "__init__") || stem.len() < 3 {
            continue;
        }
        for t in &todas {
            let tn = t.file_name().unwrap_or_default().to_string_lossy().to_lowercase();
            let s = stem.to_lowercase();
            let por_nombre = tn.starts_with(&format!("test_{s}.")) || tn.starts_with(&format!("{s}_test.")) || tn.starts_with(&format!("{s}.test.")) || tn.starts_with(&format!("{s}.spec.")) || tn.starts_with(&format!("{s}test.")) || tn.starts_with(&format!("{s}tests."));
            if (por_nombre || std::fs::read_to_string(t).is_ok_and(|x| nombra(&x, &stem))) && corredor.atiende(t) && !pruebas.contains(t) {
                pruebas.push(t.clone());
            }
        }
    }
    for u in &usan {
        if es_prueba(root, u) && corredor.atiende(u) && !pruebas.contains(u) {
            pruebas.push(u.clone());
        }
    }
    modulos.sort();
    modulos.dedup();
    let mut out = Vec::new();
    match corredor {
        Corredor::Cargo { .. } => {
            // las de `tests/` por `--test`; los módulos de `src/` por filtro
            let integ: Vec<PathBuf> = pruebas.into_iter().filter(|p| p.strip_prefix(root).unwrap_or(p).components().any(|c| c.as_os_str() == "tests")).collect();
            if !integ.is_empty() {
                out.push(Que::Archivos(integ));
            }
            if !modulos.is_empty() {
                out.push(Que::Varias(modulos));
            }
        }
        // sin archivos por separado: por el nombre de la clase
        Corredor::Swift | Corredor::Gradle { .. } | Corredor::Maven | Corredor::Dotnet => {
            let mut nombres: Vec<String> = pruebas.iter().chain(propios.iter().copied()).filter_map(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned())).collect();
            nombres.sort();
            nombres.dedup();
            if !nombres.is_empty() {
                let comodin = matches!(corredor, Corredor::Gradle { .. } | Corredor::Maven);
                out.push(Que::Varias(nombres.into_iter().map(|n| if comodin { format!("*{n}*") } else { n }).collect()));
            }
        }
        _ if !pruebas.is_empty() => out.push(Que::Archivos(pruebas)),
        _ => {}
    }
    out
}

/// La orden de cobertura del corredor, si el proyecto ya tiene la herramienta.
fn cobertura(corredor: &Corredor, root: &Path) -> Option<String> {
    let en_path = |p: &str| crate::entorno::en_path(p);
    match corredor {
        Corredor::Cargo { .. } if en_path("cargo-llvm-cov") => Some("cargo llvm-cov --workspace --summary-only".into()),
        Corredor::Cargo { .. } if en_path("cargo-tarpaulin") => Some("cargo tarpaulin --workspace --skip-clean".into()),
        Corredor::Pytest { py } => {
            let tiene = std::process::Command::new(py.trim_matches('\'')).args(["-c", "import pytest_cov"]).current_dir(root).output().is_ok_and(|o| o.status.success());
            tiene.then(|| format!("{py} -m pytest -q --cov --cov-report=term"))
        }
        Corredor::Jest { bin } => Some(format!("{bin} --coverage --coverageReporters=text-summary")),
        Corredor::Vitest { bin } if root.join("node_modules/@vitest/coverage-v8").is_dir() || root.join("node_modules/@vitest/coverage-istanbul").is_dir() => Some(format!("{bin} run --coverage --coverage.reporter=text-summary")),
        Corredor::Go => Some("go test -cover ./...".into()),
        Corredor::Deno => Some("deno test --coverage=.gotv/pruebas/cobertura && deno coverage .gotv/pruebas/cobertura".into()),
        Corredor::Bun => Some("bun test --coverage".into()),
        _ => None,
    }
}

/// Los corredores para una petición: el pedido por nombre, los que atienden el archivo, o todos.
fn elegir(root: &Path, args: &Value) -> Result<Vec<Corredor>> {
    let todos = Corredor::detectar(root);
    if todos.is_empty() {
        bail!("{}", crate::i18n::t(
            "no encontré cómo se corren las pruebas de este proyecto (cargo, pytest/unittest, jest/vitest/node/bun/deno, go, phpunit, swift, gradle, maven, dotnet). Usa `bash` con la orden del proyecto.",
            "I did not find how this project runs its tests (cargo, pytest/unittest, jest/vitest/node/bun/deno, go, phpunit, swift, gradle, maven, dotnet). Use `bash` with the project's command."
        ));
    }
    if let Some(n) = args["corredor"].as_str().filter(|n| !n.is_empty()) {
        let v: Vec<Corredor> = todos.iter().filter(|c| c.nombre() == n).cloned().collect();
        if v.is_empty() {
            bail!("{}", crate::tr!("el proyecto no usa «{n}»; tiene: {}", "the project does not use «{n}»; it has: {}", todos.iter().map(Corredor::nombre).collect::<Vec<_>>().join(", ")));
        }
        return Ok(v);
    }
    if let Some(a) = args["archivo"].as_str().filter(|a| !a.is_empty()) {
        let p = crate::paths::resolve(root, a);
        let v: Vec<Corredor> = todos.iter().filter(|c| c.atiende(&p)).cloned().collect();
        if !v.is_empty() {
            return Ok(v);
        }
    }
    Ok(todos)
}

/// La herramienta: `descubrir`, `correr`, `fallidas`, `afectadas`, `cobertura`.
pub async fn ejecutar(args: &Value, cx: Contexto<'_>) -> Result<String> {
    let root = cx.root;
    let tope = Duration::from_secs(args["segundos"].as_u64().unwrap_or(600).clamp(10, 3600));
    let accion = args["action"].as_str().unwrap_or("correr");
    let corredores = elegir(root, args)?;
    let mut partes = Vec::new();
    match accion {
        "descubrir" | "discover" => {
            for c in &corredores {
                partes.push(crate::tr!("{} — todas: `{}` · una: `{}`", "{} — all: `{}` · one: `{}`", c.nombre(), c.orden(&Que::Todas, root, Path::new("<informe>")), c.como_una()));
            }
            let u = ultima(root);
            let pendientes: usize = u.values().map(Vec::len).sum();
            if pendientes > 0 {
                partes.push(crate::tr!("la última corrida dejó {pendientes} fallidas (action=fallidas las repite)", "the last run left {pendientes} failed (action=fallidas reruns them)"));
            }
        }
        "correr" | "run" => {
            let que = match (args["archivo"].as_str().filter(|a| !a.is_empty()), args["nombre"].as_str().filter(|n| !n.is_empty())) {
                (_, Some(n)) => Que::Nombre(n.to_string()),
                (Some(a), None) => Que::Archivo(crate::paths::resolve(root, a)),
                (None, None) => Que::Todas,
            };
            for c in &corredores {
                let r = correr(&cx, c, &que, tope).await;
                guardar_ultima(root, &r);
                partes.push(r.texto(root));
            }
        }
        "fallidas" | "failed" => {
            let u = ultima(root);
            for c in &corredores {
                match u.get(c.nombre()).filter(|v| !v.is_empty()) {
                    Some(v) => {
                        let r = correr(&cx, c, &Que::Varias(v.clone()), tope).await;
                        guardar_ultima(root, &r);
                        partes.push(r.texto(root));
                    }
                    None => partes.push(crate::tr!("{}: no hay fallidas guardadas de la última corrida", "{}: no failed tests saved from the last run", c.nombre())),
                }
            }
        }
        "afectadas" | "affected" => {
            let mut archivos: Vec<PathBuf> = cx.tocados.to_vec();
            for a in args["archivos"].as_array().into_iter().flatten().filter_map(Value::as_str) {
                archivos.push(crate::paths::resolve(root, a));
            }
            if archivos.is_empty() {
                bail!("{}", crate::i18n::t("este turno no tocó archivos: pasa `archivos` o usa action=correr", "this turn touched no files: pass `archivos` or use action=correr"));
            }
            for c in &corredores {
                for que in afectadas(c, root, &archivos, &cx.usan) {
                    let r = correr(&cx, c, &que, tope).await;
                    guardar_ultima(root, &r);
                    partes.push(r.texto(root));
                }
            }
            if partes.is_empty() {
                partes.push(crate::i18n::t("no encontré pruebas de lo tocado (ni por nombre, ni que lo usen): si las hay, usa action=correr", "I found no tests of what was touched (by name or using it): if there are, use action=correr").into());
            }
        }
        "cobertura" | "coverage" => {
            for c in &corredores {
                let Some(orden) = cobertura(c, root) else {
                    partes.push(crate::tr!("{}: el proyecto no tiene herramienta de cobertura instalada", "{}: the project has no coverage tool installed", c.nombre()));
                    continue;
                };
                let mut k = crate::compuerta::en_caja(cx.spec.as_ref(), &orden, root);
                let salida = match tokio::time::timeout(tope, k.output()).await {
                    Ok(Ok(o)) => format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr)),
                    Ok(Err(e)) => e.to_string(),
                    Err(_) => crate::i18n::t("(tiempo agotado)", "(timed out)").into(),
                };
                let limpia = super::limpia(&salida);
                let lineas: Vec<&str> = limpia.lines().filter(|l| l.contains('%')).take(40).collect();
                partes.push(format!("{} · `{orden}`\n{}", c.nombre(), if lineas.is_empty() { limpia.lines().rev().take(10).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join("\n") } else { lineas.join("\n") }));
            }
        }
        otra => bail!("{}", crate::tr!("acción desconocida: {otra} (descubrir · correr · fallidas · afectadas · cobertura)", "unknown action: {otra} (descubrir · correr · fallidas · afectadas · cobertura)")),
    }
    Ok(partes.join("\n\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cx(root: &Path) -> Contexto<'_> {
        Contexto { root, spec: None, tocados: &[], usan: vec![] }
    }

    /// De verdad, con `python3 -m unittest`: un proyecto con una prueba rota; `correr` la encuentra con su
    /// archivo y línea, `fallidas` repite sólo esa, y arreglada queda en verde. `afectadas` de `calc.py` es
    /// su prueba (por nombre y porque la importa).
    #[tokio::test]
    async fn unittest_de_verdad() {
        if !crate::entorno::en_path("python3") {
            return;
        }
        let d = crate::CarpetaPrueba::nueva("gotv-pruebas-py");
        let r = d.canonicalize().unwrap();
        std::fs::create_dir_all(r.join("tests")).unwrap();
        std::fs::write(r.join("tests/__init__.py"), "").unwrap();
        std::fs::write(r.join("calc.py"), "def doble(x):\n    return x * 3\n").unwrap();
        std::fs::write(r.join("tests/test_calc.py"), "import unittest\nfrom calc import doble\n\nclass T(unittest.TestCase):\n    def test_bien(self):\n        self.assertEqual(doble(0), 0)\n\n    def test_doble(self):\n        self.assertEqual(doble(2), 4)\n").unwrap();
        std::fs::write(r.join("tests/test_otro.py"), "import unittest\n\nclass O(unittest.TestCase):\n    def test_nada(self):\n        pass\n").unwrap();
        let t = ejecutar(&serde_json::json!({"action": "correr"}), cx(&r)).await.unwrap();
        assert!(t.contains("Pasaron 2 · Fallaron 1") && t.contains("✗ test_doble — tests/test_calc.py:9") && t.contains("6 != 4"), "{t}");
        assert!(r.join(".gotv/registros/pruebas-unittest.log").is_file());
        assert!(ejecutar(&serde_json::json!({"action": "descubrir"}), cx(&r)).await.unwrap().contains("unittest — "));
        let t = ejecutar(&serde_json::json!({"action": "fallidas"}), cx(&r)).await.unwrap();
        assert!(t.contains("Pasaron 0 · Fallaron 1"), "sólo la que falló: {t}");
        std::fs::write(r.join("calc.py"), "def doble(x):\n    return x * 2\n").unwrap();
        let tocados = [r.join("calc.py")];
        let t = ejecutar(&serde_json::json!({"action": "afectadas"}), Contexto { root: &r, spec: None, tocados: &tocados, usan: vec![] }).await.unwrap();
        assert!(t.contains("Pasaron 2 · Fallaron 0"), "sólo test_calc (2), no test_otro: {t}");
        assert!(ejecutar(&serde_json::json!({"action": "correr", "corredor": "jest"}), cx(&r)).await.is_err());
    }

    /// De verdad, con `cargo test`: el proyecto juguete de las salidas guardadas, por archivo y por nombre.
    #[tokio::test]
    async fn cargo_de_verdad() {
        if !crate::entorno::en_path("cargo") {
            return;
        }
        let d = crate::CarpetaPrueba::nueva("gotv-pruebas-rs");
        let r = d.canonicalize().unwrap();
        std::fs::create_dir_all(r.join("src")).unwrap();
        std::fs::create_dir_all(r.join("tests")).unwrap();
        std::fs::write(r.join("Cargo.toml"), "[package]\nname = \"juguete\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[workspace]\n").unwrap();
        std::fs::write(r.join("src/lib.rs"), "pub fn suma(a: i32, b: i32) -> i32 { a + b }\n\n#[cfg(test)]\nmod tests {\n    #[test]\n    fn suma_mal() {\n        assert_eq!(super::suma(2, 2), 5);\n    }\n}\n").unwrap();
        std::fs::write(r.join("tests/integra.rs"), "#[test]\nfn integra() { assert_eq!(juguete::suma(1, 1), 2); }\n").unwrap();
        let t = ejecutar(&serde_json::json!({"action": "correr"}), cx(&r)).await.unwrap();
        assert!(t.contains("Pasaron 1 · Fallaron 1") && t.contains("✗ tests::suma_mal — src/lib.rs:7"), "{t}");
        let t = ejecutar(&serde_json::json!({"action": "correr", "archivo": "tests/integra.rs"}), cx(&r)).await.unwrap();
        assert!(t.contains("Pasaron 1 · Fallaron 0"), "{t}");
        // no compila: el fallo dice dónde
        std::fs::write(r.join("src/lib.rs"), "pub fn suma(a: i32, b: i32) -> i32 { a + \"b\" }\n").unwrap();
        let t = ejecutar(&serde_json::json!({"action": "correr"}), cx(&r)).await.unwrap();
        assert!(t.contains("src/lib.rs:1"), "{t}");
    }

    #[test]
    fn afectadas_por_nombre_y_por_uso() {
        let d = crate::CarpetaPrueba::nueva("gotv-pruebas-afectadas");
        let r = d.canonicalize().unwrap();
        for (f, t) in [("src/precio.ts", ""), ("src/precio.test.ts", ""), ("src/otro.test.ts", "import { precio } from './precio'"), ("src/nada.test.ts", ""), ("src/usa_lsp.test.ts", "")] {
            std::fs::create_dir_all(r.join(f).parent().unwrap()).unwrap();
            std::fs::write(r.join(f), t).unwrap();
        }
        let q = afectadas(&Corredor::Bun, &r, &[r.join("src/precio.ts"), r.join("README.md")], &[r.join("src/usa_lsp.test.ts"), r.join("src/precio.ts")]);
        let Que::Archivos(v) = &q[0] else { panic!("{q:?}") };
        let mut nombres: Vec<String> = v.iter().map(|p| p.file_name().unwrap().to_string_lossy().into_owned()).collect();
        nombres.sort();
        assert_eq!(nombres, ["otro.test.ts", "precio.test.ts", "usa_lsp.test.ts"]);
        assert!(afectadas(&Corredor::Go, &r, &[r.join("src/precio.ts")], &[]).is_empty(), "otro lenguaje");
        // Rust: módulos de src por filtro, tests/ por --test
        let q = afectadas(&Corredor::Cargo { release: false }, &r, &[r.join("src/datos.rs"), r.join("tests/integra.rs")], &[]);
        assert_eq!(q, vec![Que::Archivos(vec![r.join("tests/integra.rs")]), Que::Varias(vec!["datos".into()])]);
    }
}
