//! F23.L-11 · La prueba que guarda la ley del guardado (PLAN-CUERPO, F23.L). Debe pasar antes de cada
//! commit de F23.
//!
//! 100 búsquedas contra dos servidores FALSOS locales: uno cuya ficha dice `guardar_resultados = "no"`
//! (Brave, su ficha real) y otro con `cache_horas = 1`. Comprueba que:
//! 1. el primero no deja NADA de sus resultados en disco (ni títulos, ni direcciones, ni resúmenes);
//! 2. lo del segundo vence a la hora (reloj simulado) y el barrido lo borra;
//! 3. ningún registro guardado carece de procedencia (F23.L-4);
//! 4. la llave no aparece en ningún archivo ni en lo que ve el modelo (F23.L-7);
//! 5. una ficha que dice `no` tampoco alimenta el índice propio (F23.L-3).

use axum::{routing::any, Router};
use gotv_core::buscar::{self, legal, Contexto, LlavesFijas};
use gotv_engine::prueba::CarpetaPrueba;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

const LLAVE_NO: &str = "LLAVE-FALSA-brave-8f3a1c9e2d";
const LLAVE_CACHE: &str = "LLAVE-FALSA-serper-77b0e4aa1f";

/// Un servidor falso que contesta en el formato real del servicio, con marcas únicas por consulta.
async fn servidor(ruta: &'static str, formato: fn(&str, usize) -> String, golpes: Arc<AtomicUsize>) -> String {
    let app = Router::new().route(
        ruta,
        any(move |cuerpo: String| {
            let g = golpes.clone();
            async move {
                let k = g.fetch_add(1, Ordering::SeqCst);
                // la consulta viaja en la URL (Brave) o en el cuerpo (Serper): la marca usa el golpe
                formato(&cuerpo, k)
            }
        }),
    );
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", l.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    base
}

fn brave(_: &str, k: usize) -> String {
    serde_json::json!({"web": {"type": "search", "results": [
        {"title": format!("MARCA-NO-titulo-{k}"), "url": format!("https://no-guardar.example/{k}"), "description": format!("MARCA-NO-resumen-{k}")},
        {"title": format!("MARCA-NO-otro-{k}"), "url": format!("https://no-guardar.example/b{k}"), "description": "MARCA-NO-fijo"}
    ]}})
    .to_string()
}

fn serper(_: &str, k: usize) -> String {
    serde_json::json!({"organic": [
        {"title": format!("MARCA-CACHE-titulo-{k}"), "link": format!("https://si-cache.example/{k}"), "snippet": format!("MARCA-CACHE-resumen-{k}"), "position": 1}
    ]})
    .to_string()
}

/// Todos los archivos bajo `dir`, con sus bytes.
fn archivos(dir: &Path, out: &mut Vec<(std::path::PathBuf, Vec<u8>)>) {
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let p = e.path();
        if p.is_dir() {
            archivos(&p, out);
        } else if let Ok(b) = std::fs::read(&p) {
            out.push((p, b));
        }
    }
}

fn contiene(b: &[u8], aguja: &str) -> bool {
    b.windows(aguja.len()).any(|w| w == aguja.as_bytes())
}

