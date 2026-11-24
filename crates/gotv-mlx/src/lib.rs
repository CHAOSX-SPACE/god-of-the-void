//! G4 · núcleo nativo de GOD OF THE VOID: el modelo corre DENTRO del proceso sobre MLX (mlx-rs),
//! sin Python ni servidor hijo. Fase 1 (2026-11-10): familia densa (qwen3, llama, mistral) con
//! pesos afines de mlx-community. Fase 2: el híbrido de Qwen3.5 (atención lineal Gated DeltaNet),
//! que cubre también los Qwen3.6/3.8 modificados; su estado recurrente se reutiliza entre turnos
//! con instantáneas (ver `modelo.rs`).
//!
//! Diseño: `mlx_rs::Array` envuelve un puntero de C y no es `Send`, así que el modelo vive en un
//! HILO PROPIO que es su dueño; el motor (`Send + Sync`, como pide `Engine`) le pasa trabajos por
//! un canal y recibe los eventos del flujo por otro. Entre turnos se conserva la caché KV: el
//! agente reenvía todo el historial y sólo se recalcula lo que cambió (prefijo común).

pub mod audio_gemma;
pub mod efectos;
pub mod gemma;
pub mod gkv;
pub mod imagen;
pub mod memoria;
pub mod modelo;
pub mod musica;
pub mod musica_fuente;
pub mod musica_lm;
pub mod pt;
pub mod separador;
pub mod separador_datos;
pub mod separador_entreno;
pub mod plantilla;
pub mod vision;
pub mod vision_gemma;
pub mod voz;
pub mod voz_decodificador;
pub mod video;
pub mod video_mp4;
pub mod whisper;

use anyhow::{anyhow, bail, Context, Result};
use async_trait::async_trait;
use gotv_engine::{tr, Capabilities, ChatRequest, ChatResponse, Engine, StreamEvent};
use mlx_rs::{ops, Array};
use modelo::{Extra, Red, M};
use std::collections::HashMap;
use plantilla::{Lector, Plantilla};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// Bloque de prellenado (acota la memoria y deja ver el avance).
const BLOQUE: usize = 512;

enum Trabajo {
    Generar { req: ChatRequest, salida: tokio::sync::mpsc::UnboundedSender<Salida> },
    /// F4: leer la conversación hasta donde abre su último mensaje (un postizo) y guardarla, sin generar
    Precargar { req: ChatRequest, salida: tokio::sync::mpsc::UnboundedSender<Salida> },
    Parar,
}

enum Salida {
    Evento(StreamEvent),
    Fin(std::result::Result<ChatResponse, String>),
    Precargada(std::result::Result<gotv_engine::Precarga, String>),
}

pub struct NativoEngine {
    dir: PathBuf,
    nombre: String,
    /// M8: el modelo trae una torre de visión que el núcleo sabe correr
    ve: bool,
    /// M1b: el modelo trae un oído que el núcleo sabe correr
    oye: bool,
    tx: Mutex<Option<std::sync::mpsc::Sender<Trabajo>>>,
    hilo: Mutex<Option<std::thread::JoinHandle<()>>>,
}

impl NativoEngine {
    pub fn new(dir: impl Into<PathBuf>) -> NativoEngine {
        let dir = dir.into();
        let nombre = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| tr!("modelo", "model"));
        let ve = NativoEngine::ve(&dir);
        let oye = NativoEngine::oye(&dir);
        NativoEngine { dir, nombre, ve, oye, tx: Mutex::new(None), hilo: Mutex::new(None) }
    }

    /// ¿Ve imágenes en el núcleo? Config con torre admitida Y pesos `vision_tower.*` en el disco
    /// (hay conversiones que la quitan: el Qwen3.8 Uncensored, p. ej.).
    pub fn ve(dir: &Path) -> bool {
        (vision::Vision::config(dir).is_some() || vision_gemma::VisionGemma::config(dir).is_some()) && tiene_pesos(dir, "vision_tower.")
    }

    /// ¿Oye audio en el núcleo? (M1b: Gemma 4 con su oído y los pesos `audio_tower.*` en el disco)
    pub fn oye(dir: &Path) -> bool {
        audio_gemma::OidoGemma::config(dir).is_some() && tiene_pesos(dir, "audio_tower.")
    }

    /// ¿Este modelo lo puede correr el núcleo nativo? (lee sólo su config.json)
    pub fn puede(dir: &Path) -> std::result::Result<(), String> {
        let c = modelo::Config::leer(dir).map_err(|e| format!("{e:#}"))?;
        match modelo::por_que_no(&c) {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }
}

/// ¿Algún .safetensors del modelo trae pesos con este prefijo? (sólo las cabeceras)
fn tiene_pesos(dir: &Path, prefijo: &str) -> bool {
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "safetensors"))
        .any(|p| cabecera_safetensors(&p).is_some_and(|(c, _)| c.keys().any(|k| k.starts_with(prefijo))))
}

/// ¿Qué sentido tiene el modelo que el núcleo aún no le da? (`ve`/`oye`: lo que el modelo trae).
/// Con el núcleo por defecto (M9), un modelo así va al servidor para no perder nada: hoy, la visión
/// o el oído de familias sin torre portada. `None` = el núcleo le da todo.
pub fn sentido_que_falta(dir: &Path, ve: bool, oye: bool) -> Option<String> {
    if oye && !NativoEngine::oye(dir) {
        return Some(tr!("el modelo oye audio y el núcleo nativo aún no con esta familia", "the model hears audio and the native core can't yet with this family"));
    }
    if ve && !NativoEngine::ve(dir) {
        return Some(tr!("el modelo ve imágenes y el núcleo nativo aún no con esta familia", "the model sees images and the native core can't yet with this family"));
    }
    None
}

/// ¿Este modelo SÓLO sirve en el núcleo nativo? Hoy: un `qwen3_5` que conserva capas `mtp` con
/// las normas ya corridas (el Fable 27B oQ4e): `mlx_lm` las corre otra vez al ver `mtp.*` y el
/// servidor responde basura (medido 2026-11-10, Errario #2549). Se decide por el VALOR de una norma,
/// leído directo del safetensors (cabecera + 4096 bf16, sin cargar el modelo ni MLX).
pub fn exige_nativo(dir: &Path) -> Option<String> {
    let c = modelo::Config::leer(dir).ok()?;
    if !c.hibrido() || modelo::por_que_no(&c).is_some() {
        return None;
    }
    let mut con_mtp = false;
    let mut media = None;
    for a in std::fs::read_dir(dir).ok()?.filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.extension().is_some_and(|x| x == "safetensors")) {
        let (cab, inicio) = cabecera_safetensors(&a)?;
        con_mtp |= cab.keys().any(|k| k.contains("mtp."));
        let t = cab.iter().find(|(k, _)| k.ends_with("model.layers.0.input_layernorm.weight")).map(|(_, v)| v.clone());
        if let Some(t) = t {
            if t["dtype"] != "BF16" {
                return None;
            }
            let o = t["data_offsets"].as_array()?;
            let (i, f) = (o[0].as_u64()?, o[1].as_u64()?);
            let mut b = vec![0u8; (f - i) as usize];
            use std::io::{Read, Seek, SeekFrom};
            let mut fh = std::fs::File::open(&a).ok()?;
            fh.seek(SeekFrom::Start(inicio + i)).ok()?;
            fh.read_exact(&mut b).ok()?;
            let v: Vec<f32> = b.as_chunks::<2>().0.iter().map(|x| f32::from_bits((u16::from_le_bytes(*x) as u32) << 16)).collect();
            media = Some(v.iter().sum::<f32>() / v.len().max(1) as f32);
        }
    }
    (con_mtp && media.is_some_and(|m| m.abs() >= 0.5)).then(|| {
        tr!(
            "este modelo trae capas mtp con las normas ya ajustadas: mlx_lm lo carga mal (responde basura); el núcleo de GOTV lo corre bien",
            "this model ships mtp layers with its norms already adjusted: mlx_lm loads it wrong (answers garbage); the GOTV core runs it right"
        )
    })
}

/// La cabecera JSON de un .safetensors y dónde empiezan los datos.
fn cabecera_safetensors(p: &Path) -> Option<(serde_json::Map<String, serde_json::Value>, u64)> {
    use std::io::Read;
    let mut f = std::fs::File::open(p).ok()?;
    let mut n = [0u8; 8];
    f.read_exact(&mut n).ok()?;
    let n = u64::from_le_bytes(n);
    if n > 100_000_000 {
        return None;
    }
    let mut j = vec![0u8; n as usize];
    f.read_exact(&mut j).ok()?;
    let v: serde_json::Value = serde_json::from_slice(&j).ok()?;
    Some((v.as_object()?.clone(), 8 + n))
}

/// La carpeta de cachés de prefijo de ESTE modelo (por su ruta y la fecha/tamaño de su config):
/// `~/.cache/gotv/prefijos/<clave>`. `GOTV_SIN_PREFIJOS=1` las desactiva.
fn carpeta_prefijos(dir: &Path) -> Option<PathBuf> {
    if std::env::var_os("GOTV_SIN_PREFIJOS").is_some() {
        return None;
    }
    let base = match std::env::var_os("GOTV_PREFIJOS_DIR") {
        Some(d) => PathBuf::from(d),
        None => PathBuf::from(std::env::var_os("HOME")?).join(".cache/gotv/prefijos"),
    };
    Some(base.join(clave_modelo(dir)?))
}

/// La clave de ESTE modelo (su ruta y la fecha/tamaño de su config): una caché guardada sólo sirve
/// al mismo modelo.
fn clave_modelo(dir: &Path) -> Option<String> {
    use std::hash::{Hash, Hasher};
    let canon = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());
    let meta = std::fs::metadata(dir.join("config.json")).ok()?;
    let mut h = std::collections::hash_map::DefaultHasher::new();
    canon.hash(&mut h);
    meta.len().hash(&mut h);
    meta.modified().ok().hash(&mut h);
    Some(format!("{:016x}", h.finish()))
}

/// La caché de una CONVERSACIÓN se reescribe sólo si creció al menos esto desde la última vez (un
/// giga por turno no; unos cientos de tokens sin guardar se releen en una fracción de segundo).
const CONVERSACION_PASO: usize = 1024;

/// Tope ELÁSTICO (prueba Cable, 2026-11-12): fuera de una llamada a herramienta, una respuesta se corta
/// aquí aunque `max_tokens` sea mayor. Sólo lo que se escribe DENTRO de una llamada (un archivo grande)
/// puede seguir hasta `max_tokens`. Medido: con 4 096 el 9B no cabía su archivo de pruebas; con 16 384
/// fijos el 35B razonó en círculo 13 minutos.
const TOPE_TEXTO: usize = 4096;

/// ¿El texto acaba en bucle? Toma los últimos 160 caracteres y cuenta cuántas veces aparecen en todo
/// lo escrito: 3 o más es el mismo tramo repetido (medido: el 35B repitió «Ah, el problema es que
/// `almacen.pagar` hace…» hasta llenar el tope). Un tramo casi sin letras (espacios, guiones) no cuenta.
pub(crate) fn repite_tramo(texto: &str) -> bool {
    const VENTANA: usize = 160;
    let t = texto.trim_end();
    let n = t.chars().count();
    if n < VENTANA * 3 {
        return false;
    }
    let desde = t.char_indices().nth(n - VENTANA).map(|(i, _)| i).unwrap_or(0);
    let cola = &t[desde..];
    if cola.chars().filter(|c| c.is_alphanumeric()).count() < VENTANA / 3 {
        return false;
    }
    t.matches(cola).count() >= 3
}

/// Los rasgos de una imagen ya vista y su rejilla (sólo Qwen la usa para las posiciones 3D).
type RasgosHechos = (Array, Option<(i32, i32, i32)>);

/// La torre de visión del modelo: la de Qwen3.5 (M8) o la de Gemma 4 (M8b).
enum Ojo {
    Qwen(vision::Vision),
    Gemma(vision_gemma::VisionGemma),
}

/// Los bytes de un `data:…;base64,…`.
fn bytes_de_url(url: &str) -> Result<Vec<u8>> {
    use base64::Engine as _;
    let b64 = url.split_once(',').map(|x| x.1).ok_or_else(|| anyhow!(tr!("adjunto sin datos", "attachment without data")))?;
    base64::engine::general_purpose::STANDARD.decode(b64.trim()).map_err(|e| anyhow!(tr!("adjunto en base64 inválido: {e}", "invalid base64 attachment: {e}")))
}

/// ¿Este adjunto es un audio? (`data:audio/…`)
pub(crate) fn es_audio(url: &str) -> bool {
    url.starts_with("data:audio")
}

/// Quita de los mensajes lo que el modelo no puede ver u oír y deja una nota en su lugar. Medido
/// 2026-11-10 en la app: una nota de voz grabada cuando el modelo oía (Gemma) quedó en el historial; con
/// el 9B, que no oye, la plantilla reventaba en CADA turno («Unexpected item type in content»), porque
/// el historial entero se reenvía. `None` si no hay nada que quitar.
pub(crate) fn sin_sentidos_que_faltan(mensajes: &[gotv_engine::Message], ve: bool, oye: bool) -> Option<Vec<gotv_engine::Message>> {
    let sobra = |m: &gotv_engine::Message| {
        let (i, a) = adjuntos(m);
        (!ve && !i.is_empty()) || (!oye && !a.is_empty())
    };
    if !mensajes.iter().any(sobra) {
        return None;
    }
    Some(
        mensajes
            .iter()
            .map(|m| {
                if !sobra(m) {
                    return m.clone();
                }
                let mut m = m.clone();
                let (n_img, n_aud) = {
                    let (i, a) = adjuntos(&m);
                    (if ve { 0 } else { i.len() }, if oye { 0 } else { a.len() })
                };
                if !oye {
                    m.audio.clear();
                    m.images.retain(|u| !es_audio(u));
                }
                if !ve {
                    m.images.retain(|u| es_audio(u));
                }
                let mut nota = Vec::new();
                if n_aud > 0 {
                    nota.push(format!("{n_aud} audio(s)"));
                }
                if n_img > 0 {
                    nota.push(tr!("{n_img} imagen(es)", "{n_img} image(s)"));
                }
                m.content = tr!(
                    "{}\n[{}: adjunto que este modelo no puede abrir; no se le pasa]",
                    "{}\n[{}: attachment this model can't open; not passed on]",
                    m.content,
                    nota.join(gotv_engine::i18n::t(" y ", " and "))
                );
                m
            })
            .collect(),
    )
}

/// Los adjuntos de un mensaje: (imágenes, audios). `user_with_media` pone los audios en
/// `Message::audio` (medido 2026-11-10: el núcleo miraba sólo `images` y el agente real no oía);
/// un `data:audio` que llegue en `images` también cuenta como audio.
pub(crate) fn adjuntos(m: &gotv_engine::Message) -> (Vec<&String>, Vec<&String>) {
    let (mut aud, img): (Vec<&String>, Vec<&String>) = m.images.iter().partition(|u| es_audio(u));
    aud.extend(m.audio.iter());
    (img, aud)
}

/// Un prompt con imágenes y/o audios ya preparado.
struct Imagenes {
    ids: Vec<u32>,
    pos: Option<[Vec<i32>; 3]>,
    cual: Vec<i32>,
    rasgos: Array,
    lugares: Vec<(usize, u64)>,
}

/// La memoria que macOS recomienda para la GPU (`max_recommended_working_set_size`, la misma que
/// usa mlx_lm): mlx-rs no la expone, se lee de mlx-c.
fn trabajo_recomendado() -> Option<usize> {
    let clave = std::ffi::CString::new("max_recommended_working_set_size").ok()?;
    // SAFETY: llamadas de mlx-c con objetos creados y liberados aquí mismo.
    unsafe {
        let dev = mlx_sys::mlx_device_new_type(mlx_sys::mlx_device_type__MLX_GPU, 0);
        let mut info = mlx_sys::mlx_device_info_new();
        let mut v: usize = 0;
        let ok = mlx_sys::mlx_device_info_get(&mut info, dev) == 0 && mlx_sys::mlx_device_info_get_size(&mut v, info, clave.as_ptr()) == 0;
        mlx_sys::mlx_device_info_free(info);
        mlx_sys::mlx_device_free(dev);
        (ok && v > 0).then_some(v)
    }
}

/// La memoria del proceso para MLX (medido 2026-11-10: al apagar el motor del 9B, MLX RETENÍA 7 GB
/// en su caché de búferes; con el 32B, el siguiente modelo cargaba encima y la Mac se iba a 28 GB de
/// swap): caché acotada mientras vive y vaciada al apagar.
const CACHE_MLX: usize = 2 * 1024 * 1024 * 1024;

fn texto_panico(p: &Box<dyn std::any::Any + Send>) -> String {
    let m = p.downcast_ref::<String>().cloned().or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string())).unwrap_or_else(|| tr!("pánico sin mensaje", "panic without a message"));
    tr!("MLX falló: {}", "MLX failed: {}", m.chars().take(400).collect::<String>())
}

/// M1 · Whisper NATIVO para `gotv_core::audio::transcribe` (el CLI y la GUI lo registran al
/// arrancar): un WAV 16 kHz mono → texto. Corre en su PROPIO hilo y su propio stream de GPU (no se
/// cruza con el motor del modelo), y al terminar suelta el modelo y vacía la caché de MLX.
pub fn transcribir_wav(dir: &Path, wav: &[u8]) -> Result<String> {
    let pcm = audio_gemma::pcm_de_wav(wav)?;
    let dir = dir.to_path_buf();
    let hilo = std::thread::Builder::new().name("gotv-whisper".into()).spawn(move || {
        fijar_metallib();
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<String> {
            // la LECTURA de los pesos va en el stream por defecto (MLX no lee archivos en un stream
            // de GPU: «Load::eval_gpu Not implemented», medido); el cálculo, en uno propio
            let w = whisper::Whisper::cargar(&dir)?;
            mlx_rs::with_device(mlx_rs::Device::gpu(), || Ok(w.transcribir(&pcm)?.texto.trim().to_string()))
        }));
        let _ = mlx_rs::memory::clear_cache();
        r.unwrap_or_else(|p| Err(anyhow!(texto_panico(&p))))
    })?;
    hilo.join().map_err(|p| anyhow!(texto_panico(&p)))?
}

