//! Biblioteca de modelos: el catálogo MLX de Hugging Face visto desde ESTA Mac.
//!
//! Una sola llamada a la API trae, por modelo, sus parámetros reales por tipo de
//! dato (`safetensors`), sus tags (bits, visión) y sus descargas. Con eso, sin bajar
//! un byte, GOTV calcula:
//!
//! - el **peso** en disco y memoria: los `U32` que reporta la API ya son parámetros
//!   lógicos (medido: Qwen3-ASR-0.6B-8bit trae 0,6 G en U32, no 2,4 G); cada uno
//!   pesa `bits + 0,5` (escala y sesgo por grupo de 64) → Qwen3-32B-4bit da 18,45 GB
//!   contra 18,43 GB reales;
//! - la **velocidad** al generar: generar un token lee todos los pesos ACTIVOS una
//!   vez, así que manda el ancho de banda de memoria del chip. Calibrado en un
//!   M1 Max (400 GB/s) con medidas propias: 0.6B → 81-88 tok/s, Qwen3-32B denso
//!   → 13,7, Huihui 35B-A3B (MoE) → 37;
//! - si **cabe**: pesos + `models::overhead` + una ventana de 8k contra el
//!   presupuesto de GPU real (o el de una Mac simulada, para elegir antes de comprar).

use crate::hardware::{Hardware, GIB};
use anyhow::Result;
use serde::Serialize;
use std::collections::HashSet;

/// Ancho de banda de memoria (GB/s) del chip y si sale de la tabla de Apple
/// (`true`) o se estimó por su gama (`false`).
pub fn chip_bandwidth(device: &str, gpu_cores: Option<u32>) -> (f64, bool) {
    let d = device.to_lowercase();
    let gen: u32 = d
        .split_whitespace()
        .find_map(|w| w.strip_prefix('m').and_then(|n| n.parse().ok()))
        .unwrap_or(0);
    let tier = if d.contains("ultra") {
        "ultra"
    } else if d.contains("max") {
        "max"
    } else if d.contains("pro") {
        "pro"
    } else {
        ""
    };
    let cores = gpu_cores.unwrap_or(0);
    let known = match (gen, tier) {
        (1, "") => Some(68.25),
        (1, "pro") | (2, "pro") => Some(200.0),
        (1, "max") | (2, "max") => Some(400.0),
        (1, "ultra") | (2, "ultra") => Some(800.0),
        (2, "") | (3, "") => Some(100.0),
        (3, "pro") => Some(150.0),
        (3, "max") => Some(if cores >= 40 { 400.0 } else { 300.0 }),
        (3, "ultra") => Some(819.0),
        (4, "") => Some(120.0),
        (4, "pro") => Some(273.0),
        (4, "max") => Some(if cores >= 40 { 546.0 } else { 410.0 }),
        (5, "") => Some(153.0),
        _ => None,
    };
    if let Some(bw) = known {
        return (bw, true);
    }
    // chip que la tabla aún no conoce: la gama manda (Pro ≈ 2×, Max ≈ 4×, Ultra ≈ 8× la base)
    let base = if gen >= 5 { 153.0 } else { 100.0 };
    let mult = match tier {
        "pro" => 2.0,
        "max" => 4.0,
        "ultra" => 8.0,
        _ => 1.0,
    };
    (base * mult, false)
}

/// Núcleos de GPU de esta Mac (distingue un M3/M4 Max de 30-32 de uno de 40).
pub fn gpu_cores() -> Option<u32> {
    let o = std::process::Command::new("ioreg")
        .args(["-rc", "AGXAccelerator", "-d1"])
        .output()
        .ok()?;
    let s = String::from_utf8_lossy(&o.stdout);
    let i = s.find("\"gpu-core-count\" = ")?;
    s[i + 19..]
        .split(|c: char| !c.is_ascii_digit())
        .next()?
        .parse()
        .ok()
}

