//! Pruebas de F4: el archivo (ida y vuelta byte a byte), permisos, la herramienta de punta a punta con el kernel
//! FALSO como proceso (el propio binario de pruebas hablando ZeroMQ), la vista, y la prueba real con ipykernel.
//! Todo este archivo es código de prueba (`#[cfg(test)]` en `cuaderno/mod.rs`): el juez de idioma no lo mira.

use super::formato::Libro;
use super::*;
use crate::permissions::{Decision, Mode, Permissions};
use serde_json::json;

/// Un notebook como lo escribe Jupyter (indent=1, claves ordenadas) con campos que GOTV no conoce.
const NB45: &str = r##"{
 "cells": [
  {
   "cell_type": "markdown",
   "id": "a1b2c3d4",
   "metadata": {
    "tags": ["titulo"]
   },
   "source": [
    "# Análisis ñandú"
   ]
  },
  {
   "cell_type": "code",
   "execution_count": 3,
   "id": "e5f6a7b8",
   "metadata": {
    "collapsed": false,
    "scrolled": true
   },
   "outputs": [
    {
     "name": "stdout",
     "output_type": "stream",
     "text": [
      "42\n"
     ]
    }
   ],
   "source": [
    "x = 41\n",
    "print(x + 1)"
   ]
  },
  {
   "cell_type": "code",
   "execution_count": null,
   "id": "c9d0e1f2",
   "metadata": {},
   "outputs": [],
   "source": []
  }
 ],
 "metadata": {
  "kernelspec": {
   "display_name": "Python 3 (ipykernel)",
   "language": "python",
   "name": "python3"
  },
  "language_info": {
   "name": "python",
   "version": "3.12.4"
  },
  "un_campo_de_otra_extension": {"x": [1, 2.50, "y"]}
 },
 "nbformat": 4,
 "nbformat_minor": 5
}
"##;

fn escribir(d: &Path, nombre: &str, texto: &str) -> PathBuf {
    let p = d.join(nombre);
    std::fs::write(&p, texto).unwrap();
    p
}

/// Lo que quedó igual al principio y al final de dos textos (en bytes).
fn comun(a: &str, b: &str) -> (usize, usize) {
    let pre = a.bytes().zip(b.bytes()).take_while(|(x, y)| x == y).count();
    let suf = a.bytes().rev().zip(b.bytes().rev()).take_while(|(x, y)| x == y).count();
    (pre, suf)
}