/// M2 · la voz de GOTV: la referencia (códigos + vector de hablante, 14 KB) va DENTRO del binario.
/// Es SINTÉTICA (Qwen3-TTS VoiceDesign con una descripción, elegida por el Portador); se recalcula con
/// `archivo/herramientas/voz/ref_qwen3tts.py`.
const VOZ_GOTV: &[u8] = include_bytes!("../recursos/voz-gotv.safetensors");
/// Lo que dice la referencia (copia de `herramientas/voz/voz-gotv/voz.json`; una prueba lo vigila).
// juez-idioma: datos — la frase EXACTA que dice el audio de referencia; no se traduce
pub const VOZ_GOTV_TEXTO: &str = "Hola, Portador. Ya terminé la compilación: pasan trescientas cuarenta y siete pruebas y no hay ninguna advertencia.";

/// Lo que midió una frase dicha.
#[derive(Debug, Clone)]
pub struct Habla {
    pub fotogramas: usize,
    pub muestras: usize,
    pub carga: std::time::Duration,
    pub primer_audio: std::time::Duration,
    pub total: std::time::Duration,
}

/// M3 · la GPU POR TURNOS entre la voz y el modelo de lenguaje del mismo proceso. Medido 2026-11-10
/// con el 32B: hablando mientras el modelo escribía, la voz caía a 4,9 fotogramas/s (el habla pide
/// 12,5) y la respuesta larga dejaba 17,6 s de silencio; sola, la voz va a ~24. Mientras la voz
/// genera una frase, el modelo espera entre token y token; la voz va al doble del habla, así que le
/// deja la GPU la otra mitad del tiempo, y el modelo sigue escribiendo más rápido de lo que se habla.
static VOZ_EN_GPU: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

fn ceder_a_la_voz() {
    while VOZ_EN_GPU.load(std::sync::atomic::Ordering::Acquire) > 0 {
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
}

/// La voz tiene la GPU mientras viva esto.
struct TurnoDeVoz;

impl TurnoDeVoz {
    fn tomar() -> TurnoDeVoz {
        VOZ_EN_GPU.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        TurnoDeVoz
    }
}

impl Drop for TurnoDeVoz {
    fn drop(&mut self) {
        VOZ_EN_GPU.fetch_sub(1, std::sync::atomic::Ordering::AcqRel);
    }
}

/// Un pedido al hilo de la voz: qué decir, adónde mandar la onda y la bandera para callarla.
struct PedidoVoz {
    dir: PathBuf,
    texto: String,
    idioma: String,
    voraz: bool,
    onda: std::sync::mpsc::Sender<Vec<f32>>,
    fin: std::sync::mpsc::Sender<Result<(usize, std::time::Duration)>>,
    callar: Arc<std::sync::atomic::AtomicBool>,
}
static VOZ: std::sync::OnceLock<Mutex<std::sync::mpsc::Sender<PedidoVoz>>> = std::sync::OnceLock::new();
/// Tope de la caché de búferes de MLX mientras la voz genera (ver `hilo_de_voz`).
const CACHE_VOZ: usize = 8 * 1024 * 1024 * 1024;
/// Tras este tiempo sin hablar, el modelo de voz se suelta (4,5 GB vuelven al sistema).
const VOZ_OCIOSA: std::time::Duration = std::time::Duration::from_secs(300);

fn hilo_de_voz() -> std::sync::mpsc::Sender<PedidoVoz> {
    let (tx, rx) = std::sync::mpsc::channel::<PedidoVoz>();
    let _ = std::thread::Builder::new().name("gotv-voz".into()).spawn(move || {
        fijar_metallib();
        let mut cargada: Option<(PathBuf, voz::Voz, voz::Referencia)> = None;
        loop {
            let p = match rx.recv_timeout(VOZ_OCIOSA) {
                Ok(p) => p,
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                    if cargada.take().is_some() {
                        let _ = mlx_rs::memory::clear_cache();
                    }
                    continue;
                }
                Err(_) => break,
            };
            let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<(usize, std::time::Duration)> {
                let t = std::time::Instant::now();
                if cargada.as_ref().is_none_or(|(d, _, _)| *d != p.dir) {
                    cargada = None;
                    let _ = mlx_rs::memory::clear_cache();
                    let ruta = std::env::temp_dir().join(format!("gotv-voz-{}.safetensors", std::process::id()));
                    std::fs::write(&ruta, VOZ_GOTV)?;
                    let r = voz::Referencia::cargar(&ruta, VOZ_GOTV_TEXTO);
                    let _ = std::fs::remove_file(&ruta);
                    let r = r?;
                    // la LECTURA de los pesos va en el stream por defecto (falla #2557)
                    let v = voz::Voz::cargar(&p.dir)?;
                    cargada = Some((p.dir.clone(), v, r));
                }
                let carga = t.elapsed();
                let (_, v, r) = cargada.as_ref().ok_or_else(|| anyhow!(tr!("voz sin cargar", "voice not loaded")))?;
                // la caché de búferes de MLX es de TODO el proceso y el motor del modelo de lenguaje
                // la acota a 2 GB: medido 2026-11-10, con ese tope la voz iba a ~7 fotogramas/s (por
                // debajo de los 12,5 del habla: se entrecortaba) y sin él a 24,5. Mientras la voz
                // genera se sube a CACHE_VOZ y al terminar se devuelve el tope que había.
                let tope_antes = mlx_rs::memory::set_cache_limit(CACHE_VOZ).ok();
                let turno = TurnoDeVoz::tomar();
                let m = if p.voraz { voz::Muestreo { temperatura: 0.0, ..Default::default() } } else { voz::Muestreo::default() };
                let n = mlx_rs::with_device(mlx_rs::Device::gpu(), || {
                    let seguir = || !p.callar.load(std::sync::atomic::Ordering::Relaxed);
                    v.decir_en_trozos(&p.texto, r, &p.idioma, m, &seguir, &mut |w| {
                        if p.callar.load(std::sync::atomic::Ordering::Relaxed) {
                            bail!("callada");
                        }
                        p.onda.send(w.to_vec()).map_err(|_| anyhow!("quien escuchaba se fue"))
                    })
                });
                drop(turno);
                if let Some(a) = tope_antes {
                    let _ = mlx_rs::memory::set_cache_limit(a);
                }
                Ok((n?, carga))
            }));
            let r = r.unwrap_or_else(|x| {
                cargada = None; // tras un pánico, el estado del modelo no es de fiar
                Err(anyhow!(texto_panico(&x)))
            });
            drop(p.onda); // cierra el canal de la onda ANTES de avisar el fin
            let _ = p.fin.send(r);
        }
    });
    tx
}

/// Para cortar una frase que se está diciendo (la interrupción de M3, el botón de la GUI).
#[derive(Clone, Default)]
pub struct Callar(Arc<std::sync::atomic::AtomicBool>);

impl Callar {
    pub fn ya(&self) {
        self.0.store(true, std::sync::atomic::Ordering::Relaxed);
    }
    pub fn pedido(&self) -> bool {
        self.0.load(std::sync::atomic::Ordering::Relaxed)
    }
    /// La bandera compartida (para saber si dos `Callar` son el mismo).
    pub fn bandera(&self) -> Arc<std::sync::atomic::AtomicBool> {
        self.0.clone()
    }
}

/// M2 · dice `texto` con la voz de GOTV y el Qwen3-TTS Base de `dir`: `cada` recibe la onda (24 kHz,
/// mono, f32) POR TROZOS en cuanto existe. Un hilo propio mantiene el modelo cargado entre frases
/// (y lo suelta tras 5 min sin hablar); `callar` la corta entre dos trozos. `voraz` = temperatura 0.
pub fn hablar(dir: &Path, texto: &str, idioma: &str, voraz: bool, callar: &Callar, cada: &mut dyn FnMut(&[f32]) -> Result<()>) -> Result<Habla> {
    let (tx_onda, rx_onda) = std::sync::mpsc::channel::<Vec<f32>>();
    let (tx_fin, rx_fin) = std::sync::mpsc::channel();
    let ini = std::time::Instant::now();
    {
        let voz = VOZ.get_or_init(|| Mutex::new(hilo_de_voz()));
        let mut g = voz.lock().map_err(|_| anyhow!("hilo de voz envenenado"))?;
        let pedido = || PedidoVoz { dir: dir.to_path_buf(), texto: texto.into(), idioma: idioma.into(), voraz, onda: tx_onda.clone(), fin: tx_fin.clone(), callar: callar.0.clone() };
        if g.send(pedido()).is_err() {
            *g = hilo_de_voz(); // el hilo murió: uno nuevo
            g.send(pedido()).map_err(|_| anyhow!(tr!("no arrancó el hilo de voz", "the voice thread did not start")))?;
        }
    }
    drop(tx_onda);
    let (mut muestras, mut primer) = (0usize, None);
    for w in rx_onda {
        primer.get_or_insert_with(|| ini.elapsed());
        muestras += w.len();
        if let Err(e) = cada(&w) {
            callar.ya();
            return Err(e);
        }
    }
    let (fotogramas, carga) = match rx_fin.recv().map_err(|_| anyhow!(tr!("el hilo de voz se cayó", "the voice thread crashed")))? {
        Err(_) if callar.pedido() => (0, std::time::Duration::ZERO),
        r => r?,
    };
    Ok(Habla { fotogramas, muestras, carga, primer_audio: primer.unwrap_or_default(), total: ini.elapsed() })
}

/// WAV PCM de 16 bits mono.
pub fn wav_pcm16(onda: &[f32], tasa: u32) -> Vec<u8> {
    let datos = (onda.len() * 2) as u32;
    let mut w = Vec::with_capacity(44 + datos as usize);
    w.extend(b"RIFF");
    w.extend((36 + datos).to_le_bytes());
    w.extend(b"WAVEfmt ");
    w.extend(16u32.to_le_bytes());
    w.extend(1u16.to_le_bytes());
    w.extend(1u16.to_le_bytes());
    w.extend(tasa.to_le_bytes());
    w.extend((tasa * 2).to_le_bytes());
    w.extend(2u16.to_le_bytes());
    w.extend(16u16.to_le_bytes());
    w.extend(b"data");
    w.extend(datos.to_le_bytes());
    for x in onda {
        w.extend(((x.clamp(-1.0, 1.0) * 32767.0).round() as i16).to_le_bytes());
    }
    w
}

/// Un pedido al hilo del embebedor: (carpeta del modelo, textos, dónde responder).
type PedidoVectores = (PathBuf, Vec<String>, std::sync::mpsc::Sender<Result<Vec<Vec<f32>>>>);
static EMBEBEDOR: std::sync::OnceLock<Mutex<std::sync::mpsc::Sender<PedidoVectores>>> = std::sync::OnceLock::new();
/// Tras este tiempo sin pedidos, el embebedor se suelta (y su memoria vuelve al sistema).
const EMBEBEDOR_OCIOSO: std::time::Duration = std::time::Duration::from_secs(300);
/// El tope de tokens por texto del servidor de embeddings de mlx_vlm (`min(model_max_length, 8192)`).
const EMBEBEDOR_MAX_TOKENS: usize = 8192;

/// M11 · los vectores de unos textos con el embebedor del RAG (Qwen3-Embedding) en el NÚCLEO, como
/// el servidor de embeddings de mlx_vlm: tokenizador con su `<|endoftext|>` final, el modelo
/// entero, último token, norma final, L2. Un hilo propio mantiene el modelo cargado mientras se
/// use (el agente busca varias veces en una tarea) y lo suelta tras 5 min sin pedidos.
pub fn incrustar_textos(dir: &Path, textos: &[String]) -> Result<Vec<Vec<f32>>> {
    let tx = EMBEBEDOR.get_or_init(|| {
        let (tx, rx) = std::sync::mpsc::channel::<PedidoVectores>();
        let _ = std::thread::Builder::new().name("gotv-embebedor".into()).spawn(move || {
            fijar_metallib();
            let mut cargado: Option<(PathBuf, modelo::Modelo, tokenizers::Tokenizer)> = None;
            loop {
                match rx.recv_timeout(EMBEBEDOR_OCIOSO) {
                    Ok((dir, textos, resp)) => {
                        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<Vec<Vec<f32>>> {
                            if cargado.as_ref().is_none_or(|(d, _, _)| *d != dir) {
                                cargado = None;
                                let _ = mlx_rs::memory::clear_cache();
                                // la LECTURA de los pesos va en el stream por defecto (falla #2557)
                                let m = modelo::Modelo::cargar(&dir)?;
                                let tok = tokenizers::Tokenizer::from_file(dir.join("tokenizer.json")).map_err(|e| anyhow!(tr!("tokenizer.json del embebedor: {e}", "embedder tokenizer.json: {e}")))?;
                                cargado = Some((dir.clone(), m, tok));
                            }
                            let (_, m, tok) = cargado.as_ref().ok_or_else(|| anyhow!(tr!("embebedor sin cargar", "embedder not loaded")))?;
                            mlx_rs::with_device(mlx_rs::Device::gpu(), || {
                                textos.iter().map(|t| vector_de(m, tok, t)).collect()
                            })
                        }));
                        let r = r.unwrap_or_else(|p| {
                            cargado = None;
                            Err(anyhow!(texto_panico(&p)))
                        });
                        let _ = resp.send(r);
                    }
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                        if cargado.take().is_some() {
                            let _ = mlx_rs::memory::clear_cache();
                        }
                    }
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
                }
            }
        });
        Mutex::new(tx)
    });
    let (rtx, rrx) = std::sync::mpsc::channel();
    tx.lock().map_err(|_| anyhow!(tr!("embebedor envenenado", "embedder lock poisoned")))?.send((dir.to_path_buf(), textos.to_vec(), rtx)).map_err(|_| anyhow!(tr!("el hilo del embebedor no está", "the embedder thread is gone")))?;
    rrx.recv().map_err(|_| anyhow!(tr!("el embebedor no respondió", "the embedder did not answer")))?
}

/// Un texto → su vector (f32). Como el tokenizador de HF con `truncation=True`: si no cabe, se
/// corta el texto y el `<|endoftext|>` final se conserva.
fn vector_de(m: &modelo::Modelo, tok: &tokenizers::Tokenizer, texto: &str) -> Result<Vec<f32>> {
    let con = tok.encode(texto, true).map_err(|e| anyhow!("tokenizar: {e}"))?.get_ids().to_vec();
    let ids = if con.len() > EMBEBEDOR_MAX_TOKENS {
        let sin = tok.encode(texto, false).map_err(|e| anyhow!("tokenizar: {e}"))?.get_ids().to_vec();
        let extra = &con[sin.len().min(con.len())..];
        let mut v = sin[..EMBEBEDOR_MAX_TOKENS - extra.len()].to_vec();
        v.extend_from_slice(extra);
        v
    } else {
        con
    };
    if ids.is_empty() {
        bail!(tr!("texto vacío", "empty text"));
    }
    let v = m.incrustar(&ids)?.as_dtype(mlx_rs::Dtype::Float32).m()?;
    Ok(v.as_slice::<f32>().to_vec())
}

/// Los kernels Metal de MLX: el binario trae grabada la ruta de la Mac donde se compiló; en otra
/// Mac (otro usuario) no existe. Se fija la copia que viaja con GOTV (`GOTV_MLX_METALLIB`, la
/// `.app` en `Contents/Resources/` o junto al binario); si no hay ninguna, MLX busca por su cuenta.
fn fijar_metallib() {
    let exe = std::env::current_exe().ok();
    let dir = exe.as_ref().and_then(|e| e.parent()).map(Path::to_path_buf);
    let candidatos: Vec<PathBuf> = std::env::var_os("GOTV_MLX_METALLIB")
        .map(PathBuf::from)
        .into_iter()
        .chain(dir.iter().map(|d| d.join("../Resources/mlx.metallib")))
        .chain(dir.iter().map(|d| d.join("mlx.metallib")))
        .collect();
    if let Some(m) = candidatos.into_iter().find(|p| p.is_file()) {
        if let Ok(m) = m.canonicalize() {
            let _ = mlx_rs::metal::set_metallib_path(m.to_string_lossy());
        }
    }
}

/// Lo que vive en el hilo del modelo.
struct Trabajador {
    modelo: Box<dyn Red>,
    cache: modelo::Cache,
    en_cache: Vec<u32>,
    tok: tokenizers::Tokenizer,
    plantilla: Plantilla,
    fin: Vec<u32>,
    /// el token que abre cada mensaje (`<|im_start|>`): el historial anterior al último termina
    /// justo antes, y ahí conviene guardar el estado recurrente
    inicio_turno: Option<u32>,
    /// M8/M8b: la torre de visión (si el modelo ve), el token de imagen y los límites de píxeles
    vision: Option<Ojo>,
    /// Gemma: las marcas que abren y cierran cada imagen en el prompt
    marcas_imagen: Option<(u32, u32)>,
    token_imagen: Option<u32>,
    pixeles: (i64, i64),
    /// M1b: el oído (si el modelo oye), su token y las marcas que abren y cierran cada audio
    oido: Option<audio_gemma::OidoGemma>,
    token_audio: Option<u32>,
    marcas_audio: Option<(u32, u32)>,
    /// rasgos ya calculados por huella del adjunto (el agente reenvía el historial en cada paso)
    rasgos_hechos: HashMap<u64, RasgosHechos>,
    /// imágenes cuyos tokens están en la caché: (primer token, huella)
    imagenes_en_cache: Vec<(usize, u64)>,
    /// cachés del prefijo del SISTEMA en disco (instrucciones + herramientas, casi siempre iguales):
    /// la conversación siguiente no las vuelve a calcular. `None` = desactivado.
    prefijos: Option<PathBuf>,
    /// la clave de este modelo, para la caché de la conversación (ver `conversaciones` en gotv-core)
    clave: Option<String>,
    /// F2: lo guardado en disco de la conversación activa (índice y tokens), para no releerlo en
    /// cada paso. Se recarga si cambia la carpeta (otra sesión, otro proyecto).
    disco: Option<memoria::Disco>,
    /// Una nota sobre la memoria leída (p. ej. que se quedó corta por el tope) para la respuesta.
    nota_memoria: Option<String>,
    /// F4: `generar` en modo precarga (lee hasta el postizo, guarda y no genera)
    precargando: bool,
    /// cuántos tokens vinieron del disco en la última petición
    del_disco: usize,
}

