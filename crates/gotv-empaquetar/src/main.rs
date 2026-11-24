//! Arma «GOD OF THE VOID.app» (macOS) desde el código, sin tauri-cli:
//!
//!     cargo run -p gotv-empaquetar              # deja la app en dist/
//!     cargo run -p gotv-empaquetar -- <destino>
//!
//! Compila en release la app, el CLI, el proceso de DuckDB y los dos ayudantes de terceros, los mete
//! en el paquete con los kernels Metal de MLX, la extensión de Chrome, las licencias de terceros, el
//! ícono y el Info.plist (inglés de base + `es.lproj`), y lo firma. Reemplaza a
//! `herramientas/empaquetar.sh` (2026-11-20): GOTV es Rust de punta a punta, también al armarse.
//! Sin dependencias: la biblioteca estándar y lo que trae macOS (sips, iconutil, codesign, security).

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::{env, fs};

/// El par ES/EN de los mensajes (el juez de idioma lo reconoce): español si `LANG` empieza por «es».
macro_rules! tr {
    ($es:literal, $en:literal) => {
        if espanol() { format!($es) } else { format!($en) }
    };
}

fn espanol() -> bool {
    env::var("LANG").is_ok_and(|l| l.starts_with("es"))
}

const APP: &str = "GOD OF THE VOID.app";
/// MLX compila sus kernels Metal para el macOS que se le pida; sin un mínimo salen para el de la Mac
/// que compila y en una más vieja no cargan (medido en una Air con macOS 26.6). Lo fija
/// `.cargo/config.toml` (MACOSX_DEPLOYMENT_TARGET); aquí sólo se comprueba.
const MACOS_MINIMO: &str = "14.0";
/// Los binarios propios que viajan dentro: la app, el CLI (`gotv setup …`) y DuckDB en su proceso
/// aparte (una sola copia de su C++; la app lo lanza sólo al usarlo).
const PROPIOS: &[&str] = &["gotv-gui", "gotv", "gotv-duckdb"];
/// Licencias y avisos de terceros que viajan con la distribución (Apache-2.0 §4, MIT): carpeta → archivos.
const LICENCIAS: &[(&str, &[&str])] = &[
    ("agent-desktop", &["terceros/agent-desktop/LICENSE", "terceros/agent-desktop/NOTICE.md"]),
    ("agent-browser", &["terceros/agent-browser/LICENSE", "terceros/agent-browser/NOTICE.md"]),
    ("codex-seatbelt", &["terceros/codex-seatbelt/LICENSE", "terceros/codex-seatbelt/NOTICE.md"]),
    ("mlx-sys", &["terceros/mlx-sys/LICENSE-MIT", "terceros/mlx-sys/LICENSE-APACHE", "terceros/mlx-sys/PARCHE-GOTV.md"]),
    ("mlx-c", &["terceros/mlx-sys/src/mlx-c/LICENSE"]),
    // los pesos de Silero VAD van DENTRO del binario (include_bytes!): su licencia viaja con él
    ("silero-vad", &["crates/gotv-sonido/recursos/LICENCIA-silero-vad.txt"]),
    ("gotv", &["LICENSE", "NOTICE"]),
];

fn main() -> ExitCode {
    match armar() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("✗ {e}");
            ExitCode::FAILURE
        }
    }
}

