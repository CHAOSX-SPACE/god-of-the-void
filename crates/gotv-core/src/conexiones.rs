//! Conexiones SSH POR PROYECTO (Ola H, 2026-11-10): GOTV controla servidores y otros computadores
//! sin que un proyecto vea las conexiones de otro.
//!
//! - Registro: `<proyecto>/.gotv/conexiones.gotv`, formato propio (nombre, usuario@host, puerto, llave, salto, nota,
//!   «sólo lectura»). Nada de secretos dentro: las llaves viven fuera del proyecto.
//! - Aislamiento: `ssh -F /dev/null` (ni el `~/.ssh/config` del usuario se filtra), llave propia por
//!   conexión (`IdentitiesOnly`), `known_hosts` PROPIO del proyecto con `StrictHostKeyChecking=yes`
//!   (la huella se fija al probar la conexión, con el usuario delante) y `BatchMode=yes` (jamás una
//!   contraseña tecleada por un modelo).
//! - Llaves: `gotv conexiones llave <nombre>` crea una ed25519 en `~/.ssh/gotv/<id-proyecto>/` y
//!   muestra la pública para autorizarla en el servidor.
//! - Multiplexado (`ControlMaster`): la 2.ª orden a la misma conexión no repite el saludo SSH.
//! - Auditoría: cada orden remota queda en `.gotv/conexiones.log` (JSON por línea).
//! - Permisos (ver `decidir`): leer pasa; lo peligroso pregunta SIEMPRE (también en bypass);
//!   «sólo lectura» niega el resto; lo demás sigue el modo.

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::Duration;

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct Conexion {
    pub nombre: String,
    pub host: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usuario: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub puerto: Option<u16>,
    /// ruta de la llave PRIVADA (fuera del proyecto); sin ella, las del agente SSH del usuario
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub llave: Option<String>,
    /// otra conexión DE ESTE proyecto por la que se salta (bastión)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub salto: Option<String>,
    /// carpeta remota donde se trabaja (las órdenes empiezan ahí)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub carpeta: Option<String>,
    /// para qué es (la ve el modelo)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nota: Option<String>,
    /// sólo órdenes que leen: el resto se niega
    #[serde(default)]
    pub solo_lectura: bool,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Registro {
    #[serde(default)]
    pub conexiones: Vec<Conexion>,
}

pub fn ruta_registro(root: &Path) -> PathBuf {
    root.join(".gotv/conexiones.gotv")
}

pub fn ruta_known_hosts(root: &Path) -> PathBuf {
    root.join(".gotv/known_hosts")
}

fn ruta_log(root: &Path) -> PathBuf {
    root.join(".gotv/conexiones.log")
}

/// Identificador estable del proyecto (para su carpeta de llaves y de control).
pub fn id_proyecto(root: &Path) -> String {
    use sha2::{Digest, Sha256};
    let r = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    let h = Sha256::digest(r.to_string_lossy().as_bytes());
    let nombre: String = r
        .file_name()
        .map(|n| n.to_string_lossy().chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' }).collect())
        .unwrap_or_default();
    format!("{}-{}", nombre.trim_matches('-'), h.iter().take(5).map(|b| format!("{b:02x}")).collect::<String>())
}

fn home() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| "/var/empty".into()))
}

/// Carpeta de llaves de ESTE proyecto (fuera del proyecto: no viaja con un `git push`).
pub fn carpeta_llaves(root: &Path) -> PathBuf {
    home().join(".ssh/gotv").join(id_proyecto(root))
}

/// Sockets de multiplexado. Ruta CORTA: el límite de un socket Unix es 104 bytes y `%C` ya son 40
/// (medido: con $TMPDIR de macOS daba 122 → «ControlPath too long»). `~/.ssh/gotv/c/<8 hex>/%C` ≈ 75.
fn carpeta_control(root: &Path) -> PathBuf {
    let id = id_proyecto(root);
    let hash = id.rsplit('-').next().unwrap_or(&id);
    home().join(".ssh/gotv/c").join(&hash[..hash.len().min(8)])
}

// ── validación (nada de inyectar opciones a ssh por un nombre o un host) ─────────
fn valida_nombre(n: &str) -> Result<()> {
    if n.is_empty() || n.len() > 40 || !n.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        bail!("{}", crate::tr!("nombre inválido «{n}»: letras, números, - y _ (máx. 40)", "invalid name «{n}»: letters, digits, - and _ (max 40)"));
    }
    Ok(())
}

fn valida_host(h: &str) -> Result<()> {
    let ok = !h.is_empty()
        && h.len() <= 253
        && !h.starts_with('-')
        && h.chars().all(|c| c.is_ascii_alphanumeric() || ".-_:[]%".contains(c));
    if !ok {
        bail!("{}", crate::tr!("host inválido «{h}»", "invalid host «{h}»"));
    }
    Ok(())
}

fn valida_usuario(u: &str) -> Result<()> {
    if u.is_empty() || u.len() > 64 || u.starts_with('-') || !u.chars().all(|c| c.is_ascii_alphanumeric() || "._-".contains(c)) {
        bail!("{}", crate::tr!("usuario inválido «{u}»", "invalid user «{u}»"));
    }
    Ok(())
}

impl Conexion {
    pub fn validar(&self) -> Result<()> {
        valida_nombre(&self.nombre)?;
        valida_host(&self.host)?;
        if let Some(u) = &self.usuario {
            valida_usuario(u)?;
        }
        if let Some(s) = &self.salto {
            valida_nombre(s)?;
            if *s == self.nombre {
                bail!("{}", crate::i18n::t("una conexión no puede saltar por sí misma", "a connection cannot jump through itself"));
            }
        }
        if let Some(l) = &self.llave {
            if l.starts_with('-') {
                bail!("{}", crate::i18n::t("ruta de llave inválida", "invalid key path"));
            }
        }
        Ok(())
    }

    /// `usuario@host` (sin puerto: va con -p)
    pub fn destino(&self) -> String {
        match &self.usuario {
            Some(u) => format!("{u}@{}", self.host),
            None => self.host.clone(),
        }
    }

