//! O1 · Las apps del Mac por su LENGUAJE de scripts (dentro de la herramienta `desktop`, con el mismo
//! permiso de Control Mac). La accesibilidad (agent-desktop) lee y pulsa lo que hay en pantalla; esto
//! le PIDE a la app lo que sabe hacer: crear un documento, escribir en una celda, guardar como PDF.
//! Medido 2026-11-11: «escríbeme en Word una carta» → Word abrió sin ventana y el 9B pidió
//! `list-windows` 27 veces; con `make new document` es una línea.
//!
//! Comandos (como primer elemento de `args` de `desktop`):
//!   - `["recetas", "Microsoft Word"]`: recetas PROBADAS en esta Mac para las apps conocidas;
//!   - `["diccionario", "App", "palabra"]`: lo que la app declara saber hacer (su `sdef`), filtrado;
//!   - `["applescript", "tell application …"]` y `["jxa", "Application('…')…"]`: ejecutar.
//!
//! Todo por `/usr/bin/osascript` y `/usr/bin/sdef` (del sistema). macOS pide su permiso de
//! Automatización la primera vez que se controla cada app.

use anyhow::{anyhow, bail, Result};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::io::AsyncWriteExt;

/// ¿Es un comando de scripts (y no de agent-desktop)?
pub fn es_comando(primero: &str) -> bool {
    matches!(primero, "recetas" | "diccionario" | "applescript" | "jxa" | "guardar" | "abrir")
}

/// K4 · ¿el AppleScript/JXA sólo LEE? (sin verbos que cambien algo). Su resultado cuenta como
/// observar lo hecho en el Mac.
pub fn script_solo_lee(script: &str) -> bool {
    let s = format!(" {} ", script.to_lowercase().replace(['\n', '\r', '(', ')'], " "));
    const CAMBIAN: &[&str] = &[
        " set ", " key code ", " keystroke ", " click ", " activate ", " launch ", " open ", " quit ", " close ", " save ",
        " make ", " delete ", " move ", " duplicate ", " do shell script ", " display ", " say ", " run ", " reopen ",
        " print ", " empty ", " eject ", " restart ", " shut down ", " sleep ", " log out ", " select ", " perform ",
        " increment ", " decrement ", " export ", " import ", " send ", " reply ", " forward ", " write ",
    ];
    !s.trim().is_empty() && !CAMBIAN.iter().any(|v| s.contains(v)) && !s.contains(".set") && !s.contains("=")
}

/// Las apps de Office viven en una caja de arena: NO pueden escribir ni abrir fuera de su contenedor
/// sin que macOS le pida al usuario elegir la carpeta (medido 2026-11-11: Word dio -1708 «no entiende
/// save as» y mostró «selecciona la carpeta», imposible en /private/tmp; PowerPoint con la ruta en
/// texto NO guardó y no avisó). GOTV no está en esa caja: la app guarda/abre en SU contenedor y GOTV
/// mueve/copia el archivo.
struct Office {
    app: &'static str,
    bundle: &'static str,
    /// extensión → script de guardar (con `{c}` = ruta POSIX en el contenedor)
    guardar: &'static [(&'static str, &'static str)],
    abrir: &'static str,
    /// la clase de sus documentos (para ver si ya hay OTRO abierto con el mismo nombre)
    clase: &'static str,
}

const OFFICE: &[Office] = &[
    Office {
        app: "Microsoft Word",
        bundle: "com.microsoft.Word",
        guardar: &[
            ("docx", "save as active document file name \"{c}\" file format format document"),
            ("pdf", "save as active document file name \"{c}\" file format format PDF"),
        ],
        abrir: "open (POSIX file \"{c}\")",
        clase: "document",
    },
    Office {
        app: "Microsoft Excel",
        bundle: "com.microsoft.Excel",
        guardar: &[
            ("xlsx", "save workbook as active workbook filename \"{c}\" file format Excel XML file format"),
            ("pdf", "save as active sheet filename \"{c}\" file format PDF file format"),
        ],
        abrir: "open workbook workbook file name \"{c}\"",
        clase: "workbook",
    },
    Office {
        app: "Microsoft PowerPoint",
        bundle: "com.microsoft.Powerpoint",
        guardar: &[
            ("pptx", "save active presentation in (POSIX file \"{c}\")"),
            ("pdf", "save active presentation in (POSIX file \"{c}\") as save as PDF"),
        ],
        abrir: "open (POSIX file \"{c}\")",
        clase: "presentation",
    },
];

fn office(app: &str) -> Option<&'static Office> {
    let a = app.to_lowercase();
    OFFICE.iter().find(|o| {
        let corto = o.app.trim_start_matches("Microsoft ").to_lowercase();
        a == o.app.to_lowercase() || a == corto
    })
}

fn contenedor(o: &Office) -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default().join("Library/Containers").join(o.bundle).join("Data/tmp/gotv")
}

