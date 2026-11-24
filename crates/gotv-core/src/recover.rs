//! Rescate de turnos defectuosos del modelo.
//!
//! Medido con Qwen3-32B 4-bit (2026-11-08), una misma orden «edita app.py» terminó en:
//! 1. pregunta «¿deseas que realice la edición?» en vez de actuar;
//! 2. la llamada ESCRITA como código (`edit(path="app.py", …)`) + «se ha realizado
//!    con éxito» sin haber llamado nada;
//! 3. «Procedo a cambiar…» y fin del turno, sin llamada;
//! 4. un bucle que repitió dos frases durante 4.096 tokens (520 s).
//!
//! Ningún ajuste de prompt ni de muestreo eliminó las cuatro. Un agente local
//! no puede elegir su modelo perfecto: tiene que rescatar al que hay.
//! Cada rescate es determinista, se anuncia al usuario y tiene tope por orden.

use gotv_engine::ToolCall;
use serde_json::{Map, Value};

/// Respuesta automática cuando el modelo pide permiso o anuncia sin actuar.
pub fn auto_continue() -> &'static str {
    crate::i18n::t(
        "Sí, aplícalo con la herramienta y continúa hasta terminar.",
        "Yes, apply it with the tool and keep going until it is done.",
    )
}

/// Aviso cuando el modelo se queda repitiendo lo mismo.
pub fn loop_notice() -> &'static str {
    crate::i18n::t(
        "Te quedaste repitiendo lo mismo en bucle y GOTV te cortó. No lo repitas: \
         si falta hacer algo, llama la herramienta ahora; si ya terminaste, responde en una línea.",
        "You got stuck repeating the same thing and GOTV cut you off. Do not repeat it: \
         if something is left to do, call the tool now; if you are done, answer in one line.",
    )
}

/// Por qué un turno de texto no es una respuesta final de verdad.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stall {
    /// «¿Deseas que lo aplique?»: pide permiso para ACTUAR (GOTV ya lo gestiona).
    AsksToProceed,
    /// «Procedo a cambiarlo.» y nada más: anunció una acción y no la hizo.
    Announces,
}

fn tail(text: &str, n: usize) -> String {
    let t = text.trim();
    let start = t
        .char_indices()
        .rev()
        .nth(n.saturating_sub(1))
        .map(|(i, _)| i)
        .unwrap_or(0);
    t[start..].to_lowercase()
}

/// ¿El texto final es un «atasco» y no una respuesta? Sólo mira el FINAL.
pub fn stall(text: &str) -> Option<Stall> {
    let t = text.trim().to_lowercase();
    if t.is_empty() {
        return None;
    }
    let end = tail(text, 220);
    // juez-idioma: datos — se detectan los DOS idiomas a propósito (bloque)
    if t.ends_with('?') {
        let verbs = [
            "realice",
            "aplique",
            "haga",
            "proceda",
            "edite",
            "cree",
            "ejecute",
            "modifique",
            "continúe",
            "guarde",
            "escriba",
            "implemente",
            "cambie",
            "actualice",
        ];
        let asks = [
            "¿deseas que",
            "¿quieres que",
            "¿te gustaría que",
            "¿prefieres que",
            "¿desea que",
            "¿quiere que",
        ];
        if asks.iter().any(|p| end.contains(p)) && verbs.iter().any(|v| end.contains(v)) {
            return Some(Stall::AsksToProceed);
        }
        let direct = [
            "¿procedo",
            "¿lo aplico",
            "¿aplico",
            "¿confirmas",
            "¿puedo proceder",
            "¿continúo",
            "¿sigo con",
            "do you want me to",
            "would you like me to",
            "shall i ",
            "should i proceed",
            "should i apply",
        ];
        if direct.iter().any(|p| end.contains(p)) {
            return Some(Stall::AsksToProceed);
        }
        return None;
    }
    // Anuncio sin acción: la ÚLTIMA oración de la PROSA promete hacer algo ahora (los
    // bloques de código no cuentan: medido, el 32b cerró con «Vamos a empezar con el primer
    // paso.» y DESPUÉS un bloque ```bash grep …```, y se tomaba el cierre del bloque).
    let prose = without_code_blocks(&t);
    let last = prose
        .rsplit(['\n'])
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("");
    let promises = [
        "procedo a",
        "procederé a",
        "voy a ",
        "ahora voy",
        "a continuación",
        "realizaré",
        "haré el cambio",
        "ahora edito",
        "ahora cambio",
        "ahora reemplazo",
        "i will now",
        "i'll now",
        "let me ",
        "now i will",
        "next, i will",
        "i'll proceed",
        "proceeding to",
        "paso siguiente",
        "siguiente paso",
        "next step",
        // medido 2026-11-09 (Qwen3-32B): «Vamos a comenzar con el primer paso.» y cerró el turno
        "vamos a ",
        "intentaré",
        "reintentaré",
        "lo reintento",
        "lo intento de nuevo",
        "let's ",
        "i'll try",
        "i will try",
    ];
    let done = [
        "he realizado",
        "he cambiado",
        "listo",
        "hecho",
        "completad",
        "terminad",
        "done",
        "completed",
    ];
    // futuro de 1.ª persona en español («verificaré», «ejecutaré», «instalaré»…): medido
    // 2026-11-09, el 32b escribió los comandos en un bloque y cerró con «Luego, verificaré
    // con curl que el servidor responde.» sin ejecutar nada
    let future_es = last.split(|c: char| !c.is_alphabetic()).any(|w| {
        w.chars().count() > 4 && (w.ends_with("aré") || w.ends_with("eré") || w.ends_with("iré"))
    });
    let future_en = ["i'll ", "i will ", "i'm going to", "we'll ", "we will "]
        .iter()
        .any(|p| last.contains(p));
    // presente de 1.ª persona tras «ahora» y frase que acaba en dos puntos (promete algo que no llega).
    // Medido (prueba Cable, 2026-11-12): el 35B cerró un trabajo largo con «OK, tengo los tests. Ahora
    // leo las secciones relevantes de `almacen.py` para ver los problemas:» y no llamó a nada.
    let ahora_presente = [
        "ahora leo", "ahora reviso", "ahora ejecuto", "ahora corro", "ahora miro", "ahora busco",
        "ahora pruebo", "ahora compruebo", "ahora verifico", "ahora arreglo", "ahora corrijo",
        "ahora abro", "ahora edito", "ahora escribo", "ahora analizo", "ahora examino",
        "ahora inspecciono", "ahora aplico", "ahora añado", "ahora agrego", "now i read", "now i run",
        "now i check", "now i fix", "now i open",
    ]
    .iter()
    .any(|p| last.contains(p));
    let promete_lo_que_sigue = last.ends_with(':');
    if (promises.iter().any(|p| last.contains(p)) || future_es || future_en || ahora_presente || promete_lo_que_sigue)
        && !done.iter().any(|d| last.contains(d))
    {
        return Some(Stall::Announces);
    }
    None
}

