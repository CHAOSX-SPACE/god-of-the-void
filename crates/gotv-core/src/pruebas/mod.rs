//! Motor de programación · supervisión de pruebas, en cada lenguaje (prueba Cable, 2026-11-13).
//!
//! Lo que GOTV hace cuando las pruebas están en rojo (D12 reorientar, D13 experimento, D14 freno a
//! doblar pruebas, D15 reescritura) depende de LEER la salida de las pruebas: si es de pruebas, si
//! está en rojo, qué pruebas fallan y en qué archivo y línea. Los errores de lógica son los mismos en
//! todos los lenguajes; aquí se reconoce cada ejecutor: unittest y pytest (Python), cargo (Rust),
//! go test, node --test, Jest, Vitest, Bun y Deno (JS/TS), JUnit (Java/Kotlin), PHPUnit, minitest y
//! RSpec (Ruby), XCTest y swift-testing (Swift).
//!
//! F15.1: `corrida` (la corrida ESTRUCTURADA de cada corredor), `junit` (JUnit y TRX) y `herramienta`
//! (la herramienta `pruebas`).
pub mod corrida;
pub mod herramienta;
pub mod junit;

/// Deno y JUnit pintan con colores aunque no haya terminal (cosecha 2026-11-13), y `bash` sólo los
/// limpia con `tty`: se leen sin ellos.
fn limpia(t: &str) -> std::borrow::Cow<'_, str> {
    if t.contains('\x1b') {
        std::borrow::Cow::Owned(crate::exec::strip_ansi(t))
    } else {
        std::borrow::Cow::Borrowed(t)
    }
}

/// F15.1: si el texto trae resúmenes de la herramienta `pruebas` («Pasaron 2 · Fallaron 1 · Omitidas 0
/// (3 s) · cargo», o en inglés), cuántos fallos suman; `None` si no trae ninguno.
fn resumen_corrida(t: &str) -> Option<u32> {
    let mut hay = false;
    let mut fallos = 0u32;
    for l in t.lines() {
        let l = l.trim();
        let tras = l.strip_prefix("Pasaron ").and_then(|r| r.split_once(" · Fallaron ")).or_else(|| l.strip_prefix("Passed ").and_then(|r| r.split_once(" · Failed ")));
        if let Some((pasaron, resto)) = tras {
            if let (Ok(_), Some(n)) = (pasaron.trim().parse::<u32>(), resto.split_whitespace().next().and_then(|n| n.parse::<u32>().ok())) {
                hay = true;
                fallos += n;
            }
        }
    }
    hay.then_some(fallos)
}

/// ¿Parece la salida de un ejecutor de pruebas?
pub fn parece_salida_de_pruebas(t: &str) -> bool {
    let t = &*limpia(t);
    (t.contains("Ran ") && t.contains(" test"))
        || t.contains(" passed")
        || t.contains(" failed")
        || t.contains("test result:")
        || t.contains("FAILED (")
        || (t.contains("Tests:") && (t.contains("total") || t.contains("Assertions:")))
        || t.contains("Test Files")
        || t.contains("Tests run:")
        || t.contains("tests successful")
        || (t.contains(" examples, ") && t.contains(" failure"))
        || (t.contains(" runs, ") && t.contains(" assertions, "))
        || (t.contains("Executed ") && t.contains(" test"))
        // F15.1: el resumen de la herramienta `pruebas`
        || resumen_corrida(t).is_some()
        || t.lines().map(str::trim).any(|l| {
            l.starts_with("--- FAIL")
                || l.starts_with("ok  \t")
                || l.starts_with("ℹ pass ")
                || l.starts_with("# pass ")
                || l.starts_with("(fail) ")
                || l.starts_with("(pass) ")
                || (l.ends_with(" pass") && l.split_whitespace().count() == 2)
        })
}

