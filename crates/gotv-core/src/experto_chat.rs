//! Ola EXPERTO · EXP-5 — el chat del estudio: del lenguaje natural a operaciones VALIDADAS.
//!
//! El modelo local recibe un resumen de la sesión (pistas numeradas, grupos, duración, tempo) y SÓLO puede
//! responder con herramientas estructuradas (`herramientas()`). Cada llamada se traduce aquí a
//! `Accion`es: operaciones de la sesión (que valida `experto::aplicar`), separar una pista, deshacer, o
//! una pregunta cuando lo pedido es ambiguo. Las pistas se nombran por número o por nombre; «la batería de
//! la pista 1» se resuelve a la pista hija separada (y, si aún no existe, primero se separa).
use crate::experto::{Efecto, Estado, Filtro, Insertos, Op, Parametro, Pista, Punto, Region};
use anyhow::Result;
use serde_json::{json, Value};

/// Lo que pide una llamada del modelo, ya entendido.
#[derive(Debug, Clone, PartialEq)]
pub enum Accion {
    Op(Op),
    /// separar la pista con HTDemucs («4», «6» o «ft»)
    Separar { pista: u32, modelo: String },
    /// una región sobre la pista hija `fuente` de `pista`, que todavía no está separada: primero se separa
    /// y luego se aplica `efecto` en `[desde, hasta]`
    SepararYRegion { pista: u32, fuente: String, desde: f64, hasta: f64, efecto: Efecto },
    /// lo generativo (EXP-4): rehacer el tramo con esta descripción
    Reemplazar { pista: u32, desde: f64, hasta: f64, descripcion: String, letra: String, idioma: String },
    /// transponer el tramo a una PISTA NUEVA (pedido del Portador: lo transpuesto, en su propia pista)
    TonoAPista { pista: u32, desde: f64, hasta: f64, semitonos: f32 },
    /// X2-6: una pista nueva desde un texto, al tempo y en la tonalidad de la sesión (0 s = lo que dure)
    Generar { descripcion: String, segundos: f64, letra: String },
    /// X2-6: la canción (o una pista) sigue `segundos` más
    Extender { pista: Option<u32>, segundos: f64, descripcion: String },
    /// X2-6: el maestro, a esta sonoridad
    Masterizar { lufs: f64 },
    Deshacer,
    Pregunta(String),
}

/// Las claves de las fuentes de HTDemucs y cómo las llama la gente.
fn fuente_de(t: &str) -> Option<&'static str> {
    let t = t.trim().to_lowercase();
    let t = t.trim_start_matches("la ").trim_start_matches("el ").trim_start_matches("the "); // juez-idioma: datos (artículos que se quitan)
    // juez-idioma: bloque — datos: palabras que se reconocen
    let tabla: [(&[&str], &str); 6] = [
        (&["drums", "batería", "bateria", "drum", "percusión", "percusion"], "drums"),
        (&["vocals", "voz", "voces", "vocal", "voice", "cantante", "canto"], "vocals"),
        (&["bass", "bajo", "bajos"], "bass"),
        (&["guitar", "guitarra", "guitarras"], "guitar"),
        (&["piano", "teclado", "keys"], "piano"),
        (&["other", "resto", "otros", "the rest"], "other"),
    ];
    tabla.iter().find(|(ps, _)| ps.contains(&t)).map(|(_, k)| *k)
}

/// La pista a la que se refiere `v` (número, «3», un nombre o una fuente como «batería»), y si se nombró
/// una fuente que no tiene pista propia, la madre y la fuente para separarla.
pub enum Destino {
    Pista(u32),
    PorSeparar { madre: u32, fuente: String },
}

pub fn resolver(e: &Estado, pista: &Value, fuente: Option<&str>) -> Result<Destino> {
    let mut id: Option<u32> = pista.as_u64().map(|n| n as u32).or_else(|| pista.as_str().and_then(|s| s.trim().trim_start_matches(['#', 'p']).trim().parse().ok()));
    if id.is_none() {
        if let Some(s) = pista.as_str().map(str::trim).filter(|s| !s.is_empty()) {
            let s_min = s.to_lowercase();
            // un nombre (exacto o contenido) o una fuente separada
            id = e.pistas.iter().find(|p| p.nombre.to_lowercase() == s_min).or_else(|| e.pistas.iter().find(|p| p.nombre.to_lowercase().contains(&s_min))).map(|p| p.id);
            if id.is_none() {
                if let Some(f) = fuente_de(s) {
                    id = e.pistas.iter().find(|p| p.fuente.as_deref() == Some(f)).map(|p| p.id);
                    if id.is_none() {
                        // «la batería» sin pista: la de la única pista que no es hija de otra
                        let madres: Vec<&Pista> = e.pistas.iter().filter(|p| p.de.is_none()).collect();
                        if madres.len() == 1 {
                            return Ok(Destino::PorSeparar { madre: madres[0].id, fuente: f.into() });
                        }
                    }
                }
            }
        }
    }
    let id = id.filter(|id| e.pistas.iter().any(|p| p.id == *id)).ok_or_else(|| gotv_engine::anyhow_tr!("no encuentro la pista «{}»", "I cannot find the track «{}»", pista.as_str().map(String::from).unwrap_or_else(|| pista.to_string())))?;
    // «la batería de la pista 1»: la hija separada de esa pista (o separarla)
    if let Some(f) = fuente.and_then(fuente_de) {
        let p = e.pistas.iter().find(|p| p.id == id).unwrap();
        if p.fuente.as_deref() == Some(f) {
            return Ok(Destino::Pista(id));
        }
        if let Some(h) = e.pistas.iter().find(|h| h.de == Some(id) && h.fuente.as_deref() == Some(f)) {
            return Ok(Destino::Pista(h.id));
        }
        return Ok(Destino::PorSeparar { madre: id, fuente: f.into() });
    }
    Ok(Destino::Pista(id))
}

fn num(v: &Value, k: &str) -> Option<f64> {
    v.get(k).and_then(|x| x.as_f64().or_else(|| x.as_str().and_then(|s| s.trim().replace(',', ".").parse().ok())))
}