#[test]
fn editar_conserva_todo_lo_demas_byte_a_byte() {
    let d = crate::CarpetaPrueba::nueva("gotv-nb-bytes");
    let p = escribir(&d, "a.ipynb", NB45);
    let libro = Libro::leer(&p).unwrap();
    assert_eq!(libro.n(), 3);
    // reemplazar la celda 2: sólo cambian SUS bytes (los números con «2.50» y la extensión ajena quedan tal cual)
    let mut c = libro.celdas()[1].clone();
    c["source"] = formato::lineas("y = 1\nprint(y)");
    let nuevo = libro.reemplazar(1, &c).unwrap();
    let (pre, suf) = comun(NB45, nuevo.texto());
    let (a, b) = (NB45.find("  {\n   \"cell_type\": \"code\",\n   \"execution_count\": 3").unwrap() + 2, NB45.find("  {\n   \"cell_type\": \"code\",\n   \"execution_count\": null").unwrap());
    assert!(pre >= a, "el principio quedó igual hasta la celda tocada ({pre} < {a})");
    assert!(suf >= NB45.len() - b + 1, "el final quedó igual desde la celda siguiente");
    assert!(nuevo.texto().contains("\"un_campo_de_otra_extension\": {\"x\": [1, 2.50, \"y\"]}"));
    // la celda reescrita, como la escribe Jupyter (y sus metadatos y su id, intactos)
    assert!(nuevo.texto().contains("   \"source\": [\n    \"y = 1\\n\",\n    \"print(y)\"\n   ]"), "{}", nuevo.texto());
    assert!(nuevo.texto().contains("\"scrolled\": true") && nuevo.texto().contains("\"id\": \"e5f6a7b8\""));
    // volver a poner la celda original deja el archivo IDÉNTICO
    let ida_y_vuelta = nuevo.reemplazar(1, &libro.celdas()[1]).unwrap();
    assert_eq!(ida_y_vuelta.texto(), NB45);
    // insertar y borrar: también idéntico al volver
    let mas = libro.insertar(1, &libro.celda_nueva("markdown", "nota")).unwrap();
    assert_eq!(mas.n(), 4);
    assert_eq!(mas.borrar(1).unwrap().texto(), NB45);
    // borrar la última (con su coma de antes) y la única
    let sin_ultima = libro.borrar(2).unwrap();
    assert_eq!(sin_ultima.n(), 2);
    let uno = escribir(&d, "uno.ipynb", r#"{"cells": [{"cell_type": "raw", "metadata": {}, "source": "x"}], "metadata": {}, "nbformat": 4, "nbformat_minor": 4}"#);
    let l = Libro::leer(&uno).unwrap().borrar(0).unwrap();
    assert_eq!(l.texto(), r#"{"cells": [], "metadata": {}, "nbformat": 4, "nbformat_minor": 4}"#);
    // a un arreglo vacío en una sola línea se inserta en una sola línea
    let l = l.insertar(0, &l.celda_nueva("code", "1")).unwrap();
    assert_eq!(l.n(), 1);
    assert!(!l.texto().contains('\n'));
}

#[test]
fn el_notebook_nuevo_queda_como_lo_escribe_jupyter() {
    let d = crate::CarpetaPrueba::nueva("gotv-nb-nuevo");
    let p = escribir(&d, "n.ipynb", &formato::vacio(Some(("python3", "Python 3", "python"))));
    let l = Libro::leer(&p).unwrap();
    assert_eq!(l.n(), 0);
    // `"cells": []` va en una línea de un archivo multilínea: la celda entra con la sangría de nbformat
    let mut c = l.celda_nueva("code", "x = 1\nprint(x)");
    c["id"] = json!("0a1b2c3d");
    let t = l.insertar(0, &c).unwrap().texto().to_string();
    let esperado = "{\n \"cells\": [\n  {\n   \"cell_type\": \"code\",\n   \"execution_count\": null,\n   \"id\": \"0a1b2c3d\",\n   \"metadata\": {},\n   \"outputs\": [],\n   \"source\": [\n    \"x = 1\\n\",\n    \"print(x)\"\n   ]\n  }\n ],\n \"metadata\": {\n  \"kernelspec\": {\n   \"display_name\": \"Python 3\",\n   \"language\": \"python\",\n   \"name\": \"python3\"\n  },\n  \"language_info\": {\n   \"name\": \"python\"\n  }\n },\n \"nbformat\": 4,\n \"nbformat_minor\": 5\n}\n";
    assert_eq!(t, esperado);
    // y al borrarla vuelve a ser idéntico al vacío
    let p2 = escribir(&d, "m.ipynb", &t);
    assert_eq!(Libro::leer(&p2).unwrap().borrar(0).unwrap().texto(), formato::vacio(Some(("python3", "Python 3", "python"))));
}

#[test]
fn formato_viejo_sin_ids_por_indice_y_sin_inventar() {
    let d = crate::CarpetaPrueba::nueva("gotv-nb-44");
    let viejo = r#"{"cells": [{"cell_type": "code", "execution_count": 1, "metadata": {}, "outputs": [], "source": ["a = 1"]}, {"cell_type": "markdown", "metadata": {}, "source": ["texto"]}], "metadata": {}, "nbformat": 4, "nbformat_minor": 4}"#;
    let p = escribir(&d, "v.ipynb", viejo);
    let libro = Libro::leer(&p).unwrap();
    assert!(!libro.con_ids());
    assert_eq!(libro.indice(&json!(2)).unwrap(), 1);
    assert_eq!(libro.indice(&json!("2")).unwrap(), 1, "un número como texto también vale");
    let e = libro.indice(&json!("abc123")).unwrap_err().to_string();
    assert!(e.contains("4.4") && (e.contains("número") || e.contains("number")), "{e}");
    assert!(libro.indice(&json!(3)).is_err());
    // la celda nueva NO lleva id (el 4.4 no lo admite)
    let nueva = libro.celda_nueva("code", "b = 2");
    assert!(nueva.get("id").is_none());
    // y en 4.5 sí: 8 hex, distinto de los que ya hay
    let p45 = escribir(&d, "n.ipynb", NB45);
    let l45 = Libro::leer(&p45).unwrap();
    let id = l45.celda_nueva("code", "").get("id").and_then(|v| v.as_str()).unwrap().to_string();
    assert!(id.len() == 8 && id.chars().all(|c| c.is_ascii_hexdigit()), "{id}");
    assert_eq!(l45.indice(&json!("c9d0e1f2")).unwrap(), 2);
}

#[test]
fn no_pisa_lo_que_cambio_otro_programa() {
    let d = crate::CarpetaPrueba::nueva("gotv-nb-pisar");
    let p = escribir(&d, "a.ipynb", NB45);
    let libro = Libro::leer(&p).unwrap();
    let nuevo = libro.borrar(0).unwrap();
    // otro programa (Jupyter abierto) lo guarda mientras tanto
    std::thread::sleep(std::time::Duration::from_millis(20));
    std::fs::write(&p, NB45.replace("ñandú", "cóndor")).unwrap();
    let e = nuevo.guardar().unwrap_err().to_string();
    assert!(e.contains("cambió") || e.contains("changed"), "{e}");
    assert!(std::fs::read_to_string(&p).unwrap().contains("cóndor"), "lo del otro programa quedó");
    // sin cambios ajenos, escribe (atómico: no quedan temporales)
    let l = Libro::leer(&p).unwrap();
    l.borrar(0).unwrap().guardar().unwrap();
    assert_eq!(Libro::leer(&p).unwrap().n(), 2);
    assert_eq!(std::fs::read_dir(&*d).unwrap().count(), 1, "sin temporales sueltos");
}

#[test]
fn permisos_leer_libre_editar_con_ruta_ejecutar_como_bash() {
    let d = crate::CarpetaPrueba::nueva("gotv-nb-perm");
    let raiz = d.canonicalize().unwrap();
    let p = Permissions::new(Mode::Default, &raiz, vec![], vec![], vec![]);
    let a = |v: Value| p.check("notebook", &v);
    assert!(matches!(a(json!({"action": "leer", "path": "a.ipynb"})), Decision::Allow));
    assert!(matches!(a(json!({"action": "kernels"})), Decision::Allow));
    assert!(matches!(a(json!({"action": "ejecutar", "path": "a.ipynb"})), Decision::Ask(_)));
    assert!(matches!(a(json!({"action": "insertar", "path": "a.ipynb", "fuente": "1", "ejecutar": true})), Decision::Ask(_)));
    // editar fuera del proyecto no pasa callado
    assert!(!matches!(a(json!({"action": "insertar", "path": "/etc/x.ipynb", "fuente": "1"})), Decision::Allow));
    // en modo plan: sólo mirar
    let plan = Permissions::new(Mode::Plan, &raiz, vec![], vec![], vec![]);
    assert!(matches!(plan.check("notebook", &json!({"action": "reemplazar", "path": "a.ipynb", "celda": 1, "fuente": "x"})), Decision::Deny(_)));
    assert!(matches!(plan.check("notebook", &json!({"action": "leer", "path": "a.ipynb"})), Decision::Allow));
    assert!(cambia(&json!({"action": "ejecutar"})) && !cambia(&json!({"action": "leer"})));
}

/// Un proyecto con el kernel FALSO instalado en su `.venv` (el propio binario de pruebas como proceso).
fn proyecto_con_kernel_falso(nombre: &str) -> (crate::CarpetaPrueba, PathBuf) {
    let d = crate::CarpetaPrueba::nueva(nombre);
    let root = d.canonicalize().unwrap();
    let k = root.join(".venv/share/jupyter/kernels/falso");
    std::fs::create_dir_all(&k).unwrap();
    let exe = std::env::current_exe().unwrap().display().to_string();
    std::fs::write(
        k.join("kernel.json"),
        json!({
            "argv": [exe, "--exact", "cuaderno::sesion::falso::kernel_falso_proceso", "--ignored", "-q", "--test-threads=1"],
            "display_name": "Falso", "language": "python",
            "env": {"GOTV_KERNEL_FALSO": "{connection_file}"}
        })
        .to_string(),
    )
    .unwrap();
    let nb = formato::vacio(Some(("falso", "Falso", "python")));
    std::fs::write(root.join("cuaderno.ipynb"), nb).unwrap();
    (d, root)
}

#[tokio::test]
async fn de_punta_a_punta_con_el_kernel_falso_como_proceso() {
    let (_d, root) = proyecto_con_kernel_falso("gotv-nb-e2e");
    let dueno = format!("{}#prueba-cuaderno", root.display());
    let nb = root.join("cuaderno.ipynb");
    let h = |v: Value| {
        let (dueno, root) = (dueno.clone(), root.clone());
        async move { herramienta(&dueno, &root, &v).await }
    };
    // texto: el kernel arranca (proceso de exec), se conecta, ejecuta y la salida queda en el notebook
    let r = h(json!({"action": "insertar", "path": "cuaderno.ipynb", "fuente": "print(hola mundo)", "ejecutar": true})).await.unwrap();
    assert!(r.contains("celda 1") || r.contains("cell 1"), "{r}");
    assert!(r.contains("hola mundo") && r.contains("falso") && r.contains("3.99"), "{r}");
    let v: Value = serde_json::from_str(&std::fs::read_to_string(&nb).unwrap()).unwrap();
    assert_eq!(v["cells"][0]["outputs"][0]["text"], json!(["hola mundo\n"]));
    assert_eq!(v["cells"][0]["execution_count"], 1);
    assert!(v["cells"][0]["id"].as_str().is_some_and(|i| i.len() == 8), "4.5: la celda nueva lleva id");
    // el archivo de conexión vive en la temporal propia de la caja (y es sólo del usuario)
    let tmp = crate::sandbox::temporal_propia(&root);
    let conexiones: Vec<PathBuf> = std::fs::read_dir(&tmp).unwrap().flatten().map(|e| e.path()).filter(|p| p.file_name().unwrap().to_string_lossy().starts_with("kernel-")).collect();
    assert_eq!(conexiones.len(), 1, "{conexiones:?}");
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&conexiones[0]).unwrap().permissions().mode() & 0o777, 0o600);
    }
    // imagen: NO entra al texto en base64; se guarda y se dice dónde mirarla
    let r = h(json!({"action": "insertar", "path": "cuaderno.ipynb", "fuente": "imagen", "ejecutar": true})).await.unwrap();
    assert!(r.contains("2×1") && r.contains(".gotv/cuadernos/cuaderno/celda-2-1.png") && r.contains("view_image"), "{r}");
    assert!(!r.contains("iVBOR"), "el base64 no va al modelo");
    assert!(image::open(root.join(".gotv/cuadernos/cuaderno/celda-2-1.png")).is_ok());
    // error: la traza llega sin colores al modelo y en crudo (como Jupyter) al notebook
    let r = h(json!({"action": "insertar", "path": "cuaderno.ipynb", "fuente": "1/0", "ejecutar": true})).await.unwrap();
    assert!(r.contains("ZeroDivisionError: division by zero") && !r.contains('\u{1b}'), "{r}");
    h(json!({"action": "insertar", "path": "cuaderno.ipynb", "fuente": "despues"})).await.unwrap();
    // todas: para en la primera que falla y dice cuántas quedaron
    let r = h(json!({"action": "ejecutar", "path": "cuaderno.ipynb", "todas": true})).await.unwrap();
    assert!(r.contains("celda 3") || r.contains("cell 3"), "{r}");
    assert!(r.contains("las 1 siguientes") || r.contains("the 1 following"), "{r}");
    let v: Value = serde_json::from_str(&std::fs::read_to_string(&nb).unwrap()).unwrap();
    assert_eq!(v["cells"][2]["outputs"][0]["ename"], "ZeroDivisionError");
    assert!(v["cells"][2]["outputs"][0]["traceback"][0].as_str().unwrap().contains('\u{1b}'));
    assert_eq!(v["cells"][3]["outputs"], json!([]), "la que no corrió sigue sin salidas");
    // la vista para la ventana: con la imagen como data URL y la celda tocada marcada
    let t = vista::tarjeta(&root, &json!({"action": "ejecutar", "path": "cuaderno.ipynb", "celda": 2})).unwrap();
    assert_eq!((t["tipo"].as_str(), t["total"].as_u64(), t["marcada"].as_u64()), (Some("cuaderno"), Some(4), Some(2)));
    assert!(t["celdas"][1]["salidas"][0]["imagen"].as_str().unwrap().starts_with("data:image/png;base64,iVBOR"));
    assert!(t["celdas"][2]["salidas"][0]["error"].as_str().unwrap().starts_with("ZeroDivisionError"));
    // colgado: pasado el tope, Ctrl-C (este kernel lo ignora) y luego se le mata; el siguiente arranca de nuevo
    let antes = std::time::Instant::now();
    let r = h(json!({"action": "reemplazar", "path": "cuaderno.ipynb", "celda": 4, "fuente": "colgado", "ejecutar": true, "segundos": 1})).await.unwrap();
    assert!(r.contains("☠") && (r.contains("maté") || r.contains("killed")), "{r}");
    assert!(antes.elapsed() < std::time::Duration::from_secs(20), "nunca sin tope: {:?}", antes.elapsed());
    let r = h(json!({"action": "estado", "path": "cuaderno.ipynb"})).await.unwrap();
    assert!(r.contains("no tiene kernel vivo") || r.contains("no live kernel"), "{r}");
    let r = h(json!({"action": "ejecutar", "path": "cuaderno.ipynb", "celda": 1})).await.unwrap();
    assert!(r.contains("hola mundo") && (r.contains("proceso #") || r.contains("process #")), "{r}");
    // apagar: el proceso termina y su archivo de conexión se borra
    let r = h(json!({"action": "apagar", "path": "cuaderno.ipynb"})).await.unwrap();
    assert!(r.contains("apagado") || r.contains("shut down"), "{r}");
    let quedan = std::fs::read_dir(&tmp).unwrap().flatten().filter(|e| e.file_name().to_string_lossy().starts_with("kernel-")).count();
    assert_eq!(quedan, 0, "los archivos de conexión se borran");
    crate::exec::kill_owner(&dueno);
}