/// ¿La salida dice que algo falló? Un `(exit N)` distinto de 0 en la primera línea (lo añade GOTV)
/// basta; un `(exit 0)` NO la da por verde (`python3 -m unittest | tail` sale con 0 aunque fallen):
/// mandan las marcas de cada ejecutor (un «0 failed» o «0 failures» no cuenta).
pub fn prueba_en_rojo(t: &str) -> bool {
    let t = &*limpia(t);
    let primera = t.lines().next().unwrap_or("");
    if primera.starts_with("(exit ") && !primera.starts_with("(exit 0)") {
        return true;
    }
    // F15.1: el resumen de `pruebas` manda (en rojo si alguna de sus corridas tuvo fallos)
    if let Some(fallos) = resumen_corrida(t) {
        return fallos > 0;
    }
    if t.contains("FAILED") || t.contains("FAILURES!") || t.contains("ERRORS!") || t.contains("--- FAIL") {
        return true;
    }
    t.lines().any(|l| cuenta_fallos(l) > 0 || l.trim_start().starts_with("(fail) ") || l.trim_start().starts_with("not ok "))
}

/// El mayor número que precede a una palabra de fallo en la línea («2 failed», «1 failures»,
/// «3 failing», «Failures: 1», «with 2 failures», «ℹ fail 2»).
fn cuenta_fallos(l: &str) -> u64 {
    let palabras: Vec<&str> = l
        .split(|c: char| c.is_whitespace() || c == ',' || c == '|' || c == '(' || c == ')' || c == '.' || c == ';')
        .filter(|w| !w.is_empty())
        .collect();
    let es_fallo = |w: &str| {
        let w = w.to_lowercase();
        matches!(w.as_str(), "failed" | "failures" | "failure" | "failing" | "fail" | "errors" | "error" | "failures:" | "errors:")
    };
    let num = |w: &str| w.parse::<u64>().ok();
    let mut max = 0;
    for (i, w) in palabras.iter().enumerate() {
        if !es_fallo(w) {
            continue;
        }
        if w.ends_with(':') {
            // «Failures: N» (JUnit, PHPUnit): sólo el número que SIGUE (el de delante es de otro campo)
            if let Some(n) = palabras.get(i + 1).and_then(|x| num(x)) {
                max = max.max(n);
            }
            continue;
        }
        // «N failed» / «with N failures» / «2 fail» (Bun)
        if let Some(n) = i.checked_sub(1).and_then(|j| num(palabras[j])) {
            max = max.max(n);
        }
        // «2 tests failed» (JUnit), «3 examples failed»
        if i >= 2 && matches!(palabras[i - 1], "tests" | "test" | "examples" | "specs") {
            if let Some(n) = num(palabras[i - 2]) {
                max = max.max(n);
            }
        }
        // «ℹ fail N» (node --test)
        if w.eq_ignore_ascii_case("fail") {
            if let Some(n) = palabras.get(i + 1).and_then(|x| num(x)) {
                max = max.max(n);
            }
        }
    }
    max
}

