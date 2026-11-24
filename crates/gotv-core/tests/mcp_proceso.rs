//! F0.4 · MCP con un servidor que es un PROCESO de verdad (el binario `gotv-mcp-eco`, en Rust): el
//! cliente lo lanza, pagina sus herramientas, lo usa, sobrevive a su notificación, ve su error, y
//! se entera cuando se cae a mitad.
use gotv_core::mcp::{Mcp, ServerConfig};
use serde_json::json;

#[tokio::test]
async fn un_servidor_mcp_proceso_de_punta_a_punta() {
    let d = std::env::temp_dir().join(format!("gotv-mcp-proceso-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    let cfg = ServerConfig { name: "eco srv".into(), command: env!("CARGO_BIN_EXE_gotv-mcp-eco").into(), ..Default::default() };
    let (mut mcp, notas) = Mcp::start(&[cfg], &d).await;
    assert!(notas[0].contains('4'), "{notas:?}");
    assert_eq!(mcp.call("mcp__eco_srv__eco", &json!({"texto": "hola"})).await.unwrap(), "eco: hola");
    assert!(mcp.call("mcp__eco_srv__falla", &json!({})).await.unwrap_err().to_string().contains("algo salió mal")); // juez-idioma: datos (lo que responde el servidor de eco)
    let e = mcp.call("mcp__eco_srv__romper", &json!({})).await.unwrap_err().to_string();
    assert!(e.contains("cerr") || e.contains("clos"), "{e}");
    // ya caído: la siguiente llamada falla al momento, no se cuelga
    let t = std::time::Instant::now();
    assert!(mcp.call("mcp__eco_srv__eco", &json!({"texto": "otra"})).await.is_err());
    assert!(t.elapsed() < std::time::Duration::from_secs(3));
    let _ = std::fs::remove_dir_all(&d);
}
