//! F16.3 · Proyecto en otro equipo: la capa `Lugar` entre las herramientas y el disco.
//!
//! - [`Local`]: el disco de esta Mac (lo de siempre). Las herramientas locales NO cambian de conducta: el
//!   proyecto local sigue por su camino de hoy en `tools.rs` (decisión: la suite entera pasa igual porque el
//!   camino local no se tocó; `Local` cumple el mismo contrato que `Remoto` y se prueba con él).
//! - [`Remoto`]: un proyecto que vive en otra máquina, sobre una conexión de `conexiones.rs` (huella fijada,
//!   llave propia, `-F /dev/null`) y UNA sesión `ssh` multiplexada por conexión (`ControlMaster` del propio
//!   ssh). Nada se instala allá: sólo shell POSIX, `cat`, `mv`, `grep`, `wc`.
//!
//! **Un proyecto remoto** es una carpeta ANCLA en esta Mac (`~/.config/gotv/remotos/<nombre>/`) con
//! `.gotv/remoto.bin` (códec propio: conexión y raíz remota) y la conexión en su registro (`.gotv/conexiones.gotv`).
//! El agente trabaja con esa ancla como raíz (sesiones, permisos, registros viven aquí) y `tools.rs` manda
//! `read_file`, `write_file`, `edit`, `multi_edit`, `list_dir`, `glob`, `grep` y `bash` al `Remoto`.
//!
//! Trampas de la ficha, una por una:
//! 1. Cada operación es un viaje: se mide (pruebas: ms por operación) y la sesión se multiplexa.
//! 2. Allá no hay caja de arena: en un proyecto remoto todo `bash` que no sea de sólo lectura PREGUNTA, también
//!    en `acceptEdits` ([`decision`]). En bypass no se pregunta (decisión del Portador: bypass = control total).
//! 3. El índice por significado, el mapa y las pruebas estructuradas necesitan los archivos aquí: en remoto NO
//!    existen y se dice ([`no_en_remoto`]). Los servidores de lenguaje SÍ: corren allá (`lsp/remoto.rs`).
//! 4. Ningún secreto de allá viaja a un registro de aquí: las órdenes van a la bitácora de conexiones (como
//!    `ssh`), la salida no se guarda en disco.
//! 5. La ruta remota nunca pasa por `canonicalize` local: se normaliza a mano ([`normalizar`]) y la que escapa
//!    de la raíz se niega.
use crate::programar::codec::{self, Escritor, Lector};
use anyhow::{anyhow, bail, Result};
use async_trait::async_trait;
use serde_json::Value;
use std::path::{Component, Path, PathBuf};
use std::time::Duration;
use tokio::io::AsyncWriteExt;

