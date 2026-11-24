//! Ola EXPERTO · EXP-6 — el intercambio con los DAW profesionales.
//!
//! **Exportar** (`exportar`) a una carpeta:
//! - `<nombre>-mezcla.wav`: la mezcla, en PCM de 24 bits (normalizada a LUFS si se pide, con tope de pico −1 dBFS);
//! - `pistas/NN-<pista>.wav`: lo que aporta CADA pista que suena (fader, paneo y grupo ya aplicados): sumadas
//!   dan la mezcla; las abre cualquier DAW (Logic, Ableton, Pro Tools, FL, Reaper) alineadas desde 0;
//! - `<nombre>.dawproject`: el formato abierto de Bitwig y PreSonus (MIT; lo leen Bitwig 5.0.9, Studio One 6.5,
//!   Cubase 14, Cubasis 3.7.1, VST Live 2.2 y n-Track 10.2.2; Reaper con ProjectConverter): cada pista con su
//!   audio ANTES del fader (regiones y efectos aplicados), y su volumen, paneo, silencio y solo como mezclador
//!   editable; los grupos como carpetas con un canal de submezcla; tempo, compás y marcadores. El paneo lo
//!   aplica el DAW con SU ley de paneo (puede diferir del de GOTV); los stems no.
//!
//! **Importar** (`importar_dawproject`): las pistas de audio (clips en segundos o en tiempos, también los clips
//! anidados de Bitwig), las carpetas como grupos, el mezclador, el tempo y los marcadores. Cada pista se
//! «aplana» a un WAV a la frecuencia de la sesión con sus clips en su sitio.
use crate::experto::{Grupo, Marcador, Op, Origen, Pista, Sesion};
use crate::experto_dsp::{self as dsp, Estereo};
use anyhow::{Context, Result};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

// ───────────────────────────── XML (escritura) ─────────────────────────────

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&apos;")
}

/// Un nombre de archivo sin lo que estorba en los sistemas de archivos.
fn sano(s: &str) -> String {
    let t: String = s.chars().map(|c| if c.is_alphanumeric() || " -_.()".contains(c) { c } else { '_' }).collect();
    let t = t.trim().trim_matches('.').to_string();
    if t.is_empty() { "pista".into() } else { t }
}

// ───────────────────────────── XML (lectura mínima) ─────────────────────────────

/// Un elemento XML (lo que hace falta para `project.xml`: nombres, atributos e hijos; el texto se ignora).
#[derive(Debug, Clone, Default)]
pub struct Nodo {
    pub nombre: String,
    pub attrs: Vec<(String, String)>,
    pub hijos: Vec<Nodo>,
}

impl Nodo {
    pub fn attr(&self, k: &str) -> Option<&str> {
        self.attrs.iter().find(|(a, _)| a == k).map(|(_, v)| v.as_str())
    }
    pub fn num(&self, k: &str) -> Option<f64> {
        self.attr(k).and_then(|v| v.trim().parse().ok())
    }
    pub fn hijo(&self, n: &str) -> Option<&Nodo> {
        self.hijos.iter().find(|h| h.nombre == n)
    }
    pub fn hijos_de<'a>(&'a self, n: &'a str) -> impl Iterator<Item = &'a Nodo> + 'a {
        self.hijos.iter().filter(move |h| h.nombre == n)
    }
}

fn entidades(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    let mut r = s;
    while let Some(i) = r.find('&') {
        o.push_str(&r[..i]);
        let resto = &r[i..];
        match resto.find(';') {
            Some(f) if f <= 10 => {
                let e = &resto[1..f];
                let c = match e {
                    "amp" => Some('&'),
                    "lt" => Some('<'),
                    "gt" => Some('>'),
                    "quot" => Some('"'),
                    "apos" => Some('\''),
                    _ if e.starts_with("#x") => u32::from_str_radix(&e[2..], 16).ok().and_then(char::from_u32),
                    _ if e.starts_with('#') => e[1..].parse().ok().and_then(char::from_u32),
                    _ => None,
                };
                match c {
                    Some(c) => {
                        o.push(c);
                        r = &resto[f + 1..];
                    }
                    None => {
                        o.push('&');
                        r = &resto[1..];
                    }
                }
            }
            _ => {
                o.push('&');
                r = &resto[1..];
            }
        }
    }
    o.push_str(r);
    o
}

