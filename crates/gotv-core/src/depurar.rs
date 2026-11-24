//! F15.2 · `depurar`: un DEPURADOR de verdad por el protocolo DAP, sobre el mismo `canal.rs` que LSP y MCP
//! (marco `Content-Length`, protocolo `Dap`). El modelo pone puntos de parada, arranca el programa o UNA
//! prueba, y cuando para mira la pila, las variables del marco y evalúa expresiones: lo que en un bucle
//! sólo se entiende mirando un valor a mitad de camino.
//!
//! Adaptadores (escalera de L1): `lldb-dap` (Xcode) para Rust, Swift, C y C++; `dlv dap` para Go; `debugpy`
//! del ENTORNO del proyecto para Python (nivel 3: el intérprete del usuario, nunca uno de GOTV). Node
//! (`js-debug`) no se ofrece: no viene con nada de esta Mac y no se instala por la espalda.
//!
//! Una sesión por proyecto; se cierra con `detener` o sola tras 10 min sin uso (el adaptador muere con
//! ella). Todo resultado dice dónde quedó el programa (parado en `archivo:línea`, corriendo o terminado
//! con su código) y la cola de su salida.
use crate::canal::{Canal, Marco, Protocolo};
use anyhow::{anyhow, bail, Result};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{Mutex, Notify};

/// Sin uso durante esto, la sesión se cierra sola.
pub const INACTIVIDAD: Duration = Duration::from_secs(600);
/// Lo que se espera a que el programa pare o termine tras arrancar o continuar.
const ESPERA: Duration = Duration::from_secs(20);
/// La salida del programa que se guarda (la cola).
const MAX_SALIDA: usize = 200_000;

#[derive(Debug, Clone)]
struct Detenido {
    hilo: i64,
    motivo: String,
    texto: String,
}

#[derive(Default)]
struct Estado {
    salida: String,
    detenido: Option<Detenido>,
    /// `Some(código)` al terminar (el código puede faltar)
    terminado: Option<Option<i64>>,
    inicializado: bool,
}

pub struct Sesion {
    canal: Arc<Canal>,
    estado: Arc<std::sync::Mutex<Estado>>,
    aviso: Arc<Notify>,
    pub adaptador: String,
    capacidades: Value,
    ultimo: Instant,
}

/// Los puntos de parada del proyecto (se ponen antes o durante la sesión): archivo → (línea, condición).
type Puntos = BTreeMap<PathBuf, Vec<(u64, Option<String>)>>;

struct Proyecto {
    sesion: Option<Sesion>,
    puntos: Puntos,
}

fn proyectos() -> &'static std::sync::Mutex<HashMap<PathBuf, Arc<Mutex<Proyecto>>>> {
    static P: std::sync::OnceLock<std::sync::Mutex<HashMap<PathBuf, Arc<Mutex<Proyecto>>>>> = std::sync::OnceLock::new();
    P.get_or_init(Default::default)
}

fn proyecto(root: &Path) -> Arc<Mutex<Proyecto>> {
    proyectos().lock().unwrap().entry(root.to_path_buf()).or_insert_with(|| Arc::new(Mutex::new(Proyecto { sesion: None, puntos: BTreeMap::new() }))).clone()
}

/// Termina los hijos directos de `pid` (sólo los suyos: `pgrep -P`, nada ajeno).
fn matar_hijos(pid: u32) {
    let Ok(o) = std::process::Command::new("pgrep").args(["-P", &pid.to_string()]).output() else { return };
    for h in String::from_utf8_lossy(&o.stdout).split_whitespace().filter(|s| s.parse::<u32>().is_ok()) {
        let _ = std::process::Command::new("kill").args(["-KILL", h]).status();
    }
}

/// El adaptador para lo que se quiere depurar: (comando, id del adaptador, argumentos de `launch` extra).
fn adaptador(root: &Path, programa: &Path) -> Result<(Vec<String>, &'static str, Value)> {
    let ext = programa.extension().and_then(|e| e.to_str()).unwrap_or("");
    match ext {
        "py" => {
            let py = [root.join(".venv/bin/python"), root.join("venv/bin/python")].into_iter().find(|p| p.is_file()).map(|p| p.display().to_string()).unwrap_or_else(|| "python3".into());
            // sin debugpy el adaptador muere al arrancar con un ModuleNotFoundError que el modelo no ve: se dice antes
            let tiene = std::process::Command::new(&py).args(["-c", "import debugpy"]).output().is_ok_and(|o| o.status.success());
            if !tiene {
                gotv_engine::bail_tr!(
                    "a `{py}` le falta `debugpy`: `{py} -m pip install debugpy` (mejor dentro del .venv del proyecto), con permiso del usuario",
                    "`{py}` is missing `debugpy`: `{py} -m pip install debugpy` (preferably inside the project's .venv), with the user's permission"
                );
            }
            Ok((vec![py, "-m".into(), "debugpy.adapter".into()], "debugpy", json!({"console": "internalConsole", "justMyCode": true})))
        }
        "js" | "mjs" | "cjs" | "ts" => bail!("{}", crate::i18n::t(
            "para Node no hay adaptador en esta Mac (js-debug no viene con nada instalado y no se instala por la espalda): depura con `console.log` o con `node --inspect` si el usuario conecta un depurador",
            "there is no Node adapter on this Mac (js-debug does not ship with anything installed and is not installed behind your back): debug with `console.log` or with `node --inspect` if the user attaches a debugger"
        )),
        "go" => {
            if !crate::entorno::en_path("dlv") {
                bail!("{}", crate::i18n::t("para Go hace falta `dlv` (Delve): `go install github.com/go-delve/delve/cmd/dlv@latest`, con permiso del usuario", "Go needs `dlv` (Delve): `go install github.com/go-delve/delve/cmd/dlv@latest`, with the user's permission"));
            }
            Ok((vec!["dlv".into(), "dap".into()], "go", json!({"mode": "debug"})))
        }
        _ => {
            // un binario (Rust, Swift, C, C++): lldb-dap de Xcode
            let lldb = std::process::Command::new("xcrun").args(["--find", "lldb-dap"]).output().ok().filter(|o| o.status.success()).map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).filter(|s| !s.is_empty()).or_else(|| crate::entorno::en_path("lldb-dap").then(|| "lldb-dap".to_string()));
            match lldb {
                Some(l) => Ok((vec![l], "lldb-dap", json!({"stopOnEntry": false}))),
                None => bail!("{}", crate::i18n::t("no encuentro `lldb-dap` (viene con Xcode o sus herramientas de línea de órdenes)", "cannot find `lldb-dap` (it comes with Xcode or its command line tools)")),
            }
        }
    }
}

