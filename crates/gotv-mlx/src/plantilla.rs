//! La plantilla de chat DEL MODELO (su `chat_template` jinja, con herramientas) y la lectura de
//! su salida: texto, `<think>…</think>` (razonamiento) y `<tool_call>{json}</tool_call>`.

use anyhow::{anyhow, Context, Result};
use gotv_engine::tr;
use gotv_engine::{Message, StreamEvent, ToolCall};
use serde_json::{json, Value};
use std::path::Path;

pub struct Plantilla {
    entorno: minijinja::Environment<'static>,
    pub bos: Option<String>,
    /// cómo marca ESTE modelo el razonamiento y las llamadas (se deduce de su plantilla)
    pub etiquetas: Etiquetas,
    /// N4: la plantilla escribe los resultados de herramienta como `<tool_response>` (familia Qwen):
    /// los avisos del programa a mitad de una orden van así y no como mensaje de usuario.
    avisos_como_resultado: bool,
}

/// Las marcas de razonamiento y de llamada de una familia.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Etiquetas {
    pub abre_t: &'static str,
    pub cierra_t: &'static str,
    pub abre_l: &'static str,
    pub cierra_l: &'static str,
}

/// Qwen, Llama, Mistral…: `<think>` y `<tool_call>`.
pub const QWEN: Etiquetas = Etiquetas { abre_t: "<think>", cierra_t: "</think>", abre_l: "<tool_call>", cierra_l: "</tool_call>" };
/// Gemma 4: canal de pensamiento y `<|tool_call>call:nombre{…}<tool_call|>`.
pub const GEMMA: Etiquetas = Etiquetas { abre_t: "<|channel>", cierra_t: "<channel|>", abre_l: "<|tool_call>", cierra_l: "<tool_call|>" };

impl Default for Etiquetas {
    fn default() -> Self {
        QWEN
    }
}

impl Plantilla {
    pub fn cargar(dir: &Path) -> Result<Plantilla> {
        let tc: Value = serde_json::from_str(&std::fs::read_to_string(dir.join("tokenizer_config.json")).context("tokenizer_config.json")?)?;
        // la plantilla vive en tokenizer_config.json o en chat_template.jinja
        let texto: String = match tc.get("chat_template") {
            Some(Value::String(s)) => s.clone(),
            _ => std::fs::read_to_string(dir.join("chat_template.jinja")).context(tr!("el modelo no trae chat_template", "the model has no chat_template"))?,
        };
        let mut entorno = minijinja::Environment::new();
        minijinja_contrib::add_to_environment(&mut entorno);
        entorno.set_unknown_method_callback(minijinja_contrib::pycompat::unknown_method_callback);
        entorno.add_function("raise_exception", |m: String| -> std::result::Result<String, minijinja::Error> {
            Err(minijinja::Error::new(minijinja::ErrorKind::InvalidOperation, m))
        });
        let etiquetas = if texto.contains("<|tool_call>") { GEMMA } else { QWEN };
        // sólo donde hace falta y se puede: la plantilla distingue «la última pregunta del usuario»
        // (es lo que reescribe los turnos anteriores) y sabe escribir resultados de herramienta
        let avisos_como_resultado = texto.contains("<tool_response>") && texto.contains("last_query_index");
        entorno.add_template_owned("chat", texto).map_err(|e| anyhow!("plantilla de chat: {e}"))?;
        let bos = tc.get("bos_token").and_then(|b| b.as_str().or_else(|| b.get("content").and_then(|c| c.as_str()))).map(String::from);
        Ok(Plantilla { entorno, bos, etiquetas, avisos_como_resultado })
    }

    /// M7 · la plantilla como `apply_chat_template(mensajes, add_generation_prompt=True)` de
    /// transformers SIN más variables (sin `enable_thinking` ni herramientas): así la aplica el LM
    /// de ACE-Step. `mensajes` = [(rol, contenido)].
    pub fn render_simple(&self, mensajes: &[(&str, &str)]) -> Result<String> {
        let msgs: Vec<Value> = mensajes.iter().map(|(r, c)| json!({"role": r, "content": c})).collect();
        let ctx = minijinja::context! { messages => msgs, add_generation_prompt => true, bos_token => self.bos.clone().unwrap_or_default() };
        let t = self.entorno.get_template("chat").map_err(|e| anyhow!("{e}"))?;
        t.render(ctx).map_err(|e| anyhow!("plantilla de chat: {e:#}"))
    }

