//! M0 · Biblioteca de GENERADORES y sus licencias (la base de las olas M1–M7: oír, hablar, imagen,
//! video, música). Esquema híbrido decidido con el Portador: el motor es nuestro (Rust); los PESOS los
//! baja cada usuario desde el repositorio oficial de su autor, con la licencia de ese autor.
//!
//! - **Catálogo verificado** contra la API OFICIAL de Hugging Face (`/api/models/<repo>`: licencia
//!   declarada, acceso restringido, commit, tamaño), no contra blogs. `gotv generadores verificar`
//!   repite la consulta y avisa si algo cambió (la tabla caduca).
//! - **Aceptación obligatoria:** sin aceptar la licencia no se descarga; se guarda localmente qué
//!   licencia, de qué commit y cuándo (`~/.cache/gotv/licencias.gotv`, formato propio). Si la licencia del catálogo
//!   cambia, la aceptación anterior deja de valer.
//! - **Restringido ≠ no libre:** FLUX.1-schnell es Apache pero de acceso restringido. En esos repos el
//!   usuario acepta EN Hugging Face con su cuenta y da su token, que va al Llavero de macOS (nunca a
//!   archivos); GOTV no acepta por él.
//! - **Ficha junto a cada archivo generado** (`escribir_ficha`): modelo, autor, licencia y aviso.
//! - Al autor se le nombra sólo como ORIGEN; nada de logos ni de insinuar respaldo.

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Cuenta del token de Hugging Face en el Llavero (`llaves`, servicio `space.chaosx.gotv`).
pub const LLAVE_HF: &str = "huggingface";
/// Fecha de la última verificación del catálogo contra la API de Hugging Face.
pub const VERIFICADO: &str = "2026-11-12";

#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Area {
    Oir,
    Hablar,
    Imagen,
    Video,
    Musica,
    /// el embebedor de la búsqueda semántica (RAG)
    Buscar,
}

impl Area {
    pub fn nombre(self) -> &'static str {
        match self {
            Area::Oir => crate::i18n::t("oír", "hear"),
            Area::Hablar => crate::i18n::t("hablar", "speak"),
            Area::Imagen => crate::i18n::t("imagen", "image"),
            Area::Video => crate::i18n::t("video", "video"),
            Area::Musica => crate::i18n::t("música", "music"),
            Area::Buscar => crate::i18n::t("búsqueda", "search"),
        }
    }
}

/// Qué permite la licencia, en una etiqueta clara.
#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Clase {
    /// Apache-2.0, MIT…: uso comercial permitido
    Libre,
    /// CC-BY: libre, pide nombrar al autor
    Atribucion,
    /// licencia comunitaria: gratis con condiciones (umbral de ingresos, territorios…)
    Condiciones,
    /// no comercial
    NoComercial,
    /// el autor NO concede licencia sobre los pesos (p. ej. HTDemucs: «the license of the weights is not
    /// clear»): sólo para investigar; ni comercial ni redistribuir lo derivado
    SinLicencia,
}

impl Clase {
    pub fn etiqueta(self) -> &'static str {
        match self {
            Clase::Libre => crate::i18n::t("libre", "free"),
            Clase::Atribucion => crate::i18n::t("libre · pide atribución", "free · attribution required"),
            Clase::Condiciones => crate::i18n::t("⚠ con condiciones", "⚠ with conditions"),
            Clase::NoComercial => crate::i18n::t("⚠ no comercial", "⚠ non-commercial"),
            Clase::SinLicencia => crate::i18n::t("⚠ sin licencia clara: sólo investigación", "⚠ no clear license: research only"),
        }
    }
}

/// Un generador del catálogo (datos leídos de la API de Hugging Face el día `VERIFICADO`).
#[derive(Serialize, Clone, Debug)]
pub struct Generador {
    /// repositorio OFICIAL del autor
    pub repo: &'static str,
    pub nombre: &'static str,
    pub area: Area,
    /// licencia tal como la declara la ficha de Hugging Face
    pub licencia: &'static str,
    pub url_licencia: &'static str,
    pub clase: Clase,
    /// acceso restringido en Hugging Face (hay que aceptar allí con la cuenta y dar token)
    pub restringido: bool,
    /// ola del plan en la que llega su motor (hasta entonces, memoria y tiempo sin medir)
    pub fase: &'static str,
    /// tamaño del repo ENTERO (incluye formatos alternativos); su motor bajará sólo lo que use
    pub bytes_repo: u64,
    /// commit verificado
    pub sha: &'static str,
    /// sólo estos archivos (prefijos); vacío = el repo entero
    pub archivos: &'static [&'static str],
    /// la conversión de otro (p. ej. a MLX por mlx-community): el repo OFICIAL del que viene y cuya
    /// licencia hereda; vacío = es el oficial
    pub origen: &'static str,
    /// ¿ya está en Hugging Face? Uno propio de GOTV aún sin publicar (sin commit) no se ofrece a bajar ni
    /// se verifica, y en Configuración sólo aparece en la Mac que ya lo tiene en su carpeta de modelos
    pub publicado: bool,
}

const APACHE: &str = "https://www.apache.org/licenses/LICENSE-2.0";
const MIT: &str = "https://opensource.org/license/mit";
const CC_BY: &str = "https://creativecommons.org/licenses/by/4.0/";
/// HTDemucs: el LICENSE (MIT) es del CÓDIGO. De los PESOS habla la sección «License» del README: «The
/// license of the weights is not clear» (MUSDB sólo para investigación; el 6s, datos internos de Meta,
/// «likely CC-NC-BY»). Leído el 2026-11-23; antes el catálogo decía MIT (falla #3137).
const DEMUCS_PESOS: &str = "https://github.com/adefossez/demucs/blob/main/README.md#license";

macro_rules! g {
    ($repo:expr, $nombre:expr, $area:ident, $lic:expr, $url:expr, $clase:ident, $restr:expr, $fase:expr, $gb:expr, $sha:expr) => {
        Generador { repo: $repo, nombre: $nombre, area: Area::$area, licencia: $lic, url_licencia: $url, clase: Clase::$clase, restringido: $restr, fase: $fase, bytes_repo: ($gb * 1e9) as u64, sha: $sha, archivos: &[], origen: "", publicado: true }
    };
}

/// El catálogo verificado (API de Hugging Face, 2026-11-10). Licencias y restricciones LEÍDAS de
/// cada ficha oficial; ninguna supuesta.
pub static CATALOGO: &[Generador] = &[
    // oír (M1): el Whisper que corre en el núcleo es la conversión a MLX fp16 de mlx-community (su
    // ficha no declara licencia: hereda la MIT del oficial de OpenAI, que se verifica en su lugar);
    // los demás, candidatos. (Verificado en la API de HF el 2026-11-12.)
    Generador {
        origen: "openai/whisper-large-v3-turbo",
        ..g!("mlx-community/whisper-large-v3-turbo-asr-fp16", "Whisper large-v3-turbo", Oir, "mit", MIT, Libre, false, "M1", 1.62, "624c19c9af")
    },
    g!("mistralai/Voxtral-Mini-4B-Realtime-2602", "Voxtral Mini 4B Realtime", Oir, "apache-2.0", APACHE, Libre, false, "M1", 17.73, "2769294da9"),
    g!("nvidia/parakeet-tdt-0.6b-v3", "Parakeet TDT 0.6B v3", Oir, "cc-by-4.0", CC_BY, Atribucion, false, "M1", 5.73, "541d1f99c6"),
    // hablar (M2)
    g!("hexgrad/Kokoro-82M", "Kokoro 82M", Hablar, "apache-2.0", APACHE, Libre, false, "M2", 0.36, "f3ff357179"),
    g!("Qwen/Qwen3-TTS-12Hz-1.7B-Base", "Qwen3-TTS 1.7B", Hablar, "apache-2.0", APACHE, Libre, false, "M2", 4.54, "fd4b254389"),
    g!("microsoft/VibeVoice-Realtime-0.5B", "VibeVoice Realtime 0.5B", Hablar, "mit", MIT, Libre, false, "M2", 2.04, "6bce5f0604"),
    g!("ResembleAI/chatterbox", "Chatterbox", Hablar, "mit", MIT, Libre, false, "M2", 13.87, "5bb1f6ee58"),
    // imagen (M4)
    g!("black-forest-labs/FLUX.1-schnell", "FLUX.1 schnell", Imagen, "apache-2.0", APACHE, Libre, true, "M4", 57.85, "741f7c3ce8"),
    // M4: el motor usa el formato por partes (diffusers); el `flux-2-klein-4b.safetensors` suelto
    // (7,75 GB) es el MISMO transformador en otro formato: no se baja (~16 GB en vez de 23,7)
    Generador {
        archivos: &["model_index.json", "LICENSE.md", "scheduler/", "text_encoder/", "tokenizer/", "transformer/", "vae/"],
        ..g!("black-forest-labs/FLUX.2-klein-4B", "FLUX.2 klein 4B", Imagen, "apache-2.0", APACHE, Libre, false, "M4", 23.74, "e7b7dc27f9")
    },
    // Ola PRO: klein BASE (sin destilar: CFG real y prompt NEGATIVO, 50 pasos). Su codificador de texto,
    // su VAE y su planificador son los MISMOS archivos que los de klein (mismos sha256 en la API de HF,
    // 2026-11-13): se baja SÓLO el transformador (7,75 GB) y el resto se lee de klein
    Generador {
        archivos: &["model_index.json", "LICENSE.md", "transformer/"],
        ..g!("black-forest-labs/FLUX.2-klein-base-4B", "FLUX.2 klein base 4B", Imagen, "apache-2.0", APACHE, Libre, false, "PRO", 23.74, "a3b4f48491")
    },
    g!("Tongyi-MAI/Z-Image-Turbo", "Z-Image Turbo", Imagen, "apache-2.0", APACHE, Libre, false, "M4", 32.90, "f332072aa7"),
    g!(
        "black-forest-labs/FLUX.1-dev",
        "FLUX.1 dev",
        Imagen,
        "flux-1-dev-non-commercial-license",
        "https://huggingface.co/black-forest-labs/FLUX.1-dev/blob/main/LICENSE.md",
        NoComercial,
        true,
        "M4",
        57.89,
        "3de623fc3c"
    ),
    // video (M6)
    // M6: el formato por partes de diffusers (todo safetensors, el mismo que usa el juez); el repo
    // original trae el codificador y el VAE en `.pth`. Sin `assets/` ni `examples/` (verificado en la
    // API de HF el 2026-11-13: 34,2 GB, Apache-2.0, sin restricción)
    Generador {
        archivos: &["model_index.json", "scheduler/", "text_encoder/", "tokenizer/", "transformer/", "vae/"],
        ..g!("Wan-AI/Wan2.2-TI2V-5B-Diffusers", "Wan 2.2 TI2V 5B", Video, "apache-2.0", APACHE, Libre, false, "M6", 34.20, "b8fff7315c")
    },
    g!(
        "Lightricks/LTX-2.5",
        "LTX 2.5",
        Video,
        "ltx-2.x-community-license-agreement",
        "https://github.com/Lightricks/LTX-2/blob/main/LICENSE-2_x",
        Condiciones,
        true,
        "M6",
        200.85,
        "49b093a39c"
    ),
    // música (M7)
    g!("ACE-Step/Ace-Step1.5", "ACE-Step 1.5", Musica, "mit", MIT, Libre, false, "M7", 10.09, "19671f406d"),
    // Ola PRO: el DiT BASE de ACE-Step 1.5 (misma arquitectura que el turbo, sin destilar): CFG con APG,
    // 50 pasos, y las tareas que sólo él sabe — separar pistas (extract), añadir una pista (lego) y
    // acompañar una voz (complete). Usa el LM, el texto y el VAE del repo de ACE-Step 1.5
    g!("ACE-Step/acestep-v15-base", "ACE-Step 1.5 base", Musica, "mit", MIT, Libre, false, "PRO", 4.79, "e432212fec"),
    // Ola PRO · PRO-4: las variantes oficiales verificadas en la API de HF el 2026-11-14 (todas MIT):
    // - SFT: misma arquitectura que el base (CFG, 50 pasos), afinado para COMPONER con más calidad;
    // - XL (turbo, sft, base): DiT de 2560 y 32 capas con codificador de 2048 (puerto propio);
    // - el LM planificador de 4B (Qwen3, como el de 1,7B): mejor plan y letra con más memoria.
    g!("ACE-Step/acestep-v15-sft", "ACE-Step 1.5 SFT", Musica, "mit", MIT, Libre, false, "PRO", 4.79, "c410d249e7"),
    g!("ACE-Step/acestep-v15-xl-turbo", "ACE-Step 1.5 XL turbo", Musica, "mit", MIT, Libre, false, "PRO", 19.95, "d4a0b288b8"),
    g!("ACE-Step/acestep-v15-xl-sft", "ACE-Step 1.5 XL SFT", Musica, "mit", MIT, Libre, false, "PRO", 19.95, "d06de46b46"),
    g!("ACE-Step/acestep-v15-xl-base", "ACE-Step 1.5 XL base", Musica, "mit", MIT, Libre, false, "PRO", 19.95, "220c1166ef"),
    g!("ACE-Step/acestep-5Hz-lm-4B", "ACE-Step LM 4B (planificador)", Musica, "mit", MIT, Libre, false, "PRO", 8.43, "0a3ec94b55"),
    // Ola PRO · PRO-6: el SEPARADOR de pistas (Hybrid Transformer Demucs). Los pesos los publica su propio
    // autor (Alexandre Défossez) en HF; la ficha no declara licencia. El MIT de github.com/adefossez/demucs
    // es del CÓDIGO: de los PESOS su README dice «not clear» (MUSDB sólo para investigación; el 6s, datos
    // internos de Meta, «likely CC-NC-BY»). Leído el 2026-11-23: `unknown` = sin licencia, sólo investigar.
    // (El Portador aceptó USARLOS el 2026-11-13; la etiqueta dice lo que dice el fabricante.)
    g!("adefossez/HTDemucs", "HTDemucs (4 pistas)", Musica, "unknown", DEMUCS_PESOS, SinLicencia, false, "PRO", 0.084, "cbc8a9b1a8"),
    g!("adefossez/HTDemucs-6s", "HTDemucs 6 pistas", Musica, "unknown", DEMUCS_PESOS, SinLicencia, false, "PRO", 0.055, "3c5ee475be"),
    g!("adefossez/HTDemucs-ft", "HTDemucs ft (afinado)", Musica, "unknown", DEMUCS_PESOS, SinLicencia, false, "PRO", 0.336, "d74ac89c3a"),
    // EXP-7 · los separadores AFINADOS por GOTV (más finos en instrumentos y voz, medidos en canciones
    // apartadas), en la organización `god-of-the-void` de HF (`chaosx-space` es la marca). Heredan la
    // licencia de su base (`origen`): hoy NINGUNA clara, así que se publican SÓLO PARA INVESTIGACIÓN
    // (`license: unknown` en su ficha; falla #3137; decisión del Portador 2026-11-23: los pesos no son el fin,
    // la app sí). Los datos CC BY se atribuyen en su `ATRIBUCION.md`. Publicados y verificados en HF el
    // 2026-11-23 (pesos bajados == locales, sha-256).
    Generador { origen: "adefossez/HTDemucs", ..g!("god-of-the-void/HTDemucs-afinado", "HTDemucs-afinado · GOTV", Musica, "unknown", DEMUCS_PESOS, SinLicencia, false, "EXP-7", 0.084, "b842d0e8c3") },
    Generador { origen: "adefossez/HTDemucs-6s", ..g!("god-of-the-void/HTDemucs-6s-afinado", "HTDemucs-6s-afinado · GOTV", Musica, "unknown", DEMUCS_PESOS, SinLicencia, false, "EXP-7", 0.055, "d5e1026294") },
    Generador { origen: "adefossez/HTDemucs", ..g!("god-of-the-void/HTDemucs-bateria-afinado", "HTDemucs-bateria-afinado · GOTV", Musica, "unknown", DEMUCS_PESOS, SinLicencia, false, "EXP-7", 0.084, "416315519b") },
    // efectos de sonido (M7c): DiT tipo Wan 2.1 sobre audio + Qwen3-1.7B + DAC 48 kHz (verificado en la API de HF el 2026-11-12)
    g!("OpenMOSS-Team/MOSS-SoundEffect-v2.0", "MOSS-SoundEffect 2.0", Musica, "apache-2.0", APACHE, Libre, false, "M7", 11.23, "e35df4d82f"),
    // búsqueda semántica (M11): el embebedor del RAG, cuantizado a 4 bits por mlx-community desde
    // el oficial de Qwen (Apache-2.0 en las dos fichas; verificado en la API de HF el 2026-11-12)
    Generador {
        origen: "Qwen/Qwen3-Embedding-0.6B",
        ..g!("mlx-community/Qwen3-Embedding-0.6B-4bit-DWQ", "Qwen3-Embedding 0.6B", Buscar, "apache-2.0", APACHE, Libre, false, "M11", 0.35, "6c3ae70858")
    },
];

