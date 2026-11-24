//! A3 · IAs por API con los créditos del usuario: catálogo de proveedores, cómo se
//! nombra un modelo de nube (`proveedor:modelo`) y su arranque.

use crate::config::{Config, Provider};
use anyhow::{anyhow, Result};
use gotv_engine::{AnthropicEngine, AnyEngine, CloudModel, Engine, OpenAiEngine};

/// Contexto por defecto en la nube. Claude admite 1M, pero GOTV reenvía la ventana en
/// cada paso del agente: 200k compacta a tiempo y cuida los créditos. El usuario puede
/// subirlo en Configuración → Contexto.
pub const CLOUD_DEFAULT_CTX: usize = 200_000;

/// Los de serie (Anthropic y OpenAI) + los que añadió el usuario (que pueden
/// redefinir uno de serie con el mismo nombre).
pub fn providers(cfg: &Config) -> Vec<Provider> {
    let mut v = vec![
        Provider {
            name: "anthropic".into(),
            kind: "anthropic".into(),
            base_url: String::new(),
            vision: true,
        },
        Provider {
            name: "openai".into(),
            kind: "openai".into(),
            base_url: "https://api.openai.com/v1".into(),
            vision: true,
        },
    ];
    for p in &cfg.providers {
        v.retain(|x| x.name != p.name);
        v.push(p.clone());
    }
    v
}

/// Atajos para añadir un proveedor sin saberse su URL.
pub const PRESETS: &[(&str, &str, &str)] = &[
    ("openrouter", "OpenRouter", "https://openrouter.ai/api/v1"),
    ("groq", "Groq", "https://api.groq.com/openai/v1"),
    ("deepseek", "DeepSeek", "https://api.deepseek.com/v1"),
    ("mistral", "Mistral", "https://api.mistral.ai/v1"),
    ("together", "Together", "https://api.together.xyz/v1"),
    ("ollama", "Ollama (LAN)", "http://192.168.1.10:11434/v1"),
    ("lmstudio", "LM Studio (LAN)", "http://192.168.1.10:1234/v1"),
];

/// `anthropic:claude-opus-5-5` → (proveedor, `claude-opus-5-5`). Una ruta local o un
/// `org/modelo` sin prefijo de proveedor conocido → `None`.
pub fn split(model: &str, cfg: &Config) -> Option<(Provider, String)> {
    let (p, m) = model.split_once(':')?;
    if m.is_empty() || p.contains('/') {
        return None;
    }
    providers(cfg)
        .into_iter()
        .find(|x| x.name == p)
        .map(|x| (x, m.to_string()))
}

pub fn is_cloud(model: &str, cfg: &Config) -> bool {
    split(model, cfg).is_some()
}

fn key_for(p: &Provider) -> Result<String> {
    match crate::llaves::get(&p.name) {
        Some(k) => Ok(k),
        // un Ollama/LM Studio de la red no lleva llave
        None if p.kind == "openai" && !p.base_url.starts_with("https://") => Ok(String::new()),
        None => Err(anyhow!(
            "{}",
            crate::tr!(
                "no hay llave para «{}»: agrégala en Configuración → Proveedores (se guarda en el Llavero de macOS)",
                "no key for «{}»: add it in Settings → Providers (it is stored in the macOS Keychain)",
                p.name
            )
        )),
    }
}

/// Modelos que ofrece un proveedor (prueba también la llave).
pub async fn models_of(p: &Provider) -> Result<Vec<CloudModel>> {
    let key = key_for(p)?;
    if p.kind == "anthropic" {
        AnthropicEngine::new("", key, None).models().await
    } else {
        OpenAiEngine::new(&p.name, &p.base_url, key, "", p.vision)
            .models()
            .await
    }
}

/// Arranca un modelo de nube y fija el contexto. Devuelve el motor, su nombre y notas.
pub async fn launch(cfg: &mut Config) -> Result<(AnyEngine, String, Vec<String>)> {
    let (p, model) = split(&cfg.model, cfg).ok_or_else(|| {
        anyhow!(
            "{}",
            crate::tr!(
                "{} no es un modelo de nube",
                "{} is not a cloud model",
                cfg.model
            )
        )
    })?;
    let key = key_for(&p)?;
    // la ventana del modelo, si el proveedor la publica
    let info = models_of(&p)
        .await
        .ok()
        .and_then(|ms| ms.into_iter().find(|m| m.id == model));
    let model_ctx = info.as_ref().and_then(|m| m.ctx).map(|c| c as usize);
    let wanted = crate::config::preset_to_budget(&cfg.context_preset).unwrap_or(CLOUD_DEFAULT_CTX);
    cfg.context_budget = model_ctx.map_or(wanted, |c| wanted.min(c));
    let effort = cfg
        .effort
        .clone()
        .filter(|e| !e.is_empty())
        .or_else(|| Some("high".into()));
    let mut engine = if p.kind == "anthropic" {
        AnyEngine::Anthropic(AnthropicEngine::new(&model, key, effort.clone()))
    } else {
        AnyEngine::OpenAi(OpenAiEngine::new(
            &p.name,
            &p.base_url,
            key,
            &model,
            p.vision,
        ))
    };
    engine.start().await?;
    let mut notes = vec![crate::tr!(
        "{} · por API ({}) · contexto {} tok{}",
        "{} · via API ({}) · context {} tok{}",
        model,
        p.name,
        cfg.context_budget,
        model_ctx
            .map(|c| crate::tr!(" de {c}", " of {c}"))
            .unwrap_or_default()
    )];
    if p.kind == "anthropic" {
        // la reserva sólo si el modelo la tiene (antes se anunciaba también con Haiku, que no la tiene)
        let reserva = if gotv_engine::cloud::server_fallback(&model) { crate::i18n::t(" · reserva del servidor si Claude declina", " · server fallback if Claude declines") } else { "" };
        notes.push(crate::tr!(
            "esfuerzo {} · caché de prompt automática{}",
            "effort {} · automatic prompt caching{}",
            effort.unwrap_or_default(),
            reserva
        ));
    }
    Ok((engine, format!("{}:{model}", p.name), notes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nombre_de_modelo_de_nube() {
        let mut cfg = Config::default();
        cfg.providers.push(Provider {
            name: "openrouter".into(),
            kind: "openai".into(),
            base_url: "https://openrouter.ai/api/v1".into(),
            vision: false,
        });
        let (p, m) = split("anthropic:claude-opus-5-5", &cfg).unwrap();
        assert_eq!(
            (p.kind.as_str(), m.as_str()),
            ("anthropic", "claude-opus-5-5")
        );
        let (p, m) = split("openrouter:meta-llama/llama-4-maverick", &cfg).unwrap();
        assert_eq!(
            (p.base_url.as_str(), m.as_str()),
            (
                "https://openrouter.ai/api/v1",
                "meta-llama/llama-4-maverick"
            )
        );
        // lo local no es nube
        assert!(split("/Users/x/.lmstudio/models/mlx-community/Qwen3-32B", &cfg).is_none());
        assert!(split("mlx-community/Qwen3-32B-4bit", &cfg).is_none());
        assert!(split("desconocido:algo", &cfg).is_none());
    }
}
