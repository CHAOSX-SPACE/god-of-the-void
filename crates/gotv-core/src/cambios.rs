//! F8.2 · El panel de cambios: qué cambió, archivo por archivo y línea por línea.
//!
//! Sin estado propio de los archivos: lo lee de los puntos de restauración (`checkpoint.rs`) y, con git
//! (F17), del repositorio. La ventaja es la vista **por turno**: funciona SIN git (las versiones están en
//! los blobs del punto de restauración) y enseña también lo que cambió la terminal (`bash`), marcado
//! («cambiado por un comando»), porque el punto de restauración compara el árbol antes y después de cada
//! paso.
//!
//! Lo único que se anota aquí es DÓNDE empieza y termina cada turno (`.gotv/checkpoints/turnos`, en el
//! códec propio): el agente lo escribe al cerrar un turno que tocó archivos. Pasos que no caen en ningún
//! turno anotado (el turno en curso, o uno cortado) salen agrupados igual: nada se esconde.
//!
//! Nunca se pinta un archivo de credenciales (F14.1: las rutas de [`crate::sandbox::CREDENCIALES`], los
//! `.env` y las llaves); en lo demás, los secretos conocidos se tapan (`secretos::enmascarar`). Sin git, un
//! archivo renombrado se ve como borrado + nuevo, y así se dice: no se adivina.

use crate::checkpoint::{self, Checkpoint, Step};
use crate::programar::codec::{self, Candado, Escritor, Lector};
use anyhow::{anyhow, bail, Result};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// Tope de líneas por archivo: lo demás se recorta con su aviso (un diff de 5 000 líneas no ahoga la ventana).
pub const MAX_LINEAS: usize = 400;
/// Turnos que recuerda el registro (los puntos de restauración guardan los últimos 100 pasos).
const MAX_TURNOS: usize = 200;
const TIPO: &[u8; 4] = b"TURN";
const VERSION: u8 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Estado {
    Nuevo,
    Cambiado,
    Borrado,
}

/// Un trozo de diferencia: desde qué línea (1 = la primera) en cada lado, y sus líneas con su signo
/// (`' '` igual, `'+'` añadida, `'-'` quitada).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Trozo {
    pub desde_viejo: u32,
    pub desde_nuevo: u32,
    pub lineas: Vec<(char, String)>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Cambio {
    pub ruta: String,
    pub estado: Estado,
    pub mas: u32,
    pub menos: u32,
    pub binario: bool,
    pub trozos: Vec<Trozo>,
    /// lo generado (build, dependencias, lockfiles): se cuenta, no se pinta
    pub generado: bool,
    /// un archivo de credenciales: NUNCA se pinta
    pub credencial: bool,
    /// lo cambió la terminal (`bash` o un programa del chat), no una edición de GOTV
    pub por_comando: bool,
    /// las líneas que tenía la diferencia si se recortó a [`MAX_LINEAS`]
    pub recortado: Option<u32>,
}

/// Un turno del registro (o un grupo de pasos sin turno anotado).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Turno {
    /// el número estable del turno (los grupos sin anotar llevan 1 000 000 + su primer paso)
    pub n: u32,
    pub sesion: Option<i64>,
    /// el último paso ANTES del turno (`None` = desde el principio)
    pub desde: Option<usize>,
    /// el último paso del turno
    pub hasta: usize,
    /// la orden del Portador (vacía en un grupo sin anotar)
    pub orden: String,
    pub fecha: String,
    /// cuántos archivos cambió (neto)
    pub archivos: usize,
    /// pasos que no caen en ningún turno anotado: el turno en curso o uno que se cortó
    pub sin_anotar: bool,
}

/// Un comentario del panel: viaja en el siguiente mensaje del Portador. La línea es la del lado NUEVO.
#[derive(Clone, Debug, PartialEq)]
pub struct Comentario {
    pub ruta: String,
    pub linea_nueva: u32,
    pub texto: String,
}

impl Comentario {
    /// `[Comentario en src/a.rs L42] …`
    pub fn para_el_mensaje(&self) -> String {
        crate::tr!("[Comentario en {} L{}] {}", "[Comment on {} L{}] {}", self.ruta, self.linea_nueva, self.texto.trim())
    }
}

/// Los comentarios del panel delante del mensaje (uno por línea); sin comentarios, el mensaje tal cual.
pub fn con_comentarios(mensaje: &str, comentarios: &[Comentario]) -> String {
    let c: Vec<String> = comentarios.iter().filter(|c| !c.texto.trim().is_empty()).map(Comentario::para_el_mensaje).collect();
    if c.is_empty() {
        return mensaje.to_string();
    }
    if mensaje.trim().is_empty() {
        c.join("\n")
    } else {
        format!("{}\n\n{mensaje}", c.join("\n"))
    }
}

// ── el registro de turnos ──────────────────────────────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq)]
struct Anotado {
    n: u32,
    sesion: Option<i64>,
    desde: Option<usize>,
    hasta: usize,
    orden: String,
    fecha: String,
}

fn ruta_registro(root: &Path) -> PathBuf {
    root.join(".gotv/checkpoints/turnos")
}

