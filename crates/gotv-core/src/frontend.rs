//! La cara del agente, abstracta: el bucle no sabe si habla con una terminal
//! cruda (CLI) o con la TUI. Ambas implementan `Frontend`.

use crate::permissions::AskAnswer;
use async_trait::async_trait;
use gotv_engine::StreamEvent;
use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};

#[async_trait]
pub trait Frontend: Send + Sync {
    /// Emite una línea de transcripción (paso, resultado, aviso).
    async fn log(&self, line: String);
    /// Pide permiso al usuario y espera su respuesta.
    async fn ask(&self, desc: &str) -> AskAnswer;
    /// Lo que llega MIENTRAS el modelo genera (streaming). Por defecto, nada.
    fn delta(&self, _ev: &StreamEvent) {}
    /// Métricas de cada respuesta del modelo (P6). Por defecto, nada.
    fn stats(&self, _s: &TurnStats) {}
    /// Actividad para paneles de tareas (GUI): `kind` = step · subagent_start ·
    /// subagent_end · compact · lsp · embedder. Por defecto, nada.
    fn activity(&self, _kind: &str, _detail: &str) {}
    /// C7: una pregunta con opciones (y respuesta libre). `None` = no hay quien responda
    /// (CLI sin terminal, pruebas): el agente decide lo más sensato y lo dice.
    async fn choose(&self, _question: &str, _options: &[String]) -> Option<String> {
        None
    }
    /// F1: una clave que el usuario teclea TAPADA (campo de contraseña en la ventana, sin eco en la
    /// terminal). Su valor no vuelve al modelo, al chat ni a un registro: quien llama la guarda en el
    /// Llavero. El rótulo lo arma GOTV, nunca el modelo. `None` = no hay quien la teclee.
    async fn secreto(&self, _etiqueta: &str) -> Option<String> {
        None
    }
    /// F10.3: el principio y el final de cada llamada a herramienta (el editor por ACP las pinta con su
    /// estado y el diff de lo editado). Por defecto, nada.
    async fn herramienta(&self, _h: Herramienta<'_>) {}
}

/// F10.3: una llamada a herramienta que empieza o termina.
#[derive(Debug, Clone, Copy)]
pub enum Herramienta<'a> {
    Empieza { id: &'a str, nombre: &'a str, args: &'a serde_json::Value },
    Termina { id: &'a str, nombre: &'a str, args: &'a serde_json::Value, salida: &'a str },
}

/// Métricas HONESTAS de una respuesta: prellenado y generación por separado
/// (medido: un "tok/s" que mezcla ambos daba 5,8 al 32b, que genera a ~10).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct TurnStats {
    /// Segundos hasta el primer token (lectura del contexto).
    pub prefill_s: f32,
    /// Tokens generados.
    pub tokens: u32,
    /// Velocidad de generación pura (sin el prellenado).
    pub gen_tok_s: f32,
    /// Contexto ocupado (estimado) y presupuesto.
    pub ctx_used: usize,
    pub ctx_budget: usize,
}

impl TurnStats {
    pub fn summary(&self) -> String {
        // contextos chicos (los modelos propios: 512) en tokens exactos; «0.0k/1k» mentía
        let ctx = if self.ctx_budget < 10_000 {
            format!("{}/{}", self.ctx_used, self.ctx_budget)
        } else {
            format!(
                "{:.1}k/{:.0}k",
                self.ctx_used as f32 / 1000.0,
                self.ctx_budget as f32 / 1000.0
            )
        };
        crate::tr!(
            "prellenado {:.1} s · generación {:.1} tok/s · contexto {ctx}",
            "prefill {:.1} s · generation {:.1} tok/s · context {ctx}",
            self.prefill_s,
            self.gen_tok_s
        )
    }
}

/// Frontend de terminal cruda: println + stdin. Lo usa el CLI.
pub struct StdFrontend {
    /// ¿Se mostró ya algo de la respuesta en vivo? (para no repetirla al final)
    streamed: AtomicBool,
    /// ¿Hay una línea a medio escribir?
    mid_line: AtomicBool,
    /// ¿Se está mostrando el avance del prellenado?
    prefill: AtomicBool,
    /// ¿La salida es una terminal? Si va a un archivo u otro programa, el
    /// progreso con `\r` ensuciaría el texto (medido): entonces no se pinta.
    tty: bool,
    /// Última métrica recibida (el CLI la resume al final).
    last: std::sync::Mutex<Option<TurnStats>>,
    /// El texto emitido en vivo DESDE la última línea de registro (herramienta, aviso). Medido
    /// (D5): con sólo `streamed` el CLI callaba la respuesta que redacta el PROGRAMA (cierre de
    /// la app en verde, «me detuve tras N pasos») porque el modelo había hablado antes.
    visto: std::sync::Mutex<String>,
    /// El ÚLTIMO bloque emitido en vivo antes de esa línea de registro. Medido 2026-11-18: tras la respuesta,
    /// el turno registra «compuerta: verificando…» o «recorrido del turno» y la respuesta salía DOS veces.
    ultimo: std::sync::Mutex<String>,
}

