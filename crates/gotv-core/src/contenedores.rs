//! F19 · Contenedores y servicios del proyecto: levantar la base o el servicio que el proyecto necesita,
//! leer sus registros y saber cuándo está SANO, sin abrir un hueco en la caja.
//!
//! GOTV habla con la API del motor por su socket desde Rust (`bollard`), FUERA de la caja y con sus propios
//! frenos; los comandos de la caja siguen sin alcanzar el socket (F14.0).
//! - **El socket se le pregunta al motor, no se supone** (medido en esta Mac: dos perfiles de Colima y el
//!   contexto activo NO es el de por omisión): `DOCKER_HOST`, o el contexto activo de `~/.docker/config.json`
//!   con su `meta.json`, o los sitios conocidos (Docker Desktop, Colima, OrbStack, Podman).
//! - **La configuración de compose la normaliza `docker compose config --format json`** (el binario nativo:
//!   interpolación, `extends`, rutas absolutas): los frenos miran lo que de verdad se montaría. Si `docker
//!   compose` no está, se dice; no se instala.
//! - **Frenos fijos** (también en bypass): nada de montar `/`, la carpeta personal, una ruta de credenciales
//!   (F14.1) ni el socket del motor; nada de modo privilegiado ni `pid`/`network` del anfitrión. Montar algo
//!   FUERA del proyecto se pregunta siempre (`permitir_fuera`).
//! - **Sólo se tocan los contenedores DEL PROYECTO** (por la etiqueta de compose con su carpeta); los demás,
//!   ni se listan.
//! - Las variables de entorno de un servicio se enmascaran SIEMPRE (las que parecen secretas).
use anyhow::{anyhow, bail, Result};
use bollard::Docker;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::time::Duration;

fn casa() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_default())
}

/// El socket del motor: de `DOCKER_HOST`, del contexto activo, o de los sitios conocidos.
pub fn socket() -> Result<PathBuf> {
    socket_en(&casa(), std::env::var("DOCKER_HOST").ok().as_deref())
}

pub fn socket_en(casa: &Path, docker_host: Option<&str>) -> Result<PathBuf> {
    if let Some(h) = docker_host.filter(|h| !h.is_empty()) {
        return h.strip_prefix("unix://").map(PathBuf::from).ok_or_else(|| anyhow!("{}", crate::tr!("DOCKER_HOST={h}: sólo se admite un socket unix://", "DOCKER_HOST={h}: only a unix:// socket is supported")));
    }
    // el contexto activo (`docker context use …` lo escribe aquí)
    let cfg: Value = std::fs::read_to_string(casa.join(".docker/config.json")).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    if let Some(ctx) = cfg["currentContext"].as_str().filter(|c| !c.is_empty() && *c != "default") {
        use sha2::{Digest, Sha256};
        let meta = casa.join(".docker/contexts/meta").join(format!("{:x}", Sha256::digest(ctx.as_bytes()))).join("meta.json");
        let m: Value = std::fs::read_to_string(&meta).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        if let Some(p) = m["Endpoints"]["docker"]["Host"].as_str().and_then(|h| h.strip_prefix("unix://")) {
            return Ok(PathBuf::from(p));
        }
    }
    let candidatos = [
        casa.join(".colima/default/docker.sock"),
        casa.join(".orbstack/run/docker.sock"),
        casa.join(".docker/run/docker.sock"),
        PathBuf::from("/var/run/docker.sock"),
        casa.join(".local/share/containers/podman/machine/podman.sock"),
    ];
    candidatos.into_iter().find(|p| p.exists()).ok_or_else(|| {
        anyhow!("{}", crate::i18n::t("no hallé el motor de contenedores (Docker Desktop, Colima, OrbStack o Podman): ¿está encendido?", "could not find the container engine (Docker Desktop, Colima, OrbStack or Podman): is it running?"))
    })
}

async fn motor() -> Result<(Docker, PathBuf)> {
    let s = socket()?;
    let d = Docker::connect_with_unix(&s.display().to_string(), 30, bollard::API_DEFAULT_VERSION).map_err(|e| anyhow!("{}: {e}", s.display()))?;
    Ok((d, s))
}