fn armar() -> Result<(), String> {
    let raiz = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let raiz = raiz.canonicalize().map_err(|e| format!("{}: {e}", raiz.display()))?;
    env::set_current_dir(&raiz).map_err(|e| e.to_string())?;
    let destino = env::args().nth(1).unwrap_or_else(|| "dist".into());
    let app = Path::new(&destino).join(APP);
    // `GOTV_FIRMA=-` = la app que se COMPARTE: firma ad hoc y nada de quien la armó dentro.
    let publicar = env::var("GOTV_FIRMA").is_ok_and(|f| f == "-");
    let hogar = env::var("HOME").unwrap_or_default();
    let target = env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| if publicar { raiz.join("target/publicar") } else { "target".into() });
    let release = target.join("release");
    let entorno = if publicar { entorno_publico(&raiz, &hogar, &target) } else { Vec::new() };

    // 1 · compilar
    cargo(&["build", "--release", "-p", "gotv-gui", "-p", "gotv-cli", "-p", "gotv-duckdb"], &entorno)?;
    let mlx = build_de_mlx(&release)?;
    // control nativo del Mac: el ayudante del portapapeles de agent-desktop (Apache-2.0) va JUNTO al
    // ejecutable (lo busca ahí); ver terceros/agent-desktop/NOTICE.md
    cargo(&["build", "--release", "-p", "agent-desktop-macos", "--bin", "agent-desktop-macos-helper"], &entorno)?;
    // navegador propio: agent-browser (Apache-2.0) desde nuestra copia, en su propio workspace
    cargo(&["build", "--release", "--manifest-path", "terceros/agent-browser/cli/Cargo.toml"], &entorno)?;
    let navegador = match (env::var_os("CARGO_TARGET_DIR"), publicar) {
        (Some(t), _) => PathBuf::from(t),
        (None, true) => target.clone(),
        (None, false) => "terceros/agent-browser/cli/target".into(),
    }
    .join("release/agent-browser");

    // 2 · el paquete
    if app.exists() {
        fs::remove_dir_all(&app).map_err(|e| format!("{}: {e}", app.display()))?;
    }
    let (macos, recursos) = (app.join("Contents/MacOS"), app.join("Contents/Resources"));
    crear(&macos)?;
    crear(&recursos)?;
    for b in PROPIOS {
        copiar(&release.join(b), &macos.join(b))?;
    }
    copiar(&release.join("agent-desktop-macos-helper"), &macos.join("agent-desktop-macos-helper"))?;
    copiar(&navegador, &macos.join("agent-browser"))?;
    // la extensión de Chrome viaja dentro (se carga con «Cargar descomprimida»)
    copiar_carpeta(Path::new("extension-chrome"), &recursos.join("extension-chrome"))?;
    // sus guías (`skills get core`): agent-browser las busca en ../skill-data desde su ejecutable
    copiar_carpeta(Path::new("terceros/agent-browser/skill-data"), &app.join("Contents/skill-data"))?;

    // 3 · los kernels Metal de MLX. El binario trae grabada la ruta de ESTA Mac (target/mlx-metal);
    // en otra Mac no existe: GOTV fija esta copia al arrancar.
    let gotv = fs::read(release.join("gotv")).map_err(|e| format!("{}: {e}", release.join("gotv").display()))?;
    let metallib = ruta_del_metallib(&gotv).ok_or_else(|| {
        tr!(
            "no encontré mlx.metallib: el núcleo nativo no funcionaría fuera de esta Mac",
            "mlx.metallib not found: the native core would not run outside this Mac"
        )
    })?;
    let kernels = fs::read(&metallib).map_err(|e| format!("{metallib}: {e}"))?;
    // se comprueba DENTRO del archivo (medido: el registro de compilación decía 14.0 y el metallib era de 27)
    if !contiene(&kernels, b"apple-macosx14") {
        return Err(tr!(
            "{metallib} no está compilado para macOS 14 (no correría en Macs más viejas)",
            "{metallib} is not built for macOS 14 (it would not run on older Macs)"
        ));
    }
    copiar(Path::new(&metallib), &recursos.join("mlx.metallib"))?;

    // 4 · licencias de terceros, con las de lo que MLX bajó al compilarse (mlx, fmt, gguflib…)
    let licencias = recursos.join("licencias");
    for (carpeta, archivos) in LICENCIAS {
        crear(&licencias.join(carpeta))?;
        for a in *archivos {
            let a = Path::new(a);
            copiar(a, &licencias.join(carpeta).join(a.file_name().unwrap_or_default()))?;
        }
    }
    for (nombre, licencia) in licencias_de_mlx(&mlx) {
        crear(&licencias.join(&nombre))?;
        copiar(&licencia, &licencias.join(&nombre).join("LICENSE"))?;
    }

    // 5 · ícono (todos los tamaños que pide macOS, desde el PNG de 512) e Info.plist
    icono(Path::new("crates/gotv-gui/icons/icon.png"), &recursos.join("gotv.icns"))?;
    let version = version_del_workspace(&leer("Cargo.toml")?)
        .ok_or_else(|| tr!("Cargo.toml sin «version»", "Cargo.toml has no «version»"))?;
    escribir(&app.join("Contents/Info.plist"), &info_plist(&version))?;
    crear(&recursos.join("es.lproj"))?;
    escribir(&recursos.join("es.lproj/InfoPlist.strings"), &plist_en_espanol())?;
    crear(&recursos.join("en.lproj"))?;
    escribir(&recursos.join("en.lproj/InfoPlist.strings"), "")?;

    // 5b · el juez de la app pública: NINGÚN archivo (ejecutables, kernels, licencias, extensión, plist)
    // lleva la carpeta personal de quien la armó, una llave con forma real ni su token de Hugging Face
    if publicar {
        juzgar_lo_publico(&app, &hogar)?;
    }

    // 6 · firma ESTABLE: con «ad hoc» (-) cada compilación es otra app para macOS y el permiso de
    // Documentos se pierde en cada versión (medido 2026-11-09). Con una identidad de firma real el
    // permiso sobrevive. GOTV_FIRMA="Apple Development: …" la fija; si no, la primera de codesign.
    firmar(&app);
    let mb = peso(&app) as f64 / 1_048_576.0;
    println!("✅ {} · {mb:.0} MB", app.display());

    // 7 · EL MOTOR SIN LA APP: para quien no quiere la ventana, la CLI `gotv` con sus kernels Metal al
    // lado (los busca junto a sí) y las licencias. Imagen, vídeo, música, voz, oído, separar pistas y
    // `gotv serve` (API de OpenAI y de Ollama), en un .tar.gz. Sale del MISMO binario de la app.
    let motor = armar_motor(Path::new(&destino), &version, &macos.join("gotv"), &recursos, publicar)?;
    let mb = fs::metadata(&motor).map(|m| m.len()).unwrap_or(0) as f64 / 1_048_576.0;
    println!("✅ {} · {mb:.0} MB", motor.display());
    if !publicar {
        println!(
            "{}",
            tr!(
                "⚠ armado DEL TALLER: lleva tu firma y tus rutas, NO se publica. Para publicar: GOTV_FIRMA=- cargo run --release -p gotv-empaquetar",
                "⚠ WORKSHOP build: it carries your signature and your paths, do NOT publish it. To publish: GOTV_FIRMA=- cargo run --release -p gotv-empaquetar"
            )
        );
    }
    Ok(())
}

