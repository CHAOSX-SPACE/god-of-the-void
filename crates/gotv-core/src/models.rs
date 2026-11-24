//! Gestor de modelos (E8): descubre, mide y decide si un modelo CABE en ESTA Mac.
//!
//! Ficha leída del `config.json` de cada modelo (nunca supuesta): capas, cabezas
//! KV, dimensión, techo de contexto, expertos (MoE), capas de atención real
//! (híbridos), atención latente (MLA, tipo DeepSeek), cuantización, pesos en disco.
//! Con eso: memoria = pesos + caché KV(contexto) + margen ≤ presupuesto leído.

use crate::hardware::{Hardware, GIB, MIB};
use serde_json::Value;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct ModelCard {
    pub name: String,
    pub path: PathBuf,
    pub source: String,
    pub arch: String,
    pub layers: u64,
    /// Capas con caché KV (en híbridos, sólo las de atención completa).
    pub attn_layers: u64,
    pub kv_heads: u64,
    pub head_dim: u64,
    /// Atención latente (MLA): bytes de caché por capa y token = (rank + rope)·2.
    pub mla_dim: Option<u64>,
    pub max_ctx: u64,
    pub experts: Option<u64>,
    pub bits: Option<u64>,
    pub vision: bool,
    pub weights: u64,
    /// ¿Lo puede servir `mlx_lm`? (los nuestros de proyectos/ia todavía no: ola S3)
    pub runnable: bool,
    /// Muestreo que recomienda el fabricante (`generation_config.json`).
    pub sampling: Option<gotv_engine::Sampling>,
    /// Tamaño del vocabulario: un borrador (P11) sólo sirve si es IDÉNTICO.
    pub vocab: Option<u64>,
    /// Oído nativo (S2): el config trae `audio_config`.
    pub audio: bool,
    /// S3: modelo de CHAT (propio, contexto corto, sin herramientas): GOTV lo corre sin
    /// el prompt de agente.
    pub chat_only: bool,
}

/// Lee el muestreo recomendado. `None` si no hay archivo o el fabricante pide voraz.
pub fn read_sampling(dir: &Path) -> Option<gotv_engine::Sampling> {
    let g: Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("generation_config.json")).ok()?)
            .ok()?;
    if g.get("do_sample").and_then(Value::as_bool) == Some(false) {
        return None;
    }
    let f = |k: &str| g.get(k).and_then(Value::as_f64).map(|v| v as f32);
    Some(gotv_engine::Sampling {
        temperature: f("temperature")?,
        top_p: f("top_p").filter(|v| *v > 0.0 && *v < 1.0),
        top_k: g
            .get("top_k")
            .and_then(Value::as_u64)
            .filter(|v| *v > 0)
            .map(|v| v as u32),
        min_p: f("min_p").filter(|v| *v > 0.0),
    })
}

