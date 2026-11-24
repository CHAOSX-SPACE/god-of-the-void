//! M3 · la CHARLA POR VOZ de GOTV, compartida por `gotv conversar` (terminal) y el modo manos libres
//! de la app: oído (VAD, turnos, borradores, interrupción) + locutor (frase a frase mientras el modelo
//! escribe) + transcripción, todo en el núcleo (sin Python). Quien la usa pone el modelo de lenguaje:
//! abre la [`Charla`], le pasa el texto en vivo y marca el principio y el fin de cada turno.
//!
//! micrófono ─┐                         ┌─▶ locutor: frases → voz de GOTV (Qwen3-TTS) ─┐
//!            ├─ VoiceProcessingIO ─────┤                                              ├─▶ altavoz
//! altavoz ◀──┘  (resta el eco)         └─▶ oído: VAD Silero → turnos (700 ms) ───────┘
//!                                            │ fin de frase → Whisper → agente → voz frase a frase
//!                                            │ voz sostenida (≥ 256 ms) mientras GOTV piensa o
//!                                            └ habla → INTERRUMPE: calla la voz y corta el turno
//!
//! El eco se midió (`vad::tests::eco_del_altavoz`): sin cancelación, el VAD oía la voz de GOTV en el
//! 82 % de las ventanas; TODO por VoiceProcessingIO, 0–1 %, pero la voz sonaba «malísimo» (oído del
//! Portador: VPIO trata la salida como audio de llamada). Queda: voz por AudioQueue (la buena) y
//! micrófono por VPIO, que resta parte del eco (16–18 %, rachas ≤ 512 ms) → interrumpir mientras
//! suena pide 640 ms de voz sostenida. GOTV saluda sin poder ser interrumpido.