/// Cuántas cachés de prefijo se guardan por modelo (cada una ~150 MB en un 9B con 3 000 tokens)…
const PREFIJOS_MAX: usize = 6;
/// …y cuánto disco pueden ocupar juntas por modelo (un denso de 32B guarda ~790 MB por prefijo).
const PREFIJOS_BYTES: u64 = 2 * 1024 * 1024 * 1024;
/// Un prefijo más corto no compensa leerlo del disco.
const PREFIJO_MIN: usize = 256;

impl Trabajador {
    fn cargar(dir: &Path) -> Result<Trabajador> {
        let modelo = modelo::cargar_red(dir)?;
        let tok = tokenizers::Tokenizer::from_file(dir.join("tokenizer.json")).map_err(|e| anyhow!("tokenizer.json: {e}"))?;
        let plantilla = Plantilla::cargar(dir)?;
        // fin de turno: los eos del modelo y de su generation_config, más los de chat conocidos
        let mut fin: Vec<u32> = Vec::new();
        let mut ids = |v: &serde_json::Value| match v {
            serde_json::Value::Number(n) => fin.extend(n.as_u64().map(|x| x as u32)),
            serde_json::Value::Array(a) => fin.extend(a.iter().filter_map(|x| x.as_u64().map(|x| x as u32))),
            _ => {}
        };
        if let Some(e) = &modelo.cfg().eos_token_id {
            ids(e);
        }
        if let Ok(t) = std::fs::read_to_string(dir.join("generation_config.json")) {
            if let Ok(g) = serde_json::from_str::<serde_json::Value>(&t) {
                ids(&g["eos_token_id"]);
            }
        }
        for t in ["<|im_end|>", "<|endoftext|>", "<|eot_id|>", "</s>"] {
            if let Some(i) = tok.token_to_id(t) {
                fin.push(i);
            }
        }
        fin.sort_unstable();
        fin.dedup();
        let cache = modelo.cache_nueva();
        let inicio_turno = tok.token_to_id("<|im_start|>").or_else(|| tok.token_to_id("<|turn>"));
        let vision = if !NativoEngine::ve(dir) {
            None
        } else if let Some(c) = vision::Vision::config(dir) {
            Some(Ojo::Qwen(vision::Vision::cargar(dir, c)?))
        } else if let Some(c) = vision_gemma::VisionGemma::config(dir) {
            Some(Ojo::Gemma(vision_gemma::VisionGemma::cargar(dir, c, modelo.cfg())?))
        } else {
            None
        };
        let marcas_imagen = match (tok.token_to_id("<|image>"), tok.token_to_id("<image|>")) {
            (Some(a), Some(b)) if matches!(vision, Some(Ojo::Gemma(_))) => Some((a, b)),
            _ => None,
        };
        // los límites del procesador del modelo (`preprocessor_config.json` → size), como mlx_vlm
        let pp: serde_json::Value = std::fs::read_to_string(dir.join("preprocessor_config.json")).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        let pixeles = (pp["size"]["shortest_edge"].as_i64().unwrap_or(56 * 56), pp["size"]["longest_edge"].as_i64().unwrap_or(14 * 14 * 4 * 1280));
        let token_imagen = match vision {
            Some(Ojo::Gemma(_)) => tok.token_to_id("<|image|>"),
            _ => tok.token_to_id("<|image_pad|>"),
        };
        let oido = match audio_gemma::OidoGemma::config(dir) {
            Some(c) if NativoEngine::oye(dir) => Some(audio_gemma::OidoGemma::cargar(dir, c, modelo.cfg())?),
            _ => None,
        };
        let token_audio = oido.as_ref().and_then(|_| tok.token_to_id("<|audio|>"));
        let marcas_audio = match (tok.token_to_id("<|audio>"), tok.token_to_id("<audio|>")) {
            (Some(a), Some(b)) if oido.is_some() => Some((a, b)),
            _ => None,
        };
        let prefijos = carpeta_prefijos(dir);
        let clave = clave_modelo(dir);
        Ok(Trabajador { clave, disco: None, nota_memoria: None, precargando: false, del_disco: 0, modelo, cache, en_cache: Vec::new(), tok, plantilla, fin, inicio_turno, vision, marcas_imagen, token_imagen, pixeles, oido, token_audio, marcas_audio, rasgos_hechos: HashMap::new(), imagenes_en_cache: Vec::new(), prefijos })
    }

    /// Dónde termina el bloque del SISTEMA (donde abre el segundo mensaje), si vale la pena guardarlo.
    fn fin_sistema(&self, ids: &[u32]) -> Option<usize> {
        let t = self.inicio_turno?;
        let mut abre = ids.iter().enumerate().filter(|(_, &x)| x == t).map(|(i, _)| i);
        let (primero, segundo) = (abre.next()?, abre.next()?);
        (primero <= 2 && segundo >= PREFIJO_MIN).then_some(segundo)
    }

    fn ruta_prefijo(&self, ids: &[u32]) -> Option<PathBuf> {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        ids.hash(&mut h);
        Some(self.prefijos.as_ref()?.join(format!("{:016x}-{}.safetensors", h.finish(), ids.len())))
    }

