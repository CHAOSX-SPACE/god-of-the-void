//! F23.1 · El metabuscador contra servidores FALSOS locales (axum), con el formato real de cada API.
//! Sin red de verdad, sin llaves de nadie. Todo este archivo es código de prueba (`#[cfg(test)]` en
//! `buscar/mod.rs`): el juez de idioma no lo mira.

use super::legal::{fecha_a_unix, RelojFijo};
use super::*;
use axum::{http::HeaderMap, routing::any, Router};
use std::sync::atomic::{AtomicUsize, Ordering};

pub(crate) struct Falso {
    pub base: String,
    pub golpes: Arc<AtomicUsize>,
    /// Las cabeceras de la última petición, para ver por dónde fue la llave.
    pub cabeceras: Arc<std::sync::Mutex<Vec<(String, String)>>>,
}

/// Un servidor que responde `cuerpo` en `ruta` (cualquier método), tras `retraso`.
pub(crate) async fn falso(ruta: &'static str, cuerpo: String, retraso: Duration) -> Falso {
    let golpes = Arc::new(AtomicUsize::new(0));
    let cabeceras = Arc::new(std::sync::Mutex::new(Vec::new()));
    let (g, c) = (golpes.clone(), cabeceras.clone());
    let app = Router::new().route(
        ruta,
        any(move |h: HeaderMap| {
            let (g, c, cuerpo) = (g.clone(), c.clone(), cuerpo.clone());
            async move {
                g.fetch_add(1, Ordering::SeqCst);
                *c.lock().unwrap() = h.iter().map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string())).collect();
                tokio::time::sleep(retraso).await;
                ([(axum::http::header::CONTENT_TYPE, "application/json")], cuerpo)
            }
        }),
    );
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", l.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    Falso { base, golpes, cabeceras }
}

pub(crate) fn brave_json(urls: &[&str]) -> String {
    let r: Vec<_> = urls.iter().enumerate().map(|(i, u)| serde_json::json!({"title": format!("Brave {i}"), "url": u, "description": format!("desc brave {i}")})).collect();
    serde_json::json!({"web": {"type": "search", "results": r}}).to_string()
}

pub(crate) fn serper_json(urls: &[&str]) -> String {
    let r: Vec<_> = urls.iter().enumerate().map(|(i, u)| serde_json::json!({"title": format!("Serper {i}"), "link": u, "snippet": format!("snip {i}"), "position": i + 1})).collect();
    serde_json::json!({"organic": r}).to_string()
}

pub(crate) fn wiki_json(claves: &[&str]) -> String {
    let r: Vec<_> = claves.iter().map(|k| serde_json::json!({"id": 1, "key": k, "title": k.replace('_', " "), "excerpt": format!("<span class=\"searchmatch\">{k}</span> es"), "description": null})).collect();
    serde_json::json!({"pages": r}).to_string()
}

const HOY: &str = "2026-11-20";

/// Un contexto de prueba con los servicios apuntando a sus falsos y su ficha con dominio 127.0.0.1.
pub(crate) fn ctx(dir: &Path, falsos: &[(&str, &Falso)], llaves: &[(&str, &str)]) -> Contexto {
    let mut c = Contexto::de_prueba(dir, Arc::new(RelojFijo::en(fecha_a_unix(HOY).unwrap())));
    for (id, f) in falsos {
        c.bases.insert(id.to_string(), f.base.clone());
        if let Some(fi) = c.fichas.get_mut(*id) {
            fi.dominio_api = "127.0.0.1".into();
            fi.limites.por_segundo = None;
        }
    }
    c.llaves = Arc::new(LlavesFijas(llaves.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect()));
    c.ajustes.orden = falsos.iter().map(|(id, _)| id.to_string()).collect();
    for (id, _) in falsos {
        c.ajustes.servicios.insert(id.to_string(), AjusteServicio { activo: Some(true), tope_mes: None });
    }
    c
}