/// El archivo de compose del proyecto (los nombres que busca el propio `docker compose`).
pub fn archivo_compose(root: &Path) -> Option<PathBuf> {
    ["compose.yaml", "compose.yml", "docker-compose.yml", "docker-compose.yaml"].iter().map(|n| root.join(n)).find(|p| p.is_file())
}

/// La configuración NORMALIZADA (`docker compose config --format json`). `None` si no hay compose.
pub fn compose_json(root: &Path) -> Result<Option<Value>> {
    if archivo_compose(root).is_none() {
        return Ok(None);
    }
    let o = crate::entorno::comando("docker")
        .args(["compose", "--project-directory"])
        .arg(root)
        .args(["config", "--format", "json"])
        .current_dir(root)
        .output()
        .map_err(|e| anyhow!("{}", crate::tr!("no pude correr `docker compose` ({e}): si falta, instálalo con Docker Desktop, Colima u OrbStack (GOTV no lo instala)", "could not run `docker compose` ({e}): if missing, install it with Docker Desktop, Colima or OrbStack (GOTV does not install it)")))?;
    if !o.status.success() {
        bail!("docker compose config: {}", String::from_utf8_lossy(&o.stderr).trim().chars().take(600).collect::<String>());
    }
    Ok(Some(serde_json::from_slice(&o.stdout)?))
}

/// F19 · los frenos fijos sobre la configuración normalizada. `Err` = se niega (con el motivo);
/// `Ok(fuera)` = montajes fuera del proyecto, que se preguntan.
pub fn frenos(cfg: &Value, root: &Path, socket: &Path) -> Result<Vec<String>> {
    let raiz = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let casa = casa().canonicalize().unwrap_or_else(|_| casa());
    let credenciales = crate::sandbox::credenciales(&raiz, &[]);
    let mut fuera = Vec::new();
    for (svc, s) in cfg["services"].as_object().into_iter().flatten() {
        if s["privileged"].as_bool() == Some(true) {
            bail!("{}", crate::tr!("«{svc}» pide modo privilegiado: GOTV no lo levanta (es dar el Mac entero al contenedor)", "«{svc}» asks for privileged mode: GOTV will not start it (it hands the whole Mac to the container)"));
        }
        for k in ["pid", "network_mode", "ipc"] {
            if s[k].as_str() == Some("host") {
                bail!("{}", crate::tr!("«{svc}» pide `{k}: host`: GOTV no lo levanta", "«{svc}» asks for `{k}: host`: GOTV will not start it"));
            }
        }
        for v in s["volumes"].as_array().into_iter().flatten() {
            if v["type"].as_str() != Some("bind") {
                continue;
            }
            let Some(origen) = v["source"].as_str() else { continue };
            let p = PathBuf::from(origen);
            let real = p.canonicalize().unwrap_or_else(|_| p.clone());
            let motivo = if real == Path::new("/") {
                Some(crate::i18n::t("monta la raíz del disco", "mounts the disk root").to_string())
            } else if real == casa {
                Some(crate::i18n::t("monta tu carpeta personal entera", "mounts your whole home folder").to_string())
            } else if real.ends_with("docker.sock") || real == socket || origen.ends_with(".sock") && origen.contains("docker") {
                Some(crate::i18n::t("monta el socket del motor (sería mandar en todos los contenedores)", "mounts the engine socket (it would command every container)").to_string())
            } else if credenciales.iter().any(|c| real.starts_with(c) || c.starts_with(&real)) {
                Some(crate::i18n::t("monta credenciales (llaves, tokens)", "mounts credentials (keys, tokens)").to_string())
            } else {
                None
            };
            if let Some(m) = motivo {
                bail!("{}", crate::tr!("«{svc}» {m} ({origen}): GOTV no lo levanta", "«{svc}» {m} ({origen}): GOTV will not start it"));
            }
            if !real.starts_with(&raiz) {
                fuera.push(format!("{svc}: {origen}"));
            }
        }
    }
    Ok(fuera)
}