/// Los nombres de las pruebas que fallan, sin repetir y en orden de aparición.
pub fn pruebas_que_fallan(salida: &str) -> Vec<String> {
    let mut v: Vec<String> = Vec::new();
    let mut pon = |n: &str| {
        let n = n.trim();
        // «FAILED (failures=1)» de unittest es el resumen, no una prueba
        if !n.is_empty() && !n.starts_with('(') && !v.iter().any(|x| x == n) {
            v.push(n.to_string());
        }
    };
    let salida = limpia(salida);
    for l in salida.lines().map(str::trim) {
        // F15.1: «✗ nombre — archivo:línea» de la herramienta `pruebas`
        if let Some(r) = l.strip_prefix("✗ ") {
            pon(r.split(" — ").next().unwrap_or(r));
            continue;
        }
        if let Some(r) = l.strip_prefix("✘ Test ").and_then(|r| r.split_once(" recorded an issue").or_else(|| r.split_once(" failed"))) {
            // swift-testing: «✘ Test parcial() recorded an issue at …» (antes que el «✘ » genérico)
            pon(r.0);
        } else if let Some(r) = l.strip_prefix("FAIL ").filter(|r| r.contains(" > ")) {
            // Vitest: « FAIL  pagos.test.ts > parcial» (el «FAIL ./x.test.js» de Jest es el archivo)
            pon(r);
        } else if let Some(r) = l.strip_prefix("● ").filter(|r| !r.starts_with("Test suite failed")) {
            // Jest sin --verbose: «● pagar › parcial»
            pon(r);
        } else if let Some((n, _)) = l.split_once(" ✘").filter(|(n, _)| n.contains('─')) {
            // JUnit (árbol de la consola): «│     ├─ parcial() ✘ expected: <600> but was: <1400>»
            pon(n.trim_start_matches(|c: char| c == '│' || c == '├' || c == '└' || c == '─' || c.is_whitespace()));
        } else if let Some((n, _)) = l.split_once(" => ./").filter(|(n, _)| !n.contains(' ') || n.split_whitespace().count() <= 6) {
            // Deno: «parcial => ./pagos_test.ts:4:6»
            pon(n);
        } else if l.ends_with(':') && !l.contains(' ') && l.contains('#') && l.chars().next().is_some_and(|c| c.is_ascii_uppercase()) {
            // minitest, error: «TestPagos#test_excede:»
            pon(l.trim_end_matches(':'));
        } else if let Some(r) = l.strip_prefix("FAIL: ").or_else(|| l.strip_prefix("ERROR: ")) {
            // unittest: «FAIL: test_x (mod.Clase.test_x)»
            pon(r.split_whitespace().next().unwrap_or(""));
        } else if let Some(r) = l.strip_prefix("FAILED ").filter(|r| !r.starts_with('|')) {
            // pytest: «FAILED tests/t.py::test_x - AssertionError…» (no el resumen de Deno «FAILED | 1 passed…»)
            pon(r.split(" - ").next().unwrap_or(""));
        } else if let Some(r) = l.strip_prefix("--- FAIL: ") {
            // go: «--- FAIL: TestX/caso (0.00s)»
            pon(r.split_whitespace().next().unwrap_or(""));
        } else if let (Some(r), true) = (l.strip_prefix("test "), l.ends_with("... FAILED")) {
            // cargo: «test tests::x ... FAILED»
            pon(r.split(" ... ").next().unwrap_or(""));
        } else if let Some(r) = l.strip_prefix("✕ ").or_else(|| l.strip_prefix("× ")).or_else(|| l.strip_prefix("✖ ")).or_else(|| l.strip_prefix("✘ ")) {
            // Jest --verbose «✕ x (3 ms)», Vitest «× x 3ms», node --test «✖ x (0.5ms)»; el título
            // «✖ failing tests:» de node no es una prueba
            if !r.ends_with(':') {
                pon(sin_duracion(r));
            }
        } else if let Some(r) = l.strip_prefix("(fail) ") {
            // Bun: «(fail) suite > x [0.12ms]»
            pon(r.split(" [").next().unwrap_or(r));
        } else if let Some(r) = l.strip_prefix("not ok ") {
            // TAP (node --test --test-reporter=tap): «not ok 2 - x»
            pon(r.split_once(" - ").map_or(r, |(_, n)| n));
        } else if let Some(r) = l.strip_prefix("rspec ") {
            // RSpec: «rspec ./spec/x_spec.rb:6 # pagar parcial»
            pon(r.split_once(" # ").map_or(r, |(_, n)| n));
        } else if let Some((_, r)) = l.split_once(") ").filter(|(n, _)| n.parse::<u32>().is_ok()) {
            // PHPUnit «1) PagosTest::testParcial»; minitest «1) Failure:» trae el nombre en la siguiente
            if r.contains("::") {
                pon(r);
            }
        } else if l.contains('#') && l.ends_with("]:") && l.contains(" [") {
            // minitest: «TestPagos#test_parcial [test/test_pagos.rb:9]:»
            pon(l.split(" [").next().unwrap_or(l));
        } else if let Some(r) = l.strip_prefix("Test Case '").and_then(|r| r.split_once("' failed")).map(|(n, _)| n) {
            // XCTest: «Test Case '-[Mod.PagosTests testParcial]' failed (0.001 seconds).»
            pon(r);
        } else if let Some(r) = l.strip_suffix(" ... FAILED").or_else(|| l.split_once(" ... FAILED (").map(|(n, _)| n)) {
            // Deno: «parcial ... FAILED (1ms)»
            pon(r);
        }
    }
    v
}