/// Lee un documento XML a su árbol (declaración, comentarios, DOCTYPE y CDATA se saltan).
pub fn leer_xml(t: &str) -> Result<Nodo> {
    let b = t.as_bytes();
    let mut i = 0;
    let mut pila: Vec<Nodo> = vec![Nodo { nombre: "#doc".into(), ..Default::default() }];
    let mal = || gotv_engine::anyhow_tr!("XML mal formado", "malformed XML");
    while i < b.len() {
        if b[i] != b'<' {
            i += 1;
            continue;
        }
        let r = &t[i..];
        if r.starts_with("<?") {
            i += r.find("?>").ok_or_else(mal)? + 2;
        } else if r.starts_with("<!--") {
            i += r.find("-->").ok_or_else(mal)? + 3;
        } else if r.starts_with("<![CDATA[") {
            i += r.find("]]>").ok_or_else(mal)? + 3;
        } else if r.starts_with("<!") {
            i += r.find('>').ok_or_else(mal)? + 1;
        } else if r.starts_with("</") {
            let fin = r.find('>').ok_or_else(mal)?;
            let nodo = pila.pop().ok_or_else(mal)?;
            if nodo.nombre != r[2..fin].trim() {
                return Err(mal());
            }
            pila.last_mut().ok_or_else(mal)?.hijos.push(nodo);
            i += fin + 1;
        } else {
            // la etiqueta de apertura: hasta el `>` que no está dentro de comillas
            let (mut k, mut comilla) = (1usize, None::<u8>);
            let rb = r.as_bytes();
            while k < rb.len() {
                match (comilla, rb[k]) {
                    (None, b'"') | (None, b'\'') => comilla = Some(rb[k]),
                    (Some(q), c) if c == q => comilla = None,
                    (None, b'>') => break,
                    _ => {}
                }
                k += 1;
            }
            if k >= rb.len() {
                return Err(mal());
            }
            let dentro = &r[1..k];
            let cerrada = dentro.ends_with('/');
            let dentro = dentro.trim_end_matches('/');
            let nombre_fin = dentro.find(|c: char| c.is_whitespace()).unwrap_or(dentro.len());
            let mut nodo = Nodo { nombre: dentro[..nombre_fin].to_string(), ..Default::default() };
            let mut a = dentro[nombre_fin..].trim();
            while !a.is_empty() {
                let eq = a.find('=').ok_or_else(mal)?;
                let clave = a[..eq].trim().to_string();
                let v = a[eq + 1..].trim_start();
                let q = v.chars().next().ok_or_else(mal)?;
                if q != '"' && q != '\'' {
                    return Err(mal());
                }
                let fin = v[1..].find(q).ok_or_else(mal)? + 1;
                nodo.attrs.push((clave, entidades(&v[1..fin])));
                a = v[fin + 1..].trim_start();
            }
            if cerrada {
                pila.last_mut().ok_or_else(mal)?.hijos.push(nodo);
            } else {
                pila.push(nodo);
            }
            i += k + 1;
        }
    }
    if pila.len() != 1 {
        return Err(mal());
    }
    pila.pop().and_then(|mut d| d.hijos.pop()).ok_or_else(mal)
}

// ───────────────────────────── exportar ─────────────────────────────

#[derive(Debug, Clone)]
pub struct Opciones {
    /// el nombre base de los archivos
    pub nombre: String,
    /// llevar la mezcla a estos LUFS (−14 es lo usual en plataformas), con tope de pico −1 dBFS
    pub normalizar_lufs: Option<f64>,
    /// exportar sólo este tramo `(desde, hasta)` en segundos
    pub tramo: Option<(f64, f64)>,
    /// 16 (con tramado), 24 o 32 (coma flotante)
    pub bits: u8,
    /// las partes que suman la mezcla (pistas y buses), una por archivo
    pub pistas: bool,
    pub dawproject: bool,
    /// además del WAV, la mezcla en «flac» o «m4a» (AAC de 256 kb/s); vacío = sólo WAV
    pub formato: String,
}