const TIPO: &[u8; 4] = b"LUGR";
/// Tope de lectura por operación (bytes).
pub const LEER_MAX: u64 = 4 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq)]
pub struct Entrada {
    pub nombre: String,
    pub dir: bool,
    pub tam: u64,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Meta {
    pub existe: bool,
    pub dir: bool,
    pub tam: u64,
}

#[derive(Debug, Clone)]
pub struct Ejecucion {
    pub segundos: u64,
}

impl Default for Ejecucion {
    fn default() -> Self {
        Ejecucion { segundos: 120 }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Salida {
    pub codigo: i32,
    pub texto: String,
}

#[async_trait]
pub trait Lugar: Send + Sync {
    /// la raíz tal como se ve ALLÁ
    fn raiz(&self) -> &Path;
    fn remoto(&self) -> bool;
    async fn leer(&self, ruta: &Path, desde: u64, tope: u64) -> Result<Vec<u8>>;
    /// atómica: temporal + comprobar tamaño + mover (un corte a mitad no deja el archivo a medias)
    async fn escribir(&self, ruta: &Path, datos: &[u8]) -> Result<()>;
    async fn listar(&self, ruta: &Path) -> Result<Vec<Entrada>>;
    async fn metadatos(&self, ruta: &Path) -> Result<Meta>;
    /// corre allá: líneas `ruta:línea:texto`
    async fn buscar(&self, patron: &str, glob: Option<&str>) -> Result<String>;
    async fn ejecutar(&self, orden: &str, op: &Ejecucion) -> Result<Salida>;
}

/// Normaliza `ruta` contra `raiz` SIN tocar el disco (sirve para rutas de otra máquina). `Err` si escapa.
pub fn normalizar(raiz: &Path, ruta: &Path) -> Result<PathBuf> {
    let mut partes: Vec<std::ffi::OsString> = Vec::new();
    let rel = if ruta.is_absolute() { ruta.strip_prefix(raiz).map_err(|_| fuera(ruta))? } else { ruta };
    for c in rel.components() {
        match c {
            Component::Normal(x) => partes.push(x.to_os_string()),
            Component::CurDir => {}
            Component::ParentDir => {
                if partes.pop().is_none() {
                    return Err(fuera(ruta));
                }
            }
            _ => return Err(fuera(ruta)),
        }
    }
    let mut p = raiz.to_path_buf();
    for x in partes {
        p.push(x);
    }
    Ok(p)
}

fn fuera(ruta: &Path) -> anyhow::Error {
    anyhow!("{}", crate::tr!("«{}» escapa de la raíz del proyecto: denegado", "«{}» escapes the project root: denied", ruta.display()))
}

/// Comillas simples de shell POSIX.
pub fn q(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

// ───────────────────────── Local ─────────────────────────

pub struct Local {
    raiz: PathBuf,
}

impl Local {
    pub fn new(raiz: &Path) -> Local {
        Local { raiz: raiz.to_path_buf() }
    }
    fn dentro(&self, ruta: &Path) -> Result<PathBuf> {
        let p = normalizar(&self.raiz, ruta)?;
        // aquí sí se puede mirar el disco: un enlace que apunta fuera tampoco vale
        let mut e = p.clone();
        while !e.exists() {
            match e.parent() {
                Some(x) => e = x.to_path_buf(),
                None => break,
            }
        }
        if e.exists() && !e.canonicalize()?.starts_with(self.raiz.canonicalize()?) {
            return Err(fuera(ruta));
        }
        Ok(p)
    }
}

#[async_trait]
impl Lugar for Local {
    fn raiz(&self) -> &Path {
        &self.raiz
    }
    fn remoto(&self) -> bool {
        false
    }
    async fn leer(&self, ruta: &Path, desde: u64, tope: u64) -> Result<Vec<u8>> {
        use std::io::{Read, Seek};
        let mut f = std::fs::File::open(self.dentro(ruta)?)?;
        f.seek(std::io::SeekFrom::Start(desde))?;
        let mut b = Vec::new();
        f.take(tope.min(LEER_MAX)).read_to_end(&mut b)?;
        Ok(b)
    }
    async fn escribir(&self, ruta: &Path, datos: &[u8]) -> Result<()> {
        let p = self.dentro(ruta)?;
        if let Some(d) = p.parent() {
            std::fs::create_dir_all(d)?;
        }
        codec::guardar(&p, datos)
    }
    async fn listar(&self, ruta: &Path) -> Result<Vec<Entrada>> {
        let mut v: Vec<Entrada> = std::fs::read_dir(self.dentro(ruta)?)?
            .flatten()
            .map(|e| {
                let m = e.metadata().ok();
                Entrada { nombre: e.file_name().to_string_lossy().to_string(), dir: m.as_ref().is_some_and(|m| m.is_dir()), tam: m.map(|m| m.len()).unwrap_or(0) }
            })
            .collect();
        v.sort_by(|a, b| a.nombre.cmp(&b.nombre));
        Ok(v)
    }
    async fn metadatos(&self, ruta: &Path) -> Result<Meta> {
        Ok(match std::fs::metadata(self.dentro(ruta)?) {
            Ok(m) => Meta { existe: true, dir: m.is_dir(), tam: m.len() },
            Err(_) => Meta::default(),
        })
    }
    async fn buscar(&self, patron: &str, glob: Option<&str>) -> Result<String> {
        let s = self.ejecutar(&orden_buscar(patron, glob), &Ejecucion::default()).await?;
        Ok(s.texto)
    }
    async fn ejecutar(&self, orden: &str, op: &Ejecucion) -> Result<Salida> {
        let mut c = tokio::process::Command::new("bash");
        c.arg("-lc").arg(orden).current_dir(&self.raiz).stdin(std::process::Stdio::null()).kill_on_drop(true);
        let o = tokio::time::timeout(Duration::from_secs(op.segundos), c.output()).await.map_err(|_| anyhow!("timeout"))??;
        Ok(Salida { codigo: o.status.code().unwrap_or(-1), texto: unir(&o.stdout, &o.stderr) })
    }
}

fn unir(out: &[u8], err: &[u8]) -> String {
    let mut s = String::from_utf8_lossy(out).to_string();
    let e = String::from_utf8_lossy(err);
    if !e.trim().is_empty() {
        if !s.is_empty() && !s.ends_with('\n') {
            s.push('\n');
        }
        s.push_str(e.trim_end());
    }
    s
}

fn orden_buscar(patron: &str, glob: Option<&str>) -> String {
    let inc = glob.map(|g| format!(" --include={}", q(g))).unwrap_or_default();
    format!("grep -rnI --exclude-dir=.git --exclude-dir=.gotv --exclude-dir=node_modules --exclude-dir=target{inc} -e {} . 2>/dev/null | sed 's|^\\./||' | head -n 400", q(patron))
}

// ───────────────────────── Remoto ─────────────────────────

pub struct Remoto {
    /// el ANCLA local (donde vive la conexión con su huella)
    pub ancla: PathBuf,
    pub conexion: String,
    raiz: PathBuf,
}

impl Remoto {
    pub fn new(ancla: &Path, conexion: &str, raiz: &Path) -> Remoto {
        Remoto { ancla: ancla.to_path_buf(), conexion: conexion.into(), raiz: raiz.to_path_buf() }
    }

    /// Una orden allá, con `entrada` por la entrada estándar. Devuelve (código, salida, error) crudos.
    /// El `ssh` que corre `orden` ALLÁ, en la raíz, por la sesión multiplexada (sin lanzarlo). Lo usa también
    /// el servidor de lenguaje remoto (F16.3 paso 5), que necesita sus tuberías vivas.
    pub(crate) fn comando(&self, orden: &str) -> Result<tokio::process::Command> {
        let mut args = crate::conexiones::argumentos_ssh(&self.ancla, &self.conexion)?;
        // siempre `sh` POSIX: la shell de inicio de sesión de allá puede ser zsh o fish (medido: zsh rompe un
        // `for` con un patrón que no casa)
        args.push(format!("sh -c {}", q(&format!("cd {} && {orden}", q(&self.raiz.display().to_string())))));
        let mut c = tokio::process::Command::new("ssh");
        c.args(&args);
        Ok(c)
    }

    /// Los argumentos de `ssh` para la terminal del USUARIO en un proyecto remoto (F16.7): con terminal allá
    /// (`-tt`), en la raíz, con su shell de inicio de sesión.
    pub(crate) fn argumentos_terminal(&self) -> Result<Vec<String>> {
        let mut a = crate::conexiones::argumentos_ssh(&self.ancla, &self.conexion)?;
        // (los dos últimos son el destino y «--»)
        let n = a.len().saturating_sub(2);
        a.insert(n, "-tt".into());
        a.push(format!("sh -c {}", q(&format!("cd {} && exec \"${{SHELL:-/bin/sh}}\" -l", q(&self.raiz.display().to_string())))));
        Ok(a)
    }

    /// Una orden allá y su salida (código, salida, error), para quien no es una herramienta (el LSP remoto).
    pub(crate) async fn correr(&self, orden: &str, segundos: u64) -> Result<(i32, String, String)> {
        let (rc, o, e) = self.ssh(orden, None, segundos).await?;
        Ok((rc, String::from_utf8_lossy(&o).into_owned(), String::from_utf8_lossy(&e).into_owned()))
    }

    async fn ssh(&self, orden: &str, entrada: Option<&[u8]>, segundos: u64) -> Result<(i32, Vec<u8>, Vec<u8>)> {
        let mut c = self.comando(orden)?;
        c.stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped()).kill_on_drop(true);
        c.stdin(if entrada.is_some() { std::process::Stdio::piped() } else { std::process::Stdio::null() });
        let t0 = std::time::Instant::now();
        let mut hijo = c.spawn().map_err(|e| anyhow!("ssh: {e}"))?;
        if let (Some(d), Some(mut w)) = (entrada, hijo.stdin.take()) {
            w.write_all(d).await?;
            w.shutdown().await?;
        }
        let o = tokio::time::timeout(Duration::from_secs(segundos.clamp(5, 3600)), hijo.wait_with_output())
            .await
            .map_err(|_| anyhow!("{}", crate::tr!("la orden remota pasó de {segundos} s (se cortó)", "the remote command went past {segundos} s (cut)")))??;
        let rc = o.status.code().unwrap_or(-1);
        // trampa 4: a la bitácora va la orden (sin datos), nunca la salida
        crate::conexiones::auditar(&self.ancla, &self.conexion, &format!("[lugar] {}", orden.chars().take(300).collect::<String>()), rc, t0.elapsed().as_millis());
        if rc == 255 {
            bail!("{}", crate::conexiones::explicar_fallo(&self.conexion, &String::from_utf8_lossy(&o.stderr)));
        }
        Ok((rc, o.stdout, o.stderr))
    }

    fn rel(&self, ruta: &Path) -> Result<String> {
        let p = normalizar(&self.raiz, ruta)?;
        let r = p.strip_prefix(&self.raiz).unwrap_or(&p).display().to_string();
        Ok(if r.is_empty() { ".".into() } else { r })
    }

    /// Antes de escribir: guarda lo que había (o que no existía) en `<raíz>/.gotv/puntos/<turno>/` ALLÁ.
    async fn punto(&self, rel: &str) -> Result<()> {
        let turno = turno_actual(&self.ancla);
        let dir = format!(".gotv/puntos/{turno}");
        let orden = format!(
            "mkdir -p {d}/archivos && if grep -qxF -- {r} {d}/lista 2>/dev/null; then :; elif [ -f {r} ]; then mkdir -p \"$(dirname {d}/archivos/{r})\" && cp -p -- {r} {d}/archivos/{r} && printf '%s\\n' {r} >> {d}/lista; else printf '%s\\n' {r} >> {d}/lista; fi",
            d = q(&dir),
            r = q(rel)
        );
        let (rc, _, e) = self.ssh(&orden, None, 60).await?;
        if rc != 0 {
            bail!("{}", crate::tr!("no pude guardar el punto de restauración allá: {}", "could not save the restore point there: {}", String::from_utf8_lossy(&e).trim()));
        }
        Ok(())
    }

    /// Deshace lo escrito en el turno `turno`: restaura lo que había y borra lo que no existía.
    pub async fn deshacer(&self, turno: u64) -> Result<usize> {
        let d = q(&format!(".gotv/puntos/{turno}"));
        let orden = format!(
            "[ -f {d}/lista ] || exit 4; n=0; while IFS= read -r r; do if [ -f {d}/archivos/\"$r\" ]; then mkdir -p \"$(dirname \"$r\")\" && cp -p -- {d}/archivos/\"$r\" \"$r\"; else rm -f -- \"$r\"; fi; n=$((n+1)); done < {d}/lista; rm -rf {d}; echo $n"
        );
        let (rc, o, e) = self.ssh(&orden, None, 120).await?;
        match rc {
            0 => Ok(String::from_utf8_lossy(&o).trim().parse().unwrap_or(0)),
            4 => bail!("{}", crate::i18n::t("ese turno no escribió nada allá", "that turn wrote nothing there")),
            _ => bail!("{}", String::from_utf8_lossy(&e).trim().to_string()),
        }
    }
}

/// La escritura atómica de allá: el tamaño se comprueba ANTES de mover. Si la conexión se corta a mitad,
/// `cat` ve el fin de la entrada con menos bytes y el archivo de verdad no se toca.
fn orden_escribir(rel: &str, n: usize) -> String {
    let r = q(rel);
    let tmp = q(&format!("{rel}.gotv-tmp-{}", std::process::id()));
    format!("mkdir -p \"$(dirname {r})\" && cat > {tmp} && n=$(wc -c < {tmp} | tr -d ' ') && if [ \"$n\" = {n} ]; then mv -f {tmp} {r}; else rm -f {tmp}; echo \"llegaron $n de {n} bytes\" >&2; exit 5; fi")
}

#[async_trait]
impl Lugar for Remoto {
    fn raiz(&self) -> &Path {
        &self.raiz
    }
    fn remoto(&self) -> bool {
        true
    }
    async fn leer(&self, ruta: &Path, desde: u64, tope: u64) -> Result<Vec<u8>> {
        let r = q(&self.rel(ruta)?);
        let orden = format!("[ -f {r} ] || {{ echo no-existe >&2; exit 3; }}; tail -c +{} -- {r} | head -c {}", desde + 1, tope.min(LEER_MAX));
        let (rc, o, e) = self.ssh(&orden, None, 60).await?;
        if rc != 0 {
            bail!("{}", crate::tr!("no pude leer {} allá: {}", "could not read {} there: {}", ruta.display(), String::from_utf8_lossy(&e).trim()));
        }
        Ok(o)
    }
    async fn escribir(&self, ruta: &Path, datos: &[u8]) -> Result<()> {
        let rel = self.rel(ruta)?;
        self.punto(&rel).await?;
        let (rc, _, e) = self.ssh(&orden_escribir(&rel, datos.len()), Some(datos), 300).await?;
        if rc != 0 {
            bail!("{}", crate::tr!("no pude escribir {} allá: {}", "could not write {} there: {}", ruta.display(), String::from_utf8_lossy(&e).trim()));
        }
        Ok(())
    }
    async fn listar(&self, ruta: &Path) -> Result<Vec<Entrada>> {
        let r = q(&self.rel(ruta)?);
        let orden = format!(
            "[ -d {r} ] || {{ echo no-es-carpeta >&2; exit 3; }}; cd {r} && for f in * .[!.]* ..?*; do [ -e \"$f\" ] || [ -L \"$f\" ] || continue; if [ -d \"$f\" ]; then printf 'd\\t0\\t%s\\n' \"$f\"; else printf 'f\\t%s\\t%s\\n' \"$(wc -c < \"$f\" 2>/dev/null | tr -d ' ')\" \"$f\"; fi; done"
        );
        let (rc, o, e) = self.ssh(&orden, None, 60).await?;
        if rc != 0 {
            bail!("{}", crate::tr!("no pude listar {} allá: {}", "could not list {} there: {}", ruta.display(), String::from_utf8_lossy(&e).trim()));
        }
        let mut v: Vec<Entrada> = String::from_utf8_lossy(&o)
            .lines()
            .filter_map(|l| {
                let mut p = l.splitn(3, '\t');
                let (k, t, n) = (p.next()?, p.next()?, p.next()?);
                Some(Entrada { nombre: n.to_string(), dir: k == "d", tam: t.parse().unwrap_or(0) })
            })
            .collect();
        v.sort_by(|a, b| a.nombre.cmp(&b.nombre));
        Ok(v)
    }
    async fn metadatos(&self, ruta: &Path) -> Result<Meta> {
        let r = q(&self.rel(ruta)?);
        let (_, o, _) = self.ssh(&format!("if [ -d {r} ]; then echo d 0; elif [ -f {r} ]; then echo f $(wc -c < {r} | tr -d ' '); else echo n 0; fi"), None, 30).await?;
        let s = String::from_utf8_lossy(&o);
        let mut p = s.split_whitespace();
        let k = p.next().unwrap_or("n");
        Ok(Meta { existe: k != "n", dir: k == "d", tam: p.next().and_then(|t| t.parse().ok()).unwrap_or(0) })
    }
    async fn buscar(&self, patron: &str, glob: Option<&str>) -> Result<String> {
        let (_, o, _) = self.ssh(&orden_buscar(patron, glob), None, 120).await?;
        Ok(String::from_utf8_lossy(&o).to_string())
    }
    async fn ejecutar(&self, orden: &str, op: &Ejecucion) -> Result<Salida> {
        let (rc, o, e) = self.ssh(&format!("bash -lc {}", q(orden)), None, op.segundos).await?;
        Ok(Salida { codigo: rc, texto: unir(&o, &e) })
    }
}

// ───────────────────────── proyectos remotos ─────────────────────────

/// Lo que guarda el ancla: la conexión y la raíz allá.
#[derive(Debug, Clone, PartialEq)]
pub struct Ficha {
    pub conexion: String,
    pub raiz: String,
}

fn ruta_ficha(ancla: &Path) -> PathBuf {
    ancla.join(".gotv/remoto.bin")
}

pub fn guardar_ficha(ancla: &Path, f: &Ficha) -> Result<()> {
    if !f.raiz.starts_with('/') {
        bail!("{}", crate::i18n::t("la raíz remota debe ser una ruta absoluta", "the remote root must be an absolute path"));
    }
    std::fs::create_dir_all(ancla.join(".gotv"))?;
    let mut e = Escritor::nuevo(TIPO, 1);
    e.texto(&f.conexion);
    e.texto(&f.raiz);
    codec::guardar(&ruta_ficha(ancla), &e.bytes())
}

pub fn ficha(ancla: &Path) -> Option<Ficha> {
    let b = std::fs::read(ruta_ficha(ancla)).ok()?;
    let (mut l, _) = Lector::nuevo(&b, TIPO, 1).ok()?;
    Some(Ficha { conexion: l.texto().ok()?, raiz: l.texto().ok()? })
}

/// ¿Es `root` el ancla de un proyecto remoto? Entonces su `Remoto`.
pub fn remoto_de(root: &Path) -> Option<Remoto> {
    let f = ficha(root)?;
    Some(Remoto::new(root, &f.conexion, Path::new(&f.raiz)))
}

/// La carpeta de las anclas.
pub fn dir_anclas() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".config/gotv/remotos")
}

/// Crea (o reutiliza) el ancla de un proyecto remoto. La conexión se agrega/fija aparte (`conexiones`).
pub fn crear_ancla(nombre: &str, f: &Ficha) -> Result<PathBuf> {
    if nombre.is_empty() || !nombre.chars().all(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | '.')) || nombre.starts_with('.') {
        bail!("{}", crate::tr!("nombre de proyecto remoto inválido: «{nombre}»", "invalid remote project name: «{nombre}»"));
    }
    let a = dir_anclas().join(nombre);
    guardar_ficha(&a, f)?;
    Ok(a)
}