    /// El prompt completo para estos mensajes y herramientas (con el turno del asistente abierto).
    pub fn prompt(&self, mensajes: &[Message], herramientas: Option<&Value>) -> Result<String> {
        let msgs: Vec<Value> = mensajes
            .iter()
            .map(|m| {
                // M8/M1b: con adjuntos el contenido va como lista — imágenes, el texto y los audios
                // (el orden de mlx_vlm) — y la plantilla pone su marca por cada uno
                let (img, aud) = crate::adjuntos(m);
                let contenido = if img.is_empty() && aud.is_empty() {
                    json!(m.content)
                } else {
                    let mut partes: Vec<Value> = img.iter().map(|_| json!({"type": "image"})).collect();
                    partes.push(json!({"type": "text", "text": m.content}));
                    partes.extend(aud.iter().map(|_| json!({"type": "audio"})));
                    Value::Array(partes)
                };
                // N4: un aviso del programa a mitad de la orden se escribe como resultado de herramienta
                // (si la plantilla los tiene): así no cambia cómo se escriben los turnos anteriores y
                // el prompt sigue siendo continuación del que ya se leyó
                if m.es_aviso() && self.avisos_como_resultado {
                    return json!({"role": "tool", "content": contenido});
                }
                let mut o = json!({"role": m.role, "content": contenido});
                if let Some(tc) = &m.tool_calls {
                    // la plantilla de Qwen admite argumentos como objeto o como texto: van como objeto
                    let llamadas: Vec<Value> = tc
                        .iter()
                        .map(|c| {
                            let f = c.get("function").cloned().unwrap_or(json!({}));
                            let args = match f.get("arguments") {
                                Some(Value::String(s)) => serde_json::from_str::<Value>(s).unwrap_or(Value::String(s.clone())),
                                Some(v) => v.clone(),
                                None => json!({}),
                            };
                            json!({"id": c.get("id"), "type": "function", "function": {"name": f.get("name"), "arguments": args}, "name": f.get("name"), "arguments": args})
                        })
                        .collect();
                    o["tool_calls"] = Value::Array(llamadas);
                }
                if let Some(id) = &m.tool_call_id {
                    o["tool_call_id"] = json!(id);
                }
                if let Some(n) = &m.name {
                    o["name"] = json!(n);
                }
                o
            })
            .collect();
        let herramientas = herramientas.and_then(|h| h.as_array()).cloned().unwrap_or_default();
        let ctx = minijinja::context! {
            messages => msgs,
            tools => if herramientas.is_empty() { Value::Null } else { Value::Array(herramientas) },
            add_generation_prompt => true,
            // un agente quiere actuar: sin la fase de «pensar» larga (como el motor servidor)
            enable_thinking => false,
            bos_token => self.bos.clone().unwrap_or_default(),
        };
        let t = self.entorno.get_template("chat").map_err(|e| anyhow!("{e}"))?;
        t.render(ctx).map_err(|e| anyhow!("plantilla de chat: {e:#}"))
    }
}

/// Lee la salida del modelo mientras llega: texto visible, razonamiento y llamadas.
#[derive(Default)]
pub struct Lector {
    pub texto: String,
    pub razonamiento: String,
    pub llamadas: Vec<ToolCall>,
    pendiente: String,
    estado: Estado,
    args: String,
    /// los esquemas de las herramientas: el formato XML (Qwen3.5/Qwen3-Coder) trae los valores como
    /// texto y el tipo de cada parámetro sale de aquí (entero, booleano, lista…)
    pub herramientas: Option<Value>,
    /// las marcas de la familia (Qwen por omisión)
    pub etiquetas: Etiquetas,
}

