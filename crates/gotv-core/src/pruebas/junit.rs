//! F15.1 · los informes XML de los corredores: JUnit (pytest, `node --test`, `bun test`, `deno test`,
//! PHPUnit, Gradle, Maven) y TRX (`dotnet test`). Con `quick-xml` (ya venía con `calamine`). Un XML roto o
//! cortado da lo que se alcanzó a leer.
use super::corrida::{lugar, recorta, Corrida, Fallo};
use quick_xml::events::{BytesStart, Event};
use quick_xml::{Reader, XmlVersion};
use std::path::{Path, PathBuf};

fn atributo(e: &BytesStart, clave: &str) -> Option<String> {
    e.attributes().flatten().find(|a| a.key.local_name().as_ref() == clave).and_then(|a| a.normalized_value(XmlVersion::Implicit1_0).ok().map(|v| v.into_owned()))
}

/// El texto de una entidad (`&lt;`, `&#10;`).
fn entidad(nombre: &str) -> String {
    if let Some(n) = nombre.strip_prefix('#') {
        let c = match n.strip_prefix('x') {
            Some(h) => u32::from_str_radix(h, 16).ok(),
            None => n.parse().ok(),
        };
        return c.and_then(char::from_u32).map(String::from).unwrap_or_default();
    }
    quick_xml::escape::resolve_predefined_entity(nombre).unwrap_or("").to_string()
}

#[derive(Default)]
enum Estado {
    #[default]
    Paso,
    Fallo,
    Omitida,
}

#[derive(Default)]
struct Caso {
    nombre: String,
    archivo: Option<String>,
    linea: Option<u32>,
    estado: Estado,
    mensaje: String,
    texto: String,
}

fn cerrar(c: &mut Corrida, k: Caso, root: &Path) {
    match k.estado {
        Estado::Paso => c.pasaron += 1,
        Estado::Omitida => c.omitidas += 1,
        Estado::Fallo => {
            let propio = k.archivo.as_ref().map(|a| if Path::new(a).is_absolute() { PathBuf::from(a) } else { root.join(a.trim_start_matches("./")) });
            // el lugar: la línea del ARCHIVO de la prueba en la pila; si no, la primera del proyecto; si no, la
            // que diga el informe
            let todo = format!("{}\n{}", k.mensaje, k.texto);
            let en_propio = propio.as_ref().and_then(|p| todo.lines().find_map(|l| lugar(l, root).filter(|(q, _)| q == p)));
            let (archivo, linea) = match en_propio.or_else(|| lugar(&todo, root)) {
                Some((p, l)) => (Some(p), Some(l)),
                None => (propio, k.linea),
            };
            let base = if k.mensaje.trim().is_empty() { &k.texto } else { &k.mensaje };
            let limpio: Vec<&str> = base.lines().filter(|l| !l.trim_start().starts_with("at ")).collect();
            c.fallaron.push(Fallo { nombre: k.nombre, archivo, linea, mensaje: recorta(&limpio.join("\n")) });
        }
    }
}

fn caso_de(e: &BytesStart) -> Caso {
    Caso {
        nombre: atributo(e, "name").unwrap_or_default(),
        // `file` (node, bun); deno pone el archivo en `classname` («./calc_test.ts»); pytest pone el módulo
        archivo: atributo(e, "file").or_else(|| atributo(e, "classname").filter(|x| x.starts_with("./") || x.contains('/'))),
        linea: atributo(e, "line").and_then(|l| l.parse().ok()),
        ..Default::default()
    }
}

fn marcar_fallo(caso: &mut Option<Caso>, e: &BytesStart) {
    if let Some(k) = caso.as_mut() {
        k.estado = Estado::Fallo;
        if k.mensaje.is_empty() {
            k.mensaje = atributo(e, "message").unwrap_or_default();
        }
    }
}