impl Default for Opciones {
    fn default() -> Self {
        Opciones { nombre: "sesion".into(), normalizar_lufs: None, tramo: None, bits: 24, pistas: true, dawproject: true, formato: String::new() }
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Exportado {
    pub mezcla: PathBuf,
    pub pistas: Vec<PathBuf>,
    pub dawproject: Option<PathBuf>,
    /// la mezcla en el otro formato pedido (FLAC o M4A)
    pub extra: Option<PathBuf>,
    /// la ganancia aplicada a la mezcla por la normalización (0 sin normalizar)
    pub ganancia_db: f64,
    /// los LUFS de la mezcla exportada
    pub lufs: Option<f64>,
    /// el pico real (entre muestras) de la mezcla exportada, en dBTP
    pub pico_real_db: f64,
    /// lo que no se pudo subir por el tope de pico
    pub falto_db: f64,
}

fn escribir(ruta: &Path, x: &Estereo, fs: u32, bits: u8) -> Result<()> {
    match bits {
        16 => dsp::escribir_wav16(ruta, x, fs),
        32 => dsp::escribir_wav(ruta, x, fs, false),
        _ => dsp::escribir_wav(ruta, x, fs, true),
    }
}

/// Exporta la sesión a `destino` (ver el encabezado del módulo). La mezcla lleva buses y maestro; las
/// partes (cada pista, o cada bus con efectos) son lo que SUMA la mezcla antes del maestro.
pub fn exportar(root: &Path, s: &Sesion, destino: &Path, o: &Opciones) -> Result<Exportado> {
    let e = s.estado();
    let fs = s.frecuencia;
    if e.pistas.is_empty() {
        anyhow::bail!("{}", crate::i18n::t("la sesión no tiene pistas que exportar", "the session has no tracks to export"));
    }
    let crudas: Vec<Estereo> = e.pistas.iter().map(|p| dsp::render_pista(&e, p, fs, root)).collect::<Result<_>>()?;
    let m = dsp::mezclar_todo(&e, fs, root)?;
    let mut mezcla = m.mezcla.clone();
    // las partes que suenan, con su nombre
    let sonando: std::collections::HashSet<&str> = e.pistas.iter().filter(|p| dsp::suena(&e, p)).map(|p| p.nombre.as_str()).collect();
    let buses: std::collections::HashSet<&str> = m.buses.iter().filter_map(|(id, _)| e.grupos.iter().find(|g| g.id == *id)).map(|g| g.nombre.as_str()).collect();
    let mut partes: Vec<(String, Estereo)> = m.partes(&e).into_iter().filter(|(n, _)| sonando.contains(n.as_str()) || buses.contains(n.as_str())).map(|(n, x)| (n, x.clone())).collect();
    if let Some((a, b)) = o.tramo {
        if !(a.is_finite() && b.is_finite() && a >= 0.0 && b > a) {
            anyhow::bail!("{}", crate::i18n::t("el tramo va de un segundo a otro MAYOR", "the span goes from one second to a LATER one"));
        }
        let (i, j) = ((a * fs as f64).round() as usize, (b * fs as f64).round() as usize);
        let corta = |x: &mut Estereo| x.iter_mut().for_each(|c| *c = (i..j).map(|k| c.get(k).copied().unwrap_or(0.0)).collect());
        corta(&mut mezcla);
        partes.iter_mut().for_each(|(_, x)| corta(x));
    }
    let (mut g, mut falto) = (0.0, 0.0);
    if let Some(objetivo) = o.normalizar_lufs {
        if let Some((gn, f)) = dsp::normalizar(&mezcla, fs, objetivo) {
            (g, falto) = (gn, f);
            let lin = 10f64.powf(g / 20.0);
            for x in partes.iter_mut().map(|(_, x)| x).chain(std::iter::once(&mut mezcla)) {
                for c in x.iter_mut() {
                    c.iter_mut().for_each(|v| *v = (*v as f64 * lin) as f32);
                }
            }
        }
    }
    // las partes, del MISMO largo que la mezcla: alineadas desde 0 y hasta el final, como las espera un DAW
    let largo = mezcla[0].len();
    partes.iter_mut().for_each(|(_, x)| x.iter_mut().for_each(|c| c.resize(largo, 0.0)));
    std::fs::create_dir_all(destino)?;
    let base = sano(&o.nombre);
    let ruta_mezcla = destino.join(format!("{base}-mezcla.wav"));
    escribir(&ruta_mezcla, &mezcla, fs, o.bits)?;
    let mut pistas = Vec::new();
    if o.pistas {
        std::fs::create_dir_all(destino.join("pistas"))?;
        for (k, (nombre, x)) in partes.iter().enumerate() {
            let r = destino.join("pistas").join(format!("{:02}-{}.wav", k + 1, sano(nombre)));
            escribir(&r, x, fs, o.bits)?;
            pistas.push(r);
        }
    }
    let mut dawproject = None;
    if o.dawproject {
        let ruta_dp = destino.join(format!("{base}.dawproject"));
        escribir_dawproject(&ruta_dp, &e, &crudas, fs, g + e.maestro.ganancia_db as f64, &o.nombre)?;
        dawproject = Some(ruta_dp);
    }
    // el otro formato lo hace `afconvert` (macOS) desde el WAV ya escrito
    let extra = match o.formato.as_str() {
        "" | "wav" => None,
        f @ ("flac" | "m4a") => {
            let r = destino.join(format!("{base}-mezcla.{f}"));
            let args: &[&str] = if f == "flac" { &["-f", "flac", "-d", "flac"] } else { &["-f", "m4af", "-d", "aac", "-b", "256000"] };
            let sal = std::process::Command::new("afconvert").args(args).arg(&ruta_mezcla).arg(&r).output().map_err(|e| gotv_engine::anyhow_tr!("afconvert: {e}", "afconvert: {e}"))?;
            if !sal.status.success() || !r.is_file() {
                anyhow::bail!("{}", crate::tr!("no se pudo escribir el {f}: {}", "could not write the {f}: {}", String::from_utf8_lossy(&sal.stderr).trim()));
            }
            Some(r)
        }
        otro => anyhow::bail!("{}", crate::tr!("formato desconocido: {otro} (wav, flac o m4a)", "unknown format: {otro} (wav, flac or m4a)")),
    };
    Ok(Exportado { mezcla: ruta_mezcla, pistas, dawproject, extra, ganancia_db: g, lufs: dsp::lufs(&mezcla, fs), pico_real_db: crate::experto_analisis::pico_real_db(&mezcla), falto_db: falto })
}

/// Un número sin exponente ni comas (lo que esperan los atributos de DAWproject).
fn n(v: f64) -> String {
    format!("{v:.6}")
}

/// El `.dawproject` (zip con `project.xml`, `metadata.xml` y `audio/`).
fn escribir_dawproject(ruta: &Path, e: &crate::experto::Estado, crudas: &[Estereo], fs: u32, ganancia_maestro_db: f64, titulo: &str) -> Result<()> {
    use std::fmt::Write as _;
    let id = std::cell::Cell::new(0usize);
    let nuevo = || {
        id.set(id.get() + 1);
        format!("id{}", id.get())
    };
    let canal = |x: &mut String, idc: &str, rol: &str, destino: Option<&str>, db: f32, pan: f32, mudo: bool, solo: bool, ids: [String; 3]| {
        let vol = 10f64.powf(db as f64 / 20.0);
        let _ = write!(
            x,
            r#"<Channel audioChannels="2"{} role="{rol}" solo="{solo}" id="{idc}"><Mute value="{mudo}" id="{}" name="Mute"/><Pan max="1.000000" min="0.000000" unit="normalized" value="{}" id="{}" name="Pan"/><Volume max="{}" min="0.000000" unit="linear" value="{}" id="{}" name="Volume"/></Channel>"#,
            destino.map(|d| format!(r#" destination="{d}""#)).unwrap_or_default(),
            ids[0],
            n((pan as f64 + 1.0) / 2.0),
            ids[1],
            n(vol.max(2.0)),
            format!("{vol:.10}"),
            ids[2]
        );
    };
    let id_maestro = nuevo();
    let pistas_id: Vec<(String, String)> = e.pistas.iter().map(|_| (nuevo(), nuevo())).collect();
    let grupos_id: Vec<(String, String)> = e.grupos.iter().map(|_| (nuevo(), nuevo())).collect();
    let mut est = String::new();
    // la pista de una pista de audio
    let pista_xml = |x: &mut String, k: usize, destino: &str| {
        let p = &e.pistas[k];
        let (tid, cid) = &pistas_id[k];
        let _ = write!(x, r#"<Track contentType="audio" loaded="true" id="{tid}" name="{}">"#, esc(&p.nombre));
        let ids = [nuevo(), nuevo(), nuevo()];
        canal(x, cid, "regular", Some(destino), p.ganancia_db, p.pan, p.silencio, p.solo, ids);
        x.push_str("</Track>");
    };
    for (gi, g) in e.grupos.iter().enumerate() {
        let (tid, cid) = &grupos_id[gi];
        let _ = write!(est, r#"<Track contentType="audio" loaded="true" id="{tid}" name="{}">"#, esc(&g.nombre));
        let ids = [nuevo(), nuevo(), nuevo()];
        canal(&mut est, cid, "submix", Some(&id_maestro), g.ganancia_db, g.pan, g.silencio, g.solo, ids);
        for (k, p) in e.pistas.iter().enumerate() {
            if p.grupo == Some(g.id) {
                pista_xml(&mut est, k, cid);
            }
        }
        est.push_str("</Track>");
    }
    for (k, p) in e.pistas.iter().enumerate() {
        if !p.grupo.is_some_and(|g| e.grupos.iter().any(|x| x.id == g)) {
            pista_xml(&mut est, k, &id_maestro);
        }
    }
    let id_pista_maestro = nuevo();
    let _ = write!(est, r#"<Track contentType="audio" loaded="true" id="{id_pista_maestro}" name="Master">"#);
    let ids = [nuevo(), nuevo(), nuevo()];
    canal(&mut est, &id_maestro, "master", None, ganancia_maestro_db as f32, 0.0, false, false, ids);
    est.push_str("</Track>");
    // el arreglo: un clip por pista, desde 0, con su audio
    let mut lanes = String::new();
    let mut audios: Vec<(String, &Estereo)> = Vec::new();
    for (k, p) in e.pistas.iter().enumerate() {
        let x = &crudas[k];
        let dur = x[0].len() as f64 / fs as f64;
        let archivo = format!("audio/{:02}-{}.wav", k + 1, sano(&p.nombre));
        let _ = write!(
            lanes,
            r#"<Lanes track="{}" id="{}"><Clips id="{}"><Clip time="0.000000" duration="{d}" playStart="0.000000" name="{}"><Audio channels="2" duration="{d}" sampleRate="{fs}" id="{}"><File path="{}"/></Audio></Clip></Clips></Lanes>"#,
            pistas_id[k].0,
            nuevo(),
            nuevo(),
            esc(&p.nombre),
            nuevo(),
            esc(&archivo),
            d = n(dur)
        );
        audios.push((archivo, x));
    }
    let marcadores = if e.marcadores.is_empty() {
        String::new()
    } else {
        let mut m = format!(r#"<Markers timeUnit="seconds" id="{}">"#, nuevo());
        for mk in &e.marcadores {
            let _ = write!(m, r#"<Marker time="{}" name="{}"/>"#, n(mk.segundo), esc(&mk.nombre));
        }
        m.push_str("</Markers>");
        m
    };
    let proyecto = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Project version="1.0"><Application name="GOD OF THE VOID" version="{}"/><Transport><Tempo max="400.000000" min="20.000000" unit="bpm" value="{}" id="{}" name="Tempo"/><TimeSignature denominator="4" numerator="{}" id="{}"/></Transport><Structure>{est}</Structure><Arrangement id="{}"><Lanes timeUnit="seconds" id="{}">{lanes}</Lanes>{marcadores}</Arrangement></Project>
"#,
        env!("CARGO_PKG_VERSION"),
        n(e.bpm as f64),
        nuevo(),
        e.compas,
        nuevo(),
        nuevo(),
        nuevo()
    );
    let meta = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<MetaData><Title>{}</Title><Comment>{}</Comment></MetaData>\n",
        esc(titulo),
        esc(crate::i18n::t("Exportado por GOD OF THE VOID (estudio Experto)", "Exported by GOD OF THE VOID (Expert studio)"))
    );
    if let Some(d) = ruta.parent() {
        std::fs::create_dir_all(d)?;
    }
    let tmp = ruta.with_extension("dawproject.tmp");
    {
        use zip::write::SimpleFileOptions;
        use zip::CompressionMethod;
        let mut w = zip::ZipWriter::new(std::fs::File::create(&tmp)?);
        let comprimido = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
        w.start_file("project.xml", comprimido)?;
        w.write_all(proyecto.as_bytes())?;
        w.start_file("metadata.xml", comprimido)?;
        w.write_all(meta.as_bytes())?;
        let wav_tmp = ruta.with_extension("wav.tmp");
        for (archivo, x) in audios {
            dsp::escribir_wav(&wav_tmp, x, fs, false)?;
            w.start_file(archivo.as_str(), SimpleFileOptions::default().compression_method(CompressionMethod::Stored).large_file(true))?;
            w.write_all(&std::fs::read(&wav_tmp)?)?;
        }
        let _ = std::fs::remove_file(&wav_tmp);
        w.finish()?;
    }
    std::fs::rename(&tmp, ruta)?;
    Ok(())
}

// ───────────────────────────── importar ─────────────────────────────

/// Un clip de audio del arreglo, en SEGUNDOS: cuándo empieza, desde dónde del archivo, cuánto dura.
#[derive(Debug, Clone, PartialEq)]
pub struct ClipAudio {
    pub inicio: f64,
    pub desde: f64,
    pub duracion: Option<f64>,
    pub archivo: String,
}

fn a_segundos(v: f64, unidad: &str, bpm: f64) -> f64 {
    if unidad == "beats" { v * 60.0 / bpm } else { v }
}

/// Los clips de audio dentro de un `Clips`/`Lanes`, con su desplazamiento acumulado (clips anidados de Bitwig).
fn clips_de(n: &Nodo, unidad: &str, bpm: f64, desplazamiento: f64, out: &mut Vec<ClipAudio>) {
    for h in &n.hijos {
        match h.nombre.as_str() {
            "Clips" | "Lanes" => clips_de(h, h.attr("timeUnit").unwrap_or(unidad), bpm, desplazamiento, out),
            "Clip" => {
                let t = desplazamiento + a_segundos(h.num("time").unwrap_or(0.0), unidad, bpm);
                let play = a_segundos(h.num("playStart").unwrap_or(0.0), h.attr("contentTimeUnit").unwrap_or(unidad), bpm);
                let dur = h.num("duration").map(|d| a_segundos(d, unidad, bpm));
                // el audio puede venir directo, dentro de Warps o dentro de otro nivel de clips
                let audio = h.hijo("Audio").or_else(|| h.hijo("Warps").and_then(|w| w.hijo("Audio")));
                if let Some(a) = audio {
                    if let Some(f) = a.hijo("File").and_then(|f| f.attr("path")) {
                        out.push(ClipAudio { inicio: t, desde: play, duracion: dur, archivo: f.to_string() });
                    }
                } else if let Some(c) = h.hijo("Clips") {
                    clips_de(c, h.attr("contentTimeUnit").unwrap_or(unidad), bpm, t - play, out);
                }
            }
            _ => {}
        }
    }
}

/// Lo que dice un `project.xml`: pistas de audio (con su carpeta), su mezclador y sus clips, tempo y marcadores.
#[derive(Debug, Clone, Default)]
pub struct Proyecto {
    pub bpm: f64,
    pub compas: u8,
    /// (nombre, volumen dB, pan −1…1, silencio, solo, carpeta, clips)
    pub pistas: Vec<(String, f32, f32, bool, bool, Option<usize>, Vec<ClipAudio>)>,
    /// (nombre, volumen dB, pan, silencio, solo)
    pub carpetas: Vec<(String, f32, f32, bool, bool)>,
    pub marcadores: Vec<(f64, String)>,
}

fn mezcla_de(t: &Nodo) -> (f32, f32, bool, bool) {
    let c = t.hijo("Channel");
    let vol = c.and_then(|c| c.hijo("Volume")).and_then(|v| v.num("value")).unwrap_or(1.0);
    let pan = c.and_then(|c| c.hijo("Pan")).and_then(|v| v.num("value")).unwrap_or(0.5);
    let mudo = c.and_then(|c| c.hijo("Mute")).and_then(|v| v.attr("value")) == Some("true");
    let solo = c.and_then(|c| c.attr("solo")) == Some("true");
    let db = if vol > 1e-6 { (20.0 * vol.log10()).clamp(-60.0, 24.0) } else { -60.0 };
    (db as f32, (pan * 2.0 - 1.0).clamp(-1.0, 1.0) as f32, mudo, solo)
}

pub fn leer_proyecto(xml: &str) -> Result<Proyecto> {
    let raiz = leer_xml(xml)?;
    if raiz.nombre != "Project" {
        anyhow::bail!("{}", crate::i18n::t("no es un project.xml de DAWproject", "not a DAWproject project.xml"));
    }
    let tr = raiz.hijo("Transport");
    let bpm = tr.and_then(|t| t.hijo("Tempo")).and_then(|t| t.num("value")).unwrap_or(120.0);
    let compas = tr.and_then(|t| t.hijo("TimeSignature")).and_then(|t| t.num("numerator")).unwrap_or(4.0) as u8;
    let mut p = Proyecto { bpm, compas, ..Default::default() };
    // los clips de cada pista, por su id
    let mut por_pista: std::collections::HashMap<String, Vec<ClipAudio>> = Default::default();
    if let Some(arr) = raiz.hijo("Arrangement") {
        if let Some(l) = arr.hijo("Lanes") {
            let unidad = l.attr("timeUnit").unwrap_or("beats");
            fn recorrer(n: &Nodo, unidad: &str, bpm: f64, m: &mut std::collections::HashMap<String, Vec<ClipAudio>>) {
                for h in n.hijos_de("Lanes") {
                    let u = h.attr("timeUnit").unwrap_or(unidad);
                    if let Some(t) = h.attr("track") {
                        let mut v = Vec::new();
                        clips_de(h, u, bpm, 0.0, &mut v);
                        m.entry(t.to_string()).or_default().extend(v);
                    }
                    recorrer(h, u, bpm, m);
                }
            }
            recorrer(l, unidad, bpm, &mut por_pista);
        }
        if let Some(m) = arr.hijo("Markers") {
            let u = m.attr("timeUnit").unwrap_or("beats");
            for mk in m.hijos_de("Marker") {
                p.marcadores.push((a_segundos(mk.num("time").unwrap_or(0.0), u, bpm), mk.attr("name").unwrap_or("").to_string()));
            }
        }
    }
    fn pistas(n: &Nodo, carpeta: Option<usize>, p: &mut Proyecto, clips: &std::collections::HashMap<String, Vec<ClipAudio>>) {
        for t in n.hijos_de("Track") {
            let rol = t.hijo("Channel").and_then(|c| c.attr("role")).unwrap_or("regular");
            if rol == "master" {
                continue;
            }
            let nombre = t.attr("name").unwrap_or("pista").to_string();
            let (db, pan, mudo, solo) = mezcla_de(t);
            if t.hijos_de("Track").next().is_some() {
                p.carpetas.push((nombre, db, pan, mudo, solo));
                let k = p.carpetas.len() - 1;
                pistas(t, Some(k), p, clips);
            } else if let Some(c) = t.attr("id").and_then(|id| clips.get(id)).filter(|c| !c.is_empty()) {
                p.pistas.push((nombre, db, pan, mudo, solo, carpeta, c.clone()));
            }
        }
    }
    if let Some(s) = raiz.hijo("Structure") {
        pistas(s, None, &mut p, &por_pista);
    }
    Ok(p)
}

/// Trae un `.dawproject` a la sesión: cada pista de audio, aplanada a un WAV con sus clips en su sitio, con
/// su mezclador; las carpetas como grupos; tempo y marcadores. Devuelve los números de las pistas nuevas.
pub fn importar_dawproject(root: &Path, s: &mut Sesion, ruta: &Path) -> Result<Vec<u32>> {
    let fs = s.frecuencia;
    let mut z = zip::ZipArchive::new(std::fs::File::open(ruta).with_context(|| ruta.display().to_string())?).map_err(|e| gotv_engine::anyhow_tr!("{} no es un .dawproject (zip): {e}", "{} is not a .dawproject (zip): {e}", ruta.display()))?;
    let mut xml = String::new();
    z.by_name("project.xml").map_err(|_| gotv_engine::anyhow_tr!("el .dawproject no trae project.xml", "the .dawproject has no project.xml"))?.read_to_string(&mut xml)?;
    let pr = leer_proyecto(&xml)?;
    let base = sano(&ruta.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
    let dir = root.join(".gotv/experto/audio").join(format!("dawproject-{base}"));
    std::fs::create_dir_all(&dir)?;
    // cada archivo de audio, una vez: sacado del zip y convertido a la frecuencia de la sesión
    let mut cache: std::collections::HashMap<String, Estereo> = Default::default();
    let mut audio_de = |archivo: &str, s: &Sesion| -> Result<Estereo> {
        if let Some(x) = cache.get(archivo) {
            return Ok(x.clone());
        }
        let destino = dir.join(sano(archivo.rsplit('/').next().unwrap_or(archivo)));
        {
            let mut f = z.by_name(archivo).map_err(|_| gotv_engine::anyhow_tr!("falta {archivo} dentro del .dawproject", "{archivo} is missing inside the .dawproject"))?;
            let mut b = Vec::new();
            f.read_to_end(&mut b)?;
            std::fs::write(&destino, b)?;
        }
        let rel = crate::experto::importar(root, s, &destino)?;
        let x = dsp::leer_estereo(&root.join(rel), fs)?;
        cache.insert(archivo.to_string(), x.clone());
        Ok(x)
    };
    let mut grupos = Vec::new();
    for (nombre, db, pan, mudo, solo) in &pr.carpetas {
        let op = s.numerar(Op::CrearGrupo { grupo: Grupo { insertos: Default::default(), id: 0, nombre: nombre.clone(), ganancia_db: *db, pan: *pan, silencio: *mudo, solo: *solo } });
        if let Op::CrearGrupo { grupo } = &op {
            grupos.push(grupo.id);
        }
        s.aplicar(op)?;
    }
    let mut nuevas = Vec::new();
    for (k, (nombre, db, pan, mudo, solo, carpeta, clips)) in pr.pistas.iter().enumerate() {
        let mut partes: Vec<Estereo> = Vec::new();
        for c in clips {
            let x = audio_de(&c.archivo, s)?;
            let desde = (c.desde * fs as f64).round() as usize;
            let hasta = c.duracion.map(|d| desde + (d * fs as f64).round() as usize).unwrap_or(x[0].len()).min(x[0].len());
            let ini = (c.inicio * fs as f64).round() as usize;
            let parte: Estereo = std::array::from_fn(|ch| {
                let mut v = vec![0f32; ini];
                v.extend_from_slice(&x[ch][desde.min(hasta)..hasta]);
                v
            });
            partes.push(parte);
        }
        let plano = dsp::sumar(&partes);
        let archivo = format!(".gotv/experto/audio/dawproject-{base}/{:02}-{}.wav", k + 1, sano(nombre));
        dsp::escribir_wav(&root.join(&archivo), &plano, fs, false)?;
        let op = s.numerar(Op::AgregarPista {
            pista: Pista { color: None, id: 0, nombre: nombre.clone(), archivo, origen: Origen::Importada, de: None, fuente: None, ganancia_db: *db, pan: *pan, silencio: *mudo, solo: *solo, grupo: carpeta.map(|c| grupos[c]), insertos: Default::default() },
        });
        if let Op::AgregarPista { pista } = &op {
            nuevas.push(pista.id);
        }
        s.aplicar(op)?;
    }
    if (20.0..=400.0).contains(&pr.bpm) {
        s.aplicar(Op::Tempo { bpm: pr.bpm as f32, compas: pr.compas.clamp(1, 16) })?;
    }
    for (t, nombre) in pr.marcadores {
        s.aplicar(Op::Marcador { marcador: Marcador { segundo: t.max(0.0), nombre } })?;
    }
    Ok(nuevas)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::experto::{Efecto, Region};

    fn seno(f: f64, a: f32, s: f64, fs: u32) -> Vec<f32> {
        (0..(s * fs as f64) as usize).map(|n| a * (2.0 * std::f64::consts::PI * f * n as f64 / fs as f64).sin() as f32).collect()
    }

    /// Una sesión con 3 pistas (dos en un grupo), una región, mezcla, tempo y un marcador.
    fn sesion(root: &Path) -> Sesion {
        let fs = 48_000;
        let mut s = Sesion::default();
        for (k, f) in [110.0, 220.0, 330.0].iter().enumerate() {
            let x: Estereo = [seno(*f, 0.2, 2.0, fs), seno(*f * 1.01, 0.2, 2.0, fs)];
            dsp::escribir_wav(&root.join(format!("p{k}.wav")), &x, fs, false).unwrap();
            let p = Pista { color: None, id: k as u32 + 1, nombre: ["Batería", "Bajo & sub", "Voz \"lead\""][k].into(), archivo: format!("p{k}.wav"), origen: Origen::Separada, de: None, fuente: None, ganancia_db: [0.0, -3.0, 2.0][k], pan: [0.0, -0.5, 0.3][k], silencio: false, solo: false, grupo: None, insertos: Default::default() };
            s.aplicar(Op::AgregarPista { pista: p }).unwrap();
        }
        s.aplicar(Op::CrearGrupo { grupo: Grupo { insertos: Default::default(), id: 10, nombre: "Ritmo".into(), ganancia_db: -1.5, pan: 0.0, silencio: false, solo: false } }).unwrap();
        s.aplicar(Op::Agrupar { pistas: vec![1, 2], grupo: Some(10) }).unwrap();
        s.aplicar(Op::Region { region: Region { apagada: false, id: 11, pista: 1, desde: 0.5, hasta: 1.0, efecto: Efecto::Ganancia { db: 4.0, fundido: 0.03 } } }).unwrap();
        s.aplicar(Op::Tempo { bpm: 96.0, compas: 3 }).unwrap();
        s.aplicar(Op::Marcador { marcador: Marcador { segundo: 1.25, nombre: "Estribillo".into() } }).unwrap();
        s
    }

    /// Exportar: mezcla de 24 bits, las pistas finales SUMAN la mezcla, y el .dawproject trae su XML y sus audios.
    #[test]
    fn exportar_deja_mezcla_pistas_y_dawproject() {
        let d = crate::CarpetaPrueba::sin_crear("gotv-export");
        std::fs::create_dir_all(&*d).unwrap();
        let s = sesion(&d);
        let x = exportar(&d, &s, &d.join("salida"), &Opciones { nombre: "Mi canción".into(), normalizar_lufs: None, ..Default::default() }).unwrap();
        assert_eq!(x.pistas.len(), 3);
        let m = dsp::leer_estereo(&x.mezcla, 48_000).unwrap();
        let suma = dsp::sumar(&x.pistas.iter().map(|p| dsp::leer_estereo(p, 48_000).unwrap()).collect::<Vec<_>>());
        let err = (0..2).flat_map(|c| m[c].iter().zip(&suma[c]).map(|(a, b)| (a - b).abs())).fold(0f32, f32::max);
        assert!(err < 5e-6, "las pistas (24 bits) suman la mezcla: {err}");
        let mut z = zip::ZipArchive::new(std::fs::File::open(x.dawproject.as_ref().unwrap()).unwrap()).unwrap();
        let mut xml = String::new();
        z.by_name("project.xml").unwrap().read_to_string(&mut xml).unwrap();
        assert!(xml.contains(r#"role="submix""#) && xml.contains("Bajo &amp; sub") && xml.contains("&quot;lead&quot;"));
        assert!(z.by_name("metadata.xml").is_ok() && z.by_name("audio/01-Batería.wav").is_ok());
        // normalizada a −14 LUFS (o hasta el tope de pico)
        let y = exportar(&d, &s, &d.join("salida2"), &Opciones { nombre: "n".into(), normalizar_lufs: Some(-14.0), ..Default::default() }).unwrap();
        assert!((y.lufs.unwrap() + 14.0 + y.falto_db).abs() < 0.05, "{:?} falto {}", y.lufs, y.falto_db);
    }

    /// Ida y vuelta: exportar e importar en otro proyecto da la MISMA sesión (nombres, grupo, mezclador,
    /// tempo, marcador) y el mismo audio antes del fader (con la región ya aplicada).
    #[test]
    fn ida_y_vuelta_da_la_misma_sesion() {
        let d = crate::CarpetaPrueba::sin_crear("gotv-export-ida");
        let otra = crate::CarpetaPrueba::sin_crear("gotv-export-vuelta");
        std::fs::create_dir_all(&*d).unwrap();
        std::fs::create_dir_all(&*otra).unwrap();
        let s = sesion(&d);
        let x = exportar(&d, &s, &d.join("salida"), &Opciones { nombre: "ida".into(), normalizar_lufs: None, ..Default::default() }).unwrap();
        let mut s2 = Sesion::default();
        let nuevas = importar_dawproject(&otra, &mut s2, x.dawproject.as_ref().unwrap()).unwrap();
        assert_eq!(nuevas.len(), 3);
        let (e1, e2) = (s.estado(), s2.estado());
        assert_eq!((e2.bpm, e2.compas), (96.0, 3));
        assert_eq!(e2.marcadores, e1.marcadores);
        assert_eq!(e2.grupos.len(), 1);
        assert_eq!(e2.grupos[0].nombre, "Ritmo");
        assert!((e2.grupos[0].ganancia_db + 1.5).abs() < 1e-5, "{}", e2.grupos[0].ganancia_db);
        for (a, b) in e1.pistas.iter().zip(&e2.pistas) {
            assert_eq!(a.nombre, b.nombre);
            assert!((a.ganancia_db - b.ganancia_db).abs() < 1e-4 && (a.pan - b.pan).abs() < 1e-6, "{a:?} {b:?}");
            assert_eq!(a.grupo.is_some(), b.grupo.is_some());
            let (xa, xb) = (dsp::render_pista(&e1, a, 48_000, &d).unwrap(), dsp::render_pista(&e2, b, 48_000, &otra).unwrap());
            assert_eq!(xa, xb, "mismo audio antes del fader: {}", a.nombre);
        }
        // y la mezcla de la sesión importada es la misma
        let (m1, _) = dsp::mezclar(&e1, 48_000, &d).unwrap();
        let (m2, _) = dsp::mezclar(&e2, 48_000, &otra).unwrap();
        let err = (0..2).flat_map(|c| m1[c].iter().zip(&m2[c]).map(|(a, b)| (a - b).abs())).fold(0f32, f32::max);
        assert!(err < 1e-5, "{err}");
    }

    /// El ejemplo de Bitwig del README de DAWproject: tiempos en BEATS y clips anidados con Warps.
    #[test]
    fn lee_el_ejemplo_de_bitwig() {
        let xml = r##"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Project version="1.0"><Application name="Bitwig Studio" version="5.0"/>
<Transport><Tempo max="666.000000" min="20.000000" unit="bpm" value="120.000000" id="id0" name="Tempo"/><TimeSignature denominator="4" numerator="4" id="id1"/></Transport>
<Structure>
<Track contentType="audio" loaded="true" id="id9" name="Drumloop" color="#b53bba"><Channel audioChannels="2" destination="id15" role="regular" solo="false" id="id10"><Mute value="false" id="id13" name="Mute"/><Pan max="1.000000" min="0.000000" unit="normalized" value="0.500000" id="id12" name="Pan"/><Volume max="2.000000" min="0.000000" unit="linear" value="0.177125" id="id11" name="Volume"/></Channel></Track>
<Track contentType="audio notes" loaded="true" id="id14" name="Master"><Channel audioChannels="2" role="master" solo="false" id="id15"/></Track>
</Structure>
<Arrangement id="id19"><Lanes timeUnit="beats" id="id20"><Lanes track="id9" id="id24"><Clips id="id25">
<Clip time="4.0" duration="8.0" playStart="0.0" name="Drumfunk3 170bpm"><Clips id="id26"><Clip time="0.0" duration="8.0" contentTimeUnit="beats" playStart="0.0"><Warps contentTimeUnit="seconds" timeUnit="beats" id="id28"><Audio algorithm="stretch" channels="2" duration="2.82" sampleRate="48000" id="id27"><File path="audio/Drumfunk3 170bpm.wav"/></Audio><Warp time="0.0" contentTime="0.0"/></Warps></Clip></Clips></Clip>
</Clips></Lanes></Lanes></Arrangement></Project>"##;
        let p = leer_proyecto(xml).unwrap();
        assert_eq!(p.pistas.len(), 1, "el maestro no es pista");
        let (nombre, db, pan, _, _, carpeta, clips) = &p.pistas[0];
        assert_eq!((nombre.as_str(), *pan, *carpeta), ("Drumloop", 0.0, None));
        assert!((*db as f64 - 20.0 * 0.177125f64.log10()).abs() < 1e-4);
        // 4 beats a 120 bpm = 2 s
        assert_eq!(clips, &vec![ClipAudio { inicio: 2.0, desde: 0.0, duracion: Some(4.0), archivo: "audio/Drumfunk3 170bpm.wav".into() }]);
        assert!(leer_xml("<a><b></a>").is_err());
        assert_eq!(leer_xml(r#"<a x="1 &amp; 2 &#65;"/>"#).unwrap().attr("x"), Some("1 & 2 A"));
    }

    /// Exporta la sesión de un proyecto REAL (`GOTV_EXPERTO_PROYECTO`) a `GOTV_EXPERTO_SALIDA` (para oírla y medirla).
    #[test]
    #[ignore]
    fn exportar_un_proyecto_real() {
        let (Some(p), Some(o)) = (std::env::var_os("GOTV_EXPERTO_PROYECTO"), std::env::var_os("GOTV_EXPERTO_SALIDA")) else { return };
        let root = PathBuf::from(p);
        let x = exportar(&root, &crate::experto::cargar(&root), &PathBuf::from(o), &Opciones { nombre: "experto".into(), normalizar_lufs: None, ..Default::default() }).unwrap();
        eprintln!("{x:#?}");
    }

    /// JUEZ: el project.xml exportado valida contra el XSD OFICIAL de `bitwig/dawproject` con xmllint
    /// (`GOTV_DAWPROJECT_XSD` = la carpeta con Project.xsd y MetaData.xsd).
    #[test]
    #[ignore]
    fn juez_valida_contra_el_xsd_oficial() {
        let Some(xsd) = std::env::var_os("GOTV_DAWPROJECT_XSD").map(PathBuf::from) else { return };
        let d = crate::CarpetaPrueba::sin_crear("gotv-export-xsd");
        std::fs::create_dir_all(&*d).unwrap();
        let s = sesion(&d);
        let x = exportar(&d, &s, &d.join("salida"), &Opciones { nombre: "xsd".into(), normalizar_lufs: None, ..Default::default() }).unwrap();
        let mut z = zip::ZipArchive::new(std::fs::File::open(x.dawproject.as_ref().unwrap()).unwrap()).unwrap();
        for (dentro, esquema) in [("project.xml", "Project.xsd"), ("metadata.xml", "MetaData.xsd")] {
            let mut t = String::new();
            z.by_name(dentro).unwrap().read_to_string(&mut t).unwrap();
            let f = d.join(dentro);
            std::fs::write(&f, t).unwrap();
            let o = std::process::Command::new("xmllint").args(["--noout", "--schema"]).arg(xsd.join(esquema)).arg(&f).output().unwrap();
            let msg = String::from_utf8_lossy(&o.stderr).to_string();
            eprintln!("{dentro}: {msg}");
            assert!(o.status.success(), "{dentro} no valida: {msg}");
        }
    }
}