    fn guardar_prefijo(&self, ids: &[u32]) {
        let (Some(ruta), Some(dir)) = (self.ruta_prefijo(ids), self.prefijos.as_ref()) else { return };
        if ruta.exists() || std::fs::create_dir_all(dir).is_err() || self.cache.largo as usize != ids.len() {
            return;
        }
        if self.cache.guardar(&ruta).is_err() {
            let _ = std::fs::remove_file(&ruta);
            return;
        }
        // sólo las más recientes de ESTE modelo (son nuestras: se regeneran solas)
        let mut todas: Vec<(std::time::SystemTime, u64, PathBuf)> = std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().is_some_and(|x| x == "safetensors") && !e.path().to_string_lossy().contains(".tmp."))
            .filter_map(|e| {
                let m = e.metadata().ok()?;
                Some((m.modified().ok()?, m.len(), e.path()))
            })
            .collect();
        todas.sort_by_key(|x| std::cmp::Reverse(x.0));
        // las más recientes primero: se quedan mientras quepan en el número Y en el tope de disco
        let mut usado = 0u64;
        for (n, (_, bytes, ruta)) in todas.into_iter().enumerate() {
            usado += bytes;
            if n >= PREFIJOS_MAX || (n > 0 && usado > PREFIJOS_BYTES) {
                let _ = std::fs::remove_file(ruta);
            }
        }
    }

    /// La caché guardada de esta conversación si sus tokens son prefijo del prompt (y su largo). Con
    /// imágenes, además, las MISMAS imágenes en los mismos lugares (dos del mismo tamaño dan los
    /// mismos tokens: la huella las distingue).
    fn leer_conversacion(&self, base: &Path, ids: &[u32], lugares: &[(usize, u64)]) -> Option<(modelo::Cache, usize)> {
        let guardados: Vec<u32> = std::fs::read(base.with_extension("ids")).ok()?.as_chunks::<4>().0.iter().map(|c| u32::from_le_bytes(*c)).collect();
        let n = guardados.len();
        if n == 0 || n >= ids.len() || ids[..n] != guardados[..] {
            return None;
        }
        let imgs: Vec<(usize, u64)> = std::fs::read(base.with_extension("imgs")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        let ahora: Vec<(usize, u64)> = lugares.iter().copied().filter(|(i, _)| *i < n).collect();
        if imgs != ahora {
            return None;
        }
        match modelo::Cache::leer(&base.with_extension("safetensors"), self.cache.capas_len()) {
            Ok(c) if c.largo as usize == n => Some((c, n)),
            _ => None,
        }
    }

    /// Lo guardado en `dir` (la carpeta de esta conversación con este modelo), cargado una vez.
    fn disco_en(&mut self, dir: &Path) -> &mut memoria::Disco {
        if self.disco.as_ref().is_none_or(|d| d.dir != dir) {
            self.disco = Some(memoria::Disco::abrir(dir));
        }
        self.disco.as_mut().unwrap()
    }

    /// F2: la memoria de esta conversación en disco para este prompt. `Ok(Some)` = caché restaurada
    /// (el prefijo más largo disponible); `Ok(None)` = la de memoria ya cubre lo mismo o más;
    /// `Err(porqué)` = no sirve (el agente lo dice si hubo que releer mucho).
    fn leer_memoria(&mut self, dir: &Path, ids: &[u32], lugares: &[(usize, u64)], comun: usize) -> std::result::Result<Option<(modelo::Cache, usize)>, String> {
        let fin_bloque = self.inicio_turno.and_then(|t| ids.iter().enumerate().filter(|(_, &x)| x == t).nth(1).map(|(i, _)| i));
        let capas = self.cache.capas_len();
        let d = self.disco_en(dir).clone();
        if d.indice.hasta() == 0 {
            // ¿la caché del formato anterior (un archivo entero)? se lee igual hasta el primer guardado
            if dir.with_extension("ids").exists() {
                return match self.leer_conversacion(dir, ids, lugares) {
                    Some((c, n)) if n > comun => Ok(Some((c, n))),
                    Some(_) => Ok(None),
                    None => Err(causa_sin_memoria(dir, ids, lugares, fin_bloque)),
                };
            }
            let otras = dir.parent().and_then(|s| std::fs::read_dir(s).ok()).map(|r| r.flatten().any(|e| e.path() != dir)).unwrap_or(false);
            return Err(if otras {
                tr!("la memoria guardada es de otro modelo", "the saved memory belongs to another model")
            } else {
                tr!("aún no hay memoria guardada de esta conversación", "there is no saved memory for this conversation yet")
            });
        }
        // al menos un token nuevo para tener logits… salvo al precargar, que no genera: ahí el estado
        // guardado JUSTO en el corte sirve (medido con el 9B: sin esto, «no hay un punto guardado»)
        let hasta_ids = if self.precargando { ids.len() } else { ids.len() - 1 };
        let p = memoria::prefijo_comun(&d, &ids[..hasta_ids], lugares);
        let r = memoria::restaurable(&d, p);
        // primero, si del disco no sale nada útil, el porqué (aunque la memoria en RAM cubra algo)
        if r < memoria::LEER_MIN {
            if p < memoria::LEER_MIN {
                return Err(diverge(p, d.ids.len(), fin_bloque));
            }
            // un híbrido sólo puede volver a un punto con estado guardado
            return Err(tr!(
                "no hay un punto guardado antes del token {p} (se rebobinó más atrás de lo guardado)",
                "there is no saved point before token {p} (it rewound past what was saved)"
            ));
        }
        if r <= comun {
            return Ok(None);
        }
        let c = memoria::leer(&d, r, capas).map_err(|e| tr!("el archivo de la memoria no se pudo leer: {e:#}", "the memory file could not be read: {e:#}"))?;
        // F3: leída = usada (el tope desaloja por uso, no por escritura)
        gotv_engine::memoria_tope::tocar(dir);
        // lo guardado es sólo un prefijo porque lo demás no cupo en el tope: se dice (hay que releer)
        if d.indice.recortado && ids.len() > r + 2000 {
            self.nota_memoria = Some(tr!(
                "la memoria guardada llega hasta el token {r} de {}: no cupo más en el tope del proyecto (súbelo en «Reabrir rápido»)",
                "the saved memory reaches token {r} of {}: nothing more fit in the project cap (raise it in «Reopen fast»)",
                ids.len()
            ));
        }
        Ok(Some((c, r)))
    }

    /// F2: añade a la memoria en disco lo nuevo de la caché (que va exactamente por `ids`).
    fn guardar_memoria(&mut self, dir: &Path, ids: &[u32], lugares: &[(usize, u64)]) {
        use gotv_engine::memoria_tope as tope;
        let t = std::time::Instant::now();
        // F3: el tope del PROYECTO (sólo si la conversación está en una carpeta de sesión `s<n>`: así
        // nunca se desaloja fuera de la memoria de GOTV, falla #2571)
        let proyecto = dir.parent().filter(|s| tope::es_sesion(s)).and_then(|s| s.parent()).map(Path::to_path_buf);
        let limite = proyecto.as_deref().map(tope::tope).unwrap_or(tope::TOPE_POR_OMISION);
        let mut cabe = |necesita: u64, d: &memoria::Disco| -> bool {
            // el disco de verdad: lo nuevo y 1 GiB de margen
            if tope::libre_en_disco(&d.dir).is_some_and(|libre| libre < necesita + (1 << 30)) {
                return false;
            }
            match &proyecto {
                Some(p) => tope::hacer_sitio(p, &d.dir, d.indice.bytes(), necesita, limite),
                None => d.indice.bytes() + necesita <= tope::utilizable(limite),
            }
        };
        let mut d = std::mem::take(self.disco_en(dir));
        let r = memoria::guardar(&mut d, &self.cache, ids, lugares, &mut cabe);
        let ocupa = d.indice.bytes();
        self.disco = Some(d);
        match r {
            Ok(g) if g.nuevos > 0 || g.cortados > 0 => {
                if std::env::var_os("GOTV_TIEMPOS").is_some() {
                    eprintln!("{}", tr!("[tiempos] memoria: +{} tok, −{} cortados, {:.1} MB en {:?}", "[timing] memory: +{} tok, −{} cut, {:.1} MB in {:?}", g.nuevos, g.cortados, g.bytes as f64 / 1e6, t.elapsed()));
                }
                // la caché del formato anterior ya no hace falta
                for ext in ["safetensors", "ids", "imgs"] {
                    let _ = std::fs::remove_file(dir.with_extension(ext));
                }
                // con el tamaño REAL (la estimación pudo quedarse corta)
                if let Some(p) = &proyecto {
                    tope::hacer_sitio(p, dir, ocupa, 0, limite);
                }
            }
            Ok(g) if g.no_cabe => {
                if std::env::var_os("GOTV_TIEMPOS").is_some() {
                    eprintln!("{}", tr!("[tiempos] memoria: no cabe más en el tope de {:.1} GB (queda el prefijo de {} tok)", "[timing] memory: nothing more fits in the {:.1} GB cap (the {} tok prefix stays)", limite as f64 / (1u64 << 30) as f64, ids.len().min(self.disco.as_ref().map(|d| d.indice.hasta()).unwrap_or(0))));
                }
            }
            Ok(_) => {}
            Err(e) => eprintln!("{}", tr!("⚠ no pude guardar la memoria de la conversación ({} tok): {e:#}", "⚠ could not save the conversation memory ({} tok): {e:#}", ids.len())),
        }
    }

    fn leer_prefijo(&self, ids: &[u32]) -> Option<modelo::Cache> {
        let ruta = self.ruta_prefijo(ids)?;
        if !ruta.exists() {
            return None;
        }
        let n = self.cache.capas_len();
        match modelo::Cache::leer(&ruta, n) {
            Ok(c) if c.largo as usize == ids.len() => {
                // usada ahora: que no sea la primera en irse
                let _ = std::fs::File::open(&ruta).and_then(|f| f.set_modified(std::time::SystemTime::now()));
                Some(c)
            }
            _ => {
                let _ = std::fs::remove_file(&ruta);
                None
            }
        }
    }

    /// Los adjuntos del prompt (imágenes y audios): sus rasgos (de la caché si ya se vieron), cada
    /// marca expandida a sus tokens (Qwen: N × pad; Gemma: <|image> + N × <|image|> + <image|> y
    /// <|audio> + N × <|audio|> + <audio|>), las posiciones 3D (sólo Qwen) y, por token, qué fila de
    /// rasgos le toca. Imágenes y audios se recorren cada uno en su orden, como los pinta la plantilla.
    fn preparar_medios(&mut self, req: &ChatRequest, ids: Vec<u32>) -> Result<Imagenes> {
        use std::hash::{Hash, Hasher};
        let (mut urls_img, mut urls_aud) = (Vec::new(), Vec::new());
        for m in &req.messages {
            let (i, a) = adjuntos(m);
            urls_img.extend(i);
            urls_aud.extend(a);
        }
        if !urls_img.is_empty() && self.vision.is_none() {
            bail!(tr!("este modelo no ve imágenes en el núcleo nativo", "this model does not see images in the native core"));
        }
        if !urls_aud.is_empty() && self.oido.is_none() {
            bail!(tr!("este modelo no oye audio en el núcleo nativo", "this model does not hear audio in the native core"));
        }
        let huella = |u: &str| {
            let mut hs = std::collections::hash_map::DefaultHasher::new();
            u.hash(&mut hs);
            hs.finish()
        };
        for u in urls_img.iter().chain(&urls_aud) {
            let h = huella(u);
            if self.rasgos_hechos.contains_key(&h) {
                continue;
            }
            let (f, rej) = if es_audio(u) {
                let o = self.oido.as_ref().ok_or_else(|| anyhow!(tr!("sin oído", "no hearing")))?;
                (o.rasgos(&audio_gemma::sonido(&audio_gemma::pcm_de_wav(&bytes_de_url(u)?)?)?)?, None)
            } else {
                match self.vision.as_ref().ok_or_else(|| anyhow!(tr!("sin visión", "no vision")))? {
                    Ojo::Qwen(v) => {
                        let img = vision::Imagen::desde_url(u, &v.c, self.pixeles.0, self.pixeles.1)?;
                        (v.rasgos(&img)?, Some(img.rejilla))
                    }
                    Ojo::Gemma(v) => (v.rasgos(&v.preparar(&bytes_de_url(u)?)?)?, None),
                }
            };
            f.eval().m()?;
            if self.rasgos_hechos.len() >= 16 {
                self.rasgos_hechos.clear();
            }
            self.rasgos_hechos.insert(h, (f, rej));
        }
        let (tim, tau) = (self.token_imagen.filter(|_| self.vision.is_some()), self.token_audio);
        let mut salida = Vec::with_capacity(ids.len());
        let mut lugares = Vec::new();
        let mut filas: Vec<Array> = Vec::new();
        let mut cual: Vec<i32> = Vec::with_capacity(ids.len());
        let mut rejillas = Vec::new();
        let (mut ki, mut ka, mut base) = (0usize, 0usize, 0i32);
        for t in ids {
            let (url, marcas, tok_medio) = if Some(t) == tim {
                let u = urls_img.get(ki).ok_or_else(|| anyhow!(tr!("el prompt tiene más marcas de imagen que imágenes", "the prompt has more image markers than images")))?;
                ki += 1;
                (u, self.marcas_imagen, t)
            } else if Some(t) == tau {
                let u = urls_aud.get(ka).ok_or_else(|| anyhow!(tr!("el prompt tiene más marcas de audio que audios", "the prompt has more audio markers than audios")))?;
                ka += 1;
                (u, self.marcas_audio, t)
            } else {
                salida.push(t);
                cual.push(-1);
                continue;
            };
            let h = huella(url);
            let (f, rej) = &self.rasgos_hechos[&h];
            let n = f.dim(0);
            if !es_audio(url) {
                rejillas.push(*rej);
            }
            if let Some((abre, _)) = marcas {
                salida.push(abre);
                cual.push(-1);
            }
            lugares.push((salida.len(), h));
            salida.extend(std::iter::repeat_n(tok_medio, n as usize));
            cual.extend(base..base + n);
            if let Some((_, cierra)) = marcas {
                salida.push(cierra);
                cual.push(-1);
            }
            filas.push(f.clone());
            base += n;
        }
        if ki != urls_img.len() || ka != urls_aud.len() {
            bail!(tr!("{} imágenes y {ki} marcas, {} audios y {ka} marcas en el prompt", "{} images and {ki} markers, {} audios and {ka} markers in the prompt", urls_img.len(), urls_aud.len()));
        }
        let pos = match (&self.vision, tim) {
            (Some(Ojo::Qwen(v)), Some(tim)) => {
                let rej: Vec<(i32, i32, i32)> = rejillas.iter().map(|r| r.unwrap_or((1, 1, 1))).collect();
                Some(vision::posiciones(&salida, tim, &rej, v.c.spatial_merge_size)?)
            }
            _ => None,
        };
        let todos = ops::concatenate(&filas, 0).m()?;
        Ok(Imagenes { ids: salida, pos, cual, rasgos: todos, lugares })
    }

    fn generar(&mut self, req: &ChatRequest, salida: &tokio::sync::mpsc::UnboundedSender<Salida>) -> Result<ChatResponse> {
        let on = |e: StreamEvent| {
            let _ = salida.send(Salida::Evento(e));
        };
        // un adjunto que ESTE modelo no entiende (audio sin oído, imagen sin visión) viaja como nota
        let limpio;
        let req = match sin_sentidos_que_faltan(&req.messages, self.vision.is_some(), self.oido.is_some()) {
            Some(m) => {
                limpio = ChatRequest { messages: m, ..req.clone() };
                &limpio
            }
            None => req,
        };
        let prompt = self.plantilla.prompt(&req.messages, req.tools.as_ref())?;
        let ids: Vec<u32> = self.tok.encode(prompt.as_str(), false).map_err(|e| anyhow!("tokenizar: {e}"))?.get_ids().to_vec();
        if ids.is_empty() {
            bail!(tr!("prompt vacío", "empty prompt"));
        }
        // M8/M1b: con imágenes o audios, los tokens se expanden y van con rasgos (y posiciones 3D)
        let (img, ids) = if req.messages.iter().any(|m| !m.images.is_empty() || !m.audio.is_empty()) {
            let i = self.preparar_medios(req, ids)?;
            let expandidos = i.ids.clone();
            (Some(i), expandidos)
        } else {
            (None, ids)
        };
        let lugares: Vec<(usize, u64)> = img.as_ref().map(|i| i.lugares.clone()).unwrap_or_default();
        self.del_disco = 0;
        // F4 · precarga: se lee hasta donde ABRE el último mensaje (un postizo que el agente añade):
        // lo de antes es exactamente lo que volverá cuando el Portador escriba de verdad
        let ids = if self.precargando {
            let marcas: Vec<usize> = self.inicio_turno.map(|t| ids.iter().enumerate().filter(|(_, &x)| x == t).map(|(i, _)| i).collect()).unwrap_or_default();
            match marcas.len().checked_sub(2).map(|i| marcas[i]) {
                Some(h) if h > 0 => ids[..h].to_vec(),
                // sin marcas de turno no se sabe dónde cortar: esta plantilla no precarga
                _ => return Ok(ChatResponse { finish_reason: "no-prefill".into(), ..Default::default() }),
            }
        } else {
            ids
        };
        // prefijo común con lo que ya está en la caché (al menos 1 token nuevo para tener logits)
        let mut comun = self.en_cache.iter().zip(&ids).take_while(|(a, b)| a == b).count();
        if self.precargando && comun >= ids.len() {
            // ya está todo en memoria
            return Ok(ChatResponse { prompt_tokens: ids.len() as u32, cached_tokens: ids.len() as u32, usage_exact: true, finish_reason: "precarga".into(), ..Default::default() });
        }
        if comun >= ids.len() {
            comun = ids.len() - 1;
        }
        // dos imágenes del MISMO tamaño dan los mismos ids: si la de la caché no es la de ahora, el
        // prefijo común termina donde empieza esa imagen
        for &(inicio, huella) in &self.imagenes_en_cache {
            if inicio < comun && !lugares.contains(&(inicio, huella)) {
                comun = comun.min(inicio);
            }
        }
        let reloj = std::time::Instant::now();
        let depurar = std::env::var_os("GOTV_TIEMPOS").is_some();
        // la caché de ESTA conversación en disco (si el proyecto la tiene activa): al reabrir, en vez de
        // releer todo el historial. Las imágenes del historial van con su huella.
        let conversacion = req.conversacion.as_ref().and_then(|d| Some(d.join(self.clave.as_ref()?)));
        // si la memoria rápida no se puede usar, el porqué (el agente lo dice si hubo que releer mucho)
        let mut sin_memoria: Option<String> = None;
        // lo que la RAM puede devolver DE VERDAD: en un híbrido, la última instantánea ≤ comun (puede
        // ser 0 aunque coincidan 53k tokens); contra eso se decide si el disco sirve
        let alcanza = self.modelo.alcanzable(&self.cache, comun as i32) as usize;
        if let Some(dir) = conversacion.as_ref().filter(|_| alcanza + CONVERSACION_PASO < ids.len()) {
            match self.leer_memoria(dir, &ids, &lugares, alcanza) {
                Ok(Some((c, n))) => {
                    if depurar {
                        eprintln!("{}", tr!("[tiempos] leer la conversación ({n} de {} tok) del disco: {:?}", "[timing] reading the conversation ({n} of {} tok) from disk: {:?}", ids.len(), reloj.elapsed()));
                    }
                    self.cache = c;
                    self.modelo.marcar(&mut self.cache);
                    self.en_cache = ids[..n].to_vec();
                    self.imagenes_en_cache = lugares.iter().copied().filter(|(i, _)| *i < n).collect();
                    comun = n;
                    self.del_disco = n;
                    sin_memoria = self.nota_memoria.take();
                }
                // la de memoria ya cubre lo mismo o más
                Ok(None) => {}
                Err(porque) => sin_memoria = Some(porque),
            }
        }
        // el bloque del sistema ya calculado en otra conversación: se lee del disco
        let fin_sis = self.fin_sistema(&ids);
        let alcanza = self.modelo.alcanzable(&self.cache, comun as i32) as usize;
        if let Some(fs) = fin_sis.filter(|&fs| alcanza < fs && fs < ids.len()) {
            if let Some(c) = self.leer_prefijo(&ids[..fs]) {
                if depurar {
                    // forzar la lectura para medirla (si no, MLX la difiere al primer uso)
                    for cc in &c.capas {
                        for a in [&cc.k, &cc.v, &cc.conv, &cc.estado].into_iter().flatten() {
                            let _ = a.eval();
                        }
                    }
                    eprintln!("{}", tr!("[tiempos] leer prefijo {fs} tok: {:?}", "[timing] reading prefix {fs} tok: {:?}", reloj.elapsed()));
                }
                self.cache = c;
                self.modelo.marcar(&mut self.cache);
                self.en_cache = ids[..fs].to_vec();
                self.imagenes_en_cache.clear();
                comun = fs;
            }
        }
        // con estado recurrente se vuelve a la última instantánea ≤ comun: `desde` puede ser menor
        let desde = self.modelo.recortar(&mut self.cache, comun as i32)? as usize;
        self.en_cache.truncate(desde);
        let total = (ids.len() - desde) as u32;
        // donde abre el último mensaje (lo de antes es historial que volverá idéntico)
        // (en precarga, el corte es el final: todo lo leído es historial que volverá idéntico)
        let corte = if self.precargando { Some(ids.len()) } else { self.inicio_turno.and_then(|t| ids.iter().rposition(|&x| x == t)) }.filter(|&c| c > desde);
        let mut logits = None;
        let mut i = desde;
        let guardar_sis = fin_sis.filter(|&fs| fs > desde && self.ruta_prefijo(&ids[..fs]).is_some_and(|r| !r.exists()));
        if depurar {
            eprintln!("{}", tr!("[tiempos] conversación: {:?} · corte {corte:?} · desde {desde} · {} tok", "[timing] conversation: {:?} · cut {corte:?} · from {desde} · {} tok", conversacion, ids.len()));
        }
        while i < ids.len() {
            let mut j = (i + BLOQUE).min(ids.len());
            for c in [corte, guardar_sis].into_iter().flatten() {
                if c > i && c < j {
                    j = c;
                }
            }
            let l = match &img {
                Some(im) => {
                    let extra = Extra { rasgos: Some((&im.rasgos, im.cual[i..j].to_vec())), pos: im.pos.as_ref().map(|p| [p[0][i..j].to_vec(), p[1][i..j].to_vec(), p[2][i..j].to_vec()]) };
                    self.modelo.adelante_ext(&ids[i..j], &extra, &mut self.cache)?
                }
                None => self.modelo.adelante(&ids[i..j], &mut self.cache)?,
            };
            l.eval().m()?;
            self.en_cache.extend_from_slice(&ids[i..j]);
            logits = Some(l);
            if Some(j) == corte || j == ids.len() || Some(j) == guardar_sis {
                self.modelo.marcar(&mut self.cache);
            }
            if Some(j) == guardar_sis {
                self.guardar_prefijo(&ids[..j]);
            }
            // el historial hasta tu último mensaje volverá idéntico: es lo que se guarda
            if let (Some(dir), true) = (conversacion.as_ref(), Some(j) == corte) {
                self.guardar_memoria(dir, &ids[..j], &lugares);
            }
            if total as usize > BLOQUE {
                on(StreamEvent::Prefill { done: (j - desde) as u32, total });
            }
            if salida.is_closed() {
                bail!("cancelado");
            }
            i = j;
        }
        if self.precargando {
            // F4: listo en memoria (y guardado al pasar por el corte); no se genera nada
            if depurar {
                eprintln!("{}", tr!("[tiempos] precarga de {} tok ({} del disco, {} leídos): {:?}", "[timing] prefill of {} tok ({} from disk, {} read): {:?}", ids.len(), self.del_disco, ids.len() - desde, reloj.elapsed()));
            }
            self.imagenes_en_cache = lugares;
            return Ok(ChatResponse { prompt_tokens: ids.len() as u32, cached_tokens: desde as u32, usage_exact: true, finish_reason: "precarga".into(), memoria_rapida: sin_memoria, ..Default::default() });
        }
        let mut logits = logits.ok_or_else(|| anyhow!(tr!("sin logits", "no logits")))?;
        if depurar {
            eprintln!("{}", tr!("[tiempos] prellenado de {} tok (desde {desde}): {:?} acumulado", "[timing] prefilled {} tok (from {desde}): {:?} cumulative", ids.len() - desde, reloj.elapsed()));
        }
        self.imagenes_en_cache = lugares;
        // tras una imagen, las posiciones siguen desde la mayor + 1 (no desde el número de tokens)
        let mut sig_pos = img.as_ref().and_then(|i| i.pos.as_ref()).map(|p| p.iter().flat_map(|e| e.iter()).copied().max().unwrap_or(-1) + 1);
        let mut lector = Lector::default();
        // F1-A: el lector tipa también los argumentos de las herramientas anunciadas en la
        // conversación (la plantilla sólo ve `tools`, que queda fija)
        lector.herramientas = req.herramientas_todas();
        lector.etiquetas = self.plantilla.etiquetas;
        let mut generados: Vec<u32> = Vec::new();
        let mut visto = String::new();
        let mut razon_fin = "length";
        let temp = req.sampling.temperature;
        let semilla = mlx_rs::random::key(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(0)).m()?;
        let mut clave = semilla;
        for _ in 0..req.max_tokens.max(1) {
            // M3: si la voz está generando una frase, el modelo espera su turno de GPU (antes de
            // evaluar el token: no queda trabajo suyo en la GPU mientras tanto)
            ceder_a_la_voz();
            let siguiente: u32 = if temp <= 0.0 {
                mlx_rs::ops::indexing::argmax_axis(&logits, -1, false).m()?.try_item_cast::<u32>().m()?
            } else {
                let (k1, k2) = mlx_rs::random::split(&clave, 2).m()?;
                clave = k1;
                let escalados = mlx_rs::ops::multiply(&logits, mlx_rs::Array::from_f32(1.0 / temp)).m()?;
                mlx_rs::random::categorical(&escalados, -1, None, &k2).m()?.try_item_cast::<u32>().m()?
            };
            if self.fin.contains(&siguiente) {
                razon_fin = "stop";
                break;
            }
            generados.push(siguiente);
            // texto nuevo: se decodifica todo lo generado y se emite la diferencia (UTF-8 seguro)
            let texto = self.tok.decode(&generados, false).map_err(|e| anyhow!("decodificar: {e}"))?;
            if texto.len() > visto.len() && texto.starts_with(&visto) && !texto.ends_with('\u{FFFD}') {
                let nuevo = texto[visto.len()..].to_string();
                lector.trozo(&nuevo, &on);
                visto = texto;
            }
            if salida.is_closed() {
                bail!("cancelado");
            }
            logits = match sig_pos.as_mut() {
                Some(p) => {
                    let extra = Extra { rasgos: None, pos: Some([vec![*p], vec![*p], vec![*p]]) };
                    *p += 1;
                    self.modelo.adelante_ext(&[siguiente], &extra, &mut self.cache)?
                }
                None => self.modelo.adelante(&[siguiente], &mut self.cache)?,
            };
            self.en_cache.push(siguiente);
            // Fuera de una llamada (texto o razonamiento) — prueba Cable, 2026-11-12: con un tope de
            // 16 384 el 35B razonó en círculo 13 minutos repitiendo el mismo párrafo.
            if !lector.en_llamada() {
                // tope ELÁSTICO: el texto se corta en TOPE_TEXTO; sólo una llamada (un archivo que se
                // escribe) puede seguir hasta `max_tokens`
                if req.max_tokens as usize > TOPE_TEXTO && generados.len() >= TOPE_TEXTO {
                    razon_fin = "tope_texto";
                    break;
                }
                // el mismo tramo repetido 3 veces = bucle: se corta ya, no al llegar al tope
                if generados.len() % 32 == 0 && repite_tramo(&visto) {
                    razon_fin = "repeticion";
                    break;
                }
            }
        }
        if depurar {
            eprintln!("{}", tr!("[tiempos] generados {} tok: {:?} acumulado", "[timing] generated {} tok: {:?} cumulative", generados.len(), reloj.elapsed()));
        }
        // fin de la respuesta: si el turno siguiente la repite tal cual, sólo se lee lo nuevo
        self.modelo.marcar(&mut self.cache);
        let texto = self.tok.decode(&generados, false).map_err(|e| anyhow!("decodificar: {e}"))?;
        if texto.len() > visto.len() && texto.starts_with(&visto) {
            lector.trozo(&texto[visto.len()..], &on);
        }
        lector.terminar(&on);
        // Si la respuesta llegó al tope, se dice «length» AUNQUE el lector haya cerrado una llamada a
        // medias: así el agente sabe que la ÚLTIMA llamada está cortada y no la ejecuta. Medido
        // (prueba Cable, 2026-11-12): con «tool_calls» el agente veía un write_file sin `content` y el
        // 9B lo repitió 91 veces hasta agotar los 100 pasos.
        let finish = if razon_fin == "length" {
            "length"
        } else if !lector.llamadas.is_empty() {
            "tool_calls"
        } else {
            razon_fin
        };
        Ok(ChatResponse {
            content: lector.texto.trim().to_string(),
            tokens: generados.len() as u32,
            prompt_tokens: ids.len() as u32,
            cached_tokens: desde as u32,
            usage_exact: true,
            finish_reason: finish.to_string(),
            tool_calls: lector.llamadas,
            memoria_rapida: sin_memoria,
            ..Default::default()
        })
    }
}