/// «x (3 ms)» → «x»; «x 3ms» (Vitest) → «x»
fn sin_duracion(r: &str) -> &str {
    if let Some(i) = r.rfind(" (") {
        if r.ends_with(')') && r[i..].chars().any(|c| c.is_ascii_digit()) && !r.ends_with("()") {
            return &r[..i];
        }
    }
    match r.rsplit_once(' ') {
        Some((n, d)) if (d.ends_with("ms") || d.ends_with('s')) && d.chars().next().is_some_and(|c| c.is_ascii_digit()) => n,
        _ => r,
    }
}

/// Extensiones de código que se buscan en las trazas.
const CODIGO: [&str; 22] = [
    ".py", ".rs", ".js", ".mjs", ".cjs", ".jsx", ".ts", ".mts", ".tsx", ".go", ".java", ".kt", ".php", ".rb", ".swift", ".c", ".h", ".cc", ".cpp", ".hpp", ".cs", ".kts",
];

/// Los lugares (archivo, línea) que señala una salida de pruebas o una traza, en orden y sin
/// repetir: `File "x.py", line 4` (Python), `ruta:12:5` (Rust, JS, TS, Go, Ruby, Swift, PHP…),
/// `(Clase.java:6)` (Java/Kotlin: sólo el nombre del archivo), `file:///ruta` (Deno),
/// `en x.php on line 4` (PHP) y `in X.cs:line 42` (C#). Quién decide si el archivo es del proyecto
/// es quien llama.
pub fn lugares_del_fallo(salida: &str) -> Vec<(String, usize)> {
    let mut v: Vec<(String, usize)> = Vec::new();
    let mut pon = |ruta: &str, n: usize| {
        let ruta = ruta.trim_start_matches("file://").trim_matches(|c| c == '\'' || c == '"' || c == '`');
        // lo que queda de una URL `file:///…` cortada por un patrón: una sola barra
        let ruta = if ruta.starts_with("//") { &ruta[ruta.len() - ruta.trim_start_matches('/').len() - 1..] } else { ruta };
        if n > 0 && !ruta.is_empty() && !v.iter().any(|(r, m)| r == ruta && *m == n) {
            v.push((ruta.to_string(), n));
        }
    };
    let numero = |s: &str| s.split(|c: char| !c.is_ascii_digit()).next().and_then(|n| n.parse::<usize>().ok());
    let salida = limpia(salida);
    for l in salida.lines() {
        let t = l.trim();
        // Python: File "/ruta/x.py", line 42, in prueba
        if let Some((ruta, resto)) = t.strip_prefix("File \"").and_then(|r| r.split_once("\", line ")) {
            if let Some(n) = numero(resto) {
                pon(ruta, n);
            }
            continue;
        }
        // PHP: «… in /ruta/x.php on line 4»
        if let Some((antes, resto)) = t.split_once(" on line ") {
            if let (Some(ruta), Some(n)) = (antes.rsplit(' ').next(), numero(resto)) {
                if CODIGO.iter().any(|e| ruta.ends_with(e)) {
                    pon(ruta, n);
                    continue;
                }
            }
        }
        for pieza in t.split(|c: char| c.is_whitespace() || c == '(' || c == ')' || c == '[' || c == ']' || c == ',') {
            // C#: «X.cs:line 42» → la línea va en la pieza siguiente; se resuelve abajo
            let pieza = pieza.trim_start_matches("file://");
            // el prefijo de ruta acaba en la extensión: «/a/b.rs:3:9:» → («/a/b.rs», «3»)
            for e in CODIGO {
                if let Some(i) = pieza.find(&format!("{e}:")) {
                    let ruta = &pieza[..i + e.len()];
                    let resto = &pieza[i + e.len() + 1..];
                    if let Some(n) = numero(resto) {
                        pon(ruta, n);
                    }
                    break;
                }
            }
        }
        // C#: «in /ruta/X.cs:line 42»
        if let Some((antes, resto)) = t.split_once(":line ") {
            if let (Some(ruta), Some(n)) = (antes.rsplit(' ').next(), numero(resto)) {
                pon(ruta, n);
            }
        }
        // F11: los patrones `archivo:línea` de los paquetes de lenguaje (los que el corte de arriba no ve:
        // `tsc` con «a.ts(3,5)», Java «(Pagos.java:6)», `bash` con «x.sh: line 4:»…)
        for r in crate::lenguajes::patrones_de_error() {
            for c in r.captures_iter(t) {
                if let (Some(a), Some(n)) = (c.name("archivo"), c.name("linea").and_then(|n| n.as_str().parse::<usize>().ok())) { // juez-idioma: datos (grupos de los patrones)
                    pon(a.as_str(), n);
                }
            }
        }
    }
    v
}