/// F15.2 paso 8 · el binario de las pruebas de Rust (`cargo test --no-run`), para depurar UNA prueba.
async fn binario_de_pruebas(root: &Path) -> Result<PathBuf> {
    let o = crate::entorno::comando_tokio("cargo").args(["test", "--no-run", "--message-format=json"]).current_dir(root).stdin(std::process::Stdio::null()).output().await?;
    let mut candidatos: Vec<(bool, PathBuf)> = Vec::new();
    for l in String::from_utf8_lossy(&o.stdout).lines() {
        let Ok(v) = serde_json::from_str::<Value>(l) else { continue };
        if v["reason"] == "compiler-artifact" && v["profile"]["test"] == true {
            if let Some(e) = v["executable"].as_str() {
                let lib = v["target"]["kind"].as_array().is_some_and(|k| k.iter().any(|x| x == "lib"));
                candidatos.push((lib, PathBuf::from(e)));
            }
        }
    }
    candidatos.sort_by_key(|(lib, _)| !*lib);
    candidatos.into_iter().next().map(|c| c.1).ok_or_else(|| anyhow!("{}", crate::i18n::t("`cargo test --no-run` no dejó ningún binario de pruebas (¿compila?)", "`cargo test --no-run` produced no test binary (does it build?)")))
}

impl Sesion {
    async fn arrancar(root: &Path, cmd: Vec<String>, id: &str) -> Result<Sesion> {
        let mut c = tokio::process::Command::new(&cmd[0]);
        c.args(&cmd[1..]).current_dir(root);
        let registro = root.join(".gotv/registros/depurar.log");
        let canal = Arc::new(Canal::lanzar_con(c, Marco::ContentLength, Protocolo::Dap, id, Some(registro)).await?);
        Ok(Sesion::sobre(canal, id).await?)
    }

    /// La sesión sobre un canal ya abierto (un adaptador real o el falso de las pruebas).
    async fn sobre(canal: Arc<Canal>, id: &str) -> Result<Sesion> {
        let estado: Arc<std::sync::Mutex<Estado>> = Arc::default();
        let aviso = Arc::new(Notify::new());
        if let Some(mut ev) = canal.eventos() {
            let (estado, aviso, canal2) = (estado.clone(), aviso.clone(), canal.clone());
            tokio::spawn(async move {
                while let Some(m) = ev.recv().await {
                    if m["type"] == "request" {
                        // `runInTerminal` y demás: la consola es la interna (la salida llega por eventos)
                        let _ = canal2.responder_dap(&m, false, json!({"error": {"id": 1, "format": "no terminal"}})).await;
                        continue;
                    }
                    let b = &m["body"];
                    {
                        let mut e = estado.lock().unwrap();
                        match m["event"].as_str().unwrap_or("") {
                            "initialized" => e.inicializado = true,
                            "stopped" => {
                                e.detenido = Some(Detenido { hilo: b["threadId"].as_i64().unwrap_or(1), motivo: b["reason"].as_str().unwrap_or("").to_string(), texto: b["text"].as_str().or(b["description"].as_str()).unwrap_or("").to_string() });
                            }
                            "continued" => e.detenido = None,
                            "output" if b["category"] != "telemetry" => {
                                e.salida.push_str(b["output"].as_str().unwrap_or(""));
                                if e.salida.len() > MAX_SALIDA {
                                    let corte = e.salida.len() - MAX_SALIDA;
                                    let corte = (corte..e.salida.len()).find(|i| e.salida.is_char_boundary(*i)).unwrap_or(e.salida.len());
                                    e.salida.drain(..corte);
                                }
                            }
                            "exited" => e.terminado = Some(b["exitCode"].as_i64()),
                            "terminated" => {
                                if e.terminado.is_none() {
                                    e.terminado = Some(None);
                                }
                            }
                            _ => {}
                        }
                    }
                    aviso.notify_waiters();
                }
                // el adaptador se fue: cuenta como terminado
                let mut e = estado.lock().unwrap();
                if e.terminado.is_none() {
                    e.terminado = Some(None);
                }
                e.detenido = None;
                drop(e);
                aviso.notify_waiters();
            });
        }
        let capacidades = canal.pedir("initialize", json!({"clientID": "gotv", "clientName": "GOD OF THE VOID", "adapterID": id, "linesStartAt1": true, "columnsStartAt1": true, "pathFormat": "path", "supportsRunInTerminalRequest": false, "supportsVariableType": true}), Duration::from_secs(30)).await?;
        Ok(Sesion { canal, estado, aviso, adaptador: id.to_string(), capacidades, ultimo: Instant::now() })
    }

    /// Espera a que `listo` sea verdad (o pase `tope`).
    async fn esperar(&self, tope: Duration, listo: impl Fn(&Estado) -> bool) -> bool {
        let fin = Instant::now() + tope;
        loop {
            let espera = self.aviso.notified();
            if listo(&self.estado.lock().unwrap()) {
                return true;
            }
            let queda = fin.saturating_duration_since(Instant::now());
            if queda.is_zero() {
                return false;
            }
            let _ = tokio::time::timeout(queda.min(Duration::from_millis(250)), espera).await;
        }
    }

