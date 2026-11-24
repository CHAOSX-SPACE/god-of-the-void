//! F16 · Paridad con la app de escritorio: lo que el Portador hace hoy en la app de escritorio de Claude, en
//! GOTV y en Rust. Cada pieza en su módulo; los archivos calientes (`agent.rs`, `permissions.rs`, `exec.rs`,
//! `gotv-gui/src/main.rs`) sólo llevan el enganche.
//!
//! | Pieza | Módulo |
//! |---|---|
//! | F16.1 escribir mientras trabaja | [`cola`] |
//! | F16.2 agentes a medida | [`agentes`] |
//! | F16.3 proyecto en otro equipo | [`crate::lugar`] |
//! | F16.4 variables guardadas | [`variables`] |
//! | F16.5 `/revisar` | [`revisar`] |
//! | F16.6 seguir un PR | [`pr`] |
//! | F16.7 editar y terminal | [`archivo`], [`terminal`] |
pub mod agentes;
pub mod archivo;
pub mod cola;
pub mod pantalla;
pub mod pr;
pub mod revisar;
pub mod terminal;
pub mod variables;

#[cfg(test)]
pub(crate) mod prueba;
#[cfg(test)]
mod con_modelo;

#[cfg(test)]
mod tests {
    use super::prueba::{agente, llamadas, texto, Guion};
    use crate::permissions::Mode;
    use serde_json::json;
    use std::sync::{Arc, Mutex};

    /// La conversación cumple el protocolo: tras cada turno de la IA con llamadas vienen SUS resultados,
    /// seguidos, antes de cualquier otra cosa.
    fn protocolo_intacto(m: &[gotv_engine::Message]) -> Result<(), String> {
        for (i, x) in m.iter().enumerate() {
            let Some(cs) = &x.tool_calls else { continue };
            for (k, c) in cs.iter().enumerate() {
                let r = m.get(i + 1 + k).ok_or(format!("falta el resultado de la llamada {k} del mensaje {i}"))?;
                if r.role != "tool" || r.tool_call_id.as_deref() != c["id"].as_str() {
                    return Err(format!("en {} hay «{}» donde iba el resultado de {}", i + 1 + k, r.role, c["id"]));
                }
            }
        }
        Ok(())
    }

    /// Ficha F16.1: un mensaje encolado (escrito MIENTRAS el modelo genera un paso con dos llamadas) entra antes
    /// de la siguiente llamada al motor y nunca entre una llamada a herramienta y su resultado.
    #[tokio::test]
    async fn un_mensaje_escrito_a_mitad_entra_entre_dos_pasos() {
        let dueno: Arc<Mutex<String>> = Default::default();
        let d2 = dueno.clone();
        let mut motor = Guion::nuevo("guion", vec![
            llamadas("a", &[("list_dir", json!({"path": "."})), ("glob", json!({"pattern": "*.rs"}))]),
            texto("cambié el rumbo: uso src"),
        ]);
        // el Portador escribe mientras el modelo genera el PRIMER paso
        motor.gancho = Some(Box::new(move |n, _| {
            if n == 0 {
                super::cola::encolar(&d2.lock().unwrap(), "mejor trabaja en src/, no en la raíz", "ventana").unwrap();
            }
        }));
        let visto = motor.visto.clone();
        let mut c = agente("cola-bucle", Mode::Bypass, motor);
        *dueno.lock().unwrap() = c.agente.proc_owner();
        let r = c.agente.run("lista lo que hay").await.unwrap();
        assert!(r.contains("cambié el rumbo"), "{r}");
        let pedidos = visto.lock().unwrap();
        assert_eq!(pedidos.len(), 2, "dos llamadas al motor: el mensaje NO detuvo el turno");
        assert!(!pedidos[0].messages.iter().any(|m| m.content.contains("mejor trabaja en src/")), "aún no se había escrito");
        let segunda = &pedidos[1].messages;
        protocolo_intacto(segunda).unwrap();
        let i = segunda.iter().position(|m| m.content.contains("mejor trabaja en src/")).expect("entró antes de la segunda llamada");
        assert!(segunda[i].es_aviso(), "entra como aviso (F0.10): no reescribe los turnos anteriores");
        assert_eq!(segunda[i - 1].role, "tool");
        assert_eq!(segunda[i - 2].role, "tool");
        assert_eq!(i, segunda.len() - 1, "es lo último antes de llamar al motor");
        protocolo_intacto(&c.agente.history).unwrap();
        assert!(c.agente.front.actividad.lock().unwrap().iter().any(|(k, _)| k == "mensaje_entro"));
        assert!(super::cola::sobrantes(&c.agente.proc_owner()).is_none(), "no quedó nada en la cola");
    }