/// Bits por peso, de los tags del repo o de su nombre.
pub fn parse_bits(tags: &[String], name: &str) -> Option<f32> {
    for t in tags {
        if let Some(n) = t.strip_suffix("-bit").and_then(|n| n.parse::<f32>().ok()) {
            return Some(n);
        }
    }
    let n = name.to_lowercase();
    if n.contains("mxfp4") || n.contains("nvfp4") {
        return Some(4.0);
    }
    if n.contains("bf16") || n.contains("fp16") || n.contains("-f16") {
        return Some(16.0);
    }
    for b in [2, 3, 4, 5, 6, 8] {
        if n.contains(&format!("{b}bit")) || n.contains(&format!("{b}-bit")) {
            return Some(b as f32);
        }
    }
    None
}

/// Parámetros ACTIVOS por token de un MoE, en miles de millones: del nombre
/// (`…-35B-A3B-…`) o de los MoE conocidos que no lo dicen.
pub fn active_params(name: &str) -> Option<f64> {
    let n = name.to_lowercase();
    for tok in n.split(['-', '_', '/']) {
        if let Some(num) = tok.strip_prefix('a').and_then(|r| r.strip_suffix('b')) {
            if let Ok(v) = num.parse::<f64>() {
                if v > 0.0 && v < 200.0 {
                    return Some(v);
                }
            }
        }
    }
    const KNOWN: &[(&str, f64)] = &[
        ("gpt-oss-120b", 5.1),
        ("gpt-oss-20b", 3.6),
        ("mixtral-8x22b", 39.0),
        ("mixtral-8x7b", 12.9),
        ("glm-4.5-air", 12.0),
        ("glm-4.6", 32.0),
        ("glm-4.5", 32.0),
        ("deepseek-v3", 37.0),
        ("deepseek-r1", 37.0),
        ("kimi-k2", 32.0),
        ("llama-4-scout", 17.0),
        ("llama-4-maverick", 17.0),
    ];
    KNOWN.iter().find(|(k, _)| n.contains(k)).map(|(_, v)| *v)
}

/// Parámetros totales del nombre (`27B`, `0.6B`, `8x7B`), en miles de millones.
pub fn params_from_name(name: &str) -> Option<f64> {
    let n = name.to_lowercase();
    for tok in n.split(['-', '_', '/']) {
        let Some(num) = tok.strip_suffix('b') else {
            continue;
        };
        if let Some((e, s)) = num.split_once('x') {
            if let (Ok(e), Ok(s)) = (e.parse::<f64>(), s.parse::<f64>()) {
                return Some(e * s);
            }
        }
        if let Ok(v) = num.parse::<f64>() {
            if v > 0.0 {
                return Some(v);
            }
        }
    }
    None
}

/// Bytes de los pesos desde el conteo por tipo de dato de la API.
pub fn weight_bytes(params: &serde_json::Map<String, serde_json::Value>, bits: f32) -> u64 {
    let per = |dtype: &str| -> f64 {
        match dtype {
            "U32" | "I32" => (bits as f64 + 0.5) / 8.0, // cuantizado: parámetro lógico
            "F32" => 4.0,
            "U8" | "I8" | "F8_E4M3" | "F8_E5M2" => 1.0,
            _ => 2.0, // BF16, F16
        }
    };
    params
        .iter()
        .map(|(k, v)| v.as_f64().unwrap_or(0.0) * per(k))
        .sum::<f64>() as u64
}

/// tok/s estimados al generar. `active` = bytes leídos por token.
pub fn estimate_tok_s(bandwidth_gbs: f64, active: u64, moe: bool) -> f64 {
    // eficiencia medida de MLX ≈ 68 % del ancho de banda + ~8 ms fijos por token;
    // el MoE paga además el ruteo y la lectura dispersa de expertos (×2, medido)
    let ms = 8.0 + active as f64 / 1e9 / (0.68 * bandwidth_gbs) * 1000.0;
    1000.0 / if moe { ms * 2.0 } else { ms }
}

/// Corrección MEDIDA por arquitectura (el tag `model_type` del repo). El denso híbrido
/// `qwen3_5` (atención lineal + torre de visión) va MUCHO más lento de lo que su peso
/// dice: Qwen3.8-27B midió 6,7 tok/s (falla #2448) y 11 (matriz E8) en el M1 Max contra
/// 15 estimados → ×0,55. Su MoE (`qwen3_5_moe`) cuadra sin corrección (37 vs 35).
pub fn arch_factor(tags: &[String]) -> f64 {
    if tags.iter().any(|t| t == "qwen3_5") {
        0.55
    } else {
        1.0
    }
}