fn filtro(s: &str) -> Option<Filtro> {
    // juez-idioma: datos (nombres de filtro que se reconocen)
    Some(match s.trim().to_lowercase().replace(['_', '-'], " ").as_str() {
        "graves" | "low shelf" | "lowshelf" | "bass" | "bajos" => Filtro::Graves,
        "agudos" | "high shelf" | "highshelf" | "treble" | "brillo" => Filtro::Agudos,
        "campana" | "peaking" | "bell" | "medios" => Filtro::Campana,
        "paso alto" | "highpass" | "high pass" | "pasa altos" => Filtro::PasoAlto, // juez-idioma: datos (nombre de filtro que se reconoce)
        "paso bajo" | "lowpass" | "low pass" | "pasa bajos" => Filtro::PasoBajo, // juez-idioma: datos (nombre de filtro que se reconoce)
        "muesca" | "notch" => Filtro::Muesca, // juez-idioma: datos (nombre de filtro que se reconoce)
        "paso banda" | "bandpass" | "band pass" | "pasa banda" => Filtro::PasoBanda, // juez-idioma: datos (nombre de filtro que se reconoce)
        _ => return None,
    })
}

/// Traduce UNA llamada del modelo a acciones (sin aplicarlas).
pub fn traducir(e: &Estado, nombre: &str, a: &Value) -> Result<Vec<Accion>> {
    let tramo = || -> Result<(f64, f64)> {
        let (d, h) = (num(a, "desde").unwrap_or(0.0), num(a, "hasta").unwrap_or(-1.0));
        if h <= d {
            anyhow::bail!("{}", crate::i18n::t("falta el tramo: «desde» y «hasta» en segundos, con «hasta» mayor", "the span is missing: «desde» and «hasta» in seconds, with «hasta» larger"));
        }
        Ok((d, h))
    };
    let region = |efecto: Efecto| -> Result<Vec<Accion>> {
        let (desde, hasta) = tramo()?;
        Ok(vec![match resolver(e, &a["pista"], a.get("fuente").and_then(Value::as_str))? {
            Destino::Pista(id) => Accion::Op(Op::Region { region: Region { apagada: false, id: 0, pista: id, desde, hasta, efecto } }),
            Destino::PorSeparar { madre, fuente } => Accion::SepararYRegion { pista: madre, fuente, desde, hasta, efecto },
        }])
    };
    let pista_sola = || -> Result<u32> {
        match resolver(e, &a["pista"], a.get("fuente").and_then(Value::as_str))? {
            Destino::Pista(id) => Ok(id),
            Destino::PorSeparar { .. } => anyhow::bail!("{}", crate::i18n::t("esa parte todavía no es una pista: pide primero separarla", "that part is not a track yet: ask to split it first")),
        }
    };
    let fundido = num(a, "fundido").unwrap_or(0.03) as f32;
    Ok(match nombre {
        "region_ganancia" => region(Efecto::Ganancia { db: num(a, "db").unwrap_or(3.0) as f32, fundido })?,
        "region_silencio" => region(Efecto::Silencio { fundido })?,
        "region_eq" => {
            let f = a["filtro"].as_str().and_then(filtro).unwrap_or(Filtro::Agudos);
            let defecto_hz = match f {
                Filtro::Graves => 120.0,
                Filtro::Agudos => 4000.0,
                Filtro::Campana => 1000.0,
                Filtro::PasoAlto => 80.0,
                Filtro::PasoBajo => 8000.0,
                Filtro::Muesca | Filtro::PasoBanda => 1000.0,
            };
            region(Efecto::Eq { filtro: f, frecuencia: num(a, "frecuencia").unwrap_or(defecto_hz) as f32, db: num(a, "db").unwrap_or(4.0) as f32, q: num(a, "q").unwrap_or(0.707) as f32, fundido })?
        }
        "region_tono" => {
            let semitonos = num(a, "semitonos").unwrap_or(2.0) as f32;
            // por omisión a su PROPIA pista (se oye sola, se ecualiza, se rehace con IA); en el sitio, si se pide
            if a["en_sitio"].as_bool() == Some(true) {
                region(Efecto::Tono { semitonos, fundido })?
            } else {
                region(Efecto::Tono { semitonos, fundido })?
                    .into_iter()
                    .map(|x| match x {
                        Accion::Op(Op::Region { region: r }) => Accion::TonoAPista { pista: r.pista, desde: r.desde, hasta: r.hasta, semitonos },
                        otra => otra,
                    })
                    .collect()
            }
        }
        "region_reemplazar" => {
            let (desde, hasta) = tramo()?;
            let descripcion = a["descripcion"].as_str().unwrap_or("").trim().to_string();
            if descripcion.is_empty() {
                anyhow::bail!("{}", crate::i18n::t("falta qué poner en su lugar («descripcion», en inglés)", "what to put in its place is missing («descripcion», in English)"));
            }
            match resolver(e, &a["pista"], a.get("fuente").and_then(Value::as_str))? {
                Destino::Pista(id) => vec![Accion::Reemplazar { pista: id, desde, hasta, descripcion, letra: a["letra"].as_str().unwrap_or("").to_string(), idioma: a["idioma"].as_str().unwrap_or("").to_string() }],
                Destino::PorSeparar { madre, fuente } => vec![Accion::Separar { pista: madre, modelo: modelo_para(&fuente) }, Accion::Pregunta(crate::i18n::t("Separé la pista para tener esa parte por aparte; repite el pedido y la cambio.", "I split the track so that part is on its own; repeat the request and I will change it.").into())],
            }
        }
        "mezcla" => vec![Accion::Op(Op::Mezcla { pista: pista_sola()?, ganancia_db: num(a, "ganancia_db").map(|v| v as f32), pan: num(a, "pan").map(|v| v as f32), silencio: a["silencio"].as_bool(), solo: a["solo"].as_bool() })],
        "agrupar" => {
            let ids: Vec<u32> = a["pistas"].as_array().map(|v| v.iter().map(|p| match resolver(e, p, None)? {
                Destino::Pista(id) => Ok(id),
                Destino::PorSeparar { .. } => anyhow::bail!("{}", crate::i18n::t("alguna de esas partes todavía no es una pista", "one of those parts is not a track yet")),
            }).collect::<Result<_>>()).transpose()?.unwrap_or_default();
            let nombre = a["nombre"].as_str().unwrap_or("").trim();
            if ids.len() < 2 || nombre.is_empty() {
                anyhow::bail!("{}", crate::i18n::t("para agrupar hacen falta al menos 2 pistas y un nombre", "grouping needs at least 2 tracks and a name"));
            }
            let id = e.grupos.iter().map(|g| g.id).chain(e.pistas.iter().map(|p| p.id)).max().unwrap_or(0) + 1000;
            vec![
                Accion::Op(Op::CrearGrupo { grupo: crate::experto::Grupo { insertos: Default::default(), id, nombre: nombre.into(), ganancia_db: 0.0, pan: 0.0, silencio: false, solo: false } }),
                Accion::Op(Op::Agrupar { pistas: ids, grupo: Some(id) }),
            ]
        }
        "separar" => {
            let id = pista_sola()?;
            let m = a["modelo"].as_str().filter(|m| matches!(*m, "4" | "6" | "ft" | "bateria")).unwrap_or("4"); // juez-idioma: datos (argumento de la herramienta)
            vec![Accion::Separar { pista: id, modelo: m.into() }]
        }
        "tempo" => {
            let (bpm, compas) = (num(a, "bpm").unwrap_or(e.bpm as f64) as f32, num(a, "compas").map(|c| c as u8).unwrap_or(e.compas));
            // medido con el Qwen3-32B: a veces pone el tempo que ya está sin que se lo pidan: no es un cambio
            if (bpm - e.bpm).abs() < 0.01 && compas == e.compas { vec![] } else { vec![Accion::Op(Op::Tempo { bpm, compas })] }
        }
        "efectos" => {
            let id = pista_sola()?;
            // lo que viene se pone, lo que viene nulo (o false) se quita y lo demás no cambia
            let actual: Insertos = e.pistas.iter().find(|p| p.id == id).map(|p| p.insertos.clone()).unwrap_or_default();
            let mut v = serde_json::to_value(&actual)?;
            for k in ["compresor", "puerta", "delay", "reverb", "limitador", "saturacion", "deesser", "chorus", "eq", "ancho"] {
                if let Some(x) = a.get(k) {
                    if x.is_null() || x == &json!(false) {
                        v.as_object_mut().map(|o| o.remove(k));
                    } else {
                        v[k] = x.clone();
                    }
                }
            }
            vec![Accion::Op(Op::Insertos { pista: id, insertos: serde_json::from_value(v)? })]
        }
        // ── X2-6 · lo que el estudio nuevo sabe hacer ──
        "partir" => {
            let id = pista_sola()?;
            let segundo = num(a, "segundo").ok_or_else(|| gotv_engine::anyhow_tr!("falta el segundo donde partir", "the second to split at is missing"))?;
            vec![Accion::Op(Op::Partir { pista: id, segundo, clip: None })]
        }
        "quitar_tramo" => {
            let (id, (desde, hasta)) = (pista_sola()?, tramo()?);
            vec![Accion::Op(Op::QuitarTramo { pista: id, desde, hasta, cerrar: a["cerrar"].as_bool().unwrap_or(false) })]
        }
        "marcador" => {
            let segundo = num(a, "segundo").unwrap_or(0.0);
            let nombre = a["nombre"].as_str().map(str::trim).filter(|n| !n.is_empty()).map(String::from).unwrap_or_else(|| crate::tr!("Marcador {}", "Marker {}", e.marcadores.len() + 1));
            vec![Accion::Op(Op::Marcador { marcador: crate::experto::Marcador { segundo, nombre } })]
        }
        "fundido" => {
            // una rampa de volumen entre dos segundos: se conserva lo que la curva tuviera fuera del tramo
            let (id, (desde, hasta)) = (pista_sola()?, tramo()?);
            let (de, hacia) = (num(a, "de_db").unwrap_or(0.0) as f32, num(a, "a_db").unwrap_or(-60.0) as f32);
            let mut puntos: Vec<Punto> = e.automatizacion(id, Parametro::Volumen).map(|c| c.puntos.iter().copied().filter(|p| p.t < desde || p.t > hasta).collect()).unwrap_or_default();
            puntos.push(Punto { t: desde, v: de });
            puntos.push(Punto { t: hasta, v: hacia });
            puntos.sort_by(|x, y| x.t.total_cmp(&y.t));
            vec![Accion::Op(Op::Automatizar { pista: id, parametro: Parametro::Volumen, puntos })]
        }
        "maestro" => {
            let insertos = match a.get("limitador") {
                None => None,
                Some(x) => {
                    let mut i = e.maestro.insertos.clone();
                    i.limitador = if x.is_null() || x == &json!(false) { None } else { Some(serde_json::from_value(x.clone())?) };
                    Some(i)
                }
            };
            vec![Accion::Op(Op::Maestro { ganancia_db: num(a, "ganancia_db").map(|g| g as f32), insertos })]
        }
        "tonalidad" => vec![Accion::Op(Op::Tonalidad { tonalidad: a["tonalidad"].as_str().unwrap_or("").trim().to_string() })],
        "masterizar" => vec![Accion::Masterizar { lufs: num(a, "lufs").unwrap_or(-14.0) }],
        "generar_pista" => {
            let descripcion = a["descripcion"].as_str().unwrap_or("").trim().to_string();
            if descripcion.is_empty() {
                anyhow::bail!("{}", crate::i18n::t("falta decir qué música generar", "what music to generate is missing"));
            }
            vec![Accion::Generar { descripcion, segundos: num(a, "segundos").unwrap_or(0.0), letra: a["letra"].as_str().unwrap_or("").trim().to_string() }]
        }
        "extender" => {
            let pista = if a.get("pista").is_some_and(|p| !p.is_null()) { Some(pista_sola()?) } else { None };
            vec![Accion::Extender { pista, segundos: num(a, "segundos").unwrap_or(30.0), descripcion: a["descripcion"].as_str().unwrap_or("").trim().to_string() }]
        }
        "deshacer" => vec![Accion::Deshacer],
        "preguntar" => vec![Accion::Pregunta(a["pregunta"].as_str().unwrap_or("").trim().to_string())],
        otro => anyhow::bail!("{}", crate::tr!("herramienta desconocida: {otro}", "unknown tool: {otro}")),
    })
}

