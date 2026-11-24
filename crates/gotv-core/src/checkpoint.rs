//! Checkpoint v2 (auditoría C5) — deshace también lo que hizo el shell, sin el
//! costo ni los riesgos del v1.
//!
//! - **Contenido por hash**: cada versión de un archivo se guarda UNA vez en
//!   `.gotv/checkpoints/blobs/<sha256>`.
//! - **Incremental**: un índice (tamaño + fecha) evita releer lo que no cambió;
//!   tras la primera pasada, una foto sólo lee los archivos tocados.
//! - **Changeset por paso**: antes y después de cada herramienta que muta se
//!   compara el árbol; se guarda sólo lo que ESE paso cambió (`before`/`after`).
//!   Un comando de sólo lectura no deja paso.
//! - **Rewind atómico y respetuoso**: si un archivo cambió después del paso de
//!   GOTV (lo editó el usuario), no se toca NADA salvo con `force`.
//! - No sigue symlinks; excluye `.git/.gotv/target/node_modules/.venv/__pycache__`.

use anyhow::{anyhow, bail, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

const EXCLUDE_DIRS: &[&str] = &[
    ".git",
    ".gotv",
    "target",
    "node_modules",
    ".venv",
    "__pycache__",
];
const MAX_FILE: u64 = 8 * 1024 * 1024; // 8 MiB
const MAX_STEPS: usize = 100;

#[derive(Serialize, Deserialize, Clone, PartialEq)]
struct Stat {
    size: u64,
    mtime_ns: u64,
    hash: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Change {
    pub path: String,
    pub before: Option<String>,
    pub after: Option<String>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Step {
    pub id: usize,
    pub label: String,
    pub changes: Vec<Change>,
}

/// Lo que guardó un paso: su id y qué archivos cambió (con sus versiones).
#[derive(Debug)]
pub struct Committed {
    pub id: usize,
    pub changes: Vec<Change>,
}

/// Estado del árbol antes de un paso (path → hash).
pub struct Pending {
    pre: BTreeMap<String, String>,
    /// Si la herramienta declara qué archivo toca, el paso sólo se atribuye ESE:
    /// lo que el usuario edite en otro archivo mientras tanto no es del agente
    /// (medido: un log redirigido dentro del proyecto entraba al paso y
    /// `rewind` lo habría revertido).
    only: Option<BTreeSet<String>>,
}

pub struct Checkpoint {
    root: PathBuf,
    dir: PathBuf,
}

impl Checkpoint {
    pub fn new(root: &Path) -> Self {
        Self {
            root: root.to_path_buf(),
            dir: root.join(".gotv/checkpoints"),
        }
    }

    fn blobs(&self) -> PathBuf {
        self.dir.join("blobs")
    }

    /// `steps` / `index` en formato propio (códec, CKPT: `<nombre>.gotv`); hasta 2026-11-17 eran
    /// `<nombre>.json` y se migran solos (el historial de deshacer no se pierde).
    fn read_json<T: for<'de> Deserialize<'de> + Serialize + Default>(&self, name: &str) -> T {
        let base = name.trim_end_matches(".json");
        crate::programar::codec::leer_serde_migrado(&self.dir.join(format!("{base}.gotv")), &self.dir.join(format!("{base}.json")), b"CKPT")
            .unwrap_or_default()
    }

    fn write_json<T: Serialize>(&self, name: &str, v: &T) -> Result<()> {
        // escritura atómica (temporal + renombrar)
        crate::programar::codec::guardar_serde(&self.dir.join(format!("{}.gotv", name.trim_end_matches(".json"))), b"CKPT", v)
    }

    fn steps(&self) -> Vec<Step> {
        self.read_json("steps.json")
    }

    /// F8.4: la ruta absoluta de una ruta guardada (`@nombre/…` es de una carpeta extra del proyecto).
    /// `None` si la extra ya no está en el proyecto: entonces no se toca nada de ella.
    pub fn abs(&self, rel: &str) -> Option<PathBuf> {
        if rel.starts_with('@') {
            return crate::raices::resolver_extra(&self.root, rel);
        }
        Some(self.root.join(rel))
    }

    /// F8.4: la ruta guardada de un archivo absoluto (de la principal o de una extra).
    pub fn relativa(&self, abs: &Path) -> Option<String> {
        let canon = self.root.canonicalize().unwrap_or_else(|_| self.root.clone());
        if let Ok(r) = abs.strip_prefix(&self.root).or_else(|_| abs.strip_prefix(&canon)) {
            return Some(r.to_string_lossy().replace('\\', "/"));
        }
        crate::raices::extras_de(&self.root).into_iter().find_map(|(n, base)| {
            let c = base.canonicalize().unwrap_or_else(|_| base.clone());
            abs.strip_prefix(&base).or_else(|_| abs.strip_prefix(&c)).ok().map(|r| format!("@{n}/{}", r.to_string_lossy().replace('\\', "/")))
        })
    }

    /// Recorre el árbol; sólo lee y hashea lo que cambió según el índice.
    fn scan(&self) -> Result<BTreeMap<String, String>> {
        let mut index: BTreeMap<String, Stat> = self.read_json("index.json");
        let mut files = Vec::new();
        walk(&self.root, &self.root, &mut files);
        // F8.4: las carpetas extra del proyecto, con sus rutas como `@nombre/…` (deshacer las devuelve juntas)
        for (n, base) in crate::raices::extras_de(&self.root) {
            let mut v = Vec::new();
            walk(&base, &base, &mut v);
            files.extend(v.into_iter().map(|(a, r)| (a, format!("@{n}/{r}"))));
        }
        fs::create_dir_all(self.blobs())?;

        let mut state = BTreeMap::new();
        let mut fresh = BTreeMap::new();
        for (abs, rel) in files {
            let Ok(meta) = fs::symlink_metadata(&abs) else {
                continue;
            };
            let mtime_ns = meta
                .modified()
                .ok()
                .and_then(|m| m.duration_since(UNIX_EPOCH).ok())
                .map(|d| d.as_nanos() as u64)
                .unwrap_or(0);
            let size = meta.len();
            let hash = match index.get(&rel) {
                Some(s) if s.size == size && s.mtime_ns == mtime_ns => s.hash.clone(),
                _ => {
                    let Ok(bytes) = fs::read(&abs) else { continue };
                    let h = hex(&Sha256::digest(&bytes));
                    let blob = self.blobs().join(&h);
                    if !blob.exists() {
                        fs::write(&blob, &bytes)?;
                    }
                    h
                }
            };
            fresh.insert(
                rel.clone(),
                Stat {
                    size,
                    mtime_ns,
                    hash: hash.clone(),
                },
            );
            state.insert(rel, hash);
        }
        index = fresh;
        self.write_json("index.json", &index)?;
        Ok(state)
    }

    /// Foto previa a un paso que puede mutar el disco.
    pub fn begin(&self) -> Result<Pending> {
        Ok(Pending {
            pre: self.scan()?,
            only: None,
        })
    }

    /// Foto previa a un paso que sólo puede tocar `files` (rutas absolutas).
    /// Las que caen fuera del proyecto no se pueden proteger y se ignoran.
    pub fn begin_scoped(&self, files: &[PathBuf]) -> Result<Pending> {
        // F8.4: también los de una carpeta extra (como `@nombre/…`)
        let only = files.iter().filter_map(|f| self.relativa(f)).collect();
        Ok(Pending {
            pre: self.scan()?,
            only: Some(only),
        })
    }

    /// Cierra el paso: guarda SÓLO lo que cambió. `None` si no cambió nada.
    pub fn commit(&self, pending: Pending, label: &str) -> Result<Option<Committed>> {
        let post = self.scan()?;
        let paths: BTreeSet<&String> = pending.pre.keys().chain(post.keys()).collect();
        let changes: Vec<Change> = paths
            .into_iter()
            .filter(|p| pending.only.as_ref().is_none_or(|o| o.contains(*p)))
            .filter_map(|p| {
                let (b, a) = (pending.pre.get(p), post.get(p));
                (b != a).then(|| Change {
                    path: p.clone(),
                    before: b.cloned(),
                    after: a.cloned(),
                })
            })
            .collect();
        if changes.is_empty() {
            return Ok(None);
        }
        let mut steps = self.steps();
        let id = steps.last().map(|s| s.id + 1).unwrap_or(0);
        let committed = Committed {
            id,
            changes: changes.clone(),
        };
        steps.push(Step {
            id,
            label: label.to_string(),
            changes,
        });
        if steps.len() > MAX_STEPS {
            let drop_n = steps.len() - MAX_STEPS;
            steps.drain(..drop_n);
            self.write_json("steps.json", &steps)?;
            self.gc(&steps)?;
        } else {
            self.write_json("steps.json", &steps)?;
        }
        Ok(Some(committed))
    }

    /// Diff unificado de lo que cambió un paso, sacado de los blobs del propio
    /// checkpoint: cubre ediciones Y lo que tocó un `bash`. Acotado a `max_lines`.
    pub fn render_diff(&self, c: &Committed, max_lines: usize) -> Vec<String> {
        let read = |h: &Option<String>| -> String {
            h.as_ref()
                .and_then(|h| fs::read(self.blobs().join(h)).ok())
                .map(|b| String::from_utf8_lossy(&b).into_owned())
                .unwrap_or_default()
        };
        let mut out = Vec::new();
        for ch in &c.changes {
            let kind = match (&ch.before, &ch.after) {
                (None, Some(_)) => crate::i18n::t("creado", "created"),
                (Some(_), None) => crate::i18n::t("borrado", "deleted"),
                _ => crate::i18n::t("editado", "edited"),
            };
            // lo GENERADO (build, dependencias, lockfiles, minificado) no se pinta línea a línea:
            // medido (D5) un `npm run build` llenaba el registro con 900 líneas de JS minificado
            if es_generado(&ch.path) {
                out.push(crate::tr!("  ┌ {} ({kind} · generado, no se muestra)", "  ┌ {} ({kind} · generated, not shown)", ch.path));
                continue;
            }
            // lo BINARIO (docx/xlsx/pptx son zip, imágenes, PDF) tampoco: medido (banco of-plantilla) un
            // `cp` de un .docx volcó el zip entero en lo que leía el modelo
            let crudo = |h: &Option<String>| h.as_ref().and_then(|h| fs::read(self.blobs().join(h)).ok()).unwrap_or_default();
            let (a, b) = (crudo(&ch.before), crudo(&ch.after));
            let binario = |x: &[u8]| x[..x.len().min(8192)].contains(&0) || std::str::from_utf8(x).is_err();
            if binario(&a) || binario(&b) {
                let kb = a.len().max(b.len()).div_ceil(1024);
                out.push(crate::tr!("  ┌ {} ({kind} · binario, {kb} KB, no se muestra)", "  ┌ {} ({kind} · binary, {kb} KB, not shown)", ch.path));
                continue;
            }
            out.push(format!("  ┌ {} ({kind})", ch.path));
            let (old, new) = (read(&ch.before), read(&ch.after));
            let diff = similar::TextDiff::from_lines(&old, &new);
            for hunk in diff.unified_diff().context_radius(2).iter_hunks() {
                out.push(format!("  │ {}", hunk.header()));
                for change in hunk.iter_changes() {
                    let sign = match change.tag() {
                        similar::ChangeTag::Delete => '-',
                        similar::ChangeTag::Insert => '+',
                        similar::ChangeTag::Equal => ' ',
                    };
                    let v = change.value().trim_end_matches('\n');
                    // una línea kilométrica (minificado, datos) se recorta
                    let v = if v.chars().count() > 240 { format!("{}…", v.chars().take(240).collect::<String>()) } else { v.to_string() };
                    out.push(format!("  │{sign}{v}"));
                    // como git: si lo único distinto es el salto final, se dice
                    if change.tag() != similar::ChangeTag::Equal && change.missing_newline() {
                        out.push(
                            crate::i18n::t(
                                "  │ \\ sin salto de línea al final",
                                "  │ \\ no newline at end of file",
                            )
                            .into(),
                        );
                    }
                }
            }
        }
        if out.len() > max_lines {
            let extra = out.len() - max_lines;
            out.truncate(max_lines);
            out.push(crate::tr!(
                "  │ … {extra} líneas más (gotv checkpoints)",
                "  │ … {extra} more lines (gotv checkpoints)"
            ));
        }
        out
    }

    /// Borra blobs que ya no referencia ningún paso ni el índice.
    fn gc(&self, steps: &[Step]) -> Result<()> {
        let index: BTreeMap<String, Stat> = self.read_json("index.json");
        let mut live: BTreeSet<String> = index.values().map(|s| s.hash.clone()).collect();
        for s in steps {
            for c in &s.changes {
                live.extend(c.before.iter().cloned());
                live.extend(c.after.iter().cloned());
            }
        }
        if let Ok(rd) = fs::read_dir(self.blobs()) {
            for e in rd.flatten() {
                if !live.contains(&e.file_name().to_string_lossy().to_string()) {
                    fs::remove_file(e.path()).ok();
                }
            }
        }
        Ok(())
    }

    /// Deshace el último paso. Atómico: si algún archivo cambió después del paso
    /// (alguien más lo editó), no toca nada salvo `force`.
    /// F8.4 (medido 2026-11-18 con Huihui): el modelo editó `app/` y `@api/` en dos llamadas = dos pasos, y
    /// deshacer el último paso dejaba el turno a medias. `rewind` deshace el ÚLTIMO TURNO entero (todos sus
    /// pasos, del último al primero), como `/rewind` de Claude Code vuelve a antes de tu último mensaje; un solo
    /// paso es [`Checkpoint::rewind_paso`]. Sin marca de turno (pasos de antes), deshace un paso.
    pub fn rewind(&self, force: bool) -> Result<String> {
        let steps = self.steps();
        let Some(ultimo) = steps.last() else {
            return Ok(crate::i18n::t("(no hay pasos que deshacer)", "(no steps to undo)").into());
        };
        let inicio = self.turnos().into_iter().filter(|t| *t <= ultimo.id).max();
        let del_turno: Vec<&Step> = match inicio {
            Some(b) => steps.iter().filter(|s| s.id >= b).collect(),
            None => vec![ultimo],
        };
        if del_turno.len() <= 1 {
            return self.rewind_paso(force);
        }
        // lo ajeno, ANTES de tocar nada: cada archivo debe seguir como lo dejó su ÚLTIMO paso del turno
        let mut ultimo_de: BTreeMap<&str, &Option<String>> = BTreeMap::new();
        for s in &del_turno {
            for c in &s.changes {
                ultimo_de.insert(c.path.as_str(), &c.after);
            }
        }
        let ajenos: Vec<&str> = ultimo_de.iter().filter(|(p, after)| current_hash(&self.abs(p).unwrap_or_default()) != ***after).map(|(p, _)| *p).collect();
        if !ajenos.is_empty() && !force {
            bail!(
                "{}",
                crate::tr!(
                    "no deshice nada: {} archivo(s) cambiaron DESPUÉS del último turno y los pisaría:\n  {}\n(usa `gotv rewind --force` si de verdad quieres volver atrás)",
                    "nothing undone: {} file(s) changed AFTER the last turn and would be overwritten:\n  {}\n(use `gotv rewind --force` if you really want to go back)",
                    ajenos.len(),
                    ajenos.join("\n  ")
                )
            );
        }
        let n = del_turno.len();
        for _ in 0..n {
            self.rewind_paso(true)?;
        }
        Ok(crate::tr!(
            "↩ deshecho el último turno: {n} paso(s), {} archivo(s) vuelven a como estaban{}",
            "↩ undid the last turn: {n} step(s), {} file(s) back to how they were{}",
            ultimo_de.len(),
            if ajenos.is_empty() { String::new() } else { crate::tr!(" — forzado sobre {} cambio(s) ajeno(s)", " — forced over {} foreign change(s)", ajenos.len()) }
        ))
    }

    /// F8.4 · el agente marca dónde EMPIEZA cada turno (el id que tendrá su primer paso): `rewind` deshace desde ahí.
    pub fn marcar_turno(&self) {
        let siguiente = self.steps().last().map(|s| s.id + 1).unwrap_or(0) as u64;
        let mut t = self.turnos_crudos();
        if t.last() != Some(&siguiente) {
            t.push(siguiente);
        }
        let sobra = t.len().saturating_sub(MAX_STEPS * 2);
        t.drain(..sobra);
        let _ = crate::programar::codec::guardar_serde(&self.dir.join("turnos.gotv"), b"CKTU", &t);
    }

    fn turnos_crudos(&self) -> Vec<u64> {
        crate::programar::codec::leer_serde(&self.dir.join("turnos.gotv"), b"CKTU").unwrap_or_default()
    }

    fn turnos(&self) -> Vec<usize> {
        self.turnos_crudos().into_iter().map(|t| t as usize).collect()
    }

    /// Deshace el ÚLTIMO PASO (un archivo o los que tocó esa llamada).
    pub fn rewind_paso(&self, force: bool) -> Result<String> {
        let mut steps = self.steps();
        let Some(step) = steps.last().cloned() else {
            return Ok(crate::i18n::t("(no hay pasos que deshacer)", "(no steps to undo)").into());
        };

        // F8.4: un paso con una carpeta extra que ya no está en el proyecto no se deshace a medias
        if let Some(c) = step.changes.iter().find(|c| self.abs(&c.path).is_none()) {
            return Err(anyhow!(
                "{}",
                crate::tr!(
                    "no deshice nada: el paso #{} toca {}, de una carpeta que ya no está en el proyecto (vuelve a añadirla con el mismo nombre)",
                    "nothing undone: step #{} touches {}, from a folder that is no longer in the project (add it again with the same name)",
                    step.id,
                    c.path
                )
            ));
        }
        let mut conflicts = Vec::new();
        for c in &step.changes {
            if current_hash(&self.abs(&c.path).unwrap_or_default()) != c.after {
                conflicts.push(c.path.clone());
            }
        }
        if !conflicts.is_empty() && !force {
            return Err(anyhow!(
                "{}",
                crate::tr!(
                    "no deshice nada: {} archivo(s) cambiaron DESPUÉS del paso #{} «{}» y los pisaría:\n  {}\n(usa `gotv rewind --force` si de verdad quieres volver atrás)",
                    "nothing undone: {} file(s) changed AFTER step #{} «{}» and would be overwritten:\n  {}\n(use `gotv rewind --force` if you really want to go back)",
                    conflicts.len(),
                    step.id,
                    step.label,
                    conflicts.join("\n  ")
                )
            ));
        }

        let (mut restored, mut removed) = (0, 0);
        for c in &step.changes {
            let target = self.abs(&c.path).unwrap_or_default();
            match &c.before {
                Some(h) => {
                    let bytes = fs::read(self.blobs().join(h)).map_err(|e| {
                        anyhow!(
                            "{}",
                            crate::tr!(
                                "falta el blob de {}: {e}",
                                "missing blob for {}: {e}",
                                c.path
                            )
                        )
                    })?;
                    if let Some(p) = target.parent() {
                        fs::create_dir_all(p)?;
                    }
                    fs::write(&target, bytes)?;
                    restored += 1;
                }
                None => {
                    if target.exists() {
                        fs::remove_file(&target)?;
                    }
                    removed += 1;
                }
            }
        }
        steps.pop();
        self.write_json("steps.json", &steps)?;
        Ok(crate::tr!(
            "↩ deshecho paso #{} «{}»: {restored} restaurado(s), {removed} creado(s) eliminado(s){}",
            "↩ undid step #{} «{}»: {restored} restored, {removed} created removed{}",
            step.id,
            step.label,
            if conflicts.is_empty() {
                String::new()
            } else {
                crate::tr!(" — forzado sobre {} cambio(s) ajeno(s)", " — forced over {} foreign change(s)", conflicts.len())
            }
        ))
    }

    /// Pasos guardados (id, etiqueta, nº de archivos cambiados), del más viejo al último.
    pub fn list(&self) -> Vec<(usize, String, usize)> {
        self.steps()
            .into_iter()
            .map(|s| (s.id, s.label, s.changes.len()))
            .collect()
    }

    pub fn count(&self) -> usize {
        self.steps().len()
    }

    /// F17 (regla L7): las rutas (relativas a la raíz) que cambió el AGENTE en algún paso guardado. Lo que
    /// no está aquí es del usuario: `git confirmar` no lo mete sin que él lo apruebe.
    pub fn rutas(&self) -> BTreeSet<String> {
        self.steps().into_iter().flat_map(|s| s.changes.into_iter().map(|c| c.path)).collect()
    }

    /// F7: el id del último paso guardado (`None` = ninguno). El turno que empieza anota desde dónde.
    pub fn ultimo_id(&self) -> Option<usize> {
        self.steps().last().map(|s| s.id)
    }

    /// F7: los pasos guardados DESPUÉS de `desde` (todos si es `None`): el changeset de un turno.
    pub fn pasos_desde(&self, desde: Option<usize>) -> Vec<Step> {
        self.steps().into_iter().filter(|s| desde.is_none_or(|d| s.id > d)).collect()
    }

    /// F8.2: la carpeta del proyecto que vigila.
    pub fn raiz(&self) -> &Path {
        &self.root
    }

    /// F8.2: el contenido guardado de una versión (por su hash), para pintar diferencias sin git.
    pub fn contenido(&self, hash: &str) -> Option<Vec<u8>> {
        if !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        fs::read(self.blobs().join(hash)).ok()
    }
}

/// Lo GENERADO (build, dependencias, lockfiles, minificado): no se pinta línea a línea (D5). Lo comparten el
/// registro del turno ([`Checkpoint::render_diff`]) y el panel de cambios (F8.2).
pub fn es_generado(ruta: &str) -> bool {
    let ruta = ruta.replace('\\', "/");
    ruta.split('/').any(|c| matches!(c, "dist" | "build" | "node_modules" | ".next" | "target" | "__pycache__" | ".venv" | "coverage"))
        || ruta.ends_with(".min.js")
        || ruta.ends_with(".map")
        || ["package-lock.json", "pnpm-lock.yaml", "yarn.lock", "Cargo.lock", "uv.lock", "poetry.lock"].iter().any(|l| ruta == *l || ruta.ends_with(&format!("/{l}")))
}

fn current_hash(p: &Path) -> Option<String> {
    let meta = fs::symlink_metadata(p).ok()?;
    if !meta.is_file() {
        return None;
    }
    fs::read(p).ok().map(|b| hex(&Sha256::digest(&b)))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Recorre sin seguir symlinks (file_type() no los sigue).
fn walk(root: &Path, dir: &Path, out: &mut Vec<(PathBuf, String)>) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let Ok(ft) = e.file_type() else { continue };
        let name = e.file_name().to_string_lossy().to_string();
        let p = e.path();
        if ft.is_symlink() {
            continue;
        }
        if ft.is_dir() {
            if !EXCLUDE_DIRS.contains(&name.as_str()) {
                walk(root, &p, out);
            }
        } else if ft.is_file()
            && name != ".DS_Store"
            && e.metadata().map(|m| m.len() <= MAX_FILE).unwrap_or(false)
        {
            let rel = p
                .strip_prefix(root)
                .unwrap_or(&p)
                .to_string_lossy()
                .to_string();
            out.push((p, rel));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::run_tool;
    use serde_json::json;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static N: AtomicUsize = AtomicUsize::new(0);

    /// Carpeta de la prueba con `a.txt`; se borra sola al soltarse.
    fn root() -> crate::CarpetaPrueba {
        let d = crate::CarpetaPrueba::nueva(&format!("gotv-cp-{}", N.fetch_add(1, Ordering::SeqCst)));
        fs::write(d.join("a.txt"), "original").unwrap();
        d
    }

    async fn step(cp: &Checkpoint, r: &Path, tool: &str, args: serde_json::Value) -> Option<usize> {
        let p = cp.begin().unwrap();
        run_tool(tool, &args, r).await;
        cp.commit(p, tool).unwrap().map(|c| c.id)
    }

    #[tokio::test]
    async fn diff_de_un_paso_incluye_lo_que_hizo_bash() {
        let r = root();
        let cp = Checkpoint::new(&r);
        let p = cp.begin().unwrap();
        run_tool(
            "bash",
            &json!({"cmd": "printf 'original\\nnueva\\n' > a.txt && echo hola > b.txt"}),
            &r,
        )
        .await;
        let c = cp.commit(p, "bash").unwrap().unwrap();
        let d = cp.render_diff(&c, 50).join("\n");
        assert!(d.contains("a.txt (editado)") && d.contains("+nueva"), "{d}");
        assert!(d.contains("b.txt (creado)") && d.contains("+hola"), "{d}");
    }

    #[tokio::test]
    async fn lo_binario_no_se_pinta_en_el_diff() {
        let r = root();
        let cp = Checkpoint::new(&r);
        let p = cp.begin().unwrap();
        // un .docx es un zip: bytes nulos y no-UTF-8
        fs::write(r.join("contrato.docx"), [b'P', b'K', 3, 4, 0, 0, 0xff, 0xfe, b'x']).unwrap();
        let c = cp.commit(p, "bash").unwrap().unwrap();
        let d = cp.render_diff(&c, 50).join("\n");
        assert!(d.contains("contrato.docx (creado · binario") && !d.contains("+PK"), "{d}");
    }

    #[tokio::test]
    async fn una_edicion_no_se_adueña_de_lo_que_cambió_otro() {
        let r = root();
        fs::write(r.join("log.txt"), "uno").unwrap();
        let cp = Checkpoint::new(&r);
        let p = cp.begin_scoped(&[r.join("a.txt")]).unwrap();
        run_tool(
            "write_file",
            &json!({"path": "a.txt", "content": "cambiado"}),
            &r,
        )
        .await;
        fs::write(r.join("log.txt"), "uno\ndos").unwrap(); // el usuario, en paralelo
        let c = cp.commit(p, "write_file").unwrap().unwrap();
        assert_eq!(c.changes.len(), 1);
        assert_eq!(c.changes[0].path, "a.txt");
        cp.rewind(false).unwrap();
        assert_eq!(fs::read_to_string(r.join("a.txt")).unwrap(), "original");
        assert_eq!(fs::read_to_string(r.join("log.txt")).unwrap(), "uno\ndos");
    }

    #[tokio::test]
    async fn el_diff_dice_cuando_solo_cambia_el_salto_final() {
        let r = root();
        fs::write(r.join("a.txt"), "uno\n").unwrap();
        let cp = Checkpoint::new(&r);
        let p = cp.begin().unwrap();
        fs::write(r.join("a.txt"), "uno").unwrap();
        let c = cp.commit(p, "x").unwrap().unwrap();
        let d = cp.render_diff(&c, 50).join("\n");
        assert!(d.contains("sin salto de línea al final"), "{d}");
    }

    /// F8.4 (medido con Huihui): un turno con DOS pasos se deshace entero; el turno anterior se queda; `--paso`
    /// sigue deshaciendo uno; y un archivo cambiado por fuera después del turno lo frena todo.
    #[tokio::test]
    async fn rewind_deshace_el_turno_entero() {
        let r = root();
        let cp = Checkpoint::new(&r);
        cp.marcar_turno();
        step(&cp, &r, "write_file", json!({"path": "a.txt", "content": "turno 1"})).await;
        cp.marcar_turno();
        step(&cp, &r, "write_file", json!({"path": "a.txt", "content": "turno 2"})).await;
        step(&cp, &r, "write_file", json!({"path": "b.txt", "content": "turno 2"})).await;
        let m = cp.rewind(false).unwrap();
        assert!(m.contains('2'), "{m}");
        assert_eq!(fs::read_to_string(r.join("a.txt")).unwrap(), "turno 1", "el turno anterior se queda");
        assert!(!r.join("b.txt").exists(), "lo creado en el turno desaparece");
        // otro turno, y algo de fuera lo toca después: no se deshace nada sin --force
        cp.marcar_turno();
        step(&cp, &r, "write_file", json!({"path": "a.txt", "content": "turno 3"})).await;
        step(&cp, &r, "write_file", json!({"path": "c.txt", "content": "turno 3"})).await;
        fs::write(r.join("c.txt"), "a mano").unwrap();
        assert!(cp.rewind(false).is_err());
        assert_eq!(fs::read_to_string(r.join("a.txt")).unwrap(), "turno 3", "nada a medias");
        // --paso: sólo el último paso
        cp.rewind_paso(true).unwrap();
        assert_eq!(fs::read_to_string(r.join("a.txt")).unwrap(), "turno 3");
        assert!(!r.join("c.txt").exists());
    }

    #[tokio::test]
    async fn deshace_una_edicion() {
        let r = root();
        let cp = Checkpoint::new(&r);
        step(
            &cp,
            &r,
            "write_file",
            json!({"path": "a.txt", "content": "cambiado"}),
        )
        .await;
        assert_eq!(fs::read_to_string(r.join("a.txt")).unwrap(), "cambiado");
        cp.rewind(false).unwrap();
        assert_eq!(fs::read_to_string(r.join("a.txt")).unwrap(), "original");
    }

    #[tokio::test]
    async fn deshace_un_rm_del_shell() {
        let r = root();
        let cp = Checkpoint::new(&r);
        step(&cp, &r, "bash", json!({"cmd": "rm a.txt"})).await;
        assert!(!r.join("a.txt").exists());
        cp.rewind(false).unwrap();
        assert_eq!(fs::read_to_string(r.join("a.txt")).unwrap(), "original");
    }

    #[tokio::test]
    async fn deshace_una_creacion_y_un_mv() {
        let r = root();
        let cp = Checkpoint::new(&r);
        step(
            &cp,
            &r,
            "bash",
            json!({"cmd": "mkdir -p sub && mv a.txt sub/b.txt && echo x > c.txt"}),
        )
        .await;
        cp.rewind(false).unwrap();
        assert!(r.join("a.txt").exists());
        assert!(!r.join("sub/b.txt").exists());
        assert!(!r.join("c.txt").exists());
    }

    #[tokio::test]
    async fn lectura_no_deja_paso() {
        let r = root();
        let cp = Checkpoint::new(&r);
        assert_eq!(
            step(&cp, &r, "bash", json!({"cmd": "ls && cat a.txt"})).await,
            None
        );
        assert_eq!(cp.count(), 0);
    }

    #[tokio::test]
    async fn no_pisa_lo_que_edito_el_usuario() {
        let r = root();
        let cp = Checkpoint::new(&r);
        step(
            &cp,
            &r,
            "write_file",
            json!({"path": "a.txt", "content": "gotv"}),
        )
        .await;
        fs::write(r.join("a.txt"), "del usuario").unwrap();
        assert!(cp.rewind(false).is_err());
        assert_eq!(fs::read_to_string(r.join("a.txt")).unwrap(), "del usuario");
        assert_eq!(cp.count(), 1, "el paso no debe perderse si no se deshizo");
        cp.rewind(true).unwrap();
        assert_eq!(fs::read_to_string(r.join("a.txt")).unwrap(), "original");
    }

    #[tokio::test]
    async fn pasos_en_pila() {
        let r = root();
        let cp = Checkpoint::new(&r);
        step(
            &cp,
            &r,
            "write_file",
            json!({"path": "a.txt", "content": "v1"}),
        )
        .await;
        step(
            &cp,
            &r,
            "write_file",
            json!({"path": "a.txt", "content": "v2"}),
        )
        .await;
        cp.rewind(false).unwrap();
        assert_eq!(fs::read_to_string(r.join("a.txt")).unwrap(), "v1");
        cp.rewind(false).unwrap();
        assert_eq!(fs::read_to_string(r.join("a.txt")).unwrap(), "original");
        assert!(cp.rewind(false).unwrap().contains("no hay pasos"));
    }

    /// Medición sobre un árbol real: `GOTV_BENCH_DIR=… cargo test --release -- --ignored --nocapture bench`
    #[test]
    #[ignore]
    fn bench_arbol_real() {
        let Ok(d) = std::env::var("GOTV_BENCH_DIR") else {
            return;
        };
        let r = PathBuf::from(d);
        let cp = Checkpoint::new(&r);
        let t = std::time::Instant::now();
        cp.begin().unwrap();
        println!("foto fría (lee y hashea todo, una vez): {:?}", t.elapsed());
        let t = std::time::Instant::now();
        let p = cp.begin().unwrap();
        println!("foto incremental (sólo stat):          {:?}", t.elapsed());
        let t = std::time::Instant::now();
        let id = cp.commit(p, "bench").unwrap();
        println!(
            "commit sin cambios:                    {:?} → paso {:?}",
            t.elapsed(),
            id
        );
        let (n, bytes) = fs::read_dir(cp.blobs())
            .unwrap()
            .flatten()
            .fold((0, 0u64), |(n, b), e| {
                (n + 1, b + e.metadata().map(|m| m.len()).unwrap_or(0))
            });
        println!("blobs: {n} ({:.1} MB)", bytes as f64 / 1e6);
    }

    #[tokio::test]
    async fn no_sigue_symlinks_ni_duplica_blobs() {
        let r = root();
        std::os::unix::fs::symlink("/usr", r.join("enlace")).unwrap();
        let cp = Checkpoint::new(&r);
        cp.begin().unwrap();
        let n1 = fs::read_dir(cp.blobs()).unwrap().count();
        cp.begin().unwrap();
        let n2 = fs::read_dir(cp.blobs()).unwrap().count();
        assert_eq!(n1, 1, "sólo a.txt; el symlink a /usr no se recorre");
        assert_eq!(n1, n2, "una segunda foto sin cambios no crea blobs");
    }
}
