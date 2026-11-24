//! M3 · `gotv conversar`: hablar con GOTV por voz, todo en el núcleo (sin Python).
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
use async_trait::async_trait;
use gotv_core::{tr, AskAnswer, Config, Frontend, StdFrontend, TurnStats};
use gotv_engine::StreamEvent;
use gotv_voz::{transcribir, Charla, HitosC, Oido};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

/// El frontend de la terminal + cada trozo de texto a la voz.
struct VozFront {
    std: StdFrontend,
    charla: Arc<Charla>,
    hitos: HitosC,
    /// mientras el modelo lee la instrucción de voz (al arrancar), lo que diga no se oye
    mudo: Arc<AtomicBool>,
}

#[async_trait]
impl Frontend for VozFront {
    async fn log(&self, line: String) {
        self.std.log(line).await
    }
    async fn ask(&self, desc: &str) -> AskAnswer {
        self.std.ask(desc).await
    }
    fn delta(&self, ev: &StreamEvent) {
        self.std.delta(ev);
        if let StreamEvent::ToolArgs(_) = ev {
            self.charla.herramienta();
        }
        if let StreamEvent::Text(t) = ev {
            if self.mudo.load(Ordering::Relaxed) {
                return;
            }
            self.hitos.lock().unwrap_or_else(|x| x.into_inner()).token.get_or_insert_with(Instant::now);
            self.charla.texto(t);
        }
    }
    fn stats(&self, s: &TurnStats) {
        self.std.stats(s)
    }
    fn activity(&self, kind: &str, detail: &str) {
        self.std.activity(kind, detail)
    }
    async fn choose(&self, question: &str, options: &[String]) -> Option<String> {
        self.std.choose(question, options).await
    }
    async fn secreto(&self, etiqueta: &str) -> Option<String> {
        self.std.secreto(etiqueta).await
    }
}

/// La instrucción de voz: se lee UNA vez (queda en la caché del núcleo y en el historial).
pub fn instruccion_de_voz() -> &'static str {
    gotv_core::i18n::t(
        "[Conversación POR VOZ con el Portador: lo que digas se lee en voz alta. Responde breve y hablado, en prosa, sin markdown, sin listas ni código salvo que te lo pida; si haces algo con herramientas, cuéntalo en una frase. Para empezar, responde sólo «Listo.»]",
        "[VOICE conversation: what you write is read aloud. Answer briefly and conversationally, in prose, no markdown, lists or code unless asked; if you use tools, say what you did in one sentence. To begin, answer only «Ready.»]",
    )
}