pub fn ficha(repo: &str) -> Option<&'static Generador> {
    CATALOGO.iter().find(|g| g.repo.eq_ignore_ascii_case(repo))
}

/// El autor tal como lo nombra su repo (sólo como ORIGEN).
pub fn autor(g: &Generador) -> &'static str {
    let r = if g.origen.is_empty() { g.repo } else { g.origen };
    r.split('/').next().unwrap_or(r)
}

/// El aviso obligatorio de cada tarjeta y de cada ficha de archivo generado.
pub fn aviso(g: &Generador) -> String {
    if g.repo.starts_with("god-of-the-void/") {
        // uno PROPIO de GOTV (un afinado): no es una conversión ajena; lo publica GOTV sobre su base
        return crate::tr!(
            "Afinado por GOTV a partir de {} (de {}) y publicado por GOTV en huggingface.co/{}, con la misma licencia que su base; los datos con que se afinó se atribuyen en su ATRIBUCION.md, que acompaña a cada resultado. Lo baja quien acepta. Licencia: {} — {}",
            "Fine-tuned by GOTV from {} (by {}) and published by GOTV at huggingface.co/{}, under the same license as its base; the data it was tuned on is credited in its ATRIBUCION.md, which travels with every result. Whoever accepts downloads it. License: {} — {}",
            g.origen,
            autor(g),
            g.repo,
            g.licencia,
            g.url_licencia
        );
    }
    if !g.origen.is_empty() {
        // una conversión: los pesos son del autor del oficial; la conversión, de quien la publicó
        return crate::tr!(
            "Estos pesos pertenecen a {} (huggingface.co/{}); GOTV usa su conversión publicada en huggingface.co/{}. GOTV no los incluye ni los redistribuye: los baja quien acepta, y su uso es responsabilidad de quien los descarga. Licencia: {} — {}",
            "These weights belong to {} (huggingface.co/{}); GOTV uses their conversion published at huggingface.co/{}. GOTV neither bundles nor redistributes them: whoever accepts downloads them, and their use is the responsibility of whoever downloads them. License: {} — {}",
            autor(g),
            g.origen,
            g.repo,
            g.licencia,
            g.url_licencia
        );
    }
    crate::tr!(
        "Estos pesos pertenecen a {} (huggingface.co/{}); GOTV no los incluye ni los redistribuye: los baja quien acepta, desde el repositorio de su autor, y su uso es responsabilidad de quien los descarga. Licencia: {} — {}",
        "These weights belong to {} (huggingface.co/{}); GOTV neither bundles nor redistributes them: whoever accepts downloads them from their author's repository, and their use is the responsibility of whoever downloads them. License: {} — {}",
        autor(g),
        g.repo,
        g.licencia,
        g.url_licencia
    )
}

/// Una licencia aceptada en ESTA Mac.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Aceptacion {
    pub licencia: String,
    pub url: String,
    /// commit del repo verificado cuando se aceptó
    pub sha: String,
    pub fecha: String,
    pub gotv: String,
}

/// Las aceptaciones viven en el formato propio (`programar::codec`, tipo LICA). Hasta 2026-11-17 eran
/// `licencias.json` junto a ella: la primera lectura las migra y renombra el viejo a `*.json.migrado`.
fn ruta_aceptaciones() -> PathBuf {
    #[cfg(test)]
    if let Some(p) = LICENCIAS_DE_PRUEBA.with(|c| c.borrow().clone()) {
        return p;
    }
    std::env::var_os("GOTV_LICENCIAS").map(PathBuf::from).unwrap_or_else(|| gotv_engine::cache_dir().join("licencias.gotv"))
}

// Las pruebas fijan su archivo de licencias POR HILO, nunca con la variable del proceso: antes una prueba ponía
// `GOTV_LICENCIAS` (y no la quitaba) y las demás, en paralelo, dejaban de ver Whisper aceptado; `listen_audio`
// aparecía o no según el orden (falla intermitente de `ninguna_forma_gasta_contexto_sin_usarse`, 2026-11-18).
#[cfg(test)]
thread_local! {
    static LICENCIAS_DE_PRUEBA: std::cell::RefCell<Option<PathBuf>> = const { std::cell::RefCell::new(None) };
}

/// Para las pruebas de otros módulos: el archivo de licencias de ESTE hilo (`None` = el de siempre).
#[cfg(test)]
pub(crate) fn licencias_de_prueba(p: Option<PathBuf>) {
    LICENCIAS_DE_PRUEBA.with(|c| *c.borrow_mut() = p);
}

fn migrar_json<T: serde::de::DeserializeOwned>(propio: &Path, viejo: &str, guardar: impl FnOnce(&T) -> Result<()>) -> Option<T> {
    crate::programar::codec::migrar_json(propio, &propio.with_file_name(viejo), guardar)
}

fn leer_aceptaciones() -> BTreeMap<String, Aceptacion> {
    let r = ruta_aceptaciones();
    if let Some(m) = migrar_json(&r, "licencias.json", guardar_aceptaciones) {
        return m;
    }
    let Ok(b) = std::fs::read(&r) else { return BTreeMap::new() };
    let leer = || -> Result<BTreeMap<String, Aceptacion>> {
        let (mut l, _) = crate::programar::codec::Lector::nuevo(&b, b"LICA", 1)?;
        let mut m = BTreeMap::new();
        for _ in 0..l.u32()? {
            let repo = l.texto()?;
            m.insert(repo, Aceptacion { licencia: l.texto()?, url: l.texto()?, sha: l.texto()?, fecha: l.texto()?, gotv: l.texto()? });
        }
        Ok(m)
    };
    leer().unwrap_or_default()
}

fn guardar_aceptaciones(m: &BTreeMap<String, Aceptacion>) -> Result<()> {
    let mut e = crate::programar::codec::Escritor::nuevo(b"LICA", 1);
    e.u32(m.len() as u32);
    for (repo, a) in m {
        for t in [repo, &a.licencia, &a.url, &a.sha, &a.fecha, &a.gotv] {
            e.texto(t);
        }
    }
    crate::programar::codec::guardar(&ruta_aceptaciones(), &e.bytes())
}

/// La aceptación VIGENTE: existe y es de la MISMA licencia que declara hoy el catálogo.
pub fn aceptacion(repo: &str) -> Option<Aceptacion> {
    let g = ficha(repo)?;
    leer_aceptaciones().remove(g.repo).filter(|a| a.licencia == g.licencia)
}

/// El usuario marcó la casilla: se guarda qué licencia, de qué commit y cuándo.
pub fn aceptar(repo: &str) -> Result<Aceptacion> {
    let g = ficha(repo).ok_or_else(|| anyhow!("{}", crate::tr!("{repo} no está en el catálogo de generadores", "{repo} is not in the generator catalog")))?;
    let ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0);
    let a = Aceptacion { licencia: g.licencia.into(), url: g.url_licencia.into(), sha: g.sha.into(), fecha: crate::serve::rfc3339(ms), gotv: env!("CARGO_PKG_VERSION").into() };
    let mut m = leer_aceptaciones();
    m.insert(g.repo.into(), a.clone());
    guardar_aceptaciones(&m)?;
    Ok(a)
}

/// Pedido del Portador (2026-11-13): en Configuración no se acepta a ciegas. Primero «Leer» abre la
/// página de la licencia (y queda anotado aquí, con la URL que se abrió); sólo entonces se desbloquea
/// «Aceptar», y sólo con la licencia aceptada, «Bajar». Si la URL del catálogo cambia, hay que volver a leer.
fn ruta_leidas() -> PathBuf {
    ruta_aceptaciones().with_file_name("licencias-leidas.gotv")
}