#[derive(Default, PartialEq)]
enum Estado {
    #[default]
    Texto,
    Pensando,
    Llamada,
}


impl Lector {
    /// ¿Está dentro de una llamada a herramienta? (el tope elástico deja seguir sólo ahí)
    pub fn en_llamada(&self) -> bool {
        self.estado == Estado::Llamada
    }

    /// Añade un trozo nuevo; emite los eventos de lo que ya es seguro (una etiqueta a medias se guarda).
    pub fn trozo(&mut self, t: &str, on: &(dyn Fn(StreamEvent) + Send + Sync)) {
        self.pendiente.push_str(t);
        let Etiquetas { abre_t, cierra_t, abre_l, cierra_l } = self.etiquetas;
        loop {
            let cierra: Option<&str> = match self.estado {
                Estado::Texto => None,
                Estado::Pensando => Some(cierra_t),
                Estado::Llamada => Some(cierra_l),
            };
            match self.estado {
                Estado::Texto => {
                    let pos = [abre_t, abre_l].iter().filter_map(|e| self.pendiente.find(e).map(|p| (p, *e))).min_by_key(|x| x.0);
                    match pos {
                        Some((p, e)) => {
                            let antes: String = self.pendiente[..p].to_string();
                            self.emitir_texto(&antes, on);
                            self.pendiente = self.pendiente[p + e.len()..].to_string();
                            self.estado = if e == abre_t { Estado::Pensando } else { Estado::Llamada };
                            self.args.clear();
                        }
                        None => {
                            // guarda una posible etiqueta a medias al final («<too»)
                            let seguro = corte_seguro(&self.pendiente, &[abre_t, abre_l]);
                            let listo: String = self.pendiente[..seguro].to_string();
                            self.emitir_texto(&listo, on);
                            self.pendiente = self.pendiente[seguro..].to_string();
                            return;
                        }
                    }
                }
                _ => {
                    let cierra = cierra.unwrap();
                    match self.pendiente.find(cierra) {
                        Some(p) => {
                            let dentro: String = self.pendiente[..p].to_string();
                            self.pendiente = self.pendiente[p + cierra.len()..].to_string();
                            if self.estado == Estado::Pensando {
                                self.razonamiento.push_str(&dentro);
                                if !dentro.trim().is_empty() {
                                    on(StreamEvent::Reasoning(dentro));
                                }
                            } else {
                                on(StreamEvent::ToolArgs(dentro.clone()));
                                self.args.push_str(&dentro);
                                let crudo = std::mem::take(&mut self.args);
                                self.cerrar_llamada(&crudo);
                            }
                            self.estado = Estado::Texto;
                        }
                        None => {
                            let seguro = corte_seguro(&self.pendiente, &[cierra]);
                            let listo: String = self.pendiente[..seguro].to_string();
                            self.pendiente = self.pendiente[seguro..].to_string();
                            if self.estado == Estado::Pensando {
                                if !listo.is_empty() {
                                    self.razonamiento.push_str(&listo);
                                    on(StreamEvent::Reasoning(listo));
                                }
                            } else if !listo.is_empty() {
                                self.args.push_str(&listo);
                                on(StreamEvent::ToolArgs(listo));
                            }
                            return;
                        }
                    }
                }
            }
        }
    }

    fn emitir_texto(&mut self, t: &str, on: &(dyn Fn(StreamEvent) + Send + Sync)) {
        if t.is_empty() {
            return;
        }
        self.texto.push_str(t);
        on(StreamEvent::Text(t.to_string()));
    }

