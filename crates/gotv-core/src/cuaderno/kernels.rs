//! F4 paso 2 · Descubrir los kernels: los `kernel.json` del entorno del PROYECTO primero (su `.venv`), luego los
//! del usuario (`~/Library/Jupyter/kernels`) y los del sistema. GOTV no trae Python ni instala kernels (nivel 3
//! de L1): si el lenguaje del notebook no tiene uno, lo dice y dice cómo se instala.

use anyhow::{bail, Result};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Un kernel instalado (su `kernel.json`).
#[derive(Clone, Debug)]
pub struct Especificacion {
    pub nombre: String,
    pub mostrar: String,
    pub lenguaje: String,
    pub argv: Vec<String>,
    pub env: BTreeMap<String, String>,
    /// `interrupt_mode: "message"`: se interrumpe con `interrupt_request` (si no, con la señal)
    pub interrumpe_por_mensaje: bool,
    /// la carpeta del `kernel.json` (su `{resource_dir}`)
    pub carpeta: PathBuf,
    /// del entorno del proyecto (no del usuario ni del sistema)
    pub del_proyecto: bool,
}

/// Los entornos de Python del proyecto, en el orden en que se prefieren.
const ENTORNOS: &[&str] = &[".venv", "venv", "env"];

/// El Python de un entorno del proyecto que TIENE ipykernel (sin él no hay kernel que lanzar).
pub fn python_con_ipykernel(root: &Path) -> Option<PathBuf> {
    ENTORNOS.iter().map(|e| root.join(e)).find_map(|v| {
        let py = v.join("bin/python");
        let lib = std::fs::read_dir(v.join("lib")).ok()?;
        let tiene = lib.flatten().any(|d| d.path().join("site-packages/ipykernel").is_dir());
        (py.exists() && tiene).then_some(py)
    })
}

/// Las carpetas de kernels, con si son del proyecto.
fn carpetas(root: &Path) -> Vec<(PathBuf, bool)> {
    let mut v: Vec<(PathBuf, bool)> = ENTORNOS.iter().map(|e| (root.join(e).join("share/jupyter/kernels"), true)).collect();
    if let Some(jp) = std::env::var_os("JUPYTER_PATH") {
        v.extend(std::env::split_paths(&jp).map(|p| (p.join("kernels"), false)));
    }
    if let Some(casa) = std::env::var_os("HOME").map(PathBuf::from) {
        v.push((casa.join("Library/Jupyter/kernels"), false));
        v.push((casa.join(".local/share/jupyter/kernels"), false));
    }
    for s in ["/opt/homebrew/share/jupyter/kernels", "/usr/local/share/jupyter/kernels", "/usr/share/jupyter/kernels"] {
        v.push((PathBuf::from(s), false));
    }
    v
}

fn leer(dir: &Path, del_proyecto: bool) -> Option<Especificacion> {
    let v: Value = serde_json::from_str(&std::fs::read_to_string(dir.join("kernel.json")).ok()?).ok()?;
    let argv: Vec<String> = v["argv"].as_array()?.iter().filter_map(|x| x.as_str().map(String::from)).collect();
    if argv.is_empty() {
        return None;
    }
    let env = v["env"].as_object().map(|m| m.iter().filter_map(|(k, x)| Some((k.clone(), x.as_str()?.to_string()))).collect()).unwrap_or_default();
    Some(Especificacion {
        nombre: dir.file_name()?.to_string_lossy().to_lowercase(),
        mostrar: v["display_name"].as_str().unwrap_or("").to_string(),
        lenguaje: v["language"].as_str().unwrap_or("").to_lowercase(),
        argv,
        env,
        interrumpe_por_mensaje: v["interrupt_mode"].as_str() == Some("message"),
        carpeta: dir.to_path_buf(),
        del_proyecto,
    })
}

/// ¿`argv[0]` es un «python» suelto (sin ruta)? Jupyter lo cambia por SU Python; GOTV, por el del proyecto.
fn python_suelto(a: &str) -> bool {
    !a.contains('/') && (a == "python" || a.starts_with("python3"))
}

