//! Ola EXPERTO II · X2-6 — la IA DENTRO del estudio. Lo generativo entra a la sesión como pistas y clips
//! normales (se mueven, se mezclan y se deshacen como cualquier otro), y siempre con el contexto de la
//! sesión: su tempo, su tonalidad, su compás y su letra.
//!
//! - `generar_pista`: una pista nueva desde un texto (ACE-Step), al tempo y en la tonalidad de la sesión.
//! - `extender`: la canción (o una pista) sigue `segundos` más: se repinta lo que hay después del final.
//! - `versionar`: otra versión de una pista (cover) con otro estilo; la original queda en silencio.
//! - `transcribir`: la letra de una pista o de la mezcla (Whisper), por trozos cortados en los silencios.
//! - `masterizar`: el limitador del maestro empujado justo hasta la sonoridad pedida.
//! - `analizar`: tempo, tonalidad, sonoridad y picos de una pista o de la mezcla.
//!
//! `rehacer_tramo` (EXP-4) y `separar` (EXP-1) viven en `experto`.
use crate::experto::{Clip, Estado, Insertos, Limitador, Op, Origen, Pista, Sesion};
use crate::experto_analisis::Analisis;
use crate::experto_dsp::{self as dsp, Estereo};
use anyhow::Result;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

type Avance<'a> = &'a (dyn Fn(String) + Sync);

fn ahora_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

fn pista_de(e: &Estado, id: u32) -> Result<Pista> {
    e.pistas.iter().find(|p| p.id == id).cloned().ok_or_else(|| gotv_engine::anyhow_tr!("esa pista no existe", "that track does not exist"))
}

/// El compás de la sesión como lo pide ACE-Step («2», «3», «4» o «6»); otro, sin fijar.
fn compas_de(e: &Estado) -> String {
    if [2, 3, 4, 6].contains(&e.compas) {
        e.compas.to_string()
    } else {
        String::new()
    }
}

/// Lo que se le pide al generar una pista nueva.
#[derive(serde::Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct PedidoPista {
    /// estilo, instrumentos, ambiente (en inglés rinde mejor: así se entrenó)
    pub descripcion: String,
    /// vacía = instrumental
    pub letra: String,
    pub idioma: String,
    /// 0 = lo que dure la sesión (30 s si está vacía)
    pub segundos: f64,
    /// usar el tempo de la sesión
    pub con_tempo: bool,
    /// usar la tonalidad de la sesión
    pub con_tonalidad: bool,
    /// 0 = al azar
    pub semilla: u64,
    pub nombre: String,
    /// dónde colocarla en la línea de tiempo (s)
    pub inicio: f64,
    /// las opciones del motor (modelo, planificador, pasos…): las mismas del Pro de Música
    pub opciones: serde_json::Value,
}

/// El audio de la sesión que sirve de CONTEXTO: una pista (con sus clips y regiones, sin sus insertos) o,
/// sin pista, la mezcla entera.
fn contexto(root: &Path, e: &Estado, pista: Option<u32>, fs: u32) -> Result<Estereo> {
    match pista {
        Some(id) => dsp::pista_con_regiones(e, &pista_de(e, id)?, fs, root),
        None => {
            if e.pistas.is_empty() {
                anyhow::bail!("{}", crate::i18n::t("la sesión no tiene pistas", "the session has no tracks"));
            }
            Ok(dsp::mezclar_todo(e, fs, root)?.mezcla)
        }
    }
}

/// Lo que dura lo que suena (la pista más larga), en segundos.
pub fn duracion(root: &Path, e: &Estado, _fs: u32) -> f64 {
    e.clips
        .iter()
        .map(|c| c.inicio + c.duracion.unwrap_or_else(|| (dsp::duracion_wav(&root.join(&c.archivo)).unwrap_or(0.0) - c.desde).max(0.0) / c.ritmo as f64))
        .fold(0.0, f64::max)
        .max(0.0)
}