fn leer_registro(root: &Path) -> Vec<Anotado> {
    let Ok(b) = std::fs::read(ruta_registro(root)) else { return vec![] };
    let leer = || -> Result<Vec<Anotado>> {
        let (mut l, _) = Lector::nuevo(&b, TIPO, VERSION)?;
        let n = l.u32()? as usize;
        (0..n)
            .map(|_| {
                Ok(Anotado {
                    n: l.u32()?,
                    sesion: l.opt_i64()?,
                    desde: l.opt_i64()?.map(|d| d.max(0) as usize),
                    hasta: l.i64()?.max(0) as usize,
                    orden: l.texto()?,
                    fecha: l.texto()?,
                })
            })
            .collect()
    };
    leer().unwrap_or_default()
}

/// Anota un turno que tocó archivos: sus pasos van de `desde` (excluido) a `hasta` (incluido). Lo llama el
/// agente al cerrar el turno. Devuelve su número.
pub fn anotar_turno(root: &Path, sesion: Option<i64>, desde: Option<usize>, hasta: usize, orden: &str) -> Result<u32> {
    if desde.is_some_and(|d| d >= hasta) {
        bail!("{}", crate::i18n::t("el turno no tiene pasos", "the turn has no steps"));
    }
    let ruta = ruta_registro(root);
    std::fs::create_dir_all(ruta.parent().unwrap_or(root))?;
    let _c = Candado::esperar(&ruta.with_extension("candado"))?;
    let mut v = leer_registro(root);
    let n = v.last().map(|a| a.n + 1).unwrap_or(1);
    let orden: String = orden.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(200).collect();
    v.push(Anotado { n, sesion, desde, hasta, orden, fecha: chrono::Local::now().format("%Y-%m-%d %H:%M").to_string() });
    if v.len() > MAX_TURNOS {
        v.drain(..v.len() - MAX_TURNOS);
    }
    let mut e = Escritor::nuevo(TIPO, VERSION);
    e.u32(v.len() as u32);
    for a in &v {
        e.u32(a.n);
        e.opt_i64(a.sesion);
        e.opt_i64(a.desde.map(|d| d as i64));
        e.i64(a.hasta as i64);
        e.texto(&a.orden);
        e.texto(&a.fecha);
    }
    codec::guardar(&ruta, &e.bytes())?;
    Ok(n)
}

/// Los turnos con cambios que aún están en los puntos de restauración, del más nuevo al más viejo. Los pasos
/// que no caen en ningún turno anotado salen agrupados (`sin_anotar`). `sesion`: sólo los de ese chat (los
/// grupos sin anotar se ven siempre).
pub fn turnos(cp: &Checkpoint, sesion: Option<i64>) -> Vec<Turno> {
    let pasos = cp.pasos_desde(None);
    let reg = leer_registro(cp.raiz());
    // el turno MÁS NUEVO que contiene el paso: tras deshacer, los ids de los pasos se reusan y un turno
    // viejo con el mismo tramo ya no es el dueño
    let dueno = |id: usize| reg.iter().rev().find(|a| a.desde.is_none_or(|d| id > d) && id <= a.hasta);
    // cada paso, a su turno; los sueltos consecutivos, a un grupo
    let mut grupos: Vec<(Option<&Anotado>, Vec<&Step>)> = Vec::new();
    for p in &pasos {
        let a = dueno(p.id);
        match grupos.last_mut() {
            Some((g, v)) if g.map(|x| x.n) == a.map(|x| x.n) => v.push(p),
            _ => grupos.push((a, vec![p])),
        }
    }
    let mut out: Vec<Turno> = grupos
        .into_iter()
        .filter(|(a, _)| sesion.is_none() || a.is_none_or(|a| a.sesion == sesion))
        .map(|(a, v)| {
            let archivos = crate::artefactos::recorrido::cambios_netos(&v.iter().map(|s| (*s).clone()).collect::<Vec<_>>()).len();
            let primero = v.first().map(|s| s.id).unwrap_or(0);
            let ultimo = v.last().map(|s| s.id).unwrap_or(0);
            match a {
                Some(a) => Turno { n: a.n, sesion: a.sesion, desde: a.desde, hasta: a.hasta, orden: a.orden.clone(), fecha: a.fecha.clone(), archivos, sin_anotar: false },
                None => Turno { n: 1_000_000 + primero as u32, sesion: None, desde: primero.checked_sub(1), hasta: ultimo, orden: String::new(), fecha: String::new(), archivos, sin_anotar: true },
            }
        })
        .filter(|t| t.archivos > 0)
        .collect();
    out.reverse();
    out
}

fn pasos_del_turno(cp: &Checkpoint, n: u32) -> Result<(Turno, Vec<Step>)> {
    let t = turnos(cp, None).into_iter().find(|t| t.n == n).ok_or_else(|| {
        anyhow!("{}", crate::tr!("el turno {n} ya no está en los puntos de restauración (guardan los últimos 100 pasos)", "turn {n} is no longer in the restore points (they keep the last 100 steps)"))
    })?;
    let pasos = cp.pasos_desde(t.desde).into_iter().filter(|s| s.id <= t.hasta).collect();
    Ok((t, pasos))
}

// ── las diferencias ────────────────────────────────────────────────────────────────────────────────────

