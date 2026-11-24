//! F12 · pasos 3 y 4 · `/objetivo <condición>`: el agente sigue, ronda tras ronda, hasta que la condición se
//! cumple, sin que el Portador escriba «sigue».
//!
//! Tras cada turno (una ronda = un `Agent::run`) la comprueba un EVALUADOR:
//! - **por pruebas** (paso 4, tras F15.1): si la condición habla de pruebas y el proyecto tiene con qué
//!   correrlas, GOTV corre la suite ENTERA (`pruebas::herramienta::correr`, en la caja de la sesión) y manda
//!   el resultado real: se cumple con 0 fallos y al menos una prueba pasada. La palabra del modelo no cuenta.
//! - **por texto** (paso 3): primero la comprobación de cierre de F0.10 (`objecion_al_cierre`: la última
//!   corrida de pruebas del turno en rojo, o código cambiado después de ella → no se cumple, sin preguntar);
//!   si nada objeta, una consulta corta y aparte (sobre una COPIA de la conversación) le pide al modelo un
//!   JSON `{cumplido, motivo}` contra lo que DEVOLVIERON las herramientas. Es más débil y así se dice.
//!
//! Topes (trampa 2): de rondas y de tiempo; el evaluador que falla dos veces seguidas detiene y avisa; el
//! Portador que escribe (o pulsa detener) lo para entre una ronda y la siguiente.
use crate::frontend::Frontend;
use crate::pruebas::corrida::{Corredor, Que};
use crate::Agent;
use anyhow::Result;
use gotv_engine::Engine;
use std::time::{Duration, Instant};

/// Rondas por omisión (a ojo, sin medir con modelo: cada ronda es un turno entero con su propio tope de pasos).
pub const TOPE_RONDAS: u32 = 10;
/// Minutos por omisión.
pub const TOPE_MINUTOS: u64 = 60;
/// Lo más que puede tardar la suite en el evaluador por pruebas.
const TOPE_SUITE: Duration = Duration::from_secs(600);
/// Fallos SEGUIDOS del evaluador que detienen el objetivo.
const FALLOS_QUE_PARAN: u32 = 2;
/// Lo que escribe el evaluador por texto.
const EVALUADOR_MAX_TOKENS: u32 = 300;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Evaluador {
    Pruebas,
    Texto,
}

/// Lo que dijo el evaluador.
#[derive(Debug, Clone, PartialEq)]
pub enum Veredicto {
    Cumplido(String),
    Falta(String),
    /// el evaluador no pudo decir (sin respuesta, JSON roto, la suite no se pudo correr)
    Fallo(String),
}

/// Qué hacer tras una ronda.
#[derive(Debug, Clone, PartialEq)]
pub enum Paso {
    /// otra ronda: la orden para el modelo y lo que se ve en el chat
    Seguir { orden: String, visible: String },
    /// se acabó: el informe para el Portador
    Fin(String),
}

#[derive(Debug, Clone)]
pub struct Objetivo {
    pub condicion: String,
    pub tope_rondas: u32,
    pub tope: Duration,
    /// la condición habla de pruebas (o se pidió `--pruebas`); `--texto` lo apaga
    pub por_pruebas: bool,
    pub rondas: u32,
    /// el evaluador de la última comprobación
    pub ultimo: Option<Evaluador>,
    inicio: Instant,
    fallos_seguidos: u32,
}

/// Sin tildes y en minúsculas.
fn plano(s: &str) -> String {
    s.to_lowercase().chars().map(|c| match c {
        'á' | 'à' | 'ä' => 'a',
        'é' | 'è' | 'ë' => 'e',
        'í' | 'ì' | 'ï' => 'i',
        'ó' | 'ò' | 'ö' => 'o',
        'ú' | 'ù' | 'ü' => 'u',
        c => c,
    }).collect()
}

/// ¿La condición habla de pruebas? («todas las pruebas pasan», «tests pass», «la suite en verde»…)
pub fn habla_de_pruebas(condicion: &str) -> bool {
    plano(condicion).split(|c: char| !c.is_alphanumeric()).any(|w| {
        // juez-idioma: datos (palabras que se reconocen, en los dos idiomas)
        w.starts_with("prueba") || w.starts_with("test") || matches!(w, "suite" | "verde" | "green" | "spec" | "specs")
    })
}