    pub fn resumen(&self) -> String {
        let mut s = format!("{} → {}", self.nombre, self.destino());
        if let Some(p) = self.puerto.filter(|p| *p != 22) {
            s += &format!(":{p}");
        }
        if let Some(j) = &self.salto {
            s += &crate::tr!(" (saltando por {j})", " (jumping through {j})");
        }
        if self.solo_lectura {
            s += crate::i18n::t(" · sólo lectura", " · read-only");
        }
        if let Some(n) = &self.nota {
            s += &format!(" — {n}");
        }
        s
    }
}

impl Registro {
    /// Formato propio (códec, CONX); hasta 2026-11-17 era `conexiones.json` (se migra solo la primera vez).
    pub fn cargar(root: &Path) -> Result<Registro> {
        use crate::programar::codec;
        let p = ruta_registro(root);
        let r = match codec::leer_serde_migrado::<Registro>(&p, &p.with_extension("json"), b"CONX") {
            Some(r) => r,
            None if !p.exists() => return Ok(Registro::default()),
            None => anyhow::bail!("{}", crate::tr!("{} está dañado: no se puede leer", "{} is damaged: it cannot be read", p.display())),
        };
        for c in &r.conexiones {
            c.validar()?;
        }
        Ok(r)
    }

    pub fn guardar(&self, root: &Path) -> Result<()> {
        crate::programar::codec::guardar_serde(&ruta_registro(root), b"CONX", self)
    }

    pub fn buscar(&self, nombre: &str) -> Option<&Conexion> {
        self.conexiones.iter().find(|c| c.nombre == nombre)
    }

    pub fn agregar(&mut self, c: Conexion) -> Result<()> {
        c.validar()?;
        if let Some(s) = &c.salto {
            if self.buscar(s).is_none() {
                bail!("{}", crate::tr!("el salto «{s}» no es una conexión de este proyecto", "jump «{s}» is not a connection of this project"));
            }
        }
        match self.conexiones.iter_mut().find(|x| x.nombre == c.nombre) {
            Some(x) => *x = c,
            None => self.conexiones.push(c),
        }
        self.conexiones.sort_by(|a, b| a.nombre.cmp(&b.nombre));
        Ok(())
    }

    pub fn quitar(&mut self, nombre: &str) -> Result<Conexion> {
        if let Some(d) = self.conexiones.iter().find(|c| c.salto.as_deref() == Some(nombre)) {
            bail!("{}", crate::tr!("«{}» salta por «{nombre}»: quítala antes", "«{}» jumps through «{nombre}»: remove it first", d.nombre));
        }
        let i = self.conexiones.iter().position(|c| c.nombre == nombre).ok_or_else(|| anyhow!("{}", crate::tr!("no hay una conexión «{nombre}» en este proyecto", "there is no connection «{nombre}» in this project")))?;
        Ok(self.conexiones.remove(i))
    }
}

/// Las opciones de `ssh`/`scp` para una conexión (sin el destino). `scp` usa -P para el puerto.
pub fn opciones(root: &Path, reg: &Registro, c: &Conexion, para_scp: bool) -> Result<Vec<String>> {
    let kh = ruta_known_hosts(root);
    let ctl = carpeta_control(root);
    // la carpeta de los sockets NO se crea aquí: sólo quien multiplexa de verdad (`preparar_control`).
    // Antes se creaba siempre y cada prueba con una carpeta temporal dejaba una vacía en ~/.ssh/gotv/c
    // (270 acumuladas, medido el 2026-11-15).
    let mut o: Vec<String> = vec![
        "-F".into(),
        "/dev/null".into(),
        "-o".into(),
        "BatchMode=yes".into(),
        "-o".into(),
        "StrictHostKeyChecking=yes".into(),
        "-o".into(),
        format!("UserKnownHostsFile={}", kh.display()),
        "-o".into(),
        "GlobalKnownHostsFile=/dev/null".into(),
        "-o".into(),
        "ConnectTimeout=10".into(),
        "-o".into(),
        "ServerAliveInterval=15".into(),
        "-o".into(),
        "ServerAliveCountMax=2".into(),
        "-o".into(),
        "ControlMaster=auto".into(),
        "-o".into(),
        format!("ControlPath={}/%C", ctl.display()),
        "-o".into(),
        "ControlPersist=120".into(),
        "-o".into(),
        "LogLevel=ERROR".into(),
    ];
    if let Some(l) = &c.llave {
        let ruta = if let Some(r) = l.strip_prefix("~/") { home().join(r) } else { PathBuf::from(l) };
        if !ruta.is_file() {
            bail!("{}", crate::tr!("la llave de «{}» no existe: {} (créala con `gotv conexiones llave {}`)", "«{}»'s key does not exist: {} (create it with `gotv conexiones llave {}`)", c.nombre, ruta.display(), c.nombre));
        }
        o.extend(["-o".into(), "IdentitiesOnly=yes".into(), "-i".into(), ruta.display().to_string()]);
    }
    if let Some(p) = c.puerto {
        o.extend([if para_scp { "-P" } else { "-p" }.to_string(), p.to_string()]);
    }
    if let Some(s) = &c.salto {
        let j = reg.buscar(s).ok_or_else(|| anyhow!("{}", crate::tr!("el salto «{s}» no está en este proyecto", "jump «{s}» is not in this project")))?;
        if j.salto.is_some() {
            bail!("{}", crate::i18n::t("sólo se admite un salto (el bastión no puede saltar a su vez)", "only one jump is supported (the bastion cannot jump in turn)"));
        }
        // el salto también va aislado: mismo known_hosts del proyecto y su propia llave
        let mut pj = format!(
            "ssh -F /dev/null -o BatchMode=yes -o StrictHostKeyChecking=yes -o UserKnownHostsFile={} -o GlobalKnownHostsFile=/dev/null",
            kh.display()
        );
        if let Some(l) = &j.llave {
            pj += &format!(" -o IdentitiesOnly=yes -i {l}");
        }
        if let Some(p) = j.puerto {
            pj += &format!(" -p {p}");
        }
        pj += &format!(" -W %h:%p {}", j.destino());
        o.extend(["-o".into(), format!("ProxyCommand={pj}")]);
    }
    Ok(o)
}

/// Crea (0700) la carpeta de los sockets de multiplexado de este proyecto, justo antes de usarla.
fn preparar_control(root: &Path) {
    let ctl = carpeta_control(root);
    std::fs::create_dir_all(&ctl).ok();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&ctl, std::fs::Permissions::from_mode(0o700));
    }
}