/// Los contenedores de ESTE proyecto (etiqueta de compose con su carpeta).
async fn del_proyecto(d: &Docker, root: &Path) -> Result<Vec<bollard::models::ContainerSummary>> {
    // compose etiqueta la carpeta TAL COMO se le dio (medido: `/var/folders/…`, no `/private/var/…`): se
    // pregunta por las dos formas (varios valores de una etiqueta en un filtro se suman con Y, no con O)
    let mut formas = vec![root.display().to_string()];
    if let Ok(c) = root.canonicalize() {
        formas.push(c.display().to_string());
    }
    formas.dedup();
    let mut out: Vec<bollard::models::ContainerSummary> = Vec::new();
    for forma in formas {
        let mut f = std::collections::HashMap::new();
        f.insert("label", vec![format!("com.docker.compose.project.working_dir={forma}")]);
        let o = bollard::query_parameters::ListContainersOptionsBuilder::new().all(true).filters(&f).build();
        for c in d.list_containers(Some(o)).await? {
            if !out.iter().any(|x| x.id == c.id) {
                out.push(c);
            }
        }
    }
    Ok(out)
}

fn texto<T: serde::Serialize>(x: &Option<T>) -> String {
    x.as_ref().and_then(|v| serde_json::to_value(v).ok()).and_then(|v| v.as_str().map(String::from)).unwrap_or_default()
}

fn servicio_de(c: &bollard::models::ContainerSummary) -> String {
    c.labels.as_ref().and_then(|l| l.get("com.docker.compose.service").cloned()).unwrap_or_default()
}

/// El contenedor de un servicio del proyecto (o error claro).
async fn contenedor(d: &Docker, root: &Path, servicio: &str) -> Result<bollard::models::ContainerSummary> {
    let todos = del_proyecto(d, root).await?;
    let nombres: Vec<String> = todos.iter().map(servicio_de).collect();
    todos.into_iter().find(|c| servicio_de(c) == servicio).ok_or_else(|| {
        anyhow!("{}", crate::tr!("«{servicio}» no tiene contenedor en este proyecto (hay: {}); ¿falta `levantar`?", "«{servicio}» has no container in this project (there are: {}); missing `levantar`?", if nombres.is_empty() { "—".into() } else { nombres.join(", ") }))
    })
}

fn puertos(c: &bollard::models::ContainerSummary) -> String {
    let mut v: Vec<String> = c.ports.iter().flatten().filter_map(|p| p.public_port.map(|pp| format!("{pp}→{}", p.private_port))).collect();
    v.sort();
    v.dedup();
    v.join(", ")
}

/// «10m», «2h», «30s» → segundos.
fn segundos(s: &str) -> Option<i64> {
    let s = s.trim();
    let (n, u) = s.split_at(s.find(|c: char| !c.is_ascii_digit())?);
    let n: i64 = n.parse().ok()?;
    Some(match u {
        "s" => n,
        "m" => n * 60,
        "h" => n * 3600,
        "d" => n * 86_400,
        _ => return None,
    })
}

/// Variables de entorno con las secretas enmascaradas (siempre).
pub fn enmascarar(env: &[String]) -> Vec<String> {
    env.iter()
        .map(|kv| match kv.split_once('=') {
            Some((k, v)) if crate::sandbox::variable_secreta(k) || (v.contains("://") && v.contains('@')) => format!("{k}=«oculto»"),
            _ => kv.clone(),
        })
        .collect()
}

/// F19 paso 5: si lo que se levantó es una base de datos con puerto publicado, cómo consultarla con F1.
fn propuesta_bd(svc: &str, s: &Value, puerto: Option<u16>) -> Option<String> {
    let img = s["image"].as_str().unwrap_or("").to_lowercase();
    let env = &s["environment"];
    let (esquema, usuario, base, defecto) = if img.contains("postgres") {
        ("postgres", env["POSTGRES_USER"].as_str().unwrap_or("postgres"), env["POSTGRES_DB"].as_str().or(env["POSTGRES_USER"].as_str()).unwrap_or("postgres"), 5432)
    } else if img.contains("mysql") || img.contains("mariadb") {
        ("mysql", env["MYSQL_USER"].as_str().or(env["MARIADB_USER"].as_str()).unwrap_or("root"), env["MYSQL_DATABASE"].as_str().or(env["MARIADB_DATABASE"].as_str()).unwrap_or(""), 3306)
    } else {
        return None;
    };
    let p = puerto.unwrap_or(defecto);
    Some(crate::tr!(
        "· «{svc}» es una base: base_datos action=agregar nombre={svc} url={esquema}://{usuario}@localhost:{p}/{base} (y su clave con action=secreto)",
        "· «{svc}» is a database: base_datos action=agregar nombre={svc} url={esquema}://{usuario}@localhost:{p}/{base} (and its password with action=secreto)"
    ))
}