/// Los proyectos remotos (para la lista de proyectos de la ventana): (ancla, ficha).
pub fn remotos() -> Vec<(PathBuf, Ficha)> {
    let Ok(rd) = std::fs::read_dir(dir_anclas()) else { return vec![] };
    let mut v: Vec<(PathBuf, Ficha)> = rd.flatten().map(|e| e.path()).filter_map(|p| ficha(&p).map(|f| (p, f))).collect();
    v.sort_by(|a, b| a.0.cmp(&b.0));
    v
}

// ───────────────────────── turnos y deshacer ─────────────────────────

fn ruta_turno(ancla: &Path) -> PathBuf {
    ancla.join(".gotv/remoto-turno.bin")
}

fn turno_actual(ancla: &Path) -> u64 {
    std::fs::read(ruta_turno(ancla)).ok().and_then(|b| b.try_into().ok()).map(u64::from_le_bytes).unwrap_or(1)
}

/// Enganche de `run_loop` (orden nueva del agente principal): lo que se escriba desde ahora es otro turno.
pub fn nuevo_turno(root: &Path) {
    if ficha(root).is_none() {
        return;
    }
    let n = turno_actual(root) + 1;
    let _ = codec::guardar(&ruta_turno(root), &n.to_le_bytes());
}

/// `/rewind` en un proyecto remoto: deshace el último turno que escribió allá.
pub async fn deshacer_ultimo(root: &Path) -> Result<String> {
    let r = remoto_de(root).ok_or_else(|| anyhow!("{}", crate::i18n::t("no es un proyecto remoto", "not a remote project")))?;
    let mut t = turno_actual(root);
    while t >= 1 {
        match r.deshacer(t).await {
            Ok(n) => return Ok(crate::tr!("↩ deshecho el turno {t} allá: {n} archivo(s) como estaban", "↩ undid turn {t} there: {n} file(s) as they were")),
            // juez-idioma: datos (se reconoce el error de `deshacer` en los dos idiomas)
            Err(e) if e.to_string().contains("nada allá") || e.to_string().contains("nothing there") => t -= 1,
            Err(e) => return Err(e),
        }
    }
    bail!("{}", crate::i18n::t("no hay turnos que deshacer allá", "there are no turns to undo there"))
}

