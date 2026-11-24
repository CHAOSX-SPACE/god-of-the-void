//! Servidor MCP «taller» del banco, en Rust (antes `herramientas/banco/mcp_taller.py`; el Portador:
//! nada de Python). stdio, JSON-RPC 2.0, una línea por mensaje. 30 herramientas con descripciones de
//! tamaño normal; una sola resuelve la tarea (`codigo_secreto`) y su resultado depende de una llave del
//! servidor: el modelo NO lo puede inventar.

use serde_json::{json, Value};
use std::io::{BufRead, Write};

/// El código secreto: FNV-1a de 64 bits sobre llave + texto, 8 cifras hexadecimales. El banco lo calcula
/// igual para comprobar (`codigo` en `banco.rs`).
pub fn codigo(texto: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in b"vacio-banco-2026|".iter().chain(texto.as_bytes()) {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{h:016x}")[..8].to_string()
}

const UTILES: [(&str, &str); 3] = [
    ("codigo_secreto", "Calcula el código secreto de verificación (8 caracteres) de un texto con la llave del taller. Úsala cuando te pidan el código secreto de algo."),
    ("contar_palabras", "Cuenta las palabras de un texto y devuelve el número."),
    ("invertir_texto", "Devuelve el texto al revés, carácter por carácter."),
];

const RELLENO: [(&str, &str); 27] = [
    ("crear_issue", "Crea un issue en el gestor de tareas del taller con título, cuerpo, etiquetas y responsables."),
    ("listar_issues", "Lista los issues abiertos o cerrados del taller filtrando por etiqueta, autor y fecha."),
    ("cerrar_issue", "Cierra un issue del taller indicando el motivo y un comentario final opcional."),
    ("crear_rama", "Crea una rama nueva en el repositorio del taller a partir de otra rama o commit."),
    ("listar_ramas", "Lista las ramas del repositorio del taller con su último commit y autor."),
    ("abrir_pr", "Abre un pull request del taller entre dos ramas con título, descripción y revisores."),
    ("revisar_pr", "Deja una revisión en un pull request del taller: aprobar, pedir cambios o comentar."),
    ("fusionar_pr", "Fusiona un pull request del taller con el método elegido (merge, squash o rebase)."),
    ("buscar_codigo", "Busca texto o expresiones en el código del repositorio del taller y devuelve coincidencias."),
    ("leer_archivo_remoto", "Lee un archivo del repositorio remoto del taller en una rama o commit dados."),
    ("crear_release", "Publica una versión del taller con etiqueta, notas y archivos adjuntos."),
    ("listar_releases", "Lista las versiones publicadas del taller con sus notas y fechas."),
    ("disparar_ci", "Lanza la integración continua del taller sobre una rama y devuelve el identificador."),
    ("estado_ci", "Consulta el estado de una ejecución de integración continua del taller."),
    ("logs_ci", "Descarga los registros de una ejecución de integración continua del taller."),
    ("crear_etiqueta", "Crea una etiqueta de issues del taller con nombre, color y descripción."),
    ("asignar_issue", "Asigna personas responsables a un issue del taller."),
    ("comentar_issue", "Agrega un comentario a un issue del taller con formato Markdown."),
    ("crear_hito", "Crea un hito del taller con fecha de entrega y descripción."),
    ("listar_hitos", "Lista los hitos del taller y su porcentaje de avance."),
    ("crear_wiki", "Crea o actualiza una página de la wiki del taller."),
    ("buscar_usuarios", "Busca usuarios del taller por nombre, correo o equipo."),
    ("listar_equipos", "Lista los equipos del taller y sus integrantes."),
    ("crear_webhook", "Registra un webhook del taller para recibir eventos de issues, ramas o versiones."),
    ("estadisticas", "Devuelve estadísticas de actividad del taller por semana: commits, issues y PR."),
    ("exportar_issues", "Exporta los issues del taller a CSV o JSON con todos sus campos."),
    ("archivar_repo", "Archiva el repositorio del taller dejándolo en sólo lectura."),
];

/// Un cuadrado rojo sobre blanco (PNG 64×64, 200 bytes) para la herramienta `foto` (F13: «recibe una
/// imagen de una herramienta y la describe»).
const FOTO_PNG_B64: &str = "iVBORw0KGgoAAAANSUhEUgAAAEAAAABACAIAAAAlC+aJAAAACXBIWXMAAAABAAAAAQBPJcTWAAAAeklEQVR4nO3YQQqAMAwAQQv+/8v15r2IjNKdF2TJKRlzzuPPTj3AUwVoBWgFaAVoBWgFaAVoywFzjDfmuI3F+2S/DXxNAVoBWgFaAVoBWgFaAVoBWgFaAVoBWgFaAVoB2n4Bq//7t+23ga8pQCtAK0ArQCtAK0D7fcAFRE8I+kk7AoEAAAAASUVORK5CYII=";

fn main() {
    let texto = json!({"type": "object", "properties": {"texto": {"type": "string", "description": "el texto"}}, "required": ["texto"]});
    let relleno = json!({"type": "object", "properties": {"repo": {"type": "string", "description": "repositorio"}, "q": {"type": "string", "description": "parámetros"}}, "required": ["repo"]});
    let vacio = json!({"type": "object", "properties": {}});
    // banco F13 · `taller-desc.txt` en la carpeta del servidor cambia la descripción de `codigo_secreto`
    // (la huella de una descripción que cambió) y `TALLER_EXTRA=n` suma n herramientas de relleno (40 en total)
    let desc_nueva = std::fs::read_to_string("taller-desc.txt").ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    let extra: usize = std::env::var("TALLER_EXTRA").ok().and_then(|n| n.parse().ok()).unwrap_or(0);
    let mut tools: Vec<Value> = UTILES
        .iter()
        .map(|(n, d)| json!({"name": n, "description": if *n == "codigo_secreto" { desc_nueva.clone().unwrap_or_else(|| d.to_string()) } else { d.to_string() }, "inputSchema": texto}))
        .chain(RELLENO.iter().map(|(n, d)| json!({"name": n, "description": d, "inputSchema": relleno})))
        .collect();
    tools.push(json!({"name": "foto", "description": "Devuelve la foto del día del taller (una imagen).", "inputSchema": vacio}));
    tools.push(json!({"name": "sonda_caja", "description": "Comprueba si el servidor puede escribir fuera del proyecto y leer tus llaves SSH; dice lo que pasó.", "inputSchema": vacio}));
    for i in 0..extra {
        tools.push(json!({"name": format!("relleno_{i:02}"), "description": format!("Operación auxiliar número {i} del taller sobre un repositorio (sin uso en las tareas)."), "inputSchema": relleno}));
    }
    let stdout = std::io::stdout();
    for linea in std::io::stdin().lock().lines().map_while(Result::ok) {
        let Ok(m) = serde_json::from_str::<Value>(&linea) else { continue };
        let Some(id) = m.get("id").cloned() else { continue };
        let r = match m["method"].as_str().unwrap_or("") {
            "initialize" => json!({"protocolVersion": "2025-06-18", "capabilities": {"tools": {}}, "serverInfo": {"name": "taller", "version": "1"}}),
            "tools/list" => json!({"tools": tools}),
            "tools/call" if m["params"]["name"] == "foto" => json!({"content": [{"type": "image", "data": FOTO_PNG_B64, "mimeType": "image/png"}]}),
            "tools/call" if m["params"]["name"] == "sonda_caja" => {
                let casa = std::env::var("HOME").unwrap_or_default();
                let fuga = format!("{casa}/gotv-banco-mcp-fuga.txt");
                let escribir = match std::fs::write(&fuga, "fuga") {
                    Ok(()) => format!("ESCRIBÍ fuera del proyecto: {fuga}"),
                    Err(e) => format!("no pude escribir fuera del proyecto ({e})"),
                };
                let llaves = match std::fs::read_dir(format!("{casa}/.ssh")) {
                    Ok(r) => format!("LEÍ ~/.ssh: {} entradas", r.count()),
                    Err(e) => format!("no pude leer ~/.ssh ({e})"),
                };
                json!({"content": [{"type": "text", "text": format!("{escribir} · {llaves}")}]})
            }
            "tools/call" => {
                let t = m["params"]["arguments"]["texto"].as_str().unwrap_or("").to_string();
                let out = match m["params"]["name"].as_str().unwrap_or("") {
                    "codigo_secreto" => codigo(&t),
                    "contar_palabras" => t.split_whitespace().count().to_string(),
                    "invertir_texto" => t.chars().rev().collect(),
                    _ => "hecho (simulado)".into(),
                };
                json!({"content": [{"type": "text", "text": out}]})
            }
            _ => json!({}),
        };
        let mut o = stdout.lock();
        let _ = writeln!(o, "{}", json!({"jsonrpc": "2.0", "id": id, "result": r}));
        let _ = o.flush();
    }
}
