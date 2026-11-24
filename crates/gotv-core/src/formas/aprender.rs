//! F12 · paso 5 · `/aprender`: la skill incluida `aprender` (en `incluidas.rs`) lee las correcciones del
//! Portador en la conversación y PROPONE una regla, una línea de memoria o una skill; GOTV enseña la
//! diferencia exacta y sólo escribe si el Portador la aprueba (trampa 4).
//!
//! La aprobación la impone el CÓDIGO, no la skill: el modelo sólo devuelve un JSON; quien escribe es GOTV,
//! y únicamente dentro de `.gotv/` del proyecto (`reglas/<nombre>.md`, `GOTV.md`, `skills/<nombre>/SKILL.md`).
//! Nunca `AGENTS.md` ni `CLAUDE.md`, que pueden ser de un repositorio que no es del Portador.
//! Por modo: Plan → sólo propone; Bypass (control total, decisión del Portador) → escribe sin preguntar y lo
//! dice; los demás (también «aceptar ediciones» y Auto: cambia cómo trabaja el agente, no el código) → pregunta.
use crate::frontend::Frontend;
use crate::permissions::{AskAnswer, Mode};
use crate::Agent;
use anyhow::Result;
use gotv_engine::Engine;
use std::path::{Path, PathBuf};

/// Lo que escribe la propuesta.
const APRENDER_MAX_TOKENS: u32 = 1_200;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tipo {
    Regla,
    Memoria,
    Skill,
    Nada,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Propuesta {
    pub tipo: Tipo,
    pub nombre: String,
    pub rutas: Vec<String>,
    pub descripcion: String,
    pub texto: String,
    pub porque: String,
}

/// Un nombre de archivo seguro: minúsculas ASCII, dígitos y guiones (nada de `/` ni `..`).
fn nombre_seguro(s: &str, si_vacio: &str) -> String {
    let mut n = String::new();
    for c in s.trim().to_lowercase().chars() {
        let c = match c {
            'á' | 'à' | 'ä' => 'a',
            'é' | 'è' | 'ë' => 'e',
            'í' | 'ì' | 'ï' => 'i',
            'ó' | 'ò' | 'ö' => 'o',
            'ú' | 'ù' | 'ü' => 'u',
            'ñ' => 'n',
            c => c,
        };
        if c.is_ascii_alphanumeric() {
            n.push(c);
        } else if !n.ends_with('-') && !n.is_empty() {
            n.push('-');
        }
    }
    let n: String = n.trim_matches('-').chars().take(40).collect();
    let n = n.trim_end_matches('-').to_string();
    if n.is_empty() {
        si_vacio.to_string()
    } else {
        n
    }
}

/// La propuesta que devolvió el modelo. Una `regla` sin rutas vale siempre: es `memoria`.
pub fn leer_propuesta(respuesta: &str) -> Result<Propuesta> {
    let v = super::objeto_json(respuesta).ok_or_else(|| {
        let t: String = super::sin_pensamiento(respuesta).chars().take(160).collect();
        gotv_engine::anyhow_tr!("el modelo no devolvió la propuesta en JSON: «{t}»", "the model did not return the proposal as JSON: «{t}»")
    })?;
    let s = |k: &str| v[k].as_str().unwrap_or("").trim().to_string();
    let rutas: Vec<String> = match &v["rutas"] {
        serde_json::Value::Array(a) => a.iter().filter_map(|x| x.as_str()).map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect(),
        serde_json::Value::String(x) => x.split(',').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect(),
        _ => vec![],
    };
    let mut tipo = match s("tipo").to_lowercase().as_str() {
        // juez-idioma: datos (los valores fijos del JSON de la skill, y sus equivalentes en inglés)
        "regla" | "rule" => Tipo::Regla,
        "memoria" | "memory" => Tipo::Memoria,
        "skill" => Tipo::Skill,
        "nada" | "none" | "nothing" => Tipo::Nada,
        otro => gotv_engine::bail_tr!("tipo desconocido en la propuesta: «{otro}»", "unknown type in the proposal: «{otro}»"),
    };
    if tipo == Tipo::Regla && rutas.is_empty() {
        tipo = Tipo::Memoria;
    }
    let texto = s("texto");
    if tipo != Tipo::Nada && texto.is_empty() {
        gotv_engine::bail_tr!("la propuesta no trae `texto`", "the proposal has no `texto`");
    }
    Ok(Propuesta { tipo, nombre: s("nombre"), rutas, descripcion: s("descripcion"), texto, porque: s("porque") })
}

/// Una ruta libre: `base`, o `base-2`, `base-3`… (nunca se pisa una regla o skill que ya existe).
fn libre(hacer: impl Fn(&str) -> PathBuf, base: &str) -> PathBuf {
    let p = hacer(base);
    if !p.exists() {
        return p;
    }
    (2..100).map(|i| hacer(&format!("{base}-{i}"))).find(|p| !p.exists()).unwrap_or(p)
}

/// Dónde se escribe y el contenido de antes y de después.
pub fn destino(root: &Path, p: &Propuesta) -> Option<(PathBuf, String, String)> {
    let comillas = |r: &str| format!("\"{}\"", r.replace('\\', "\\\\").replace('"', "\\\""));
    match p.tipo {
        Tipo::Nada => None,
        Tipo::Regla => {
            let base = nombre_seguro(&p.nombre, "regla");
            let ruta = libre(|n| root.join(".gotv/reglas").join(format!("{n}.md")), &base);
            let rutas: Vec<String> = p.rutas.iter().map(|r| comillas(r)).collect();
            Some((ruta, String::new(), format!("---\nrutas: [{}]\n---\n{}\n", rutas.join(", "), p.texto.trim())))
        }
        Tipo::Memoria => {
            let ruta = root.join(".gotv/GOTV.md");
            let antes = std::fs::read_to_string(&ruta).unwrap_or_default();
            let mut despues = antes.clone();
            if !despues.is_empty() {
                if !despues.ends_with('\n') {
                    despues.push('\n');
                }
                despues.push('\n');
            }
            despues.push_str(p.texto.trim());
            despues.push('\n');
            Some((ruta, antes, despues))
        }
        Tipo::Skill => {
            let base = nombre_seguro(&p.nombre, "skill");
            let dir = libre(|n| root.join(".gotv/skills").join(n), &base);
            let nombre = dir.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or(base);
            let desc = if p.descripcion.is_empty() { p.porque.lines().next().unwrap_or("").to_string() } else { p.descripcion.clone() };
            Some((dir.join("SKILL.md"), String::new(), format!("---\nname: {nombre}\ndescription: {}\n---\n\n{}\n", desc.replace('\n', " "), p.texto.trim())))
        }
    }
}

/// La diferencia en formato unificado (`-` lo que se va, `+` lo que llega).
pub fn diferencia(antes: &str, despues: &str) -> String {
    let d = similar::TextDiff::from_lines(antes, despues);
    let mut s = String::new();
    for c in d.iter_all_changes() {
        let signo = match c.tag() {
            similar::ChangeTag::Delete => '-',
            similar::ChangeTag::Insert => '+',
            similar::ChangeTag::Equal => ' ',
        };
        s.push(signo);
        s.push_str(c.value().trim_end_matches('\n'));
        s.push('\n');
    }
    s
}

fn nombre_del_tipo(t: Tipo) -> &'static str {
    match t {
        Tipo::Regla => crate::i18n::t("regla por ruta", "path rule"),
        Tipo::Memoria => crate::i18n::t("memoria del proyecto", "project memory"),
        Tipo::Skill => "skill",
        Tipo::Nada => "—",
    }
}