/// ¿Es un archivo de credenciales (no se pinta nunca)?
pub fn es_credencial(root: &Path, ruta: &str) -> bool {
    let nombre = ruta.rsplit('/').next().unwrap_or(ruta);
    let env = nombre == ".env" || (nombre.starts_with(".env.") && !nombre.ends_with(".example") && !nombre.ends_with(".sample") && !nombre.ends_with(".template"));
    let llave = [".pem", ".key", ".p12", ".pfx", ".keystore", ".jks"].iter().any(|e| nombre.ends_with(e)) || nombre.starts_with("id_rsa") || nombre.starts_with("id_ed25519") || nombre.starts_with("id_ecdsa");
    let conocido = crate::sandbox::CREDENCIALES.iter().any(|c| {
        let c = c.trim_end_matches('/');
        ruta == c || ruta.starts_with(&format!("{c}/")) || ruta.ends_with(&format!("/{c}")) || ruta.contains(&format!("/{c}/")) || nombre == c.rsplit('/').next().unwrap_or(c) && c.starts_with('.') && !c.contains('/')
    });
    env || llave || conocido || crate::sandbox::es_credencial(&root.join(ruta)).is_some()
}

fn es_binario(b: &[u8]) -> bool {
    b[..b.len().min(8192)].contains(&0) || std::str::from_utf8(b).is_err()
}

/// La diferencia de UN archivo entre dos versiones (`None` = no existía).
fn diferencia(root: &Path, ruta: &str, viejo: Option<&[u8]>, nuevo: Option<&[u8]>, secretos: &[(String, String)]) -> Cambio {
    let estado = match (viejo, nuevo) {
        (None, _) => Estado::Nuevo,
        (_, None) => Estado::Borrado,
        _ => Estado::Cambiado,
    };
    let mut c = Cambio { ruta: ruta.to_string(), estado, mas: 0, menos: 0, binario: false, trozos: vec![], generado: false, credencial: false, por_comando: false, recortado: None };
    if es_credencial(root, ruta) {
        c.credencial = true;
        return c;
    }
    let (a, b) = (viejo.unwrap_or_default(), nuevo.unwrap_or_default());
    if es_binario(a) || es_binario(b) {
        c.binario = true;
        return c;
    }
    let (a, b) = (std::str::from_utf8(a).unwrap_or_default(), std::str::from_utf8(b).unwrap_or_default());
    let diff = similar::TextDiff::from_lines(a, b);
    let generado = checkpoint::es_generado(ruta);
    let mut total = 0usize;
    for g in diff.grouped_ops(3) {
        let (Some(primero), Some(_)) = (g.first(), g.last()) else { continue };
        let mut t = Trozo { desde_viejo: primero.old_range().start as u32 + 1, desde_nuevo: primero.new_range().start as u32 + 1, lineas: vec![] };
        for op in &g {
            for ch in diff.iter_changes(op) {
                let signo = match ch.tag() {
                    similar::ChangeTag::Delete => {
                        c.menos += 1;
                        '-'
                    }
                    similar::ChangeTag::Insert => {
                        c.mas += 1;
                        '+'
                    }
                    similar::ChangeTag::Equal => ' ',
                };
                total += 1;
                if !generado && total <= MAX_LINEAS {
                    let v = ch.value().trim_end_matches(['\n', '\r']);
                    // una línea kilométrica (minificado, datos) se recorta
                    let v = if v.chars().count() > 400 { format!("{}…", v.chars().take(400).collect::<String>()) } else { v.to_string() };
                    t.lineas.push((signo, crate::secretos::enmascarar(&v, secretos).0));
                }
            }
        }
        if !t.lineas.is_empty() {
            c.trozos.push(t);
        }
    }
    if generado {
        c.generado = true;
    } else if total > MAX_LINEAS {
        c.recortado = Some(total as u32);
    }
    c
}

/// F8.2: lo que cambió el turno `n` (de [`turnos`]), archivo por archivo. Funciona SIN git. Un archivo creado
/// y borrado dentro del mismo turno no aparece (al final no cambió nada).
pub fn por_turno(cp: &Checkpoint, n: u32) -> Result<Vec<Cambio>> {
    let (_, pasos) = pasos_del_turno(cp, n)?;
    Ok(cambios_de_pasos(cp, &pasos))
}

/// Los cambios netos de una serie de pasos, con quién los hizo.
pub fn cambios_de_pasos(cp: &Checkpoint, pasos: &[Step]) -> Vec<Cambio> {
    let secretos = crate::secretos::del_proyecto(cp.raiz());
    // qué herramientas tocaron cada archivo (de la etiqueta del paso: «herramienta · paso N»)
    let mut quien: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for p in pasos {
        let h = p.label.split(" · ").next().unwrap_or("");
        for c in &p.changes {
            quien.entry(c.path.as_str()).or_default().insert(h);
        }
    }
    crate::artefactos::recorrido::versiones_netas(pasos)
        .into_iter()
        .filter(|(_, a, d)| a != d)
        .map(|(ruta, a, d)| {
            let (va, vd) = (a.as_deref().and_then(|h| cp.contenido(h)), d.as_deref().and_then(|h| cp.contenido(h)));
            let mut c = diferencia(cp.raiz(), &ruta, a.as_ref().map(|_| va.as_deref().unwrap_or_default()), d.as_ref().map(|_| vd.as_deref().unwrap_or_default()), &secretos);
            c.por_comando = quien.get(ruta.as_str()).is_some_and(|s| s.iter().any(|h| matches!(*h, "bash" | "process")));
            c
        })
        .collect()
}