/// El nombre del archivo del motor suelto (lo que se sube a las versiones de GitHub).
fn nombre_del_motor(version: &str) -> String {
    format!("god-of-the-void-engine-{version}-macos-arm64")
}

/// Arma `<destino>/god-of-the-void-engine-<v>-macos-arm64.tar.gz`: `gotv`, `mlx.metallib`, LICENSE,
/// NOTICE, `licenses/` y README.txt. `tar` y `codesign` son de macOS: sin dependencias.
fn armar_motor(destino: &Path, version: &str, gotv: &Path, recursos: &Path, publicar: bool) -> Result<PathBuf, String> {
    let nombre = nombre_del_motor(version);
    let dir = destino.join(&nombre);
    let tgz = destino.join(format!("{nombre}.tar.gz"));
    if dir.exists() {
        fs::remove_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    crear(&dir)?;
    copiar(gotv, &dir.join("gotv"))?;
    copiar(&recursos.join("mlx.metallib"), &dir.join("mlx.metallib"))?;
    copiar(Path::new("LICENSE"), &dir.join("LICENSE"))?;
    copiar(Path::new("NOTICE"), &dir.join("NOTICE"))?;
    copiar_carpeta(&recursos.join("licencias"), &dir.join("licenses"))?;
    escribir(&dir.join("README.txt"), &leeme_del_motor(version))?;
    if publicar {
        correr("codesign", &["--force", "--sign", "-", &dir.join("gotv").to_string_lossy()])?;
        juzgar_lo_publico(&dir, &env::var("HOME").unwrap_or_default())?;
    }
    correr("tar", &["-czf", &tgz.to_string_lossy(), "-C", &destino.to_string_lossy(), &nombre])?;
    fs::remove_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    Ok(tgz)
}

/// El README.txt del motor suelto, en inglés y en español.
fn leeme_del_motor(version: &str) -> String {
    format!(
        "GOD OF THE VOID — the engine, without the app · v{version}
Apple Silicon (M1 or newer) · macOS 14 or newer · Apache-2.0 · https://github.com/CHAOSX-SPACE/god-of-the-void

Keep `gotv` and `mlx.metallib` in the same folder: the engine loads its Metal kernels from there.

  ./gotv --help
  ./gotv generadores                    models, their licenses, accept and download
  ./gotv image \"a lighthouse at dusk\" --out lighthouse.png
  ./gotv music \"lo-fi piano, rain\" --seconds 30 --out song.wav
  ./gotv say \"Hello from the void\" --out hello.wav
  ./gotv listen --file talk.m4a
  ./gotv split song.mp3
  ./gotv serve                          OpenAI- and Ollama-compatible API on 127.0.0.1:11435

No model ships here. Each one keeps its own license: GOTV shows it, you accept it, and it is
downloaded from its publisher on Hugging Face.

If macOS blocks `gotv` after a browser download, download it with curl instead, or run
  xattr -d com.apple.quarantine gotv mlx.metallib

— Español —
El motor de GOD OF THE VOID sin la app. Deja `gotv` y `mlx.metallib` en la misma carpeta.
`./gotv --help` lo explica todo; los modelos se aceptan y se bajan con `./gotv generadores`.
"
    )
}

/// Las variables de la compilación PÚBLICA. Rust y C/C++ graban rutas de fuente (ubicaciones de
/// pánico, `__FILE__`): se reescriben a `~` (la carpeta personal) y `gotv` (el repo; va después
/// porque la última regla que coincide manda). Los kernels Metal salen a una carpeta sin usuario.
/// Carpeta aparte (`target/publicar`): cambiar RUSTFLAGS en la del taller la recompilaría entera.
fn entorno_publico(raiz: &Path, hogar: &str, target: &Path) -> Vec<(&'static str, String)> {
    let raiz = raiz.display();
    let rust = [format!("--remap-path-prefix={hogar}=~"), format!("--remap-path-prefix={raiz}=gotv")];
    let c = format!("-ffile-prefix-map={hogar}=~ -ffile-prefix-map={raiz}=gotv");
    vec![
        ("CARGO_TARGET_DIR", target.display().to_string()),
        // separadas por 0x1f: una ruta con espacios no rompe la bandera
        ("CARGO_ENCODED_RUSTFLAGS", rust.join("\x1f")),
        ("CFLAGS", c.clone()),
        ("CXXFLAGS", c),
        ("MLX_RS_METAL_PATH", "/Users/Shared/gotv/mlx-metal".into()),
    ]
}

/// Prefijos de llaves reales y el largo mínimo del cuerpo que las delata (letras, cifras, `_` y `-`).
/// Lo que el código de GOTV lleva para RECONOCER llaves son prefijos sueltos: no llegan al largo.
const LLAVES: &[(&str, usize)] = &[
    ("hf_", 30),
    ("ghp_", 36),
    ("gho_", 36),
    ("github_pat_", 60),
    ("sk-ant-", 40),
    ("sk-proj-", 40),
    ("xoxb-", 40),
    ("AKIA", 16),
    ("AIza", 35),
];

/// Cuántas llaves con forma real hay en `bytes`: un prefijo de [`LLAVES`] seguido de un cuerpo largo.
fn llaves_en(bytes: &[u8]) -> usize {
    let cuerpo = |b: u8| b.is_ascii_alphanumeric() || b == b'_' || b == b'-';
    let mut n = 0;
    for (prefijo, minimo) in LLAVES {
        let p = prefijo.as_bytes();
        let mut i = 0;
        while let Some(j) = buscar(&bytes[i..], p) {
            let ini = i + j + p.len();
            let largo = bytes[ini..].iter().take_while(|&&b| cuerpo(b)).count();
            // la llave empieza en el prefijo: si el byte anterior es parte de una palabra, no es llave
            let suelto = i + j == 0 || !cuerpo(bytes[i + j - 1]);
            if largo >= *minimo && suelto {
                n += 1;
            }
            i = ini;
        }
    }
    // una llave privada con su cuerpo SEGUIDO (base64 con saltos de línea), no el texto que las nombra
    // para reconocerlas: en la tabla de textos de un binario sobran letras sueltas (falso positivo medido
    // el 2026-11-23 con el patrón de la Purga, «…PRIVATE KEY-----anthropic.s…»)
    let mut i = 0;
    while let Some(j) = buscar(&bytes[i..], b"PRIVATE KEY-----") {
        let ini = i + j + 16;
        let b64 = bytes[ini..]
            .iter()
            .take_while(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'/' | b'=' | b'\n' | b'\r'))
            .filter(|b| !matches!(b, b'\n' | b'\r'))
            .count();
        if b64 >= 60 {
            n += 1;
        }
        i = ini;
    }
    n
}

