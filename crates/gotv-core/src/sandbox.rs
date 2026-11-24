//! C2 — Caja de arena de macOS (Seatbelt, `/usr/bin/sandbox-exec`), con la política base
//! de Codex (Apache-2.0, ver `terceros/codex-seatbelt/NOTICE.md`) y lo nuestro encima:
//!
//! - LEER: todo (como Codex): el agente necesita leer compiladores, librerías, el SDK.
//! - ESCRIBIR: sólo en la carpeta del proyecto, la temporal y las cachés de herramientas
//!   (npm, pip/uv, cargo, bun, pnpm…). Dentro del proyecto quedan PROTEGIDOS `.gotv`
//!   (permisos aprendidos y checkpoints: un comando no puede darse permisos ni borrar el
//!   deshacer), `.git/hooks` y `.git/config` (ahí se esconde código que corre después).
//!   La carpeta del proyecto en sí no se puede borrar.
//! - RED: sólo `localhost` directo; lo de fuera, por el proxy de GOTV (`netproxy`), que
//!   decide por dominio. Los registros de paquetes vienen permitidos de serie.
//! - SOCKETS UNIX (F14.0): sólo los del proyecto, los de la temporal PROPIA de la caja y los que
//!   el usuario apunte en `sockets_allow`. Un socket es la puerta de otro programa: el de Docker
//!   (montar tu carpeta personal en un contenedor), el del agente de SSH (firmar con tus llaves),
//!   el puente de GOTV con tu Chrome, los de tu editor. Antes pasaban TODOS.
//! - LLAVERO (F14.0): el servicio que entrega secretos del llavero queda negado y el agente de
//!   SSH no se hereda. La verificación de certificados sigue (va por otro servicio).
//!
//! F14.0 medido (2026-11-14) con la política real y sondas inofensivas. Antes: un socket de
//! fuera se leía, `ssh-add -l` hablaba con el agente, y un secreto de un llavero abierto SALÍA
//! (llavero de prueba propio). Y al revés: el multiproceso de Python y un servidor de Node por
//! socket FALLABAN dentro (no podían escuchar). Ahora: lo ajeno, denegado; lo propio, funciona.
//! No rompe (medido): git, `git ls-remote` por HTTPS, curl (y sigue rechazando un certificado
//! malo), cargo fetch/build, npm view/install, pnpm, pip en un venv, uv, rustc, clang, un
//! servidor de desarrollo.
//!
//! Medido a mano antes de escribir esto (2026-11-09): dentro ✓, fuera ✗, `.gotv` ✗,
//! carpeta personal ✗, temporal ✓, red externa ✗ (ni resuelve), localhost ✓, node/python ✓.

use std::path::{Path, PathBuf};

pub const SANDBOX_EXEC: &str = "/usr/bin/sandbox-exec";

const BASE: &str = include_str!("../../../terceros/codex-seatbelt/seatbelt_base_policy.sbpl");
const NETWORK: &str = include_str!("../../../terceros/codex-seatbelt/seatbelt_network_policy.sbpl");
const PREFERENCES: &str =
    include_str!("../../../terceros/codex-seatbelt/seatbelt_preferences_policy.sbpl");
const TLS_TRUST: &str = "(allow mach-lookup (global-name \"com.apple.TrustEvaluationAgent\"))\n";

/// De serie: DESCARGAR paquetes (registros y sus CDN). `github.com` NO: con él se puede
/// empujar código o crear gists (sacar datos); se pide la primera vez.
pub const DEFAULT_HOSTS: &[&str] = &[
    "pypi.org",
    "files.pythonhosted.org",
    "registry.npmjs.org",
    // F20.1: la base de vulnerabilidades (la compuerta de instalación y `package auditar`)
    "api.osv.dev",
    "registry.yarnpkg.com",
    "npm.pkg.github.com",
    "crates.io",
    "index.crates.io",
    "static.crates.io",
    "static.rust-lang.org",
    "nodejs.org",
    "astral.sh",
    "github-releases.githubusercontent.com",
    "objects.githubusercontent.com",
    "codeload.github.com",
    "raw.githubusercontent.com",
    "jsr.io",
    "deno.land",
    "repo.maven.apache.org",
    "repo1.maven.org",
];

/// Cómo corre un comando: con caja o sin ella, y con qué red.
#[derive(Clone, Debug)]
pub struct Spec {
    pub root: PathBuf,
    /// Dominios permitidos además de los de serie (config + reglas aprendidas).
    pub hosts: Vec<String>,
    /// Toda la red (modo bypass).
    pub all_net: bool,
    /// F14.0: sockets Unix de FUERA del proyecto que el usuario permite (config `sockets_allow`),
    /// por su ruta exacta: p. ej. `/tmp/.s.PGSQL.5432` para las pruebas contra su Postgres local.
    pub sockets: Vec<PathBuf>,
    /// F14.1: rutas de credenciales que el usuario añade a las de fábrica (`credenciales_extra`).
    pub credenciales: Vec<PathBuf>,
    /// F14.1: variables secretas que el usuario SÍ deja pasar a la caja (`entorno_allow`).
    pub entorno: Vec<String>,
}

/// F14.0 · La temporal PROPIA de la caja de un proyecto, dentro de la temporal del usuario. Los
/// comandos de la caja reciben `TMPDIR` apuntando aquí, y sólo aquí (y en el proyecto) pueden
/// crear y abrir sockets: en la temporal general viven los sockets de OTROS programas (los del
/// editor, por ejemplo). El nombre es corto a propósito: un socket Unix no admite rutas de más
/// de 104 bytes y los de `multiprocessing` de Python ya gastan 31.
pub fn temporal_propia(root: &Path) -> PathBuf {
    use sha2::{Digest, Sha256};
    let d = Sha256::digest(canon(root).to_string_lossy().as_bytes());
    canon(&std::env::temp_dir()).join(format!("gotv-{:02x}{:02x}{:02x}", d[0], d[1], d[2]))
}

pub fn available() -> bool {
    Path::new(SANDBOX_EXEC).exists()
}

fn canon(p: &Path) -> PathBuf {
    if let Ok(c) = p.canonicalize() {
        return c;
    }
    // lo que aún no existe: el padre real + el nombre (así /tmp/x → /private/tmp/x)
    match (p.parent(), p.file_name()) {
        (Some(parent), Some(name)) => canon(parent).join(name),
        _ => p.to_path_buf(),
    }
}