#[tokio::test]
async fn sin_kernel_lo_dice_y_no_inventa_uno() {
    let d = crate::CarpetaPrueba::nueva("gotv-nb-sinkernel");
    let root = d.canonicalize().unwrap();
    std::fs::write(root.join("r.ipynb"), formato::vacio(Some(("rustzz", "Rust ZZ", "rustzz")))).unwrap();
    let dueno = format!("{}#prueba", root.display());
    herramienta(&dueno, &root, &json!({"action": "insertar", "path": "r.ipynb", "fuente": "1"})).await.unwrap();
    let e = herramienta(&dueno, &root, &json!({"action": "ejecutar", "path": "r.ipynb"})).await.unwrap_err().to_string();
    assert!(e.contains("rustzz") && (e.contains("no trae") || e.contains("ships no")), "{e}");
    // las ediciones no necesitan kernel; leer tampoco
    let r = herramienta(&dueno, &root, &json!({"action": "leer", "path": "r.ipynb"})).await.unwrap();
    assert!(r.contains("1 celdas") || r.contains("1 cells"), "{r}");
    // tipo de celda, posición y acción inválidos se explican
    assert!(herramienta(&dueno, &root, &json!({"action": "insertar", "path": "r.ipynb", "fuente": "x", "posicion": 9})).await.is_err());
    assert!(herramienta(&dueno, &root, &json!({"action": "insertar", "path": "r.ipynb", "fuente": "x", "tipo": "video"})).await.is_err());
    assert!(herramienta(&dueno, &root, &json!({"action": "volar", "path": "r.ipynb"})).await.is_err());
    assert!(herramienta(&dueno, &root, &json!({"action": "leer", "path": "r.txt"})).await.is_err());
}