    /// Lo que llega cuando el turno ya cerró (después del último paso) no se pierde: queda para la orden siguiente.
    #[tokio::test]
    async fn lo_que_llega_tras_el_ultimo_paso_queda_para_la_orden_siguiente() {
        let dueno: Arc<Mutex<String>> = Default::default();
        let d2 = dueno.clone();
        let mut motor = Guion::nuevo("guion", vec![texto("hecho")]);
        motor.gancho = Some(Box::new(move |n, _| {
            if n == 0 {
                super::cola::encolar(&d2.lock().unwrap(), "y ahora los tests", "ventana").unwrap();
            }
        }));
        let mut c = agente("cola-sobra", Mode::Bypass, motor);
        *dueno.lock().unwrap() = c.agente.proc_owner();
        assert_eq!(c.agente.run("algo corto").await.unwrap(), "hecho");
        assert_eq!(super::cola::sobrantes(&c.agente.proc_owner()).as_deref(), Some("y ahora los tests"));
    }

    /// Ficha F16.2: un agente con `herramientas: [read_file]` no puede escribir aunque el modelo lo intente
    /// (también en bypass), ni ve las que no son suyas; y usa el modelo declarado (el del chat no recibe
    /// sus pasos). Al volver, el agente principal tiene todo como antes.
    #[tokio::test]
    async fn un_agente_a_medida_usa_su_modelo_y_solo_sus_herramientas() {
        let principal = Guion::nuevo("guion", vec![
            llamadas("p", &[("task", json!({"agente": "revisor", "prompt": "revisa main.rs", "description": "revisar"}))]),
            llamadas("q", &[("write_file", json!({"path": "despues.txt", "content": "el principal sí escribe"}))]),
            texto("fin"),
        ]);
        let visto_principal = principal.visto.clone();
        let mut c = agente("agente-medida", Mode::Bypass, principal);
        std::fs::create_dir_all(c.raiz.join(".gotv/agentes")).unwrap();
        std::fs::write(c.raiz.join(".gotv/agentes/revisor.md"), "---\nnombre: revisor\ndescripcion: revisa sin tocar\nherramientas: [read_file]\nmodelo: modelo-del-revisor-f16\n---\nLee y opina; no cambies nada.\n").unwrap();
        std::fs::write(c.raiz.join("main.rs"), "fn main() {}\n").unwrap();
        // su modelo: otro motor de guion, que INTENTA escribir
        let sub = Guion::nuevo("modelo-del-revisor-f16", vec![
            llamadas("s", &[("write_file", json!({"path": "hackeo.txt", "content": "no debería existir"})), ("read_file", json!({"path": "main.rs"}))]),
            texto("informe: main.rs está vacío"),
        ]);
        let visto_sub = sub.visto.clone();
        let sub: super::agentes::Motor = Arc::new(sub);
        let pedidos_modelo: Arc<Mutex<Vec<String>>> = Default::default();
        let pm = pedidos_modelo.clone();
        super::agentes::registrar_fabrica_para(&c.agente.proc_owner(), super::agentes::fabrica(move |m: String| {
            pm.lock().unwrap().push(m);
            let s = sub.clone();
            async move { Ok(s) }
        }));
        let r = c.agente.run("revisa el proyecto con el revisor").await.unwrap();
        assert_eq!(r, "fin");
        assert_eq!(*pedidos_modelo.lock().unwrap(), vec!["modelo-del-revisor-f16".to_string()], "cargó el modelo declarado");
        assert!(!c.raiz.join("hackeo.txt").exists(), "el agente a medida NO pudo escribir");
        assert!(c.raiz.join("despues.txt").exists(), "el principal, al volver, sí puede");
        let vs = visto_sub.lock().unwrap();
        assert_eq!(vs.len(), 2, "los dos pasos del subagente fueron a SU modelo");
        let nombres: Vec<String> = vs[0].tools.as_ref().unwrap().as_array().unwrap().iter().map(|t| t["function"]["name"].as_str().unwrap().to_string()).collect();
        assert!(nombres.contains(&"read_file".to_string()) && !nombres.contains(&"write_file".to_string()) && !nombres.contains(&"bash".to_string()), "{nombres:?}");
        assert!(vs[0].messages.iter().any(|m| m.content.contains("Lee y opina; no cambies nada.")), "lleva sus instrucciones");
        assert!(vs[0].conversacion.is_none(), "otro modelo no usa la caché en disco del chat");
        let negado = vs[1].messages.iter().find(|m| m.tool_call_id.as_deref() == Some("s0")).unwrap();
        assert!(negado.content.contains("write_file") && (negado.content.contains("DENEGADO") || negado.content.contains("DENIED")), "{}", negado.content);
        let leido = vs[1].messages.iter().find(|m| m.tool_call_id.as_deref() == Some("s1")).unwrap();
        assert!(leido.content.contains("fn main"), "{}", leido.content);
        let vp = visto_principal.lock().unwrap();
        assert_eq!(vp.len(), 3, "el modelo del chat sólo vio los pasos del principal");
        assert!(vp[1].messages.iter().any(|m| m.content.contains("main.rs está vacío")), "el informe vuelve al principal");
        assert!(c.agente.perms.solo_herramientas.is_none());
    }