    fn cerrar_llamada(&mut self, crudo: &str) {
        let v: Value = match serde_json::from_str(crudo.trim()) {
            Ok(v) => v,
            // Qwen3.5 / Qwen3-Coder: <function=nombre><parameter=p>valor</parameter></function>
            Err(_) if crudo.contains("<function=") => llamada_xml(crudo, self.herramientas.as_ref()),
            // Gemma 4: call:nombre{clave:<|"|>texto<|"|>,n:3}
            Err(_) if crudo.trim_start().starts_with("call:") => llamada_gemma(crudo),
            // roto: el agente lo repara o le dice al modelo por qué (como con el servidor)
            Err(_) => json!({"name": "", "arguments": crudo.trim()}),
        };
        let nombre = v.get("name").and_then(|n| n.as_str()).unwrap_or("").to_string();
        let args = match v.get("arguments").or(v.get("parameters")) {
            Some(Value::String(s)) => serde_json::from_str(s).unwrap_or(Value::String(s.clone())),
            Some(a) => a.clone(),
            None => json!({}),
        };
        let id = format!("call_{}", self.llamadas.len() + 1);
        self.llamadas.push(ToolCall { id, name: nombre, arguments: args });
    }

    /// Fin del flujo: lo que quedó pendiente se vuelca donde corresponde.
    pub fn terminar(&mut self, on: &(dyn Fn(StreamEvent) + Send + Sync)) {
        let resto = std::mem::take(&mut self.pendiente);
        match self.estado {
            Estado::Texto => self.emitir_texto(&resto, on),
            Estado::Pensando => self.razonamiento.push_str(&resto),
            Estado::Llamada => {
                self.args.push_str(&resto);
                let crudo = std::mem::take(&mut self.args);
                self.cerrar_llamada(&crudo);
            }
        }
    }
}

/// Una llamada en el formato XML de Qwen3-Coder/Qwen3.5, como `mlx_lm/tool_parsers/qwen3_coder.py`:
/// cada valor llega como texto (sin el salto de línea de cada borde) y se convierte según el tipo
/// que declara el esquema de la herramienta; sin esquema queda como texto.
fn llamada_xml(crudo: &str, herramientas: Option<&Value>) -> Value {
    let Some(ini) = crudo.find("<function=") else { return json!({"name": "", "arguments": crudo.trim()}) };
    let cuerpo = &crudo[ini + "<function=".len()..];
    let Some(fin_nombre) = cuerpo.find('>') else { return json!({"name": "", "arguments": crudo.trim()}) };
    let nombre = cuerpo[..fin_nombre].trim().to_string();
    let props = herramientas
        .and_then(|h| h.as_array())
        .and_then(|h| h.iter().find(|t| t["function"]["name"].as_str() == Some(nombre.as_str())))
        .map(|t| t["function"]["parameters"]["properties"].clone())
        .unwrap_or(Value::Null);
    let mut args = serde_json::Map::new();
    let mut resto = &cuerpo[fin_nombre + 1..];
    while let Some(a) = resto.find("<parameter=") {
        let tras = &resto[a + "<parameter=".len()..];
        let (Some(cierre_n), Some(fin)) = (tras.find('>'), tras.find("</parameter>")) else { break };
        if cierre_n > fin {
            break;
        }
        let p = tras[..cierre_n].trim().to_string();
        let mut val = &tras[cierre_n + 1..fin];
        val = val.strip_prefix('\n').unwrap_or(val);
        val = val.strip_suffix('\n').unwrap_or(val);
        args.insert(p.clone(), valor_tipado(val, props.get(&p)));
        resto = &tras[fin + "</parameter>".len()..];
    }
    json!({"name": nombre, "arguments": Value::Object(args)})
}

fn valor_tipado(v: &str, esquema: Option<&Value>) -> Value {
    if v.eq_ignore_ascii_case("null") {
        return Value::Null;
    }
    let tipo = esquema.and_then(|e| e["type"].as_str()).unwrap_or("string").to_lowercase();
    let texto = || Value::String(v.to_string());
    match tipo.as_str() {
        "string" | "str" | "text" | "varchar" | "char" | "enum" => texto(),
        t if t.starts_with("int") || t.starts_with("uint") || t.starts_with("long") => v.trim().parse::<i64>().map(Value::from).unwrap_or_else(|_| texto()),
        t if t.starts_with("num") || t.starts_with("float") => match v.trim().parse::<f64>() {
            Ok(f) if f.fract() == 0.0 && f.abs() < 9e15 => Value::from(f as i64),
            Ok(f) => Value::from(f),
            Err(_) => texto(),
        },
        "boolean" | "bool" => Value::Bool(v.trim().eq_ignore_ascii_case("true")),
        _ => serde_json::from_str(v).unwrap_or_else(|_| texto()),
    }
}