/// Escribe `despues` en `ruta`. (Los puntos de restauración excluyen `.gotv/` por diseño, así que `/rewind`
/// no lo deshace: el informe dice cómo.)
fn escribir(ruta: &Path, despues: &str) -> Result<()> {
    if let Some(d) = ruta.parent() {
        std::fs::create_dir_all(d)?;
    }
    std::fs::write(ruta, despues)?;
    Ok(())
}

/// `/aprender`: propone con la skill y escribe sólo con la aprobación del Portador.
pub async fn aprender<E: Engine, F: Frontend>(agent: &mut Agent<E, F>) -> Result<String> {
    if !agent.history.iter().any(|m| m.role == "user" && !m.es_aviso()) {
        return Ok(crate::i18n::t("Aún no hay conversación de la que aprender: /aprender repasa tus correcciones de ESTA conversación.", "There is no conversation to learn from yet: /learn reviews your corrections in THIS conversation.").to_string());
    }
    agent.front.activity("step", crate::i18n::t("repasando tus correcciones", "reviewing your corrections"));
    let pedido = format!(
        "{}\n\n{}",
        super::incluidas::cuerpo("aprender").unwrap_or(""),
        crate::i18n::t("(Pedido por el usuario con /aprender. Responde AHORA sólo el objeto JSON.)", "(Requested by the user with /learn. Answer NOW with the JSON object only.)")
    );
    let r = super::generar_aparte(agent, pedido, APRENDER_MAX_TOKENS).await?;
    let p = leer_propuesta(&r.content)?;
    let Some((ruta, antes, despues)) = destino(&agent.root, &p) else {
        return Ok(crate::tr!("No vi correcciones que convertir en regla. {}", "I saw no corrections to turn into a rule. {}", p.porque));
    };
    let rel = ruta.strip_prefix(&agent.root).unwrap_or(&ruta).display().to_string();
    let propuesta = crate::tr!(
        "📚 /aprender propone una {} en {rel}\nPor qué: {}\n```diff\n{}```",
        "📚 /learn proposes a {} in {rel}\nWhy: {}\n```diff\n{}```",
        nombre_del_tipo(p.tipo),
        if p.porque.is_empty() { "—" } else { &p.porque },
        diferencia(&antes, &despues)
    );
    let escribe = match agent.perms.mode {
        Mode::Plan => return Ok(crate::tr!("{propuesta}\n(Modo plan: no se escribe nada.)", "{propuesta}\n(Plan mode: nothing is written.)")),
        Mode::Bypass => true,
        _ => matches!(agent.front.ask(&propuesta).await, AskAnswer::Yes | AskAnswer::Always),
    };
    if !escribe {
        return Ok(crate::tr!("{propuesta}\nNo se escribió: no la aprobaste.", "{propuesta}\nNot written: you did not approve it."));
    }
    escribir(&ruta, &despues)?;
    let (cuando, deshacer) = match p.tipo {
        Tipo::Regla => (crate::i18n::t("se anunciará al tocar un archivo que case", "it will be announced when a matching file is touched"), crate::i18n::t("borra ese archivo", "delete that file")),
        Tipo::Memoria => (
            crate::i18n::t("va en la memoria del proyecto desde la próxima orden (cambia el mensaje de sistema: el motor relee la conversación una vez)", "it goes in the project memory from the next order (it changes the system message: the engine rereads the conversation once)"),
            crate::i18n::t("quita ese párrafo del archivo", "remove that paragraph from the file"),
        ),
        _ => (crate::i18n::t("se carga con la herramienta `skill` o con /<nombre>", "it is loaded with the `skill` tool or with /<name>"), crate::i18n::t("borra su carpeta", "delete its folder")),
    };
    let modo = if agent.perms.mode == Mode::Bypass { crate::i18n::t(" (modo bypass: sin preguntar)", " (bypass mode: without asking)") } else { "" };
    Ok(crate::tr!("{propuesta}\n✅ Escrito en {rel}{modo}: {cuando}. Para deshacerlo, {deshacer} (lo de .gotv/ queda fuera de /rewind).", "{propuesta}\n✅ Written to {rel}{modo}: {cuando}. To undo it, {deshacer} (.gotv/ is outside /rewind)."))
}