/// ¿Este comando corre pruebas? (para que GOTV lo repita al reorientar).
pub fn es_comando_de_pruebas(cmd: &str) -> bool {
    const SENALES: [&str; 12] = ["test", "pytest", "rspec", "phpunit", "jest", "vitest", "mvn ", "gradle", "ctest", "xcodebuild", "mocha", "go vet"];
    let bajo = cmd.to_lowercase();
    SENALES.iter().any(|s| bajo.contains(s))
}

/// D14: ¿es un archivo de pruebas? Por carpeta (`tests/`, `test/`, `spec/`, `__tests__/`,
/// `src/test/`) o por nombre, según la convención de cada lenguaje.
pub fn es_archivo_de_pruebas(ruta: &str) -> bool {
    let r = ruta.replace('\\', "/");
    let bajo = r.to_lowercase();
    let nombre = r.rsplit('/').next().unwrap_or(&r);
    let nombre_bajo = nombre.to_lowercase();
    let carpeta = ["tests/", "test/", "spec/", "__tests__/"]
        .iter()
        .any(|c| bajo.starts_with(c) || bajo.contains(&format!("/{c}")));
    let (base, ext) = nombre.rsplit_once('.').unwrap_or((nombre, ""));
    carpeta
        || nombre_bajo.starts_with("test_")
        || base.ends_with("_test")
        || base.ends_with("_spec")
        || [".test", ".spec"].iter().any(|s| base.to_lowercase().ends_with(s))
        // Java, Kotlin, PHP, Swift, C#: PagosTest.java, PagosTests.swift
        || (matches!(ext, "java" | "kt" | "php" | "swift" | "cs") && (base.ends_with("Test") || base.ends_with("Tests") || base.ends_with("IT")))
}

