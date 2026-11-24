//! F16.5 · `/revisar` y `/revisar seguridad`: una revisión de los cambios con hallazgos ESTRUCTURADOS en una
//! tarjeta (archivo, línea, resumen, cómo falla) y «arreglar éste» sobre uno.
//!
//! - **Qué se revisa** lo calcula GOTV, no el modelo: `git diff HEAD` (lo que aún no se confirmó, con lo
//!   preparado) más los archivos nuevos, o `--rama [base]` (lo de esta rama frente a su base), o una ruta.
//! - **Las dos skills** (revisión de código y de seguridad) son constantes de Rust en los dos idiomas, como
//!   las de F12: no gastan contexto si no se piden.
//! - **El turno de revisión sólo lee:** se le pone la lista de herramientas de lectura con la misma puerta de
//!   F16.2 (`permissions.rs`): un modelo que «decide» arreglar mientras revisa se encuentra el no.
//! - **Los hallazgos se validan contra el disco:** el archivo existe dentro del proyecto y la línea existe;
//!   la tarjeta lleva el TEXTO real de esa línea. Lo que no se puede ubicar se descarta y se dice.
//! - «Arreglar éste» es una orden normal (con permisos y puntos de restauración) sobre un hallazgo guardado.
use crate::frontend::Frontend;
use crate::Agent;
use anyhow::{bail, Result};
use gotv_engine::Engine;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::sync::{Mutex, OnceLock};