// ───────────────────────── permisos y herramientas ─────────────────────────

/// Trampa 2: en un proyecto remoto, `bash` que no sólo lee pregunta (también en acceptEdits; no en bypass).
pub fn decision(root: &Path, tool: &str, args: &Value, modo: crate::permissions::Mode) -> Option<crate::permissions::Decision> {
    if tool != "bash" || modo == crate::permissions::Mode::Bypass {
        return None;
    }
    let f = ficha(root)?;
    let cmd = args["cmd"].as_str().or(args["command"].as_str()).unwrap_or("");
    if crate::conexiones::solo_lee(cmd) {
        return None;
    }
    Some(crate::permissions::Decision::Ask(crate::tr!(
        "correr en el equipo REMOTO «{}» ({}), sin caja de arena: {cmd}",
        "run on the REMOTE machine «{}» ({}), without a sandbox: {cmd}",
        f.conexion,
        f.raiz
    )))
}

/// Trampa 3: lo que en remoto no existe (necesita los archivos aquí).
pub fn no_en_remoto(tool: &str) -> bool {
    // (`lsp` sí: su servidor corre allá, `lsp/remoto.rs`)
    matches!(tool, "rag_search" | "mapa" | "diagnostics" | "pruebas" | "depurar" | "process" | "pagina" | "isometrica" | "app_nueva" | "api_nueva" | "excel" | "word" | "powerpoint" | "pdf" | "view_image" | "listen_audio" | "package" | "contenedores" | "vigilar" | "git" | "generar_imagen" | "generar_musica" | "generar_efecto" | "separar_pistas" | "generar_video")
}

fn numerar(texto: &str, desde: usize, n: usize) -> String {
    texto.lines().enumerate().skip(desde.saturating_sub(1)).take(n).map(|(i, l)| format!("{:>6}\t{l}", i + 1)).collect::<Vec<_>>().join("\n")
}

/// Enganche de `tools.rs`: en un proyecto remoto, estas herramientas corren ALLÁ. `None` = no es de las suyas.
pub async fn herramienta(l: &dyn Lugar, nombre: &str, args: &Value) -> Option<Result<String>> {
    let s = |k: &str| args[k].as_str().unwrap_or("").to_string();
    let ruta = |k: &str| PathBuf::from(s(k));
    let r = match nombre {
        "read_file" => leer_texto(l, &ruta("path")).await.map(|t| {
            let total = t.lines().count();
            let desde = args["offset"].as_u64().unwrap_or(1).max(1) as usize;
            let n = args["limit"].as_u64().unwrap_or(400).clamp(1, 2000) as usize;
            let mut out = numerar(&t, desde, n);
            if desde + n <= total {
                out.push_str(&crate::tr!("\n(… {total} líneas; sigue con offset={})", "\n(… {total} lines; continue with offset={})", desde + n));
            }
            out
        }),
        "write_file" => async {
            let p = ruta("path");
            let mut datos = s("content");
            if args["append"].as_bool() == Some(true) && l.metadatos(&p).await?.existe {
                datos = format!("{}{datos}", leer_texto(l, &p).await?);
            }
            l.escribir(&p, datos.as_bytes()).await?;
            Ok::<String, anyhow::Error>(crate::tr!("escrito allá: {} ({} bytes)", "written there: {} ({} bytes)", p.display(), datos.len()))
        }
        .await,
        // los nombres de GOTV (`old`/`new`, los que deja el bucle tras normalizar) y los de Claude Code
        "edit" => editar(l, &ruta("path"), &[par(args)]).await,
        "multi_edit" => {
            let cambios: Vec<(String, String, bool)> = args["edits"].as_array().into_iter().flatten().map(par).collect();
            editar(l, &ruta("path"), &cambios).await
        }
        "list_dir" => {
            let p = if s("path").is_empty() { PathBuf::from(".") } else { ruta("path") };
            l.listar(&p).await.map(|v| v.iter().map(|e| if e.dir { format!("{}/", e.nombre) } else { format!("{}  ({} B)", e.nombre, e.tam) }).collect::<Vec<_>>().join("\n"))
        }
        "glob" => {
            let pat = s("pattern");
            let nombre = Path::new(&pat).file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or(pat.clone());
            l.ejecutar(&format!("find . -path ./.git -prune -o -path ./.gotv -prune -o -type f -name {} -print | sed 's|^\\./||' | head -n 500", q(&nombre)), &Ejecucion::default()).await.map(|x| x.texto)
        }
        "grep" => l.buscar(&s("pattern"), args["glob"].as_str().or(args["include"].as_str())).await.map(|t| if t.trim().is_empty() { crate::i18n::t("(sin coincidencias)", "(no matches)").into() } else { t }),
        "bash" => {
            let seg = args["timeout"].as_u64().unwrap_or(120).clamp(5, 3600);
            l.ejecutar(&s("cmd"), &Ejecucion { segundos: seg }).await.map(|x| format!("(exit {}) [{}]\n{}", x.codigo, crate::i18n::t("en el equipo remoto", "on the remote machine"), x.texto))
        }
        t if no_en_remoto(t) => Err(anyhow!(
            "{}",
            crate::tr!(
                "«{t}» no está en un proyecto REMOTO: necesita los archivos en esta Mac. Usa bash allá (p. ej. corre las pruebas con su orden) o read_file/grep",
                "«{t}» is not available in a REMOTE project: it needs the files on this Mac. Use bash there (e.g. run the tests with their command) or read_file/grep"
            )
        )),
        _ => return None,
    };
    Some(r)
}

/// (viejo, nuevo, todos) de una edición.
fn par(e: &Value) -> (String, String, bool) {
    let t = |a: &str, b: &str| e[a].as_str().or(e[b].as_str()).unwrap_or("").to_string();
    (t("old", "old_string"), t("new", "new_string"), e["replace_all"].as_bool() == Some(true))
}

async fn leer_texto(l: &dyn Lugar, p: &Path) -> Result<String> {
    let b = l.leer(p, 0, LEER_MAX).await?;
    String::from_utf8(b).map_err(|_| anyhow!("{}", crate::tr!("{} es binario (no UTF-8)", "{} is binary (not UTF-8)", p.display())))
}