fn compose(root: &Path, socket: &Path) -> std::process::Command {
    let mut k = crate::entorno::comando("docker");
    k.args(["compose", "--project-directory"]).arg(root).current_dir(root).env("DOCKER_HOST", format!("unix://{}", socket.display()));
    k
}

/// La herramienta `contenedores`.
pub async fn ejecutar(args: &Value, root: &Path) -> Result<String> {
    let s = |k: &str| args.get(k).and_then(|v| v.as_str()).map(str::trim).filter(|v| !v.is_empty());
    let accion = s("action").unwrap_or("listar");
    let (d, sock) = motor().await?;
    match accion {
        "listar" => {
            let cfg = compose_json(root)?;
            let vivos = del_proyecto(&d, root).await?;
            let mut out = Vec::new();
            let definidos: Vec<String> = cfg.as_ref().and_then(|c| c["services"].as_object()).map(|m| m.keys().cloned().collect()).unwrap_or_default();
            for svc in &definidos {
                let img = cfg.as_ref().map(|c| c["services"][svc]["image"].as_str().unwrap_or("(build)").to_string()).unwrap_or_default();
                match vivos.iter().find(|c| &servicio_de(c) == svc) {
                    Some(c) => out.push(format!("{svc} · {img} · {} {} · {}", texto(&c.state), c.status.clone().unwrap_or_default(), puertos(c))),
                    None => out.push(crate::tr!("{svc} · {img} · sin contenedor (levantar)", "{svc} · {img} · no container (levantar)")),
                }
            }
            for c in vivos.iter().filter(|c| !definidos.contains(&servicio_de(c))) {
                out.push(crate::tr!("{} · {} · {} (fuera del compose actual)", "{} · {} · {} (outside the current compose)", servicio_de(c), c.image.clone().unwrap_or_default(), texto(&c.state)));
            }
            if out.is_empty() {
                return Ok(crate::i18n::t("Este proyecto no tiene compose ni contenedores.", "This project has no compose file nor containers.").into());
            }
            Ok(out.join("\n"))
        }
        "registros" => {
            use futures_util::StreamExt;
            let svc = s("servicio").ok_or_else(|| anyhow!("{}", crate::i18n::t("falta `servicio`", "missing `servicio`")))?;
            let c = contenedor(&d, root, svc).await?;
            let lineas = args.get("lineas").and_then(|v| v.as_u64()).unwrap_or(200).clamp(1, 5_000).to_string();
            let mut o = bollard::query_parameters::LogsOptionsBuilder::new().stdout(true).stderr(true).timestamps(false).tail(&lineas);
            if let Some(dt) = s("desde").and_then(segundos) {
                let ahora = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
                o = o.since((ahora - dt) as i32);
            }
            let filtro = s("filtro").map(str::to_lowercase);
            let mut flujo = d.logs(c.id.as_deref().unwrap_or(""), Some(o.build()));
            let mut texto = String::new();
            while let Some(Ok(l)) = flujo.next().await {
                texto.push_str(&l.to_string());
            }
            let t: Vec<&str> = texto.lines().filter(|l| filtro.as_ref().is_none_or(|f| l.to_lowercase().contains(f))).collect();
            Ok(if t.is_empty() { crate::i18n::t("(sin registros con ese filtro)", "(no logs with that filter)").into() } else { t.join("\n") })
        }
        "esperar" => {
            let svc = s("servicio").ok_or_else(|| anyhow!("{}", crate::i18n::t("falta `servicio`", "missing `servicio`")))?;
            let tope = Duration::from_secs(args.get("segundos").and_then(|v| v.as_u64()).unwrap_or(60).clamp(1, 600));
            let t0 = std::time::Instant::now();
            loop {
                let c = contenedor(&d, root, svc).await?;
                let i = d.inspect_container(c.id.as_deref().unwrap_or(""), None).await?;
                let st = i.state.unwrap_or_default();
                let estado = texto(&st.status);
                if estado == "exited" || estado == "dead" {
                    bail!("{}", crate::tr!("«{svc}» terminó (código {}): mira sus `registros`", "«{svc}» exited (code {}): look at its `registros`", st.exit_code.unwrap_or(-1)));
                }
                match st.health.as_ref().map(|h| texto(&h.status)) {
                    Some(h) if h == "healthy" => return Ok(crate::tr!("✓ «{svc}» sano en {:.0} s", "✓ «{svc}» healthy in {:.0} s", t0.elapsed().as_secs_f32())),
                    Some(h) if h == "unhealthy" => {
                        let ultima = st.health.as_ref().and_then(|h| h.log.as_ref()).and_then(|l| l.last()).and_then(|r| r.output.clone()).unwrap_or_default();
                        bail!("{}", crate::tr!("«{svc}» enfermo: {}", "«{svc}» unhealthy: {}", ultima.trim()));
                    }
                    Some(_) => {}
                    // sin salud declarada: el primer puerto publicado que responda
                    None if estado == "running" => {
                        let pub_: Vec<u16> = c.ports.iter().flatten().filter_map(|p| p.public_port).collect();
                        if pub_.is_empty() {
                            return Ok(crate::tr!("✓ «{svc}» corriendo (no declara salud ni publica puertos: corriendo es lo más que se sabe)", "✓ «{svc}» running (no healthcheck nor published ports: running is all that is known)"));
                        }
                        for p in pub_ {
                            if tokio::net::TcpStream::connect(("127.0.0.1", p)).await.is_ok() {
                                return Ok(crate::tr!("✓ «{svc}» responde en el puerto {p} ({:.0} s; no declara salud)", "✓ «{svc}» answers on port {p} ({:.0} s; no healthcheck)", t0.elapsed().as_secs_f32()));
                            }
                        }
                    }
                    None => {}
                }
                if t0.elapsed() > tope {
                    bail!("{}", crate::tr!("«{svc}» no estuvo listo en {} s (estado: {estado}): mira sus `registros`", "«{svc}» was not ready within {} s (state: {estado}): look at its `registros`", tope.as_secs()));
                }
                tokio::time::sleep(Duration::from_millis(800)).await;
            }
        }
        "levantar" | "parar" => {
            let cfg = compose_json(root)?.ok_or_else(|| anyhow!("{}", crate::i18n::t("este proyecto no tiene compose.yaml ni docker-compose.yml", "this project has no compose.yaml nor docker-compose.yml")))?;
            let servicios: Vec<String> = match args.get("servicios").and_then(|v| v.as_array()) {
                Some(a) => a.iter().filter_map(|x| x.as_str().map(String::from)).collect(),
                None => s("servicio").map(|x| vec![x.to_string()]).unwrap_or_default(),
            };
            if accion == "levantar" {
                let fuera = frenos(&cfg, root, &sock)?;
                if !fuera.is_empty() && args.get("permitir_fuera").and_then(|v| v.as_bool()) != Some(true) {
                    return Ok(crate::tr!(
                        "⚠ NO se levantó: monta carpetas FUERA del proyecto ({}). Si es lo que quieres, repite con permitir_fuera=true (el usuario lo aprueba).",
                        "⚠ NOT started: it mounts folders OUTSIDE the project ({}). If that is what you want, repeat with permitir_fuera=true (the user approves it).",
                        fuera.join("; ")
                    ));
                }
            }
            let mut k = compose(root, &sock);
            if accion == "levantar" {
                k.args(["up", "-d"]);
            } else {
                k.arg("stop");
            }
            k.args(&servicios);
            let o = tokio::task::spawn_blocking(move || k.output()).await.map_err(|e| anyhow!("{e}"))??;
            let salida = format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr));
            if !o.status.success() {
                bail!("docker compose: {}", salida.trim());
            }
            let mut out = if accion == "levantar" {
                crate::tr!("✓ levantado. Siguiente: action=esperar servicio=… (sano o puerto que responde)", "✓ started. Next: action=esperar servicio=… (healthy or answering port)")
            } else {
                crate::i18n::t("✓ parado (los datos y volúmenes siguen ahí)", "✓ stopped (data and volumes are still there)").to_string()
            };
            if accion == "levantar" {
                // medido 2026-11-16 (banco `compose`): `up -d` termina bien aunque un servicio muera al
                // arrancar; «✓ levantado» bastó para que el modelo dijera que todo corría. Se mira el estado
                // de cada uno un momento después y, del que cayó, sus últimas líneas
                tokio::time::sleep(Duration::from_secs(3)).await;
                let ahora = del_proyecto(&d, root).await.unwrap_or_default();
                for c in &ahora {
                    let svc = servicio_de(c);
                    if !servicios.is_empty() && !servicios.contains(&svc) {
                        continue;
                    }
                    let estado = c.state.as_ref().map(|s| format!("{s:?}").to_lowercase()).unwrap_or_default();
                    if estado.contains("exited") || estado.contains("dead") || estado.contains("restarting") {
                        let mut l = compose(root, &sock);
                        l.args(["logs", "--no-color", "--tail", "15", &svc]);
                        let reg = tokio::task::spawn_blocking(move || l.output()).await.ok().and_then(|r| r.ok()).map(|o| format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr))).unwrap_or_default();
                        out.push_str(&crate::tr!(
                            "\n⚠ «{svc}» NO quedó corriendo ({}): sus últimas líneas:\n{}",
                            "\n⚠ «{svc}» did NOT stay up ({}): its last lines:\n{}",
                            c.status.as_deref().unwrap_or(&estado),
                            reg.trim()
                        ));
                    }
                }
                let vivos = del_proyecto(&d, root).await.unwrap_or_default();
                for (svc, def) in cfg["services"].as_object().into_iter().flatten() {
                    if !servicios.is_empty() && !servicios.contains(svc) {
                        continue;
                    }
                    let puerto = vivos.iter().find(|c| &servicio_de(c) == svc).and_then(|c| c.ports.iter().flatten().find_map(|p| p.public_port));
                    if let Some(p) = propuesta_bd(svc, def, puerto) {
                        out.push('\n');
                        out.push_str(&p);
                    }
                }
            }
            Ok(out)
        }
        "ejecutar" => {
            use futures_util::StreamExt;
            let svc = s("servicio").ok_or_else(|| anyhow!("{}", crate::i18n::t("falta `servicio`", "missing `servicio`")))?;
            let orden = s("orden").ok_or_else(|| anyhow!("{}", crate::i18n::t("falta `orden`", "missing `orden`")))?;
            let c = contenedor(&d, root, svc).await?;
            let id = c.id.clone().unwrap_or_default();
            let ex = d
                .create_exec(&id, bollard::models::ExecConfig { attach_stdout: Some(true), attach_stderr: Some(true), cmd: Some(vec!["sh".into(), "-c".into(), orden.into()]), ..Default::default() })
                .await?;
            let mut texto = String::new();
            let corre = async {
                if let bollard::exec::StartExecResults::Attached { mut output, .. } = d.start_exec(&ex.id, None).await? {
                    while let Some(Ok(l)) = output.next().await {
                        texto.push_str(&l.to_string());
                    }
                }
                Ok::<_, anyhow::Error>(())
            };
            tokio::time::timeout(Duration::from_secs(args.get("timeout").and_then(|v| v.as_u64()).unwrap_or(120).clamp(5, 1800)), corre)
                .await
                .map_err(|_| anyhow!("{}", crate::i18n::t("la orden pasó de su tope y se dejó (sigue en el contenedor)", "the command went past its cap and was left (it continues in the container)")))??;
            let codigo = d.inspect_exec(&ex.id).await.ok().and_then(|i| i.exit_code).unwrap_or(-1);
            Ok(format!("(exit {codigo})\n{}", texto.trim_end()))
        }
        "inspeccionar" => {
            let svc = s("servicio").ok_or_else(|| anyhow!("{}", crate::i18n::t("falta `servicio`", "missing `servicio`")))?;
            let c = contenedor(&d, root, svc).await?;
            let i = d.inspect_container(c.id.as_deref().unwrap_or(""), None).await?;
            let env = i.config.as_ref().and_then(|c| c.env.clone()).unwrap_or_default();
            let montajes: Vec<String> = i.mounts.iter().flatten().map(|m| format!("{} {} → {}", texto(&m.typ), m.source.clone().unwrap_or_default(), m.destination.clone().unwrap_or_default())).collect();
            let redes: Vec<String> = i.network_settings.as_ref().and_then(|n| n.networks.as_ref()).map(|n| n.keys().cloned().collect()).unwrap_or_default();
            let st = i.state.clone().unwrap_or_default();
            Ok(json!({
                "servicio": svc,
                "imagen": c.image,
                "estado": texto(&st.status),
                "salud": st.health.as_ref().map(|h| texto(&h.status)),
                "reinicios": i.restart_count,
                "puertos": puertos(&c),
                "entorno": enmascarar(&env),
                "montajes": montajes,
                "redes": redes,
            })
            .to_string())
        }
        otra => Err(anyhow!("{}", crate::tr!("acción «{otra}» desconocida: listar · registros · esperar · levantar · parar · ejecutar · inspeccionar", "unknown action «{otra}»: listar · registros · esperar · levantar · parar · ejecutar · inspeccionar"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// El socket se le pregunta al contexto activo, no se supone.
    #[test]
    fn el_socket_sale_del_contexto_activo() {
        use sha2::{Digest, Sha256};
        let c = crate::CarpetaPrueba::nueva("gotv-docker-ctx");
        std::fs::create_dir_all(c.join(".docker")).unwrap();
        std::fs::write(c.join(".docker/config.json"), r#"{"currentContext": "colima-otro"}"#).unwrap();
        let meta = c.join(".docker/contexts/meta").join(format!("{:x}", Sha256::digest(b"colima-otro")));
        std::fs::create_dir_all(&meta).unwrap();
        std::fs::write(meta.join("meta.json"), r#"{"Name": "colima-otro", "Endpoints": {"docker": {"Host": "unix:///x/otro/docker.sock"}}}"#).unwrap();
        assert_eq!(socket_en(&c, None).unwrap(), PathBuf::from("/x/otro/docker.sock"));
        assert_eq!(socket_en(&c, Some("unix:///y.sock")).unwrap(), PathBuf::from("/y.sock"), "DOCKER_HOST manda");
        assert!(socket_en(&c, Some("tcp://1.2.3.4:2375")).is_err());
    }

    /// Cada freno con su caso, sobre la configuración ya normalizada (el formato de `compose config`).
    #[test]
    fn los_frenos_fijos() {
        let d = crate::CarpetaPrueba::nueva("gotv-compose-frenos");
        let raiz = d.canonicalize().unwrap().display().to_string();
        let sock = PathBuf::from("/x/docker.sock");
        let svc = |extra: Value| json!({"services": {"web": extra}});
        let bind = |src: &str| json!({"image": "x", "volumes": [{"type": "bind", "source": src, "target": "/d"}]});
        assert!(frenos(&svc(json!({"image": "x", "privileged": true})), &d, &sock).unwrap_err().to_string().contains("privilegiado"));
        assert!(frenos(&svc(json!({"image": "x", "network_mode": "host"})), &d, &sock).is_err());
        assert!(frenos(&svc(bind("/")), &d, &sock).unwrap_err().to_string().contains("raíz"));
        assert!(frenos(&svc(bind(&casa().display().to_string())), &d, &sock).unwrap_err().to_string().contains("personal"));
        assert!(frenos(&svc(bind(&casa().join(".ssh").display().to_string())), &d, &sock).unwrap_err().to_string().contains("credenciales"));
        assert!(frenos(&svc(bind("/var/run/docker.sock")), &d, &sock).unwrap_err().to_string().contains("socket"));
        // dentro del proyecto: pasa; fuera (sin ser peligroso): se pregunta
        assert!(frenos(&svc(bind(&format!("{raiz}/datos"))), &d, &sock).unwrap().is_empty());
        assert_eq!(frenos(&svc(bind("/opt/compartido")), &d, &sock).unwrap(), vec!["web: /opt/compartido".to_string()]);
        // un volumen con nombre no es una ruta del Mac
        assert!(frenos(&svc(json!({"image": "x", "volumes": [{"type": "volume", "source": "datos", "target": "/var/lib/postgresql/data"}]})), &d, &sock).unwrap().is_empty());
    }

    #[test]
    fn enmascarar_y_proponer() {
        assert_eq!(enmascarar(&["POSTGRES_PASSWORD=x".into(), "POSTGRES_DB=app".into(), "DATABASE_URL=postgres://u:p@h/b".into()]), vec!["POSTGRES_PASSWORD=«oculto»", "POSTGRES_DB=app", "DATABASE_URL=«oculto»"]);
        assert_eq!(segundos("10m"), Some(600));
        assert_eq!(segundos("x"), None);
        let p = propuesta_bd("db", &json!({"image": "postgres:16", "environment": {"POSTGRES_USER": "ana", "POSTGRES_DB": "tienda"}}), Some(55432)).unwrap();
        assert!(p.contains("postgres://ana@localhost:55432/tienda"), "{p}");
        assert!(propuesta_bd("web", &json!({"image": "nginx"}), Some(80)).is_none());
    }

    /// Contra el motor REAL (`GOTV_PRUEBA_DOCKER=1`): un compose con MariaDB (imagen ya bajada) se levanta,
    /// se espera sano, se lee, se le ejecuta una orden, se inspecciona (clave oculta) y se para y se borra.
    #[tokio::test]
    #[ignore = "levanta un contenedor real: GOTV_PRUEBA_DOCKER=1"]
    async fn contra_el_motor_real() {
        if std::env::var_os("GOTV_PRUEBA_DOCKER").is_none() {
            return;
        }
        let d = crate::CarpetaPrueba::nueva("gotv-compose-real");
        let puerto = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        std::fs::write(
            d.join("compose.yaml"),
            format!(
                "services:\n  db:\n    image: mariadb:11.4\n    environment:\n      MARIADB_ROOT_PASSWORD: s3cr3t\n      MARIADB_DATABASE: tienda\n    ports:\n      - \"127.0.0.1:{puerto}:3306\"\n    healthcheck:\n      test: [\"CMD\", \"healthcheck.sh\", \"--connect\"]\n      interval: 2s\n      retries: 30\n"
            ),
        )
        .unwrap();
        let h = |a: Value| {
            let r = d.to_path_buf();
            async move { ejecutar(&a, &r).await }
        };
        let fin = || {
            let _ = compose(&d, &socket().unwrap()).args(["down", "-v"]).output();
        };
        let r = async {
            assert!(h(json!({"action": "listar"})).await?.contains("db · mariadb:11.4 · sin contenedor"));
            let l = h(json!({"action": "levantar"})).await?;
            assert!(l.contains("mysql://root@localhost") && l.contains("/tienda"), "{l}");
            let e = h(json!({"action": "esperar", "servicio": "db", "segundos": 120})).await?;
            assert!(e.contains("sano"), "{e}");
            assert!(h(json!({"action": "listar"})).await?.contains("running"));
            assert!(!h(json!({"action": "registros", "servicio": "db", "lineas": 50})).await?.is_empty());
            let x = h(json!({"action": "ejecutar", "servicio": "db", "orden": "echo hola-desde-dentro"})).await?;
            assert!(x.starts_with("(exit 0)") && x.contains("hola-desde-dentro"), "{x}");
            let i = h(json!({"action": "inspeccionar", "servicio": "db"})).await?;
            assert!(i.contains("MARIADB_ROOT_PASSWORD=«oculto»") && !i.contains("s3cr3t") && i.contains("MARIADB_DATABASE=tienda"), "{i}");
            assert!(h(json!({"action": "parar"})).await?.contains("parado"));
            Ok::<_, anyhow::Error>(())
        }
        .await;
        fin();
        r.unwrap();
    }
}
