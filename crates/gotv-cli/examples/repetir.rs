//! Repite un tramo de una conversación GUARDADA contra el núcleo nativo y muestra qué haría el modelo
//! en el paso siguiente, SIN ejecutar ninguna herramienta (nada toca el Mac). Sirve para medir un
//! cambio de instrucciones sobre el historial real del Portador. Trabaja sobre una COPIA de la base.
//!
//! ```text
//! GOTV_MLX_DIR=<modelo> repetir <carpeta del proyecto (.gotv/sesiones)> <hasta: n.º de mensajes> [<índice> <buscar> <reemplazo>]…
//! ```
//! Cada trío reemplaza `buscar` por `reemplazo` en el mensaje `índice` antes de preguntar.

use gotv_engine::{ChatRequest, Engine, Sampling};

#[tokio::main]
async fn main() {
    let a: Vec<String> = std::env::args().collect();
    let (Some(raiz), Some(hasta)) = (a.get(1), a.get(2).and_then(|s| s.parse::<usize>().ok())) else {
        eprintln!("uso: repetir <carpeta del proyecto> <hasta> [<índice> <buscar> <reemplazo>]…");
        std::process::exit(2);
    };
    let modelo = std::env::var("GOTV_MLX_DIR").expect("GOTV_MLX_DIR=<carpeta del modelo>");
    let (_, mut h) = gotv_core::session::Session::resume_last(std::path::Path::new(raiz), None).unwrap().expect("sin conversación");
    h.truncate(hasta);
    for t in a[3..].chunks(3) {
        let [i, buscar, poner] = t else { panic!("los reemplazos van de a tres") };
        let i: usize = i.parse().unwrap();
        assert!(h[i].content.contains(buscar.as_str()), "el mensaje {i} no contiene «{buscar}»");
        h[i].content = h[i].content.replace(buscar.as_str(), poner);
    }
    let ultimo = h.last().map(|m| format!("{}: {}", m.role, m.content.chars().take(120).collect::<String>())).unwrap_or_default();
    println!("{} mensajes · el último → {ultimo}", h.len());
    let mut e = gotv_mlx::NativoEngine::new(&modelo);
    e.start().await.unwrap();
    let t = std::time::Instant::now();
    let r = e
        .complete(ChatRequest { messages: h, max_tokens: 300, sampling: Sampling { temperature: 0.0, top_p: None, top_k: None, min_p: None }, tools: None, conversacion: None, herramientas_aparte: None })
        .await
        .unwrap();
    println!("{} tok de prompt en {:?}", r.prompt_tokens, t.elapsed());
    for c in &r.tool_calls {
        println!("→ herramienta {} {}", c.name, c.arguments);
    }
    if !r.content.trim().is_empty() {
        println!("→ texto: {}", r.content.trim());
    }
    e.stop().await.unwrap();
}
