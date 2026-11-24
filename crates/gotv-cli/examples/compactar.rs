//! Reproduce la llamada de COMPACTACIÓN del agente sobre una conversación guardada, contra el modelo
//! que se pida (nube o local por servidor), midiendo cuánto tarda y cómo termina. No guarda nada: la
//! conversación queda igual. Nació del cuelgue del 2026-11-11 (Sonnet 5.5 se quedó «trabajando»
//! tras compactar y el stop no respondía).
//!
//! ```text
//! GOTV_MODEL=anthropic:claude-sonnet-5-5 compactar <carpeta del proyecto (.gotv/sesiones)> [segundos tope]
//! ```

use gotv_engine::{ChatRequest, Engine, Message, Sampling};

#[tokio::main]
async fn main() {
    let a: Vec<String> = std::env::args().collect();
    let Some(raiz) = a.get(1) else {
        eprintln!("uso: GOTV_MODEL=<modelo> compactar <carpeta del proyecto> [segundos tope]");
        std::process::exit(2);
    };
    let tope: u64 = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(300);
    let raiz = std::path::Path::new(raiz);
    let mut cfg = gotv_core::config::Config::load(raiz);
    if let Ok(m) = std::env::var("GOTV_MODEL") {
        cfg.model = m;
    }
    let (engine, nombre, _) = gotv_core::nube::launch(&mut cfg).await.expect("motor de nube");
    let (_, h) = gotv_core::session::Session::resume_last(raiz, None).unwrap().expect("sin conversación");
    // lo mismo que `Agent::compact`: sistema y primera orden fuera, la cola de 4 fuera, el resto resumido
    let mut cut = h.len() - 4;
    while cut > 2 && h[cut].role == "tool" {
        cut -= 1;
    }
    let region: String = h[2..cut]
        .iter()
        .map(|m| {
            let corto: String = m.content.chars().take(1500).collect();
            format!("[{}] {corto}", m.role)
        })
        .collect::<Vec<_>>()
        .join("\n");
    let cap = cfg.context_budget * 2;
    let n = region.chars().count();
    let region: String = if n > cap { region.chars().skip(n - cap).collect() } else { region };
    println!("{nombre} · {} mensajes · a resumir {} caracteres (presupuesto {} tok)", h.len(), region.chars().count(), cfg.context_budget);
    let req = ChatRequest {
        messages: vec![
            Message::system("Resume en 5-8 líneas lo hecho hasta ahora (archivos tocados, hallazgos, decisiones) para poder continuar la tarea. Concreto, sin relleno. /no_think"),
            Message::user(region),
        ],
        max_tokens: 400,
        sampling: Sampling { temperature: cfg.temperature.unwrap_or(0.7), top_p: None, top_k: None, min_p: None },
        tools: None,
        conversacion: None,
        herramientas_aparte: None,
    };
    let t = std::time::Instant::now();
    match tokio::time::timeout(std::time::Duration::from_secs(tope), engine.complete(req)).await {
        Err(_) => println!("❌ COLGADA: sin respuesta tras {tope} s"),
        Ok(Err(e)) => println!("✗ error en {:?}: {e:#}", t.elapsed()),
        Ok(Ok(r)) => println!("✓ en {:?} · {} caracteres · fin «{}»\n{}", t.elapsed(), r.content.len(), r.finish_reason, r.content.chars().take(600).collect::<String>()),
    }
}