// lo que sigue son las pruebas (para el juez de idioma, como en los módulos `tests`)
#[cfg(test)]
#[tokio::test]
async fn cien_busquedas_dejan_en_disco_solo_lo_que_la_ley_permite() {
    let dir = CarpetaPrueba::nueva("gotv-legal-guardado");
    let (g_no, g_cache) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
    let base_no = servidor("/res/v1/web/search", brave, g_no.clone()).await;
    let base_cache = servidor("/search", serper, g_cache.clone()).await;

    let inicio = legal::fecha_a_unix("2026-11-20").unwrap() + 9 * 3_600;
    let reloj = Arc::new(legal::RelojFijo::en(inicio));
    let mut ctx = Contexto::de_prueba(dir.ruta(), reloj.clone());
    // fichas: Brave tal cual (guardar = no); Serper con cache_horas = 1 (servicio «falso» de caché)
    for (id, base) in [("brave", &base_no), ("serper", &base_cache)] {
        let f = ctx.fichas.get_mut(id).unwrap();
        f.dominio_api = "127.0.0.1".into();
        f.limites.por_segundo = None;
        ctx.bases.insert(id.into(), base.clone());
        ctx.ajustes.servicios.insert(id.into(), buscar::AjusteServicio { activo: Some(true), tope_mes: None });
    }
    let s = ctx.fichas.get_mut("serper").unwrap();
    s.guardar_resultados = "cache".into();
    s.cache_horas = Some(1);
    assert_eq!(ctx.fichas["brave"].guardar(), legal::Guardar::No, "la ficha real de Brave dice «no»");
    ctx.ajustes.orden = vec!["brave".into(), "serper".into()];
    ctx.llaves = Arc::new(LlavesFijas([("brave".to_string(), LLAVE_NO.to_string()), ("serper".to_string(), LLAVE_CACHE.to_string())].into()));
    // el índice propio existe y se le ofrece todo: sólo debe aceptar lo que la ficha deja
    let indice = gotv_core::indice::Indice::abrir(&ctx.almacen.indice_dir).unwrap();

    // 100 búsquedas: 50 consultas distintas, cada una dos veces (la segunda debe salir de la caché
    // para Serper y otra vez de la red para Brave, que no deja guardar)
    let mut vistas = String::new();
    for i in 0..100 {
        let q = format!("consulta numero {}", i % 50);
        let r = buscar::buscar(&ctx, &q, 5, None).await.unwrap();
        let t = buscar::presentar(&r);
        vistas.push_str(&t);
        for id in ["brave", "serper"] {
            let f = &ctx.fichas[id];
            let hs: Vec<buscar::Hallazgo> = r
                .hallazgos
                .iter()
                .filter(|h| h.fuentes.iter().any(|x| x == id))
                .map(|h| buscar::Hallazgo { titulo: h.titulo.clone(), url: h.url.clone(), resumen: h.resumen.clone(), fuente: id.into(), puesto: 1 })
                .collect();
            indice.agregar_hallazgos(f, &hs, reloj.0.load(Ordering::SeqCst)).unwrap();
        }
        assert!(t.contains("MARCA-NO-titulo"), "Brave se ve en el turno (memoria), vuelta {i}");
        reloj.adelantar(10);
    }
    assert_eq!(g_no.load(Ordering::SeqCst), 100, "Brave sin caché: las 100 salieron");
    assert_eq!(g_cache.load(Ordering::SeqCst), 50, "Serper: la repetida salió de la caché");

    // 1 · nada de Brave en disco; 4 · ninguna llave en ningún archivo
    let mut todo = Vec::new();
    archivos(dir.ruta(), &mut todo);
    assert!(!todo.is_empty());
    for (p, b) in &todo {
        assert!(!contiene(b, "MARCA-NO"), "F23.L-3: {} guarda resultados de un servicio que dice «no»", p.display());
        assert!(!contiene(b, "no-guardar.example"), "F23.L-3: {} guarda direcciones de Brave", p.display());
        assert!(!contiene(b, LLAVE_NO) && !contiene(b, LLAVE_CACHE), "F23.L-7: la llave está en {}", p.display());
    }
    assert!(!vistas.contains(LLAVE_NO) && !vistas.contains(LLAVE_CACHE), "F23.L-7: la llave llegó al modelo");
    // lo de Serper sí está (es lo que su ficha permite) …
    assert!(todo.iter().any(|(_, b)| contiene(b, "MARCA-CACHE-titulo")), "Serper sí se cachea");
    // … y la consulta del usuario no está en claro (F23.L-8)
    assert!(!todo.iter().any(|(_, b)| contiene(b, "consulta numero")));

    // 3 · procedencia completa en cada fila
    let regs = ctx.almacen.cache().registros().unwrap();
    assert_eq!(regs.len(), 50);
    for r in &regs {
        r.procedencia.validar().unwrap();
        assert_eq!(r.procedencia.origen, "serper");
        assert_eq!(r.procedencia.regla, "F23.L-3");
        assert_eq!(r.procedencia.vence - r.procedencia.fecha, 3_600);
    }
    // 5 · el índice: nada (ni Brave «no» ni Serper «cache» entran; sólo «si»)
    assert_eq!(indice.paginas().unwrap(), 0);

    // 2 · a la hora, vence: la misma consulta vuelve a salir a la red y el barrido borra lo viejo
    reloj.adelantar(3_600);
    let antes = g_cache.load(Ordering::SeqCst);
    buscar::buscar(&ctx, "consulta numero 49", 5, None).await.unwrap();
    assert_eq!(g_cache.load(Ordering::SeqCst), antes + 1, "lo vencido no se sirve");
    let quedan = ctx.almacen.cache().registros().unwrap();
    assert_eq!(quedan.len(), 1, "el barrido de la búsqueda borró las 50 vencidas; queda la nueva");
    reloj.adelantar(3_600);
    assert_eq!(ctx.almacen.cache().barrer(reloj.0.load(Ordering::SeqCst)).unwrap(), 1);
    assert!(ctx.almacen.cache().registros().unwrap().is_empty());

    // F23.L-12: borrar caché e índice deja intacto el contador de uso y todo lo demás
    ctx.almacen.borrar_cache_e_indice().unwrap();
    assert!(!ctx.almacen.cache_db.exists() && !ctx.almacen.indice_dir.exists());
    assert!(ctx.almacen.uso_db.exists());
}

/// Las fichas de fábrica cumplen la ley tal como están escritas: los servicios cuyas condiciones
/// prohíben o callan sobre guardar dicen «no»; las 8 tienen fuentes oficiales y dominio de llave.
#[test]
fn las_fichas_de_fabrica_dicen_lo_que_leimos() {
    let f = legal::fichas();
    for id in ["brave", "exa", "tavily", "serper", "github", "duckduckgo"] {
        assert_eq!(f[id].guardar(), legal::Guardar::No, "{id}: sus condiciones no permiten guardar o no dicen nada");
    }
    for id in ["wikipedia", "arxiv"] {
        assert_eq!(f[id].guardar(), legal::Guardar::Si, "{id}: CC BY-SA / CC0");
    }
    assert!(f["arxiv"].atribucion.contains("Thank you to arXiv"), "atribución exigida por arXiv");
    assert!(f["wikipedia"].atribucion.contains("CC BY-SA"));
    assert_eq!(f["brave"].prohibe_entrenar_ia, "si");
    // F23.L-9: DuckDuckGo funciona marcado y sólo él tiene la excepción
    assert_eq!(f["duckduckgo"].estado, "sin_revisar");
    assert!(f["duckduckgo"].aviso().is_some());
    for (id, x) in &f {
        if id != "duckduckgo" {
            assert!(x.excepcion.is_empty(), "{id}");
            assert_eq!(x.estado, "revisada", "{id}");
        }
        assert!(!x.notas_legales.incompleto() && !x.resumen.incompleto() && !x.fuentes.is_empty(), "{id}: textos en los dos idiomas y fuentes");
        assert_eq!(x.necesita_llave, !x.donde_llave.is_empty(), "{id}: dónde sacar la llave");
    }
}