use anyhow::Result;
use gotv_core::tr;
use gotv_sonido::vad::{Evento, Turnos, Vad, VENTANA};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc as smpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Ventanas de 32 ms seguidas con voz (≥ 0,5) que interrumpen a GOTV mientras PIENSA (no suena
/// nada: no hay eco que confundir): 256 ms.
const RACHA_INTERRUMPE: usize = 8;
/// …y mientras su voz SUENA: 640 ms. La voz sale por AudioQueue (la calidad que el Portador
/// aprobó; por VoiceProcessingIO sonaba «malísimo»), así que el cancelador del micrófono sólo resta
/// PARTE del eco: medido 16–18 % de ventanas con «voz», rachas de hasta 512 ms.
const RACHA_INTERRUMPE_SONANDO: usize = 20;

/// Lo que va al locutor.
pub(crate) enum Dicho {
    /// empieza un turno: su bandera para callarlo
    Turno(gotv_mlx::Callar),
    /// texto de la respuesta, tal como llega
    Texto(String),
    /// terminó la respuesta: decir lo que quede y avisar cuando acabe de sonar
    Fin(smpsc::Sender<()>),
}

/// Lo que oye el hilo del micrófono.
#[derive(Debug)]
pub enum Oido {
    Inicio,
    /// una pausa de `BORRADOR_MS` dentro de la frase: lo dicho hasta ahí, para transcribirlo YA
    Borrador(u64, Vec<f32>),
    /// la frase cerrada (tras 700 ms de silencio) y el borrador que sigue valiendo, si no se volvió a hablar
    Frase(Vec<f32>, Option<u64>),
    Interrumpir,
}

/// Pausa dentro de una frase a partir de la cual se transcribe por adelantado lo dicho. Whisper tarda
/// ~1 s haga lo que haga (con 3 s de audio, 0,75 s: procesa siempre una ventana de 30 s), así que se
/// le adelanta el trabajo: si la frase se cierra sin volver a hablar (700 ms), el texto ya está casi.
pub const BORRADOR_MS: usize = 250;

/// Instantes de un turno, para el desglose de la latencia.
#[derive(Default)]
pub struct Hitos {
    pub token: Option<Instant>,
    pub frase: Option<Instant>,
    pub audio: Option<Instant>,
}
pub type HitosC = Arc<std::sync::Mutex<Hitos>>;

/// Parte en frases el texto que llega en vivo, para que la voz empiece con la primera y no al final.
/// Se salta los bloques de código aunque lleguen en varios trozos; lo demás lo limpia
/// `gotv_core::lectura`.
#[derive(Default)]
pub struct Cortador {
    linea: String,
    prosa: String,
    en_codigo: bool,
    codigo_dicho: bool,
    soltadas: usize,
}

impl Cortador {
    pub fn meter(&mut self, t: &str, es: bool) -> Vec<String> {
        for ch in t.chars() {
            if ch == '\n' {
                let l = std::mem::take(&mut self.linea);
                self.cerrar_linea(&l, es);
            } else {
                self.linea.push(ch);
            }
        }
        // las frases completas de la línea a medias (una respuesta suele ser UN párrafo largo)
        let lt = self.linea.trim_start();
        // (la línea del sello «🕳️ …» se descarta ENTERA al cerrarse: no se sueltan sus frases antes)
        if !self.en_codigo && !lt.starts_with('`') && !lt.starts_with('~') && !lt.starts_with('|') && !lt.starts_with('🕳') {
            if let Some(i) = cortes(&self.linea).into_iter().last() {
                let hecho: String = self.linea.drain(..i).collect();
                self.prosa.push_str(&hecho);
                self.prosa.push('\n'); // ya es fin de frase: que cuente como corte
            }
            // la PRIMERA frase también se suelta en su coma (≥ 25 caracteres) aunque la línea siga:
            // medido, una primera oración de 150 caracteres hacía esperar 5,5 s su punto
            if self.soltadas == 0 && self.prosa.trim().is_empty() {
                if let Some(i) = self.linea.match_indices(", ").map(|(i, _)| i + 1).find(|&i| self.linea[..i].trim().chars().count() >= 25) {
                    let hecho: String = self.linea.drain(..i).collect();
                    self.prosa.push_str(&hecho);
                    self.prosa.push('\n');
                }
            }
        }
        self.sacar(false, es)
    }

    pub fn terminar(&mut self, es: bool) -> Vec<String> {
        let l = std::mem::take(&mut self.linea);
        self.cerrar_linea(&l, es);
        self.sacar(true, es)
    }

    fn cerrar_linea(&mut self, l: &str, es: bool) {
        let lt = l.trim_start();
        if lt.starts_with("```") || lt.starts_with("~~~") {
            self.en_codigo = !self.en_codigo;
            if self.en_codigo && !self.codigo_dicho {
                self.prosa.push_str(if es { "\n(ahí va un bloque de código).\n" } else { "\n(here is a code block).\n" }); // juez-idioma: par (if es)
                self.codigo_dicho = true;
            }
            return;
        }
        if self.en_codigo || lt.starts_with('🕳') {
            return;
        }
        self.prosa.push_str(l);
        self.prosa.push('\n');
    }

    /// Suelta trozos de `prosa` que terminan en fin de frase o de línea: el primero del turno con poco
    /// texto (o en su coma, si es largo) para que la voz arranque ya; los demás de al menos ~60
    /// caracteres (menos llamadas a la voz).
    fn sacar(&mut self, todo: bool, es: bool) -> Vec<String> {
        let mut out = Vec::new();
        loop {
            let minimo = if self.soltadas == 0 { 12 } else { 60 };
            let mut corte = cortes(&self.prosa).into_iter().find(|&i| self.prosa[..i].chars().count() >= minimo);
            if self.soltadas == 0 {
                let coma = self.prosa.match_indices(", ").map(|(i, _)| i + 1).find(|&i| self.prosa[..i].chars().count() >= 25);
                corte = match (corte, coma) {
                    (Some(a), Some(b)) => Some(a.min(b)),
                    (a, b) => a.or(b),
                };
            }
            let Some(i) = corte else { break };
            let trozo: String = self.prosa.drain(..i).collect();
            self.soltar(&trozo, es, &mut out);
        }
        if todo && !self.prosa.trim().is_empty() {
            let trozo = std::mem::take(&mut self.prosa);
            self.soltar(&trozo, es, &mut out);
        }
        out
    }

    fn soltar(&mut self, trozo: &str, es: bool, out: &mut Vec<String>) {
        for t in gotv_core::lectura::trozos(trozo, es) {
            self.soltadas += 1;
            out.push(t);
        }
    }
}

/// Índices (en bytes, tras el separador) donde termina una frase: signo + espacio, o salto de línea.
fn cortes(s: &str) -> Vec<usize> {
    let b: Vec<(usize, char)> = s.char_indices().collect();
    let mut v = Vec::new();
    for (k, &(i, c)) in b.iter().enumerate() {
        let sig = b.get(k + 1).map(|x| x.1);
        if c == '\n' || (matches!(c, '.' | '!' | '?' | ';' | ':') && sig.is_some_and(char::is_whitespace)) {
            v.push(i + c.len_utf8());
        }
    }
    v
}

/// Voz que se junta antes de empezar a sonar desde el silencio: 0,5 s a 24 kHz.
const ARRANQUE: usize = 12_000;

/// El hilo que habla: frase a frase MIENTRAS el modelo escribe, y cada frase suena mientras se genera.
/// No se corta a mitad de frase porque la voz tiene la GPU por TURNOS (`gotv_mlx`: el modelo espera
/// entre token y token mientras la voz genera) y así va a ~21 fotogramas/s, por delante de los 12,5
/// del habla; sin el turno caía a 4,9 y se cortaba 144 veces en un turno. Entre frase y frase puede
/// quedar una pausa (el modelo escribe la siguiente): se mide aparte.
fn locutor(rx: smpsc::Receiver<Dicho>, altavoz: Arc<gotv_sonido::Altavoz>, dir: std::path::PathBuf, idioma: String, es: bool, hitos: HitosC) {
    let mut callar = gotv_mlx::Callar::default();
    let muda = std::env::var_os("GOTV_PRUEBA_MUDA").is_some();
    let perfil = std::env::var_os("GOTV_VOZ_PERFIL").is_some();
    // reloj de lo que SONARÍA (para medir los huecos entre frases aun en silencio)
    #[derive(Default)]
    struct Reloj {
        fin: Option<Instant>,
        /// silencio al EMPEZAR una frase (el modelo aún la escribía): una pausa
        pausas: f64,
        pausa_max: f64,
        /// silencio a MITAD de una frase: un corte (lo que suena mal)
        cortes: usize,
        corte_s: f64,
        frases: usize,
    }
    let mut reloj = Reloj::default();
    let decir = |frase: &str, callar: &gotv_mlx::Callar, rl: &mut Reloj| {
        if callar.pedido() {
            return;
        }
        hitos.lock().unwrap_or_else(|x| x.into_inner()).frase.get_or_insert_with(Instant::now);
        let mut primero = true;
        // si no queda nada sonando, el arranque de la frase se RETIENE hasta juntar `ARRANQUE` de voz:
        // medido, soltando el primer trozo (0,32 s) a secas el segundo llegaba tarde (la voz espera el
        // token en curso del modelo) → 3 cortes de 0,08 s a mitad de frase en una respuesta larga
        let mut retenido: Vec<f32> = Vec::new();
        let mut soltar = |w: &[f32], rl: &mut Reloj| {
            let listo = Instant::now();
            let dura = Duration::from_secs_f64(w.len() as f64 / 24000.0);
            if let Some(f) = rl.fin {
                if listo > f {
                    let h = (listo - f).as_secs_f64();
                    if primero {
                        rl.pausas += h;
                        rl.pausa_max = rl.pausa_max.max(h);
                    } else {
                        rl.cortes += 1;
                        rl.corte_s += h;
                    }
                }
            }
            primero = false;
            rl.fin = Some(rl.fin.map_or(listo, |f| f.max(listo)) + dura);
            hitos.lock().unwrap_or_else(|x| x.into_inner()).audio.get_or_insert(listo);
            if !muda {
                altavoz.empujar(w);
            }
        };
        let r = gotv_mlx::hablar(&dir, frase, &idioma, false, callar, &mut |w| {
            if callar.pedido() {
                altavoz.callar();
                return Ok(());
            }
            let sonando = rl.fin.is_some_and(|f| f > Instant::now());
            if !sonando || !retenido.is_empty() {
                retenido.extend_from_slice(w);
                if retenido.len() < ARRANQUE {
                    return Ok(());
                }
                let r = std::mem::take(&mut retenido);
                soltar(&r, rl);
            } else {
                soltar(w, rl);
            }
            Ok(())
        });
        if !retenido.is_empty() && !callar.pedido() {
            let r = std::mem::take(&mut retenido);
            soltar(&r, rl);
        }
        if callar.pedido() {
            return;
        }
        rl.frases += 1;
        match &r {
            Ok(h) if perfil => {
                let gen = (h.total - h.carga).as_secs_f64().max(1e-6);
                eprintln!("{}", tr!("🔊 {:.1} s de voz en {:.1} s ({:.1} fotogramas/s; el habla va a 12,5)", "🔊 {:.1} s of voice in {:.1} s ({:.1} frames/s; speech runs at 12.5)", h.muestras as f64 / 24000.0, gen, h.fotogramas as f64 / gen));
            }
            Ok(_) => {}
            Err(e) => eprintln!("✗ voz: {e}"),
        }
    };
    let mut corte = Cortador::default();
    for m in rx {
        match m {
            Dicho::Turno(c) => {
                callar = c;
                corte = Cortador::default();
                reloj = Reloj::default();
                *hitos.lock().unwrap_or_else(|x| x.into_inner()) = Hitos::default();
            }
            Dicho::Texto(t) => {
                for f in corte.meter(&t, es) {
                    decir(&f, &callar, &mut reloj);
                }
            }
            Dicho::Fin(listo) => {
                for f in corte.terminar(es) {
                    decir(&f, &callar, &mut reloj);
                }
                if reloj.cortes > 0 && !callar.pedido() {
                    eprintln!("{}", tr!("⚠ la voz se cortó {} veces a mitad de frase ({:.2} s)", "⚠ the voice cut out {} times mid-sentence ({:.2} s)", reloj.cortes, reloj.corte_s));
                }
                if perfil {
                    eprintln!("🔊 {} frases · pausas entre frases {:.2} s (la mayor {:.2} s) · cortes a mitad de frase {} ({:.2} s)", reloj.frases, reloj.pausas, reloj.pausa_max, reloj.cortes, reloj.corte_s);
                }
                // que acabe de sonar (o que lo callen)
                if muda {
                    if let Some(f) = reloj.fin {
                        while Instant::now() < f && !callar.pedido() {
                            std::thread::sleep(Duration::from_millis(30));
                        }
                    }
                } else {
                    altavoz.esperar_o(&|| callar.pedido());
                }
                if callar.pedido() {
                    altavoz.callar();
                }
                let _ = listo.send(());
            }
        }
    }
}

/// El hilo que oye: VAD + turnos + la regla de interrupción y el filtro del eco.
fn oido(duplex: Arc<gotv_sonido::Duplex>, altavoz: Arc<gotv_sonido::Altavoz>, tx: tokio::sync::mpsc::UnboundedSender<Oido>, activo: Arc<AtomicBool>, saludando: Arc<AtomicBool>, cerrar: Arc<AtomicBool>) {
    let Ok(mut vad) = Vad::nuevo() else { return };
    let mut turnos = Turnos::por_defecto();
    let mut racha = 0usize;
    // ¿esta frase del micrófono cuenta? (fuera de un turno de GOTV sí; dentro, sólo si interrumpió)
    let mut cuenta = true;
    let (mut borrador, mut sig_borrador): (Option<u64>, u64) = (None, 0);
    // PRUEBAS: `GOTV_CONVERSAR_WAV=<wav 16 kHz>` alimenta el oído con un archivo a ritmo real (en vez
    // del micrófono) y luego silencio: recorre VAD → Whisper → agente → voz sin que nadie hable
    let mut archivo: Option<std::collections::VecDeque<f32>> = std::env::var_os("GOTV_CONVERSAR_WAV").and_then(|f| std::fs::read(f).ok()).and_then(|b| {
        let i = b.windows(4).position(|w| w == b"data")? + 8;
        Some(b[i..].as_chunks::<2>().0.iter().map(|c| i16::from_le_bytes(*c) as f32 / 32768.0).collect())
    });
    loop {
        if cerrar.load(Ordering::Relaxed) || tx.is_closed() {
            return;
        }
        let b = match archivo.as_mut() {
            Some(_) if saludando.load(Ordering::Relaxed) => {
                let _ = duplex.leer(Duration::from_millis(100));
                continue; // el archivo «habla» cuando GOTV terminó de saludar
            }
            Some(a) => {
                // el micrófono real se lee y se descarta (marca el ritmo: un bloque cada 32 ms)
                let _ = duplex.leer(Duration::from_millis(100));
                if a.len() >= VENTANA {
                    a.drain(..VENTANA).collect()
                } else {
                    vec![0.0; VENTANA]
                }
            }
            None => match duplex.leer(Duration::from_secs(5)) {
                Some(b) => b,
                None => {
                    eprintln!("{}", gotv_core::i18n::t("✗ el micrófono no entrega audio (¿permiso de micrófono para esta terminal en Privacidad y seguridad?)", "✗ the microphone delivers no audio (microphone permission for this terminal in Privacy & Security?)"));
                    return;
                }
            },
        };
        if b.len() != VENTANA {
            continue;
        }
        let p = vad.prob(&b);
        let gotv = activo.load(Ordering::Relaxed);
        racha = if p >= 0.5 { racha + 1 } else { 0 };
        let hace_falta = if altavoz.sonando() { RACHA_INTERRUMPE_SONANDO } else { RACHA_INTERRUMPE };
        if gotv && racha >= hace_falta && !saludando.load(Ordering::Relaxed) && !cuenta {
            cuenta = true;
            let _ = tx.send(Oido::Interrumpir);
        }
        match turnos.paso(&b, p) {
            Some(Evento::Inicio(_)) => {
                cuenta = !gotv;
                borrador = None;
                if cuenta {
                    let _ = tx.send(Oido::Inicio);
                }
            }
            Some(Evento::Fin { audio, .. }) => {
                if cuenta {
                    let _ = tx.send(Oido::Frase(audio, borrador.take()));
                }
                borrador = None;
                cuenta = true;
            }
            Some(Evento::Corto(_)) => borrador = None,
            None => {}
        }
        // borrador: a los BORRADOR_MS de pausa, lo dicho hasta ahí; si se vuelve a hablar, deja de valer
        if cuenta && turnos.hablando() {
            let s = turnos.silencio_ms();
            if s == 0 {
                borrador = None;
            } else if s >= BORRADOR_MS && borrador.is_none() {
                sig_borrador += 1;
                borrador = Some(sig_borrador);
                let _ = tx.send(Oido::Borrador(sig_borrador, turnos.audio_en_curso().to_vec()));
            }
        }
    }
}

/// K5 · ¿la transcripción salió dudosa? Medido en la charla de voz del Portador (2026-11-11):
/// Whisper escribió «Śwesne! Po» y el modelo lo repitió como si fuera una orden. Dudosa = letras
/// que no son del español ni del inglés (otro alfabeto, ś, ł, ż…) o las alucinaciones conocidas
/// de Whisper sobre el silencio. Entonces se pide repetir en vez de obedecer.
pub fn transcripcion_dudosa(texto: &str) -> bool {
    const PROPIAS: &str = "áéíóúüñÁÉÍÓÚÜÑçÇàèìòùÀÈÌÒÙ"; // juez-idioma: datos (letras propias de idiomas latinos)
    if texto.chars().any(|c| c.is_alphabetic() && !c.is_ascii() && !PROPIAS.contains(c)) {
        return true;
    }
    let t = texto.to_lowercase();
    // lo que Whisper «oye» en el silencio o el ruido (aprendido de subtítulos de video)
    const ALUCINA: &[&str] = &["amara.org", "subtítulos realizados", "subtitulos realizados", "gracias por ver", "suscríbete", "suscribete", "thanks for watching", "subtitles by"]; // juez-idioma: datos (alucinaciones conocidas de Whisper, en los dos idiomas)
    ALUCINA.iter().any(|a| t.contains(a))
}

/// Whisper (nativo) sobre audio de 16 kHz.
pub fn transcribir(audio: &[f32]) -> Result<String> {
    // un nombre por transcripción: con los borradores puede haber dos a la vez
    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    let tmp = std::env::temp_dir().join(format!("gotv-conversar-{}-{n}.wav", std::process::id()));
    std::fs::write(&tmp, gotv_mlx::wav_pcm16(audio, 16000))?;
    let t = gotv_core::audio::transcribe(&tmp);
    let _ = std::fs::remove_file(&tmp);
    Ok(t?.trim().to_string())
}

/// Qué se DICE de lo que escribe el modelo (pedido del Portador: «que sólo lea el mensaje, nada de
/// comandos ni trabajos del agente»). Cada paso del modelo se retiene hasta saber qué es: si termina
/// en una llamada a herramienta, era trabajo del agente («Voy a leer el archivo…») y se descarta; si
/// pasa de dos frases o 220 caracteres, es el mensaje y empieza a sonar; al final del turno sale lo
/// que quede (la respuesta corta).
#[derive(Default)]
pub struct FiltroDeVoz {
    retenido: String,
    liberado: bool,
    en_herramienta: bool,
}

impl FiltroDeVoz {
    /// Texto que llega; devuelve lo que ya se puede decir.
    pub fn texto(&mut self, t: &str) -> Option<String> {
        if self.en_herramienta {
            // tras una herramienta empieza un paso nuevo del modelo
            self.en_herramienta = false;
            self.liberado = false;
            self.retenido.clear();
        }
        if self.liberado {
            return Some(t.to_string());
        }
        self.retenido.push_str(t);
        let frases = cortes(&self.retenido).len();
        if frases >= 2 || self.retenido.chars().count() >= 220 {
            self.liberado = true;
            return Some(std::mem::take(&mut self.retenido));
        }
        None
    }

    /// El modelo está llamando a una herramienta: lo retenido de este paso era trabajo del agente.
    pub fn herramienta(&mut self) {
        if !self.liberado {
            self.retenido.clear();
        }
        self.en_herramienta = true;
        self.liberado = false;
    }

    /// Fin del turno: lo que quedó retenido (una respuesta corta) se dice.
    pub fn terminar(&mut self) -> Option<String> {
        let r = std::mem::take(&mut self.retenido);
        let r = (!self.en_herramienta && !r.trim().is_empty()).then_some(r);
        *self = FiltroDeVoz::default();
        r
    }
}

/// Una charla por voz abierta: altavoz + micrófono con cancelación de eco + los hilos del locutor y
/// del oído. Al soltarla, el oído se apaga y el locutor termina.
pub struct Charla {
    pub altavoz: Arc<gotv_sonido::Altavoz>,
    tx: smpsc::Sender<Dicho>,
    /// GOTV tiene el turno (piensa o habla): el oído sólo atiende una interrupción
    pub activo: Arc<AtomicBool>,
    saludando: Arc<AtomicBool>,
    callar: Mutex<gotv_mlx::Callar>,
    pub hitos: HitosC,
    cerrar: Arc<AtomicBool>,
    filtro: Mutex<FiltroDeVoz>,
}

impl Charla {
    /// Abre el audio y los hilos. `voz` = la carpeta del Qwen3-TTS Base; `es` = español.
    pub fn abrir(voz: std::path::PathBuf, es: bool) -> Result<(Charla, tokio::sync::mpsc::UnboundedReceiver<Oido>)> {
        let idioma = if es { "spanish" } else { "english" }.to_string();
        // la voz sale por AudioQueue (la calidad de `gotv decir`); el micrófono entra por
        // VoiceProcessingIO (resta parte del eco) a 16 kHz
        let altavoz = Arc::new(gotv_sonido::Altavoz::abrir(24000)?);
        let duplex = Arc::new(gotv_sonido::Duplex::abrir(24000, 16000, VENTANA)?);
        let (activo, saludando, cerrar) = (Arc::new(AtomicBool::new(false)), Arc::new(AtomicBool::new(false)), Arc::new(AtomicBool::new(false)));
        let hitos: HitosC = Arc::default();
        let (tx, rx_dicho) = smpsc::channel::<Dicho>();
        {
            let (d, v, i, p) = (altavoz.clone(), voz, idioma, hitos.clone());
            std::thread::Builder::new().name("gotv-locutor".into()).spawn(move || locutor(rx_dicho, d, v, i, es, p))?;
        }
        let (tx_oido, rx_oido) = tokio::sync::mpsc::unbounded_channel::<Oido>();
        {
            let (d, al, a, s, c) = (duplex, altavoz.clone(), activo.clone(), saludando.clone(), cerrar.clone());
            std::thread::Builder::new().name("gotv-oido".into()).spawn(move || oido(d, al, tx_oido, a, s, c))?;
        }
        Ok((Charla { altavoz, tx, activo, saludando, callar: Mutex::new(gotv_mlx::Callar::default()), hitos, cerrar, filtro: Mutex::default() }, rx_oido))
    }

    /// Empieza el turno de GOTV: lo que llegue por [`Charla::texto`] se dirá frase a frase.
    pub fn empezar(&self) -> gotv_mlx::Callar {
        let c = gotv_mlx::Callar::default();
        *self.callar.lock().unwrap_or_else(|x| x.into_inner()) = c.clone();
        self.activo.store(true, Ordering::Relaxed);
        *self.filtro.lock().unwrap_or_else(|x| x.into_inner()) = FiltroDeVoz::default();
        let _ = self.tx.send(Dicho::Turno(c.clone()));
        c
    }

    /// Texto de la respuesta, tal como llega del modelo (el [`FiltroDeVoz`] decide qué se dice).
    pub fn texto(&self, t: &str) {
        if let Some(d) = self.filtro.lock().unwrap_or_else(|x| x.into_inner()).texto(t) {
            let _ = self.tx.send(Dicho::Texto(d));
        }
    }

    /// El modelo está llamando a una herramienta: lo de este paso no se dice.
    pub fn herramienta(&self) {
        self.filtro.lock().unwrap_or_else(|x| x.into_inner()).herramienta();
    }

    /// Terminó la respuesta: dice lo que quede; al acabar de sonar (o si la callan), el turno vuelve a
    /// quien habla y `listo` avisa.
    pub fn terminar(&self) -> smpsc::Receiver<()> {
        let (listo_tx, listo_rx) = smpsc::channel();
        let (tx_fuera, rx_fuera) = smpsc::channel();
        if let Some(d) = self.filtro.lock().unwrap_or_else(|x| x.into_inner()).terminar() {
            let _ = self.tx.send(Dicho::Texto(d));
        }
        let _ = self.tx.send(Dicho::Fin(listo_tx));
        let activo = self.activo.clone();
        let _ = std::thread::Builder::new().name("gotv-fin-turno".into()).spawn(move || {
            let _ = listo_rx.recv();
            activo.store(false, Ordering::Relaxed);
            let _ = tx_fuera.send(());
        });
        rx_fuera
    }

    /// Calla YA la voz del turno en curso (la interrupción).
    pub fn interrumpir(&self) {
        self.callar.lock().unwrap_or_else(|x| x.into_inner()).ya();
        self.altavoz.callar();
    }

    /// El saludo al abrir: sin poder ser interrumpido (el cancelador de eco aprende la sala mientras
    /// tanto). Vuelve cuando terminó de sonar.
    pub fn saludar(&self, texto: &str) {
        self.saludando.store(true, Ordering::Relaxed);
        self.empezar();
        self.texto(texto);
        let _ = self.terminar().recv();
        self.saludando.store(false, Ordering::Relaxed);
    }
}

impl Charla {
    /// Apaga la charla: calla la voz y el oído deja de escuchar (su canal se cierra).
    pub fn cerrar(&self) {
        self.cerrar.store(true, Ordering::Relaxed);
        self.interrumpir();
    }
}

impl Drop for Charla {
    fn drop(&mut self) {
        self.cerrar();
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn la_transcripcion_dudosa_pide_repetir() {
        // lo real que se coló en la charla del Portador, y alucinaciones típicas de Whisper
        for t in ["Śwesne! Po", "Привет", "こんにちは", "Subtítulos realizados por la comunidad de Amara.org", "¡Gracias por ver el video!", "Zażółć gęślą"] {
            assert!(transcripcion_dudosa(t), "{t}");
        }
        // lo que dijo de verdad el Portador pasa
        for t in ["Dame la hora del Mac, por favor.", "¿Qué procesador tiene esta Mac?", "Sí, hazlo con sudo.", "Entra por terminal y dime qué procesador tiene la MAC.", "pon el volumen al 30", "open the Safari app", "Françoise López, año 2026"] {
            assert!(!transcripcion_dudosa(t), "{t}");
        }
    }

    fn todo(trozos: &[&str]) -> Vec<String> {
        let mut c = Cortador::default();
        let mut v: Vec<String> = trozos.iter().flat_map(|t| c.meter(t, true)).collect();
        v.extend(c.terminar(true));
        v
    }

    #[test]
    fn el_sello_no_se_dice_aunque_llegue_por_trozos() {
        let v = todo(&["Ya quedó listo y probado en tu Mac.\n\n🕳️ Lo devorado no ", "retorna. Lo que entregué, ", "sobrevivió al Juicio."]);
        let t = v.join(" ");
        assert!(t.contains("Ya quedó listo") && !t.contains("devorado") && !t.contains("Juicio"), "{t}");
    }

    #[test]
    fn la_voz_dice_el_mensaje_y_no_el_trabajo_del_agente() {
        let mut f = FiltroDeVoz::default();
        // un paso que termina en herramienta: su «voy a…» no se dice
        assert_eq!(f.texto("Voy a revisar el archivo."), None);
        f.herramienta();
        // el paso siguiente (la respuesta) sí, en cuanto se ve que es mensaje
        assert_eq!(f.texto("Revisé main.rs y "), None);
        let d = f.texto("encontré el error. Estaba en la ruta del servidor. ").unwrap();
        assert!(d.starts_with("Revisé main.rs") && !d.contains("Voy a"), "{d}");
        assert_eq!(f.texto("Ya lo corregí.").as_deref(), Some("Ya lo corregí."), "liberado: lo demás pasa directo");
        assert_eq!(f.terminar(), None);
        // una respuesta corta sale al final del turno
        let mut f = FiltroDeVoz::default();
        assert_eq!(f.texto("¡Hola!"), None);
        assert_eq!(f.terminar().as_deref(), Some("¡Hola!"));
        // si el turno acaba en herramienta, nada
        let mut f = FiltroDeVoz::default();
        f.texto("Leo el archivo");
        f.herramienta();
        assert_eq!(f.terminar(), None);
    }

    #[test]
    fn la_primera_frase_sale_sin_esperar_el_resto() {
        let mut c = Cortador::default();
        assert!(c.meter("Claro, Porta", true).is_empty());
        let v = c.meter("dor. Ahora mismo lo reviso", true);
        assert_eq!(v, vec!["Claro, Portador.".to_string()], "la primera frase sale en cuanto termina");
        let v = c.meter(" y te cuento.", true);
        assert!(v.is_empty(), "las siguientes esperan a juntar ~60 caracteres: {v:?}");
        assert_eq!(c.terminar(true), vec!["Ahora mismo lo reviso y te cuento.".to_string()]);
    }

    #[test]
    fn una_primera_oracion_larga_se_corta_en_la_coma() {
        let mut c = Cortador::default();
        let v = c.meter("Qué bonito que tengas a Cero, un samoyedo blanco tan hermoso; y", true);
        assert_eq!(v, vec!["Qué bonito que tengas a Cero,".to_string()]);
    }

    #[test]
    fn la_coma_de_la_primera_sirve_aunque_la_linea_siga() {
        let mut c = Cortador::default();
        let v = c.meter("Los sismos ocurren cuando hay un deslizamiento repentino de bloques de roca en la corteza terrestre, causado por", true);
        assert_eq!(v, vec!["Los sismos ocurren cuando hay un deslizamiento repentino de bloques de roca en la corteza terrestre,".to_string()]);
        // la segunda ya no se corta en comas
        let v = c.meter(" el movimiento de las placas, que se atascan, y", true);
        assert!(v.is_empty(), "{v:?}");
    }

    #[test]
    fn el_codigo_partido_en_trozos_no_se_lee() {
        let v = todo(&["Mira el arreglo:\n``", "`rust\nfn main() {\n  println!(\"hola\");\n}\n``", "`\nCon eso queda listo, y la prueba pasa sin problemas en tu Mac.\n"]);
        let s = v.join(" ");
        assert!(!s.contains("println") && !s.contains("fn main"), "{s}");
        assert!(s.contains("(ahí va un bloque de código)"), "{s}");
        assert!(s.contains("Con eso queda listo"), "{s}");
    }

    #[test]
    fn no_parte_en_el_punto_de_un_archivo() {
        let v = todo(&["Edité main.rs y ", "config.toml, y ya compila."]);
        assert_eq!(v.join(" "), "Edité main.rs y config.toml, y ya compila.");
        assert!(v.iter().all(|x| !x.ends_with("main.") && !x.ends_with("config.")), "{v:?}");
    }

    #[test]
    fn varios_parrafos_salen_por_partes() {
        let p = "El sismo empieza cuando dos placas se rozan. La energía viaja en ondas.\n\nPrimero llegan las ondas P, que son rápidas. Después llegan las ondas S, que sacuden más.\n\nPor eso conviene tener un plan de salida en casa.";
        let v = todo(&[p]);
        assert!(v.len() >= 3, "{v:?}");
        assert_eq!(v[0], "El sismo empieza cuando dos placas se rozan.");
    }
}
