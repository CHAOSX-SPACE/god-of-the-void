//! F0.4 · Servidor MCP de ECO, en Rust, para las pruebas (sustituye al guion de Python que usaban
//! `mcp.rs` y `agent.rs`: ley L1). Corre igual dentro del proceso (sobre un `duplex`), como binario
//! propio (`gotv-mcp-eco`, prueba de integración `tests/mcp_proceso.rs`) y por stdio desde el binario
//! de pruebas (la prueba de punta a punta del agente, que necesita un proceso real).
//!
//! Herramientas (en dos páginas de `tools/list`, para probar el paginado):
//! - `eco {texto}` → `eco: <texto>` (antes manda una notificación: el cliente no se confunde);
//! - `falla` → resultado con `isError` («algo salió mal»);
//! - `imagen` → un texto y una imagen (el cliente aún no la entrega: F13.5);
//! - `romper` → el servidor se cae a mitad (el cliente debe enterarse y no colgarse).
#![doc(hidden)]

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};

/// Atiende un cliente MCP por líneas hasta que cierre (o hasta `romper`).
pub async fn servir<R, W>(lector: R, mut escritor: W)
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut lineas = BufReader::new(lector).lines();
    // F13.6: tras «cambia», la lista trae una herramienta más (para probar `tools/list_changed`)
    let mut extra = false;
    while let Ok(Some(l)) = lineas.next_line().await {
        let Ok(m) = serde_json::from_str::<Value>(l.trim()) else { continue };
        let Some(id) = m.get("id").cloned() else { continue };
        let resultado = match m["method"].as_str().unwrap_or("") {
            "initialize" => json!({"protocolVersion": "2025-06-18", "capabilities": {"tools": {}}, "serverInfo": {"name": "eco", "version": "1"}}),
            "tools/list" => {
                if m["params"]["cursor"].is_null() {
                    json!({"tools": [{"name": "eco", "description": "repite", "inputSchema": {"type": "object", "properties": {"texto": {"type": "string"}}, "required": ["texto"]}}], "nextCursor": "p2"})
                } else {
                    let mut t = json!([
                        {"name": "falla", "description": "siempre falla", "inputSchema": {"type": "object"}},
                        {"name": "imagen", "description": "devuelve una imagen", "inputSchema": {"type": "object"}}, // juez-idioma: datos (respuestas del servidor de eco de PRUEBAS)
                        {"name": "romper", "description": "se cae a mitad", "inputSchema": {"type": "object"}}
                    ]);
                    if extra {
                        t.as_array_mut().unwrap().push(json!({"name": "nueva", "description": "aparece a mitad de sesión", "inputSchema": {"type": "object"}})); // juez-idioma: datos (servidor de eco de PRUEBAS)
                    }
                    json!({"tools": t})
                }
            }
            // F13.6: recursos y prompts
            "resources/list" => json!({"resources": [{"uri": "mem://nota", "name": "nota", "description": "una nota"}]}), // juez-idioma: datos (servidor de eco de PRUEBAS)
            "resources/read" => json!({"contents": [{"uri": m["params"]["uri"], "mimeType": "text/plain", "text": "contenido de la nota"}]}), // juez-idioma: datos (servidor de eco de PRUEBAS)
            "prompts/list" => json!({"prompts": [{"name": "resumir", "description": "resume un tema", "arguments": [{"name": "tema", "required": true}]}]}), // juez-idioma: datos (servidor de eco de PRUEBAS)
            "prompts/get" => json!({"messages": [{"role": "user", "content": {"type": "text", "text": format!("Resume {}", m["params"]["arguments"]["tema"].as_str().unwrap_or(""))}}]}),
            "tools/call" => match m["params"]["name"].as_str().unwrap_or("") {
                "eco" => {
                    let aviso = json!({"jsonrpc": "2.0", "method": "notifications/message", "params": {"level": "info", "data": "eco en camino"}});
                    if escritor.write_all(format!("{aviso}\n").as_bytes()).await.is_err() {
                        return;
                    }
                    // F13.6: el SERVIDOR pide algo al cliente y espera su respuesta antes de contestar
                    let pide = match m["params"]["arguments"]["texto"].as_str().unwrap_or("") {
                        "pregúntame" => Some(json!({"jsonrpc": "2.0", "id": "srv-1", "method": "elicitation/create", "params": {"message": "¿Qué color?", "requestedSchema": {"type": "object", "properties": {"color": {"type": "string", "enum": ["rojo", "azul"]}, "edad": {"type": "integer"}}, "required": ["color"]}}})), // juez-idioma: datos (servidor de eco de PRUEBAS)
                        "raíces" => Some(json!({"jsonrpc": "2.0", "id": "srv-1", "method": "roots/list"})), // juez-idioma: datos (servidor de eco de PRUEBAS)
                        "sorpresa" => Some(json!({"jsonrpc": "2.0", "id": "srv-1", "method": "sampling/createMessage", "params": {}})),
                        "cambia" => {
                            extra = true;
                            let _ = escritor.write_all(format!("{}\n", json!({"jsonrpc": "2.0", "method": "notifications/tools/list_changed"})).as_bytes()).await;
                            None
                        }
                        _ => None,
                    };
                    if let Some(p) = pide {
                        if escritor.write_all(format!("{p}\n").as_bytes()).await.is_err() {
                            return;
                        }
                        let mut respuesta = Value::Null;
                        while let Ok(Some(l2)) = lineas.next_line().await {
                            if let Ok(r) = serde_json::from_str::<Value>(l2.trim()) {
                                if r["id"] == "srv-1" {
                                    respuesta = if r.get("error").is_some() { r["error"].clone() } else { r["result"].clone() };
                                    break;
                                }
                            }
                        }
                        json!({"content": [{"type": "text", "text": format!("eco: {respuesta}")}]})
                    } else {
                    json!({"content": [{"type": "text", "text": format!("eco: {}", m["params"]["arguments"]["texto"].as_str().unwrap_or(""))}]})
                    }
                }
                "falla" => json!({"content": [{"type": "text", "text": "algo salió mal"}], "isError": true}), // juez-idioma: datos (respuestas del servidor de eco de PRUEBAS)
                // F13.5: todos los tipos de contenido de una vez
                "imagen" => json!({"content": [
                    {"type": "text", "text": "una imagen:"}, // juez-idioma: datos (respuestas del servidor de eco de PRUEBAS)
                    {"type": "image", "data": "iVBORw0KGgo=", "mimeType": "image/png"},
                    {"type": "audio", "data": "UklGRg==", "mimeType": "audio/wav"},
                    {"type": "resource_link", "uri": "file:///datos/informe.md", "name": "informe"},
                    {"type": "resource", "resource": {"uri": "mem://nota", "mimeType": "text/plain", "text": "contenido del recurso"}} // juez-idioma: datos (respuestas del servidor de eco de PRUEBAS)
                ], "structuredContent": {"total": 3}}), // juez-idioma: datos (respuestas del servidor de eco de PRUEBAS)
                "romper" => return,
                otro => json!({"content": [{"type": "text", "text": format!("no existe {otro}")}], "isError": true}),
            },
            _ => {
                let error = json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": "método desconocido"}}); // juez-idioma: datos (respuestas del servidor de eco de PRUEBAS)
                if escritor.write_all(format!("{error}\n").as_bytes()).await.is_err() {
                    return;
                }
                let _ = escritor.flush().await;
                continue;
            }
        };
        let r = json!({"jsonrpc": "2.0", "id": id, "result": resultado});
        if escritor.write_all(format!("{r}\n").as_bytes()).await.is_err() || escritor.flush().await.is_err() {
            return;
        }
    }
}