/// Genera una pista nueva y la agrega a la sesión. Devuelve su número.
pub fn generar_pista(root: &Path, s: &mut Sesion, p: &PedidoPista, progreso: Avance, cancelar: &AtomicBool) -> Result<u32> {
    if p.descripcion.trim().is_empty() {
        anyhow::bail!("{}", crate::i18n::t("di qué música quieres (estilo, instrumentos, ambiente)", "say what music you want (style, instruments, mood)"));
    }
    let e = s.estado();
    let ms = ahora_ms();
    let total = duracion(root, &e, s.frecuencia);
    let segundos = if p.segundos > 0.0 {
        p.segundos
    } else if total > 1.0 {
        (total - p.inicio.max(0.0)).max(5.0)
    } else {
        30.0
    };
    let salida = root.join(format!(".gotv/experto/audio/generada-{ms}.wav"));
    let pedido = crate::generadores::PedidoMusica {
        descripcion: p.descripcion.trim().to_string(),
        letra: p.letra.trim().to_string(),
        idioma: p.idioma.trim().to_string(),
        segundos: segundos as f32,
        bpm: p.con_tempo.then(|| e.bpm.round() as u32).filter(|b| (30..=300).contains(b)),
        tonalidad: if p.con_tonalidad { e.tonalidad.clone() } else { String::new() },
        compas: if p.con_tempo { compas_de(&e) } else { String::new() },
        semilla: if p.semilla == 0 { ms % 1_000_000_007 } else { p.semilla },
        opciones: p.opciones.clone(),
        ..Default::default()
    };
    crate::generadores::generar_musica(&pedido, &salida, progreso, cancelar)?;
    let nombre = Some(p.nombre.trim()).filter(|n| !n.is_empty()).map(String::from).unwrap_or_else(|| {
        let d: String = p.descripcion.trim().chars().take(28).collect();
        format!("✦ {d}")
    });
    let op = crate::experto::op_importar(root, s, &salida, Some(&nombre), Origen::Generada)?;
    let id = match &op {
        Op::AgregarPista { pista } => pista.id,
        _ => 0,
    };
    s.aplicar(op)?;
    if p.inicio > 0.0 {
        if let Some(c) = s.estado().clips_de(id).next().map(|c| c.id) {
            s.aplicar(Op::MoverClip { clip: c, inicio: p.inicio, pista: None })?;
        }
    }
    Ok(id)
}