#[cfg(test)]
mod tests {
    use super::super::prueba::{agente, texto};
    use super::*;

    const REGLA: &str = r#"{"tipo": "regla", "nombre": "Migraciones SQL", "rutas": ["**/*.sql"], "texto": "Nunca edites una migración aplicada: crea otra.", "porque": "«no toques 0001, crea una nueva»"}"#;

    fn con_correccion(tag: &str, modo: Mode, aparte: Vec<&str>) -> super::super::prueba::ConCarpeta {
        agente(tag, modo, vec![texto("Edité 0001_init.sql."), texto("Creé 0002_fix.sql.")], aparte)
    }

    async fn conversacion(c: &mut super::super::prueba::ConCarpeta) {
        c.agente.run("añade la columna email").await.unwrap();
        c.agente.run("no toques 0001, crea una nueva migración").await.unwrap();
    }

    #[test]
    fn la_propuesta_se_lee_y_se_limpia() {
        let p = leer_propuesta(&format!("Aquí va:\n```json\n{REGLA}\n```")).unwrap();
        assert_eq!((p.tipo, p.nombre.as_str(), p.rutas.as_slice()), (Tipo::Regla, "Migraciones SQL", &["**/*.sql".to_string()][..]));
        // una regla sin rutas vale siempre: memoria
        assert_eq!(leer_propuesta(r#"{"tipo": "regla", "texto": "x"}"#).unwrap().tipo, Tipo::Memoria);
        assert!(leer_propuesta("no hay JSON").is_err() && leer_propuesta(r#"{"tipo": "otra"}"#).is_err() && leer_propuesta(r#"{"tipo": "skill"}"#).is_err());
        assert_eq!(leer_propuesta(r#"{"tipo": "nada", "porque": "sin correcciones"}"#).unwrap().tipo, Tipo::Nada);
        // nombres sin trampas de ruta
        assert_eq!(nombre_seguro("../../AGENTS", "x"), "agents");
        assert_eq!(nombre_seguro("Migración ñandú", "x"), "migracion-nandu");
        assert_eq!(nombre_seguro("///", "regla"), "regla");
    }

    #[test]
    fn el_destino_vive_en_gotv_y_nunca_pisa() {
        let d = crate::CarpetaPrueba::nueva("gotv-aprender-destino");
        let p = leer_propuesta(REGLA).unwrap();
        let (ruta, antes, despues) = destino(&d, &p).unwrap();
        assert_eq!(ruta, d.join(".gotv/reglas/migraciones-sql.md"));
        assert!(antes.is_empty() && despues.starts_with("---\nrutas: [\"**/*.sql\"]\n---\nNunca edites"));
        // la regla escrita se lee como regla por ruta
        std::fs::create_dir_all(ruta.parent().unwrap()).unwrap();
        std::fs::write(&ruta, &despues).unwrap();
        let r = crate::formas::reglas::cabeceras(&d);
        assert!(r.len() == 1 && r[0].casa("db/0002.sql"));
        // otra con el mismo nombre no pisa la anterior
        assert_eq!(destino(&d, &p).unwrap().0, d.join(".gotv/reglas/migraciones-sql-2.md"));
        // memoria: se AÑADE a .gotv/GOTV.md
        std::fs::write(d.join(".gotv/GOTV.md"), "Usa tabs.").unwrap();
        let m = leer_propuesta(r#"{"tipo": "memoria", "texto": "Responde en español."}"#).unwrap();
        let (ruta, antes, despues) = destino(&d, &m).unwrap();
        assert_eq!((ruta, antes.as_str(), despues.as_str()), (d.join(".gotv/GOTV.md"), "Usa tabs.", "Usa tabs.\n\nResponde en español.\n"));
        assert_eq!(diferencia("a\nb\n", "a\nc\n"), " a\n-b\n+c\n");
    }

    /// La trampa 4: propone y sólo escribe con la aprobación del Portador. Sin aprobar no hay archivo; la
    /// pregunta lleva la diferencia exacta; el historial no cambia (la propuesta se pide aparte).
    #[tokio::test]
    async fn propone_y_solo_escribe_si_el_portador_aprueba() {
        let mut c = con_correccion("aprender-no", Mode::Default, vec![REGLA, REGLA]);
        conversacion(&mut c).await;
        let antes = super::super::prueba::foto(&c.agente);
        let r = aprender(&mut c.agente).await.unwrap();
        assert!(r.contains("No se escribió") || r.contains("Not written"), "{r}");
        assert!(!c.agente.root.join(".gotv/reglas").exists(), "sin aprobación no hay archivo");
        let pregunta = c.agente.front.preguntas.lock().unwrap()[0].clone();
        assert!(pregunta.contains("+rutas: [\"**/*.sql\"]") && pregunta.contains("+Nunca edites una migración aplicada"), "{pregunta}");
        assert_eq!(super::super::prueba::foto(&c.agente), antes, "/aprender no entra al historial");
        // aprobada: se escribe y queda como regla por ruta
        *c.agente.front.respuesta.lock().unwrap() = Some(AskAnswer::Yes);
        let r = aprender(&mut c.agente).await.unwrap();
        assert!(r.contains("✅") && r.contains("borra ese archivo"), "{r}");
        assert!(crate::formas::reglas::cabeceras(&c.agente.root).iter().any(|x| x.archivo == ".gotv/reglas/migraciones-sql.md" && x.casa("db/0002.sql")));
        // lo que pidió: la skill entera y sus correcciones a la vista
        let visto = c.agente.engine.visto.lock().unwrap();
        let u = &visto.last().unwrap().messages;
        assert!(u.last().unwrap().content.contains("Nunca `AGENTS.md`") && u.iter().any(|m| m.content.contains("no toques 0001")));
    }

    #[tokio::test]
    async fn en_plan_solo_propone_y_en_bypass_escribe_sin_preguntar() {
        let mut c = con_correccion("aprender-plan", Mode::Plan, vec![REGLA]);
        conversacion(&mut c).await;
        let r = aprender(&mut c.agente).await.unwrap();
        assert!(r.contains("plan") && !c.agente.root.join(".gotv").join("reglas").exists(), "{r}");
        let mut c = con_correccion("aprender-bypass", Mode::Bypass, vec![REGLA]);
        conversacion(&mut c).await;
        let r = aprender(&mut c.agente).await.unwrap();
        assert!(r.contains("bypass") && c.agente.root.join(".gotv/reglas/migraciones-sql.md").is_file(), "{r}");
        assert!(c.agente.front.preguntas.lock().unwrap().is_empty(), "en bypass, ninguna pregunta");
        // sin conversación no hay nada que aprender (y no se llama al modelo)
        let mut v = agente("aprender-vacio", Mode::Default, vec![], vec![]);
        assert!(aprender(&mut v.agente).await.unwrap().contains("/"));
        assert!(v.agente.engine.visto.lock().unwrap().is_empty());
    }
}