fn leer_leidas() -> BTreeMap<String, (String, String)> {
    let r = ruta_leidas();
    if let Some(m) = migrar_json(&r, "licencias-leidas.json", guardar_leidas) {
        return m;
    }
    let Ok(b) = std::fs::read(&r) else { return BTreeMap::new() };
    let leer = || -> Result<BTreeMap<String, (String, String)>> {
        let (mut l, _) = crate::programar::codec::Lector::nuevo(&b, b"LEID", 1)?;
        let mut m = BTreeMap::new();
        for _ in 0..l.u32()? {
            m.insert(l.texto()?, (l.texto()?, l.texto()?));
        }
        Ok(m)
    };
    leer().unwrap_or_default()
}

fn guardar_leidas(m: &BTreeMap<String, (String, String)>) -> Result<()> {
    let mut e = crate::programar::codec::Escritor::nuevo(b"LEID", 1);
    e.u32(m.len() as u32);
    for (repo, (url, fecha)) in m {
        for t in [repo, url, fecha] {
            e.texto(t);
        }
    }
    crate::programar::codec::guardar(&ruta_leidas(), &e.bytes())
}

/// Anota que la licencia de `repo` se abrió para leerla (la URL vigente del catálogo y la fecha).
pub fn marcar_leida(repo: &str) -> Result<&'static str> {
    let g = ficha(repo).ok_or_else(|| anyhow!("{}", crate::tr!("{repo} no está en el catálogo de generadores", "{repo} is not in the generator catalog")))?;
    let ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0);
    let mut m = leer_leidas();
    m.insert(g.repo.into(), (g.url_licencia.into(), crate::serve::rfc3339(ms)));
    guardar_leidas(&m)?;
    Ok(g.url_licencia)
}

/// ¿Se abrió la licencia VIGENTE de `repo` (la misma URL del catálogo)?
pub fn leida(repo: &str) -> bool {
    ficha(repo).is_some_and(|g| leer_leidas().get(g.repo).is_some_and(|(url, _)| url == g.url_licencia))
}

/// `aceptar` desde la interfaz: exige haber abierto antes la licencia con «Leer».
pub fn aceptar_leida(repo: &str) -> Result<Aceptacion> {
    if !leida(repo) {
        bail!("{}", crate::i18n::t("primero lee la licencia (botón «Leer»): después se desbloquea «Aceptar»", "read the license first («Read» button): then «Accept» unlocks"));
    }
    aceptar(repo)
}

pub fn revocar(repo: &str) -> Result<()> {
    let mut m = leer_aceptaciones();
    if let Some(g) = ficha(repo) {
        m.remove(g.repo);
    }
    guardar_aceptaciones(&m)
}

/// El generador de la voz (M2) y dónde está: `GOTV_VOZ_DIR` manda; si no, el del catálogo, bajado
/// con su licencia aceptada (`gotv generadores aceptar|bajar`). CHAOS jamás acepta por el Portador.
pub const VOZ_REPO: &str = "Qwen/Qwen3-TTS-12Hz-1.7B-Base";
/// La conversión a MLX del mismo modelo (mismos pesos), tal como la guarda la caché de Hugging Face.
const VOZ_CONVERSION: &str = "models--mlx-community--Qwen3-TTS-12Hz-1.7B-Base-bf16";

pub fn voz_dir() -> anyhow::Result<std::path::PathBuf> {
    if let Some(d) = std::env::var_os("GOTV_VOZ_DIR") {
        return Ok(d.into());
    }
    let g = ficha(VOZ_REPO).ok_or_else(|| gotv_engine::anyhow_tr!("{VOZ_REPO} falta en el catálogo", "{VOZ_REPO} is missing from the catalog"))?;
    let d = carpeta(g);
    let completa = |d: &std::path::Path| d.join("config.json").is_file() && d.join("speech_tokenizer/config.json").is_file();
    if completa(&d) {
        return Ok(d);
    }
    // con la licencia ACEPTADA, sirve la conversión de mlx-community que ya esté en la caché de
    // Hugging Face: medido, el talker tiene los mismos 404 tensores (nombre, tipo y forma) que el oficial
    // y el speech_tokenizer el mismo sha (no hace falta bajar 4,5 GB otra vez)
    if aceptacion(VOZ_REPO).is_some() {
        // la caché de Hugging Face donde ELLOS la ponen: HF_HUB_CACHE, HF_HOME/hub o ~/.cache/huggingface/hub
        let hub = std::env::var_os("HF_HUB_CACHE")
            .map(std::path::PathBuf::from)
            .or_else(|| std::env::var_os("HF_HOME").map(|h| std::path::PathBuf::from(h).join("hub")))
            .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".cache/huggingface/hub")));
        if let Some(hub) = hub {
            let snaps = hub.join(VOZ_CONVERSION).join("snapshots");
            if let Some(d) = std::fs::read_dir(&snaps).ok().into_iter().flatten().filter_map(|e| e.ok()).map(|e| e.path()).find(|p| completa(p)) {
                return Ok(d);
            }
        }
    }
    anyhow::bail!(
        "{}",
        crate::tr!(
            "la voz de GOTV usa {} ({}, {:.1} GB): lee su licencia y, si la aceptas, en Configuración → Generadores o con\n  gotv generadores aceptar {VOZ_REPO}\n  gotv generadores bajar {VOZ_REPO}\n(si ya está la copia de mlx-community en la caché de Hugging Face, basta aceptar)",
            "GOTV's voice uses {} ({}, {:.1} GB): read its license and, if you accept it, in Settings → Generators or with\n  gotv generadores aceptar {VOZ_REPO}\n  gotv generadores bajar {VOZ_REPO}\n(if the mlx-community copy is already in the Hugging Face cache, accepting is enough)",
            g.nombre,
            g.licencia,
            g.bytes_repo as f64 / 1e9
        )
    )
}

/// El generador de imagen (M4) y dónde está: `GOTV_IMAGEN_DIR` manda; si no, el del catálogo,
/// bajado ENTERO (texto, transformador, VAE) y con su licencia aceptada. CHAOS no acepta por nadie.
pub const IMAGEN_REPO: &str = "black-forest-labs/FLUX.2-klein-4B";

pub fn imagen_dir() -> anyhow::Result<std::path::PathBuf> {
    if let Some(d) = std::env::var_os("GOTV_IMAGEN_DIR") {
        return Ok(d.into());
    }
    let g = ficha(IMAGEN_REPO).ok_or_else(|| gotv_engine::anyhow_tr!("{IMAGEN_REPO} falta en el catálogo", "{IMAGEN_REPO} is missing from the catalog"))?;
    let d = carpeta(g);
    let completa = ["model_index.json", "tokenizer/tokenizer.json", "text_encoder/config.json", "transformer/diffusion_pytorch_model.safetensors", "vae/diffusion_pytorch_model.safetensors"]
        .iter()
        .all(|f| d.join(f).is_file());
    if aceptacion(IMAGEN_REPO).is_some() && completa {
        return Ok(d);
    }
    anyhow::bail!(
        "{}",
        crate::tr!(
            "las imágenes de GOTV usan {} ({}, {:.1} GB): lee su licencia y, si la aceptas, en Configuración → Generadores o con\n  gotv generadores aceptar {IMAGEN_REPO}\n  gotv generadores bajar {IMAGEN_REPO}",
            "GOTV's images use {} ({}, {:.1} GB): read its license and, if you accept it, in Settings → Generators or with\n  gotv generadores aceptar {IMAGEN_REPO}\n  gotv generadores bajar {IMAGEN_REPO}",
            g.nombre,
            g.licencia,
            16.0
        )
    )
}

/// Ola PRO · klein BASE (sin destilar: CFG y prompt negativo). Sólo su transformador: el texto y el VAE
/// son los de klein.
pub const IMAGEN_BASE_REPO: &str = "black-forest-labs/FLUX.2-klein-base-4B";

/// La carpeta del transformador de klein base si está aceptado y bajado (`GOTV_IMAGEN_BASE_DIR` manda).
/// Si no, el error dice dónde aceptarlo y bajarlo (la interfaz, como todo modelo).
pub fn imagen_base_transformador() -> anyhow::Result<std::path::PathBuf> {
    // `GOTV_IMAGEN_BASE_DIR` manda (las pruebas), pero sólo si de verdad trae el transformador
    if let Some(d) = std::env::var_os("GOTV_IMAGEN_BASE_DIR").map(|d| std::path::PathBuf::from(d).join("transformer")) {
        if d.join("diffusion_pytorch_model.safetensors").is_file() && d.join("config.json").is_file() {
            return Ok(d);
        }
        anyhow::bail!("{}", crate::tr!("GOTV_IMAGEN_BASE_DIR ({}) no trae el transformador de klein base", "GOTV_IMAGEN_BASE_DIR ({}) does not hold the klein base transformer", d.display()));
    }
    let g = ficha(IMAGEN_BASE_REPO).ok_or_else(|| gotv_engine::anyhow_tr!("{IMAGEN_BASE_REPO} falta en el catálogo", "{IMAGEN_BASE_REPO} is missing from the catalog"))?;
    let t = carpeta(g).join("transformer");
    if aceptacion(IMAGEN_BASE_REPO).is_some() && t.join("diffusion_pytorch_model.safetensors").is_file() && t.join("config.json").is_file() {
        return Ok(t);
    }
    anyhow::bail!(
        "{}",
        crate::tr!(
            "el prompt negativo y la guía (CFG) piden {} ({}, 7,8 GB: sólo su transformador). Léela y, si la aceptas, bájala en Configuración → Generadores.",
            "the negative prompt and guidance (CFG) need {} ({}, 7.8 GB: only its transformer). Read it and, if you accept it, download it in Settings → Generators.",
            g.nombre,
            g.licencia
        )
    )
}

/// Ola PRO · el DiT BASE de ACE-Step (CFG; separar pistas, añadir una pista, acompañar una voz).
pub const MUSICA_BASE_REPO: &str = "ACE-Step/acestep-v15-base";

/// La carpeta del DiT base de ACE-Step si está aceptado y bajado (`GOTV_MUSICA_BASE_DIR` manda).
pub fn musica_base_dir() -> anyhow::Result<std::path::PathBuf> {
    // `GOTV_MUSICA_BASE_DIR` manda (las pruebas), pero sólo si de verdad trae el modelo
    if let Some(d) = std::env::var_os("GOTV_MUSICA_BASE_DIR").map(std::path::PathBuf::from) {
        if ["model.safetensors", "config.json", "silence_latent.pt"].iter().all(|f| d.join(f).is_file()) {
            return Ok(d);
        }
        anyhow::bail!("{}", crate::tr!("GOTV_MUSICA_BASE_DIR ({}) no trae el DiT base", "GOTV_MUSICA_BASE_DIR ({}) does not hold the base DiT", d.display()));
    }
    let g = ficha(MUSICA_BASE_REPO).ok_or_else(|| gotv_engine::anyhow_tr!("{MUSICA_BASE_REPO} falta en el catálogo", "{MUSICA_BASE_REPO} is missing from the catalog"))?;
    let d = carpeta(g);
    if aceptacion(MUSICA_BASE_REPO).is_some() && ["model.safetensors", "config.json", "silence_latent.pt"].iter().all(|f| d.join(f).is_file()) {
        return Ok(d);
    }
    anyhow::bail!(
        "{}",
        crate::tr!(
            "separar pistas, añadir una pista o acompañar una voz piden {} ({}, 4,8 GB). Léela y, si la aceptas, bájala en Configuración → Generadores.",
            "splitting stems, adding a track or accompanying a vocal need {} ({}, 4.8 GB). Read it and, if you accept it, download it in Settings → Generators.",
            g.nombre,
            g.licencia
        )
    )
}

/// Ola PRO · PRO-4: el DiT de música elegido («sft», «xl-turbo», «xl-sft», «xl-base», «base»); `None` = el
/// turbo de siempre (el del repo de ACE-Step 1.5).
pub fn musica_modelo_repo(modelo: &str) -> Option<&'static str> {
    Some(match modelo.trim().to_lowercase().as_str() {
        "sft" => "ACE-Step/acestep-v15-sft",
        "xl-turbo" | "xl" => "ACE-Step/acestep-v15-xl-turbo",
        "xl-sft" => "ACE-Step/acestep-v15-xl-sft",
        "xl-base" => "ACE-Step/acestep-v15-xl-base",
        "base" => MUSICA_BASE_REPO,
        _ => return None,
    })
}