/// La canción sigue `segundos` más. Se le da al modelo lo que hay y se REPINTA desde el final: lo nuevo
/// entra como un clip que arranca 250 ms antes del final y cruza con lo anterior. Con `pista`, el clip
/// va en esa pista; sin ella, lo que se extiende es la mezcla y va en una pista nueva. Devuelve la pista.
#[allow(clippy::too_many_arguments)]
pub fn extender(root: &Path, s: &mut Sesion, pista: Option<u32>, segundos: f64, descripcion: &str, letra: &str, idioma: &str, opciones: &serde_json::Value, progreso: Avance, cancelar: &AtomicBool) -> Result<u32> {
    const CRUCE: f64 = 0.25;
    if !(segundos.is_finite() && (2.0..=240.0).contains(&segundos)) {
        anyhow::bail!("{}", crate::i18n::t("se extiende entre 2 y 240 segundos", "extending goes from 2 to 240 seconds"));
    }
    if descripcion.trim().is_empty() {
        anyhow::bail!("{}", crate::i18n::t("di cómo sigue la música (estilo, instrumentos, ambiente)", "say how the music goes on (style, instruments, mood)"));
    }
    let e = s.estado();
    let fs = s.frecuencia;
    let x = contexto(root, &e, pista, fs)?;
    let dur = x[0].len() as f64 / fs as f64;
    if dur < 1.0 {
        anyhow::bail!("{}", crate::i18n::t("no hay audio que extender", "there is no audio to extend"));
    }
    let ms = ahora_ms();
    let tmp = root.join(".gotv/experto/tmp");
    std::fs::create_dir_all(&tmp)?;
    let (entrada, salida) = (tmp.join(format!("extender-{ms}.wav")), tmp.join(format!("extendido-{ms}.wav")));
    dsp::escribir_wav(&entrada, &x, fs, false)?;
    let pedido = crate::generadores::PedidoMusica {
        descripcion: descripcion.trim().to_string(),
        letra: letra.trim().to_string(),
        idioma: idioma.trim().to_string(),
        segundos: (dur + segundos) as f32,
        bpm: Some(e.bpm.round() as u32).filter(|b| (30..=300).contains(b)),
        tonalidad: e.tonalidad.clone(),
        compas: compas_de(&e),
        semilla: ms % 1_000_000_007,
        fuente: Some(entrada.clone()),
        repintar: Some((dur as f32, (dur + segundos) as f32)),
        fuerza_cover: 1.0,
        opciones: opciones.clone(),
    };
    let r = crate::generadores::generar_musica(&pedido, &salida, progreso, cancelar).and_then(|_| dsp::leer_estereo(&salida, fs));
    let _ = std::fs::remove_file(&entrada);
    let y = match r {
        Ok(y) => y,
        Err(e) => {
            let _ = std::fs::remove_file(&salida);
            return Err(e);
        }
    };
    let _ = std::fs::remove_file(&salida);
    let _ = std::fs::remove_file(salida.with_extension("wav.licencia.txt"));
    let a = (((dur - CRUCE).max(0.0)) * fs as f64) as usize;
    if y[0].len() <= a + fs as usize {
        anyhow::bail!("{}", crate::i18n::t("el modelo no devolvió audio nuevo después del final", "the model returned no new audio after the end"));
    }
    let cola: Estereo = [y[0][a..].to_vec(), y[1][a..].to_vec()];
    let archivo = format!(".gotv/experto/audio/extension-{ms}.wav");
    dsp::escribir_wav(&root.join(&archivo), &cola, fs, false)?;
    let largo = cola[0].len() as f64 / fs as f64;
    let inicio = a as f64 / fs as f64;
    let destino = match pista {
        Some(id) => id,
        None => {
            // una pista nueva sólo para la extensión: nace con su clip entero en 0 y se lleva a su sitio
            let op = s.numerar(Op::AgregarPista {
                pista: Pista { color: None, id: 0, nombre: crate::i18n::t("Extensión", "Extension").to_string(), archivo: archivo.clone(), origen: Origen::Generada, de: None, fuente: None, ganancia_db: 0.0, pan: 0.0, silencio: false, solo: false, grupo: None, insertos: Default::default() },
            });
            let id = match &op {
                Op::AgregarPista { pista } => pista.id,
                _ => 0,
            };
            s.aplicar(op)?;
            let c = s.estado().clips_de(id).next().map(|c| c.id).unwrap_or(0);
            s.aplicar(Op::MoverClip { clip: c, inicio, pista: None })?;
            s.aplicar(Op::ClipAjustes { clip: c, ganancia_db: None, fundido_entrada: Some(CRUCE as f32), fundido_salida: None, tono: None, ritmo: None, invertido: None, silencio: None, nombre: None })?;
            return Ok(id);
        }
    };
    let mut clip = Clip::entero(0, destino, &archivo, Some(largo));
    clip.inicio = inicio;
    clip.fundido_entrada = CRUCE as f32;
    clip.nombre = crate::i18n::t("Extensión", "Extension").to_string();
    s.aplicar(Op::AgregarClip { clip })?;
    Ok(destino)
}