#[tokio::test]
async fn varios_a_la_vez_fusionados_y_citados() {
    let d = crate::CarpetaPrueba::nueva("gotv-buscar-varios");
    let b = falso("/res/v1/web/search", brave_json(&["https://a.io/doc?utm_source=brave", "https://b.io"]), Duration::ZERO).await;
    let s = falso("/search", serper_json(&["http://www.a.io/doc/", "https://c.io"]), Duration::ZERO).await;
    let w = falso("/w/rest.php/v1/search/page", wiki_json(&["Rust"]), Duration::ZERO).await;
    let c = ctx(d.ruta(), &[("brave", &b), ("serper", &s), ("wikipedia", &w)], &[("brave", "llave-brave-falsa"), ("serper", "llave-serper-falsa")]);
    let r = buscar(&c, "rust docs", 8, None).await.unwrap();
    assert_eq!(r.hallazgos[0].fuentes, vec!["brave", "serper"], "a.io con y sin utm_ es una y gana");
    assert_eq!(r.hallazgos.len(), 4);
    assert!(r.pasos.iter().all(|p| matches!(p.estado, Estado::Respondio(_))), "{:?}", r.pasos);
    // la llave fue en su cabecera, nunca en la URL
    assert!(b.cabeceras.lock().unwrap().iter().any(|(k, v)| k == "x-subscription-token" && v == "llave-brave-falsa"));
    assert!(s.cabeceras.lock().unwrap().iter().any(|(k, v)| k == "x-api-key" && v == "llave-serper-falsa"));
    let t = presentar(&r);
    assert!(t.contains("[1] ") && t.contains("(brave, serper)"), "{t}");
    assert!(t.contains("Wikipedia (CC BY-SA 4.0)"), "F23.L-10: atribución\n{t}");
    assert!(!t.contains("llave-brave-falsa"));
}

#[tokio::test]
async fn el_que_no_responde_a_tiempo_se_omite_y_se_dice() {
    let d = crate::CarpetaPrueba::nueva("gotv-buscar-lento");
    let b = falso("/res/v1/web/search", brave_json(&["https://a.io"]), Duration::from_secs(5)).await;
    let w = falso("/w/rest.php/v1/search/page", wiki_json(&["Rust"]), Duration::ZERO).await;
    let mut c = ctx(d.ruta(), &[("brave", &b), ("wikipedia", &w)], &[("brave", "k")]);
    c.tope = Duration::from_millis(300);
    let t0 = std::time::Instant::now();
    let r = buscar(&c, "rust", 5, None).await.unwrap();
    assert!(t0.elapsed() < Duration::from_secs(2), "no espera al lento");
    assert_eq!(r.hallazgos.len(), 1);
    assert!(r.pasos.iter().any(|p| p.servicio == "brave" && p.estado == Estado::SinRespuesta));
    assert!(presentar(&r).contains(crate::tr!("sin respuesta en", "no answer within").as_str()));
}

#[tokio::test]
async fn al_agotarse_el_tope_entra_el_respaldo_sin_gastar_de_mas() {
    let d = crate::CarpetaPrueba::nueva("gotv-buscar-tope");
    let b = falso("/res/v1/web/search", brave_json(&["https://a.io"]), Duration::ZERO).await;
    let s = falso("/search", serper_json(&["https://c.io"]), Duration::ZERO).await;
    let mut c = ctx(d.ruta(), &[("brave", &b), ("serper", &s)], &[("brave", "k"), ("serper", "k2")]);
    c.ajustes.orden = vec!["brave".into()];
    c.ajustes.respaldo = vec!["serper".into()];
    c.ajustes.servicios.get_mut("brave").unwrap().tope_mes = Some(2);
    for i in 0..5 {
        let r = buscar(&c, &format!("consulta {i}"), 3, None).await.unwrap();
        let servicio = &r.hallazgos[0].fuentes[0];
        assert_eq!(servicio, if i < 2 { "brave" } else { "serper" }, "vuelta {i}");
    }
    assert_eq!(b.golpes.load(Ordering::SeqCst), 2, "Brave no recibió ni una de más");
    assert_eq!(s.golpes.load(Ordering::SeqCst), 3);
    assert_eq!(c.almacen.uso().del_mes("brave", c.reloj.ahora()), 2);
    let r = buscar(&c, "otra", 3, None).await.unwrap();
    assert!(presentar(&r).contains("2"), "dice el tope alcanzado");
}