/// Un modelo del catálogo, con su licencia aceptada y sus pesos (de un archivo o en partes) bajados: su
/// carpeta. Si no, el error dice cuál falta y dónde se acepta.
fn modelo_bajado(repo: &str, para: &str) -> anyhow::Result<PathBuf> {
    let g = ficha(repo).ok_or_else(|| gotv_engine::anyhow_tr!("{repo} falta en el catálogo", "{repo} is missing from the catalog"))?;
    let d = carpeta(g);
    let pesos = d.join("model.safetensors").is_file() || d.join("model.safetensors.index.json").is_file();
    if aceptacion(repo).is_some() && d.join("config.json").is_file() && pesos {
        return Ok(d);
    }
    anyhow::bail!(
        "{}",
        crate::tr!(
            "{para} usa {} ({}, {:.1} GB). Léela y, si la aceptas, bájala en Configuración → Generadores.",
            "{para} uses {} ({}, {:.1} GB). Read it and, if you accept it, download it in Settings → Generators.",
            g.nombre,
            g.licencia,
            g.bytes_repo as f64 / 1e9
        )
    )
}

/// La carpeta del DiT de música elegido (`None` = el turbo de siempre).
pub fn musica_modelo_dir(modelo: &str) -> anyhow::Result<Option<PathBuf>> {
    match musica_modelo_repo(modelo) {
        None => Ok(None),
        Some(r) if r == MUSICA_BASE_REPO => musica_base_dir().map(Some),
        Some(r) => modelo_bajado(r, crate::i18n::t("ese modelo de música", "that music model")).map(Some),
    }
}

/// El panel de Música: qué opciones del motor están en ESTA Mac (aceptadas y bajadas), por su clave del panel.
/// Así no se elige a ciegas un modelo que no está (2026-11-19: «xl-sft» y el planificador «4b» sin bajar).
pub fn musica_listos() -> serde_json::Value {
    let mut m = serde_json::Map::new();
    m.insert("turbo".into(), serde_json::json!(musica_dir().is_ok()));
    for clave in ["sft", "xl-turbo", "xl-sft", "xl-base"] {
        m.insert(clave.into(), serde_json::json!(musica_modelo_dir(clave).is_ok()));
    }
    m.insert("base".into(), serde_json::json!(musica_base_dir().is_ok()));
    m.insert("4b".into(), serde_json::json!(musica_lm_dir("4b").is_ok()));
    serde_json::Value::Object(m)
}

/// El LM planificador de 4B (Ola PRO · PRO-4).
pub const MUSICA_LM4B_REPO: &str = "ACE-Step/acestep-5Hz-lm-4B";

/// Lo que le falta a una función de la app para correr en ESTA Mac: el repo del catálogo que hay que
/// aceptar y bajar (su fila en Configuración → Generadores), o `None` si está todo. Con eso la ventana lleva
/// a esa fila en cuanto se enciende la función (el Portador, 2026-11-20: «si no lo tengo descargado, que
/// me lleve a configuración a descargarlo»).
/// `uso`: oir, hablar, manos (oír + hablar), imagen, video, musica, efecto, buscar. `pro`: los ajustes del
/// panel PRO de ese perfil, porque lo elegido ahí también pide modelos (klein base, el DiT, el planificador,
/// el separador…).
pub fn falta_para(uso: &str, pro: &serde_json::Value) -> Option<&'static str> {
    // un repo propio por variable de entorno (fuera del catálogo) no tiene fila a la que llevar
    let del_catalogo = |r: &str| ficha(r).map(|g| g.repo);
    match uso {
        "oir" => crate::audio::whisper_dir().is_none().then(|| del_catalogo(&crate::audio::stt_repo())).flatten(),
        "hablar" => voz_dir().is_err().then_some(VOZ_REPO),
        "manos" => falta_para("oir", pro).or_else(|| falta_para("hablar", pro)),
        "imagen" if imagen_dir().is_err() => Some(IMAGEN_REPO),
        "imagen" => (pro["base"].as_bool() == Some(true) && imagen_base_transformador().is_err()).then_some(IMAGEN_BASE_REPO),
        "video" => video_dir().is_err().then_some(VIDEO_REPO),
        "efecto" => efecto_dir().is_err().then_some(EFECTO_REPO),
        "buscar" => crate::rag::embedder_dir().is_none().then(|| del_catalogo(&crate::rag::embed_repo())).flatten(),
        "musica" => musica_falta(&serde_json::from_value(pro.clone()).unwrap_or_default()),
        _ => None,
    }
}

/// Música: el primer modelo que falta de lo que pide el panel, en el orden en que el núcleo los usa al
/// generar (el turbo con su planificador, el DiT elegido, el base, el planificador de 4B, el separador).
pub fn musica_falta(p: &crate::pro::ProMusica) -> Option<&'static str> {
    if musica_dir().is_err() {
        return Some(MUSICA_REPO);
    }
    let tarea = !p.tarea.trim().is_empty();
    let modelo = p.modelo.trim();
    if !modelo.is_empty() && modelo != "turbo" && !tarea && musica_modelo_dir(modelo).is_err() {
        return musica_modelo_repo(modelo);
    }
    if (tarea || p.base) && musica_base_dir().is_err() {
        return Some(MUSICA_BASE_REPO);
    }
    if musica_lm_dir(&p.planificador).is_err() {
        return Some(MUSICA_LM4B_REPO);
    }
    if !p.separar.is_empty() {
        let m = separador_para(&p.separar, p.separador == "ft");
        if separador_dir(m).is_err() {
            return Some(separador_repo(m));
        }
    }
    None
}

/// El generador que nombra un aviso de «falta» (un error que manda a Configuración → Generadores): su repo,
/// para que la ventana ofrezca ir a su fila. Primero por el repo escrito; si no, por el nombre más largo
/// que aparezca («ACE-Step 1.5 SFT» antes que «ACE-Step 1.5»).
pub fn repo_de_aviso(texto: &str) -> Option<&'static str> {
    if !(texto.contains("Generadores") || texto.contains("Generators")) {
        return None;
    }
    CATALOGO
        .iter()
        .filter(|g| texto.contains(g.repo))
        .max_by_key(|g| g.repo.len())
        .or_else(|| CATALOGO.iter().filter(|g| texto.contains(g.nombre)).max_by_key(|g| g.nombre.len()))
        .map(|g| g.repo)
}

/// La carpeta del LM planificador elegido («4b»; `None` = el de 1,7B de siempre).
pub fn musica_lm_dir(planificador: &str) -> anyhow::Result<Option<PathBuf>> {
    match planificador.trim().to_lowercase().as_str() {
        "4b" => modelo_bajado(MUSICA_LM4B_REPO, crate::i18n::t("el planificador de 4B", "the 4B planner")).map(Some),
        _ => Ok(None),
    }
}

/// El generador de música (M7) y dónde está: `GOTV_MUSICA_DIR` manda; si no, el del catálogo,
/// bajado entero (texto, DiT, VAE) y con su licencia aceptada.
pub const MUSICA_REPO: &str = "ACE-Step/Ace-Step1.5";

pub fn musica_dir() -> anyhow::Result<std::path::PathBuf> {
    if let Some(d) = std::env::var_os("GOTV_MUSICA_DIR") {
        return Ok(d.into());
    }
    let g = ficha(MUSICA_REPO).ok_or_else(|| gotv_engine::anyhow_tr!("{MUSICA_REPO} falta en el catálogo", "{MUSICA_REPO} is missing from the catalog"))?;
    let d = carpeta(g);
    let completa = [
        "Qwen3-Embedding-0.6B/tokenizer.json",
        "Qwen3-Embedding-0.6B/model.safetensors",
        "acestep-v15-turbo/model.safetensors",
        "acestep-v15-turbo/silence_latent.pt",
        "vae/diffusion_pytorch_model.safetensors",
        // M7b: el LM planificador
        "acestep-5Hz-lm-1.7B/model.safetensors",
        "acestep-5Hz-lm-1.7B/tokenizer.json",
    ]
        .iter()
        .all(|f| d.join(f).is_file());
    if aceptacion(MUSICA_REPO).is_some() && completa {
        return Ok(d);
    }
    anyhow::bail!(
        "{}",
        crate::tr!(
            "la música de GOTV usa {} ({}, {:.1} GB): lee su licencia y, si la aceptas, en Configuración → Generadores o con\n  gotv generadores aceptar {MUSICA_REPO}\n  gotv generadores bajar {MUSICA_REPO}",
            "GOTV's music uses {} ({}, {:.1} GB): read its license and, if you accept it, in Settings → Generators or with\n  gotv generadores aceptar {MUSICA_REPO}\n  gotv generadores bajar {MUSICA_REPO}",
            g.nombre,
            g.licencia,
            g.bytes_repo as f64 / 1e9
        )
    )
}

/// El motor de imagen vive en `gotv-mlx`; lo registra el binario (CLI o app) al arrancar:
/// (modelo, descripción, ancho, alto, pasos, semilla, guías (hasta 4), salida .png, progreso, cancelar) → (ancho, alto).
/// Con guías (SVG/PNG/JPEG, M5) y ancho/alto 0 la imagen sale del tamaño de la primera; con tamaño, de ése.
pub type MotorImagen = fn(&Path, &str, u32, u32, u32, u64, &[PathBuf], &serde_json::Value, &Path, &(dyn Fn(String) + Sync), &std::sync::atomic::AtomicBool) -> Result<(u32, u32)>;
static MOTOR_IMAGEN: std::sync::OnceLock<MotorImagen> = std::sync::OnceLock::new();

pub fn registrar_motor_imagen(f: MotorImagen) {
    let _ = MOTOR_IMAGEN.set(f);
}

/// Genera una imagen con el modelo del catálogo (licencia aceptada y bajado) y deja su ficha de
/// licencia al lado. Devuelve (ancho, alto, ruta de la ficha).
#[allow(clippy::too_many_arguments)]
pub fn generar_imagen(prompt: &str, ancho: u32, alto: u32, pasos: u32, semilla: u64, guias: &[PathBuf], opciones: &serde_json::Value, salida: &Path, progreso: &(dyn Fn(String) + Sync), cancelar: &std::sync::atomic::AtomicBool) -> Result<(u32, u32, PathBuf)> {
    let dir = imagen_dir()?;
    let f = MOTOR_IMAGEN.get().ok_or_else(|| anyhow!("{}", crate::i18n::t("este binario no trae el motor de imagen", "this binary has no image engine")))?;
    if let Some(d) = salida.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(d)?;
    }
    let (w, h) = f(&dir, prompt, ancho, alto, pasos, semilla, guias, opciones, salida, progreso, cancelar)?;
    let ficha = escribir_ficha(salida, IMAGEN_REPO)?;
    Ok((w, h, ficha))
}

/// El motor de música vive en `gotv-mlx`; lo registra el binario (CLI o app) al arrancar:
/// (modelo, descripción, letra, idioma, segundos, bpm, tonalidad, compás, semilla, salida .wav,
/// progreso, cancelar) → segundos de audio.
/// Devuelve los segundos de audio y lo que decidió el LM planificador (bpm · tono · compás), si planeó.
pub type MotorMusica =
    fn(&Path, &str, &str, &str, f32, Option<u32>, &str, &str, u64, Option<(&Path, Option<(f32, f32)>, f32)>, &serde_json::Value, &Path, &(dyn Fn(String) + Sync), &std::sync::atomic::AtomicBool) -> Result<(f32, Option<String>)>;
static MOTOR_MUSICA: std::sync::OnceLock<MotorMusica> = std::sync::OnceLock::new();

pub fn registrar_motor_musica(f: MotorMusica) {
    let _ = MOTOR_MUSICA.set(f);
}

/// Lo que se le pide al motor de música.
#[derive(Clone, Debug, Default)]
pub struct PedidoMusica {
    pub descripcion: String,
    pub letra: String,
    pub idioma: String,
    pub segundos: f32,
    pub bpm: Option<u32>,
    pub tonalidad: String,
    pub compas: String,
    pub semilla: u64,
    /// M7d: un audio propio para versionar (cover) o, con `repintar`, rehacer una parte (repaint)
    pub fuente: Option<PathBuf>,
    /// (desde, hasta) en segundos; hasta < 0 = hasta el final
    pub repintar: Option<(f32, f32)>,
    /// cuánto guía el audio propio en un cover (1 = toda la pieza; ~0,2 = sólo el arranque)
    pub fuerza_cover: f32,
    /// Ola PRO: shift, pensar, temperatura, cfg_lm, top_p, top_k, excluir, ruido_cover, fuerza_repintar
    /// (todo opcional; lo que falta, el valor oficial)
    pub opciones: serde_json::Value,
}