/// Otra VERSIÓN de una pista (cover): el modelo sigue su melodía y su estructura con el estilo pedido.
/// `fuerza` (0…1): cuánto manda la pista original (1 = muy pegada; 0,3 = sólo la idea). Entra como pista
/// nueva al lado de la original, que queda en silencio (se comparan con un clic). Devuelve la nueva.
#[allow(clippy::too_many_arguments)]
pub fn versionar(root: &Path, s: &mut Sesion, pista: u32, descripcion: &str, fuerza: f32, letra: &str, idioma: &str, opciones: &serde_json::Value, progreso: Avance, cancelar: &AtomicBool) -> Result<u32> {
    if descripcion.trim().is_empty() {
        anyhow::bail!("{}", crate::i18n::t("di con qué estilo la quieres", "say which style you want it in"));
    }
    if !(fuerza.is_finite() && (0.0..=1.0).contains(&fuerza)) {
        anyhow::bail!("{}", crate::i18n::t("la fidelidad va de 0 a 1", "fidelity goes from 0 to 1"));
    }
    let e = s.estado();
    let fs = s.frecuencia;
    let p = pista_de(&e, pista)?;
    let x = dsp::pista_con_regiones(&e, &p, fs, root)?;
    let dur = x[0].len() as f64 / fs as f64;
    if dur < 1.0 {
        anyhow::bail!("{}", crate::i18n::t("esa pista no tiene audio", "that track has no audio"));
    }
    let ms = ahora_ms();
    let tmp = root.join(".gotv/experto/tmp");
    std::fs::create_dir_all(&tmp)?;
    let entrada = tmp.join(format!("version-{ms}.wav"));
    dsp::escribir_wav(&entrada, &x, fs, false)?;
    let salida = root.join(format!(".gotv/experto/audio/version-{ms}.wav"));
    let pedido = crate::generadores::PedidoMusica {
        descripcion: descripcion.trim().to_string(),
        letra: if letra.trim().is_empty() { e.letra.clone() } else { letra.trim().to_string() },
        idioma: idioma.trim().to_string(),
        segundos: dur as f32,
        bpm: Some(e.bpm.round() as u32).filter(|b| (30..=300).contains(b)),
        tonalidad: e.tonalidad.clone(),
        compas: compas_de(&e),
        semilla: ms % 1_000_000_007,
        fuente: Some(entrada.clone()),
        repintar: None,
        fuerza_cover: fuerza,
        opciones: opciones.clone(),
    };
    let r = crate::generadores::generar_musica(&pedido, &salida, progreso, cancelar);
    let _ = std::fs::remove_file(&entrada);
    r?;
    let estilo: String = descripcion.trim().chars().take(24).collect();
    let op = crate::experto::op_importar(root, s, &salida, Some(&format!("{} · {estilo}", p.nombre)), Origen::Generada)?;
    let id = match &op {
        Op::AgregarPista { pista } => pista.id,
        _ => 0,
    };
    s.aplicar(op)?;
    if p.grupo.is_some() {
        s.aplicar(Op::Agrupar { pistas: vec![id], grupo: p.grupo })?;
    }
    s.aplicar(Op::Mezcla { pista: id, ganancia_db: Some(p.ganancia_db), pan: Some(p.pan), silencio: None, solo: None })?;
    s.aplicar(Op::Mezcla { pista, ganancia_db: None, pan: None, silencio: Some(true), solo: None })?;
    Ok(id)
}

/// Dónde cortar cerca de `n` (± 5 s): el punto más callado, medido en ventanas de 50 ms.
fn corte_callado(x: &Estereo, n: usize, fs: u32) -> usize {
    let v = (fs as usize / 20).max(1);
    let (a, b) = (n.saturating_sub(5 * fs as usize), (n + 5 * fs as usize).min(x[0].len().saturating_sub(v)));
    (a..b).step_by(v).min_by(|i, j| {
        let e = |k: usize| x.iter().map(|c| c[k..k + v].iter().map(|s| (*s as f64).powi(2)).sum::<f64>()).sum::<f64>();
        e(*i).total_cmp(&e(*j))
    }).map(|k| k + v / 2).unwrap_or(n)
}

/// Los trozos `[desde, hasta)` (en muestras) de a lo más `tope_s` segundos, cortados en los silencios.
pub fn trozos(x: &Estereo, fs: u32, tope_s: f64) -> Vec<(usize, usize)> {
    let (n, tope) = (x[0].len(), (tope_s * fs as f64) as usize);
    let mut v = Vec::new();
    let mut a = 0;
    while a < n {
        let b = if n - a <= tope { n } else { corte_callado(x, a + tope - 5 * fs as usize, fs).clamp(a + fs as usize, n) };
        v.push((a, b));
        a = b;
    }
    v
}

