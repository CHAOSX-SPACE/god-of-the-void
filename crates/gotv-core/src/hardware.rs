//! Perfil de hardware LEÍDO, nunca fijado (E8, orden del Portador: "que use bien
//! la IA en cualquier Mac, de 16 GB a una Ultra de 512 GB").
//!
//! Fuente: Metal (`MTLDevice.recommendedMaxWorkingSetSize`), en Rust, sin Python:
//! que es el presupuesto real que MLX puede usar en ESA máquina (refleja el
//! `iogpu.wired_limit_mb` si el usuario lo subió). Respaldo sin MLX: sysctl.

use serde::{Deserialize, Serialize};

use std::process::Command;

pub const GIB: u64 = 1 << 30;
pub const MIB: u64 = 1 << 20;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Hardware {
    pub device: String,
    /// Memoria unificada total.
    pub memory: u64,
    /// Lo que MLX puede usar de verdad (presupuesto de GPU).
    pub gpu_budget: u64,
    /// Tamaño máximo de UN tensor.
    pub max_buffer: u64,
    /// `iogpu.wired_limit_mb` vigente (0 = el de fábrica).
    pub wired_limit_mb: u64,
    /// "mlx" | "estimado (sin mlx)" | "simulado"
    pub source: String,
}

impl Hardware {
    /// Perfil sintético para pruebas (16 GB, 64 GB, 512 GB…).
    pub fn simulated(device: &str, memory_gb: u64, budget_gb: f64) -> Self {
        Self {
            device: device.into(),
            memory: memory_gb * GIB,
            gpu_budget: (budget_gb * GIB as f64) as u64,
            max_buffer: memory_gb * GIB / 2,
            wired_limit_mb: 0,
            source: "simulado".into(),
        }
    }

    /// Lee el perfil real directo de Metal (Rust, sin Python). Es la misma
    /// fuente de la que MLX saca su `max_recommended_working_set_size`.
    pub fn detect() -> Self {
        let memory = sysctl_u64("hw.memsize").unwrap_or(0);
        let wired = sysctl_u64("iogpu.wired_limit_mb").unwrap_or(0);
        from_metal(memory, wired).unwrap_or_else(|| estimate(memory, wired))
    }

    pub fn summary(&self) -> String {
        crate::tr!(
            "{} · memoria {:.0} GB · presupuesto MLX {:.1} GB · tensor máx {:.1} GB · límite GPU {} · fuente: {}",
            "{} · memory {:.0} GB · MLX budget {:.1} GB · max tensor {:.1} GB · GPU limit {} · source: {}",
            self.device,
            self.memory as f64 / GIB as f64,
            self.gpu_budget as f64 / GIB as f64,
            self.max_buffer as f64 / GIB as f64,
            if self.wired_limit_mb == 0 {
                crate::i18n::t("de fábrica", "factory default").to_string()
            } else {
                format!("{} MB", self.wired_limit_mb)
            },
            self.source
        )
    }
}

fn from_metal(memory: u64, wired: u64) -> Option<Hardware> {
    let dev = metal::Device::system_default()?;
    let budget = dev.recommended_max_working_set_size();
    if budget == 0 {
        return None;
    }
    Some(Hardware {
        device: dev.name().to_string(),
        memory,
        gpu_budget: budget,
        max_buffer: dev.max_buffer_length(),
        wired_limit_mb: wired,
        source: "metal".into(),
    })
}

/// Sin MLX: estimación de la política de macOS (≈2/3 de la RAM en equipos
/// chicos, ≈3/4 en grandes) o el límite fijado por el usuario.
fn estimate(memory: u64, wired: u64) -> Hardware {
    let device = Command::new("sysctl")
        .args(["-n", "machdep.cpu.brand_string"])
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "Mac".into());
    let gpu_budget = if wired > 0 {
        wired * MIB
    } else if memory >= 36 * GIB {
        memory / 4 * 3
    } else {
        memory / 3 * 2
    };
    Hardware {
        device,
        memory,
        gpu_budget,
        max_buffer: memory / 2,
        wired_limit_mb: wired,
        source: crate::i18n::t("estimado (sin mlx)", "estimated (no mlx)").into(),
    }
}

pub fn sysctl_u64(name: &str) -> Option<u64> {
    let o = Command::new("sysctl").args(["-n", name]).output().ok()?;
    String::from_utf8_lossy(&o.stdout).trim().parse().ok()
}

/// Memoria que se puede ocupar YA (libre + inactiva + especulativa + purgable).
/// Si otro programa tiene un modelo cargado, aquí se nota.
pub fn available_now() -> Option<u64> {
    let o = Command::new("vm_stat").output().ok()?;
    parse_vm_stat(&String::from_utf8_lossy(&o.stdout))
}

pub fn parse_vm_stat(text: &str) -> Option<u64> {
    let page: u64 = text
        .lines()
        .next()?
        .split("page size of ")
        .nth(1)?
        .split_whitespace()
        .next()?
        .parse()
        .ok()?;
    let mut pages = 0u64;
    for line in text.lines() {
        let Some((k, v)) = line.split_once(':') else {
            continue;
        };
        if matches!(
            k.trim(),
            "Pages free" | "Pages inactive" | "Pages speculative" | "Pages purgeable"
        ) {
            pages += v.trim().trim_end_matches('.').parse::<u64>().unwrap_or(0);
        }
    }
    Some(pages * page)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lee_vm_stat_real() {
        let t = "Mach Virtual Memory Statistics: (page size of 16384 bytes)\n\
                 Pages free:                                  2275017.\n\
                 Pages active:                                 705965.\n\
                 Pages inactive:                               688335.\n\
                 Pages speculative:                             20014.\n\
                 Pages wired down:                             241004.\n\
                 Pages purgeable:                               13514.\n";
        let b = parse_vm_stat(t).unwrap();
        assert_eq!(b, (2275017 + 688335 + 20014 + 13514) * 16384);
    }

    #[test]
    fn estimacion_sin_mlx() {
        assert_eq!(estimate(64 * GIB, 0).gpu_budget, 48 * GIB);
        assert_eq!(estimate(64 * GIB, 57344).gpu_budget, 56 * GIB);
        assert_eq!(estimate(18 * GIB, 0).gpu_budget, 12 * GIB);
    }
}