/// Nombres y parámetros requeridos/permitidos de cada herramienta de una lista de esquemas (la del agente trae además las de diseño,
/// oficina e imagen: `pagina`, `isometrica`, `generar_imagen`…).
fn specs_de(schemas: &Value) -> Vec<(String, Vec<String>, Vec<String>)> {
    schemas
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|t| {
                    let f = &t["function"];
                    let name = f["name"].as_str()?.to_string();
                    let req = f["parameters"]["required"]
                        .as_array()
                        .map(|r| {
                            r.iter()
                                .filter_map(|x| x.as_str().map(String::from))
                                .collect()
                        })
                        .unwrap_or_default();
                    let props = f["parameters"]["properties"]
                        .as_object()
                        .map(|o| o.keys().cloned().collect())
                        .unwrap_or_default();
                    Some((name, req, props))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Posiciones donde aparece `name(` como llamada (no `glob.glob(` ni `my_edit(`).
fn call_sites<'a>(text: &'a str, name: &'a str) -> impl Iterator<Item = usize> + 'a {
    let pat = format!("{name}(");
    text.match_indices(pat.as_str())
        .filter(|(i, _)| {
            !text[..*i]
                .chars()
                .next_back()
                .is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '.')
        })
        .map(move |(i, _)| i + name.len() + 1)
        .collect::<Vec<_>>()
        .into_iter()
}

/// ¿El texto trae una herramienta escrita como código? Devuelve su nombre.
pub fn pseudo_call_name(text: &str) -> Option<String> {
    pseudo_call_name_en(text, &crate::tools::schemas())
}

/// Igual, contra la lista de herramientas del agente.
pub fn pseudo_call_name_en(text: &str, schemas: &Value) -> Option<String> {
    specs_de(schemas)
        .into_iter()
        .find(|(name, _, _)| call_sites(text, name).next().is_some())
        .map(|(name, _, _)| name)
}

/// Ejecución FINGIDA: un bloque de shell con un comando y, después, su «respuesta» o
/// «salida» en otro bloque, sin haberlo llamado. Medido 2026-11-09 (Qwen3-32B): escribió
/// ```bash curl -s http://localhost:5173``` y pegó un HTML inventado («Demo» en vez de
/// «demo») como si lo hubiera verificado. Devuelve el comando.
pub fn fabricated_run(text: &str) -> Option<String> {
    let lower = text.to_lowercase();
    // juez-idioma: datos — se detectan los DOS idiomas a propósito (bloque)
    let claims = [
        "la respuesta del servidor",
        "la respuesta es",
        "la salida es",
        "salida:",
        "resultado:",
        "el resultado es",
        "responde con",
        "devuelve:",
        "the response is",
        "the output is",
        "output:",
        "result:",
        "returns:",
        "it returns",
    ];
    let mut rest = text;
    while let Some(i) = rest.find("```") {
        let after = &rest[i + 3..];
        let lang_end = after.find('\n')?;
        let lang = after[..lang_end].trim().to_lowercase();
        let body_start = lang_end + 1;
        let close = after[body_start..].find("```")?;
        let body = after[body_start..body_start + close].trim();
        let tail_text = &after[body_start + close + 3..];
        if matches!(lang.as_str(), "bash" | "sh" | "shell" | "zsh" | "console")
            && !body.is_empty()
            && body.lines().count() <= 3
        {
            let tail_lower = tail_text.to_lowercase();
            let next_block = tail_lower.find("```");
            let window =
                &tail_lower[..next_block.unwrap_or(tail_lower.len()).min(tail_lower.len())];
            // otro bloque PEGADO al comando (sólo espacios entre ambos) también es su «salida»
            let glued = next_block.is_some_and(|n| tail_lower[..n].trim().is_empty());
            let claimed = glued
                || claims.iter().any(|c| window.contains(c))
                || (next_block.is_some() && claims.iter().any(|c| lower.contains(c)));
            if claimed && next_block.is_some() {
                return Some(body.trim_start_matches("$ ").to_string());
            }
        }
        rest = tail_text;
    }
    None
}

/// El primer bloque de shell de un texto (los comandos que el modelo PLANEA correr).
pub fn shell_block(text: &str) -> Option<String> {
    let mut rest = text;
    while let Some(i) = rest.find("```") {
        let after = &rest[i + 3..];
        let lang_end = after.find('\n')?;
        let lang = after[..lang_end].trim().to_lowercase();
        let body_start = lang_end + 1;
        let close = after[body_start..].find("```")?;
        let body = after[body_start..body_start + close].trim();
        if matches!(lang.as_str(), "bash" | "sh" | "shell" | "zsh" | "console") && !body.is_empty()
        {
            let cmds: Vec<&str> = body
                .lines()
                .map(|l| l.trim().trim_start_matches("$ "))
                .filter(|l| !l.is_empty() && !l.starts_with('#'))
                .collect();
            if !cmds.is_empty() {
                return Some(cmds.join("\n"));
            }
        }
        rest = &after[body_start + close + 3..];
    }
    None
}

/// ¿El texto da por HECHAS acciones («se creó», «se instaló correctamente», «done»)?
/// Junto con «no se ejecutó ninguna herramienta en este turno», es un informe inventado
/// (medido 2026-11-09: el 32b escribió los comandos y un informe de éxito de 5 pasos sin
/// correr nada; la carpeta quedó vacía).
pub fn claims_done(text: &str) -> bool {
    let t = text.to_lowercase();
    // juez-idioma: datos — se detectan los DOS idiomas a propósito (bloque)
    let claims = [
        "se creó",
        "se creo ",
        "se instaló",
        "se instalo ",
        "se ejecutó",
        "se ejecuto ",
        "se completaron",
        "se completó",
        "exitosamente",
        "correctamente",
        "con éxito",
        "con exito",
        "funcionó",
        "quedó corriendo",
        "está corriendo",
        "successfully",
        "was created",
        "were installed",
        "was installed",
        "it ran",
        "is running",
        "completed",
    ];
    claims.iter().filter(|c| t.contains(*c)).count() >= 2
}

pub fn invented_report_notice() -> &'static str {
    crate::i18n::t(
        "En este turno NO ejecutaste ninguna herramienta: todo lo que dices que pasó es INVENTADO (no hay archivos ni resultados). Hazlo de verdad AHORA llamando las herramientas (`bash`, `write_file`…) paso a paso, y responde sólo con lo que te devuelvan.",
        "In this turn you did NOT run any tool: everything you say happened is MADE UP (there are no files or results). Do it for real NOW by calling the tools (`bash`, `write_file`…) step by step, and answer only with what they return.",
    )
}