impl ModelCard {
    /// Lee la ficha desde la carpeta del modelo.
    pub fn read(dir: &Path, source: &str, name: &str) -> Option<ModelCard> {
        let c: Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join("config.json")).ok()?).ok()?;
        let mut card = Self::from_config(&c, weights_size(dir), dir, source, name)?;
        card.sampling = if card.runnable {
            read_sampling(dir)
        } else {
            None
        };
        Some(card)
    }

    /// La ficha desde un `config.json` y el peso de los pesos — sirve también para
    /// un modelo REMOTO que aún no se bajó (P12: catálogo filtrado por lo que cabe).
    pub fn from_config(
        c: &Value,
        weights: u64,
        dir: &Path,
        source: &str,
        name: &str,
    ) -> Option<ModelCard> {
        let t = c.get("text_config").cloned().unwrap_or(Value::Null);
        let g = |k: &str| {
            c.get(k)
                .filter(|v| !v.is_null())
                .or_else(|| t.get(k).filter(|v| !v.is_null()))
        };
        let n = |k: &str| g(k).and_then(|v| v.as_u64());
        if weights == 0 {
            return None;
        }

        // Arquitectura propia (proyectos/ia): llaves n_layers / d_model / n_heads.
        if c.get("model_type").is_none() && t.get("model_type").is_none() {
            let layers = n("n_layers")?;
            let heads = n("n_heads").unwrap_or(1).max(1);
            return Some(ModelCard {
                name: name.into(),
                path: dir.to_path_buf(),
                source: source.into(),
                arch: "propia".into(),
                layers,
                attn_layers: layers,
                kv_heads: heads,
                head_dim: n("d_model").unwrap_or(0) / heads,
                mla_dim: None,
                max_ctx: n("context_length").unwrap_or(512),
                experts: None,
                bits: None,
                vision: false,
                weights,
                runnable: false,
                sampling: None,
                vocab: n("vocab_size"),
                audio: false,
                chat_only: true,
            });
        }

        let arch = g("model_type")
            .and_then(|v| v.as_str())
            .unwrap_or("?")
            .to_string();
        let layers = n("num_hidden_layers").unwrap_or(0);
        let heads = n("num_attention_heads").unwrap_or(1).max(1);
        let kv_heads = n("num_key_value_heads").unwrap_or(heads);
        let head_dim = n("head_dim").unwrap_or(n("hidden_size").unwrap_or(0) / heads);
        let attn_layers = match g("layer_types").and_then(|v| v.as_array()) {
            Some(lt) => lt
                .iter()
                .filter(|x| x.as_str().map(|s| s.contains("full")).unwrap_or(false))
                .count() as u64,
            None => match n("full_attention_interval") {
                Some(i) if i > 0 => layers / i,
                _ => layers,
            },
        };
        let mla_dim = n("kv_lora_rank").map(|r| r + n("qk_rope_head_dim").unwrap_or(0));
        let experts = n("num_experts")
            .or(n("num_local_experts"))
            .or(n("n_routed_experts"));
        let bits = g("quantization")
            .and_then(|q| q.get("bits"))
            .and_then(|b| b.as_u64());
        // Sólo un modelo de LENGUAJE es servible por mlx_lm (medido: timesfm,
        // de series de tiempo, se colaba como "denso ✅").
        let archs: Vec<String> = g("architectures")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        let is_lm = archs.iter().any(|a| {
            a.contains("CausalLM") || a.contains("ConditionalGeneration") || a.contains("LMHead")
        });
        // Medido con el catálogo (P12): Qwen3-ASR (voz→texto) y Qwen3-Embedding pasaban
        // como modelos de chat. No son agentes: ni el nombre ni la arquitectura engañan.
        let not_chat = ["asr", "embedding", "reranker", "tts", "whisper", "speech"]
            .iter()
            .any(|w| {
                name.to_lowercase().contains(w)
                    || archs.iter().any(|a| a.to_lowercase().contains(w))
            });
        let runnable = !not_chat && (is_lm || (archs.is_empty() && bits.is_some()));
        Some(ModelCard {
            name: name.into(),
            path: dir.to_path_buf(),
            source: source.into(),
            arch,
            layers,
            attn_layers,
            kv_heads,
            head_dim,
            mla_dim,
            max_ctx: n("max_position_embeddings").unwrap_or(4096),
            experts,
            bits,
            vision: c.get("vision_config").is_some(),
            weights,
            runnable,
            sampling: None,
            vocab: n("vocab_size"),
            audio: c.get("audio_config").is_some(),
            chat_only: c
                .get("gotv_chat_only")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
        })
    }

    /// Bytes de caché KV por token (fp16).
    pub fn kv_per_token(&self) -> u64 {
        match self.mla_dim {
            Some(d) => self.attn_layers * d * 2,
            None => 2 * self.attn_layers * self.kv_heads * self.head_dim * 2,
        }
    }

    /// Memoria total a un contexto dado.
    pub fn need(&self, ctx: u64) -> u64 {
        self.weights + overhead(self.weights) + self.kv_per_token() * ctx
    }

    pub fn kind(&self) -> String {
        let mut k = vec![];
        if self.arch == "propia" {
            k.push(crate::i18n::t("propio", "own").to_string());
        } else if self.experts.is_some() {
            k.push(format!("MoE·{}", self.experts.unwrap_or(0)));
        } else if self.attn_layers < self.layers {
            k.push(crate::i18n::t("híbrido", "hybrid").into());
        } else {
            k.push(crate::i18n::t("denso", "dense").into());
        }
        if self.mla_dim.is_some() {
            k.push("MLA".into());
        }
        if self.vision {
            k.push(crate::i18n::t("visión", "vision").into());
        }
        k.join("+")
    }
}