/// La etiqueta de velocidad: muy-rapido · rapido · medio · lento · muy-lento.
pub fn speed_tag(tok_s: f64) -> &'static str {
    match tok_s {
        t if t >= 50.0 => "muy-rapido",
        t if t >= 25.0 => "rapido",
        t if t >= 12.0 => "medio",
        t if t >= 6.0 => "lento",
        _ => "muy-lento",
    }
}

/// Una Mac contra la que se mide la biblioteca (la real o una simulada).
#[derive(Serialize, Clone, Debug)]
pub struct MacProfile {
    pub id: String,
    pub label: String,
    pub device: String,
    pub memory_gb: u64,
    pub budget: u64,
    pub bandwidth: f64,
    /// el ancho de banda sale de la tabla de Apple (y no de la gama)
    pub exact: bool,
    pub this_mac: bool,
}

impl MacProfile {
    pub fn this_mac(hw: &Hardware) -> Self {
        let (bandwidth, exact) = chip_bandwidth(&hw.device, gpu_cores());
        let gb = (hw.memory as f64 / GIB as f64).round() as u64;
        Self {
            id: "this".into(),
            label: crate::tr!(
                "Esta Mac · {} · {} GB",
                "This Mac · {} · {} GB",
                hw.device.trim_start_matches("Apple "),
                gb
            ),
            device: hw.device.clone(),
            memory_gb: gb,
            budget: hw.gpu_budget,
            bandwidth,
            exact,
            this_mac: true,
        }
    }

    /// Una Mac simulada: `"M4 Pro|48"`. El presupuesto de GPU sigue la regla de
    /// macOS (75 % desde 36 GB, 67 % por debajo), igual que `gotv models search --mem`.
    pub fn simulated(id: &str) -> Option<Self> {
        let (chip, gb) = id.split_once('|')?;
        let gb: u64 = gb.parse().ok().filter(|g| (8..=1024).contains(g))?;
        let device = format!("Apple {chip}");
        let cores = if chip.contains("Max") { Some(40) } else { None };
        let (bandwidth, exact) = chip_bandwidth(&device, cores);
        let frac = if gb >= 36 { 0.75 } else { 0.67 };
        Some(Self {
            id: id.into(),
            label: format!("{chip} · {gb} GB"),
            device,
            memory_gb: gb,
            budget: (gb as f64 * frac * GIB as f64) as u64,
            bandwidth,
            exact,
            this_mac: false,
        })
    }
}

/// Las Macs típicas para comparar (de un Air de 8 GB a una Ultra de 512 GB).
pub const PRESETS: &[&str] = &[
    "M1|8",
    "M1|16",
    "M2|8",
    "M2|16",
    "M2|24",
    "M3|16",
    "M3|24",
    "M4|16",
    "M4|32",
    "M5|16",
    "M5|32",
    "M1 Pro|16",
    "M1 Pro|32",
    "M2 Pro|32",
    "M3 Pro|18",
    "M3 Pro|36",
    "M4 Pro|24",
    "M4 Pro|48",
    "M1 Max|32",
    "M1 Max|64",
    "M2 Max|64",
    "M2 Max|96",
    "M3 Max|48",
    "M3 Max|128",
    "M4 Max|64",
    "M4 Max|128",
    "M1 Ultra|128",
    "M2 Ultra|192",
    "M3 Ultra|256",
    "M3 Ultra|512",
];

/// Un modelo de la biblioteca, ya medido contra una Mac.
#[derive(Serialize, Clone, Debug)]
pub struct Entry {
    pub repo: String,
    pub downloads: u64,
    pub likes: u64,
    pub updated: String,
    /// parámetros totales y activos (miles de millones)
    pub params: f64,
    pub active: Option<f64>,
    pub bits: Option<f32>,
    pub bytes: u64,
    pub vision: bool,
    pub tok_s: f64,
    pub speed: &'static str,
    /// memoria que pide con 8k de ventana
    pub need: u64,
    pub fits: bool,
    /// cabe, pero deja poco aire (> 80 % del presupuesto)
    pub tight: bool,
    pub installed: bool,
}