/// La LETRA de una pista (o de la mezcla, sin `pista`) con Whisper. El audio largo se parte en trozos de
/// hasta 100 s cortados en los silencios (Whisper trabaja por ventanas cortas), y se junta.
pub fn transcribir(root: &Path, s: &Sesion, pista: Option<u32>, progreso: Avance) -> Result<String> {
    let e = s.estado();
    let fs = s.frecuencia;
    let x = contexto(root, &e, pista, fs)?;
    let tmp = root.join(".gotv/experto/tmp");
    std::fs::create_dir_all(&tmp)?;
    let ms = ahora_ms();
    let partes = trozos(&x, fs, 100.0);
    let mut letra: Vec<String> = Vec::new();
    for (k, (a, b)) in partes.iter().enumerate() {
        progreso(crate::tr!("transcribiendo la letra ({} de {})", "transcribing the lyrics ({} of {})", k + 1, partes.len()));
        let wav = tmp.join(format!("letra-{ms}-{k}.wav"));
        dsp::escribir_wav(&wav, &[x[0][*a..*b].to_vec(), x[1][*a..*b].to_vec()], fs, false)?;
        let r = crate::audio::transcribe(&wav);
        let _ = std::fs::remove_file(&wav);
        match r {
            Ok(t) => letra.push(t.trim().to_string()),
            // un trozo sin voz no es un error; que falte Whisper, sí
            Err(err) if partes.len() > 1 && k > 0 && !letra.is_empty() => progreso(format!("{err}")),
            Err(err) => return Err(err),
        }
    }
    Ok(letra.join("\n"))
}

/// Lo que resultó de masterizar.
#[derive(serde::Serialize, Clone, Debug)]
pub struct Masterizado {
    pub lufs_antes: f64,
    pub lufs: f64,
    pub empuje_db: f32,
    pub ganancia_db: f32,
    /// lo que faltó para llegar (el empuje tiene tope)
    pub falto_db: f64,
}

/// Masteriza a `objetivo` LUFS con techo `techo_db`: busca (por bisección) cuánto empujar el limitador
/// del maestro para que la sonoridad integrada de la mezcla sea la pedida. Si la mezcla ya suena más
/// fuerte, se baja con la ganancia del maestro. Los demás efectos del maestro se respetan.
pub fn masterizar(root: &Path, s: &mut Sesion, objetivo: f64, techo_db: f32) -> Result<Masterizado> {
    if !((-30.0..=-6.0).contains(&objetivo) && (-12.0..=0.0).contains(&techo_db)) {
        anyhow::bail!("{}", crate::i18n::t("se masteriza entre −30 y −6 LUFS, con techo de −12 a 0 dB", "mastering goes from −30 to −6 LUFS, with a ceiling from −12 to 0 dB"));
    }
    let fs = s.frecuencia;
    let mut e = s.estado();
    let previo = e.maestro.insertos.limitador;
    e.maestro.insertos.limitador = None;
    e.maestro.ganancia_db = 0.0;
    let base = dsp::mezclar_todo(&e, fs, root)?.mezcla;
    let antes = dsp::lufs(&base, fs).ok_or_else(|| gotv_engine::anyhow_tr!("la mezcla está en silencio", "the mix is silent"))?;
    let liberacion = previo.map(|l| l.liberacion_ms).unwrap_or(120.0);
    let medir = |empuje: f32| -> f64 {
        let mut x = base.clone();
        crate::experto_fx::limitar(&mut x, &Limitador { techo_db, empuje_db: empuje, liberacion_ms: liberacion }, fs);
        dsp::lufs(&x, fs).unwrap_or(-70.0)
    };
    let (mut empuje, mut ganancia) = (0f32, 0f32);
    let mut final_ = medir(0.0);
    if final_ > objetivo {
        // ya suena más fuerte que el objetivo: se baja después del limitador
        ganancia = (objetivo - final_) as f32;
        final_ = objetivo;
    } else {
        let (mut a, mut b) = (0f32, 24f32);
        if medir(b) < objetivo {
            empuje = b;
            final_ = medir(b);
        } else {
            // 8 pasos: 24 dB / 2^8 ≈ 0,1 dB, lo que se redondea después (con 12, una canción de 3 min tardaba 7 s)
            for _ in 0..8 {
                let m = (a + b) / 2.0;
                if medir(m) < objetivo {
                    a = m;
                } else {
                    b = m;
                }
            }
            empuje = ((a + b) / 2.0 * 10.0).round() / 10.0;
            final_ = medir(empuje);
        }
    }
    let insertos = Insertos { limitador: Some(Limitador { techo_db, empuje_db: empuje, liberacion_ms: liberacion }), ..s.estado().maestro.insertos };
    s.aplicar(Op::Maestro { ganancia_db: Some(ganancia), insertos: Some(insertos) })?;
    Ok(Masterizado { lufs_antes: antes, lufs: final_, empuje_db: empuje, ganancia_db: ganancia, falto_db: (objetivo - final_).max(0.0) })
}