/// Guitarra o piano sólo los separa el modelo de 6 pistas.
pub fn modelo_para(fuente: &str) -> String {
    if matches!(fuente, "guitar" | "piano") { "6" } else { "4" }.into()
}

/// El resumen de la sesión que ve el modelo: cada pista con su número, su origen y su estado.
pub fn resumen(e: &Estado, duraciones: &dyn Fn(&Pista) -> Option<f64>) -> String {
    let mut s = format!("TEMPO {} bpm, {}/4\nTRACKS:\n", e.bpm.round(), e.compas);
    for p in &e.pistas {
        let mut extra = Vec::new();
        if let Some(f) = &p.fuente {
            extra.push(format!("part={f}"));
        }
        if let Some(m) = p.de {
            extra.push(format!("split from track {m}"));
        }
        if let Some(g) = p.grupo.and_then(|g| e.grupos.iter().find(|x| x.id == g)) {
            extra.push(format!("group «{}»", g.nombre));
        }
        if p.silencio {
            extra.push("muted".into());
        }
        if p.solo {
            extra.push("solo".into());
        }
        if p.ganancia_db != 0.0 {
            extra.push(format!("{:+} dB", p.ganancia_db));
        }
        if let Some(d) = duraciones(p) {
            extra.push(format!("{d:.1} s"));
        }
        s.push_str(&format!("  {} · «{}» ({})\n", p.id, p.nombre, extra.join(", ")));
    }
    if e.pistas.is_empty() {
        s.push_str("  (none)\n");
    }
    for r in &e.regiones {
        s.push_str(&format!("REGION {} on track {}: {:.2}-{:.2} s {}\n", r.id, r.pista, r.desde, r.hasta, serde_json::to_string(&r.efecto).unwrap_or_default()));
    }
    s
}

