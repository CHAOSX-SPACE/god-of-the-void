//! F0.4 · El servidor MCP de eco como proceso propio, para probar el lanzamiento de verdad
//! (`tests/mcp_proceso.rs`). Sólo pruebas: no viaja en la app.
#[tokio::main(flavor = "current_thread")]
async fn main() {
    gotv_core::mcp_eco::servir(tokio::io::stdin(), tokio::io::stdout()).await;
}