/// Genera música con el modelo del catálogo (licencia aceptada y bajado) y deja su ficha de
/// licencia al lado. Devuelve (segundos de audio, ruta de la ficha).
pub fn generar_musica(p: &PedidoMusica, salida: &Path, progreso: &(dyn Fn(String) + Sync), cancelar: &std::sync::atomic::AtomicBool) -> Result<(f32, PathBuf, Option<String>)> {
    let dir = musica_dir()?;
    let f = MOTOR_MUSICA.get().ok_or_else(|| anyhow!("{}", crate::i18n::t("este binario no trae el motor de música", "this binary has no music engine")))?;
    if let Some(d) = salida.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(d)?;
    }
    let (s, plan) = f(&dir, &p.descripcion, &p.letra, &p.idioma, p.segundos, p.bpm, &p.tonalidad, &p.compas, p.semilla, p.fuente.as_deref().map(|f| (f, p.repintar, p.fuerza_cover)), &p.opciones, salida, progreso, cancelar)?;
    let ficha = escribir_ficha(salida, MUSICA_REPO)?;
    Ok((s, ficha, plan))
}

/// El generador de efectos de sonido (M7c) y dónde está: `GOTV_EFECTO_DIR` manda; si no, el del
/// catálogo, bajado entero (texto, DiT, DAC) y con su licencia aceptada.
pub const EFECTO_REPO: &str = "OpenMOSS-Team/MOSS-SoundEffect-v2.0";

pub fn efecto_dir() -> anyhow::Result<std::path::PathBuf> {
    if let Some(d) = std::env::var_os("GOTV_EFECTO_DIR") {
        return Ok(d.into());
    }
    let g = ficha(EFECTO_REPO).ok_or_else(|| gotv_engine::anyhow_tr!("{EFECTO_REPO} falta en el catálogo", "{EFECTO_REPO} is missing from the catalog"))?;
    let d = carpeta(g);
    let completa = ["tokenizer/tokenizer.json", "text_encoder/config.json", "text_encoder/model.safetensors.index.json", "transformer/config.json", "transformer/diffusion_pytorch_model.safetensors", "vae/vae_128d_48k.pth"]
        .iter()
        .all(|f| d.join(f).is_file());
    if aceptacion(EFECTO_REPO).is_some() && completa {
        return Ok(d);
    }
    anyhow::bail!(
        "{}",
        crate::tr!(
            "los efectos de sonido de GOTV usan {} ({}, {:.1} GB): lee su licencia y, si la aceptas, en Configuración → Generadores o con\n  gotv generadores aceptar {EFECTO_REPO}\n  gotv generadores bajar {EFECTO_REPO}",
            "GOTV's sound effects use {} ({}, {:.1} GB): read its license and, if you accept it, in Settings → Generators or with\n  gotv generadores aceptar {EFECTO_REPO}\n  gotv generadores bajar {EFECTO_REPO}",
            g.nombre,
            g.licencia,
            g.bytes_repo as f64 / 1e9
        )
    )
}

/// El motor de efectos vive en `gotv-mlx`; lo registra el binario (CLI o app) al arrancar:
/// (modelo, descripción, segundos, semilla, salida .wav, progreso, cancelar) → segundos de audio.
pub type MotorEfecto = fn(&Path, &str, f32, u64, &Path, &(dyn Fn(String) + Sync), &std::sync::atomic::AtomicBool) -> Result<f32>;
static MOTOR_EFECTO: std::sync::OnceLock<MotorEfecto> = std::sync::OnceLock::new();

pub fn registrar_motor_efecto(f: MotorEfecto) {
    let _ = MOTOR_EFECTO.set(f);
}

/// Genera un efecto de sonido con el modelo del catálogo y deja su ficha al lado: (segundos, ficha).
pub fn generar_efecto(descripcion: &str, segundos: f32, semilla: u64, salida: &Path, progreso: &(dyn Fn(String) + Sync), cancelar: &std::sync::atomic::AtomicBool) -> Result<(f32, PathBuf)> {
    let dir = efecto_dir()?;
    let f = MOTOR_EFECTO.get().ok_or_else(|| anyhow!("{}", crate::i18n::t("este binario no trae el motor de efectos de sonido", "this binary has no sound-effect engine")))?;
    if let Some(d) = salida.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(d)?;
    }
    let s = f(&dir, descripcion, segundos, semilla, salida, progreso, cancelar)?;
    let ficha = escribir_ficha(salida, EFECTO_REPO)?;
    Ok((s, ficha))
}

/// El generador de video (M6) y dónde está: `GOTV_VIDEO_DIR` manda; si no, el del catálogo, bajado
/// entero (UMT5, transformador, VAE) y con su licencia aceptada.
pub const VIDEO_REPO: &str = "Wan-AI/Wan2.2-TI2V-5B-Diffusers";

/// El negativo oficial de Wan (`sample_neg_prompt` de `wan/configs/shared_config.py`): el modo
/// profesional le SUMA el de la persona. Es copia del de `gotv_mlx::video::NEGATIVO` (el núcleo no
/// depende de MLX); la prueba `negativo_de_video_igual_al_del_motor` de la app los mantiene iguales.
pub const NEGATIVO_VIDEO: &str = "色调艳丽，过曝，静态，细节模糊不清，字幕，风格，作品，画作，画面，静止，整体发灰，最差质量，低质量，JPEG压缩残留，丑陋的，残缺的，多余的手指，画得不好的手部，画得不好的脸部，畸形的，毁容的，形态畸形的肢体，手指融合，静止不动的画面，杂乱的背景，三条腿，背景人很多，倒着走";

pub fn video_dir() -> anyhow::Result<std::path::PathBuf> {
    if let Some(d) = std::env::var_os("GOTV_VIDEO_DIR") {
        return Ok(d.into());
    }
    let g = ficha(VIDEO_REPO).ok_or_else(|| gotv_engine::anyhow_tr!("{VIDEO_REPO} falta en el catálogo", "{VIDEO_REPO} is missing from the catalog"))?;
    let d = carpeta(g);
    let completa = ["tokenizer/tokenizer.json", "text_encoder/model.safetensors.index.json", "transformer/config.json", "transformer/diffusion_pytorch_model.safetensors.index.json", "vae/diffusion_pytorch_model.safetensors"]
        .iter()
        .all(|f| d.join(f).is_file());
    if aceptacion(VIDEO_REPO).is_some() && completa {
        return Ok(d);
    }
    anyhow::bail!(
        "{}",
        crate::tr!(
            "los videos de GOTV usan {} ({}, {:.1} GB): lee su licencia y, si la aceptas, en Configuración → Generadores o con\n  gotv generadores aceptar {VIDEO_REPO}\n  gotv generadores bajar {VIDEO_REPO}",
            "GOTV's videos use {} ({}, {:.1} GB): read its license and, if you accept it, in Settings → Generators or with\n  gotv generadores aceptar {VIDEO_REPO}\n  gotv generadores bajar {VIDEO_REPO}",
            g.nombre,
            g.licencia,
            g.bytes_repo as f64 / 1e9
        )
    )
}

/// El motor de video vive en `gotv-mlx`; lo registra el binario (CLI o app) al arrancar:
/// (modelo, prompt, negativo (None = el oficial), segundos, ancho, alto, pasos, guía, semilla, imagen
/// del primer cuadro, clip a CONTINUAR (Ola PRO: sale unido), salida .mp4, progreso, cancelar) → (segundos, ancho, alto): con imagen el tamaño sale de su proporción.
pub type MotorVideo = fn(&Path, &str, Option<&str>, f32, u32, u32, u32, f32, u64, Option<&Path>, Option<&Path>, &Path, &(dyn Fn(String) + Sync), &std::sync::atomic::AtomicBool) -> Result<(f32, u32, u32)>;
static MOTOR_VIDEO: std::sync::OnceLock<MotorVideo> = std::sync::OnceLock::new();

pub fn registrar_motor_video(f: MotorVideo) {
    let _ = MOTOR_VIDEO.set(f);
}

/// Genera un video con el modelo del catálogo y deja su ficha al lado: (segundos, ancho, alto, ficha).
#[allow(clippy::too_many_arguments)]
pub fn generar_video(prompt: &str, negativo: Option<&str>, segundos: f32, ancho: u32, alto: u32, pasos: u32, guia: f32, semilla: u64, imagen: Option<&Path>, continuar: Option<&Path>, salida: &Path, progreso: &(dyn Fn(String) + Sync), cancelar: &std::sync::atomic::AtomicBool) -> Result<(f32, u32, u32, PathBuf)> {
    let dir = video_dir()?;
    let f = MOTOR_VIDEO.get().ok_or_else(|| anyhow!("{}", crate::i18n::t("este binario no trae el motor de video", "this binary has no video engine")))?;
    if let Some(d) = salida.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(d)?;
    }
    let (s, w, h) = f(&dir, prompt, negativo, segundos, ancho, alto, pasos, guia, semilla, imagen, continuar, salida, progreso, cancelar)?;
    let ficha = escribir_ficha(salida, VIDEO_REPO)?;
    Ok((s, w, h, ficha))
}

/// PRO-6 · el SEPARADOR de pistas (HTDemucs): «4» (voz, batería, bajo, resto), «6» (+ guitarra y
/// piano), «ft» (las 4 con un modelo afinado por pista: más fino, 4 veces más lento) o «bateria» (EXP-7:
/// una pista de batería partida en bombo, caja, platos y toms; sólo existe afinado por GOTV).
pub fn separador_repo(modelo: &str) -> &'static str {
    match modelo {
        "6" => "adefossez/HTDemucs-6s",
        "ft" => "adefossez/HTDemucs-ft",
        // «4» y «bateria» (un afinado de HTDemucs: hereda su licencia, hoy sin aclarar)
        _ => "adefossez/HTDemucs",
    }
}

/// Las pistas que sabe sacar cada separador (los nombres de `sources` del modelo).
pub fn pistas_del_separador(modelo: &str) -> &'static [&'static str] {
    if modelo == "bateria" {
        &["kick", "snare", "cymbals", "toms"]
    } else if modelo == "6" {
        &["drums", "bass", "other", "vocals", "guitar", "piano"]
    } else {
        &["drums", "bass", "other", "vocals"]
    }
}

/// El separador que toca: guitarra o piano SÓLO los saca el de 6 pistas (y entonces todo sale de él,
/// para que «resto» no las repita); si no, el afinado si se pide fino, o el de 4.
pub fn separador_para(pistas: &[String], fino: bool) -> &'static str {
    if pistas.iter().any(|p| p == "guitar" || p == "piano") {
        "6"
    } else if fino {
        "ft"
    } else {
        "4"
    }
}

fn tiene_yaml(d: &Path) -> bool {
    std::fs::read_dir(d).map(|r| r.filter_map(|e| e.ok()).any(|e| e.path().extension().and_then(|x| x.to_str()) == Some("yaml"))).unwrap_or(false)
}

/// EXP-7 · el repo del separador AFINADO por GOTV para `modelo` (su entrada del catálogo). El «ft» (uno
/// por pista) no tiene afinado propio.
pub fn afinado_repo(modelo: &str) -> Option<&'static str> {
    match modelo {
        "6" => Some("god-of-the-void/HTDemucs-6s-afinado"),
        "4" => Some("god-of-the-void/HTDemucs-afinado"),
        "bateria" => Some("god-of-the-void/HTDemucs-bateria-afinado"),
        _ => None,
    }
}

/// El afinado de `modelo` como cualquier otro peso: bajado (con su YAML) en `<modelos>/<repo>` y con SU
/// licencia aceptada en Configuración → Generadores. Nada más cuenta.
fn afinado_en(modelos: &Path, modelo: &str, aceptado: &dyn Fn(&str) -> bool) -> Option<PathBuf> {
    let repo = afinado_repo(modelo)?;
    let d = modelos.join(repo);
    (aceptado(repo) && tiene_yaml(&d)).then_some(d)
}