/// Por qué la caché guardada de la conversación no sirve para este prompt (F1.a del plano de la
/// memoria rápida). Se calcula sólo cuando falla la lectura: lee los tokens guardados (~300 KB).
fn causa_sin_memoria(base: &Path, ids: &[u32], lugares: &[(usize, u64)], fin_sistema: Option<usize>) -> String {
    let Ok(b) = std::fs::read(base.with_extension("ids")) else {
        // ¿hay memoria en la sesión, pero de otro modelo? (cada modelo guarda con su clave)
        let otras = base.parent().and_then(|d| std::fs::read_dir(d).ok()).map(|r| r.flatten().any(|e| e.path().extension().is_some_and(|x| x == "ids"))).unwrap_or(false);
        return if otras {
            tr!("la memoria guardada es de otro modelo", "the saved memory belongs to another model")
        } else {
            tr!("aún no hay memoria guardada de esta conversación", "there is no saved memory for this conversation yet")
        };
    };
    let guardados: Vec<u32> = b.as_chunks::<4>().0.iter().map(|c| u32::from_le_bytes(*c)).collect();
    let p = guardados.iter().zip(ids).take_while(|(a, b)| a == b).count();
    if p < guardados.len() {
        return diverge(p, guardados.len(), fin_sistema);
    }
    if guardados.len() >= ids.len() {
        return tr!(
            "la guardada ({} tokens) es más larga que esta conversación ({})",
            "the saved one ({} tokens) is longer than this conversation ({})",
            guardados.len(),
            ids.len()
        );
    }
    let imgs: Vec<(usize, u64)> = std::fs::read(base.with_extension("imgs")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    let ahora: Vec<(usize, u64)> = lugares.iter().copied().filter(|(i, _)| *i < guardados.len()).collect();
    if imgs != ahora {
        return tr!("las imágenes de la conversación no son las mismas", "the conversation's images are not the same");
    }
    tr!(
        "el archivo de la memoria no se pudo leer (¿a medias o de otra versión?)",
        "the memory file could not be read (half-written or from another version?)"
    )
}

/// «Cambió el inicio»: dónde difiere lo guardado del prompt de ahora.
fn diverge(p: usize, total: usize, fin_sistema: Option<usize>) -> String {
    let donde = match fin_sistema {
        Some(f) if p < f => tr!("en las instrucciones o la lista de herramientas", "in the instructions or the tool list"),
        _ => tr!(
            "en el historial (se compactó, se rebobinó o cambió un mensaje)",
            "in the history (it was compacted, rewound or a message changed)"
        ),
    };
    tr!(
        "cambió el inicio: la guardada difiere en el token {p} de {total} ({donde})",
        "the start changed: the saved one differs at token {p} of {total} ({donde})"
    )
}

#[async_trait]
impl Engine for NativoEngine {
    fn name(&self) -> String {
        format!("MLX nativo · {}", self.nombre)
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities { text: true, vision: self.ve, audio_in: self.oye }
    }

    async fn start(&mut self) -> Result<()> {
        if self.tx.lock().unwrap().is_some() {
            return Ok(());
        }
        NativoEngine::puede(&self.dir).map_err(|e| anyhow!(e))?;
        let (tx, rx) = std::sync::mpsc::channel::<Trabajo>();
        let (listo_tx, listo_rx) = std::sync::mpsc::channel::<std::result::Result<(), String>>();
        let dir = self.dir.clone();
        let hilo = std::thread::Builder::new()
            .name("gotv-mlx".into())
            .spawn(move || {
                fijar_metallib();
                if std::env::var_os("GOTV_SIN_TOPE_CACHE").is_none() {
                    let _ = mlx_rs::memory::set_cache_limit(CACHE_MLX);
                }
                // carga + CALENTAMIENTO: un token por el modelo ANTES de decir «listo». Medido en la
                // Air (macOS 26): un metallib incompatible reventaba en la primera respuesta; ahora
                // revienta aquí y GOTV vuelve al servidor. MLX hace `unwrap` en algunas funciones:
                // un pánico también se atrapa.
                let carga = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<Trabajador> {
                    let mut t = Trabajador::cargar(&dir)?;
                    let l = t.modelo.adelante(&[0], &mut t.cache)?;
                    l.eval().m()?;
                    t.cache = t.modelo.cache_nueva();
                    Ok(t)
                }));
                let mut t = match carga {
                    Ok(Ok(t)) => {
                        let _ = listo_tx.send(Ok(()));
                        t
                    }
                    Ok(Err(e)) => {
                        let _ = mlx_rs::memory::clear_cache();
                        let _ = listo_tx.send(Err(format!("{e:#}")));
                        return;
                    }
                    Err(p) => {
                        let _ = listo_tx.send(Err(texto_panico(&p)));
                        return;
                    }
                };
                while let Ok(trabajo) = rx.recv() {
                    match trabajo {
                        Trabajo::Generar { req, salida } => {
                            // los pesos FIJOS en memoria mientras se genera (como `wired_limit` de mlx_lm):
                            // con presión de memoria macOS no los manda al swap a mitad de respuesta
                            let antes = if std::env::var_os("GOTV_SIN_WIRED").is_some() { None } else { trabajo_recomendado().and_then(|w| mlx_rs::memory::set_wired_limit(w).ok()) };
                            let r = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| t.generar(&req, &salida))) {
                                Ok(r) => r.map_err(|e| format!("{e:#}")),
                                Err(p) => Err(texto_panico(&p)),
                            };
                            if let Some(a) = antes {
                                let _ = mlx_rs::memory::set_wired_limit(a);
                            }
                            if r.is_err() {
                                // la caché pudo quedar a medias: se descarta
                                t.cache = t.modelo.cache_nueva();
                                t.en_cache.clear();
                                t.imagenes_en_cache.clear();
                            }
                            let _ = salida.send(Salida::Fin(r));
                        }
                        Trabajo::Precargar { req, salida } => {
                            let antes = if std::env::var_os("GOTV_SIN_WIRED").is_some() { None } else { trabajo_recomendado().and_then(|w| mlx_rs::memory::set_wired_limit(w).ok()) };
                            t.precargando = true;
                            let r = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| t.generar(&req, &salida))) {
                                Ok(r) => r.map_err(|e| format!("{e:#}")),
                                Err(p) => Err(texto_panico(&p)),
                            };
                            t.precargando = false;
                            if let Some(a) = antes {
                                let _ = mlx_rs::memory::set_wired_limit(a);
                            }
                            if r.is_err() {
                                t.cache = t.modelo.cache_nueva();
                                t.en_cache.clear();
                                t.imagenes_en_cache.clear();
                            }
                            let r = r.map(|c| gotv_engine::Precarga {
                                admitida: c.finish_reason != "no-prefill",
                                tokens: c.prompt_tokens as usize,
                                del_disco: t.del_disco,
                                calculados: c.prompt_tokens.saturating_sub(c.cached_tokens) as usize,
                                bytes: t.disco.as_ref().map(|d| d.indice.bytes()).unwrap_or(0),
                                nota: c.memoria_rapida,
                            });
                            let _ = salida.send(Salida::Precargada(r));
                        }
                        Trabajo::Parar => break,
                    }
                }
                // el modelo se suelta y su memoria vuelve al sistema (no a la caché de MLX)
                drop(t);
                let _ = mlx_rs::memory::clear_cache();
            })
            .context(tr!("no pude crear el hilo del modelo", "could not create the model thread"))?;
        let r = tokio::task::spawn_blocking(move || listo_rx.recv()).await.map_err(|e| anyhow!("{e}"))?;
        match r {
            Ok(Ok(())) => {
                *self.tx.lock().unwrap() = Some(tx);
                *self.hilo.lock().unwrap() = Some(hilo);
                Ok(())
            }
            Ok(Err(e)) => {
                let _ = hilo.join();
                bail!("{e}")
            }
            Err(_) => bail!(tr!("el hilo del modelo murió al cargar", "the model thread died while loading")),
        }
    }

    async fn complete(&self, req: ChatRequest) -> Result<ChatResponse> {
        self.complete_stream(req, &|_| {}).await
    }

    async fn complete_stream(&self, req: ChatRequest, on: &(dyn Fn(StreamEvent) + Send + Sync)) -> Result<ChatResponse> {
        let (tx_s, mut rx_s) = tokio::sync::mpsc::unbounded_channel();
        {
            let g = self.tx.lock().unwrap();
            let tx = g.as_ref().ok_or_else(|| anyhow!(tr!("el motor nativo no está encendido", "the native engine is not running")))?;
            tx.send(Trabajo::Generar { req, salida: tx_s }).map_err(|_| anyhow!(tr!("el hilo del modelo ya no responde", "the model thread no longer responds")))?;
        }
        while let Some(s) = rx_s.recv().await {
            match s {
                Salida::Evento(e) => on(e),
                Salida::Fin(r) => return r.map_err(|e| anyhow!(e)),
                Salida::Precargada(_) => bail!(tr!("respuesta de precarga a una petición normal", "prefill answer to a normal request")),
            }
        }
        bail!(tr!("el hilo del modelo se cayó a mitad de la respuesta", "the model thread crashed mid-answer"))
    }

    async fn precargar(&self, req: ChatRequest, on: &(dyn Fn(StreamEvent) + Send + Sync)) -> Result<gotv_engine::Precarga> {
        let (tx_s, mut rx_s) = tokio::sync::mpsc::unbounded_channel();
        {
            let g = self.tx.lock().unwrap();
            let tx = g.as_ref().ok_or_else(|| anyhow!(tr!("el motor nativo no está encendido", "the native engine is not running")))?;
            tx.send(Trabajo::Precargar { req, salida: tx_s }).map_err(|_| anyhow!(tr!("el hilo del modelo ya no responde", "the model thread no longer responds")))?;
        }
        while let Some(s) = rx_s.recv().await {
            match s {
                Salida::Evento(e) => on(e),
                Salida::Precargada(r) => return r.map_err(|e| anyhow!(e)),
                Salida::Fin(_) => bail!(tr!("respuesta normal a una precarga", "normal answer to a prefill")),
            }
        }
        bail!(tr!("el hilo del modelo se cayó a mitad de la precarga", "the model thread crashed mid-prefill"))
    }

    /// Otro `Sender` al MISMO hilo del modelo: el servicio de API atiende con el modelo ya cargado
    /// (los trabajos van en fila con los del agente). El asa no tiene hilo propio: soltarla no apaga
    /// nada; sólo el dueño lo apaga con `stop`.
    fn compartir(&self) -> Option<std::sync::Arc<dyn Engine>> {
        let tx = self.tx.lock().unwrap().clone()?;
        Some(std::sync::Arc::new(NativoEngine { dir: self.dir.clone(), nombre: self.nombre.clone(), ve: self.ve, oye: self.oye, tx: Mutex::new(Some(tx)), hilo: Mutex::new(None) }))
    }

    async fn stop(&mut self) -> Result<()> {
        if let Some(tx) = self.tx.lock().unwrap().take() {
            let _ = tx.send(Trabajo::Parar);
        }
        let hilo = self.hilo.lock().unwrap().take();
        if let Some(h) = hilo {
            let _ = tokio::task::spawn_blocking(move || h.join()).await;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    /// Prueba Cable (2026-11-12): el bucle real del 35B se detecta; un texto largo que no se repite no.
    #[test]
    fn el_tramo_repetido_se_detecta() {
        let parrafo = "Ah, el problema es que `almacen.pagar` hace `SELECT COALESCE(SUM(multa), 0) FROM prestamo WHERE socio_id = ? AND activo = 0` y obtiene `deuda = 3000`. Pero luego actualiza `socio.deuda = MAX(0, deuda - monto)`. Y luego devuelve 2000.\n\nPero el test dice que devuelve 0. ¿Por qué?\n\n";
        assert!(!super::repite_tramo(&parrafo.repeat(2)), "dos veces todavía no es bucle");
        assert!(super::repite_tramo(&parrafo.repeat(4)), "cuatro veces el mismo párrafo es bucle");
        let distinto: String = (0..200).map(|i| format!("Paso {i}: reviso la función número {i} y su prueba {}.\n", i * 7)).collect();
        assert!(!super::repite_tramo(&distinto), "un texto largo que avanza no es bucle");
        assert!(!super::repite_tramo(&"-".repeat(2000)), "una raya larga no cuenta como texto repetido");
    }

    /// Sonda «sólo Rust»: qué motor le toca a cada modelo con el núcleo por defecto, con las MISMAS
    /// funciones que usan la CLI y la app. `GOTV_SONDA_MODELOS=<dir>:<dir>…`
    #[test]
    #[ignore]
    fn sonda_motor_por_modelo() {
        let Ok(l) = std::env::var("GOTV_SONDA_MODELOS") else { return };
        for d in l.split(':').filter(|s| !s.is_empty()) {
            let dir = PathBuf::from(d);
            let nombre = dir.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            let (ve, oye) = (dir.join("preprocessor_config.json").is_file() || super::NativoEngine::ve(&dir), super::NativoEngine::oye(&dir));
            let motor = match (super::NativoEngine::puede(&dir), super::sentido_que_falta(&dir, ve, oye), super::exige_nativo(&dir)) {
                (_, _, Some(_)) => "NÚCLEO (exigido)".to_string(),
                (Ok(()), None, None) => "NÚCLEO".to_string(),
                (Ok(()), Some(f), None) => format!("SERVIDOR PYTHON: {f}"),
                (Err(e), _, None) => format!("SERVIDOR PYTHON: {e}"),
            };
            eprintln!("{nombre:<52} {motor}");
        }
    }

    /// Sonda de generación corta en el núcleo (voraz, plantilla de chat del modelo):
    /// `GOTV_SONDA_GENERAR=<dir>` y `GOTV_SONDA_PREGUNTA=<texto>`.
    #[test]
    #[ignore]
    fn sonda_generar_corto() {
        use crate::modelo::M;
        let Ok(d) = std::env::var("GOTV_SONDA_GENERAR") else { return };
        let dir = PathBuf::from(d);
        let pregunta = std::env::var("GOTV_SONDA_PREGUNTA").unwrap_or_else(|_| "What is the capital of France?".into());
        let t0 = std::time::Instant::now();
        let m = crate::modelo::Modelo::cargar(&dir).unwrap();
        let tk = tokenizers::Tokenizer::from_file(dir.join("tokenizer.json")).unwrap();
        let pl = crate::plantilla::Plantilla::cargar(&dir).unwrap();
        let prompt = pl.render_simple(&[("user", &pregunta)]).unwrap();
        let mut ids = tk.encode(prompt.as_str(), false).unwrap().get_ids().to_vec();
        eprintln!("cargado en {:.1} s; {} tokens de prompt", t0.elapsed().as_secs_f32(), ids.len());
        let mut c = m.cache_nueva();
        let mut salida = Vec::new();
        let t1 = std::time::Instant::now();
        let mut entrada = ids.clone();
        for _ in 0..40 {
            let l = m.adelante(&entrada, &mut c).unwrap().as_dtype(mlx_rs::Dtype::Float32).m().unwrap();
            l.eval().unwrap();
            let v = l.as_slice::<f32>();
            let t = v.iter().enumerate().fold(0, |b, (i, x)| if *x > v[b] { i } else { b }) as u32;
            if tk.id_to_token(t).is_some_and(|s| s.contains("eot") || s.contains("im_end") || s == "</s>") {
                break;
            }
            salida.push(t);
            ids.push(t);
            entrada = vec![t];
        }
        eprintln!("{} tokens en {:.1} s → «{}»", salida.len(), t1.elapsed().as_secs_f32(), tk.decode(&salida, true).unwrap());
    }

    /// El hilo de voz: la 2.ª frase no recarga el modelo, y `callar` corta una frase larga a mitad.
    /// `GOTV_TTS_DIR=<Qwen3-TTS Base> cargo test -p gotv-mlx --release -- --ignored --nocapture voz_en_hilo`
    #[test]
    #[ignore]
    fn voz_en_hilo_reutiliza_y_calla() {
        let Ok(d) = std::env::var("GOTV_TTS_DIR") else { return };
        let d = PathBuf::from(d);
        let nada = Callar::default();
        let h1 = hablar(&d, "Primera frase.", "spanish", true, &nada, &mut |_| Ok(())).unwrap();
        let h2 = hablar(&d, "Segunda frase.", "spanish", true, &nada, &mut |_| Ok(())).unwrap();
        println!("carga 1.ª {:?} · 2.ª {:?} · {} y {} fotogramas", h1.carga, h2.carga, h1.fotogramas, h2.fotogramas);
        assert!(h2.carga < std::time::Duration::from_millis(50), "la segunda no recarga");
        assert!(h1.fotogramas > 5 && h2.fotogramas > 5);
        // callar tras el primer trozo de una frase larga
        let c = Callar::default();
        let t = std::time::Instant::now();
        let mut trozos = 0;
        let larga = "Esta es una frase muy larga que no debería terminar de decirse, porque el Portador me interrumpe en cuanto empiezo a hablar y la voz tiene que callarse al instante.";
        let mut cuando = None;
        let h = hablar(&d, larga, "spanish", true, &c, &mut |_| {
            trozos += 1;
            c.ya();
            cuando.get_or_insert_with(std::time::Instant::now);
            Ok(())
        })
        .unwrap();
        let tras = cuando.map(|x| x.elapsed()).unwrap_or_default();
        println!("callada tras {trozos} trozo(s): devolvió {:?} después de pedirlo ({} muestras, total {:?})", tras, h.muestras, t.elapsed());
        assert!(tras < std::time::Duration::from_millis(150), "la generación para en un fotograma");
        assert!(trozos <= 2, "se calló entre trozos");
        assert!(t.elapsed() < std::time::Duration::from_secs(2));
        // y el hilo sigue sano después
        let h3 = hablar(&d, "Sigo aquí.", "spanish", true, &nada, &mut |_| Ok(())).unwrap();
        assert!(h3.fotogramas > 3);
    }

    /// ¿Qué frena la voz dentro de `gotv conversar`? Decir, transcribir con Whisper y volver a decir.
    /// `GOTV_TTS_DIR=… GOTV_WHISPER=… GOTV_WAV=<wav 16k> cargo test -p gotv-mlx --release -- --ignored --nocapture voz_tras_whisper`
    #[test]
    #[ignore]
    fn voz_tras_whisper() {
        let (Ok(d), Ok(w), Ok(wav)) = (std::env::var("GOTV_TTS_DIR"), std::env::var("GOTV_WHISPER"), std::env::var("GOTV_WAV")) else { return };
        let d = PathBuf::from(d);
        let frase = "Entiendo, fue un momento difícil. Me alegra mucho que estés bien después del sismo, Portador.";
        let medir = |n: &str| {
            let h = hablar(&d, frase, "spanish", true, &Callar::default(), &mut |_| Ok(())).unwrap();
            println!("{n}: {:.1} fotogramas/s", h.fotogramas as f64 / (h.total - h.carga).as_secs_f64());
        };
        medir("1 · sola");
        medir("2 · sola otra vez");
        let t = std::time::Instant::now();
        let txt = transcribir_wav(Path::new(&w), &std::fs::read(&wav).unwrap()).unwrap();
        println!("   Whisper {:?}: {}", t.elapsed(), txt.chars().take(40).collect::<String>());
        medir("3 · tras Whisper");
        medir("4 · tras Whisper, otra vez");
    }

    #[test]
    fn un_adjunto_que_el_modelo_no_entiende_viaja_como_nota() {
        use gotv_engine::Message;
        let audio = "data:audio/wav;base64,AAAA".to_string();
        let img = "data:image/png;base64,BBBB".to_string();
        let msgs = vec![Message::user("hola"), Message::user_with_media("(adjuntos del usuario: voz.m4a)", vec![audio.clone()]), Message::user_with_media("mira", vec![img.clone()])];
        assert!(sin_sentidos_que_faltan(&msgs, true, true).is_none(), "si ve y oye, nada cambia");
        // el 9B: ve, no oye → el audio se va con su nota; la imagen se queda
        let m = sin_sentidos_que_faltan(&msgs, true, false).unwrap();
        assert!(m[1].audio.is_empty() && m[1].images.is_empty());
        assert!(m[1].content.starts_with("(adjuntos del usuario: voz.m4a)") && m[1].content.contains("1 audio(s)"), "{}", m[1].content);
        assert_eq!(m[2].images, vec![img.clone()]);
        assert_eq!(m[0].content, "hola");
        // un modelo de sólo texto: fuera las dos
        let m = sin_sentidos_que_faltan(&msgs, false, false).unwrap();
        assert!(m[2].images.is_empty() && m[2].content.contains("1 imagen(es)"));
    }

    #[test]
    fn la_voz_embebida_dice_su_texto() {
        // el taller (herramientas/voz, que NO viaja en el repo público) guarda cómo se hizo la voz: donde
        // esté, dice lo mismo que el binario; sin él (un clon del repo público) la prueba sigue compilando
        let taller = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../herramientas/voz/voz-gotv/voz.json");
        if let Ok(t) = std::fs::read_to_string(&taller) {
            let j: serde_json::Value = serde_json::from_str(&t).unwrap();
            assert_eq!(j["texto_de_la_referencia"].as_str(), Some(VOZ_GOTV_TEXTO), "voz.json y el binario dicen lo mismo");
        }
        let w = wav_pcm16(&[0.0, 1.0, -1.0], 24000);
        assert_eq!((&w[0..4], w.len()), (&b"RIFF"[..], 50));
        assert_eq!(i16::from_le_bytes([w[46], w[47]]), 32767);
    }

    use super::*;
    use gotv_engine::{Message, Sampling};

    /// `GOTV_MLX_DIR` elige el modelo (p. ej. el 9B híbrido); si no, el Qwen3-0.6B de la caché.
    fn dir_prueba() -> Option<PathBuf> {
        let d = match std::env::var("GOTV_MLX_DIR") {
            Ok(d) => PathBuf::from(d),
            Err(_) => PathBuf::from(std::env::var("HOME").ok()?).join(".cache/gotv/models/mlx-community/Qwen3-0.6B-4bit"),
        };
        d.exists().then_some(d)
    }

    fn req(msgs: Vec<Message>, tools: Option<serde_json::Value>, max: u32) -> ChatRequest {
        ChatRequest { messages: msgs, max_tokens: max, sampling: Sampling { temperature: 0.0, top_p: None, top_k: None, min_p: None }, tools, conversacion: None, herramientas_aparte: None }
    }

    /// Con el modelo real (Qwen3-0.6B-4bit en la caché de GOTV): responde, llama herramientas y
    /// reutiliza el prefijo entre turnos. `cargo test -p gotv-mlx -- --ignored --nocapture`
    /// La caché de la CONVERSACIÓN en disco: un motor nuevo (como al reabrir la app) lee el historial
    /// del disco en vez de recalcularlo, y la respuesta voraz es IDÉNTICA.
    /// `GOTV_MLX_DIR=<modelo> GOTV_HISTORIAL=<json> cargo test -p gotv-mlx --release -- --ignored --nocapture conversacion_desde_disco`
    #[tokio::test]
    #[ignore]
    async fn conversacion_desde_disco() {
        let (Some(dir), Ok(h)) = (dir_prueba(), std::env::var("GOTV_HISTORIAL")) else { return };
        let msgs: Vec<Message> = serde_json::from_str(&std::fs::read_to_string(h).unwrap()).unwrap();
        // como en producción: <base>/<proyecto>/s<sesión> (la poda de sesiones mira dos niveles arriba)
        let raiz = gotv_engine::prueba::CarpetaPrueba::sin_crear("gotv-conv-motor");
        let carpeta = raiz.join("proyecto").join("s1");
        let _ = std::fs::remove_dir_all(&carpeta);
        let mut pedido = req(msgs, None, 40);
        pedido.conversacion = Some(carpeta.clone());
        let mut respuestas = Vec::new();
        for vuelta in ["1 · fría (lee todo y guarda)", "2 · motor nuevo (lee del disco)"] {
            let mut e = NativoEngine::new(&dir);
            e.start().await.unwrap();
            let t = std::time::Instant::now();
            let r = e.complete(pedido.clone()).await.unwrap();
            println!("{vuelta}: {:?} · {} tok del prompt, {} de caché · {:?}", t.elapsed(), r.prompt_tokens, r.cached_tokens, r.content.chars().take(60).collect::<String>());
            respuestas.push((r.content, r.cached_tokens, t.elapsed()));
            e.stop().await.unwrap();
        }
        let peso: u64 = std::fs::read_dir(&carpeta).into_iter().flatten().flat_map(|e| e.ok()).map(|e| e.metadata().map(|m| m.len()).unwrap_or(0)).sum();
        println!("en disco: {:.2} GB", peso as f64 / 1e9);
        let _ = std::fs::remove_dir_all(carpeta.parent().and_then(|p| p.parent()).unwrap());
        assert_eq!(respuestas[0].0, respuestas[1].0, "la misma respuesta");
        assert!(respuestas[1].1 > respuestas[0].1 + 1000, "la 2.ª lee el historial del disco");
        assert!(respuestas[1].2 * 5 < respuestas[0].2, "y es mucho más rápida");
    }

    /// Una conversación sintética larga (≈ 6 000 tokens) con herramientas, para las pruebas de la
    /// memoria rápida: no depende de ningún historial del Portador.
    fn conversacion_larga(turnos: usize) -> Vec<Message> {
        let mut m = vec![Message::system("Eres un asistente de programación. Responde breve.")];
        for i in 0..turnos {
            let relleno: String = (0..60).map(|j| format!("La función número {i}-{j} suma dos enteros y devuelve el resultado; ")).collect();
            m.push(Message::user(format!("Paso {i}: revisa este código. {relleno}")));
            m.push(Message::assistant(format!("Revisado el paso {i}: todo correcto. {}", &relleno[..relleno.len() / 2])));
        }
        m.push(Message::user("¿Cuántos pasos revisaste? Responde con un número."));
        m
    }

    fn herramientas(n: usize) -> serde_json::Value {
        let todas = ["leer", "escribir", "buscar", "navegar"];
        serde_json::Value::Array(
            todas[..n]
                .iter()
                .map(|h| serde_json::json!({"type": "function", "function": {"name": h, "description": format!("{h} un archivo"), "parameters": {"type": "object", "properties": {"ruta": {"type": "string"}}, "required": ["ruta"]}}}))
                .collect(),
        )
    }

    /// F1 del plano de la memoria rápida, con el modelo real: (1) un motor NUEVO (reabrir la app)
    /// lee la conversación del disco y sólo prellena la cola; (2) si cambia la lista de
    /// herramientas, el núcleo DICE por qué no la usó (antes releía en silencio minutos enteros);
    /// (3) sin nada guardado, también lo dice. 0.6B por omisión (segundos), o `GOTV_MLX_DIR=<9B>`.
    /// `cargo test -p gotv-mlx --release -- --ignored --nocapture memoria_rapida_se_usa_y_dice_por_que_no`
    #[tokio::test]
    #[ignore]
    async fn memoria_rapida_se_usa_y_dice_por_que_no() {
        let Some(dir) = dir_prueba() else { return };
        // como en producción: <base>/<proyecto>/s<sesión> (la poda de sesiones mira dos niveles arriba)
        let raiz = gotv_engine::prueba::CarpetaPrueba::sin_crear("gotv-memoria-f1");
        let carpeta = raiz.join("proyecto").join("s1");
        let _ = std::fs::remove_dir_all(&carpeta);
        let mut pedido = req(conversacion_larga(8), Some(herramientas(3)), 8);
        pedido.conversacion = Some(carpeta.clone());
        let correr = |p: ChatRequest| {
            let dir = dir.clone();
            async move {
                let mut e = NativoEngine::new(&dir);
                e.start().await.unwrap();
                let r = e.complete(p).await.unwrap();
                e.stop().await.unwrap();
                r
            }
        };
        // (3) nada guardado aún
        let r1 = correr(pedido.clone()).await;
        println!("1 · fría: {} tok, {} de caché · {:?}", r1.prompt_tokens, r1.cached_tokens, r1.memoria_rapida);
        assert!(r1.prompt_tokens > 5000, "la conversación de prueba es larga: {}", r1.prompt_tokens);
        assert!(r1.memoria_rapida.as_deref().is_some_and(|c| c.contains("aún no hay")), "{:?}", r1.memoria_rapida);
        // (1) motor nuevo, misma conversación y lista: del disco, sólo la cola
        let r2 = correr(pedido.clone()).await;
        let cola = r2.prompt_tokens - r2.cached_tokens;
        println!("2 · reabrir: {} tok, {} de caché (cola {cola}) · {:?}", r2.prompt_tokens, r2.cached_tokens, r2.memoria_rapida);
        assert!(r2.memoria_rapida.is_none(), "{:?}", r2.memoria_rapida);
        assert!(cola < 1100, "sólo se prellena la cola: {cola}");
        assert_eq!(r1.content, r2.content, "la misma respuesta");
        // (2) cambia la lista de herramientas → lo dice, y señala dónde
        let mut otra = pedido.clone();
        otra.tools = Some(herramientas(4));
        let r3 = correr(otra).await;
        println!("3 · otra lista: {} tok, {} de caché · {:?}", r3.prompt_tokens, r3.cached_tokens, r3.memoria_rapida);
        assert!(r3.memoria_rapida.as_deref().is_some_and(|c| c.contains("herramientas")), "{:?}", r3.memoria_rapida);
        let _ = std::fs::remove_dir_all(carpeta.parent().and_then(|p| p.parent()).unwrap());
    }

    /// F3 con el modelo real (denso 0.6B) y un tope DIMINUTO: la conversación no cabe entera → se guarda
    /// el prefijo que cabe, se desaloja primero la memoria vieja de otra sesión, nunca se pasa del 90 %
    /// del tope, y al reabrir se lee ese prefijo y se dice por qué no está todo.
    /// `cargo test -p gotv-mlx --release -- --ignored --nocapture memoria_con_tope`
    #[tokio::test]
    #[ignore]
    async fn memoria_con_tope() {
        use gotv_engine::memoria_tope as tope;
        let Some(dir) = dir_prueba() else { return };
        let raiz = gotv_engine::prueba::CarpetaPrueba::sin_crear("gotv-memoria-f3");
        let _ = std::fs::remove_dir_all(&raiz);
        let proyecto = raiz.join("proyecto");
        let carpeta = proyecto.join("s2");
        // una memoria VIEJA de otra sesión (100 MB, usada hace una hora)
        let vieja = proyecto.join("s1").join("otra-clave");
        std::fs::create_dir_all(&vieja).unwrap();
        std::fs::write(vieja.join("t-0000000.gkv"), vec![0u8; 100 << 20]).unwrap();
        std::fs::write(vieja.join("indice.gotv"), b"{}").unwrap();
        std::fs::File::options().write(true).open(vieja.join("indice.gotv")).unwrap().set_modified(std::time::SystemTime::now() - std::time::Duration::from_secs(3600)).unwrap();
        tope::fijar_tope(&proyecto, 400 << 20).unwrap(); // utilizable: 360 MB
        let mut p = req(conversacion_larga(8), Some(herramientas(3)), 8);
        p.conversacion = Some(carpeta.clone());
        let mut e = NativoEngine::new(&dir);
        e.start().await.unwrap();
        let r1 = e.complete(p.clone()).await.unwrap();
        e.stop().await.unwrap();
        let ocupado = tope::ocupado(&proyecto);
        let entrada = std::fs::read_dir(&carpeta).unwrap().flatten().next().unwrap().path();
        let hasta = memoria::Disco::abrir(&entrada).indice.hasta();
        println!("1 · {} tok · guardado el prefijo de {hasta} tok · {:.0} MB ocupados (≤ {:.0}) · vieja desalojada: {}", r1.prompt_tokens, ocupado as f64 / 1e6, tope::utilizable(400 << 20) as f64 / 1e6, !vieja.exists());
        // la regla: la vieja sale cuando hace falta sitio. Con el 0.6B (por omisión) lo nuevo no cabe junto a
        // ella; con otro modelo (Huihui, MoE híbrido: memoria chica) puede caber y entonces se QUEDA — medido
        // 2026-11-17: 345 MB con la vieja dentro, bajo el tope de 377. Lo que vale siempre: nunca se pasa.
        if std::env::var_os("GOTV_MLX_DIR").is_none() {
            assert!(!vieja.exists(), "la memoria vieja de otra sesión sale primero");
        } else {
            assert!(!vieja.exists() || ocupado <= tope::utilizable(400 << 20), "si la vieja se queda, todo cabe bajo el tope");
        }
        assert!(hasta >= 1024 && hasta < r1.prompt_tokens as usize, "se guarda un prefijo, no todo");
        assert!(ocupado <= tope::utilizable(400 << 20), "siempre queda el 10 % libre");
        // reabrir: se lee el prefijo y se dice que no cupo más
        let mut e = NativoEngine::new(&dir);
        e.start().await.unwrap();
        let r2 = e.complete(p).await.unwrap();
        e.stop().await.unwrap();
        println!("2 · reabrir: {} de caché · {:?}", r2.cached_tokens, r2.memoria_rapida);
        assert!(r2.cached_tokens as usize >= hasta);
        // el aviso de «tope» sólo existe si el tope recortó (0.6B); con Huihui cupo casi todo (el prefijo se
        // corta en el último estado recurrente guardado, no por el tope) y no hay nada que avisar
        if std::env::var_os("GOTV_MLX_DIR").is_none() {
            assert!(r2.memoria_rapida.as_deref().is_some_and(|n| n.contains("tope")), "{:?}", r2.memoria_rapida);
        }
        assert_eq!(r1.content, r2.content);
        let _ = std::fs::remove_dir_all(&raiz);
    }

    /// Una conversación de agente con llamadas a herramientas y sus resultados en el historial.
    /// El caso del Portador (2026-11-11, 9B híbrido): un turno con 11 pasos de herramienta y luego una
    /// orden NUEVA. La plantilla quita el `<think>` de los pasos anteriores al llegar la orden, así que
    /// hay que volver al inicio del turno anterior; antes esa instantánea ya no estaba (sólo 8, se
    /// soltaba la más vieja) y releía la conversación entera («trabajando… 17 %»). Ahora sólo se
    /// relee el turno anterior. `GOTV_MLX_DIR=<9B> cargo test -p gotv-mlx --release -- --ignored --nocapture turno_largo_y_orden_nueva`
    #[tokio::test]
    #[ignore]
    async fn turno_largo_y_orden_nueva_no_relee_todo() {
        let Some(dir) = dir_prueba() else { return };
        let mut e = NativoEngine::new(&dir);
        e.start().await.unwrap();
        let mut m = conversacion_larga(4);
        m.pop();
        m.push(Message::user("Abre main.rs y revisa cada módulo, uno por uno."));
        let historia = {
            let r = e.complete(req(m.clone(), Some(herramientas(3)), 1)).await.unwrap();
            r.prompt_tokens as usize
        };
        for k in 0..11 {
            let llamada = gotv_engine::ToolCall { id: format!("c{k}"), name: "leer".into(), arguments: serde_json::json!({"ruta": format!("mod{k}.rs")}) };
            m.push(Message::assistant_calls("", std::slice::from_ref(&llamada)));
            m.push(Message::tool_result(&llamada, &format!("pub fn f{k}() -> u32 {{ {k} }} // módulo {k} sin errores")));
            let r = e.complete(req(m.clone(), Some(herramientas(3)), 1)).await.unwrap();
            assert!(r.prompt_tokens - r.cached_tokens < 400, "paso {k}: dentro del turno sólo lo nuevo ({} de {})", r.prompt_tokens - r.cached_tokens, r.prompt_tokens);
        }
        m.push(Message::assistant("Revisé los 11 módulos: todos correctos."));
        m.push(Message::user("Ahora calcula 8 por 9."));
        let t = std::time::Instant::now();
        let r = e.complete(req(m, Some(herramientas(3)), 1)).await.unwrap();
        let nuevos = r.prompt_tokens - r.cached_tokens;
        println!("orden nueva: {} tok de prompt, {} de caché, {nuevos} calculados en {:?} (la historia antes del turno: {historia})", r.prompt_tokens, r.cached_tokens, t.elapsed());
        assert!(r.cached_tokens as usize >= historia - 64, "vuelve al inicio del turno anterior, no a cero");
        e.stop().await.unwrap();
    }

    fn conversacion_con_herramientas() -> Vec<Message> {
        let mut m = conversacion_larga(3);
        m.pop(); // sin la pregunta final
        let llamada = gotv_engine::ToolCall { id: "c1".into(), name: "leer".into(), arguments: serde_json::json!({"ruta": "main.rs"}) };
        m.push(Message::user("Lee main.rs y dime qué hace."));
        m.push(Message::assistant_calls("", std::slice::from_ref(&llamada)));
        m.push(Message::tool_result(&llamada, "fn main() { println!(\"hola\"); }"));
        m.push(Message::assistant("Imprime «hola»."));
        m
    }

    /// F4: la precarga lee hasta donde ABRE el mensaje postizo; eso debe ser EXACTAMENTE el inicio del
    /// prompt cuando llega el mensaje real, en cada familia (si no, la precarga no serviría). Sólo
    /// tokenizer y plantilla: rápido. Prueba las del 0.6B, el 9B y Gemma 4 si están.
    /// `cargo test -p gotv-mlx --release -- --ignored --nocapture la_precarga_corta_donde_vuelve_identico`
    #[test]
    #[ignore]
    fn la_precarga_corta_donde_vuelve_identico() {
        let home = PathBuf::from(std::env::var("HOME").unwrap());
        let mut dirs: Vec<PathBuf> = ["Qwen3-0.6B-4bit", "Qwen3.5-9B-4bit"].iter().map(|n| home.join(".cache/gotv/models/mlx-community").join(n)).collect();
        if let Some(g) = std::fs::read_dir(home.join(".cache/huggingface/hub/models--mlx-community--gemma-4-e4b-it-4bit/snapshots")).ok().and_then(|mut r| r.next()).and_then(|e| e.ok()) {
            dirs.push(g.path());
        }
        let mut probadas = 0;
        for dir in dirs.into_iter().filter(|d| d.join("tokenizer.json").exists()) {
            let tok = tokenizers::Tokenizer::from_file(dir.join("tokenizer.json")).unwrap();
            let plantilla = Plantilla::cargar(&dir).unwrap();
            let inicio = tok.token_to_id("<|im_start|>").or_else(|| tok.token_to_id("<|turn>"));
            let ids = |m: &[Message]| tok.encode(plantilla.prompt(m, Some(&herramientas(3))).unwrap().as_str(), false).unwrap().get_ids().to_vec();
            for (nombre, historial) in [("charla", conversacion_larga(3)[..7].to_vec()), ("con herramientas", conversacion_con_herramientas())] {
                let mut con_postizo = historial.clone();
                con_postizo.push(Message::user("·"));
                let mut con_real = historial.clone();
                con_real.push(Message::user("Ahora revisa el archivo de pruebas, por favor."));
                let (a, b) = (ids(&con_postizo), ids(&con_real));
                let marcas: Vec<usize> = a.iter().enumerate().filter(|(_, &x)| Some(x) == inicio).map(|(i, _)| i).collect();
                let hasta = marcas[marcas.len() - 2];
                let iguales = a[..hasta] == b[..hasta.min(b.len())];
                println!("{} · {nombre}: corta en {hasta} de {} · ¿prefijo exacto del real? {iguales}", dir.file_name().unwrap().to_string_lossy(), a.len());
                assert!(iguales, "{}: lo precargado no vuelve idéntico ({nombre})", dir.display());
            }
            probadas += 1;
        }
        assert!(probadas > 0, "no hay modelos para probar");
    }

    /// F4 con el modelo real: precargar deja la conversación lista y la primera orden sólo lee el
    /// mensaje nuevo; la respuesta es la misma que sin precarga. Y una precarga cancelada a medias no
    /// rompe nada: la orden siguiente funciona. 0.6B por omisión o `GOTV_MLX_DIR=<9B>`.
    /// `cargo test -p gotv-mlx --release -- --ignored --nocapture precarga_con_el_modelo`
    #[tokio::test]
    #[ignore]
    async fn precarga_con_el_modelo() {
        let Some(dir) = dir_prueba() else { return };
        // la guardia borra la raíz al soltarse, pase la prueba o falle
        let raiz = gotv_engine::prueba::CarpetaPrueba::sin_crear("gotv-precarga");
        let carpeta = raiz.join("proyecto").join("s1");
        let historial: Vec<Message> = conversacion_larga(8)[..16].to_vec();
        let pedir = |ultimo: &str, memoria: bool| {
            let mut m = historial.clone();
            m.push(Message::user(ultimo));
            let mut p = req(m, Some(herramientas(3)), 8);
            if memoria {
                p.conversacion = Some(carpeta.clone());
            }
            p
        };
        let pregunta = "¿Cuántos pasos revisaste? Responde con un número.";
        // 1) cancelada a medias (sin nada en disco aún): se suelta a los 300 ms
        let mut e = NativoEngine::new(&dir);
        e.start().await.unwrap();
        let cancelada = tokio::time::timeout(std::time::Duration::from_millis(300), e.precargar(pedir("·", true), &|_| {})).await;
        println!("1 · precarga cancelada: {}", if cancelada.is_err() { "sí (a los 300 ms)" } else { "terminó antes" });
        // 2) precarga completa (construye y guarda) en el MISMO motor tras la cancelación
        let t = std::time::Instant::now();
        let p = e.precargar(pedir("·", true), &|_| {}).await.unwrap();
        println!("2 · precarga: {p:?} en {:?}", t.elapsed());
        assert!(p.admitida && p.tokens > 10_000);
        e.stop().await.unwrap();
        // 3) motor nuevo (reabrir la app): precarga del DISCO y la primera orden sólo lee lo nuevo
        let mut e = NativoEngine::new(&dir);
        e.start().await.unwrap();
        let t = std::time::Instant::now();
        let p = e.precargar(pedir("·", true), &|_| {}).await.unwrap();
        let t_precarga = t.elapsed();
        let t = std::time::Instant::now();
        let r = e.complete(pedir(pregunta, true)).await.unwrap();
        let t_orden = t.elapsed();
        e.stop().await.unwrap();
        println!("3 · reabrir: precarga {} del disco de {} en {t_precarga:?} · la orden lee {} tok en {t_orden:?} → {:?}", p.del_disco, p.tokens, r.prompt_tokens - r.cached_tokens, r.content);
        assert!(p.del_disco > 10_000 && p.calculados < 300, "{p:?}");
        assert!(r.prompt_tokens - r.cached_tokens < 60, "la orden sólo lee tu mensaje");
        // 4) la misma pregunta sin memoria ni precarga
        let mut e = NativoEngine::new(&dir);
        e.start().await.unwrap();
        let sin = e.complete(pedir(pregunta, false)).await.unwrap();
        e.stop().await.unwrap();
        println!("4 · sin memoria: {:?}", sin.content);
        assert_eq!(r.content, sin.content);
    }

    /// Diagnóstico: ¿el prompt de una conversación es prefijo del de la misma conversación con un
    /// turno más? Imprime dónde difieren y el texto alrededor.
    /// `cargo test -p gotv-mlx --release -- --ignored --nocapture prompt_creciente_es_prefijo`
    #[test]
    #[ignore]
    fn prompt_creciente_es_prefijo() {
        let Some(dir) = dir_prueba() else { return };
        let tok = tokenizers::Tokenizer::from_file(dir.join("tokenizer.json")).unwrap();
        let plantilla = Plantilla::cargar(&dir).unwrap();
        let largo = conversacion_larga(8);
        let mut corto = largo[..1 + 2 * 4].to_vec();
        corto.push(Message::user("¿Cuántos pasos revisaste? Responde con un número."));
        let ids = |m: &[Message]| tok.encode(plantilla.prompt(m, Some(&herramientas(3))).unwrap().as_str(), false).unwrap().get_ids().to_vec();
        let (a, b) = (ids(&corto), ids(&largo));
        let p = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
        let ver = |v: &[u32]| tok.decode(&v[p.saturating_sub(30)..(p + 12).min(v.len())], false).unwrap();
        println!("corto {} · largo {} · difieren en {p}\n--- corto ---\n{}\n--- largo ---\n{}", a.len(), b.len(), ver(&a), ver(&b));
    }

    /// F2 con el modelo real: la memoria por TRAMOS crece añadiendo sólo lo nuevo, se lee al reabrir,
    /// aprovecha el prefijo al rebobinar y se corta al compactar. Cada respuesta debe ser IDÉNTICA a la
    /// de un motor sin memoria (temperatura 0). 0.6B por omisión, o `GOTV_MLX_DIR=<9B>` (híbrido).
    /// `cargo test -p gotv-mlx --release -- --ignored --nocapture memoria_por_tramos_con_el_modelo`
    #[tokio::test]
    #[ignore]
    async fn memoria_por_tramos_con_el_modelo() {
        let Some(dir) = dir_prueba() else { return };
        // como en producción: <base>/<proyecto>/s<sesión> (la poda de sesiones mira dos niveles arriba)
        let raiz = gotv_engine::prueba::CarpetaPrueba::sin_crear("gotv-memoria-f2");
        let carpeta = raiz.join("proyecto").join("s1");
        let _ = std::fs::remove_dir_all(&carpeta);
        let con_memoria = |msgs: Vec<Message>| {
            let mut p = req(msgs, Some(herramientas(3)), 8);
            p.conversacion = Some(carpeta.clone());
            p
        };
        let sin_memoria = |msgs: Vec<Message>| req(msgs, Some(herramientas(3)), 8);
        let fresco = |p: ChatRequest| {
            let dir = dir.clone();
            async move {
                let mut e = NativoEngine::new(&dir);
                e.start().await.unwrap();
                let r = e.complete(p).await.unwrap();
                e.stop().await.unwrap();
                r
            }
        };
        let indice = || {
            let d = std::fs::read_dir(&carpeta).unwrap().flatten().next().unwrap().path();
            memoria::Disco::abrir(&d).indice
        };
        // 1) crecer: la conversación SIGUE (4 turnos + tu 5.º mensaje, y luego hasta el final), con el
        // MISMO motor (la app abierta): sólo se añade lo nuevo. Lo guardado llega hasta donde empieza
        // la respuesta: incluye tu último mensaje, que en la vuelta siguiente vuelve igual.
        let largo = conversacion_larga(8);
        let corto_de = |l: &[Message]| l[..1 + 2 * 4 + 1].to_vec();
        let corto = corto_de(&largo);
        let mut e = NativoEngine::new(&dir);
        e.start().await.unwrap();
        e.complete(con_memoria(corto)).await.unwrap();
        let tramos_a = indice().tramos.len();
        e.complete(con_memoria(largo.clone())).await.unwrap();
        e.stop().await.unwrap();
        let ind = indice();
        println!("1 · crecer: {} tramos, hasta {} tok, {:.1} MB, {} estados", ind.tramos.len(), ind.hasta(), ind.bytes() as f64 / 1e6, ind.estados.len());
        assert!(ind.tramos.len() > tramos_a, "el 2.º turno añade tramos, no reescribe");
        assert_eq!(ind.tramos[0].desde, 0);
        // 2) reabrir: motor nuevo, del disco; respuesta idéntica a la de un motor sin memoria
        let r = fresco(con_memoria(largo.clone())).await;
        let referencia = fresco(sin_memoria(largo.clone())).await;
        println!("2 · reabrir: {} tok, {} de caché · {:?} = {:?}", r.prompt_tokens, r.cached_tokens, r.content, referencia.content);
        assert!(r.prompt_tokens - r.cached_tokens < 1100, "sólo la cola");
        assert_eq!(r.content, referencia.content);
        // 3) rebobinar al turno 6 y preguntar otra cosa: se aprovecha lo guardado de antes
        let mut rebobinado = largo[..1 + 2 * 6].to_vec();
        rebobinado.push(Message::user("Resume en una frase lo que revisaste."));
        let r = fresco(con_memoria(rebobinado.clone())).await;
        // referencia: el MISMO camino sin disco (la app abierta: tras el 5.º mensaje se rebobina y la
        // caché EN MEMORIA vuelve a su instantánea). Leer del disco tiene que dar exactamente lo mismo.
        // Contra una lectura completa desde cero puede diferir en texto libre: el prellenado se parte
        // en otros bloques y bf16 redondea distinto (pasa igual con la caché en memoria de siempre).
        // el camino en memoria que parte del MISMO punto que el disco: un denso recorta exacto (la
        // memoria que vio toda la conversación); un híbrido vuelve a su instantánea (la del 5.º mensaje)
        let antes = if indice().recurrente { corto_de(&largo) } else { largo.clone() };
        let mut f = NativoEngine::new(&dir);
        f.start().await.unwrap();
        f.complete(sin_memoria(antes)).await.unwrap();
        let en_memoria = f.complete(sin_memoria(rebobinado.clone())).await.unwrap();
        f.stop().await.unwrap();
        let desde_cero = fresco(sin_memoria(rebobinado)).await;
        println!("3 · rebobinar: {} tok, {} de caché · en memoria {} de caché · ¿igual que desde cero? {}", r.prompt_tokens, r.cached_tokens, en_memoria.cached_tokens, r.content == desde_cero.content);
        assert!(r.cached_tokens > 1000, "aprovecha el prefijo guardado");
        assert_eq!(r.cached_tokens, en_memoria.cached_tokens, "el mismo punto de partida");
        assert_eq!(r.content, en_memoria.content, "leer del disco = seguir en memoria");
        // 4) compactar (cambia el mensaje 1): no sirve, lo dice, y lo guardado se corta y se rehace
        let mut compactado = vec![largo[0].clone(), Message::user("Resumen: revisamos ocho pasos de código, todos correctos.")];
        compactado.extend_from_slice(&largo[largo.len() - 5..]);
        let r = fresco(con_memoria(compactado.clone())).await;
        println!("4 · compactar: {} tok, {} de caché · {:?}", r.prompt_tokens, r.cached_tokens, r.memoria_rapida);
        assert!(r.memoria_rapida.as_deref().is_some_and(|c| c.contains("historial")), "{:?}", r.memoria_rapida);
        let r2 = fresco(con_memoria(compactado)).await;
        assert!(r2.prompt_tokens - r2.cached_tokens < 1100, "tras compactar se vuelve a guardar y a leer");
        let _ = std::fs::remove_dir_all(carpeta.parent().and_then(|p| p.parent()).unwrap());
    }

    /// El caso de la app (2026-11-10): un historial REAL con una nota de voz vieja + «hola», con un
    /// modelo que no oye. Antes: «Unexpected item type in content» en cada turno.
    /// `GOTV_MLX_DIR=<9B> GOTV_HISTORIAL=<json> cargo test -p gotv-mlx --release -- --ignored --nocapture historial_con_audio`
    #[tokio::test]
    #[ignore]
    async fn historial_con_audio_en_un_modelo_que_no_oye() {
        let (Some(dir), Ok(h)) = (dir_prueba(), std::env::var("GOTV_HISTORIAL")) else { return };
        let msgs: Vec<Message> = serde_json::from_str(&std::fs::read_to_string(h).unwrap()).unwrap();
        assert!(msgs.iter().any(|m| !m.audio.is_empty()), "el historial trae la nota de voz");
        // el fallo tal cual: la plantilla del modelo, con el audio, revienta
        let e0 = plantilla::Plantilla::cargar(&dir).unwrap().prompt(&msgs, None).unwrap_err().to_string();
        println!("sin limpiar: {e0}");
        assert!(e0.contains("Unexpected item type in content"), "{e0}");
        let mut e = NativoEngine::new(&dir);
        assert!(!e.capabilities().audio_in, "este modelo no oye");
        e.start().await.unwrap();
        let r = e.complete(req(msgs, None, 40)).await;
        println!("{:?}", r.as_ref().map(|r| &r.content));
        assert!(r.is_ok(), "{r:?}");
    }

    #[tokio::test]
    #[ignore]
    async fn el_nucleo_nativo_responde_llama_herramientas_y_reutiliza_la_cache() {
        let Some(dir) = dir_prueba() else { return };
        let mut e = NativoEngine::new(&dir);
        let t0 = std::time::Instant::now();
        e.start().await.unwrap();
        println!("carga: {:?}", t0.elapsed());
        let t1 = std::time::Instant::now();
        let r = e.complete(req(vec![Message::user("¿Cuál es la capital de Francia? Responde con una sola palabra.")], None, 20)).await.unwrap();
        println!("respuesta: {:?} · {} tok en {:?} ({:.1} tok/s)", r.content, r.tokens, t1.elapsed(), r.tokens as f64 / t1.elapsed().as_secs_f64());
        assert!(r.content.to_lowercase().contains("par"), "{r:?}");
        let tools = serde_json::json!([{"type": "function", "function": {"name": "read_file", "description": "Lee un archivo del proyecto", "parameters": {"type": "object", "properties": {"path": {"type": "string"}}, "required": ["path"]}}}]);
        let msgs = vec![Message::system("Eres un agente de programación. Usa las herramientas."), Message::user("Lee el archivo notas.txt")];
        let r = e.complete(req(msgs.clone(), Some(tools.clone()), 120)).await.unwrap();
        println!("llamada: {:?} · finish {}", r.tool_calls, r.finish_reason);
        assert_eq!(r.tool_calls.len(), 1, "{r:?}");
        assert_eq!(r.tool_calls[0].name, "read_file");
        assert_eq!(r.tool_calls[0].arguments["path"], "notas.txt");
        // el turno siguiente reutiliza el prefijo (system + tools + pregunta)
        let mut asst = Message::assistant("");
        asst.tool_calls = Some(vec![r.tool_calls[0].to_openai()]);
        let mut herr = Message::user("");
        herr.role = "tool".into();
        herr.content = "comprar pan".into();
        herr.tool_call_id = Some(r.tool_calls[0].id.clone());
        let mut msgs2 = msgs.clone();
        msgs2.push(asst);
        msgs2.push(herr);
        let t2 = std::time::Instant::now();
        let r2 = e.complete(req(msgs2, Some(tools), 60)).await.unwrap();
        println!("segundo turno: {:?} · caché {} de {} tokens · {:?}", r2.content, r2.cached_tokens, r2.prompt_tokens, t2.elapsed());
        assert!(r2.cached_tokens > r2.prompt_tokens / 2, "reutiliza el prefijo: {r2:?}");
        assert!(r2.content.to_lowercase().contains("pan"), "{r2:?}");
        e.stop().await.unwrap();
    }

    /// Un safetensors mínimo escrito a mano: con `mtp` y normas ≈ 1 → sólo nativo; sin `mtp`, o con
    /// normas crudas (≈ 0), el servidor sirve.
    #[test]
    fn detecta_el_modelo_que_solo_corre_en_el_nucleo() {
        let d = gotv_engine::prueba::CarpetaPrueba::sin_crear("gotv-mlx-mtp");
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("config.json"), r#"{"model_type":"qwen3_5","text_config":{"hidden_size":4,"num_hidden_layers":1,"num_attention_heads":1,"num_key_value_heads":1}}"#).unwrap();
        let escribir = |norma: f32, mtp: bool| {
            let bf = |x: f32| ((x.to_bits() >> 16) as u16).to_le_bytes();
            let datos: Vec<u8> = (0..4).flat_map(|_| bf(norma)).collect();
            let mut cab = serde_json::json!({"language_model.model.layers.0.input_layernorm.weight": {"dtype": "BF16", "shape": [4], "data_offsets": [0, 8]}});
            if mtp {
                cab["language_model.mtp.fc.weight"] = serde_json::json!({"dtype": "BF16", "shape": [4], "data_offsets": [0, 8]});
            }
            let j = serde_json::to_vec(&cab).unwrap();
            let mut f = (j.len() as u64).to_le_bytes().to_vec();
            f.extend(j);
            f.extend(datos);
            std::fs::write(d.join("model.safetensors"), f).unwrap();
        };
        escribir(0.97, true);
        assert!(exige_nativo(&d).is_some());
        escribir(0.97, false);
        assert!(exige_nativo(&d).is_none());
        escribir(0.02, true);
        assert!(exige_nativo(&d).is_none(), "crudas: mlx_lm las corrige bien");
        let _ = std::fs::remove_dir_all(&d);
    }

    /// Con los modelos reales del Portador (si están): el Fable exige el núcleo; el 9B y el Uncensored no.
    #[test]
    #[ignore]
    fn el_fable_exige_el_nucleo() {
        let h = PathBuf::from(std::env::var("HOME").unwrap());
        let fable = h.join(".cache/gotv/models/Solstice-AI/Qwen3.8-27B-TURBO-Fable-Cold-Fusion-NEO-CODER-mlx-oQ4e");
        let nueve = h.join(".cache/gotv/models/mlx-community/Qwen3.5-9B-4bit");
        if fable.exists() {
            assert!(exige_nativo(&fable).is_some());
        }
        if nueve.exists() {
            assert!(exige_nativo(&nueve).is_none());
        }
    }

    /// M8 con el motor completo: describe como mlx_vlm, mismo largo de prompt, reutiliza la caché
    /// con la MISMA imagen y no con otra del mismo tamaño.
    /// `GOTV_MLX_DIR=<9B> GOTV_IMG=<png> GOTV_IMG2=<png del mismo tamaño> cargo test -p gotv-mlx
    /// --release -- --ignored --nocapture ve_imagenes_en_el_motor`
    #[tokio::test]
    #[ignore]
    async fn ve_imagenes_en_el_motor() {
        use base64::Engine as _;
        let (Ok(dir), Ok(a), Ok(b)) = (std::env::var("GOTV_MLX_DIR"), std::env::var("GOTV_IMG"), std::env::var("GOTV_IMG2")) else { return };
        let url = |p: &str| format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(std::fs::read(p).unwrap()));
        let mut e = NativoEngine::new(&dir);
        assert!(e.capabilities().vision, "el 9B ve en el núcleo");
        e.start().await.unwrap();
        let pide = |img: &str| vec![Message::user_with_images("Describe esta imagen en una frase.", vec![url(img)])];
        let t = std::time::Instant::now();
        let r = e.complete(req(pide(&a), None, 40)).await.unwrap();
        println!("1 · {:?} · prompt {} tok · {:?}", r.content, r.prompt_tokens, t.elapsed());
        assert_eq!(r.prompt_tokens, 277, "mismo prompt que mlx_vlm");
        assert!(r.content.contains("anillos"), "{r:?}");
        let t = std::time::Instant::now();
        let r2 = e.complete(req(pide(&a), None, 40)).await.unwrap();
        println!("2 · misma imagen: caché {}/{} · {:?}", r2.cached_tokens, r2.prompt_tokens, t.elapsed());
        assert!(r2.cached_tokens > 250, "reutiliza la imagen ya vista: {r2:?}");
        let r3 = e.complete(req(pide(&b), None, 40)).await.unwrap();
        println!("3 · otra imagen del mismo tamaño: caché {}/{} · {:?}", r3.cached_tokens, r3.prompt_tokens, r3.content);
        assert!(r3.cached_tokens < 10, "NO reutiliza la memoria de la otra imagen: {r3:?}");
        e.stop().await.unwrap();
    }

    /// M1b con el motor completo: Gemma OYE en el núcleo (los dígitos del audio), reutiliza la
    /// caché con el MISMO audio y, con `GOTV_IMG`, ve y oye en el mismo mensaje.
    /// `GOTV_MLX_DIR=<gemma> GOTV_WAV=<wav> [GOTV_IMG=<png>] cargo test -p gotv-mlx --release --
    /// --ignored --nocapture oye_audio_en_el_motor`
    #[tokio::test]
    #[ignore]
    async fn oye_audio_en_el_motor() {
        use base64::Engine as _;
        let (Ok(dir), Ok(wav)) = (std::env::var("GOTV_MLX_DIR"), std::env::var("GOTV_WAV")) else { return };
        let b64 = |p: &str| base64::engine::general_purpose::STANDARD.encode(std::fs::read(p).unwrap());
        let audio = format!("data:audio/wav;base64,{}", b64(&wav));
        let mut e = NativoEngine::new(&dir);
        assert!(e.capabilities().audio_in, "Gemma oye en el núcleo");
        assert!(sentido_que_falta(Path::new(&dir), true, true).is_none(), "ya no le falta ningún sentido");
        e.start().await.unwrap();
        // el constructor REAL del agente (pone el audio en `Message::audio`)
        let pide = || vec![Message::user_with_media("¿Qué números dice el audio? Responde sólo los dígitos.", vec![audio.clone()])];
        let t = std::time::Instant::now();
        let r = e.complete(req(pide(), None, 12)).await.unwrap();
        println!("1 · {:?} · prompt {} tok · {:?}", r.content, r.prompt_tokens, t.elapsed());
        assert!(r.content.contains("7341"), "{r:?}");
        let t = std::time::Instant::now();
        let r2 = e.complete(req(pide(), None, 12)).await.unwrap();
        println!("2 · mismo audio: caché {}/{} · {:?}", r2.cached_tokens, r2.prompt_tokens, t.elapsed());
        assert!(r2.cached_tokens > 60, "reutiliza el audio ya oído: {r2:?}");
        if let Ok(img) = std::env::var("GOTV_IMG") {
            let m = vec![Message::user_with_media("Di en una frase qué hay en la imagen y luego los números del audio.", vec![format!("data:image/png;base64,{}", b64(&img)), audio.clone()])];
            let r3 = e.complete(req(m, None, 60)).await.unwrap();
            // medido: mlx_vlm (`ref_mixto.py`) responde IGUAL, letra por letra, con el mismo prompt de
            // 365 tokens — el modelo de 4B separa los dígitos y apenas mira la imagen
            println!("3 · imagen + audio: {:?} · prompt {} tok", r3.content, r3.prompt_tokens);
            let digitos: String = r3.content.chars().filter(|c| c.is_ascii_digit()).collect();
            assert!(digitos.contains("7341"), "{r3:?}");
        }
        e.stop().await.unwrap();
    }

    /// La caché del prefijo del SISTEMA leída del disco da la MISMA respuesta que calcularlo, y
    /// mucho más rápido: un motor lo guarda, otro NUEVO (otra conversación) lo lee.
    /// `GOTV_MLX_DIR=<9B> cargo test -p gotv-mlx --release -- --ignored --nocapture prefijo_del_sistema`
    #[tokio::test]
    #[ignore]
    async fn prefijo_del_sistema_desde_disco() {
        let Some(dir) = dir_prueba() else { return };
        let carpeta = gotv_engine::prueba::CarpetaPrueba::sin_crear("gotv-prefijos");
        std::env::set_var("GOTV_PREFIJOS_DIR", &carpeta);
        let sistema = format!("Eres un agente de programación. {}", "Sigue las reglas del proyecto, usa las herramientas y verifica lo que haces antes de terminar. ".repeat(60));
        let tools = serde_json::json!([{"type": "function", "function": {"name": "read_file", "description": "Lee un archivo del proyecto", "parameters": {"type": "object", "properties": {"path": {"type": "string"}}, "required": ["path"]}}}]);
        let msgs = vec![Message::system(&sistema), Message::user("Lee el archivo notas.txt")];
        let mut resp = vec![];
        for vuelta in 0..2 {
            let mut e = NativoEngine::new(&dir);
            e.start().await.unwrap();
            let t = std::time::Instant::now();
            let r = e.complete(req(msgs.clone(), Some(tools.clone()), 60)).await.unwrap();
            println!("vuelta {vuelta}: {:?} · caché {}/{} · {:?}", r.tool_calls.first().map(|c| (&c.name, &c.arguments)), r.cached_tokens, r.prompt_tokens, t.elapsed());
            resp.push((r.tool_calls.first().map(|c| (c.name.clone(), c.arguments.clone())), r.content.clone(), r.cached_tokens, t.elapsed()));
            e.stop().await.unwrap();
            let gb = |b: usize| b as f64 / 1e9;
            println!("   tras apagar: MLX activa {:.1} GB · en su caché {:.1} GB", gb(mlx_rs::memory::active_memory().unwrap()), gb(mlx_rs::memory::cache_memory().unwrap()));
        }
        let guardados = std::fs::read_dir(&carpeta).map(|d| d.flatten().flat_map(|m| std::fs::read_dir(m.path()).into_iter().flatten().flatten()).count()).unwrap_or(0);
        println!("cachés guardadas: {guardados}");
        assert_eq!(resp[0].2, 0, "la primera calcula todo");
        assert!(resp[1].2 > 256, "la segunda lee el sistema del disco");
        assert_eq!((&resp[0].0, &resp[0].1), (&resp[1].0, &resp[1].1), "MISMA respuesta con el prefijo leído");
        assert!(resp[1].3 < resp[0].3, "y más rápido");
        let _ = std::fs::remove_dir_all(&carpeta);
    }

    #[test]
    fn rechaza_lo_que_aun_no_corre() {
        let d = gotv_engine::prueba::CarpetaPrueba::sin_crear("gotv-mlx-arq");
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("config.json"), r#"{"model_type":"phi3","hidden_size":8,"num_hidden_layers":1,"num_attention_heads":1,"num_key_value_heads":1}"#).unwrap();
        assert!(NativoEngine::puede(&d).unwrap_err().contains("phi3"));
        // fase 4: Gemma 4 entra; sus variantes sin modelo con el que validarlas, no
        std::fs::write(d.join("config.json"), r#"{"model_type":"gemma4","text_config":{"hidden_size":8,"num_hidden_layers":1,"num_attention_heads":1,"num_key_value_heads":1}}"#).unwrap();
        assert!(NativoEngine::puede(&d).is_ok());
        std::fs::write(d.join("config.json"), r#"{"model_type":"gemma4","text_config":{"hidden_size":8,"num_hidden_layers":1,"num_attention_heads":1,"num_key_value_heads":1,"enable_moe_block":true}}"#).unwrap();
        assert!(NativoEngine::puede(&d).unwrap_err().contains("expertos"));
        std::fs::write(d.join("config.json"), r#"{"model_type":"gemma4","text_config":{"hidden_size":8,"num_hidden_layers":1,"num_attention_heads":1,"num_key_value_heads":1,"attention_k_eq_v":true}}"#).unwrap();
        assert!(NativoEngine::puede(&d).is_err());
        std::fs::write(d.join("config.json"), r#"{"model_type":"qwen3","hidden_size":8,"num_hidden_layers":1,"num_attention_heads":1,"num_key_value_heads":1,"quantization":{"group_size":32,"bits":4,"mode":"mxfp4"}}"#).unwrap();
        assert!(NativoEngine::puede(&d).is_err());
        std::fs::write(d.join("config.json"), r#"{"model_type":"qwen3","hidden_size":8,"num_hidden_layers":1,"num_attention_heads":1,"num_key_value_heads":1,"quantization":{"group_size":64,"bits":4}}"#).unwrap();
        assert!(NativoEngine::puede(&d).is_ok());
        let _ = std::fs::remove_dir_all(&d);
    }
}

#[cfg(test)]
mod prueba_whisper_wav {
    /// M1: `transcribir_wav` (la función que registra el CLI) con un WAV de `afconvert`.
    /// `GOTV_WHISPER=<dir> GOTV_WAV=<wav> cargo test -p gotv-mlx --release -- --ignored --nocapture transcribe_un_wav`
    #[test]
    #[ignore]
    fn transcribe_un_wav() {
        let (Ok(d), Ok(w)) = (std::env::var("GOTV_WHISPER"), std::env::var("GOTV_WAV")) else { return };
        let t = std::time::Instant::now();
        let r = super::transcribir_wav(std::path::Path::new(&d), &std::fs::read(&w).unwrap());
        println!("{:?} · {:?}", r, t.elapsed());
        assert!(r.is_ok());
    }

    /// ¿Cuánto del segundo de Whisper es CARGAR el modelo y cuánto transcribir?
    /// `GOTV_WHISPER=<dir> GOTV_WAV=<wav> cargo test -p gotv-mlx --release -- --ignored --nocapture whisper_carga_y_calculo`
    #[test]
    #[ignore]
    fn whisper_carga_y_calculo() {
        let (Ok(d), Ok(w)) = (std::env::var("GOTV_WHISPER"), std::env::var("GOTV_WAV")) else { return };
        let pcm = super::audio_gemma::pcm_de_wav(&std::fs::read(&w).unwrap()).unwrap();
        super::fijar_metallib();
        for i in 1..=2 {
            let t = std::time::Instant::now();
            let m = super::whisper::Whisper::cargar(std::path::Path::new(&d)).unwrap();
            let carga = t.elapsed();
            let t = std::time::Instant::now();
            let txt = mlx_rs::with_device(mlx_rs::Device::gpu(), || m.transcribir(&pcm).map(|r| r.texto)).unwrap();
            let c1 = t.elapsed();
            let t = std::time::Instant::now();
            let _ = mlx_rs::with_device(mlx_rs::Device::gpu(), || m.transcribir(&pcm).map(|r| r.texto)).unwrap();
            println!("{i} · carga {carga:?} · 1.ª transcripción {c1:?} · 2.ª con el modelo ya cargado {:?} · {:.0} s de audio · «{}»", t.elapsed(), pcm.len() as f64 / 16000.0, txt.trim().chars().take(30).collect::<String>());
        }
    }
}

#[cfg(test)]
mod prueba_embebedor {
    /// M11 · juez contra el servidor de embeddings de mlx_vlm (`ref_embed.py`): mismos tokens y
    /// coseno ≥ 0,9999 con cada vector; y la segunda tanda reutiliza el modelo cargado.
    /// `GOTV_EMBED_DIR=<dir> GOTV_REF_EMBED=<ref.json> cargo test -p gotv-mlx --release -- --ignored
    /// --nocapture incrusta_como_mlx_vlm`
    #[test]
    #[ignore]
    fn incrusta_como_mlx_vlm() {
        let (Ok(d), Ok(r)) = (std::env::var("GOTV_EMBED_DIR"), std::env::var("GOTV_REF_EMBED")) else { return };
        let dir = std::path::Path::new(&d);
        let refs: Vec<serde_json::Value> = serde_json::from_str(&std::fs::read_to_string(&r).unwrap()).unwrap();
        let tok = tokenizers::Tokenizer::from_file(dir.join("tokenizer.json")).unwrap();
        let textos: Vec<String> = refs.iter().map(|x| x["texto"].as_str().unwrap().to_string()).collect();
        for (t, x) in textos.iter().zip(&refs) {
            let ids: Vec<u32> = x["ids"].as_array().unwrap().iter().map(|v| v.as_u64().unwrap() as u32).collect();
            assert_eq!(tok.encode(t.as_str(), true).unwrap().get_ids(), ids.as_slice(), "tokens de {t:?}");
        }
        let t0 = std::time::Instant::now();
        let v = super::incrustar_textos(dir, &textos).unwrap();
        let t1 = t0.elapsed();
        let t0 = std::time::Instant::now();
        let v2 = super::incrustar_textos(dir, &textos[..1]).unwrap();
        let t2 = t0.elapsed();
        assert_eq!(v[0], v2[0], "determinista");
        for (a, x) in v.iter().zip(&refs) {
            let b: Vec<f32> = x["vector"].as_array().unwrap().iter().map(|v| v.as_f64().unwrap() as f32).collect();
            // coseno de verdad: los vectores salen de bf16 y su norma no es exactamente 1
            let n = |v: &[f32]| v.iter().map(|x| x * x).sum::<f32>().sqrt();
            let cos: f32 = a.iter().zip(&b).map(|(p, q)| p * q).sum::<f32>() / (n(a) * n(&b));
            let dif = a.iter().zip(&b).map(|(p, q)| (p - q).abs()).fold(0.0, f32::max);
            let iguales = a.iter().zip(&b).filter(|(p, q)| p == q).count();
            println!("{:>4} tok · coseno {cos:.6} · dif máx {dif:e} · iguales {iguales}/{}", x["ids"].as_array().unwrap().len(), a.len());
            assert!(cos > 0.9999, "vector distinto");
        }
        println!("5 textos {t1:?} (con carga) · 1 texto {t2:?} (modelo ya cargado)");
    }
}