#[tokio::test]
async fn reemplazar_cambia_de_tipo_y_limpia_salidas() {
    let d = crate::CarpetaPrueba::nueva("gotv-nb-tipo");
    let root = d.canonicalize().unwrap();
    std::fs::write(root.join("a.ipynb"), NB45).unwrap();
    let dueno = format!("{}#prueba", root.display());
    herramienta(&dueno, &root, &json!({"action": "reemplazar", "path": "a.ipynb", "celda": "e5f6a7b8", "fuente": "## ahora es texto", "tipo": "markdown"})).await.unwrap();
    let v: Value = serde_json::from_str(&std::fs::read_to_string(root.join("a.ipynb")).unwrap()).unwrap();
    let c = &v["cells"][1];
    assert_eq!((c["cell_type"].as_str(), c["id"].as_str()), (Some("markdown"), Some("e5f6a7b8")));
    assert!(c.get("outputs").is_none() && c.get("execution_count").is_none(), "markdown no lleva salidas: {c}");
    assert_eq!(c["metadata"]["scrolled"], true, "los metadatos de la celda quedan");
    // limpiar_salidas (todas) en el original
    std::fs::write(root.join("b.ipynb"), NB45).unwrap();
    let r = herramienta(&dueno, &root, &json!({"action": "limpiar_salidas", "path": "b.ipynb"})).await.unwrap();
    assert!(r.contains(" 1 "), "{r}");
    let v: Value = serde_json::from_str(&std::fs::read_to_string(root.join("b.ipynb")).unwrap()).unwrap();
    assert_eq!((v["cells"][1]["outputs"].clone(), v["cells"][1]["execution_count"].clone()), (json!([]), Value::Null));
    assert_eq!(v["metadata"]["un_campo_de_otra_extension"]["x"][1], 2.5);
    // crear: nbformat 4.5 vacío y válido, con su primera celda
    let r = herramienta(&dueno, &root, &json!({"action": "crear", "path": "nuevo/c.ipynb", "fuente": "print(1)"})).await.unwrap();
    assert!(r.contains("4.5"), "{r}");
    let l = Libro::leer(&root.join("nuevo/c.ipynb")).unwrap();
    assert_eq!((l.n(), l.con_ids(), l.lenguaje().as_deref()), (1, true, Some("python")));
    assert!(herramienta(&dueno, &root, &json!({"action": "crear", "path": "nuevo/c.ipynb"})).await.is_err(), "no pisa uno que existe");
}