    async fn poner_puntos(&self, archivo: &Path, lineas: &[(u64, Option<String>)]) -> Result<String> {
        let pts: Vec<Value> = lineas.iter().map(|(l, c)| match c {
            Some(c) => json!({"line": l, "condition": c}),
            None => json!({"line": l}),
        }).collect();
        let r = self.canal.pedir("setBreakpoints", json!({"source": {"path": archivo.display().to_string()}, "breakpoints": pts}), Duration::from_secs(20)).await?;
        let mut v = Vec::new();
        for (i, b) in r["breakpoints"].as_array().into_iter().flatten().enumerate() {
            let pedida = lineas.get(i).map(|x| x.0).unwrap_or(0);
            let real = b["line"].as_u64().unwrap_or(pedida);
            v.push(match b["verified"].as_bool() {
                Some(true) if real != pedida => crate::tr!("{pedida}→{real} ✓", "{pedida}→{real} ✓"),
                Some(true) => format!("{real} ✓"),
                _ => crate::tr!("{pedida} ✗ ({})", "{pedida} ✗ ({})", b["message"].as_str().unwrap_or(crate::i18n::t("sin verificar", "unverified"))),
            });
        }
        Ok(v.join(", "))
    }

    /// Dónde está ahora el programa, para el modelo.
    async fn donde(&self, root: &Path) -> String {
        let (det, term, salida) = {
            let e = self.estado.lock().unwrap();
            (e.detenido.clone(), e.terminado, e.salida.clone())
        };
        let cola: String = {
            let lineas: Vec<&str> = salida.lines().collect();
            lineas[lineas.len().saturating_sub(12)..].join("\n")
        };
        let con_salida = |s: String| if cola.trim().is_empty() { s } else { crate::tr!("{s}\nsalida (lo último):\n{cola}", "{s}\noutput (the latest):\n{cola}") };
        // terminado manda (un adaptador que se cae estando parado ya no está parado)
        if let Some(c) = term {
            let codigo = c.map(|c| c.to_string()).unwrap_or_else(|| "?".into());
            return con_salida(crate::tr!("⏹ el programa TERMINÓ (código {codigo})", "⏹ the program FINISHED (exit code {codigo})"));
        }
        if let Some(d) = det {
            let marco = self.canal.pedir("stackTrace", json!({"threadId": d.hilo, "startFrame": 0, "levels": 1}), Duration::from_secs(10)).await.ok().and_then(|r| r["stackFrames"].get(0).cloned());
            let lugar = marco.map(|f| marco_texto(root, &f)).unwrap_or_default();
            let motivo = if d.texto.is_empty() { d.motivo.clone() } else { format!("{} — {}", d.motivo, d.texto) };
            return con_salida(crate::tr!("⏸ PARADO ({motivo}) en {lugar}", "⏸ STOPPED ({motivo}) at {lugar}"));
        }
        con_salida(crate::i18n::t("▶ el programa SIGUE corriendo (no paró en ningún punto todavía): `salida` para ver qué imprime, `detener` para pararlo", "▶ the program is STILL running (it has not stopped at any breakpoint yet): `salida` to see what it prints, `detener` to stop it").to_string())
    }

    fn hilo(&self) -> Result<i64> {
        let e = self.estado.lock().unwrap();
        if e.terminado.is_some() {
            bail!("{}", crate::i18n::t("el programa ya terminó: `iniciar` de nuevo (los puntos se conservan)", "the program already finished: `iniciar` again (breakpoints are kept)"));
        }
        e.detenido.as_ref().map(|d| d.hilo).ok_or_else(|| anyhow!("{}", crate::i18n::t("el programa no está parado: pon un `punto` y `continuar`, o espera a que llegue", "the program is not stopped: set a `punto` and `continuar`, or wait for it to get there")))
    }
}

/// `archivo:línea (función)` de un marco de la pila, relativo al proyecto.
fn marco_texto(root: &Path, f: &Value) -> String {
    let ruta = f["source"]["path"].as_str().unwrap_or("?");
    let raiz = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let rel = Path::new(ruta).strip_prefix(&raiz).or_else(|_| Path::new(ruta).strip_prefix(root)).map(|p| p.display().to_string()).unwrap_or_else(|_| ruta.to_string());
    format!("{rel}:{} ({})", f["line"].as_u64().unwrap_or(0), f["name"].as_str().unwrap_or("?"))
}

fn s<'a>(a: &'a Value, k: &str) -> Option<&'a str> {
    a[k].as_str().map(str::trim).filter(|x| !x.is_empty())
}