/// `["guardar", "Microsoft Word", "carta.pdf"]`: guarda el documento ACTIVO de la app
/// en ese formato (por la extensión). Si el destino existe, el anterior queda como respaldo.
async fn guardar(app: &str, destino: &str, root: &Path, timeout_s: u64) -> Result<String> {
    let o = office(app).ok_or_else(|| anyhow!("{}", crate::tr!("`guardar` es para Word, Excel y PowerPoint («{app}»): en otras apps, su receta o su diccionario", "`guardar` is for Word, Excel and PowerPoint («{app}»): for other apps, its recipe or dictionary")))?;
    // relativa = dentro del proyecto (medido: el 9B inventó «/Users/void/carta.pdf»). Y SÓLO dentro
    // del proyecto, como `write_file` y `bash`: escribir fuera no se cuela por aquí.
    let pedida = Path::new(destino.trim());
    let d = crate::paths::resolve(root, destino);
    if (pedida.is_absolute() && !pedida.starts_with(root)) || !d.starts_with(root) {
        return Err(anyhow!("{}", crate::tr!(
            "no guardo fuera del proyecto ({}): usa una ruta dentro de {} (p. ej. «carta.pdf»); si el usuario lo quiere en otro sitio, que te lo pida y lo mueves con su permiso",
            "I do not save outside the project ({}): use a path inside {} (e.g. «carta.pdf»); if the user wants it elsewhere, they will ask and you move it with their permission",
            d.display(),
            root.display()
        )));
    }
    let ext = d.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    let Some((_, plantilla)) = o.guardar.iter().find(|(e, _)| *e == ext) else {
        let v: Vec<&str> = o.guardar.iter().map(|(e, _)| *e).collect();
        return Err(anyhow!("{}", crate::tr!("{} guarda como: {} (pediste .{ext})", "{} saves as: {} (you asked .{ext})", o.app, v.join(", "))));
    };
    let caja = contenedor(o);
    std::fs::create_dir_all(&caja)?;
    let nombre = d.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| format!("documento.{ext}"));
    limpiar_contenedor(&caja);
    // Office no deja dos documentos abiertos con el MISMO nombre: guardar el activo como «x.xlsx»
    // con otro «x.xlsx» abierto da «Error de parámetros (-50)» (medido 2026-11-11: el de la corrida
    // anterior seguía abierto). No se cierra solo (podría ser trabajo sin guardar): se dice.
    if ext != "pdf" {
        let ver = format!(
            // (probado en las tres apps; `repeat with d in every workbook` da -50 en Excel)
            "tell application \"{app}\"\nset ns to name of every {c}\nif (count of ns) = 0 then return \"ninguno\"\nif name of active {c} is \"{n}\" then return \"activo\"\nif ns contains \"{n}\" then return \"otro\"\nreturn \"libre\"\nend tell",
            app = o.app,
            c = o.clase,
            n = nombre.replace('"', "")
        );
        match script(false, &ver, timeout_s).await.as_deref() {
            Ok("otro") => {
                return Err(anyhow!("{}", crate::tr!(
                    "{} ya tiene OTRO {} abierto llamado «{nombre}» y no deja dos con el mismo nombre. Si ese otro no hace falta, ciérralo ([\"applescript\",\"tell application \\\"{}\\\" to close {} \\\"{nombre}\\\" saving no\"]) — sólo si no es trabajo del usuario —, o guarda con otro nombre.",
                    "{} already has ANOTHER {} open named «{nombre}» and does not allow two with the same name. If that other one is not needed, close it ([\"applescript\",\"tell application \\\"{}\\\" to close {} \\\"{nombre}\\\" saving no\"]) — only if it is not the user's work —, or save under another name.",
                    o.app, o.clase, o.app, o.clase
                )));
            }
            Ok("ninguno") => {
                return Err(anyhow!("{}", crate::tr!("{} no tiene ningún documento abierto: créalo o ábrelo primero ([\"recetas\",\"{}\"])", "{} has no open document: create or open one first ([\"recetas\",\"{}\"])", o.app, o.app)));
            }
            _ => {}
        }
    }
    // Cada guardado en una carpeta NUEVA del contenedor, y el archivo de dentro no se toca después
    // (medido 2026-11-11, dos diálogos que colgaron la app): si se mueve, el documento abierto pierde
    // su archivo y Word pide «Conceder acceso al archivo»; si se guarda encima, Excel pregunta
    // «¿Deseas reemplazar el archivo existente?». Se COPIA afuera.
    let sello = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let dentro = caja.join(format!("{sello}")).join(&nombre);
    std::fs::create_dir_all(dentro.parent().unwrap())?;
    let orden = plantilla.replace("{c}", &dentro.to_string_lossy());
    script(false, &format!("tell application \"{}\"\n{orden}\nend tell", o.app), timeout_s).await?;
    if !dentro.exists() {
        return Err(anyhow!("{}", crate::tr!("{} no escribió el archivo (¿tiene un documento abierto? [\"applescript\",\"tell application \\\"{}\\\" to count documents\"])", "{} did not write the file (does it have an open document? [\"applescript\",\"tell application \\\"{}\\\" to count documents\"])", o.app, o.app)));
    }
    if let Some(dir) = d.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut nota = String::new();
    if d.exists() {
        let r = respaldo(&d);
        std::fs::rename(&d, &r)?;
        nota = crate::tr!(" · el anterior quedó en {}", " · the previous one is at {}", r.display());
    }
    std::fs::copy(&dentro, &d)?;
    let kb = std::fs::metadata(&d).map(|m| m.len() / 1024).unwrap_or(0);
    Ok(crate::tr!("✓ guardado {} ({kb} KB){nota}", "✓ saved {} ({kb} KB){nota}", d.display()))
}

/// `["abrir", "Microsoft Excel", "/ruta/ABSOLUTA/datos.xlsx"]`: abre una COPIA en la app (desde su
/// contenedor, sin pedirle carpetas al usuario). Para guardar los cambios: `guardar` a la ruta original.
async fn abrir(app: &str, origen: &str, root: &Path, timeout_s: u64) -> Result<String> {
    let o = office(app).ok_or_else(|| anyhow!("{}", crate::tr!("`abrir` es para Word, Excel y PowerPoint («{app}»): en otras apps, [\"launch\",…] o su receta", "`abrir` is for Word, Excel and PowerPoint («{app}»)")))?;
    // abrir lee una COPIA: puede venir de cualquier carpeta del usuario
    let p = crate::paths::resolve(root, origen);
    if !p.is_file() {
        return Err(anyhow!("{}", crate::tr!("no existe {origen} (relativa al proyecto o absoluta)", "{origen} does not exist (relative to the project or absolute)")));
    }
    let caja = contenedor(o);
    limpiar_contenedor(&caja);
    // carpeta nueva: no se pisa una copia que la app tenga abierta
    let sello = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let dentro = caja.join(format!("{sello}")).join(p.file_name().unwrap());
    std::fs::create_dir_all(dentro.parent().unwrap())?;
    std::fs::copy(&p, &dentro)?;
    let orden = o.abrir.replace("{c}", &dentro.to_string_lossy());
    script(false, &format!("tell application \"{}\"\nactivate\n{orden}\nend tell", o.app), timeout_s).await?;
    Ok(crate::tr!(
        "✓ abierto en {} (una copia). Para guardar los cambios: [\"guardar\",\"{}\",\"{origen}\"] (el original queda como respaldo).",
        "✓ opened in {} (a copy). To save the changes: [\"guardar\",\"{}\",\"{origen}\"] (the original is kept as a backup).",
        o.app,
        o.app
    ))
}

