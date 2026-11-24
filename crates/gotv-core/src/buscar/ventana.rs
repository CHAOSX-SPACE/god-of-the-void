//! F23.2 · Lo que necesita Configuración → Conexiones → «Buscadores» (Anexo B: `buscadores_lista`,
//! `buscadores_llave`, `buscadores_probar`, `buscadores_tope`, `buscadores_orden`, `buscadores_ficha`,
//! `buscadores_uso`, `busquedas_borrar`). Cada orden de la ventana es una llamada de aquí.
//!
//! La llave va al Llavero y NUNCA vuelve (la ventana sólo sabe si hay una). Los ajustes se escriben
//! en `[buscadores]` del `config.toml` del USUARIO; un proyecto no los puede aflojar.

use super::legal::{Ficha, Guardar};
use super::{Ajustes, Contexto};
use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use std::path::Path;

/// La consulta fija del botón «Probar» (genérica, sin nada del usuario: F23.L-8).
pub const CONSULTA_PRUEBA: &str = "Rust programming language"; // juez-idioma: datos (consulta fija)

fn guardar_texto(f: &Ficha) -> &'static str {
    match f.guardar() {
        Guardar::No => "no",
        Guardar::Cache { .. } => "cache",
        Guardar::Si => "si",
    }
}

/// Una fila por servicio, en el orden en que se consultan (y luego los demás).
pub fn lista(ctx: &Contexto) -> Value {
    let ahora = ctx.reloj.ahora();
    let uso = ctx.almacen.uso();
    let mut ids: Vec<String> = ctx.ajustes.orden.clone();
    for r in &ctx.ajustes.respaldo {
        if !ids.contains(r) {
            ids.push(r.clone());
        }
    }
    for id in ctx.fichas.keys() {
        if !ids.contains(id) {
            ids.push(id.clone());
        }
    }
    let filas: Vec<Value> = ids
        .iter()
        .filter_map(|id| ctx.fichas.get(id))
        .map(|f| {
            let problemas = f.problemas(ahora, uso.hash_visto(&f.id).as_deref());
            json!({
                "id": f.id,
                "nombre": f.nombre,
                "publica": f.publica,
                "necesita_llave": f.necesita_llave,
                "tiene_llave": !f.necesita_llave || ctx.llaves.llave(&f.id).is_some(),
                "donde_llave": f.donde_llave,
                "activo": ctx.ajustes.activo(f) && problemas.is_empty(),
                "pedido_activo": ctx.ajustes.activo(f),
                "activable": problemas.is_empty(),
                "problemas": problemas,
                "aviso": f.aviso(),
                "en_orden": ctx.ajustes.orden.contains(&f.id),
                "en_respaldo": ctx.ajustes.respaldo.contains(&f.id),
                "tope_mes": ctx.ajustes.tope(&f.id),
                "usado_mes": uso.del_mes(&f.id, ahora),
                "precio": f.precio.texto.ver(),
                "precio_fecha": f.precio.fecha,
                "guardar": guardar_texto(f),
                "que_guarda": f.resumen(),
                "atribucion": f.atribucion,
            })
        })
        .collect();
    let (cache, indice) = ctx.almacen.ocupa();
    json!({
        "servicios": filas,
        "paralelo": ctx.ajustes.paralelo,
        "solo_local": ctx.ajustes.solo_local,
        "contacto": ctx.ajustes.contacto,
        "ocupa": { "cache": cache, "indice": indice },
    })
}

/// «Ver ficha legal»: la ficha entera (sin nada secreto: las fichas son públicas).
pub fn ficha(ctx: &Contexto, id: &str) -> Result<Value> {
    let f = ctx.fichas.get(id).ok_or_else(|| anyhow!("{}", crate::tr!("no hay ficha de «{id}»", "there is no sheet for «{id}»")))?;
    let ahora = ctx.reloj.ahora();
    Ok(json!({
        "id": f.id, "nombre": f.nombre, "publica": f.publica,
        "condiciones_url": f.condiciones_url, "condiciones_revisadas": f.condiciones_revisadas,
        "condiciones_hash": f.condiciones_hash, "hash_visto": ctx.almacen.uso().hash_visto(id),
        "revisada_por": f.revisada_por, "estado": f.estado,
        "guardar": guardar_texto(f), "cache_horas": f.cache_horas, "que_guarda": f.resumen(),
        "mostrar_al_usuario": f.mostrar_al_usuario, "atribucion": f.atribucion,
        "prohibe_entrenar_ia": f.prohibe_entrenar_ia,
        "limites": { "por_segundo": f.limites.por_segundo, "por_mes": f.limites.por_mes, "nota": f.limites.nota.ver() },
        "precio": { "texto": f.precio.texto.ver(), "fecha": f.precio.fecha, "fuente": f.precio.fuente },
        "dominio_api": f.dominio_api, "donde_llave": f.donde_llave,
        "notas_legales": f.notas_legales.ver(), "fuentes": f.fuentes,
        "problemas": f.problemas(ahora, ctx.almacen.uso().hash_visto(id).as_deref()),
    }))
}

