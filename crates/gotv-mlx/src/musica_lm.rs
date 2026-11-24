//! M7 · el LM planificador de ACE-Step 1.5 (`acestep-5Hz-lm-1.7B`, un Qwen3 de 1,7B), como el
//! `LLMHandler` oficial con `thinking=True`:
//! - **fase 2 (códigos)**: con la CoT de metadatos en el prompt genera `<|audio_code_N|>` a 5 Hz con
//!   CFG (escala 2, incondicional «NO USER INPUT» con `<think>` vacío) SÓLO sobre los tokens de
//!   código; el EOS se bloquea hasta ⌊segundos·5⌋ códigos y ahí se fuerza; top-p 0,9 sobre los logits
//!   del CFG y luego temperatura 0,85 (el orden del oficial).
//!
//! Los códigos guían al DiT: FSQ → destokenizador → pistas a 25 Hz (`musica::AceStep::pistas`).

use crate::modelo::{Modelo, M};
use crate::plantilla::Plantilla;
use anyhow::{anyhow, bail, Result};
use gotv_engine::{anyhow_tr, bail_tr};
use mlx_rs::Dtype;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

/// `DEFAULT_LM_INSTRUCTION` del oficial.
const INSTRUCCION_LM: &str = "Generate audio semantic tokens based on the given conditions:";
const NEGATIVO: &str = "NO USER INPUT";
/// el último código válido (`MAX_AUDIO_CODE`, libro de 64 000)
pub const MAX_CODIGO: u32 = 63_999;

/// Los parámetros de muestreo del oficial (`GenerationParams`).
#[derive(Clone, Debug)]
pub struct Muestreo {
    pub temperatura: f32,
    pub cfg: f32,
    pub top_p: f32,
    /// Ola PRO · `lm_top_k` (0 = apagado, como el oficial)
    pub top_k: usize,
    /// Ola PRO · `lm_negative_prompt`: lo que NO debe sonar. El oficial lo pone TAL CUAL como el
    /// mensaje del usuario del prompt incondicional del CFG (en lugar de «NO USER INPUT")
    pub negativo: String,
}

impl Default for Muestreo {
    fn default() -> Self {
        Muestreo { temperatura: 0.85, cfg: 2.0, top_p: 0.9, top_k: 0, negativo: NEGATIVO.into() }
    }
}

/// Deja los `k` candidatos de logit más alto (0 = todos), antes del top-p (el orden de transformers).
pub(crate) fn top_k(c: &mut Vec<(u32, f32)>, k: usize) {
    if k == 0 || k >= c.len() {
        return;
    }
    c.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    c.truncate(k);
}

/// Los metadatos de la CoT (`_format_metadata_as_cot`: YAML con claves ordenadas).
#[derive(Clone, Debug, Default)]
pub struct Metas {
    pub bpm: Option<u32>,
    pub caption: Option<String>,
    pub duracion: Option<u32>,
    pub tonalidad: Option<String>,
    pub idioma: Option<String>,
    pub compas: Option<String>,
}

/// ¿PyYAML puede escribir este texto SIN comillas como valor de un mapeo en bloque?
/// (`analyze_scalar` + `choose_scalar_style` con `allow_unicode`; además, un texto que el
/// resolvedor leería como bool, null o número no puede ir plano.) Los caracteres de control y los
/// tabuladores irían entre comillas dobles: el LM no los escribe (la descripción es una línea).
fn yaml_plano(s: &str) -> bool {
    let c: Vec<char> = s.chars().collect();
    let n = c.len();
    if n == 0 || s.starts_with("---") || s.starts_with("...") || c[0] == ' ' || c[n - 1] == ' ' {
        return false;
    }
    let blanco = |i: usize| i >= n || " \t\r\n\u{85}\u{2028}\u{2029}\0".contains(c[i]);
    for i in 0..n {
        let ch = c[i];
        if i == 0 {
            if "#,[]{}&*!|>'\"%@`".contains(ch) || ("?:".contains(ch) && blanco(1)) || (ch == '-' && blanco(1)) {
                return false;
            }
        } else if (ch == ':' && blanco(i + 1)) || (ch == '#' && " \t".contains(c[i - 1])) {
            return false;
        }
        if ch == '\n' || ch == '\t' || (ch as u32) < 0x20 {
            return false;
        }
    }
    const IMPLICITOS: &[&str] = &[
        "yes", "Yes", "YES", "no", "No", "NO", "true", "True", "TRUE", "false", "False", "FALSE", "on", "On", "ON", "off", "Off", "OFF", "~", "null", "Null", "NULL", "<<", "=",
    ];
    let numero = {
        let t = s.trim_start_matches(['+', '-']).replace('_', "");
        t.parse::<f64>().is_ok() || t.starts_with("0x") || t.starts_with("0b") || t.starts_with("0o") || matches!(t.to_lowercase().as_str(), ".inf" | ".nan")
    };
    !(IMPLICITOS.contains(&s) || numero)
}

