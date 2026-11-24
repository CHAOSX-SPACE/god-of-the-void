//! F10.3 · Los archivos por el EDITOR: si el editor (ACP) ofrece `fs/read_text_file` y `fs/write_text_file`,
//! las herramientas `read_file`, `write_file`, `edit` y `multi_edit` leen y escriben por él, así ven lo que
//! tiene SIN GUARDAR (su búfer manda sobre el disco). Fuera de un chat del editor, el disco, como siempre.
//! `grep`, `glob` y `bash` siguen mirando el disco: lo dice la ficha de la entrega.
use async_trait::async_trait;
use std::path::Path;
use std::sync::Arc;

#[async_trait]
pub trait Editor: Send + Sync {
    /// El texto del archivo según el editor (`None` = no lo sabe dar: se lee del disco).
    async fn leer(&self, ruta: &Path) -> Option<String>;
    async fn escribir(&self, ruta: &Path, texto: &str) -> anyhow::Result<()>;
}

tokio::task_local! {
    /// El editor del chat que corre en esta tarea (lo pone `acp` alrededor de cada orden).
    pub static EDITOR: Arc<dyn Editor>;
}

fn editor() -> Option<Arc<dyn Editor>> {
    EDITOR.try_with(|e| e.clone()).ok()
}

pub async fn leer_texto(ruta: &Path) -> std::io::Result<String> {
    if let Some(e) = editor() {
        if let Some(t) = e.leer(ruta).await {
            return Ok(t);
        }
    }
    std::fs::read_to_string(ruta)
}

pub async fn leer_bytes(ruta: &Path) -> std::io::Result<Vec<u8>> {
    if let Some(e) = editor() {
        if let Some(t) = e.leer(ruta).await {
            return Ok(t.into_bytes());
        }
    }
    std::fs::read(ruta)
}

pub async fn escribir(ruta: &Path, texto: &str) -> std::io::Result<()> {
    match editor() {
        Some(e) => e.escribir(ruta, texto).await.map_err(std::io::Error::other),
        None => std::fs::write(ruta, texto),
    }
}

/// `write_file` con `append`: por el editor (su búfer + lo nuevo). `false` = no hay editor: lo hace el disco.
pub async fn anexar(ruta: &Path, texto: &str) -> std::io::Result<bool> {
    let Some(e) = editor() else { return Ok(false) };
    let previo = match e.leer(ruta).await {
        Some(t) => t,
        None => std::fs::read_to_string(ruta).unwrap_or_default(),
    };
    e.escribir(ruta, &format!("{previo}{texto}")).await.map_err(std::io::Error::other)?;
    Ok(true)
}
