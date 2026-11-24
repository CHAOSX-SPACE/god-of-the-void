//! F1 paso 6 · Túnel SSH hacia una base remota, sobre las conexiones SSH del PROYECTO (`conexiones.rs`):
//! la misma huella fijada (`StrictHostKeyChecking=yes` contra el `known_hosts` del proyecto), la misma
//! llave y el mismo bastión, sin agente ni `~/.ssh/config` del usuario.
//!
//! `ssh -N -L 127.0.0.1:<puerto libre>:<host>:<puerto>`, vivo SÓLO mientras dura la llamada (el proceso
//! muere con el `Tunel`: `kill_on_drop`). Sin multiplexado: en ssh gana la PRIMERA `-o` de cada opción,
//! así que `ControlMaster=no` y `ControlPath=none` van delante de las del proyecto (un reenvío por un
//! maestro compartido sobreviviría a la llamada). `ExitOnForwardFailure=yes`: si el reenvío no se puede
//! abrir, ssh sale con error en vez de quedarse vivo sin túnel.
use anyhow::{anyhow, bail, Result};
use std::path::Path;
use std::time::Duration;

pub struct Tunel {
    pub puerto: u16,
    _ssh: tokio::process::Child,
}

fn puerto_libre() -> Result<u16> {
    Ok(std::net::TcpListener::bind("127.0.0.1:0")?.local_addr()?.port())
}

/// Abre el túnel por la conexión SSH `nombre` del proyecto hasta `host:puerto` (vistos DESDE el servidor
/// SSH: `127.0.0.1` es el propio servidor). Espera hasta 20 s a que el puerto local responda.
pub async fn abrir(root: &Path, nombre: &str, host: &str, puerto: u16) -> Result<Tunel> {
    if host.is_empty() || host.starts_with('-') || host.contains(char::is_whitespace) {
        bail!("{}", crate::tr!("destino inválido para el túnel: «{host}»", "invalid tunnel destination: «{host}»"));
    }
    let reg = crate::conexiones::Registro::cargar(root)?;
    let c = reg.buscar(nombre).ok_or_else(|| anyhow!("{}", crate::tr!("no hay una conexión SSH «{nombre}» en este proyecto (`gotv conexiones agregar`)", "there is no SSH connection «{nombre}» in this project (`gotv conexiones agregar`)")))?;
    if !crate::conexiones::ruta_known_hosts(root).is_file() {
        bail!("{}", crate::tr!("«{nombre}» aún no tiene la huella del servidor fijada: el usuario debe correr `gotv conexiones probar {nombre}` una vez", "«{nombre}» has no pinned server fingerprint yet: the user must run `gotv conexiones probar {nombre}` once"));
    }
    let local = puerto_libre()?;
    // IPv6 literal entre corchetes en la especificación de -L
    let destino = if host.contains(':') { format!("[{host}]") } else { host.to_string() };
    let mut args: Vec<String> = ["-o", "ControlMaster=no", "-o", "ControlPath=none", "-o", "ExitOnForwardFailure=yes", "-N", "-L"].iter().map(|s| s.to_string()).collect();
    args.push(format!("127.0.0.1:{local}:{destino}:{puerto}"));
    args.extend(crate::conexiones::opciones(root, &reg, c, false)?);
    args.push(c.destino());
    let mut k = tokio::process::Command::new("ssh");
    k.args(&args).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::piped()).kill_on_drop(true);
    let mut ssh = k.spawn().map_err(|e| anyhow!("ssh: {e}"))?;
    let t0 = std::time::Instant::now();
    loop {
        if let Some(estado) = ssh.try_wait()? {
            let mut err = String::new();
            if let Some(mut e) = ssh.stderr.take() {
                use tokio::io::AsyncReadExt;
                let _ = e.read_to_string(&mut err).await;
            }
            crate::conexiones::auditar_tunel(root, nombre, host, puerto, estado.code().unwrap_or(-1), t0.elapsed().as_millis());
            return Err(anyhow!("{}", crate::conexiones::explicar_fallo(nombre, &err)));
        }
        if tokio::net::TcpStream::connect(("127.0.0.1", local)).await.is_ok() {
            crate::conexiones::auditar_tunel(root, nombre, host, puerto, 0, t0.elapsed().as_millis());
            return Ok(Tunel { puerto: local, _ssh: ssh });
        }
        if t0.elapsed() > Duration::from_secs(20) {
            let _ = ssh.kill().await;
            bail!("{}", crate::tr!("el túnel por «{nombre}» no abrió en 20 s", "the tunnel through «{nombre}» did not open within 20 s"));
        }
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
}