/// F8.2: devuelve UN archivo a como estaba antes del turno `n`; los demás archivos del turno no se tocan. Crea
/// su propio punto de restauración (se deshace con «deshacer»). Si el archivo cambió DESPUÉS del turno, no se
/// pisa salvo `forzar`.
pub fn revertir_archivo(cp: &Checkpoint, n: u32, ruta: &Path, forzar: bool) -> Result<String> {
    let (t, pasos) = pasos_del_turno(cp, n)?;
    // F8.4: también un archivo de una carpeta extra (su ruta guardada es `@nombre/…`)
    let rel = cp.relativa(ruta).unwrap_or_else(|| ruta.to_string_lossy().replace('\\', "/"));
    let Some((_, antes, despues)) = crate::artefactos::recorrido::versiones_netas(&pasos).into_iter().find(|(p, _, _)| *p == rel) else {
        bail!("{}", crate::tr!("{rel} no cambió en el turno {n}", "{rel} did not change in turn {n}"));
    };
    let abs = cp.abs(&rel).ok_or_else(|| anyhow!("{}", crate::tr!("{rel} es de una carpeta que ya no está en el proyecto", "{rel} belongs to a folder that is no longer in the project")))?;
    let ahora = std::fs::read(&abs).ok().map(|b| hex_sha(&b));
    if ahora != despues && !forzar {
        bail!(
            "{}",
            crate::tr!(
                "{rel} cambió DESPUÉS del turno {}: no lo piso (revertir de todos modos lo forzaría)",
                "{rel} changed AFTER turn {}: I will not overwrite it (reverting anyway would force it)",
                t.n
            )
        );
    }
    let pend = cp.begin_scoped(std::slice::from_ref(&abs))?;
    match &antes {
        Some(h) => {
            let b = cp.contenido(h).ok_or_else(|| anyhow!("{}", crate::tr!("falta la versión anterior de {rel}", "the previous version of {rel} is missing")))?;
            if let Some(d) = abs.parent() {
                std::fs::create_dir_all(d)?;
            }
            std::fs::write(&abs, b)?;
        }
        None => {
            if abs.is_file() {
                std::fs::remove_file(&abs)?;
            }
        }
    }
    let etiqueta = crate::tr!("revertir · {rel} al turno {}", "revert · {rel} to turn {}", t.n);
    match cp.commit(pend, &etiqueta)? {
        Some(c) => Ok(crate::tr!("↩ {rel} volvió a como estaba antes del turno {} (paso #{}: se deshace con «deshacer»)", "↩ {rel} is back to how it was before turn {} (step #{}: «undo» reverts it)", t.n, c.id)),
        None => Ok(crate::tr!("{rel} ya estaba como antes del turno {}", "{rel} was already as before turn {}", t.n)),
    }
}

fn hex_sha(b: &[u8]) -> String {
    use sha2::Digest;
    sha2::Sha256::digest(b).iter().map(|x| format!("{x:02x}")).collect()
}

// ── con git (F17) ──────────────────────────────────────────────────────────────────────────────────────

/// El contenido de `ruta` en un árbol de git (`None` = no está).
fn en_arbol(repo: &gix::Repository, arbol: Option<&gix::Tree<'_>>, ruta: &str) -> Option<Vec<u8>> {
    let e = arbol?.lookup_entry_by_path(ruta).ok().flatten()?;
    if !e.mode().is_blob() {
        return None;
    }
    repo.find_object(e.object_id()).ok().map(|o| o.data.clone())
}

/// Las rutas que la carpeta tiene distintas del índice o de HEAD (estado de git), relativas al repositorio.
fn rutas_de_estado(repo: &gix::Repository) -> Result<BTreeSet<String>> {
    let mut v = BTreeSet::new();
    for item in repo.status(gix::progress::Discard)?.into_iter(Vec::<gix::bstr::BString>::new())?.flatten() {
        match item {
            gix::status::Item::IndexWorktree(i) => {
                if i.summary().is_some() {
                    v.insert(i.rela_path().to_string());
                }
            }
            gix::status::Item::TreeIndex(c) => {
                v.insert(c.location().to_string());
            }
        }
    }
    Ok(v)
}

/// Cada archivo de un árbol con su id.
fn archivos_de(arbol: &gix::Tree<'_>) -> BTreeMap<String, gix::ObjectId> {
    let mut rec = gix::traverse::tree::Recorder::default();
    if arbol.traverse().breadthfirst(&mut rec).is_err() {
        return BTreeMap::new();
    }
    rec.records.into_iter().filter(|e| e.mode.is_blob()).map(|e| (e.filepath.to_string(), e.oid)).collect()
}

/// Lo que la carpeta tiene distinto de `base` (un árbol) en `rutas`, sólo dentro de `root`.
fn contra_carpeta(repo: &gix::Repository, root: &Path, base: Option<&gix::Tree<'_>>, rutas: BTreeSet<String>) -> Result<Vec<Cambio>> {
    let w = crate::git::leer::carpeta(repo)?;
    let root_c = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let secretos = crate::secretos::del_proyecto(root);
    let mut out = Vec::new();
    for r in rutas {
        let abs = w.join(&r);
        let Ok(rel) = abs.strip_prefix(&root_c) else { continue };
        let rel = rel.to_string_lossy().replace('\\', "/");
        let viejo = en_arbol(repo, base, &r);
        let nuevo = if abs.is_file() { std::fs::read(&abs).ok() } else { None };
        if viejo == nuevo {
            continue;
        }
        out.push(diferencia(root, &rel, viejo.as_deref(), nuevo.as_deref(), &secretos));
    }
    Ok(out)
}

