//! F4 · Notebooks de Jupyter: la herramienta `notebook`. Edita celdas SIN rehacer el resto del archivo
//! (`formato`), descubre el kernel del proyecto (`kernels`), habla el protocolo de Jupyter por ZeroMQ desde
//! Rust con sus mensajes firmados (`protocolo`, `sesion`), escribe las salidas en el `.ipynb` y pinta la vista
//! en la ventana (`vista`). GOTV no trae Python: el kernel es el del proyecto (nivel 3 de L1).

pub mod formato;
pub mod kernels;
pub mod protocolo;
pub mod sesion;
pub mod vista;

use crate::permissions::Kind;
use anyhow::{anyhow, bail, Result};
use formato::Libro;
use serde_json::{json, Value};
use sesion::{Fin, Kernel, Resultado};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

/// Tope por celda, por omisión y máximo (trampa 4: nunca sin tope).
const TOPE: u64 = 120;
const TOPE_MAX: u64 = 1800;
/// Un kernel sin uso tanto tiempo se apaga (su memoria vuelve al sistema).
const INACTIVIDAD: Duration = Duration::from_secs(30 * 60);
/// Lo que se le muestra al modelo de cada salida de texto (lo demás queda en el notebook).
const TEXTO_MAX: usize = 3_000;

fn accion(args: &Value) -> &str {
    args["action"].as_str().or(args["accion"].as_str()).unwrap_or("leer")
}

/// La ficha de la herramienta (va en `tools::schemas`; nace diferida).
pub fn esquema() -> Value {
    let s = |d: &str| json!({ "type": "string", "description": d });
    let i = |d: &str| json!({ "type": "integer", "description": d });
    let b = |d: &str| json!({ "type": "boolean", "description": d });
    json!({
        "type": "function",
        "function": {
            "name": "notebook",
            "description": crate::i18n::t(
                "Notebooks de Jupyter (.ipynb): editar celdas y EJECUTARLAS con el kernel del proyecto (su .venv con ipykernel; evcxr para Rust). action: leer · crear (lenguaje) · insertar (fuente, tipo=code|markdown, posicion: el número que tendrá; sin ella, al final) · reemplazar (celda, fuente) · borrar (celda) · limpiar_salidas (celda, o todas) · ejecutar (celda, o todas=true; segundos = tope por celda) · kernels · estado · interrumpir · reiniciar · apagar. `celda` = su número (1 = la primera) o su id. Con ejecutar=true, insertar y reemplazar la corren al momento. Las salidas quedan en el notebook. Una gráfica se MUESTRA en la celda (plt.show(), o la figura como último valor): su imagen queda DENTRO del notebook y te llega su ruta para mirarla con view_image; no uses matplotlib.use('Agg') ni savefig salvo que el usuario pida un archivo de imagen. Nunca edites un .ipynb con edit o write_file.",
                "Jupyter notebooks (.ipynb): edit cells and RUN them with the project's kernel (its .venv with ipykernel; evcxr for Rust). action: leer · crear (lenguaje) · insertar (fuente, tipo=code|markdown, posicion: the number it will have; without it, at the end) · reemplazar (celda, fuente) · borrar (celda) · limpiar_salidas (celda, or all) · ejecutar (celda, or todas=true; segundos = cap per cell) · kernels · estado · interrumpir · reiniciar · apagar. `celda` = its number (1 = the first) or its id. With ejecutar=true, insertar and reemplazar run it right away. Outputs stay in the notebook. A chart is SHOWN in the cell (plt.show(), or the figure as the last value): its image stays INSIDE the notebook and you get its path to look at it with view_image; do not use matplotlib.use('Agg') or savefig unless the user asks for an image file. Never edit a .ipynb with edit or write_file."
            ),
            "parameters": {
                "type": "object",
                "properties": {
                    "action": { "type": "string", "enum": ["leer", "crear", "insertar", "reemplazar", "borrar", "limpiar_salidas", "ejecutar", "kernels", "estado", "interrumpir", "reiniciar", "apagar"] },
                    "path": s(crate::i18n::t("el notebook (.ipynb)", "the notebook (.ipynb)")),
                    "celda": { "type": ["integer", "string"], "description": crate::i18n::t("su número (1 = la primera) o su id", "its number (1 = the first) or its id") },
                    "fuente": s(crate::i18n::t("con insertar/reemplazar: el código o el texto de la celda", "with insertar/reemplazar: the cell's code or text")),
                    "tipo": s(crate::i18n::t("code (por omisión), markdown o raw", "code (default), markdown or raw")),
                    "posicion": i(crate::i18n::t("con insertar: el número que tendrá la celda nueva (1 = al principio)", "with insertar: the number the new cell will have (1 = at the start)")),
                    "ejecutar": b(crate::i18n::t("con insertar/reemplazar: ejecutarla al momento", "with insertar/reemplazar: run it right away")),
                    "todas": b(crate::i18n::t("con ejecutar: todas las celdas de código en orden (se detiene en el primer error)", "with ejecutar: all code cells in order (stops at the first error)")),
                    "segundos": i(crate::i18n::t("tope por celda (120, máx. 1800): pasado, se interrumpe", "cap per cell (120, max 1800): past it, it is interrupted")),
                    "lenguaje": s(crate::i18n::t("con crear: python (por omisión), rust…", "with crear: python (default), rust…"))
                },
                "required": ["action"]
            }
        }
    })
}