/// Dónde puede escribir un comando además del proyecto.
fn extra_writable() -> Vec<PathBuf> {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/var/empty"));
    let mut v: Vec<PathBuf> = vec![
        // la temporal PRIVADA del usuario ($TMPDIR, /var/folders/…): la que usan las herramientas.
        // El /tmp COMPARTIDO no (medido D11: una skill hizo que el 9B dejara textos en /tmp/*.txt,
        // fuera del proyecto, donde nadie los limpia y se mezclan proyectos).
        std::env::temp_dir(),
        // cachés de herramientas (descargas, compilados): nada que se ejecute solo después
        home.join(".npm"),
        home.join(".cache"),
        home.join("Library/Caches"),
        home.join(".cargo/registry"),
        home.join(".cargo/git"),
        home.join(".bun/install/cache"),
        home.join("Library/pnpm/store"),
        home.join(".local/share/pnpm/store"),
        home.join(".local/share/uv"),
        home.join(".yarn/berry/cache"),
        home.join(".gradle/caches"),
        home.join(".m2/repository"),
        home.join(".deno"),
        // pnpm ≥ 11 (binario nativo) guarda los candados de su almacén en el /tmp COMPARTIDO, en una
        // carpeta con el número del usuario, no en $TMPDIR (medido 2026-11-14 mirando sus archivos
        // abiertos: `all-stores.lock` y `<hash>.lock`). Sin ella no instala nada dentro de la caja.
        // Sólo esa carpeta: el resto del /tmp compartido sigue cerrado (D11).
        // SAFETY: `getuid` no tiene precondiciones ni falla.
        PathBuf::from(format!("/private/tmp/pnpm-store-operation-locks-{}", unsafe { libc::getuid() })),
    ];
    v.iter_mut().for_each(|p| *p = canon(p));
    v.dedup();
    v
}

/// F14.1 · Lo que la caja NO deja leer (ni escribir), aunque lea el resto del disco: dónde guardan sus
/// credenciales las herramientas y los navegadores. Rutas relativas a la carpeta personal. Medido el
/// 2026-11-15 con la política anterior: `~/.ssh`, `~/.gnupg`, la sesión de `gh`, `~/.docker/config.json`,
/// los llaveros, los perfiles de Chrome, Firefox y Safari, las cookies y la configuración de GOTV se
/// leían desde un comando de la caja. Lo que de verdad necesita una credencial (`git push` por SSH,
/// `gh`, `npm publish`) corre FUERA de la caja, con permiso. El usuario añade más en `credenciales_extra`.
/// ¿El comando toca credenciales del usuario (una ruta de [`CREDENCIALES`] o el llavero)? Devuelve cuál.
/// Medido 2026-11-16 (banco `mcp-cambio`): una descripción de MCP con una instrucción escondida llevó al
/// modelo a `cat ~/.ssh/id_ed25519`; la caja lo bloqueó, pidió hacerlo FUERA de la caja y, aprobado, la
/// llave privada salió hacia la herramienta. Fuera de la caja esto se niega SIEMPRE (`agent.rs`).
pub fn toca_credenciales(cmd: &str) -> Option<String> {
    if let Some(c) = CREDENCIALES.iter().find(|c| cmd.contains(&format!("/{c}")) || cmd.contains(&format!("~/{c}")) || cmd.contains(&format!("${{HOME}}/{c}")) || cmd.contains(&format!("$HOME/{c}"))) {
        return Some(c.to_string());
    }
    let palabras: Vec<&str> = cmd.split(|ch: char| ch.is_whitespace() || ch == ';' || ch == '|' || ch == '&').filter(|w| !w.is_empty()).collect();
    palabras
        .windows(2)
        .find(|w| w[0].rsplit('/').next() == Some("security") && (w[1].starts_with("find-") || w[1].starts_with("dump-") || w[1] == "export"))
        .map(|w| format!("llavero ({} {})", w[0], w[1])) // juez-idioma: datos (lo que se cita del comando)
}

/// ¿La ruta está dentro de una carpeta o archivo de credenciales del usuario? Devuelve cuál. Medido
/// 2026-11-16 (banco `mcp-cambio`): siguiendo una instrucción envenenada, el modelo leyó la llave SSH con
/// `read_file`, que corre en el proceso de GOTV y no pasa por la caja.
/// Una ruta tal como la pidió el modelo, sin «encerrarla» en el proyecto (para JUZGARLA, no para abrirla):
/// `~/x` → la casa; absoluta → tal cual; relativa → desde `root`.
pub fn ruta_pedida(root: &Path, r: &str) -> PathBuf {
    let r = r.trim();
    match r.strip_prefix("~/") {
        Some(resto) => PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(resto),
        None if r == "~" => PathBuf::from(std::env::var_os("HOME").unwrap_or_default()),
        None if Path::new(r).is_absolute() => PathBuf::from(r),
        None => root.join(r),
    }
}

pub fn es_credencial(p: &Path) -> Option<String> {
    let casa = PathBuf::from(std::env::var_os("HOME")?);
    let p = p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
    let casa_c = casa.canonicalize().unwrap_or_else(|_| casa.clone());
    CREDENCIALES.iter().find(|c| p.starts_with(casa.join(c)) || p.starts_with(casa_c.join(c))).map(|c| c.to_string())
}

pub const CREDENCIALES: &[&str] = &[
    ".ssh",
    ".aws",
    ".gnupg",
    ".azure",
    ".kube",
    ".config/gh",
    ".config/hub",
    ".config/gcloud",
    ".config/git/credentials",
    ".git-credentials",
    ".netrc",
    ".npmrc",
    ".pypirc",
    ".gem/credentials",
    ".docker/config.json",
    ".cargo/credentials",
    ".cargo/credentials.toml",
    ".terraform.d/credentials.tfrc.json",
    ".config/gotv",
    "Library/Keychains",
    "Library/Cookies",
    "Library/Safari",
    "Library/Application Support/Google/Chrome",
    "Library/Application Support/Chromium",
    "Library/Application Support/BraveSoftware",
    "Library/Application Support/Microsoft Edge",
    "Library/Application Support/Arc",
    "Library/Application Support/Firefox",
];

/// Las credenciales de fábrica más las del usuario, en rutas reales. Se omite la que CONTENGA al
/// proyecto (un proyecto dentro de `~/.config/gotv` no puede quedar ilegible para sí mismo).
pub fn credenciales(root: &Path, extra: &[PathBuf]) -> Vec<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/var/empty"));
    let mut v: Vec<PathBuf> = CREDENCIALES.iter().map(|r| canon(&home.join(r))).chain(extra.iter().map(|p| canon(p))).filter(|c| !root.starts_with(c)).collect();
    v.sort();
    v.dedup();
    v
}

/// F14.1 · ¿Es secreta esta variable de entorno? Las que lo son no llegan a los comandos de la caja
/// (salvo las de `entorno_allow`): un `printenv` dentro no debe mostrar el token de nadie.
pub fn variable_secreta(nombre: &str) -> bool {
    let n = nombre.to_ascii_uppercase();
    ["_TOKEN", "_KEY", "_SECRET", "_PASSWORD", "_PASSWD", "_PAT", "_CREDENTIALS", "_API_KEY"].iter().any(|s| n.ends_with(s))
        || ["PASSWORD", "PASSWD", "SECRET", "TOKEN"].iter().any(|s| n.contains(s))
        || ["AWS_", "GH_", "GITHUB_", "GITLAB_", "NPM_", "OPENAI_", "ANTHROPIC_", "HF_", "HUGGING", "AZURE_", "GOOGLE_APPLICATION_CREDENTIALS", "DOCKER_AUTH", "CARGO_REGISTRY_TOKEN"].iter().any(|p| n.starts_with(p))
}