async fn editar(l: &dyn Lugar, p: &Path, cambios: &[(String, String, bool)]) -> Result<String> {
    let mut t = leer_texto(l, p).await?;
    for (viejo, nuevo, todos) in cambios {
        if viejo.is_empty() {
            bail!("{}", crate::i18n::t("old_string vacío", "empty old_string"));
        }
        let n = t.matches(viejo.as_str()).count();
        match (n, todos) {
            (0, _) => bail!("{}", crate::tr!("old_string no aparece en {}", "old_string does not appear in {}", p.display())),
            (k, false) if k > 1 => bail!("{}", crate::tr!("old_string aparece {k} veces en {}: dale más contexto o usa replace_all", "old_string appears {k} times in {}: give more context or use replace_all", p.display())),
            _ => t = t.replace(viejo.as_str(), nuevo.as_str()),
        }
    }
    l.escribir(p, t.as_bytes()).await?;
    Ok(crate::tr!("editado allá: {}", "edited there: {}", p.display()))
}

#[cfg(test)]
pub(crate) mod pruebas {
    //! Un servidor SSH DESECHABLE propio (`/usr/sbin/sshd` sin root, puerto alto, llaves temporales), para
    //! probar `Remoto` sin tocar máquinas del Portador. Se apaga al soltarlo.
    use super::*;

    pub struct Servidor {
        pub dir: crate::CarpetaPrueba,
        pub puerto: u16,
        pub llave: PathBuf,
        hijo: std::process::Child,
    }

    impl Drop for Servidor {
        fn drop(&mut self) {
            let _ = self.hijo.kill();
            let _ = self.hijo.wait();
        }
    }

    pub fn servidor(tag: &str) -> Option<Servidor> {
        if !Path::new("/usr/sbin/sshd").is_file() {
            return None;
        }
        let dir = crate::CarpetaPrueba::nueva(&format!("gotv-sshd-{tag}"));
        let kg = |f: &str| assert!(std::process::Command::new("ssh-keygen").args(["-q", "-t", "ed25519", "-N", "", "-f"]).arg(dir.join(f)).status().unwrap().success());
        kg("host");
        kg("cliente");
        std::fs::copy(dir.join("cliente.pub"), dir.join("autorizadas")).unwrap();
        let puerto = {
            let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            l.local_addr().unwrap().port()
        };
        std::fs::write(
            dir.join("sshd_config"),
            format!(
                "Port {puerto}\nListenAddress 127.0.0.1\nHostKey {}\nAuthorizedKeysFile {}\nPidFile {}\nUsePAM no\nStrictModes no\nPasswordAuthentication no\nKbdInteractiveAuthentication no\nPubkeyAuthentication yes\n",
                dir.join("host").display(),
                dir.join("autorizadas").display(),
                dir.join("pid").display()
            ),
        )
        .unwrap();
        let hijo = std::process::Command::new("/usr/sbin/sshd").args(["-D", "-e", "-f"]).arg(dir.join("sshd_config")).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn().ok()?;
        for _ in 0..100 {
            if std::net::TcpStream::connect(("127.0.0.1", puerto)).is_ok() {
                break;
            }
            std::thread::sleep(Duration::from_millis(30));
        }
        let llave = dir.join("cliente");
        Some(Servidor { dir, puerto, llave, hijo })
    }

    /// Un ancla local con la conexión `nodo` al servidor y su huella FIJADA (como `gotv conexiones probar`).
    pub fn ancla_con_conexion(s: &Servidor, ancla: &Path, raiz_remota: &Path) {
        let mut reg = crate::conexiones::Registro::cargar(ancla).unwrap_or_default();
        let c = crate::conexiones::Conexion {
            nombre: "nodo".into(),
            host: "127.0.0.1".into(),
            usuario: std::env::var("USER").ok(),
            puerto: Some(s.puerto),
            llave: Some(s.llave.display().to_string()),
            ..Default::default()
        };
        let (lineas, _) = crate::conexiones::escanear(&c).unwrap();
        crate::conexiones::fijar_huella(ancla, &c, &lineas).unwrap();
        reg.agregar(c).unwrap();
        reg.guardar(ancla).unwrap();
        guardar_ficha(ancla, &Ficha { conexion: "nodo".into(), raiz: raiz_remota.display().to_string() }).unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    /// «Hecho cuando» de F16.3, por el bucle REAL del agente (motor de guion): un proyecto en otro equipo se
    /// abre desde esta Mac, el agente lee, edita, corre sus pruebas ALLÁ (bash pregunta, también en
    /// acceptEdits) y el turno se deshace allá.
    #[tokio::test]
    async fn el_agente_trabaja_en_un_proyecto_remoto_y_se_deshace_el_turno() {
        use crate::escritorio::prueba::{agente, llamadas, texto, Guion};
        use serde_json::json;
        let Some(s) = pruebas::servidor("agente") else { return };
        let raiz = s.dir.join("app");
        std::fs::create_dir_all(&raiz).unwrap();
        std::fs::write(raiz.join("calc.sh"), "suma() { echo $(( $1 - $2 )); }\n").unwrap();
        std::fs::write(raiz.join("prueba.sh"), ". ./calc.sh; [ \"$(suma 2 3)\" = 5 ] && echo PASA || { echo FALLA; exit 1; }\n").unwrap();
        let motor = Guion::nuevo("guion", vec![
            llamadas("a", &[("read_file", json!({"path": "calc.sh"})), ("bash", json!({"cmd": "sh prueba.sh"}))]),
            llamadas("b", &[("edit", json!({"path": "calc.sh", "old_string": "$1 - $2", "new_string": "$1 + $2"}))]),
            llamadas("c", &[("bash", json!({"cmd": "sh prueba.sh"}))]),
            texto("arreglado: suma restaba; prueba.sh PASA allá"),
        ]);
        // F16.1 en un proyecto remoto: el Portador escribe mientras el modelo genera el primer paso
        let dueno: std::sync::Arc<std::sync::Mutex<String>> = Default::default();
        let d2 = dueno.clone();
        let mut motor = motor;
        motor.gancho = Some(Box::new(move |n, _| {
            if n == 0 {
                crate::escritorio::cola::encolar(&d2.lock().unwrap(), "ojo: la suma resta", "ventana").unwrap();
            }
        }));
        let visto = motor.visto.clone();
        let mut c = agente("remoto", crate::permissions::Mode::AcceptEdits, motor);
        *dueno.lock().unwrap() = c.agente.proc_owner();
        *c.agente.front.respuesta.lock().unwrap() = Some(crate::permissions::AskAnswer::Yes);
        pruebas::ancla_con_conexion(&s, &c.agente.root.clone(), &raiz);
        let r = c.agente.run("arregla la suma del proyecto remoto").await.unwrap();
        assert!(r.contains("arreglado"), "{r}");
        let v = visto.lock().unwrap();
        let resultado = |i: usize, id: &str| v[i].messages.iter().find(|m| m.tool_call_id.as_deref() == Some(id)).map(|m| m.content.clone()).unwrap_or_default();
        assert!(resultado(1, "a0").contains("$1 - $2"), "leyó el archivo de ALLÁ: {}", resultado(1, "a0"));
        assert!(resultado(1, "a1").contains("FALLA") && resultado(1, "a1").contains("remot"), "corrió las pruebas allá: {}", resultado(1, "a1"));
        assert!(resultado(3, "c0").contains("PASA"), "{}", resultado(3, "c0"));
        // la cola entró entre dos pasos (tras los dos resultados del primero), como en un proyecto local
        let i = v[1].messages.iter().position(|m| m.content.contains("ojo: la suma resta")).expect("entró");
        assert!(v[1].messages[i].es_aviso() && v[1].messages[i - 1].role == "tool");
        assert_eq!(std::fs::read_to_string(raiz.join("calc.sh")).unwrap(), "suma() { echo $(( $1 + $2 )); }\n", "la edición está ALLÁ");
        let preguntas = c.agente.front.preguntas.lock().unwrap().clone();
        // (la pregunta la redacta la puerta por lotes con su formato: «🔴 alto · bash …»)
        assert!(preguntas.iter().filter(|p| p.contains("prueba.sh")).count() >= 2, "bash allá pregunta en acceptEdits: {preguntas:?}");
        // nada del proyecto remoto se escribió en el ancla de aquí
        assert!(!c.agente.root.join("calc.sh").exists());
        // deshacer el turno: allá vuelve a como estaba
        let d = deshacer_ultimo(&c.agente.root).await.unwrap();
        assert!(d.contains('1'), "{d}");
        assert_eq!(std::fs::read_to_string(raiz.join("calc.sh")).unwrap(), "suma() { echo $(( $1 - $2 )); }\n");
        // F16.7: la terminal del usuario de un proyecto remoto es la de ALLÁ, en su raíz
        let ancla = c.agente.root.clone();
        let id = crate::escritorio::terminal::abrir(&ancla, 100, 30, std::sync::Arc::new(|_, _| {})).unwrap();
        crate::escritorio::terminal::escribir(id, b"cd . && echo DONDE=$(pwd)\r").unwrap();
        let mut visto_t = String::new();
        for _ in 0..200 {
            visto_t = crate::escritorio::terminal::texto(id, 50).unwrap_or_default();
            if visto_t.contains(&format!("DONDE={}", raiz.display())) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(30)).await;
        }
        crate::escritorio::terminal::cerrar(id);
        assert!(visto_t.contains(&format!("DONDE={}", raiz.display())), "la terminal abrió allá: {visto_t}");
    }

