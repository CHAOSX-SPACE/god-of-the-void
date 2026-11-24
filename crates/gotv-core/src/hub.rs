//! Catálogo y descarga desde Hugging Face (P12), en Rust, sin `huggingface_hub`.
//!
//! - `search`: modelos MLX (`mlx-community` por defecto) más bajados que coinciden;
//! - `inspect`: lee su `config.json` y el peso real de sus `.safetensors` SIN bajar
//!   nada pesado, y arma la ficha → GOTV dice si cabe en ESTA Mac (o en un perfil
//!   simulado: `--mem 512`) antes de gastar un byte;
//! - `pull`: baja a `~/.cache/gotv/models/<org>/<nombre>/`, reanuda lo cortado
//!   (`.part` + `Range`), comprueba tamaños y el espacio en disco ANTES de empezar.
//!
//! Defensas: el repo debe ser `org/nombre` con caracteres seguros, y ninguna ruta
//! del árbol remoto puede salir de su carpeta (`..`, absolutas).

use crate::models::ModelCard;
use anyhow::{anyhow, bail, Result};
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

const HF: &str = "https://huggingface.co";

pub fn models_dir() -> PathBuf {
    gotv_engine::cache_dir().join("models")
}

/// `org/nombre` con letras, números, `.`, `_`, `-` — nada más.
pub fn valid_repo(repo: &str) -> bool {
    let mut parts = repo.split('/');
    let ok = |p: Option<&str>| {
        p.is_some_and(|s| {
            !s.is_empty()
                && s != "."
                && s != ".."
                && s.chars()
                    .all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
        })
    };
    ok(parts.next()) && ok(parts.next()) && parts.next().is_none()
}

/// Una ruta del árbol remoto es segura si es relativa y no sube.
pub fn safe_rel(p: &str) -> bool {
    !p.is_empty()
        && Path::new(p)
            .components()
            .all(|c| matches!(c, Component::Normal(_)))
}

fn client() -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .user_agent("GOD-OF-THE-VOID/0.1 (local coding agent)")
        .connect_timeout(Duration::from_secs(20))
        .build()?)
}

/// GET de JSON con 3 intentos (medido: un «Connection reset by peer» pasajero
/// tumbaba el comando entero).
pub(crate) async fn get_json(url: reqwest::Url) -> Result<serde_json::Value> {
    get_json_con(url, None).await
}

/// GET de un texto (p. ej. el `LICENSE` de un repositorio oficial fuera de Hugging Face).
pub(crate) async fn get_texto(url: reqwest::Url) -> Result<String> {
    Ok(client()?.get(url).timeout(Duration::from_secs(30)).send().await?.error_for_status()?.text().await?)
}

/// `get_json` con el token de Hugging Face (repos de acceso restringido, M0).
pub(crate) async fn get_json_con(url: reqwest::Url, token: Option<&str>) -> Result<serde_json::Value> {
    let http = client()?;
    let mut last = None;
    for attempt in 0..3u64 {
        if attempt > 0 {
            tokio::time::sleep(Duration::from_secs(2 * attempt)).await;
        }
        let mut req = http.get(url.clone());
        if let Some(t) = token {
            req = req.bearer_auth(t);
        }
        match req
            .timeout(Duration::from_secs(30))
            .send()
            .await
            .and_then(|r| r.error_for_status())
        {
            Ok(r) => match r.json().await {
                Ok(v) => return Ok(v),
                Err(e) => last = Some(anyhow!(e)),
            },
            // un 404 no mejora reintentando
            Err(e) if e.status().is_some_and(|s| s.is_client_error()) => return Err(anyhow!(e)),
            Err(e) => last = Some(anyhow!(e)),
        }
    }
    Err(last.unwrap_or_else(|| anyhow!("?")))
}

#[derive(Debug, Clone)]
pub struct Found {
    pub repo: String,
    pub downloads: u64,
}