/// El análisis de una pista (como suena antes de su fader) o, sin `pista`, de la mezcla final.
pub fn analizar(root: &Path, s: &Sesion, pista: Option<u32>) -> Result<Analisis> {
    let e = s.estado();
    let fs = s.frecuencia;
    let ruta: PathBuf = match pista {
        Some(id) => dsp::pista_en_cache(root, &e, &pista_de(&e, id)?, fs)?,
        None => {
            if e.pistas.is_empty() {
                anyhow::bail!("{}", crate::i18n::t("la sesión no tiene pistas", "the session has no tracks"));
            }
            dsp::mezcla_en_cache(root, &e, fs)?
        }
    };
    crate::experto_analisis::analizar_archivo(&ruta, fs)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seno(f: f64, a: f32, s: f64) -> Vec<f32> {
        (0..(s * 48_000.0) as usize).map(|n| a * (2.0 * std::f64::consts::PI * f * n as f64 / 48_000.0).sin() as f32).collect()
    }

    fn sesion(d: &crate::CarpetaPrueba, x: &Estereo) -> Sesion {
        dsp::escribir_wav(&d.join("a.wav"), x, 48_000, false).unwrap();
        let mut s = Sesion::default();
        s.raiz = Some(d.to_path_buf());
        s.aplicar(Op::AgregarPista { pista: Pista { color: None, id: 1, nombre: "a".into(), archivo: "a.wav".into(), origen: Origen::Importada, de: None, fuente: None, ganancia_db: 0.0, pan: 0.0, silencio: false, solo: false, grupo: None, insertos: Default::default() } }).unwrap();
        s
    }

    /// Masterizar deja la mezcla en la sonoridad pedida (±0,2 LU) sin pasar del techo; si ya suena más
    /// fuerte, baja; y se deshace de un paso.
    #[test]
    fn masterizar_llega_a_la_sonoridad_pedida() {
        let d = crate::CarpetaPrueba::sin_crear("gotv-ia-master");
        // música «con picos»: un seno flojo con golpes
        let mut m = seno(220.0, 0.05, 12.0);
        for (i, v) in m.iter_mut().enumerate() {
            if i % 24_000 < 600 {
                *v += 0.5 * (-((i % 24_000) as f32) / 150.0).exp();
            }
        }
        let mut s = sesion(&d, &[m.clone(), m]);
        let r = masterizar(&d, &mut s, -14.0, -1.0).unwrap();
        assert!(r.lufs_antes < -20.0 && r.empuje_db > 3.0, "{r:?}");
        let e = s.estado();
        let mezcla = dsp::mezclar_todo(&e, 48_000, &d).unwrap().mezcla;
        let l = dsp::lufs(&mezcla, 48_000).unwrap();
        assert!((l + 14.0).abs() < 0.2, "quedó a {l} LUFS");
        let pico = mezcla[0].iter().fold(0f32, |a, v| a.max(v.abs()));
        assert!(pico <= 10f32.powf(-1.0 / 20.0) + 1e-6, "pico {pico}");
        assert!(s.deshacer() && s.estado().maestro == Default::default());
        // ya muy fuerte: se baja con la ganancia
        let fuerte = seno(1000.0, 0.9, 6.0);
        let d2 = crate::CarpetaPrueba::sin_crear("gotv-ia-master2");
        let mut s2 = sesion(&d2, &[fuerte.clone(), fuerte]);
        let r = masterizar(&d2, &mut s2, -16.0, -1.0).unwrap();
        assert!(r.ganancia_db < -5.0 && r.empuje_db == 0.0, "{r:?}");
        let l = dsp::lufs(&dsp::mezclar_todo(&s2.estado(), 48_000, &d2).unwrap().mezcla, 48_000).unwrap();
        assert!((l + 16.0).abs() < 0.2, "{l}");
        assert!(masterizar(&d2, &mut s2, -2.0, -1.0).is_err());
    }

    /// Los trozos para Whisper no pasan del tope, cubren todo el audio sin huecos y cortan en el silencio.
    #[test]
    fn los_trozos_cortan_en_el_silencio() {
        // 250 s de tono con un silencio de 1 s en el segundo 97
        let mut m = seno(440.0, 0.3, 250.0);
        for v in m[97 * 48_000..98 * 48_000].iter_mut() {
            *v = 0.0;
        }
        let x: Estereo = [m.clone(), m];
        let t = trozos(&x, 48_000, 100.0);
        assert_eq!(t.first().unwrap().0, 0);
        assert_eq!(t.last().unwrap().1, x[0].len());
        assert!(t.windows(2).all(|w| w[0].1 == w[1].0), "sin huecos ni solapes");
        assert!(t.iter().all(|(a, b)| (b - a) as f64 / 48_000.0 <= 100.0 + 1e-9));
        let corte = t[0].1 as f64 / 48_000.0;
        assert!((97.0..=98.0).contains(&corte), "cortó en {corte} s");
        // uno corto: un solo trozo
        assert_eq!(trozos(&[vec![0.1; 48_000], vec![0.1; 48_000]], 48_000, 100.0), vec![(0, 48_000)]);
    }

    /// El análisis de una pista y de la mezcla sale de la caché y coincide con analizar la señal.
    #[test]
    fn analizar_pista_y_mezcla() {
        let d = crate::CarpetaPrueba::sin_crear("gotv-ia-analisis");
        let m = seno(440.0, 0.25, 4.0);
        let s = sesion(&d, &[m.clone(), m.clone()]);
        let a = analizar(&d, &s, Some(1)).unwrap();
        assert!((a.pico_db - 20.0 * 0.25f64.log10()).abs() < 0.01 && (a.segundos - 4.0).abs() < 1e-6);
        assert_eq!(analizar(&d, &s, None).unwrap().lufs, a.lufs);
        assert!(analizar(&d, &s, Some(9)).is_err());
        assert!((duracion(&d, &s.estado(), 48_000) - 4.0).abs() < 1e-9);
    }

    /// MEDIDA (a mano): con un audio REAL (`GOTV_EXPERTO_AUDIO`), cuánto tardan el render de una pista con
    /// efectos, la mezcla con maestro, masterizar y el análisis, y qué dice el análisis. Para saber si la
    /// ventana responde con una canción de verdad.
    #[test]
    #[ignore]
    fn medir_un_audio_real() {
        let Some(mut audio) = std::env::var_os("GOTV_EXPERTO_AUDIO").map(PathBuf::from) else { return };
        let d = crate::CarpetaPrueba::sin_crear("gotv-ia-real");
        std::fs::create_dir_all(&*d).unwrap();
        // `GOTV_EXPERTO_REPETIR`: el audio, tantas veces seguidas (para medir con el largo de una canción entera)
        if let Some(n) = std::env::var("GOTV_EXPERTO_REPETIR").ok().and_then(|v| v.parse::<usize>().ok()).filter(|n| *n > 1) {
            let x = dsp::leer_estereo(&audio, 48_000).unwrap();
            audio = d.join("largo.wav");
            dsp::escribir_wav(&audio, &[x[0].repeat(n), x[1].repeat(n)], 48_000, false).unwrap();
        }
        let mut s = Sesion::default();
        s.raiz = Some(d.to_path_buf());
        let t = std::time::Instant::now();
        let op = crate::experto::op_importar(&d, &s, &audio, Some("real"), Origen::Importada).unwrap();
        s.aplicar(op).unwrap();
        let e = s.estado();
        println!("importar: {:.2} s · {:.1} s de audio", t.elapsed().as_secs_f64(), duracion(&d, &e, 48_000));
        let t = std::time::Instant::now();
        let x = dsp::render_pista(&e, &e.pistas[0], 48_000, &d).unwrap();
        println!("render sin efectos: {:.2} s ({} muestras)", t.elapsed().as_secs_f64(), x[0].len());
        // el canal de «voz clara»: EQ de 4 bandas, compresor, de-esser y reverb
        let b = |filtro, frecuencia: f32, db: f32, q: f32| crate::experto::Banda { filtro, frecuencia, db, q, apagada: false };
        use crate::experto::Filtro as F;
        let ins = Insertos {
            eq: vec![b(F::PasoAlto, 90.0, 0.0, 0.707), b(F::Campana, 250.0, -2.5, 1.2), b(F::Campana, 3500.0, 3.0, 1.0), b(F::Agudos, 10000.0, 2.5, 0.707)],
            compresor: Some(crate::experto::Compresor { umbral_db: -20.0, razon: 3.0, ataque_ms: 8.0, liberacion_ms: 120.0, ganancia_db: 3.0 }),
            deesser: Some(crate::experto::DeEsser { frecuencia: 6000.0, umbral_db: -32.0, reduccion_db: 6.0 }),
            reverb: Some(crate::experto::Reverb { tamano: 0.45, amortiguacion: 0.6, mezcla: 0.12 }),
            ..Default::default()
        };
        s.aplicar(Op::Insertos { pista: e.pistas[0].id, insertos: ins }).unwrap();
        let e = s.estado();
        let t = std::time::Instant::now();
        dsp::render_pista(&e, &e.pistas[0], 48_000, &d).unwrap();
        println!("render con EQ + compresor + de-esser + reverb: {:.2} s", t.elapsed().as_secs_f64());
        let t = std::time::Instant::now();
        let r = masterizar(&d, &mut s, -14.0, -1.0).unwrap();
        println!("masterizar: {:.2} s · {r:?}", t.elapsed().as_secs_f64());
        let t = std::time::Instant::now();
        let ruta = dsp::mezcla_en_cache(&d, &s.estado(), 48_000).unwrap();
        println!("mezcla final a la caché: {:.2} s · {:.1} MB", t.elapsed().as_secs_f64(), std::fs::metadata(&ruta).unwrap().len() as f64 / 1e6);
        let t = std::time::Instant::now();
        let a = analizar(&d, &s, None).unwrap();
        println!("analizar: {:.2} s · {a:?}", t.elapsed().as_secs_f64());
        // partir en tres y transponer el del medio: lo que cuesta un clip con vocoder
        let id = s.estado().pistas[0].id;
        let total = duracion(&d, &s.estado(), 48_000);
        s.aplicar(Op::Partir { pista: id, segundo: total / 3.0, clip: None }).unwrap();
        s.aplicar(Op::Partir { pista: id, segundo: total * 2.0 / 3.0, clip: None }).unwrap();
        let medio = s.estado().clips[1].id;
        s.aplicar(Op::ClipAjustes { clip: medio, ganancia_db: None, fundido_entrada: None, fundido_salida: None, tono: Some(2.0), ritmo: None, invertido: None, silencio: None, nombre: None }).unwrap();
        let e = s.estado();
        let t = std::time::Instant::now();
        dsp::montar_clips(&e, &e.pistas[0], 48_000, &d).unwrap();
        println!("montar 3 clips ({:.0} s transpuestos +2): {:.2} s", total / 3.0, t.elapsed().as_secs_f64());
    }
}