impl Objetivo {
    /// `[--turnos N] [--minutos M] [--pruebas | --texto] condición` (también `--turns`, `--minutes`, `--tests`, `--text`).
    pub fn nuevo(arg: &str) -> Result<Objetivo> {
        let mut o = Objetivo { condicion: String::new(), tope_rondas: TOPE_RONDAS, tope: Duration::from_secs(TOPE_MINUTOS * 60), por_pruebas: false, rondas: 0, ultimo: None, inicio: Instant::now(), fallos_seguidos: 0 };
        let mut forzado: Option<bool> = None;
        let mut palabras = arg.split_whitespace().peekable();
        let mut resto = Vec::new();
        while let Some(w) = palabras.next() {
            // juez-idioma: datos (las opciones en los dos idiomas)
            match w {
                "--turnos" | "--turns" | "--rondas" => o.tope_rondas = palabras.next().and_then(|n| n.parse().ok()).filter(|n| *n > 0).ok_or_else(|| gotv_engine::anyhow_tr!("--turnos lleva un número mayor que 0", "--turns takes a number above 0"))?,
                "--minutos" | "--minutes" => o.tope = palabras.next().and_then(|n| n.parse::<u64>().ok()).filter(|n| *n > 0).map(|m| Duration::from_secs(m * 60)).ok_or_else(|| gotv_engine::anyhow_tr!("--minutos lleva un número mayor que 0", "--minutes takes a number above 0"))?,
                "--pruebas" | "--tests" => forzado = Some(true),
                "--texto" | "--text" => forzado = Some(false),
                _ => resto.push(w),
            }
        }
        o.condicion = resto.join(" ");
        if o.condicion.is_empty() {
            gotv_engine::bail_tr!(
                "uso: /objetivo [--turnos N] [--minutos M] [--pruebas|--texto] <condición comprobable> — p. ej. /objetivo todas las pruebas pasan",
                "usage: /goal [--turns N] [--minutes M] [--tests|--text] <checkable condition> — e.g. /goal all tests pass"
            );
        }
        o.por_pruebas = forzado.unwrap_or_else(|| habla_de_pruebas(&o.condicion));
        Ok(o)
    }

    fn nombre_del_evaluador(&self, e: Evaluador, root: &std::path::Path) -> String {
        match e {
            Evaluador::Pruebas => {
                let c: Vec<&str> = Corredor::detectar(root).iter().map(|c| c.nombre()).collect();
                crate::tr!("las pruebas reales ({})", "the real tests ({})", c.join(", "))
            }
            Evaluador::Texto => crate::i18n::t(
                "por texto: el modelo, aparte, contra lo que devolvieron las herramientas (más débil que correr las pruebas)",
                "by text: the model, on the side, against what the tools returned (weaker than running the tests)",
            )
            .to_string(),
        }
    }

    /// El evaluador de esta ronda: por pruebas si se pidió y HAY con qué correrlas (en una carpeta vacía
    /// aparecen cuando el modelo crea el proyecto).
    fn evaluador(&self, root: &std::path::Path) -> Evaluador {
        if self.por_pruebas && !Corredor::detectar(root).is_empty() {
            Evaluador::Pruebas
        } else {
            Evaluador::Texto
        }
    }

    /// La línea que anuncia el objetivo.
    pub fn cabecera(&self, root: &std::path::Path) -> String {
        crate::tr!(
            "🎯 /objetivo «{}» · hasta {} rondas o {} min · se comprueba con {}",
            "🎯 /goal «{}» · up to {} rounds or {} min · checked with {}",
            self.condicion,
            self.tope_rondas,
            self.tope.as_secs() / 60,
            self.nombre_del_evaluador(self.evaluador(root), root)
        )
    }

    /// La primera orden: `(para el modelo, lo que se ve en el chat)`.
    pub fn primera_orden(&self, escrito: &str) -> (String, String) {
        let como = if self.por_pruebas {
            crate::i18n::t("corriendo las pruebas del proyecto", "by running the project's tests")
        } else {
            crate::i18n::t("contra lo que devuelvan las herramientas", "against what the tools return")
        };
        (
            crate::tr!(
                "Objetivo: «{}». Trabaja hasta que se cumpla. Al final de cada turno GOTV lo comprueba {como} y, si falta algo, te dice qué y sigues.",
                "Goal: «{}». Work until it holds. At the end of each turn GOTV checks it {como} and, if something is missing, tells you what and you continue.",
                self.condicion
            ),
            escrito.trim().to_string(),
        )
    }