/// Busca en Hugging Face. `org` vacío = `mlx-community` (formato que sirve mlx_lm).
pub async fn search(query: &str, org: &str, limit: usize) -> Result<Vec<Found>> {
    let mut url = reqwest::Url::parse(&format!("{HF}/api/models"))?;
    url.query_pairs_mut()
        .append_pair("author", if org.is_empty() { "mlx-community" } else { org })
        .append_pair("search", query)
        .append_pair("sort", "downloads")
        .append_pair("direction", "-1")
        .append_pair("limit", &limit.to_string());
    let v = get_json(url).await?;
    Ok(v.as_array()
        .map(|a| {
            a.iter()
                .filter_map(|m| {
                    Some(Found {
                        repo: m["id"].as_str()?.to_string(),
                        downloads: m["downloads"].as_u64().unwrap_or(0),
                    })
                })
                .filter(|f| valid_repo(&f.repo))
                .collect()
        })
        .unwrap_or_default())
}

/// Archivos del repo (ruta, bytes).
pub async fn files(repo: &str) -> Result<Vec<(String, u64)>> {
    files_con(repo, None).await
}

/// Archivos del repo con el token de Hugging Face (los restringidos no listan sin él).
pub async fn files_con(repo: &str, token: Option<&str>) -> Result<Vec<(String, u64)>> {
    if !valid_repo(repo) {
        bail!(
            "{}",
            crate::tr!(
                "repo inválido: «{repo}» (forma org/nombre)",
                "invalid repo: «{repo}» (org/name form)"
            )
        );
    }
    let v = get_json_con(
        reqwest::Url::parse(&format!("{HF}/api/models/{repo}/tree/main?recursive=true"))?,
        token,
    )
    .await?;
    let mut out = Vec::new();
    for f in v.as_array().into_iter().flatten() {
        if f["type"].as_str() != Some("file") {
            continue;
        }
        let path = f["path"].as_str().unwrap_or("").to_string();
        if !safe_rel(&path) {
            bail!(
                "{}",
                crate::tr!(
                    "ruta peligrosa en el repo: «{path}»",
                    "dangerous path in the repo: «{path}»"
                )
            );
        }
        // LFS trae el tamaño real en lfs.size; el resto en size
        let size = f["lfs"]["size"]
            .as_u64()
            .or(f["size"].as_u64())
            .unwrap_or(0);
        out.push((path, size));
    }
    Ok(out)
}

/// La ficha de un modelo REMOTO: su config + el peso de sus safetensors.
pub async fn inspect(repo: &str) -> Result<(ModelCard, Vec<(String, u64)>)> {
    let fs = files(repo).await?;
    let weights: u64 = fs
        .iter()
        .filter(|(p, _)| p.ends_with(".safetensors"))
        .map(|(_, s)| s)
        .sum();
    let cfg = get_json(reqwest::Url::parse(&format!(
        "{HF}/{repo}/resolve/main/config.json"
    ))?)
    .await?;
    let card = ModelCard::from_config(&cfg, weights, Path::new(repo), "Hugging Face", repo)
        .ok_or_else(|| {
            anyhow!(
                "{}",
                crate::tr!(
                    "{repo}: sin pesos .safetensors o config ilegible",
                    "{repo}: no .safetensors weights or unreadable config"
                )
            )
        })?;
    Ok((card, fs))
}

/// Baja (o reanuda) un archivo hacia su `.part` y comprueba el tamaño.
#[allow(clippy::too_many_arguments)]
async fn fetch_into(
    http: &reqwest::Client,
    repo: &str,
    token: Option<&str>,
    path: &str,
    part: &Path,
    size: u64,
    done: &mut u64,
    total: u64,
    progress: &(dyn Fn(u64, u64) + Sync),
) -> Result<()> {
    let start = std::fs::metadata(part).map(|m| m.len()).unwrap_or(0);
    let mut req = http.get(format!("{HF}/{repo}/resolve/main/{path}"));
    if let Some(t) = token {
        req = req.bearer_auth(t);
    }
    if start > 0 {
        req = req.header(reqwest::header::RANGE, format!("bytes={start}-"));
    }
    let mut resp = req.send().await?.error_for_status()?;
    // si el servidor ignoró el Range, se empieza de cero
    let append = start > 0 && resp.status() == reqwest::StatusCode::PARTIAL_CONTENT;
    if start > 0 && !append {
        *done = done.saturating_sub(start);
    }
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(append)
        .write(true)
        .truncate(!append)
        .open(part)?;
    use std::io::Write;
    while let Some(chunk) = resp.chunk().await? {
        f.write_all(&chunk)?;
        *done += chunk.len() as u64;
        progress(*done, total);
    }
    f.flush()?;
    let got = std::fs::metadata(part)?.len();
    if size > 0 && got != size {
        bail!(
            "{}",
            crate::tr!(
                "{path}: llegaron {got} bytes de {size}; vuelve a correr pull para reanudar",
                "{path}: got {got} bytes of {size}; run pull again to resume"
            )
        );
    }
    Ok(())
}