    /// `task { agente: "?" }` lista; uno que no existe lo dice sin lanzar nada.
    #[tokio::test]
    async fn listar_y_agente_inexistente() {
        let principal = Guion::nuevo("guion", vec![
            llamadas("p", &[("task", json!({"agente": "?"})), ("task", json!({"agente": "fantasma", "prompt": "x"}))]),
            texto("fin"),
        ]);
        let visto = principal.visto.clone();
        let mut c = agente("agente-lista", Mode::Bypass, principal);
        std::fs::create_dir_all(c.raiz.join(".claude/agents")).unwrap();
        std::fs::write(c.raiz.join(".claude/agents/docs.md"), "---\nname: docs\ndescription: writes docs\ntools: Read, Write\n---\nWrite docs.").unwrap();
        c.agente.run("¿qué agentes hay?").await.unwrap();
        let v = visto.lock().unwrap();
        let lista = v[1].messages.iter().find(|m| m.tool_call_id.as_deref() == Some("p0")).unwrap();
        assert!(lista.content.contains("docs") && lista.content.contains("read_file, write_file"), "{}", lista.content);
        let no = v[1].messages.iter().find(|m| m.tool_call_id.as_deref() == Some("p1")).unwrap();
        assert!(no.content.starts_with("ERROR") && no.content.contains("fantasma"), "{}", no.content);
    }