/// [`afinado_en`] con la carpeta de modelos y las aceptaciones de verdad.
fn afinado_de(modelo: &str) -> Option<PathBuf> {
    afinado_en(&crate::hub::models_dir(), modelo, &|r| aceptacion(r).is_some())
}

/// EXP-7 · ¿es una pieza de la batería partida (la sacan sólo los separadores «bateria»)?
pub fn es_de_bateria(pista: &str) -> bool {
    pistas_del_separador("bateria").contains(&pista)
}

/// El afinado de GOTV que se usaría AHORA para `modelo` (el mismo criterio que [`separador_dir`]): bajado,
/// con su licencia aceptada y sin `GOTV_SEPARADOR_DIR` mandando.
pub fn afinado_en_uso(modelo: &str) -> Option<PathBuf> {
    if std::env::var_os("GOTV_SEPARADOR_DIR").is_some() {
        return None;
    }
    afinado_de(modelo)
}

/// El repo con el que se separa de verdad: el del afinado de GOTV si está en uso, si no el de la base.
pub fn repo_del_separador(modelo: &str) -> &'static str {
    afinado_en_uso(modelo).and(afinado_repo(modelo)).unwrap_or_else(|| separador_repo(modelo))
}

/// El separador como lo ve la persona: el repo con el que separa y, si es un afinado, sobre qué base.
pub fn nombre_separador(modelo: &str) -> String {
    let repo = repo_del_separador(modelo);
    let base = separador_repo(modelo);
    if repo == base {
        repo.to_string()
    } else {
        crate::tr!("{repo} · afinado por GOTV sobre {base}", "{repo} · fine-tuned by GOTV from {base}")
    }
}

/// Los afinados de GOTV de un repo de la base que hay en ESTA Mac (para su tarjeta en Configuración).
pub fn afinados_de(repo: &str) -> Vec<String> {
    ["4", "6", "bateria"].iter().filter(|m| separador_repo(m) == repo).filter_map(|m| afinado_de(m)).filter_map(|d| d.file_name().map(|n| n.to_string_lossy().into_owned())).collect()
}

/// EXP-7 · partir la batería en bombo, caja, platos y toms. Si `audio` es una MEZCLA (no sólo batería),
/// primero sale su batería con el separador de 4 pistas (queda en disco y se reutiliza) y se parte ESA:
/// el afinado de batería aprendió sobre baterías solas.
pub fn separar_bateria(audio: &Path, solo_bateria: bool, progreso: &(dyn Fn(String) + Sync), cancelar: &std::sync::atomic::AtomicBool) -> Result<(Vec<(String, PathBuf)>, bool)> {
    let (fuente, separo_antes) = if solo_bateria {
        (audio.to_path_buf(), false)
    } else {
        let (pistas, separo) = separar_audio(audio, "4", progreso, cancelar)?;
        let d = pistas.into_iter().find(|(k, _)| k == "drums").map(|(_, r)| r).ok_or_else(|| anyhow!("{}", crate::i18n::t("el separador no sacó la batería", "the separator did not extract the drums")))?;
        (d, separo)
    };
    let (r, separo) = separar_audio(&fuente, "bateria", progreso, cancelar)?;
    Ok((r, separo || separo_antes))
}

/// ¿Es este audio YA una batería sola? (la pista «drums» que dejó el separador: `…-drums.wav`)
pub fn parece_bateria_sola(audio: &Path) -> bool {
    audio.file_stem().is_some_and(|s| s.to_string_lossy().ends_with("-drums"))
}

/// Si el separador usado es un afinado, su atribución (los datos CC BY) va también en la ficha.
pub fn anotar_atribucion(ficha: &Path, dir_modelo: &Path) -> Result<()> {
    if let Ok(t) = std::fs::read_to_string(dir_modelo.join("ATRIBUCION.md")) {
        let mut f = std::fs::read_to_string(ficha)?;
        f.push('\n');
        f.push_str(&t);
        std::fs::write(ficha, f)?;
    }
    Ok(())
}

/// La carpeta del separador: `GOTV_SEPARADOR_DIR` manda (con su YAML dentro); si no, el afinado de
/// GOTV ([`afinado_en`]), bajado y aceptado; si no, el de su base, bajado y aceptado. Los dos, como
/// cualquier peso de Configuración → Generadores.
pub fn separador_dir(modelo: &str) -> anyhow::Result<PathBuf> {
    if let Some(d) = std::env::var_os("GOTV_SEPARADOR_DIR").map(PathBuf::from) {
        if tiene_yaml(&d) {
            return Ok(d);
        }
        anyhow::bail!("{}", crate::tr!("GOTV_SEPARADOR_DIR ({}) no tiene el modelo", "GOTV_SEPARADOR_DIR ({}) has no model", d.display()));
    }
    let repo = separador_repo(modelo);
    if let Some(d) = afinado_de(modelo) {
        return Ok(d);
    }
    // el HTDemucs del catálogo NO parte la batería: sin su afinado no hay a quién caer
    let repo = if modelo == "bateria" { afinado_repo(modelo).unwrap_or(repo) } else { repo };
    let g = ficha(repo).ok_or_else(|| gotv_engine::anyhow_tr!("{repo} falta en el catálogo", "{repo} is missing from the catalog"))?;
    let d = carpeta(g);
    if aceptacion(repo).is_some() && tiene_yaml(&d) {
        return Ok(d);
    }
    anyhow::bail!(
        "{}",
        crate::tr!(
            "separar pistas usa {} ({}, {:.0} MB): lee su licencia y, si la aceptas, en Configuración → Generadores o con\n  gotv generadores aceptar {repo}\n  gotv generadores bajar {repo}",
            "splitting tracks uses {} ({}, {:.0} MB): read its license and, if you accept it, in Settings → Generators or with\n  gotv generadores aceptar {repo}\n  gotv generadores bajar {repo}",
            g.nombre,
            g.licencia,
            g.bytes_repo as f64 / 1e6
        )
    )
}

/// El motor del separador vive en `gotv-mlx`; lo registra el binario al arrancar:
/// (modelo, audio de entrada, carpeta de salida, nombre base, progreso, cancelar) → [(pista, ruta .wav)]
/// (a la frecuencia del audio de entrada, en coma flotante).
pub type MotorSeparar = fn(&Path, &Path, &Path, &str, &(dyn Fn(String) + Sync), &std::sync::atomic::AtomicBool) -> Result<Vec<(String, PathBuf)>>;
static MOTOR_SEPARAR: std::sync::OnceLock<MotorSeparar> = std::sync::OnceLock::new();

pub fn registrar_motor_separar(f: MotorSeparar) {
    let _ = MOTOR_SEPARAR.set(f);
}

/// Dónde quedan las pistas de `audio` con el separador `modelo`: `<audio>-pistas-<modelo>/<audio>-<pista>.wav`.
pub fn ruta_pista(audio: &Path, modelo: &str, pista: &str) -> PathBuf {
    let base = audio.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "audio".into());
    audio.with_file_name(format!("{base}-pistas-{modelo}")).join(format!("{base}-{pista}.wav")) // juez-idioma: datos (nombre de carpeta)
}

/// Separa `audio` en sus pistas (TODAS las del modelo, de una vez) con su ficha de licencia al lado
/// de cada una. Si ya estaban separadas DESPUÉS de la última vez que cambió el audio, no se repite.
/// Devuelve [(pista, ruta)] y si hubo que separar.
pub fn separar_audio(audio: &Path, modelo: &str, progreso: &(dyn Fn(String) + Sync), cancelar: &std::sync::atomic::AtomicBool) -> Result<(Vec<(String, PathBuf)>, bool)> {
    let pistas = pistas_del_separador(modelo);
    let cambio = std::fs::metadata(audio).and_then(|m| m.modified()).with_context(|| audio.display().to_string())?;
    let hechas: Vec<(String, PathBuf)> = pistas.iter().map(|p| (p.to_string(), ruta_pista(audio, modelo, p))).collect();
    if hechas.iter().all(|(_, r)| std::fs::metadata(r).and_then(|m| m.modified()).is_ok_and(|t| t >= cambio)) {
        return Ok((hechas, false));
    }
    let dir = separador_dir(modelo)?;
    // quien mira el avance sabe con QUÉ se separa (el afinado de GOTV o el del catálogo)
    progreso(nombre_separador(modelo));
    let f = MOTOR_SEPARAR.get().ok_or_else(|| anyhow!("{}", crate::i18n::t("este binario no trae el separador de pistas", "this binary has no track separator")))?;
    let salida = hechas[0].1.parent().map(Path::to_path_buf).unwrap_or_default();
    std::fs::create_dir_all(&salida)?;
    let base = audio.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "audio".into());
    let r = f(&dir, audio, &salida, &base, progreso, cancelar)?;
    for (_, ruta) in &r {
        // la ficha cita el repo con el que se separó (el afinado, con su atribución) y no sólo la base
        anotar_atribucion(&escribir_ficha(ruta, repo_del_separador(modelo))?, &dir)?;
    }
    Ok((r, true))
}

/// Dónde queda bajado (la misma carpeta de modelos de GOTV).
pub fn carpeta(g: &Generador) -> PathBuf {
    crate::hub::models_dir().join(g.repo)
}

/// Lo que hay en ESTA Mac de un generador (para la tarjeta de la GUI).
#[derive(Serialize, Clone, Debug)]
pub struct Estado {
    #[serde(flatten)]
    pub g: Generador,
    pub etiqueta: &'static str,
    pub area_nombre: &'static str,
    pub aviso: String,
    pub aceptada: Option<Aceptacion>,
    pub bytes_local: u64,
    pub token_hf: bool,
    /// se abrió su licencia con «Leer» (desbloquea «Aceptar»)
    pub leida: bool,
    /// EXP-7: los afinados de GOTV de este modelo que hay en ESTA Mac (se usan con su licencia aceptada)
    pub afinados: Vec<String>,
}

fn bytes_en(d: &Path) -> u64 {
    let Ok(rd) = std::fs::read_dir(d) else { return 0 };
    rd.filter_map(|e| e.ok())
        .map(|e| {
            let p = e.path();
            if p.is_dir() {
                bytes_en(&p)
            } else if p.extension().is_some_and(|x| x == "part") {
                0
            } else {
                e.metadata().map(|m| m.len()).unwrap_or(0)
            }
        })
        .sum()
}

pub fn estados() -> Vec<Estado> {
    let token = crate::llaves::has(LLAVE_HF);
    CATALOGO
        .iter()
        // uno propio aún sin publicar sólo lo ve la Mac que ya lo tiene
        .filter(|g| g.publicado || bytes_en(&carpeta(g)) > 0)
        .map(|g| Estado { g: g.clone(), etiqueta: g.clase.etiqueta(), area_nombre: g.area.nombre(), aviso: aviso(g), aceptada: aceptacion(g.repo), bytes_local: bytes_en(&carpeta(g)), token_hf: token, leida: leida(g.repo), afinados: afinados_de(g.repo) })
        .collect()
}

/// Lo que dice HOY la API de Hugging Face de un generador, y qué cambió respecto al catálogo.
#[derive(Serialize, Clone, Debug)]
pub struct Verificacion {
    pub repo: String,
    pub licencia: String,
    pub restringido: bool,
    pub sha: String,
    pub bytes: u64,
    pub cambios: Vec<String>,
}

/// La licencia de un `LICENSE` de GitHub (`github.com/<dueño>/<repo>/blob/<rama>/LICENSE`), leída de su
/// versión cruda y reconocida por su primera línea: «mit», «apache-2.0»… o `None` si no se reconoce.
pub fn licencia_de_texto(texto: &str) -> Option<String> {
    let t = texto.trim_start();
    if t.starts_with("MIT License") {
        Some("mit".into())
    } else if t.contains("Apache License") && t.contains("Version 2.0") {
        Some("apache-2.0".into())
    } else {
        None
    }
}

/// La licencia de los PESOS según la sección «License» de un README: si el autor dice que la de los
/// pesos no está clara, es `unknown` (sin licencia: sólo investigar). `None` si la sección no lo dice.
pub fn licencia_de_readme(texto: &str) -> Option<String> {
    let seccion = texto.split("\n## ").find(|s| s.to_ascii_lowercase().starts_with("license"))?;
    let s = seccion.to_ascii_lowercase();
    (s.contains("license of the weights is not clear") || (s.contains("weights") && s.contains("not clear"))).then(|| "unknown".into())
}