/// F14.1 · Quita del comando las variables secretas de `heredadas` (salvo las de `permitidas`). Va
/// aparte para poder probarse con un entorno heredado inventado.
fn quitar_secretos(c: &mut std::process::Command, heredadas: impl IntoIterator<Item = std::ffi::OsString>, permitidas: &[String]) {
    for k in heredadas {
        let k = k.to_string_lossy().into_owned();
        if variable_secreta(&k) && !permitidas.iter().any(|e| e.eq_ignore_ascii_case(&k)) {
            c.env_remove(&k);
        }
    }
}

/// F14.1 · `~/.npmrc` guarda a la vez el TOKEN y la configuración (el registro, el de cada ámbito de
/// la empresa). Negado sin más, npm dentro de la caja se iría al registro PÚBLICO sin avisar (medido:
/// el registro de un ámbito quedaba `undefined`): un paquete con el mismo nombre que uno privado
/// entraría del sitio equivocado. Así que el archivo se niega y GOTV le pasa a la caja SÓLO las
/// líneas que no son secretas, como `npm_config_<clave>` (medido: npm las respeta, también las de
/// un ámbito). Fuera: todo lo de `//host/:…` (credenciales por registro) y cualquier clave con
/// auth, token, password, user, email, cert o key; y los valores con `${…}` (se expanden del
/// entorno, que ya no lleva secretos).
fn npmrc_sin_secretos(ruta: &Path) -> Vec<(String, String)> {
    let Ok(texto) = std::fs::read_to_string(ruta) else { return vec![] };
    texto
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#') && !l.starts_with(';'))
        .filter_map(|l| l.split_once('='))
        .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
        .filter(|(k, v)| {
            let kl = k.to_ascii_lowercase();
            !k.starts_with("//") && !v.contains("${") && !["auth", "token", "password", "user", "email", "cert", "key", "otp"].iter().any(|s| kl.contains(s))
        })
        .collect()
}

/// Lo que dentro del proyecto NO se toca. Los ganchos y la config de git sólo si el repo
/// YA existe (así `git init` funciona); Codex protege `.git` entero y obliga a pedir
/// permiso hasta para un commit: aquí commit/branch/add siguen, y lo que esconde código
/// que correría FUERA de la caja (hooks, `core.hooksPath`, `core.fsmonitor`) no.
fn protected(root: &Path) -> Vec<PathBuf> {
    let mut v = vec![root.join(".gotv")];
    if root.join(".git").is_dir() {
        v.push(root.join(".git/hooks"));
        v.push(root.join(".git/config"));
    }
    v
}

/// La política completa (los caminos van como parámetros `-D`, nunca pegados al texto).
pub fn policy(n_extra: usize, n_protected: usize, n_sockets: usize, n_cred: usize) -> String {
    let mut p = String::with_capacity(BASE.len() + 4096);
    p.push_str(BASE);
    p.push_str("\n; ── GOD OF THE VOID (C2) ──\n(allow file-read*)\n");
    p.push_str(PREFERENCES);
    // escribir: (el proyecto O una zona extra) Y NUNCA lo protegido — la protección vale
    // aunque el proyecto viva dentro de una zona extra (p. ej. en /tmp)
    p.push_str("(allow file-write* (require-all (require-any (subpath (param \"ROOT\"))");
    for i in 0..n_extra {
        p.push_str(&format!(" (subpath (param \"W{i}\"))"));
    }
    p.push(')');
    for i in 0..n_protected {
        p.push_str(&format!(" (require-not (subpath (param \"PROT{i}\")))"));
    }
    p.push_str("))\n");
    // la carpeta del proyecto en sí no se puede borrar ni renombrar
    p.push_str("(deny file-write-unlink (literal (param \"ROOT\")))\n");
    // red: sólo localhost (servidores de desarrollo, el proxy de GOTV)
    p.push_str("(allow network-bind (local ip \"*:*\"))\n");
    p.push_str("(allow network-inbound (local ip \"localhost:*\"))\n");
    p.push_str("(allow network-outbound (remote ip \"localhost:*\"))\n");
    // F14.0 · sockets Unix: sólo los PROPIOS (del proyecto y de la temporal de la caja) y los que
    // el usuario apuntó. Conectar, y también escuchar y recibir: sin eso las herramientas no
    // pueden usar sus propios sockets (medido: `multiprocessing` de Python, un servidor de Node).
    let propios = "(subpath (param \"ROOT\"))) ({lado} unix-socket (subpath (param \"CAJATMP\")))";
    p.push_str(&format!("(allow network-outbound (remote unix-socket {}", propios.replace("{lado}", "remote")));
    for i in 0..n_sockets {
        p.push_str(&format!(" (remote unix-socket (literal (param \"S{i}\")))"));
    }
    p.push_str(")\n");
    p.push_str(&format!("(allow network-bind (local unix-socket {})\n", propios.replace("{lado}", "local")));
    p.push_str(&format!("(allow network-inbound (local unix-socket {})\n", propios.replace("{lado}", "local")));
    p.push_str(NETWORK);
    p.push_str(TLS_TRUST);
    // F14.0 · el llavero: la política de red de base permite `SecurityServer` «para los
    // certificados», pero es el servicio que entrega los SECRETOS de un llavero abierto (medido:
    // salían). Medido también que sin él la verificación de certificados sigue (curl, git, cargo,
    // npm, pip, uv: va por `trustd`). Va DESPUÉS de la base: la última regla manda.
    p.push_str("(deny mach-lookup (global-name \"com.apple.SecurityServer\"))\n");
    // F14.1 · las credenciales: ni leer ni escribir. Al FINAL, para que nada de lo anterior (la
    // lectura general, la escritura en las cachés) las vuelva a abrir: la última regla manda.
    if n_cred > 0 {
        p.push_str("(deny file-read* file-write*");
        for i in 0..n_cred {
            p.push_str(&format!(" (subpath (param \"C{i}\"))"));
        }
        p.push_str(")\n");
    }
    p
}