/// Lo que puede usar el turno de revisión (sólo mirar).
pub const SOLO_LECTURA: &[&str] = &["read_file", "list_dir", "glob", "grep", "diagnostics", "mapa", "rag_search", "skill", "skills"];
/// Tope del diff que viaja en la orden.
const DIFF_MAX: usize = 60_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tipo {
    Codigo,
    Seguridad,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Hallazgo {
    pub id: u32,
    pub archivo: String,
    pub linea: u32,
    pub resumen: String,
    pub como_falla: String,
    pub severidad: String,
    /// el texto real de esa línea en el disco
    pub codigo: String,
}

impl Hallazgo {
    pub fn json(&self) -> Value {
        // juez-idioma: datos (claves fijas del JSON que pinta la ventana)
        json!({"id": self.id, "archivo": self.archivo, "linea": self.linea, "resumen": self.resumen, "como_falla": self.como_falla, "severidad": self.severidad, "codigo": self.codigo})
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Revision {
    pub hallazgos: Vec<Hallazgo>,
    /// los que no se pudieron ubicar (archivo o línea inexistentes), con el porqué
    pub descartados: Vec<String>,
    pub sin_json: bool,
}

impl Revision {
    /// La tarjeta para la ventana (`{"kind": "revision", …}`).
    pub fn tarjeta(&self, tipo: Tipo) -> Value {
        json!({
            "kind": "revision",
            "tipo": if tipo == Tipo::Seguridad { "seguridad" } else { "codigo" },
            "hallazgos": self.hallazgos.iter().map(Hallazgo::json).collect::<Vec<_>>(),
            "descartados": self.descartados,
            "sin_json": self.sin_json,
        })
    }

    /// El informe en texto (CLI, TUI, y lo que queda en el chat).
    pub fn informe(&self) -> String {
        let mut s = if self.hallazgos.is_empty() {
            crate::i18n::t("✅ Revisión: sin hallazgos.", "✅ Review: no findings.").to_string()
        } else {
            crate::tr!("🔎 Revisión: {} hallazgo(s)", "🔎 Review: {} finding(s)", self.hallazgos.len())
        };
        for h in &self.hallazgos {
            s.push_str(&format!("\n{}. [{}] {}:{} — {}\n   {}: {}", h.id, h.severidad, h.archivo, h.linea, h.resumen, crate::i18n::t("cómo falla", "how it fails"), h.como_falla));
        }
        if !self.descartados.is_empty() {
            s.push_str(&crate::tr!("\n⚠ {} descartado(s) por no poder ubicarlos: {}", "\n⚠ {} discarded because they could not be located: {}", self.descartados.len(), self.descartados.join("; ")));
        }
        if self.sin_json {
            s.push_str(crate::i18n::t("\n⚠ el modelo no devolvió la lista de hallazgos en JSON", "\n⚠ the model did not return the findings list as JSON"));
        }
        s
    }
}

/// `seguridad` / `security` al principio del argumento → revisión de seguridad.
pub fn tipo_de(arg: &str) -> (Tipo, &str) {
    let a = arg.trim();
    for p in ["seguridad", "security"] {
        if let Some(r) = a.strip_prefix(p) {
            if r.is_empty() || r.starts_with(' ') {
                return (Tipo::Seguridad, r.trim());
            }
        }
    }
    (Tipo::Codigo, a)
}

fn git(root: &Path, args: &[&str]) -> Result<String> {
    let o = crate::entorno::comando("git").arg("-C").arg(root).args(args).stdin(std::process::Stdio::null()).output()?;
    if !o.status.success() {
        bail!("git {}: {}", args.join(" "), String::from_utf8_lossy(&o.stderr).trim());
    }
    Ok(String::from_utf8_lossy(&o.stdout).to_string())
}

/// Lo que se revisa: el diff (con un tope) y una línea que dice qué es.
pub fn que_revisar(root: &Path, arg: &str) -> Result<(String, String)> {
    let partes: Vec<&str> = arg.split_whitespace().collect();
    let (diff, de) = if let Some(i) = partes.iter().position(|p| *p == "--rama" || *p == "--branch") {
        let base = partes.get(i + 1).copied().unwrap_or("main");
        if base.starts_with('-') {
            bail!("{}", crate::i18n::t("base de rama inválida", "invalid branch base"));
        }
        (git(root, &["diff", "--no-color", "--no-ext-diff", &format!("{base}...HEAD")])?, crate::tr!("lo de esta rama frente a {base}", "this branch against {base}"))
    } else {
        let ruta: Vec<&str> = partes.iter().copied().filter(|p| !p.starts_with('-')).collect();
        let mut args = vec!["diff", "--no-color", "--no-ext-diff", "HEAD"];
        if !ruta.is_empty() {
            args.push("--");
            args.extend(ruta.iter().copied());
        }
        let mut d = git(root, &args)?;
        // los archivos nuevos (aún sin git add): su contenido entero, como un diff
        let mut nuevos_args = vec!["ls-files", "--others", "--exclude-standard"];
        if !ruta.is_empty() {
            nuevos_args.push("--");
            nuevos_args.extend(ruta.iter().copied());
        }
        for f in git(root, &nuevos_args)?.lines().filter(|l| !l.trim().is_empty()) {
            if let Ok(t) = std::fs::read_to_string(root.join(f)) {
                d.push_str(&format!("\n--- /dev/null\n+++ b/{f}\n"));
                for (i, l) in t.lines().enumerate() {
                    d.push_str(&format!("+{l}    ⟵{}\n", i + 1));
                }
            }
        }
        let de = if ruta.is_empty() { crate::i18n::t("los cambios sin confirmar", "the uncommitted changes").to_string() } else { crate::tr!("los cambios en {}", "the changes in {}", ruta.join(" ")) };
        (d, de)
    };
    if diff.trim().is_empty() {
        bail!("{}", crate::tr!("no hay cambios que revisar ({de})", "there are no changes to review ({de})"));
    }
    let mut d: String = diff.chars().take(DIFF_MAX).collect();
    if d.len() < diff.len() {
        d.push_str(crate::i18n::t("\n[… diff recortado: mira el resto con read_file]", "\n[… diff truncated: look at the rest with read_file]"));
    }
    Ok((d, de))
}

/// La orden del turno de revisión.
pub fn orden(tipo: Tipo, diff: &str, de: &str) -> String {
    let skill = match tipo {
        Tipo::Codigo => crate::i18n::t(es::CODIGO, en::CODIGO),
        Tipo::Seguridad => crate::i18n::t(es::SEGURIDAD, en::SEGURIDAD),
    };
    crate::tr!(
        "{skill}\n\n# Qué revisar: {de}\n```diff\n{diff}\n```",
        "{skill}\n\n# What to review: {de}\n```diff\n{diff}\n```"
    )
}

/// Ruta relativa dentro del proyecto, sin salir de él (sin `..` ni absoluta fuera).
fn dentro(root: &Path, archivo: &str) -> Option<PathBuf> {
    let a = archivo.trim().trim_start_matches("./").trim_start_matches("a/").trim_start_matches("b/");
    let p = Path::new(a);
    let rel = if p.is_absolute() { p.strip_prefix(root).ok()?.to_path_buf() } else { p.to_path_buf() };
    if rel.components().any(|c| !matches!(c, Component::Normal(_))) {
        return None;
    }
    let completo = root.join(&rel);
    // un enlace que apunta fuera tampoco vale
    let real = completo.canonicalize().ok()?;
    real.starts_with(root.canonicalize().ok()?).then_some(rel)
}

/// Valida lo que devolvió el modelo contra el disco.
pub fn validar(root: &Path, respuesta: &str) -> Revision {
    let Some(v) = crate::formas::objeto_json(respuesta) else { return Revision { sin_json: true, ..Default::default() } };
    let mut r = Revision::default();
    for (i, h) in v["hallazgos"].as_array().or(v["findings"].as_array()).into_iter().flatten().enumerate() {
        // juez-idioma: datos (claves fijas del JSON que devuelve el modelo)
        let archivo =h["archivo"].as_str().or(h["file"].as_str()).unwrap_or("").to_string();
        let linea = h["linea"].as_u64().or(h["line"].as_u64()).or_else(|| h["linea"].as_str().and_then(|s| s.parse().ok())).unwrap_or(0) as u32;
        let resumen = h["resumen"].as_str().or(h["summary"].as_str()).unwrap_or("").trim().to_string();
        let como = h["como_falla"].as_str().or(h["failure"].as_str()).unwrap_or("").trim().to_string();
        let sev = h["severidad"].as_str().or(h["severity"].as_str()).unwrap_or("media").to_string();
        let Some(rel) = dentro(root, &archivo) else {
            r.descartados.push(crate::tr!("{archivo}: no existe en el proyecto", "{archivo}: does not exist in the project"));
            continue;
        };
        let texto = std::fs::read_to_string(root.join(&rel)).unwrap_or_default();
        let Some(codigo) = (linea >= 1).then(|| texto.lines().nth(linea as usize - 1)).flatten() else {
            r.descartados.push(crate::tr!("{archivo}:{linea}: esa línea no existe ({} líneas)", "{archivo}:{linea}: that line does not exist ({} lines)", texto.lines().count()));
            continue;
        };
        if resumen.is_empty() {
            r.descartados.push(crate::tr!("{archivo}:{linea}: sin resumen", "{archivo}:{linea}: no summary"));
            continue;
        }
        r.hallazgos.push(Hallazgo { id: (i + 1) as u32, archivo: rel.display().to_string(), linea, resumen, como_falla: como, severidad: sev, codigo: codigo.trim_end().chars().take(300).collect() });
    }
    // los ids, seguidos tras descartar
    for (i, h) in r.hallazgos.iter_mut().enumerate() {
        h.id = (i + 1) as u32;
    }
    r
}

fn guardadas() -> &'static Mutex<HashMap<String, Revision>> {
    static G: OnceLock<Mutex<HashMap<String, Revision>>> = OnceLock::new();
    G.get_or_init(Default::default)
}

/// La última revisión del chat (para «arreglar éste»).
pub fn guardar(dueno: &str, r: &Revision) {
    guardadas().lock().unwrap().insert(dueno.into(), r.clone());
}

/// La orden de «arreglar éste» para el hallazgo `id` de la última revisión del chat.
pub fn orden_arreglar(dueno: &str, id: u32) -> Option<(String, String)> {
    let g = guardadas().lock().unwrap();
    let h = g.get(dueno)?.hallazgos.iter().find(|h| h.id == id)?.clone();
    let orden = crate::tr!(
        "Arregla este hallazgo de la revisión, y sólo éste:\n- {}:{} — {}\n- Cómo falla: {}\n- La línea hoy: `{}`\nHaz el cambio mínimo que lo corrige, sin tocar nada más, y compruébalo (pruebas o ejecutándolo). Si al mirarlo resulta que no es un fallo, dilo y no cambies nada.",
        "Fix this review finding, and only this one:\n- {}:{} — {}\n- How it fails: {}\n- The line today: `{}`\nMake the minimal change that fixes it, touching nothing else, and verify it (tests or running it). If on inspection it is not a bug, say so and change nothing.",
        h.archivo,
        h.linea,
        h.resumen,
        h.como_falla,
        h.codigo
    );
    let visible = crate::tr!("🔧 arreglar el hallazgo {} ({}:{})", "🔧 fix finding {} ({}:{})", h.id, h.archivo, h.linea);
    Some((orden, visible))
}

/// Pone el turno en sólo lectura y devuelve lo que había (para [`restaurar`]).
pub fn solo_lectura<E: Engine, F: Frontend>(a: &mut Agent<E, F>) -> Option<Vec<String>> {
    std::mem::replace(&mut a.perms.solo_herramientas, Some(SOLO_LECTURA.iter().map(|s| s.to_string()).collect()))
}

pub fn restaurar<E: Engine, F: Frontend>(a: &mut Agent<E, F>, antes: Option<Vec<String>>) {
    a.perms.solo_herramientas = antes;
}

/// `/revisar` entero (CLI, TUI y pruebas; la ventana usa las piezas para que sea un turno suyo).
pub async fn revisar<E: Engine, F: Frontend>(a: &mut Agent<E, F>, arg: &str) -> Result<(Tipo, Revision)> {
    let (tipo, resto) = tipo_de(arg);
    let (diff, de) = que_revisar(&a.root, resto)?;
    let antes = solo_lectura(a);
    let r = match a.run(&orden(tipo, &diff, &de)).await {
        Ok(t) => {
            let mut rev = validar(&a.root, &t);
            if rev.sin_json {
                // aún en sólo lectura: el trabajo ya está hecho, falta el formato
                if let Ok(t2) = a.run(&orden_solo_json()).await {
                    rev = validar(&a.root, &t2);
                }
            }
            Ok(rev)
        }
        Err(e) => Err(e),
    };
    restaurar(a, antes);
    let rev = r?;
    guardar(&a.proc_owner(), &rev);
    Ok((tipo, rev))
}

/// Cuando la revisión terminó en prosa sin el objeto JSON (medido 2026-11-18 con Huihui: halló el fallo
/// correcto y lo contó, pero sin el JSON la tarjeta salía vacía): se le pide UNA vez, sólo el JSON.
pub fn orden_solo_json() -> String {
    crate::tr!(
        "Te faltó el objeto JSON final de la revisión. Devuélvelo ahora con lo que ya encontraste, sin volver a revisar y sin nada más: {}",
        "You left out the final JSON object of the review. Return it now with what you already found, without reviewing again and with nothing else: {}",
        "{\"hallazgos\": [{\"archivo\": \"ruta/relativa\", \"linea\": 12, \"severidad\": \"alta|media|baja\", \"resumen\": \"…\", \"como_falla\": \"…\"}]}"
    )
}

mod es {
    // juez-idioma: par (rama ES; su gemela inglesa en `en`) — bloque
    pub const CODIGO: &str = "# Revisión de código (GOTV)\n\
        Revisa los cambios de abajo buscando FALLOS reales: lógica equivocada, casos límite (vacío, cero, negativo, None/null, desbordes), errores que se tragan, condiciones de carrera, recursos que no se cierran, cambios que rompen a quien llama, pruebas que no prueban lo que dicen.\n\
        - Mira el código de alrededor con read_file/grep cuando haga falta: un hallazgo sin comprobar no vale.\n\
        - Sólo lo que de verdad falla, con un caso concreto. Nada de estilo, nombres ni gustos.\n\
        - NO cambies archivos: en este turno sólo se mira (GOTV no te deja escribir).\n\
        Termina con UN objeto JSON y nada después:\n\
        {\"hallazgos\": [{\"archivo\": \"ruta/relativa\", \"linea\": 12, \"severidad\": \"alta|media|baja\", \"resumen\": \"qué está mal, en una línea\", \"como_falla\": \"la entrada o el estado concreto → lo que pasa\"}]}\n\
        `linea` es la del archivo tal como está AHORA en el disco. Si no hay fallos: {\"hallazgos\": []}.";
    pub const SEGURIDAD: &str = "# Revisión de seguridad (GOTV)\n\
        Revisa los cambios de abajo buscando VULNERABILIDADES reales: inyección (SQL, órdenes de la shell, rutas con ..), entrada sin validar que llega a algo peligroso, secretos en el código o en registros, autenticación o permisos que se saltan, deserialización insegura, SSRF, XSS, criptografía mal usada, dependencias inventadas.\n\
        - Sigue el dato desde donde entra hasta donde hace daño (read_file/grep): un hallazgo sin ese camino no vale.\n\
        - Sólo lo explotable, con el ataque concreto. Nada de «buenas prácticas» genéricas.\n\
        - NO cambies archivos: en este turno sólo se mira (GOTV no te deja escribir).\n\
        Termina con UN objeto JSON y nada después:\n\
        {\"hallazgos\": [{\"archivo\": \"ruta/relativa\", \"linea\": 12, \"severidad\": \"alta|media|baja\", \"resumen\": \"la vulnerabilidad, en una línea\", \"como_falla\": \"el ataque concreto → lo que consigue\"}]}\n\
        `linea` es la del archivo tal como está AHORA en el disco. Si no hay nada: {\"hallazgos\": []}.";
}

mod en {
    // juez-idioma: datos (rama EN; `hallazgos`, `archivo`, `linea`… son las claves fijas del JSON) — bloque
    pub const CODIGO: &str = "# Code review (GOTV)\n\
        Review the changes below looking for REAL bugs: wrong logic, edge cases (empty, zero, negative, None/null, overflow), swallowed errors, race conditions, resources not closed, changes that break callers, tests that do not test what they claim.\n\
        - Look at the surrounding code with read_file/grep when needed: an unverified finding is worthless.\n\
        - Only what really fails, with a concrete case. No style, naming or taste.\n\
        - Do NOT change files: this turn only looks (GOTV will not let you write).\n\
        End with ONE JSON object and nothing after it:\n\
        {\"hallazgos\": [{\"archivo\": \"relative/path\", \"linea\": 12, \"severidad\": \"alta|media|baja\", \"resumen\": \"what is wrong, in one line\", \"como_falla\": \"the concrete input or state → what happens\"}]}\n\
        `linea` is the line of the file as it is NOW on disk. If there are no bugs: {\"hallazgos\": []}.";
    pub const SEGURIDAD: &str = "# Security review (GOTV)\n\
        Review the changes below looking for REAL vulnerabilities: injection (SQL, shell commands, paths with ..), unvalidated input reaching something dangerous, secrets in code or logs, authentication or permissions that can be bypassed, unsafe deserialization, SSRF, XSS, misused cryptography, invented dependencies.\n\
        - Follow the data from where it enters to where it does harm (read_file/grep): a finding without that path is worthless.\n\
        - Only what is exploitable, with the concrete attack. No generic «best practices».\n\
        - Do NOT change files: this turn only looks (GOTV will not let you write).\n\
        End with ONE JSON object and nothing after it:\n\
        {\"hallazgos\": [{\"archivo\": \"relative/path\", \"linea\": 12, \"severidad\": \"alta|media|baja\", \"resumen\": \"the vulnerability, in one line\", \"como_falla\": \"the concrete attack → what it achieves\"}]}\n\
        `linea` is the line of the file as it is NOW on disk. If there is nothing: {\"hallazgos\": []}.";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn el_tipo_se_lee_en_los_dos_idiomas() {
        assert_eq!(tipo_de("seguridad src/"), (Tipo::Seguridad, "src/"));
        assert_eq!(tipo_de("security"), (Tipo::Seguridad, ""));
        assert_eq!(tipo_de("seguridadx"), (Tipo::Codigo, "seguridadx"));
        assert_eq!(tipo_de("--rama main"), (Tipo::Codigo, "--rama main"));
    }

    #[test]
    fn valida_contra_el_disco() {
        let d = crate::CarpetaPrueba::nueva("gotv-revisar-validar");
        std::fs::create_dir_all(d.join("src")).unwrap();
        std::fs::write(d.join("src/lib.rs"), "pub fn mitad(x: i32) -> i32 {\n    x / 0\n}\n").unwrap();
        let resp = r#"Revisé. {"hallazgos": [
            {"archivo": "src/lib.rs", "linea": 2, "severidad": "alta", "resumen": "divide entre cero", "como_falla": "mitad(4) → pánico"},
            {"archivo": "src/no_existe.rs", "linea": 3, "resumen": "inventado", "como_falla": "-"},
            {"archivo": "src/lib.rs", "linea": 40, "resumen": "línea inventada", "como_falla": "-"},
            {"archivo": "../fuera.rs", "linea": 1, "resumen": "fuera", "como_falla": "-"}]}"#;
        let r = validar(&d, resp);
        assert_eq!(r.hallazgos.len(), 1);
        let h = &r.hallazgos[0];
        assert_eq!((h.id, h.archivo.as_str(), h.linea, h.codigo.as_str()), (1, "src/lib.rs", 2, "    x / 0"));
        assert_eq!(r.descartados.len(), 3, "{:?}", r.descartados);
        assert!(validar(&d, "no hay json").sin_json);
        let t = r.tarjeta(Tipo::Codigo);
        assert_eq!(t["hallazgos"][0]["codigo"], "    x / 0");
    }
}