/// El juez de lo que se PUBLICA: recorre `raiz` entera y falla si un archivo lleva la carpeta personal
/// de quien armó (`hogar`), una llave con forma real o el token de Hugging Face de esta Mac (se compara
/// su valor, que jamás se imprime). Las rutas que viajan son globales: las de quien instala.
fn juzgar_lo_publico(raiz: &Path, hogar: &str) -> Result<(), String> {
    let mut token = fs::read(Path::new(hogar).join(".cache/huggingface/token")).unwrap_or_default();
    token.retain(|b| !b.is_ascii_whitespace());
    let mut sucios = Vec::new();
    let mut pendientes = vec![raiz.to_path_buf()];
    while let Some(d) = pendientes.pop() {
        for e in fs::read_dir(&d).map_err(|e| format!("{}: {e}", d.display()))?.flatten() {
            let p = e.path();
            if p.is_dir() {
                pendientes.push(p);
                continue;
            }
            let bytes = fs::read(&p).map_err(|er| format!("{}: {er}", p.display()))?;
            let rel = p.strip_prefix(raiz).unwrap_or(&p).display().to_string();
            let rutas = if hogar.len() > 1 { contar(&bytes, format!("{hogar}/").as_bytes()) } else { 0 };
            if rutas > 0 {
                sucios.push(tr!("{rel}: {rutas} rutas de tu carpeta personal", "{rel}: {rutas} paths from your home folder"));
            }
            let llaves = llaves_en(&bytes);
            if llaves > 0 {
                sucios.push(tr!("{rel}: {llaves} llaves con forma real", "{rel}: {llaves} real-looking keys"));
            }
            if token.len() >= 20 && contar(&bytes, &token) > 0 {
                sucios.push(tr!("{rel}: TU token de Hugging Face", "{rel}: YOUR Hugging Face token"));
            }
        }
    }
    if sucios.is_empty() {
        println!("{}", tr!("⚖ juicio limpio: ni rutas tuyas, ni llaves, ni tu token", "⚖ clean judgement: no paths of yours, no keys, no token"));
        Ok(())
    } else {
        let lista = sucios.join("\n  ");
        Err(tr!("lo público NO puede salir así:\n  {lista}", "the public build can NOT ship like this:\n  {lista}"))
    }
}