impl Default for StdFrontend {
    fn default() -> Self {
        use std::io::IsTerminal;
        Self {
            streamed: AtomicBool::new(false),
            mid_line: AtomicBool::new(false),
            prefill: AtomicBool::new(false),
            tty: std::io::stdout().is_terminal(),
            last: std::sync::Mutex::new(None),
            visto: std::sync::Mutex::new(String::new()),
            ultimo: std::sync::Mutex::new(String::new()),
        }
    }
}

impl StdFrontend {
    pub fn last_stats(&self) -> Option<TurnStats> {
        *self.last.lock().unwrap()
    }

    pub fn streamed(&self) -> bool {
        self.streamed.load(Ordering::Relaxed)
    }

    /// ¿Esta respuesta ya se vio en vivo? (es lo último que se emitió, sin herramientas después)
    pub fn ya_visto(&self, respuesta: &str) -> bool {
        let r = respuesta.trim();
        let v = self.visto.lock().unwrap();
        let u = self.ultimo.lock().unwrap();
        !r.is_empty() && (v.trim().ends_with(r) || (v.trim().is_empty() && u.trim().ends_with(r)))
    }

    fn close_line(&self) {
        if self.prefill.swap(false, Ordering::Relaxed) {
            print!("\r{:<44}\r", "");
        }
        if self.mid_line.swap(false, Ordering::Relaxed) {
            println!();
        }
    }
}

#[async_trait]
impl Frontend for StdFrontend {
    async fn secreto(&self, etiqueta: &str) -> Option<String> {
        self.close_line();
        let e = etiqueta.to_string();
        tokio::task::spawn_blocking(move || leer_sin_eco(&format!("   🔑 {e}\n   ")))
            .await
            .ok()
            .flatten()
    }

    async fn log(&self, line: String) {
        {
            let mut v = self.visto.lock().unwrap();
            if !v.trim().is_empty() {
                *self.ultimo.lock().unwrap() = std::mem::take(&mut *v);
            }
        }
        self.close_line();
        if self.tty {
            // diffs en color (verde agrega, rojo quita), sólo en terminal
            let color = if line.starts_with("  │+") {
                "\x1b[32m"
            } else if line.starts_with("  │-") {
                "\x1b[31m"
            } else if line.starts_with("  │ @@") || line.starts_with("  ┌") {
                "\x1b[36m"
            } else {
                ""
            };
            if !color.is_empty() {
                println!("{color}{line}\x1b[0m");
                return;
            }
        }
        println!("{line}");
    }

    fn stats(&self, s: &TurnStats) {
        *self.last.lock().unwrap() = Some(*s);
    }

    fn delta(&self, ev: &StreamEvent) {
        let mut out = std::io::stdout();
        match ev {
            StreamEvent::Prefill { done, total } if *total > 1 && self.tty => {
                let pct = (*done as u64 * 100 / *total as u64).min(100);
                print!(
                    "{}",
                    crate::tr!(
                        "\r⏳ leyendo contexto {pct:>3} %",
                        "\r⏳ reading context {pct:>3} %"
                    )
                );
                self.prefill.store(true, Ordering::Relaxed);
            }
            StreamEvent::Text(t) => {
                let starting = !self.mid_line.load(Ordering::Relaxed);
                // los saltos en blanco del arranque de turno no se pintan
                if starting && t.trim().is_empty() {
                    return;
                }
                if self.prefill.swap(false, Ordering::Relaxed) {
                    print!("\r{:<44}\r", "");
                }
                if starting {
                    print!("🧠 {}", t.trim_start());
                } else {
                    print!("{t}");
                }
                self.mid_line.store(true, Ordering::Relaxed);
                self.streamed.store(true, Ordering::Relaxed);
                self.visto.lock().unwrap().push_str(t);
            }
            _ => {}
        }
        out.flush().ok();
    }

    async fn choose(&self, question: &str, options: &[String]) -> Option<String> {
        use std::io::IsTerminal;
        if !std::io::stdin().is_terminal() {
            return None;
        }
        self.close_line();
        let (q, opts) = (question.to_string(), options.to_vec());
        tokio::task::spawn_blocking(move || {
            println!("   ❔ {q}");
            for (i, o) in opts.iter().enumerate() {
                println!("      {}. {o}", i + 1);
            }
            print!(
                "{}",
                crate::i18n::t(
                    "   número u otra respuesta: ",
                    "   number or another answer: "
                )
            );
            std::io::stdout().flush().ok();
            let mut line = String::new();
            std::io::stdin().read_line(&mut line).ok()?;
            let t = line.trim();
            if t.is_empty() {
                return None;
            }
            Some(match t.parse::<usize>() {
                Ok(n) if n >= 1 && n <= opts.len() => opts[n - 1].clone(),
                _ => t.to_string(),
            })
        })
        .await
        .ok()
        .flatten()
    }