#[test]
fn la_vista_al_reabrir_solo_la_ultima_de_cada_notebook() {
    let d = crate::CarpetaPrueba::nueva("gotv-nb-reabrir");
    let root = d.canonicalize().unwrap();
    std::fs::write(root.join("a.ipynb"), NB45).unwrap();
    let tc = |id: &str, a: Value| gotv_engine::ToolCall { id: id.into(), name: "notebook".into(), arguments: a };
    let llamada = |id: &str, a: Value| gotv_engine::Message::assistant_calls("", &[tc(id, a)]);
    let resultado = |id: &str, t: &str| gotv_engine::Message::tool_result(&tc(id, json!({})), t);
    let h = vec![
        llamada("1", json!({"action": "leer", "path": "a.ipynb"})),
        resultado("1", "[notebook]"),
        llamada("2", json!({"action": "reemplazar", "path": "a.ipynb", "celda": 2, "fuente": "x"})),
        resultado("2", "✓"),
        llamada("3", json!({"action": "estado", "path": "a.ipynb"})),
        resultado("3", "kernel"),
    ];
    assert!(vista::de_historial(&h, 1, &root).is_none(), "hay una posterior del mismo notebook");
    let t = vista::de_historial(&h, 3, &root).expect("la última");
    assert_eq!((t["marcada"].as_u64(), t["celdas"][0]["fuente"].as_str()), (Some(2), Some("# Análisis ñandú")));
    assert!(vista::de_historial(&h, 5, &root).is_none(), "estado no muestra el notebook");
}