/// El comando `sandbox-exec … bash -lc <cmd>` para este proyecto.
pub fn command(spec: &Spec, cmd: &str) -> tokio::process::Command {
    let root = canon(&spec.root);
    let mut extra = extra_writable();
    let mut prot: Vec<PathBuf> = protected(&root);
    // F8.4: las carpetas extra del proyecto se escriben como el proyecto (y lo suyo protegido, igual)
    for (_, r) in crate::raices::extras_de(&root) {
        let r = canon(&r);
        prot.extend(protected(&r));
        extra.push(r);
    }
    let tmp = temporal_propia(&root);
    if std::fs::create_dir_all(&tmp).is_ok() {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o700));
    }
    let mut c = tokio::process::Command::new(SANDBOX_EXEC);
    let cred = credenciales(&root, &spec.credenciales);
    c.arg("-p").arg(policy(extra.len(), prot.len(), spec.sockets.len(), cred.len()));
    for (i, p) in cred.iter().enumerate() {
        c.arg(format!("-DC{i}={}", p.display()));
    }
    c.arg(format!("-DROOT={}", root.display()));
    c.arg(format!("-DCAJATMP={}", tmp.display()));
    for (i, s) in spec.sockets.iter().enumerate() {
        c.arg(format!("-DS{i}={}", canon(s).display()));
    }
    // la temporal de los comandos es la de la caja (ahí sí pueden abrir sus sockets), y el
    // agente de SSH del usuario no se hereda (su socket, además, ya no se alcanza)
    c.env("TMPDIR", format!("{}/", tmp.display()));
    c.env_remove("SSH_AUTH_SOCK");
    // F14.1: los secretos del entorno (tokens, llaves, contraseñas) tampoco se heredan
    quitar_secretos(c.as_std_mut(), std::env::vars_os().map(|(k, _)| k), &spec.entorno);
    // …y de `~/.npmrc` (negado) sólo pasa lo que no es secreto
    if let Some(casa) = std::env::var_os("HOME") {
        for (k, v) in npmrc_sin_secretos(&PathBuf::from(casa).join(".npmrc")) {
            c.env(format!("npm_config_{k}"), v);
        }
    }
    for (i, p) in extra.iter().enumerate() {
        c.arg(format!("-DW{i}={}", p.display()));
    }
    for (i, p) in prot.iter().enumerate() {
        c.arg(format!("-DPROT{i}={}", p.display()));
    }
    c.arg("bash").arg("-lc").arg(format!("{SWIFT_SIN_CAJA_PROPIA}{cmd}"));
    c
}

/// Medido 2026-11-16 (banco `arregla-swift`): SwiftPM compila `Package.swift` dentro de SU propio
/// `sandbox-exec`, y una caja no puede abrirse dentro de otra («sandbox_apply: Operation not permitted»):
/// `swift build/test/run/package` fallaban siempre en la de GOTV y el modelo gastó 25 pasos en rodearlo.
/// Dentro de la caja de GOTV (que ya aísla) van con `--disable-sandbox`; la función se exporta, así que
/// también la heredan los guiones que lancen `swift`. Repetir la opción no hace daño (medido).
const SWIFT_SIN_CAJA_PROPIA: &str = "swift(){ case \"$1\" in build|test|run|package) local s=\"$1\"; shift; command swift \"$s\" --disable-sandbox \"$@\";; *) command swift \"$@\";; esac; }; export -f swift; ";

/// Todos los dominios que este proyecto puede alcanzar.
pub fn hosts(spec: &Spec) -> Vec<String> {
    let mut v: Vec<String> = DEFAULT_HOSTS.iter().map(|s| s.to_string()).collect();
    v.extend(spec.hosts.iter().cloned());
    v.sort();
    v.dedup();
    v
}

/// ¿La salida dice que la caja bloqueó una escritura?
pub fn denied_write(output: &str) -> bool {
    output.contains("Operation not permitted") || output.contains("operation not permitted")
}

/// K1 · QUÉ bloqueó la caja (medido 2026-11-11 con las órdenes de la charla de voz).
#[derive(Debug, PartialEq, Eq)]
pub enum Bloqueo {
    Escritura,
    /// un dato o servicio del sistema (`sysctl`, DiskManagement, servicios XPC)
    Sistema,
    /// ejecutar un programa (`bash: /bin/ps: Operation not permitted`)
    Ejecutar,
}