/// El texto sin sus bloques de código (```…```).
pub fn without_code_blocks(t: &str) -> String {
    let mut out = String::new();
    let mut inside = false;
    for line in t.lines() {
        if line.trim_start().starts_with("```") {
            inside = !inside;
            continue;
        }
        if !inside {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

/// Archivos que la respuesta dice haber escrito/creado/guardado y que NO existen en el
/// proyecto (medido 2026-11-09: «He escrito esta información en `investigacion.md`» y el
/// archivo no existía). La prueba es el disco, no las palabras.
pub fn claimed_missing_files(text: &str, root: &std::path::Path) -> Vec<String> {
    let lower = text.to_lowercase();
    // juez-idioma: datos — se detectan los DOS idiomas a propósito (bloque)
    let verbs = [
        "he escrito",
        "he creado",
        "he guardado",
        "he generado",
        "escribí",
        "creé",
        "guardé",
        "se escribió",
        "se creó",
        "se guardó",
        "quedó guardad",
        "i wrote",
        "i've written",
        "i created",
        "i've created",
        "i saved",
        "was written",
        "was created",
        "was saved",
        "has been written",
        "has been created",
        "has been saved",
    ];
    let mut out = Vec::new();
    for v in verbs {
        let mut from = 0;
        while let Some(i) = lower[from..].find(v) {
            let s = from + i + v.len();
            // un nombre de archivo en las siguientes ~80 letras
            let window: String = text[s..].chars().take(80).collect();
            for tok in window.split(|c: char| {
                c.is_whitespace() || matches!(c, '`' | '"' | '\'' | '(' | ')' | ',' | ':' | '*')
            }) {
                let tok = tok.trim_end_matches(['.', ';']);
                let looks_file = tok.contains('.')
                    && !tok.starts_with("http")
                    && tok.rsplit('.').next().is_some_and(|e| {
                        (1..=5).contains(&e.len()) && e.chars().all(|c| c.is_ascii_alphanumeric())
                    });
                if looks_file && !root.join(tok).exists() && !out.contains(&tok.to_string()) {
                    out.push(tok.to_string());
                }
            }
            from = s;
        }
    }
    out
}

pub fn missing_files_notice(files: &[String]) -> String {
    crate::tr!(
        "Dices que escribiste {} pero NO existe en el proyecto (lo comprobé en el disco). Escríbelo de verdad con write_file y responde sólo después.",
        "You say you wrote {} but it does NOT exist in the project (I checked the disk). Write it for real with write_file and answer only afterwards.",
        files.join(", ")
    )
}

/// Todos los bloques de shell de un texto, cada uno con sus comandos limpios.
pub fn shell_blocks(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(b) = shell_block(rest) {
        out.push(b);
        // avanzar más allá del bloque encontrado
        let Some(i) = rest.find("```") else { break };
        let after = &rest[i + 3..];
        let Some(close) = after.find("```") else {
            break;
        };
        rest = &after[close + 3..];
    }
    out
}

/// Los bloques de shell que el texto da por hechos y que NO se ejecutaron en este turno
/// (medido 2026-11-09: el 32b corrió 2 comandos y en su informe dio por hechos otros 3 —
/// `main.py` y el archivo de la carpeta personal— que nunca corrió).
pub fn unexecuted_blocks(text: &str, executed: &[String]) -> Vec<String> {
    let norm = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
    let ran: Vec<String> = executed.iter().map(|c| norm(c)).collect();
    shell_blocks(text)
        .into_iter()
        .filter(|b| {
            let nb = norm(b);
            !ran.iter()
                .any(|r| r == &nb || r.contains(&nb) || nb.contains(r.as_str()) && !r.is_empty())
        })
        .collect()
}

/// ¿Este resultado de herramienta es un FALLO? (salida ≠ 0, la caja negó algo, error o
/// permiso negado). Devuelve las líneas que lo prueban.
pub fn failure_lines(result: &str) -> Option<String> {
    let first = result.lines().next().unwrap_or("");
    let failed_exit = first.starts_with("(exit ") && !first.starts_with("(exit 0)");
    let blocked = result.contains("🔒") || result.contains("Operation not permitted");
    let error =
        first.starts_with("ERROR") || first.starts_with("DENEGADO") || first.starts_with("DENIED");
    if !(failed_exit || blocked || error) {
        return None;
    }
    let key: Vec<&str> = result
        .lines()
        .filter(|l| {
            let t = l.to_lowercase();
            t.contains("error")
                || t.contains("not permitted")
                || t.contains("no such file")
                || t.contains("denied")
                || t.contains("denegado")
                || t.starts_with("(exit")
                || t.contains("🔒")
                || t.contains("traceback")
        })
        .take(6)
        .collect();
    Some(if key.is_empty() {
        first.to_string()
    } else {
        key.join("\n")
    })
}

pub fn contradicts_result_notice(evidence: &str) -> String {
    crate::tr!(
        "Tu respuesta dice que todo salió bien, pero tu ÚLTIMA herramienta FALLÓ:\n{evidence}\nNo afirmes lo que no pasó. Corrige lo que falló con las herramientas (p. ej. crea el archivo que falta con write_file) o informa con honestidad qué funcionó y qué no.",
        "Your answer says everything went well, but your LAST tool FAILED:\n{evidence}\nDo not claim what did not happen. Fix what failed with the tools (e.g. create the missing file with write_file) or report honestly what worked and what did not."
    )
}

pub fn unrun_block_notice() -> &'static str {
    crate::i18n::t(
        "Los comandos que escribiste en el bloque NO se ejecutaron: escribirlos no los corre. Llama la herramienta `bash` con ellos AHORA (un servidor con background=true) y sigue con su salida real.",
        "The commands you wrote in the block did NOT run: writing them does not run them. Call the `bash` tool with them NOW (a server with background=true) and continue with their real output.",
    )
}

pub fn fabricated_run_notice(cmd: &str) -> String {
    crate::tr!(
        "NO ejecutaste `{cmd}`: lo escribiste en el texto y la «respuesta» que pusiste es inventada. Ejecútalo \
         de verdad con la herramienta `bash` y responde SÓLO con la salida real que te devuelva.",
        "You did NOT run `{cmd}`: you wrote it in the text and the «response» you showed is made up. Run it \
         for real with the `bash` tool and answer ONLY with the real output it returns."
    )
}

pub fn fake_call_notice(name: &str) -> String {
    crate::tr!(
        "No se ejecutó nada: escribir `{name}(…)` en el texto NO llama a la herramienta y el disco sigue igual. \
         Llama la herramienta de verdad (tool call) y espera su resultado antes de decir que está hecho.",
        "Nothing ran: writing `{name}(…)` in the text does NOT call the tool and the disk is unchanged. \
         Call the tool for real (tool call) and wait for its result before saying it is done."
    )
}

/// Convierte las llamadas ESCRITAS como código en llamadas reales, sólo si cada
/// una se analiza sin ambigüedad y trae exactamente los parámetros del esquema.
/// Acepta `edit(path="a", old='x', new="""y""")` y `edit({"path": "a", …})`.
pub fn parse_pseudo_calls(text: &str, step: u32) -> Vec<ToolCall> {
    parse_pseudo_calls_en(text, step, &crate::tools::schemas())
}

/// Igual, contra la lista de herramientas del agente (medido 2026-11-12: el Qwen3-32B escribió
/// ```json {"name": "isometrica", "arguments": {…}}``` y no se rescató porque `isometrica` no está
/// entre las del núcleo).
pub fn parse_pseudo_calls_en(text: &str, step: u32, schemas: &Value) -> Vec<ToolCall> {
    let mut found: Vec<(usize, ToolCall)> = Vec::new();
    let specs = specs_de(schemas);
    for (name, required, props) in &specs {
        for start in call_sites(text, name) {
            let Some(args) = parse_args(&text[start..]) else {
                continue;
            };
            let ok_keys = args.keys().all(|k| props.contains(k));
            let has_req = required.iter().all(|r| args.contains_key(r));
            if ok_keys && has_req && !args.is_empty() {
                found.push((
                    start,
                    ToolCall {
                        id: String::new(),
                        name: name.clone(),
                        arguments: Value::Object(args),
                    },
                ));
            }
        }
    }
    // Forma 5 (medida con --lang en): el bloque JSON de la llamada SIN etiquetas,
    // ```json {"name": "read_file", "arguments": {...}} ```.
    for (start, _) in text.match_indices('{') {
        let mut de = serde_json::Deserializer::from_str(&text[start..]).into_iter::<Value>();
        let Some(Ok(v)) = de.next() else { continue };
        let (Some(name), Some(args)) = (v.get("name").and_then(Value::as_str), v.get("arguments"))
        else {
            continue;
        };
        let args = match args {
            Value::Object(o) => o.clone(),
            Value::String(s) => match serde_json::from_str::<Value>(s) {
                Ok(Value::Object(o)) => o,
                _ => continue,
            },
            _ => continue,
        };
        if let Some((_, required, props)) = specs.iter().find(|(n, _, _)| n == name) {
            if args.keys().all(|k| props.contains(k))
                && required.iter().all(|r| args.contains_key(r))
            {
                found.push((
                    start,
                    ToolCall {
                        id: String::new(),
                        name: name.to_string(),
                        arguments: Value::Object(args),
                    },
                ));
            }
        }
    }
    found.sort_by_key(|(i, _)| *i);
    found.dedup_by(|a, b| a.1.name == b.1.name && a.1.arguments == b.1.arguments);
    found
        .into_iter()
        .enumerate()
        .map(|(i, (_, mut c))| {
            c.id = format!("call_{step}_txt{i}");
            c
        })
        .collect()
}

/// Argumentos tras el `(`: un objeto JSON, o `clave=valor` al estilo Python.
fn parse_args(s: &str) -> Option<Map<String, Value>> {
    let mut p = Cursor { s, i: 0 };
    p.ws();
    if p.peek() == Some('{') {
        let mut de = serde_json::Deserializer::from_str(&s[p.i..]).into_iter::<Value>();
        let v = de.next()?.ok()?;
        p.i += de.byte_offset();
        p.ws();
        return (p.peek() == Some(')')).then_some(v.as_object()?.clone());
    }
    let mut out = Map::new();
    loop {
        p.ws();
        if p.eat(')') {
            return Some(out);
        }
        let key = p.ident()?;
        p.ws();
        if !p.eat('=') && !p.eat(':') {
            return None;
        }
        p.ws();
        let val = p.value()?;
        out.insert(key, val);
        p.ws();
        if p.eat(',') {
            continue;
        }
        p.ws();
        return p.eat(')').then_some(out);
    }
}

struct Cursor<'a> {
    s: &'a str,
    i: usize,
}

impl Cursor<'_> {
    fn rest(&self) -> &str {
        &self.s[self.i..]
    }
    fn peek(&self) -> Option<char> {
        self.rest().chars().next()
    }
    fn eat(&mut self, c: char) -> bool {
        if self.peek() == Some(c) {
            self.i += c.len_utf8();
            true
        } else {
            false
        }
    }
    fn ws(&mut self) {
        while let Some(c) = self.peek() {
            if c.is_whitespace() {
                self.i += c.len_utf8();
            } else if c == '#' {
                // comentario de Python hasta fin de línea
                let n = self.rest().find('\n').unwrap_or(self.rest().len());
                self.i += n;
            } else {
                break;
            }
        }
    }
    fn ident(&mut self) -> Option<String> {
        let n = self
            .rest()
            .char_indices()
            .find(|(_, c)| !(c.is_alphanumeric() || *c == '_'))
            .map(|(i, _)| i)
            .unwrap_or(self.rest().len());
        if n == 0 {
            return None;
        }
        let id = self.rest()[..n].to_string();
        self.i += n;
        Some(id)
    }
    fn value(&mut self) -> Option<Value> {
        let r = self.rest();
        for (lit, v) in [
            ("True", Value::Bool(true)),
            ("False", Value::Bool(false)),
            ("None", Value::Null),
            ("true", Value::Bool(true)),
            ("false", Value::Bool(false)),
        ] {
            if r.starts_with(lit)
                && !r[lit.len()..].starts_with(|c: char| c.is_alphanumeric() || c == '_')
            {
                self.i += lit.len();
                return Some(v);
            }
        }
        match self.peek()? {
            '"' | '\'' => self.string().map(Value::String),
            '[' | '{' => {
                let mut de = serde_json::Deserializer::from_str(r).into_iter::<Value>();
                let v = de.next()?.ok()?;
                self.i += de.byte_offset();
                Some(v)
            }
            c if c == '-' || c.is_ascii_digit() => {
                let n = r
                    .char_indices()
                    .find(|(_, c)| !(c.is_ascii_digit() || *c == '.' || *c == '-'))
                    .map(|(i, _)| i)
                    .unwrap_or(r.len());
                let v: Value = serde_json::from_str(&r[..n]).ok()?;
                self.i += n;
                Some(v)
            }
            _ => None,
        }
    }
    /// Cadena de Python: '…', "…", '''…''' o """…""", con escapes básicos.
    fn string(&mut self) -> Option<String> {
        let r = self.rest();
        let q = r.chars().next()?;
        let triple: String = std::iter::repeat_n(q, 3).collect();
        let (delim, open) = if r.starts_with(&triple) {
            (triple.clone(), 3)
        } else {
            (q.to_string(), 1)
        };
        let body = &r[open..];
        let mut out = String::new();
        let mut it = body.char_indices();
        while let Some((i, c)) = it.next() {
            if body[i..].starts_with(&delim) {
                self.i += open + i + delim.len();
                return Some(out);
            }
            if c == '\\' {
                let (_, e) = it.next()?;
                out.push(match e {
                    'n' => '\n',
                    't' => '\t',
                    'r' => '\r',
                    '0' => '\0',
                    other => other,
                });
            } else if c == '\n' && open == 1 {
                return None; // una cadena simple no cruza líneas
            } else {
                out.push(c);
            }
        }
        None
    }
}

