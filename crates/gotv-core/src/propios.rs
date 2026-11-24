//! S3 · Los modelos PROPIOS del Portador (`proyectos/ia`) dentro de GOTV.
//!
//! Su exportación ya guarda los pesos con los nombres de Llama de Hugging Face
//! (`model.layers.N.self_attn.q_proj`…) y su RoPE rota mitades, igual que HF. Así que
//! no hace falta un motor aparte: se escribe una vez la ficha de Llama (config,
//! plantilla de chat IDÉNTICA a la del SFT, tokens especiales, parada en `<|fin|>`) y
//! `mlx_lm.server` lo sirve como a cualquier modelo — streaming, caché, todo.
//! Los pesos se ENLAZAN (hard link), no se copian.
//!
//! Son modelos de CHAT (contexto 512, sin herramientas): `gotv_chat_only` en su config
//! hace que GOTV los corra sin el prompt de agente, que ni cabría.

use anyhow::{anyhow, bail, Result};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

/// La plantilla de `src/tokenizador.py::aplicar_plantilla_chat`, en Jinja: <|bos|>, cada
/// turno como <|papel|>contenido<|fin|>, y <|asistente|> para generar.
pub const CHAT_TEMPLATE: &str = "{{ '<|bos|>' }}{% for m in messages %}{% if m['role'] == 'system' %}{{ '<|sistema|>' }}{% elif m['role'] == 'assistant' %}{{ '<|asistente|>' }}{% else %}{{ '<|usuario|>' }}{% endif %}{{ m['content'] | trim }}{{ '<|fin|>' }}{% endfor %}{% if add_generation_prompt %}{{ '<|asistente|>' }}{% endif %}";

pub fn dest(name: &str) -> PathBuf {
    crate::hub::models_dir().join("propios").join(name)
}

/// Importa una exportación de `proyectos/ia` (config.json propio + safetensors + tokenizer.json).
pub fn import(src: &Path, name: Option<&str>) -> Result<PathBuf> {
    let cfg: Value = serde_json::from_str(&std::fs::read_to_string(src.join("config.json"))?)?;
    let n = |k: &str| {
        cfg[k].as_u64().ok_or_else(|| {
            anyhow!(
                "{}",
                crate::tr!("config.json sin «{k}»", "config.json without «{k}»")
            )
        })
    };
    let weights = std::fs::read_dir(src)?
        .flatten()
        .map(|e| e.path())
        .find(|p| p.extension().is_some_and(|x| x == "safetensors"))
        .ok_or_else(|| {
            anyhow!(
                "{}",
                crate::i18n::t(
                    "no hay .safetensors (exporta con `make exportar`)",
                    "no .safetensors (export with `make exportar`)"
                )
            )
        })?;
    let tok_path = src.join("tokenizer.json");
    let tok: Value = serde_json::from_str(&std::fs::read_to_string(&tok_path)?)?;
    let special = |content: &str| -> Option<u64> {
        tok["added_tokens"]
            .as_array()?
            .iter()
            .find(|t| t["content"] == content)?["id"]
            .as_u64()
    };
    let (Some(bos), Some(fin)) = (special("<|bos|>"), special("<|fin|>")) else {
        bail!(
            "{}",
            crate::i18n::t(
                "el tokenizador no trae <|bos|> y <|fin|>: no es una exportación de proyectos/ia",
                "the tokenizer lacks <|bos|> and <|fin|>: not a proyectos/ia export"
            )
        );
    };
    let eos = special("<|eos|>");
    let heads = n("n_heads")?;
    let d = n("d_model")?;
    let name = name
        .map(String::from)
        .unwrap_or_else(|| cfg["nombre"].as_str().unwrap_or("propio").to_string());
    if !crate::hub::valid_repo(&format!("propios/{name}")) {
        bail!(
            "{}",
            crate::tr!("nombre inválido: «{name}»", "invalid name: «{name}»")
        );
    }
    let out = dest(&name);
    std::fs::create_dir_all(&out)?;
    let config = json!({
        "architectures": ["LlamaForCausalLM"],
        "model_type": "llama",
        "hidden_size": d,
        "intermediate_size": n("ffn_hidden")?,
        "num_hidden_layers": n("n_layers")?,
        "num_attention_heads": heads,
        "num_key_value_heads": cfg["n_kv_heads"].as_u64().unwrap_or(heads),
        "head_dim": d / heads,
        "max_position_embeddings": n("context_length")?,
        "rope_theta": cfg["rope_theta"].as_f64().unwrap_or(10000.0),
        "rms_norm_eps": cfg["norm_eps"].as_f64().unwrap_or(1e-5),
        "vocab_size": n("vocab_size")?,
        "tie_word_embeddings": cfg["tie_embeddings"].as_bool().unwrap_or(true),
        "hidden_act": "silu",
        "attention_bias": false,
        "mlp_bias": false,
        "bos_token_id": bos,
        "eos_token_id": [fin],
        "torch_dtype": "float16",
        "gotv_chat_only": true,
        "gotv_origen": src.display().to_string(),
    });
    std::fs::write(out.join("config.json"), serde_json::to_vec_pretty(&config)?)?;
    let mut eos_ids = vec![fin];
    eos_ids.extend(eos);
    std::fs::write(
        out.join("generation_config.json"),
        serde_json::to_vec_pretty(&json!({"bos_token_id": bos, "eos_token_id": eos_ids}))?,
    )?;
    std::fs::write(
        out.join("tokenizer_config.json"),
        serde_json::to_vec_pretty(&json!({
            "tokenizer_class": "PreTrainedTokenizerFast",
            "bos_token": "<|bos|>",
            "eos_token": "<|fin|>",
            "pad_token": "<|pad|>",
            "add_bos_token": false,
            "chat_template": CHAT_TEMPLATE,
        }))?,
    )?;
    std::fs::copy(&tok_path, out.join("tokenizer.json"))?;
    let w = out.join("model.safetensors");
    let _ = std::fs::remove_file(&w);
    if std::fs::hard_link(&weights, &w).is_err() {
        std::fs::copy(&weights, &w)?; // otro disco: se copia
    }
    Ok(out)
}