/// Uso del mes por servicio («Brave: 312 de 1 000 este mes»).
pub fn uso(ctx: &Contexto) -> Value {
    let ahora = ctx.reloj.ahora();
    let u = ctx.almacen.uso();
    Value::Object(
        ctx.fichas
            .keys()
            .map(|id| (id.clone(), json!({ "usado": u.del_mes(id, ahora), "tope": ctx.ajustes.tope(id), "mes": super::legal::mes_de(ahora) })))
            .collect(),
    )
}

/// F23.L-12 · «Borrar caché e índice»: lo liberado en bytes.
pub fn borrar(ctx: &Contexto) -> Result<Value> {
    let (c, i) = ctx.almacen.borrar_cache_e_indice()?;
    Ok(json!({ "cache": c, "indice": i }))
}

/// F23.L-7 · La llave al Llavero (`buscador:<id>`). Nunca se devuelve ni se escribe en otro sitio.
pub fn poner_llave(ctx: &Contexto, id: &str, llave: &str) -> Result<()> {
    let f = ctx.fichas.get(id).ok_or_else(|| anyhow!("{}", crate::tr!("no hay ficha de «{id}»", "there is no sheet for «{id}»")))?;
    if !f.necesita_llave {
        return Err(anyhow!("{}", crate::tr!("{} no usa llave", "{} uses no key", f.nombre)));
    }
    crate::llaves::set(&format!("buscador:{id}"), llave)
}

pub fn quitar_llave(id: &str) {
    crate::llaves::delete(&format!("buscador:{id}"));
}

/// «Probar»: 3 resultados de una consulta fija SÓLO a ese servicio, y lo gastado este mes.
pub async fn probar(ctx: &Contexto, id: &str) -> Result<Value> {
    let r = super::buscar(ctx, CONSULTA_PRUEBA, 3, Some(vec![id.to_string()])).await?;
    let paso = r.pasos.iter().find(|p| p.servicio == id).cloned();
    let (ok, detalle) = match paso.as_ref().map(|p| &p.estado) {
        Some(super::Estado::Respondio(k)) | Some(super::Estado::DeLaCache(k)) => (*k > 0, String::new()),
        Some(super::Estado::SinRespuesta) => (false, crate::i18n::t("no respondió a tiempo", "did not answer in time").to_string()),
        Some(super::Estado::Fallo(m)) | Some(super::Estado::Omitido(m)) => (false, m.clone()),
        None => (false, String::new()),
    };
    Ok(json!({
        "ok": ok,
        "detalle": detalle,
        "resultados": r.hallazgos.iter().map(|h| json!({ "titulo": h.titulo, "url": h.url })).collect::<Vec<_>>(),
        "usado_mes": ctx.almacen.uso().del_mes(id, ctx.reloj.ahora()),
        "tope_mes": ctx.ajustes.tope(id),
    }))
}

/// F23.L-2 · Vuelve a leer las condiciones y anota su huella. Si cambió, la ficha queda vencida
/// (el servicio se apaga) hasta una revisión nueva.
pub async fn revisar_condiciones(ctx: &Contexto, id: &str) -> Result<Value> {
    let f = ctx.fichas.get(id).ok_or_else(|| anyhow!("{}", crate::tr!("no hay ficha de «{id}»", "there is no sheet for «{id}»")))?;
    let h = huella_remota(&f.condiciones_url, &ctx.ajustes.contacto).await?;
    ctx.almacen.uso().anotar_hash(id, &h, ctx.reloj.ahora())?;
    Ok(json!({ "igual": h == f.condiciones_hash, "huella": h, "ficha": f.condiciones_hash }))
}

/// La huella del texto de unas condiciones publicadas (HTML → texto; PDF → sus bytes).
pub async fn huella_remota(url: &str, contacto: &str) -> Result<String> {
    let c = reqwest::Client::builder()
        .user_agent(super::servicios::user_agent(contacto))
        .timeout(std::time::Duration::from_secs(20))
        .build()?;
    let r = c.get(url).send().await?;
    let pdf = r.headers().get("content-type").and_then(|v| v.to_str().ok()).is_some_and(|t| t.contains("pdf")) || url.ends_with(".pdf");
    let estado = r.status();
    if !estado.is_success() {
        return Err(anyhow!("HTTP {estado}"));
    }
    let b = r.bytes().await?;
    Ok(if pdf {
        use sha2::{Digest, Sha256};
        format!("sha256:{}", Sha256::digest(&b).iter().map(|x| format!("{x:02x}")).collect::<String>())
    } else {
        super::legal::huella_de_texto(&String::from_utf8_lossy(&b))
    })
}