/// Detector de bucle sobre el texto que va llegando: el mismo trozo repetido
/// SEGUIDO al final, al menos 4 veces y cubriendo ≥ 240 bytes (un trozo corto
/// necesita más vueltas). Medido: «I will read the file first:\n\n» (29 B) se
/// repitió cientos de veces; con el umbral viejo de 60 B no se cazaba (951 s).
pub fn is_looping(text: &str) -> bool {
    repite(text, 4, 240)
}

/// Para los ARGUMENTOS de una llamada (un archivo que se escribe): vara más estricta. Un HTML
/// legítimo puede traer 10 `<div class="col"></div>` iguales; un bucle de verdad (medido en D5:
/// «Pistacho Chía Coco Vainilla…» durante 11 k caracteres) repite decenas de veces.
pub fn is_looping_args(text: &str) -> bool {
    repite(text, 8, 600)
}

fn repite(text: &str, min_reps: usize, min_bytes: usize) -> bool {
    let b = text.as_bytes();
    let n = b.len();
    if n < min_bytes {
        return false;
    }
    for period in 12..=600.min(n / min_reps) {
        let reps = min_reps.max(min_bytes.div_ceil(period));
        if period * reps > n {
            continue;
        }
        let block = &b[n - period..];
        // `====…` o `0000…` (separadores, relleno) no son un bucle del modelo
        let distinct = block
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len();
        if distinct >= 4 && (1..reps).all(|k| *block == b[n - period * (k + 1)..n - period * k]) {
            return true;
        }
    }
    false
}