/// La herramienta `depurar`.
pub async fn ejecutar(root: &Path, args: &Value) -> Result<String> {
    let accion = s(args, "action").unwrap_or("estado");
    let pr = proyecto(root);
    let mut p = pr.lock().await;
    // inactividad: una sesión olvidada no se queda con el programa y el adaptador vivos
    if p.sesion.as_ref().is_some_and(|x| x.ultimo.elapsed() > INACTIVIDAD) {
        if let Some(x) = p.sesion.take() {
            let _ = x.canal.pedir("disconnect", json!({"terminateDebuggee": true}), Duration::from_secs(3)).await;
        }
        if accion != "iniciar" && accion != "punto" {
            return Ok(crate::i18n::t("la sesión de depuración se cerró sola tras 10 min sin uso; `iniciar` de nuevo (los puntos se conservan)", "the debugging session closed itself after 10 min without use; `iniciar` again (breakpoints are kept)").into());
        }
    }
    if let Some(x) = p.sesion.as_mut() {
        x.ultimo = Instant::now();
    }
    match accion {
        "punto" | "breakpoint" => {
            let archivo = crate::paths::resolve(root, s(args, "archivo").ok_or_else(|| anyhow!("{}", crate::i18n::t("falta `archivo`", "missing `archivo`")))?);
            let archivo = archivo.canonicalize().unwrap_or(archivo);
            let linea = args["linea"].as_u64().ok_or_else(|| anyhow!("{}", crate::i18n::t("falta `linea`", "missing `linea`")))?;
            let quitar = args["quitar"].as_bool().unwrap_or(false);
            let lista = p.puntos.entry(archivo.clone()).or_default();
            lista.retain(|(l, _)| *l != linea);
            if !quitar {
                lista.push((linea, s(args, "condicion").map(String::from)));
            }
            let lista = lista.clone();
            match &p.sesion {
                Some(x) => Ok(crate::tr!("puntos en {}: {}", "breakpoints in {}: {}", archivo.display(), x.poner_puntos(&archivo, &lista).await?)),
                None => Ok(crate::tr!("punto {} en {}:{linea} (se pone al `iniciar`)", "breakpoint {} at {}:{linea} (set on `iniciar`)", if quitar { crate::i18n::t("quitado", "removed") } else { crate::i18n::t("anotado", "noted") }, archivo.display())),
            }
        }
        "iniciar" | "start" => {
            if let Some(x) = p.sesion.take() {
                let _ = x.canal.pedir("disconnect", json!({"terminateDebuggee": true}), Duration::from_secs(3)).await;
            }
            let lista_args: Vec<String> = args["args"].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default();
            let cwd = s(args, "cwd").map(|c| crate::paths::resolve(root, c)).unwrap_or_else(|| root.to_path_buf());
            // una PRUEBA de Rust: su binario, filtrado por nombre y en un solo hilo
            let (programa, lista_args) = match (s(args, "prueba"), s(args, "programa")) {
                (Some(nombre), None) => {
                    let b = binario_de_pruebas(root).await?;
                    (b, vec![nombre.to_string(), "--nocapture".into(), "--test-threads=1".into()])
                }
                (_, Some(prog)) => (crate::paths::resolve(root, prog), lista_args),
                (None, None) => bail!("{}", crate::i18n::t("falta `programa` (el binario o el script) o `prueba` (el nombre de una prueba de Rust)", "missing `programa` (the binary or script) or `prueba` (the name of a Rust test)")),
            };
            if !programa.exists() {
                bail!("{}", crate::tr!("no existe {}: compílalo primero (con símbolos: el perfil de desarrollo de tu proyecto)", "{} does not exist: build it first (with symbols: your project's dev profile)", programa.display()));
            }
            let (cmd, id, extra) = adaptador(root, &programa)?;
            let sesion = Sesion::arrancar(root, cmd, id).await?;
            let mut lanzar = json!({"name": "gotv", "type": id, "request": "launch", "program": programa.display().to_string(), "args": lista_args, "cwd": cwd.display().to_string()});
            if let (Some(o), Some(e)) = (lanzar.as_object_mut(), extra.as_object()) {
                o.extend(e.clone());
            }
            // el `launch` puede no responder hasta después de `configurationDone`: va aparte
            let c = sesion.canal.clone();
            let t_lanzar = Instant::now();
            let lanzado = tokio::spawn(async move { c.pedir("launch", lanzar, Duration::from_secs(60)).await });
            sesion.esperar(Duration::from_secs(15), |e| e.inicializado || e.terminado.is_some()).await;
            let mut puestos = Vec::new();
            for (archivo, lineas) in &p.puntos {
                if !lineas.is_empty() {
                    puestos.push(format!("{}: {}", archivo.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(), sesion.poner_puntos(archivo, lineas).await.unwrap_or_else(|e| format!("✗ {e}"))));
                }
            }
            if sesion.capacidades["supportsConfigurationDoneRequest"] != false {
                let _ = sesion.canal.pedir("configurationDone", Value::Null, Duration::from_secs(20)).await;
            }
            match lanzado.await {
                Ok(Ok(_)) => {}
                // medido 2026-11-15: tras reiniciar la Mac, macOS vuelve a pedir permiso para que un
                // depurador controle otro programa; mientras nadie lo autoriza, `launch` no responde
                Ok(Err(e)) if id == "lldb-dap" && t_lanzar.elapsed() >= Duration::from_secs(59) => {
                    // el `debugserver` que lanzó se queda esperando ese permiso en su propia sesión
                    // (`--setsid`) y sobrevive al adaptador: se termina AHORA, mientras aún es su hijo
                    if let Some(pid) = sesion.canal.pid() {
                        matar_hijos(pid);
                    }
                    gotv_engine::bail_tr!(
                        "{e}. Lo más probable: macOS pide permiso para depurar (pasa tras reiniciar la Mac). Pide al usuario que acepte el diálogo «Herramientas de desarrollo» con su contraseña, o que active el modo desarrollador una vez con `sudo DevToolsSecurity -enable`; luego vuelve a iniciar",
                        "{e}. Most likely: macOS is asking for permission to debug (it happens after the Mac restarts). Ask the user to accept the «Developer Tools» dialog with their password, or to enable developer mode once with `sudo DevToolsSecurity -enable`; then start again"
                    )
                }
                Ok(Err(e)) => bail!("{e}"),
                Err(e) => bail!("{e}"),
            }
            sesion.esperar(ESPERA, |e| e.detenido.is_some() || e.terminado.is_some()).await;
            let donde = sesion.donde(root).await;
            let puntos = if puestos.is_empty() { crate::i18n::t("sin puntos de parada", "no breakpoints").to_string() } else { crate::tr!("puntos: {}", "breakpoints: {}", puestos.join(" · ")) };
            let t = crate::tr!("depurando {} con {} ({puntos})\n{donde}", "debugging {} with {} ({puntos})\n{donde}", programa.display(), sesion.adaptador);
            p.sesion = Some(sesion);
            Ok(t)
        }
        "detener" | "stop" => match p.sesion.take() {
            Some(x) => {
                let _ = x.canal.pedir("disconnect", json!({"terminateDebuggee": true}), Duration::from_secs(5)).await;
                Ok(crate::i18n::t("sesión de depuración cerrada (el programa y el adaptador terminaron)", "debugging session closed (the program and the adapter ended)").into())
            }
            None => Ok(crate::i18n::t("no había sesión de depuración", "there was no debugging session").into()),
        },
        otra => {
            let Some(x) = p.sesion.as_ref() else {
                return Ok(crate::i18n::t("no hay sesión de depuración: pon `punto`s e `iniciar`", "there is no debugging session: set `punto`s and `iniciar`").into());
            };
            match otra {
                "estado" | "status" => Ok(x.donde(root).await),
                "salida" | "output" => {
                    let t = x.estado.lock().unwrap().salida.clone();
                    Ok(if t.trim().is_empty() { crate::i18n::t("(el programa no ha escrito nada)", "(the program has written nothing)").into() } else { t })
                }
                "continuar" | "continue" | "paso" | "step" => {
                    let hilo = x.hilo()?;
                    let cmd = match (otra, s(args, "tipo").unwrap_or("saltar")) {
                        ("continuar" | "continue", _) => "continue",
                        (_, "entrar" | "in") => "stepIn",
                        (_, "salir" | "out") => "stepOut",
                        _ => "next",
                    };
                    x.estado.lock().unwrap().detenido = None;
                    x.canal.pedir(cmd, json!({"threadId": hilo}), Duration::from_secs(20)).await?;
                    x.esperar(ESPERA, |e| e.detenido.is_some() || e.terminado.is_some()).await;
                    Ok(x.donde(root).await)
                }
                "pila" | "stack" => {
                    let hilo = x.hilo()?;
                    let r = x.canal.pedir("stackTrace", json!({"threadId": hilo, "startFrame": 0, "levels": 20}), Duration::from_secs(10)).await?;
                    let marcos: Vec<String> = r["stackFrames"].as_array().into_iter().flatten().enumerate().map(|(i, f)| format!("#{i} {}", marco_texto(root, f))).collect();
                    Ok(marcos.join("\n"))
                }
                "variables" | "evaluar" | "evaluate" => {
                    let hilo = x.hilo()?;
                    let n = args["marco"].as_u64().unwrap_or(0) as usize;
                    let r = x.canal.pedir("stackTrace", json!({"threadId": hilo, "startFrame": 0, "levels": n + 1}), Duration::from_secs(10)).await?;
                    let marco = r["stackFrames"].get(n).ok_or_else(|| anyhow!("{}", crate::tr!("no hay marco {n} en la pila", "there is no frame {n} in the stack")))?;
                    let id = marco["id"].clone();
                    if otra != "variables" {
                        let e = s(args, "expresion").ok_or_else(|| anyhow!("{}", crate::i18n::t("falta `expresion`", "missing `expresion`")))?;
                        // `watch`: el valor limpio (en `repl`, lldb devuelve el texto de su consola: «(int) $0 = 6»)
                        let r = x.canal.pedir("evaluate", json!({"expression": e, "frameId": id, "context": "watch"}), Duration::from_secs(20)).await?;
                        let tipo = r["type"].as_str().map(|t| format!(" ({t})")).unwrap_or_default();
                        return Ok(format!("{e} = {}{tipo}", r["result"].as_str().unwrap_or("")));
                    }
                    let scopes = x.canal.pedir("scopes", json!({"frameId": id}), Duration::from_secs(10)).await?;
                    let mut out = vec![crate::tr!("en {}:", "at {}:", marco_texto(root, marco))];
                    for sc in scopes["scopes"].as_array().into_iter().flatten() {
                        // los registros y lo «caro» (globales enteras) no se vuelcan: sólo el primer nivel de lo local
                        if sc["expensive"] == true || sc["name"].as_str().is_some_and(|n| n.to_lowercase().contains("regist")) {
                            continue;
                        }
                        let vars = x.canal.pedir("variables", json!({"variablesReference": sc["variablesReference"]}), Duration::from_secs(10)).await?;
                        out.push(format!("[{}]", sc["name"].as_str().unwrap_or("?")));
                        for v in vars["variables"].as_array().into_iter().flatten().take(60) {
                            let valor: String = v["value"].as_str().unwrap_or("").chars().take(200).collect();
                            let tipo = v["type"].as_str().filter(|t| !t.is_empty()).map(|t| format!(": {t}")).unwrap_or_default();
                            let mas = if v["variablesReference"].as_i64().unwrap_or(0) > 0 { crate::i18n::t(" (+ tiene dentro: `evaluar` para verlo)", " (+ has inner values: `evaluar` to see them)") } else { "" };
                            out.push(format!("  {}{tipo} = {valor}{mas}", v["name"].as_str().unwrap_or("?")));
                        }
                    }
                    Ok(out.join("\n"))
                }
                _ => bail!("{}", crate::tr!("acción desconocida «{otra}»: punto · iniciar · continuar · paso · pila · variables · evaluar · salida · estado · detener", "unknown action «{otra}»: punto · iniciar · continuar · paso · pila · variables · evaluar · salida · estado · detener")),
            }
        }
    }
}