/// Una llamada de Gemma 4 (`call:nombre{…}`) → `{"name", "arguments"}`, como
/// `mlx_lm/tool_parsers/gemma4.py`: los textos van entre `<|"|>` y las claves sin comillas.
fn llamada_gemma(crudo: &str) -> Value {
    let roto = || json!({"name": "", "arguments": crudo.trim()});
    let t = crudo.trim().trim_start_matches("call:");
    let (Some(a), Some(z)) = (t.find('{'), t.rfind('}')) else { return roto() };
    let nombre = t[..a].trim().to_string();
    // 1) los textos <|"|>…<|"|> salen a un lado (dentro pueden ir llaves, comas o dos puntos)
    let mut textos: Vec<String> = Vec::new();
    let mut cuerpo = String::new();
    let mut resto = &t[a..=z];
    const C: &str = "<|\"|>";
    while let Some(i) = resto.find(C) {
        cuerpo.push_str(&resto[..i]);
        let tras = &resto[i + C.len()..];
        let Some(f) = tras.find(C) else { return roto() };
        cuerpo.push_str(&format!("\u{0}{}\u{0}", textos.len()));
        textos.push(tras[..f].to_string());
        resto = &tras[f + C.len()..];
    }
    cuerpo.push_str(resto);
    // 2) comillas a las claves desnudas: justo tras «{» o «,», una palabra y «:»
    let cs: Vec<char> = cuerpo.chars().collect();
    let mut conv = String::new();
    let mut i = 0;
    while i < cs.len() {
        conv.push(cs[i]);
        if cs[i] == '{' || cs[i] == ',' {
            let mut j = i + 1;
            while j < cs.len() && (cs[j].is_alphanumeric() || cs[j] == '_') {
                j += 1;
            }
            if j > i + 1 && j < cs.len() && cs[j] == ':' {
                conv.push('"');
                conv.extend(&cs[i + 1..j]);
                conv.push('"');
                i = j;
                continue;
            }
        }
        i += 1;
    }
    // 3) los textos vuelven como cadenas JSON bien escapadas
    for (n, s) in textos.iter().enumerate() {
        conv = conv.replace(&format!("\u{0}{n}\u{0}"), &serde_json::to_string(s).unwrap_or_default());
    }
    match serde_json::from_str::<Value>(&conv) {
        Ok(args) => json!({"name": nombre, "arguments": args}),
        Err(_) => roto(),
    }
}

