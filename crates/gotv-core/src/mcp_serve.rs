//! F13.10 · `gotv mcp serve`: GOTV como SERVIDOR MCP por stdio (un mensaje JSON-RPC por línea), para que
//! Claude Code, Claude Desktop u otro cliente usen las manos que sólo GOTV tiene: Word, Excel, PowerPoint y
//! PDF nativos, búsqueda por significado en el código, `base_datos` de SÓLO lectura, y lo que el usuario
//! habilite (imagen, música…).
//!
//! Los permisos siguen valiendo y no hay ventana a quien preguntar: lo que la configuración no concede de
//! antemano (`allow`) y pediría permiso, se NIEGA con la explicación de cómo concederlo. En `base_datos`
//! sólo pasan las acciones que leen. Lo que pase por aquí corre en el proyecto donde se lanzó el servidor.
use crate::permissions::{Decision, Permissions};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use tokio::io::{AsyncBufReadExt, AsyncWrite, AsyncWriteExt, BufReader};

/// Las herramientas que se ofrecen por omisión (las que no tienen un igual en el cliente típico).
pub const OFRECIDAS: &[&str] = &["word", "excel", "powerpoint", "pdf", "rag_search", "base_datos"];

/// Acciones de `base_datos` que sólo leen (lo demás no sale por aquí).
const BD_LECTURA: &[&str] = &["conexiones", "detectar", "probar", "esquema", "consultar", "explicar", "comparar"];

/// Los esquemas MCP (`name`, `description`, `inputSchema`) de las herramientas ofrecidas.
pub fn herramientas(ofrecidas: &[String]) -> Vec<Value> {
    let mut todas: Vec<Value> = crate::tools::schemas().as_array().cloned().unwrap_or_default();
    todas.extend(crate::agent::esquemas_de_manos());
    todas
        .iter()
        .filter(|t| t["function"]["name"].as_str().is_some_and(|n| ofrecidas.iter().any(|o| o == n)))
        .map(|t| {
            let mut d = t["function"]["description"].as_str().unwrap_or("").to_string();
            if t["function"]["name"] == "base_datos" {
                d.push_str(crate::i18n::t(" [por MCP: sólo las acciones que leen]", " [over MCP: read-only actions only]"));
            }
            json!({"name": t["function"]["name"], "description": d, "inputSchema": t["function"]["parameters"]})
        })
        .collect()
}

/// Atiende un cliente por líneas hasta que cierre.
pub async fn servir<R, W>(lector: R, mut escritor: W, root: PathBuf, perms: Permissions, ofrecidas: Vec<String>)
where
    R: tokio::io::AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut lineas = BufReader::new(lector).lines();
    let dueno = format!("{}#mcp-serve", root.display());
    while let Ok(Some(l)) = lineas.next_line().await {
        let Ok(m) = serde_json::from_str::<Value>(l.trim()) else { continue };
        let Some(id) = m.get("id").cloned() else { continue };
        let respuesta = match m["method"].as_str().unwrap_or("") {
            "initialize" => Ok(json!({
                "protocolVersion": m["params"]["protocolVersion"].as_str().unwrap_or("2025-06-18"),
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "god-of-the-void", "version": env!("CARGO_PKG_VERSION")}
            })),
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({"tools": herramientas(&ofrecidas)})),
            "tools/call" => Ok(llamar(&root, &perms, &ofrecidas, &dueno, &m["params"]).await),
            otro => Err(json!({"code": -32601, "message": crate::tr!("método desconocido: {otro}", "unknown method: {otro}")})),
        };
        let r = match respuesta {
            Ok(r) => json!({"jsonrpc": "2.0", "id": id, "result": r}),
            Err(e) => json!({"jsonrpc": "2.0", "id": id, "error": e}),
        };
        if escritor.write_all(format!("{r}\n").as_bytes()).await.is_err() || escritor.flush().await.is_err() {
            return;
        }
    }
}

fn error(texto: String) -> Value {
    json!({"content": [{"type": "text", "text": texto}], "isError": true})
}