/// Activaciones, búferes de prellenado y el intérprete: 1,5 GiB + 5 % de los pesos.
pub fn overhead(weights: u64) -> u64 {
    3 * GIB / 2 + weights / 20
}

fn weights_size(dir: &Path) -> u64 {
    std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .filter(|e| e.file_name().to_string_lossy().ends_with(".safetensors"))
                .map(|e| std::fs::metadata(e.path()).map(|m| m.len()).unwrap_or(0))
                .sum()
        })
        .unwrap_or(0)
}

// ── Chequeo de memoria ──────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct Fit {
    pub ok: bool,
    pub need: u64,
    pub limit: u64,
    /// Contexto máximo que cabe (0 si ni los pesos caben).
    pub max_ctx: u64,
    pub why: String,
}

/// ¿Cabe `card` a `ctx` tokens en `hw`, con `available` bytes libres ahora?
pub fn fit(card: &ModelCard, hw: &Hardware, available: Option<u64>, ctx: u64) -> Fit {
    let limit = (hw.gpu_budget.min(available.unwrap_or(u64::MAX)) as f64 * 0.95) as u64;
    let base = card.weights + overhead(card.weights);
    let kvt = card.kv_per_token().max(1);
    let max_ctx = if limit > base {
        ((limit - base) / kvt).min(card.max_ctx)
    } else {
        0
    };
    let need = base + kvt * ctx;
    let gb = |b: u64| b as f64 / GIB as f64;
    if need <= limit {
        return Fit {
            ok: true,
            need,
            limit,
            max_ctx,
            why: String::new(),
        };
    }
    // ¿Qué límite manda: la RAM libre de AHORA o el presupuesto de GPU?
    // (Medido: con el límite de GPU ya en 56 GB y 40,7 GB libres, la versión
    // anterior recetaba BAJAR el límite a 43 GB. Nunca más.)
    let gpu_lim = (hw.gpu_budget as f64 * 0.95) as u64;
    let free_binds = available
        .map(|a| ((a as f64 * 0.95) as u64) < gpu_lim)
        .unwrap_or(false);
    let why = if base > limit {
        if free_binds && base <= gpu_lim {
            crate::tr!(
                "necesita {:.1} GB y ahora sólo se pueden usar {:.1} GB ({:.1} GB libres menos \
                 5 % de margen para el sistema): otras aplicaciones ocupan la memoria. Cierra \
                 programas y reintenta (con la Mac libre sí cabe: presupuesto de GPU {:.1} GB)",
                "needs {:.1} GB and only {:.1} GB can be used now ({:.1} GB free minus a 5 % \
                 margin for the system): other apps are using the memory. Close programs and \
                 retry (with the Mac free it does fit: GPU budget {:.1} GB)",
                gb(need),
                gb(limit),
                gb(available.unwrap_or(0)),
                gb(gpu_lim)
            )
        } else if need < (hw.memory as f64 * 0.94) as u64 {
            // Manda la GPU: subir el límite (siempre a MÁS que el actual).
            let wanted_mb = (need as f64 / 0.95 / MIB as f64).ceil() as u64;
            crate::tr!(
                "los pesos ({:.1} GB) + margen no caben en el presupuesto de GPU ({:.1} GB). \
                 Cabría subiendo el límite: `sudo sysctl iogpu.wired_limit_mb={wanted_mb}` \
                 (lo corres tú){}",
                "the weights ({:.1} GB) + margin do not fit the GPU budget ({:.1} GB). \
                 It would fit by raising the limit: `sudo sysctl iogpu.wired_limit_mb={wanted_mb}` \
                 (you run it){}",
                gb(card.weights),
                gb(gpu_lim),
                if free_binds {
                    crate::i18n::t(" y liberando memoria", " and freeing memory")
                } else {
                    ""
                }
            )
        } else {
            crate::tr!(
                "necesita {:.1} GB y esta Mac sólo puede dar {:.1} GB: hace falta una \
                 cuantización menor o más memoria",
                "needs {:.1} GB and this Mac can only give {:.1} GB: it needs a smaller \
                 quantization or more memory",
                gb(need),
                gb(limit)
            )
        }
    } else {
        crate::tr!(
            "a {ctx} tokens necesita {:.1} GB de {:.1} GB; caben hasta ~{max_ctx} tokens{}",
            "at {ctx} tokens it needs {:.1} GB of {:.1} GB; up to ~{max_ctx} tokens fit{}",
            gb(need),
            gb(limit),
            if free_binds {
                crate::i18n::t(
                    " con la memoria libre de ahora",
                    " with the memory free right now",
                )
            } else {
                ""
            }
        )
    };
    Fit {
        ok: false,
        need,
        limit,
        max_ctx,
        why,
    }
}