/// Lo que corre una orden remota: empieza en la carpeta de la conexión si la tiene.
pub fn orden_remota(c: &Conexion, cmd: &str) -> String {
    match &c.carpeta {
        Some(d) => format!("cd {} && {cmd}", comillas(d)),
        None => cmd.to_string(),
    }
}

/// Comillas simples de shell POSIX.
fn comillas(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

async fn correr(mut c: tokio::process::Command, timeout_s: u64) -> Result<(i32, String)> {
    c.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped()).kill_on_drop(true);
    let hijo = c.spawn().map_err(|e| anyhow!("ssh: {e}"))?;
    let o = tokio::time::timeout(Duration::from_secs(timeout_s.clamp(5, 600)), hijo.wait_with_output())
        .await
        .map_err(|_| anyhow!("{}", crate::tr!("la orden remota pasó de {timeout_s} s (se cortó)", "the remote command went past {timeout_s} s (cut)")))??;
    let mut out = String::from_utf8_lossy(&o.stdout).to_string();
    let err = String::from_utf8_lossy(&o.stderr);
    if !err.trim().is_empty() {
        if !out.is_empty() && !out.ends_with('\n') {
            out.push('\n');
        }
        out.push_str(err.trim_end());
    }
    Ok((o.status.code().unwrap_or(-1), out))
}

/// F1: el túnel hacia una base queda en la misma bitácora que las órdenes remotas.
pub(crate) fn auditar_tunel(root: &Path, nombre: &str, host: &str, puerto: u16, rc: i32, ms: u128) {
    auditar(root, nombre, &format!("tunel -> {host}:{puerto}"), rc, ms);
}

/// Por qué falló una conexión SSH, en claro (la misma explicación que las órdenes remotas).
pub(crate) fn explicar_fallo(nombre: &str, salida: &str) -> String {
    explicar_255(nombre, salida)
}

/// F16.3: los argumentos de `ssh` para la conexión `nombre` de este proyecto (opciones aisladas, huella
/// fijada, multiplexado, destino y `--`), para quien manda su propia orden remota con datos por la entrada
/// (el `Lugar` remoto).
pub(crate) fn argumentos_ssh(root: &Path, nombre: &str) -> Result<Vec<String>> {
    let reg = Registro::cargar(root)?;
    let c = reg.buscar(nombre).ok_or_else(|| anyhow!("{}", no_existe(&reg, nombre)))?;
    if !ruta_known_hosts(root).is_file() {
        bail!("{}", crate::tr!("«{nombre}» aún no tiene la huella del servidor fijada: el usuario debe correr `gotv conexiones probar {nombre}` una vez", "«{nombre}» has no pinned server fingerprint yet: the user must run `gotv conexiones probar {nombre}` once"));
    }
    preparar_control(root);
    let mut args = opciones(root, &reg, c, false)?;
    args.push(c.destino());
    args.push("--".into());
    Ok(args)
}

/// Una línea de la bitácora de conexiones.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Bitacora {
    pub cuando: String,
    pub conexion: String,
    pub orden: String,
    pub salida: i32,
    pub ms: u128,
}

/// La bitácora es TEXTO que se va añadiendo (una línea por orden, campos separados por tabulador:
/// cuando · conexión · salida · ms · orden). Hasta 2026-11-17 cada línea era un objeto JSON: `registro`
/// sigue leyendo esas.
pub(crate) fn auditar(root: &Path, nombre: &str, que: &str, rc: i32, ms: u128) {
    use std::io::Write;
    let limpio = |s: &str| s.replace(['\t', '\n', '\r'], " ");
    let orden: String = que.chars().take(2000).collect();
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(ruta_log(root)) {
        let _ = writeln!(f, "{}\t{}\t{rc}\t{ms}\t{}", chrono_simple(), limpio(nombre), limpio(&orden));
    }
}

/// Las últimas `n` líneas de la bitácora (las de antes en JSON también), de la más vieja a la más nueva.
pub fn registro(root: &Path, n: usize) -> Vec<Bitacora> {
    let t = std::fs::read_to_string(ruta_log(root)).unwrap_or_default();
    let lineas: Vec<&str> = t.lines().filter(|l| !l.trim().is_empty()).collect();
    lineas[lineas.len().saturating_sub(n)..]
        .iter()
        .filter_map(|l| {
            if l.starts_with('{') {
                let v: serde_json::Value = serde_json::from_str(l).ok()?;
                return Some(Bitacora {
                    cuando: v["cuando"].as_str().unwrap_or("").into(),
                    conexion: v["conexion"].as_str().unwrap_or("").into(),
                    orden: v["orden"].as_str().unwrap_or("").into(),
                    salida: v["salida"].as_i64().unwrap_or(-1) as i32,
                    ms: v["ms"].as_u64().unwrap_or(0) as u128,
                });
            }
            let c: Vec<&str> = l.splitn(5, '\t').collect();
            (c.len() == 5).then(|| Bitacora { cuando: c[0].into(), conexion: c[1].into(), salida: c[2].parse().unwrap_or(-1), ms: c[3].parse().unwrap_or(0), orden: c[4].into() })
        })
        .collect()
}

/// Fecha y hora UTC sin dependencias (AAAA-MM-DDTHH:MM:SSZ).
fn chrono_simple() -> String {
    let s = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0) as i64;
    let (dias, resto) = (s.div_euclid(86_400), s.rem_euclid(86_400));
    // civil_from_days (Howard Hinnant)
    let z = dias + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", resto / 3600, resto % 3600 / 60, resto % 60)
}

