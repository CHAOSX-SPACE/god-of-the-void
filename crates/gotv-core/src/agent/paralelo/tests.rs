//! F9 · pruebas de los subagentes a la vez, con un motor de guion que contesta POR CONVERSACIÓN (la marca
//! de cada pieza en su pedido elige su guion): varios generan a la vez y el orden de llegada no importa.
//! Todo este archivo es código de prueba (`#[cfg(test)]` en `agent/paralelo.rs`): el juez de idioma no lo mira.

use super::super::Agent;
use super::FrenteHijo;
use crate::checkpoint::Checkpoint;
use crate::enjambre::{buzon, tablero};
use crate::formas::prueba::{llamadas, texto, Mudo};
use crate::permissions::{AskAnswer, Mode, Permissions};
use anyhow::Result;
use async_trait::async_trait;
use gotv_engine::{Capabilities, ChatRequest, ChatResponse, Engine};
use serde_json::json;
use std::collections::HashMap;
use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Motor de guion por conversación. `nube` = se hace pasar por un motor de nube; `lote` = lo que declara
/// generar en lote; cuenta cuántas peticiones tuvo EN VUELO a la vez (lo que mide el paralelismo).
struct PorPieza {
    guiones: Mutex<HashMap<String, Vec<ChatResponse>>>,
    vistos: Mutex<Vec<(String, ChatRequest)>>,
    fallan: Mutex<Vec<String>>,
    demora: Duration,
    nube: bool,
    lote: usize,
    en_vuelo: AtomicUsize,
    max_en_vuelo: AtomicUsize,
    /// (marca, desde, hasta) de cada petición: el reloj que pasó generando cada conversación
    tramos: Mutex<Vec<(String, Instant, Instant)>>,
}