/// Contexto seguro cuando lo limita la MEMORIA: 90 % del máximo que cabe, para
/// dejar aire a los picos de activación del prellenado y al sistema. Si lo
/// limita el techo del propio modelo, se usa ese techo entero.
pub fn safe_ctx(card: &ModelCard, f: &Fit) -> u64 {
    if f.max_ctx >= card.max_ctx {
        card.max_ctx
    } else {
        f.max_ctx * 9 / 10
    }
}

/// Contexto automático: lo más que cabe con margen, sin pasar el techo del modelo. Sin más tope: «auto»
/// cortaba en 128k aunque el modelo diera 262k y cupiera (35B-A3B; regla del Portador 2026-11-13: el
/// contexto lo mandan el modelo o el usuario, nunca un tope escondido).
pub fn auto_context(card: &ModelCard, hw: &Hardware, available: Option<u64>) -> u64 {
    safe_ctx(card, &fit(card, hw, available, 0))
}

/// Elige un modelo cuando el usuario no fijó ninguno: primero sus preferidos que
/// quepan, luego el denso más grande que quepa, luego cualquiera que quepa.
pub fn choose<'a>(
    cards: &'a [ModelCard],
    hw: &Hardware,
    available: Option<u64>,
    prefer: &[String],
    min_ctx: u64,
) -> Option<&'a ModelCard> {
    let fits = |c: &&ModelCard| c.runnable && fit(c, hw, available, min_ctx.min(c.max_ctx)).ok;
    for p in prefer {
        let p = p.to_lowercase();
        if let Some(c) = cards
            .iter()
            .filter(fits)
            .find(|c| c.name.to_lowercase().contains(&p))
        {
            return Some(c);
        }
    }
    cards
        .iter()
        .filter(fits)
        .filter(|c| c.experts.is_none() && c.attn_layers == c.layers)
        .max_by_key(|c| c.weights)
        .or_else(|| cards.iter().filter(fits).max_by_key(|c| c.weights))
}

// ── Descubrimiento ──────────────────────────────────────────────────────────