/// JUnit XML.
pub fn leer_junit(xml: &str, root: &Path) -> Corrida {
    let mut c = Corrida::default();
    let mut r = Reader::from_str(xml);
    let mut caso: Option<Caso> = None;
    // dentro de <failure>/<error>: su texto es el detalle
    let mut en_fallo = false;
    loop {
        let ev = match r.read_event() {
            Ok(Event::Eof) | Err(_) => break,
            Ok(ev) => ev,
        };
        match ev {
            Event::Start(e) if e.local_name().as_ref() == "testcase" => {
                if let Some(k) = caso.take() {
                    cerrar(&mut c, k, root);
                }
                caso = Some(caso_de(&e));
            }
            // <testcase/> vacío: pasó
            Event::Empty(e) if e.local_name().as_ref() == "testcase" => {
                if let Some(k) = caso.take() {
                    cerrar(&mut c, k, root);
                }
                cerrar(&mut c, caso_de(&e), root);
            }
            // sólo un <failure>…</failure> trae texto (un <failure/> no tiene cierre)
            Event::Start(e) if matches!(e.local_name().as_ref(), "failure" | "error") => {
                marcar_fallo(&mut caso, &e);
                en_fallo = true;
            }
            Event::Empty(e) if matches!(e.local_name().as_ref(), "failure" | "error") => marcar_fallo(&mut caso, &e),
            Event::Start(e) | Event::Empty(e) if e.local_name().as_ref() == "skipped" => {
                if let Some(k) = caso.as_mut() {
                    k.estado = Estado::Omitida;
                }
            }
            Event::Text(t) if en_fallo => {
                if let Some(k) = caso.as_mut() {
                    k.texto.push_str(&t);
                }
            }
            Event::CData(t) if en_fallo => {
                if let Some(k) = caso.as_mut() {
                    k.texto.push_str(&t);
                }
            }
            Event::GeneralRef(g) if en_fallo => {
                if let Some(k) = caso.as_mut() {
                    k.texto.push_str(&entidad(&g));
                }
            }
            Event::End(e) => match e.local_name().as_ref() {
                "failure" | "error" => en_fallo = false,
                "testcase" => {
                    if let Some(k) = caso.take() {
                        cerrar(&mut c, k, root);
                    }
                }
                _ => {}
            },
            _ => {}
        }
    }
    // un <testcase> que quedó abierto (XML cortado): sólo cuenta si ya traía fallo u omisión; dar por
    // aprobada una prueba a medias sería un verde falso
    if let Some(k) = caso.take().filter(|k| !matches!(k.estado, Estado::Paso)) {
        cerrar(&mut c, k, root);
    }
    c
}