/// Un valor de texto como `yaml.dump` (ancho 80, sangría 2): plano o entre comillas simples, con la
/// línea partida en un espacio SUELTO cuando la columna ya pasó de 80 (`write_plain`,
/// `write_single_quoted`; la continuación va con dos espacios). `col` = columna tras «clave:».
fn yaml_valor(s: &str, col: usize) -> String {
    let c: Vec<char> = s.chars().collect();
    let n = c.len();
    let mut out = String::new();
    let plano = yaml_plano(s);
    let mut col = col + 1;
    out.push(' ');
    if !plano {
        out.push('\'');
        col += 1;
    }
    let (mut espacios, mut ini, mut fin) = (false, 0usize, 0usize);
    let tramo = |out: &mut String, col: &mut usize, a: usize, b: usize| {
        out.extend(&c[a..b]);
        *col += b - a;
    };
    while fin <= n {
        let ch = c.get(fin).copied();
        if espacios {
            if ch != Some(' ') {
                let partir = ini + 1 == fin && col > 80 && (plano || (ini != 0 && fin != n));
                if partir {
                    out.push_str("\n  ");
                    col = 2;
                } else {
                    tramo(&mut out, &mut col, ini, fin);
                }
                ini = fin;
            }
        } else if (ch.is_none() || ch == Some(' ') || (!plano && ch == Some('\''))) && ini < fin {
            tramo(&mut out, &mut col, ini, fin);
            ini = fin;
        }
        if !plano && ch == Some('\'') {
            out.push_str("''");
            col += 2;
            ini = fin + 1;
        }
        if let Some(ch) = ch {
            espacios = ch == ' ';
        }
        fin += 1;
    }
    if !plano {
        out.push('\'');
    }
    out
}

/// La CoT `<think>\n{yaml}\n</think>` (`_format_metadata_as_cot`): claves en orden alfabético
/// (bpm, caption, duration, keyscale, language, timesignature); el compás «N/4» se escribe «N» y
/// los textos de sólo dígitos van como enteros.
pub fn cot(m: &Metas) -> String {
    let mut l: Vec<String> = Vec::new();
    let mut texto = |clave: &str, v: &str| {
        let v = if clave == "timesignature" { v.strip_suffix("/4").unwrap_or(v) } else { v };
        if !v.is_empty() && v.chars().all(|c| c.is_ascii_digit()) {
            l.push(format!("{clave}: {}", v.parse::<u128>().map(|x| x.to_string()).unwrap_or_else(|_| v.to_string())));
        } else {
            l.push(format!("{clave}:{}", yaml_valor(v, clave.chars().count() + 1)));
        }
    };
    if let Some(b) = m.bpm {
        texto("bpm", &b.to_string());
    }
    if let Some(c) = &m.caption {
        texto("caption", c);
    }
    if let Some(d) = m.duracion {
        texto("duration", &d.to_string());
    }
    if let Some(k) = &m.tonalidad {
        texto("keyscale", k);
    }
    if let Some(i) = &m.idioma {
        texto("language", i);
    }
    if let Some(t) = &m.compas {
        texto("timesignature", t);
    }
    format!("<think>\n{}\n</think>", l.join("\n"))
}

/// `parse_lm_output` sobre la salida de la fase 1: cada línea sin sangría con «:» abre un campo;
/// las sangradas lo continúan; las que empiezan por «<» se saltan. La descripción se aplana
/// (`postprocess_caption`: líneas recortadas unidas con un espacio).
pub fn parsear(texto: &str) -> Metas {
    let razon = match (texto.find("<think>"), texto.find("</think>")) {
        (Some(a), Some(b)) if b > a => texto[a + 7..b].trim().to_string(),
        _ => texto.split("<|audio_code_").next().unwrap_or("").trim().to_string(),
    };
    let mut m = Metas::default();
    let mut guardar = |clave: &str, lineas: &[String]| {
        if lineas.is_empty() {
            return;
        }
        let v = lineas.join("\n");
        let t = v.trim().to_string();
        match clave {
            "bpm" => m.bpm = t.parse().ok(),
            "caption" => m.caption = Some(v.split('\n').map(str::trim).filter(|x| !x.is_empty()).collect::<Vec<_>>().join(" ")),
            "duration" => m.duracion = t.parse().ok(),
            "keyscale" => m.tonalidad = Some(t),
            "language" => m.idioma = Some(t),
            "timesignature" => m.compas = Some(t),
            _ => {}
        }
    };
    let (mut clave, mut lineas): (Option<String>, Vec<String>) = (None, Vec::new());
    for linea in razon.split('\n') {
        if linea.trim().starts_with('<') {
            continue;
        }
        if !linea.is_empty() && !linea.starts_with(char::is_whitespace) && linea.contains(':') {
            if let Some(k) = clave.take() {
                guardar(&k, &lineas);
            }
            lineas.clear();
            let (k, v) = linea.split_once(':').unwrap_or((linea, ""));
            clave = Some(k.trim().to_lowercase());
            if !v.trim().is_empty() {
                lineas.push(v.to_string());
            }
        } else if (linea.starts_with(' ') || linea.starts_with('\t')) && clave.is_some() {
            lineas.push(linea.to_string());
        }
    }
    if let Some(k) = clave.take() {
        guardar(&k, &lineas);
    }
    m
}

// ───────────────────────── fase 1: la máquina de estados ─────────────────────────

/// `VALID_LANGUAGES` del oficial.
const IDIOMAS: &[&str] = &[
    "ar", "az", "bg", "bn", "ca", "cs", "da", "de", "el", "en", "es", "fa", "fi", "fr", "he", "hi", "hr", "ht", "hu", "id", "is", "it", "ja", "ko", "la", "lt", "ms", "ne", "nl", "no", "pa", "pl", // juez-idioma: datos (códigos de idioma: «el», «la»)
    "pt", "ro", "ru", "sa", "sk", "sr", "sv", "sw", "ta", "te", "th", "tl", "tr", "uk", "ur", "vi", "yue", "zh", "unknown",
];

/// Árbol de prefijos POR TOKENS (`_build_*_prefix_tree`): cada valor se tokeniza tras su contexto
/// («bpm: 120» → lo que sigue a los tokens de «bpm:»), y al final de cada valor cabe el salto.
struct Arbol(std::collections::HashMap<Vec<u32>, std::collections::BTreeSet<u32>>);