/// Cuántas veces aparece `aguja` en `pajar` (el juez de la app pública).
fn contar(pajar: &[u8], aguja: &[u8]) -> usize {
    if aguja.is_empty() {
        return 0;
    }
    pajar.windows(aguja.len()).filter(|w| *w == aguja).count()
}

fn cargo(args: &[&str], entorno: &[(&str, String)]) -> Result<(), String> {
    let cargo = env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let ok = Command::new(&cargo)
        .args(args)
        .envs(entorno.iter().map(|(k, v)| (*k, v.as_str())))
        .status()
        .map_err(|e| format!("{cargo}: {e}"))?
        .success();
    if ok {
        Ok(())
    } else {
        let orden = args.join(" ");
        Err(tr!("falló: cargo {orden}", "failed: cargo {orden}"))
    }
}

/// La carpeta de compilación de mlx-sys que salió para macOS 14 (la más reciente si hay varias).
fn build_de_mlx(release: &Path) -> Result<PathBuf, String> {
    let mut buenas: Vec<(std::time::SystemTime, PathBuf)> = fs::read_dir(release.join("build"))
        .map_err(|e| e.to_string())?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("mlx-sys-")))
        .filter(|p| fs::read_to_string(p.join("output")).is_ok_and(|s| s.contains(&format!("Building for macOS {MACOS_MINIMO}"))))
        .filter_map(|p| Some((fs::metadata(p.join("output")).ok()?.modified().ok()?, p)))
        .collect();
    buenas.sort();
    buenas.pop().map(|(_, p)| p).ok_or_else(|| {
        tr!(
            "MLX no quedó compilado para macOS {MACOS_MINIMO}: el núcleo nativo no correría en Macs más viejas",
            "MLX was not built for macOS {MACOS_MINIMO}: the native core would not run on older Macs"
        )
    })
}

/// `<nombre>/LICENSE` de cada dependencia que CMake bajó para MLX (out/build/_deps/<nombre>-src).
fn licencias_de_mlx(build: &Path) -> Vec<(String, PathBuf)> {
    let Ok(rd) = fs::read_dir(build.join("out/build/_deps")) else { return vec![] };
    let mut v: Vec<(String, PathBuf)> = rd
        .flatten()
        .filter_map(|e| {
            let n = e.file_name().to_string_lossy().into_owned();
            let nombre = n.strip_suffix("-src")?.to_string();
            let l = e.path().join("LICENSE");
            l.is_file().then_some((nombre, l))
        })
        .collect();
    v.sort();
    v
}

/// Como `strings gotv | grep -m1 '/mlx\.metallib$'`: la primera tira imprimible que TERMINA en
/// `/mlx.metallib`.
fn ruta_del_metallib(binario: &[u8]) -> Option<String> {
    let aguja = b"/mlx.metallib";
    let imprimible = |b: u8| b == b'\t' || (0x20..0x7f).contains(&b);
    let mut desde = 0;
    while let Some(i) = buscar(&binario[desde..], aguja) {
        let fin = desde + i + aguja.len();
        if binario.get(fin).is_none_or(|&b| !imprimible(b)) {
            let mut ini = desde + i;
            while ini > 0 && imprimible(binario[ini - 1]) {
                ini -= 1;
            }
            if fin - ini >= 4 {
                return Some(String::from_utf8_lossy(&binario[ini..fin]).into_owned());
            }
        }
        desde += i + 1;
    }
    None
}

fn buscar(pajar: &[u8], aguja: &[u8]) -> Option<usize> {
    pajar.windows(aguja.len()).position(|w| w == aguja)
}

fn contiene(pajar: &[u8], aguja: &[u8]) -> bool {
    buscar(pajar, aguja).is_some()
}

/// La primera línea `version = "…"` del Cargo.toml raíz (la de [workspace.package]).
fn version_del_workspace(toml: &str) -> Option<String> {
    let l = toml.lines().find(|l| l.starts_with("version"))?;
    Some(l.split('"').nth(1)?.to_string())
}