/// Bytes libres en el disco de `p`.
pub fn free_disk(p: &Path) -> Option<u64> {
    let probe = p.ancestors().find(|a| a.exists())?;
    let c = std::ffi::CString::new(probe.to_string_lossy().as_bytes()).ok()?;
    let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
    // SAFETY: statvfs llena la estructura que le pasamos; la cadena vive hasta aquí.
    (unsafe { libc::statvfs(c.as_ptr(), &mut st) } == 0)
        .then(|| st.f_bavail as u64 * st.f_frsize as u64)
}

/// Cómo bajar un repo: con token (restringidos), sólo ciertos archivos y si se exige formato MLX.
#[derive(Default)]
pub struct Descarga<'a> {
    /// token de Hugging Face (del Llavero) para los repos de acceso restringido
    pub token: Option<&'a str>,
    /// sólo los archivos cuya ruta empieza por alguno (vacío = el repo entero)
    pub prefijos: &'a [&'a str],
    /// exigir `config.json` + pesos MLX (los modelos de lenguaje; los generadores no)
    pub exigir_mlx: bool,
}

/// Baja el repo entero. `progress(hecho, total)` en bytes. Reanuda lo cortado.
pub async fn pull(repo: &str, progress: &(dyn Fn(u64, u64) + Sync)) -> Result<PathBuf> {
    pull_con(repo, &Descarga { exigir_mlx: true, ..Default::default() }, progress).await
}

/// Un modelo de lenguaje MLX trae pesos (safetensors; npz en los de voz) Y un `config.json` en la raíz.
fn parece_mlx(fs: &[(String, u64)]) -> bool {
    fs.iter().any(|(p, _)| p.ends_with(".safetensors") || p.ends_with(".npz")) && fs.iter().any(|(p, _)| p == "config.json")
}