impl Arbol {
    fn de(tk: &tokenizers::Tokenizer, valores: &[String], campo: &str, nl: u32, nota: bool) -> Result<Arbol> {
        let enc = |s: &str| -> Result<Vec<u32>> { Ok(tk.encode(s, false).map_err(|e| anyhow_tr!("tokenizar: {e}", "tokenizing: {e}"))?.get_ids().to_vec()) };
        let ctx = enc(&format!("{campo}:"))?;
        let mut m: std::collections::HashMap<Vec<u32>, std::collections::BTreeSet<u32>> = Default::default();
        for v in valores {
            let todo = enc(&format!("{campo}: {v}"))?;
            if !todo.starts_with(&ctx) {
                continue;
            }
            let ids = &todo[ctx.len()..];
            if ids.is_empty() {
                continue;
            }
            if nota {
                // la tonalidad: el primer token debe empezar por una nota
                let p = tk.decode(&ids[..1], false).unwrap_or_default();
                if !p.trim_start().chars().next().is_some_and(|c| "ABCDEFG".contains(c.to_ascii_uppercase())) {
                    continue;
                }
            }
            for i in 0..=ids.len() {
                let e = m.entry(ids[..i].to_vec()).or_default();
                e.insert(if i < ids.len() { ids[i] } else { nl });
            }
        }
        Ok(Arbol(m))
    }
    fn hijos(&self, pre: &[u32]) -> Vec<u32> {
        self.0.get(pre).map(|s| s.iter().copied().collect()).unwrap_or_default()
    }
    fn completo(&self, pre: &[u32], nl: u32) -> bool {
        self.0.get(pre).is_some_and(|s| s.contains(&nl))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Fsm {
    Think,
    NlThink,
    BpmN,
    BpmV,
    CapN,
    CapV,
    DurN,
    DurV,
    KeyN,
    KeyV,
    LangN,
    LangV,
    TsN,
    TsV,
    FinThink,
    Hecho,
}

impl Fsm {
    /// el texto fijo de los estados de nombre
    fn fijo(self) -> Option<&'static str> {
        Some(match self {
            Fsm::Think => "<think>",
            Fsm::NlThink => "\n",
            Fsm::BpmN => "bpm:",
            Fsm::CapN => "caption:",
            Fsm::DurN => "duration:",
            Fsm::KeyN => "keyscale:",
            Fsm::LangN => "language:",
            Fsm::TsN => "timesignature:",
            Fsm::FinThink => "</think>",
            _ => return None,
        })
    }
    /// `next_state` con `skip_genres` (la descripción y el idioma sí se generan)
    fn siguiente(self) -> Fsm {
        use Fsm::*;
        match self {
            Think => NlThink,
            NlThink => BpmN,
            BpmN => BpmV,
            BpmV => CapN,
            CapN => CapV,
            CapV => DurN,
            DurN => DurV,
            DurV => KeyN,
            KeyN => KeyV,
            KeyV => LangN,
            LangN => LangV,
            LangV => TsN,
            TsN => TsV,
            TsV => FinThink,
            FinThink | Hecho => Hecho,
        }
    }
}

/// Qué tokens deja pasar el paso.
#[derive(Debug, PartialEq)]
pub(crate) enum Filtro {
    /// sólo estos (los demás a −∞)
    Lista(Vec<u32>),
    /// texto libre de la descripción: todo menos el acento grave y los códigos de audio
    Texto,
    /// sin tocar
    Libre,
}

/// `MetadataConstrainedLogitsProcessor` en la fase «cot» con `stop_at_reasoning`.
pub(crate) struct Cot<'a> {
    lm: &'a Lm,
    dados: Metas,
    estado: Fsm,
    pos: usize,
    acc: Vec<u32>,
    cola: Vec<u32>,
    campo_cola: Fsm,
    tras_nl: bool,
    n_cap: usize,
    cierre: bool,
    pendiente: String,
}