/// Prueba REAL con ipykernel (sin modelo): un `.venv` con ipykernel y matplotlib en `GOTV_VENV_IPYKERNEL`. Corre
/// el kernel DENTRO de la caja del chat (F14.0), dibuja una gráfica, ve el error, interrumpe por señal un
/// `sleep` y comprueba que el estado sigue.
#[tokio::test]
#[ignore = "necesita un .venv con ipykernel y matplotlib en GOTV_VENV_IPYKERNEL"]
async fn real_con_ipykernel_en_la_caja() {
    let venv = PathBuf::from(std::env::var("GOTV_VENV_IPYKERNEL").expect("GOTV_VENV_IPYKERNEL"));
    let d = crate::CarpetaPrueba::nueva("gotv-nb-real");
    let root = d.canonicalize().unwrap();
    std::os::unix::fs::symlink(&venv, root.join(".venv")).unwrap();
    let dueno = format!("{}#prueba-real", root.display());
    crate::exec::set_sandbox(&dueno, Some(crate::sandbox::Spec { root: root.clone(), hosts: vec![], all_net: false, sockets: vec![], credenciales: vec![], entorno: vec![] }));
    let h = |v: Value| {
        let (dueno, root) = (dueno.clone(), root.clone());
        async move { herramienta(&dueno, &root, &v).await }
    };
    let r = h(json!({"action": "kernels"})).await.unwrap();
    assert!(r.lines().next().unwrap().contains("python") && (r.contains("del proyecto") || r.contains("project's")), "{r}");
    h(json!({"action": "crear", "path": "real.ipynb"})).await.unwrap();
    let t0 = std::time::Instant::now();
    let r = h(json!({"action": "insertar", "path": "real.ipynb", "fuente": "import sys\nx = 41\nprint(sys.prefix, x + 1)", "ejecutar": true})).await.unwrap();
    eprintln!("── arranque + celda 1 ({:?}):\n{r}", t0.elapsed());
    assert!(r.contains(" 42"), "{r}");
    let t0 = std::time::Instant::now();
    let r = h(json!({"action": "insertar", "path": "real.ipynb", "fuente": "import matplotlib.pyplot as plt\nplt.plot([1, 4, 9, 16])\nplt.title('cuadrados')\nplt.show()", "ejecutar": true, "segundos": 120})).await.unwrap();
    eprintln!("── gráfica ({:?}):\n{r}", t0.elapsed());
    assert!(r.contains("🖼") && r.contains(".png"), "{r}");
    let png = root.join(".gotv/cuadernos/real/celda-2-1.png");
    let img = image::open(&png).unwrap();
    assert!(img.width() > 100 && img.height() > 100);
    let r = h(json!({"action": "insertar", "path": "real.ipynb", "fuente": "1/0", "ejecutar": true})).await.unwrap();
    eprintln!("── error:\n{r}");
    assert!(r.contains("ZeroDivisionError") && !r.contains('\u{1b}'), "{r}");
    // corregir el error de la celda anterior (lo que hará el modelo) y volver a correrla
    let r = h(json!({"action": "reemplazar", "path": "real.ipynb", "celda": 3, "fuente": "x / 1", "ejecutar": true})).await.unwrap();
    assert!(r.contains("⇒ 41.0"), "{r}");
    // tope: SIGINT → KeyboardInterrupt, y el kernel SIGUE con su estado
    let t0 = std::time::Instant::now();
    let r = h(json!({"action": "insertar", "path": "real.ipynb", "fuente": "import time\ntime.sleep(60)", "ejecutar": true, "segundos": 2})).await.unwrap();
    eprintln!("── interrumpida ({:?}):\n{r}", t0.elapsed());
    assert!(r.contains("KeyboardInterrupt") && t0.elapsed() < std::time::Duration::from_secs(15), "{r}");
    let r = h(json!({"action": "insertar", "path": "real.ipynb", "fuente": "x + 1", "ejecutar": true})).await.unwrap();
    assert!(r.contains("⇒ 42"), "el estado sigue: {r}");
    let v: Value = serde_json::from_str(&std::fs::read_to_string(root.join("real.ipynb")).unwrap()).unwrap();
    assert!(v["cells"][1]["outputs"].as_array().unwrap().iter().any(|o| o["data"]["image/png"].is_string()), "la gráfica quedó en el notebook");
    // un kernel NUEVO (sin estado) dibuja sin rehacer la caché de fuentes de matplotlib (MPLCONFIGDIR persistente)
    let r = h(json!({"action": "reiniciar", "path": "real.ipynb"})).await.unwrap();
    assert!(r.contains("python"), "{r}");
    let t0 = std::time::Instant::now();
    let r = h(json!({"action": "ejecutar", "path": "real.ipynb", "celda": 2})).await.unwrap();
    eprintln!("── gráfica tras reiniciar ({:?}):\n{r}", t0.elapsed());
    assert!(r.contains("🖼") && !r.contains("font cache") && !r.contains("not a writable"), "{r}");
    h(json!({"action": "apagar", "path": "real.ipynb"})).await.unwrap();
    crate::exec::set_sandbox(&dueno, None);
    crate::exec::kill_owner(&dueno);
}