/// Qué clase de permiso pide cada acción: mirar es leer; editar celdas es editar (con su ruta); ejecutar corre
/// código del notebook en el kernel (como `bash`).
pub fn clase(args: &Value) -> Kind {
    match accion(args) {
        "leer" | "kernels" | "estado" | "interrumpir" | "apagar" => Kind::Read,
        "ejecutar" | "reiniciar" => Kind::Exec,
        "insertar" | "reemplazar" if args["ejecutar"].as_bool() == Some(true) => Kind::Exec,
        _ => Kind::Edit,
    }
}

/// ¿La acción puede cambiar el `.ipynb`? (el agente toma su punto de restauración antes)
pub fn cambia(args: &Value) -> bool {
    !matches!(accion(args), "leer" | "kernels" | "estado" | "interrumpir" | "apagar" | "reiniciar")
}

type Ranura = Arc<tokio::sync::Mutex<Option<Kernel>>>;

/// Un kernel por chat y notebook (como Jupyter: uno por notebook abierto).
fn ranuras() -> &'static std::sync::Mutex<HashMap<(String, PathBuf), Ranura>> {
    static R: std::sync::OnceLock<std::sync::Mutex<HashMap<(String, PathBuf), Ranura>>> = std::sync::OnceLock::new();
    R.get_or_init(Default::default)
}

fn ranura(dueno: &str, nb: &Path) -> Ranura {
    ranuras().lock().unwrap().entry((dueno.to_string(), nb.to_path_buf())).or_default().clone()
}

/// Los kernels de OTROS notebooks sin uso desde hace 30 min se apagan (se mira en cada llamada a `notebook`;
/// los demás mueren con su chat, como todo proceso de `exec`).
pub async fn podar_inactivos(excepto: Option<(&str, &Path)>) {
    let todas: Vec<Ranura> = ranuras()
        .lock()
        .unwrap()
        .iter()
        .filter(|((d, p), _)| excepto.is_none_or(|(ed, ep)| d.as_str() != ed || p.as_path() != ep))
        .map(|(_, r)| r.clone())
        .collect();
    for r in todas {
        let Ok(mut g) = r.try_lock() else { continue };
        if g.as_ref().is_some_and(|k| k.ultimo.elapsed() > INACTIVIDAD || !k.vivo()) {
            if let Some(k) = g.take() {
                k.apagar().await;
            }
        }
    }
}

fn relativa(root: &Path, p: &Path) -> String {
    p.strip_prefix(root).unwrap_or(p).display().to_string()
}

/// El texto de una salida para el modelo: cabeza y cola si es largo (los errores suelen estar al final).
fn acotar(t: &str) -> String {
    let t = t.trim_end();
    if t.chars().count() <= TEXTO_MAX {
        return t.to_string();
    }
    let c: Vec<char> = t.chars().collect();
    let cab: String = c[..TEXTO_MAX / 3].iter().collect();
    let col: String = c[c.len() - TEXTO_MAX * 2 / 3..].iter().collect();
    crate::tr!("{cab}\n… ({} caracteres omitidos; completos en el notebook) …\n{col}", "{cab}\n… ({} characters omitted; complete in the notebook) …\n{col}", c.len() - TEXTO_MAX)
}