/// Filtro de streaming: no pintar las llamadas que el modelo escribe como TEXTO —
/// `<tool_call>{…}</tool_call>` crudo y los bloques ```json {"name": …} ``` / ```python
/// edit(…) ``` que `parse_pseudo_calls` reconoce—. GOTV las ejecuta igual (y el
/// trabajo se ve en su desplegable); verlas en la respuesta sólo ensucia. Un bloque de
/// código cualquiera se retiene hasta que cierra y se muestra entero si NO es llamada.
#[derive(Default)]
pub struct CallFilter {
    inside: bool,
    fence: bool,
    pending: String,
    /// las herramientas del agente (sin ellas, sólo las del núcleo)
    esquemas: Option<Value>,
}

const OPEN: &str = "<tool_call>";
const CLOSE: &str = "</tool_call>";
const FENCE: &str = "```";

impl CallFilter {
    /// Un filtro que reconoce TODAS las herramientas del agente (diseño, oficina, imagen…).
    pub fn con(esquemas: Value) -> CallFilter {
        CallFilter { esquemas: Some(esquemas), ..Default::default() }
    }

    fn es_llamada(&self, block: &str) -> bool {
        match &self.esquemas {
            Some(e) => !parse_pseudo_calls_en(block, 0, e).is_empty(),
            None => is_call_block(block),
        }
    }