    /// «Hecho cuando» de F16.3 con un MODELO DE VERDAD (Huihui por `gotv serve`) sobre el `sshd` desechable de
    /// esta Mac: el agente lee, edita y corre las pruebas ALLÁ, y el turno se deshace allá. Mide el tiempo.
    /// `GOTV_PRUEBA_OPENAI=http://127.0.0.1:11437/v1 GOTV_PRUEBA_MODELO=<modelo> cargo test -p gotv-core --release
    ///  --lib -- --ignored --nocapture el_modelo_trabaja_en_un_proyecto_remoto`
    #[tokio::test]
    #[ignore]
    async fn el_modelo_trabaja_en_un_proyecto_remoto() {
        let (Ok(url), Ok(modelo)) = (std::env::var("GOTV_PRUEBA_OPENAI"), std::env::var("GOTV_PRUEBA_MODELO")) else { return };
        let Some(s) = pruebas::servidor("modelo") else { panic!("sin /usr/sbin/sshd") };
        let raiz = s.dir.join("app");
        std::fs::create_dir_all(&raiz).unwrap();
        let malo = "suma() { echo $(( $1 - $2 )); }\n";
        std::fs::write(raiz.join("calc.sh"), malo).unwrap();
        std::fs::write(raiz.join("prueba.sh"), ". ./calc.sh; [ \"$(suma 2 3)\" = 5 ] && echo PASA || { echo FALLA; exit 1; }\n").unwrap();
        let motor = gotv_engine::cloud::OpenAiEngine::new("gotv", &url, String::new(), &modelo, false);
        let mut c = crate::escritorio::prueba::agente_con("remoto-modelo", crate::permissions::Mode::AcceptEdits, motor);
        c.agente.max_steps = 20;
        c.agente.max_tokens = 4096;
        c.agente.context_budget = 65536;
        *c.agente.front.respuesta.lock().unwrap() = Some(crate::permissions::AskAnswer::Yes);
        pruebas::ancla_con_conexion(&s, &c.agente.root.clone(), &raiz);
        let t = Instant::now();
        let r = c.agente.run("La prueba prueba.sh de este proyecto falla. Arréglalo y comprueba que pasa.").await.unwrap();
        eprintln!("{modelo} · {} s · {}", t.elapsed().as_secs(), r.chars().take(300).collect::<String>());
        let ahora = std::fs::read_to_string(raiz.join("calc.sh")).unwrap();
        assert!(ahora.contains("$1 + $2"), "la edición está ALLÁ: {ahora}");
        let o = std::process::Command::new("sh").arg("prueba.sh").current_dir(&raiz).output().unwrap();
        assert!(String::from_utf8_lossy(&o.stdout).contains("PASA"), "la prueba pasa allá");
        let preguntas = c.agente.front.preguntas.lock().unwrap().clone();
        eprintln!("preguntas (bash allá pregunta también en acceptEdits): {}", preguntas.len());
        assert!(!preguntas.is_empty(), "correr allá preguntó");
        assert!(!c.agente.root.join("calc.sh").exists(), "nada del proyecto en el ancla de aquí");
        let d = deshacer_ultimo(&c.agente.root).await.unwrap();
        eprintln!("deshacer: {d}");
        assert_eq!(std::fs::read_to_string(raiz.join("calc.sh")).unwrap(), malo, "deshecho ALLÁ");
    }

