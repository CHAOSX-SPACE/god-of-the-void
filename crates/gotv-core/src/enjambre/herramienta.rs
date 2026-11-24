//! F9 · la herramienta `enjambre` (diferida): lo que comparte un equipo de agentes.
//! - `anotar` / `tablero`: el tablero común (escribir una nota, leer desde la n).
//! - `fusionar`: `git merge` de las ramas que se nombran; lo que choca se reporta sin fusionar.
//! - `verificar`: el verificador (lo corre `agent/paralelo.rs`, que tiene el bucle del agente).
//! - `sesiones` / `mensaje`: las sesiones de GOTV de este proyecto y un mensaje a otra (lo ve entre pasos).

use super::{buzon, ramas, tablero};
use anyhow::{anyhow, bail, Result};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

/// Lo que la herramienta necesita saber de quien la llama.
pub struct Contexto {
    /// la raíz del proyecto del agente que llama (un subagente: su rama)
    pub raiz: PathBuf,
    /// el tablero del equipo
    pub tablero: PathBuf,
    /// su nombre en el tablero
    pub yo: String,
    /// ¿es un subagente? (no fusiona ni escribe a otras sesiones: eso es del principal)
    pub subagente: bool,
    /// su sesión (para firmar mensajes)
    pub sesion: Option<i64>,
    /// modo plan: sólo mira
    pub plan: bool,
}

pub fn esquema() -> Value {
    json!({"type": "function", "function": {
        "name": "enjambre",
        "description": crate::i18n::t(
            "Trabajo en EQUIPO de agentes (varios `task` en un mismo turno corren a la vez, cada uno en su rama). accion: «anotar» (texto: una nota corta en el tablero común del equipo), «tablero» (leerlo; desde: la última nota que ya viste), «fusionar» (ramas: git merge de esas ramas con su commit; lo que choca se reporta y su rama queda), «verificar» (criterio: otra conversación —en otro modelo si das modelo `proveedor:modelo`— corre las pruebas SIN editar y da un veredicto), «sesiones» (las sesiones de GOTV de este proyecto), «mensaje» (para: número de sesión, texto: lo que le dices; la otra sesión lo ve entre pasos).",
            "Work in a TEAM of agents (several `task` calls in one turn run at once, each on its own branch). accion: «anotar» (texto: a short note on the team's shared board), «tablero» (read it; desde: the last note you already saw), «fusionar» (ramas: git merge of those branches with its commit; what conflicts is reported and its branch stays), «verificar» (criterio: another conversation —on another model if you give modelo `provider:model`— runs the tests WITHOUT editing and gives a verdict), «sesiones» (this project's GOTV sessions), «mensaje» (para: session number, texto: what you tell it; the other session sees it between steps)."),
        "parameters": {"type": "object", "properties": {
            "accion": {"type": "string", "enum": ["anotar", "tablero", "fusionar", "verificar", "sesiones", "mensaje"]},
            "texto": {"type": "string", "description": crate::i18n::t("la nota o el mensaje", "the note or the message")},
            "desde": {"type": "integer", "description": crate::i18n::t("con tablero: sólo las notas después de ésta", "with tablero: only the notes after this one")},
            "ramas": {"type": "array", "items": {"type": "string"}, "description": crate::i18n::t("con fusionar: las ramas, en orden", "with fusionar: the branches, in order")},
            "criterio": {"type": "string", "description": crate::i18n::t("con verificar: qué correr y qué debe cumplirse", "with verificar: what to run and what must hold")},
            "modelo": {"type": "string", "description": crate::i18n::t("con verificar: otro modelo `proveedor:modelo` (sin él, el mismo)", "with verificar: another model `provider:model` (without it, the same one)")},
            "para": {"type": "integer", "description": crate::i18n::t("con mensaje: el número de la sesión destino", "with mensaje: the target session number")},
            "proyecto": {"type": "string", "description": crate::i18n::t("con mensaje o sesiones: la carpeta de otro proyecto (por omisión, éste)", "with mensaje or sesiones: another project's folder (default: this one)")}
        }, "required": ["accion"]}
    }})
}

fn texto(args: &Value, k: &str) -> Option<String> {
    args[k].as_str().map(str::trim).filter(|s| !s.is_empty()).map(String::from)
}

fn proyecto(args: &Value, c: &Contexto) -> PathBuf {
    texto(args, "proyecto").map(PathBuf::from).unwrap_or_else(|| c.raiz.clone())
}

/// Las acciones que no necesitan el bucle del agente. `verificar` → `None` (la atiende el agente).
pub fn simple(args: &Value, c: &Contexto) -> Option<Result<String>> {
    let accion = args["accion"].as_str().unwrap_or("");
    Some(match accion {
        "anotar" => anotar(args, c),
        "tablero" => {
            let desde = args["desde"].as_u64().unwrap_or(0) as u32;
            Ok(tablero::texto(&tablero::leer(&c.tablero), desde))
        }
        "fusionar" => fusionar(args, c),
        "sesiones" => sesiones(&proyecto(args, c), c),
        "mensaje" => mensaje(args, c),
        "verificar" => return None,
        otra => Err(anyhow!("{}", crate::tr!("acción desconocida «{otra}»: anotar, tablero, fusionar, verificar, sesiones o mensaje", "unknown action «{otra}»: anotar, tablero, fusionar, verificar, sesiones or mensaje"))),
    })
}