    pub fn feed(&mut self, chunk: &str) -> String {
        self.pending.push_str(chunk);
        let mut out = String::new();
        loop {
            if self.inside {
                match self.pending.find(CLOSE) {
                    Some(i) => {
                        self.pending.drain(..i + CLOSE.len());
                        self.inside = false;
                    }
                    None => {
                        // guardar sólo lo que podría ser el inicio del cierre
                        let keep = partial_suffix(&self.pending, CLOSE);
                        let cut = self.pending.len() - keep;
                        self.pending.drain(..cut);
                        return out;
                    }
                }
            } else if self.fence {
                // el bloque va desde su ``` hasta el ``` que lo cierra (tras la 1.ª línea)
                let body_from = self.pending.find('\n').map(|i| i + 1);
                let close = body_from.and_then(|f| self.pending[f..].find(FENCE).map(|j| f + j));
                let Some(c) = close else { return out };
                let block: String = self.pending.drain(..c + FENCE.len()).collect();
                self.fence = false;
                if !self.es_llamada(&block) {
                    out.push_str(&block);
                }
            } else {
                let next = [
                    (self.pending.find(OPEN), true),
                    (self.pending.find(FENCE), false),
                ]
                .into_iter()
                .filter_map(|(i, tag)| i.map(|i| (i, tag)))
                .min_by_key(|(i, _)| *i);
                match next {
                    Some((i, true)) => {
                        out.push_str(&self.pending[..i]);
                        self.pending.drain(..i + OPEN.len());
                        self.inside = true;
                    }
                    Some((i, false)) => {
                        out.push_str(&self.pending[..i]);
                        self.pending.drain(..i);
                        self.fence = true;
                    }
                    None => {
                        let keep = partial_suffix(&self.pending, OPEN)
                            .max(partial_suffix(&self.pending, FENCE));
                        let cut = self.pending.len() - keep;
                        out.push_str(&self.pending[..cut]);
                        self.pending.drain(..cut);
                        return out;
                    }
                }
            }
        }
    }

    /// Fin del stream: suelta lo retenido (un bloque sin cerrar se muestra si no es
    /// una llamada; un `<tool_call>` sin cerrar se descarta).
    /// ¿Lo que llega ahora está DENTRO de una llamada escrita como texto (`<tool_call>` crudo o un
    /// bloque de código aún sin cerrar)? Para el tope elástico: eso es escribir, no hablar.
    pub fn en_llamada(&self) -> bool {
        self.inside || self.fence
    }

    pub fn finish(&mut self) -> String {
        let rest = std::mem::take(&mut self.pending);
        let (inside, fence) = (self.inside, self.fence);
        self.inside = false;
        self.fence = false;
        if inside || (fence && self.es_llamada(&rest)) {
            String::new()
        } else {
            rest
        }
    }
}

/// ¿Este bloque de código es una llamada escrita como texto?
fn is_call_block(block: &str) -> bool {
    !parse_pseudo_calls(block, 0).is_empty()
}

/// El texto de una respuesta SIN las llamadas escritas como texto (lo que verá el
/// usuario al reabrir la conversación: lo mismo que vio en vivo).
pub fn strip_calls(text: &str) -> String {
    strip_calls_con(text, CallFilter::default())
}

/// Igual, reconociendo todas las herramientas del agente.
pub fn strip_calls_en(text: &str, esquemas: &Value) -> String {
    strip_calls_con(text, CallFilter::con(esquemas.clone()))
}

fn strip_calls_con(text: &str, mut f: CallFilter) -> String {
    let mut out = f.feed(text);
    out.push_str(&f.finish());
    // las llamadas quitadas dejan saltos de línea huérfanos
    let mut s = out.trim().to_string();
    while s.contains("\n\n\n") {
        s = s.replace("\n\n\n", "\n\n");
    }
    s
}