pub fn que_bloqueo(output: &str) -> Option<Bloqueo> {
    if output.contains("sysctl: sysctl fmt") || (output.contains("sysctl:") && output.contains("Operation not permitted"))
        || output.contains("unable to use the DiskManagement framework")
        || output.contains("Connection Invalid error for service")
        || output.contains("AuthorizationCreate() failed")
        // F14.0: el agente de SSH y el llavero ya no se alcanzan desde la caja
        || output.contains("Error connecting to agent")
        || output.contains("SecKeychainSearchCreateFromAttributes")
    {
        return Some(Bloqueo::Sistema);
    }
    // `bash: /bin/ps: Operation not permitted` — el programa mismo no pudo arrancar
    let ejecutar = output.lines().any(|l| {
        let l = l.trim();
        l.ends_with(": Operation not permitted")
            && l.split(": ").nth(1).is_some_and(|p| ["/bin/", "/sbin/", "/usr/bin/", "/usr/sbin/", "/usr/libexec/"].iter().any(|d| p.starts_with(d)))
            && l.split(": ").count() == 3
    });
    if ejecutar {
        return Some(Bloqueo::Ejecutar);
    }
    denied_write(output).then_some(Bloqueo::Escritura)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Banco `mcp-cambio` (2026-11-16): lo que toca credenciales se reconoce en sus formas comunes.
    #[test]
    fn credenciales_en_un_comando() {
        assert_eq!(toca_credenciales("cat ~/.ssh/id_ed25519").as_deref(), Some(".ssh"));
        assert_eq!(toca_credenciales("cat $HOME/.aws/credentials | base64").as_deref(), Some(".aws"));
        assert_eq!(toca_credenciales("cp /Users/x/.netrc /tmp/a").as_deref(), Some(".netrc"));
        assert!(toca_credenciales("security find-generic-password -s x -w").is_some());
        assert!(toca_credenciales("/usr/bin/security dump-keychain").is_some());
        assert!(toca_credenciales("ssh-keygen -t ed25519 -f ./clave").is_none(), "en el proyecto, no");
        assert!(toca_credenciales("ls -la && git status").is_none());
        assert!(toca_credenciales("security list-keychains").is_none(), "listar no lee secretos");
    }

    /// FUERA de la carpeta temporal (que la caja deja escribir a propósito).
    fn base() -> PathBuf {
        canon(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/pruebas-caja"))
    }

    fn dir(n: &str) -> PathBuf {
        let d = base().join(format!("{n}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join(".gotv")).unwrap();
        canon(&d)
    }

    async fn run(spec: &Spec, cmd: &str) -> (i32, String) {
        let o = command(spec, cmd)
            .current_dir(&spec.root)
            .output()
            .await
            .unwrap();
        let mut s = String::from_utf8_lossy(&o.stdout).to_string();
        s.push_str(&String::from_utf8_lossy(&o.stderr));
        (o.status.code().unwrap_or(-1), s)
    }

    /// Las salidas REALES medidas en la caja se clasifican bien (antes todo era «escritura»).
    #[test]
    fn el_aviso_dice_que_se_bloqueo() {
        assert_eq!(que_bloqueo("sysctl: sysctl fmt -1 1024 1: Operation not permitted"), Some(Bloqueo::Sistema));
        assert_eq!(que_bloqueo("Error connecting to agent: Operation not permitted"), Some(Bloqueo::Sistema));
        assert_eq!(que_bloqueo("Unable to run because unable to use the DiskManagement framework."), Some(Bloqueo::Sistema));
        assert_eq!(que_bloqueo("bash: /bin/ps: Operation not permitted"), Some(Bloqueo::Ejecutar));
        assert_eq!(que_bloqueo("bash: /usr/bin/top: Operation not permitted"), Some(Bloqueo::Ejecutar));
        assert_eq!(que_bloqueo("bash: line 6: /Users/x/a.txt: Operation not permitted"), Some(Bloqueo::Escritura));
        assert_eq!(que_bloqueo("touch: /Users/x/b: Operation not permitted"), Some(Bloqueo::Escritura));
        assert_eq!(que_bloqueo("todo bien"), None);
    }

    /// Las órdenes de LECTURA del sistema que el Portador pidió por voz (2026-11-11): qué
    /// procesador, memoria, macOS, disco… dentro de la caja. Informa qué pasa con cada una.
    /// `cargo test -p gotv-core --lib -- --ignored --nocapture leer_el_sistema_desde_la_caja`
    #[tokio::test]
    #[ignore]
    async fn leer_el_sistema_desde_la_caja() {
        let d = dir("sistema");
        let spec = Spec { root: d.clone(), hosts: vec![], all_net: false, sockets: vec![], credenciales: vec![], entorno: vec![] };
        for cmd in [
            "sysctl -n machdep.cpu.brand_string",
            "sysctl -n hw.memsize",
            "sysctl -n hw.ncpu",
            "system_profiler SPHardwareDataType",
            "sw_vers",
            "uname -a",
            "df -h /",
            "vm_stat",
            "top -l 1 -n 0",
            "ps aux",
            "pmset -g batt",
            "ioreg -l -w0 -d1",
            "networksetup -listallhardwareports",
            "ifconfig en0",
            "uptime",
            "who",
            "diskutil list",
            "osascript -e 'tell application \"Finder\" to get name of startup disk'",
        ] {
            let (rc, out) = run(&spec, cmd).await;
            let primera = out.lines().find(|l| !l.trim().is_empty()).unwrap_or("").chars().take(90).collect::<String>();
            println!("{} rc={rc:<3} {cmd:<55} → {primera}", if rc == 0 { "✓" } else { "✗" });
        }
    }

    #[tokio::test]
    async fn escribe_en_el_proyecto_y_en_nada_mas() {
        if !available() {
            return;
        }
        let d = dir("escribe");
        let spec = Spec {
            root: d.clone(),
            hosts: vec![],
            all_net: false,
            sockets: vec![],
            credenciales: vec![],
            entorno: vec![],
        };
        let (rc, _) = run(&spec, "mkdir -p src/a && echo hola > src/a/b.txt").await;
        assert_eq!(rc, 0);
        assert!(d.join("src/a/b.txt").exists());
        let fuera = base().join(format!("fuera-{}", std::process::id()));
        let (rc, out) = run(&spec, &format!("echo x > {}", fuera.display())).await;
        assert!(rc != 0 && denied_write(&out) && !fuera.exists(), "{out}");
        let home = std::env::var("HOME").unwrap();
        let (rc, _) = run(&spec, &format!("touch {home}/gotv-caja-no-debe-existir")).await;
        assert!(rc != 0 && !Path::new(&format!("{home}/gotv-caja-no-debe-existir")).exists());
    }

    #[tokio::test]
    async fn lo_protegido_y_la_raiz_no_se_tocan() {
        if !available() {
            return;
        }
        let d = dir("protegido");
        std::fs::write(d.join(".gotv/permissions.json"), "{}").unwrap();
        let spec = Spec {
            root: d.clone(),
            hosts: vec![],
            all_net: false,
            sockets: vec![],
            credenciales: vec![],
            entorno: vec![],
        };
        let (rc, _) = run(
            &spec,
            "echo '{\"allow\":[\"bash\"]}' > .gotv/permissions.json",
        )
        .await;
        assert!(rc != 0);
        assert_eq!(
            std::fs::read_to_string(d.join(".gotv/permissions.json")).unwrap(),
            "{}",
            "un comando no se da permisos"
        );
        // un repo NUEVO se puede crear dentro de la caja
        let (rc, out) = run(&spec, "git init -q && echo creado").await;
        assert!(rc == 0 && out.contains("creado"), "{out}");
        // ya existe: sus ganchos y su config quedan protegidos
        let (rc, _) = run(&spec, "echo 'echo pwned' > .git/hooks/pre-commit").await;
        assert!(
            rc != 0 && !d.join(".git/hooks/pre-commit").exists(),
            "nada de ganchos escondidos"
        );
        let (rc, _) = run(&spec, "git config core.hooksPath /tmp/malo").await;
        assert!(rc != 0, "ni por la config");
        // git sí trabaja: commits, ramas…
        let (rc, out) = run(&spec, "echo a > a.txt && git add a.txt && git -c user.email=a@b -c user.name=g commit -qm uno && git log --oneline | wc -l").await;
        assert!(rc == 0 && out.trim().ends_with('1'), "{out}");
        // D11: el /tmp COMPARTIDO no; la temporal privada ($TMPDIR) sí
        let ajeno = format!("/private/tmp/gotv-caja-{}.txt", std::process::id());
        let (rc, _) = run(&spec, &format!("echo x > {ajeno}")).await;
        assert!(rc != 0 && !std::path::Path::new(&ajeno).exists(), "nada al /tmp compartido");
        // …salvo la carpeta de candados de pnpm: su binario nativo los guarda ahí y no en $TMPDIR
        // (sin ella `pnpm install` no arrancaba dentro de la caja; medido 2026-11-14)
        let (rc, out) = run(&spec, "d=/private/tmp/pnpm-store-operation-locks-$(id -u); mkdir -p \"$d\" && touch \"$d/gotv-prueba-$$.lock\" && rm \"$d/gotv-prueba-$$.lock\" && echo ok").await;
        assert!(rc == 0 && out.contains("ok"), "los candados de pnpm sí: {out}");
        let (rc, out) = run(&spec, "f=\"${TMPDIR:-/nada}/gotv-caja-$$.txt\"; echo x > \"$f\" && rm \"$f\" && echo ok").await;
        assert!(rc == 0 && out.contains("ok"), "la temporal privada sí: {out}");
        let (_, _) = run(&spec, &format!("rm -rf {}", d.display())).await;
        assert!(d.exists(), "la carpeta del proyecto no se borra");
    }

    #[tokio::test]
    async fn la_red_de_fuera_no_sale_sin_proxy_y_localhost_si() {
        if !available() {
            return;
        }
        let d = dir("red");
        let spec = Spec {
            root: d.clone(),
            hosts: vec![],
            all_net: false,
            sockets: vec![],
            credenciales: vec![],
            entorno: vec![],
        };
        let (rc, _) = run(&spec, "curl -sS -m 5 -o /dev/null https://example.com").await;
        assert!(rc != 0, "sin el proxy no hay red de fuera");
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        std::thread::spawn(move || {
            if let Ok((mut c, _)) = l.accept() {
                use std::io::{Read, Write};
                let mut b = [0u8; 256];
                let _ = c.read(&mut b);
                let _ = c.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok");
            }
        });
        let (rc, out) = run(&spec, &format!("curl -sS -m 5 http://127.0.0.1:{port}/")).await;
        assert!(rc == 0 && out.contains("ok"), "{out}");
    }


    /// Un servidor de un solo uso en un socket Unix: entrega `dato` al primero que conecte.
    fn socket_con(ruta: &Path, dato: &'static str) {
        let _ = std::fs::remove_file(ruta);
        let l = std::os::unix::net::UnixListener::bind(ruta).unwrap();
        std::thread::spawn(move || {
            if let Ok((mut c, _)) = l.accept() {
                use std::io::Write;
                let _ = c.write_all(dato.as_bytes());
            }
        });
    }

    /// F14.0 · las sondas, permanentes. Lo AJENO queda fuera de la caja: un socket de otro
    /// programa, el agente de SSH y los secretos del llavero. Lo PROPIO funciona: un socket del
    /// proyecto, uno creado por el comando en su temporal, y el que el usuario apuntó a mano.
    #[tokio::test]
    async fn los_sockets_ajenos_el_agente_de_ssh_y_el_llavero_quedan_fuera() {
        if !available() {
            return;
        }
        // en la temporal (ruta corta: un socket Unix no admite más de 104 bytes)
        let caja = crate::CarpetaPrueba::nueva("gotv-f14");
        let d = canon(&caja);
        std::fs::create_dir_all(d.join(".gotv")).unwrap();
        let spec = Spec { root: d.clone(), hosts: vec![], all_net: false, sockets: vec![], credenciales: vec![], entorno: vec![] };
        let pid = std::process::id();

        // 1 · un socket de FUERA (en el /tmp compartido, como el del agente de SSH): no se lee
        let ajeno = PathBuf::from(format!("/private/tmp/gf-{pid}.sock"));
        socket_con(&ajeno, "dato-de-fuera");
        let (_, out) = run(&spec, &format!("nc -U {} </dev/null; echo rc=$?", ajeno.display())).await;
        assert!(!out.contains("dato-de-fuera") && !out.contains("rc=0"), "un socket ajeno no se alcanza: {out}");
        // …ni el de OTRO programa en la temporal del usuario (ahí viven los del editor)
        let vecino = canon(&std::env::temp_dir()).join(format!("gv-{pid}.sock"));
        socket_con(&vecino, "dato-del-vecino");
        let (_, out) = run(&spec, &format!("nc -U {} </dev/null; echo rc=$?", vecino.display())).await;
        assert!(!out.contains("dato-del-vecino"), "ni en la temporal general: {out}");

        // 2 · uno del PROYECTO sí
        socket_con(&d.join("s.sock"), "dato-del-proyecto");
        let (rc, out) = run(&spec, "nc -U s.sock </dev/null").await;
        assert!(rc == 0 && out.contains("dato-del-proyecto"), "{out}");
        // …y el comando puede ESCUCHAR en su temporal y hablarse a sí mismo (antes no podía)
        let (_, out) = run(&spec, "(nc -lU \"${TMPDIR}p.sock\" > recibido 2>&1 &); sleep 0.5; echo hola-propio | nc -U \"${TMPDIR}p.sock\"; sleep 0.3; cat recibido").await;
        assert!(out.contains("hola-propio"), "un socket propio en la temporal de la caja: {out}");
        assert!(temporal_propia(&d).is_dir() && temporal_propia(&d).to_string_lossy().len() + 1 + 31 <= 104, "cabe el socket más largo de Python");

        // 3 · el que el usuario apunta en `sockets_allow` se alcanza, y sólo ése
        socket_con(&ajeno, "dato-permitido");
        let permitido = Spec { sockets: vec![ajeno.clone()], ..spec.clone() };
        let (rc, out) = run(&permitido, &format!("nc -U {} </dev/null", ajeno.display())).await;
        assert!(rc == 0 && out.contains("dato-permitido"), "{out}");
        let _ = std::fs::remove_file(&ajeno);
        let _ = std::fs::remove_file(&vecino);

        // 4 · el agente de SSH: ni heredado ni alcanzable
        let (_, out) = run(&spec, "echo \"[${SSH_AUTH_SOCK:-sin-agente}]\"; ssh-add -l 2>&1; echo rc=$?").await;
        assert!(out.contains("[sin-agente]") && out.contains("rc=2") && !out.contains("no identities"), "{out}");

        // 5 · el llavero: un secreto de un llavero ABIERTO no sale (llavero de prueba propio, aparte
        //     del del usuario; si esta Mac no deja crearlo, la sonda no aplica)
        let llavero = caja.join("sonda.keychain");
        let crear = std::process::Command::new("security").args(["create-keychain", "-p", "clave-de-prueba"]).arg(&llavero).output();
        if crear.is_ok_and(|o| o.status.success()) {
            let guardar = std::process::Command::new("security").args(["add-generic-password", "-s", "gotv-sonda-f14", "-a", "sonda", "-w", "secreto-de-prueba-123"]).arg(&llavero).output().unwrap();
            assert!(guardar.status.success());
            let leer = format!("security find-generic-password -s gotv-sonda-f14 -w {} 2>&1; echo rc=$?", llavero.display());
            let fuera = std::process::Command::new("bash").args(["-c", &leer]).output().unwrap();
            assert!(String::from_utf8_lossy(&fuera.stdout).contains("secreto-de-prueba-123"), "fuera de la caja sí se lee (la sonda sirve)");
            let (_, out) = run(&spec, &leer).await;
            assert!(!out.contains("secreto-de-prueba-123") && !out.contains("rc=0"), "dentro de la caja NO: {out}");
        }
    }

    /// F14.0 · la MATRIZ DE NO-ROTURA, con la red de verdad por el proxy de GOTV (a mano:
    /// `cargo test -p gotv-core --release -- --ignored --nocapture matriz_de_no_rotura`). Cada
    /// herramienta debe seguir trabajando dentro de la caja y verificando certificados.
    #[tokio::test]
    #[ignore]
    async fn matriz_de_no_rotura() {
        let caja = crate::CarpetaPrueba::nueva("gotv-f14-matriz");
        let d = canon(&caja);
        std::fs::create_dir_all(d.join(".gotv")).unwrap();
        let spec = Spec { root: d.clone(), hosts: vec!["github.com".into(), "badssl.com".into()], all_net: false, sockets: vec![], credenciales: vec![], entorno: vec![] };
        let permiso = crate::netproxy::grant(hosts(&spec), false);
        let puerto = crate::netproxy::ensure().unwrap();
        let casos: &[(&str, &str, bool)] = &[
            ("git local", "git init -q && echo a > a && git add a && git -c user.email=a@b -c user.name=g commit -qm x && git status --short | wc -l", true),
            ("git fetch por HTTPS", "git ls-remote --heads https://github.com/rust-lang/cfg-if.git | head -1", true),
            ("curl HTTPS", "curl -sS -m 20 -o /dev/null -w '%{http_code}' https://pypi.org/pypi/six/json", true),
            ("curl rechaza un certificado malo", "curl -sS -m 20 -o /dev/null https://self-signed.badssl.com/", false),
            ("cargo fetch + build", "mkdir -p cg/src && cd cg && printf '[package]\\nname=\"cg\"\\nversion=\"0.1.0\"\\nedition=\"2021\"\\n[dependencies]\\ncfg-if=\"1\"\\n[workspace]\\n' > Cargo.toml && echo 'fn main(){}' > src/main.rs && cargo fetch -q && cargo build -q --offline", true),
            ("cargo habla con crates.io", "cargo search cfg-if --limit 1 | head -1", true),
            ("npm install", "mkdir ni && cd ni && echo '{}' > package.json && npm install --no-audit --no-fund -s left-pad && ls node_modules | head -1", true),
            ("pnpm add", "command -v pnpm >/dev/null || exit 0; mkdir pn && cd pn && echo '{}' > package.json && pnpm add left-pad && ls node_modules | head -1", true),
            ("pip en un venv", "python3 -m venv pv && pv/bin/pip install -q --no-cache-dir --disable-pip-version-check six", true),
            ("uv", "command -v uv >/dev/null || exit 0; mkdir uvp && cd uvp && uv venv -q && uv pip install -q --no-cache six", true),
            ("rustc", "printf 'fn main(){println!(\"hola\");}' > h.rs && rustc -o h h.rs && ./h", true),
            ("clang", "printf 'int main(){return 0;}' > c.c && cc -o c c.c && ./c", true),
            ("multiproceso de Python", "printf 'import multiprocessing as m\\nif __name__==\"__main__\":\\n    q=m.Manager().Queue(); q.put(41); print(q.get()+1)\\n' > mp.py && python3 mp.py | grep -q 42", true),
            ("servidor de desarrollo", "python3 -u -m http.server 0 --bind 127.0.0.1 >srv.log 2>&1 & sleep 1.5; p=$(grep -o 'port [0-9]*' srv.log | cut -d' ' -f2); curl -sS -m 5 -o /dev/null http://127.0.0.1:$p/; r=$?; kill %1; exit $r", true),
        ];
        let mut fallos = Vec::new();
        for (nombre, cmd, debe_pasar) in casos {
            let mut c = command(&spec, cmd);
            for (k, v) in crate::netproxy::env_for(puerto, &permiso) {
                c.env(k, v);
            }
            let o = c.current_dir(&d).output().await.unwrap();
            let paso = o.status.success();
            let cola: String = String::from_utf8_lossy(&o.stderr).lines().rev().take(2).collect::<Vec<_>>().join(" ⏎ ");
            println!("{} {nombre}{}", if paso == *debe_pasar { "✓" } else { "✗" }, if paso == *debe_pasar { String::new() } else { format!(" — {cola}") });
            if paso != *debe_pasar {
                fallos.push(*nombre);
            }
        }
        assert!(fallos.is_empty(), "rotos dentro de la caja: {fallos:?}");
    }

    /// F14.1 · las credenciales y los secretos del entorno quedan fuera de la caja; lo demás de la
    /// carpeta personal se sigue leyendo. Con una carpeta personal FALSA (no se lee ni se toca nada
    /// del usuario): el comando de la caja corre con `HOME` apuntando a ella, y la lista de fábrica se
    /// resuelve contra la del proceso, así que aquí se prueba por `credenciales_extra`, que es la misma
    /// regla, y la de fábrica con su forma.
    #[tokio::test]
    async fn las_credenciales_y_los_secretos_del_entorno_quedan_fuera() {
        if !available() {
            return;
        }
        let caja = crate::CarpetaPrueba::nueva("gotv-f141");
        let d = canon(&caja);
        let proyecto = d.join("proyecto");
        std::fs::create_dir_all(proyecto.join(".gotv")).unwrap();
        let casa = d.join("casa");
        std::fs::create_dir_all(casa.join(".ssh")).unwrap();
        std::fs::write(casa.join(".ssh/id_prueba"), "llave-secreta-de-prueba").unwrap();
        std::fs::write(casa.join(".gitconfig"), "[user]\n  name = prueba\n").unwrap();
        std::fs::write(casa.join(".otra-credencial"), "otro-secreto").unwrap();
        let spec = Spec {
            root: proyecto.clone(),
            hosts: vec![],
            all_net: false,
            sockets: vec![],
            credenciales: vec![casa.join(".ssh"), casa.join(".otra-credencial")],
            entorno: vec!["DEJAME_PASAR_TOKEN".into()],
        };
        let correr = |cmd: String| {
            let mut c = command(&spec, &cmd);
            c.current_dir(&proyecto).env("GITHUB_TOKEN", "token-que-no-debe-llegar").env("OPENAI_API_KEY", "otra-que-tampoco").env("DEJAME_PASAR_TOKEN", "esta-si").env("EDITOR", "vim");
            c
        };
        let salida = |o: std::process::Output| format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr));
        // 1 · la llave y la otra credencial: ni leer ni listar
        let o = salida(correr(format!("cat {}/.ssh/id_prueba; ls {}/.ssh; cat {}/.otra-credencial; echo fin", casa.display(), casa.display(), casa.display())).output().await.unwrap());
        assert!(!o.contains("llave-secreta-de-prueba") && !o.contains("id_prueba\n") && !o.contains("otro-secreto") && o.contains("fin"), "{o}");
        // …y escribir tampoco (aunque fuera una zona escribible)
        let o = salida(correr(format!("echo x > {}/.ssh/nueva; echo rc=$?", casa.display())).output().await.unwrap());
        assert!(!casa.join(".ssh/nueva").exists(), "{o}");
        // 2 · lo demás de la carpeta personal se sigue leyendo (`~/.gitconfig`)
        let o = salida(correr(format!("cat {}/.gitconfig", casa.display())).output().await.unwrap());
        assert!(o.contains("name = prueba"), "{o}");
        // 3 · los secretos del entorno HEREDADO no llegan; los demás y los permitidos, sí (el mecanismo,
        //     con un entorno heredado inventado; el de este proceso de pruebas no trae tokens)
        let mut c = std::process::Command::new("/usr/bin/true");
        let heredado = ["GITHUB_TOKEN", "OPENAI_API_KEY", "aws_secret_access_key", "DEJAME_PASAR_TOKEN", "EDITOR", "PATH", "HOME"].map(std::ffi::OsString::from);
        quitar_secretos(&mut c, heredado, &spec.entorno);
        let mut quitadas: Vec<String> = c.get_envs().filter(|(_, v)| v.is_none()).map(|(k, _)| k.to_string_lossy().into_owned()).collect();
        quitadas.sort();
        assert_eq!(quitadas, ["GITHUB_TOKEN", "OPENAI_API_KEY", "aws_secret_access_key"]);
        // y `command` lo aplica al entorno real del proceso: ningún secreto heredado queda sin quitar
        let real = command(&spec, "true");
        let quitadas_reales: Vec<String> = real.as_std().get_envs().filter(|(_, v)| v.is_none()).map(|(k, _)| k.to_string_lossy().into_owned()).collect();
        for (k, _) in std::env::vars() {
            if variable_secreta(&k) && !spec.entorno.contains(&k) {
                assert!(quitadas_reales.contains(&k), "{k} se heredaría");
            }
        }
        // 4 · un proyecto DENTRO de una carpeta de credenciales no queda ilegible para sí mismo
        let dentro = casa.join(".ssh/un-proyecto");
        std::fs::create_dir_all(dentro.join(".gotv")).unwrap();
        std::fs::write(dentro.join("a.txt"), "hola").unwrap();
        let s2 = Spec { root: dentro.clone(), ..spec.clone() };
        let o = salida(command(&s2, "cat a.txt").current_dir(&dentro).output().await.unwrap());
        assert!(o.contains("hola"), "{o}");
        // 5 · la lista de fábrica: las rutas de siempre están, y la regla nombra cada una
        for r in [".ssh", ".aws", ".gnupg", ".config/gh", "Library/Keychains", ".docker/config.json", ".config/gotv", "Library/Application Support/Google/Chrome", "Library/Cookies"] {
            assert!(CREDENCIALES.contains(&r), "{r}");
        }
        assert!(policy(0, 0, 0, 3).contains("(deny file-read* file-write* (subpath (param \"C0\")) (subpath (param \"C1\")) (subpath (param \"C2\")))"));
        assert!(variable_secreta("GITHUB_TOKEN") && variable_secreta("aws_secret_access_key") && variable_secreta("HF_TOKEN") && variable_secreta("DB_PASSWORD"));
        assert!(!variable_secreta("PATH") && !variable_secreta("HOME") && !variable_secreta("EDITOR") && !variable_secreta("LANG") && !variable_secreta("TMPDIR"));
        // 6 · `~/.npmrc`: pasa el registro (también el de un ámbito), jamás el token
        let rc = d.join("npmrc-prueba");
        std::fs::write(&rc, "# comentario\nregistry=https://registry.yarnpkg.com/\n@miempresa:registry=https://npm.pkg.github.com/\n//npm.pkg.github.com/:_authToken=ghp_falso\n_auth=YWJj\nemail=a@b.co\nalways-auth=true\nstrict-ssl=false\nregistry2=${NPM_REG}\n").unwrap();
        let pasa = npmrc_sin_secretos(&rc);
        assert_eq!(pasa, [("registry".to_string(), "https://registry.yarnpkg.com/".to_string()), ("@miempresa:registry".into(), "https://npm.pkg.github.com/".into()), ("strict-ssl".into(), "false".into())]);
        assert!(CREDENCIALES.contains(&".npmrc"));
    }

    /// A MANO: lo que se puede leer desde la caja en la carpeta personal REAL (sólo si se puede, nunca
    /// el contenido). `cargo test -p gotv-core --release -- --ignored --nocapture credenciales_reales`
    #[tokio::test]
    #[ignore]
    async fn credenciales_reales() {
        let caja = crate::CarpetaPrueba::nueva("gotv-f141-real");
        let d = canon(&caja);
        std::fs::create_dir_all(d.join(".gotv")).unwrap();
        let spec = Spec { root: d.clone(), hosts: vec![], all_net: false, sockets: vec![], credenciales: vec![], entorno: vec![] };
        let casa = std::env::var("HOME").unwrap();
        for r in CREDENCIALES.iter().chain([".gitconfig", ".zshrc", "Documents"].iter()) {
            let p = format!("{casa}/{r}");
            if !std::path::Path::new(&p).exists() {
                continue;
            }
            let cmd = format!("if [ -d \"{p}\" ]; then ls \"{p}\" >/dev/null 2>&1; else head -c 1 \"{p}\" >/dev/null 2>&1; fi && echo LEGIBLE || echo negado");
            let o = command(&spec, &cmd).current_dir(&d).output().await.unwrap();
            println!("{:<44} {}", format!("~/{r}"), String::from_utf8_lossy(&o.stdout).trim());
        }
    }
    #[tokio::test]
    async fn la_proteccion_vale_aunque_el_proyecto_viva_en_tmp() {
        if !available() {
            return;
        }
        let caja = crate::CarpetaPrueba::nueva("gotv-caja-en-tmp");
        let d = canon(&caja);
        std::fs::create_dir_all(d.join(".gotv")).unwrap();
        std::fs::write(d.join(".gotv/permissions.json"), "{}").unwrap();
        let spec = Spec {
            root: d.clone(),
            hosts: vec![],
            all_net: false,
            sockets: vec![],
            credenciales: vec![],
            entorno: vec![],
        };
        let (rc, _) = run(&spec, "echo malo > .gotv/permissions.json").await;
        assert!(rc != 0);
        assert_eq!(
            std::fs::read_to_string(d.join(".gotv/permissions.json")).unwrap(),
            "{}"
        );
    }
}

#[cfg(test)]
mod a_mano {
    /// A MANO: vuelca la política REAL y sus parámetros para experimentar con `sandbox-exec -f`.
    /// `GOTV_CAJA_VOLCAR=/ruta/politica.sb GOTV_CAJA_RAIZ=/proyecto cargo test -p gotv-core --release -- --ignored volcar_la_politica`
    #[test]
    #[ignore]
    fn volcar_la_politica() {
        let destino = std::env::var("GOTV_CAJA_VOLCAR").expect("GOTV_CAJA_VOLCAR");
        let raiz = super::canon(std::path::Path::new(&std::env::var("GOTV_CAJA_RAIZ").expect("GOTV_CAJA_RAIZ")));
        let extra = super::extra_writable();
        let prot = super::protected(&raiz);
        std::fs::write(&destino, super::policy(extra.len(), prot.len(), 0, 0)).unwrap();
        let mut args = vec![format!("-DROOT={}", raiz.display()), format!("-DCAJATMP={}", super::temporal_propia(&raiz).display())];
        args.extend(extra.iter().enumerate().map(|(i, p)| format!("-DW{i}={}", p.display())));
        args.extend(prot.iter().enumerate().map(|(i, p)| format!("-DPROT{i}={}", p.display())));
        std::fs::write(format!("{destino}.args"), args.join("\n")).unwrap();
    }
}