/// Corre `cmd` en la conexión `nombre` de ESTE proyecto. Devuelve (código, salida).
pub async fn ejecutar(root: &Path, nombre: &str, cmd: &str, timeout_s: u64) -> Result<(i32, String)> {
    let reg = Registro::cargar(root)?;
    let c = reg.buscar(nombre).ok_or_else(|| anyhow!("{}", no_existe(&reg, nombre)))?;
    if !ruta_known_hosts(root).is_file() {
        bail!("{}", crate::tr!("«{nombre}» aún no tiene la huella del servidor fijada: el usuario debe correr `gotv conexiones probar {nombre}` una vez", "«{nombre}» has no pinned server fingerprint yet: the user must run `gotv conexiones probar {nombre}` once"));
    }
    preparar_control(root);
    let mut args = opciones(root, &reg, c, false)?;
    args.push(c.destino());
    args.push("--".into());
    args.push(orden_remota(c, cmd));
    let mut k = tokio::process::Command::new("ssh");
    k.args(&args);
    let t0 = std::time::Instant::now();
    let r = correr(k, timeout_s).await;
    let rc = r.as_ref().map(|x| x.0).unwrap_or(-1);
    auditar(root, nombre, cmd, rc, t0.elapsed().as_millis());
    let (rc, out) = r?;
    if rc == 255 {
        return Err(anyhow!("{}", explicar_255(nombre, &out)));
    }
    Ok((rc, out))
}

fn no_existe(reg: &Registro, nombre: &str) -> String {
    let hay: Vec<&str> = reg.conexiones.iter().map(|c| c.nombre.as_str()).collect();
    if hay.is_empty() {
        crate::tr!("este proyecto no tiene conexiones (el usuario las agrega con `gotv conexiones agregar`)", "this project has no connections (the user adds them with `gotv conexiones agregar`)")
    } else {
        crate::tr!("no hay una conexión «{nombre}» en este proyecto; hay: {}", "there is no connection «{nombre}» in this project; there are: {}", hay.join(", "))
    }
}

/// ssh sale con 255 cuando falla la CONEXIÓN (no la orden): se dice por qué, en claro.
fn explicar_255(nombre: &str, out: &str) -> String {
    let o = out.to_lowercase();
    let por = if o.contains("host key verification failed") || o.contains("remote host identification has changed") {
        crate::tr!("la huella del servidor NO coincide con la fijada (¿lo reinstalaron, o alguien se hace pasar por él?). No me conecto: el usuario debe verificarlo y volver a `gotv conexiones probar {nombre}`", "the server fingerprint does NOT match the pinned one (reinstalled, or someone impersonating it?). Not connecting: the user must verify and re-run `gotv conexiones probar {nombre}`")
    } else if o.contains("permission denied") {
        crate::tr!("el servidor rechazó la llave: autoriza la pública (`gotv conexiones llave {nombre}` la muestra) en ~/.ssh/authorized_keys del servidor", "the server rejected the key: authorize the public key (`gotv conexiones llave {nombre}` shows it) in the server's ~/.ssh/authorized_keys")
    } else if o.contains("administratively prohibited") || o.contains("stdio forwarding failed") {
        crate::i18n::t("el bastión NO permite reenviar conexiones (en su /etc/ssh/sshd_config hace falta `AllowTcpForwarding yes`, y que la PRIMERA línea con esa opción lo diga)", "the bastion does NOT allow forwarding connections (its /etc/ssh/sshd_config needs `AllowTcpForwarding yes`, and the FIRST line with that option must say so)").to_string()
    } else if o.contains("timed out") || o.contains("no route") || o.contains("could not resolve") || o.contains("connection refused") {
        crate::i18n::t("el servidor no responde (red, apagado o puerto cerrado)", "the server does not answer (network, off or closed port)").to_string()
    } else {
        String::new()
    };
    crate::tr!("no pude conectar con «{nombre}»: {por}\n{}", "could not connect to «{nombre}»: {por}\n{}", out.trim())
}

/// Sube (`subir`) o baja (`bajar`) un archivo o carpeta. `local` SIEMPRE dentro del proyecto.
pub async fn transferir(root: &Path, nombre: &str, subir: bool, local: &str, remoto: &str, timeout_s: u64) -> Result<String> {
    let reg = Registro::cargar(root)?;
    let c = reg.buscar(nombre).ok_or_else(|| anyhow!("{}", no_existe(&reg, nombre)))?;
    let l = crate::paths::resolve(root, local);
    let raiz = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    let l_abs = if l.exists() { std::fs::canonicalize(&l)? } else { l.clone() };
    if !l_abs.starts_with(&raiz) && !l.starts_with(root) {
        bail!("{}", crate::i18n::t("la ruta local debe estar dentro del proyecto", "the local path must be inside the project"));
    }
    if remoto.trim().is_empty() || remoto.starts_with('-') {
        bail!("{}", crate::i18n::t("ruta remota inválida", "invalid remote path"));
    }
    if subir && !l.exists() {
        bail!("{}", crate::tr!("no existe {}", "{} does not exist", l.display()));
    }
    if !subir {
        if let Some(p) = l.parent() {
            std::fs::create_dir_all(p)?;
        }
    }
    let remoto_completo = match (&c.carpeta, remoto.starts_with('/') || remoto.starts_with('~')) {
        (Some(d), false) => format!("{}/{remoto}", d.trim_end_matches('/')),
        _ => remoto.to_string(),
    };
    preparar_control(root);
    let mut args = opciones(root, &reg, c, true)?;
    args.push("-r".into());
    args.push("-q".into());
    let lado_remoto = format!("{}:{}", c.destino(), remoto_completo);
    if subir {
        args.extend(["--".into(), l.display().to_string(), lado_remoto]);
    } else {
        args.extend(["--".into(), lado_remoto, l.display().to_string()]);
    }
    let mut k = tokio::process::Command::new("scp");
    k.args(&args);
    let t0 = std::time::Instant::now();
    let r = correr(k, timeout_s).await;
    let que = format!("{} {} {} {}", if subir { "subir" } else { "bajar" }, local, if subir { "→" } else { "←" }, remoto_completo);
    auditar(root, nombre, &que, r.as_ref().map(|x| x.0).unwrap_or(-1), t0.elapsed().as_millis());
    let (rc, out) = r?;
    if rc != 0 {
        bail!("{}", if rc == 255 { explicar_255(nombre, &out) } else { out });
    }
    Ok(crate::tr!("✓ {que}", "✓ {que}"))
}

// ── llaves y huellas (los usa la CLI, con el usuario delante) ───────────────────