fn icono(png: &Path, icns: &Path) -> Result<(), String> {
    let set = env::temp_dir().join(format!("gotv-icono-{}", std::process::id())).join("gotv.iconset");
    crear(&set)?;
    let png = png.to_string_lossy().into_owned();
    for s in [16u32, 32, 128, 256, 512] {
        for (lado, nombre) in [(s, format!("icon_{s}x{s}.png")), (s * 2, format!("icon_{s}x{s}@2x.png"))] {
            if lado > 1024 {
                continue;
            }
            let lado = lado.to_string();
            correr("sips", &["-z", &lado, &lado, &png, "--out", &set.join(nombre).to_string_lossy()])?;
        }
    }
    correr("iconutil", &["-c", "icns", &set.to_string_lossy(), "-o", &icns.to_string_lossy()])?;
    let _ = fs::remove_dir_all(set.parent().unwrap_or(&set));
    Ok(())
}

fn correr(programa: &str, args: &[&str]) -> Result<(), String> {
    let s = Command::new(programa).args(args).output().map_err(|e| format!("{programa}: {e}"))?;
    if s.status.success() {
        Ok(())
    } else {
        Err(format!("{programa}: {}", String::from_utf8_lossy(&s.stderr).trim()))
    }
}

/// Info.plist con los textos de permiso en inglés (la base); el español va en es.lproj.
fn info_plist(version: &str) -> String {
    let permisos: String = PERMISOS.iter().map(|(clave, en, _)| format!("  <key>{clave}</key><string>{en}</string>\n")).collect();
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleName</key><string>GOD OF THE VOID</string>
  <key>CFBundleDisplayName</key><string>GOD OF THE VOID</string>
  <key>CFBundleIdentifier</key><string>space.chaosx.gotv</string>
  <key>CFBundleVersion</key><string>{version}</string>
  <key>CFBundleShortVersionString</key><string>{version}</string>
  <key>CFBundleExecutable</key><string>gotv-gui</string>
  <key>CFBundleIconFile</key><string>gotv</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleDevelopmentRegion</key><string>en</string>
  <key>CFBundleLocalizations</key><array><string>en</string><string>es</string></array>
  <key>LSMinimumSystemVersion</key><string>{MACOS_MINIMO}</string>
  <key>NSHighResolutionCapable</key><true/>
{permisos}</dict></plist>
"#
    )
}

/// es.lproj/InfoPlist.strings: los mismos permisos en español.
fn plist_en_espanol() -> String {
    PERMISOS.iter().map(|(clave, _, es)| format!("\"{clave}\" = \"{es}\";\n")).collect()
}

/// Por qué pide cada permiso macOS: (clave, inglés, español).
const PERMISOS: &[(&str, &str, &str)] = &[
    // juez-idioma: bloque datos (los textos de permiso del Info.plist en sus dos idiomas, ya en par)
    (
        "NSAppleEventsUsageDescription",
        "GOD OF THE VOID controls the keyboard only if you turn on computer_use.",
        "GOD OF THE VOID controla el teclado sólo si activas computer_use.",
    ),
    (
        "NSMicrophoneUsageDescription",
        "GOD OF THE VOID listens only while the microphone button or hands-free mode (🎧) is on; what it hears is transcribed on this Mac.",
        "GOD OF THE VOID escucha tu voz sólo mientras el botón del micrófono o el modo manos libres (🎧) están activos; lo que oye se transcribe en esta Mac.",
    ),
    (
        "NSCameraUsageDescription",
        "GOD OF THE VOID turns the camera on only to take a photo you approve each time (tool «equipos», action «foto»); the photo stays in your project.",
        "GOD OF THE VOID enciende la cámara sólo para tomar una foto que tú apruebas cada vez (herramienta «equipos», acción «foto»); la foto se queda en tu proyecto.",
    ),
    (
        "NSDocumentsFolderUsageDescription",
        "Lets you open a project you keep in Documents. GOD OF THE VOID works only inside the project folder you choose, not all of Documents.",
        "Permite abrir un proyecto que guardas en Documentos. GOD OF THE VOID trabaja sólo dentro de la carpeta de proyecto que elijas, no en todo Documentos.",
    ),
    (
        "NSDesktopFolderUsageDescription",
        "GOD OF THE VOID works in the projects you open on the Desktop.",
        "GOD OF THE VOID trabaja en los proyectos que tú abres en el Escritorio.",
    ),
    (
        "NSDownloadsFolderUsageDescription",
        "GOD OF THE VOID works in the projects you open in Downloads.",
        "GOD OF THE VOID trabaja en los proyectos que tú abres en Descargas.",
    ),
    (
        "NSRemovableVolumesUsageDescription",
        "GOD OF THE VOID works in the projects you open on external drives.",
        "GOD OF THE VOID trabaja en los proyectos que tú abres en discos externos.",
    ),
];