/// Hasta dónde se puede emitir sin partir una etiqueta que quizá viene a medias.
fn corte_seguro(s: &str, etiquetas: &[&str]) -> usize {
    let mut corte = s.len();
    for e in etiquetas {
        for k in (1..e.len()).rev() {
            if s.ends_with(&e[..k]) {
                corte = corte.min(s.len() - k);
                break;
            }
        }
    }
    while !s.is_char_boundary(corte) {
        corte -= 1;
    }
    corte
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lee_texto_razonamiento_y_llamadas_aunque_lleguen_partidos() {
        let eventos = std::sync::Mutex::new(Vec::new());
        let on = |e: StreamEvent| eventos.lock().unwrap().push(e);
        let mut l = Lector::default();
        let salida = "<think>miro</think>Voy a leerlo.<tool_call>\n{\"name\": \"read_file\", \"arguments\": {\"path\": \"a.txt\"}}\n</tool_call>";
        // de 3 en 3 caracteres: las etiquetas llegan partidas
        let cs: Vec<char> = salida.chars().collect();
        for t in cs.chunks(3) {
            l.trozo(&t.iter().collect::<String>(), &on);
        }
        l.terminar(&on);
        assert_eq!(l.texto, "Voy a leerlo.");
        assert_eq!(l.razonamiento, "miro");
        assert_eq!(l.llamadas.len(), 1);
        assert_eq!(l.llamadas[0].name, "read_file");
        assert_eq!(l.llamadas[0].arguments, json!({"path": "a.txt"}));
        let ev = eventos.lock().unwrap();
        assert!(ev.iter().any(|e| matches!(e, StreamEvent::ToolArgs(_))), "los argumentos se ven mientras llegan");
        assert!(!ev.iter().any(|e| matches!(e, StreamEvent::Text(t) if t.contains('<'))), "ninguna etiqueta sale como texto");
    }

    /// N4 (medido en el Air, 2026-11-14): la plantilla de Qwen escribe `<think>` sólo en los turnos de
    /// la IA POSTERIORES al último mensaje del usuario. Un aviso de GOTV a mitad del trabajo, si va
    /// como mensaje de usuario, cambia cómo se escriben todos los turnos anteriores de la orden: el
    /// prompt ya no continúa el anterior y la caché no sirve (9 171 tokens releídos por un aviso).
    /// El mismo aviso como resultado de herramienta deja el prompt anterior intacto.
    /// Sólo corre si está la plantilla (`GOTV_PLANTILLA_PRUEBA=<carpeta del modelo>`).
    #[test]
    fn un_aviso_a_mitad_de_la_orden_no_reescribe_lo_anterior() {
        let Some(dir) = std::env::var_os("GOTV_PLANTILLA_PRUEBA").map(std::path::PathBuf::from) else { return };
        let p = Plantilla::cargar(&dir).unwrap();
        let llamada = |i: usize| gotv_engine::ToolCall { id: format!("c{i}"), name: "bash".into(), arguments: json!({"cmd": format!("ls {i}")}) };
        let mut m = vec![Message::system("sistema"), Message::user("la orden")];
        for i in 0..6 {
            m.push(Message::assistant_calls("", std::slice::from_ref(&llamada(i))));
            m.push(Message::tool_result(&llamada(i), "salida ".repeat(200)));
        }
        let comun = |a: &str, b: &str| a.chars().zip(b.chars()).take_while(|(x, y)| x == y).count();
        let antes = p.prompt(&m, None).unwrap();
        // lo que el motor ya leyó: todo menos el turno del asistente que se abre al final
        let leido = antes.rfind("<|im_start|>").unwrap();
        let mut como_usuario = m.clone();
        como_usuario.push(Message::user("GOTV: aviso"));
        let mut como_resultado = m.clone();
        como_resultado.push(Message::aviso("GOTV: aviso"));
        let (u, r) = (comun(&antes, &p.prompt(&como_usuario, None).unwrap()), comun(&antes, &p.prompt(&como_resultado, None).unwrap()));
        eprintln!("prompt ya leído: {leido} caracteres · sigue igual con el aviso como usuario: {u} · como resultado de herramienta: {r}");
        assert!(u < leido / 4, "como usuario se reescribe casi todo: {u} de {leido}");
        assert!(r + 64 >= leido, "como resultado de herramienta el prompt anterior queda intacto: {r} de {leido}");
    }

    /// El formato XML de Qwen3.5 tal como lo emitió el 9B real, con tipos del esquema.
    #[test]
    fn lee_llamadas_en_xml_de_qwen35() {
        let on = |_e: StreamEvent| {};
        let mut l = Lector { herramientas: Some(json!([{"type":"function","function":{"name":"edit","parameters":{"type":"object","properties":{
            "path":{"type":"string"},"linea":{"type":"integer"},"todas":{"type":"boolean"},"lista":{"type":"array"}}}}}])), ..Default::default() };
        for trozo in ["<tool_call>\n<function=read_file>\n<parameter=path>\nnotas.txt\n</parameter>\n</function>\n</tool_call>",
            "<tool_call>\n<function=edit>\n<parameter=path>\na b.py\n</parameter>\n<parameter=linea>\n12\n</parameter>\n<parameter=todas>\ntrue\n</parameter>\n<parameter=lista>\n[1, 2]\n</parameter>\n<parameter=texto>\nuna\ndos\n</parameter>\n</function>\n</tool_call>"] {
            for c in trozo.as_bytes().chunks(7) {
                l.trozo(std::str::from_utf8(c).unwrap(), &on);
            }
        }
        l.terminar(&on);
        assert_eq!(l.llamadas.len(), 2);
        assert_eq!(l.llamadas[0].name, "read_file");
        assert_eq!(l.llamadas[0].arguments, json!({"path": "notas.txt"}));
        assert_eq!(l.llamadas[1].name, "edit");
        assert_eq!(l.llamadas[1].arguments, json!({"path": "a b.py", "linea": 12, "todas": true, "lista": [1, 2], "texto": "una\ndos"}));
        assert!(l.texto.trim().is_empty(), "{:?}", l.texto);
    }

    /// El formato de Gemma 4 (como lo emite el e4b), partido en trozos al azar.
    #[test]
    fn lee_llamadas_de_gemma4() {
        let on = |_e: StreamEvent| {};
        let mut l = Lector { etiquetas: GEMMA, ..Default::default() };
        let salida = "Voy a leerlo.<|tool_call>call:read_file{path:<|\"|>notas, {raro}: 1.txt<|\"|>}<tool_call|><|tool_call>call:edit{n:3,todas:true,lista:[1,2],dentro:{clave:<|\"|>a\"b<|\"|>}}<tool_call|>";
        for c in salida.as_bytes().chunks(5) {
            l.trozo(&String::from_utf8_lossy(c), &on);
        }
        l.terminar(&on);
        assert_eq!(l.texto, "Voy a leerlo.");
        assert_eq!(l.llamadas.len(), 2, "{:?}", l.llamadas);
        assert_eq!(l.llamadas[0].name, "read_file");
        assert_eq!(l.llamadas[0].arguments, json!({"path": "notas, {raro}: 1.txt"}));
        assert_eq!(l.llamadas[1].name, "edit");
        assert_eq!(l.llamadas[1].arguments, json!({"n": 3, "todas": true, "lista": [1, 2], "dentro": {"clave": "a\"b"}}));
    }

    #[test]
    fn una_llamada_cortada_no_se_pierde() {
        let on = |_e: StreamEvent| {};
        let mut l = Lector::default();
        l.trozo("<tool_call>{\"name\": \"bash\", \"arguments\": {\"cmd\": \"ls\"}}", &on);
        l.terminar(&on);
        assert_eq!(l.llamadas[0].name, "bash");
        assert_eq!(l.llamadas[0].arguments, json!({"cmd": "ls"}));
    }

    #[test]
    fn la_plantilla_de_qwen3_con_herramientas() {
        let dir = std::path::PathBuf::from(std::env::var("HOME").unwrap()).join(".cache/gotv/models/mlx-community/Qwen3-0.6B-4bit");
        if !dir.exists() {
            eprintln!("sin el modelo de prueba: se salta");
            return;
        }
        let p = Plantilla::cargar(&dir).unwrap();
        let mut asst = Message::assistant("");
        asst.tool_calls = Some(vec![json!({"id": "call_1", "type": "function", "function": {"name": "read_file", "arguments": "{\"path\":\"a.txt\"}"}})]);
        let mut herr = Message::user("hola");
        herr.role = "tool".into();
        herr.content = "contenido".into();
        herr.tool_call_id = Some("call_1".into());
        let msgs = vec![Message::system("Eres GOTV."), Message::user("lee a.txt"), asst, herr];
        let tools = json!([{"type": "function", "function": {"name": "read_file", "description": "lee", "parameters": {"type": "object", "properties": {"path": {"type": "string"}}}}}]);
        let s = p.prompt(&msgs, Some(&tools)).unwrap();
        assert!(s.starts_with("<|im_start|>system\nEres GOTV."), "{s}");
        assert!(s.contains("<tools>") && s.contains("\"name\": \"read_file\"") || s.contains("\"name\":\"read_file\""), "{s}");
        assert!(s.contains("<tool_call>") && s.contains("<tool_response>\ncontenido\n</tool_response>"), "{s}");
        assert!(s.ends_with("<|im_start|>assistant\n<think>\n\n</think>\n\n") || s.ends_with("<|im_start|>assistant\n"), "{s}");
    }
}