/// Sistema corto para un modelo de chat propio: el MISMO con el que se ajustó (en
/// español siempre: el modelo se entrenó así y no habla inglés).
// juez-idioma: datos (frase exacta del SFT)
pub const SISTEMA_CHAT: &str = "Eres un asistente amable que responde en español de forma breve.";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn importa_una_exportacion_y_la_marca_como_chat() {
        let d = crate::CarpetaPrueba::sin_crear("gotv-propio");
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("config.json"), r#"{"nombre":"sft","vocab_size":16384,"d_model":512,"n_layers":12,"n_heads":8,"ffn_hidden":1408,"context_length":512,"rope_theta":10000.0,"norm_eps":1e-5,"tie_embeddings":true}"#).unwrap();
        std::fs::write(d.join("pesos.safetensors"), b"x").unwrap();
        std::fs::write(d.join("tokenizer.json"), r#"{"added_tokens":[{"id":0,"content":"<|bos|>"},{"id":1,"content":"<|eos|>"},{"id":6,"content":"<|fin|>"}]}"#).unwrap();
        let name = format!("prueba-{}", std::process::id());
        let out = import(&d, Some(&name)).unwrap();
        let c: Value =
            serde_json::from_str(&std::fs::read_to_string(out.join("config.json")).unwrap())
                .unwrap();
        assert_eq!(
            (
                c["hidden_size"].as_u64(),
                c["head_dim"].as_u64(),
                c["eos_token_id"][0].as_u64()
            ),
            (Some(512), Some(64), Some(6))
        );
        assert_eq!(c["gotv_chat_only"], true);
        assert!(
            out.join("model.safetensors").exists() && out.join("tokenizer_config.json").exists()
        );
        let card = crate::models::ModelCard::read(&out, "t", &name).unwrap();
        assert!(card.runnable && card.chat_only && card.max_ctx == 512);
        std::fs::remove_dir_all(&out).ok();
    }
}