/// Carpetas donde buscar: LM Studio, caché de Hugging Face y las que diga la config.
pub fn discover(extra_dirs: &[String]) -> Vec<ModelCard> {
    let home = PathBuf::from(std::env::var("HOME").unwrap_or_default());
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    let mut push = |card: Option<ModelCard>, out: &mut Vec<ModelCard>| {
        if let Some(c) = card {
            let key = c.path.canonicalize().unwrap_or(c.path.clone());
            if seen.insert(key) {
                out.push(c);
            }
        }
    };

    // LM Studio: <org>/<nombre>/
    let lms = home.join(".lmstudio/models");
    for org in read_dirs(&lms) {
        for m in read_dirs(&org) {
            let name = format!("{}/{}", file_name(&org), file_name(&m));
            push(ModelCard::read(&m, "LM Studio", &name), &mut out);
        }
    }
    // Bajados por `gotv models pull` (P12): ~/.cache/gotv/models/<org>/<nombre>/
    for org in read_dirs(&crate::hub::models_dir()) {
        for m in read_dirs(&org) {
            let name = format!("{}/{}", file_name(&org), file_name(&m));
            push(ModelCard::read(&m, "GOTV", &name), &mut out);
        }
    }
    // Hugging Face: models--org--nombre/snapshots/<hash>/
    let hf = std::env::var("HF_HOME")
        .map(|h| PathBuf::from(h).join("hub"))
        .unwrap_or_else(|_| home.join(".cache/huggingface/hub"));
    for repo in read_dirs(&hf) {
        let rn = file_name(&repo);
        let Some(id) = rn.strip_prefix("models--") else {
            continue;
        };
        let name = id.replacen("--", "/", 1);
        for snap in read_dirs(&repo.join("snapshots")) {
            push(ModelCard::read(&snap, "Hugging Face", &name), &mut out);
        }
    }
    // Carpetas extra (p. ej. nuestros modelos de proyectos/ia), hasta 3 niveles.
    for d in extra_dirs {
        let d = PathBuf::from(shellexpand(d));
        scan_extra(&d, 3, &mut |c| push(Some(c), &mut out));
    }
    out.sort_by_key(|c| c.weights);
    out
}

fn scan_extra(dir: &Path, depth: u32, f: &mut dyn FnMut(ModelCard)) {
    if depth == 0 {
        return;
    }
    if dir.join("config.json").exists() {
        if let Some(c) = ModelCard::read(dir, "propio", &file_name(dir)) {
            f(c);
            return;
        }
    }
    for sub in read_dirs(dir) {
        scan_extra(&sub, depth - 1, f);
    }
}

fn read_dirs(p: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(p)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.path())
                .filter(|p| p.is_dir())
                .collect()
        })
        .unwrap_or_default()
}

/// Nombre legible de una ruta de modelo: en la caché de HF
/// (`models--org--nombre/snapshots/<hash>`) devuelve `org/nombre`, no el hash.
pub fn display_name(p: &Path) -> String {
    if let Some(snaps) = p.parent() {
        if snaps.file_name().map(|s| s == "snapshots").unwrap_or(false) {
            if let Some(id) = snaps
                .parent()
                .map(file_name)
                .and_then(|r| r.strip_prefix("models--").map(String::from))
            {
                return id.replacen("--", "/", 1);
            }
        }
    }
    file_name(p)
}

fn file_name(p: &Path) -> String {
    p.file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default()
}

fn shellexpand(p: &str) -> String {
    match p.strip_prefix("~/") {
        Some(rest) => format!("{}/{rest}", std::env::var("HOME").unwrap_or_default()),
        None => p.to_string(),
    }
}