/// Todos los kernels que se ven desde el proyecto (los del proyecto primero; un nombre repetido, el primero).
pub fn descubrir(root: &Path) -> Vec<Especificacion> {
    let py = python_con_ipykernel(root);
    let mut v: Vec<Especificacion> = Vec::new();
    for (c, propio) in carpetas(root) {
        let Ok(rd) = std::fs::read_dir(&c) else { continue };
        let mut dirs: Vec<PathBuf> = rd.flatten().map(|d| d.path()).filter(|p| p.is_dir()).collect();
        dirs.sort();
        for d in dirs {
            let Some(mut e) = leer(&d, propio) else { continue };
            if python_suelto(&e.argv[0]) {
                // el del entorno donde vive el kernel.json; si no, el del proyecto; si no, el del PATH
                let venv_py = c.parent().and_then(|p| p.parent()).and_then(|p| p.parent()).map(|v| v.join("bin/python")).filter(|p| propio && p.exists());
                if let Some(p) = venv_py.or_else(|| py.clone()) {
                    e.argv[0] = p.display().to_string();
                }
            }
            if !v.iter().any(|x| x.nombre == e.nombre) {
                v.push(e);
            }
        }
    }
    // ipykernel instalado en el .venv sin su kernel.json (pasa con `pip install ipykernel` sin `--sys-prefix`
    // en algunas versiones): se lanza igual que lo haría Jupyter
    if let Some(p) = py {
        if !v.iter().any(|e| e.del_proyecto && e.lenguaje == "python") {
            v.insert(
                0,
                Especificacion {
                    nombre: "python3".into(),
                    mostrar: "Python 3 (.venv)".into(),
                    lenguaje: "python".into(),
                    argv: vec![p.display().to_string(), "-m".into(), "ipykernel_launcher".into(), "-f".into(), "{connection_file}".into()],
                    env: BTreeMap::new(),
                    interrumpe_por_mensaje: false,
                    carpeta: p.parent().unwrap_or(root).to_path_buf(),
                    del_proyecto: true,
                },
            );
            v.retain(|e| e.del_proyecto || e.nombre != "python3");
        }
    }
    v
}

/// Cómo se instala el kernel de un lenguaje (GOTV no lo hace: lo dice).
fn como_instalar(lenguaje: &str) -> String {
    match lenguaje {
        "python" | "" => crate::i18n::t(
            "instala ipykernel en el entorno del proyecto (p. ej. `uv venv && uv pip install ipykernel`, o `.venv/bin/python -m pip install ipykernel`)",
            "install ipykernel in the project's environment (e.g. `uv venv && uv pip install ipykernel`, or `.venv/bin/python -m pip install ipykernel`)",
        )
        .into(),
        "rust" => crate::i18n::t(
            "instala evcxr (`cargo install evcxr_jupyter && evcxr_jupyter --install`)",
            "install evcxr (`cargo install evcxr_jupyter && evcxr_jupyter --install`)",
        )
        .into(),
        l => crate::tr!("instala el kernel de Jupyter de {l} con su propio instalador", "install the {l} Jupyter kernel with its own installer"),
    }
}

/// El kernel para un notebook: por el nombre de su `kernelspec` (prefiriendo el del proyecto), si no por su lenguaje.
pub fn elegir(root: &Path, nombre: Option<&str>, lenguaje: Option<&str>) -> Result<Especificacion> {
    let todos = descubrir(root);
    let lenguaje = lenguaje.map(str::to_lowercase).unwrap_or_default();
    if let Some(n) = nombre.map(str::to_lowercase) {
        // un nombre de Python (python3, python) se resuelve con el del proyecto aunque el notebook venga de otro lado
        let python = n.starts_with("python");
        if let Some(e) = todos.iter().find(|e| e.nombre == n && (e.del_proyecto || !python)).or_else(|| todos.iter().find(|e| python && e.del_proyecto && e.lenguaje == "python")) {
            return Ok(e.clone());
        }
        if let Some(e) = todos.iter().find(|e| e.nombre == n) {
            return Ok(e.clone());
        }
    }
    let lengua = if lenguaje.is_empty() { "python".to_string() } else { lenguaje.clone() };
    if let Some(e) = todos.iter().find(|e| e.lenguaje == lengua) {
        return Ok(e.clone());
    }
    let vistos: Vec<String> = todos.iter().map(|e| format!("{} ({})", e.nombre, e.lenguaje)).collect();
    bail!(
        "{}",
        crate::tr!(
            "no hay kernel de {lengua} para este notebook{}: {}. GOTV no trae Python propio ni instala kernels: usa el del proyecto. Instalados: {}",
            "there is no {lengua} kernel for this notebook{}: {}. GOTV ships no Python of its own and installs no kernels: it uses the project's. Installed: {}",
            nombre.map(|n| format!(" («{n}»)")).unwrap_or_default(),
            como_instalar(&lengua),
            if vistos.is_empty() { crate::i18n::t("ninguno", "none").to_string() } else { vistos.join(", ") }
        )
    )
}

