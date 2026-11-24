//! F5 · un servidor de lenguaje FALSO, en Rust, para probar el gestor sin instalar nada (como el eco MCP
//! de F0.4). Habla LSP de verdad (marco `Content-Length`): al abrir, cambiar o guardar un documento
//! publica sus errores —una línea con `ERROR` es un error en esa línea— y, si el texto trae
//! `ROMPE:<ruta>`, también publica un error en ESE otro archivo (como un servidor real tras cambiar una
//! firma). `CAER` lo hace caerse; `LENTO` tarda 2 s en contestar el `initialize`.
#![doc(hidden)]

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};

async fn leer<R: AsyncRead + Unpin>(r: &mut BufReader<R>) -> Option<Value> {
    let mut largo = 0usize;
    loop {
        let mut l = String::new();
        if r.read_line(&mut l).await.ok()? == 0 {
            return None;
        }
        let l = l.trim_end();
        if l.is_empty() {
            break;
        }
        if let Some(n) = l.strip_prefix("Content-Length:") {
            largo = n.trim().parse().ok()?;
        }
    }
    let mut b = vec![0u8; largo];
    r.read_exact(&mut b).await.ok()?;
    serde_json::from_slice(&b).ok()
}

async fn escribir<W: AsyncWrite + Unpin>(w: &mut W, m: &Value) -> bool {
    let s = m.to_string();
    w.write_all(format!("Content-Length: {}\r\n\r\n{s}", s.len()).as_bytes()).await.is_ok() && w.flush().await.is_ok()
}

fn errores(texto: &str) -> Vec<Value> {
    texto
        .lines()
        .enumerate()
        .filter(|(_, l)| l.contains("ERROR"))
        .map(|(i, l)| json!({"range": {"start": {"line": i, "character": 0}, "end": {"line": i, "character": l.len()}}, "severity": 1, "message": format!("falso: {}", l.trim())})) // juez-idioma: datos (servidor falso de PRUEBAS)
        .collect()
}

/// La palabra en (línea, carácter) de un texto (ASCII en las pruebas).
fn palabra(texto: &str, linea: u64, car: u64) -> String {
    let l = texto.lines().nth(linea as usize).unwrap_or("");
    let es = |c: char| c.is_alphanumeric() || c == '_';
    let car = (car as usize).min(l.len());
    let ini = l[..car].rfind(|c: char| !es(c)).map(|i| i + 1).unwrap_or(0);
    let fin = l[car..].find(|c: char| !es(c)).map(|i| car + i).unwrap_or(l.len());
    l[ini..fin].to_string()
}

/// Cada aparición de `p` como palabra entera: (línea, desde, hasta).
fn apariciones(texto: &str, p: &str) -> Vec<(u64, u64, u64)> {
    let mut v = Vec::new();
    for (i, l) in texto.lines().enumerate() {
        let mut d = 0;
        while let Some(j) = l[d..].find(p) {
            let a = d + j;
            let b = a + p.len();
            let antes = l[..a].chars().last().is_some_and(|c| c.is_alphanumeric() || c == '_');
            let despues = l[b..].chars().next().is_some_and(|c| c.is_alphanumeric() || c == '_');
            if !antes && !despues {
                v.push((i as u64, a as u64, b as u64));
            }
            d = b;
        }
    }
    v
}

fn rango(l: u64, a: u64, b: u64) -> Value {
    json!({"start": {"line": l, "character": a}, "end": {"line": l, "character": b}})
}

/// Los textos del proyecto: lo abierto manda; lo demás, del disco (la raíz, sin bajar a carpetas).
fn textos(raiz: &str, docs: &std::collections::BTreeMap<String, String>) -> Vec<(String, String)> {
    let mut v: Vec<(String, String)> = docs.iter().map(|(u, t)| (u.clone(), t.clone())).collect();
    for e in std::fs::read_dir(raiz).into_iter().flatten().flatten() {
        let u = format!("file://{}", e.path().display());
        if e.path().is_file() && !docs.contains_key(&u) {
            if let Ok(t) = std::fs::read_to_string(e.path()) {
                v.push((u, t));
            }
        }
    }
    v
}