/// Mide un modelo de la API contra una Mac. `None` si no es un modelo de chat
/// o no hay forma de saber su tamaño.
pub fn entry_from_api(
    m: &serde_json::Value,
    mac: &MacProfile,
    installed: &HashSet<String>,
) -> Option<Entry> {
    let repo = m["id"].as_str()?.to_string();
    if !crate::hub::valid_repo(&repo) {
        return None;
    }
    let pipe = m["pipeline_tag"].as_str().unwrap_or("");
    let tags: Vec<String> = m["tags"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|t| t.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    let chat = matches!(
        pipe,
        "text-generation" | "image-text-to-text" | "any-to-any"
    ) || (pipe.is_empty() && tags.iter().any(|t| t == "conversational"));
    if !chat {
        return None;
    }
    let bits = parse_bits(&tags, &repo);
    let st = &m["safetensors"];
    let (params, mut bytes) = match st["parameters"].as_object() {
        Some(p) => {
            let b = bits.unwrap_or(if p.contains_key("U32") { 4.0 } else { 16.0 });
            (
                st["total"].as_f64().unwrap_or(0.0) / 1e9,
                weight_bytes(p, b),
            )
        }
        None => {
            let p = params_from_name(&repo)?;
            (
                p,
                (p * 1e9 * (bits.unwrap_or(16.0) as f64 + 0.5) / 8.0) as u64,
            )
        }
    };
    // el peso REAL (suma de los .safetensors) manda sobre la estimación: medido 2026-11-10,
    // `Qwen3.5-4B-OptiQ-4bit` (precisión mixta) se estimaba en 2,4 GB y pesa 4,02 GB
    if let Some(real) = m["gotv_bytes"].as_u64().filter(|b| *b > 0) {
        bytes = real;
    }
    if bytes == 0 || params <= 0.0 {
        return None;
    }
    let active = active_params(&repo).filter(|a| *a < params);
    let active_bytes = match active {
        Some(a) => (bytes as f64 * a / params) as u64,
        None => bytes,
    };
    let tok_s = estimate_tok_s(mac.bandwidth, active_bytes, active.is_some()) * arch_factor(&tags);
    // 8k de ventana: ~1 GB en la mayoría; el detalle real lo da `hub::inspect` + `fit`
    let need = bytes + crate::models::overhead(bytes) + GIB;
    let limit = (mac.budget as f64 * 0.95) as u64;
    Some(Entry {
        downloads: m["downloads"].as_u64().unwrap_or(0),
        likes: m["likes"].as_u64().unwrap_or(0),
        updated: m["lastModified"]
            .as_str()
            .unwrap_or("")
            .chars()
            .take(10)
            .collect(),
        params,
        active,
        bits,
        bytes,
        vision: pipe == "image-text-to-text"
            || tags
                .iter()
                .any(|t| t == "image-text-to-text" || t == "vision"),
        tok_s,
        speed: speed_tag(tok_s),
        need,
        fits: need <= limit,
        tight: need <= limit && need as f64 > limit as f64 * 0.8,
        installed: installed.contains(&repo.to_lowercase()),
        repo,
    })
}

/// Orden de la búsqueda en Hugging Face.
pub fn sort_key(sort: &str) -> &'static str {
    match sort {
        "trending" => "trendingScore",
        "recent" => "lastModified",
        "likes" => "likes",
        _ => "downloads",
    }
}