/// F8.2 con git: lo que la carpeta tiene SIN CONFIRMAR respecto del último commit (lo preparado, lo no
/// preparado y lo nuevo sin seguir).
pub fn sin_confirmar(root: &Path) -> Result<Vec<Cambio>> {
    let repo = crate::git::leer::abrir(root)?;
    let head = repo.head_commit().ok().and_then(|c| c.tree().ok());
    let rutas = rutas_de_estado(&repo)?;
    contra_carpeta(&repo, root, head.as_ref(), rutas)
}

/// F8.2 con git: lo que la RAMA cambió desde que salió de `base` (el ancestro común con `base`) hasta la
/// carpeta tal como está (commits de la rama + lo sin confirmar).
pub fn de_rama(root: &Path, base: &str) -> Result<Vec<Cambio>> {
    let repo = crate::git::leer::abrir(root)?;
    let head = repo.head_id().map_err(|_| anyhow!("{}", crate::i18n::t("la rama aún no tiene commits", "the branch has no commits yet")))?.detach();
    let otra = repo.rev_parse_single(base).map_err(|_| anyhow!("{}", crate::tr!("no conozco la rama «{base}»", "I don't know the branch «{base}»")))?.detach();
    let comun = repo.merge_base(head, otra)?.ok_or_else(|| anyhow!("{}", crate::tr!("la rama no tiene un ancestro común con «{base}»", "the branch has no common ancestor with «{base}»")))?.detach();
    let arbol_comun = repo.find_commit(comun)?.tree()?;
    let arbol_head = repo.find_commit(head)?.tree()?;
    let (a, b) = (archivos_de(&arbol_comun), archivos_de(&arbol_head));
    let mut rutas: BTreeSet<String> = a.keys().chain(b.keys()).filter(|k| a.get(*k) != b.get(*k)).cloned().collect();
    rutas.extend(rutas_de_estado(&repo)?);
    contra_carpeta(&repo, root, Some(&arbol_comun), rutas)
}