/// D14: las líneas con aserciones (o filas de una tabla de casos) que están en `viejo` y ya no en
/// `nuevo`. Una aserción en cualquier lenguaje: `assert…` (Python, Rust, JUnit, PHPUnit
/// `$this->assert…`, XCTest `XCTAssert…`, minitest), `expect(` (Jest, Vitest, RSpec, swift-testing
/// `#expect(`), `t.Error`/`t.Fatal` (Go), `require.` (testify), `.should`, `must_`. Una fila de tabla:
/// `{"parcial", 1000, 400, 600},` (Go) o `(1000, 400, 600),` (pytest, Rust), con algún literal.
pub fn aserciones_quitadas(viejo: &str, nuevo: &str) -> Vec<String> {
    let es_asercion = |l: &str| {
        let t = l.trim();
        let b = t.to_lowercase();
        let fila = (t.starts_with('{') || t.starts_with('(') || t.starts_with('['))
            && (t.ends_with("},") || t.ends_with("),") || t.ends_with("],"))
            && t.contains(',')
            && t.chars().any(|c| c.is_ascii_digit() || c == '"' || c == '\'');
        b.contains("assert")
            || b.contains("expect(")
            || b.contains("t.error")
            || b.contains("t.fatal")
            || b.contains("require.")
            || b.contains(".should")
            || b.contains("must_")
            || b.starts_with("with pytest.raises")
            || fila
    };
    let nuevas: std::collections::HashSet<&str> = nuevo.lines().map(str::trim).collect();
    viejo
        .lines()
        .filter(|l| es_asercion(l))
        .map(str::trim)
        .filter(|l| !nuevas.contains(l))
        .map(String::from)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cuenta_fallos_sin_falsos() {
        assert_eq!(cuenta_fallos("3 examples, 2 failures"), 2);
        assert_eq!(cuenta_fallos("3 examples, 0 failures"), 0);
        assert_eq!(cuenta_fallos("Tests: 3, Assertions: 3, Failures: 1, Errors: 1."), 1);
        assert_eq!(cuenta_fallos("ℹ fail 2"), 2);
        assert_eq!(cuenta_fallos("Executed 2 tests, with 1 failure (0 unexpected) in 0.002 seconds"), 1);
        assert_eq!(cuenta_fallos("test result: ok. 3 passed; 0 failed; 0 ignored"), 0);
    }

    #[test]
    fn rojo_aunque_la_tuberia_salga_con_0() {
        assert!(prueba_en_rojo("(exit 0)\nRan 3 tests in 0.1s\n\nFAILED (failures=1)"));
        assert!(!prueba_en_rojo("(exit 0)\nRan 3 tests in 0.1s\n\nOK"));
        assert!(prueba_en_rojo("(exit 1)\nalgo"));
        assert!(!prueba_en_rojo("(exit 0)\n3 examples, 0 failures"));
    }

    #[test]
    fn archivos_de_pruebas_por_lenguaje() {
        for r in [
            "tests/test_biblioteca.py", "pkg/tests/x.py", "test_a.py", "a_test.go", "src/x.test.ts", "web/__tests__/a.js", "b.spec.js",
            "src/test/java/pagos/PagosTest.java", "PagosTests.swift", "tests/PagosTest.php", "spec/pagos_spec.rb", "test/test_pagos.rb", "pagos_test.ts",
            "app/src/test/kotlin/PagosTest.kt", "Pagos.Tests/PagosTests.cs",
        ] {
            assert!(es_archivo_de_pruebas(r), "{r}");
        }
        for r in ["src/pagos.py", "lib/pagos.rb", "src/main/java/pagos/Pagos.java", "Sources/Pagos/Pagos.swift", "contest.py", "latest.js"] {
            assert!(!es_archivo_de_pruebas(r), "{r}");
        }
    }

    #[test]
    fn aserciones_de_cada_lenguaje() {
        let casos = [
            ("self.assertEqual(pagar(1000, 400), 600)", "self.assertEqual(pagar(1000, 400), 1400)"),
            ("assert_eq!(pagar(1000, 400), 600);", "assert_eq!(pagar(1000, 400), 1400);"),
            ("expect(pagar(1000, 400)).toBe(600);", "expect(pagar(1000, 400)).toBe(1400);"),
            ("assertEquals(600, Pagos.pagar(1000, 400));", "assertEquals(1400, Pagos.pagar(1000, 400));"),
            ("$this->assertSame(600, pagar(1000, 400));", "$this->assertSame(1400, pagar(1000, 400));"),
            ("assert_equal 600, pagar(1000, 400)", "assert_equal 1400, pagar(1000, 400)"),
            ("XCTAssertEqual(pagar(1000, 400), 600)", "XCTAssertEqual(pagar(1000, 400), 1400)"),
            ("t.Errorf(\"got %d, want 600\", got)", ""),
            ("{\"parcial\", 1000, 400, 600},", "{\"parcial\", 1000, 400, 1400},"),
            ("(1000, 400, 600),", "(1000, 400, 1400),"),
        ];
        for (viejo, nuevo) in casos {
            assert_eq!(aserciones_quitadas(viejo, nuevo).len(), 1, "{viejo}");
        }
        // un cambio que no toca aserciones no frena
        assert!(aserciones_quitadas("import os\nx = 1", "import os\nx = 2").is_empty());
    }

    /// F15.1: D12, D14 y N5 leen también el resumen de la herramienta `pruebas`.
    #[test]
    fn el_resumen_de_la_herramienta_pruebas() {
        let rojo = "Pasaron 2 · Fallaron 1 · Omitidas 0 (3 s) · cargo\n✗ tests::suma_mal — src/lib.rs:9\n    dos y dos";
        assert!(parece_salida_de_pruebas(rojo) && prueba_en_rojo(rojo));
        assert_eq!(pruebas_que_fallan(rojo), vec!["tests::suma_mal"]);
        assert_eq!(lugares_del_fallo(rojo), vec![("src/lib.rs".to_string(), 9)]);
        // dos corredores en verde (uno en inglés): verde, aunque un mensaje diga FAILED
        let verde = "Passed 4 · Failed 0 · Skipped 1 (2 s) · go\n\nPasaron 1 · Fallaron 0 · Omitidas 0 (1 s) · vitest";
        assert!(parece_salida_de_pruebas(verde) && !prueba_en_rojo(verde));
        assert!(prueba_en_rojo(&format!("{verde}\n\nPasaron 0 · Fallaron 1 · Omitidas 0 (1 s) · go\n✗ (no corrió)")));
        assert!(pruebas_que_fallan("✗ (no corrió) — go.mod").is_empty(), "lo que no es una prueba no se repite por nombre");
    }

    #[test]
    fn lugares_de_trazas() {
        let s = "  File \"/p/pkg/pagos.py\", line 4, in pagar\n\
                 thread 'tests::x' panicked at src/lib.rs:3:9:\n\
                 at pagar (file:///p/pagos.ts:2:26)\n\
                 \tat pagos.Pagos.pagar(Pagos.java:6)\n\
                 /p/lib/pagos.rb:2:in 'pagar'\n\
                 PHP Parse error: syntax error in /p/a.php on line 2\n\
                 at X.F() in /p/X.cs:line 42\n\
                     pagos_test.go:21: got 1400, want 600";
        assert_eq!(
            lugares_del_fallo(s),
            vec![
                ("/p/pkg/pagos.py".to_string(), 4),
                ("src/lib.rs".to_string(), 3),
                ("/p/pagos.ts".to_string(), 2),
                ("Pagos.java".to_string(), 6),
                ("/p/lib/pagos.rb".to_string(), 2),
                ("/p/a.php".to_string(), 2),
                ("/p/X.cs".to_string(), 42),
                ("pagos_test.go".to_string(), 21),
            ]
        );
    }

    /// Salidas REALES (cosecha 2026-11-13: un proyecto por lenguaje con una prueba verde, una aserción
    /// falsa y una excepción en el código; rutas reducidas a `/p`, colores conservados, SIN la línea
    /// `(exit N)`): el rojo tiene que leerse en la propia salida.
    #[test]
    fn salidas_reales_de_cada_ejecutor() {
        type Caso = (&'static str, &'static str, &'static [&'static str], &'static [(&'static str, usize)]);
        let casos: &[Caso] = &[
            ("unittest", include_str!("../../datos/salidas-pruebas/py-unittest.txt"), &["test_excede", "test_parcial"], &[("/p/py/tests/test_pagos.py", 9), ("/p/py/pkg/pagos.py", 3)]),
            ("pytest", include_str!("../../datos/salidas-pruebas/py-pytest.txt"), &["tests/test_pagos.py::TestPagar::test_excede", "tests/test_pagos.py::TestPagar::test_parcial"], &[("tests/test_pagos.py", 9), ("pkg/pagos.py", 3)]),
            ("cargo", include_str!("../../datos/salidas-pruebas/rs-cargo.txt"), &["tests::interna"], &[("src/lib.rs", 13)]),
            ("go test", include_str!("../../datos/salidas-pruebas/go-test.txt"), &["TestTabla", "TestTabla/parcial", "TestTabla/total", "TestExcede"], &[("pagos_test.go", 23), ("/p/go/pagos.go", 5), ("/p/go/pagos_test.go", 30)]),
            ("node --test", include_str!("../../datos/salidas-pruebas/js-node-test.txt"), &["parcial", "excede"], &[("/p/js/pagos.test.js", 5), ("/p/js/pagos.js", 2)]),
            ("bun", include_str!("../../datos/salidas-pruebas/ts-bun.txt"), &["parcial", "excede"], &[("/p/bun/pagos.test.ts", 4), ("/p/bun/pagos.ts", 2)]),
            ("jest", include_str!("../../datos/salidas-pruebas/js-jest.txt"), &["pagar › parcial", "pagar › excede"], &[("pagos.test.js", 4), ("pagos.js", 2)]),
            ("vitest", include_str!("../../datos/salidas-pruebas/ts-vitest.txt"), &["parcial", "excede", "pagos.test.ts > parcial", "pagos.test.ts > excede"], &[("pagos.test.ts", 4), ("pagos.ts", 2)]),
            ("junit", include_str!("../../datos/salidas-pruebas/java-junit.txt"), &["excede()", "parcial()"], &[("Pagos.java", 6), ("PagosTest.java", 14), ("PagosTest.java", 11)]),
            ("phpunit", include_str!("../../datos/salidas-pruebas/php-phpunit.txt"), &["PagosTest::testExcede", "PagosTest::testParcial"], &[("/p/php/src/Pagos.php", 4), ("/p/php/tests/PagosTest.php", 6)]),
            ("minitest", include_str!("../../datos/salidas-pruebas/rb-minitest.txt"), &["TestPagos#test_excede", "TestPagos#test_parcial"], &[("/p/rb/lib/pagos.rb", 2), ("test/test_pagos.rb", 11), ("test/test_pagos.rb", 8)]),
            ("rspec", include_str!("../../datos/salidas-pruebas/rb-rspec.txt"), &["pagar parcial", "pagar excede"], &[("./lib/pagos.rb", 2), ("./spec/pagos_spec.rb", 7)]),
            ("xctest", include_str!("../../datos/salidas-pruebas/sw-xctest.txt"), &["-[PagosTests.PagosTests testParcial]"], &[("/p/sw/Tests/PagosTests/PagosTests.swift", 5)]),
            ("deno", include_str!("../../datos/salidas-pruebas/ts-deno.txt"), &["parcial", "excede"], &[("/p/deno/pagos.ts", 2), ("/p/deno/pagos_test.ts", 4)]),
        ];
        for (quien, salida, nombres, lugares) in casos {
            assert!(parece_salida_de_pruebas(salida), "{quien}: no se reconoce como salida de pruebas");
            assert!(prueba_en_rojo(salida), "{quien}: no se ve en rojo");
            assert_eq!(pruebas_que_fallan(salida), nombres.iter().map(|s| s.to_string()).collect::<Vec<_>>(), "{quien}");
            let hallados = lugares_del_fallo(salida);
            for (ruta, linea) in lugares.iter() {
                assert!(hallados.iter().any(|(r, l)| r == ruta && l == linea), "{quien}: falta {ruta}:{linea} en {hallados:?}");
            }
        }
    }

    #[test]
    fn comandos_de_pruebas() {
        for c in ["python3 -m unittest", "bundle exec rspec", "php phpunit.phar tests", "./gradlew check", "mvn -q verify", "swift test", "npx vitest run"] {
            assert!(es_comando_de_pruebas(c), "{c}");
        }
        assert!(!es_comando_de_pruebas("ls -la"));
    }
}