fn firmar(app: &Path) {
    let ruta = app.to_string_lossy().into_owned();
    let id = env::var("GOTV_FIRMA").ok().filter(|s| !s.is_empty()).or_else(|| {
        let o = Command::new("security").args(["find-identity", "-v", "-p", "codesigning"]).output().ok()?;
        identidad_de_desarrollo(&String::from_utf8_lossy(&o.stdout))
    });
    // `GOTV_FIRMA=-`: la app que se COMPARTE. Una identidad de desarrollo graba en la firma el nombre y el
    // correo de su dueño (`codesign -dvv` los muestra); la versión pública va ad hoc, sin nadie dentro.
    if id.as_deref() == Some("-") {
        if correr("codesign", &["--force", "--deep", "--sign", "-", &ruta]).is_ok() {
            println!(
                "{}",
                tr!(
                    "🔏 firmada ad hoc: lista para publicar, sin identidad personal",
                    "🔏 signed ad hoc: ready to publish, no personal identity inside"
                )
            );
        }
        return;
    }
    if let Some(id) = &id {
        match correr("codesign", &["--force", "--deep", "--sign", id, &ruta]) {
            Ok(()) => {
                let corto = id.split(" (").next().unwrap_or(id);
                println!("{}", tr!("🔏 firmada con: {corto}…", "🔏 signed with: {corto}…"));
                return;
            }
            Err(e) => {
                let ultima = e.lines().last().unwrap_or("").to_string();
                println!(
                    "{}",
                    tr!(
                        "⚠ no pude firmar con «{id}» ({ultima}); queda ad hoc: macOS pedirá permisos en cada versión",
                        "⚠ could not sign with «{id}» ({ultima}); left ad hoc: macOS will ask for permissions on every version"
                    )
                );
            }
        }
    }
    let _ = correr("codesign", &["--force", "--deep", "--sign", "-", &ruta]);
}

/// La primera «Apple Development: …» de `security find-identity -v -p codesigning`.
fn identidad_de_desarrollo(salida: &str) -> Option<String> {
    salida.lines().find_map(|l| {
        let i = l.find("\"Apple Development")?;
        let resto = &l[i + 1..];
        Some(resto[..resto.find('"')?].to_string())
    })
}

fn peso(p: &Path) -> u64 {
    match fs::symlink_metadata(p) {
        Ok(m) if m.is_dir() => fs::read_dir(p).map(|rd| rd.flatten().map(|e| peso(&e.path())).sum()).unwrap_or(0),
        Ok(m) => m.len(),
        Err(_) => 0,
    }
}

fn crear(p: &Path) -> Result<(), String> {
    fs::create_dir_all(p).map_err(|e| format!("{}: {e}", p.display()))
}

fn leer(p: &str) -> Result<String, String> {
    fs::read_to_string(p).map_err(|e| format!("{p}: {e}"))
}

fn escribir(p: &Path, texto: &str) -> Result<(), String> {
    fs::write(p, texto).map_err(|e| format!("{}: {e}", p.display()))
}

/// Copia un archivo conservando sus permisos (los ejecutables siguen siéndolo).
fn copiar(de: &Path, a: &Path) -> Result<(), String> {
    fs::copy(de, a).map(|_| ()).map_err(|e| format!("{} → {}: {e}", de.display(), a.display()))
}