/// Las sesiones que llevan más de 10 min sin uso se cierran (lo llama el agente de vez en cuando).
pub async fn podar_inactivas() {
    let todos: Vec<Arc<Mutex<Proyecto>>> = proyectos().lock().unwrap().values().cloned().collect();
    for pr in todos {
        let mut p = pr.lock().await;
        if p.sesion.as_ref().is_some_and(|x| x.ultimo.elapsed() > INACTIVIDAD) {
            if let Some(x) = p.sesion.take() {
                let _ = x.canal.pedir("disconnect", json!({"terminateDebuggee": true}), Duration::from_secs(3)).await;
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod falso {
    //! Un adaptador DAP FALSO, dentro del proceso: un «programa» con un bucle cuya variable `i` vale 3 en el
    //! punto de la línea 5; responde las peticiones de verdad del protocolo y emite sus eventos.
    use super::*;
    use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

    async fn leer<R: tokio::io::AsyncRead + Unpin>(r: &mut BufReader<R>) -> Option<Value> {
        let mut largo = 0;
        loop {
            let mut l = String::new();
            if r.read_line(&mut l).await.ok()? == 0 {
                return None;
            }
            if l.trim().is_empty() {
                break;
            }
            if let Some(v) = l.trim().strip_prefix("Content-Length:") {
                largo = v.trim().parse().ok()?;
            }
        }
        let mut b = vec![0; largo];
        r.read_exact(&mut b).await.ok()?;
        serde_json::from_slice(&b).ok()
    }

    async fn enviar<W: tokio::io::AsyncWrite + Unpin>(w: &mut W, v: Value) {
        let t = v.to_string();
        let _ = w.write_all(format!("Content-Length: {}\r\n\r\n{t}", t.len()).as_bytes()).await;
        let _ = w.flush().await;
    }

    /// `caer_en`: el adaptador se cae al recibir esa orden.
    pub async fn servir<R: tokio::io::AsyncRead + Unpin, W: tokio::io::AsyncWrite + Unpin>(r: R, mut w: W, caer_en: Option<&'static str>) {
        let mut r = BufReader::new(r);
        let mut seq = 1000;
        let mut punto: Option<u64> = None;
        let mut parado = false;
        while let Some(m) = leer(&mut r).await {
            let cmd = m["command"].as_str().unwrap_or("").to_string();
            if Some(cmd.as_str()) == caer_en {
                return;
            }
            seq += 1;
            let ok = |cuerpo: Value| json!({"seq": seq, "type": "response", "request_seq": m["seq"], "command": cmd, "success": true, "body": cuerpo});
            let resp = match cmd.as_str() {
                "initialize" => ok(json!({"supportsConfigurationDoneRequest": true, "supportsConditionalBreakpoints": true})),
                "launch" => {
                    enviar(&mut w, ok(json!({}))).await;
                    enviar(&mut w, json!({"seq": 1, "type": "event", "event": "initialized"})).await;
                    continue;
                }
                "setBreakpoints" => {
                    punto = m["arguments"]["breakpoints"][0]["line"].as_u64();
                    let bs: Vec<Value> = m["arguments"]["breakpoints"].as_array().unwrap().iter().map(|b| json!({"verified": b["line"] != 99, "line": b["line"], "message": "línea sin código"})).collect(); // juez-idioma: datos (adaptador falso de PRUEBAS)
                    ok(json!({"breakpoints": bs}))
                }
                "configurationDone" => {
                    enviar(&mut w, ok(json!({}))).await;
                    enviar(&mut w, json!({"seq": 2, "type": "event", "event": "output", "body": {"category": "stdout", "output": "empezando\n"}})).await;
                    match punto {
                        Some(_) => {
                            parado = true;
                            enviar(&mut w, json!({"seq": 3, "type": "event", "event": "stopped", "body": {"reason": "breakpoint", "threadId": 7}})).await;
                        }
                        None => {
                            enviar(&mut w, json!({"seq": 3, "type": "event", "event": "exited", "body": {"exitCode": 0}})).await;
                            enviar(&mut w, json!({"seq": 4, "type": "event", "event": "terminated"})).await;
                        }
                    }
                    continue;
                }
                "stackTrace" => ok(json!({"stackFrames": [{"id": 1, "name": "bucle", "line": punto.unwrap_or(1), "source": {"path": "/p/src/main.rs"}}, {"id": 2, "name": "main", "line": 12, "source": {"path": "/p/src/main.rs"}}]})),
                "scopes" => ok(json!({"scopes": [{"name": "Locals", "variablesReference": 10, "expensive": false}, {"name": "Registers", "variablesReference": 11, "expensive": false}]})),
                "variables" => ok(json!({"variables": [{"name": "i", "value": "3", "type": "i32", "variablesReference": 0}, {"name": "v", "value": "size=4", "type": "Vec<i32>", "variablesReference": 12}]})),
                "evaluate" => ok(json!({"result": if m["arguments"]["expression"] == "i * 2" { "6" } else { "?" }, "type": "i32"})),
                "continue" | "next" => {
                    enviar(&mut w, ok(json!({}))).await;
                    if parado {
                        parado = false;
                        enviar(&mut w, json!({"seq": 5, "type": "event", "event": "exited", "body": {"exitCode": 1}})).await;
                        enviar(&mut w, json!({"seq": 6, "type": "event", "event": "terminated"})).await;
                    }
                    continue;
                }
                "disconnect" => {
                    enviar(&mut w, ok(json!({}))).await;
                    return;
                }
                _ => json!({"seq": seq, "type": "response", "request_seq": m["seq"], "command": cmd, "success": false, "message": "no sé"}),
            };
            enviar(&mut w, resp).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Una sesión sobre el adaptador falso, sin lanzar nada (como el LSP falso).
    async fn con_falso(root: &Path, caer_en: Option<&'static str>) {
        let (a, b) = tokio::io::duplex(1 << 16);
        let (ar, aw) = tokio::io::split(a);
        let (br, bw) = tokio::io::split(b);
        tokio::spawn(falso::servir(br, bw, caer_en));
        let canal = Arc::new(Canal::sobre_con(ar, aw, Marco::ContentLength, Protocolo::Dap, "falso"));
        let s = Sesion::sobre(canal, "falso").await.unwrap();
        proyecto(root).lock().await.sesion = Some(s);
    }

    /// La secuencia del protocolo contra el falso: puntos, launch → initialized → setBreakpoints →
    /// configurationDone, parada, pila, variables (sin registros), evaluar, continuar hasta el final.
    #[tokio::test]
    async fn la_sesion_completa_contra_el_adaptador_falso() {
        let d = crate::CarpetaPrueba::nueva("gotv-depurar-falso");
        std::fs::create_dir_all(d.join("src")).unwrap();
        std::fs::write(d.join("src/main.rs"), "fn main() {}\n").unwrap();
        std::fs::write(d.join("programa"), "").unwrap();
        let raiz = d.to_path_buf();
        let a = move |v: Value| {
            let r = raiz.clone();
            async move { ejecutar(&r, &v).await }
        };
        // un punto ANTES de iniciar se anota
        let t = a(json!({"action": "punto", "archivo": "src/main.rs", "linea": 5})).await.unwrap();
        assert!(t.contains("anotado") || t.contains("set on"), "{t}");
        // el canal se monta sobre el falso dentro del proceso; `iniciar` con un adaptador real lo prueba lldb-dap
        con_falso(&d, None).await;
        {
            // lo que haría `iniciar` tras arrancar el adaptador
            let pr = proyecto(&d);
            let p = pr.lock().await;
            let x = p.sesion.as_ref().unwrap();
            let c = x.canal.clone();
            let l = tokio::spawn(async move { c.pedir("launch", json!({"program": "programa"}), Duration::from_secs(5)).await });
            assert!(x.esperar(Duration::from_secs(5), |e| e.inicializado).await);
            let (archivo, lineas) = p.puntos.iter().next().unwrap();
            assert_eq!(x.poner_puntos(archivo, lineas).await.unwrap(), "5 ✓");
            x.canal.pedir("configurationDone", Value::Null, Duration::from_secs(5)).await.unwrap();
            l.await.unwrap().unwrap();
            assert!(x.esperar(Duration::from_secs(5), |e| e.detenido.is_some()).await);
        }
        let t = a(json!({"action": "estado"})).await.unwrap();
        assert!(t.contains("PARADO") || t.contains("STOPPED"), "{t}");
        assert!(t.contains("main.rs:5 (bucle)") && t.contains("empezando"), "{t}");
        let t = a(json!({"action": "pila"})).await.unwrap();
        assert!(t.contains("#0 /p/src/main.rs:5 (bucle)") && t.contains("#1 /p/src/main.rs:12 (main)"), "{t}");
        let t = a(json!({"action": "variables"})).await.unwrap();
        assert!(t.contains("i: i32 = 3") && t.contains("v: Vec<i32> = size=4") && !t.contains("Registers"), "{t}");
        assert_eq!(a(json!({"action": "evaluar", "expresion": "i * 2"})).await.unwrap(), "i * 2 = 6 (i32)");
        let t = a(json!({"action": "continuar"})).await.unwrap();
        assert!(t.contains("TERMINÓ (código 1)") || t.contains("FINISHED (exit code 1)"), "{t}");
        assert!(a(json!({"action": "variables"})).await.unwrap_err().to_string().contains("terminó") || a(json!({"action": "variables"})).await.unwrap_err().to_string().contains("finished"));
        let t = a(json!({"action": "detener"})).await.unwrap();
        assert!(t.contains("cerrada") || t.contains("closed"), "{t}");
        assert!(a(json!({"action": "pila"})).await.unwrap().contains("no hay sesión") || a(json!({"action": "pila"})).await.unwrap().contains("no debugging session"));
    }

    /// Un adaptador que se cae: la sesión lo dice (terminado), nada se cuelga.
    #[tokio::test]
    async fn el_adaptador_que_se_cae() {
        let d = crate::CarpetaPrueba::nueva("gotv-depurar-cae");
        con_falso(&d, Some("stackTrace")).await;
        {
            let pr = proyecto(&d);
            let p = pr.lock().await;
            p.sesion.as_ref().unwrap().estado.lock().unwrap().detenido = Some(Detenido { hilo: 7, motivo: "breakpoint".into(), texto: String::new() });
        }
        let t = tokio::time::timeout(Duration::from_secs(15), ejecutar(&d, &json!({"action": "pila"}))).await.expect("no se cuelga");
        assert!(t.is_err(), "{t:?}");
        let t = ejecutar(&d, &json!({"action": "estado"})).await.unwrap();
        assert!(t.contains("TERMINÓ") || t.contains("FINISHED"), "{t}");
    }

    /// Tras 10 min sin uso la sesión se cierra sola (con el reloj adelantado).
    #[tokio::test]
    async fn el_tope_de_inactividad() {
        let d = crate::CarpetaPrueba::nueva("gotv-depurar-ocio");
        con_falso(&d, None).await;
        proyecto(&d).lock().await.sesion.as_mut().unwrap().ultimo = Instant::now() - INACTIVIDAD - Duration::from_secs(1);
        let t = ejecutar(&d, &json!({"action": "estado"})).await.unwrap();
        assert!(t.contains("10 min"), "{t}");
        assert!(proyecto(&d).lock().await.sesion.is_none());
    }

    /// El arranque real con lldb-dap, salvo que macOS aún no deje depurar (tras reiniciar pide su
    /// permiso con contraseña): entonces la prueba no aplica, como sin Xcode, y lo dice.
    fn sin_permiso_de_macos(r: anyhow::Result<String>) -> Option<String> {
        match r {
            Err(e) if e.to_string().contains("DevToolsSecurity") => {
                eprintln!("⚠ no aplica: macOS aún no autorizó depurar en esta sesión ({e})");
                None
            }
            r => Some(r.unwrap()),
        }
    }

    /// F15.2 paso 8 · depurar UNA prueba de Rust: `cargo test --no-run` da su binario y el depurador para
    /// dentro de la prueba con su valor.
    #[tokio::test]
    async fn una_prueba_de_rust_con_lldb_dap() {
        let guarda = crate::CarpetaPrueba::nueva("gotv-depurar-prueba");
        let d = guarda.canonicalize().unwrap();
        std::fs::create_dir_all(d.join("src")).unwrap();
        std::fs::write(d.join("Cargo.toml"), "[package]\nname = \"juguete\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[workspace]\n").unwrap();
        std::fs::write(d.join("src/lib.rs"), "pub fn triple(x: i32) -> i32 {\n    x * 3\n}\n\n#[cfg(test)]\nmod tests {\n    #[test]\n    fn triple_de_siete() {\n        let r = super::triple(7);\n        assert_eq!(r, 21);\n    }\n}\n").unwrap();
        if adaptador(&d, &d.join("x")).is_err() || !crate::entorno::en_path("cargo") {
            return; // sin Xcode o sin cargo: no aplica
        }
        // el juguete se compila en SU carpeta: con un CARGO_TARGET_DIR heredado (el de quien corre las
        // pruebas) su binario caía en una carpeta compartida con corridas anteriores (mismo nombre, otra
        // ruta) y el depurador ponía el punto sobre un binario ajeno (medido 2026-11-17)
        std::env::remove_var("CARGO_TARGET_DIR");
        let raiz = d.clone();
        let a = move |v: Value| {
            let r = raiz.clone();
            async move { ejecutar(&r, &v).await }
        };
        a(json!({"action": "punto", "archivo": "src/lib.rs", "linea": 10})).await.unwrap();
        let Some(t) = sin_permiso_de_macos(a(json!({"action": "iniciar", "prueba": "triple_de_siete"})).await) else { return };
        assert!(t.contains("lib.rs:10") && (t.contains("PARADO") || t.contains("STOPPED")), "{t}");
        let t = a(json!({"action": "evaluar", "expresion": "r"})).await.unwrap();
        assert!(t.starts_with("r = 21"), "{t}");
        a(json!({"action": "detener"})).await.unwrap();
    }

    /// ML de verdad: `lldb-dap` de Xcode con un binario de Rust compilado con símbolos. Un valor
    /// conocido en una variable a mitad de un bucle.
    #[tokio::test]
    async fn lldb_dap_de_verdad() {
        let guarda = crate::CarpetaPrueba::nueva("gotv-depurar-lldb");
        let d = guarda.canonicalize().unwrap();
        let fuente = "fn main() {\n    let mut suma = 0;\n    for i in 0..5 {\n        let doble = i * 2;\n        suma += doble;\n    }\n    println!(\"{suma}\");\n}\n";
        std::fs::write(d.join("bucle.rs"), fuente).unwrap();
        let rustc = std::process::Command::new("rustc").args(["-g", "-o", "bucle", "bucle.rs"]).current_dir(&d).status();
        if !rustc.is_ok_and(|s| s.success()) || adaptador(&d, &d.join("bucle")).is_err() {
            return; // sin rustc o sin Xcode: no aplica
        }
        let raiz = d.to_path_buf();
        let a = move |v: Value| {
            let r = raiz.clone();
            async move { ejecutar(&r, &v).await }
        };
        a(json!({"action": "punto", "archivo": "bucle.rs", "linea": 5, "condicion": "i == 3"})).await.unwrap();
        let Some(t) = sin_permiso_de_macos(a(json!({"action": "iniciar", "programa": "bucle"})).await) else { return };
        assert!(t.contains("bucle.rs:5") && (t.contains("PARADO") || t.contains("STOPPED")), "{t}");
        let t = a(json!({"action": "variables"})).await.unwrap();
        assert!(t.contains("i") && t.contains("= 3") && t.contains("doble") && t.contains("= 6"), "{t}");
        let t = a(json!({"action": "evaluar", "expresion": "suma"})).await.unwrap();
        assert!(t.starts_with("suma = 6"), "0+2+4 = 6 antes de sumar el doble de 3: {t}");
        let t = a(json!({"action": "continuar"})).await.unwrap();
        assert!(t.contains("TERMINÓ (código 0)") || t.contains("FINISHED (exit code 0)"), "{t}");
        assert!(t.contains("20"), "la salida del programa (0+2+4+6+8): {t}");
        a(json!({"action": "detener"})).await.unwrap();
    }

    /// Python de verdad con `debugpy`: el mismo bucle que el de lldb, por el `.venv` del proyecto. El
    /// entorno con debugpy lo da `GOTV_PRUEBA_DEBUGPY` (la carpeta de un venv); sin ella no aplica.
    #[tokio::test]
    async fn debugpy_de_verdad() {
        let Some(venv) = std::env::var_os("GOTV_PRUEBA_DEBUGPY").map(PathBuf::from).filter(|v| v.join("bin/python").is_file()) else {
            return;
        };
        let guarda = crate::CarpetaPrueba::nueva("gotv-depurar-debugpy");
        let d = guarda.canonicalize().unwrap();
        std::os::unix::fs::symlink(&venv, d.join(".venv")).unwrap();
        let fuente = "suma = 0\nfor i in range(5):\n    doble = i * 2\n    suma += doble\nprint(suma)\n";
        std::fs::write(d.join("bucle.py"), fuente).unwrap();
        let raiz = d.clone();
        let a = move |v: Value| {
            let r = raiz.clone();
            async move { ejecutar(&r, &v).await }
        };
        a(json!({"action": "punto", "archivo": "bucle.py", "linea": 4, "condicion": "i == 3"})).await.unwrap();
        let t = a(json!({"action": "iniciar", "programa": "bucle.py"})).await.unwrap();
        assert!(t.contains("bucle.py:4") && (t.contains("PARADO") || t.contains("STOPPED")), "{t}");
        let t = a(json!({"action": "variables"})).await.unwrap();
        assert!(t.contains("i") && t.contains("= 3") && t.contains("doble") && t.contains("= 6"), "{t}");
        let t = a(json!({"action": "evaluar", "expresion": "suma"})).await.unwrap();
        assert!(t.starts_with("suma = 6"), "0+2+4 = 6 antes de sumar el doble de 3: {t}");
        let t = a(json!({"action": "continuar"})).await.unwrap();
        assert!(t.contains("TERMINÓ (código 0)") || t.contains("FINISHED (exit code 0)"), "{t}");
        assert!(t.contains("20"), "la salida del programa (0+2+4+6+8): {t}");
        a(json!({"action": "detener"})).await.unwrap();
    }

    /// Sin debugpy, el error dice qué falta y cómo ponerlo, en vez de un adaptador que muere mudo.
    #[test]
    fn sin_debugpy_lo_dice() {
        let guarda = crate::CarpetaPrueba::nueva("gotv-depurar-sin-debugpy");
        let d = guarda.canonicalize().unwrap();
        std::fs::create_dir_all(d.join(".venv/bin")).unwrap();
        // un «python» que no tiene nada: falla todo import
        std::fs::write(d.join(".venv/bin/python"), "#!/bin/sh\nexit 1\n").unwrap();
        std::fs::set_permissions(d.join(".venv/bin/python"), std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        let e = adaptador(&d, &d.join("x.py")).unwrap_err().to_string();
        assert!(e.contains("debugpy") && e.contains("pip install"), "{e}");
    }
}