    async fn ask(&self, desc: &str) -> AskAnswer {
        self.close_line();
        let desc = desc.to_string();
        tokio::task::spawn_blocking(move || {
            use std::io::{stdin, stdout};
            // P10: una pregunta en lote trae la lista debajo de su primera línea
            match desc.split_once('\n') {
                Some((head, list)) => {
                    println!(
                        "{}",
                        crate::tr!("   ❓ permitir {head}", "   ❓ allow {head}")
                    );
                    for l in list.lines() {
                        println!("   {l}");
                    }
                    print!(
                        "{}",
                        crate::tr!(
                            "   (s=sí a todas · n=no · a=siempre · x=nunca): ",
                            "   (y=yes to all · n=no · a=always · x=never): "
                        )
                    );
                }
                None => print!(
                    "{}",
                    crate::tr!(
                        "   ❓ permitir [{desc}]  (s=sí · n=no · a=siempre · x=nunca): ",
                        "   ❓ allow [{desc}]  (y=yes · n=no · a=always · x=never): "
                    )
                ),
            }
            stdout().flush().ok();
            let mut line = String::new();
            if stdin().read_line(&mut line).is_err() {
                return AskAnswer::No;
            }
            match line.trim().to_lowercase().as_str() {
                "s" | "si" | "sí" | "y" | "yes" => AskAnswer::Yes, // juez-idioma: datos (acepta ambos)
                "a" | "always" | "siempre" => AskAnswer::Always,
                "x" | "never" | "nunca" => AskAnswer::Never,
                _ => AskAnswer::No,
            }
        })
        .await
        .unwrap_or(AskAnswer::No)
    }
}

#[cfg(test)]
mod pruebas_visto {
    use super::*;

    /// La respuesta del modelo emitida en vivo no se repite; la que redacta el PROGRAMA tras
    /// las herramientas (cierre de la app en verde) sí se imprime.
    #[tokio::test]
    async fn la_respuesta_del_programa_se_ve_y_la_del_modelo_no_se_repite() {
        let f = StdFrontend::default();
        f.delta(&StreamEvent::Text("Voy a crear la app.".into()));
        f.log("· paso 1: app_nueva".into()).await;
        assert!(!f.ya_visto("La aplicación quedó lista en `cafe`"), "lo del programa se imprime");
        f.delta(&StreamEvent::Text("Listo: la app ".into()));
        f.delta(&StreamEvent::Text("funciona.".into()));
        assert!(f.ya_visto("Listo: la app funciona."), "lo del modelo ya se vio");
        assert!(!f.ya_visto(""));
        // medido 2026-11-18: lo que el turno registra DESPUÉS de la respuesta no la hace salir dos veces
        f.log("· compuerta: verificando lo que tocó el turno…".into()).await;
        f.log("· 🧭 recorrido del turno: .gotv/chats/1/recorrido.md".into()).await;
        assert!(f.ya_visto("Listo: la app funciona."), "se vio antes de esas líneas");
        // pero lo que el PROGRAMA redacta tras una herramienta sigue imprimiéndose
        f.delta(&StreamEvent::Text("Voy a probarla.".into()));
        f.log("· paso 2: bash".into()).await;
        assert!(!f.ya_visto("La aplicación quedó lista en `cafe`"));
    }
}

/// F1: lee una línea de la terminal SIN eco (lo tecleado no se ve ni queda en la pantalla).
/// `None` si no hay terminal o la línea viene vacía. El eco se restaura pase lo que pase con la lectura.
pub fn leer_sin_eco(rotulo: &str) -> Option<String> {
    use std::io::IsTerminal;
    if !std::io::stdin().is_terminal() {
        return None;
    }
    print!("{rotulo}");
    std::io::stdout().flush().ok();
    let fd = libc::STDIN_FILENO;
    // SAFETY: `termios` es una estructura C plana; `tcgetattr` la llena entera o falla.
    let mut t: libc::termios = unsafe { std::mem::zeroed() };
    if unsafe { libc::tcgetattr(fd, &mut t) } != 0 {
        return None;
    }
    let antes = t;
    t.c_lflag &= !libc::ECHO;
    t.c_lflag |= libc::ECHONL;
    // SAFETY: `fd` es la entrada estándar (una terminal, comprobado arriba) y `t` salió de `tcgetattr`.
    if unsafe { libc::tcsetattr(fd, libc::TCSANOW, &t) } != 0 {
        return None;
    }
    let mut l = String::new();
    let r = std::io::stdin().read_line(&mut l);
    // SAFETY: se devuelve la configuración que había.
    unsafe { libc::tcsetattr(fd, libc::TCSANOW, &antes) };
    r.ok()?;
    let v = l.trim_end_matches(['\n', '\r']).to_string();
    (!v.trim().is_empty()).then_some(v)
}