#[tokio::test]
async fn la_llave_no_sale_de_su_dominio_ni_vuelve_en_un_error() {
    let d = crate::CarpetaPrueba::nueva("gotv-buscar-dominio");
    // un servidor que «repite» la llave en su error
    let b = falso("/res/v1/web/search", "{\"error\":\"llave invalida: SECRETO-123\"}".into(), Duration::ZERO).await;
    let mut c = ctx(d.ruta(), &[("brave", &b)], &[("brave", "SECRETO-123")]);
    // 1) la ficha dice que la llave va a api.search.brave.com, la dirección es otra → no sale nada
    c.fichas.get_mut("brave").unwrap().dominio_api = "api.search.brave.com".into();
    let r = buscar(&c, "x", 3, None).await.unwrap();
    assert_eq!(b.golpes.load(Ordering::SeqCst), 0, "ni una petición");
    assert!(matches!(&r.pasos[0].estado, Estado::Fallo(m) if m.contains("F23.L-7")), "{:?}", r.pasos);
    // 2) un 401 que repite la llave: el error no la trae
    c.fichas.get_mut("brave").unwrap().dominio_api = "127.0.0.1".into();
    let srv = falso_estado(401, "{\"message\":\"bad key SECRETO-123\"}").await;
    c.bases.insert("brave".into(), srv);
    let r = buscar(&c, "y", 3, None).await.unwrap();
    let t = presentar(&r);
    assert!(t.contains("401") && !t.contains("SECRETO-123"), "{t}");
}

async fn falso_estado(estado: u16, cuerpo: &'static str) -> String {
    let app = Router::new().route(
        "/res/v1/web/search",
        any(move || async move { (axum::http::StatusCode::from_u16(estado).unwrap(), cuerpo) }),
    );
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", l.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    base
}

#[tokio::test]
async fn solo_local_no_manda_nada_afuera() {
    let d = crate::CarpetaPrueba::nueva("gotv-buscar-local");
    let b = falso("/res/v1/web/search", brave_json(&["https://a.io"]), Duration::ZERO).await;
    let mut c = ctx(d.ruta(), &[("brave", &b)], &[("brave", "k")]);
    c.ajustes.solo_local = true;
    let idx = crate::indice::Indice::abrir(&c.almacen.indice_dir).unwrap();
    let pr = super::legal::Procedencia { origen: "docs.rs".into(), url: "https://docs.rs/axum".into(), fecha: 10, vence: i64::MAX / 2, regla: "F23.L-5".into() };
    idx.agregar(&crate::indice::Pagina { url: "https://docs.rs/axum".into(), titulo: "axum".into(), texto: "framework web en Rust".into() }, &pr).unwrap();
    c.local = Some(Arc::new(idx));
    let r = buscar(&c, "framework web", 5, Some(vec!["brave".into()])).await.unwrap();
    assert_eq!(b.golpes.load(Ordering::SeqCst), 0);
    assert_eq!(r.hallazgos[0].url, "https://docs.rs/axum");
    assert!(r.pasos.iter().all(|p| p.servicio == "local"));
}