    /// Ficha F16.5: `/revisar` deja una tarjeta con hallazgos de archivo y línea REALES (con el texto de la
    /// línea), el turno de revisión no puede escribir, y «arreglar éste» aplica uno.
    #[tokio::test]
    async fn revisar_deja_una_tarjeta_real_y_arreglar_aplica_uno() {
        let respuesta = r#"Revisé el cambio. {"hallazgos": [
            {"archivo": "src/calc.rs", "linea": 2, "severidad": "alta", "resumen": "divide entre cero cuando n es 0", "como_falla": "promedio(&[]) → pánico por división entre cero"},
            {"archivo": "src/fantasma.rs", "linea": 9, "resumen": "no existe", "como_falla": "-"}]}"#;
        let motor = Guion::nuevo("guion", vec![
            // el modelo intenta «arreglar» mientras revisa: no puede
            llamadas("r", &[("write_file", json!({"path": "src/calc.rs", "content": "roto"})), ("read_file", json!({"path": "src/calc.rs"}))]),
            texto(respuesta),
            // «arreglar éste»
            llamadas("f", &[("edit", json!({"path": "src/calc.rs", "old_string": "s / n", "new_string": "if n == 0 { 0 } else { s / n }"}))]),
            texto("arreglado"),
        ]);
        let mut c = agente("revisar", Mode::Bypass, motor);
        let raiz = c.raiz.to_path_buf();
        let git = |args: &[&str]| assert!(std::process::Command::new("git").arg("-C").arg(&raiz).args(args).output().unwrap().status.success(), "git {args:?}");
        git(&["init", "-q"]);
        git(&["config", "user.email", "f16@prueba"]);
        git(&["config", "user.name", "f16"]);
        std::fs::create_dir_all(raiz.join("src")).unwrap();
        std::fs::write(raiz.join("src/calc.rs"), "pub fn promedio(v: &[i32]) -> i32 {\n    0\n}\n").unwrap();
        git(&["add", "."]);
        git(&["commit", "-qm", "base"]);
        std::fs::write(raiz.join("src/calc.rs"), "pub fn promedio(v: &[i32]) -> i32 {\n    let (s, n) = (v.iter().sum::<i32>(), v.len() as i32); s / n\n}\n").unwrap();
        let (tipo, rev) = super::revisar::revisar(&mut c.agente, "").await.unwrap();
        assert_eq!(tipo, super::revisar::Tipo::Codigo);
        assert_eq!(std::fs::read_to_string(raiz.join("src/calc.rs")).unwrap().lines().count(), 3, "la revisión no escribió");
        assert!(c.agente.perms.solo_herramientas.is_none(), "al terminar, todo como antes");
        assert_eq!(rev.hallazgos.len(), 1);
        let h = &rev.hallazgos[0];
        assert_eq!((h.archivo.as_str(), h.linea), ("src/calc.rs", 2));
        assert!(h.codigo.contains("s / n"), "la tarjeta lleva el texto real de la línea: {}", h.codigo);
        assert_eq!(rev.descartados.len(), 1);
        let t = rev.tarjeta(tipo);
        assert_eq!(t["kind"], "revision");
        // la orden llevaba el diff calculado por GOTV
        let (orden, _) = super::revisar::orden_arreglar(&c.agente.proc_owner(), 1).unwrap();
        assert!(orden.contains("src/calc.rs:2") && orden.contains("s / n"));
        assert!(super::revisar::orden_arreglar(&c.agente.proc_owner(), 9).is_none());
        c.agente.run(&orden).await.unwrap();
        let ahora = std::fs::read_to_string(raiz.join("src/calc.rs")).unwrap();
        assert!(ahora.contains("if n == 0 { 0 } else { s / n }"), "arreglar éste aplicó el cambio: {ahora}");
    }

    /// Una revisión que termina en PROSA (sin el JSON) no deja la tarjeta vacía: se pide una vez el JSON, aún en
    /// sólo lectura (medido 2026-11-18 con Huihui: halló el fallo, lo contó, y la tarjeta salía sin nada).
    #[tokio::test]
    async fn una_revision_en_prosa_pide_su_json_una_vez() {
        let motor = Guion::nuevo("guion", vec![
            texto("Hallazgo: la línea 2 divide entre cero cuando la lista está vacía."),
            // al pedirle el JSON, intenta escribir: sigue en sólo lectura
            llamadas("w", &[("write_file", json!({"path": "src/calc.rs", "content": "roto"}))]),
            texto(r#"{"hallazgos": [{"archivo": "src/calc.rs", "linea": 2, "severidad": "alta", "resumen": "divide entre cero", "como_falla": "promedio(&[]) → pánico"}]}"#),
        ]);
        let visto = motor.visto.clone();
        let mut c = agente("revisar-prosa", Mode::Bypass, motor);
        let raiz = c.raiz.to_path_buf();
        super::prueba::git(&raiz, &["init", "-q"]);
        super::prueba::git(&raiz, &["config", "user.email", "f16@prueba"]);
        super::prueba::git(&raiz, &["config", "user.name", "f16"]);
        std::fs::create_dir_all(raiz.join("src")).unwrap();
        std::fs::write(raiz.join("src/calc.rs"), "pub fn promedio(v: &[i32]) -> i32 {\n    0\n}\n").unwrap();
        super::prueba::git(&raiz, &["add", "."]);
        super::prueba::git(&raiz, &["commit", "-qm", "base"]);
        let malo = "pub fn promedio(v: &[i32]) -> i32 {\n    v.iter().sum::<i32>() / v.len() as i32\n}\n";
        std::fs::write(raiz.join("src/calc.rs"), malo).unwrap();
        let (_, rev) = super::revisar::revisar(&mut c.agente, "").await.unwrap();
        assert!(!rev.sin_json && rev.hallazgos.len() == 1, "{rev:?}");
        assert_eq!((rev.hallazgos[0].archivo.as_str(), rev.hallazgos[0].linea), ("src/calc.rs", 2));
        assert_eq!(std::fs::read_to_string(raiz.join("src/calc.rs")).unwrap(), malo, "el segundo pedido también es de sólo lectura");
        assert_eq!(visto.lock().unwrap().len(), 3, "una sola vez más");
        assert!(c.agente.perms.solo_herramientas.is_none(), "al terminar, todo como antes");
    }

    /// Ficha F16.6 («hecho cuando»): un PR con el CI en rojo queda en verde sin fusionarse solo. GitHub FALSO
    /// local (axum) cuyo «CI» mira el contenido de la rama en un remoto git LOCAL; nada sale a GitHub.
    #[tokio::test]
    async fn un_pr_en_rojo_queda_en_verde_sin_fusionarse() {
        use super::prueba::git;
        use std::sync::atomic::Ordering;
        let fuera = crate::CarpetaPrueba::nueva("gotv-f16-pr-remoto");
        let motor = Guion::nuevo("guion", vec![llamadas("w", &[("write_file", json!({"path": "calc.txt", "content": "2+2=4\n"}))]), texto("arreglado: 2+2=4")]);
        let mut c = agente("pr", Mode::Default, motor);
        *c.agente.front.respuesta.lock().unwrap() = Some(crate::permissions::AskAnswer::Yes);
        let raiz = c.raiz.to_path_buf();
        let bare = super::prueba::repo_del_pr(&raiz, &fuera);
        let (dir, fusiones) = super::prueba::github_falso(bare.clone()).await;
        let mut s = super::pr::Seguimiento::nuevo(&raiz, "7 --arreglar --repo yo/repo").unwrap();
        s.base = format!("http://{dir}");
        s.token = None;
        s.ritmo = std::time::Duration::from_millis(20);
        s.espera = std::time::Duration::from_secs(5);
        let r = super::pr::seguir(&mut c.agente, s).await;
        assert!(r.contains("PR #7") && (r.contains("en verde") || r.contains("is green")), "{r}");
        assert!(r.contains("no lo fusiona") || r.contains("does not merge"), "{r}");
        assert_eq!(fusiones.load(Ordering::SeqCst), 0, "jamás se pidió fusionar");
        assert_eq!(git(&bare, &["show", "refs/heads/arreglo:calc.txt"]), "2+2=4", "el arreglo llegó a la rama del PR");
        let preguntas = c.agente.front.preguntas.lock().unwrap();
        assert!(preguntas.iter().any(|p| p.contains("EMPUJAR") || p.contains("PUSH")), "empujar pidió permiso: {preguntas:?}");
    }

    /// El agente que confirma (y quizá empuja) ÉL MISMO deja el árbol limpio: eso no es «la ronda no cambió nada»
    /// (medido 2026-11-18 con Huihui, que confirmó y empujó con la herramienta `git`).
    #[tokio::test]
    async fn un_pr_que_el_agente_confirma_el_mismo_sigue_adelante() {
        use std::sync::atomic::Ordering;
        for empuja_el in [true, false] {
            let fuera = crate::CarpetaPrueba::nueva("gotv-f16-pr-solo-remoto");
            let mut pasos = vec![
                llamadas("w", &[("write_file", json!({"path": "calc.txt", "content": "2+2=4\n"}))]),
                llamadas("g", &[("git", json!({"action": "preparar", "rutas": ["calc.txt"]})), ("git", json!({"action": "confirmar", "mensaje": "arreglo", "rutas": ["calc.txt"]}))]),
            ];
            if empuja_el {
                pasos.push(llamadas("p", &[("git", json!({"action": "empujar", "remoto": "origin", "nombre": "arreglo"}))]));
            }
            pasos.push(texto("hecho"));
            let mut c = agente("pr-solo", Mode::Default, Guion::nuevo("guion", pasos));
            *c.agente.front.respuesta.lock().unwrap() = Some(crate::permissions::AskAnswer::Yes);
            let raiz = c.raiz.to_path_buf();
            let bare = super::prueba::repo_del_pr(&raiz, &fuera);
            let (dir, fusiones) = super::prueba::github_falso(bare.clone()).await;
            let mut s = super::pr::Seguimiento::nuevo(&raiz, "7 --arreglar --repo yo/repo").unwrap();
            s.base = format!("http://{dir}");
            s.token = None;
            s.ritmo = std::time::Duration::from_millis(20);
            s.espera = std::time::Duration::from_secs(5);
            let r = super::pr::seguir(&mut c.agente, s).await;
            assert!(r.contains("en verde") || r.contains("is green"), "empuja_el={empuja_el}: {r}");
            assert_eq!(super::prueba::git(&bare, &["show", "refs/heads/arreglo:calc.txt"]), "2+2=4");
            assert_eq!(super::prueba::git(&bare, &["log", "-1", "--format=%s", "refs/heads/arreglo"]), "arreglo", "llegó el commit DEL AGENTE (empuja_el={empuja_el})");
            assert_eq!(fusiones.load(Ordering::SeqCst), 0);
        }
    }

    /// Un subagente no se come el mensaje del Portador: va al agente principal.
    #[tokio::test]
    async fn el_subagente_no_se_come_el_mensaje() {
        let dueno: Arc<Mutex<String>> = Default::default();
        let d2 = dueno.clone();
        let mut motor = Guion::nuevo("guion", vec![
            llamadas("p", &[("task", json!({"prompt": "lista", "description": "listar"}))]),
            llamadas("s", &[("list_dir", json!({"path": "."}))]),
            texto("informe del sub"),
            texto("fin principal"),
        ]);
        motor.gancho = Some(Box::new(move |n, _| {
            if n == 1 {
                super::cola::encolar(&d2.lock().unwrap(), "dato del Portador", "ventana").unwrap();
            }
        }));
        let visto = motor.visto.clone();
        let mut c = agente("cola-sub", Mode::Bypass, motor);
        *dueno.lock().unwrap() = c.agente.proc_owner();
        c.agente.run("delega").await.unwrap();
        let pedidos = visto.lock().unwrap();
        assert!(!pedidos[2].messages.iter().any(|m| m.content.contains("dato del Portador")), "el sub no lo ve");
        assert!(pedidos[3].messages.iter().any(|m| m.content.contains("dato del Portador")), "el principal sí, al volver");
        protocolo_intacto(&pedidos[3].messages).unwrap();
    }
}
