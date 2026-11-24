//! F16 · los «hecho cuando» con un MODELO DE VERDAD (no el guion): Huihui servido por `gotv serve`, hablado
//! como OpenAI. Todas `#[ignore]`: cargan un modelo (regla del Portador: sólo cuando se pide). Todo este archivo
//! es código de prueba (`#[cfg(test)]` en `escritorio/mod.rs`): el juez de idioma no lo mira.
//! ```text
//! GOTV_MODEL=<modelo> gotv serve --port 11437                     # el del chat
//! GOTV_MODEL=<otro>   gotv serve --port 11438                     # el del revisor (sólo `revisor_…`)
//! GOTV_PRUEBA_OPENAI=http://127.0.0.1:11437/v1 GOTV_PRUEBA_MODELO=<modelo> \
//! GOTV_PRUEBA_OPENAI2=http://127.0.0.1:11438/v1 GOTV_PRUEBA_MODELO2=<otro> \
//!   cargo test -p gotv-core --release --lib -- --ignored --nocapture --test-threads=1 escritorio::con_modelo
//! ```
//! Cada una imprime su tiempo y lo que hizo el modelo; lo que se afirma es lo que quedó en disco y en el bucle.
use super::prueba::{agente_con, ConCarpeta};
use crate::permissions::{AskAnswer, Mode};
use anyhow::Result;
use async_trait::async_trait;
use gotv_engine::cloud::OpenAiEngine;
use gotv_engine::{Capabilities, ChatRequest, ChatResponse, Engine, StreamEvent};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

fn motor(url_var: &str, modelo_var: &str) -> Option<(OpenAiEngine, String)> {
    let (Ok(url), Ok(modelo)) = (std::env::var(url_var), std::env::var(modelo_var)) else {
        eprintln!("sin {url_var}/{modelo_var}: no se mide");
        return None;
    };
    Some((OpenAiEngine::new("gotv", &url, String::new(), &modelo, false), modelo))
}

fn agente_real(tag: &str, modo: Mode, m: OpenAiEngine) -> ConCarpeta<OpenAiEngine> {
    let mut c = agente_con(tag, modo, m);
    c.agente.max_steps = 30;
    c.agente.max_tokens = 4096;
    c.agente.context_budget = 65536;
    c
}

/// Un motor que guarda lo que se le pide (para ver qué herramientas recibió y cuántos pasos dio).
struct Espia {
    dentro: OpenAiEngine,
    visto: Arc<Mutex<Vec<ChatRequest>>>,
}

#[async_trait]
impl Engine for Espia {
    fn name(&self) -> String {
        self.dentro.name()
    }
    fn capabilities(&self) -> Capabilities {
        self.dentro.capabilities()
    }
    async fn start(&mut self) -> Result<()> {
        self.dentro.start().await
    }
    async fn complete(&self, req: ChatRequest) -> Result<ChatResponse> {
        self.visto.lock().unwrap().push(req.clone());
        self.dentro.complete(req).await
    }
    async fn complete_stream(&self, req: ChatRequest, on: &(dyn Fn(StreamEvent) + Send + Sync)) -> Result<ChatResponse> {
        self.visto.lock().unwrap().push(req.clone());
        self.dentro.complete_stream(req, on).await
    }
    async fn stop(&mut self) -> Result<()> {
        self.dentro.stop().await
    }
}