/// La carpeta de las imágenes de un notebook (`.gotv/cuadernos/<nombre>/`).
pub fn carpeta_imagenes(root: &Path, nb: &Path) -> PathBuf {
    let nombre: String = nb.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default().chars().map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '-' }).collect();
    root.join(".gotv/cuadernos").join(if nombre.is_empty() { "notebook".to_string() } else { nombre })
}

/// Trampa 2: las imágenes en base64 NO entran al contexto: se guardan y el modelo las mira con `view_image`.
fn guardar_imagen(dir: &Path, numero: usize, k: usize, mime: &str, b64: &str) -> Option<(PathBuf, u32, u32)> {
    use base64::Engine;
    let limpio: String = b64.chars().filter(|c| !c.is_whitespace()).collect();
    let bytes = base64::engine::general_purpose::STANDARD.decode(limpio).ok()?;
    let ext = if mime == "image/jpeg" { "jpg" } else { "png" };
    std::fs::create_dir_all(dir).ok()?;
    let p = dir.join(format!("celda-{numero}-{k}.{ext}"));
    std::fs::write(&p, &bytes).ok()?;
    let (w, h) = image::load_from_memory(&bytes).map(|i| (i.width(), i.height())).unwrap_or((0, 0));
    Some((p, w, h))
}

/// Lo que ve el modelo de una celda ejecutada.
fn resumen(root: &Path, nb: &Path, numero: usize, r: &Resultado, tope: u64) -> String {
    let cuenta = r.cuenta.map(|n| format!(" [{n}]")).unwrap_or_default();
    let fin = match r.fin {
        Fin::Ok => "✓".to_string(),
        Fin::Error => crate::i18n::t("✗ error", "✗ error").to_string(),
        Fin::Interrumpida => crate::tr!("⏱ interrumpida al pasar el tope de {tope} s", "⏱ interrupted past the {tope} s cap"),
        Fin::Matado => crate::i18n::t("☠ kernel matado", "☠ kernel killed").to_string(),
        Fin::Murio => crate::i18n::t("☠ el kernel murió", "☠ the kernel died").to_string(),
    };
    let mut o = vec![format!("── {}{cuenta} {fin} ({}) ──", crate::tr!("celda {numero}", "cell {numero}"), crate::mensajes::duracion(r.segundos))];
    let dir = carpeta_imagenes(root, nb);
    let mut k = 0;
    for s in &r.salidas {
        match s["output_type"].as_str().unwrap_or("") {
            "stream" => {
                let t = formato::texto_de(&s["text"]);
                if !t.trim().is_empty() {
                    o.push(if s["name"] == "stderr" { format!("stderr: {}", acotar(&t)) } else { acotar(&t) });
                }
            }
            "execute_result" | "display_data" => {
                let d = &s["data"];
                if let Some((mime, b64)) = ["image/png", "image/jpeg"].iter().find_map(|m| d[*m].as_str().map(|b| (*m, b))) {
                    k += 1;
                    match guardar_imagen(&dir, numero, k, mime, b64) {
                        Some((p, w, h)) => o.push(crate::tr!(
                            "🖼 imagen {w}×{h} → {} (mírala con view_image)",
                            "🖼 image {w}×{h} → {} (look at it with view_image)",
                            relativa(root, &p)
                        )),
                        None => o.push(crate::i18n::t("🖼 imagen (no se pudo decodificar)", "🖼 image (could not be decoded)").into()),
                    }
                } else if d["text/plain"].is_array() || d["text/plain"].is_string() {
                    let html = if d["text/html"].is_null() { "" } else { " (+ html)" };
                    let pre = if s["output_type"] == "execute_result" { "⇒ " } else { "" };
                    o.push(format!("{pre}{}{html}", acotar(&formato::texto_de(&d["text/plain"]))));
                } else if let Some(m) = d.as_object().and_then(|m| m.keys().next()) {
                    o.push(crate::tr!("(salida {m})", "({m} output)"));
                }
            }
            "error" => {
                let traza: Vec<String> = s["traceback"].as_array().map(|a| a.iter().filter_map(|l| l.as_str()).map(crate::exec::strip_ansi).collect()).unwrap_or_default();
                let traza = traza.join("\n");
                let lineas: Vec<&str> = traza.lines().collect();
                let ultimas = lineas[lineas.len().saturating_sub(40)..].join("\n");
                o.push(format!("✗ {}: {}\n{}", s["ename"].as_str().unwrap_or("Error"), s["evalue"].as_str().unwrap_or(""), acotar(&ultimas)));
            }
            _ => {}
        }
    }
    match r.fin {
        Fin::Matado => o.push(crate::i18n::t(
            "El kernel no volvió ni con Ctrl-C: lo maté. Su estado (variables, imports) se perdió; la próxima ejecución arranca uno nuevo (ejecuta antes las celdas que preparan los datos).",
            "The kernel did not come back even with Ctrl-C: I killed it. Its state (variables, imports) is lost; the next run starts a new one (run the cells that prepare the data first).",
        ).into()),
        Fin::Murio => o.push(crate::i18n::t(
            "El kernel murió durante la celda (¿se quedó sin memoria?). Su estado se perdió; la próxima ejecución arranca uno nuevo.",
            "The kernel died during the cell (out of memory?). Its state is lost; the next run starts a new one.",
        ).into()),
        _ => {}
    }
    o.join("\n")
}