impl PorPieza {
    fn nuevo(nube: bool, lote: usize, demora_ms: u64) -> PorPieza {
        PorPieza { guiones: Mutex::default(), vistos: Mutex::default(), fallan: Mutex::default(), demora: Duration::from_millis(demora_ms), nube, lote, en_vuelo: AtomicUsize::new(0), max_en_vuelo: AtomicUsize::new(0), tramos: Mutex::default() }
    }
    /// El reloj durante el que AL MENOS un subagente esperaba al motor (la unión de sus tramos): en serie es
    /// la suma; a la vez, la del más largo.
    fn reloj_de_subagentes(&self) -> Duration {
        let mut t: Vec<(Instant, Instant)> = self.tramos.lock().unwrap().iter().filter(|(m, _, _)| m != "principal").map(|(_, a, b)| (*a, *b)).collect();
        t.sort();
        let mut total = Duration::ZERO;
        let mut actual: Option<(Instant, Instant)> = None;
        for (a, b) in t {
            actual = match actual {
                Some((x, y)) if a <= y => Some((x, y.max(b))),
                Some((x, y)) => {
                    total += y - x;
                    Some((a, b))
                }
                None => Some((a, b)),
            };
        }
        if let Some((x, y)) = actual {
            total += y - x;
        }
        total
    }
    fn guion(&self, marca: &str, r: Vec<ChatResponse>) {
        self.guiones.lock().unwrap().insert(marca.into(), r);
    }
    /// Todo el texto que vio cada marca.
    fn visto_por(&self, marca: &str) -> String {
        self.vistos.lock().unwrap().iter().filter(|(m, _)| m == marca).map(|(_, r)| serde_json::to_string(&r.messages).unwrap()).collect::<Vec<_>>().join("\n")
    }
    fn max(&self) -> usize {
        self.max_en_vuelo.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl Engine for PorPieza {
    fn name(&self) -> String {
        "guion por pieza".into()
    }
    fn capabilities(&self) -> Capabilities {
        Capabilities { text: true, vision: false, audio_in: false }
    }
    fn local(&self) -> bool {
        !self.nube
    }
    fn lote_max(&self) -> usize {
        self.lote
    }
    async fn start(&mut self) -> Result<()> {
        Ok(())
    }
    async fn stop(&mut self) -> Result<()> {
        Ok(())
    }
    async fn complete(&self, req: ChatRequest) -> Result<ChatResponse> {
        // el pedido de la conversación (el 2.º mensaje) lleva la marca de la pieza; sin marca, el principal
        let marcas: Vec<String> = self.guiones.lock().unwrap().keys().cloned().collect();
        let pedido = req.messages.get(1).map(|m| m.content.clone()).unwrap_or_default();
        let marca = marcas.iter().filter(|m| *m != "principal").find(|m| pedido.contains(m.as_str())).cloned().unwrap_or_else(|| "principal".into());
        let ahora = self.en_vuelo.fetch_add(1, Ordering::SeqCst) + 1;
        self.max_en_vuelo.fetch_max(ahora, Ordering::SeqCst);
        let desde = Instant::now();
        tokio::time::sleep(self.demora).await;
        self.tramos.lock().unwrap().push((marca.clone(), desde, Instant::now()));
        self.en_vuelo.fetch_sub(1, Ordering::SeqCst);
        let resumen = req.tools.is_none();
        self.vistos.lock().unwrap().push((marca.clone(), req));
        if resumen {
            return Ok(texto("RESUMEN: lo hecho"));
        }
        if self.fallan.lock().unwrap().contains(&marca) {
            anyhow::bail!("el proveedor no responde (prueba)");
        }
        let mut g = self.guiones.lock().unwrap();
        let cola = g.entry(marca).or_default();
        Ok(if cola.is_empty() { texto("fin") } else { cola.remove(0) })
    }
}

fn agente(raiz: &Path, motor: PorPieza, modo: Mode) -> Agent<PorPieza, Mudo> {
    let root = raiz.canonicalize().unwrap();
    let front = Mudo::default();
    *front.respuesta.lock().unwrap() = Some(AskAnswer::Yes);
    Agent {
        engine: motor,
        perms: Permissions::new(modo, &root, vec![], vec![], vec![]),
        checkpoint: Some(Checkpoint::new(&root)),
        root,
        max_steps: 12,
        max_tokens: 512,
        sampling: gotv_engine::Sampling::GREEDY,
        context_budget: 32768,
        front,
        history: vec![],
        session: None,
        cancel: Default::default(),
        mcp: None,
        attachments: Vec::new(),
        user_media: Vec::new(),
        chat_only: false,
        embedder: None,
        lsp: None,
        computer_use: false,
        procs: Default::default(),
        tools_loaded: Default::default(),
        bases_nube: Default::default(),
        guardia: Default::default(),
        perfil: Default::default(),
        pro: Default::default(),
        motor_dormido: false,
        visible: None,
        estado_de: None,
        calibre: None,
        recorto_ubicaciones: false,
        tope_paso: 0,
    }
}

fn git(d: &Path, args: &[&str]) -> String {
    let o = Command::new("git").arg("-C").arg(d).args(args).output().unwrap();
    assert!(o.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&o.stderr));
    String::from_utf8_lossy(&o.stdout).trim().to_string()
}

/// Un repo con un commit (`a.txt`) y `.gotv/` ignorada (como la deja GOTV en un proyecto).
fn repo(tag: &str) -> crate::CarpetaPrueba {
    let d = crate::CarpetaPrueba::nueva(&format!("gotv-f9-{tag}"));
    git(&d, &["init", "-q", "-b", "main"]);
    std::fs::write(d.join("a.txt"), "uno\n").unwrap();
    std::fs::write(d.join(".gitignore"), ".gotv/\n").unwrap();
    git(&d, &["add", "-A"]);
    git(&d, &["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "-m", "base"]);
    d
}

fn tarea(marca: &str, desc: &str) -> (&'static str, serde_json::Value) {
    ("task", json!({"description": desc, "prompt": format!("{marca}: haz tu parte")}))
}

fn escribe(path: &str, contenido: &str) -> ChatResponse {
    llamadas(&[("write_file", json!({"path": path, "content": contenido}))])
}

/// Las salidas de herramientas que vio el agente principal.
fn resultados(a: &Agent<PorPieza, Mudo>) -> Vec<String> {
    a.history.iter().filter(|m| m.role == "tool").map(|m| m.content.clone()).collect()
}

fn ramas_de_gotv(d: &Path) -> String {
    git(d, &["branch", "--list", "gotv/sub-*"])
}

#[tokio::test]
async fn dos_subagentes_a_la_vez_no_mezclan_conversaciones_y_traen_sus_cambios() {
    let d = repo("mezcla");
    let m = PorPieza::nuevo(true, 1, 60);
    m.guion("principal", vec![llamadas(&[tarea("PIEZA-A", "parte A"), tarea("PIEZA-B", "parte B")]), texto("listo: A y B")]);
    m.guion("PIEZA-A", vec![escribe("a2.txt", "de A\n"), texto("hice A en a2.txt")]);
    m.guion("PIEZA-B", vec![escribe("b2.txt", "de B\n"), texto("hice B en b2.txt")]);
    let mut a = agente(&d, m, Mode::Default);
    a.run("reparte el trabajo en dos").await.unwrap();
    // a la vez de verdad
    assert!(a.engine.max() >= 2, "en vuelo a la vez: {}", a.engine.max());
    // cada conversación sólo con lo suyo
    let (va, vb) = (a.engine.visto_por("PIEZA-A"), a.engine.visto_por("PIEZA-B"));
    assert!(va.contains("PIEZA-A") && !va.contains("PIEZA-B") && !va.contains("b2.txt"), "A vio lo de B");
    assert!(vb.contains("PIEZA-B") && !vb.contains("PIEZA-A") && !vb.contains("a2.txt"), "B vio lo de A");
    // sus cambios, en el árbol del usuario, sin commit; sus ramas ya no hacen falta
    assert_eq!(std::fs::read_to_string(d.join("a2.txt")).unwrap(), "de A\n");
    assert_eq!(std::fs::read_to_string(d.join("b2.txt")).unwrap(), "de B\n");
    assert_eq!(git(&d, &["rev-list", "--count", "HEAD"]), "1");
    assert!(ramas_de_gotv(&d).is_empty(), "{}", ramas_de_gotv(&d));
    // al principal vuelven sólo los informes, en el orden pedido; no las llamadas de los subagentes
    let r = resultados(&a);
    assert!(r[0].contains("hice A") && r[1].contains("hice B"), "{r:?}");
    assert!(r[0].contains("✓"), "dice que entró: {}", r[0]);
    let llamadas_principal = serde_json::to_string(&a.history.iter().filter_map(|m| m.tool_calls.clone()).collect::<Vec<_>>()).unwrap();
    assert!(!llamadas_principal.contains("write_file"), "{llamadas_principal}");
    // el registro dice cómo corrieron
    let lineas = a.front.lineas.lock().unwrap().join("\n");
    assert!(lineas.contains("en paralelo") || lineas.contains("in parallel"), "{lineas}");
}

#[test]
fn cada_subagente_lleva_sus_permisos_y_su_raiz() {
    let d = crate::CarpetaPrueba::nueva("gotv-f9-permisos");
    let a = agente(&d, PorPieza::nuevo(true, 1, 0), Mode::Default);
    let turno = tokio::sync::Mutex::new(());
    let otra = d.join("otra");
    let f = |n: &str| FrenteHijo { padre: &a.front, nombre: n.into(), turno: &turno };
    let mut h1 = a.hijo(&a.engine, d.join("uno"), a.perms.clone(), None, f("sub-1"));
    let h2 = a.hijo(&a.engine, otra.clone(), a.perms.clone(), None, f("sub-2"));
    // lo que uno concede en su sesión no lo ve el otro ni el chat
    h1.perms.net_session.push("ejemplo.com".into());
    assert!(h2.perms.net_session.is_empty() && a.perms.net_session.is_empty());
    assert_eq!(h2.root, otra);
    assert_ne!(h1.proc_owner(), h2.proc_owner(), "sus procesos son suyos");
    assert!(h1.history.is_empty() && h1.session.is_none());
    assert!(h1.tools_loaded.contains("enjambre"));
}

#[tokio::test]
async fn un_subagente_que_falla_no_tumba_al_otro() {
    let d = repo("falla");
    let m = PorPieza::nuevo(true, 1, 20);
    m.guion("principal", vec![llamadas(&[tarea("PIEZA-A", "A"), tarea("PIEZA-B", "B")]), texto("seguí sin A")]);
    m.guion("PIEZA-A", vec![escribe("a2.txt", "x\n")]);
    m.guion("PIEZA-B", vec![escribe("b2.txt", "de B\n"), texto("hice B")]);
    m.fallan.lock().unwrap().push("PIEZA-A".into());
    let mut a = agente(&d, m, Mode::Default);
    let fin = a.run("reparte").await.unwrap();
    let r = resultados(&a);
    assert!(r[0].contains("ERROR del subagente") || r[0].contains("subagent ERROR"), "{}", r[0]);
    assert!(r[1].contains("hice B"), "{}", r[1]);
    assert_eq!(std::fs::read_to_string(d.join("b2.txt")).unwrap(), "de B\n");
    assert!(!d.join("a2.txt").exists());
    assert!(fin.contains("seguí sin A"), "el turno siguió: {fin}");
}

/// Tres piezas de dos pasos cada una, con 250 ms por respuesta del motor. Devuelve el reloj total, el reloj
/// que los subagentes pasaron esperando al motor, cuántos hubo a la vez como mucho y la carpeta.
async fn tres_piezas(tag: &str, nube: bool, lote: usize) -> (Duration, Duration, usize, crate::CarpetaPrueba) {
    let d = repo(tag);
    let m = PorPieza::nuevo(nube, lote, 250);
    m.guion("principal", vec![llamadas(&[tarea("PIEZA-1", "uno"), tarea("PIEZA-2", "dos"), tarea("PIEZA-3", "tres")]), texto("listo")]);
    for i in 1..=3 {
        m.guion(&format!("PIEZA-{i}"), vec![escribe(&format!("p{i}.txt"), &format!("{i}\n")), texto(&format!("hice {i}"))]);
    }
    let mut a = agente(&d, m, Mode::Default);
    let t0 = Instant::now();
    a.run("añade validación a los 3 manejadores").await.unwrap();
    let t = t0.elapsed();
    for i in 1..=3 {
        assert_eq!(std::fs::read_to_string(d.join(format!("p{i}.txt"))).unwrap(), format!("{i}\n"), "{tag}: falta p{i}");
    }
    (t, a.engine.reloj_de_subagentes(), a.engine.max(), d)
}

/// Multihilo, como la app y la CLI: cada subagente en su hilo (las demás pruebas corren en un runtime de un
/// hilo y cubren el camino de `join_all`).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn en_la_nube_en_paralelo_y_en_local_por_turnos_o_por_lotes() {
    let (t_nube, gen_nube, max_nube, _d1) = tres_piezas("nube", true, 1).await;
    let (t_local, gen_local, max_local, _d2) = tres_piezas("local", false, 1).await;
    let (t_lote, gen_lote, max_lote, _d3) = tres_piezas("lote", false, 4).await;
    eprintln!("reloj total: nube {t_nube:?} · local por turnos {t_local:?} · local por lotes {t_lote:?}");
    eprintln!("reloj generando (subagentes): nube {gen_nube:?} · por turnos {gen_local:?} · por lotes {gen_lote:?}");
    assert_eq!(max_nube, 3, "nube: los tres a la vez");
    assert_eq!(max_local, 1, "local sin lotes: de a uno");
    assert_eq!(max_lote, 3, "local con lotes: los tres en el mismo lote");
    // generando: por turnos 3×2 respuestas de 250 ms (1,5 s); a la vez, 2 (0,5 s). El reloj TOTAL suma además
    // las ramas (~0,3 s por rama en esta Mac, más con carga) y la máquina compartida lo vuelve ruidoso: el
    // total con un modelo de nube real lo mide el banco (`equipo` contra `equipo-serie`).
    assert!(gen_nube.as_secs_f32() < gen_local.as_secs_f32() * 0.5, "nube {gen_nube:?} vs por turnos {gen_local:?}");
    assert!(gen_lote.as_secs_f32() < gen_local.as_secs_f32() * 0.5, "lotes {gen_lote:?} vs por turnos {gen_local:?}");
}

#[tokio::test]
async fn dos_ramas_que_chocan_se_reportan_sin_fusionar() {
    let d = repo("choque");
    let m = PorPieza::nuevo(true, 1, 20);
    m.guion("principal", vec![llamadas(&[tarea("PIEZA-A", "A"), tarea("PIEZA-B", "B")]), texto("hubo un choque")]);
    m.guion("PIEZA-A", vec![escribe("a.txt", "de A\n"), texto("cambié a.txt")]);
    m.guion("PIEZA-B", vec![escribe("a.txt", "de B\n"), texto("también cambié a.txt")]);
    let mut a = agente(&d, m, Mode::Default);
    a.run("reparte").await.unwrap();
    let r = resultados(&a);
    assert!(r[0].contains("✓"), "{}", r[0]);
    assert!(r[1].contains("✗") && r[1].contains("a.txt"), "{}", r[1]);
    // entró la primera; la segunda NO, y su rama queda para revisarla
    assert_eq!(std::fs::read_to_string(d.join("a.txt")).unwrap(), "de A\n");
    let ramas = ramas_de_gotv(&d);
    assert_eq!(ramas.lines().count(), 1, "{ramas}");
    assert!(r[1].contains(ramas.trim_start_matches(['*', '+', ' '])), "el informe nombra la rama: {}", r[1]);
    assert!(!d.join(".git/MERGE_HEAD").exists());
}

#[tokio::test]
async fn sin_git_van_de_a_uno_en_el_arbol() {
    let d = crate::CarpetaPrueba::nueva("gotv-f9-sin-git");
    let m = PorPieza::nuevo(true, 1, 20);
    m.guion("principal", vec![llamadas(&[tarea("PIEZA-A", "A"), tarea("PIEZA-B", "B")]), texto("listo")]);
    m.guion("PIEZA-A", vec![escribe("a2.txt", "A\n"), texto("hice A")]);
    m.guion("PIEZA-B", vec![escribe("b2.txt", "B\n"), texto("hice B")]);
    let mut a = agente(&d, m, Mode::Default);
    a.run("reparte").await.unwrap();
    assert_eq!(a.engine.max(), 1, "sin ramas no se pisan: de a uno");
    assert!(d.join("a2.txt").exists() && d.join("b2.txt").exists());
    let lineas = a.front.lineas.lock().unwrap().join("\n");
    assert!(lineas.contains("sin repositorio git") || lineas.contains("without a git repository"), "{lineas}");
}

#[tokio::test]
async fn los_subagentes_escriben_en_el_tablero_del_chat_y_el_principal_lo_lee() {
    let d = repo("tablero");
    let m = PorPieza::nuevo(true, 1, 20);
    m.guion(
        "principal",
        vec![llamadas(&[tarea("PIEZA-A", "A"), tarea("PIEZA-B", "B")]), llamadas(&[("enjambre", json!({"accion": "tablero"}))]), texto("leí el tablero")],
    );
    m.guion("PIEZA-A", vec![llamadas(&[("enjambre", json!({"accion": "anotar", "texto": "toco sólo a2.txt"}))]), texto("anoté")]);
    m.guion("PIEZA-B", vec![llamadas(&[("enjambre", json!({"accion": "anotar", "texto": "toco sólo b2.txt"}))]), texto("anoté")]);
    let mut a = agente(&d, m, Mode::Default);
    a.run("reparte").await.unwrap();
    let r = resultados(&a);
    let leido = r.last().unwrap();
    assert!(leido.contains("[sub-1] toco sólo a2.txt") && leido.contains("[sub-2] toco sólo b2.txt"), "{leido}");
    // el tablero vive en la carpeta de ESTA conversación, en el proyecto (no en las ramas)
    let ruta = tablero::ruta(crate::artefactos::chat_de(&a.root, None, &a.proc_owner()).dir());
    assert_eq!(tablero::leer(&ruta).len(), 2);
    // ya no hay registro de los subagentes
    assert!(ramas_de_gotv(&d).is_empty());
}

#[tokio::test]
async fn el_verificador_va_en_otro_motor_y_no_edita() {
    let d = repo("verif");
    let a = agente(&d, PorPieza::nuevo(false, 1, 0), Mode::Default);
    let otro = PorPieza::nuevo(true, 1, 0);
    otro.guion("VERIF", vec![escribe("trampa.txt", "x\n"), texto("intenté editar y no pude\nVEREDICTO: PASA")]);
    let r = a.correr_verificador("VERIF: corre las pruebas", Some((&otro as &dyn Engine, 50_000)), &crate::hooks::Hooks::default()).await.unwrap();
    assert!(r.contains("VEREDICTO: PASA"), "{r}");
    assert!(a.engine.vistos.lock().unwrap().is_empty(), "el motor del chat no se usó");
    assert!(!d.join("trampa.txt").exists(), "el verificador no edita");
    let visto = otro.visto_por("VERIF");
    assert!(visto.contains("regla deny") || visto.contains("deny rule"), "su escritura se negó: {visto}");
}

#[tokio::test]
async fn el_verificador_por_la_herramienta_deja_su_veredicto_en_el_tablero() {
    let d = repo("verif-herr");
    let m = PorPieza::nuevo(true, 1, 0);
    m.guion("principal", vec![llamadas(&[("enjambre", json!({"accion": "verificar", "criterio": "VERIF2: corre node --test"}))]), texto("el verificador falló")]);
    m.guion("VERIF2", vec![texto("corrí node --test: 1 falla\nVEREDICTO: FALLA — prueba de suma")]);
    let mut a = agente(&d, m, Mode::Default);
    a.run("verifica").await.unwrap();
    let r = resultados(&a);
    assert!(r[0].contains("✗ FALLA") || r[0].contains("✗ FAIL"), "{}", r[0]);
    let ruta = tablero::ruta(crate::artefactos::chat_de(&a.root, None, &a.proc_owner()).dir());
    let notas = tablero::leer(&ruta);
    assert!(notas.iter().any(|n| n.texto.contains("FALLA") || n.texto.contains("FAIL")), "{notas:?}");
}

#[tokio::test]
async fn un_mensaje_de_otra_sesion_llega_entre_pasos() {
    let base = crate::CarpetaPrueba::nueva("gotv-f9-buzon");
    *buzon::BASE_PRUEBA.lock().unwrap() = Some(base.to_path_buf());
    let d = crate::CarpetaPrueba::nueva("gotv-f9-buzon-proyecto");
    let m = PorPieza::nuevo(true, 1, 0);
    m.guion("principal", vec![texto("visto")]);
    let mut a = agente(&d, m, Mode::Default);
    a.session = Some(crate::session::Session::open_new(&a.root, "guion").unwrap());
    let id = a.session.as_ref().unwrap().id;
    buzon::enviar(&base, &buzon::direccion(&a.root, id), "sesión #9", "MENSAJE-X: la API ya está").unwrap();
    a.run("sigue").await.unwrap();
    let visto = a.engine.visto_por("principal");
    assert!(visto.contains("MENSAJE-X: la API ya está"), "{visto}");
    // una sola vez: el buzón quedó vacío
    assert!(buzon::recoger(&base, &buzon::direccion(&a.root, id)).is_empty());
}

#[tokio::test]
async fn el_agente_principal_trabaja_en_su_rama_y_la_fusiona() {
    let d = repo("rama");
    let mut a = agente(&d, PorPieza::nuevo(true, 1, 0), Mode::Default);
    let raiz = a.root.clone();
    assert!(crate::formas::orden(&mut a, "/rama", || false).await.unwrap().unwrap().contains("sin rama") || a.root == raiz);
    let r = crate::formas::orden(&mut a, "/rama nueva probar algo", || false).await.unwrap().unwrap();
    assert!(r.contains("gotv/sub-"), "{r}");
    assert_ne!(a.root, raiz);
    assert!(a.checkpoint.is_none(), "su deshacer es la rama");
    std::fs::write(a.root.join("nuevo.txt"), "de la rama\n").unwrap();
    assert!(!raiz.join("nuevo.txt").exists(), "el árbol no cambia mientras tanto");
    assert!(crate::formas::orden(&mut a, "/rama nueva otra", || false).await.unwrap().is_err(), "una a la vez");
    let r = crate::formas::orden(&mut a, "/rama fusionar", || false).await.unwrap().unwrap();
    assert!(r.contains("✓"), "{r}");
    assert_eq!(a.root, raiz);
    assert!(a.checkpoint.is_some());
    assert_eq!(std::fs::read_to_string(raiz.join("nuevo.txt")).unwrap(), "de la rama\n");
    assert_eq!(git(&raiz, &["rev-list", "--merges", "--count", "HEAD"]), "1");
    assert!(crate::formas::orden(&mut a, "/rama cerrar", || false).await.unwrap().is_err(), "ya no hay rama");
}