/// Lo que pasó en la conversación (para leer por qué falló): rol, llamadas y texto recortado de cada mensaje.
fn rastro(h: &[gotv_engine::Message]) -> String {
    h.iter()
        .map(|m| {
            let llamadas = m.tool_calls.as_ref().map(|c| c.iter().map(|x| format!("{}({})", x["function"]["name"].as_str().or(x["name"].as_str()).unwrap_or("?"), x["function"]["arguments"].to_string().chars().take(160).collect::<String>())).collect::<Vec<_>>().join(", ")).unwrap_or_default();
            format!("[{}] {} {}", m.role, llamadas, m.content.chars().take(600).collect::<String>().replace('\n', " ⏎ "))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn nombres(req: &ChatRequest) -> Vec<String> {
    req.tools.as_ref().and_then(|t| t.as_array().cloned()).unwrap_or_default().iter().filter_map(|t| t["function"]["name"].as_str().map(String::from)).collect()
}

/// F16.1: con el agente trabajando, un mensaje escrito a mitad cambia su rumbo SIN detenerlo.
#[tokio::test]
#[ignore]
async fn la_cola_cambia_el_rumbo_con_el_modelo() {
    let Some((m, modelo)) = motor("GOTV_PRUEBA_OPENAI", "GOTV_PRUEBA_MODELO") else { return };
    let mut c = agente_real("cola-real", Mode::Bypass, m);
    let lista = c.raiz.join("lista.txt");
    let dueno = c.agente.proc_owner();
    // el Portador escribe en cuanto ve la primera línea (con el modelo a mitad de su trabajo)
    let (l2, d2) = (lista.clone(), dueno.clone());
    let escrito: Arc<Mutex<Option<usize>>> = Default::default();
    let e2 = escrito.clone();
    let vigia = tokio::spawn(async move {
        for _ in 0..3000 {
            let n = std::fs::read_to_string(&l2).map(|s| s.lines().count()).unwrap_or(0);
            if n >= 1 {
                super::cola::encolar(&d2, "Cambio de planes: deja los números que ya escribiste como están y, desde ahora, sigue con las letras a, b, c… (una por línea, una llamada a bash por letra) hasta que lista.txt tenga 6 líneas en total.", "ventana").unwrap();
                *e2.lock().unwrap() = Some(n);
                return;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    });
    let t = Instant::now();
    let r = c.agente.run("Crea lista.txt con los números del 1 al 6, uno por línea. Hazlo de a UNA línea por llamada: una llamada a bash `echo N >> lista.txt` por cada número, esperando el resultado de cada una antes de la siguiente.").await;
    vigia.abort();
    let s = std::fs::read_to_string(&lista).unwrap_or_default();
    eprintln!("{modelo} · {} s · escrito con {:?} línea(s) · lista:\n{s}\nrespuesta: {}", t.elapsed().as_secs(), escrito.lock().unwrap(), r.as_deref().unwrap_or("ERROR").chars().take(300).collect::<String>());
    let r = r.expect("el turno terminó solo (no se detuvo)");
    assert!(!r.is_empty());
    assert!(escrito.lock().unwrap().is_some(), "el mensaje se escribió con el agente trabajando");
    assert!(c.agente.front.actividad.lock().unwrap().iter().any(|(k, _)| k == "mensaje_entro"), "el mensaje ENTRÓ a mitad");
    let lineas: Vec<&str> = s.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    let primera_letra = lineas.iter().position(|l| l.chars().all(|ch| ch.is_ascii_lowercase()));
    let i = primera_letra.expect("cambió el rumbo: hay letras");
    assert!(i >= 1 && lineas[..i].iter().all(|l| l.parse::<u32>().is_ok()), "empezó con números: {lineas:?}");
    assert!(lineas[i..].iter().all(|l| l.parse::<u32>().is_err()), "tras el cambio, ningún número más: {lineas:?}");
    assert!(super::cola::sobrantes(&dueno).is_none(), "no quedó nada en la cola");
}

/// F16.2: `task { agente: "revisor" }` corre con OTRO modelo y sólo con sus herramientas.
#[tokio::test]
#[ignore]
async fn el_revisor_corre_con_otro_modelo_y_sus_herramientas() {
    let Some((m, modelo)) = motor("GOTV_PRUEBA_OPENAI", "GOTV_PRUEBA_MODELO") else { return };
    let (Ok(url2), Ok(modelo2)) = (std::env::var("GOTV_PRUEBA_OPENAI2"), std::env::var("GOTV_PRUEBA_MODELO2")) else {
        eprintln!("sin GOTV_PRUEBA_OPENAI2/GOTV_PRUEBA_MODELO2: no se mide");
        return;
    };
    let mut c = agente_real("revisor-real", Mode::Bypass, m);
    let calc = "function promedio(v) {\n  let s = 0;\n  for (const x of v) s += x;\n  return s / v.length;\n}\n\nfunction ultimo(v) {\n  return v[v.length];\n}\n\nmodule.exports = { promedio, ultimo };\n";
    std::fs::write(c.raiz.join("calc.js"), calc).unwrap();
    std::fs::create_dir_all(c.raiz.join(".gotv/agentes")).unwrap();
    std::fs::write(c.raiz.join(".gotv/agentes/revisor.md"), format!("---\nnombre: revisor\ndescripcion: revisa código sin tocarlo\nherramientas: [read_file, list_dir, grep]\nmodelo: {modelo2}\n---\nEres un revisor de código. Lee el archivo que te piden y di qué fallos reales tiene, con su línea. No cambies nada.\n")).unwrap();
    let pedidos: Arc<Mutex<Vec<String>>> = Default::default();
    let visto: Arc<Mutex<Vec<ChatRequest>>> = Default::default();
    let (pm, vm) = (pedidos.clone(), visto.clone());
    super::agentes::registrar_fabrica_para(&c.agente.proc_owner(), super::agentes::fabrica(move |mo: String| {
        pm.lock().unwrap().push(mo.clone());
        let e: super::agentes::Motor = Arc::new(Espia { dentro: OpenAiEngine::new("gotv", &url2, String::new(), &mo, false), visto: vm.clone() });
        async move { Ok(e) }
    }));
    let t = Instant::now();
    let r = c.agente.run("Pídele al agente «revisor» (herramienta task con agente: \"revisor\") que revise calc.js. Tú no cambies nada; al final dime en dos líneas qué encontró.").await;
    eprintln!("{modelo} + revisor {modelo2} · {} s · respuesta: {}", t.elapsed().as_secs(), r.as_deref().unwrap_or("ERROR"));
    r.unwrap();
    super::agentes::soltar_guardado();
    assert_eq!(*pedidos.lock().unwrap(), vec![modelo2.clone()], "cargó el modelo DECLARADO por el revisor");
    let v = visto.lock().unwrap();
    assert!(!v.is_empty(), "el revisor habló con SU modelo");
    eprintln!("el revisor dio {} paso(s) con {modelo2}; herramientas que vio: {:?}", v.len(), nombres(&v[0]));
    for req in v.iter() {
        let ns = nombres(req);
        // las suyas y las que todo agente lleva porque no actúan fuera de su conversación (`agentes::SIEMPRE`)
        assert!(ns.iter().all(|n| ["read_file", "list_dir", "grep"].contains(&n.as_str()) || super::agentes::SIEMPRE.contains(&n.as_str())), "sólo sus herramientas: {ns:?}");
        assert!(!ns.iter().any(|n| ["write_file", "edit", "bash"].contains(&n.as_str())), "ni ve las de escribir: {ns:?}");
    }
    assert_eq!(std::fs::read_to_string(c.raiz.join("calc.js")).unwrap(), calc, "nadie tocó calc.js");
    assert!(c.agente.perms.solo_herramientas.is_none(), "el principal vuelve con todo");
}

/// F16.5: `/revisar` deja una tarjeta con hallazgos de archivo y línea reales, y «arreglar éste» aplica uno.
#[tokio::test]
#[ignore]
async fn revisar_y_arreglar_con_el_modelo() {
    use super::prueba::git;
    let Some((m, modelo)) = motor("GOTV_PRUEBA_OPENAI", "GOTV_PRUEBA_MODELO") else { return };
    let mut c = agente_real("revisar-real", Mode::Bypass, m);
    let raiz = c.raiz.to_path_buf();
    git(&raiz, &["init", "-q"]);
    git(&raiz, &["config", "user.email", "f16@prueba"]);
    git(&raiz, &["config", "user.name", "f16"]);
    std::fs::write(raiz.join(".gitignore"), ".gotv/\n").unwrap();
    std::fs::write(raiz.join("pila.js"), "function cima(v) {\n  return null;\n}\n\nmodule.exports = { cima };\n").unwrap();
    git(&raiz, &["add", "."]);
    git(&raiz, &["commit", "-qm", "base"]);
    // el cambio a revisar: lee una posición después del final
    let con_fallo = "// la cima de la pila: el último elemento\nfunction cima(v) {\n  if (v.length === 0) return null;\n  return v[v.length];\n}\n\nmodule.exports = { cima };\n";
    std::fs::write(raiz.join("pila.js"), con_fallo).unwrap();
    let linea_mala = con_fallo.lines().position(|l| l.contains("v[v.length]")).unwrap() as u32 + 1;
    let t = Instant::now();
    let (tipo, rev) = super::revisar::revisar(&mut c.agente, "").await.unwrap();
    eprintln!("{modelo} · /revisar en {} s · {} hallazgo(s), {} descartado(s), sin_json={}", t.elapsed().as_secs(), rev.hallazgos.len(), rev.descartados.len(), rev.sin_json);
    for h in &rev.hallazgos {
        eprintln!("  {}:{} [{}] {} — {}", h.archivo, h.linea, h.severidad, h.resumen, h.codigo);
    }
    if rev.hallazgos.is_empty() {
        eprintln!("rastro:\n{}", rastro(&c.agente.history));
    }
    assert_eq!(std::fs::read_to_string(raiz.join("pila.js")).unwrap(), con_fallo, "la revisión no escribió");
    let tarjeta = rev.tarjeta(tipo);
    assert_eq!(tarjeta["kind"], "revision");
    let h = rev.hallazgos.iter().find(|h| h.archivo == "pila.js" && h.linea == linea_mala).unwrap_or_else(|| panic!("halló el fallo en pila.js:{linea_mala}: {:?}", rev.hallazgos));
    assert!(h.codigo.contains("v[v.length]"), "la tarjeta lleva el texto real de la línea");
    let (orden, _) = super::revisar::orden_arreglar(&c.agente.proc_owner(), h.id).unwrap();
    let t = Instant::now();
    let r = c.agente.run(&orden).await.unwrap();
    eprintln!("«arreglar éste» en {} s: {}", t.elapsed().as_secs(), r.chars().take(300).collect::<String>());
    let o = std::process::Command::new("node").arg("-e").arg("const {cima}=require('./pila.js'); if (cima([1,2,3])!==3 || cima([])!==null) { console.log(cima([1,2,3])); process.exit(1) }").current_dir(&raiz).output().unwrap();
    assert!(o.status.success(), "arreglado de verdad: {}", String::from_utf8_lossy(&o.stdout));
    let ahora = std::fs::read_to_string(raiz.join("pila.js")).unwrap();
    assert!(ahora.contains("// la cima de la pila") && ahora.contains("module.exports"), "sólo ese cambio: {ahora}");
}

/// F16.6: un PR con el CI en rojo queda en verde SIN fusionarse solo (GitHub falso local; nada sale a GitHub).
#[tokio::test]
#[ignore]
async fn un_pr_en_rojo_queda_en_verde_con_el_modelo() {
    use std::sync::atomic::Ordering;
    let Some((m, modelo)) = motor("GOTV_PRUEBA_OPENAI", "GOTV_PRUEBA_MODELO") else { return };
    let fuera = crate::CarpetaPrueba::nueva("gotv-f16-pr-real-remoto");
    let mut c = agente_real("pr-real", Mode::Default, m);
    *c.agente.front.respuesta.lock().unwrap() = Some(AskAnswer::Yes);
    let raiz = c.raiz.to_path_buf();
    let bare = super::prueba::repo_del_pr(&raiz, &fuera);
    let (dir, fusiones) = super::prueba::github_falso(bare.clone()).await;
    let mut s = super::pr::Seguimiento::nuevo(&raiz, "7 --arreglar --repo yo/repo").unwrap();
    s.base = format!("http://{dir}");
    s.token = None;
    s.ritmo = Duration::from_millis(200);
    s.espera = Duration::from_secs(600);
    let t = Instant::now();
    let r = super::pr::seguir(&mut c.agente, s).await;
    eprintln!("{modelo} · {} s · {r}\nrastro:\n{}", t.elapsed().as_secs(), rastro(&c.agente.history));
    assert!(r.contains("PR #7") && (r.contains("en verde") || r.contains("is green")), "{r}");
    assert_eq!(fusiones.load(Ordering::SeqCst), 0, "jamás se pidió fusionar");
    assert_eq!(super::prueba::git(&bare, &["show", "refs/heads/arreglo:calc.txt"]), "2+2=4", "el arreglo llegó a la rama del PR");
    let preguntas = c.agente.front.preguntas.lock().unwrap();
    eprintln!("permisos pedidos: {}", preguntas.len());
    assert!(preguntas.iter().any(|p| p.contains("EMPUJAR") || p.contains("PUSH")), "empujar pidió permiso: {preguntas:?}");
}