/// Crea (si falta) la llave ed25519 de la conexión y devuelve (ruta privada, línea pública).
pub fn generar_llave(root: &Path, nombre: &str) -> Result<(PathBuf, String)> {
    valida_nombre(nombre)?;
    let dir = carpeta_llaves(root);
    std::fs::create_dir_all(&dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
    }
    let privada = dir.join(format!("{nombre}_ed25519"));
    if !privada.is_file() {
        let comentario = format!("gotv {nombre} @ {}", id_proyecto(root));
        let st = std::process::Command::new("ssh-keygen")
            .args(["-q", "-t", "ed25519", "-N", "", "-C", &comentario, "-f"])
            .arg(&privada)
            .stdin(std::process::Stdio::null())
            .status()?;
        if !st.success() {
            bail!("ssh-keygen: {st}");
        }
    }
    // ssh-keygen deja la pública al lado: <privada>.pub
    let publica = std::fs::read_to_string(format!("{}.pub", privada.display()))?;
    Ok((privada, publica.trim().to_string()))
}

/// Las llaves públicas que ofrece el servidor (ssh-keyscan) y sus huellas SHA256, para que el
/// usuario las compare ANTES de fijarlas.
pub fn escanear(c: &Conexion) -> Result<(String, Vec<String>)> {
    let mut k = std::process::Command::new("ssh-keyscan");
    k.args(["-T", "6"]);
    if let Some(p) = c.puerto {
        k.args(["-p", &p.to_string()]);
    }
    k.arg("--").arg(&c.host);
    let o = k.stdin(std::process::Stdio::null()).output()?;
    let lineas = String::from_utf8_lossy(&o.stdout).lines().filter(|l| !l.starts_with('#') && !l.trim().is_empty()).collect::<Vec<_>>().join("\n");
    if lineas.is_empty() {
        bail!("{}", crate::tr!("{} no respondió (¿encendido, puerto {} abierto?)", "{} did not answer (on? port {} open?)", c.host, c.puerto.unwrap_or(22)));
    }
    let mut kg = std::process::Command::new("ssh-keygen")
        .args(["-l", "-f", "-"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()?;
    {
        use std::io::Write;
        kg.stdin.take().unwrap().write_all(lineas.as_bytes())?;
    }
    let huellas = String::from_utf8_lossy(&kg.wait_with_output()?.stdout).lines().map(String::from).collect();
    Ok((lineas, huellas))
}

/// Fija las llaves del servidor en el known_hosts del PROYECTO (reemplaza las anteriores de ese host).
pub fn fijar_huella(root: &Path, c: &Conexion, lineas: &str) -> Result<()> {
    let kh = ruta_known_hosts(root);
    std::fs::create_dir_all(kh.parent().unwrap())?;
    let clave_host = match c.puerto {
        Some(p) if p != 22 => format!("[{}]:{p}", c.host),
        _ => c.host.clone(),
    };
    let previo = std::fs::read_to_string(&kh).unwrap_or_default();
    let mut nuevo: Vec<String> = previo.lines().filter(|l| l.split_whitespace().next() != Some(clave_host.as_str())).map(String::from).collect();
    nuevo.extend(lineas.lines().map(String::from));
    std::fs::write(&kh, nuevo.join("\n") + "\n")?;
    Ok(())
}

/// Importa una entrada `Host` del ~/.ssh/config del usuario (HostName, User, Port, IdentityFile):
/// el usuario la trae EXPLÍCITAMENTE a un proyecto; sin esto, los proyectos no la ven.
pub fn importar_de_ssh_config(alias: &str, nombre: &str) -> Result<Conexion> {
    let texto = std::fs::read_to_string(home().join(".ssh/config")).context("~/.ssh/config")?;
    let mut dentro = false;
    let mut c = Conexion { nombre: nombre.to_string(), host: alias.to_string(), ..Default::default() };
    let mut hallado = false;
    for l in texto.lines() {
        let l = l.trim();
        let mut p = l.splitn(2, |ch: char| ch.is_whitespace() || ch == '=');
        let (k, v) = (p.next().unwrap_or("").to_lowercase(), p.next().unwrap_or("").trim().trim_matches('"').to_string());
        if k == "host" {
            dentro = v.split_whitespace().any(|h| h == alias);
            hallado |= dentro;
            continue;
        }
        if !dentro || v.is_empty() {
            continue;
        }
        match k.as_str() {
            "hostname" => c.host = v,
            "user" => c.usuario = Some(v),
            "port" => c.puerto = v.parse().ok(),
            "identityfile" if c.llave.is_none() => c.llave = Some(v),
            _ => {}
        }
    }
    if !hallado {
        bail!("{}", crate::tr!("«{alias}» no está en ~/.ssh/config", "«{alias}» is not in ~/.ssh/config"));
    }
    c.validar()?;
    Ok(c)
}

// ── permisos ───────────────────────────────────────────────────────────────────

/// Órdenes remotas que SÓLO leen (pasan siempre, también en modo plan).
const LEEN: &[&str] = &[
    "ls", "cat", "head", "tail", "grep", "egrep", "rg", "find", "stat", "wc", "du", "df", "free", "uptime", "uname", "whoami", "id",
    "hostname", "ps", "pgrep", "date", "pwd", "which", "file", "lsb_release", "nproc", "lscpu", "lsblk", "w", "last", "ss", "netstat",
    "ip", "journalctl", "dmesg", "top", "vmstat", "iostat", "sha256sum", "md5sum", "tree", "less", "sort", "uniq", "cut", "awk", "sed",
    "echo", "printf", "test", "true", "sw_vers", "system_profiler", "diskutil", "sysctl", "launchctl",
];

/// Subórdenes de lectura de herramientas que también escriben.
const LEEN_SUB: &[(&str, &[&str])] = &[
    ("systemctl", &["status", "list-units", "list-unit-files", "is-active", "is-enabled", "show", "cat"]),
    ("docker", &["ps", "logs", "images", "inspect", "stats", "version", "info", "top"]),
    ("git", &["status", "log", "diff", "show", "branch", "remote", "rev-parse", "describe"]),
    ("kubectl", &["get", "describe", "logs", "top", "version"]),
    ("apt", &["list", "show", "policy"]),
    ("brew", &["list", "info", "outdated"]),
    ("npm", &["ls", "list", "view", "outdated"]),
    ("pm2", &["list", "ls", "status", "logs", "show"]),
    ("nginx", &["-t", "-T", "-v", "-V"]),
];

/// ¿Esta orden remota sólo lee? (cada trozo de `&&`, `;`, `|`; sin redirecciones a archivo)
pub fn solo_lee(cmd: &str) -> bool {
    if cmd.contains('>') || cmd.contains('`') || cmd.contains("$(") {
        return false;
    }
    cmd.split(['|', ';', '&', '\n']).map(str::trim).filter(|p| !p.is_empty()).all(|parte| {
        let w: Vec<&str> = parte.split_whitespace().collect();
        let Some(p0) = w.first() else { return true };
        let p0 = p0.rsplit('/').next().unwrap_or(p0);
        if p0 == "sudo" {
            return false;
        }
        if p0 == "find" && w.iter().any(|t| matches!(*t, "-delete" | "-exec" | "-execdir" | "-ok" | "-fprint")) {
            return false;
        }
        if p0 == "sed" && w.iter().any(|t| t.starts_with("-i")) {
            return false;
        }
        if p0 == "awk" && parte.contains("system(") {
            return false;
        }
        if LEEN.contains(&p0) {
            return true;
        }
        LEEN_SUB.iter().any(|(h, subs)| *h == p0 && w.get(1).is_some_and(|s| subs.contains(s)))
    })
}

/// Lo que puede dejar un servidor inservible o sin acceso: pregunta SIEMPRE (también en bypass).
pub fn peligro_remoto(cmd: &str) -> Option<&'static str> {
    let c = cmd.to_lowercase();
    let w: Vec<&str> = c.split(|ch: char| ch.is_whitespace() || ch == ';' || ch == '&' || ch == '|').filter(|t| !t.is_empty()).collect();
    let tiene = |x: &str| w.contains(&x);
    if tiene("reboot") || tiene("shutdown") || tiene("halt") || tiene("poweroff") || c.contains("init 0") || c.contains("init 6") {
        return Some(crate::i18n::t("reinicia o apaga el servidor", "reboots or shuts down the server"));
    }
    if w.iter().any(|t| t.starts_with("mkfs")) || tiene("fdisk") || tiene("parted") || tiene("wipefs") || c.contains("dd if=") && c.contains("of=/dev/") {
        return Some(crate::i18n::t("formatea o escribe un disco", "formats or writes a disk"));
    }
    if c.contains("rm -rf /") && !c.contains("rm -rf /tmp/") || c.contains("rm -rf ~") || c.contains("rm -rf *") || c.contains("rm -rf .") {
        return Some(crate::i18n::t("borra en masa", "mass deletion"));
    }
    if (tiene("iptables") && (c.contains(" -f") || c.contains("--flush") || c.contains("-p input drop")))
        || c.contains("ufw disable")
        || c.contains("ufw default deny")
        || c.contains("nft flush")
    {
        return Some(crate::i18n::t("toca el cortafuegos (puede dejarte fuera)", "touches the firewall (it can lock you out)"));
    }
    if (c.contains("sshd") || c.contains("ssh.service") || c.contains(" ssh ")) && (tiene("stop") || tiene("disable") || tiene("mask")) || c.contains("sshd_config") && (c.contains('>') || c.contains("sed -i")) {
        return Some(crate::i18n::t("toca el acceso SSH (puedes quedarte fuera)", "touches SSH access (you can get locked out)"));
    }
    if c.contains("authorized_keys") && (c.contains('>') || c.contains("rm ") || c.contains("sed -i")) {
        return Some(crate::i18n::t("cambia las llaves autorizadas", "changes the authorized keys"));
    }
    if tiene("userdel") || tiene("passwd") || tiene("chpasswd") || tiene("deluser") || tiene("usermod") {
        return Some(crate::i18n::t("cambia usuarios o contraseñas", "changes users or passwords"));
    }
    if c.contains("chmod -r 777 /") || c.contains("chown -r") && c.contains(" /") && !c.contains(" /home") && !c.contains(" /var/www") && !c.contains(" /opt") {
        return Some(crate::i18n::t("cambia permisos de todo el sistema", "changes permissions system-wide"));
    }
    if ["drop database", "drop table", "drop schema", "truncate table", "dropdb "].iter().any(|x| c.contains(x)) {
        return Some(crate::i18n::t("borra una base de datos o tabla", "drops a database or table"));
    }
    if c.contains("docker system prune") || c.contains("docker volume rm") || c.contains("docker volume prune") {
        return Some(crate::i18n::t("borra datos de Docker", "deletes Docker data"));
    }
    if c.contains("crontab -r") {
        return Some(crate::i18n::t("borra las tareas programadas", "deletes the scheduled tasks"));
    }
    None
}

/// Qué hacer con una llamada `ssh`/`ssh_archivo` (después de las reglas deny del usuario).
pub fn decidir(root: &Path, tool: &str, args: &serde_json::Value, modo: crate::permissions::Mode, permitido_explicito: bool) -> crate::permissions::Decision {
    use crate::permissions::{Decision, Mode};
    let nombre = args["conexion"].as_str().unwrap_or("");
    let reg = Registro::cargar(root).unwrap_or_default();
    let Some(c) = reg.buscar(nombre) else {
        return Decision::Deny(no_existe(&reg, nombre));
    };
    if tool == "ssh_archivo" {
        let subir = args["direccion"].as_str() == Some("subir");
        if subir && c.solo_lectura {
            return Decision::Deny(crate::tr!("«{nombre}» es de sólo lectura: no se sube nada", "«{nombre}» is read-only: nothing is uploaded"));
        }
        let desc = crate::tr!(
            "🛰 REMOTO · {} {} · {}",
            "🛰 REMOTE · {} {} · {}",
            nombre,
            if subir { "↑ subir" } else { "↓ bajar" },
            format!("{} {} {}", args["local"].as_str().unwrap_or(""), if subir { "→" } else { "←" }, args["remoto"].as_str().unwrap_or(""))
        );
        return match modo {
            Mode::Plan => Decision::Deny(crate::i18n::t("modo plan: sólo lectura", "plan mode: read-only").into()),
            _ if permitido_explicito => Decision::Allow,
            Mode::Bypass => Decision::Allow,
            // bajar al proyecto es como editar el proyecto; subir cambia el servidor
            Mode::AcceptEdits | Mode::Auto if !subir => Decision::Allow,
            _ => Decision::Ask(desc),
        };
    }
    let cmd = args["cmd"].as_str().unwrap_or("");
    if solo_lee(cmd) {
        return Decision::Allow;
    }
    if c.solo_lectura {
        return Decision::Deny(crate::tr!(
            "«{nombre}» es de sólo lectura: esa orden cambia algo en el servidor",
            "«{nombre}» is read-only: that command changes something on the server"
        ));
    }
    if modo == Mode::Plan {
        return Decision::Deny(crate::i18n::t("modo plan: en el servidor sólo se lee", "plan mode: only reading on the server").into());
    }
    if let Some(por) = peligro_remoto(cmd) {
        if !permitido_explicito {
            return Decision::Ask(crate::tr!("⚠ REMOTO PELIGROSO · {nombre} · {cmd} — {por}", "⚠ DANGEROUS REMOTE · {nombre} · {cmd} — {por}"));
        }
    }
    if permitido_explicito || modo == Mode::Bypass {
        return Decision::Allow;
    }
    Decision::Ask(crate::tr!("🛰 REMOTO · {nombre} · {cmd}", "🛰 REMOTE · {nombre} · {cmd}"))
}

/// Lo que ve el modelo en la descripción de la herramienta (sólo las de ESTE proyecto).
pub fn para_el_modelo(root: &Path) -> Option<String> {
    let reg = Registro::cargar(root).ok()?;
    if reg.conexiones.is_empty() {
        return None;
    }
    Some(reg.conexiones.iter().map(|c| format!("- {}", c.resumen())).collect::<Vec<_>>().join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::permissions::{Decision, Mode};
    use serde_json::json;

    fn proyecto(n: &str) -> crate::CarpetaPrueba {
        let d = crate::CarpetaPrueba::sin_crear(&format!("gotv-conex-{n}"));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// La bitácora es texto propio (sin JSON); las líneas JSON de antes se siguen leyendo, y un tabulador o
    /// un salto dentro de la orden no rompe la línea. Y el registro de antes en JSON se migra.
    #[test]
    fn bitacora_de_texto_y_registro_migrado() {
        let d = proyecto("bitacora");
        std::fs::create_dir_all(d.join(".gotv")).unwrap();
        std::fs::write(ruta_log(&d), "{\"cuando\":\"ayer\",\"conexion\":\"vieja\",\"orden\":\"ls\",\"salida\":0,\"ms\":5}\n").unwrap();
        auditar(&d, "prod", "echo a\tb\nc", 1, 42);
        let r = registro(&d, 10);
        assert_eq!(r.len(), 2);
        assert_eq!((r[0].conexion.as_str(), r[0].salida, r[0].ms), ("vieja", 0, 5));
        assert_eq!((r[1].conexion.as_str(), r[1].orden.as_str(), r[1].salida, r[1].ms), ("prod", "echo a b c", 1, 42));
        assert!(!std::fs::read_to_string(ruta_log(&d)).unwrap().lines().nth(1).unwrap().starts_with('{'), "lo nuevo no es JSON");
        assert_eq!(registro(&d, 1).len(), 1, "sólo las últimas");
        // el registro de conexiones de antes (JSON) se migra al formato propio sin perder la conexión
        std::fs::write(d.join(".gotv/conexiones.json"), r#"{"conexiones":[{"nombre":"prod","host":"10.0.0.5"}]}"#).unwrap();
        assert_eq!(Registro::cargar(&d).unwrap().conexiones[0].host, "10.0.0.5");
        assert!(ruta_registro(&d).is_file() && d.join(".gotv/conexiones.json.migrado").is_file());
    }

    #[test]
    fn cada_proyecto_ve_solo_sus_conexiones() {
        let (a, b) = (proyecto("a"), proyecto("b"));
        let mut r = Registro::default();
        r.agregar(Conexion { nombre: "prod".into(), host: "10.0.0.5".into(), usuario: Some("deploy".into()), ..Default::default() }).unwrap();
        r.guardar(&a).unwrap();
        assert_eq!(Registro::cargar(&a).unwrap().conexiones.len(), 1);
        assert!(Registro::cargar(&b).unwrap().conexiones.is_empty(), "el otro proyecto no la ve");
        assert!(para_el_modelo(&b).is_none() && para_el_modelo(&a).unwrap().contains("prod → deploy@10.0.0.5"));
        assert_ne!(carpeta_llaves(&a), carpeta_llaves(&b), "llaves separadas por proyecto");
        assert!(carpeta_control(&a).join("c07b70ce757d9f440a5f7144fec82c092cbdc129").to_string_lossy().len() < 104, "el socket cabe");
        assert!(!carpeta_llaves(&a).starts_with(&a), "las llaves NO viven dentro del proyecto");
        // decidir en el proyecto equivocado: no existe
        assert!(matches!(decidir(&b, "ssh", &json!({"conexion": "prod", "cmd": "ls"}), Mode::Bypass, false), Decision::Deny(_)));
        let _ = std::fs::remove_dir_all(&a);
        let _ = std::fs::remove_dir_all(&b);
    }

    #[test]
    fn nada_se_inyecta_como_opcion_de_ssh() {
        for mal in ["-oProxyCommand=sh", "a b", "x;rm", ""] {
            assert!(Conexion { nombre: "x".into(), host: mal.into(), ..Default::default() }.validar().is_err(), "{mal}");
        }
        assert!(Conexion { nombre: "x".into(), host: "h".into(), usuario: Some("-l".into()), ..Default::default() }.validar().is_err());
        assert!(Conexion { nombre: "a b".into(), host: "h".into(), ..Default::default() }.validar().is_err());
        assert!(Conexion { nombre: "web-1".into(), host: "servidor.ejemplo.com".into(), usuario: Some("ubuntu".into()), ..Default::default() }.validar().is_ok());
        assert!(Conexion { nombre: "v6".into(), host: "fe80::1%en0".into(), ..Default::default() }.validar().is_ok());
    }

    #[test]
    fn las_opciones_aislan_y_fijan_la_huella() {
        let a = proyecto("op");
        let mut r = Registro::default();
        r.agregar(Conexion { nombre: "bastion".into(), host: "b.ejemplo.com".into(), usuario: Some("j".into()), puerto: Some(2222), ..Default::default() }).unwrap();
        r.agregar(Conexion { nombre: "db".into(), host: "10.0.0.9".into(), usuario: Some("pg".into()), salto: Some("bastion".into()), carpeta: Some("/srv/app".into()), ..Default::default() }).unwrap();
        let o = opciones(&a, &r, r.buscar("db").unwrap(), false).unwrap();
        // armar las opciones no deja carpetas en ~/.ssh (antes, una vacía por prueba)
        assert!(!carpeta_control(&a).exists(), "opciones() no crea la carpeta de control");
        let t = o.join(" ");
        for k in ["-F /dev/null", "BatchMode=yes", "StrictHostKeyChecking=yes", "GlobalKnownHostsFile=/dev/null", "ControlMaster=auto"] {
            assert!(t.contains(k), "falta {k}: {t}");
        }
        assert!(t.contains(&format!("UserKnownHostsFile={}", ruta_known_hosts(&a).display())));
        assert!(t.contains("ProxyCommand=ssh -F /dev/null") && t.contains("-p 2222 -W %h:%p j@b.ejemplo.com"), "{t}");
        let p = opciones(&a, &r, r.buscar("bastion").unwrap(), true).unwrap();
        assert!(p.windows(2).any(|w| w == ["-P", "2222"]), "scp usa -P");
        assert_eq!(orden_remota(r.buscar("db").unwrap(), "ls"), "cd '/srv/app' && ls");
        // un salto que no es del proyecto, o dos saltos encadenados, no
        assert!(r.clone().agregar(Conexion { nombre: "z".into(), host: "z".into(), salto: Some("nadie".into()), ..Default::default() }).is_err());
        let mut r2 = r.clone();
        r2.agregar(Conexion { nombre: "dos".into(), host: "d".into(), salto: Some("db".into()), ..Default::default() }).unwrap();
        assert!(opciones(&a, &r2, r2.buscar("dos").unwrap(), false).is_err());
        // no se quita un bastión del que otra depende
        assert!(r.quitar("bastion").is_err() && r.quitar("db").is_ok() && r.quitar("bastion").is_ok());
        let _ = std::fs::remove_dir_all(&a);
    }

    #[test]
    fn huella_fijada_por_host_y_puerto() {
        let a = proyecto("kh");
        let c = Conexion { nombre: "x".into(), host: "h.ejemplo".into(), puerto: Some(2200), ..Default::default() };
        fijar_huella(&a, &c, "[h.ejemplo]:2200 ssh-ed25519 AAAAuno").unwrap();
        fijar_huella(&a, &Conexion { nombre: "y".into(), host: "otro".into(), ..Default::default() }, "otro ssh-ed25519 AAAAotro").unwrap();
        fijar_huella(&a, &c, "[h.ejemplo]:2200 ssh-ed25519 AAAAdos").unwrap();
        let kh = std::fs::read_to_string(ruta_known_hosts(&a)).unwrap();
        assert!(kh.contains("AAAAdos") && !kh.contains("AAAAuno") && kh.contains("AAAAotro"), "{kh}");
        let _ = std::fs::remove_dir_all(&a);
    }

    #[test]
    fn leer_pasa_lo_peligroso_pregunta_siempre_y_solo_lectura_niega() {
        for c in ["ls -la /var/www", "df -h && free -m", "systemctl status nginx", "journalctl -u app -n 50 | grep ERROR", "docker ps", "git log --oneline -5", "cat /etc/os-release"] {
            assert!(solo_lee(c), "lee: {c}");
        }
        for c in ["rm x", "echo hola > a", "sed -i s/a/b/ f", "find . -delete", "systemctl restart nginx", "sudo cat /etc/shadow", "cat $(ls)", "docker rm x"] {
            assert!(!solo_lee(c), "no sólo lee: {c}");
        }
        for c in ["mysql -e \"drop table users\"", "dropdb app", "sudo reboot", "rm -rf / --no-preserve-root", "mkfs.ext4 /dev/sdb", "ufw disable", "systemctl stop sshd", "echo k > ~/.ssh/authorized_keys", "passwd root", "psql -c 'DROP DATABASE app'", "crontab -r"] {
            assert!(peligro_remoto(c).is_some(), "peligroso: {c}");
        }
        for c in ["systemctl restart nginx", "rm -rf /tmp/build", "npm ci && pm2 restart app", "docker compose up -d"] {
            assert!(peligro_remoto(c).is_none(), "no peligroso: {c}");
        }
        let a = proyecto("perm");
        let mut r = Registro::default();
        r.agregar(Conexion { nombre: "web".into(), host: "w".into(), ..Default::default() }).unwrap();
        r.agregar(Conexion { nombre: "ver".into(), host: "v".into(), solo_lectura: true, ..Default::default() }).unwrap();
        r.guardar(&a).unwrap();
        let d = |n: &str, cmd: &str, m: Mode| decidir(&a, "ssh", &json!({"conexion": n, "cmd": cmd}), m, false);
        assert!(matches!(d("web", "uptime", Mode::Plan), Decision::Allow), "leer pasa hasta en plan");
        assert!(matches!(d("web", "systemctl restart app", Mode::Bypass), Decision::Allow));
        assert!(matches!(d("web", "systemctl restart app", Mode::Default), Decision::Ask(_)));
        assert!(matches!(d("web", "sudo reboot", Mode::Bypass), Decision::Ask(_)), "peligroso pregunta también en bypass");
        assert!(matches!(d("web", "systemctl restart app", Mode::Plan), Decision::Deny(_)));
        assert!(matches!(d("ver", "systemctl restart app", Mode::Bypass), Decision::Deny(_)), "sólo lectura niega");
        assert!(matches!(d("ver", "df -h", Mode::Default), Decision::Allow));
        let t = |dir: &str, m: Mode| decidir(&a, "ssh_archivo", &json!({"conexion": "web", "direccion": dir, "local": "x", "remoto": "/tmp/x"}), m, false);
        assert!(matches!(t("bajar", Mode::AcceptEdits), Decision::Allow) && matches!(t("subir", Mode::AcceptEdits), Decision::Ask(_)));
        assert!(matches!(decidir(&a, "ssh_archivo", &json!({"conexion": "ver", "direccion": "subir"}), Mode::Bypass, false), Decision::Deny(_)));
        let _ = std::fs::remove_dir_all(&a);
    }

    #[test]
    fn la_fecha_del_registro_es_valida() {
        let f = chrono_simple();
        assert!(f.len() == 20 && f.ends_with('Z') && f.starts_with("20"), "{f}");
    }
}