async fn licencia_de_github(url: &str) -> Option<String> {
    let (ruta, ancla) = url.split_once('#').unwrap_or((url, ""));
    let crudo = ruta.strip_prefix("https://github.com/")?.replacen("/blob/", "/", 1);
    let texto = crate::hub::get_texto(reqwest::Url::parse(&format!("https://raw.githubusercontent.com/{crudo}")).ok()?).await.ok()?;
    if ancla == "license" {
        licencia_de_readme(&texto)
    } else {
        licencia_de_texto(&texto)
    }
}

pub async fn verificar(g: &Generador) -> Result<Verificacion> {
    if !g.publicado {
        gotv_engine::bail_tr!("aún no está publicado en Hugging Face", "not published on Hugging Face yet");
    }
    let v =crate::hub::get_json(reqwest::Url::parse(&format!("https://huggingface.co/api/models/{}?blobs=true", g.repo))?).await?;
    let mut licencia = v["cardData"]["license_name"].as_str().or_else(|| v["cardData"]["license"].as_str()).unwrap_or("?").to_string();
    if licencia == "?" && !g.origen.is_empty() {
        // una conversión sin licencia en su ficha hereda la del oficial: se lee ahí
        let o = crate::hub::get_json(reqwest::Url::parse(&format!("https://huggingface.co/api/models/{}", g.origen))?).await?;
        licencia = o["cardData"]["license_name"].as_str().or_else(|| o["cardData"]["license"].as_str()).unwrap_or("?").to_string();
    }
    if licencia == "?" {
        // la ficha de HF no la declara y la licencia vive en el repositorio OFICIAL de GitHub (HTDemucs):
        // se lee ese LICENSE y se reconoce su texto
        if let Some(texto) = licencia_de_github(g.url_licencia).await {
            licencia = texto;
        }
    }
    let restringido = !matches!(v["gated"], serde_json::Value::Bool(false) | serde_json::Value::Null);
    let sha: String = v["sha"].as_str().unwrap_or("").chars().take(10).collect();
    let bytes: u64 = v["siblings"].as_array().into_iter().flatten().filter_map(|s| s["size"].as_u64()).sum();
    let mut cambios = Vec::new();
    if licencia != g.licencia {
        cambios.push(crate::tr!("LICENCIA: {} → {licencia} (las aceptaciones anteriores dejan de valer)", "LICENSE: {} → {licencia} (previous acceptances no longer apply)", g.licencia));
    }
    if restringido != g.restringido {
        cambios.push(crate::tr!("acceso restringido: {} → {restringido}", "gated access: {} → {restringido}", g.restringido));
    }
    if sha != g.sha {
        cambios.push(crate::tr!("commit nuevo: {} → {sha}", "new commit: {} → {sha}", g.sha));
    }
    Ok(Verificacion { repo: g.repo.into(), licencia, restringido, sha, bytes, cambios })
}

/// Baja un generador. Exige la licencia aceptada; en los restringidos, el token del Llavero.
pub async fn bajar(repo: &str, progress: &(dyn Fn(u64, u64) + Sync)) -> Result<PathBuf> {
    let g = ficha(repo).ok_or_else(|| anyhow!("{}", crate::tr!("{repo} no está en el catálogo de generadores", "{repo} is not in the generator catalog")))?;
    if !g.publicado {
        bail!("{}", crate::tr!("{} aún no está publicado en Hugging Face: llega con una versión próxima de GOTV", "{} is not published on Hugging Face yet: it arrives with an upcoming GOTV version", g.nombre));
    }
    if aceptacion(g.repo).is_none() {
        bail!(
            "{}",
            crate::tr!(
                "falta aceptar la licencia de {} ({}): `gotv generadores aceptar {}` o la casilla en Configuración → Generadores",
                "the license of {} ({}) has not been accepted: `gotv generadores aceptar {}` or the checkbox in Settings → Generators",
                g.nombre,
                g.licencia,
                g.repo
            )
        );
    }
    let token = crate::llaves::get(LLAVE_HF);
    if g.restringido && token.is_none() {
        bail!("{}", explicar_restringido(g));
    }
    let op = crate::hub::Descarga { token: token.as_deref(), prefijos: g.archivos, exigir_mlx: false };
    crate::hub::pull_con(g.repo, &op, progress).await.map_err(|e| {
        let txt = format!("{e:#}");
        if g.restringido && (txt.contains("401") || txt.contains("403")) {
            anyhow!("{}\n({txt})", explicar_restringido(g))
        } else {
            e
        }
    })
}

/// Qué hacer con un repo de acceso restringido: GOTV no acepta por el usuario.
pub fn explicar_restringido(g: &Generador) -> String {
    crate::tr!(
        "{} es de acceso restringido en Hugging Face: 1) entra con TU cuenta a https://huggingface.co/{} y acepta sus condiciones; 2) crea un token de lectura en https://huggingface.co/settings/tokens; 3) guárdalo en el Llavero de macOS con `gotv generadores token` (o en Configuración → Generadores). GOTV no acepta por ti.",
        "{} is gated on Hugging Face: 1) sign in with YOUR account at https://huggingface.co/{} and accept its conditions; 2) create a read token at https://huggingface.co/settings/tokens; 3) store it in the macOS Keychain with `gotv generadores token` (or in Settings → Generators). GOTV does not accept on your behalf.",
        g.nombre,
        g.repo
    )
}