fn copiar_carpeta(de: &Path, a: &Path) -> Result<(), String> {
    crear(a)?;
    for e in fs::read_dir(de).map_err(|e| format!("{}: {e}", de.display()))?.flatten() {
        let (origen, destino) = (e.path(), a.join(e.file_name()));
        if origen.is_dir() {
            copiar_carpeta(&origen, &destino)?;
        } else {
            copiar(&origen, &destino)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn halla_la_ruta_del_metallib_como_strings_y_grep() {
        let mut b = b"\x00\x01basura\x00/a/target/mlx-metal/mlx.metallib.bak\x00".to_vec();
        b.extend_from_slice(b"\x02/Users/x/p/target/mlx-metal/mlx.metallib\x00resto");
        assert_eq!(ruta_del_metallib(&b).as_deref(), Some("/Users/x/p/target/mlx-metal/mlx.metallib"), "la que TERMINA ahí");
        assert_eq!(ruta_del_metallib(b"\x00nada\x00"), None);
        assert_eq!(ruta_del_metallib(b"/mlx.metallib").as_deref(), Some("/mlx.metallib"), "al final del archivo también");
    }

    #[test]
    fn la_version_es_la_del_workspace() {
        let t = "[workspace]\nmembers = []\n\n[workspace.package]\nversion = \"0.1.0\"\nedition = \"2021\"\n";
        assert_eq!(version_del_workspace(t).as_deref(), Some("0.1.0"));
        assert_eq!(version_del_workspace("[workspace]\n"), None);
    }

    #[test]
    fn la_identidad_sale_de_security() {
        let s = "  1) ABCDEF \"Apple Development: Nadie (XYZ123)\"\n     1 valid identities found\n";
        assert_eq!(identidad_de_desarrollo(s).as_deref(), Some("Apple Development: Nadie (XYZ123)"));
        assert_eq!(identidad_de_desarrollo("     0 valid identities found\n"), None);
    }

    #[test]
    fn el_plist_es_valido_y_bilingue() {
        let p = info_plist("9.9.9");
        assert!(p.contains("<string>9.9.9</string>") && p.contains("<string>14.0</string>"));
        assert_eq!(p.matches("UsageDescription").count(), PERMISOS.len());
        assert!(!p.contains("sólo"), "la base va en inglés");
        let es = plist_en_espanol();
        assert_eq!(es.lines().count(), PERMISOS.len());
        assert!(es.lines().all(|l| l.starts_with("\"NS") && l.ends_with("\";")));
    }

    #[test]
    fn el_juez_reconoce_llaves_reales_y_no_sus_prefijos() {
        // llaves inventadas, armadas en la prueba (ninguna existe)
        let hf = format!("x hf_{} y", "Q".repeat(34));
        assert_eq!(llaves_en(hf.as_bytes()), 1);
        assert_eq!(llaves_en(format!("AKIA{}", "Z7".repeat(8)).as_bytes()), 1);
        let pem = format!("-----BEGIN RSA PRIVATE KEY-----\n{}", "MIIEow".repeat(20));
        assert_eq!(llaves_en(pem.as_bytes()), 1);
        // lo que el código lleva para RECONOCERLAS no cuenta
        assert_eq!(llaves_en(b"starts_with(\"hf_\") || starts_with(\"sk-ant-\")"), 0);
        assert_eq!(llaves_en(b"-----BEGIN PRIVATE KEY-----\" => secreto"), 0);
        // lo que la Purga lleva en el binario: el patrón seguido de OTROS textos (medido 2026-11-23)
        let patron = format!("-END [A-Z0-9 ]*PRIVATE KEY-----anthropic.s{}", "ignore previous instructions ".repeat(10));
        assert_eq!(llaves_en(patron.as_bytes()), 0, "letras sueltas tras el patrón no son un cuerpo");
        // un símbolo de Rust con «hf_» pegado a una palabra (sqlparser) no es llave
        assert_eq!(llaves_en(format!("__RINvXshf_{}9sqlparser", "NtCsebWzpB9GJ1".repeat(4)).as_bytes()), 0);
        assert_eq!(llaves_en(b"chf_abcdefghijklmnopqrstuvwxyz0123456789"), 0, "pegado a una palabra no es llave");
    }

    #[test]
    fn el_juez_de_lo_publico_recorre_todo() {
        let d = env::temp_dir().join(format!("gotv-juez-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(d.join("Contents/Resources/x")).unwrap();
        fs::write(d.join("Contents/Resources/x/limpio.txt"), "nada").unwrap();
        assert!(juzgar_lo_publico(&d, "/Users/alguien").is_ok());
        fs::write(d.join("Contents/Resources/x/sucio.json"), "{\"p\":\"/Users/alguien/proyecto\"}").unwrap();
        let e = juzgar_lo_publico(&d, "/Users/alguien").unwrap_err();
        assert!(e.contains("sucio.json"), "{e}");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn el_motor_suelto_se_nombra_y_se_explica() {
        assert_eq!(nombre_del_motor("0.1.0"), "god-of-the-void-engine-0.1.0-macos-arm64");
        let l = leeme_del_motor("0.1.0");
        assert!(l.contains("v0.1.0") && l.contains("mlx.metallib") && l.contains("gotv serve") && l.contains("Español"));
        assert!(!l.contains("/Users/"), "nada de rutas de quien lo armó");
    }

    #[test]
    fn el_juez_cuenta_cada_ruta_personal() {
        assert_eq!(contar(b"/Users/x/a\0/Users/x/b\0/Users/xy", b"/Users/x/"), 2);
        assert_eq!(contar(b"nada", b""), 0);
    }

    #[test]
    fn la_compilacion_publica_reescribe_el_repo_despues_del_hogar() {
        let e = entorno_publico(Path::new("/Users/x/mi repo"), "/Users/x", Path::new("/Users/x/mi repo/target/publicar"));
        let v = |k: &str| e.iter().find(|(c, _)| *c == k).map(|(_, v)| v.clone()).unwrap();
        // la última regla que coincide manda: el repo va después del hogar
        assert_eq!(v("CARGO_ENCODED_RUSTFLAGS"), "--remap-path-prefix=/Users/x=~\x1f--remap-path-prefix=/Users/x/mi repo=gotv");
        assert!(v("CXXFLAGS").ends_with("-ffile-prefix-map=/Users/x/mi repo=gotv"));
        assert!(!v("MLX_RS_METAL_PATH").contains("/Users/x"));
    }
}