/// `pull` con opciones (M0: generadores, repos restringidos, archivos elegidos).
pub async fn pull_con(repo: &str, op: &Descarga<'_>, progress: &(dyn Fn(u64, u64) + Sync)) -> Result<PathBuf> {
    let mut fs = files_con(repo, op.token).await?;
    if !op.prefijos.is_empty() {
        fs.retain(|(p, _)| op.prefijos.iter().any(|x| p.starts_with(x)));
        if fs.is_empty() {
            bail!("{}", crate::tr!("{repo}: ningún archivo coincide con lo pedido", "{repo}: no file matches the request"));
        }
    }
    // pesos en safetensors o, para modelos de voz (mlx-audio los carga), npz. Medido (M4, FLUX.2
    // klein): sin paréntesis, `exigir && !pesos || !config` exigía un config.json en la RAÍZ también
    // a los generadores en formato diffusers (cada pieza trae el suyo en su carpeta)
    if op.exigir_mlx && !parece_mlx(&fs) {
        bail!(
            "{}",
            crate::tr!(
                "{repo} no parece un modelo MLX (faltan config.json o .safetensors)",
                "{repo} does not look like an MLX model (config.json or .safetensors missing)"
            )
        );
    }
    let dest = models_dir().join(repo);
    std::fs::create_dir_all(&dest)?;
    let total: u64 = fs.iter().map(|(_, s)| s).sum();
    let have: u64 = fs
        .iter()
        .map(|(p, s)| {
            std::fs::metadata(dest.join(p))
                .map(|m| m.len().min(*s))
                .unwrap_or(0)
                + std::fs::metadata(dest.join(format!("{p}.part")))
                    .map(|m| m.len())
                    .unwrap_or(0)
        })
        .sum();
    let missing = total.saturating_sub(have);
    if let Some(free) = free_disk(&dest) {
        if missing + crate::hardware::GIB > free {
            bail!(
                "{}",
                crate::tr!(
                    "no hay espacio: faltan {:.1} GB y el disco tiene {:.1} GB libres",
                    "not enough space: {:.1} GB missing and the disk has {:.1} GB free",
                    missing as f64 / 1e9,
                    free as f64 / 1e9
                )
            );
        }
    }
    let http = client()?;
    let mut done = have;
    for (path, size) in &fs {
        let target = dest.join(path);
        if std::fs::metadata(&target)
            .map(|m| m.len() == *size)
            .unwrap_or(false)
        {
            continue; // ya está entero
        }
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let part = PathBuf::from(format!("{}.part", target.display()));
        // hasta 3 intentos: cada uno REANUDA desde lo que ya llegó al `.part`
        let mut attempt = 0u64;
        loop {
            match fetch_into(&http, repo, op.token, path, &part, *size, &mut done, total, progress).await {
                Ok(()) => break,
                Err(_) if attempt < 2 => {
                    attempt += 1;
                    tokio::time::sleep(Duration::from_secs(2 * attempt)).await;
                }
                Err(e) => return Err(e),
            }
        }
        std::fs::rename(&part, &target)?;
    }
    progress(total, total);
    Ok(dest)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parece_mlx_exige_pesos_y_config_en_la_raiz() {
        let f = |v: &[&str]| v.iter().map(|p| (p.to_string(), 1u64)).collect::<Vec<_>>();
        assert!(parece_mlx(&f(&["config.json", "model.safetensors"])));
        assert!(parece_mlx(&f(&["config.json", "weights.npz"])));
        assert!(!parece_mlx(&f(&["model.safetensors"])));
        assert!(!parece_mlx(&f(&["config.json", "README.md"])));
        // diffusers (FLUX.2 klein): cada pieza con su config en su carpeta → no es un LLM MLX
        assert!(!parece_mlx(&f(&["model_index.json", "transformer/config.json", "transformer/diffusion_pytorch_model.safetensors"])));
    }

    #[test]
    fn repos_y_rutas_seguras() {
        assert!(valid_repo("mlx-community/Qwen3-32B-4bit-DWQ"));
        assert!(valid_repo("org/model.v2_x"));
        for bad in [
            "../etc",
            "org/..",
            "org",
            "org/a/b",
            "/abs/x",
            "org/a b",
            "org/$(rm)",
            "",
        ] {
            assert!(!valid_repo(bad), "{bad}");
        }
        assert!(safe_rel("model.safetensors") && safe_rel("sub/dir/x.json"));
        for bad in ["../x", "/etc/passwd", "a/../../b", ""] {
            assert!(!safe_rel(bad), "{bad}");
        }
    }

    #[test]
    fn ficha_remota_desde_config() {
        // config real de Qwen3-32B (recortada) + peso de sus safetensors
        let cfg = serde_json::json!({"model_type": "qwen3", "architectures": ["Qwen3ForCausalLM"], "num_hidden_layers": 64,
            "num_attention_heads": 64, "num_key_value_heads": 8, "head_dim": 128, "max_position_embeddings": 40960,
            "vocab_size": 151936, "quantization": {"bits": 4}});
        let c = ModelCard::from_config(
            &cfg,
            18 * crate::hardware::GIB,
            Path::new("mlx-community/Qwen3-32B-4bit"),
            "Hugging Face",
            "mlx-community/Qwen3-32B-4bit",
        )
        .unwrap();
        assert!(c.runnable);
        assert_eq!(c.kv_per_token(), 256 * 1024);
        assert_eq!(c.vocab, Some(151936));
    }

    #[test]
    fn hay_disco_libre_medible() {
        assert!(free_disk(&std::env::temp_dir()).unwrap_or(0) > 0);
    }
}