impl<'a> Cot<'a> {
    pub(crate) fn new(lm: &'a Lm, dados: &Metas) -> Cot<'a> {
        Cot { lm, dados: dados.clone(), estado: Fsm::Think, pos: 0, acc: Vec::new(), cola: Vec::new(), campo_cola: Fsm::Hecho, tras_nl: false, n_cap: 0, cierre: false, pendiente: String::new() }
    }

    fn pasar(&mut self, e: Fsm) {
        self.estado = e;
        self.pos = 0;
        self.acc.clear();
        self.tras_nl = false;
        self.n_cap = 0;
        self.cierre = false;
        self.pendiente.clear();
    }

    /// El valor que dio el usuario para este campo, como lo encola el oficial: « {valor}\n».
    fn dado(&self) -> Option<String> {
        let d = &self.dados;
        match self.estado {
            Fsm::BpmV => d.bpm.map(|b| b.to_string()),
            Fsm::CapV => d.caption.clone(),
            Fsm::DurV => d.duracion.map(|b| b.to_string()),
            Fsm::KeyV => d.tonalidad.clone(),
            Fsm::LangV => d.idioma.clone(),
            Fsm::TsV => d.compas.clone(),
            _ => None,
        }
    }

    pub(crate) fn procesar(&mut self, logits: &[f32]) -> Result<Filtro> {
        let lm = self.lm;
        if let Some(&t) = self.cola.first() {
            return Ok(Filtro::Lista(vec![t]));
        }
        if let Some(f) = self.estado.fijo() {
            if self.estado == Fsm::FinThink {
                // `stop_at_reasoning`: quedan ≤ 10 caracteres de «</think>» → EOS
                return Ok(Filtro::Lista(vec![lm.eos]));
            }
            let resto: Vec<char> = f.chars().skip(self.pos).collect();
            for fin in (1..=resto.len()).rev() {
                let p: String = resto[..fin].iter().collect();
                let ids = lm.tk.encode(p.as_str(), false).map_err(|e| anyhow_tr!("tokenizar: {e}", "tokenizing: {e}"))?;
                if ids.get_ids().len() == 1 {
                    return Ok(Filtro::Lista(vec![ids.get_ids()[0]]));
                }
            }
            bail!("«{f}» no se puede escribir token a token");
        }
        // un campo que dio el usuario se escribe tal cual (sólo al empezar el valor)
        if self.acc.is_empty() && self.n_cap == 0 {
            if let Some(v) = self.dado() {
                self.cola = lm.tk.encode(format!(" {v}\n").as_str(), false).map_err(|e| anyhow_tr!("tokenizar: {e}", "tokenizing: {e}"))?.get_ids().to_vec();
                self.campo_cola = self.estado;
                if let Some(&t) = self.cola.first() {
                    return Ok(Filtro::Lista(vec![t]));
                }
            }
        }
        let nl = lm.nl;
        Ok(match self.estado {
            // bpm: hijos + salto si ya es un valor completo (el árbol ya lo trae)
            Fsm::BpmV => Filtro::Lista(lm.arbol_bpm.hijos(&self.acc)),
            Fsm::DurV => Filtro::Lista(lm.arbol_dur.hijos(&self.acc)),
            Fsm::CapV => {
                if self.tras_nl {
                    self.tras_nl = false;
                    let top = argmax(logits);
                    let s = lm.tk.decode(&[top], false).unwrap_or_default();
                    if s.chars().next().is_some_and(|c| c != ' ' && c != '\t') {
                        self.cierre = true;
                        self.pendiente.clear();
                        return Ok(Filtro::Libre);
                    }
                }
                if self.cierre {
                    Filtro::Libre
                } else if self.n_cap >= 512 {
                    Filtro::Lista(vec![nl])
                } else {
                    Filtro::Texto
                }
            }
            // tonalidad y compás: completo → sólo el salto; si no, los hijos
            Fsm::KeyV => {
                if lm.arbol_tono.completo(&self.acc, nl) {
                    Filtro::Lista(vec![nl])
                } else {
                    let h = lm.arbol_tono.hijos(&self.acc);
                    Filtro::Lista(if h.is_empty() { vec![nl] } else { h })
                }
            }
            Fsm::TsV => {
                if lm.arbol_compas.completo(&self.acc, nl) {
                    Filtro::Lista(vec![nl])
                } else {
                    Filtro::Lista(lm.arbol_compas.hijos(&self.acc))
                }
            }
            // idioma: el PRIMER token es el mejor de los candidatos (voraz); luego, el árbol
            Fsm::LangV => {
                if self.acc.is_empty() {
                    let c = lm.arbol_idioma.hijos(&[]);
                    match c.iter().copied().max_by(|a, b| logits[*a as usize].partial_cmp(&logits[*b as usize]).unwrap_or(std::cmp::Ordering::Equal)) {
                        Some(t) => Filtro::Lista(vec![t]),
                        None => Filtro::Lista(vec![nl]),
                    }
                } else {
                    if lm.arbol_idioma.completo(&self.acc, nl) {
                        Filtro::Lista(vec![nl])
                    } else {
                        let h = lm.arbol_idioma.hijos(&self.acc);
                        Filtro::Lista(if h.is_empty() { vec![nl] } else { h })
                    }
                }
            }
            _ => Filtro::Libre,
        })
    }

    /// `update_state` con el token elegido.
    pub(crate) fn actualizar(&mut self, t: u32) {
        let lm = self.lm;
        if self.estado == Fsm::Hecho {
            return;
        }
        if !self.cola.is_empty() {
            self.cola.remove(0);
            if self.cola.is_empty() {
                let s = self.campo_cola.siguiente();
                self.pasar(s);
            }
            return;
        }
        let s = lm.tk.decode(&[t], false).unwrap_or_default();
        if let Some(f) = self.estado.fijo() {
            self.pos += s.chars().count();
            if self.pos >= f.chars().count() {
                let s = self.estado.siguiente();
                self.pasar(s);
            }
            return;
        }
        match self.estado {
            Fsm::BpmV | Fsm::DurV | Fsm::TsV | Fsm::KeyV | Fsm::LangV => {
                if t == lm.nl {
                    let s = self.estado.siguiente();
                    self.pasar(s);
                } else {
                    self.acc.push(t);
                }
            }
            Fsm::CapV => {
                self.n_cap += 1;
                self.tras_nl = s.contains('\n');
                if self.cierre {
                    self.pendiente.push_str(&s);
                    if s.contains(':') {
                        let campo = self.pendiente.trim().trim_end_matches(':').trim().to_lowercase();
                        let destino = match campo.as_str() {
                            "duration" => Some(Fsm::DurV),
                            "keyscale" => Some(Fsm::KeyV),
                            "language" => Some(Fsm::LangV),
                            "timesignature" => Some(Fsm::TsV),
                            // «genres» lleva en el oficial a un estado sin salida cuando se
                            // saltan los géneros; el LM escribe «duration» tras la descripción
                            // (orden alfabético del entrenamiento): aquí sigue como un campo
                            // desconocido
                            _ => None,
                        };
                        match destino {
                            Some(d) => self.pasar(d),
                            None => self.pasar(Fsm::DurN),
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

fn argmax(l: &[f32]) -> u32 {
    let mut m = 0usize;
    for (i, x) in l.iter().enumerate() {
        if *x > l[m] {
            m = i;
        }
    }
    m as u32
}


/// xorshift64* — el azar del muestreo, reproducible con la semilla (el del oficial es el de torch).
pub(crate) struct Azar(u64);

impl Azar {
    pub(crate) fn new(semilla: u64) -> Azar {
        Azar(semilla.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }
    /// uniforme en [0, 1)
    pub(crate) fn uniforme(&mut self) -> f64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        (self.0.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 11) as f64 / (1u64 << 53) as f64
    }
}

/// Top-p del oficial sobre los candidatos (índice, logit): ordena de mayor a menor, softmax en
/// f32, suma acumulada; se quita lo que pasa de `p` CORRIDO uno (el primero siempre queda).
pub(crate) fn top_p(c: &mut Vec<(u32, f32)>, p: f32) {
    if !(0.0..1.0).contains(&p) || p <= 0.0 {
        return;
    }
    c.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    let m = c[0].1;
    let total: f32 = c.iter().map(|x| (x.1 - m).exp()).sum();
    let mut acum = 0f32;
    let mut corte = c.len();
    for (i, x) in c.iter().enumerate() {
        // se quita i si la suma ANTERIOR (sin él) ya pasaba p
        if i > 0 && acum > p {
            corte = i;
            break;
        }
        acum += (x.1 - m).exp() / total;
    }
    c.truncate(corte);
}

/// softmax(l/T) y una muestra.
pub(crate) fn muestrear(c: &[(u32, f32)], t: f32, azar: &mut Azar) -> u32 {
    if t <= 0.0 {
        return c.iter().max_by(|a, b| a.1.partial_cmp(&b.1).unwrap()).map(|x| x.0).unwrap_or(0);
    }
    let m = c.iter().map(|x| x.1).fold(f32::NEG_INFINITY, f32::max);
    let pesos: Vec<f64> = c.iter().map(|x| (((x.1 - m) / t) as f64).exp()).collect();
    let total: f64 = pesos.iter().sum();
    let mut u = azar.uniforme() * total;
    for (x, w) in c.iter().zip(&pesos) {
        if u < *w {
            return x.0;
        }
        u -= w;
    }
    c.last().map(|x| x.0).unwrap_or(0)
}

/// El LM con su tokenizador, su plantilla y el mapa de tokens de código.
pub struct Lm {
    modelo: Modelo,
    tk: tokenizers::Tokenizer,
    plantilla: Plantilla,
    /// id del token `<|audio_code_0|>`; los códigos son contiguos (código N = base + N)
    base: u32,
    eos: u32,
    /// `<|endoftext|>`: también corta (`_check_eos_token`)
    pad: u32,
    /// el salto de línea (cierra cada valor)
    nl: u32,
    acento: Option<u32>,
    arbol_bpm: Arbol,
    arbol_dur: Arbol,
    arbol_tono: Arbol,
    arbol_compas: Arbol,
    arbol_idioma: Arbol,
}

impl Lm {
    pub fn cargar(dir: &Path) -> Result<Lm> {
        let tk = tokenizers::Tokenizer::from_file(dir.join("tokenizer.json")).map_err(|e| anyhow_tr!("tokenizer del LM: {e}", "LM tokenizer: {e}"))?;
        let base = tk.token_to_id("<|audio_code_0|>").ok_or_else(|| anyhow_tr!("el LM no trae <|audio_code_0|>", "the LM has no <|audio_code_0|>"))?;
        // contiguos: se comprueba en los extremos y en el medio
        for n in [1, 31_999, MAX_CODIGO] {
            if tk.token_to_id(&format!("<|audio_code_{n}|>")) != Some(base + n) {
                bail_tr!("los tokens de código del LM no son contiguos (<|audio_code_{n}|>)", "the LM code tokens are not contiguous (<|audio_code_{n}|>)");
            }
        }
        let eos = tk.token_to_id("<|im_end|>").ok_or_else(|| anyhow_tr!("el LM no trae <|im_end|>", "the LM has no <|im_end|>"))?;
        let pad = tk.token_to_id("<|endoftext|>").ok_or_else(|| anyhow_tr!("el LM no trae <|endoftext|>", "the LM has no <|endoftext|>"))?;
        let ultimo = |s: &str| -> Result<u32> { tk.encode(s, false).map_err(|e| anyhow_tr!("tokenizar: {e}", "tokenizing: {e}"))?.get_ids().last().copied().ok_or_else(|| anyhow_tr!("«{s}» sin tokens", "«{s}» without tokens")) };
        let nl = ultimo("\n")?;
        let acento = ultimo("`").ok();
        let rango = |a: u32, b: u32| (a..=b).map(|v| v.to_string()).collect::<Vec<_>>();
        let mut tonos = Vec::new();
        for nota in ["A", "B", "C", "D", "E", "F", "G"] {
            for alt in ["", "#", "b", "\u{266f}", "\u{266d}"] {
                for modo in ["major", "minor"] {
                    tonos.push(format!("{nota}{alt} {modo}"));
                }
            }
        }
        let idiomas: Vec<String> = IDIOMAS.iter().map(|s| s.to_string()).collect();
        Ok(Lm {
            arbol_bpm: Arbol::de(&tk, &rango(30, 300), "bpm", nl, false)?,
            arbol_dur: Arbol::de(&tk, &rango(10, 600), "duration", nl, false)?,
            arbol_tono: Arbol::de(&tk, &tonos, "keyscale", nl, true)?,
            arbol_compas: Arbol::de(&tk, &["2", "3", "4", "6"].map(String::from), "timesignature", nl, false)?,
            arbol_idioma: Arbol::de(&tk, &idiomas, "language", nl, false)?,
            modelo: Modelo::cargar_f32(dir)?,
            plantilla: Plantilla::cargar(dir)?,
            tk,
            base,
            eos,
            pad,
            nl,
            acento,
        })
    }

    /// Los dos prompts de la fase 2 (`build_formatted_prompt_with_cot`): condicional con la CoT,
    /// incondicional con «NO USER INPUT» y `<think>` vacío.
    pub fn prompts(&self, caption: &str, letra: &str, cot: &str) -> Result<(String, String)> {
        self.prompts_con(caption, letra, cot, NEGATIVO)
    }

    /// Igual, con el negativo de la persona (`_has_meaningful_negative_prompt`: vacío = el de siempre).
    pub fn prompts_con(&self, caption: &str, letra: &str, cot: &str, negativo: &str) -> Result<(String, String)> {
        let negativo = Some(negativo.trim()).filter(|n| !n.is_empty()).unwrap_or(NEGATIVO);
        let sistema = format!("# Instruction\n{INSTRUCCION_LM}\n\n");
        let c = self.plantilla.render_simple(&[("system", &sistema), ("user", &format!("# Caption\n{caption}\n\n# Lyric\n{letra}\n"))])? + cot + "\n\n";
        let u = self.plantilla.render_simple(&[("system", &sistema), ("user", negativo)])? + "<think>\n\n</think>" + "\n\n";
        Ok((c, u))
    }

    pub fn tokens(&self, texto: &str) -> Result<Vec<u32>> {
        Ok(self.tk.encode(texto, true).map_err(|e| anyhow_tr!("tokenizar: {e}", "tokenizing: {e}"))?.get_ids().to_vec())
    }

    fn logits(&self, ids: &[u32], c: &mut crate::modelo::Cache) -> Result<Vec<f32>> {
        let l = self.modelo.adelante(ids, c)?.as_dtype(Dtype::Float32).m()?;
        l.eval().m()?;
        Ok(l.as_slice::<f32>().to_vec())
    }

    /// El paso condicional y el incondicional con UNA sola espera a la GPU (los dos grafos se
    /// evalúan juntos): mismo cálculo, la mitad de idas y vueltas.
    fn logits2(&self, t: u32, cc: &mut crate::modelo::Cache, cu: &mut crate::modelo::Cache) -> Result<(Vec<f32>, Vec<f32>)> {
        let a = self.modelo.adelante(&[t], cc)?.as_dtype(Dtype::Float32).m()?;
        let b = self.modelo.adelante(&[t], cu)?.as_dtype(Dtype::Float32).m()?;
        mlx_rs::transforms::eval([&a, &b]).m()?;
        Ok((a.as_slice::<f32>().to_vec(), b.as_slice::<f32>().to_vec()))
    }

    /// Los logits del CFG restringidos a los códigos (+ EOS) para el siguiente paso, como
    /// `_generate_with_cfg_custom` + el FSM en `CODES_GENERATION`: (token, logit).
    pub(crate) fn candidatos(&self, lc: &[f32], lu: &[f32], cfg: f32, codigos: usize, meta: usize) -> Vec<(u32, f32)> {
        let combina = |i: usize| lu[i] + cfg * (lc[i] - lu[i]);
        if codigos >= meta {
            // forzado: sólo el EOS (con su logit)
            return vec![(self.eos, combina(self.eos as usize))];
        }
        (0..=MAX_CODIGO).map(|n| (self.base + n, combina((self.base + n) as usize))).collect()
    }

    /// La fase 2: los códigos (0..63 999) para `segundos`, con la CoT dada.
    #[allow(clippy::too_many_arguments)]
    pub fn codigos(&self, caption: &str, letra: &str, cot: &str, segundos: f32, m: &Muestreo, semilla: u64, cancelar: &AtomicBool, progreso: &dyn Fn(usize, usize)) -> Result<Vec<u32>> {
        let meta = (segundos as f64 * 5.0) as usize;
        let tope = meta + 10;
        let (pc, pu) = self.prompts_con(caption, letra, cot, &m.negativo)?;
        let (ic, iu) = (self.tokens(&pc)?, self.tokens(&pu)?);
        let (mut cc, mut cu) = (self.modelo.cache_nueva(), self.modelo.cache_nueva());
        let (mut lc, mut lu) = (self.logits(&ic, &mut cc)?, self.logits(&iu, &mut cu)?);
        let mut azar = Azar::new(semilla);
        let mut salida = Vec::with_capacity(meta);
        for _ in 0..tope {
            if cancelar.load(Ordering::Relaxed) {
                bail!("cancelado");
            }
            let mut c = self.candidatos(&lc, &lu, m.cfg, salida.len(), meta);
            top_k(&mut c, m.top_k);
            top_p(&mut c, m.top_p);
            let t = muestrear(&c, m.temperatura, &mut azar);
            if t == self.eos {
                break;
            }
            salida.push(t - self.base);
            progreso(salida.len(), meta);
            (lc, lu) = self.logits2(t, &mut cc, &mut cu)?;
        }
        Ok(salida)
    }

    /// La fase 1 (CoT): los metadatos que faltan, la descripción y el idioma que escribe el LM.
    /// Como `_generate_with_constrained_decoding` (sin CFG: el oficial lo apaga en esta fase): la
    /// máquina de estados filtra, luego top-p 0,9 y temperatura 0,85; para en el EOS que la
    /// máquina fuerza al llegar a `</think>`. Devuelve los metadatos de `parse_lm_output`.
    pub fn plan(&self, caption: &str, letra: &str, dados: &Metas, semilla: u64, cancelar: &AtomicBool) -> Result<Metas> {
        let (texto, _) = self.plan_tokens(caption, letra, dados, semilla, cancelar, None)?;
        Ok(parsear(&texto))
    }

    /// La fase 1 token a token. `forzados`: los del juez (se siguen en vez de muestrear y cada paso
    /// devuelve si el token forzado estaba entre los candidatos tras el top-p).
    pub fn plan_tokens(&self, caption: &str, letra: &str, dados: &Metas, semilla: u64, cancelar: &AtomicBool, forzados: Option<&[u32]>) -> Result<(String, Vec<bool>)> {
        let sistema = format!("# Instruction\n{INSTRUCCION_LM}\n\n");
        let prompt = self.plantilla.render_simple(&[("system", &sistema), ("user", &format!("# Caption\n{caption}\n\n# Lyric\n{letra}\n"))])?;
        let ids = self.tokens(&prompt)?;
        let m = Muestreo::default();
        let mut cache = self.modelo.cache_nueva();
        let mut logits = self.logits(&ids, &mut cache)?;
        let mut azar = Azar::new(semilla ^ 0xC07);
        let mut fsm = Cot::new(self, dados);
        let (mut salida, mut dentro) = (Vec::new(), Vec::new());
        // `_compute_max_new_tokens` sin duración objetivo: 600·5 + 500
        for paso in 0..3500 {
            if cancelar.load(Ordering::Relaxed) {
                bail!("cancelado");
            }
            let mut c: Vec<(u32, f32)> = match fsm.procesar(&logits)? {
                Filtro::Lista(mut l) => {
                    l.sort_unstable();
                    l.dedup();
                    l.into_iter().map(|t| (t, logits[t as usize])).collect()
                }
                Filtro::Texto => {
                    let codigo = self.base..=self.base + MAX_CODIGO;
                    (0..logits.len() as u32).filter(|t| !codigo.contains(t) && Some(*t) != self.acento).map(|t| (t, logits[t as usize])).collect()
                }
                Filtro::Libre => (0..logits.len() as u32).map(|t| (t, logits[t as usize])).collect(),
            };
            if c.is_empty() {
                bail_tr!("la máquina de la fase 1 no dejó ningún token (paso {paso})", "the phase-1 machine allowed no token (step {paso})");
            }
            top_p(&mut c, m.top_p);
            let t = match forzados {
                Some(f) => {
                    let Some(&t) = f.get(paso) else { break };
                    dentro.push(c.iter().any(|x| x.0 == t));
                    t
                }
                None => muestrear(&c, m.temperatura, &mut azar),
            };
            fsm.actualizar(t);
            salida.push(t);
            if t == self.eos || t == self.pad {
                break;
            }
            logits = self.logits(&[t], &mut cache)?;
        }
        Ok((self.tk.decode(&salida, false).map_err(|e| anyhow!("{e}"))?, dentro))
    }

    /// Para el juez: los candidatos del CFG (antes de top-p) tras forzar los tokens dados, paso a
    /// paso, con `MAX_CODIGO` + 1 logits por paso.
    pub fn cfg_forzado(&self, caption: &str, letra: &str, cot: &str, forzados: &[u32], cfg: f32) -> Result<Vec<Vec<f32>>> {
        let (pc, pu) = self.prompts(caption, letra, cot)?;
        let (ic, iu) = (self.tokens(&pc)?, self.tokens(&pu)?);
        let (mut cc, mut cu) = (self.modelo.cache_nueva(), self.modelo.cache_nueva());
        let (mut lc, mut lu) = (self.logits(&ic, &mut cc)?, self.logits(&iu, &mut cu)?);
        let mut pasos = Vec::new();
        for &t in forzados {
            pasos.push(self.candidatos(&lc, &lu, cfg, 0, usize::MAX).iter().map(|x| x.1).collect());
            lc = self.logits(&[t], &mut cc)?;
            lu = self.logits(&[t], &mut cu)?;
        }
        Ok(pasos)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Ola PRO: `lm_top_k` deja los k de logit más alto (0 = todos); el negativo por omisión es el
    /// oficial y uno vacío vuelve a él.
    #[test]
    fn top_k_y_negativo_por_omision() {
        let mut c = vec![(1, 0.5), (2, 3.0), (3, -1.0), (4, 2.0)];
        top_k(&mut c, 2);
        assert_eq!(c.iter().map(|x| x.0).collect::<Vec<_>>(), vec![2, 4]);
        let mut d = vec![(1, 0.5), (2, 3.0)];
        top_k(&mut d, 0);
        assert_eq!(d.len(), 2);
        assert_eq!(Muestreo::default().negativo, "NO USER INPUT");
    }

    #[test]
    fn la_cot_es_la_del_oficial() {
        // la que capturó el juez (bpm 85, 10 s, C major, 4/4)
        let m = Metas { bpm: Some(85), duracion: Some(10), tonalidad: Some("C major".into()), compas: Some("4".into()), ..Default::default() };
        assert_eq!(cot(&m), "<think>\nbpm: 85\nduration: 10\nkeyscale: C major\ntimesignature: 4\n</think>");
        let m2 = Metas { compas: Some("3/4".into()), tonalidad: Some("F# minor".into()), ..Default::default() };
        assert_eq!(cot(&m2), "<think>\nkeyscale: F# minor\ntimesignature: 3\n</think>");
        assert_eq!(yaml_valor("C major", 9), " C major");
        assert_eq!(yaml_valor("no", 9), " 'no'", "«no» (noruego) es un bool de YAML 1.1");
        assert_eq!(yaml_valor("a: b", 8), " 'a: b'");
        assert_eq!(yaml_valor("it's", 8), " it's");
        assert_eq!(yaml_valor("'x' y", 8), " '''x'' y'");
    }

    #[test]
    fn top_p_como_el_oficial() {
        // probabilidades 0.5, 0.3, 0.15, 0.05: con p = 0.7 quedan las dos primeras (la 2.ª entra porque
        // la suma ANTERIOR, 0.5, no pasa p) y la 3.ª sale (la suma anterior, 0.8, sí pasa)
        let l = |p: f32| p.ln();
        let mut c = vec![(3, l(0.05)), (0, l(0.5)), (2, l(0.15)), (1, l(0.3))];
        top_p(&mut c, 0.7);
        assert_eq!(c.iter().map(|x| x.0).collect::<Vec<_>>(), vec![0, 1]);
        let mut d = vec![(0, 5.0), (1, 1.0)];
        top_p(&mut d, 0.9);
        assert_eq!(d.len(), 1, "el primero solo ya pasa p");
    }

    /// Juez: los prompts y los logits del CFG con forzado del maestro contra el volcado oficial.
    /// `GOTV_JUEZ_LM=<carpeta con meta.json>`; deja `gotv_cfg.safetensors` al lado.
    #[test]
    #[ignore]
    fn juez_cfg_forzado() {
        let dir = std::path::PathBuf::from(std::env::var("GOTV_JUEZ_LM").expect("GOTV_JUEZ_LM"));
        let meta: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dir.join("meta.json")).unwrap()).unwrap();
        let lm = Lm::cargar(&dirs_lm()).unwrap();
        let m = Metas { bpm: Some(85), duracion: Some(10), tonalidad: Some("C major".into()), compas: Some("4".into()), ..Default::default() };
        let (pc, pu) = lm.prompts("calm lo-fi hip hop beat with warm piano and soft drums", "[Instrumental]", &cot(&m)).unwrap();
        let ids = |v: &serde_json::Value| v.as_array().unwrap().iter().map(|x| x.as_u64().unwrap() as u32).collect::<Vec<_>>();
        assert_eq!(lm.tokens(&pc).unwrap(), ids(&meta["cfg_ids"][0]), "prompt condicional");
        assert_eq!(lm.tokens(&pu).unwrap(), ids(&meta["cfg_ids"][1]), "prompt incondicional");
        let salida = ids(&meta["cfg_salida"]);
        let t0 = std::time::Instant::now();
        let pasos = lm.cfg_forzado("calm lo-fi hip hop beat with warm piano and soft drums", "[Instrumental]", &cot(&m), &salida[..salida.len() - 1], 2.0).unwrap();
        eprintln!("{} pasos en {:.2} s", pasos.len(), t0.elapsed().as_secs_f32());
        let n = pasos.len() as i32;
        let plano: Vec<f32> = pasos.into_iter().flatten().collect();
        let a = mlx_rs::Array::from_slice(&plano, &[n, MAX_CODIGO as i32 + 1]);
        mlx_rs::Array::save_safetensors([("cfg", &a)], None, dir.join("gotv_cfg.safetensors")).unwrap();
    }

    /// Juez de la fase 1: `GOTV_JUEZ_F1=<carpeta con meta.json>` (captura de `juez_ace_lm.py … fase1`)
    /// y `GOTV_JUEZ_F1_ARGS=<caption>|<letra>|<segundos>`. Fuerza los tokens oficiales por la máquina:
    /// cada uno debe estar entre los candidatos; el texto, el parser, la CoT y el prompt de la fase 2
    /// deben salir idénticos.
    #[test]
    #[ignore]
    fn juez_fase1() {
        let dir = std::path::PathBuf::from(std::env::var("GOTV_JUEZ_F1").expect("GOTV_JUEZ_F1"));
        let args = std::env::var("GOTV_JUEZ_F1_ARGS").expect("GOTV_JUEZ_F1_ARGS");
        let a: Vec<&str> = args.splitn(3, '|').collect();
        let (caption, letra, seg) = (a[0], a[1].replace("\\n", "\n"), a[2].parse::<u32>().unwrap());
        let meta: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dir.join("meta.json")).unwrap()).unwrap();
        let ids = |v: &serde_json::Value| v.as_array().unwrap().iter().map(|x| x.as_u64().unwrap() as u32).collect::<Vec<_>>();
        let lm = Lm::cargar(&dirs_lm()).unwrap();
        let sistema = format!("# Instruction\n{INSTRUCCION_LM}\n\n");
        let prompt = lm.plantilla.render_simple(&[("system", &sistema), ("user", &format!("# Caption\n{caption}\n\n# Lyric\n{letra}\n"))]).unwrap();
        assert_eq!(lm.tokens(&prompt).unwrap(), ids(&meta["fase1"][0]["ids"]), "prompt de la fase 1");
        let oficial = ids(&meta["fase1"][0]["salida"]);
        let dados = Metas { duracion: Some(seg), ..Default::default() };
        let (texto, dentro) = lm.plan_tokens(caption, &letra, &dados, 0, &AtomicBool::new(false), Some(&oficial)).unwrap();
        let fuera: Vec<usize> = dentro.iter().enumerate().filter(|(_, d)| !**d).map(|(i, _)| i).collect();
        eprintln!("{} tokens forzados; fuera de los candidatos: {fuera:?}", dentro.len());
        assert!(fuera.is_empty());
        assert_eq!(dentro.len(), oficial.len());
        assert_eq!(texto, lm.tk.decode(&oficial, false).unwrap());
        let m = parsear(&texto);
        eprintln!("{m:?}");
        let prompts: Vec<&serde_json::Value> = meta["prompts_lm"].as_array().unwrap().iter().filter(|p| p["fn"] == "build_formatted_prompt_with_cot").collect();
        let (pc, _) = lm.prompts(caption, &letra, &cot(&m)).unwrap();
        // el oficial construye el prompt SIN el «\n\n» final (lo añade al tokenizar)
        let esperado = prompts[0]["texto"].as_str().unwrap();
        assert_eq!(pc.trim_end_matches('\n'), esperado.trim_end_matches('\n'), "prompt de la fase 2");
    }

    fn dirs_lm() -> std::path::PathBuf {
        std::path::PathBuf::from(std::env::var("HOME").unwrap()).join(".cache/gotv/models/ACE-Step/Ace-Step1.5/acestep-5Hz-lm-1.7B")
    }

    #[test]
    fn el_azar_es_reproducible_y_uniforme() {
        let (mut a, mut b) = (Azar::new(42), Azar::new(42));
        let xs: Vec<f64> = (0..1000).map(|_| a.uniforme()).collect();
        assert!(xs.iter().zip((0..1000).map(|_| b.uniforme())).all(|(x, y)| *x == y));
        let media = xs.iter().sum::<f64>() / 1000.0;
        assert!((media - 0.5).abs() < 0.05 && xs.iter().all(|x| (0.0..1.0).contains(x)));
    }
}