#[tokio::test]
async fn fuentes_elige_y_en_serie_para_al_primero_que_da() {
    let d = crate::CarpetaPrueba::nueva("gotv-buscar-fuentes");
    let b = falso("/res/v1/web/search", brave_json(&["https://a.io"]), Duration::ZERO).await;
    let s = falso("/search", serper_json(&["https://c.io"]), Duration::ZERO).await;
    let mut c = ctx(d.ruta(), &[("brave", &b), ("serper", &s)], &[("brave", "k"), ("serper", "k")]);
    let r = buscar(&c, "x", 3, Some(vec!["serper".into()])).await.unwrap();
    assert_eq!((b.golpes.load(Ordering::SeqCst), s.golpes.load(Ordering::SeqCst)), (0, 1));
    assert_eq!(r.pasos.len(), 1);
    c.ajustes.paralelo = false;
    let r = buscar(&c, "y", 3, None).await.unwrap();
    assert_eq!((b.golpes.load(Ordering::SeqCst), s.golpes.load(Ordering::SeqCst)), (1, 1), "serper no salió");
    assert!(r.pasos.iter().any(|p| p.servicio == "serper" && matches!(p.estado, Estado::Omitido(_))));
    assert_eq!(c.almacen.uso().del_mes("serper", c.reloj.ahora()), 1, "su cupo reservado volvió");
}

#[tokio::test]
async fn sin_ficha_revisada_o_sin_llave_no_se_consulta() {
    let d = crate::CarpetaPrueba::nueva("gotv-buscar-ficha");
    let b = falso("/res/v1/web/search", brave_json(&["https://a.io"]), Duration::ZERO).await;
    let t = falso("/search", "{\"results\":[]}".into(), Duration::ZERO).await;
    let mut c = ctx(d.ruta(), &[("brave", &b), ("tavily", &t)], &[("brave", "k")]);
    c.fichas.get_mut("brave").unwrap().estado = "sin_revisar".into();
    // aunque copie la excepción de DuckDuckGo: sólo vale para DuckDuckGo
    c.fichas.get_mut("brave").unwrap().excepcion = "F23.L-9".into();
    let r = buscar(&c, "x", 3, None).await.unwrap();
    assert_eq!(b.golpes.load(Ordering::SeqCst) + t.golpes.load(Ordering::SeqCst), 0);
    assert!(r.pasos.iter().all(|p| matches!(p.estado, Estado::Omitido(_))));
    // las condiciones cambiaron (otra huella vista) → vencida
    c.fichas.get_mut("brave").unwrap().estado = "revisada".into();
    c.almacen.uso().anotar_hash("brave", "sha256:otra", 1).unwrap();
    buscar(&c, "x", 3, None).await.unwrap();
    assert_eq!(b.golpes.load(Ordering::SeqCst), 0);
    // un servicio de llave apagado por omisión aunque tenga ficha y llave
    let mut c2 = ctx(d.ruta(), &[], &[("brave", "k")]);
    c2.bases.insert("brave".into(), b.base.clone());
    c2.fichas.get_mut("brave").unwrap().dominio_api = "127.0.0.1".into();
    let r = buscar(&c2, "z", 3, Some(vec!["brave".into()])).await.unwrap();
    assert_eq!(b.golpes.load(Ordering::SeqCst), 0);
    assert!(presentar(&r).contains(crate::i18n::t("apagado", "off")));
}

#[tokio::test]
async fn la_cache_responde_sin_gastar_ni_salir() {
    let d = crate::CarpetaPrueba::nueva("gotv-buscar-cache");
    let w = falso("/w/rest.php/v1/search/page", wiki_json(&["Rust", "Cargo"]), Duration::ZERO).await;
    let c = ctx(d.ruta(), &[("wikipedia", &w)], &[]);
    buscar(&c, "rust", 5, None).await.unwrap();
    let r = buscar(&c, "Rust", 5, None).await.unwrap();
    assert_eq!(w.golpes.load(Ordering::SeqCst), 1);
    assert_eq!(r.pasos[0].estado, Estado::DeLaCache(2));
    assert_eq!(c.almacen.uso().del_mes("wikipedia", c.reloj.ahora()), 1);
}