/// Las instrucciones del modelo (en inglés: el modelo las sigue mejor; responde en el idioma de la persona).
pub fn sistema() -> &'static str {
    "You are the assistant of a music studio (the «Expert» window of GOTV). The user gives orders about the session tracks in any language. \
     You MUST answer ONLY with real tool calls (never write JSON or prose in the reply), and ONLY the tools the order asks for: do NOT set the tempo, effects or anything else that was not asked. Use `preguntar` only when the order is truly ambiguous. \
     Tracks are referred to by their NUMBER in the TRACKS list or by name; «the drums of track 1» means `pista`: 1 and `fuente`: «drums» (it is split automatically if needed). \
     Times are seconds; «from second 5 to 10» → desde 5, hasta 10. \
     Rules: «louder» without amount = +3 dB, «much louder» = +6 dB, «softer» = −3 dB. «brighter/more treble/más agudo» meaning SOUND COLOR → region_eq agudos +4 dB at 4000 Hz; \
     «higher pitch/transpose/semitones/octave» → region_tono (12 semitones = one octave). If «más agudo»/«higher» could be either and nothing says which, prefer region_eq agudos. \
     «another type of drums/change the instrument/replace» in a span → region_reemplazar with an ENGLISH description of the new sound. \
     «cut/split at second X» → partir; «delete/remove from X to Y» → quitar_tramo (cerrar true only if asked to close the gap); «fade out the last N seconds» → fundido with de_db 0 and a_db −60 over those seconds; «marker at X» → marcador; \
     «master it/make it loud for streaming» → masterizar; «master volume» → maestro; «add/generate a new track with …» → generar_pista (ENGLISH description); «continue/extend the song N seconds» → extender. \
     A change to a WHOLE track with no times («lower the rest 4 dB», «louder vocals») → `mezcla` with ganancia_db = the track current dB shown in TRACKS plus the change. «mute track 2» → mezcla silencio true; «pan left» → pan −0.7; «group drums and bass as Rhythm» → agrupar. «undo» → deshacer. Several orders → several tool calls. /no_think"
}