    fn informe(&self, icono: &str, por_que: &str, root: &std::path::Path) -> String {
        let ev = self.ultimo.map(|e| self.nombre_del_evaluador(e, root)).unwrap_or_else(|| "—".into());
        crate::tr!(
            "{icono} /objetivo «{}»: {por_que}\n({} ronda(s) en {:.1} min · evaluador: {ev})",
            "{icono} /goal «{}»: {por_que}\n({} round(s) in {:.1} min · checker: {ev})",
            self.condicion,
            self.rondas,
            self.inicio.elapsed().as_secs_f32() / 60.0
        )
    }

    /// Tras una ronda: `respuesta` es lo que contestó el turno (o por qué no terminó), `desde` dónde empezó
    /// en `history`, y `escribio` si el Portador escribió mientras tanto.
    pub async fn tras_ronda<E: Engine, F: Frontend>(&mut self, agent: &mut Agent<E, F>, respuesta: std::result::Result<&str, &str>, desde: usize, escribio: bool) -> Paso {
        self.rondas += 1;
        let root = agent.root.clone();
        let respuesta = match respuesta {
            Ok(r) => r,
            Err(e) => return Paso::Fin(self.informe("⚠", &crate::tr!("la ronda {} no terminó ({e}); se para aquí", "round {} did not finish ({e}); stopping here", self.rondas), &root)),
        };
        if respuesta == crate::agent::interrupted() {
            return Paso::Fin(self.informe("⏹", crate::i18n::t("detenido por el Portador", "stopped by the user"), &root));
        }
        if escribio {
            return Paso::Fin(self.informe("✋", crate::i18n::t("el Portador escribió: el objetivo se para y va lo que escribiste", "the user wrote: the goal stops and what you wrote goes next"), &root));
        }
        let e = self.evaluador(&root);
        self.ultimo = Some(e);
        agent.front.activity("step", crate::i18n::t("comprobando el objetivo", "checking the goal"));
        let v = match e {
            Evaluador::Pruebas => evaluar_por_pruebas(agent).await,
            Evaluador::Texto => evaluar_por_texto(agent, &self.condicion, desde).await,
        };
        let motivo = match v {
            Veredicto::Cumplido(m) => return Paso::Fin(self.informe("✅", &crate::tr!("cumplido — {m}", "achieved — {m}"), &root)),
            Veredicto::Falta(m) => {
                self.fallos_seguidos = 0;
                m
            }
            Veredicto::Fallo(m) => {
                self.fallos_seguidos += 1;
                if self.fallos_seguidos >= FALLOS_QUE_PARAN {
                    return Paso::Fin(self.informe(
                        "⚠",
                        &crate::tr!(
                            "el evaluador falló {} veces seguidas ({m}): GOTV para para no girar sin saber si se cumplió",
                            "the checker failed {} times in a row ({m}): GOTV stops so as not to spin without knowing whether it holds",
                            self.fallos_seguidos
                        ),
                        &root,
                    ));
                }
                crate::tr!("no se pudo comprobar ({m})", "it could not be checked ({m})")
            }
        };
        if self.rondas >= self.tope_rondas {
            return Paso::Fin(self.informe("⏱", &crate::tr!("tope de {} rondas sin cumplirse. Lo último medido: {motivo}", "limit of {} rounds reached without it holding. Last measured: {motivo}", self.tope_rondas), &root));
        }
        if self.inicio.elapsed() >= self.tope {
            return Paso::Fin(self.informe("⏱", &crate::tr!("tope de {} min sin cumplirse. Lo último medido: {motivo}", "limit of {} min reached without it holding. Last measured: {motivo}", self.tope.as_secs() / 60), &root));
        }
        let siguiente = self.rondas + 1;
        let corto: String = motivo.lines().next().unwrap_or("").chars().take(140).collect();
        Paso::Seguir {
            orden: crate::tr!(
                "GOTV · /objetivo, ronda {siguiente} de {}: «{}» aún NO se cumple. Lo que midió GOTV:\n{motivo}\nSigue trabajando hasta cumplirlo; al final de este turno se vuelve a comprobar.",
                "GOTV · /goal, round {siguiente} of {}: «{}» does NOT hold yet. What GOTV measured:\n{motivo}\nKeep working until it holds; it is checked again at the end of this turn.",
                self.tope_rondas,
                self.condicion
            ),
            visible: crate::tr!("↻ /objetivo · ronda {siguiente}/{} · {corto}", "↻ /goal · round {siguiente}/{} · {corto}", self.tope_rondas),
        }
    }
}