// ── Pruebas: de 0,6b a 671b, en Macs de 16 GB a 512 GB ─────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Ficha sintética con números reales de cada familia.
    fn card(
        name: &str,
        w_gb: f64,
        layers: u64,
        attn: u64,
        kvh: u64,
        hd: u64,
        ctx: u64,
    ) -> ModelCard {
        ModelCard {
            name: name.into(),
            path: PathBuf::from(name),
            source: "test".into(),
            arch: "test".into(),
            layers,
            attn_layers: attn,
            kv_heads: kvh,
            head_dim: hd,
            mla_dim: None,
            max_ctx: ctx,
            experts: None,
            bits: Some(4),
            vision: false,
            weights: (w_gb * GIB as f64) as u64,
            runnable: true,
            sampling: None,
            vocab: None,
            audio: false,
            chat_only: false,
        }
    }

    #[test]
    fn asr_y_embeddings_no_son_modelos_de_chat() {
        let cfg = |arch: &str| {
            serde_json::json!({"model_type": "qwen3", "architectures": [arch], "num_hidden_layers": 28,
            "num_attention_heads": 16, "num_key_value_heads": 8, "head_dim": 128, "max_position_embeddings": 32768})
        };
        let p = Path::new("x");
        assert!(
            ModelCard::from_config(
                &cfg("Qwen3ForCausalLM"),
                GIB,
                p,
                "t",
                "mlx-community/Qwen3-0.6B-4bit"
            )
            .unwrap()
            .runnable
        );
        assert!(
            !ModelCard::from_config(
                &cfg("Qwen3ASRForConditionalGeneration"),
                GIB,
                p,
                "t",
                "mlx-community/Qwen3-ASR-0.6B-8bit"
            )
            .unwrap()
            .runnable
        );
        assert!(
            !ModelCard::from_config(
                &cfg("Qwen3ForCausalLM"),
                GIB,
                p,
                "t",
                "mlx-community/Qwen3-Embedding-0.6B-4bit-DWQ"
            )
            .unwrap()
            .runnable
        );
    }

    #[test]
    fn lee_el_muestreo_del_fabricante() {
        let d = crate::CarpetaPrueba::sin_crear("gotv-gen");
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(
            d.join("generation_config.json"),
            r#"{"do_sample": true, "temperature": 0.6, "top_p": 0.95, "top_k": 20}"#,
        )
        .unwrap();
        let s = read_sampling(&d).unwrap();
        assert_eq!(
            (s.temperature, s.top_p, s.top_k, s.min_p),
            (0.6, Some(0.95), Some(20), None)
        );
        // top_p 1.0 y top_k 0 son «sin filtro»: no se envían
        std::fs::write(
            d.join("generation_config.json"),
            r#"{"temperature": 1.0, "top_p": 1.0, "top_k": 0}"#,
        )
        .unwrap();
        let s = read_sampling(&d).unwrap();
        assert_eq!((s.top_p, s.top_k), (None, None));
        // el fabricante pide voraz → no se impone nada
        std::fs::write(
            d.join("generation_config.json"),
            r#"{"do_sample": false, "temperature": 0.6}"#,
        )
        .unwrap();
        assert!(read_sampling(&d).is_none());
        std::fs::remove_dir_all(&d).ok();
    }

    fn q06() -> ModelCard {
        card("Qwen3-0.6B", 0.6, 28, 28, 8, 128, 40960)
    }
    fn q8() -> ModelCard {
        card("Qwen3-8B", 4.6, 36, 36, 8, 128, 40960)
    }
    fn q32() -> ModelCard {
        card("Qwen3-32B", 18.4, 64, 64, 8, 128, 40960)
    }
    fn hybrid27() -> ModelCard {
        card("Qwen3.8-27B", 19.4, 64, 16, 4, 256, 262144)
    }
    fn h70() -> ModelCard {
        card("Hermes-4-70B", 39.7, 80, 80, 8, 128, 131072)
    }
    fn q235() -> ModelCard {
        let mut c = card("Qwen3-235B-A22B", 132.0, 94, 94, 4, 128, 131072);
        c.experts = Some(128);
        c
    }
    fn ds671() -> ModelCard {
        let mut c = card("DeepSeek-V3-671B", 377.0, 61, 61, 128, 128, 163840);
        c.mla_dim = Some(512 + 64);
        c.experts = Some(256);
        c
    }

    fn mac16() -> Hardware {
        Hardware::simulated("M4 16GB", 16, 16.0 * 2.0 / 3.0)
    }
    fn mac64() -> Hardware {
        Hardware::simulated("M1 Max 64GB", 64, 56.0)
    }
    fn mac192() -> Hardware {
        Hardware::simulated("M2 Ultra 192GB", 192, 144.0)
    }
    fn mac512() -> Hardware {
        Hardware::simulated("Ultra 512GB", 512, 384.0)
    }

    #[test]
    fn kv_por_token_segun_arquitectura() {
        assert_eq!(q32().kv_per_token(), 256 * 1024); // denso: 2·64·8·128·2
        assert_eq!(hybrid27().kv_per_token(), 64 * 1024); // híbrido: sólo 16 capas
        assert_eq!(h70().kv_per_token(), 320 * 1024);
        assert_eq!(ds671().kv_per_token(), 61 * 576 * 2); // MLA latente: ~70 KB
    }

    #[test]
    fn mac_de_16gb() {
        let hw = mac16();
        assert!(fit(&q06(), &hw, None, 16384).ok);
        assert!(fit(&q8(), &hw, None, 8192).ok);
        let f = fit(&q32(), &hw, None, 8192);
        assert!(
            !f.ok && f.max_ctx == 0,
            "los pesos del 32b no caben en 16 GB"
        );
        assert!(f.why.contains("cuantización menor") || f.why.contains("wired_limit"));
    }

    #[test]
    fn mac_de_64gb_como_la_del_portador() {
        let hw = mac64();
        assert!(
            fit(&q32(), &hw, None, 40960).ok,
            "32b a contexto completo cabe"
        );
        assert!(
            fit(&hybrid27(), &hw, None, 131072).ok,
            "el híbrido aguanta 128k"
        );
        // 70b: cabe, pero con contexto recortado. Con TODA la memoria libre:
        // (53,2 − 43,2 GB) / 320 KB ≈ 32,8k tokens. (Con 44,9 GB libres reales
        // `gotv models` dio 7.670: el libre manda.)
        let f = fit(&h70(), &hw, None, 131072);
        assert!(
            !f.ok && f.max_ctx > 16384 && f.max_ctx < 40000,
            "70b: max_ctx={}",
            f.max_ctx
        );
        assert!(fit(&h70(), &hw, Some(44 * GIB), 16384).max_ctx < 10000);
        assert!(!fit(&q235(), &hw, None, 4096).ok);
    }

    #[test]
    fn mac_de_192gb() {
        let hw = mac192();
        assert!(fit(&h70(), &hw, None, 131072).ok, "70b a 128k completos");
        // 235b-4bit: 132 GB + margen ≈ 140 GB > 136,8 GB del límite de fábrica.
        // No cabe por defecto; debe proponer subir el límite, y con él entra.
        let f = fit(&q235(), &hw, None, 4096);
        assert!(!f.ok && f.why.contains("wired_limit_mb="), "{}", f.why);
        let mut alto = mac192();
        alto.gpu_budget = 176 * GIB;
        assert!(fit(&q235(), &alto, None, 32768).ok);
        assert!(!fit(&ds671(), &hw, None, 4096).ok);
    }

    #[test]
    fn ultra_de_512gb() {
        let hw = mac512();
        assert!(fit(&q235(), &hw, None, 131072).ok, "235b a 128k");
        // 671b con el límite de fábrica (≈75 %) no entra, pero sí subiéndolo
        let f = fit(&ds671(), &hw, None, 32768);
        assert!(!f.ok);
        assert!(
            f.why.contains("wired_limit_mb="),
            "debe proponer subir el límite: {}",
            f.why
        );
        let mut alto = mac512();
        alto.gpu_budget = 480 * GIB;
        assert!(
            fit(&ds671(), &alto, None, 32768).ok,
            "con el límite subido, 671b entra"
        );
    }

    #[test]
    fn nombre_de_ruta_hf_es_el_repo() {
        let p =
            PathBuf::from("/x/hub/models--mlx-community--gemma-4-e4b-it-4bit/snapshots/475b90/");
        assert_eq!(display_name(&p), "mlx-community/gemma-4-e4b-it-4bit");
        assert_eq!(
            display_name(&PathBuf::from("/m/org/Qwen3-32B")),
            "Qwen3-32B"
        );
    }

    #[test]
    fn falta_ram_libre_no_receta_bajar_el_limite() {
        // Lo medido en la matriz: límite de GPU ya en 56 GB, 40,7 GB libres, 70b.
        let mut hw = mac64();
        hw.wired_limit_mb = 57344;
        let f = fit(&h70(), &hw, Some((40.7 * GIB as f64) as u64), 4096);
        assert!(!f.ok);
        assert!(f.why.contains("libres"), "{}", f.why);
        assert!(
            !f.why.contains("wired_limit"),
            "no debe recetar el sysctl: {}",
            f.why
        );
        // Con la Mac libre, el mismo modelo sí cabe.
        assert!(fit(&h70(), &hw, None, 4096).ok);
    }

    #[test]
    fn memoria_ocupada_por_otro_programa_cuenta() {
        let hw = mac64();
        assert!(fit(&q32(), &hw, None, 16384).ok);
        assert!(
            !fit(&q32(), &hw, Some(15 * GIB), 16384).ok,
            "con 15 GB libres no debe cargar"
        );
    }

    #[test]
    fn contexto_automatico_se_adapta_a_la_maquina() {
        assert_eq!(
            auto_context(&q32(), &mac64(), None),
            40960,
            "techo del modelo"
        );
        let small = auto_context(&h70(), &mac64(), None);
        let big = auto_context(&h70(), &mac192(), None);
        assert!(small < 40000 && big == 131072, "64GB→{small}, 192GB→{big}");
        // sin tope escondido: un modelo de 262k que cabe usa sus 262k (antes «auto» cortaba en 128k)
        let largo = auto_context(&hybrid27(), &mac64(), None);
        assert!(largo > 131_072, "un modelo de 262k que cabe no se corta en 128k: {largo}");
        // una IA de UN MILLÓN (híbrida, KV chica): donde cabe usa su millón entero; en 64 GB, lo que
        // permita la memoria (con margen) y nunca más que su techo
        let millon = card("Qwen-1M-hibrido", 19.4, 64, 16, 4, 256, 1_010_000);
        assert_eq!(auto_context(&millon, &mac512(), None), 1_010_000, "techo del modelo: un millón");
        let en64 = auto_context(&millon, &mac64(), None);
        assert!(en64 > 262_144 && en64 <= 1_010_000, "64 GB: {en64}");
    }

    #[test]
    fn eleccion_respeta_preferencia_y_maquina() {
        let cards = vec![q06(), q8(), q32(), h70(), q235()];
        let pref = vec!["qwen3-32b".to_string()];
        assert_eq!(
            choose(&cards, &mac64(), None, &pref, 16384).unwrap().name,
            "Qwen3-32B"
        );
        // sin preferencia: el denso más grande que quepa a 16k. Con toda la RAM
        // libre, en 64 GB es el 70b; con 44 GB libres (lo real hoy) baja al 32b.
        assert_eq!(
            choose(&cards, &mac64(), None, &[], 16384).unwrap().name,
            "Hermes-4-70B"
        );
        assert_eq!(
            choose(&cards, &mac64(), Some(44 * GIB), &[], 16384)
                .unwrap()
                .name,
            "Qwen3-32B"
        );
        assert_eq!(
            choose(&cards, &mac192(), None, &[], 16384).unwrap().name,
            "Hermes-4-70B"
        );
        assert_eq!(
            choose(&cards, &mac16(), None, &[], 8192).unwrap().name,
            "Qwen3-8B"
        );
    }

    #[test]
    fn lee_fichas_reales_de_esta_mac() {
        // Si existen en el disco, la lectura debe coincidir con lo medido a mano.
        let base = PathBuf::from(std::env::var("HOME").unwrap()).join(".lmstudio/models");
        if let Some(c) = ModelCard::read(
            &base.join("mlx-community/Qwen3-32B-4bit-DWQ"),
            "LM Studio",
            "q32",
        ) {
            assert_eq!(
                (c.layers, c.kv_heads, c.head_dim, c.max_ctx),
                (64, 8, 128, 40960)
            );
            assert_eq!(c.kind(), "denso");
        }
        if let Some(c) = ModelCard::read(
            &base.join("mlx-community/Qwen3.8-27B-Uncensored-OptiQ-4bit"),
            "LM Studio",
            "h",
        ) {
            assert_eq!(c.attn_layers, 16);
            assert_eq!(c.kind(), "híbrido");
        }
        if let Some(c) = ModelCard::read(
            &base.join("vanch007/Huihui-Qwen3.6-35B-A3B-abliterated-mlx-4bit"),
            "LM Studio",
            "m",
        ) {
            assert_eq!(c.experts, Some(256));
            assert!(c.vision, "este MoE trae visión nativa");
        }
    }
}