/// `herramientas`: el agente completo (archivos, terminal…) también por voz. Sin ella, CHARLA: el
/// sistema de chat, sin herramientas ni skills. Medido 2026-11-10 con Qwen3-32B: en modo agente, una
/// anécdota contada por voz («vivo en una torre… el día del sismo») se tomó como TAREA de escritura,
/// cargó una skill y el empujón «anunció sin hacer» lo dejó dando vueltas.
pub async fn conversar(cfg: &mut Config, root: std::path::PathBuf, args: &[String]) -> Result<()> {
    let herramientas = args.iter().any(|a| a == "--herramientas" || a == "--tools");
    // `--modelo <repo>`: otro modelo SÓLO para conversar (uno menor lee y habla antes); la
    // configuración del Portador no se toca
    if let Some(m) = args.iter().position(|a| a == "--modelo" || a == "--model").and_then(|i| args.get(i + 1)) { // juez-idioma: datos (nombre de opción)
        cfg.model = m.clone();
    }
    let es = !gotv_core::i18n::is_en();
    let voz = gotv_core::generadores::voz_dir()?;
    println!("{}", tr!("🕳️  GOD OF THE VOID · conversación por voz (Ctrl-C para salir)\n", "🕳️  GOD OF THE VOID · voice conversation (Ctrl-C to quit)\n"));
    let engine = super::start_motor(cfg, false, false).await?;
    let perms = super::perms_from(cfg, &root);
    let session = gotv_core::session::Session::open_new(&root, &cfg.model).ok();
    let cancel = Arc::new(AtomicBool::new(false));
    let (charla, mut rx_oido) = Charla::abrir(voz, es)?;
    let charla = Arc::new(charla);
    let hitos = charla.hitos.clone();
    let mudo = Arc::new(AtomicBool::new(true));
    let front = VozFront { std: StdFrontend::default(), charla: charla.clone(), hitos: hitos.clone(), mudo: mudo.clone() };
    let mut agent = super::armar_agente(cfg, root.clone(), engine, front, perms, Some(gotv_core::Checkpoint::new(&root)), vec![], session, cancel.clone());
    agent.chat_only = !herramientas;
    println!("{}", if herramientas { gotv_core::i18n::t("· modo agente: puede usar herramientas en este proyecto", "· agent mode: it may use tools in this project") } else { gotv_core::i18n::t("· modo charla (sin herramientas): `--herramientas` para el agente completo", "· chat mode (no tools): `--tools` for the full agent") });

    // 1) el modelo lee la instrucción de voz UNA vez: queda en la caché del núcleo (que reutiliza el
    // prefijo común) y en el historial; cada turno sólo lee tu frase. Medido: repetirla en cada turno
    // eran ~75 tokens más por turno (≈1,2 s del 32B, que lee a 62 tok/s).
    let t = Instant::now();
    let _ = agent.run(instruccion_de_voz()).await;
    mudo.store(false, Ordering::Relaxed);
    println!();
    // 2) DESPUÉS el saludo, con la GPU para la voz sola (medido: saludando mientras el modelo leía, la
    // voz iba a 1,2–5 fotogramas/s y se entrecortaba)
    {
        let c = charla.clone();
        tokio::task::spawn_blocking(move || c.saludar(gotv_core::i18n::t("Te escucho, Portador.", "I'm listening."))).await?;
    }
    println!("{}", tr!("🔊 voz lista en {:.1} s · 🎧 te escucho: habla y haz una pausa", "🔊 voice ready in {:.1} s · 🎧 listening: speak and pause", t.elapsed().as_secs_f64()));

    let mut pendiente: Option<(Vec<f32>, Option<u64>)> = None;
    // el borrador en curso: su número y la transcripción que corre en segundo plano
    let mut en_curso: Option<(u64, tokio::task::JoinHandle<Result<String>>)> = None;
    let borrador = |en_curso: &mut Option<(u64, tokio::task::JoinHandle<Result<String>>)>, id: u64, a: Vec<f32>| {
        *en_curso = Some((id, tokio::task::spawn_blocking(move || transcribir(&a))));
    };
    loop {
        let (audio, vale) = match pendiente.take() {
            Some(x) => x,
            None => match rx_oido.recv().await {
                Some(Oido::Inicio) => {
                    println!("🎙 …");
                    continue;
                }
                Some(Oido::Borrador(id, a)) => {
                    borrador(&mut en_curso, id, a);
                    continue;
                }
                Some(Oido::Frase(a, b)) => (a, b),
                Some(Oido::Interrumpir) => continue,
                None => anyhow::bail!("{}", gotv_core::i18n::t("el micrófono se cerró", "the microphone closed")),
            },
        };
        let t = Instant::now(); // el turno se cerró (tras 0,7 s de silencio): de aquí a la 1.ª palabra
        let (texto, adelantado) = match (en_curso.take(), vale) {
            (Some((id, h)), Some(v)) if id == v => (h.await??, true),
            _ => (tokio::task::spawn_blocking(move || transcribir(&audio)).await??, false),
        };
        let whisper = t.elapsed();
        if texto.chars().filter(|c| c.is_alphanumeric()).count() < 2 {
            continue;
        }
        let adel = if adelantado { gotv_core::i18n::t(", empezado en tu pausa", ", started during your pause") } else { "" };
        println!("{}", tr!("🗣  tú: {texto}   ({:.1} s de Whisper{adel})", "🗣  you: {texto}   ({:.1} s of Whisper{adel})", t.elapsed().as_secs_f64()));

        // el turno de GOTV: piensa (texto en vivo → voz) y habla; voz sostenida lo interrumpe
        cancel.store(false, Ordering::Relaxed);
        charla.empezar();
        let mut interrumpido = false;
        let interrumpir = || {
            cancel.store(true, Ordering::Relaxed);
            charla.interrumpir();
            println!("{}", gotv_core::i18n::t("\n✋ (me callo: te escucho)", "\n✋ (stopping: listening)"));
        };
        {
            let mut corre = std::pin::pin!(agent.run(&texto));
            loop {
                tokio::select! {
                    r = &mut corre => {
                        if let Err(e) = r {
                            eprintln!("✗ {e}");
                        }
                        break;
                    }
                    ev = rx_oido.recv() => match ev {
                        Some(Oido::Interrumpir) if !interrumpido => {
                            interrumpido = true;
                            interrumpir();
                        }
                        Some(Oido::Frase(a, b)) => pendiente = Some((a, b)),
                        Some(Oido::Borrador(id, a)) => borrador(&mut en_curso, id, a),
                        _ => {}
                    },
                }
            }
        }
        println!();
        // terminar de decir la respuesta, atento a una interrupción
        let listo = charla.terminar();
        let mut espera = tokio::task::spawn_blocking(move || listo.recv());
        loop {
            tokio::select! {
                _ = &mut espera => break,
                ev = rx_oido.recv() => match ev {
                    Some(Oido::Interrumpir) if !interrumpido => {
                        interrumpido = true;
                        interrumpir();
                    }
                    Some(Oido::Frase(a, b)) => pendiente = Some((a, b)),
                    Some(Oido::Borrador(id, a)) => borrador(&mut en_curso, id, a),
                    _ => {}
                },
            }
        }
        let h = std::mem::take(&mut *hitos.lock().unwrap_or_else(|x| x.into_inner()));
        if let (Some(tok), Some(fr), Some(au)) = (h.token, h.frase, h.audio) {
            let s = |a: Instant, b: Instant| a.saturating_duration_since(b).as_secs_f64();
            println!(
                "{}",
                tr!(
                    "⏱  primera palabra {:.1} s después de tu pausa de {:.1} s = Whisper {:.1} · el modelo lee y empieza {:.1} · completa la 1.ª frase {:.1} · voz {:.1}",
                    "⏱  first word {:.1} s after your {:.1} s pause = Whisper {:.1} · model reads and starts {:.1} · completes 1st sentence {:.1} · voice {:.1}",
                    s(au, t),
                    gotv_sonido::vad::SILENCIO_MS as f64 / 1000.0,
                    whisper.as_secs_f64(),
                    s(tok, t + whisper),
                    s(fr, tok),
                    s(au, fr)
                )
            );
        }
        if interrumpido && pendiente.is_none() {
            // la frase que interrumpió llega al cerrar su pausa
            loop {
                match rx_oido.recv().await {
                    Some(Oido::Frase(a, b)) => {
                        pendiente = Some((a, b));
                        break;
                    }
                    Some(Oido::Borrador(id, a)) => borrador(&mut en_curso, id, a),
                    Some(_) => {}
                    None => break,
                }
            }
        }
    }
}