/// Paso 4: la suite entera, con cada corredor del proyecto, en la caja de la sesión.
pub async fn evaluar_por_pruebas<E: Engine, F: Frontend>(agent: &Agent<E, F>) -> Veredicto {
    let corredores = Corredor::detectar(&agent.root);
    if corredores.is_empty() {
        return Veredicto::Fallo(crate::i18n::t("el proyecto no tiene con qué correr pruebas", "the project has nothing to run tests with").into());
    }
    let cx = crate::pruebas::herramienta::Contexto { root: &agent.root, spec: agent.sandbox_spec(), tocados: &[], usan: vec![] };
    let (mut pasaron, mut fallos) = (0u32, Vec::new());
    for c in &corredores {
        let r = crate::pruebas::herramienta::correr(&cx, c, &Que::Todas, TOPE_SUITE).await;
        pasaron += r.pasaron;
        fallos.extend(r.fallaron);
    }
    let nombres: Vec<&str> = corredores.iter().map(|c| c.nombre()).collect();
    if fallos.is_empty() && pasaron > 0 {
        return Veredicto::Cumplido(crate::tr!("{pasaron} prueba(s) pasan y ninguna falla ({})", "{pasaron} test(s) pass and none fails ({})", nombres.join(", ")));
    }
    if fallos.is_empty() {
        return Veredicto::Falta(crate::tr!("no corrió ninguna prueba ({})", "no test ran ({})", nombres.join(", ")));
    }
    let mut s = crate::tr!("fallan {} y pasan {pasaron} ({}):", "{} fail and {pasaron} pass ({}):", fallos.len(), nombres.join(", "));
    for f in fallos.iter().take(8) {
        let donde = match (&f.archivo, f.linea) {
            (Some(a), Some(l)) => format!(" — {}:{l}", a.strip_prefix(&agent.root).unwrap_or(a).display()),
            (Some(a), None) => format!(" — {}", a.strip_prefix(&agent.root).unwrap_or(a).display()),
            _ => String::new(),
        };
        let m: String = f.mensaje.lines().next().unwrap_or("").chars().take(160).collect();
        s.push_str(&format!("\n✗ {}{donde} {m}", f.nombre));
    }
    if fallos.len() > 8 {
        s.push_str(&crate::tr!("\n… y {} más", "\n… and {} more", fallos.len() - 8));
    }
    Veredicto::Falta(s)
}

/// Paso 3: lo medido por F0.10 primero; si nada objeta, el modelo aparte con un JSON fijo.
pub async fn evaluar_por_texto<E: Engine, F: Frontend>(agent: &Agent<E, F>, condicion: &str, desde: usize) -> Veredicto {
    let del_turno = &agent.history[desde.min(agent.history.len())..];
    if let Some(objecion) = crate::agent::objecion_al_cierre(del_turno, &[]) {
        return Veredicto::Falta(objecion);
    }
    let pedido = crate::tr!(
        "GOTV · comprobación del objetivo «{condicion}». Mira lo que DE VERDAD devolvieron las herramientas en esta conversación, no lo que se dijo. ¿Se cumple YA? Responde SÓLO un objeto JSON: {{\"cumplido\": true | false, \"motivo\": \"una frase con la evidencia\"}}",
        "GOTV · goal check «{condicion}». Look at what the tools REALLY returned in this conversation, not at what was said. Does it hold NOW? Answer ONLY a JSON object: {{\"cumplido\": true | false, \"motivo\": \"one sentence with the evidence\"}}"
    );
    let r = match super::generar_aparte(agent, pedido, EVALUADOR_MAX_TOKENS).await {
        Ok(r) => r,
        Err(e) => return Veredicto::Fallo(e.to_string()),
    };
    let Some(v) = super::objeto_json(&r.content) else {
        let t: String = super::sin_pensamiento(&r.content).chars().take(120).collect();
        return Veredicto::Fallo(crate::tr!("respuesta ilegible: «{t}»", "unreadable answer: «{t}»"));
    };
    let motivo = v["motivo"].as_str().unwrap_or("").trim().to_string();
    match v["cumplido"].as_bool().or_else(|| v["cumplido"].as_str().map(|s| matches!(plano(s).as_str(), "true" | "si" | "yes"))) {
        Some(true) => Veredicto::Cumplido(motivo),
        Some(false) => Veredicto::Falta(if motivo.is_empty() { crate::i18n::t("el modelo dice que aún no", "the model says not yet").into() } else { motivo }),
        None => Veredicto::Fallo(crate::i18n::t("el JSON no dice «cumplido»", "the JSON does not say «cumplido»").into()),
    }
}