fn anotar(args: &Value, c: &Contexto) -> Result<String> {
    let t = texto(args, "texto").ok_or_else(|| anyhow!("{}", crate::i18n::t("falta `texto`", "missing `texto`")))?;
    let n = tablero::anotar(&c.tablero, &c.yo, &t)?;
    Ok(crate::tr!("anotado en el tablero como #{n}", "noted on the board as #{n}"))
}

fn fusionar(args: &Value, c: &Contexto) -> Result<String> {
    if c.subagente {
        bail!("{}", crate::i18n::t("fusionar ramas lo hace el agente principal, no un subagente: anota en el tablero lo que hiciste", "merging branches is the main agent's job, not a subagent's: note on the board what you did"));
    }
    if c.plan {
        bail!("{}", crate::i18n::t("modo plan: sólo lectura (fusionar cambia el repositorio)", "plan mode: read-only (merging changes the repository)"));
    }
    let lista: Vec<String> = args["ramas"].as_array().map(|a| a.iter().filter_map(|v| v.as_str()).map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()).unwrap_or_default();
    if lista.is_empty() {
        bail!("{}", crate::i18n::t("falta `ramas` (las que dio el reparto, p. ej. gotv/sub-…)", "missing `ramas` (the ones the split gave, e.g. gotv/sub-…)"));
    }
    let mut hechas = Vec::new();
    for r in &lista {
        match ramas::fusionar(&c.raiz, r, &format!("gotv: {r}")) {
            Ok(f) => hechas.push(f),
            Err(e) => hechas.push(ramas::Fusion::NoSeTrajo { rama: r.clone(), motivo: e.to_string() }),
        }
    }
    Ok(ramas::informe(&hechas))
}

fn sesiones(raiz: &Path, c: &Contexto) -> Result<String> {
    let lista = crate::session::Session::list(raiz)?;
    if lista.is_empty() {
        return Ok(crate::i18n::t("(no hay sesiones guardadas en ese proyecto)", "(no saved sessions in that project)").to_string());
    }
    let mismo = std::fs::canonicalize(raiz).ok() == std::fs::canonicalize(&c.raiz).ok();
    Ok(lista
        .iter()
        .map(|s| {
            let tu = if mismo && Some(s.id) == c.sesion { crate::i18n::t(" (ésta)", " (this one)") } else { "" };
            crate::tr!("#{} {}{tu} · {} mensajes", "#{} {}{tu} · {} messages", s.id, s.title, s.messages)
        })
        .collect::<Vec<_>>()
        .join("\n"))
}

fn mensaje(args: &Value, c: &Contexto) -> Result<String> {
    if c.subagente {
        bail!("{}", crate::i18n::t("los mensajes a otras sesiones los manda el agente principal: anota en el tablero", "messages to other sessions are sent by the main agent: note on the board"));
    }
    let para = args["para"].as_i64().ok_or_else(|| anyhow!("{}", crate::i18n::t("falta `para` (número de sesión; míralo con accion «sesiones»)", "missing `para` (session number; see it with accion «sesiones»)")))?;
    let t = texto(args, "texto").ok_or_else(|| anyhow!("{}", crate::i18n::t("falta `texto`", "missing `texto`")))?;
    let destino = proyecto(args, c);
    if !destino.is_dir() {
        bail!("{}", crate::tr!("no existe la carpeta {}", "folder {} does not exist", destino.display()));
    }
    let de = match c.sesion {
        Some(id) => buzon::direccion(&c.raiz, id),
        None => crate::i18n::t("una sesión sin guardar", "an unsaved session").to_string(),
    };
    buzon::enviar(&buzon::base(), &buzon::direccion(&destino, para), &de, &t)?;
    Ok(crate::tr!("mensaje dejado para la sesión #{para}: lo verá entre pasos", "message left for session #{para}: it will see it between steps"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(raiz: &Path, sub: bool) -> Contexto {
        Contexto { raiz: raiz.to_path_buf(), tablero: tablero::ruta(&raiz.join(".gotv/chats/1")), yo: if sub { "sub-1".into() } else { "principal".into() }, subagente: sub, sesion: Some(1), plan: false }
    }

    #[test]
    fn anotar_y_leer_y_lo_del_principal_no_lo_hace_un_subagente() {
        let d = crate::CarpetaPrueba::nueva("gotv-herr-enjambre");
        let c = ctx(&d, true);
        assert!(simple(&json!({"accion": "anotar", "texto": "toco sólo pedidos.js"}), &c).unwrap().unwrap().contains("#1"));
        let t = simple(&json!({"accion": "tablero"}), &c).unwrap().unwrap();
        assert!(t.contains("[sub-1] toco sólo pedidos.js"), "{t}");
        assert!(simple(&json!({"accion": "fusionar", "ramas": ["x"]}), &c).unwrap().is_err());
        assert!(simple(&json!({"accion": "mensaje", "para": 2, "texto": "x"}), &c).unwrap().is_err());
        assert!(simple(&json!({"accion": "verificar", "criterio": "x"}), &c).is_none());
        assert!(simple(&json!({"accion": "otra"}), &c).unwrap().is_err());
        // en modo plan el principal tampoco fusiona
        let mut p = ctx(&d, false);
        p.plan = true;
        assert!(simple(&json!({"accion": "fusionar", "ramas": ["x"]}), &p).unwrap().is_err());
    }
}
