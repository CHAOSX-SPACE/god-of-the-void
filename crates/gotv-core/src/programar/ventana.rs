//! F2 · Configuración → «Automatizaciones» (F2.1 paso 3, F2.3 paso 6): los interruptores de avisos
//! (`[avisos]` de la config global), las tareas programadas con su próxima corrida, encender/apagar y
//! quitar, y si el `LaunchAgent` está puesto. La ventana sólo pinta esto; todo pasa por aquí.
use super::tareas::Registro;
use anyhow::{anyhow, bail, Result};
use std::path::Path;

/// Una tarea como la ve el humano.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct VistaTarea {
    pub id: String,
    pub nombre: String,
    pub cron: String,
    pub proyecto: String,
    pub orden: String,
    pub activa: bool,
    /// Hora local legible («—» si no vuelve a ocurrir).
    pub proxima: String,
    pub ultima: String,
    pub estado: String,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct Estado {
    pub turno: bool,
    pub permiso: bool,
    pub programadas: bool,
    pub tareas: Vec<VistaTarea>,
    /// Está el `LaunchAgent` (corre las tareas con la app cerrada).
    pub agente: bool,
}

pub fn estado_en(registro: &Path, config_global: &Path, casa: &Path) -> Result<Estado> {
    let mut i = crate::avisos::Interruptores::default();
    i.aplicar(&std::fs::read_to_string(config_global).unwrap_or_default());
    let r = Registro::cargar(registro)?;
    let tareas = r
        .tareas
        .iter()
        .map(|t| VistaTarea {
            id: t.id.clone(),
            nombre: t.nombre.clone(),
            cron: t.cron.clone(),
            proyecto: t.proyecto.display().to_string(),
            orden: t.orden.chars().take(300).collect(),
            activa: t.activa,
            proxima: if t.activa { super::hora_local(t.proxima) } else { "—".into() },
            ultima: super::hora_local(t.ultima),
            estado: t.estado.clone().unwrap_or_default(),
        })
        .collect();
    Ok(Estado { turno: i.turno, permiso: i.permiso, programadas: i.programadas, tareas, agente: super::launchd::ruta_plist(casa).is_file() })
}

pub fn estado() -> Result<Estado> {
    estado_en(&super::tareas::ruta_por_omision(), &crate::config::global_path(), &crate::paths::home())
}

/// Un interruptor de `[avisos]` en la config (sólo los tres que existen).
pub fn poner_aviso(config: &Path, clave: &str, si: bool) -> Result<()> {
    if !matches!(clave, "turno" | "permiso" | "programadas") {
        bail!("{}", crate::tr!("no hay aviso «{clave}»", "there is no notification «{clave}»"));
    }
    let mut t: toml::Table = match std::fs::read_to_string(config) {
        Ok(s) => s.parse().map_err(|e| anyhow!("{}: {e}", config.display()))?,
        Err(_) => Default::default(),
    };
    let a = t.entry("avisos").or_insert_with(|| toml::Value::Table(Default::default()));
    if !a.is_table() {
        *a = toml::Value::Table(Default::default());
    }
    a.as_table_mut().expect("tabla").insert(clave.into(), toml::Value::Boolean(si));
    if let Some(d) = config.parent() {
        std::fs::create_dir_all(d)?;
    }
    std::fs::write(config, toml::to_string_pretty(&t)?)?;
    Ok(())
}

pub fn activar(registro: &Path, id: &str, si: bool) -> Result<()> {
    Registro::con(registro, |r| r.activar(id, si, super::chat::ahora()))
}

pub fn quitar(registro: &Path, id: &str) -> Result<()> {
    Registro::con(registro, |r| r.quitar(id).map(|_| ()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::programar::tareas::Nueva;

    /// Lo que pinta la sección: interruptores de la config, tareas con su próxima corrida, apagar (sin
    /// próxima), encender (vuelve a calcularla), quitar; y un aviso escrito deja el resto de la config intacta.
    #[test]
    fn la_seccion_de_automatizaciones() {
        let d = crate::CarpetaPrueba::nueva("gotv-auto-ventana");
        let (reg, cfg) = (d.join("programadas.bin"), d.join("config.toml"));
        std::fs::write(&cfg, "mode = \"plan\"\n").unwrap();
        let id = Registro::con(&reg, |r| r.crear(Nueva { nombre: "pruebas".into(), cron: "0 9 * * *".into(), proyecto: d.to_path_buf(), orden: "corre las pruebas".into(), ..Default::default() }, crate::programar::chat::ahora()).map(|t| t.id)).unwrap();
        let e = estado_en(&reg, &cfg, &d).unwrap();
        assert!(e.turno && e.permiso && e.programadas && !e.agente);
        assert_eq!((e.tareas.len(), e.tareas[0].nombre.as_str(), e.tareas[0].activa), (1, "pruebas", true));
        assert_ne!(e.tareas[0].proxima, "—");
        activar(&reg, &id, false).unwrap();
        assert_eq!(estado_en(&reg, &cfg, &d).unwrap().tareas[0].proxima, "—");
        activar(&reg, &id, true).unwrap();
        assert_ne!(estado_en(&reg, &cfg, &d).unwrap().tareas[0].proxima, "—");
        poner_aviso(&cfg, "turno", false).unwrap();
        assert!(poner_aviso(&cfg, "otro", false).is_err());
        let e = estado_en(&reg, &cfg, &d).unwrap();
        assert!(!e.turno && e.permiso);
        assert!(std::fs::read_to_string(&cfg).unwrap().contains("mode = \"plan\""), "el resto de la config se queda");
        quitar(&reg, &id).unwrap();
        assert!(estado_en(&reg, &cfg, &d).unwrap().tareas.is_empty());
        assert!(quitar(&reg, &id).is_err());
        // una config ROTA no se pisa (se perdería todo lo demás que tiene)
        std::fs::write(&cfg, "mode = \"plan\"\n[roto\nmodel = \"x\"\n").unwrap();
        assert!(poner_aviso(&cfg, "turno", true).is_err());
        assert!(std::fs::read_to_string(&cfg).unwrap().contains("model = \"x\""), "el archivo quedó intacto");
    }
}