/// La ficha de licencia que acompaña a un archivo generado: `<archivo>.licencia.txt`.
pub fn escribir_ficha(salida: &Path, repo: &str) -> Result<PathBuf> {
    let g = ficha(repo).ok_or_else(|| gotv_engine::anyhow_tr!("{repo} no está en el catálogo de generadores", "{repo} is not in the generator catalog"))?;
    let a = aceptacion(g.repo);
    let ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0);
    let nombre = salida.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let texto = format!(
        "{}\n\n{}: {}\n{}: {} ({})\n{}: {} — {}\n{}: {}\n{}: {}\n\n{}\n",
        crate::tr!("Ficha de licencia de {nombre}", "License sheet for {nombre}"),
        crate::i18n::t("Generado", "Generated"),
        crate::serve::rfc3339(ms),
        crate::i18n::t("Modelo", "Model"),
        g.nombre,
        g.repo,
        crate::i18n::t("Licencia", "License"),
        g.licencia,
        g.url_licencia,
        crate::i18n::t("Clase", "Class"),
        g.clase.etiqueta(),
        crate::i18n::t("Aceptada", "Accepted"),
        a.map(|a| if a.sha.is_empty() { crate::tr!("{} (sin publicar aún)", "{} (not published yet)", a.fecha) } else { format!("{} (commit {})", a.fecha, a.sha) }).unwrap_or_else(|| "—".into()),
        aviso(g)
    );
    let ruta = PathBuf::from(format!("{}.licencia.txt", salida.display()));
    std::fs::write(&ruta, texto)?;
    Ok(ruta)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Cada función sabe qué modelo le falta y su aviso lleva a esa fila (sin licencias aceptadas, todo falta).
    /// Otras pruebas ponen `GOTV_*_DIR` para todo el proceso: aquí sólo se mira lo que esas variables no tocan.
    #[test]
    fn cada_funcion_sabe_que_modelo_le_falta() {
        let d = crate::CarpetaPrueba::sin_crear("gotv-falta-para");
        std::fs::create_dir_all(&d).unwrap();
        licencias_de_prueba(Some(d.join("licencias.gotv")));
        if std::env::var_os("GOTV_STT_REPO").is_none() {
            assert_eq!(falta_para("manos", &serde_json::json!({})), Some(crate::audio::STT_REPO), "manos libres empieza por oír");
            assert_eq!(falta_para("oir", &serde_json::Value::Null), Some(crate::audio::STT_REPO));
        }
        assert_eq!(falta_para("nada-de-esto", &serde_json::json!({})), None);
        // el panel de música: lo elegido cuenta (detrás del turbo, si la prueba de al lado no lo puso)
        let r = falta_para("musica", &serde_json::json!({"modelo": "sft"}));
        assert!(matches!(r, Some(MUSICA_REPO) | Some("ACE-Step/acestep-v15-sft")), "{r:?}");
        // el aviso de un DiT o del planificador nombra SU fila, no la del turbo
        let e = format!("{:#}", musica_modelo_dir("sft").unwrap_err());
        assert_eq!(repo_de_aviso(&e), Some("ACE-Step/acestep-v15-sft"), "{e}");
        let e = format!("{:#}", musica_lm_dir("4b").unwrap_err());
        assert_eq!(repo_de_aviso(&e), Some(MUSICA_LM4B_REPO), "{e}");
        assert_eq!(repo_de_aviso("falló la red con ACE-Step 1.5"), None, "sin mandar a Generadores no hay fila");
        licencias_de_prueba(None);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn el_catalogo_es_coherente() {
        let mut vistos = std::collections::HashSet::new();
        for g in CATALOGO {
            assert!(crate::hub::valid_repo(g.repo), "{}", g.repo);
            assert!(vistos.insert(g.repo), "repetido: {}", g.repo);
            assert!(g.url_licencia.starts_with("https://"), "{}", g.repo);
            // publicado = commit verificado; sin publicar = sin commit, y sólo uno propio de GOTV con su base
            if g.publicado {
                assert_eq!(g.sha.len(), 10, "{}", g.repo);
            } else {
                assert!(g.sha.is_empty() && g.repo.starts_with("god-of-the-void/") && ficha(g.origen).is_some(), "{}", g.repo);
            }
            // la clase debe seguir a la licencia: las SPDX libres no pueden marcarse restrictivas, ni al revés
            match g.licencia {
                "apache-2.0" | "mit" => assert_eq!(g.clase, Clase::Libre, "{}", g.repo),
                "cc-by-4.0" => assert_eq!(g.clase, Clase::Atribucion, "{}", g.repo),
                "unknown" => assert_eq!(g.clase, Clase::SinLicencia, "{}", g.repo),
                _ => assert!(matches!(g.clase, Clase::Condiciones | Clase::NoComercial), "{}", g.repo),
            }
        }
        // restringido ≠ no libre: FLUX.1-schnell es Apache y de acceso restringido
        let s = ficha("black-forest-labs/FLUX.1-schnell").unwrap();
        assert!(s.restringido && s.clase == Clase::Libre);
        assert!(aviso(s).contains("black-forest-labs") && aviso(s).contains("apache-2.0"));
    }

    /// PRO-4: cada nombre del panel apunta a su repo del catálogo (MIT), y el turbo no necesita carpeta aparte.
    #[test]
    fn los_modelos_de_musica_apuntan_a_su_repo() {
        for (m, r) in [("sft", "ACE-Step/acestep-v15-sft"), ("xl-turbo", "ACE-Step/acestep-v15-xl-turbo"), ("xl-sft", "ACE-Step/acestep-v15-xl-sft"), ("xl-base", "ACE-Step/acestep-v15-xl-base"), ("base", MUSICA_BASE_REPO)] {
            assert_eq!(musica_modelo_repo(m), Some(r));
            let g = ficha(r).unwrap();
            assert!(g.area == Area::Musica && g.licencia == "mit", "{r}");
        }
        assert_eq!(musica_modelo_repo(""), None);
        assert_eq!(musica_modelo_repo("turbo"), None);
        assert!(musica_modelo_dir("turbo").unwrap().is_none());
        assert!(musica_lm_dir("").unwrap().is_none());
        assert!(ficha("ACE-Step/acestep-5Hz-lm-4B").is_some());
    }

    /// La licencia que vive en GitHub (HTDemucs) se reconoce por su texto; el separador elige modelo.
    #[test]
    fn licencia_de_github_y_separador() {
        assert_eq!(licencia_de_texto("MIT License\n\nCopyright (c) Meta Platforms").as_deref(), Some("mit"));
        assert_eq!(licencia_de_texto("                                 Apache License\n                           Version 2.0, January 2004").as_deref(), Some("apache-2.0"));
        assert_eq!(licencia_de_texto("GNU GENERAL PUBLIC LICENSE"), None);
        // falla #3137: el MIT de Demucs es del código; de los pesos el README dice «not clear»
        let readme = "# Demucs\n\n## Requirements\n\nMIT\n\n## License\n\nDemucs code is released under the MIT license as found in the [LICENSE](LICENSE) file.\nThe license of the weights is not clear: MusDB contains some data that were only usable for research purpose.\n";
        assert_eq!(licencia_de_readme(readme).as_deref(), Some("unknown"));
        assert_eq!(licencia_de_readme("# X\n\n## License\n\nMIT, code and weights.\n"), None);
        for r in ["adefossez/HTDemucs", "adefossez/HTDemucs-6s", "adefossez/HTDemucs-ft", "god-of-the-void/HTDemucs-afinado", "god-of-the-void/HTDemucs-6s-afinado", "god-of-the-void/HTDemucs-bateria-afinado"] {
            let g = ficha(r).unwrap();
            assert!(g.area == Area::Musica && g.licencia == "unknown" && g.clase == Clase::SinLicencia, "{r}: sin licencia clara, sólo investigación");
            assert!(g.url_licencia.ends_with("README.md#license"), "{r}: la licencia de los PESOS vive en el README");
        }
        let v = |p: &[&str]| p.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(separador_para(&v(&["vocals", "drums"]), false), "4");
        assert_eq!(separador_para(&v(&["vocals"]), true), "ft");
        assert_eq!(separador_para(&v(&["other", "piano"]), true), "6", "guitarra o piano mandan: 6 pistas");
        let r = ruta_pista(Path::new("generado/musica-1.wav"), "4", "vocals");
        assert_eq!(r, PathBuf::from("generado/musica-1-pistas-4/musica-1-vocals.wav"));
    }

    /// EXP-7 · las piezas de la batería las saca sólo su separador, y la batería sola se reconoce.
    #[test]
    fn la_bateria_partida_se_reconoce() {
        assert!(["kick", "snare", "cymbals", "toms"].iter().all(|p| es_de_bateria(p)));
        assert!(!es_de_bateria("drums") && !es_de_bateria("vocals"));
        assert!(parece_bateria_sola(Path::new("generado/cancion-pistas-4/cancion-drums.wav")));
        assert!(!parece_bateria_sola(Path::new("generado/cancion.wav")));
        assert_eq!(separador_para(&["guitar".into()], false), "6");
    }

    /// Medición en una Mac que tiene los tres afinados bajados y aceptados (la del Portador): el separador
    /// los elige sin cargar ningún modelo. `cargo test … -- --ignored afinados_de_esta_mac`.
    #[test]
    #[ignore]
    fn afinados_de_esta_mac() {
        for m in ["4", "6", "bateria"] {
            let d = separador_dir(m).unwrap();
            assert_eq!(d, crate::hub::models_dir().join(afinado_repo(m).unwrap()), "{m}");
            println!("{m}: {} → {}", nombre_separador(m), d.display());
        }
        assert_eq!(repo_del_separador("ft"), "adefossez/HTDemucs-ft", "el ft sigue con su base");
    }

    /// EXP-7 · el afinado de GOTV es un peso más: su repo en el catálogo (publicado en HF con su commit
    /// verificado y su base como origen), en `<modelos>/<repo>` con su YAML y con SU licencia aceptada;
    /// nada de carpetas del taller. Y su atribución acompaña a la ficha.
    #[test]
    fn el_afinado_es_un_peso_mas() {
        let d = crate::CarpetaPrueba::sin_crear("gotv-afinado");
        for m in ["4", "6", "bateria"] {
            let g = ficha(afinado_repo(m).unwrap()).unwrap();
            // hereda la licencia de su base: hoy ninguna clara (falla #3137); se baja de HF como cualquiera
            assert!(g.publicado && g.sha.len() == 10 && g.origen == separador_repo(m) && g.area == Area::Musica && g.licencia == ficha(g.origen).unwrap().licencia, "{m}");
        }
        assert_eq!(afinado_repo("ft"), None, "el ft no tiene afinado propio");
        let seis = d.join("god-of-the-void/HTDemucs-6s-afinado");
        std::fs::create_dir_all(&seis).unwrap();
        let todo = |_: &str| true;
        assert_eq!(afinado_en(&d, "6", &todo), None, "a medio bajar (sin YAML) no hay modelo");
        std::fs::write(seis.join("afinado.yaml"), "models: ['afinado']").unwrap();
        assert_eq!(afinado_en(&d, "6", &todo), Some(seis.clone()));
        let solo_base = |r: &str| r == "adefossez/HTDemucs-6s";
        assert_eq!(afinado_en(&d, "6", &solo_base), None, "aceptar la base no basta: cada peso con su licencia");
        assert_eq!(afinado_en(&d, "4", &todo), None, "cada afinado por su lado");
        let taller = d.join("propios/HTDemucs-afinado");
        std::fs::create_dir_all(&taller).unwrap();
        std::fs::write(taller.join("afinado.yaml"), "models: ['afinado']").unwrap();
        assert_eq!(afinado_en(&d, "4", &todo), None, "una carpeta fuera de su repo no cuenta");
        assert_eq!(pistas_del_separador("bateria"), ["kick", "snare", "cymbals", "toms"]);
        let ficha_p = d.join("pista.wav.licencia.txt");
        std::fs::write(&ficha_p, "Ficha").unwrap();
        anotar_atribucion(&ficha_p, &d).unwrap();
        assert_eq!(std::fs::read_to_string(&ficha_p).unwrap(), "Ficha", "sin ATRIBUCION.md no se toca");
        std::fs::write(seis.join("ATRIBUCION.md"), "Slakh2100 (CC BY 4.0)").unwrap();
        anotar_atribucion(&ficha_p, &seis).unwrap();
        assert_eq!(std::fs::read_to_string(&ficha_p).unwrap(), "Ficha\nSlakh2100 (CC BY 4.0)");
    }

    /// Una sola secuencia (la ruta de aceptaciones es del proceso: dos pruebas en paralelo chocarían).
    #[test]
    fn aceptar_bajar_caducar_y_ficha() {
        let d = crate::CarpetaPrueba::sin_crear("gotv-licencias");
        std::fs::create_dir_all(&d).unwrap();
        // el archivo de licencias de ESTE hilo (las demás pruebas siguen con el suyo); el motor va en este mismo hilo
        LICENCIAS_DE_PRUEBA.with(|c| *c.borrow_mut() = Some(d.join("licencias.gotv")));
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let repo = "hexgrad/Kokoro-82M";
        // lo aceptado ANTES (en JSON) se migra solo una vez, sin perder nada, y el viejo queda renombrado
        std::fs::write(
            d.join("licencias.json"),
            r#"{"black-forest-labs/FLUX.2-klein-4B":{"licencia":"apache-2.0","url":"u","sha":"s","fecha":"f","gotv":"0.1"}}"#,
        )
        .unwrap();
        std::fs::write(d.join("licencias-leidas.json"), r#"{"hexgrad/Kokoro-82M":["u","f"]}"#).unwrap();
        assert_eq!(leer_aceptaciones()["black-forest-labs/FLUX.2-klein-4B"].licencia, "apache-2.0");
        assert_eq!(leer_leidas()["hexgrad/Kokoro-82M"], ("u".to_string(), "f".to_string()));
        assert!(d.join("licencias.gotv").is_file() && d.join("licencias-leidas.gotv").is_file());
        assert!(!d.join("licencias.json").exists() && d.join("licencias.json.migrado").is_file());
        assert_eq!(leer_aceptaciones().len(), 1, "la segunda lectura ya es de lo propio");
        guardar_aceptaciones(&BTreeMap::new()).unwrap();
        guardar_leidas(&BTreeMap::new()).unwrap();
        // sin la casilla no hay aceptación ni descarga (se corta ANTES de tocar la red)
        assert!(aceptacion(repo).is_none());
        let e = rt.block_on(bajar(repo, &|_, _| {})).unwrap_err().to_string();
        assert!(e.contains("licencia") || e.contains("license"), "{e}");
        assert!(rt.block_on(bajar("org/no-existe", &|_, _| {})).is_err(), "fuera del catálogo no se baja");
        let a = aceptar(repo).unwrap();
        assert_eq!(a.licencia, "apache-2.0");
        assert_eq!(aceptacion(repo), Some(a.clone()));
        // si la licencia guardada ya no es la del catálogo, la aceptación no vale
        let mut m = leer_aceptaciones();
        m.get_mut(repo).unwrap().licencia = "otra".into();
        guardar_aceptaciones(&m).unwrap();
        assert!(aceptacion(repo).is_none(), "licencia cambiada → hay que volver a aceptar");
        // la ficha junto al archivo generado
        aceptar(repo).unwrap();
        let wav = d.join("hola.wav");
        std::fs::write(&wav, b"RIFF").unwrap();
        let f = escribir_ficha(&wav, repo).unwrap();
        let t = std::fs::read_to_string(&f).unwrap();
        assert!(f.ends_with("hola.wav.licencia.txt") && t.contains("hexgrad/Kokoro-82M") && t.contains("apache-2.0") && t.contains("GOTV"));
        revocar(repo).unwrap();
        assert!(aceptacion(repo).is_none());
        // desde la interfaz NO se acepta a ciegas: primero «Leer» (abre la licencia del catálogo)
        let e = aceptar_leida("adefossez/HTDemucs").unwrap_err().to_string();
        assert!(e.contains("Leer") || e.contains("Read"), "{e}");
        assert!(!leida("adefossez/HTDemucs"));
        assert_eq!(marcar_leida("adefossez/HTDemucs").unwrap(), "https://github.com/adefossez/demucs/blob/main/README.md#license");
        assert!(leida("adefossez/HTDemucs") && !leida("adefossez/HTDemucs-6s"), "cada modelo se lee por su lado");
        assert_eq!(aceptar_leida("adefossez/HTDemucs").unwrap().licencia, "unknown");
        assert!(marcar_leida("org/no-existe").is_err());
        revocar("adefossez/HTDemucs").unwrap();
        // Whisper y el embebedor del RAG: como todo modelo, sólo con la casilla (aunque los archivos
        // ya estén en la Mac); su aviso nombra al dueño de los pesos y a quien hizo la conversión
        for (r, dir) in [(crate::audio::stt_repo(), crate::audio::whisper_dir as fn() -> Option<PathBuf>), (crate::rag::embed_repo(), crate::rag::embedder_dir)] {
            assert!(dir().is_none(), "{r} sin aceptar");
            aceptar(&r).unwrap();
            assert_eq!(dir().is_some(), crate::hub::models_dir().join(&r).join("config.json").is_file(), "{r} aceptado");
            revocar(&r).unwrap();
            let g = ficha(&r).unwrap();
            assert!(!g.origen.is_empty() && aviso(g).contains(g.origen) && aviso(g).contains(g.repo), "{}", aviso(g));
        }
        // la voz: la copia de mlx-community en la caché de HF sirve SÓLO con la licencia aceptada
        // SAFETY: única prueba que toca estas variables (y las devuelve como estaban al final)
        let (voz_antes, hf_antes) = (std::env::var_os("GOTV_VOZ_DIR"), std::env::var_os("HF_HUB_CACHE"));
        unsafe {
            std::env::remove_var("GOTV_VOZ_DIR");
            std::env::set_var("HF_HUB_CACHE", d.join("hf"));
        }
        let snap = d.join("hf").join(VOZ_CONVERSION).join("snapshots/abc");
        std::fs::create_dir_all(snap.join("speech_tokenizer")).unwrap();
        std::fs::write(snap.join("config.json"), "{}").unwrap();
        std::fs::write(snap.join("speech_tokenizer/config.json"), "{}").unwrap();
        if !carpeta(ficha(VOZ_REPO).unwrap()).join("config.json").is_file() {
            assert!(voz_dir().is_err(), "sin aceptar la licencia no se usa");
            aceptar(VOZ_REPO).unwrap();
            assert_eq!(voz_dir().unwrap(), snap, "aceptada, sirve la copia de la caché");
            revocar(VOZ_REPO).unwrap();
        }
        // SAFETY: devuelve lo que esta prueba cambió
        unsafe {
            match voz_antes {
                Some(v) => std::env::set_var("GOTV_VOZ_DIR", v),
                None => std::env::remove_var("GOTV_VOZ_DIR"),
            }
            match hf_antes {
                Some(v) => std::env::set_var("HF_HUB_CACHE", v),
                None => std::env::remove_var("HF_HUB_CACHE"),
            }
        }
        LICENCIAS_DE_PRUEBA.with(|c| *c.borrow_mut() = None);
        let _ = std::fs::remove_dir_all(&d);
    }
}