#[tokio::test]
async fn una_consulta_con_rutas_personales_no_sale() {
    let d = crate::CarpetaPrueba::nueva("gotv-buscar-privada");
    let b = falso("/res/v1/web/search", brave_json(&["https://a.io"]), Duration::ZERO).await;
    let c = ctx(d.ruta(), &[("brave", &b)], &[("brave", "k")]);
    let home = crate::paths::home().to_string_lossy().to_string();
    for q in [format!("error en {home}/proyecto/main.rs"), "falla en /Users/ana/app/x.rs".to_string()] {
        assert!(buscar(&c, &q, 3, None).await.is_err(), "{q}");
    }
    assert_eq!(b.golpes.load(Ordering::SeqCst), 0);
    assert!(consulta_privada("cómo usar /usr/bin/env en macOS").is_none());
}

#[tokio::test]
async fn el_reordenador_cambia_el_orden() {
    struct Falso2;
    #[async_trait::async_trait]
    impl Reordenador for Falso2 {
        async fn vectores(&self, t: &[String]) -> Result<Vec<Vec<f32>>> {
            // la consulta ≈ lo que dice «tokio»
            Ok(t.iter().enumerate().map(|(i, x)| if i == 0 || x.contains("Brave 1") { vec![1.0, 0.0] } else { vec![0.0, 1.0] }).collect())
        }
    }
    let d = crate::CarpetaPrueba::nueva("gotv-buscar-reordenar");
    let b = falso("/res/v1/web/search", brave_json(&["https://a.io", "https://b.io"]), Duration::ZERO).await;
    let mut c = ctx(d.ruta(), &[("brave", &b)], &[("brave", "k")]);
    c.reordenador = Some(Arc::new(Falso2));
    let r = buscar(&c, "tokio", 5, None).await.unwrap();
    assert_eq!(r.hallazgos[0].url, "https://b.io");
    c.ajustes.reordenar = false;
    let r = buscar(&c, "tokio 2", 5, None).await.unwrap();
    assert_eq!(r.hallazgos[0].url, "https://a.io");
}

/// F23.L-2: mide la huella de las condiciones de cada ficha (dos veces, para ver si la página es
/// estable) y dice si coincide con la escrita. Lee páginas públicas: `GOTV_PRUEBA_RED=1 … --ignored huellas`.
#[tokio::test]
#[ignore]
async fn huellas_de_las_condiciones() {
    if std::env::var("GOTV_PRUEBA_RED").as_deref() != Ok("1") {
        return;
    }
    for (id, f) in legal::fichas() {
        let a = ventana::huella_remota(&f.condiciones_url, "").await;
        let b = ventana::huella_remota(&f.condiciones_url, "").await;
        match (a, b) {
            (Ok(a), Ok(b)) => eprintln!("HUELLA {id} {a} estable={} coincide={}", a == b, a == f.condiciones_hash),
            (a, b) => eprintln!("HUELLA {id} ERROR {:?} {:?}", a.err().map(|e| e.to_string()), b.err().map(|e| e.to_string())),
        }
    }
}