/// TRX de `dotnet test`: `<UnitTestResult testName outcome>` con `<Message>` y `<StackTrace>` (` in
/// ruta:line 23`).
pub fn leer_trx(xml: &str, root: &Path) -> Corrida {
    let mut c = Corrida::default();
    let mut r = Reader::from_str(xml);
    let mut caso: Option<(String, String)> = None;
    let (mut mensaje, mut pila) = (String::new(), String::new());
    let mut dentro: Option<&'static str> = None;
    let cerrar_trx = |c: &mut Corrida, (nombre, resultado): (String, String), mensaje: &str, pila: &str| match resultado.as_str() {
        "Passed" => c.pasaron += 1,
        "Failed" | "Error" | "Timeout" | "Aborted" => {
            let lugar = pila.lines().find_map(|l| {
                let (_, resto) = l.split_once(" in ")?;
                let (ruta, n) = resto.rsplit_once(":line ")?;
                let p = PathBuf::from(ruta.trim());
                (p.starts_with(root) || p.is_relative()).then_some(())?;
                Some((if p.is_relative() { root.join(p) } else { p }, n.trim().parse::<u32>().ok()?))
            });
            c.fallaron.push(Fallo { nombre, archivo: lugar.as_ref().map(|x| x.0.clone()), linea: lugar.map(|x| x.1), mensaje: recorta(mensaje) });
        }
        _ => c.omitidas += 1,
    };
    loop {
        let ev = match r.read_event() {
            Ok(Event::Eof) | Err(_) => break,
            Ok(ev) => ev,
        };
        match ev {
            Event::Start(e) | Event::Empty(e) if e.local_name().as_ref() == "UnitTestResult" => {
                if let Some(k) = caso.take() {
                    cerrar_trx(&mut c, k, &mensaje, &pila);
                }
                mensaje.clear();
                pila.clear();
                caso = Some((atributo(&e, "testName").unwrap_or_default(), atributo(&e, "outcome").unwrap_or_default()));
            }
            Event::Start(e) if e.local_name().as_ref() == "Message" => dentro = Some("m"),
            Event::Start(e) if e.local_name().as_ref() == "StackTrace" => dentro = Some("p"),
            Event::Text(t) => match dentro {
                Some("m") => mensaje.push_str(&t),
                Some("p") => pila.push_str(&t),
                _ => {}
            },
            Event::GeneralRef(g) => match dentro {
                Some("m") => mensaje.push_str(&entidad(&g)),
                Some("p") => pila.push_str(&entidad(&g)),
                _ => {}
            },
            Event::End(e) => match e.local_name().as_ref() {
                "Message" | "StackTrace" => dentro = None,
                "UnitTestResult" => {
                    if let Some(k) = caso.take() {
                        cerrar_trx(&mut c, k, &mensaje, &pila);
                    }
                }
                _ => {}
            },
            _ => {}
        }
    }
    if let Some(k) = caso.take() {
        cerrar_trx(&mut c, k, &mensaje, &pila);
    }
    c
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dato(n: &str) -> String {
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/datos/pruebas").join(n)).unwrap()
    }

    /// Salidas REALES de pytest, `node --test`, `bun test` y `deno test` con un fallo conocido: nombre,
    /// archivo y línea exactos.
    #[test]
    fn junit_de_cada_corredor() {
        let r = PathBuf::from("/RAIZ");
        for (f, pasaron, fallaron, omitidas, nombre, archivo, linea) in [
            ("pytest-junit.xml", 1, 2, 1, "test_mal", "/RAIZ/tests/test_calc.py", 9),
            ("node-junit.xml", 1, 1, 1, "mal", "/RAIZ/calc.test.js", 6),
            ("bun-junit.xml", 1, 1, 1, "mal", "/RAIZ/calc.test.ts", 5),
            ("deno-junit.xml", 1, 1, 1, "mal", "/RAIZ/calc_test.ts", 5),
        ] {
            let c = leer_junit(&dato(f), &r);
            assert_eq!((c.pasaron, c.fallaron.len() as u32, c.omitidas), (pasaron, fallaron, omitidas), "{f}: {c:?}");
            let x = c.fallaron.iter().find(|x| x.nombre == nombre).unwrap_or_else(|| panic!("{f}: {c:?}"));
            assert_eq!((x.archivo.clone(), x.linea), (Some(PathBuf::from(archivo)), Some(linea)), "{f}");
            assert!(!x.mensaje.is_empty() && !x.mensaje.contains("node:internal"), "{f}: {}", x.mensaje);
        }
        // swift-testing sin pruebas: nada, sin romper
        assert_eq!(leer_junit(&dato("swift-testing.xml"), &r).total(), 0);
        // cortado a la mitad: lo que alcanzó
        let medio: String = dato("pytest-junit.xml").chars().take(400).collect();
        assert!(leer_junit(&medio, &r).pasaron == 1);
    }

    /// TRX: muestra con la forma de `dotnet test --logger trx` (en esta Mac no hay .NET para sacar una real).
    #[test]
    fn trx_de_dotnet() {
        let xml = r#"<?xml version="1.0" encoding="utf-8"?>
<TestRun xmlns="http://microsoft.com/schemas/VisualStudio/TeamTest/2010"><Results>
<UnitTestResult testName="Calc.Tests.Bien" outcome="Passed" duration="00:00:00.01" />
<UnitTestResult testName="Calc.Tests.Mal" outcome="Failed"><Output><ErrorInfo><Message>Assert.Equal() Failure&#10;Expected: 7&#10;Actual:   6</Message><StackTrace>   at Calc.Tests.Mal() in /RAIZ/Calc.Tests/UnitTest1.cs:line 14</StackTrace></ErrorInfo></Output></UnitTestResult>
<UnitTestResult testName="Calc.Tests.Luego" outcome="NotExecuted" />
</Results></TestRun>"#;
        let c = leer_trx(xml, Path::new("/RAIZ"));
        assert_eq!((c.pasaron, c.fallaron.len(), c.omitidas), (1, 1, 1), "{c:?}");
        assert_eq!((c.fallaron[0].archivo.clone(), c.fallaron[0].linea), (Some(PathBuf::from("/RAIZ/Calc.Tests/UnitTest1.cs")), Some(14)));
        assert!(c.fallaron[0].mensaje.contains("Expected: 7\nActual:   6"), "{}", c.fallaron[0].mensaje);
    }
}