    /// F16.3 contra un equipo DE VERDAD (`ssh gotv-nodo`, la Air por Thunderbolt). A mano y sólo con
    /// `GOTV_NODO=<alias de ~/.ssh/config>`: `cargo test --release -p gotv-core --lib nodo_real -- --ignored
    /// --nocapture`. Importa la conexión del `~/.ssh/config` (su llave), fija la huella en un ancla temporal de
    /// aquí, crea UNA carpeta propia en `/tmp` de allá (lo único que toca), corre el contrato (con tiempos), las
    /// herramientas, la terminal, deshacer y el servidor de lenguaje si allá hay uno; al final borra esa carpeta
    /// (ruta exacta) y comprueba que ya no existe.
    #[tokio::test]
    #[ignore]
    async fn nodo_real() {
        let Ok(alias) = std::env::var("GOTV_NODO") else { return };
        let ancla = crate::CarpetaPrueba::nueva("gotv-nodo-ancla");
        let c = crate::conexiones::importar_de_ssh_config(&alias, "nodo").unwrap();
        let (lineas, huellas) = crate::conexiones::escanear(&c).unwrap();
        eprintln!("huella de {alias}: {huellas:?}");
        crate::conexiones::fijar_huella(&ancla, &c, &lineas).unwrap();
        let mut reg = crate::conexiones::Registro::default();
        reg.agregar(c).unwrap();
        reg.guardar(&ancla).unwrap();
        // la carpeta de prueba, creada desde /tmp (la única que se toca allá)
        let nombre = format!("gotv-f16-prueba-{}", std::process::id());
        let tmp = Remoto::new(&ancla, "nodo", Path::new("/tmp"));
        let (rc, _, e) = tmp.correr(&format!("mkdir {}", q(&nombre)), 30).await.unwrap();
        assert_eq!(rc, 0, "{e}");
        let raiz = PathBuf::from(format!("/tmp/{nombre}"));
        guardar_ficha(&ancla, &Ficha { conexion: "nodo".into(), raiz: raiz.display().to_string() }).unwrap();
        let r = remoto_de(&ancla).unwrap();
        let resultado = async {
            let ms = contrato(&r).await;
            eprintln!("ms por operación contra {alias}: {ms:?}");
            let t = Instant::now();
            r.leer(Path::new("src/hola.txt"), 0, 100).await.unwrap();
            eprintln!("lectura con la sesión abierta: {} ms", t.elapsed().as_millis());
            assert!(herramienta(&r, "edit", &serde_json::json!({"path": "src/hola.txt", "old": "dos", "new": "DOS"})).await.unwrap().is_ok());
            assert!(herramienta(&r, "read_file", &serde_json::json!({"path": "src/hola.txt"})).await.unwrap().unwrap().contains("DOS con clave"));
            nuevo_turno(&ancla);
            herramienta(&r, "write_file", &serde_json::json!({"path": "src/hola.txt", "content": "pisado\n"})).await.unwrap().unwrap();
            deshacer_ultimo(&ancla).await.unwrap();
            assert!(String::from_utf8(r.leer(Path::new("src/hola.txt"), 0, 100).await.unwrap()).unwrap().contains("DOS con clave"), "deshecho allá");
            // terminal allá
            let id = crate::escritorio::terminal::abrir(&ancla, 100, 30, std::sync::Arc::new(|_, _| {})).unwrap();
            crate::escritorio::terminal::escribir(id, b"echo DONDE=$(pwd)\r").unwrap();
            let mut t = String::new();
            for _ in 0..300 {
                t = crate::escritorio::terminal::texto(id, 50).unwrap_or_default();
                if t.contains(&format!("DONDE={}", raiz.display())) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(30)).await;
            }
            crate::escritorio::terminal::cerrar(id);
            assert!(t.contains(&format!("DONDE={}", raiz.display())), "{t}");
            // el servidor de lenguaje que tenga la Air (si tiene)
            let b = crate::lsp::remoto::Puente::de(&ancla).unwrap();
            match b.buscar(&[vec!["rust-analyzer".into()], vec!["typescript-language-server".into(), "--stdio".into()], vec!["pyright-langserver".into(), "--stdio".into()]], "-").await {
                Ok((cmd, _)) => eprintln!("servidor de lenguaje allá: {cmd:?}"),
                Err(e) => eprintln!("sin servidor de lenguaje allá: {e}"),
            }
        };
        let fin = futures_util::FutureExt::catch_unwind(std::panic::AssertUnwindSafe(resultado)).await;
        // limpieza SIEMPRE, por ruta exacta
        let (rc, _, e) = tmp.correr(&format!("rm -rf -- {} && [ ! -e {} ]", q(&nombre), q(&nombre)), 30).await.unwrap();
        assert_eq!(rc, 0, "la carpeta de prueba se borró: {e}");
        if let Err(p) = fin {
            std::panic::resume_unwind(p);
        }
    }

    /// `nodo_real` sin la Air (el Portador no la tiene, 2026-11-17): ESTA Mac hace de «otro equipo» por un `sshd`
    /// desechable (llave y puerto propios, carpeta temporal; nada del sistema ni del `~/.ssh` del usuario). Mismo
    /// recorrido que `nodo_real` —contrato, herramientas, deshacer, terminal allá— y además un servidor de
    /// lenguaje REAL (rust-analyzer) corriendo allá sobre un proyecto Rust con un error de tipos de verdad.
    /// `cargo test -p gotv-core --release --lib -- --ignored --nocapture esta_mac_como_otro_equipo`
    #[tokio::test]
    #[ignore]
    async fn esta_mac_como_otro_equipo() {
        use crate::lsp::gestor::Gestor;
        let Some(s) = pruebas::servidor("esta-mac") else { panic!("sin /usr/sbin/sshd") };
        let ancla = crate::CarpetaPrueba::nueva("gotv-esta-mac-ancla");
        let raiz = s.dir.join("proyecto");
        std::fs::create_dir_all(raiz.join("src")).unwrap();
        pruebas::ancla_con_conexion(&s, &ancla, &raiz);
        let ancla = ancla.canonicalize().unwrap();
        let r = remoto_de(&ancla).unwrap();
        // 1 · contrato y tiempos
        let ms = contrato(&r).await;
        eprintln!("ms por operación: {ms:?}");
        // 2 · herramientas, deshacer
        assert!(herramienta(&r, "edit", &serde_json::json!({"path": "src/hola.txt", "old_string": "dos", "new_string": "DOS"})).await.unwrap().is_ok());
        assert!(herramienta(&r, "read_file", &serde_json::json!({"path": "src/hola.txt"})).await.unwrap().unwrap().contains("DOS con clave"));
        nuevo_turno(&ancla);
        herramienta(&r, "write_file", &serde_json::json!({"path": "src/hola.txt", "content": "pisado\n"})).await.unwrap().unwrap();
        deshacer_ultimo(&ancla).await.unwrap();
        assert!(std::fs::read_to_string(raiz.join("src/hola.txt")).unwrap().contains("DOS con clave"), "deshecho allá");
        // 3 · la terminal del usuario se abre ALLÁ, en la carpeta del proyecto
        let id = crate::escritorio::terminal::abrir(&ancla, 100, 30, std::sync::Arc::new(|_, _| {})).unwrap();
        crate::escritorio::terminal::escribir(id, b"echo DONDE=$(pwd)\r").unwrap();
        let mut t = String::new();
        for _ in 0..300 {
            t = crate::escritorio::terminal::texto(id, 50).unwrap_or_default();
            if t.contains(&format!("DONDE={}", raiz.display())) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(30)).await;
        }
        crate::escritorio::terminal::cerrar(id);
        assert!(t.contains(&format!("DONDE={}", raiz.display())), "{t}");
        // 4 · rust-analyzer DE VERDAD, allá: un proyecto sano y luego un error de tipos escrito por la herramienta
        herramienta(&r, "write_file", &serde_json::json!({"path": "Cargo.toml", "content": "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[workspace]\n"})).await.unwrap().unwrap();
        let sano = "fn sumar(a: i32, b: i32) -> i32 {\n    a + b\n}\n\nfn main() {\n    let x: i32 = sumar(1, 2);\n    println!(\"{}\", x);\n}\n";
        herramienta(&r, "write_file", &serde_json::json!({"path": "src/main.rs", "content": sano})).await.unwrap().unwrap();
        let mut g = Gestor::new(&ancla);
        let main = ancla.join("src/main.rs");
        let primera = g.diagnostics_full(&main, Duration::from_secs(90)).await;
        let mut g = match primera {
            Ok((d, _)) => {
                eprintln!("con el PATH de allá: rust-analyzer encontrado ({} avisos en el sano)", d.len());
                g
            }
            Err(e) => {
                // una sesión ssh no interactiva no lee el perfil: `~/.cargo/bin` puede no estar en su PATH. Se
                // dice (como lo diría en la Air) y se prueba el servidor real por su ruta.
                eprintln!("con el PATH de allá: {e}");
                let ra = std::process::Command::new("rustup").args(["which", "rust-analyzer"]).output().unwrap();
                let ra = String::from_utf8_lossy(&ra.stdout).trim().to_string();
                assert!(!ra.is_empty(), "sin rust-analyzer en esta Mac");
                let mut defs = crate::lsp::registro::defs();
                defs.iter_mut().filter(|d| d.id == "rust").for_each(|d| d.candidatos = vec![vec![ra.clone()]]);
                let mut g = Gestor::con(&ancla, defs);
                let (d, _) = g.diagnostics_full(&main, Duration::from_secs(90)).await.unwrap();
                assert!(d.iter().all(|x| x.severity != 1), "sano: {d:?}");
                g
            }
        };
        assert!(!main.exists(), "el ancla no tiene el archivo: vive allá");
        herramienta(&r, "edit", &serde_json::json!({"path": "src/main.rs", "old_string": "let x: i32", "new_string": "let x: String"})).await.unwrap().unwrap();
        let mut malo = Vec::new();
        for _ in 0..6 {
            malo = g.diagnostics_full(&main, Duration::from_secs(30)).await.unwrap().0;
            if malo.iter().any(|x| x.severity == 1) {
                break;
            }
        }
        eprintln!("rust-analyzer allá, tras el error: {malo:?}");
        assert!(malo.iter().any(|x| x.severity == 1 && x.line == 6 && x.message.contains("mismatched")), "{malo:?}");
        assert!(!ancla.join("src").exists(), "nada se escribió en el ancla");
    }

    #[test]
    fn normaliza_sin_tocar_el_disco_y_niega_lo_que_escapa() {
        let r = Path::new("/srv/Proyecto");
        assert_eq!(normalizar(r, Path::new("src/./a.rs")).unwrap(), Path::new("/srv/Proyecto/src/a.rs"));
        assert_eq!(normalizar(r, Path::new("src/../b.rs")).unwrap(), Path::new("/srv/Proyecto/b.rs"));
        assert_eq!(normalizar(r, Path::new("/srv/Proyecto/x")).unwrap(), Path::new("/srv/Proyecto/x"));
        assert!(normalizar(r, Path::new("../fuera")).is_err());
        assert!(normalizar(r, Path::new("a/../../fuera")).is_err());
        assert!(normalizar(r, Path::new("/etc/passwd")).is_err());
        // mayúsculas: la ruta remota se respeta tal cual (sin `canonicalize` local)
        assert!(normalizar(r, Path::new("/srv/proyecto/x")).is_err());
    }

    /// El contrato de `Lugar` (leer, escribir atómico, listar, metadatos, buscar, ejecutar), para cualquiera.
    async fn contrato(l: &dyn Lugar) -> Vec<(&'static str, u128)> {
        let mut ms = Vec::new();
        let mut t = Instant::now();
        let mut medir = |n: &'static str, t: &mut Instant| {
            ms.push((n, t.elapsed().as_millis()));
            *t = Instant::now();
        };
        l.escribir(Path::new("src/hola.txt"), b"uno\ndos con clave\ntres\n").await.unwrap();
        medir("escribir", &mut t);
        assert_eq!(l.leer(Path::new("src/hola.txt"), 0, 1000).await.unwrap(), b"uno\ndos con clave\ntres\n");
        medir("leer", &mut t);
        assert_eq!(l.leer(Path::new("src/hola.txt"), 4, 3).await.unwrap(), b"dos");
        let m = l.metadatos(Path::new("src/hola.txt")).await.unwrap();
        medir("metadatos", &mut t);
        assert_eq!((m.existe, m.dir, m.tam), (true, false, 23));
        assert!(!l.metadatos(Path::new("no.txt")).await.unwrap().existe);
        let v = l.listar(Path::new("src")).await.unwrap();
        medir("listar", &mut t);
        assert_eq!(v, vec![Entrada { nombre: "hola.txt".into(), dir: false, tam: 23 }]);
        let b = l.buscar("clave", None).await.unwrap();
        medir("buscar", &mut t);
        assert!(b.contains("src/hola.txt:2:dos con clave"), "{b}");
        let s = l.ejecutar("echo $((6*7)); exit 3", &Ejecucion::default()).await.unwrap();
        medir("ejecutar", &mut t);
        assert_eq!((s.codigo, s.texto.trim()), (3, "42"));
        assert!(l.leer(Path::new("../fuera"), 0, 10).await.is_err());
        assert!(l.escribir(Path::new("../../fuera.txt"), b"x").await.is_err());
        ms
    }

    #[tokio::test]
    async fn local_cumple_el_contrato() {
        let d = crate::CarpetaPrueba::nueva("gotv-lugar-local");
        let l = Local::new(&d.canonicalize().unwrap());
        contrato(&l).await;
    }

    /// Ficha F16.3: `Remoto` contra un servidor SSH desechable (leer, escribir atómico, listar, buscar,
    /// ejecutar), medido el tiempo por operación; las herramientas del agente corren allá; un corte a mitad
    /// de una escritura no deja el archivo a medias; un turno se deshace.
    #[tokio::test]
    async fn remoto_contra_un_sshd_desechable() {
        let Some(s) = pruebas::servidor("lugar") else {
            eprintln!("sin /usr/sbin/sshd: prueba saltada");
            return;
        };
        let ancla = crate::CarpetaPrueba::nueva("gotv-lugar-ancla");
        let raiz = s.dir.join("proyecto-allá");
        std::fs::create_dir_all(&raiz).unwrap();
        pruebas::ancla_con_conexion(&s, &ancla, &raiz);
        let r = remoto_de(&ancla).expect("el ancla es de un proyecto remoto");
        let ms = contrato(&r).await;
        eprintln!("ms por operación (sesión multiplexada): {ms:?}");
        // con la sesión ya abierta, una lectura no pasa de 300 ms (trampa 1)
        let t = Instant::now();
        r.leer(Path::new("src/hola.txt"), 0, 100).await.unwrap();
        let lectura = t.elapsed().as_millis();
        eprintln!("lectura con la sesión abierta: {lectura} ms");
        assert!(lectura < 300, "una lectura tardó {lectura} ms");
        assert_eq!(std::fs::read_to_string(raiz.join("src/hola.txt")).unwrap(), "uno\ndos con clave\ntres\n", "lo escrito está ALLÁ");

        // las herramientas del agente, por el enrutado
        let h = |n: &'static str, a: Value| {
            let r = &r;
            async move { herramienta(r, n, &a).await.unwrap() }
        };
        assert!(h("read_file", serde_json::json!({"path": "src/hola.txt"})).await.unwrap().contains("     2\tdos con clave"));
        h("edit", serde_json::json!({"path": "src/hola.txt", "old_string": "dos", "new_string": "DOS"})).await.unwrap();
        assert!(h("grep", serde_json::json!({"pattern": "DOS"})).await.unwrap().contains("src/hola.txt:2"));
        assert!(h("glob", serde_json::json!({"pattern": "**/*.txt"})).await.unwrap().contains("src/hola.txt"));
        assert!(h("bash", serde_json::json!({"cmd": "pwd"})).await.unwrap().contains("proyecto-allá"));
        assert!(h("mapa", serde_json::json!({})).await.unwrap_err().to_string().contains("REMOT"));
        assert!(herramienta(&r, "ask_user", &serde_json::json!({})).await.is_none());

        // deshacer un turno: lo escrito en él vuelve a como estaba (y lo creado desaparece)
        nuevo_turno(&ancla);
        h("write_file", serde_json::json!({"path": "src/hola.txt", "content": "pisado\n"})).await.unwrap();
        h("write_file", serde_json::json!({"path": "nuevo.txt", "content": "nuevo\n"})).await.unwrap();
        let d = deshacer_ultimo(&ancla).await.unwrap();
        assert!(d.contains('2'), "{d}");
        assert_eq!(std::fs::read_to_string(raiz.join("src/hola.txt")).unwrap(), "uno\nDOS con clave\ntres\n");
        assert!(!raiz.join("nuevo.txt").exists());

        // cortar la conexión a mitad de una escritura: el archivo de verdad queda entero
        let grande = vec![b'x'; 8 * 1024 * 1024];
        let escritura = r.escribir(Path::new("src/hola.txt"), &grande);
        let corte = tokio::time::timeout(Duration::from_millis(40), escritura).await;
        assert!(corte.is_err(), "se cortó a mitad");
        tokio::time::sleep(Duration::from_millis(500)).await;
        assert_eq!(std::fs::read_to_string(raiz.join("src/hola.txt")).unwrap(), "uno\nDOS con clave\ntres\n", "sin archivo a medias");
        // y de forma determinista: la misma orden recibe MENOS bytes de los anunciados (lo que deja un corte)
        let (rc, _, e) = r.ssh(&orden_escribir("src/hola.txt", 100), Some(&[b'x'; 50]), 30).await.unwrap();
        assert_eq!(rc, 5, "{}", String::from_utf8_lossy(&e));
        assert_eq!(std::fs::read_to_string(raiz.join("src/hola.txt")).unwrap(), "uno\nDOS con clave\ntres\n", "un envío truncado no toca el archivo");
        assert!(!std::fs::read_dir(raiz.join("src")).unwrap().flatten().any(|e| e.file_name().to_string_lossy().contains("gotv-tmp")), "ni deja el temporal");

        // permisos: bash que escribe pregunta incluso en acceptEdits; leer no; bypass no pregunta
        use crate::permissions::{Decision, Mode};
        assert!(matches!(decision(&ancla, "bash", &serde_json::json!({"cmd": "rm -rf build"}), Mode::AcceptEdits), Some(Decision::Ask(_))));
        assert!(decision(&ancla, "bash", &serde_json::json!({"cmd": "ls -la"}), Mode::AcceptEdits).is_none());
        assert!(decision(&ancla, "bash", &serde_json::json!({"cmd": "rm -rf build"}), Mode::Bypass).is_none());
        assert!(decision(&raiz, "bash", &serde_json::json!({"cmd": "rm -rf build"}), Mode::AcceptEdits).is_none(), "un proyecto local no cambia");
    }
}