async fn llamar(root: &Path, perms: &Permissions, ofrecidas: &[String], dueno: &str, p: &Value) -> Value {
    let nombre = p["name"].as_str().unwrap_or("");
    let args = if p["arguments"].is_object() { p["arguments"].clone() } else { json!({}) };
    if !ofrecidas.iter().any(|o| o == nombre) {
        return error(crate::tr!("«{nombre}» no se ofrece por MCP", "«{nombre}» is not offered over MCP"));
    }
    if nombre == "base_datos" && !BD_LECTURA.contains(&args["action"].as_str().unwrap_or("")) {
        return error(crate::i18n::t("por MCP, `base_datos` sólo lee: para escribir, hazlo desde GOTV (pregunta y confirma las filas)", "over MCP, `base_datos` only reads: to write, do it from GOTV (it asks and confirms the rows)").into());
    }
    match perms.check(nombre, &args) {
        Decision::Allow => {}
        Decision::Deny(por) => return error(crate::tr!("negado: {por}", "denied: {por}")),
        Decision::Ask(que) => {
            return error(crate::tr!(
                "esto pediría permiso y por MCP no hay a quién preguntar: {que}. Para concederlo de antemano, agrégalo a `allow` en ~/.config/gotv/config.toml",
                "this would ask for permission and over MCP there is no one to ask: {que}. To grant it beforehand, add it to `allow` in ~/.config/gotv/config.toml"
            ))
        }
    }
    let r = crate::tools::run_tool_as(nombre, &args, root, dueno).await;
    json!({"content": [{"type": "text", "text": r.output}], "isError": !r.ok})
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::permissions::Mode;

    async fn pedir(w: &mut (impl AsyncWrite + Unpin), lineas: &mut tokio::io::Lines<BufReader<impl tokio::io::AsyncRead + Unpin>>, m: Value) -> Value {
        w.write_all(format!("{m}\n").as_bytes()).await.unwrap();
        serde_json::from_str(&lineas.next_line().await.unwrap().unwrap()).unwrap()
    }

    /// De punta a punta por stdio (sobre un `duplex`): el cliente MCP de GOTV mismo habla con `gotv mcp serve`.
    #[tokio::test]
    async fn gotv_como_servidor_mcp() {
        let d = crate::CarpetaPrueba::nueva("gotv-mcp-serve");
        let c = rusqlite::Connection::open(d.join("app.db")).unwrap();
        c.execute_batch("CREATE TABLE t (id INTEGER PRIMARY KEY, nombre TEXT); INSERT INTO t VALUES (1, 'uno');").unwrap();
        drop(c);
        let perms = Permissions::new(Mode::Default, &d, vec![], vec![], vec![]);
        let (cliente, servidor) = tokio::io::duplex(1 << 16);
        let (sr, sw) = tokio::io::split(servidor);
        tokio::spawn(servir(sr, sw, d.to_path_buf(), perms, OFRECIDAS.iter().map(|s| s.to_string()).collect()));
        let (cr, mut cw) = tokio::io::split(cliente);
        let mut l = BufReader::new(cr).lines();
        let r = pedir(&mut cw, &mut l, json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-06-18"}})).await;
        assert_eq!(r["result"]["serverInfo"]["name"], "god-of-the-void");
        let r = pedir(&mut cw, &mut l, json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"})).await;
        let nombres: Vec<&str> = r["result"]["tools"].as_array().unwrap().iter().filter_map(|t| t["name"].as_str()).collect();
        assert!(nombres.contains(&"word") && nombres.contains(&"base_datos") && !nombres.contains(&"bash"), "{nombres:?}");
        // leer una base: pasa
        let r = pedir(&mut cw, &mut l, json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {"name": "base_datos", "arguments": {"action": "consultar", "conexion": "app.db", "sql": "SELECT nombre FROM t"}}})).await;
        assert!(r["result"]["content"][0]["text"].as_str().unwrap().contains("uno") && r["result"]["isError"] == false, "{r}");
        // escribir: no por aquí
        let r = pedir(&mut cw, &mut l, json!({"jsonrpc": "2.0", "id": 4, "method": "tools/call", "params": {"name": "base_datos", "arguments": {"action": "ejecutar", "conexion": "app.db", "sql": "DELETE FROM t"}}})).await;
        assert_eq!(r["result"]["isError"], true);
        // lo no ofrecido, tampoco (aunque exista en GOTV)
        let r = pedir(&mut cw, &mut l, json!({"jsonrpc": "2.0", "id": 5, "method": "tools/call", "params": {"name": "bash", "arguments": {"cmd": "rm -rf /"}}})).await;
        assert!(r["result"]["isError"] == true && r["result"]["content"][0]["text"].as_str().unwrap().contains("no se ofrece"));
        // método desconocido: error JSON-RPC
        let r = pedir(&mut cw, &mut l, json!({"jsonrpc": "2.0", "id": 6, "method": "sampling/createMessage"})).await;
        assert_eq!(r["error"]["code"], -32601);
        let n: i64 = rusqlite::Connection::open(d.join("app.db")).unwrap().query_row("SELECT count(*) FROM t", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 1);
    }
}