// ── escribir `[buscadores]` en el config.toml del usuario ────────────────

fn leer_tabla(ruta: &Path) -> Result<toml::Table> {
    match std::fs::read_to_string(ruta) {
        Ok(t) => t.parse::<toml::Table>().map_err(|e| anyhow!("{}: {e}", ruta.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(toml::Table::new()),
        Err(e) => Err(e.into()),
    }
}

fn escribir_buscadores(ruta: &Path, f: impl FnOnce(&mut toml::Table)) -> Result<()> {
    let mut t = leer_tabla(ruta)?;
    let b = t.entry("buscadores").or_insert_with(|| toml::Value::Table(Default::default()));
    let b = b.as_table_mut().ok_or_else(|| anyhow!("[buscadores] no es una tabla"))?; // juez-idioma: datos (clave de config)
    f(b);
    if let Some(d) = ruta.parent() {
        std::fs::create_dir_all(d)?;
    }
    std::fs::write(ruta, toml::to_string_pretty(&t)?)?;
    Ok(())
}

fn servicio_mut<'a>(b: &'a mut toml::Table, id: &str) -> &'a mut toml::Table {
    let v = b.entry(id.to_string()).or_insert_with(|| toml::Value::Table(Default::default()));
    if !v.is_table() {
        *v = toml::Value::Table(Default::default());
    }
    v.as_table_mut().expect("tabla")
}

/// Encender o apagar. Encender un servicio sin ficha completa, revisada y al día NO se puede
/// (F23.L-1: lo impide el código), ni uno de llave sin su llave.
pub fn activar(ctx: &Contexto, config: &Path, id: &str, si: bool) -> Result<()> {
    let f = ctx.fichas.get(id).ok_or_else(|| anyhow!("{}", crate::tr!("no hay ficha de «{id}»", "there is no sheet for «{id}»")))?;
    if si {
        f.activable(ctx.reloj.ahora(), ctx.almacen.uso().hash_visto(id).as_deref())?;
        if f.necesita_llave && ctx.llaves.llave(id).is_none() {
            return Err(anyhow!("{}", crate::tr!("{}: primero pon su llave", "{}: set its key first", f.nombre)));
        }
    }
    escribir_buscadores(config, |b| {
        servicio_mut(b, id).insert("activo".into(), toml::Value::Boolean(si));
    })
}

/// Tope mensual (0 = sin consultas; `None` = sin tope).
pub fn poner_tope(config: &Path, id: &str, tope: Option<u64>) -> Result<()> {
    escribir_buscadores(config, |b| {
        let s = servicio_mut(b, id);
        match tope {
            Some(n) => {
                s.insert("tope_mes".into(), toml::Value::Integer(n.min(i64::MAX as u64) as i64));
            }
            None => {
                s.remove("tope_mes");
            }
        }
    })
}

/// Orden, respaldo, paralelo, sólo local y contacto. Ids desconocidos fuera.
pub fn poner_orden(ctx: &Contexto, config: &Path, orden: &[String], respaldo: &[String], paralelo: bool, solo_local: bool) -> Result<()> {
    let conocido = |x: &String| ctx.fichas.contains_key(x) || x == "local";
    let orden: Vec<toml::Value> = orden.iter().filter(|x| conocido(x)).map(|x| toml::Value::String(x.clone())).collect();
    let respaldo: Vec<toml::Value> = respaldo.iter().filter(|x| conocido(x)).map(|x| toml::Value::String(x.clone())).collect();
    escribir_buscadores(config, |b| {
        b.insert("orden".into(), toml::Value::Array(orden));
        b.insert("respaldo".into(), toml::Value::Array(respaldo));
        b.insert("paralelo".into(), toml::Value::Boolean(paralelo));
        b.insert("solo_local".into(), toml::Value::Boolean(solo_local));
    })
}

pub fn poner_contacto(config: &Path, contacto: &str) -> Result<()> {
    escribir_buscadores(config, |b| {
        b.insert("contacto".into(), toml::Value::String(contacto.trim().to_string()));
    })
}

/// Recarga los ajustes desde un `config.toml` concreto (pruebas y la ventana tras escribir).
pub fn ajustes_de(config: &Path) -> Ajustes {
    let t = leer_tabla(config).ok();
    Ajustes::desde(t.as_ref().and_then(|t| t.get("buscadores")), None)
}