/// La celda `c` con las salidas de `r` (sus demás campos —metadatos, id, lo que GOTV no conoce— quedan).
fn con_salidas(c: &Value, r: &Resultado) -> Value {
    let mut c = c.clone();
    c["outputs"] = Value::Array(r.salidas.clone());
    c["execution_count"] = r.cuenta.map(Value::from).unwrap_or(Value::Null);
    c
}

/// Una celda con sus salidas borradas (como «Clear Output» de Jupyter).
fn limpia(c: &Value) -> Value {
    let mut c = c.clone();
    if c["cell_type"] == "code" {
        c["outputs"] = json!([]);
        c["execution_count"] = Value::Null;
    }
    c
}

/// La celda pedida en `args["celda"]`.
fn indice(libro: &Libro, args: &Value) -> Result<usize> {
    libro.indice(&args["celda"])
}

fn tipo_valido(t: &str) -> Result<&str> {
    match t {
        "code" | "markdown" | "raw" => Ok(t),
        // juez-idioma: datos (lo que un modelo puede escribir en `tipo`)
        "codigo" | "código" => Ok("code"),
        "texto" => Ok("markdown"),
        otro => bail!("{}", crate::tr!("tipo «{otro}» no existe: code · markdown · raw", "type «{otro}» does not exist: code · markdown · raw")),
    }
}

/// Cambia el tipo de una celda dejando sólo los campos que su tipo admite (nbformat lo valida).
fn de_tipo(c: &Value, tipo: &str) -> Value {
    let mut c = c.clone();
    c["cell_type"] = Value::from(tipo);
    if let Some(m) = c.as_object_mut() {
        if tipo == "code" {
            m.entry("outputs").or_insert(json!([]));
            m.entry("execution_count").or_insert(Value::Null);
        } else {
            m.remove("outputs");
            m.remove("execution_count");
        }
    }
    c
}

/// El kernel de este notebook (vivo), arrancándolo si hace falta. Devuelve también una nota si se (re)arrancó.
async fn kernel_de<'a>(g: &'a mut Option<Kernel>, dueno: &str, root: &Path, nb: &Path, libro: &Libro) -> Result<(&'a mut Kernel, String)> {
    let mut nota = String::new();
    if g.as_ref().is_some_and(|k| !k.vivo()) {
        *g = None;
        nota = crate::i18n::t("(el kernel anterior ya no vivía: arranqué uno nuevo, sin el estado de antes)\n", "(the previous kernel was no longer alive: I started a new one, without the earlier state)\n").into();
    }
    if g.is_none() {
        let spec = kernels::elegir(root, libro.kernel_nombre().as_deref(), libro.lenguaje().as_deref())?;
        let cwd = nb.parent().unwrap_or(root);
        let k = Kernel::lanzar(dueno, root, cwd, spec).await?;
        nota.push_str(&crate::tr!(
            "(kernel {} · {} {} · proceso #{})\n",
            "(kernel {} · {} {} · process #{})\n",
            k.spec.nombre,
            k.lenguaje,
            k.version,
            k.proceso().map(|n| n.to_string()).unwrap_or_else(|| "-".into())
        ));
        *g = Some(k);
    }
    Ok((g.as_mut().expect("kernel"), nota))
}