/// O2 · RECALCULAR un .xlsx con el Excel del usuario (el `recalc.py` de Claude Code usa LibreOffice;
/// aquí es un resto roto y el Portador usa Office): una librería escribe las fórmulas SIN valor; Excel
/// abre una copia en SU contenedor, `calculate full`, la guarda (guardado normal: «guardar como»
/// encima abre el diálogo «¿reemplazar?», medido) y se cierra; GOTV copia el resultado sobre el
/// original. Sin Excel instalado: error que lo dice.
pub async fn recalcular_excel(path: &Path, timeout_s: u64) -> Result<()> {
    if ruta_app("Microsoft Excel").is_none() {
        bail!("{}", crate::i18n::t("no hay Microsoft Excel en este Mac: las fórmulas quedan sin valor guardado (Excel las calcula al abrir el archivo)", "Microsoft Excel is not on this Mac: formulas stay without a cached value (Excel computes them when the file is opened)"));
    }
    let o = office("Microsoft Excel").expect("Excel está en OFFICE");
    let nombre = path.file_name().map(|n| n.to_string_lossy().into_owned()).ok_or_else(|| anyhow!("ruta sin nombre de archivo"))?;
    let caja = contenedor(o);
    limpiar_contenedor(&caja);
    // ¿estaba abierto? PRIMERO y con `is running`, que no lo arranca (medido: un `tell application`
    // antes de esta pregunta lo arrancaba y Excel nunca se cerraba). Si no lo estaba, se cierra al
    // terminar; si lo estaba, se queda como estaba (puede tener trabajo del usuario).
    let abierto = script(false, "return application \"Microsoft Excel\" is running", 15).await.map(|r| r.trim() == "true").unwrap_or(true);
    // un libro abierto con el MISMO nombre impide abrir este (Office no deja dos iguales: -50);
    // con Excel cerrado no puede haberlo
    if abierto {
        let ya = script(false, &format!("tell application \"Microsoft Excel\"\nif (count of workbooks) = 0 then return \"no\"\nif (name of every workbook) contains \"{}\" then return \"si\"\nreturn \"no\"\nend tell", nombre.replace('"', "")), timeout_s).await.unwrap_or_default();
        if ya.trim() == "si" {
            bail!("{}", crate::tr!("Excel ya tiene abierto otro libro llamado «{nombre}»: ciérralo (si no es trabajo del usuario) o recalcula con otro nombre", "Excel already has another workbook named «{nombre}» open: close it (if it is not the user's work) or recalculate under another name"));
        }
    }
    let sello = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let dentro = caja.join(format!("{sello}")).join(&nombre);
    std::fs::create_dir_all(dentro.parent().unwrap())?;
    std::fs::copy(path, &dentro)?;
    let c = dentro.to_string_lossy().replace('"', "");
    let r = script(
        false,
        &format!("tell application \"Microsoft Excel\"\nopen workbook workbook file name \"{c}\"\ncalculate full\nsave workbook \"{nombre}\"\nclose workbook \"{nombre}\" saving no\nend tell", nombre = nombre.replace('"', "")),
        timeout_s,
    )
    .await;
    if !abierto {
        let _ = script(false, "tell application \"Microsoft Excel\" to if (count of workbooks) = 0 then quit", 30).await;
        // el cierre tarda: si otro recálculo llega antes, lo vería «abierto» y lo dejaría así
        for _ in 0..40 {
            if script(false, "return application \"Microsoft Excel\" is running", 10).await.map(|r| r.trim() != "true").unwrap_or(true) {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        }
    }
    r?;
    std::fs::copy(&dentro, path)?;
    Ok(())
}

/// O3 · Un documento de Office → PDF con SU app (Word/Excel/PowerPoint según la extensión), sin
/// mostrar nada al usuario más que la app un instante: copia en el contenedor de la app, abrir,
/// guardar como PDF ahí dentro, cerrar sin guardar, copiar el PDF al destino; si la app no estaba
/// abierta, se cierra (el `recalc.py`/`soffice --convert-to pdf` de Claude Code, con el Office del
/// usuario). Si la app no puede abrir el archivo (está roto), el error lo dice: también es un juez.
pub async fn exportar_pdf(origen: &Path, destino: &Path, timeout_s: u64) -> Result<()> {
    let ext = origen.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    let app = match ext.as_str() {
        "docx" | "doc" | "rtf" => "Microsoft Word",
        "xlsx" | "xls" | "csv" => "Microsoft Excel",
        "pptx" | "ppt" => "Microsoft PowerPoint",
        _ => gotv_engine::bail_tr!("no sé exportar .{ext} a PDF con Office (docx, xlsx, pptx)", "I don't know how to export .{ext} to PDF with Office (docx, xlsx, pptx)"),
    };
    if ruta_app(app).is_none() {
        bail!("{}", crate::tr!("no hay {app} en este Mac para convertir a PDF", "{app} is not on this Mac to convert to PDF"));
    }
    let o = office(app).expect("está en OFFICE");
    let nombre = origen.file_name().map(|n| n.to_string_lossy().replace('"', "")).ok_or_else(|| anyhow!("ruta sin nombre"))?;
    let abierto = script(false, &format!("return application \"{app}\" is running"), 15).await.map(|r| r.trim() == "true").unwrap_or(true);
    let caja = contenedor(o);
    limpiar_contenedor(&caja);
    let sello = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let dentro = caja.join(format!("{sello}")).join(&nombre);
    std::fs::create_dir_all(dentro.parent().unwrap())?;
    std::fs::copy(origen, &dentro)?;
    let pdf = dentro.with_extension("pdf");
    let (c, p) = (dentro.to_string_lossy().replace('"', ""), pdf.to_string_lossy().replace('"', ""));
    let orden = match app {
        "Microsoft Word" => format!("open (POSIX file \"{c}\")\nsave as active document file name \"{p}\" file format format PDF\nclose active document saving no"),
        // el libro ENTERO (todas las hojas), no sólo la activa
        "Microsoft Excel" => format!("open workbook workbook file name \"{c}\"\nsave workbook as active workbook filename \"{p}\" file format PDF file format\nclose active workbook saving no"),
        _ => format!("open (POSIX file \"{c}\")\nsave active presentation in (POSIX file \"{p}\") as save as PDF\nclose active presentation saving no"),
    };
    let r = script(false, &format!("tell application \"{app}\"\n{orden}\nend tell"), timeout_s).await;
    if !abierto {
        let _ = script(false, &format!("tell application \"{app}\" to quit saving no"), 30).await;
        for _ in 0..40 {
            if script(false, &format!("return application \"{app}\" is running"), 10).await.map(|r| r.trim() != "true").unwrap_or(true) {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        }
    }
    r.map_err(|e| anyhow!("{app} no pudo abrir/convertir {nombre}: {e}"))?;
    if !pdf.exists() {
        gotv_engine::bail_tr!("{app} no escribió el PDF de {nombre}", "{app} did not write the PDF of {nombre}");
    }
    if let Some(d) = destino.parent() {
        std::fs::create_dir_all(d)?;
    }
    std::fs::copy(&pdf, destino)?;
    Ok(())
}

/// Las copias de trabajo en el contenedor de la app (`…/Data/tmp/gotv`, sólo esa carpeta) con más
/// de un día se borran: un documento que sigue abierto se toca al guardar, así que no es «viejo».
fn limpiar_contenedor(caja: &Path) {
    if !caja.ends_with("Data/tmp/gotv") {
        return;
    }
    let dia = std::time::Duration::from_secs(24 * 3600);
    for e in std::fs::read_dir(caja).into_iter().flatten().flatten() {
        let viejo = e.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_some_and(|d| d > dia);
        // sólo las subcarpetas de sello (números) y los archivos sueltos de esta carpeta
        let sello = e.file_name().to_string_lossy().chars().all(|c| c.is_ascii_digit());
        if viejo && e.path().is_dir() && sello {
            let _ = std::fs::remove_dir_all(e.path());
        } else if viejo && e.path().is_file() {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

/// `x.docx` → `x.respaldo-AAAAMMDD-HHMMSS.docx` (al lado).
fn respaldo(p: &Path) -> PathBuf {
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let stem = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let ext = p.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    p.with_file_name(format!("{stem}.respaldo-{t}{ext}"))
}

/// Ejecuta un comando de scripts (`args[0]` ya es uno de `es_comando`).
pub async fn ejecutar(args: &[String], root: &Path, timeout_s: u64) -> Result<String> {
    let resto = &args[1..];
    match args[0].as_str() {
        // todas las palabras (medido: el 9B mandó ["recetas","platform","PowerPoint"])
        "recetas" => Ok(recetas(&resto.join(" "))),
        c @ ("guardar" | "abrir") => {
            let (Some(app), Some(ruta)) = (resto.first(), resto.get(1)) else {
                return Err(anyhow!("{}", crate::tr!("uso: [\"{c}\",\"Microsoft Word\",\"/ruta/ABSOLUTA/archivo.docx\"]", "usage: [\"{c}\",\"Microsoft Word\",\"/ABSOLUTE/path/file.docx\"]")));
            };
            if c == "guardar" { guardar(app, ruta, root, timeout_s).await } else { abrir(app, ruta, root, timeout_s).await }
        }
        "diccionario" => {
            let app = resto.first().ok_or_else(|| anyhow!("{}", crate::i18n::t("falta la app: [\"diccionario\",\"Microsoft Word\",\"document\"]", "missing app: [\"dictionary\",\"Microsoft Word\",\"document\"]")))?;
            diccionario(app, &resto[1..]).await
        }
        l @ ("applescript" | "jxa") => {
            let mut codigo = resto.join("\n");
            // la RUTA de un archivo de script (medido: el 9B lo escribió con write_file y pasó la ruta)
            let t = codigo.trim();
            if t.starts_with('/') && !t.contains('\n') && Path::new(t).is_file() {
                codigo = std::fs::read_to_string(t)?;
            }
            if codigo.trim().is_empty() {
                return Err(anyhow!("{}", crate::i18n::t("falta el código del script", "missing the script code")));
            }
            if let Some(m) = atajo_a_la_terminal(&codigo) {
                return Err(anyhow!("{m}"));
            }
            // Excel: `set x to value of range "B5"…` + `return x` da «missing value» aunque la celda
            // tenga valor (medido 2026-11-11: el 9B leyó así el total 5 veces); `get value of range`
            // directo sí lo da. Se reintenta una vez leyendo directo.
            // fórmulas de Excel en español → inglés (lo único que entiende `formula`)
            let mut nota_formulas = String::new();
            if l == "applescript" && codigo.contains("set formula") {
                let (c, cambios) = formulas_en_ingles(&codigo);
                if !cambios.is_empty() {
                    codigo = c;
                    nota_formulas = crate::tr!(
                        "\n(fórmula traducida al inglés, que es lo que entiende Excel por script: {})",
                        "\n(formula translated to English, which is what Excel understands by script: {})",
                        cambios.join(", ")
                    );
                }
            }
            // UNA sola ejecución; los reintentos de abajo sólo corren lo que es seguro repetir (una
            // lectura, o un script que no llegó a correr por error de sintaxis)
            let primero = script(l == "jxa", &codigo, timeout_s).await.map(|r| format!("{r}{nota_formulas}"));
            if let (true, Ok(r)) = (l == "applescript", &primero) {
                if r.trim() == "missing value" {
                    if let Some(directo) = lectura_directa(&codigo) {
                        if let Ok(r2) = script(false, &directo, timeout_s).await {
                            if r2.trim() != "missing value" {
                                return Ok(crate::tr!("{r2}\n(leído con `get … of range …` directo: `set x to value of range` da «missing value» en Excel)", "{r2}\n(read with a direct `get … of range …`: `set x to value of range` gives «missing value» in Excel)"));
                            }
                        }
                    }
                }
            }
            match primero {
                // error de SINTAXIS con lo que un modelo suele mandar (medido con el 9B): comillas
                // sobre-escapadas de un JSON o varias órdenes en una línea con «;» → una vez normalizado
                Err(e) if l == "applescript" && (e.to_string().contains("-2741") || e.to_string().contains("-2740")) => match normalizar(&codigo) {
                    Some(n) => script(false, &n, timeout_s).await.map_err(|e2| {
                        anyhow!("{e2}\n{}", crate::i18n::t("(también lo probé normalizado: una orden por línea, sin «;», comillas normales sin barras, y `tell application \"X\"` … `end tell` alrededor)", "(also tried normalized: one statement per line, no «;», plain quotes without backslashes, and `tell application \"X\"` … `end tell` around)"))
                    }),
                    None => Err(e),
                },
                r => r,
            }
        }
        _ => unreachable!("es_comando"),
    }
}

/// Los scripts son para MANEJAR APPS, no una puerta trasera a la terminal: `do shell script`, los
/// eventos crudos («event …») y el puente a Cocoa de JXA (`ObjC`, `$.`, `doShellScript`) correrían
/// comandos FUERA de la caja de `bash` y de sus permisos. Se rechazan y se dice por dónde ir.
pub fn atajo_a_la_terminal(codigo: &str) -> Option<String> {
    let c = codigo.to_lowercase();
    let prohibido = ["do shell script", "doshellscript", "objc.", "objc[", "«event", "<<event", "nstask", "nsapplescript", "$."];
    let hallado = prohibido.iter().find(|p| c.contains(**p))?;
    Some(crate::tr!(
        "NO lo ejecuté: «{hallado}» correría comandos fuera de la caja. Para la terminal usa la herramienta `bash`; los scripts son sólo para manejar apps.",
        "NOT run: «{hallado}» would run commands outside the sandbox. For the terminal use the `bash` tool; scripts are only for driving apps."
    ))
}

/// Funciones de Excel en español → inglés: la propiedad `formula` de AppleScript sólo entiende los
/// nombres en INGLÉS (medido 2026-11-11: el 9B puso «=SUMA(B2:B4)» y la celda quedó sin valor).
const FUNCIONES: &[(&str, &str)] = &[
    ("SUMA", "SUM"), ("PROMEDIO", "AVERAGE"), ("CONTAR", "COUNT"), ("CONTARA", "COUNTA"), ("SI", "IF"),
    ("REDONDEAR", "ROUND"), ("BUSCARV", "VLOOKUP"), ("BUSCARH", "HLOOKUP"), ("CONCATENAR", "CONCATENATE"),
    ("HOY", "TODAY"), ("AHORA", "NOW"), ("SUMAR.SI", "SUMIF"), ("CONTAR.SI", "COUNTIF"), ("Y", "AND"), ("O", "OR"),
    ("ENTERO", "INT"), ("ABS", "ABS"), ("MAX", "MAX"), ("MIN", "MIN"), ("TEXTO", "TEXT"), ("IZQUIERDA", "LEFT"),
    ("DERECHA", "RIGHT"), ("LARGO", "LEN"), ("MAYUSC", "UPPER"), ("MINUSC", "LOWER"), ("SUMAPRODUCTO", "SUMPRODUCT"),
];

/// En cada `set formula of … to "=…"`: funciones en español → inglés y `;` → `,` (el separador de
/// argumentos en inglés). Devuelve el código y lo que cambió (vacío si nada).
pub fn formulas_en_ingles(codigo: &str) -> (String, Vec<String>) {
    let mut cambios = vec![];
    let lineas: Vec<String> = codigo
        .lines()
        .map(|l| {
            let Some(i) = l.find("set formula") else { return l.to_string() };
            let Some(a) = l[i..].find("\"=").map(|a| i + a + 1) else { return l.to_string() };
            let Some(b) = l[a..].find('"').map(|b| a + b) else { return l.to_string() };
            let f = &l[a..b];
            let mut nueva = String::new();
            let mut palabra = String::new();
            let mut en_texto = false;
            for ch in f.chars() {
                if ch == '\'' || ch == '\u{201c}' {
                    en_texto = !en_texto;
                }
                if !en_texto && (ch.is_alphanumeric() || ch == '.') {
                    palabra.push(ch);
                    continue;
                }
                if ch == '(' {
                    if let Some((es, en)) = FUNCIONES.iter().find(|(es, _)| es.eq_ignore_ascii_case(&palabra)) {
                        if !es.eq_ignore_ascii_case(en) {
                            cambios.push(format!("{}→{en}", palabra.to_uppercase()));
                        }
                        palabra = en.to_string();
                    }
                }
                nueva.push_str(&palabra);
                palabra.clear();
                nueva.push(if ch == ';' && !en_texto { ',' } else { ch });
            }
            nueva.push_str(&palabra);
            if f.contains(';') {
                cambios.push(";→,".into());
            }
            format!("{}{nueva}{}", &l[..a], &l[b..])
        })
        .collect();
    cambios.dedup();
    (lineas.join("\n"), cambios)
}

/// `set total to (get) value of range "B5" of active sheet` (+ `return total`) → la lectura directa
/// `get value of range "B5" of active sheet` dentro del mismo `tell`. `None` si no hay tal lectura.
pub fn lectura_directa(codigo: &str) -> Option<String> {
    let mut var = None;
    let mut out = vec![];
    for l in codigo.lines() {
        let t = l.trim();
        if let Some(r) = t.strip_prefix("set ") {
            if let Some((v, expr)) = r.split_once(" to ") {
                let e = expr.trim().strip_prefix("get ").unwrap_or(expr.trim());
                if e.contains(" of range ") && var.is_none() {
                    var = Some(v.trim().to_string());
                    out.push(format!("get {e}"));
                    continue;
                }
            }
        }
        if var.as_deref().is_some_and(|v| t == format!("return {v}") || t == v) {
            continue;
        }
        out.push(l.to_string());
    }
    var.map(|_| out.join("\n"))
}

/// Lo que un modelo suele romper en AppleScript, arreglado (o `None` si no hay nada que arreglar):
///   - `\"` fuera de lugar (copiado de un JSON) → `"`;
///   - varias órdenes en una línea con `;` (AppleScript no lo admite) → una por línea, sin tocar los
///     `;` que están DENTRO de comillas;
///   - `tell application "X" to a; b; c` → `tell application "X"` / a / b / c / `end tell`.
pub fn normalizar(codigo: &str) -> Option<String> {
    let mut c = codigo.replace("\\\"", "\"");
    // `;` fuera de las comillas → salto de línea
    let mut fuera = String::with_capacity(c.len());
    let (mut en_cadena, mut escape) = (false, false);
    for ch in c.chars() {
        match ch {
            '\\' if en_cadena => {
                escape = !escape;
                fuera.push(ch);
                continue;
            }
            '"' if !escape => en_cadena = !en_cadena,
            ';' if !en_cadena => {
                fuera.push('\n');
                escape = false;
                continue;
            }
            _ => {}
        }
        escape = false;
        fuera.push(ch);
    }
    let lineas: Vec<String> = fuera.lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect();
    // `tell application "X" to <orden>` seguido de más órdenes → bloque tell … end tell
    if lineas.len() > 1 {
        if let Some(r) = lineas[0].strip_prefix("tell application ") {
            if let Some((app, primera)) = r.split_once(" to ") {
                let mut b = vec![format!("tell application {app}"), primera.to_string()];
                b.extend(lineas[1..].iter().filter(|l| l.as_str() != "end tell").cloned());
                b.push("end tell".into());
                c = b.join("\n");
                return (c != codigo).then_some(c);
            }
        }
    }
    c = lineas.join("\n");
    (c != codigo.trim()).then_some(c)
}

/// Corre un script con `osascript` (el código por stdin: sin comillas que escapar).
pub async fn script(jxa: bool, codigo: &str, timeout_s: u64) -> Result<String> {
    let mut c = tokio::process::Command::new("/usr/bin/osascript");
    if jxa {
        c.args(["-l", "JavaScript"]);
    }
    c.stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped()).kill_on_drop(true);
    let mut hijo = c.spawn()?;
    if let Some(mut e) = hijo.stdin.take() {
        e.write_all(codigo.as_bytes()).await?;
    }
    let salida = tokio::time::timeout(Duration::from_secs(timeout_s), hijo.wait_with_output())
        .await
        .map_err(|_| anyhow!("{}", crate::tr!("el script no terminó en {timeout_s} s (¿la app espera un diálogo?)", "the script did not finish in {timeout_s} s (is the app waiting on a dialog?)")))??;
    let out = String::from_utf8_lossy(&salida.stdout).trim().to_string();
    if salida.status.success() {
        return Ok(if out.is_empty() { crate::i18n::t("✓ hecho (el script no devolvió nada; para LEER un valor, la última orden debe ser la expresión SIN `set x to`, p. ej. tell application \"Microsoft Excel\" to get value of range \"B6\" of active sheet)", "✓ done (the script returned nothing; to READ a value, the last statement must be the expression WITHOUT `set x to`, e.g. tell application \"Microsoft Excel\" to get value of range \"B6\" of active sheet)").into() } else { out });
    }
    let err = String::from_utf8_lossy(&salida.stderr).trim().to_string();
    // -1743: macOS no dio permiso de Automatización; -600: la app no está abierta
    let pista = if err.contains("-1743") {
        crate::i18n::t(" → macOS no dio permiso para controlar esa app: el usuario debe permitirlo en Ajustes del Sistema → Privacidad y seguridad → Automatización.", " → macOS did not allow controlling that app: the user must allow it in System Settings → Privacy & Security → Automation.")
    } else if err.contains("-1728") || err.contains("-1700") {
        crate::i18n::t(" → ese objeto o propiedad no existe así: mira [\"diccionario\",\"App\",\"palabra\"] o [\"recetas\",\"App\"].", " → that object or property does not exist like that: check [\"diccionario\",\"App\",\"word\"] or [\"recetas\",\"App\"].")
    } else {
        ""
    };
    Err(anyhow!("{err}{pista}"))
}

/// El `.app` de una app: por nombre exacto en las carpetas de apps, o por su nombre MOSTRADO (los
/// nombres traducidos: «Calculadora» es Calculator.app) con Spotlight.
pub fn ruta_app(nombre: &str) -> Option<PathBuf> {
    let n = nombre.trim().trim_end_matches(".app");
    if n.starts_with('/') {
        return Path::new(n).exists().then(|| PathBuf::from(n)).or_else(|| Some(PathBuf::from(format!("{n}.app"))).filter(|p| p.exists()));
    }
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    for d in [PathBuf::from("/Applications"), PathBuf::from("/System/Applications"), PathBuf::from("/System/Applications/Utilities"), home.join("Applications")] {
        let p = d.join(format!("{n}.app"));
        if p.exists() {
            return Some(p);
        }
    }
    let q = format!("kMDItemContentType == 'com.apple.application-bundle' && (kMDItemDisplayName == '{0}' || kMDItemDisplayName == '{0}.app')", n.replace('\'', ""));
    let o = std::process::Command::new("/usr/bin/mdfind").arg(q).output().ok()?;
    String::from_utf8_lossy(&o.stdout).lines().map(PathBuf::from).find(|p| p.exists())
}

/// Lo que la app declara saber hacer (su diccionario de scripts), filtrado por palabras: comandos
/// y clases con sus propiedades. El de Word pesa 842 KB: sin filtro sólo van los nombres.
pub async fn diccionario(app: &str, palabras: &[String]) -> Result<String> {
    let ruta = ruta_app(app).ok_or_else(|| anyhow!("{}", crate::tr!("no encuentro la app «{app}»: [\"list-apps\"] da los nombres", "app «{app}» not found: [\"list-apps\"] gives the names")))?;
    let o = tokio::process::Command::new("/usr/bin/sdef").arg(&ruta).output().await?;
    if !o.status.success() || o.stdout.is_empty() {
        return Ok(crate::tr!(
            "«{app}» no tiene diccionario de scripts: manéjala con la accesibilidad ([\"snapshot\",\"--app\",\"{app}\",\"-i\",\"--compact\"], click, type, press, menús).",
            "«{app}» has no scripting dictionary: drive it through accessibility ([\"snapshot\",\"--app\",\"{app}\",\"-i\",\"--compact\"], click, type, press, menus)."
        ));
    }
    Ok(resumir_sdef(&String::from_utf8_lossy(&o.stdout), &ruta, palabras))
}

/// Un elemento del diccionario: comando o clase, con su descripción y sus propiedades/parámetros.
struct Entrada {
    clase: bool,
    nombre: String,
    desc: String,
    hijos: Vec<String>,
}

/// Lee las etiquetas del sdef sin dependencias (su XML es regular: atributos con comillas dobles).
fn resumir_sdef(xml: &str, ruta: &Path, palabras: &[String]) -> String {
    let mut entradas: Vec<Entrada> = vec![];
    let mut dentro = false;
    let mut i = 0;
    while let Some(a) = xml[i..].find('<') {
        let ini = i + a;
        let Some(b) = xml[ini..].find('>') else { break };
        let tag = &xml[ini + 1..ini + b];
        i = ini + b + 1;
        let cierre = tag.starts_with('/');
        let nombre_tag = tag.trim_start_matches('/').split(|c: char| c.is_whitespace() || c == '/').next().unwrap_or("");
        let attr = |k: &str| -> String {
            let pat = format!("{k}=\"");
            tag.find(&pat).and_then(|p| tag[p + pat.len()..].split('"').next()).map(desescapar).unwrap_or_default()
        };
        match nombre_tag {
            "command" | "class" | "class-extension" if !cierre => {
                let n = if nombre_tag == "class-extension" { attr("extends") } else { attr("name") };
                entradas.push(Entrada { clase: nombre_tag != "command", nombre: n, desc: attr("description"), hijos: vec![] });
                dentro = !tag.ends_with('/');
            }
            "command" | "class" | "class-extension" => dentro = false,
            "property" | "parameter" | "direct-parameter" | "element" | "responds-to" if dentro && !cierre => {
                if let Some(e) = entradas.last_mut() {
                    let n = match nombre_tag {
                        "direct-parameter" => format!("(directo: {})", attr("type")),
                        "element" => format!("[{}]", attr("type")),
                        "responds-to" => format!("→{}", attr("command").if_empty(attr("name"))),
                        _ => {
                            let t = attr("type");
                            if t.is_empty() { attr("name") } else { format!("{}:{t}", attr("name")) }
                        }
                    };
                    if !n.is_empty() {
                        e.hijos.push(n);
                    }
                }
            }
            _ => {}
        }
    }
    let app = ruta.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let cmds = entradas.iter().filter(|e| !e.clase).count();
    let clases = entradas.iter().filter(|e| e.clase).count();
    let p: Vec<String> = palabras.iter().map(|w| w.to_lowercase()).filter(|w| !w.is_empty()).collect();
    if p.is_empty() {
        // sin filtro: sólo los nombres (el completo de Word no cabe en ningún contexto)
        let nombres = |clase: bool| -> String {
            let mut v: Vec<&str> = entradas.iter().filter(|e| e.clase == clase).map(|e| e.nombre.as_str()).filter(|n| !n.is_empty()).collect();
            v.sort_unstable();
            v.dedup();
            let total = v.len();
            let mut s = v.into_iter().take(180).collect::<Vec<_>>().join(", ");
            if total > 180 {
                s.push_str(&format!(" … (+{})", total - 180));
            }
            s
        };
        return crate::tr!(
            "{app}: {cmds} comandos y {clases} clases. Pide el detalle con palabras: [\"diccionario\",\"{app}\",\"document\",\"save\"].\nCOMANDOS: {}\nCLASES: {}",
            "{app}: {cmds} commands and {clases} classes. Ask for detail with words: [\"diccionario\",\"{app}\",\"document\",\"save\"].\nCOMMANDS: {}\nCLASSES: {}",
            nombres(false),
            nombres(true)
        );
    }
    let mut out = String::new();
    let mut n = 0;
    for e in &entradas {
        let texto = format!("{} {} {}", e.nombre, e.desc, e.hijos.join(" ")).to_lowercase();
        if !p.iter().all(|w| texto.contains(w.as_str())) {
            continue;
        }
        n += 1;
        if out.len() > 6000 {
            continue;
        }
        let hijos: String = e.hijos.iter().take(40).cloned().collect::<Vec<_>>().join(", ");
        out.push_str(&format!("{} «{}»: {}{}\n", if e.clase { "clase" } else { "comando" }, e.nombre, e.desc.chars().take(160).collect::<String>(), if hijos.is_empty() { String::new() } else { format!(" · {hijos}") }));
    }
    if n == 0 {
        return crate::tr!("nada en el diccionario de {app} con {p:?}: prueba otras palabras (en inglés: document, save, range, slide…).", "nothing in {app}'s dictionary with {p:?}: try other words (document, save, range, slide…).");
    }
    format!("{app} · {n} coincidencia(s) con {p:?}{}\n{out}", if out.len() > 6000 { " (las primeras)" } else { "" })
}

trait SiVacio {
    fn if_empty(self, otro: String) -> String;
}

impl SiVacio for String {
    fn if_empty(self, otro: String) -> String {
        if self.is_empty() { otro } else { self }
    }
}

fn desescapar(s: &str) -> String {
    s.replace("&quot;", "\"").replace("&apos;", "'").replace("&lt;", "<").replace("&gt;", ">").replace("&amp;", "&")
}

/// Recetas PROBADAS en esta Mac (2026-11-11). Sólo van las que se ejecutaron de verdad; para las
/// demás apps, el diccionario.
/// Extensiones de documentos que un modelo dice «guardé/creé»: si no están en disco, mintió.
const DOCUMENTOS: [&str; 14] = ["docx", "xlsx", "pptx", "pdf", "doc", "xls", "ppt", "csv", "odt", "ods", "odp", "rtf", "pages", "numbers"];

/// Los documentos que la respuesta dice haber GUARDADO/CREADO y no existen (relativos = dentro del
/// proyecto, buscados hasta 3 niveles). Sin un verbo de entrega no cuenta («podrías crear un PDF»
/// tampoco: sólo pasado o afirmación).
pub fn documentos_ausentes(texto: &str, root: &Path) -> Vec<String> {
    let t = texto.to_lowercase();
    let entrega = ["guardé", "guardado", "guardada", "se guardó", "creé", "he creado", "creado", "exporté", "exportado", "generé", "generado", "escribí", "saved", "created", "exported", "wrote", "generated"]; // juez-idioma: datos (lo que dice una respuesta, en los dos idiomas)
    if !entrega.iter().any(|v| t.contains(v)) {
        return vec![];
    }
    let mut faltan = vec![];
    for crudo in texto.split(|c: char| c.is_whitespace() || "`\"'()[]{},;«»*|<>".contains(c)) {
        let tok = crudo.trim_end_matches(['.', ':', '!', '?']);
        let Some((_, ext)) = tok.rsplit_once('.') else { continue };
        if !DOCUMENTOS.contains(&ext.to_lowercase().as_str()) || tok.len() <= ext.len() + 1 || tok.contains("://") {
            continue;
        }
        let p = Path::new(tok);
        let existe = if p.is_absolute() {
            p.exists()
        } else if tok.contains('/') {
            root.join(tok).exists()
        } else {
            existe_en(root, tok, 3)
        };
        if !existe && !faltan.contains(&tok.to_string()) {
            faltan.push(tok.to_string());
        }
    }
    faltan
}

fn existe_en(dir: &Path, nombre: &str, hondo: u32) -> bool {
    if dir.join(nombre).exists() {
        return true;
    }
    hondo > 0
        && std::fs::read_dir(dir).into_iter().flatten().flatten().any(|e| {
            let n = e.file_name();
            let n = n.to_string_lossy();
            e.path().is_dir() && !n.starts_with('.') && n != "node_modules" && n != "target" && existe_en(&e.path(), nombre, hondo - 1)
        })
}

/// Las recetas de la app que se NOMBRE en el texto (para `skill`, que el 9B usa primero).
pub fn recetas_para(texto: &str) -> Option<&'static str> {
    let a = texto.to_lowercase();
    RECETAS.iter().find(|(n, ..)| a.contains(n)).map(|(_, es, en)| crate::i18n::t(es, en))
}

pub fn recetas(app: &str) -> String {
    match recetas_para(app) {
        Some(r) => r.to_string(),
        None => {
            let conocidas: Vec<&str> = RECETAS.iter().map(|(n, ..)| *n).collect();
            crate::tr!(
                "sin recetas probadas para «{app}» (hay para: {}). Usa [\"diccionario\",\"{app}\",\"palabra\"] para ver lo que sabe hacer, o la accesibilidad.",
                "no tested recipes for «{app}» (there are for: {}). Use [\"diccionario\",\"{app}\",\"word\"] to see what it can do, or accessibility.",
                conocidas.join(", ")
            )
        }
    }
}

/// (clave en minúsculas, recetas). Rutas: SIEMPRE absolutas (POSIX).
/// (clave en minúsculas, recetas). El código va TAL CUAL (medido: con las comillas escapadas de un
/// JSON, el 9B copió las barras y AppleScript dio error de sintaxis). Rutas: SIEMPRE absolutas.
/// (clave, español, inglés): `guardar`/`abrir` son acciones de la herramienta y no se traducen.
const RECETAS: &[(&str, &str, &str)] = &[
    ("word", include_str!("../recursos/recetas/word.es.txt"), include_str!("../recursos/recetas/word.en.txt")),
    ("excel", include_str!("../recursos/recetas/excel.es.txt"), include_str!("../recursos/recetas/excel.en.txt")),
    ("powerpoint", include_str!("../recursos/recetas/powerpoint.es.txt"), include_str!("../recursos/recetas/powerpoint.en.txt")),
];

#[cfg(test)]
mod pruebas {
    use super::*;

    #[test]
    fn un_script_que_solo_lee_es_observar() {
        for s in ["output volume of (get volume settings)", "tell application \"System Events\" to tell appearance preferences to get dark mode", "tell application \"Finder\" to get name of startup disk", "Application('Music').currentTrack.name()"] {
            assert!(script_solo_lee(s), "{s}");
        }
        for s in ["set volume output volume 30", "tell application \"System Events\" to key code 144", "tell application \"Safari\" to activate", "tell application \"Word\" to save active document", "tell application \"System Events\" to tell appearance preferences to set dark mode to true", "Application('Music').volume = 30", ""] {
            assert!(!script_solo_lee(s), "{s}");
        }
    }

    /// La respuesta REAL del 9B (precios.xlsx no existía) se atrapa; lo que sí existe, lo que sólo
    /// se propone y las URL no.
    #[test]
    fn documentos_prometidos_que_no_existen() {
        let raiz = gotv_engine::prueba::CarpetaPrueba::nueva("gotv-entrega");
        std::fs::create_dir_all(raiz.join("docs")).unwrap();
        std::fs::write(raiz.join("docs/carta.pdf"), b"%PDF").unwrap();
        let real = "El total calculado por Excel es **20500**.\n\nEl archivo se ha guardado correctamente como `precios.xlsx` en el proyecto.";
        assert_eq!(documentos_ausentes(real, &raiz), vec!["precios.xlsx".to_string()]);
        assert!(documentos_ausentes("He guardado la carta en carta.pdf.", &raiz).is_empty(), "existe (en docs/)");
        assert!(documentos_ausentes("Puedo crear un informe.pdf si quieres", &raiz).is_empty(), "sólo propone");
        assert!(documentos_ausentes("Creé el enlace https://x.com/a.pdf", &raiz).is_empty(), "una URL no es un archivo del proyecto");
        assert_eq!(documentos_ausentes("Saved to reports/q3.docx and q3.pdf", &raiz), vec!["reports/q3.docx".to_string(), "q3.pdf".to_string()]);
    }

    /// `guardar` no escribe fuera del proyecto (la ruta que inventó el 9B, y una carpeta real del
    /// usuario); se comprueba ANTES de tocar la app.
    #[tokio::test]
    async fn guardar_no_sale_del_proyecto() {
        let root = Path::new("/Users/x/Documents/proyectos/agente-local/target/prueba-raiz");
        for fuera in ["/Users/void/carta.pdf", "/Users/x/Desktop/carta.pdf", "../carta.pdf"] {
            let e = ejecutar(&[String::from("guardar"), "Microsoft Word".into(), fuera.into()], root, 5).await.unwrap_err().to_string();
            assert!(e.contains("fuera del proyecto") || e.contains("outside the project"), "{fuera}: {e}");
        }
    }

    /// Lo que el 9B mandó de verdad (2026-11-11): comillas sobre-escapadas y órdenes con «;».
    /// La fórmula EXACTA del 9B («=SUMA(B2:B4)») y otras comunes en español.
    #[test]
    fn las_formulas_van_en_ingles() {
        let (c, cambios) = formulas_en_ingles("    set formula of range \"B5\" of active sheet to \"=SUMA(B2:B4)\"");
        assert_eq!(c, "    set formula of range \"B5\" of active sheet to \"=SUM(B2:B4)\"");
        assert_eq!(cambios, vec!["SUMA→SUM"]);
        let (c, _) = formulas_en_ingles("set formula of range \"C1\" of active sheet to \"=SI(PROMEDIO(A1:A3)>5;REDONDEAR(A1;2);0)\"");
        assert_eq!(c, "set formula of range \"C1\" of active sheet to \"=IF(AVERAGE(A1:A3)>5,ROUND(A1,2),0)\"");
        let (c, cambios) = formulas_en_ingles("set formula of range \"B4\" of active sheet to \"=B3*2\"");
        assert!(cambios.is_empty() && c.ends_with("\"=B3*2\""), "sin funciones: igual");
        assert!(formulas_en_ingles("set value of range \"A1\" of active sheet to \"SUMA(\"").1.is_empty(), "sólo las fórmulas");
    }

    /// El código EXACTO del 9B que dio «missing value» → la lectura directa que sí da el valor.
    #[test]
    fn la_lectura_de_excel_se_hace_directa() {
        let del_9b = "tell application \"Microsoft Excel\"\nset total to get value of range \"B5\" of active sheet\nreturn total\nend tell";
        assert_eq!(lectura_directa(del_9b).unwrap(), "tell application \"Microsoft Excel\"\nget value of range \"B5\" of active sheet\nend tell");
        assert!(lectura_directa("tell application \"Microsoft Excel\" to make new workbook").is_none(), "sin lectura: nada que reintentar");
    }

    #[test]
    fn normaliza_lo_que_manda_el_9b() {
        let n = normalizar("tell application \"Microsoft Word\" to activate; make new document; set content of text object of active document to \"Hola; mundo\"").unwrap();
        assert_eq!(n, "tell application \"Microsoft Word\"\nactivate\nmake new document\nset content of text object of active document to \"Hola; mundo\"\nend tell", "el «;» de dentro de la cadena queda");
        let n = normalizar("tell application \\\"Microsoft Word\\\"\nactivate\nend tell").unwrap();
        assert_eq!(n, "tell application \"Microsoft Word\"\nactivate\nend tell");
        assert!(normalizar("tell application \"Microsoft Word\"\nactivate\nend tell").is_none(), "lo correcto no se toca");
    }

    #[test]
    fn los_scripts_no_son_una_puerta_a_la_terminal() {
        for malo in [
            "do shell script \"rm -rf ~\"",
            "tell application \"Finder\" to DO SHELL SCRIPT \"ls\"",
            "var app = Application.currentApplication(); app.includeStandardAdditions = true; app.doShellScript('ls')",
            "ObjC.import('Foundation'); $.NSTask.alloc",
            "«event sysoexec» \"ls\"",
        ] {
            assert!(atajo_a_la_terminal(malo).is_some(), "{malo}");
        }
        for bueno in [
            "tell application \"Microsoft Word\" to make new document",
            "Application('Microsoft Excel').activeSheet.ranges['B3'].value()",
            "tell application \"Microsoft Word\" to save as active document file name \"/tmp/a.pdf\" file format format PDF",
        ] {
            assert!(atajo_a_la_terminal(bueno).is_none(), "{bueno}");
        }
    }

    fn a(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    /// O1 de punta a punta con Word, Excel y PowerPoint REALES, con los MISMOS comandos de las
    /// recetas y destino FUERA de su caja de arena: crear, guardar (.docx/.pdf, .xlsx, .pptx/.pdf),
    /// volver a abrir y leer; guardar encima deja respaldo. Abre las apps en pantalla.
    /// `cargo test -p gotv-core --release -- --ignored --nocapture office_de_punta_a_punta`
    #[tokio::test]
    #[ignore]
    async fn office_de_punta_a_punta() {
        let raiz = gotv_engine::prueba::CarpetaPrueba::nueva("gotv-office");
        let d = |n: &str| raiz.join(n).to_string_lossy().into_owned();
        // Word
        ejecutar(&a(&["applescript", "tell application \"Microsoft Word\"\nactivate\nmake new document\nset content of text object of active document to \"Carta de GOTV\" & return & \"Segundo párrafo.\"\nend tell"]), &raiz, 120).await.unwrap();
        println!("{}", ejecutar(&a(&["guardar", "Microsoft Word", &d("carta.docx")]), &raiz, 120).await.unwrap());
        println!("{}", ejecutar(&a(&["guardar", "Microsoft Word", &d("carta.pdf")]), &raiz, 120).await.unwrap());
        ejecutar(&a(&["applescript", "tell application \"Microsoft Word\" to close active document saving no"]), &raiz, 60).await.unwrap();
        println!("{}", ejecutar(&a(&["abrir", "Microsoft Word", &d("carta.docx")]), &raiz, 120).await.unwrap());
        let texto = ejecutar(&a(&["applescript", "tell application \"Microsoft Word\" to get content of text object of active document"]), &raiz, 60).await.unwrap();
        assert!(texto.contains("Carta de GOTV"), "Word reabre lo guardado: {texto}");
        ejecutar(&a(&["applescript", "tell application \"Microsoft Word\" to close active document saving no"]), &raiz, 60).await.unwrap();
        // Excel
        ejecutar(&a(&["applescript", "tell application \"Microsoft Excel\"\nactivate\nmake new workbook\nset value of range \"B3\" of active sheet to 42\nset formula of range \"B4\" of active sheet to \"=B3*2\"\nend tell"]), &raiz, 120).await.unwrap();
        println!("{}", ejecutar(&a(&["guardar", "Microsoft Excel", &d("libro.xlsx")]), &raiz, 120).await.unwrap());
        // guardar ENCIMA: el anterior queda de respaldo
        let r = ejecutar(&a(&["guardar", "Microsoft Excel", &d("libro.xlsx")]), &raiz, 120).await.unwrap();
        assert!(r.contains("respaldo"), "{r}");
        // OTRO libro con el mismo nombre abierto (la corrida real: -50): se explica, no se cuelga
        ejecutar(&a(&["applescript", "tell application \"Microsoft Excel\" to make new workbook"]), &raiz, 60).await.unwrap();
        let e = ejecutar(&a(&["guardar", "Microsoft Excel", &d("libro.xlsx")]), &raiz, 60).await.unwrap_err().to_string();
        assert!(e.contains("OTRO") || e.contains("ANOTHER"), "{e}");
        ejecutar(&a(&["applescript", "tell application \"Microsoft Excel\" to close active workbook saving no"]), &raiz, 60).await.unwrap();
        ejecutar(&a(&["applescript", "tell application \"Microsoft Excel\" to close active workbook saving no"]), &raiz, 60).await.unwrap();
        println!("{}", ejecutar(&a(&["abrir", "Microsoft Excel", &d("libro.xlsx")]), &raiz, 120).await.unwrap());
        let v = ejecutar(&a(&["applescript", "tell application \"Microsoft Excel\" to get value of range \"B4\" of active sheet"]), &raiz, 60).await.unwrap();
        assert_eq!(v, "84.0", "la fórmula sobrevive a guardar y abrir");
        ejecutar(&a(&["applescript", "tell application \"Microsoft Excel\" to close active workbook saving no"]), &raiz, 60).await.unwrap();
        // PowerPoint
        ejecutar(&a(&["applescript", "tell application \"Microsoft PowerPoint\"\nactivate\nmake new presentation\nset s to make new slide at end of active presentation with properties {layout:slide layout title slide}\nset content of text range of text frame of shape 1 of s to \"Informe\"\nset s2 to make new slide at end of active presentation with properties {layout:slide layout text slide}\nset content of text range of text frame of shape 1 of s2 to \"Puntos\"\nend tell"]), &raiz, 150).await.unwrap();
        println!("{}", ejecutar(&a(&["guardar", "Microsoft PowerPoint", &d("pres.pptx")]), &raiz, 120).await.unwrap());
        println!("{}", ejecutar(&a(&["guardar", "Microsoft PowerPoint", &d("pres.pdf")]), &raiz, 120).await.unwrap());
        ejecutar(&a(&["applescript", "tell application \"Microsoft PowerPoint\" to close active presentation saving no"]), &raiz, 60).await.unwrap();
        println!("{}", ejecutar(&a(&["abrir", "Microsoft PowerPoint", &d("pres.pptx")]), &raiz, 120).await.unwrap());
        let n = ejecutar(&a(&["applescript", "tell application \"Microsoft PowerPoint\" to count slides of active presentation"]), &raiz, 60).await.unwrap();
        assert_eq!(n, "2");
        ejecutar(&a(&["applescript", "tell application \"Microsoft PowerPoint\" to close active presentation saving no"]), &raiz, 60).await.unwrap();
        // los archivos son lo que dicen ser
        for (f, cab) in [("carta.docx", &b"PK"[..]), ("carta.pdf", b"%PDF"), ("libro.xlsx", b"PK"), ("pres.pptx", b"PK"), ("pres.pdf", b"%PDF")] {
            let b = std::fs::read(raiz.join(f)).unwrap();
            assert!(b.starts_with(cab), "{f}");
        }
        // un formato que la app no guarda: error que dice cuáles sí
        let e = ejecutar(&a(&["guardar", "Microsoft Word", &d("x.xlsx")]), &raiz, 30).await.unwrap_err().to_string();
        assert!(e.contains("docx"), "{e}");
    }

    /// Con las apps de ESTA Mac (sdef del sistema): Notas tiene diccionario, el filtro funciona, y un
    /// nombre traducido («Calculadora») se encuentra. `cargo test -p gotv-core -- --ignored diccionario_real`
    #[tokio::test]
    #[ignore]
    async fn diccionario_real() {
        assert!(ruta_app("Calculadora").is_some_and(|p| p.ends_with("Calculator.app")), "nombre traducido");
        let notas = diccionario("Notes", &[]).await.unwrap();
        println!("{notas}");
        assert!(notas.contains("COMANDOS") || notas.contains("COMMANDS"));
        let w = diccionario("Microsoft Word", &["document".into(), "save".into()]).await.unwrap();
        println!("{}", &w[..w.len().min(1500)]);
        assert!(w.contains("save"), "Word: save en el filtro");
    }
}