/// ML2 · como un servidor de verdad tras cambiar una firma: una línea `USA:x` de un archivo ABIERTO es un
/// error si ningún archivo define `function x` / `fn x`.
fn errores_de_uso(texto: &str, todos: &[(String, String)]) -> Vec<Value> {
    texto
        .lines()
        .enumerate()
        .filter_map(|(i, l)| {
            let x = l.split("USA:").nth(1)?.split(|c: char| !(c.is_alphanumeric() || c == '_')).next()?;
            let definida = todos.iter().any(|(_, t)| t.contains(&format!("function {x}(")) || t.contains(&format!("fn {x}(")));
            (!definida).then(|| json!({"range": rango(i as u64, 0, l.len() as u64), "severity": 1, "message": format!("falso: «{x}» no existe")})) // juez-idioma: datos (servidor falso de PRUEBAS)
        })
        .collect()
}

/// Los errores de un archivo abierto (los suyos y los de uso).
fn todos_los_errores(texto: &str, raiz: &str, docs: &std::collections::BTreeMap<String, String>) -> Vec<Value> {
    let mut e = errores(texto);
    e.extend(errores_de_uso(texto, &textos(raiz, docs)));
    e
}

pub async fn servir<R: AsyncRead + Unpin, W: AsyncWrite + Unpin>(r: R, mut w: W) {
    let mut r = BufReader::new(r);
    // los documentos abiertos (uri → texto), para renombrar y demás
    let mut docs: std::collections::BTreeMap<String, String> = Default::default();
    let mut raiz = String::new();
    while let Some(m) = leer(&mut r).await {
        let metodo = m["method"].as_str().unwrap_or("");
        let uri_de = m["params"]["textDocument"]["uri"].as_str().unwrap_or("").to_string();
        let (pl, pc) = (m["params"]["position"]["line"].as_u64().unwrap_or(0), m["params"]["position"]["character"].as_u64().unwrap_or(0));
        match (m.get("id").cloned(), metodo) {
            (Some(id), "textDocument/rename") => {
                let p = palabra(docs.get(&uri_de).map(String::as_str).unwrap_or(""), pl, pc);
                let nuevo = m["params"]["newName"].as_str().unwrap_or("");
                let mut changes = serde_json::Map::new();
                for (u, t) in &docs {
                    let e: Vec<Value> = apariciones(t, &p).into_iter().map(|(l, a, b)| json!({"range": rango(l, a, b), "newText": nuevo})).collect();
                    if !e.is_empty() {
                        changes.insert(u.clone(), Value::Array(e));
                    }
                }
                let _ = escribir(&mut w, &json!({"jsonrpc": "2.0", "id": id, "result": {"changes": changes}})).await;
            }
            (Some(id), "textDocument/codeAction") => {
                let hay = m["params"]["context"]["diagnostics"].as_array().is_some_and(|d| !d.is_empty());
                let r = if hay { json!([{"title": "importar", "kind": "quickfix", "isPreferred": true, "edit": {"changes": {uri_de.clone(): [{"range": rango(0, 0, 0), "newText": "use falso::Algo;\n"}]}}}]) } else { json!([]) };
                let _ = escribir(&mut w, &json!({"jsonrpc": "2.0", "id": id, "result": r})).await;
            }
            (Some(id), "textDocument/implementation" | "textDocument/typeDefinition") => {
                let p = palabra(docs.get(&uri_de).map(String::as_str).unwrap_or(""), pl, pc);
                let loc: Vec<Value> = docs.iter().flat_map(|(u, t)| apariciones(t, &p).into_iter().take(1).map(move |(l, a, b)| json!({"uri": u, "range": rango(l, a, b)}))).take(1).collect();
                let _ = escribir(&mut w, &json!({"jsonrpc": "2.0", "id": id, "result": loc})).await;
            }
            (Some(id), "textDocument/prepareCallHierarchy") => {
                let p = palabra(docs.get(&uri_de).map(String::as_str).unwrap_or(""), pl, pc);
                let _ = escribir(&mut w, &json!({"jsonrpc": "2.0", "id": id, "result": [{"name": p, "kind": 12, "uri": uri_de, "range": rango(pl, 0, 1), "selectionRange": rango(pl, 0, 1)}]})).await;
            }
            (Some(id), "callHierarchy/incomingCalls" | "callHierarchy/outgoingCalls") => {
                let lado = if metodo.ends_with("incomingCalls") { "from" } else { "to" };
                let otro = docs.keys().find(|u| **u != m["params"]["item"]["uri"]).cloned().unwrap_or_default();
                let _ = escribir(&mut w, &json!({"jsonrpc": "2.0", "id": id, "result": [{lado: {"name": if lado == "from" { "llamador" } else { "llamado" }, "kind": 12, "uri": otro, "range": rango(2, 0, 1), "selectionRange": rango(2, 0, 1)}, "fromRanges": []}]})).await;
            }
            (Some(id), "textDocument/references") => {
                let p = palabra(docs.get(&uri_de).map(String::as_str).unwrap_or(""), pl, pc);
                let locs: Vec<Value> = textos(&raiz, &docs).iter().flat_map(|(u, t)| apariciones(t, &p).into_iter().map(move |(l, a, b)| json!({"uri": u, "range": rango(l, a, b)}))).collect();
                let _ = escribir(&mut w, &json!({"jsonrpc": "2.0", "id": id, "result": locs})).await;
            }
            // ML2 · diagnósticos a petición (3.17): lo mismo que publica
            (Some(id), "textDocument/diagnostic") => {
                let t = docs.get(&uri_de).cloned().unwrap_or_default();
                let _ = escribir(&mut w, &json!({"jsonrpc": "2.0", "id": id, "result": {"kind": "full", "items": todos_los_errores(&t, &raiz, &docs)}})).await;
            }
            (Some(id), "initialize") => {
                if std::env::var_os("GOTV_LSP_FALSO_LENTO").is_some() {
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                }
                raiz = m["params"]["rootUri"].as_str().unwrap_or("").trim_start_matches("file://").to_string();
                let caps = json!({"capabilities": {"textDocumentSync": 1, "documentSymbolProvider": true, "workspaceSymbolProvider": true, "referencesProvider": true, "diagnosticProvider": {"interFileDependencies": true, "workspaceDiagnostics": false}}, "serverInfo": {"name": "falso"}});
                if !escribir(&mut w, &json!({"jsonrpc": "2.0", "id": id, "result": caps})).await {
                    return;
                }
            }
            (Some(id), "shutdown") => {
                let _ = escribir(&mut w, &json!({"jsonrpc": "2.0", "id": id, "result": null})).await;
            }
            (Some(id), "textDocument/documentSymbol") => {
                let mut s = vec![json!({"name": "falso", "kind": 13, "range": rango(0, 0, 0), "selectionRange": rango(0, 0, 0)})];
                // cada `function x` / `fn x`: un símbolo en su línea
                for (i, l) in docs.get(&uri_de).map(String::as_str).unwrap_or("").lines().enumerate() {
                    for clave in ["function ", "fn "] {
                        if let Some(j) = l.find(clave) {
                            let a = j + clave.len();
                            let n: String = l[a..].chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
                            if !n.is_empty() {
                                s.push(json!({"name": n, "kind": 12, "range": rango(i as u64, 0, l.len() as u64), "selectionRange": rango(i as u64, a as u64, (a + n.len()) as u64)}));
                            }
                        }
                    }
                }
                let _ = escribir(&mut w, &json!({"jsonrpc": "2.0", "id": id, "result": s})).await;
            }
            (Some(id), _) => {
                let _ = escribir(&mut w, &json!({"jsonrpc": "2.0", "id": id, "result": null})).await;
            }
            (None, "exit") => return,
            // ML6: un cambio en disco (por fuera) puede romper o arreglar lo abierto que usa algo
            (None, "workspace/didChangeWatchedFiles") => {
                for (u, t) in docs.iter().filter(|(_, t)| t.contains("USA:")) {
                    let _ = escribir(&mut w, &json!({"jsonrpc": "2.0", "method": "textDocument/publishDiagnostics", "params": {"uri": u, "diagnostics": todos_los_errores(t, &raiz, &docs)}})).await;
                }
            }
            (None, "textDocument/didClose") => {
                docs.remove(m["params"]["textDocument"]["uri"].as_str().unwrap_or(""));
            }
            (None, "textDocument/didOpen" | "textDocument/didChange") => {
                let uri = m["params"]["textDocument"]["uri"].as_str().unwrap_or("").to_string();
                let texto = m["params"]["textDocument"]["text"].as_str().or(m["params"]["contentChanges"][0]["text"].as_str()).unwrap_or("").to_string();
                docs.insert(uri.clone(), texto.clone());
                if texto.contains("CAER") {
                    return;
                }
                // `CARGA_LENTA`: como rust-analyzer en frío — dice que está cargando, publica una lista VACÍA y los
                // errores de verdad llegan 2,5 s después, cuando dice que ya está quieto
                if texto.contains("CARGA_LENTA") {
                    let _ = escribir(&mut w, &json!({"jsonrpc": "2.0", "method": "experimental/serverStatus", "params": {"health": "ok", "quiescent": false}})).await;
                    let _ = escribir(&mut w, &json!({"jsonrpc": "2.0", "method": "textDocument/publishDiagnostics", "params": {"uri": uri, "diagnostics": []}})).await;
                    tokio::time::sleep(std::time::Duration::from_millis(2500)).await;
                    let _ = escribir(&mut w, &json!({"jsonrpc": "2.0", "method": "experimental/serverStatus", "params": {"health": "ok", "quiescent": true}})).await;
                }
                let _ = escribir(&mut w, &json!({"jsonrpc": "2.0", "method": "textDocument/publishDiagnostics", "params": {"uri": uri, "diagnostics": todos_los_errores(&texto, &raiz, &docs)}})).await;
                // los OTROS abiertos que usan algo (`USA:`) se revisan de nuevo: el cambio pudo romperlos
                for (u, t) in docs.iter().filter(|(u, t)| **u != uri && t.contains("USA:")) {
                    let _ = escribir(&mut w, &json!({"jsonrpc": "2.0", "method": "textDocument/publishDiagnostics", "params": {"uri": u, "diagnostics": todos_los_errores(t, &raiz, &docs)}})).await;
                }
                for otro in texto.lines().filter_map(|l| l.split("ROMPE:").nth(1)) {
                    let d = json!([{"range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 1}}, "severity": 1, "message": "falso: lo rompió otro archivo"}]); // juez-idioma: datos (servidor falso de PRUEBAS)
                    let _ = escribir(&mut w, &json!({"jsonrpc": "2.0", "method": "textDocument/publishDiagnostics", "params": {"uri": format!("file://{}", otro.trim()), "diagnostics": d}})).await;
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
pub mod tests {
    /// El servidor falso como PROCESO real: el propio binario de pruebas corriendo `lsp_falso_por_stdio`.
    pub fn comando_falso() -> Vec<String> {
        vec![std::env::current_exe().unwrap().to_string_lossy().into_owned(), "--exact".into(), "lsp::falso::tests::lsp_falso_por_stdio".into(), "--ignored".into(), "-q".into(), "--test-threads=1".into()]
    }

    /// No es una prueba: es el servidor falso por stdio (lo lanza `comando_falso` con `GOTV_LSP_FALSO=1`).
    #[tokio::test]
    #[ignore]
    async fn lsp_falso_por_stdio() {
        if std::env::var_os("GOTV_LSP_FALSO").is_some() {
            super::servir(tokio::io::stdin(), tokio::io::stdout()).await;
        }
    }
}