/// `/objetivo` en la CLI y la TUI: ronda tras ronda con `Agent::run`. `parar` dice si el Portador escribió.
pub async fn perseguir<E: Engine, F: Frontend, P: FnMut() -> bool + Send>(agent: &mut Agent<E, F>, arg: &str, mut parar: P) -> Result<String> {
    let mut o = Objetivo::nuevo(arg)?;
    agent.front.log(o.cabecera(&agent.root)).await;
    let escrito = crate::tr!("/objetivo {arg}", "/goal {arg}");
    let (mut orden, mut visible) = o.primera_orden(&escrito);
    loop {
        let desde = agent.history.len();
        agent.visible = Some(visible);
        let r = agent.run(&orden).await;
        let respuesta = r.as_deref().map_err(|e| e.to_string());
        let paso = o.tras_ronda(agent, respuesta.as_deref().map_err(|e| e.as_str()), desde, parar()).await;
        match paso {
            Paso::Seguir { orden: o2, visible: v2 } => {
                agent.front.log(v2.clone()).await;
                orden = o2;
                visible = v2;
            }
            Paso::Fin(informe) => return Ok(informe),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::prueba::{agente, es_aparte, llamadas, texto, Guion, Mudo};
    use super::*;
    use crate::permissions::Mode;
    use serde_json::json;

    fn evaluaciones(a: &Agent<Guion, Mudo>) -> usize {
        a.engine.visto.lock().unwrap().iter().filter(|r| es_aparte(r)).count()
    }

    #[test]
    fn opciones_y_evaluador_elegido() {
        let o = Objetivo::nuevo("--turnos 3 --minutos 5 todas las pruebas pasan").unwrap();
        assert_eq!((o.tope_rondas, o.tope, o.condicion.as_str(), o.por_pruebas), (3, Duration::from_secs(300), "todas las pruebas pasan", true));
        assert!(Objetivo::nuevo("the tests pass").unwrap().por_pruebas);
        assert!(!Objetivo::nuevo("el README explica cómo instalar").unwrap().por_pruebas);
        assert!(!Objetivo::nuevo("--texto la suite en verde").unwrap().por_pruebas);
        assert!(Objetivo::nuevo("--pruebas compila sin avisos").unwrap().por_pruebas);
        assert!(Objetivo::nuevo("").is_err() && Objetivo::nuevo("--turnos 0 x").is_err() && Objetivo::nuevo("--turnos").is_err());
        // sin corredor en la carpeta, aunque hable de pruebas, se comprueba por texto (y así lo dice)
        let d = crate::CarpetaPrueba::nueva("gotv-objetivo-vacio");
        assert_eq!(o.evaluador(&d), Evaluador::Texto);
        assert!(o.cabecera(&d).contains("más débil") || o.cabecera(&d).contains("weaker"));
    }

    /// Para al cumplirse: la primera ronda no basta (el evaluador dice qué falta y eso llega como orden), la
    /// segunda sí. El evaluador va APARTE: su pregunta no queda en el historial.
    #[tokio::test]
    async fn para_al_cumplirse() {
        let mut c = agente("obj-cumple", Mode::Default, vec![texto("hice la mitad"), texto("hice el resto")], vec![r#"{"cumplido": false, "motivo": "falta el README"}"#, r#"```json
{"cumplido": true, "motivo": "README.md existe"}
```"#]);
        let r = perseguir(&mut c.agente, "el README explica cómo instalar", || false).await.unwrap();
        assert!(r.starts_with("✅") && r.contains("README.md existe") && r.contains("2 ronda"), "{r}");
        assert_eq!(evaluaciones(&c.agente), 2);
        let ordenes: Vec<String> = c.agente.history.iter().filter(|m| m.role == "user" && !m.es_aviso()).map(|m| m.content.clone()).collect();
        assert_eq!(ordenes.len(), 2, "dos rondas, ningún «sigue» del Portador: {ordenes:?}");
        assert!(ordenes[1].contains("falta el README"), "lo que falta llega al modelo: {}", ordenes[1]);
        assert!(!c.agente.history.iter().any(|m| m.content.contains("comprobación del objetivo")), "el evaluador no entra al historial");
        assert!(crate::mensajes::visible(c.agente.history.iter().find(|m| m.role == "user").unwrap()).starts_with("/objetivo"));
    }

    #[tokio::test]
    async fn para_al_llegar_al_tope_de_rondas() {
        let no = r#"{"cumplido": false, "motivo": "aún no"}"#;
        let mut c = agente("obj-tope", Mode::Default, vec![], vec![no; 5]);
        let r = perseguir(&mut c.agente, "--turnos 3 algo imposible", || false).await.unwrap();
        assert!(r.starts_with("⏱") && r.contains("3 ronda") && r.contains("aún no"), "{r}");
        assert_eq!(evaluaciones(&c.agente), 3);
    }

    #[tokio::test]
    async fn para_cuando_el_portador_escribe() {
        let mut c = agente("obj-escribe", Mode::Default, vec![], vec![r#"{"cumplido": false, "motivo": "x"}"#; 3]);
        let mut veces = 0;
        let r = perseguir(&mut c.agente, "algo largo", || {
            veces += 1;
            veces >= 2
        })
        .await
        .unwrap();
        assert!(r.starts_with("✋") && r.contains("2 ronda"), "{r}");
        assert_eq!(evaluaciones(&c.agente), 1, "tras escribir no se evalúa: se para");
    }

    #[tokio::test]
    async fn detener_el_turno_para_el_objetivo() {
        let mut c = agente("obj-stop", Mode::Default, vec![], vec![]);
        c.agente.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        let mut o = Objetivo::nuevo("algo").unwrap();
        let p = o.tras_ronda(&mut c.agente, Ok(crate::agent::interrupted()), 0, false).await;
        assert!(matches!(p, Paso::Fin(ref t) if t.starts_with("⏹")), "{p:?}");
    }

    /// Trampa 2: un evaluador que no sabe responder no deja al agente en bucle (tope de 10, para en 2).
    #[tokio::test]
    async fn un_evaluador_que_falla_no_deja_al_agente_en_bucle() {
        let mut c = agente("obj-falla", Mode::Default, vec![], vec!["claro que sí", "no sé"]);
        let r = perseguir(&mut c.agente, "algo", || false).await.unwrap();
        assert!(r.starts_with("⚠") && r.contains("2 ronda") && r.contains("2 "), "{r}");
        assert_eq!(evaluaciones(&c.agente), 2);
        // un fallo suelto no para: falla, luego dice que falta, luego cumple
        let mut c = agente("obj-falla-1", Mode::Default, vec![], vec!["???", r#"{"cumplido": false, "motivo": "a"}"#, "???", r#"{"cumplido": true, "motivo": "b"}"#]);
        let r = perseguir(&mut c.agente, "algo", || false).await.unwrap();
        assert!(r.starts_with("✅") && r.contains("4 ronda"), "{r}");
    }

    /// F0.10 manda sobre la palabra del modelo: con su última corrida de pruebas en ROJO no se le pregunta.
    #[tokio::test]
    async fn lo_medido_manda_sobre_la_palabra_del_modelo() {
        let rojo = "running 2 tests\ntest a ... ok\ntest b ... FAILED\n\nfailures:\n    b\n\ntest result: FAILED. 1 passed; 1 failed; 0 ignored";
        let mut c = agente("obj-rojo", Mode::Bypass, vec![llamadas(&[("bash", json!({"cmd": format!("printf '{}'", rojo.replace('\n', "\\n"))}))]), texto("Listo: todas las pruebas pasan.")], vec![]);
        let mut o = Objetivo::nuevo("--texto todo funciona").unwrap();
        let desde = c.agente.history.len();
        let a = c.agente.run("arregla b").await.unwrap();
        let p = o.tras_ronda(&mut c.agente, Ok(&a), desde, false).await;
        assert_eq!(evaluaciones(&c.agente), 0, "no hizo falta preguntarle");
        assert!(matches!(&p, Paso::Seguir { orden, .. } if orden.contains("ROJO") || orden.contains("RED")), "{p:?}");
    }

    /// El «hecho cuando» de la ficha, sin modelo (motor de guion) y con una suite REAL de `cargo test`: 4
    /// pruebas en rojo; la ronda 1 arregla dos y cierra; GOTV corre la suite, ve 2 en rojo y sigue solo; la ronda 2 arregla las otras dos; la suite queda en verde y el objetivo para.
    /// Nadie escribe «sigue».
    #[tokio::test]
    async fn objetivo_deja_en_verde_una_suite_con_4_fallos_sin_que_nadie_escriba_sigue() {
        if !crate::entorno::en_path("cargo") {
            return;
        }
        let malo = "pub fn doble(x: i32) -> i32 { x * 3 }\npub fn triple(x: i32) -> i32 { x * 4 }\npub fn mitad(x: i32) -> i32 { x / 3 }\npub fn cero() -> i32 { 1 }\n\n#[cfg(test)]\nmod tests {\n    use super::*;\n    #[test]\n    fn t_doble() { assert_eq!(doble(2), 4); }\n    #[test]\n    fn t_triple() { assert_eq!(triple(2), 6); }\n    #[test]\n    fn t_mitad() { assert_eq!(mitad(8), 4); }\n    #[test]\n    fn t_cero() { assert_eq!(cero(), 0); }\n}\n";
        let medio = malo.replace("x * 3 }", "x * 2 }").replace("x * 4 }", "x * 3 }");
        let bueno = medio.replace("x / 3 }", "x / 2 }").replace("{ 1 }", "{ 0 }");
        let mut c = agente(
            "obj-suite",
            Mode::Bypass,
            vec![
                llamadas(&[("read_file", json!({"path": "src/lib.rs"}))]),
                llamadas(&[("write_file", json!({"path": "src/lib.rs", "content": medio}))]),
                texto("Arreglé doble y triple."),
                llamadas(&[("read_file", json!({"path": "src/lib.rs"}))]),
                llamadas(&[("write_file", json!({"path": "src/lib.rs", "content": bueno}))]),
                texto("Arreglé mitad y cero."),
            ],
            vec![],
        );
        let r = c.agente.root.clone();
        std::fs::create_dir_all(r.join("src")).unwrap();
        std::fs::write(r.join("Cargo.toml"), "[package]\nname = \"objetivo_juguete\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[workspace]\n").unwrap();
        std::fs::write(r.join("src/lib.rs"), malo).unwrap();
        // como `cargo_de_verdad` de F15.1: la suite fuera de la caja (en la caja, cargo escribiría en el
        // CARGO_TARGET_DIR de quien corre las pruebas, que cae fuera del proyecto)
        c.agente.perms.sandbox = false;
        let informe = perseguir(&mut c.agente, "todas las pruebas pasan", || false).await.unwrap();
        assert!(informe.starts_with("✅") && informe.contains("2 ronda") && informe.contains("4 prueba(s) pasan"), "{informe}");
        assert_eq!(std::fs::read_to_string(r.join("src/lib.rs")).unwrap(), bueno);
        let ordenes: Vec<String> = c.agente.history.iter().filter(|m| m.role == "user" && !m.es_aviso()).map(|m| m.content.clone()).collect();
        assert_eq!(ordenes.len(), 2, "{ordenes:?}");
        assert!(ordenes[1].contains("t_mitad") && ordenes[1].contains("t_cero") && !ordenes[1].contains("t_doble"), "la ronda 2 recibe lo que REALMENTE falla: {}", ordenes[1]);
        assert_eq!(evaluaciones(&c.agente), 0, "con pruebas, la palabra del modelo no se consulta");
    }
}