/// Ejecuta las celdas `indices` (en orden; para en la primera que no termina bien), escribe sus salidas en el
/// notebook y devuelve lo que ve el modelo.
async fn correr(dueno: &str, root: &Path, nb: &Path, indices: Vec<usize>, tope: u64) -> Result<String> {
    let mut libro = Libro::leer(nb)?;
    let r = ranura(dueno, nb);
    let mut g = r.lock().await;
    let (k, nota) = kernel_de(&mut g, dueno, root, nb, &libro).await?;
    let mut o = vec![nota.trim_end().to_string()];
    let total = indices.len();
    let mut perdido = false;
    for (n, i) in indices.iter().copied().enumerate() {
        let c = libro.celdas()[i].clone();
        let codigo = formato::texto_de(&c["source"]);
        if codigo.trim().is_empty() {
            continue;
        }
        let res = k.ejecutar(&codigo, Duration::from_secs(tope)).await?;
        o.push(resumen(root, nb, i + 1, &res, tope));
        libro = libro.reemplazar(i, &con_salidas(&c, &res))?;
        if res.fin != Fin::Ok {
            perdido = matches!(res.fin, Fin::Matado | Fin::Murio);
            if n + 1 < total {
                o.push(crate::tr!(
                    "Se detuvo en la celda {}: las {} siguientes no se ejecutaron.",
                    "Stopped at cell {}: the {} following ones were not run.",
                    i + 1,
                    total - n - 1
                ));
            }
            break;
        }
    }
    if perdido {
        *g = None;
    }
    drop(g);
    // trampa 5: si otro programa lo cambió mientras corría, NO se pisa (las salidas se dicen igual)
    match libro.guardar() {
        Ok(()) => o.push(crate::tr!("✓ salidas guardadas en {}", "✓ outputs saved in {}", relativa(root, nb))),
        Err(e) => o.push(format!("⚠ {e}")),
    }
    Ok(o.into_iter().filter(|l| !l.is_empty()).collect::<Vec<_>>().join("\n"))
}