/// F23.L-9 · ¿DuckDuckGo lite responde a un User-Agent honesto? Una consulta genérica con cada uno
/// (el honesto de GOTV y el de Safari que usa hoy). `GOTV_PRUEBA_RED=1 … --ignored duckduckgo_honesto`.
#[tokio::test]
#[ignore]
async fn duckduckgo_honesto() {
    if std::env::var("GOTV_PRUEBA_RED").as_deref() != Ok("1") {
        return;
    }
    for (nombre, ua) in [("honesto", servicios::user_agent("")), ("safari", "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Safari/605.1.15".to_string())] {
        let c = reqwest::Client::builder().user_agent(ua).timeout(Duration::from_secs(15)).build().unwrap();
        let r = c.post("https://lite.duckduckgo.com/lite/").form(&[("q", "rust programming language")]).send().await.unwrap();
        let estado = r.status().as_u16();
        let html = r.text().await.unwrap_or_default();
        eprintln!("DDG {nombre}: HTTP {estado} · {} resultados · anomaly={}", crate::research::parse_ddg_lite(&html).len(), html.contains("anomaly"));
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

#[test]
fn el_proyecto_solo_restringe() {
    let g: toml::Value = toml::from_str("orden = [\"brave\", \"wikipedia\"]\nrespaldo = [\"serper\"]\n[brave]\nactivo = true\ntope_mes = 100\n").unwrap();
    let p: toml::Value = toml::from_str("solo_local = false\nparalelo = false\norden = [\"wikipedia\", \"exa\"]\n[brave]\ntope_mes = 500\n[exa]\nactivo = true\n[serper]\nactivo = false\n").unwrap();
    let a = Ajustes::desde(Some(&g), Some(&p));
    assert_eq!(a.orden, vec!["wikipedia"], "el proyecto estrecha, no añade exa");
    assert!(a.respaldo.is_empty());
    assert!(!a.paralelo);
    assert_eq!(a.tope("brave"), Some(100), "no sube el tope");
    let fichas = legal::fichas();
    assert!(!a.activo(&fichas["exa"]), "no enciende un servicio de pago");
    assert!(!a.activo(&fichas["serper"]));
    assert!(a.activo(&fichas["brave"]));
    let p2: toml::Value = toml::from_str("solo_local = true\n").unwrap();
    assert!(Ajustes::desde(Some(&g), Some(&p2)).solo_local);
    assert!(Ajustes::desde(None, None).activo(&fichas["wikipedia"]));
    assert!(!Ajustes::desde(None, None).activo(&fichas["brave"]));
}

#[tokio::test]
async fn la_ventana_lista_activa_y_borra() {
    let d = crate::CarpetaPrueba::nueva("gotv-buscar-ventana");
    let cfg = d.ruta().join("config.toml");
    std::fs::write(&cfg, "lang = \"es\"\n").unwrap();
    let b = falso("/res/v1/web/search", brave_json(&["https://a.io", "https://b.io", "https://c.io"]), Duration::ZERO).await;
    let mut c = ctx(d.ruta(), &[("brave", &b)], &[]);
    // sin llave no se puede encender
    assert!(ventana::activar(&c, &cfg, "brave", true).is_err());
    c.llaves = Arc::new(LlavesFijas([("brave".to_string(), "k".to_string())].into()));
    ventana::activar(&c, &cfg, "brave", true).unwrap();
    ventana::poner_tope(&cfg, "brave", Some(1000)).unwrap();
    ventana::poner_orden(&c, &cfg, &["brave".into(), "inventado".into()], &["wikipedia".into()], true, false).unwrap();
    let a = ventana::ajustes_de(&cfg);
    assert_eq!(a.orden, vec!["brave"]);
    assert_eq!(a.tope("brave"), Some(1000));
    assert!(std::fs::read_to_string(&cfg).unwrap().contains("lang = \"es\""), "lo demás del config se queda");
    c.ajustes = a;
    let p = ventana::probar(&c, "brave").await.unwrap();
    assert_eq!(p["ok"], true);
    assert_eq!(p["resultados"].as_array().unwrap().len(), 3);
    assert_eq!(p["usado_mes"], 1);
    let l = ventana::lista(&c);
    let brave = l["servicios"].as_array().unwrap().iter().find(|s| s["id"] == "brave").unwrap().clone();
    assert_eq!(brave["usado_mes"], 1);
    assert_eq!(brave["tope_mes"], 1000);
    assert_eq!(brave["guardar"], "no");
    assert!(brave["que_guarda"].as_str().unwrap().len() > 10);
    assert!(!l.to_string().contains("\"k\""), "la llave no vuelve a la ventana");
    // una ficha que no se puede activar (sin revisar, no DuckDuckGo)
    c.fichas.get_mut("exa").unwrap().estado = "sin_revisar".into();
    c.llaves = Arc::new(LlavesFijas([("exa".to_string(), "k".to_string())].into()));
    assert!(ventana::activar(&c, &cfg, "exa", true).is_err());
    let f = ventana::ficha(&c, "brave").unwrap();
    assert!(f["notas_legales"].as_str().unwrap().contains("3(b)"));
    let borr = ventana::borrar(&c).unwrap();
    assert_eq!(borr["cache"], 0, "Brave no dejó caché que borrar");
}
