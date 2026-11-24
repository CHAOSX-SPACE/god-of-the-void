//! A3 · Llaves de los proveedores de IA en el **Llavero de macOS** (servicio
//! `space.chaosx.gotv`, cuenta = nombre del proveedor). Nunca en `config.toml`, nunca
//! en un registro, nunca de vuelta a la ventana: la GUI sólo sabe si HAY llave.
//! Respaldo para la terminal: `ANTHROPIC_API_KEY` / `OPENAI_API_KEY` / `GOTV_KEY_<NOMBRE>`.

const SERVICE: &str = "space.chaosx.gotv";

pub fn set(provider: &str, key: &str) -> anyhow::Result<()> {
    let key = key.trim();
    anyhow::ensure!(
        !key.is_empty(),
        "{}",
        crate::i18n::t("la llave está vacía", "the key is empty")
    );
    security_framework::passwords::set_generic_password(SERVICE, provider, key.as_bytes()).map_err(
        |e| {
            anyhow::anyhow!(
                "{}: {e}",
                crate::i18n::t(
                    "el Llavero no guardó la llave",
                    "the Keychain did not save the key"
                )
            )
        },
    )
}

pub fn delete(provider: &str) {
    let _ = security_framework::passwords::delete_generic_password(SERVICE, provider);
}

/// La llave del proveedor: Llavero primero, luego variables de entorno.
pub fn get(provider: &str) -> Option<String> {
    if let Ok(b) = security_framework::passwords::get_generic_password(SERVICE, provider) {
        if let Ok(s) = String::from_utf8(b) {
            if !s.trim().is_empty() {
                return Some(s);
            }
        }
    }
    let env = match provider {
        "anthropic" => "ANTHROPIC_API_KEY".to_string(),
        "openai" => "OPENAI_API_KEY".to_string(),
        other => format!("GOTV_KEY_{}", other.to_uppercase().replace(['-', '.'], "_")),
    };
    std::env::var(env).ok().filter(|s| !s.trim().is_empty())
}

pub fn has(provider: &str) -> bool {
    get(provider).is_some()
}