/// Busca en Hugging Face y mide cada modelo contra `mac`. `org` vacío = todo lo MLX.
pub async fn search(
    query: &str,
    org: &str,
    sort: &str,
    limit: usize,
    mac: &MacProfile,
    installed: &HashSet<String>,
) -> Result<Vec<Entry>> {
    let mut url = reqwest::Url::parse("https://huggingface.co/api/models")?;
    {
        let mut q = url.query_pairs_mut();
        q.append_pair("filter", "mlx");
        if !org.is_empty() {
            q.append_pair("author", org);
        }
        if !query.trim().is_empty() {
            q.append_pair("search", query.trim());
        }
        q.append_pair("sort", sort_key(sort))
            .append_pair("direction", "-1")
            .append_pair("limit", &limit.clamp(1, 200).to_string());
        for e in [
            "safetensors",
            "downloads",
            "likes",
            "pipeline_tag",
            "tags",
            "lastModified",
        ] {
            q.append_pair("expand[]", e);
        }
    }
    let mut v = crate::hub::get_json(url).await?;
    // el peso real de cada resultado, en paralelo (8 s de tope; si falla, queda la estimación)
    if let Some(lista) = v.as_array_mut() {
        let reales = futures_util::future::join_all(lista.iter().map(|m| {
            let repo = m["id"].as_str().unwrap_or("").to_string();
            async move {
                let pesa = tokio::time::timeout(std::time::Duration::from_secs(8), crate::hub::files(&repo)).await.ok()?.ok()?;
                Some(pesa.iter().filter(|(p, _)| p.ends_with(".safetensors")).map(|(_, b)| b).sum::<u64>())
            }
        }))
        .await;
        for (m, real) in lista.iter_mut().zip(reales) {
            if let Some(b) = real.filter(|b| *b > 0) {
                m["gotv_bytes"] = serde_json::json!(b);
            }
        }
    }
    Ok(v.as_array()
        .into_iter()
        .flatten()
        .filter_map(|m| entry_from_api(m, mac, installed))
        .collect())
}