/// La herramienta `notebook`.
pub async fn herramienta(dueno: &str, root: &Path, args: &Value) -> Result<String> {
    let a = accion(args);
    if a == "kernels" {
        let ks = kernels::descubrir(root);
        if ks.is_empty() {
            return Ok(crate::i18n::t(
                "no hay kernels de Jupyter instalados. Para Python: instala ipykernel en el entorno del proyecto (p. ej. `uv venv && uv pip install ipykernel`). GOTV no trae Python propio.",
                "no Jupyter kernels installed. For Python: install ipykernel in the project's environment (e.g. `uv venv && uv pip install ipykernel`). GOTV ships no Python of its own.",
            )
            .into());
        }
        return Ok(ks
            .iter()
            .map(|k| {
                let de = if k.del_proyecto { crate::i18n::t("del proyecto", "project's") } else { crate::i18n::t("del usuario/sistema", "user/system") };
                format!("{} · {} · {} · {de} · {}", k.nombre, k.lenguaje, k.mostrar, k.argv.join(" "))
            })
            .collect::<Vec<_>>()
            .join("\n"));
    }
    let ruta = args["path"].as_str().filter(|p| !p.trim().is_empty()).ok_or_else(|| anyhow!("{}", crate::i18n::t("falta `path` (el .ipynb)", "missing `path` (the .ipynb)")))?;
    let nb = crate::paths::resolve(root, ruta);
    if nb.extension().map(|e| e.to_string_lossy().to_lowercase()) != Some("ipynb".into()) {
        bail!("{}", crate::i18n::t("`notebook` trabaja con .ipynb (path: «analisis.ipynb»)", "`notebook` works with .ipynb (path: «analysis.ipynb»)"));
    }
    let rel = relativa(root, &nb);
    podar_inactivos(Some((dueno, &nb))).await;
    if matches!(clase(args), Kind::Exec) && !nb.starts_with(root) {
        bail!("{}", crate::i18n::t("sólo se ejecutan notebooks del proyecto", "only project notebooks are run"));
    }
    let tope = args["segundos"].as_u64().unwrap_or(TOPE).clamp(1, TOPE_MAX);
    match a {
        "leer" => crate::docs::notebook(&nb),
        "crear" => {
            if nb.exists() {
                bail!("{}", crate::tr!("{rel} ya existe: para cambiarlo usa insertar o reemplazar", "{rel} already exists: to change it use insertar or reemplazar"));
            }
            let lengua = args["lenguaje"].as_str().unwrap_or("python").to_lowercase();
            let spec = kernels::elegir(root, None, Some(&lengua)).ok();
            let texto = match &spec {
                Some(k) => formato::vacio(Some((k.nombre.as_str(), if k.mostrar.is_empty() { k.nombre.as_str() } else { k.mostrar.as_str() }, k.lenguaje.as_str()))),
                None if lengua == "python" => formato::vacio(Some(("python3", "Python 3", "python"))),
                None => formato::vacio(None),
            };
            if let Some(d) = nb.parent() {
                std::fs::create_dir_all(d)?;
            }
            std::fs::write(&nb, texto)?;
            let mut out = crate::tr!("✓ creado {rel} (nbformat 4.5)", "✓ created {rel} (nbformat 4.5)");
            if spec.is_none() {
                out.push_str(&crate::tr!(
                    "\n⚠ no veo un kernel de {lengua} en el proyecto: podrás editarlo, pero para ejecutarlo hace falta uno (action=kernels)",
                    "\n⚠ I see no {lengua} kernel in the project: you can edit it, but running it needs one (action=kernels)"
                ));
            }
            if let Some(f) = args["fuente"].as_str() {
                let libro = Libro::leer(&nb)?;
                let tipo = tipo_valido(args["tipo"].as_str().unwrap_or("code"))?;
                libro.insertar(0, &libro.celda_nueva(tipo, f))?.guardar()?;
                out.push_str(&crate::i18n::t(" con su primera celda", " with its first cell"));
            }
            Ok(out)
        }
        "insertar" => {
            let libro = Libro::leer(&nb)?;
            let fuente = args["fuente"].as_str().ok_or_else(|| anyhow!("{}", crate::i18n::t("falta `fuente`", "missing `fuente`")))?;
            let tipo = tipo_valido(args["tipo"].as_str().unwrap_or("code"))?;
            let n = libro.n();
            let pos = match args["posicion"].as_u64() {
                Some(p) if p >= 1 && (p as usize) <= n + 1 => p as usize - 1,
                Some(p) => bail!("{}", crate::tr!("posicion {p} no vale: va de 1 a {}", "posicion {p} is not valid: it goes from 1 to {}", n + 1)),
                None => n,
            };
            let c = libro.celda_nueva(tipo, fuente);
            let id = c["id"].as_str().map(|i| format!(", id {i}")).unwrap_or_default();
            libro.insertar(pos, &c)?.guardar()?;
            let hecho = crate::tr!("✓ celda {} insertada ({tipo}{id}) · {rel} tiene {} celdas", "✓ cell {} inserted ({tipo}{id}) · {rel} has {} cells", pos + 1, n + 1);
            if args["ejecutar"].as_bool() == Some(true) && tipo == "code" {
                return Ok(format!("{hecho}\n{}", correr(dueno, root, &nb, vec![pos], tope).await?));
            }
            Ok(hecho)
        }
        "reemplazar" => {
            let libro = Libro::leer(&nb)?;
            let i = indice(&libro, args)?;
            let fuente = args["fuente"].as_str().ok_or_else(|| anyhow!("{}", crate::i18n::t("falta `fuente`", "missing `fuente`")))?;
            let vieja = &libro.celdas()[i];
            let tipo = match args["tipo"].as_str() {
                Some(t) => tipo_valido(t)?.to_string(),
                None => vieja["cell_type"].as_str().unwrap_or("code").to_string(),
            };
            // la fuente nueva deja viejas sus salidas: se borran (como al editar y volver a correr)
            let mut c = limpia(&de_tipo(vieja, &tipo));
            c["source"] = formato::lineas(fuente);
            libro.reemplazar(i, &c)?.guardar()?;
            let hecho = crate::tr!("✓ celda {} reemplazada ({tipo}) en {rel}", "✓ cell {} replaced ({tipo}) in {rel}", i + 1);
            if args["ejecutar"].as_bool() == Some(true) && tipo == "code" {
                return Ok(format!("{hecho}\n{}", correr(dueno, root, &nb, vec![i], tope).await?));
            }
            Ok(hecho)
        }
        "borrar" => {
            let libro = Libro::leer(&nb)?;
            let i = indice(&libro, args)?;
            let n = libro.n();
            libro.borrar(i)?.guardar()?;
            Ok(crate::tr!("✓ celda {} borrada · {rel} tiene {} celdas", "✓ cell {} deleted · {rel} has {} cells", i + 1, n - 1))
        }
        "limpiar_salidas" => {
            let mut libro = Libro::leer(&nb)?;
            let indices: Vec<usize> = if args["celda"].is_null() { (0..libro.n()).collect() } else { vec![indice(&libro, args)?] };
            let mut cuantas = 0;
            for i in indices {
                let c = &libro.celdas()[i];
                if c["cell_type"] == "code" && (c["outputs"].as_array().is_some_and(|o| !o.is_empty()) || !c["execution_count"].is_null()) {
                    let l = limpia(c);
                    libro = libro.reemplazar(i, &l)?;
                    cuantas += 1;
                }
            }
            if cuantas > 0 {
                libro.guardar()?;
            }
            Ok(crate::tr!("✓ salidas limpiadas en {cuantas} celdas de {rel}", "✓ outputs cleared in {cuantas} cells of {rel}"))
        }
        "ejecutar" => {
            let libro = Libro::leer(&nb)?;
            let indices: Vec<usize> = if args["todas"].as_bool() == Some(true) || args["celda"].is_null() {
                (0..libro.n()).filter(|&i| libro.celdas()[i]["cell_type"] == "code").collect()
            } else {
                let i = indice(&libro, args)?;
                if libro.celdas()[i]["cell_type"] != "code" {
                    bail!("{}", crate::tr!("la celda {} es {}: sólo se ejecutan celdas de código", "cell {} is {}: only code cells are run", i + 1, libro.celdas()[i]["cell_type"].as_str().unwrap_or("?")));
                }
                vec![i]
            };
            if indices.is_empty() {
                return Ok(crate::tr!("{rel} no tiene celdas de código", "{rel} has no code cells"));
            }
            correr(dueno, root, &nb, indices, tope).await
        }
        "estado" => {
            let r = ranura(dueno, &nb);
            let g = r.lock().await;
            Ok(match g.as_ref() {
                Some(k) if k.vivo() => crate::tr!(
                    "kernel {} vivo · {} {} · {} ejecuciones · proceso #{}",
                    "kernel {} alive · {} {} · {} runs · process #{}",
                    k.spec.nombre,
                    k.lenguaje,
                    k.version,
                    k.ejecutadas,
                    k.proceso().map(|n| n.to_string()).unwrap_or_else(|| "-".into())
                ),
                _ => crate::tr!("{rel} no tiene kernel vivo (se arranca al ejecutar)", "{rel} has no live kernel (one starts when running)"),
            })
        }
        "interrumpir" => {
            let r = ranura(dueno, &nb);
            let mut g = r.lock().await;
            match g.as_mut() {
                Some(k) if k.vivo() => {
                    k.interrumpir().await;
                    Ok(crate::i18n::t("✓ Ctrl-C enviado al kernel", "✓ Ctrl-C sent to the kernel").into())
                }
                _ => Ok(crate::i18n::t("no hay kernel vivo que interrumpir", "there is no live kernel to interrupt").into()),
            }
        }
        "apagar" | "reiniciar" => {
            let r = ranura(dueno, &nb);
            let mut g = r.lock().await;
            if let Some(k) = g.take() {
                k.apagar().await;
            }
            if a == "apagar" {
                return Ok(crate::i18n::t("✓ kernel apagado", "✓ kernel shut down").into());
            }
            let libro = Libro::leer(&nb)?;
            let (_, nota) = kernel_de(&mut g, dueno, root, &nb, &libro).await?;
            Ok(crate::tr!("✓ kernel reiniciado (sin estado) {}", "✓ kernel restarted (no state) {}", nota.trim()))
        }
        otra => bail!(
            "{}",
            crate::tr!(
                "acción «{otra}» no existe: leer · crear · insertar · reemplazar · borrar · limpiar_salidas · ejecutar · kernels · estado · interrumpir · reiniciar · apagar",
                "action «{otra}» does not exist: leer · crear · insertar · reemplazar · borrar · limpiar_salidas · ejecutar · kernels · estado · interrumpir · reiniciar · apagar"
            )
        ),
    }
}

#[cfg(test)]
mod pruebas;