impl Especificacion {
    /// La orden con `{connection_file}` y `{resource_dir}` puestos (en `argv` y en `env`, como Jupyter 7+).
    pub fn orden(&self, conexion: &Path) -> (Vec<String>, BTreeMap<String, String>) {
        let poner = |s: &str| s.replace("{connection_file}", &conexion.display().to_string()).replace("{resource_dir}", &self.carpeta.display().to_string());
        (self.argv.iter().map(|a| poner(a)).collect(), self.env.iter().map(|(k, v)| (k.clone(), poner(v))).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kernel(dir: &Path, nombre: &str, json: &str) {
        let d = dir.join(nombre);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("kernel.json"), json).unwrap();
    }

    #[test]
    fn el_del_proyecto_primero_y_python_suelto_es_el_del_venv() {
        let d = crate::CarpetaPrueba::nueva("gotv-kernels");
        let root = d.join("proy");
        // un .venv con ipykernel y su kernel.json con «python» suelto (como lo instala ipykernel)
        let venv = root.join(".venv");
        std::fs::create_dir_all(venv.join("bin")).unwrap();
        std::fs::write(venv.join("bin/python"), "").unwrap();
        std::fs::create_dir_all(venv.join("lib/python3.12/site-packages/ipykernel")).unwrap();
        kernel(&venv.join("share/jupyter/kernels"), "python3", r#"{"argv": ["python", "-m", "ipykernel_launcher", "-f", "{connection_file}"], "display_name": "Python 3 (ipykernel)", "language": "python", "metadata": {"debugger": true}}"#);
        let e = elegir(&root, Some("python3"), Some("python")).unwrap();
        assert!(e.del_proyecto && e.argv[0] == venv.join("bin/python").display().to_string(), "{e:?}");
        let (argv, _) = e.orden(Path::new("/t/k.json"));
        assert_eq!(argv.last().unwrap(), "/t/k.json");
        // sin kernel.json pero con ipykernel: se sintetiza el de Jupyter
        std::fs::remove_dir_all(venv.join("share")).unwrap();
        let e = elegir(&root, Some("python3"), None).unwrap();
        assert!(e.del_proyecto && e.argv[1..3] == ["-m".to_string(), "ipykernel_launcher".into()], "{e:?}");
        // sin ipykernel en el proyecto: el error dice cómo instalarlo y no inventa nada
        let otro = d.join("vacio");
        std::fs::create_dir_all(&otro).unwrap();
        if descubrir(&otro).iter().all(|e| e.lenguaje != "cobol") {
            let m = elegir(&otro, Some("cobol9"), Some("cobol")).unwrap_err().to_string();
            assert!(m.contains("cobol") && (m.contains("instala") || m.contains("install")), "{m}");
        }
    }

    #[test]
    fn modo_de_interrupcion_y_env_con_su_conexion() {
        let d = crate::CarpetaPrueba::nueva("gotv-kernels-env");
        let ks = d.join("k");
        kernel(&ks, "Falso", r#"{"argv": ["/bin/falso", "{resource_dir}/x"], "language": "Python", "interrupt_mode": "message", "env": {"CONEXION": "{connection_file}"}}"#);
        let e = leer(&ks.join("Falso"), false).unwrap();
        assert!(e.interrumpe_por_mensaje && e.nombre == "falso" && e.lenguaje == "python");
        let (argv, env) = e.orden(Path::new("/c.json"));
        assert_eq!(argv[1], format!("{}/x", ks.join("Falso").display()));
        assert_eq!(env["CONEXION"], "/c.json");
    }
}