/// Los repos ya instalados (`org/nombre` en minúsculas), de lo que la app descubre.
pub fn installed_set(extra_dirs: &[String]) -> HashSet<String> {
    crate::models::discover(extra_dirs)
        .into_iter()
        .map(|c| c.name.to_lowercase())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn el_ancho_de_banda_sale_del_chip() {
        assert_eq!(chip_bandwidth("Apple M1 Max", Some(32)), (400.0, true));
        assert_eq!(chip_bandwidth("Apple M4 Max", Some(32)), (410.0, true));
        assert_eq!(chip_bandwidth("Apple M4 Max", Some(40)), (546.0, true));
        assert_eq!(chip_bandwidth("Apple M3 Pro", None), (150.0, true));
        assert_eq!(chip_bandwidth("Apple M2", None), (100.0, true));
        // chip que la tabla no conoce: estimado por gama, y lo dice
        let (bw, exact) = chip_bandwidth("Apple M7 Max", None);
        assert!(!exact && bw > 500.0);
    }

    #[test]
    fn bits_parametros_y_moe_desde_el_nombre() {
        assert_eq!(parse_bits(&["4-bit".into()], "x"), Some(4.0));
        assert_eq!(parse_bits(&[], "mlx-community/Qwen3-32B-bf16"), Some(16.0));
        assert_eq!(
            parse_bits(&[], "mlx-community/gpt-oss-20b-MXFP4-Q8"),
            Some(4.0)
        );
        assert_eq!(parse_bits(&[], "org/Model-6bit"), Some(6.0));
        assert_eq!(
            active_params("vanch007/Huihui-Qwen3.6-35B-A3B-abliterated-mlx-4bit"),
            Some(3.0)
        );
        assert_eq!(
            active_params("mlx-community/Qwen3-235B-A22B-4bit"),
            Some(22.0)
        );
        assert_eq!(
            active_params("mlx-community/gpt-oss-20b-MXFP4-Q8"),
            Some(3.6)
        );
        assert_eq!(active_params("mlx-community/Qwen3-32B-4bit"), None);
        assert_eq!(params_from_name("mlx-community/Qwen3-0.6B-8bit"), Some(0.6));
        assert_eq!(
            params_from_name("mlx-community/Mixtral-8x7B-4bit"),
            Some(56.0)
        );
    }

    /// La fórmula reproduce lo MEDIDO en este M1 Max (±20 %), y el peso, lo real.
    #[test]
    fn la_velocidad_estimada_cuadra_con_lo_medido() {
        let mut p = serde_json::Map::new();
        p.insert("U32".into(), serde_json::json!(32_000_000_000u64));
        p.insert("BF16".into(), serde_json::json!(800_000_000u64));
        let b = weight_bytes(&p, 4.0);
        assert!((b as f64 / 1e9 - 19.6).abs() < 0.5, "{b}");
        let near = |got: f64, want: f64| (got - want).abs() / want < 0.2;
        // Qwen3-32B-4bit-DWQ: 18,43 GB → 13,7 tok/s medidos
        assert!(near(estimate_tok_s(400.0, 18_430_000_000, false), 13.7));
        // Huihui 35B-A3B MoE: ~1,7 GB activos → 37 tok/s medidos
        assert!(near(estimate_tok_s(400.0, 1_700_000_000, true), 37.0));
        // 0.6B 8-bit: ~0,65 GB → 81-88 medidos
        assert!(near(estimate_tok_s(400.0, 650_000_000, false), 88.0));
        assert_eq!(speed_tag(90.0), "muy-rapido");
        assert_eq!(speed_tag(13.7), "medio");
        assert_eq!(speed_tag(3.0), "muy-lento");
    }

    #[test]
    fn un_modelo_de_la_api_se_mide_contra_la_mac() {
        let m = serde_json::json!({"id": "mlx-community/Qwen3.8-27B-4bit", "downloads": 414046, "likes": 234,
            "pipeline_tag": "image-text-to-text", "tags": ["mlx", "qwen3_5", "4-bit"], "lastModified": "2026-09-14T12:37:43.000Z",
            "safetensors": {"parameters": {"U32": 26893352960u64, "BF16": 463375600u64}, "total": 27356728560u64}});
        let big = MacProfile::simulated("M1 Max|64").unwrap();
        let small = MacProfile::simulated("M2|8").unwrap();
        let mut inst = HashSet::new();
        inst.insert("mlx-community/qwen3.8-27b-4bit".to_string());
        let e = entry_from_api(&m, &big, &inst).unwrap();
        assert!(e.vision && e.installed && e.fits && e.bits == Some(4.0));
        assert!((e.bytes as f64 / 1e9 - 16.1).abs() < 0.5, "{}", e.bytes);
        // el denso híbrido qwen3_5 va lento DE VERDAD (6,7–11 tok/s medidos en este M1 Max)
        assert_eq!(e.speed, "lento", "{}", e.tok_s);
        let e8 = entry_from_api(&m, &small, &HashSet::new()).unwrap();
        assert!(!e8.fits, "27B en 8 GB no cabe");
        // el peso real (suma de los archivos) manda sobre la estimación por parámetros
        let mut optiq = serde_json::json!({"id": "mlx-community/Qwen3.5-4B-OptiQ-4bit", "pipeline_tag": "text-generation", "tags": ["mlx", "4-bit"],
            "safetensors": {"parameters": {"U32": 4204789760u64, "BF16": 960768u64}, "total": 4205751296u64}});
        let estimado = entry_from_api(&optiq, &big, &Default::default()).unwrap().bytes;
        assert!((estimado as f64 / 1e9 - 2.4).abs() < 0.2, "{estimado}");
        optiq["gotv_bytes"] = serde_json::json!(4_020_000_000u64);
        assert_eq!(entry_from_api(&optiq, &big, &Default::default()).unwrap().bytes, 4_020_000_000);
        // la voz y los embeddings no son modelos de chat: fuera de la biblioteca
        let asr = serde_json::json!({"id": "mlx-community/Qwen3-ASR-0.6B-8bit", "tags": ["speech-to-text"], "safetensors": {"parameters": {"U32": 1}, "total": 1}});
        assert!(entry_from_api(&asr, &big, &HashSet::new()).is_none());
    }

    #[test]
    fn las_macs_simuladas_son_validas() {
        for id in PRESETS {
            let p = MacProfile::simulated(id).unwrap_or_else(|| panic!("{id}"));
            assert!(p.bandwidth > 60.0 && p.budget > 4 * GIB, "{id}");
        }
        assert!(MacProfile::simulated("M1|3").is_none());
        assert!(MacProfile::simulated("basura").is_none());
    }
}
