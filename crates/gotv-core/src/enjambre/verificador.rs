//! F9 · el verificador: OTRA conversación —que no escribió el código— corre las pruebas sobre lo que quedó
//! en el árbol y da un veredicto. Puede ir en otro modelo: trabajadores en el local y verificador en la nube
//! (sin modelo nuevo en RAM), o al revés con un modelo servido por un proveedor compatible (LM Studio, Ollama,
//! `mlx_lm.server`): el modelo se nombra como en Configuración, `proveedor:modelo`.
//!
//! No edita: lleva los permisos del chat más reglas `deny` para toda herramienta que escribe archivos
//! ([`NO_EDITA`]); comandos sí (correr pruebas es su trabajo), con la misma puerta de siempre, y lo que
//! cambie por la terminal lo deshace el punto de restauración del turno. El agente que lo corre está en
//! `agent/paralelo.rs` (necesita el bucle del agente).

use crate::permissions::Permissions;
use anyhow::{anyhow, Result};
use std::path::{Path, PathBuf};

/// Lo que el verificador NO puede usar: todo lo que escribe archivos, y repartir trabajo.
pub const NO_EDITA: &[&str] = &[
    "write_file", "edit", "multi_edit", "notebook", "pagina", "isometrica", "generar_imagen", "generar_musica", "generar_efecto", "separar_pistas", "generar_video", "app_nueva", "api_nueva", "excel", "word", "powerpoint", "pdf", "task", "enjambre",
];

/// Los permisos del verificador: los del chat, en `raiz`, sin herramientas que editen.
pub fn permisos(base: &Permissions, raiz: PathBuf) -> Permissions {
    let mut p = base.clone();
    p.set_root(raiz);
    for t in NO_EDITA {
        p.negar(t);
    }
    p
}

/// Su pedido.
pub fn pedido(criterio: &str) -> String {
    crate::tr!(
        "Eres el VERIFICADOR de un trabajo que hicieron otros agentes. NO escribiste ese código y NO lo cambias: no edites archivos (tus herramientas de edición están negadas) ni los cambies por la terminal. Comprueba con las herramientas —lee lo justo, CORRE las pruebas y lo que pida el criterio— y juzga sólo con lo que devuelven.\n\nCriterio: {criterio}\n\nTermina con un informe breve: qué corriste, qué salió (cita la salida real) y, en la ÚLTIMA línea, exactamente «VEREDICTO: PASA» o «VEREDICTO: FALLA — <lo que falla>».",
        "You are the VERIFIER of work done by other agents. You did NOT write that code and you do NOT change it: do not edit files (your editing tools are denied) or change them through the terminal. Check with the tools —read just enough, RUN the tests and whatever the criterion asks— and judge only by what they return.\n\nCriterion: {criterio}\n\nFinish with a short report: what you ran, what came out (quote the real output) and, on the LAST line, exactly «VERDICT: PASS» or «VERDICT: FAIL — <what fails>»."
    )
}

/// El veredicto del informe: `Some(true)` pasa, `Some(false)` falla, `None` no lo dijo.
pub fn veredicto(informe: &str) -> Option<bool> {
    // juez-idioma: datos (las marcas exactas que se le piden al verificador, en los dos idiomas)
    let marcas = [("VEREDICTO: PASA", true), ("VERDICT: PASS", true), ("VEREDICTO: FALLA", false), ("VERDICT: FAIL", false)];
    informe.lines().rev().find_map(|l| {
        let l = l.trim().trim_matches(|c| c == '*' || c == '`' || c == '«' || c == '»').trim();
        marcas.iter().find(|(m, _)| l.to_uppercase().starts_with(m)).map(|(_, v)| *v)
    })
}

/// El motor de otro modelo (`proveedor:modelo`), arrancado con la config del proyecto. Devuelve el motor y
/// el presupuesto de contexto que le corresponde.
pub async fn motor_de(modelo: &str, raiz: &Path) -> Result<(gotv_engine::AnyEngine, usize)> {
    let mut cfg = crate::config::Config::load(raiz);
    if !crate::nube::is_cloud(modelo, &cfg) {
        return Err(anyhow!(
            "{}",
            crate::tr!(
                "«{modelo}» no es un modelo por API: el verificador en otro modelo se nombra `proveedor:modelo` (un proveedor de Configuración → Proveedores; también LM Studio u Ollama). El modelo local de la app es el mismo motor: omite «modelo».",
                "«{modelo}» is not an API model: the verifier on another model is named `provider:model` (a provider from Settings → Providers; LM Studio or Ollama too). The app's local model is the same engine: omit «modelo»."
            )
        ));
    }
    cfg.model = modelo.to_string();
    let (e, _nombre, _notas) = crate::nube::launch(&mut cfg).await?;
    Ok((e, cfg.context_budget))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::permissions::{Decision, Mode};
    use serde_json::json;

    #[test]
    fn el_veredicto_se_lee_de_la_ultima_marca() {
        assert_eq!(veredicto("corrí npm test\n3 pasan\nVEREDICTO: PASA"), Some(true));
        assert_eq!(veredicto("ran it\n**VERDICT: FAIL — 2 tests fail**"), Some(false));
        assert_eq!(veredicto("VEREDICTO: FALLA — x\n...\nVEREDICTO: PASA"), Some(true));
        assert_eq!(veredicto("todo bien"), None);
    }

    #[test]
    fn el_verificador_no_edita_pero_corre_comandos() {
        let d = crate::CarpetaPrueba::nueva("gotv-verificador-perms");
        let base = Permissions::new(Mode::Bypass, &d, vec![], vec![], vec![]);
        let p = permisos(&base, d.to_path_buf());
        assert!(matches!(p.check("write_file", &json!({"path": "a.txt", "content": "x"})), Decision::Deny(_)));
        assert!(matches!(p.check("edit", &json!({"path": "a.txt"})), Decision::Deny(_)));
        assert!(matches!(p.check("bash", &json!({"cmd": "npm test"})), Decision::Allow));
        // la copia no toca los permisos del chat
        assert!(matches!(base.check("write_file", &json!({"path": "a.txt", "content": "x"})), Decision::Allow));
    }
}