/// F8.2 · Confirmar (commit) desde el panel de Cambios: lo hace la PERSONA, así que entra cualquier archivo
/// que marque (la regla de «ajenos» de la herramienta `git` es para el modelo). Se confirma EXACTAMENTE lo
/// marcado (`git commit --only`), aunque haya otras cosas preparadas en el índice; las credenciales no se
/// confirman desde aquí; el mensaje es obligatorio; nada sale de la carpeta del repositorio. Sin `push`: subir
/// está en pausa hasta producción (decisión del Portador).
pub async fn confirmar(root: &Path, rutas: &[String], mensaje: &str) -> Result<String> {
    let mensaje = mensaje.trim();
    if mensaje.is_empty() {
        bail!("{}", crate::i18n::t("escribe el mensaje del commit", "write the commit message"));
    }
    if rutas.is_empty() {
        bail!("{}", crate::i18n::t("marca al menos un archivo", "tick at least one file"));
    }
    for r in rutas {
        let p = Path::new(r);
        if r.is_empty() || p.is_absolute() || p.components().any(|c| !matches!(c, std::path::Component::Normal(_))) {
            bail!("{}", crate::tr!("ruta no válida para confirmar: «{r}»", "invalid path to commit: «{r}»"));
        }
        if es_credencial(root, r) {
            bail!("{}", crate::tr!("«{r}» es una credencial: no se confirma desde aquí", "«{r}» is a credential: it is not committed from here"));
        }
    }
    let corto = std::time::Duration::from_secs(60);
    let mut a = vec!["add", "--all", "--"];
    a.extend(rutas.iter().map(String::as_str));
    let (ok, s) = crate::git::git(root, &a, corto).await?;
    if !ok {
        bail!("{}", s.trim().lines().take(20).collect::<Vec<_>>().join("\n"));
    }
    // un hook (pruebas) puede tardar
    let mut c = vec!["commit", "--only", "-m", mensaje, "--"];
    c.extend(rutas.iter().map(String::as_str));
    let (ok, s) = crate::git::git(root, &c, std::time::Duration::from_secs(300)).await?;
    if !ok {
        bail!("{}", crate::tr!("el commit no se hizo:\n{}", "the commit was not made:\n{}", s.trim().lines().take(40).collect::<Vec<_>>().join("\n")));
    }
    Ok(s.lines().next().unwrap_or("").trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// «Hecho cuando» de F8.2 con un MODELO REAL (Huihui por `gotv serve`), en un proyecto SIN git: un turno
    /// toca 3 archivos y el panel da sus 3 diferencias; un comentario en una línea viaja con la orden siguiente
    /// y el agente lo atiende; y se revierte UN archivo sin tocar los otros.
    /// `GOTV_PRUEBA_OPENAI=http://127.0.0.1:11437/v1 GOTV_PRUEBA_MODELO=<nombre> cargo test -p gotv-core --release
    ///  --lib -- --ignored --nocapture panel_sin_git_con_el_modelo_real`
    #[tokio::test]
    #[ignore]
    async fn panel_sin_git_con_el_modelo_real() {
        let (Ok(url), Ok(modelo)) = (std::env::var("GOTV_PRUEBA_OPENAI"), std::env::var("GOTV_PRUEBA_MODELO")) else { return };
        let motor = gotv_engine::cloud::OpenAiEngine::new("gotv", &url, String::new(), &modelo, false);
        let mut p = crate::artefactos::pruebas::agente_con("f8-2-real", crate::permissions::Mode::Bypass, motor);
        p.a.max_steps = 20;
        p.a.max_tokens = 4096;
        let r = p.a.root.clone();
        assert!(!r.join(".git").exists(), "proyecto SIN git");
        for f in ["a.js", "b.js", "c.js"] {
            std::fs::write(r.join(f), format!("// {f}\nexport function total(precio) {{\n  return precio * 1.19;\n}}\n")).unwrap();
        }
        let r1 = p.a.run("El IVA pasó del 19 % al 16 %: cámbialo en a.js, b.js y c.js.").await.unwrap();
        eprintln!("— turno 1 —\n{r1}");
        let cp = Checkpoint::new(&r);
        let t = turnos(&cp, None);
        let n = t.last().expect("un turno anotado").n;
        let cs = por_turno(&cp, n).unwrap();
        eprintln!("panel del turno {n}: {:?}", cs.iter().map(|c| (&c.ruta, c.mas, c.menos)).collect::<Vec<_>>());
        assert_eq!(cs.len(), 3, "las 3 diferencias");
        assert!(cs.iter().all(|c| c.mas >= 1 && c.menos >= 1 && !c.trozos.is_empty()));
        // un comentario en la línea nueva de b.js, con la orden siguiente (como la ventana)
        let linea = std::fs::read_to_string(r.join("b.js")).unwrap().lines().position(|l| l.contains("1.16")).map(|i| i as u32 + 1).unwrap_or(3);
        let c = Comentario { ruta: "b.js".into(), linea_nueva: linea, texto: "pon el 16 % en una constante IVA = 0.16 y úsala aquí".into() };
        let r2 = p.a.run(&con_comentarios("atiende mi comentario", &[c])).await.unwrap();
        eprintln!("— turno 2 —\n{r2}\n— b.js —\n{}", std::fs::read_to_string(r.join("b.js")).unwrap());
        let b = std::fs::read_to_string(r.join("b.js")).unwrap();
        assert!(b.contains("IVA") && b.contains("0.16"), "atendió el comentario: {b}");
        // revertir SÓLO c.js del turno 1
        let m = revertir_archivo(&cp, n, std::path::Path::new("c.js"), false).unwrap();
        eprintln!("{m}");
        assert!(std::fs::read_to_string(r.join("c.js")).unwrap().contains("1.19"), "c.js volvió");
        assert!(std::fs::read_to_string(r.join("a.js")).unwrap().contains("1.16"), "a.js se queda");
        assert!(std::fs::read_to_string(r.join("b.js")).unwrap().contains("IVA"), "b.js se queda");
    }

    /// Confirmar desde la ventana: sólo lo marcado (otro archivo preparado se queda preparado, sin entrar),
    /// también un archivo borrado; sin mensaje, sin archivos, una credencial o una ruta que sale: nada.
    #[tokio::test]
    async fn confirmar_solo_lo_marcado() {
        let d = proyecto("confirmar");
        let g = |a: &[&str]| std::process::Command::new("git").args(a).current_dir(&*d).output().unwrap();
        g(&["init", "-q", "-b", "main"]);
        g(&["config", "user.email", "p@x"]);
        g(&["config", "user.name", "Prueba"]);
        for f in ["a.txt", "b.txt", "c.txt"] {
            std::fs::write(d.join(f), "uno\n").unwrap();
        }
        g(&["add", "."]);
        g(&["commit", "-q", "-m", "base"]);
        std::fs::write(d.join("a.txt"), "dos\n").unwrap();
        std::fs::write(d.join("b.txt"), "dos\n").unwrap();
        std::fs::remove_file(d.join("c.txt")).unwrap();
        g(&["add", "b.txt"]); // preparado por el usuario, pero NO marcado
        std::fs::write(d.join(".env"), "CLAVE=x\n").unwrap();
        assert!(confirmar(&d, &["a.txt".into()], "  ").await.is_err(), "sin mensaje");
        assert!(confirmar(&d, &[], "x").await.is_err(), "sin archivos");
        assert!(confirmar(&d, &[".env".into()], "x").await.is_err(), "una credencial no");
        assert!(confirmar(&d, &["../fuera".into()], "x").await.is_err(), "no sale del repo");
        let r = confirmar(&d, &["a.txt".into(), "c.txt".into()], "Cambia a y quita c").await.unwrap();
        assert!(r.contains("Cambia a y quita c"), "{r}");
        let ultimo = String::from_utf8_lossy(&g(&["show", "--name-status", "--format=%s", "HEAD"]).stdout).to_string();
        assert!(ultimo.contains("M\ta.txt") && ultimo.contains("D\tc.txt") && !ultimo.contains("b.txt"), "{ultimo}");
        let estado = String::from_utf8_lossy(&g(&["status", "--porcelain"]).stdout).to_string();
        assert!(estado.contains("M  b.txt"), "b sigue preparado y sin confirmar: {estado}");
    }

    fn proyecto(tag: &str) -> crate::CarpetaPrueba {
        let d = crate::CarpetaPrueba::sin_crear(&format!("gotv-cambios-{tag}"));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// Un paso como los del agente: foto, cambio, cierre con la etiqueta de la herramienta.
    fn paso(cp: &Checkpoint, herramienta: &str, f: impl FnOnce()) {
        let p = cp.begin().unwrap();
        f();
        cp.commit(p, &format!("{herramienta} · paso 1")).unwrap();
    }

    /// Un turno que tocó 3 archivos (uno nuevo, uno cambiado, uno borrado) en un proyecto SIN git: 3 cambios.
    #[test]
    fn un_turno_de_tres_archivos_da_tres_cambios_sin_git() {
        let r = proyecto("tres");
        std::fs::write(r.join("a.rs"), "fn a() {}\nfn b() {}\nfn c() {}\n").unwrap();
        std::fs::write(r.join("viejo.txt"), "adiós\n").unwrap();
        let cp = Checkpoint::new(&r);
        cp.begin().unwrap(); // índice inicial
        let desde = cp.ultimo_id();
        paso(&cp, "edit", || std::fs::write(r.join("a.rs"), "fn a() {}\nfn b2() {}\nfn c() {}\n").unwrap());
        paso(&cp, "write_file", || std::fs::write(r.join("nuevo.md"), "# hola\n").unwrap());
        paso(&cp, "bash", || std::fs::remove_file(r.join("viejo.txt")).unwrap());
        let n = anotar_turno(&r, Some(7), desde, cp.ultimo_id().unwrap(), "  cambia   b\n y limpia ").unwrap();
        assert!(!r.join(".git").exists());
        let ts = turnos(&cp, None);
        assert_eq!(ts.len(), 1);
        assert_eq!((ts[0].n, ts[0].archivos, ts[0].orden.as_str(), ts[0].sin_anotar), (n, 3, "cambia b y limpia", false));
        assert_eq!(turnos(&cp, Some(7)).len(), 1);
        assert!(turnos(&cp, Some(8)).is_empty(), "el filtro por chat");
        let cs = por_turno(&cp, n).unwrap();
        assert_eq!(cs.len(), 3);
        let a = cs.iter().find(|c| c.ruta == "a.rs").unwrap();
        assert_eq!((a.estado, a.mas, a.menos, a.por_comando), (Estado::Cambiado, 1, 1, false));
        assert_eq!(a.trozos.len(), 1);
        assert_eq!((a.trozos[0].desde_viejo, a.trozos[0].desde_nuevo), (1, 1));
        assert!(a.trozos[0].lineas.contains(&('-', "fn b() {}".into())) && a.trozos[0].lineas.contains(&('+', "fn b2() {}".into())));
        let nuevo = cs.iter().find(|c| c.ruta == "nuevo.md").unwrap();
        assert_eq!((nuevo.estado, nuevo.mas), (Estado::Nuevo, 1));
        let borrado = cs.iter().find(|c| c.ruta == "viejo.txt").unwrap();
        assert_eq!((borrado.estado, borrado.menos, borrado.por_comando), (Estado::Borrado, 1, true), "lo borró la terminal, y se marca");
    }

    /// Creado y borrado en el mismo turno: no aparece. Un binario: `binario` y ningún trozo. Una credencial:
    /// nunca se pinta. Un diff de 5 000 líneas llega recortado a 400 con su aviso.
    #[test]
    fn efimero_binario_credencial_y_recorte() {
        let r = proyecto("casos");
        let cp = Checkpoint::new(&r);
        cp.begin().unwrap();
        paso(&cp, "bash", || std::fs::write(r.join("tmp.log"), "x\n").unwrap());
        paso(&cp, "bash", || std::fs::remove_file(r.join("tmp.log")).unwrap());
        paso(&cp, "bash", || std::fs::write(r.join("img.png"), [0x89, b'P', b'N', b'G', 0, 0, 1, 2]).unwrap());
        paso(&cp, "write_file", || std::fs::write(r.join(".env"), "API_KEY=supersecreto123\n").unwrap());
        paso(&cp, "write_file", || std::fs::write(r.join("largo.txt"), (0..5000).map(|i| format!("línea {i}\n")).collect::<String>()).unwrap());
        // sin anotar: sale igual, como un grupo
        let ts = turnos(&cp, None);
        assert_eq!(ts.len(), 1);
        assert!(ts[0].sin_anotar);
        let cs = por_turno(&cp, ts[0].n).unwrap();
        assert!(!cs.iter().any(|c| c.ruta == "tmp.log"), "creado y borrado en el turno: no está");
        let png = cs.iter().find(|c| c.ruta == "img.png").unwrap();
        assert!(png.binario && png.trozos.is_empty());
        let env = cs.iter().find(|c| c.ruta == ".env").unwrap();
        assert!(env.credencial && env.trozos.is_empty());
        assert!(!format!("{cs:?}").contains("supersecreto123"));
        let l = cs.iter().find(|c| c.ruta == "largo.txt").unwrap();
        assert_eq!(l.recortado, Some(5000));
        assert_eq!(l.trozos.iter().map(|t| t.lineas.len()).sum::<usize>(), MAX_LINEAS);
        assert_eq!(l.mas, 5000, "las cuentas son de la diferencia entera");
        assert!(es_credencial(&r, "config/id_rsa") && es_credencial(&r, ".npmrc") && es_credencial(&r, "a/.ssh/config") && !es_credencial(&r, ".env.example") && !es_credencial(&r, "src/main.rs"));
    }

    /// `revertir_archivo` devuelve UN archivo, deja los otros dos del turno como estaban, no pisa lo editado
    /// después, y se deshace con «deshacer» (`rewind`).
    #[test]
    fn revertir_un_archivo_deja_los_demas_y_se_deshace() {
        let r = proyecto("revertir");
        for f in ["a", "b", "c"] {
            std::fs::write(r.join(f), format!("{f} antes\n")).unwrap();
        }
        let cp = Checkpoint::new(&r);
        cp.begin().unwrap();
        let desde = cp.ultimo_id();
        for f in ["a", "b", "c"] {
            paso(&cp, "edit", || std::fs::write(r.join(f), format!("{f} después\n")).unwrap());
        }
        let n = anotar_turno(&r, None, desde, cp.ultimo_id().unwrap(), "cambia los tres").unwrap();
        let msg = revertir_archivo(&cp, n, &r.join("b"), false).unwrap();
        assert!(msg.contains('b'));
        let leer = |f: &str| std::fs::read_to_string(r.join(f)).unwrap();
        assert_eq!((leer("a"), leer("b"), leer("c")), ("a después\n".into(), "b antes\n".into(), "c después\n".into()));
        // el revertir es un paso propio: «deshacer» lo devuelve
        cp.rewind(false).unwrap();
        assert_eq!(leer("b"), "b después\n");
        // editado DESPUÉS del turno: no se pisa salvo forzar
        std::fs::write(r.join("c"), "c a mano\n").unwrap();
        assert!(revertir_archivo(&cp, n, &r.join("c"), false).is_err());
        assert_eq!(leer("c"), "c a mano\n");
        revertir_archivo(&cp, n, &r.join("c"), true).unwrap();
        assert_eq!(leer("c"), "c antes\n");
        // un archivo que el turno no tocó, o un turno que no existe
        assert!(revertir_archivo(&cp, n, &r.join("zzz"), false).is_err());
        assert!(revertir_archivo(&cp, 99, &r.join("a"), false).is_err());
    }

    #[test]
    fn el_comentario_viaja_en_el_siguiente_mensaje() {
        let c = vec![Comentario { ruta: "src/a.rs".into(), linea_nueva: 42, texto: " usa un Result aquí ".into() }, Comentario { ruta: "x".into(), linea_nueva: 1, texto: "  ".into() }];
        let m = con_comentarios("arréglalo", &c);
        assert!(m.starts_with(&crate::tr!("[Comentario en src/a.rs L42] usa un Result aquí", "[Comment on src/a.rs L42] usa un Result aquí")));
        assert!(m.ends_with("\n\narréglalo"));
        assert!(!m.contains("[Comentario en x") && !m.contains("[Comment on x"), "el vacío no viaja");
        assert_eq!(con_comentarios("hola", &[]), "hola");
    }

    /// Con git (F17): lo sin confirmar y lo de la rama, contra el binario `git` sólo para PREPARAR el repo.
    #[test]
    fn con_git_sin_confirmar_y_de_rama() {
        let r = proyecto("git");
        let git = |args: &[&str]| {
            let o = std::process::Command::new("git").args(args).current_dir(&r).env("GIT_AUTHOR_NAME", "t").env("GIT_AUTHOR_EMAIL", "t@t").env("GIT_COMMITTER_NAME", "t").env("GIT_COMMITTER_EMAIL", "t@t").output().unwrap();
            assert!(o.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&o.stderr));
        };
        git(&["init", "-q", "-b", "main"]);
        std::fs::write(r.join("a.txt"), "uno\n").unwrap();
        std::fs::write(r.join("b.txt"), "dos\n").unwrap();
        git(&["add", "."]);
        git(&["commit", "-q", "-m", "base"]);
        git(&["checkout", "-q", "-b", "rama"]);
        std::fs::write(r.join("a.txt"), "uno cambiado\n").unwrap();
        git(&["commit", "-q", "-am", "en la rama"]);
        // sin confirmar: b cambiado y c nuevo
        std::fs::write(r.join("b.txt"), "dos cambiado\n").unwrap();
        std::fs::write(r.join("c.txt"), "tres\n").unwrap();
        let mut s: Vec<(String, Estado)> = sin_confirmar(&r).unwrap().into_iter().map(|c| (c.ruta, c.estado)).collect();
        s.sort_by(|a, b| a.0.cmp(&b.0));
        assert_eq!(s, vec![("b.txt".into(), Estado::Cambiado), ("c.txt".into(), Estado::Nuevo)]);
        let mut d: Vec<String> = de_rama(&r, "main").unwrap().into_iter().map(|c| c.ruta).collect();
        d.sort();
        assert_eq!(d, ["a.txt", "b.txt", "c.txt"], "la rama: su commit y lo sin confirmar");
        assert!(de_rama(&r, "no-existe").is_err());
        assert!(sin_confirmar(&proyecto("sin-git")).is_err() || std::env::temp_dir().join(".git").exists());
    }
}