/// Largo del sufijo de `s` que es prefijo de `tag` (un `<tool_` partido entre trozos).
fn partial_suffix(s: &str, tag: &str) -> usize {
    (1..tag.len().min(s.len() + 1))
        .rev()
        .find(|&k| s.is_char_boundary(s.len() - k) && tag.starts_with(&s[s.len() - k..]))
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    #[test]
    fn bucle_en_argumentos_con_vara_estricta() {
        let html: String = (0..10).map(|_| "<div class=\"col\"></div>\n").collect();
        assert!(
            !is_looping_args(&html),
            "diez celdas iguales son HTML legítimo"
        );
        let filas: String = (0..60)
            .map(|i| {
                format!(
                    "  {{ id: {i}, nombre: \"Producto {i}\", precio: {} }},\n",
                    1000 + i * 50
                )
            })
            .collect();
        assert!(
            !is_looping_args(&filas),
            "filas con datos distintos no son bucle"
        );
        let bucle = format!(
            "{{ id: 73, nombre: \"Cappuccino {}",
            "Pistacho Chía Coco Vainilla Menta Caramelo ".repeat(40)
        );
        assert!(is_looping_args(&bucle), "el bucle medido en D5 se corta");
    }

    use super::*;
    use serde_json::json;

    #[test]
    fn atascos_reales_del_32b() {
        assert_eq!(
            stall("Aquí está el cambio.\n\n¿Deseas que realice la edición en el archivo?"),
            Some(Stall::AsksToProceed)
        );
        assert_eq!(
            stall("¿Quieres que aplique estos cambios?"),
            Some(Stall::AsksToProceed)
        );
        assert_eq!(
            stall("Listo el plan. ¿Procedo?"),
            Some(Stall::AsksToProceed)
        );
        assert_eq!(stall("Shall I apply it?"), Some(Stall::AsksToProceed));
        assert_eq!(
            stall("Tengo permiso para hacer cambios en el archivo app.py. Procedo a cambiar el texto de despedida()."),
            Some(Stall::Announces)
        );
        assert_eq!(stall("I'll now edit the file."), Some(Stall::Announces));
        // respuestas legítimas: NO se tocan
        assert_eq!(stall("¿Qué nombre le pongo al archivo?"), None);
        assert_eq!(stall("¿Quieres que use la versión A o la B?"), None);
        assert_eq!(stall("Hecho: cambié saludo()."), None);
        assert_eq!(stall("Listo, ya está hecho. Voy a dejarlo así."), None);
        assert_eq!(stall("El archivo tiene 3 funciones: a, b y c."), None);
        // prueba Cable (2026-11-12): presente tras «ahora» y frase que promete lo que sigue con «:»
        assert_eq!(
            stall("OK, tengo los tests. Ahora leo las secciones relevantes de `almacen.py` para ver los problemas:"),
            Some(Stall::Announces)
        );
        assert_eq!(stall("Ahora ejecuto las pruebas para ver qué falla."), Some(Stall::Announces));
        assert_eq!(stall("Estos son los cambios que faltan:"), Some(Stall::Announces));
        assert_eq!(stall("Listo, las pruebas pasan. Esto es lo que hice:"), None, "lo terminado no se empuja");
    }

    #[test]
    fn llamada_escrita_como_python_se_vuelve_real() {
        let t = "Ahora, reemplazo:\n```python\n# Reemplazo el texto\nedit(\n    path=\"app.py\",\n    old='return \"adios\"',\n    new='return \"hasta pronto\"'\n)\n```\nLa edición se ha realizado con éxito.";
        let c = parse_pseudo_calls(t, 3);
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].name, "edit");
        assert_eq!(
            c[0].arguments,
            json!({"path": "app.py", "old": "return \"adios\"", "new": "return \"hasta pronto\""})
        );
        assert_eq!(c[0].id, "call_3_txt0");
    }

    #[test]
    fn llamada_json_y_triple_comilla() {
        let c = parse_pseudo_calls("write_file({\"path\": \"a.txt\", \"content\": \"x\"})", 1);
        assert_eq!(c[0].arguments, json!({"path": "a.txt", "content": "x"}));
        let c = parse_pseudo_calls(
            "write_file(path='b.py', content=\"\"\"def f():\n    return 1\n\"\"\", append=False)",
            1,
        );
        assert_eq!(
            c[0].arguments,
            json!({"path": "b.py", "content": "def f():\n    return 1\n", "append": false})
        );
    }

    #[test]
    fn bloque_json_sin_etiquetas_se_vuelve_real() {
        let t = "```json\n{\n  \"name\": \"read_file\",\n  \"arguments\": {\n    \"path\": \"app.py\"\n  }\n}\n```";
        let c = parse_pseudo_calls(t, 1);
        assert_eq!(c.len(), 1);
        assert_eq!(
            (c[0].name.as_str(), &c[0].arguments),
            ("read_file", &json!({"path": "app.py"}))
        );
        // arguments como cadena JSON (estilo OpenAI) también
        let c = parse_pseudo_calls(r#"{"name": "bash", "arguments": "{\"cmd\": \"ls\"}"}"#, 1);
        assert_eq!(c[0].arguments, json!({"cmd": "ls"}));
        // un JSON cualquiera con "name" que no es herramienta: nada
        assert!(parse_pseudo_calls(r#"{"name": "Ana", "arguments": {}}"#, 1).is_empty());
        assert!(
            parse_pseudo_calls(r#"{"name": "read_file", "arguments": {"ruta": "x"}}"#, 1)
                .is_empty()
        );
    }

    #[test]
    fn lo_ambiguo_no_se_ejecuta() {
        // falta un parámetro requerido, sobra uno inventado, o es código del usuario
        assert!(parse_pseudo_calls("edit(path=\"a\", old=\"x\")", 1).is_empty());
        assert!(
            parse_pseudo_calls("edit(path=\"a\", old=\"x\", new=\"y\", force=True)", 1).is_empty()
        );
        assert!(parse_pseudo_calls("files = glob.glob('*.py')", 1).is_empty());
        assert!(parse_pseudo_calls("def my_edit(path='a'): pass", 1).is_empty());
        assert!(parse_pseudo_calls("edit(path=variable, old='x', new='y')", 1).is_empty());
        assert_eq!(
            pseudo_call_name("edit(path=variable)").as_deref(),
            Some("edit")
        );
    }

    #[test]
    fn bucle_real_se_detecta_y_texto_normal_no() {
        let frase = "The despedida() function in app.py now returns \"hasta pronto\" instead of \"adios\". \n\nI've completed the task. The file has been updated with the new return value. \n\n";
        let mut t = String::from("I'll start by reading the file.\n");
        for _ in 0..3 {
            t.push_str(frase);
        }
        assert!(!is_looping(&t), "3 repeticiones aún no es bucle");
        t.push_str(frase);
        assert!(is_looping(&t));
        // el bucle corto real (29 B) también
        let corto = format!("Plan:\n{}", "I will read the file first:\n\n".repeat(9));
        assert!(is_looping(&corto));
        assert!(!is_looping(&format!(
            "Plan:\n{}",
            "I will read the file first:\n\n".repeat(3)
        )));
        assert!(!is_looping(&format!("x = 1\n{}", "=".repeat(400))));
        assert!(!is_looping(&"0".repeat(1000)));
        // un archivo con líneas parecidas pero no idénticas: no es bucle
        let csv: String = (0..200)
            .map(|i| format!("{i},sensor,{},ok\n", i * 7))
            .collect();
        assert!(!is_looping(&csv));
    }

    #[test]
    fn filtro_oculta_tool_call_aunque_llegue_partido() {
        let mut f = CallFilter::default();
        let mut out = String::new();
        for chunk in [
            "Leo el archivo.\n\n<tool",
            "_call>\n{\"name\": \"read_file\", ",
            "\"arguments\": {}}\n</tool_",
            "call>",
            " fin",
        ] {
            out += &f.feed(chunk);
        }
        assert_eq!(out, "Leo el archivo.\n\n fin");
        let mut g = CallFilter::default();
        assert_eq!(g.feed("a < b y c <tool"), "a < b y c ");
        assert_eq!(g.feed("s>"), "<tools>");
    }

    /// Caso real del Portador (2026-11-09): el 32B escribió la llamada como bloque JSON.
    #[test]
    fn filtro_oculta_bloque_json_de_llamada_y_deja_el_codigo_normal() {
        let mut f = CallFilter::default();
        let mut out = String::new();
        for chunk in ["Voy a listar la carpeta.\n\n``", "`json\n{\n  \"name\": \"list_dir\",\n  \"arguments\": {\n    \"path\": \"\"\n  }\n}\n`", "``\n", "Listo."] {
            out += &f.feed(chunk);
        }
        out += &f.finish();
        assert_eq!(out, "Voy a listar la carpeta.\n\n\nListo.");
        // un bloque de código de verdad se muestra entero
        let code = "Así:\n```rust\nfn main() { println!(\"hola\"); }\n```\nfin";
        let mut g = CallFilter::default();
        let shown = g.feed(&code[..15]) + &g.feed(&code[15..]) + &g.finish();
        assert_eq!(shown, code);
        // sin cerrar al final del stream: se muestra (no era llamada)
        let mut h = CallFilter::default();
        assert_eq!(h.feed("```bash\nls -la"), "");
        assert_eq!(h.finish(), "```bash\nls -la");
        // la historia al reabrir queda igual de limpia
        assert_eq!(strip_calls("Voy.\n\n```json\n{\"name\": \"list_dir\", \"arguments\": {\"path\": \".\"}}\n```\n\nHecho."), "Voy.\n\nHecho.");
    }

    #[test]
    fn detecta_una_verificacion_inventada() {
        let t = "El servidor corre.\n\nVerificando con `curl`:\n```bash\ncurl -s http://localhost:5173\n```\n\nLa respuesta del servidor es:\n```html\n<title>Demo</title>\n```\nTodo bien.";
        assert_eq!(
            fabricated_run(t).as_deref(),
            Some("curl -s http://localhost:5173")
        );
        // instrucciones PARA el usuario (sin «respuesta» inventada) no son fingir
        let u = "Para arrancarlo:\n```bash\nnpm run dev\n```\nLuego abre el navegador.";
        assert_eq!(fabricated_run(u), None);
        // medido: la 2.ª vez pegó el HTML justo detrás, sin «la respuesta es»
        let g = "```bash\ncurl -s http://localhost:5173\n```\n```html\n<title>Demo</title>\n```";
        assert_eq!(
            fabricated_run(g).as_deref(),
            Some("curl -s http://localhost:5173")
        );
    }

    #[test]
    fn el_futuro_en_primera_persona_es_un_anuncio() {
        assert_eq!(stall("```bash\nnpm install\n```\n\nLuego, verificaré con `curl` que el servidor responde."), Some(Stall::Announces));
        assert_eq!(
            stall("Instalaré las dependencias y arrancaré el servidor."),
            Some(Stall::Announces)
        );
        assert_eq!(stall("I'll run the tests next."), Some(Stall::Announces));
        // lo hecho no es anuncio, y un futuro ajeno tampoco
        assert_eq!(
            stall("Listo: lo verificaré si me lo pides, pero ya está hecho."),
            None
        );
        assert_eq!(
            stall("El servidor responde en http://localhost:5173."),
            None
        );
    }

    #[test]
    fn saca_los_comandos_de_un_bloque() {
        let t = "Así:\n```bash\n$ npm create vite@latest demo -- --template react-ts --no-interactive\ncd demo\n# deps\nnpm install\n```\nLuego verificaré.";
        assert_eq!(shell_block(t).as_deref(), Some("npm create vite@latest demo -- --template react-ts --no-interactive\ncd demo\nnpm install"));
        assert_eq!(shell_block("```rust\nfn main(){}\n```"), None);
    }

    #[test]
    fn un_informe_de_exito_se_reconoce() {
        assert!(claims_done(
            "1. El entorno se creó exitosamente.\n2. requests se instaló correctamente."
        ));
        assert!(!claims_done("Voy a crear el entorno."));
        assert!(!claims_done(
            "El servidor responde en http://localhost:5173."
        ));
    }

    #[test]
    fn los_bloques_no_ejecutados_se_separan_de_los_corridos() {
        let t = "```bash\npython3 -m venv .venv\n```\n```bash\ncat > main.py <<'EOF'\nprint(1)\nEOF\n```\n```bash\necho hola > ~/x.txt\n```";
        assert_eq!(shell_blocks(t).len(), 3);
        let faltan = unexecuted_blocks(t, &["python3 -m venv .venv".to_string()]);
        assert_eq!(faltan.len(), 2);
        assert!(faltan[1].contains("~/x.txt"));
    }

    #[test]
    fn reconoce_un_resultado_fallido() {
        let r = "(exit 1)\nCollecting requests\npython: can't open file 'main.py': [Errno 2] No such file or directory\nbash: line 6: /Users/x/a.txt: Operation not permitted\n🔒 la caja bloqueó una escritura";
        let e = failure_lines(r).unwrap();
        assert!(
            e.contains("No such file") && e.contains("not permitted"),
            "{e}"
        );
        assert!(failure_lines("(exit 0)\nok").is_none());
        assert!(failure_lines("DENEGADO por permisos: x").is_some());
    }

    #[test]
    fn el_anuncio_antes_de_un_bloque_y_los_archivos_afirmados_que_no_existen() {
        let t = "Plan:\n1. leer\n\nVamos a empezar con el primer paso.\n\n```bash\ngrep -r \"def suma\" .\n```";
        assert_eq!(
            stall(t),
            Some(Stall::Announces),
            "la prosa manda, no el cierre del bloque"
        );
        let d = crate::CarpetaPrueba::sin_crear("gotv-afirma");
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("existe.md"), "x").unwrap();
        let r = "He escrito esta información en el archivo `investigacion.md` con la fuente. También se creó existe.md.";
        assert_eq!(
            claimed_missing_files(r, &d),
            vec!["investigacion.md".to_string()]
        );
        assert!(claimed_missing_files("Lee la guía en https://x.dev/guide.html", &d).is_empty());
    }
}