/// Los esquemas de las herramientas (formato OpenAI).
pub fn herramientas() -> Value {
    let pista = json!({"type": ["integer", "string"], "description": "track number from the TRACKS list, or its name"});
    let fuente = json!({"type": "string", "enum": ["vocals", "drums", "bass", "other", "guitar", "piano"], "description": "a PART inside the track (its split stem), e.g. the drums of track 1"});
    let t = |n: &str, d: &str, props: Value, req: &[&str]| json!({"type": "function", "function": {"name": n, "description": d, "parameters": {"type": "object", "properties": props, "required": req}}});
    json!([
        t("region_ganancia", "Make a time span of a track louder (+dB) or softer (−dB).", json!({"pista": pista, "fuente": fuente, "desde": {"type": "number"}, "hasta": {"type": "number"}, "db": {"type": "number", "description": "−60…+24"}}), &["pista", "desde", "hasta", "db"]),
        t("region_silencio", "Silence a time span of a track.", json!({"pista": pista, "fuente": fuente, "desde": {"type": "number"}, "hasta": {"type": "number"}}), &["pista", "desde", "hasta"]),
        t("region_eq", "Equalize a time span: brighter (agudos +dB), darker (agudos −dB), more/less bass (graves), a bell (campana), or cut lows/highs (paso_alto / paso_bajo).", json!({"pista": pista, "fuente": fuente, "desde": {"type": "number"}, "hasta": {"type": "number"}, "filtro": {"type": "string", "enum": ["graves", "agudos", "campana", "paso_alto", "paso_bajo"]}, "frecuencia": {"type": "number", "description": "Hz"}, "db": {"type": "number", "description": "−24…+24"}}), &["pista", "desde", "hasta", "filtro", "db"]),
        t("region_tono", "Transpose (change PITCH of) a time span by semitones without changing its duration. The transposed part goes to its OWN new track (the original is silenced there) unless en_sitio is true.", json!({"pista": pista, "fuente": fuente, "desde": {"type": "number"}, "hasta": {"type": "number"}, "semitonos": {"type": "number", "description": "−12…+12"}, "en_sitio": {"type": "boolean", "description": "transpose inside the same track instead (only if the user asks)"}}), &["pista", "desde", "hasta", "semitonos"]),
        t("region_reemplazar", "Regenerate a time span with a DIFFERENT sound (another kind of drums, another instrument), keeping the rest of the song as context.", json!({"pista": pista, "fuente": fuente, "desde": {"type": "number"}, "hasta": {"type": "number"}, "descripcion": {"type": "string", "description": "the new sound, in English, concrete"}, "letra": {"type": "string", "description": "for VOCALS: the lyrics to sing (the whole song lyrics if known), in their language"}, "idioma": {"type": "string", "description": "lyrics language code (es, en…)"}}), &["pista", "desde", "hasta", "descripcion"]),
        t("mezcla", "Track mixer: volume (ganancia_db), pan (−1 left … 1 right), mute (silencio), solo.", json!({"pista": pista, "fuente": fuente, "ganancia_db": {"type": "number"}, "pan": {"type": "number"}, "silencio": {"type": "boolean"}, "solo": {"type": "boolean"}}), &["pista"]),
        t("agrupar", "Group tracks into a bus with a name (e.g. drums and bass as «Rhythm»).", json!({"pistas": {"type": "array", "items": pista}, "nombre": {"type": "string"}}), &["pistas", "nombre"]),
        t("separar", "Split a track into its parts with HTDemucs: «4» (vocals, drums, bass, other), «6» (+ guitar, piano), «ft» (finer) or «bateria» (a DRUMS track into kick, snare, cymbals, toms; on a full mix its drums are extracted first).", json!({"pista": pista, "modelo": {"type": "string", "enum": ["4", "6", "ft", "bateria"]}}), &["pista"]), // juez-idioma: datos (esquema de la herramienta)
        t("tempo", "Set the session tempo and meter.", json!({"bpm": {"type": "number"}, "compas": {"type": "integer"}}), &["bpm"]),
        t("efectos", "Track effects; pass null to remove one. compresor {umbral_db, razon, ataque_ms, liberacion_ms, ganancia_db}, puerta {umbral_db, ataque_ms, liberacion_ms}, delay {ms, realimentacion, mezcla}, reverb {tamano, amortiguacion, mezcla}, limitador {techo_db, empuje_db, liberacion_ms}, saturacion {empuje_db, mezcla}, deesser {frecuencia, umbral_db, reduccion_db}, chorus {velocidad_hz, profundidad_ms, mezcla}, ancho (stereo width 0…2), eq [{filtro, frecuencia, db, q}] (up to 8 bands).", json!({"pista": pista, "fuente": fuente, "limitador": {"type": ["object", "null"]}, "saturacion": {"type": ["object", "null"]}, "deesser": {"type": ["object", "null"]}, "chorus": {"type": ["object", "null"]}, "ancho": {"type": ["number", "null"]}, "eq": {"type": ["array", "null"]}, "compresor": {"type": ["object", "null"]}, "puerta": {"type": ["object", "null"]}, "delay": {"type": ["object", "null"]}, "reverb": {"type": ["object", "null"]}}), &["pista"]),
        t("partir", "Split (cut) the audio of a track at a second, into two clips.", json!({"pista": pista, "fuente": fuente, "segundo": {"type": "number"}}), &["pista", "segundo"]),
        t("quitar_tramo", "Delete the audio of a track between two seconds; cerrar true closes the gap (what comes after moves left).", json!({"pista": pista, "fuente": fuente, "desde": {"type": "number"}, "hasta": {"type": "number"}, "cerrar": {"type": "boolean"}}), &["pista", "desde", "hasta"]),
        t("fundido", "A volume ramp (fade) on a track between two seconds: fade out = de_db 0, a_db -60; fade in = de_db -60, a_db 0.", json!({"pista": pista, "fuente": fuente, "desde": {"type": "number"}, "hasta": {"type": "number"}, "de_db": {"type": "number"}, "a_db": {"type": "number"}}), &["pista", "desde", "hasta"]),
        t("marcador", "Put a named marker at a second.", json!({"segundo": {"type": "number"}, "nombre": {"type": "string"}}), &["segundo"]),
        t("maestro", "The master bus: its volume (ganancia_db) and its limiter {techo_db, empuje_db, liberacion_ms} (null removes it).", json!({"ganancia_db": {"type": "number"}, "limitador": {"type": ["object", "null"]}}), &[]),
        t("masterizar", "Master the whole mix to a target loudness in LUFS (-14 for streaming).", json!({"lufs": {"type": "number"}}), &[]),
        t("tonalidad", "Set the session key, e.g. «A minor» or «C major».", json!({"tonalidad": {"type": "string"}}), &["tonalidad"]),
        t("generar_pista", "Generate a NEW track with AI from an ENGLISH description (style, instruments, mood), at the session tempo and key. segundos 0 = as long as the session.", json!({"descripcion": {"type": "string"}, "segundos": {"type": "number"}, "letra": {"type": "string"}}), &["descripcion"]),
        t("extender", "Make the song (or one track) go on for more seconds with AI, from an ENGLISH description of how it continues.", json!({"pista": pista, "segundos": {"type": "number"}, "descripcion": {"type": "string"}}), &["segundos", "descripcion"]),
        t("deshacer", "Undo the last change.", json!({}), &[]),
        t("preguntar", "Ask the user ONE short question when the order is truly ambiguous (in the user's language).", json!({"pregunta": {"type": "string"}}), &["pregunta"]),
    ])
}

/// Una frase de lo que hace una acción (para la tarjeta del chat).
pub fn describir(e: &Estado, a: &Accion) -> String {
    let nombre = |id: u32| e.pistas.iter().find(|p| p.id == id).map(|p| format!("{id} «{}»", p.nombre)).unwrap_or_else(|| id.to_string());
    match a {
        Accion::Op(Op::Region { region: r }) => format!("{} · {:.2}–{:.2} s · {}", nombre(r.pista), r.desde, r.hasta, serde_json::to_string(&r.efecto).unwrap_or_default()),
        Accion::Op(op) => crate::experto::etiqueta_op(op),
        Accion::Generar { descripcion, .. } => crate::tr!("pista nueva: {descripcion}", "new track: {descripcion}"),
        Accion::Extender { segundos, descripcion, .. } => crate::tr!("extender {segundos:.0} s: {descripcion}", "extend {segundos:.0} s: {descripcion}"),
        Accion::Masterizar { lufs } => crate::tr!("masterizar a {lufs:.0} LUFS", "master to {lufs:.0} LUFS"),
        Accion::Separar { pista, modelo } => crate::tr!("separar {} (HTDemucs {modelo})", "split {} (HTDemucs {modelo})", nombre(*pista)),
        Accion::SepararYRegion { pista, fuente, desde, hasta, .. } => crate::tr!("separar {} y editar su «{fuente}» de {desde:.2} a {hasta:.2} s", "split {} and edit its «{fuente}» from {desde:.2} to {hasta:.2} s", nombre(*pista)),
        Accion::TonoAPista { pista, desde, hasta, semitonos } => crate::tr!("transponer {} {semitonos:+} semitonos de {desde:.2} a {hasta:.2} s en una pista nueva", "transpose {} {semitonos:+} semitones from {desde:.2} to {hasta:.2} s into a new track", nombre(*pista)),
        Accion::Reemplazar { pista, desde, hasta, descripcion, .. } => crate::tr!("rehacer {} de {desde:.2} a {hasta:.2} s: {descripcion}", "redo {} from {desde:.2} to {hasta:.2} s: {descripcion}", nombre(*pista)),
        Accion::Deshacer => crate::i18n::t("deshacer", "undo").into(),
        Accion::Pregunta(p) => p.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::experto::{Grupo, Origen, Sesion};

    fn sesion() -> Estado {
        let mut s = Sesion::default();
        let p = |id: u32, nombre: &str, de: Option<u32>, fuente: Option<&str>| Pista { color: None, id, nombre: nombre.into(), archivo: format!("{nombre}.wav"), origen: if de.is_some() { Origen::Separada } else { Origen::Generada }, de, fuente: fuente.map(String::from), ganancia_db: 0.0, pan: 0.0, silencio: false, solo: false, grupo: None, insertos: Default::default() };
        s.aplicar(Op::AgregarPista { pista: p(1, "Canción", None, None) }).unwrap();
        s.aplicar(Op::AgregarPista { pista: p(2, "Canción · Voz", Some(1), Some("vocals")) }).unwrap();
        s.aplicar(Op::AgregarPista { pista: p(3, "Canción · Batería", Some(1), Some("drums")) }).unwrap();
        s.aplicar(Op::AgregarPista { pista: p(4, "Canción · Bajo", Some(1), Some("bass")) }).unwrap();
        s.aplicar(Op::AgregarPista { pista: p(5, "Sintetizador", None, None) }).unwrap();
        s.aplicar(Op::CrearGrupo { grupo: Grupo { insertos: Default::default(), id: 9, nombre: "Ritmo".into(), ganancia_db: 0.0, pan: 0.0, silencio: false, solo: false } }).unwrap();
        s.estado()
    }

    fn region(pista: u32, desde: f64, hasta: f64, efecto: Efecto) -> Vec<Accion> {
        vec![Accion::Op(Op::Region { region: Region { apagada: false, id: 0, pista, desde, hasta, efecto } })]
    }

    /// 30 órdenes (español e inglés), escritas como llamadas del modelo guionado → la acción EXACTA.
    #[test]
    fn treinta_ordenes_dan_la_accion_exacta() {
        let e = sesion();
        let f = 0.03;
        let casos: Vec<(&str, Value, Vec<Accion>)> = vec![
            // «modifica en la pista 1 la batería que va del segundo 5 al 10 para que suene más fuerte»
            ("region_ganancia", json!({"pista": 1, "fuente": "drums", "desde": 5, "hasta": 10, "db": 3}), region(3, 5.0, 10.0, Efecto::Ganancia { db: 3.0, fundido: f })),
            ("region_ganancia", json!({"pista": "1", "fuente": "batería", "desde": "5", "hasta": "10", "db": 6}), region(3, 5.0, 10.0, Efecto::Ganancia { db: 6.0, fundido: f })),
            ("region_ganancia", json!({"pista": 3, "desde": 5, "hasta": 10, "db": 3}), region(3, 5.0, 10.0, Efecto::Ganancia { db: 3.0, fundido: f })),
            ("region_ganancia", json!({"pista": "batería", "desde": 0, "hasta": 4.5, "db": -3}), region(3, 0.0, 4.5, Efecto::Ganancia { db: -3.0, fundido: f })),
            ("region_ganancia", json!({"pista": "drums", "desde": 12, "hasta": 20, "db": 2}), region(3, 12.0, 20.0, Efecto::Ganancia { db: 2.0, fundido: f })),
            ("region_ganancia", json!({"pista": "voz", "desde": 30, "hasta": 45, "db": -6}), region(2, 30.0, 45.0, Efecto::Ganancia { db: -6.0, fundido: f })),
            ("region_ganancia", json!({"pista": "Sintetizador", "desde": 1, "hasta": 2, "db": 4}), region(5, 1.0, 2.0, Efecto::Ganancia { db: 4.0, fundido: f })),
            ("region_ganancia", json!({"pista": "sinte", "desde": 1, "hasta": 2, "db": 4}), region(5, 1.0, 2.0, Efecto::Ganancia { db: 4.0, fundido: f })),
            ("region_silencio", json!({"pista": 2, "desde": 8, "hasta": 9.5}), region(2, 8.0, 9.5, Efecto::Silencio { fundido: f })),
            ("region_silencio", json!({"pista": 1, "fuente": "vocals", "desde": 60, "hasta": 70}), region(2, 60.0, 70.0, Efecto::Silencio { fundido: f })),
            // «más agudo» de color
            ("region_eq", json!({"pista": 1, "fuente": "drums", "desde": 5, "hasta": 10, "filtro": "agudos", "db": 4}), region(3, 5.0, 10.0, Efecto::Eq { filtro: Filtro::Agudos, frecuencia: 4000.0, db: 4.0, q: 0.707, fundido: f })),
            ("region_eq", json!({"pista": 3, "desde": 5, "hasta": 10, "filtro": "high_shelf", "frecuencia": 6000, "db": 5}), region(3, 5.0, 10.0, Efecto::Eq { filtro: Filtro::Agudos, frecuencia: 6000.0, db: 5.0, q: 0.707, fundido: f })),
            ("region_eq", json!({"pista": "bajo", "desde": 0, "hasta": 30, "filtro": "graves", "db": 3}), region(4, 0.0, 30.0, Efecto::Eq { filtro: Filtro::Graves, frecuencia: 120.0, db: 3.0, q: 0.707, fundido: f })),
            ("region_eq", json!({"pista": "bass", "desde": 0, "hasta": 30, "filtro": "paso_alto", "frecuencia": 40, "db": 0}), region(4, 0.0, 30.0, Efecto::Eq { filtro: Filtro::PasoAlto, frecuencia: 40.0, db: 0.0, q: 0.707, fundido: f })),
            ("region_eq", json!({"pista": 2, "desde": 10, "hasta": 20, "filtro": "campana", "frecuencia": 3000, "db": -2.5}), region(2, 10.0, 20.0, Efecto::Eq { filtro: Filtro::Campana, frecuencia: 3000.0, db: -2.5, q: 0.707, fundido: f })),
            ("region_eq", json!({"pista": 5, "desde": 2, "hasta": 3, "filtro": "lowpass", "frecuencia": 2000, "db": 0}), region(5, 2.0, 3.0, Efecto::Eq { filtro: Filtro::PasoBajo, frecuencia: 2000.0, db: 0.0, q: 0.707, fundido: f })),
            // «más agudo» de tono
            ("region_tono", json!({"pista": 2, "desde": 5, "hasta": 10, "semitonos": 2}), vec![Accion::TonoAPista { pista: 2, desde: 5.0, hasta: 10.0, semitonos: 2.0 }]),
            ("region_tono", json!({"pista": "voice", "desde": 5, "hasta": 10, "semitonos": -12, "en_sitio": true}), region(2, 5.0, 10.0, Efecto::Tono { semitonos: -12.0, fundido: f })),
            // lo generativo
            ("region_reemplazar", json!({"pista": 1, "fuente": "drums", "desde": 5, "hasta": 10, "descripcion": "jazz brush drums"}), vec![Accion::Reemplazar { pista: 3, desde: 5.0, hasta: 10.0, descripcion: "jazz brush drums".into(), letra: String::new(), idioma: String::new() }]),
            // mezcla
            ("mezcla", json!({"pista": 2, "silencio": true}), vec![Accion::Op(Op::Mezcla { pista: 2, ganancia_db: None, pan: None, silencio: Some(true), solo: None })]),
            ("mezcla", json!({"pista": "bajo", "ganancia_db": -4}), vec![Accion::Op(Op::Mezcla { pista: 4, ganancia_db: Some(-4.0), pan: None, silencio: None, solo: None })]),
            ("mezcla", json!({"pista": 5, "pan": -0.7}), vec![Accion::Op(Op::Mezcla { pista: 5, ganancia_db: None, pan: Some(-0.7), silencio: None, solo: None })]),
            ("mezcla", json!({"pista": "Voz", "solo": true}), vec![Accion::Op(Op::Mezcla { pista: 2, ganancia_db: None, pan: None, silencio: None, solo: Some(true) })]),
            ("mezcla", json!({"pista": 1, "fuente": "bass", "ganancia_db": 2}), vec![Accion::Op(Op::Mezcla { pista: 4, ganancia_db: Some(2.0), pan: None, silencio: None, solo: None })]),
            // separar, tempo, efectos, deshacer, preguntar
            ("separar", json!({"pista": 5}), vec![Accion::Separar { pista: 5, modelo: "4".into() }]),
            ("separar", json!({"pista": "Sintetizador", "modelo": "6"}), vec![Accion::Separar { pista: 5, modelo: "6".into() }]),
            ("tempo", json!({"bpm": 92}), vec![Accion::Op(Op::Tempo { bpm: 92.0, compas: 4 })]),
            ("efectos", json!({"pista": 2, "reverb": {"tamano": 0.6, "amortiguacion": 0.5, "mezcla": 0.3}}), vec![Accion::Op(Op::Insertos { pista: 2, insertos: Insertos { reverb: Some(crate::experto::Reverb { tamano: 0.6, amortiguacion: 0.5, mezcla: 0.3 }), ..Default::default() } })]),
            ("deshacer", json!({}), vec![Accion::Deshacer]),
            ("preguntar", json!({"pregunta": "¿Más agudo de color o de tono?"}), vec![Accion::Pregunta("¿Más agudo de color o de tono?".into())]),
        ];
        assert_eq!(casos.len(), 30);
        assert_eq!(traducir(&e, "tempo", &json!({"bpm": 120, "compas": 4})).unwrap(), vec![], "el tempo que ya está no es un cambio");
        for (n, a, esperado) in casos {
            assert_eq!(traducir(&e, n, &a).unwrap_or_else(|x| panic!("{n} {a}: {x}")), esperado, "{n} {a}");
        }
        // agrupar: crea el grupo con un número libre y mete las pistas
        let g = traducir(&e, "agrupar", &json!({"pistas": ["batería", 4], "nombre": "Ritmo 2"})).unwrap();
        assert!(matches!(&g[..], [Accion::Op(Op::CrearGrupo { grupo }), Accion::Op(Op::Agrupar { pistas, grupo: Some(x) })] if grupo.nombre == "Ritmo 2" && *pistas == vec![3, 4] && *x == grupo.id));
    }

    /// Una parte que aún no es pista: se separa primero (guitarra y piano con el de 6), y lo imposible es error.
    #[test]
    fn lo_que_falta_se_separa_y_lo_imposible_se_dice() {
        let mut e = sesion();
        let a = traducir(&e, "region_ganancia", &json!({"pista": 1, "fuente": "guitarra", "desde": 5, "hasta": 10, "db": 3})).unwrap();
        assert_eq!(a, vec![Accion::SepararYRegion { pista: 1, fuente: "guitar".into(), desde: 5.0, hasta: 10.0, efecto: Efecto::Ganancia { db: 3.0, fundido: 0.03 } }]);
        assert_eq!(modelo_para("guitar"), "6");
        assert!(traducir(&e, "region_ganancia", &json!({"pista": 99, "desde": 5, "hasta": 10, "db": 3})).is_err());
        assert!(traducir(&e, "region_ganancia", &json!({"pista": 1, "desde": 10, "hasta": 5, "db": 3})).is_err());
        assert!(traducir(&e, "volar", &json!({})).is_err());
        // con una sola canción sin separar, «la batería» a secas apunta a ella
        e.pistas.retain(|p| p.id == 1);
        let a = traducir(&e, "region_silencio", &json!({"pista": "batería", "desde": 1, "hasta": 2})).unwrap();
        assert!(matches!(&a[0], Accion::SepararYRegion { pista: 1, fuente, .. } if fuente == "drums"));
    }

    #[test]
    fn el_resumen_numera_las_pistas() {
        let e = sesion();
        let r = resumen(&e, &|p| (p.id == 1).then_some(184.2));
        assert!(r.contains("1 · «Canción» (184.2 s)") && r.contains("3 · «Canción · Batería» (part=drums, split from track 1)"), "{r}");
        assert_eq!(herramientas().as_array().unwrap().len(), 21);
    }

    /// X2-6 · Las órdenes del estudio nuevo dan la acción EXACTA: partir, quitar un tramo, fundidos,
    /// marcadores, maestro, masterizar, tonalidad, generar una pista, extender y los efectos nuevos.
    #[test]
    fn las_ordenes_del_estudio_nuevo() {
        use crate::experto::{Limitador, Marcador};
        let e = sesion();
        let uno = |n: &str, a: Value| traducir(&e, n, &a).unwrap();
        assert_eq!(uno("partir", json!({"pista": "batería", "segundo": 12.5})), vec![Accion::Op(Op::Partir { pista: 3, segundo: 12.5, clip: None })]);
        assert_eq!(uno("quitar_tramo", json!({"pista": 2, "desde": 5, "hasta": 10})), vec![Accion::Op(Op::QuitarTramo { pista: 2, desde: 5.0, hasta: 10.0, cerrar: false })]);
        assert_eq!(uno("quitar_tramo", json!({"pista": "bass", "desde": 0, "hasta": 4, "cerrar": true})), vec![Accion::Op(Op::QuitarTramo { pista: 4, desde: 0.0, hasta: 4.0, cerrar: true })]);
        assert_eq!(uno("marcador", json!({"segundo": 32, "nombre": "Estribillo"})), vec![Accion::Op(Op::Marcador { marcador: Marcador { segundo: 32.0, nombre: "Estribillo".into() } })]);
        // un fundido de salida: de 0 a −60 dB entre los dos segundos
        assert_eq!(
            uno("fundido", json!({"pista": 5, "desde": 170, "hasta": 180})),
            vec![Accion::Op(Op::Automatizar { pista: 5, parametro: Parametro::Volumen, puntos: vec![Punto { t: 170.0, v: 0.0 }, Punto { t: 180.0, v: -60.0 }] })]
        );
        assert_eq!(
            uno("fundido", json!({"pista": 5, "desde": 0, "hasta": 4, "de_db": -60, "a_db": 0})),
            vec![Accion::Op(Op::Automatizar { pista: 5, parametro: Parametro::Volumen, puntos: vec![Punto { t: 0.0, v: -60.0 }, Punto { t: 4.0, v: 0.0 }] })]
        );
        // un segundo fundido conserva lo que la curva tenía fuera de su tramo
        let mut s = Sesion::default();
        for op in [
            Op::AgregarPista { pista: Pista { color: None, id: 5, nombre: "Sintetizador".into(), archivo: "s.wav".into(), origen: Origen::Importada, de: None, fuente: None, ganancia_db: 0.0, pan: 0.0, silencio: false, solo: false, grupo: None, insertos: Default::default() } },
            Op::Automatizar { pista: 5, parametro: Parametro::Volumen, puntos: vec![Punto { t: 0.0, v: -60.0 }, Punto { t: 4.0, v: 0.0 }] },
        ] {
            s.aplicar(op).unwrap();
        }
        let a = traducir(&s.estado(), "fundido", &json!({"pista": 5, "desde": 170, "hasta": 180})).unwrap();
        assert!(matches!(&a[..], [Accion::Op(Op::Automatizar { puntos, .. })] if puntos.iter().map(|p| p.t).collect::<Vec<_>>() == vec![0.0, 4.0, 170.0, 180.0]));
        assert_eq!(uno("maestro", json!({"ganancia_db": -2})), vec![Accion::Op(Op::Maestro { ganancia_db: Some(-2.0), insertos: None })]);
        let lim = Limitador { techo_db: -1.0, empuje_db: 4.0, liberacion_ms: 120.0 };
        assert_eq!(
            uno("maestro", json!({"limitador": {"techo_db": -1, "empuje_db": 4, "liberacion_ms": 120}})),
            vec![Accion::Op(Op::Maestro { ganancia_db: None, insertos: Some(Insertos { limitador: Some(lim), ..Default::default() }) })]
        );
        assert_eq!(uno("tonalidad", json!({"tonalidad": "A minor"})), vec![Accion::Op(Op::Tonalidad { tonalidad: "A minor".into() })]);
        assert_eq!(uno("masterizar", json!({})), vec![Accion::Masterizar { lufs: -14.0 }]);
        assert_eq!(uno("masterizar", json!({"lufs": -9})), vec![Accion::Masterizar { lufs: -9.0 }]);
        assert_eq!(uno("generar_pista", json!({"descripcion": "warm rhodes piano", "segundos": 30})), vec![Accion::Generar { descripcion: "warm rhodes piano".into(), segundos: 30.0, letra: String::new() }]);
        assert!(traducir(&e, "generar_pista", &json!({"descripcion": "  "})).is_err());
        assert_eq!(uno("extender", json!({"segundos": 20, "descripcion": "big final chorus"})), vec![Accion::Extender { pista: None, segundos: 20.0, descripcion: "big final chorus".into() }]);
        assert_eq!(uno("extender", json!({"pista": "voz", "segundos": 8, "descripcion": "ad libs"})), vec![Accion::Extender { pista: Some(2), segundos: 8.0, descripcion: "ad libs".into() }]);
        // los efectos nuevos: se ponen, se quitan con null y lo demás no cambia; lo que no vale lo rechaza el núcleo
        let a = uno("efectos", json!({"pista": 2, "deesser": {"frecuencia": 6000, "umbral_db": -30, "reduccion_db": 6}, "ancho": 1.3, "eq": [{"filtro": "paso_alto", "frecuencia": 90}]}));
        let Accion::Op(Op::Insertos { pista: 2, insertos }) = &a[0] else { panic!("{a:?}") };
        assert!(insertos.deesser.is_some() && insertos.ancho == Some(1.3) && insertos.eq.len() == 1 && insertos.eq[0].q == 0.707 && insertos.compresor.is_none());
        let mut s2 = Sesion::default();
        s2.aplicar(Op::AgregarPista { pista: Pista { color: None, id: 2, nombre: "Voz".into(), archivo: "v.wav".into(), origen: Origen::Importada, de: None, fuente: None, ganancia_db: 0.0, pan: 0.0, silencio: false, solo: false, grupo: None, insertos: insertos.clone() } }).unwrap();
        let quita = traducir(&s2.estado(), "efectos", &json!({"pista": 2, "deesser": null, "reverb": {"tamano": 0.5, "amortiguacion": 0.5, "mezcla": 0.2}})).unwrap();
        let Accion::Op(Op::Insertos { insertos: i2, .. }) = &quita[0] else { panic!() };
        assert!(i2.deesser.is_none() && i2.reverb.is_some() && i2.ancho == Some(1.3) && i2.eq.len() == 1, "{i2:?}");
        assert!(s2.aplicar(Op::Insertos { pista: 2, insertos: Insertos { ancho: Some(9.0), ..Default::default() } }).is_err());
        // las frases de las tarjetas del chat ya no son JSON
        assert!(!describir(&e, &a[0]).contains('{'), "{}", describir(&e, &a[0]));
        assert!(describir(&e, &Accion::Masterizar { lufs: -14.0 }).contains("-14"));
    }
}
